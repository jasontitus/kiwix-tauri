// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU-time readings for diagnostics and time-limit tests.
//!
//! Wall-clock time charges a suspended app or a descheduled thread; CPU time
//! does not. Both readings are advisory: `None` means the platform call is
//! unavailable and the caller must fall back to wall time. Values are whole
//! microseconds, monotonic and per the clock named by each function.

/// This thread's CPU time since creation, in whole microseconds.
///
/// Unix reads `CLOCK_THREAD_CPUTIME_ID`; Windows sums the kernel and user times
/// from `GetThreadTimes`, which advances per clock tick (about 15.6 ms).
#[cfg(any(unix, windows))]
pub fn thread_cpu_time_micros() -> Option<u128> {
    #[cfg(unix)]
    {
        Some(micros(clock(rustix::time::ClockId::ThreadCPUTime)?))
    }
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::FILETIME,
            System::Threading::{GetCurrentThread, GetThreadTimes},
        };
        let mut times = [FILETIME::default(); 4];
        let [creation, exit, kernel, user] = &mut times;
        // SAFETY: `GetCurrentThread` returns a pseudo-handle for the calling
        // thread that needs no closing. All four pointers refer to the live
        // `times` array. Only `kernel` and `user` are read: Windows leaves the
        // exit time undefined for a thread that has not exited, which is always
        // true for the current thread.
        unsafe { GetThreadTimes(GetCurrentThread(), creation, exit, kernel, user) }.ok()?;
        // Kernel plus user time in 100 ns units; one microsecond is ten of them.
        let ticks = |time: &FILETIME| {
            (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
        };
        Some(u128::from(ticks(kernel).saturating_add(ticks(user))) / 10)
    }
}

/// Process-wide CPU time, all threads included, in whole microseconds.
///
/// The Android evaluation bridge charges the native import and search workers
/// that the measured operation starts, which a thread clock would miss; the
/// ignored throughput harness also reports whole-process CPU time. The helper
/// stays available on every Unix target without a dead-code warning elsewhere.
#[cfg(unix)]
#[allow(dead_code, reason = "only the debug Android evaluation bridge and the ignored throughput harness read process CPU time")]
pub fn process_cpu_time_micros() -> Option<u128> {
    Some(micros(clock(rustix::time::ClockId::ProcessCPUTime)?))
}

/// Reads one of the process/thread CPU clocks. `ClockId` only names clocks the
/// OS guarantees, but the call stays fallible so an unavailable clock reports
/// `None` instead of aborting the diagnostic that asked for it.
#[cfg(unix)]
fn clock(id: rustix::time::ClockId) -> Option<rustix::time::Timespec> {
    rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Known(id)).ok()
}

#[cfg(unix)]
fn micros(time: rustix::time::Timespec) -> u128 {
    // A CPU clock never reports a negative field; the clamps keep the widening
    // casts total instead of relying on that.
    time.tv_sec.max(0) as u128 * 1_000_000 + time.tv_nsec.clamp(0, 999_999_999) as u128 / 1000
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn cpu_micros_converts_fields_and_clamps_impossible_values() {
        let time = |tv_sec, tv_nsec| rustix::time::Timespec { tv_sec, tv_nsec };
        assert_eq!(micros(time(0, 0)), 0);
        assert_eq!(micros(time(2, 500_000)), 2_000_500);
        assert_eq!(micros(time(0, 999_999_999)), 999_999);
        // Sub-microsecond remainder truncates; a negative field cannot wrap.
        assert_eq!(micros(time(-1, -1)), 0);
    }
}
