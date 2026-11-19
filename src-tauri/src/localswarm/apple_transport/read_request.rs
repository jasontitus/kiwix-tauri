// SPDX-License-Identifier: GPL-3.0-or-later
//! Bind a single Network.framework completion to its live receive operation.
#![forbid(unsafe_code)]

use dispatch2::DispatchData;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tokio::sync::oneshot;

type ReadResult = Result<Vec<u8>, String>;

struct Pending {
    maximum: usize,
    reply: oneshot::Sender<ReadResult>,
}

#[derive(Default)]
struct Registry {
    last_id: u64,
    pending: HashMap<u64, Pending>,
}

static READS: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

pub(super) struct Ticket(u64);

impl Ticket {
    pub(super) fn id(&self) -> u64 {
        self.0
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        READS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .remove(&self.0);
    }
}

pub(super) fn register(maximum: usize) -> Result<(Ticket, oneshot::Receiver<ReadResult>), String> {
    let (reply, received) = oneshot::channel();
    let mut reads = READS.lock().unwrap_or_else(|e| e.into_inner());
    let id = reads
        .last_id
        .checked_add(1)
        .ok_or("Apple QUIC receive IDs exhausted")?;
    reads.last_id = id;
    reads.pending.insert(id, Pending { maximum, reply });
    Ok((Ticket(id), received))
}

pub(super) extern "C" fn complete(id: u64, status: i32, data: Option<&DispatchData>) {
    let pending = READS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pending
        .remove(&id);
    let Some(Pending { maximum, reply }) = pending else {
        return;
    };
    if reply.is_closed() {
        return;
    }
    let result = match data {
        _ if status != 0 => {
            Err("LocalSwarm Apple QUIC connection failed or left the local link".into())
        }
        Some(data) if data.size() > maximum => {
            Err("Apple QUIC receive exceeded its request".into())
        }
        Some(data) => Ok(data.to_vec()),
        None => Ok(Vec::new()),
    };
    let _ = reply.send(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_reads_respect_the_individual_request_limit() {
        let first = DispatchData::from_bytes(b"abc");
        let last = DispatchData::from_bytes(b"def");
        let data = first.concat(&last);
        let (small, short_result) = register(5).unwrap();
        let (full, full_result) = register(6).unwrap();
        complete(full.id(), 0, Some(&data));
        complete(small.id(), 0, Some(&data));
        assert!(short_result.blocking_recv().unwrap().is_err());
        assert_eq!(full_result.blocking_recv().unwrap().unwrap(), b"abcdef");
    }

    #[test]
    fn dropped_and_duplicate_callbacks_cannot_complete_another_request() {
        let (old, cancelled) = register(3).unwrap();
        let stale = old.id();
        drop(old);
        assert!(cancelled.blocking_recv().is_err());
        let (new, mut current) = register(3).unwrap();
        let bytes = DispatchData::from_bytes(b"now");
        complete(stale, 0, Some(&bytes));
        assert!(matches!(
            current.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        complete(new.id(), 0, Some(&bytes));
        complete(new.id(), -1, None);
        assert_eq!(current.blocking_recv().unwrap().unwrap(), b"now");
    }

    #[test]
    fn empty_data_and_transport_failure_survive_the_callback() {
        for (status, data) in [(0, None), (0, Some(DispatchData::empty())), (-1, None)] {
            let (ticket, received) = register(1).unwrap();
            complete(ticket.id(), status, data);
            let result = received.blocking_recv().unwrap();
            if status == 0 {
                assert!(result.unwrap().is_empty());
            } else {
                assert!(result.is_err());
            }
        }
    }
}
