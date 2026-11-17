// SPDX-License-Identifier: GPL-3.0-or-later
//! Projection of the existing acquisition snapshot onto an iOS Live Activity.
//!
//! The activity is a *view*: `acquisition::DownloadManager::list()` stays the
//! only source of truth, no transfer is started, paused or completed here, and
//! the projection keeps no durable state. Exactly one activity is shown at a
//! time and it follows the most recently updated transfer whose status is still
//! `queued`/`downloading`/`verifying`/`paused` — the same row the Downloads list
//! presents for concurrent work. A terminal transfer ends the activity (after a
//! short confirmation), and a row that leaves the list dismisses it.
//!
//! Private mode, read-only mode and unavailable native notifications suppress
//! the surface entirely: the first suppressed tick clears whatever the system
//! still shows and nothing new is requested until the policy allows it again.
//!
//! The decision logic is platform-neutral so it is unit-tested on the host; only
//! the Service glue and the native call are iOS-only.

use crate::acquisition::DownloadJob;
#[cfg(target_os = "ios")]
use crate::torrent::TorrentJob;
use crate::localization::Localizer;
use serde_json::{json, Value};
#[cfg(target_os = "ios")]
use std::collections::HashMap;
#[cfg(target_os = "ios")]
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Statuses that keep the activity on screen.
const ACTIVE: [&str; 10] = [
    "queued", "connecting", "resolving", "initializing", "metadata", "checking",
    "downloading", "verifying", "publishing", "paused",
];
/// Statuses that end it.
const TERMINAL: [&str; 6] = ["complete", "completed", "failed", "error", "cancelled", "canceled"];

/// Minimum spacing between progress refreshes for the same job and status.
const REFRESH: Duration = Duration::from_secs(2);

pub(crate) struct Projection {
    /// Job the native activity is showing, as last requested.
    tracked: Option<String>,
    status: Option<String>,
    downloaded: Option<u64>,
    sent: Option<Instant>,
    /// A clear has already been delivered; nothing is shown.
    clear: bool,
    /// Nothing was shown and no activity is expected to exist.
    idle: bool,
    /// Native notification availability is a process-lifetime property.
    #[cfg(target_os = "ios")]
    available: Option<bool>,
}

#[cfg(target_os = "ios")]
static PROJECTION: Mutex<Projection> = Mutex::new(Projection {
    tracked: None,
    status: None,
    downloaded: None,
    sent: None,
    clear: false,
    idle: false,
    #[cfg(target_os = "ios")]
    available: None,
});

/// Localizer for the current app setting and system locale. Building one
/// resolves the system locale, loads the CLDR directionality and parses the
/// localized catalogs, so the maintenance tick reuses it until either input
/// changes.
#[cfg(target_os = "ios")]
static APP_LOCALIZER: LazyLock<Mutex<Option<((String, String), Localizer)>>> =
    LazyLock::new(|| Mutex::new(None));

/// Run `f` with the cached localizer for `setting`. The cache lock is only held
/// for the projection decision, never across the native call.
#[cfg(target_os = "ios")]
fn with_app_localizer<T>(setting: &str, f: impl FnOnce(&Localizer) -> T) -> T {
    let system = sys_locale::get_locale().unwrap_or_else(|| "en".into());
    let mut cache = APP_LOCALIZER.lock().unwrap_or_else(|error| error.into_inner());
    let current = cache.as_ref().is_some_and(|(cached, _)| cached.0 == setting && cached.1 == system);
    if !current {
        let localizer = Localizer::new(setting, &system);
        *cache = Some(((setting.to_owned(), system), localizer));
    }
    f(&cache.as_ref().expect("localizer was just cached").1)
}

