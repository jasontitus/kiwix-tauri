// SPDX-License-Identifier: GPL-3.0-or-later
//! Apple completion notifications are emitted only by a new, verified library
//! admission. Transport completion and persisted history are not notice authority.
//! The native delegate is installed before the platform application main loop;
//! configuration/permission are deliberately separate, explicit operations.
use crate::{apple_replies::{Replies, Reply}, store::{Book, Store}};
use cap_fs_ext::MetadataExt;
use objc2::rc::Retained;
use objc2_foundation::NSData;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    os::fd::AsRawFd,
    sync::{Arc, LazyLock, Mutex, Weak},
};

#[deny(improper_ctypes)]
unsafe extern "C" {
    safe fn kiwix_notifications_install_early() -> bool;
    safe fn kiwix_notifications_set_waker(wake: extern "C" fn());
    // Synchronous borrow of a Foundation object, with a +1 ARC return value.
    safe fn kiwix_notifications_call(request: &NSData) -> Option<Retained<NSData>>;
    // No Rust references escape. Native borrows an immutable diagnostic only
    // for the duration of the completion call.
    safe fn kiwix_notifications_request_permission(
        id: u64,
        is_live: extern "C" fn(u64) -> bool,
        callback: extern "C" fn(u64, i32, Option<&NSData>),
    );
}

/// Call synchronously from the Apple Rust entry point, before creating/running
/// Tauri's event loop. In particular, Builder::setup is too late for scene-based
/// iOS cold notification responses. Installation never asks for authorization.
pub(crate) fn install_early() {
    if !kiwix_notifications_install_early() {
        eprintln!("Native notification installation requires the main thread");
    }
}

static SERVICE_WAKER: Mutex<Weak<crate::service::Service>> = Mutex::new(Weak::new());

pub(crate) fn install_waker(service: &Arc<crate::service::Service>) -> Result<(), String> {
    *SERVICE_WAKER.lock()
        .map_err(|_| "Notification wakeup is unavailable")? = Arc::downgrade(service);
    kiwix_notifications_set_waker(wake_root);
    Ok(())
}

extern "C" fn wake_root() {
    let service = SERVICE_WAKER.lock().ok().and_then(|weak| weak.upgrade());
    if let Some(service) = service { service.notify_library_changed(); }
}

