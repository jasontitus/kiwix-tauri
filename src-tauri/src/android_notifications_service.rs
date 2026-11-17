// SPDX-License-Identifier: GPL-3.0-or-later
//! Android's durable handoff uses the same verified receipt/install identity as
//! library admission. No transport callback is notification authority.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(target_os = "android")]
use std::sync::atomic::AtomicU64;
#[cfg(target_os = "android")]
use tauri_plugin_kiwix_mobile::MobileExt;

#[cfg(target_os = "android")]
#[derive(Default)]
pub(super) struct AndroidNotifications {
    policy: tokio::sync::Mutex<Policy>,
    revision: AtomicU64,
}
#[cfg(target_os = "android")]
#[derive(Default)]
struct Policy { initialized: bool, profile_id: Option<String> }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NotificationTap {
    id: String,
    receipt: String,
    book_id: String,
    installation_id: String,
    profile_id: String,
}
fn notice_id(profile: &str, receipt: &str, installation: &str) -> String {
    let mut digest = Sha256::new();
    digest.update((receipt.len() as u64).to_le_bytes());
    digest.update(receipt.as_bytes());
    digest.update(installation.as_bytes());
    format!("org.kiwix.acquisition.{profile}.{}", hex::encode(digest.finalize()))
}
fn current_installation(store: &Store, tap: &NotificationTap, profile: Option<&str>) -> Result<Option<Book>, String> {
    let settings = store.settings()?;
    if !settings.completion_notifications || settings.private_mode || settings.read_only
        || profile != Some(tap.profile_id.as_str()) || tap.receipt.is_empty() || tap.receipt.len() > 512
        || tap.id != notice_id(&tap.profile_id, &tap.receipt, &tap.installation_id)
        || !store.acquisition_consumed(&tap.receipt)? { return Ok(None); }
    Ok(store.book(&tap.book_id)?.filter(|book| !tap.installation_id.is_empty()
        && book.installation_id == tap.installation_id && book.managed && book.available && book.source_reference.is_none()))
}
fn configured_profile(reply: &Value) -> Result<Option<String>, String> {
    match reply.get("profileId") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == *id) => Ok(Some(id.clone())),
        _ => Err("Invalid Android notification profile identity".into()),
    }
}