impl Projection {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self {
            tracked: None,
            status: None,
            downloaded: None,
            sent: None,
            clear: false,
            idle: false,
            #[cfg(target_os = "ios")]
            available: None,
        }
    }

    fn clear(&mut self) -> Value {
        self.tracked = None;
        self.status = None;
        self.downloaded = None;
        self.sent = None;
        self.clear = true;
        self.idle = true;
        json!({"state": "clear"})
    }

    /// Decide what, if anything, the native activity should be told now.
    pub(crate) fn decide(
        &mut self,
        suppressed: bool,
        jobs: &[DownloadJob],
        locale: &Localizer,
    ) -> Option<Value> {
        if suppressed {
            if self.clear {
                return None;
            }
            return Some(self.clear());
        }
        self.clear = false;

        // The shown transfer may have finished or lost its row entirely.
        if let Some(id) = self.tracked.clone() {
            match jobs.iter().find(|job| job.id == id) {
                None => {
                    self.tracked = None;
                    self.status = None;
                    self.downloaded = None;
                    self.sent = None;
                    self.idle = true;
                    return Some(json!({"state": "dismiss", "jobId": id}));
                }
                Some(job) if TERMINAL.contains(&job.status.as_str()) => {
                    let payload = snapshot(job, "terminal", locale);
                    self.tracked = None;
                    self.status = None;
                    self.downloaded = None;
                    self.sent = None;
                    self.idle = true;
                    return Some(payload);
                }
                Some(_) => {}
            }
        }

        let Some(job) = newest_active(jobs) else {
            // A previous process may have left an activity behind; retire it
            // once instead of trusting that nothing is on screen.
            if self.idle {
                return None;
            }
            self.idle = true;
            return Some(json!({"state": "dismiss"}));
        };
        self.idle = false;

        let changed = self.tracked.as_deref() != Some(job.id.as_str())
            || self.status.as_deref() != Some(job.status.as_str())
            || self.downloaded != Some(job.downloaded);
        let due = self.sent.is_none_or(|at| at.elapsed() >= REFRESH);
        if !changed && !due {
            return None;
        }
        self.tracked = Some(job.id.clone());
        self.status = Some(job.status.clone());
        self.downloaded = Some(job.downloaded);
        self.sent = Some(Instant::now());
        Some(snapshot(job, "active", locale))
    }

    /// A failed native call must not turn into a per-tick storm.
    #[cfg(target_os = "ios")]
    fn throttled(&mut self) {
        self.sent = Some(Instant::now());
    }
}

pub(crate) fn newest_active(jobs: &[DownloadJob]) -> Option<&DownloadJob> {
    jobs.iter()
        .filter(|job| ACTIVE.contains(&job.status.as_str()))
        .max_by(|a, b| {
            a.updated_at
                .cmp(&b.updated_at)
                .then_with(|| a.created_at.cmp(&b.created_at))
        })
}

fn label(status: &str) -> &'static str {
    match status {
        "queued" => "Queued",
        "connecting" => "Connecting",
        "resolving" => "Resolving",
        "initializing" => "Preparing",
        "metadata" => "Fetching metadata",
        "checking" => "Checking",
        "downloading" => "Downloading",
        "verifying" => "Verifying",
        "publishing" => "Preparing",
        "paused" => "Paused",
        "complete" | "completed" => "Completed",
        "failed" | "error" => "Failed",
        "cancelled" | "canceled" => "Cancelled",
        _ => "Downloading",
    }
}

#[cfg(target_os = "ios")]
static TORRENT_STAMPS: LazyLock<Mutex<HashMap<String, (String, u64, String)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(target_os = "ios")]
fn torrent_job(job: &TorrentJob) -> DownloadJob {
    let stamp = {
        let mut stamps = TORRENT_STAMPS.lock().unwrap_or_else(|error| error.into_inner());
        let current = (job.status.clone(), job.downloaded);
        match stamps.get(&job.id) {
            Some((status, downloaded, stamp))
                if *status == current.0 && *downloaded == current.1 => stamp.clone(),
            _ => {
                let stamp = crate::timestamps::now();
                stamps.insert(job.id.clone(), (current.0, current.1, stamp.clone()));
                stamp
            }
        }
    };
    DownloadJob {
        id: job.id.clone(),
        url: job.url.clone(),
        file_name: job.file_name.clone(),
        destination: job.destination.clone(),
        status: job.status.clone(),
        interrupted: false,
        downloaded: job.downloaded,
        total: job.total,
        error: job.error.clone(),
        history_warning: None,
        speed: job.speed.max(0.0) as u64,
        eta_seconds: job.eta_seconds,
        created_at: stamp.clone(),
        updated_at: stamp,
        sha256: None,
    }
}

#[cfg(target_os = "ios")]
fn prune_torrent_stamps(active: &[DownloadJob]) {
    let ids: std::collections::HashSet<&str> = active.iter().map(|job| job.id.as_str()).collect();
    if let Ok(mut stamps) = TORRENT_STAMPS.lock() {
        stamps.retain(|id, _| ids.contains(id.as_str()));
    }
}

