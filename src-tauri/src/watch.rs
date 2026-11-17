// SPDX-License-Identifier: GPL-3.0-or-later
//! Live monitoring of the user's monitored folders.
//!
//! This mirrors `kiwix-desktop`'s `ContentManager` watcher: the OS reports
//! directory activity, a ~1 s stability window absorbs copies and event storms,
//! and the settled observation is handed to the ordinary monitored-folder scan
//! so identity, ownership, claim exclusion and integrity rules stay identical.
//! Nothing is imported from this module directly.
//!
//! The periodic `library_maintenance` scan remains the fallback: it keeps
//! running when no OS watcher can be installed, when a backend silently misses
//! an event, and when the watcher thread has already stopped.
use crate::store::Store;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TryRecvError, TrySendError},
        Arc, Mutex, Weak,
    },
    time::{Duration, Instant, SystemTime},
};

/// A file is considered stable once it has been quiet for this long.
const SETTLE: Duration = Duration::from_secs(1);
/// Worker wake-up interval; bounds event latency and shutdown latency.
const TICK: Duration = Duration::from_millis(200);
/// How often the worker re-reads the monitored-directory configuration.
const CONFIG_POLL: Duration = Duration::from_secs(1);
/// How often unchanged directory paths are re-canonicalized. The settings
/// poll stays at one second; this bounds filesystem resolution of paths that
/// may be reached through a symlink while avoiding a canonicalize per poll.
const DIRECTORY_REFRESH: Duration = Duration::from_secs(30);
/// Full directory re-check cadence, for coalesced or dropped OS events.
const RESCAN: Duration = Duration::from_secs(30);
/// Bound on configured monitored directories we are willing to watch.
const MAX_DIRECTORIES: usize = 64;
/// Bound on per-file settle entries retained between ticks.
const MAX_PENDING: usize = 2048;
/// Bound on raw OS events buffered between worker wake-ups.
const MAX_EVENTS: usize = 4096;
/// Bound on remembered failed imports (unchanged files are not retried).
const MAX_KNOWN_BAD: usize = 1024;
/// Bound on settled observations waiting for the async consumer.
const MESSAGE_QUEUE: usize = 32;

/// A settled observation, produced only after the stability window elapsed.
pub(crate) enum WatchMessage {
    /// A candidate archive path (absolute) has been quiet for `SETTLE`.
    SettledFile(PathBuf),
    /// A monitored directory (absolute) has been quiet for `SETTLE`.
    SettledDirectory(PathBuf),
}

/// Size and modification time identify a file version for the stability check
/// and for the unchanged-known-bad cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

impl Fingerprint {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

/// Stat without following symlinks: the manual scan never imports a symlink,
/// and a symlink replacing an archive must not look like an update.
fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    metadata.is_file().then(|| Fingerprint::of(&metadata))
}

/// True when the file has not been written inside the stability window. The
/// directory diff uses this as its last line of defence: an event-driven
/// per-file settle may be skipped (startup, coalescing), but a file that was
/// written a moment ago is never handed to the importer.
pub(crate) fn is_settled(path: &Path) -> bool {
    match fingerprint(path) {
        Some(current) => current.modified.is_none_or(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .is_ok_and(|age| age >= SETTLE)
        }),
        None => false,
    }
}

/// Monitored-folder scan policy, shared with the periodic scan: direct
/// children only, case-insensitive `.zim`, and `.zimaa` except in portable
/// profiles (which must keep the whole archive inside the portable root).
pub(crate) fn is_candidate(path: &Path, portable: bool) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    extension.eq_ignore_ascii_case("zim")
        || (!portable && extension.eq_ignore_ascii_case("zimaa"))
}

/// Compare paths with the same alias/case folding used for acquisition claims.
pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || crate::store::acquisition_paths_equal(
            &left.to_string_lossy(),
            &right.to_string_lossy(),
            "",
        )
}

pub(crate) fn is_direct_child(path: &Path, directory: &Path) -> bool {
    path.parent().is_some_and(|parent| same_path(parent, directory))
}

