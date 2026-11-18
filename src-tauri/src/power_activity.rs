// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Keep platform power management away from live transfers.
//!
//! A transfer that is not user-visible can be slowed by the system even while
//! its sockets are busy: macOS naps and coalesces timers for the process, and
//! Android Wi-Fi firmware drops into power save between the bursts of a QUIC
//! exchange. TCP hides both because its continuous ack stream keeps the radio
//! and the scheduler busy. The supported fixes are `NSProcessInfo` activity
//! assertions on macOS and a `WIFI_MODE_FULL_HIGH_PERF` Wi-Fi lock on Android.
//! One hold is shared by every live `Guard`, so nested and concurrent
//! transfers hold exactly one and the platform token is released as soon as
//! the last one ends.
//!
//! This is deliberately not `caffeinate` and not a global setting: the token
//! only exists while a transfer is running, needs no permission, and
//! disappears on cancellation, failure, shutdown and process exit (the kernel
//! releases it with the process).

use std::sync::Mutex;

/// One shared assertion and the number of guards holding it. The count and the
/// platform's first/last transitions share one lock: a new holder cannot begin
/// a replacement assertion while the previous last release is still ending it.
///
/// The platform hooks are function pointers so the balance can be tested with
/// local counters instead of the process-wide platform token.
struct Activity {
    holders: Mutex<usize>,
    begin: fn(&'static str),
    end: fn(),
}

impl Activity {
    const fn new(begin: fn(&'static str), end: fn()) -> Self {
        Self { holders: Mutex::new(0), begin, end }
    }

    /// Take a hold; the caller owns the matching release.
    fn acquire(&self, reason: &'static str) {
        let mut holders = self.holders.lock().unwrap_or_else(|error| error.into_inner());
        if *holders == 0 {
            (self.begin)(reason);
        }
        *holders += 1;
    }

    /// Drop a hold. The platform assertion ends with the last one.
    fn release(&self) {
        let mut holders = self.holders.lock().unwrap_or_else(|error| error.into_inner());
        *holders -= 1;
        if *holders == 0 {
            (self.end)();
        }
    }

    #[cfg(test)]
    fn holders(&self) -> usize {
        *self.holders.lock().unwrap_or_else(|error| error.into_inner())
    }
}

static PROCESS: Activity = Activity::new(platform::begin, platform::end);

/// Give the Android platform hook the application handle it needs to reach the
/// mobile plugin bridge. No-op platforms have nothing to install.
#[cfg(target_os = "android")]
pub(crate) fn install(app: tauri::AppHandle) {
    platform::install(app);
}

/// RAII hold on the process activity assertion.
///
/// The guard carries no platform handle: the one shared token lives in the
/// platform module, and the guard only balances the process-wide count. That
/// keeps the guard `Send`, so a transfer task can hold it across awaits.
pub(crate) struct Guard {
    activity: &'static Activity,
    held: bool,
}

impl Guard {
    /// Hold the assertion, beginning it if this is the first live guard.
    ///
    /// `reason` is shown by `pmset -g assertions`. Only the first holder's
    /// reason is used; later holders join the same assertion.
    pub(crate) fn acquire(reason: &'static str) -> Self {
        Self::acquire_on(&PROCESS, reason)
    }

