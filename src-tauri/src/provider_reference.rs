// SPDX-License-Identifier: GPL-3.0-or-later
//! External authority is an opaque native-owned reference, not a trusted path.
//! Native selection alone creates it; synchronous acquisition is for blocking
//! Service archive consumers, including reader/search, integrity and export.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderReference {
    pub reference: String,
    pub display_name: String,
    pub persistent: bool,
}
impl ProviderReference {
    pub fn validate(&self) -> Result<(), String> {
        let id = uuid::Uuid::parse_str(&self.reference).map_err(|_| "Kiwix cannot reopen this file from the other app. Choose it again")?;
        if id.get_version_num() != 4 || !id.to_string().eq_ignore_ascii_case(&self.reference) {
            return Err("Kiwix cannot reopen this file from the other app. Choose it again".into());
        }
        validate_name(&self.display_name)?;
        Ok(())
    }
    /// A non-filesystem marker; never pass it to an app-owned-path exception.
    pub fn path_marker(&self) -> String { format!("external-provider:{}", self.reference) }
    pub fn validate_book(&self, book: &crate::store::Book) -> Result<(), String> {
        self.validate()?;
        if book.managed || book.path != self.path_marker() {
            return Err("External archive references cannot be managed filesystem paths".into());
        }
        Ok(())
    }
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.chars().any(|c| c == '/' || c == '\\' || c.is_control()) {
        return Err("Invalid external archive display name".into());
    }
    Ok(())
}
#[cfg(any(mobile, test))]
pub(crate) fn validate_parts(names: &[&str]) -> Result<(), String> {
    if names.is_empty() || names.len() > 676 { return Err("Invalid external archive part count".into()); }
    for name in names { validate_name(name)?; }
    if names.len() == 1 && names[0].to_ascii_lowercase().ends_with(".zim") { return Ok(()); }
    let first = names[0];
    if !first.to_ascii_lowercase().ends_with(".zimaa") { return Err("Split external archives require an explicitly selected same-folder capability".into()); }
    let base = &first[..first.len() - 2];
    for (index, name) in names.iter().enumerate() {
        let suffix = [b'a' + (index / 26) as u8, b'a' + (index % 26) as u8];
        // File names may differ only by extension case (for example FOO.ZIMAA).
        if name.len() != base.len() + 2 ||
            !name.as_bytes()[..base.len()].eq_ignore_ascii_case(base.as_bytes()) ||
            !name.as_bytes()[base.len()..].eq_ignore_ascii_case(&suffix) {
            return Err("External archive parts are missing, duplicated, or out of order".into());
        }
    }
    Ok(())
}

/// Attach the installation principal only to a trusted acquisition envelope for
/// this exact provider reference; ordinary and destructive errors stay intact.
pub(crate) fn archive_error_context(book: &crate::store::Book, error: String) -> String {
    let Some(reference) = &book.source_reference else { return error; };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&error) else { return error; };
    if value.get("sourceReference").and_then(serde_json::Value::as_str) != Some(reference.reference.as_str())
        || value.get("code").and_then(serde_json::Value::as_str).is_none() {
        return error;
    }
    let Some(object) = value.as_object_mut() else { return error; };
    object.insert("bookId".into(), serde_json::json!(book.metadata.id));
    object.insert("installationId".into(), serde_json::json!(book.installation_id));
    value.to_string()
}

/// Native acquisition failures retain their machine code across adapter layers.
/// Absence is retryable with the existing authority; revoked permission is not.
#[cfg(any(mobile, test))]
fn acquisition_response(reference: &ProviderReference, response: serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(error) = response.get("error") else { return Ok(response); };
    let code = error.get("code").and_then(serde_json::Value::as_str)
        .ok_or("Native provider returned an invalid acquisition error")?;
    let retry = code == "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT";
    let message = if retry {
        "This archive is temporarily unavailable. Reconnect its storage, then reopen or reload the article."
    } else if code == "PROVIDER_REFERENCE_REVOKED" {
        "Access to this archive is no longer authorized. Choose Locate in the library to select it again."
    } else {
        "This archive could not be opened from its document provider. Choose Locate in the library, or import a copy."
    };
    Err(serde_json::json!({
        "code": code, "message": message, "retry": retry, "sourceReference": reference.reference,
    }).to_string())
}

#[cfg(mobile)]
mod native {
    use super::*;
    use crate::archive::Archive;
    use serde_json::{json, Value};
    use std::{fs::File, os::fd::RawFd, path::Path, sync::Arc};
    use tauri_plugin_kiwix_mobile::MobileExt;