/// The canonical monitored directories for the current settings. Canonical
/// paths are what the OS reports, and they keep comparisons consistent when a
/// configured directory is reached through a symlink (for example `/tmp`).
pub(crate) fn monitored_directories(settings: &crate::store::Settings) -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = settings
        .monitor_directories
        .iter()
        .filter(|directory| !directory.is_empty())
        .map(PathBuf::from)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.canonicalize().unwrap_or(directory))
        .collect();
    directories.sort();
    directories.dedup();
    directories.truncate(MAX_DIRECTORIES);
    directories
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Imports that already failed for an unchanged file version. Re-opening a
/// corrupt archive on every directory re-check is pointless and expensive;
/// a changed size or modification time retries, matching `kiwix-desktop`'s
/// unchanged-known-bad bookkeeping.
#[derive(Default)]
pub(crate) struct FailureLog(HashMap<String, Fingerprint>);

impl FailureLog {
    pub(crate) fn should_skip(&self, path: &Path) -> bool {
        match (self.0.get(&path_key(path)), fingerprint(path)) {
            (Some(recorded), Some(current)) => *recorded == current,
            _ => false,
        }
    }

    pub(crate) fn record(&mut self, path: &Path) {
        // A split archive's first part fails until its whole cohort is visible;
        // its own fingerprint cannot change when a later part arrives, so
        // caching that failure would suppress the retry. The periodic scan
        // retries anyway, but live watching should not lag behind it.
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zimaa"))
        {
            self.0.remove(&path_key(path));
            return;
        }
        let Some(current) = fingerprint(path) else {
            self.0.remove(&path_key(path));
            return;
        };
        let key = path_key(path);
        if self.0.len() >= MAX_KNOWN_BAD && !self.0.contains_key(&key) {
            self.0.clear();
        }
        self.0.insert(key, current);
    }

    pub(crate) fn clear(&mut self, path: &Path) {
        self.0.remove(&path_key(path));
    }
}

#[derive(Clone, Copy)]
struct PendingFile {
    last_change: Instant,
    fingerprint: Fingerprint,
}

/// Debounce state: one deadline per touched directory and one per candidate
/// file. Directories are re-listed on settle, so removals and renames need no
/// per-event bookkeeping; files additionally need two matching observations
/// separated by the stability window, because OS event delivery coalesces.
#[derive(Default)]
struct SettleState {
    files: HashMap<PathBuf, PendingFile>,
    directories: HashMap<PathBuf, Instant>,
}

impl SettleState {
    fn observe_file(&mut self, path: PathBuf, now: Instant, fingerprint: Fingerprint) {
        self.files.insert(
            path,
            PendingFile {
                last_change: now,
                fingerprint,
            },
        );
    }

    fn observe_directory(&mut self, path: PathBuf, now: Instant) {
        self.directories.insert(path, now);
    }

    fn is_full(&self) -> bool {
        self.files.len() >= MAX_PENDING
    }

    fn retain_directories(&mut self, directories: &[PathBuf]) {
        self.directories
            .retain(|directory, _| directories.contains(directory));
        self.files.retain(|path, _| {
            path.parent()
                .is_some_and(|parent| directories.iter().any(|directory| same_path(parent, directory)))
        });
    }

    /// Move every observation that has been quiet for `settle` into messages.
    /// A file whose size or modification time moved without a delivered event
    /// is re-armed; a vanished file asks for a directory reconciliation, which
    /// is where the library entry is retired.
    fn take_settled(
        &mut self,
        now: Instant,
        wall: SystemTime,
        settle: Duration,
        stat: impl Fn(&Path) -> Option<Fingerprint>,
    ) -> Vec<WatchMessage> {
        let mut messages = Vec::new();
        self.directories.retain(|directory, observed| {
            if now.duration_since(*observed) >= settle {
                messages.push(WatchMessage::SettledDirectory(directory.clone()));
                false
            } else {
                true
            }
        });
        let files = std::mem::take(&mut self.files);
        for (path, pending) in files {
            if now.duration_since(pending.last_change) < settle {
                self.files.insert(path, pending);
                continue;
            }
            match stat(&path) {
                None => {
                    // The observation outlived its file; reconcile the
                    // directory so the library entry can be retired.
                    if let Some(parent) = path.parent() {
                        messages.push(WatchMessage::SettledDirectory(parent.to_path_buf()));
                    }
                }
                Some(current) if current != pending.fingerprint => {
                    self.files.insert(
                        path,
                        PendingFile {
                            last_change: now,
                            fingerprint: current,
                        },
                    );
                }
                Some(current) => {
                    // A writer can touch a file without an event reaching us
                    // (FSEvents coalescing); the modification time is the last
                    // word on whether the stability window has passed.
                    let fresh = current.modified.is_some_and(|modified| {
                        wall.duration_since(modified)
                            .map_or(true, |age| age < settle)
                    });
                    if fresh {
                        self.files.insert(
                            path,
                            PendingFile {
                                last_change: now,
                                fingerprint: current,
                            },
                        );
                    } else {
                        messages.push(WatchMessage::SettledFile(path));
                    }
                }
            }
        }
        messages
    }
}

