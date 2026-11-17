// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::notifications::{NotificationTap, NotificationCapability};

/// Absolute bound on waiting for the root to regain foreground authority after
/// UIKit's authorization sheet. A request that never regains it must fail
/// instead of sleeping forever.
#[cfg(mobile)]
const FOREGROUND_REACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

impl Service {
    pub(crate) fn initialize_notifications(self: &Arc<Self>) {
        if self.notification_gui.swap(true, Ordering::SeqCst) { return; }
        let result = (|| {
            let capability = crate::notifications::capability()?;
            if !capability.available { return Ok(()); }
            crate::notifications::install_waker(self)?;
            let settings = self.store.settings()?;
            self.notifications.configure(settings.completion_notifications, settings.private_mode || settings.read_only)
        })();
        if let Err(error) = result { self.notification_warning(error); }
    }

    pub(super) fn notification_warning(&self, error: String) {
        if let Ok(mut warnings) = self.startup_warnings.lock() {
            let language = self.store.locale().unwrap_or_default();
            let locale = crate::localization::Localizer::for_app(&language);
            let prefix = locale.text("Completion notifications are unavailable", &[]);
            let message = format!("{prefix}: {}", error.chars().take(500).collect::<String>());
            if !warnings.contains(&message) {
                if warnings.len() >= 32 { warnings.remove(0); }
                warnings.push(message);
            }
        }
    }

    pub(super) fn notification_capability(&self) -> NotificationCapability {
        if !self.notification_gui.load(Ordering::SeqCst) {
            return NotificationCapability { available: false, reason: Some("Native completion notifications are unavailable in headless operation".into()) };
        }
        crate::notifications::capability().unwrap_or_else(|error| NotificationCapability { available: false, reason: Some(error) })
    }

    pub(super) async fn request_completion_notifications(self: &Arc<Self>, args: Value) -> Result<Value, String> {
        let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main");
        if label != "main" { return Err("Completion notification permission requires the trusted root window".into()); }
        #[cfg(mobile)]
        let root_generation;
        let (generation, policy_generation) = {
            let _mode = self.privacy_gate.read().await;
            self.ensure_running()?;
            #[cfg(mobile)]
            self.mobile_admit("request_completion_notifications", &args)?;
            self.ensure_window_active(label)?;
            self.normal_writable()?;
            let generation = self.ensure_generation(&args)?;
            let capability = self.notification_capability();
            if !capability.available { return Err(capability.reason.unwrap_or_else(|| "Native completion notifications are unavailable".into())); }
            #[cfg(mobile)]
            { root_generation = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?.generation; }
            (generation, self.privacy_policy_generation.load(Ordering::SeqCst))
        };
        // UIKit's authorization sheet deactivates the root. Neither this wait nor
        // its native callback owns the privacy lease or proves resumed authority.
        let (granted, revision) = self.notifications.request_permission().await?;
        #[cfg(mobile)]
        let (_mode, foreground) = {
            // UIKit's sheet may leave the root inactive; the loop below waits for
            // the real foreground authority, but never without an absolute bound.
            let deadline = tokio::time::Instant::now() + FOREGROUND_REACQUIRE_TIMEOUT;
            loop {
                // The native completion may precede didBecomeActive. Wait for the
                // original request's actual foreground authority, never while
                // retaining the privacy lease needed by lifecycle resumption.
                self.ensure_running()?;
                self.ensure_window_active(label)?;
                if self.launch_generation() != generation
                    || self.privacy_policy_generation.load(Ordering::SeqCst) != policy_generation {
                    return Err("Notification permission expired after a privacy or lifecycle transition".into());
                }
                let foreground = {
                    let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                    if state.generation != root_generation {
                        return Err("Notification permission expired after a lifecycle transition".into());
                    }
                    state.active && state.desired_active && !state.inactive
                };
                if !foreground {
                    if tokio::time::Instant::now() >= deadline {
                        return Err("Notification permission expired before the app returned to the foreground".into());
                    }
                    tokio::time::sleep_until(deadline.min(tokio::time::Instant::now() + std::time::Duration::from_millis(25))).await;
                    continue;
                }
                let _mode = tokio::time::timeout_at(deadline, self.privacy_gate.write()).await
                    .map_err(|_| "Notification permission expired before the app returned to the foreground".to_string())?;
                self.ensure_running()?;
                self.ensure_window_active(label)?;
                self.normal_writable()?;
                if self.launch_generation() != generation
                    || self.privacy_policy_generation.load(Ordering::SeqCst) != policy_generation {
                    return Err("Notification permission expired after a privacy or lifecycle transition".into());
                }
                // Retain authority through the synchronous policy/settings commit:
                // native lifecycle callbacks do not acquire privacy_gate.
                let foreground = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                if foreground.generation != root_generation {
                    return Err("Notification permission expired after a lifecycle transition".into());
                }
                if !foreground.active || !foreground.desired_active || foreground.inactive {
                    drop(foreground);
                    drop(_mode);
                    continue;
                }
                break (_mode, foreground);
            }
        };
        #[cfg(not(mobile))]
        let _mode = {
            // Desktop has no native lifecycle wait, but retains the same
            // authority checks before and after acquiring the write lease.
            self.ensure_running()?;
            self.ensure_window_active(label)?;
            if self.launch_generation() != generation
                || self.privacy_policy_generation.load(Ordering::SeqCst) != policy_generation {
                return Err("Notification permission expired after a privacy or lifecycle transition".into());
            }
            let mode = self.privacy_gate.write().await;
            self.ensure_running()?;
            self.ensure_window_active(label)?;
            self.normal_writable()?;
            if self.launch_generation() != generation
                || self.privacy_policy_generation.load(Ordering::SeqCst) != policy_generation {
                return Err("Notification permission expired after a privacy or lifecycle transition".into());
            }
            mode
        };
        self.notifications.validate_permission_request(revision)?;
        if !granted { return Ok(json!({"granted":false})); }
        self.notifications.configure(true, false)?;
        if let Err(error) = self.store.update(|state| { state.settings.completion_notifications = true; Ok(()) }) {
            let _ = self.notifications.configure(false, false);
            return Err(error);
        }
        #[cfg(mobile)]
        drop(foreground);
        self.notify_library_changed();
        Ok(json!({"granted":true}))
    }