    pub(crate) struct ProviderLease { app: tauri::AppHandle, token: String }
    static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
    /// Install only the app-owned native plugin handle during native setup.
    pub(crate) fn install(app: tauri::AppHandle) -> Result<(), String> {
        APP.set(app).map_err(|_| "Provider native resolver already installed".into())
    }
    pub(crate) fn retain_registered(reference: &ProviderReference) -> Result<ProviderFiles, String> {
        retain(APP.get().ok_or("Native provider resolver is not initialized")?, reference)
    }
    pub(crate) fn forget_registered(reference: &ProviderReference) -> Result<(), String> {
        forget(APP.get().ok_or("Native provider resolver is not initialized")?, reference)
    }
    #[cfg(target_os = "android")]
    pub(crate) fn prepare_move_registered(reference: &ProviderReference) -> Result<String, String> {
        reference.validate()?;
        let response = APP.get().ok_or("Native provider resolver is not initialized")?
            .kiwix_mobile().call_sync("prepareExternalArchiveMove", json!({"reference": reference.reference}))
            .map_err(|error| format!("The provider has not authorized moving this archive: {error}. Choose Copy to keep the original."))?;
        let receipt = response.get("receipt").and_then(Value::as_str).ok_or("Native provider move receipt missing")?;
        let id = uuid::Uuid::parse_str(receipt).map_err(|_| "Native provider move receipt invalid")?;
        if id.get_version_num() != 4 || id.to_string() != receipt { return Err("Native provider move receipt invalid".into()); }
        Ok(receipt.into())
    }
    #[cfg(target_os = "android")]
    pub(crate) fn delete_move_registered(reference: &ProviderReference, receipt: &str, parts: &[Value]) -> Result<(), String> {
        reference.validate()?;
        uuid::Uuid::parse_str(receipt).map_err(|_| "Native provider move receipt invalid")?;
        let response = APP.get().ok_or("Native provider resolver is not initialized")?
            .kiwix_mobile().call_sync("deleteExternalArchiveMove", json!({"reference": reference.reference, "receipt": receipt, "parts": parts}))
            .map_err(|error| format!("The managed copy is ready, but the provider original was preserved: {error}"))?;
        if response.get("deleted").and_then(Value::as_bool) != Some(true) {
            return Err("Provider deletion was not confirmed; the managed copy and recovery journal were preserved".into());
        }
        Ok(())
    }
    impl std::fmt::Debug for ProviderLease {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("ProviderLease(native scope retained)")
        }
    }
    impl Drop for ProviderLease {
        fn drop(&mut self) {
            let app = self.app.clone();
            let token = self.token.clone();
            // Never synchronously wait for UI dispatch in a destructor. Native
            // closes its originals/scope only after libzim and Rust FDs drop;
            // the owned asynchronous release extends, never shortens, authority.
            tauri::async_runtime::spawn(async move {
                let _ = app.kiwix_mobile().call("releaseExternalArchive", json!({"lease":token})).await;
            });
        }
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Descriptor { name: String, fd: RawFd }
    pub(crate) struct ProviderFiles {
        pub files: Vec<(String, File)>,
        pub lease: Arc<ProviderLease>,
    }
    impl ProviderFiles {
        pub fn open(&self) -> Result<Archive, String> {
            Archive::open_files_scoped(Path::new(""), &self.files, Some(self.lease.clone()))
        }
    }
    pub(crate) fn retain(app: &tauri::AppHandle, reference: &ProviderReference) -> Result<ProviderFiles, String> {
        reference.validate()?;
        // This response comes only from the native PluginHandle invocation, not
        // from an IPC argument. Each native FD stays owned by its lease while
        // fcntl makes an independent Rust owner in this same application process.
        let response = acquisition_response(reference,
            app.kiwix_mobile().call_sync("acquireExternalArchive", json!({"reference":reference.reference}))
                .map_err(|e| format!("External archive unavailable: {e}. Reconnect its storage and retry, or choose Locate if access was revoked."))?)?;
        let token = response.get("lease").and_then(Value::as_str).ok_or("Native provider lease missing")?;
        uuid::Uuid::parse_str(token).map_err(|_| "Native provider lease invalid")?;
        let lease = Arc::new(ProviderLease { app: app.clone(), token: token.into() });
        let descriptors: Vec<Descriptor> = serde_json::from_value(response.get("files").cloned().ok_or("Native provider files missing")?)
            .map_err(|e| e.to_string())?;
        validate_parts(&descriptors.iter().map(|part| part.name.as_str()).collect::<Vec<_>>())?;
        let mut files = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors {
            if descriptor.fd < 0 { return Err("Native provider descriptor invalid".into()); }
            // SAFETY: the native lease owns `descriptor.fd` for the duration of
            // this call; F_DUPFD_CLOEXEC does not take ownership, and the new
            // descriptor is owned only by the `File` built from it.
            let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(descriptor.fd) };
            let duplicate = rustix::io::fcntl_dupfd_cloexec(borrowed, 0)
                .map_err(|error| std::io::Error::from(error).to_string())?;
            let file = File::from(duplicate);
            if !file.metadata().map_err(|e| e.to_string())?.is_file() || rustix::fs::tell(&file).is_err() {
                return Err("This provider does not offer a seekable regular file. Choose Import archives to make a managed copy.".into());
            }
            files.push((descriptor.name, file));
        }
        Ok(ProviderFiles { files, lease })
    }
    pub(crate) fn forget(app: &tauri::AppHandle, reference: &ProviderReference) -> Result<(), String> {
        reference.validate()?;
        app.kiwix_mobile().call_sync("forgetExternalArchive", json!({"reference":reference.reference})).map(|_| ())
    }
}
#[cfg(mobile)]
pub(crate) use native::{forget_registered, install, retain_registered, ProviderFiles};
#[cfg(target_os = "android")]
pub(crate) use native::{prepare_move_registered, delete_move_registered};