fn snapshot(job: &DownloadJob, state: &str, locale: &Localizer) -> Value {
    json!({
        "state": state,
        "jobId": job.id,
        "fileName": job.file_name,
        "status": job.status,
        "statusLabel": locale.text(label(&job.status), &[]),
        "downloaded": job.downloaded,
        "total": job.total,
    })
}

#[cfg(target_os = "ios")]
fn notifications_available(projection: &mut Projection) -> bool {
    if let Some(available) = projection.available {
        return available;
    }
    let available = crate::notifications::capability()
        .map(|capability| capability.available)
        .unwrap_or(false);
    projection.available = Some(available);
    available
}

#[cfg(target_os = "ios")]
impl crate::service::Service {
    /// Refresh the download Live Activity from the current acquisition
    /// snapshot. Called by the mobile maintenance tick while the app is in the
    /// foreground or an activity is already shown, so it must stay cheap when
    /// idle.
    pub(crate) async fn mobile_live_activity_update(&self) -> Result<(), String> {
        let settings = self.store.settings()?;
        let suppressed = settings.private_mode || settings.read_only;
        let mut jobs = if suppressed {
            Vec::new()
        } else {
            self.download_manager().await?.list()?
        };
        if !suppressed {
            if let Ok(manager) = self.torrent_manager() {
                let torrents: Vec<_> = manager.list().await.iter().map(torrent_job).collect();
                prune_torrent_stamps(&torrents);
                jobs.extend(torrents);
            }
        }
        let payload = {
            let mut projection = PROJECTION
                .lock()
                .map_err(|_| "Live Activity projection is unavailable")?;
            let suppressed = suppressed || !notifications_available(&mut projection);
            with_app_localizer(&settings.locale, |locale| projection.decide(suppressed, &jobs, locale))
        };
        let Some(payload) = payload else { return Ok(()); };
        if let Err(error) = self.mobile_call("liveActivity", payload).await {
            if let Ok(mut projection) = PROJECTION.lock() {
                projection.throttled();
            }
            return Err(error);
        }
        Ok(())
    }

