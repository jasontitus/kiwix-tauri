// Temporary owned-device diagnosis only; remove after the bounded capture.
#![forbid(unsafe_code)]
use std::time::Instant;
#[cfg(feature = "localswarm-profiling")]
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex};

#[cfg(feature = "localswarm-profiling")]
const PHASES: [&str; 12] = [
    "source_read_hash", "source_transport_hash", "payload_send", "payload_receive_hash",
    "transport_hash", "write_chunk", "checkpoint", "archive_sync",
    "socket_wait", "staging_check", "blocking_wait", "prefetch_wait",
];
#[cfg(feature = "localswarm-profiling")]
static ENABLED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "localswarm-profiling")]
static TOTALS: Mutex<[(u64, u64, u128); 12]> = Mutex::new([(0, 0, 0); 12]);

#[cfg(feature = "localswarm-profiling")]
pub(super) fn configure() { ENABLED.store(true, Ordering::Relaxed); }
#[cfg(all(feature = "localswarm-profiling", test))]
pub(super) fn reset() {
    *TOTALS.lock().unwrap_or_else(|error| error.into_inner()) = [(0, 0, 0); 12];
}
#[inline]
pub(super) fn start() -> Option<Instant> {
    #[cfg(feature = "localswarm-profiling")]
    if ENABLED.load(Ordering::Relaxed) { return Some(Instant::now()); }
    None
}
#[inline]
pub(super) fn record(_phase: usize, _bytes: usize, _started: Option<Instant>) {
    #[cfg(feature = "localswarm-profiling")]
    if let Some(started) = _started {
        let elapsed = started.elapsed();
        let mut totals = TOTALS.lock().unwrap_or_else(|error| error.into_inner());
        let (count, total_bytes, nanos) = &mut totals[_phase];
        if *count >= 4096 { return; }
        *count += 1;
        *total_bytes += _bytes as u64;
        *nanos += elapsed.as_nanos();
    }
}

#[cfg(feature = "localswarm-profiling")]
pub(super) fn snapshot() -> serde_json::Value {
    let totals = TOTALS.lock().unwrap_or_else(|error| error.into_inner());
    serde_json::Value::Array(PHASES.iter().zip(totals.iter()).map(|(phase, (count, bytes, nanos))|
        serde_json::json!({"phase":phase,"count":count,"bytes":bytes,"elapsed_ns":nanos.to_string()})).collect())
}
