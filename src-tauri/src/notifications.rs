// SPDX-License-Identifier: GPL-3.0-or-later
//! Apple completion notifications are emitted only by a new, verified library
//! admission. Transport completion and persisted history are not notice authority.
//! The native delegate is installed before the platform application main loop;
//! configuration/permission are deliberately separate, explicit operations.
use crate::store::{Book, Store};
use cap_fs_ext::MetadataExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    os::fd::AsRawFd,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::oneshot;

unsafe extern "C" {
    fn kiwix_notifications_install_early();
    fn kiwix_notifications_set_waker(wake: unsafe extern "C" fn());
    fn kiwix_notifications_call(request: *const c_char) -> *mut c_char;
    fn kiwix_notifications_free(value: *mut c_char);
    fn kiwix_notifications_request_permission(
        context: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void, c_int, *const c_char),
    );
}

/// Call synchronously from the Apple Rust entry point, before creating/running
/// Tauri's event loop. In particular, Builder::setup is too late for scene-based
/// iOS cold notification responses. Installation never asks for authorization.
pub(crate) fn install_early() {
    // SAFETY: the native installer takes no arguments, is idempotent
    // (`dispatch_once`) and touches only the Apple notification delegate.
    unsafe { kiwix_notifications_install_early() }
}

static SERVICE_WAKER: Mutex<Weak<crate::service::Service>> = Mutex::new(Weak::new());

pub(crate) fn install_waker(service: &Arc<crate::service::Service>) -> Result<(), String> {
    *SERVICE_WAKER.lock()
        .map_err(|_| "Notification wakeup is unavailable")? = Arc::downgrade(service);
    // SAFETY: `wake_root` is an `extern "C"` function with a 'static lifetime and
    // no captured state; native stores it atomically and may call it from any
    // thread, where it only locks `SERVICE_WAKER`.
    unsafe { kiwix_notifications_set_waker(wake_root) };
    Ok(())
}

unsafe extern "C" fn wake_root() {
    let service = SERVICE_WAKER.lock().ok().and_then(|weak| weak.upgrade());
    if let Some(service) = service { service.notify_library_changed(); }
}

fn native(request: Value) -> Result<Value, String> {
    let encoded = CString::new(serde_json::to_vec(&request).map_err(|error| error.to_string())?)
        .map_err(|_| "Invalid native notification request")?;
    // SAFETY: `encoded` is a live NUL-terminated CString for this call; native
    // reads it synchronously with `strlen` and returns a newly allocated
    // NUL-terminated reply (or NULL) owned by the caller.
    let reply = unsafe { kiwix_notifications_call(encoded.as_ptr()) };
    if reply.is_null() {
        return Err("Allocating native notification reply".into());
    }
    // The native allocation is freed even when decoding fails.
    // SAFETY: `reply` is non-NULL here, NUL-terminated and valid until the
    // matching `kiwix_notifications_free`; the bytes are copied into `Value`.
    let decoded = unsafe { serde_json::from_slice::<Value>(CStr::from_ptr(reply).to_bytes()) };
    // SAFETY: `reply` came from the native allocator and is freed exactly once
    // after the copy above; it is not used again.
    unsafe { kiwix_notifications_free(reply) };
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
    /// Returns the OS grant and policy revision for foreground revalidation.
    /// Denial is an ordinary false grant, not an acquisition/settings failure.
    pub(crate) async fn request_permission(&self) -> Result<(bool, u64), String> {
        let revision = {
            let policy = self.policy.lock().map_err(|_| "Notification policy is unavailable")?;
            if policy.protected || self.store.private_mode()? || self.store.read_only()? {
                return Err("Leave private or read-only mode before enabling notifications".into());
            }
            policy.revision
        };
        let (sender, receiver) = oneshot::channel::<Result<bool, String>>();
        let context = Box::into_raw(Box::new(sender)).cast::<c_void>();
        // SAFETY: `context` is the unique `Box<oneshot::Sender>` for this request
        // and is transferred to native, which calls `permission_result` exactly
        // once on every path and never retains the pointer afterwards.
        unsafe { kiwix_notifications_request_permission(context, permission_result) };
        let granted = receiver.await.map_err(|_| "Native notification permission response was lost")??;
        Ok((granted, revision))
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

unsafe extern "C" fn permission_result(context: *mut c_void, result: c_int, error: *const c_char) {
    // Native calls this exactly once, including immediate policy/busy failures.
    // SAFETY: native invokes this callback exactly once, so this is the unique
    // reclaim of the pointer from `Box::into_raw` in `request_permission`.
    let sender = unsafe { Box::from_raw(context.cast::<oneshot::Sender<Result<bool, String>>>()) };
    let value = if result < 0 {
        Err(if error.is_null() { "Native notification authorization failed".into() }
            else {
                // SAFETY: the native callback passes either NULL or a
                // NUL-terminated message valid for this callback; it is copied.
                unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned()
            })
    } else { Ok(result == 1) };
    let _ = sender.send(value);
}

#[cfg(test)]
mod tests {
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
