// SPDX-License-Identifier: GPL-3.0-or-later
//! Match a typed Apple callback to its live Rust waiter without sharing ownership.
#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};
use tokio::sync::oneshot;

struct State<T> {
    last_id: u64,
    pending: HashMap<u64, oneshot::Sender<T>>,
}

pub(crate) struct Replies<T>(Mutex<State<T>>);

impl<T> Default for Replies<T> {
    fn default() -> Self {
        Self(Mutex::new(State {
            last_id: 0,
            pending: HashMap::new(),
        }))
    }
}

impl<T> Replies<T> {
    fn state(&self) -> MutexGuard<'_, State<T>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn register(&self) -> Option<Reply<'_, T>> {
        let mut state = self.state();
        let id = state.last_id.checked_add(1)?;
        let (sender, receiver) = oneshot::channel();
        state.last_id = id;
        state.pending.insert(id, sender);
        Some(Reply {
            owner: self,
            id,
            receiver,
        })
    }

    pub(crate) fn is_live(&self, id: u64) -> bool {
        self.state()
            .pending
            .get(&id)
            .is_some_and(|sender| !sender.is_closed())
    }

    pub(crate) fn finish(&self, id: u64, decode: impl FnOnce() -> T) {
        let sender = self.state().pending.remove(&id);
        if let Some(sender) = sender.filter(|sender| !sender.is_closed()) {
            let _ = sender.send(decode());
        }
    }
}

pub(crate) struct Reply<'a, T> {
    owner: &'a Replies<T>,
    id: u64,
    receiver: oneshot::Receiver<T>,
}

impl<T> Reply<'_, T> {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) async fn wait(mut self) -> Result<T, oneshot::error::RecvError> {
        (&mut self.receiver).await
    }
}

impl<T> Drop for Reply<'_, T> {
    fn drop(&mut self) {
        let sender = self.owner.state().pending.remove(&self.id);
        drop(sender);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_and_duplicate_replies_skip_decoding_and_cannot_reach_a_new_waiter() {
        let replies = Replies::default();
        let old = replies.register().unwrap();
        let old_id = old.id();
        assert!(replies.is_live(old_id));
        drop(old);
        assert!(!replies.is_live(old_id));

        let current = replies.register().unwrap();
        let id = current.id();
        assert!(id > old_id);
        replies.finish(old_id, || panic!("cancelled reply decoded"));
        assert!(replies.is_live(id));
        replies.finish(id, || {
            assert!(!replies.is_live(id));
            27
        });
        replies.finish(id, || panic!("duplicate reply decoded"));
        assert_eq!(current.wait().await.unwrap(), 27);
    }

    #[test]
    fn exhausted_ids_are_never_reused() {
        let replies = Replies::<()>::default();
        replies.state().last_id = u64::MAX - 1;
        let last = replies.register().unwrap();
        assert_eq!(last.id(), u64::MAX);
        assert!(replies.register().is_none());
        assert!(replies.is_live(last.id()));
        drop(last);
        assert!(replies.register().is_none());
    }

    #[test]
    fn dropping_an_awaiting_future_retires_its_request() {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };

        let replies = Replies::<()>::default();
        let reply = replies.register().unwrap();
        let id = reply.id();
        let mut waiter = Box::pin(reply.wait());
        assert_eq!(
            waiter
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        );
        drop(waiter);
        assert!(!replies.is_live(id));
        replies.finish(id, || panic!("reply after abort decoded"));
    }
}