    fn acquire_on(activity: &'static Activity, reason: &'static str) -> Self {
        activity.acquire(reason);
        Self { activity, held: true }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.held {
            return;
        }
        self.held = false;
        self.activity.release();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use block2::RcBlock;
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    use std::sync::{mpsc, Mutex};
    use std::thread;

    // `NSActivityUserInitiated` already contains `IdleSystemSleepDisabled`; the
    // explicit bit documents the intent and keeps the set stable if Foundation
    // ever redefines the composite.
    const OPTIONS: NSActivityOptions = NSActivityOptions::from_bits_truncate(
        NSActivityOptions::UserInitiated.0
            | NSActivityOptions::LatencyCritical.0
            | NSActivityOptions::IdleSystemSleepDisabled.0,
    );

    struct Request {
        reason: &'static str,
        ready: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
        ended: mpsc::SyncSender<()>,
    }

    fn run(queue: mpsc::Receiver<Request>) {
        for request in queue {
            let Request { reason, ready, release, ended } = request;
            objc2::rc::autoreleasepool(|_| {
                let block = RcBlock::new(move || {
                    if ready.send(()).is_ok() {
                        let _ = release.recv();
                    }
                });
                NSProcessInfo::processInfo().performActivityWithOptions_reason_usingBlock(
                    OPTIONS, &NSString::from_str(reason), &block,
                );
            });
            let _ = ended.send(());
        }
    }

    // Foundation owns the assertion for the duration of its synchronous block.
    // Only the worker enters that block. Transfer guards exchange Rust channels.
    struct Worker {
        queue: Option<mpsc::Sender<Request>>,
        active: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    }

    static WORKER: Mutex<Worker> = Mutex::new(Worker { queue: None, active: None });

    pub(super) fn begin(reason: &'static str) {
        let mut worker = WORKER.lock().unwrap_or_else(|error| error.into_inner());
        if worker.queue.is_none() {
            let (sender, receiver) = mpsc::channel();
            match thread::Builder::new().name("kiwix-power".into()).spawn(move || run(receiver)) {
                Ok(_) => worker.queue = Some(sender),
                Err(error) => {
                    eprintln!("Could not start the macOS transfer activity: {error}");
                    return;
                }
            }
        }
        let (ready, started) = mpsc::sync_channel(0);
        let (finish, release) = mpsc::channel();
        let (ended, done) = mpsc::sync_channel(0);
        let request = Request { reason, ready, release, ended };
        if worker.queue.as_ref().is_some_and(|queue| queue.send(request).is_ok())
            && started.recv().is_ok() {
            worker.active = Some((finish, done));
        } else {
            worker.queue = None;
            eprintln!("Could not start the macOS transfer activity");
        }
    }

    pub(super) fn end() {
        let mut worker = WORKER.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((finish, done)) = worker.active.take() {
            drop(finish);
            let _ = done.recv();
        }
    }
}

#[cfg(target_os = "android")]
mod platform {
    use std::sync::OnceLock;
    use tauri::AppHandle;
    use tauri_plugin_kiwix_mobile::MobileExt;

    /// Installed once during application setup; before that (and during
    /// shutdown) the hooks are no-ops rather than a panic.
    static APP: OnceLock<AppHandle> = OnceLock::new();

    pub(crate) fn install(app: AppHandle) {
        let _ = APP.set(app);
    }