struct Worker {
    store: Arc<Store>,
    stop: Arc<AtomicBool>,
    tx: tokio::sync::mpsc::Sender<WatchMessage>,
    failures: Arc<Mutex<FailureLog>>,
    events: mpsc::Receiver<notify::Result<Event>>,
    overflow: Arc<AtomicBool>,
    watcher: Option<RecommendedWatcher>,
    watched: Vec<PathBuf>,
    state: SettleState,
    logged_failures: HashSet<PathBuf>,
    portable: bool,
    last_config: Option<Instant>,
    last_directory_refresh: Option<Instant>,
    last_monitor_settings: Vec<String>,
    last_blocked: bool,
    last_rescan: Instant,
}

impl Worker {
    fn new(
        store: Arc<Store>,
        tx: tokio::sync::mpsc::Sender<WatchMessage>,
        stop: Arc<AtomicBool>,
        failures: Arc<Mutex<FailureLog>>,
    ) -> Self {
        let (event_tx, events) = mpsc::sync_channel::<notify::Result<Event>>(MAX_EVENTS);
        let overflow = Arc::new(AtomicBool::new(false));
        let watcher = {
            let overflow = overflow.clone();
            let handler = move |result: notify::Result<Event>| {
                match event_tx.try_send(result) {
                    Ok(()) => {}
                    // A storm must not grow memory without bound. Coalescing is
                    // safe because settling re-lists the whole directory.
                    Err(TrySendError::Full(_)) => overflow.store(true, Ordering::SeqCst),
                    Err(TrySendError::Disconnected(_)) => {}
                }
            };
            match notify::recommended_watcher(handler) {
                Ok(watcher) => Some(watcher),
                Err(error) => {
                    eprintln!(
                        "Monitored folders: native watching is unavailable, falling back to periodic scans: {error}"
                    );
                    None
                }
            }
        };
        Self {
            store,
            stop,
            tx,
            failures,
            events,
            overflow,
            watcher,
            watched: Vec::new(),
            state: SettleState::default(),
            logged_failures: HashSet::new(),
            portable: false,
            last_config: None,
            last_directory_refresh: None,
            last_monitor_settings: Vec::new(),
            last_blocked: false,
            last_rescan: Instant::now(),
        }
    }

    fn run(mut self) {
        while !self.stop.load(Ordering::SeqCst) {
            if self
                .last_config
                .is_none_or(|polled| polled.elapsed() >= CONFIG_POLL)
            {
                self.configure();
                self.last_config = Some(Instant::now());
            }
            self.drain_events();
            if self.last_rescan.elapsed() >= RESCAN {
                self.last_rescan = Instant::now();
                let now = Instant::now();
                for directory in self.watched.clone() {
                    self.state.observe_directory(directory, now);
                }
            }
            let settled = self.state.take_settled(
                Instant::now(),
                SystemTime::now(),
                SETTLE,
                fingerprint,
            );
            for message in settled {
                if self.tx.blocking_send(message).is_err() {
                    // The consumer is gone; shutdown is in progress.
                    return;
                }
            }
            std::thread::sleep(TICK);
        }
    }