    pub(super) fn notify_verified_admission(&self, receipt: &str, books: &[Book]) {
        if !self.notification_gui.load(Ordering::SeqCst) { return; }
        let language = self.store.locale().unwrap_or_default();
        let locale = crate::localization::Localizer::for_app(&language);
        let title = locale.text("Archive ready", &[]);
        let body = locale.text("A verified archive was added to your library.", &[]);
        for book in books {
            if let Err(error) = self.notifications.admitted(receipt, book, &title, &body) {
                self.notification_warning(error);
            }
        }
    }

    pub(super) fn queue_notification_launch(&self, label: &str) -> Result<(), String> {
        if label != "main" || !self.notification_gui.load(Ordering::SeqCst)
            || self.normal_writable().is_err() { return Ok(()); }
        self.ensure_window_active(label)?;
        // All launch paths order gate -> lifecycle -> queue -> native policy.
        let _admission = self.notification_launch_gate.lock()
            .map_err(|_| "Notification launch admission unavailable")?;
        #[cfg(mobile)]
        let _foreground = self.notification_foreground_guard()?;
        let generation = self.launch_generation();
        let pending = match self.notifications.pending_tap() {
            Ok(pending) => pending,
            Err(error) => { self.notification_warning(error); return Ok(()); }
        };
        let mut queue = self.launch_locations.lock().map_err(|_| "Launch queue unavailable")?;
        // A lifecycle retirement changes admission authority, not delivery
        // identity. Keep the launchId only for the exact still-pending native
        // tap, profile and installation, and readmit only in the foreground.
        // A launch queue normally holds ordinary tab locations only, so this
        // single scan is the whole cost while no notification is queued.
        let mut retained = false;
        let mut invalid = HashSet::new();
        for location in queue.iter_mut() {
            let Some(identity) = location.get("__notification").cloned() else { continue; };
            // Without a pending native tap every queued notification launch is
            // stale, so its identity never has to be parsed or revalidated.
            let valid = match pending.as_ref() {
                Some(expected) => {
                    let tap: NotificationTap = serde_json::from_value(identity).map_err(|error| error.to_string())?;
                    expected == &tap && self.notifications.validate_tap(&tap)?.is_some()
                }
                None => false,
            };
            if valid {
                location["__notificationGeneration"] = json!(generation);
                retained = true;
            } else {
                invalid.insert(location["launchId"].as_str().unwrap_or_default().to_owned());
            }
        }
        if !invalid.is_empty() {
            queue.retain(|location| !invalid.contains(location["launchId"].as_str().unwrap_or_default()));
        }
        if retained { return Ok(()); }
        let Some(tap) = pending else { return Ok(()); };
        let Some(book) = self.notifications.validate_tap(&tap)? else { return Ok(()); };
        if queue.len() >= 10000 { return Err("Too many pending launch locations".into()); }
        let mut location = value(store::TabLocation {
            book_id: book.metadata.id.clone(),
            path: crate::server::encode_path(&book.metadata.main_path),
            title: book.metadata.title.clone(), scroll: 0.0,
        })?;
        location["launchId"] = json!(Uuid::new_v4().to_string());
        location["targetWindow"] = json!(label);
        location["__notification"] = value(tap)?;
        location["__notificationGeneration"] = json!(generation);
        queue.push(location);
        Ok(())
    }

    #[cfg(mobile)]
    pub(super) fn notification_foreground_guard(&self) -> Result<std::sync::MutexGuard<'_, crate::mobile::Lifecycle>, String> {
        let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
        if !state.active || !state.desired_active || state.inactive {
            return Err(crate::user_error::suspended());
        }
        Ok(state)
    }

    pub(super) fn acknowledge_notification_launches(&self, label: &str, ids: &HashSet<String>) -> Result<bool, String> {
        // Caller owns notification_launch_gate through native ack AND removal;
        // do not reacquire it here (the gate is deliberately non-reentrant).
        let queue = self.launch_locations.lock().map_err(|_| "Launch queue unavailable")?;
        let mut acknowledged = false;
        for location in queue.iter().filter(|location| location["targetWindow"].as_str() == Some(label)
            && location["launchId"].as_str().is_some_and(|id| ids.contains(id))) {
            let Some(identity) = location.get("__notification") else { continue; };
            if label != "main" { return Err("Notification opens belong to the trusted root window".into()); }
            self.normal_writable()?;
            if location["__notificationGeneration"].as_u64() != Some(self.launch_generation()) {
                return Err("Notification open expired after a privacy or lifecycle transition".into());
            }
            let tap: NotificationTap = serde_json::from_value(identity.clone()).map_err(|error| error.to_string())?;
            if self.notifications.pending_tap()?.as_ref() != Some(&tap) {
                return Err("The notification open is no longer pending".into());
            }
            if self.notifications.validate_tap(&tap)?.is_none() {
                return Err("The notified archive installation is no longer available".into());
            }
            // A failure leaves both the native tap and runtime launch queued.
            // App's consumedLaunches set prevents duplicate tabs while retrying.
            self.notifications.acknowledge_tap(&tap.id)?;
            acknowledged = true;
        }
        Ok(acknowledged)
    }
}