    pub(super) fn begin(_reason: &'static str) { set(true); }
    pub(super) fn end() { set(false); }

    /// The Wi-Fi lock is an application resource, so it is set through the
    /// same trusted Rust-only plugin bridge as every other native operation.
    /// This is a synchronous JNI call on the transfer worker: the Java side
    /// sets the lock inline and never waits for the UI thread.
    fn set(held: bool) {
        let Some(app) = APP.get() else { return };
        if let Err(error) = app
            .kiwix_mobile()
            .call_sync("wifiPerformance", serde_json::json!({ "held": held }))
        {
            eprintln!("LocalSwarm Wi-Fi performance lock: {error}");
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "android")))]
mod platform {
    pub(super) fn begin(_reason: &'static str) {}
    pub(super) fn end() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static BEGINS: AtomicUsize = AtomicUsize::new(0);
    static ENDS: AtomicUsize = AtomicUsize::new(0);
    static TEST: Activity = Activity::new(test_begin, test_end);

    fn test_begin(_reason: &'static str) { BEGINS.fetch_add(1, Ordering::AcqRel); }
    fn test_end() { ENDS.fetch_add(1, Ordering::AcqRel); }

    #[test]
    fn nested_guards_hold_one_assertion() {
        let activity = &TEST;
        assert_eq!(activity.holders(), 0);

        let first = Guard::acquire_on(activity, "test transfer");
        assert_eq!(activity.holders(), 1);
        assert_eq!(BEGINS.load(Ordering::Acquire), 1, "first guard begins exactly one assertion");

        let second = Guard::acquire_on(activity, "test transfer");
        assert_eq!(activity.holders(), 2);
        assert_eq!(BEGINS.load(Ordering::Acquire), 1, "nested guard reuses the assertion");

        drop(first);
        assert_eq!(activity.holders(), 1);
        assert_eq!(ENDS.load(Ordering::Acquire), 0, "assertion survives while a guard remains");

        drop(second);
        assert_eq!(activity.holders(), 0);
        assert_eq!(ENDS.load(Ordering::Acquire), 1, "last guard ends the assertion once");
    }

    #[test]
    fn a_new_holder_cannot_overtake_the_previous_native_end() {
        use std::sync::{Arc, Barrier, LazyLock, mpsc};
        use std::thread;
        use std::time::Duration;

        struct Probe {
            ending: Barrier,
            finish_end: Barrier,
            next_token: AtomicUsize,
            active_token: AtomicUsize,
            ends: AtomicUsize,
            transitions: Mutex<Vec<(&'static str, usize)>>,
        }
        static PROBE: LazyLock<Probe> = LazyLock::new(|| Probe {
            ending: Barrier::new(2), finish_end: Barrier::new(2),
            next_token: AtomicUsize::new(0), active_token: AtomicUsize::new(0),
            ends: AtomicUsize::new(0), transitions: Mutex::new(Vec::new()),
        });
        static ACTIVITY: Activity = Activity::new(begin, end);

        fn begin(_reason: &'static str) {
            let probe = &*PROBE;
            let token = probe.next_token.fetch_add(1, Ordering::AcqRel) + 1;
            probe.active_token.store(token, Ordering::Release);
            probe.transitions.lock().unwrap().push(("begin", token));
        }
        fn end() {
            let probe = &*PROBE;
            if probe.ends.fetch_add(1, Ordering::AcqRel) == 0 {
                // Freeze the old last-release hook before it touches the native
                // resource, then attempt a replacement acquisition concurrently.
                probe.ending.wait();
                probe.finish_end.wait();
            }
            let token = probe.active_token.swap(0, Ordering::AcqRel);
            probe.transitions.lock().unwrap().push(("end", token));
        }

        let probe = &*PROBE;
        let old = Guard::acquire_on(&ACTIVITY, "old transfer");
        let releasing = thread::spawn(move || drop(old));
        probe.ending.wait();

        let attempting = Arc::new(Barrier::new(2));
        let (sender, receiver) = mpsc::channel();
        let acquiring = {
            let attempting = attempting.clone();
            thread::spawn(move || {
                attempting.wait();
                assert!(sender.send(Guard::acquire_on(&ACTIVITY, "new transfer")).is_ok());
            })
        };
        attempting.wait();
        let early = receiver.recv_timeout(Duration::from_millis(100)).ok();
        let overtook_end = early.is_some();
        // Always release the controlled hook before asserting, including on
        // failure, so a regression cannot strand either worker thread.
        probe.finish_end.wait();
        releasing.join().unwrap();
        acquiring.join().unwrap();
        let new = early.unwrap_or_else(|| receiver.recv_timeout(Duration::from_secs(5)).unwrap());

        assert!(!overtook_end, "A new guard returned while the old native end was still running");
        assert_eq!(ACTIVITY.holders(), 1);
        assert_eq!(probe.active_token.load(Ordering::Acquire), 2,
            "The old release must not retire the new holder's native token");
        assert_eq!(*probe.transitions.lock().unwrap(), [("begin", 1), ("end", 1), ("begin", 2)]);
        drop(new);
        assert_eq!(ACTIVITY.holders(), 0);
        assert_eq!(probe.active_token.load(Ordering::Acquire), 0);
        assert_eq!(*probe.transitions.lock().unwrap(),
            [("begin", 1), ("end", 1), ("begin", 2), ("end", 2)]);
    }
}