#[cfg(target_os = "android")]
impl Service {
    // After bootstrap the caller owns the privacy lease. Serializing with
    // post/peek/ack orders revocation before settings transition acknowledgement.
    pub(crate) async fn initialize_android_notifications(&self) -> Result<(), String> {
        let mut policy = self.android_notifications.policy.lock().await;
        if policy.initialized { return Ok(()); }
        let settings = self.store.settings()?;
        let protected = settings.private_mode || settings.read_only;
        let configured = self.mobile_call("configureCompletionNotifications", json!({"enabled":settings.completion_notifications,
            "protected":protected})).await;
        if protected {
            self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?
                .entries.retain(|entry| entry.kind != "download-action");
        }
        let reply = configured?;
        policy.profile_id = configured_profile(&reply)?;
        policy.initialized = true;
        Ok(())
    }
    pub(super) async fn configure_android_notifications(&self, enabled: bool, protected: bool) -> Result<(), String> {
        self.android_notifications.revision.fetch_add(1, Ordering::SeqCst);
        let mut policy = self.android_notifications.policy.lock().await;
        policy.initialized = false;
        policy.profile_id = None;
        let configured = self.mobile_call("configureCompletionNotifications", json!({"enabled":enabled,"protected":protected})).await;
        if protected {
            self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?
                .entries.retain(|entry| entry.kind != "download-action");
        }
        let reply = configured?;
        policy.profile_id = configured_profile(&reply)?;
        if enabled && !protected && policy.profile_id.is_none() { return Err("Android notification profile unavailable".into()); }
        policy.initialized = true;
        Ok(())
    }
    pub(super) async fn android_notification_capability(&self) -> Value {
        match self.mobile_call("completionNotificationCapability", json!({})).await {
            Ok(reply) if reply.get("available").and_then(Value::as_bool).is_some() => reply,
            Ok(_) => json!({"available":false,"reason":"Invalid Android notification capability"}),
            Err(error) => json!({"available":false,"reason":error}),
        }
    }
    pub(super) async fn request_completion_notifications(self: &Arc<Self>, args: Value) -> Result<Value, String> {
        let label = args.get("__windowLabel").and_then(Value::as_str).unwrap_or("main");
        if label != "main" { return Err("Completion notification permission requires the trusted root window".into()); }
        let (generation, policy_generation, root_generation, revision) = {
            let _mode = self.privacy_gate.read().await;
            self.ensure_running()?;
            self.mobile_admit("request_completion_notifications", &args)?;
            self.ensure_window_active(label)?;
            self.normal_writable()?;
            self.initialize_android_notifications().await?;
            let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            (self.ensure_generation(&args)?, self.privacy_policy_generation.load(Ordering::SeqCst), state.generation,
                self.android_notifications.revision.load(Ordering::SeqCst))
        };
        // The permission sheet retires foreground admission. Hold no privacy
        // lease while waiting for it or for the original root to resume.
        let reply = self.mobile_call("requestCompletionNotifications", json!({})).await?;
        let granted = reply.get("granted").and_then(Value::as_bool).ok_or("Missing Android notification permission result")?;
        loop {
            self.ensure_running()?;
            self.ensure_window_active(label)?;
            if self.launch_generation() != generation || self.privacy_policy_generation.load(Ordering::SeqCst) != policy_generation
                || self.android_notifications.revision.load(Ordering::SeqCst) != revision {
                return Err("Notification permission expired after a privacy or lifecycle transition".into());
            }
            let active = {
                let state = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                if state.generation != root_generation { return Err("Notification permission expired after a lifecycle transition".into()); }
                state.active && state.desired_active && !state.inactive
            };
            if !active { tokio::time::sleep(std::time::Duration::from_millis(25)).await; continue; }
            let _mode = self.privacy_gate.write().await;
            self.mobile_admit("request_completion_notifications", &args)?;
            self.normal_writable()?;
            if self.launch_generation() != generation || self.privacy_policy_generation.load(Ordering::SeqCst) != policy_generation
                || self.android_notifications.revision.load(Ordering::SeqCst) != revision {
                return Err("Notification permission expired after a privacy or lifecycle transition".into());
            }
            if !granted { return Ok(json!({"granted":false})); }
            self.configure_android_notifications(true, false).await?;
            // Retain lifecycle authority through the synchronous settings commit,
            // after native IO has returned; never carry this guard across await.
            let committed = {
                let foreground = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
                if !foreground.active || !foreground.desired_active || foreground.inactive
                    || foreground.generation != root_generation || self.launch_generation() != generation {
                    Err("Notification permission expired after a lifecycle transition".to_owned())
                } else {
                    self.ensure_running().and_then(|_| self.ensure_window_active(label))
                        .and_then(|_| self.store.update(|state| { state.settings.completion_notifications = true; Ok(()) }))
                }
            };
            if let Err(error) = committed {
                let _ = self.configure_android_notifications(false, false).await;
                return Err(error);
            }
            self.notify_library_changed();
            return Ok(json!({"granted":true}));
        }
    }

    pub(super) fn notify_verified_admission(&self, receipt: &str, books: &[Book]) {
        let Ok(settings) = self.store.settings() else { return; };
        if !settings.completion_notifications || settings.private_mode || settings.read_only { return; }
        let Some(app) = self.app.lock().ok().and_then(|app| app.clone()) else { return; };
        let state = self.android_notifications.clone();
        let revision = state.revision.load(Ordering::SeqCst);
        let store = self.store.clone();
        let gate = self.privacy_gate.clone();
        let receipt = receipt.to_owned();
        let books = books.to_vec();
        tauri::async_runtime::spawn(async move {
            let _mode = gate.read().await;
            let result: Result<(), String> = async {
                let mut policy = state.policy.lock().await;
                let settings = store.settings()?;
                if state.revision.load(Ordering::SeqCst) != revision || !settings.completion_notifications
                    || settings.private_mode || settings.read_only { return Ok(()); }
                if !policy.initialized {
                    let reply = app.kiwix_mobile().call("configureCompletionNotifications", json!({"enabled":true,"protected":false})).await?;
                    policy.profile_id = configured_profile(&reply)?;
                    policy.initialized = true;
                }
                let profile = policy.profile_id.as_deref().ok_or("Android notification profile unavailable")?;
                let language = store.locale().unwrap_or_default();
                let locale = crate::localization::Localizer::for_app(&language);
                for expected in books {
                    let tap = NotificationTap { id:notice_id(profile, &receipt, &expected.installation_id), receipt:receipt.clone(),
                        book_id:expected.metadata.id.clone(), installation_id:expected.installation_id.clone(), profile_id:profile.into() };
                    if current_installation(&store, &tap, Some(profile))?.as_ref() != Some(&expected) { continue; }
                    app.kiwix_mobile().call("admittedCompletionNotification", json!({"notice":tap,
                        "title":locale.text("Archive ready", &[]),"body":locale.text("A verified archive was added to your library.", &[]),
                        "channelName":locale.text("Completion notifications", &[])})).await?;
                }
                Ok(())
            }.await;
            if let Err(error) = result { eprintln!("Android completion notification: {error}"); }
        });
    }