    /// True when no activity is shown and none needs clearing, so a background
    /// maintenance tick has nothing to project. A new transfer cannot start in
    /// the background; the foregrounded tick always refreshes.
    pub(crate) fn mobile_live_activity_idle(&self) -> bool {
        PROJECTION.lock().is_ok_and(|projection| projection.idle && projection.tracked.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, status: &str, downloaded: u64, updated_at: &str) -> DownloadJob {
        DownloadJob {
            id: id.into(),
            url: format!("https://example.org/{id}.zim"),
            file_name: format!("{id}.zim"),
            destination: format!("/tmp/{id}.zim"),
            status: status.into(),
            interrupted: false,
            downloaded,
            total: Some(1000),
            error: None,
            history_warning: None,
            speed: 0,
            eta_seconds: None,
            created_at: "2026-10-31T10:00:00+00:00".into(),
            updated_at: updated_at.into(),
            sha256: None,
        }
    }

    fn locale() -> Localizer {
        Localizer::for_app("en")
    }

    #[test]
    fn picks_the_most_recently_updated_active_transfer() {
        let jobs = vec![
            job("old", "downloading", 10, "2026-10-31T10:00:01+00:00"),
            job("new", "verifying", 900, "2026-10-31T10:05:00+00:00"),
            job("done", "complete", 1000, "2026-10-31T10:09:00+00:00"),
        ];
        assert_eq!(newest_active(&jobs).map(|job| job.id.as_str()), Some("new"));
    }

    #[test]
    fn paused_transfers_keep_the_activity_and_terminal_ones_end_it() {
        let jobs = vec![job("paused", "paused", 400, "2026-10-31T10:01:00+00:00")];
        assert_eq!(newest_active(&jobs).map(|job| job.id.as_str()), Some("paused"));
        let jobs = vec![job("failed", "failed", 400, "2026-10-31T10:01:00+00:00")];
        assert!(newest_active(&jobs).is_none());
    }

    #[test]
    fn status_labels_are_the_translated_download_vocabulary() {
        assert_eq!(label("queued"), "Queued");
        assert_eq!(label("downloading"), "Downloading");
        assert_eq!(label("verifying"), "Verifying");
        assert_eq!(label("paused"), "Paused");
        assert_eq!(label("complete"), "Completed");
        assert_eq!(label("failed"), "Failed");
        assert_eq!(label("cancelled"), "Cancelled");
        assert_eq!(label("resolving"), "Resolving");
        assert_eq!(label("metadata"), "Fetching metadata");
        assert_eq!(label("completed"), "Completed");
    }

    #[test]
    fn torrent_statuses_keep_the_activity_and_terminals_end_it() {
        let mut active = job("torrent", "metadata", 20, "");
        assert_eq!(newest_active(&[active.clone()]).map(|job| job.id.as_str()), Some("torrent"));
        active.status = "completed".into();
        assert!(newest_active(&[active]).is_none());
    }

    #[test]
    fn starts_updates_throttles_and_ends_one_activity() {
        let mut projection = Projection::new();
        let locale = locale();

        // Nothing has been shown yet: retire anything a previous process left.
        assert_eq!(
            projection.decide(false, &[], &locale),
            Some(json!({"state": "dismiss"}))
        );
        assert_eq!(projection.decide(false, &[], &locale), None);

        let mut jobs = vec![job("a", "downloading", 100, "2026-10-31T10:00:01+00:00")];
        let first = projection.decide(false, &jobs, &locale).expect("start");
        assert_eq!(first["state"], json!("active"));
        assert_eq!(first["jobId"], json!("a"));
        assert_eq!(first["statusLabel"], json!("Downloading"));
        assert_eq!(first["downloaded"], json!(100));
        assert_eq!(first["total"], json!(1000));

        // Same bytes and status inside the refresh window: no native traffic.
        assert_eq!(projection.decide(false, &jobs, &locale), None);

        jobs[0].downloaded = 250;
        let update = projection.decide(false, &jobs, &locale).expect("progress");
        assert_eq!(update["downloaded"], json!(250));

        jobs[0].status = "paused".into();
        let paused = projection.decide(false, &jobs, &locale).expect("pause");
        assert_eq!(paused["status"], json!("paused"));
        assert_eq!(paused["statusLabel"], json!("Paused"));

        jobs[0].status = "verifying".into();
        let verifying = projection.decide(false, &jobs, &locale).expect("verify");
        assert_eq!(verifying["statusLabel"], json!("Verifying"));

        jobs[0].status = "complete".into();
        let terminal = projection.decide(false, &jobs, &locale).expect("end");
        assert_eq!(terminal["state"], json!("terminal"));
        assert_eq!(terminal["status"], json!("complete"));
        assert_eq!(terminal["statusLabel"], json!("Completed"));
        // The completed row stays in the list; it must not restart anything.
        assert_eq!(projection.decide(false, &jobs, &locale), None);
    }

    #[test]
    fn a_removed_row_dismisses_the_activity() {
        let mut projection = Projection::new();
        let locale = locale();
        let jobs = vec![job("a", "downloading", 10, "2026-10-31T10:00:01+00:00")];
        assert!(projection.decide(false, &jobs, &locale).is_some());
        let dismiss = projection.decide(false, &[], &locale).expect("dismiss");
        assert_eq!(dismiss["state"], json!("dismiss"));
        assert_eq!(dismiss["jobId"], json!("a"));
        assert_eq!(projection.decide(false, &[], &locale), None);
    }

    #[test]
    fn suppression_clears_once_and_resumes_after_policy_reopens() {
        let mut projection = Projection::new();
        let locale = locale();
        let jobs = vec![job("a", "downloading", 10, "2026-10-31T10:00:01+00:00")];
        assert!(projection.decide(false, &jobs, &locale).is_some());

        // Private/read-only/unavailable notifications: no content anywhere.
        let clear = projection.decide(true, &jobs, &locale).expect("clear");
        assert_eq!(clear, json!({"state": "clear"}));
        assert_eq!(projection.decide(true, &jobs, &locale), None);
        assert_eq!(projection.decide(true, &[], &locale), None);

        // Policy reopens: the same transfer is published again from scratch.
        let restart = projection.decide(false, &jobs, &locale).expect("restart");
        assert_eq!(restart["state"], json!("active"));
        assert_eq!(restart["jobId"], json!("a"));
    }

    #[test]
    fn a_second_transfer_takes_over_the_single_activity() {
        let mut projection = Projection::new();
        let locale = locale();
        let first = vec![job("a", "downloading", 10, "2026-10-31T10:00:01+00:00")];
        assert!(projection.decide(false, &first, &locale).is_some());
        let both = vec![
            job("a", "downloading", 10, "2026-10-31T10:00:01+00:00"),
            job("b", "downloading", 5, "2026-10-31T10:00:02+00:00"),
        ];
        let switched = projection.decide(false, &both, &locale).expect("switch");
        assert_eq!(switched["jobId"], json!("b"));
    }
}