    /// Apply the current settings: watch every configured directory, stop
    /// watching the rest, and drop debounce state for directories we no longer
    /// own. Private and read-only modes watch nothing at all, so no import can
    /// even be attempted while the policy forbids it.
    fn configure(&mut self) {
        let Ok(settings) = self.store.settings() else {
            return;
        };
        self.portable = settings.portable_mode;
        let blocked = settings.read_only || settings.private_mode;
        let refresh = self
            .last_directory_refresh
            .is_none_or(|refreshed| refreshed.elapsed() >= DIRECTORY_REFRESH);
        if !refresh
            && blocked == self.last_blocked
            && settings.monitor_directories == self.last_monitor_settings
        {
            return;
        }
        self.last_directory_refresh = Some(Instant::now());
        self.last_blocked = blocked;
        self.last_monitor_settings = settings.monitor_directories.clone();
        let desired = if blocked {
            Vec::new()
        } else {
            monitored_directories(&settings)
        };
        if desired == self.watched {
            return;
        }
        let mut next = Vec::with_capacity(desired.len());
        for directory in &desired {
            if self.watched.contains(directory) {
                next.push(directory.clone());
                continue;
            }
            let Some(watcher) = self.watcher.as_mut() else {
                break;
            };
            match watcher.watch(directory, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.logged_failures.remove(directory);
                    // A newly monitored folder may already hold archives.
                    self.state.observe_directory(directory.clone(), Instant::now());
                    next.push(directory.clone());
                }
                Err(error) => {
                    if self.logged_failures.insert(directory.clone()) {
                        eprintln!(
                            "Monitored folders: cannot watch {}: {error}",
                            directory.display()
                        );
                    }
                }
            }
        }
        if let Some(watcher) = self.watcher.as_mut() {
            for directory in &self.watched {
                if !next.contains(directory) {
                    let _ = watcher.unwatch(directory);
                }
            }
        }
        self.state.retain_directories(&next);
        self.watched = next;
    }

    fn drain_events(&mut self) {
        let now = Instant::now();
        let mut drained = 0usize;
        while drained < MAX_EVENTS {
            match self.events.try_recv() {
                Ok(Ok(event)) => {
                    drained += 1;
                    self.observe_event(&event, now);
                }
                Ok(Err(_)) => drained += 1,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if self.overflow.swap(false, Ordering::SeqCst) {
            for directory in self.watched.clone() {
                self.state.observe_directory(directory, now);
            }
        }
    }

    fn observe_event(&mut self, event: &Event, now: Instant) {
        for path in &event.paths {
            let Some(directory) = self
                .watched
                .iter()
                .find(|directory| *path == **directory || is_direct_child(path, directory))
                .cloned()
            else {
                continue;
            };
            // Any activity settles the directory, including creates, renames
            // and removals that the directory diff resolves.
            self.state.observe_directory(directory, now);
            if !is_candidate(path, self.portable) {
                continue;
            }
            let Some(current) = fingerprint(path) else {
                continue;
            };
            if self
                .failures
                .lock()
                .map(|failures| failures.should_skip(path))
                .unwrap_or(false)
            {
                continue;
            }
            self.state.observe_file(path.clone(), now, current);
            if self.state.is_full() {
                // Collapse the storm into directory re-checks; bounded memory
                // matters more than per-file precision under a flood.
                self.state = SettleState::default();
                for directory in self.watched.clone() {
                    self.state.observe_directory(directory, now);
                }
                return;
            }
        }
    }
}

/// Handle for the watcher thread and its consumer task. Dropping it stops the
/// thread; `shutdown` additionally joins both.
pub(crate) struct FolderWatch {
    stop: Arc<AtomicBool>,
    quit: tokio_util::sync::CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

/// Longest Quit waits for an observation that is being applied.
const DRAIN_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Deliver settled observations one at a time until `quit`. Cancellation only
/// interrupts the wait for the next message, never an observation being
/// applied: that work holds the privacy read lease while it runs blocking
/// archive verification and library registration, and aborting the future
/// would drop the lease while the blocking import kept running underneath
/// service shutdown.
async fn consume<M, F, Fut>(
    mut messages: tokio::sync::mpsc::Receiver<M>,
    quit: tokio_util::sync::CancellationToken,
    mut apply: F,
) where
    F: FnMut(M) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    loop {
        let message = tokio::select! {
            biased;
            _ = quit.cancelled() => break,
            message = messages.recv() => match message {
                Some(message) => message,
                None => break,
            },
        };
        if !apply(message).await {
            break;
        }
    }
}

impl FolderWatch {
    pub(crate) fn start(
        store: Arc<Store>,
        service: Weak<crate::service::Service>,
        failures: Arc<Mutex<FailureLog>>,
    ) -> Result<Self, String> {
        let (tx, rx) = tokio::sync::mpsc::channel::<WatchMessage>(MESSAGE_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("kiwix-folder-watch".into())
            .spawn(move || Worker::new(store, tx, worker_stop, failures).run())
            .map_err(|error| format!("Starting the monitored folder watcher: {error}"))?;
        let quit = tokio_util::sync::CancellationToken::new();
        let task = tokio::spawn(consume(rx, quit.clone(), move |message| {
            let service = service.clone();
            async move {
                let Some(service) = service.upgrade() else {
                    return false;
                };
                if service.ensure_running().is_err() {
                    return false;
                }
                if let Err(error) = service.apply_watch(message).await {
                    eprintln!("Monitored folders: {error}");
                }
                true
            }
        }));
        Ok(Self {
            stop,
            quit,
            thread: Some(thread),
            task: Some(task),
        })
    }

    pub(crate) async fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(mut task) = self.task.take() {
            // Drain, do not abort: see `consume`. Verifying a very large
            // archive can take a while, so the drain is bounded; after that
            // the consumer is aborted and the still-running blocking import
            // refuses to register because the service is already stopping
            // (`import_monitored_paths`).
            self.quit.cancel();
            if tokio::time::timeout(DRAIN_LIMIT, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
    }
}

impl Drop for FolderWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            task.abort();
        }
        // Detach rather than join: Drop may run on a runtime thread. The worker
        // observes the stop flag on its next tick and the aborted consumer has
        // already dropped the receiving end.
        self.thread.take();
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn shutdown_drains_the_observation_being_applied_instead_of_aborting_it() {
        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(4);
        let quit = tokio_util::sync::CancellationToken::new();
        let (entered_tx, entered) = tokio::sync::oneshot::channel::<()>();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let entered_tx = std::sync::Mutex::new(Some(entered_tx));
        let released = std::sync::Mutex::new(Some(released));
        let applied = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = applied.clone();
        let task = tokio::spawn(consume(rx, quit.clone(), move |message| {
            let entered = entered_tx.lock().unwrap().take();
            let released = released.lock().unwrap().take();
            let counter = counter.clone();
            async move {
                if let Some(entered) = entered { let _ = entered.send(()); }
                if let Some(released) = released { let _ = released.await; }
                counter.fetch_add(message, Ordering::SeqCst);
                true
            }
        }));
        tx.send(1).await.unwrap();
        entered.await.unwrap();
        // Shutdown starts while the observation (lease + blocking import in
        // production) is in flight; a queued observation must not start.
        tx.send(100).await.unwrap();
        quit.cancel();
        for _ in 0..10 { tokio::task::yield_now().await; }
        assert!(!task.is_finished(), "shutdown abandoned in-flight work");
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), task).await.unwrap().unwrap();
        assert_eq!(applied.load(Ordering::SeqCst), 1);
    }

    use super::*;
    use std::time::{Duration, SystemTime};

    fn at(seconds: u64) -> Instant {
        // A monotonic base far from process start; only differences matter.
        Instant::now() + Duration::from_secs(seconds)
    }

    fn old(seconds: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(seconds)
    }

    #[test]
    fn candidate_extensions_match_the_periodic_scan() {
        assert!(is_candidate(Path::new("/x/a.zim"), false));
        assert!(is_candidate(Path::new("/x/a.ZIM"), false));
        assert!(is_candidate(Path::new("/x/a.zimaa"), false));
        assert!(!is_candidate(Path::new("/x/a.zimaa"), true));
        assert!(!is_candidate(Path::new("/x/.kiwix-1.part"), false));
        assert!(!is_candidate(Path::new("/x/a.zim.part"), false));
        assert!(!is_candidate(Path::new("/x/a"), false));
    }

    #[test]
    fn files_settle_only_after_a_quiet_stability_window() {
        let mut state = SettleState::default();
        let start = at(0);
        let path = PathBuf::from("/monitored/a.zim");
        let version = Fingerprint { len: 10, modified: Some(old(5)) };
        state.observe_file(path.clone(), start, version);

        // Still inside the window.
        let settled = state.take_settled(start + Duration::from_millis(900), SystemTime::now(),
            SETTLE, |_| Some(version));
        assert!(settled.is_empty());

        let settled = state.take_settled(start + Duration::from_millis(1100), SystemTime::now(),
            SETTLE, |_| Some(version));
        assert!(matches!(settled.as_slice(), [WatchMessage::SettledFile(settled)] if settled == &path));
    }

    #[test]
    fn growth_restarts_the_stability_window_even_without_new_events() {
        let mut state = SettleState::default();
        let start = at(0);
        let path = PathBuf::from("/monitored/slow.zim");
        state.observe_file(path.clone(), start, Fingerprint { len: 10, modified: Some(old(5)) });

        // The file grew without a delivered event: re-arm, do not import.
        let grown = Fingerprint { len: 20, modified: Some(old(5)) };
        let settled = state.take_settled(start + Duration::from_secs(2), SystemTime::now(),
            SETTLE, |_| Some(grown));
        assert!(settled.is_empty());
        assert!(state.files.contains_key(&path));

        let settled = state.take_settled(start + Duration::from_secs(4), SystemTime::now(),
            SETTLE, |_| Some(grown));
        assert!(matches!(settled.as_slice(), [WatchMessage::SettledFile(_)]));
    }

    #[test]
    fn fresh_modification_time_defers_a_settled_file() {
        let mut state = SettleState::default();
        let start = at(0);
        let path = PathBuf::from("/monitored/touched.zim");
        // Modification time in the future (clock skew) must be treated as fresh.
        let version = Fingerprint { len: 10, modified: Some(SystemTime::now() + Duration::from_secs(5)) };
        state.observe_file(path.clone(), start, version);
        let settled = state.take_settled(start + Duration::from_secs(2), SystemTime::now(),
            SETTLE, |_| Some(version));
        assert!(settled.is_empty());
    }

    #[test]
    fn vanished_files_ask_for_a_directory_reconciliation() {
        let mut state = SettleState::default();
        let start = at(0);
        let path = PathBuf::from("/monitored/gone.zim");
        state.observe_file(path, start, Fingerprint { len: 10, modified: Some(old(5)) });
        let settled = state.take_settled(start + Duration::from_secs(2), SystemTime::now(),
            SETTLE, |_| None);
        assert!(matches!(settled.as_slice(),
            [WatchMessage::SettledDirectory(directory)] if directory == Path::new("/monitored")));
    }

    #[test]
    fn directory_settle_is_independent_of_file_settle() {
        let mut state = SettleState::default();
        let start = at(0);
        let version = Fingerprint { len: 10, modified: Some(old(5)) };
        state.observe_directory(PathBuf::from("/monitored"), start);
        state.observe_file(PathBuf::from("/monitored/a.zim"), start, version);
        let settled = state.take_settled(start + Duration::from_secs(2), SystemTime::now(),
            SETTLE, |_| Some(version));
        assert_eq!(settled.len(), 2);
    }

    #[test]
    fn unchanged_failures_are_not_retried_but_updates_are() {
        let directory = std::env::temp_dir().join(format!("kiwix-watch-failure-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("bad.zim");
        std::fs::write(&path, b"bad").unwrap();
        let mut failures = FailureLog::default();
        failures.record(&path);
        assert!(failures.should_skip(&path));
        std::fs::write(&path, b"a longer bad version").unwrap();
        assert!(!failures.should_skip(&path));
        failures.clear(&path);
        assert!(!failures.should_skip(&path));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn split_first_parts_are_retried_when_a_later_part_arrives() {
        let directory = std::env::temp_dir().join(format!("kiwix-watch-split-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let first = directory.join("book.zimaa");
        std::fs::write(&first, b"incomplete cohort").unwrap();
        let mut failures = FailureLog::default();
        failures.record(&first);
        assert!(!failures.should_skip(&first));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn paths_compare_with_the_same_folding_as_acquisition_claims() {
        assert!(same_path(Path::new("/a/b.zim"), Path::new("/a/b.zim")));
        assert!(is_direct_child(Path::new("/a/b.zim"), Path::new("/a")));
        assert!(!is_direct_child(Path::new("/a/b/c.zim"), Path::new("/a")));
    }
}