    pub(super) async fn queue_android_notification_launch(&self, label: &str) -> Result<(), String> {
        if label != "main" || self.normal_writable().is_err() { return Ok(()); }
        self.initialize_android_notifications().await?;
        let policy = self.android_notifications.policy.lock().await;
        let mut pending = None;
        for _ in 0..32 {
            let reply = self.mobile_call("takeCompletionNotificationTap", json!({})).await?;
            let value = reply.get("tap").ok_or("Download notifications are unavailable. Restart the app")?;
            if value.is_null() { break; }
            let tap: NotificationTap = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
            if let Some(book) = current_installation(&self.store, &tap, policy.profile_id.as_deref())? {
                pending = Some((tap, book)); break;
            }
            self.mobile_call("acknowledgeCompletionNotificationTap", json!({"id":tap.id})).await?;
        }
        self.ensure_window_active(label)?;
        let foreground = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
        if !foreground.active || !foreground.desired_active || foreground.inactive { return Err(crate::user_error::suspended()); }
        let generation = self.launch_generation();
        let mut queue = self.launch_locations.lock().map_err(|_| "Launch queue unavailable")?;
        let mut retained = false;
        queue.retain_mut(|location| {
            let Some(identity) = location.get("__notification") else { return true; };
            if pending.as_ref().is_some_and(|(tap, _)| serde_json::to_value(tap).ok().as_ref() == Some(identity)) {
                location["__notificationGeneration"] = json!(generation);
                retained = true;
                true
            } else { false }
        });
        if retained { return Ok(()); }
        let Some((tap, book)) = pending else { return Ok(()); };
        if queue.len() >= 10000 { return Err("Too many pending launch locations".into()); }
        let mut location = value(store::TabLocation { book_id:book.metadata.id, path:encode_path(&book.metadata.main_path),
            title:book.metadata.title, scroll:0.0 })?;
        location["launchId"] = json!(Uuid::new_v4().to_string());
        location["targetWindow"] = json!(label);
        location["__notification"] = value(tap)?;
        location["__notificationGeneration"] = json!(generation);
        queue.push(location);
        Ok(())
    }