fn native(request: Value) -> Result<Value, String> {
    let encoded = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    let reply = kiwix_notifications_call(&NSData::with_bytes(&encoded))
        .ok_or("Allocating native notification reply")?;
    let decoded = serde_json::from_slice::<Value>(&reply.to_vec());
    let decoded = decoded.map_err(|error| format!("Invalid native notification reply: {error}"))?;
    if let Some(error) = decoded.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    decoded.get("value").cloned().ok_or_else(|| "Missing native notification reply".into())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NotificationCapability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Safe even when early installation was intentionally skipped for headless
/// operation. Unbundled executables report unavailable without accessing the
/// system notification center or creating native journal state.
pub(crate) fn capability() -> Result<NotificationCapability, String> {
    serde_json::from_value(native(json!({"command":"capability"}))?)
        .map_err(|error| format!("Invalid native notification capability: {error}"))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NotificationTap {
    pub id: String,
    pub receipt: String,
    pub book_id: String,
    pub installation_id: String,
    pub profile_id: String,
}

#[derive(Clone, Default)]
struct Policy {
    enabled: bool,
    protected: bool,
    revision: u64,
    profile_id: Option<String>,
}

pub(crate) struct NotificationManager {
    store: Arc<Store>,
    // Configuration, post admission and tap validation share one ordering. The
    // Service additionally retains its privacy read/write lease around calls.
    policy: Mutex<Policy>,
}

impl NotificationManager {
    pub(crate) fn new(store: Arc<Store>) -> Self {
        Self { store, policy: Mutex::new(Policy { protected: true, ..Policy::default() }) }
    }

    /// The settings owner calls this during bootstrap and before acknowledging a
    /// policy transition. Pass protected=true BEFORE entering private/read-only
    /// mode, then configure again after an authorized exit. Native revocation
    /// invalidates outstanding authorization/delivery callbacks immediately;
    /// protected operation never creates or rewrites the native journal.
    pub(crate) fn configure(&self, enabled: bool, protected: bool) -> Result<(), String> {
        let mut policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
        let protected = protected || self.store.private_mode()? || self.store.read_only()?;
        let revision = policy.revision.checked_add(1).ok_or("Notification policy revision exhausted")?;
        // Fail closed in Rust even if the OS checkpoint fails. The caller must
        // not acknowledge enabling a preference when configure returns an error.
        *policy = Policy { enabled: false, protected: true, revision, profile_id: None };
        let (_, directory) = match self.store.acquisition_profile_directory() {
            Ok(profile) => profile,
            Err(error) => {
                // Even a replaced/inaccessible Store root must retire the
                // previous native admission. Missing authority fails closed
                // natively without opening any replacement profile.
                let _ = native(json!({"command":"configure", "enabled":false, "protected":true}));
                return Err(error);
            }
        };
        let metadata = directory.dir_metadata().map_err(|error| error.to_string())?;
        // Keep this Store-validated borrowed descriptor alive until the
        // synchronous native call has duplicated and identity-checked it.
        let configured = native(json!({"command":"configure", "enabled":enabled, "protected":protected,
            "profileFd":directory.as_raw_fd(), "profileDevice":metadata.dev(), "profileInode":metadata.ino()}))?;
        let profile_id = match configured.get("profileId") {
            Some(Value::Null) => None,
            Some(Value::String(id)) if uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == *id) => Some(id.clone()),
            _ => return Err("Invalid native notification profile identity".into()),
        };
        if enabled && !protected && profile_id.is_none() {
            return Err("Native notification profile was not admitted".into());
        }
        *policy = Policy { enabled, protected, revision, profile_id };
        Ok(())
    }

    /// Invoke ONLY from an active, trusted root user action (never startup,
    /// restore, polling, import completion or a reader-webview command).
    /// Submit while the Service still holds its privacy lease, then release
    /// the lease before waiting for the authorization sheet to finish.
    pub(crate) fn request_permission(&self) -> Result<PermissionRequest, String> {
        let policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
        if policy.protected || self.store.private_mode()? || self.store.read_only()? {
            return Err("Leave private or read-only mode before enabling notifications".into());
        }
        let reply = PERMISSION_REQUESTS.register().ok_or("Native notification request IDs exhausted")?;
        kiwix_notifications_request_permission(reply.id(), permission_is_live, permission_result);
        Ok(PermissionRequest { reply, revision: policy.revision })
    }

    /// Revalidate only after Service has regained the same root's foreground
    /// authority. A superseded request is not an ordinary permission denial.
    pub(crate) fn validate_permission_request(&self, revision: u64) -> Result<(), String> {
        let policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
        if revision != policy.revision || policy.protected
            || self.store.private_mode()? || self.store.read_only()? {
            return Err("Notification permission expired after a policy transition".into());
        }
        Ok(())
    }

    fn current_installation(&self, receipt: &str, book_id: &str, installation_id: &str) -> Result<Option<Book>, String> {
        if !self.store.acquisition_consumed(receipt)? { return Ok(None); }
        Ok(self.store.book(book_id)?.filter(|book| {
            book.installation_id == installation_id && !installation_id.is_empty()
                && book.managed && book.available && book.source_reference.is_none()
        }))
    }

    /// Call immediately AFTER a NEW successful register_books transaction with a
    /// consumed receipt, while retaining the service's normal-writable lease.
    /// Never call when replaying already-consumed receipts or loading history.
    /// Labels must be generic localized application strings, not archive titles,
    /// paths, URLs or other archive-supplied metadata.
    pub(crate) fn admitted(&self, receipt: &str, expected: &Book, title: &str, body: &str) -> Result<bool, String> {
        let policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
        if !policy.enabled || policy.protected || self.store.private_mode()? || self.store.read_only()? {
            return Ok(false);
        }
        let Some(current) = self.current_installation(receipt, &expected.metadata.id, &expected.installation_id)? else {
            return Ok(false);
        };
        if &current != expected {
            return Ok(false);
        }
        if receipt.is_empty() || receipt.len() > 512 {
            return Err("Invalid acquisition notification receipt".into());
        }
        let profile_id = policy.profile_id.as_deref().ok_or("Notification profile is unavailable")?;
        let id = notice_id(profile_id, receipt, &expected.installation_id);
        let value = native(json!({"command":"post", "title":title, "body":body,
            "notice":{"id":id,"receipt":receipt,"bookId":expected.metadata.id,
                "installationId":expected.installation_id,"profileId":profile_id}}))?;
        value.get("accepted").and_then(Value::as_bool).ok_or_else(|| "Missing native notification admission result".into())
    }

    /// Peek/ack is intentional: after writable bootstrap a cold tap stays durable
    /// until the root completes session restoration and accepts an explicit open.
    /// Invalid/replaced/removed installations are discarded, never redirected.
    pub(crate) fn pending_tap(&self) -> Result<Option<NotificationTap>, String> {
        let policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
        if !policy.enabled || policy.protected || self.store.private_mode()? || self.store.read_only()? {
            return Ok(None);
        }
        for _ in 0..32 {
            let value = native(json!({"command":"peek"}))?;
            let tap = value.get("tap").ok_or("Download notifications are unavailable. Restart the app")?;
            if tap.is_null() { return Ok(None); }
            let tap: NotificationTap = serde_json::from_value(tap.clone()).map_err(|_| "Download notifications are unavailable. Restart the app".to_string())?;
            if policy.profile_id.as_deref() == Some(tap.profile_id.as_str())
                && tap.id == notice_id(&tap.profile_id, &tap.receipt, &tap.installation_id)
                && self.current_installation(&tap.receipt, &tap.book_id, &tap.installation_id)?.is_some() {
                return Ok(Some(tap));
            }
            native(json!({"command":"ack", "id":tap.id}))?;
        }
        Ok(None)
    }

    /// The service checks this again before exposing/acknowledging a queued
    /// launch. A valid archive UUID alone never authorizes another installation.
    pub(crate) fn validate_tap(&self, tap: &NotificationTap) -> Result<Option<Book>, String> {
        let policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
        if !policy.enabled || policy.protected || self.store.private_mode()? || self.store.read_only()?
            || policy.profile_id.as_deref() != Some(tap.profile_id.as_str())
            || tap.id != notice_id(&tap.profile_id, &tap.receipt, &tap.installation_id) {
            return Ok(None);
        }
        self.current_installation(&tap.receipt, &tap.book_id, &tap.installation_id)
    }

    /// Parent must revalidate the exact installation when accepting the open,
    /// under its normal-writable/root-active lease, and only then acknowledge.
    pub(crate) fn acknowledge_tap(&self, id: &str) -> Result<(), String> {
        let policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
        if !policy.enabled || policy.protected || self.store.private_mode()? || self.store.read_only()? {
            return Err("Notifications are off in private or read-only mode".into());
        }
        native(json!({"command":"ack", "id":id}))?;
        Ok(())
    }
}

fn notice_id(profile_id: &str, receipt: &str, installation_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update((receipt.len() as u64).to_le_bytes());
    digest.update(receipt.as_bytes());
    digest.update(installation_id.as_bytes());
    format!("org.kiwix.acquisition.{profile_id}.{}", hex::encode(digest.finalize()))
}

type PermissionResult = Result<bool, String>;

static PERMISSION_REQUESTS: LazyLock<Replies<PermissionResult>> = LazyLock::new(Replies::default);

pub(crate) struct PermissionRequest {
    reply: Reply<'static, PermissionResult>,
    revision: u64,
}

impl PermissionRequest {
    pub(crate) async fn wait(self) -> Result<(bool, u64), String> {
        let granted = self.reply.wait().await.map_err(|_| "Native notification permission response was lost")??;
        Ok((granted, self.revision))
    }
}

extern "C" fn permission_is_live(id: u64) -> bool {
    PERMISSION_REQUESTS.is_live(id)
}

extern "C" fn permission_result(id: u64, result: i32, error: Option<&NSData>) {
    PERMISSION_REQUESTS.finish(id, || match result {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(error.map(|bytes| String::from_utf8_lossy(&bytes.to_vec()).into_owned())
            .unwrap_or_else(|| "Native notification authorization failed".into())),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_json_bridge_preserves_headless_and_request_limit_errors() {
        let request = br#"{"command":"capability"}"#;
        let mut padded = request.to_vec();
        padded.resize(16384, b' ');
        let reply = kiwix_notifications_call(&NSData::with_bytes(&padded)).unwrap();
        let reply: Value = serde_json::from_slice(&reply.to_vec()).unwrap();
        assert_eq!(reply["value"]["available"], false);

        padded.push(b' ');
        for bytes in [&padded[..], b"[42]", b"{\"command\":\"capability\"}\0junk"] {
            let reply = kiwix_notifications_call(&NSData::with_bytes(bytes)).unwrap();
            let reply: Value = serde_json::from_slice(&reply.to_vec()).unwrap();
            assert_eq!(reply["error"], "Invalid notification request");
        }
        assert_eq!(
            native(json!({"command":"peek"})).unwrap_err(),
            "Notification delegate was not installed before launch",
        );
    }

    #[tokio::test]
    async fn permission_results_preserve_grants_denials_and_diagnostics() {
        let reason = NSData::with_bytes("Authorization failed: オフライン".as_bytes());
        for (status, error, expected) in [
            (1, None, Ok(true)),
            (0, None, Ok(false)),
            (-1, Some(&*reason), Err("Authorization failed: オフライン")),
            (-1, None, Err("Native notification authorization failed")),
        ] {
            let reply = PERMISSION_REQUESTS.register().unwrap();
            assert!(permission_is_live(reply.id()));
            permission_result(reply.id(), status, error);
            assert!(!permission_is_live(reply.id()));
            assert_eq!(reply.wait().await.unwrap().as_ref().copied().map_err(String::as_str), expected);
        }
    }

    struct Profile(std::path::PathBuf);
    impl Drop for Profile {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn fixture() -> (Profile, super::NotificationManager, crate::store::Book) {
        let root = std::env::temp_dir().join(format!("kiwix-notifications-{}", uuid::Uuid::new_v4()));
        let store = std::sync::Arc::new(crate::store::Store::new(root.clone()).unwrap());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let book = crate::imports::load_book(path.to_str().unwrap(), true, false).unwrap();
        (Profile(root), super::NotificationManager::new(store), book)
    }

    #[test]
    fn consumed_receipt_does_not_authorize_a_replacement_installation() {
        let (_profile, manager, book) = fixture();
        assert!(manager.current_installation("http:one", &book.metadata.id, &book.installation_id).unwrap().is_none());
        manager.store.update(|state| {
            state.books.push(book.clone());
            state.consumed_acquisitions.insert("http:one".into());
            Ok(())
        }).unwrap();
        assert!(manager.current_installation("http:one", &book.metadata.id, &book.installation_id).unwrap().is_some());
        manager.store.update(|state| {
            state.books[0].installation_id = crate::store::new_installation_id();
            Ok(())
        }).unwrap();
        assert!(manager.current_installation("http:one", &book.metadata.id, &book.installation_id).unwrap().is_none());
        assert!(manager.store.acquisition_consumed("http:one").unwrap());
    }

    #[test]
    fn missing_unmanaged_or_unavailable_books_are_not_notice_authority() {
        let (_profile, manager, book) = fixture();
        manager.store.update(|state| {
            state.books.push(book.clone());
            state.consumed_acquisitions.insert("http:one".into());
            state.books[0].available = false;
            Ok(())
        }).unwrap();
        assert!(manager.current_installation("http:one", &book.metadata.id, &book.installation_id).unwrap().is_none());
        manager.store.update(|state| {
            state.books[0].available = true;
            state.books[0].managed = false;
            Ok(())
        }).unwrap();
        assert!(manager.current_installation("http:one", &book.metadata.id, &book.installation_id).unwrap().is_none());
        manager.store.update(|state| { state.books.clear(); Ok(()) }).unwrap();
        assert!(manager.current_installation("http:one", &book.metadata.id, &book.installation_id).unwrap().is_none());
    }
}