#[cfg(test)]
mod tests {
    use super::*;
    fn reference() -> ProviderReference {
        ProviderReference { reference: "2d1d57d1-3762-4e59-9fe6-00e888402a2b".into(), display_name: "USB.zim".into(), persistent: true }
    }
    #[test]
    fn temporary_provider_absence_keeps_authority_and_next_acquisition_can_read() {
        use serde_json::json;
        let source = reference();
        let original = source.clone();
        let unavailable = acquisition_response(&source,
            json!({"error":{"code":"PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT"}})).unwrap_err();
        let error: serde_json::Value = serde_json::from_str(&unavailable).unwrap();
        assert_eq!(error["code"], "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT");
        assert_eq!(error["sourceReference"], source.reference);
        assert_eq!(error["retry"], true);
        assert!(!error["message"].as_str().unwrap().contains("Locate"));
        assert!(!unavailable.contains("java.lang"));
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let mut book = crate::imports::load_book(fixture.to_str().unwrap(), false, false).unwrap();
        book.path = source.path_marker();
        book.source_reference = Some(source.clone());
        let original_book = book.clone();
        let contextual = archive_error_context(&book, unavailable);
        let contextual: serde_json::Value = serde_json::from_str(&contextual).unwrap();
        assert_eq!(contextual["bookId"], book.metadata.id);
        assert_eq!(contextual["installationId"], book.installation_id);
        assert_eq!(contextual["sourceReference"], source.reference);
        assert_eq!(book, original_book);
        // Unix (and the mobile providers this models) open the retained
        // descriptor alone and ignore the directory. The Windows bridge
        // re-binds every retained handle to a nonreplaceable lease on its
        // absolute native path (native/archive.cpp), so it needs the file's
        // real location; descriptor-only opens do not exist there.
        #[cfg(windows)]
        let (directory, name) = (
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures"),
            "small.zim",
        );
        #[cfg(not(windows))]
        let (directory, name) = (std::path::PathBuf::new(), "USB.zim");
        let files = vec![(name.into(), std::fs::File::open(fixture).unwrap())];
        let restored = acquisition_response(&source, json!({"lease":uuid::Uuid::new_v4().to_string()})).unwrap();
        assert!(restored.get("lease").is_some());
        let archive = crate::archive::Archive::open_files(&directory, &files).unwrap();
        assert!(archive.check_integrity().unwrap());
        let title = archive.title_batch(0, 1).unwrap().titles.remove(0);
        assert!(!archive.search(&title, false, 0, 10).unwrap().hits.is_empty());
        assert_eq!(source, original);
    }