    pub(super) async fn acknowledge_android_notification_launches(&self, label: &str, ids: &HashSet<String>) -> Result<(), String> {
        let policy = self.android_notifications.policy.lock().await;
        let locations: Vec<_> = self.launch_locations.lock().map_err(|_| "Launch queue unavailable")?.iter()
            .filter(|location| location["targetWindow"].as_str() == Some(label)
                && location["launchId"].as_str().is_some_and(|id| ids.contains(id)) && location.get("__notification").is_some()).cloned().collect();
        for location in locations {
            if label != "main" { return Err("Notification opens belong to the trusted root window".into()); }
            self.normal_writable()?;
            self.ensure_window_active(label)?;
            if !self.mobile_active() || location["__notificationGeneration"].as_u64() != Some(self.launch_generation()) {
                return Err("Notification open expired after a privacy or lifecycle transition".into());
            }
            let tap: NotificationTap = serde_json::from_value(location["__notification"].clone()).map_err(|error| error.to_string())?;
            if current_installation(&self.store, &tap, policy.profile_id.as_deref())?.is_none() {
                return Err("The notified archive installation is no longer available".into());
            }
            let pending = self.mobile_call("takeCompletionNotificationTap", json!({})).await?;
            if pending.get("tap") != Some(&value(&tap)?) { return Err("The notification open is no longer pending".into()); }
            if !self.mobile_active() || location["__notificationGeneration"].as_u64() != Some(self.launch_generation()) {
                return Err("Notification open expired after a lifecycle transition".into());
            }
            self.mobile_call("acknowledgeCompletionNotificationTap", json!({"id":tap.id})).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Profile(PathBuf);
    impl Drop for Profile {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn fixture() -> (Profile, Store, Book, NotificationTap) {
        let root = std::env::temp_dir().join(format!("kiwix-android-notifications-{}", Uuid::new_v4()));
        let store = Store::new(root.clone()).unwrap();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let book = crate::imports::load_book(path.to_str().unwrap(), true, false).unwrap();
        let profile = Uuid::new_v4().to_string();
        let tap = NotificationTap { id:notice_id(&profile, "http:one", &book.installation_id),
            receipt:"http:one".into(), book_id:book.metadata.id.clone(),
            installation_id:book.installation_id.clone(), profile_id:profile };
        store.update(|state| {
            state.settings.completion_notifications = true;
            state.books.push(book.clone());
            state.consumed_acquisitions.insert(tap.receipt.clone());
            Ok(())
        }).unwrap();
        (Profile(root), store, book, tap)
    }

    #[test]
    fn notification_id_binds_profile_receipt_and_installation() {
        let id = notice_id("profile", "http:one", "installation");
        assert_eq!(id, notice_id("profile", "http:one", "installation"));
        assert_ne!(id, notice_id("other-profile", "http:one", "installation"));
        assert_ne!(id, notice_id("profile", "http:two", "installation"));
        assert_ne!(id, notice_id("profile", "http:one", "replacement"));
        assert_ne!(notice_id("profile", "ab", "c"), notice_id("profile", "a", "bc"));
    }

    #[test]
    fn notification_tap_requires_consumed_receipt_and_exact_current_installation() {
        let (_directory, store, book, tap) = fixture();
        let current = || current_installation(&store, &tap, Some(&tap.profile_id)).unwrap();
        assert_eq!(current().as_ref(), Some(&book));
        assert!(current_installation(&store, &tap, Some("other-profile")).unwrap().is_none());
        let mut forged = tap.clone();
        forged.id.push('x');
        assert!(current_installation(&store, &forged, Some(&tap.profile_id)).unwrap().is_none());
        store.update(|state| { state.consumed_acquisitions.clear(); Ok(()) }).unwrap();
        assert!(current().is_none());
        store.update(|state| {
            state.consumed_acquisitions.insert(tap.receipt.clone());
            state.books[0].installation_id = crate::store::new_installation_id();
            Ok(())
        }).unwrap();
        assert!(current().is_none());
        store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
        assert!(current().is_none());
    }

    #[test]
    fn notification_tap_rejects_disabled_private_read_only_unmanaged_and_unavailable() {
        let (_directory, store, _book, tap) = fixture();
        let current = || current_installation(&store, &tap, Some(&tap.profile_id)).unwrap();
        store.update(|state| { state.settings.completion_notifications = false; Ok(()) }).unwrap();
        assert!(current().is_none());
        store.update(|state| { state.settings.completion_notifications = true; Ok(()) }).unwrap();
        store.set_runtime_mode(true, false).unwrap();
        assert!(current().is_none());
        store.set_runtime_mode(false, true).unwrap();
        assert!(current().is_none());
        store.set_runtime_mode(false, false).unwrap();
        store.update(|state| { state.books[0].managed = false; Ok(()) }).unwrap();
        assert!(current().is_none());
        store.update(|state| { state.books[0].managed = true; state.books[0].available = false; Ok(()) }).unwrap();
        assert!(current().is_none());
    }

    #[test]
    fn notification_profile_identity_must_be_native_canonical_uuid() {
        let profile = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned();
        assert_eq!(configured_profile(&json!({"profileId":profile})).unwrap(), Some(profile.clone()));
        assert_eq!(configured_profile(&json!({"profileId":null})).unwrap(), None);
        assert!(configured_profile(&json!({"profileId":"not-a-uuid"})).is_err());
        assert!(configured_profile(&json!({"profileId":profile.to_uppercase()})).is_err());
        assert!(configured_profile(&json!({})).is_err());
    }
}