    #[test]
    fn revoked_or_changed_provider_authority_is_not_a_temporary_storage_retry() {
        for code in ["PROVIDER_REFERENCE_REVOKED", "PROVIDER_ARCHIVE_PARTS_CHANGED",
            "PROVIDER_MOVE_CONTENT_CHANGED", "PROVIDER_PERMISSION_DENIED",
            "PROVIDER_NOT_SEEKABLE_USE_COPY_IMPORT", "PROVIDER_UNAVAILABLE_USE_COPY_IMPORT"] {
            let error = acquisition_response(&reference(), serde_json::json!({"error":{"code":code}})).unwrap_err();
            let error: serde_json::Value = serde_json::from_str(&error).unwrap();
            assert_eq!(error["code"], code);
            assert_eq!(error["retry"], false);
        }
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let mut book = crate::imports::load_book(fixture.to_str().unwrap(), false, false).unwrap();
        book.source_reference = Some(reference());
        let deletion_error = "The managed copy is ready, but the provider original was preserved: PROVIDER_MOVE_CONTENT_CHANGED".to_string();
        assert_eq!(archive_error_context(&book, deletion_error.clone()), deletion_error);
    }
    #[test]
    fn provider_authority_is_never_a_path_or_uri() {
        for value in ["/private/book.zim", "content://provider/book", "42", "00000000-0000-0000-0000-000000000000"] {
            let mut source = reference(); source.reference = value.into(); assert!(source.validate().is_err());
        }
        assert!(reference().validate().is_ok());
        assert!(serde_json::from_value::<ProviderReference>(serde_json::json!({"reference":reference().reference,"displayName":"x.zim","persistent":true,"uri":"content://evil"})).is_err());
    }
    #[test]
    fn provider_parts_do_not_broaden_folder_authority() {
        assert!(validate_parts(&["one.zim"]).is_ok());
        assert!(validate_parts(&["split.zimaa", "split.zimab"]).is_ok());
        for parts in [vec!["../one.zim"], vec!["one.zim", "other.zim"], vec!["split.zimab"], vec!["split.zimaa", "split.zimac"], vec!["split.zimaa", "split.zimaa"]] {
            assert!(validate_parts(&parts).is_err());
        }
    }
    #[test]
    fn source_markers_never_become_managed_paths() {
        let mut source = reference();
        source.persistent = false; // Volatile authority has exactly the same source semantics.
        let mut book = crate::imports::load_book(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim").to_str().unwrap(), false, false).unwrap();
        book.source_reference = Some(source.clone());
        assert!(source.validate_book(&book).is_err());
        book.path = source.path_marker(); assert!(source.validate_book(&book).is_ok());
        book.managed = true; assert!(source.validate_book(&book).is_err());
    }
    #[test]
    fn protected_provider_overlay_cannot_reach_durable_profile() {
        let root = std::env::temp_dir().join(format!("kiwix-provider-overlay-{}", uuid::Uuid::new_v4()));
        let store = crate::store::Store::new(root.clone()).unwrap();
        store.set_runtime_mode(true, false).unwrap();
        let before = std::fs::read(root.join("state.json")).unwrap();
        let mut source = reference(); source.persistent = false;
        let mut book = crate::imports::load_book(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim").to_str().unwrap(), false, false).unwrap();
        book.path = source.path_marker(); book.source_reference = Some(source.clone());
        store.register_provider_book(book.clone()).unwrap();
        assert_eq!(store.book(&book.metadata.id).unwrap().unwrap(), book);
        assert_eq!(store.read().unwrap().books, vec![book.clone()]);
        assert_eq!(std::fs::read(root.join("state.json")).unwrap(), before);
        assert!(store.register_provider_book(book.clone()).is_err());
        assert!(store.update(|state| { state.books.push(book.clone()); Ok(()) }).is_err());
        store.update(|state| { state.settings.theme = "dark".into(); Ok(()) }).unwrap();
        let durable: crate::store::State = serde_json::from_slice(&std::fs::read(root.join("state.json")).unwrap()).unwrap();
        assert!(durable.books.is_empty());
        assert_eq!(store.clear_provider_books().unwrap(), vec![book]);
        assert!(store.read().unwrap().books.is_empty());
        drop(store); std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn read_only_reconnection_is_atomic_and_never_saved() {
        let root = std::env::temp_dir().join(format!("kiwix-provider-readonly-{}", uuid::Uuid::new_v4()));
        let store = crate::store::Store::new(root.clone()).unwrap();
        let before = std::fs::read(root.join("state.json")).unwrap();
        store.set_runtime_mode(false, true).unwrap();
        let mut source = reference(); source.persistent = false;
        let mut book = crate::imports::load_book(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim").to_str().unwrap(), false, false).unwrap();
        book.path = source.path_marker(); book.source_reference = Some(source);
        store.register_provider_book(book.clone()).unwrap();
        let mut replacement = book.clone();
        replacement.installation_id = crate::store::new_installation_id();
        replacement.source_reference.as_mut().unwrap().reference = uuid::Uuid::new_v4().to_string();
        replacement.path = replacement.source_reference.as_ref().unwrap().path_marker();
        assert!(store.replace_provider_book("stale-installation", replacement.clone()).is_err());
        assert_eq!(store.book(&book.metadata.id).unwrap().unwrap(), book);
        store.replace_provider_book(&book.installation_id, replacement.clone()).unwrap();
        assert_eq!(store.book(&book.metadata.id).unwrap().unwrap(), replacement);
        assert_eq!(std::fs::read(root.join("state.json")).unwrap(), before);
        assert!(store.remove_provider_book(&book.metadata.id).unwrap().is_some());
        assert!(store.book(&book.metadata.id).unwrap().is_none());
        assert_eq!(std::fs::read(root.join("state.json")).unwrap(), before);
        drop(store); std::fs::remove_dir_all(root).unwrap();
    }
}
