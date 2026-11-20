// SPDX-License-Identifier: GPL-3.0-or-later
//! System share-sheet handoff. The renderer never supplies a shareable path:
//! article PDFs come from the existing native renderer and archives come from
//! the library store. Every shared byte is either caller-owned scratch (deleted
//! when the sheet finishes) or a file the library already owns.
use super::*;
#[cfg(target_os = "macos")]
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "macos")]
mod macos {
    use crate::{
        apple_requests::Registry,
        archive_source::RetainedSource,
        native_pdf::{Authority, Commit, OwnedPdf, Phase},
    };
    use std::{
        ffi::CString,
        fs::File,
        os::unix::fs::MetadataExt,
        path::PathBuf,
        sync::{Arc, LazyLock},
        time::Duration,
    };
    use tokio::sync::{OwnedSemaphorePermit, Semaphore};
    use tokio_util::sync::CancellationToken;

    unsafe extern "C" {
        safe fn kiwix_share_present(
            id: u64, host: &objc2_app_kit::NSWindow, path: crate::ffi::CPath<'_>,
            admit: extern "C" fn(u64) -> bool, live: extern "C" fn(u64) -> bool,
            complete: extern "C" fn(u64, u8, i64),
        ) -> bool;
        safe fn kiwix_share_cancel(id: u64);
    }

    pub(super) enum Item {
        Archive { _source: Arc<RetainedSource> },
        Pdf { _pdf: Arc<OwnedPdf> },
    }

    #[derive(serde::Serialize)]
    pub(super) struct Offer {
        path: PathBuf,
        device: u64,
        inode: u64,
    }
    impl Offer {
        pub(super) fn new(path: PathBuf, file: &File) -> Result<Self, String> {
            let metadata = file.metadata().map_err(|error| format!("Reading the file to share: {error}"))?;
            Ok(Self { path, device: metadata.dev(), inode: metadata.ino() })
        }
    }

    struct Lease {
        _item: Item,
        _permit: OwnedSemaphorePermit,
    }

    static SHARES: LazyLock<Registry<Lease>> = LazyLock::new(Registry::default);
    static ADMISSION: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));

    pub(super) fn reserve() -> Result<OwnedSemaphorePermit, String> {
        ADMISSION.clone().try_acquire_owned()
            .map_err(|_| "Finish or close the current macOS share before sharing another file".into())
    }

    extern "C" fn admitted(id: u64) -> bool { SHARES.admitted(id) }
    extern "C" fn is_live(id: u64) -> bool { SHARES.is_live(id) }

    // Shared with ShareStatus in native/share.mm. Status and error numbers are
    // values, so a late callback never dereferences a native buffer.
    extern "C" fn complete(id: u64, status: u8, detail: i64) {
        let Some(pending) = SHARES.take(id) else { return; };
        let result = match status {
            0 | 1 => Ok(()), // Shared or user dismissed the unselected picker.
            2 => Err("The file to share is unavailable".into()),
            3 => Err("Activate the original window and try sharing again".into()),
            4 => Err("Sharing authority expired. Open the item and try again".into()),
            5 => Err("Sharing was cancelled".into()),
            6 => Err("The share sheet did not complete".into()),
            7 => Err(format!("The macOS share service failed ({detail})")),
            9 => Err("This file does not have a stable macOS share reference. Copy it to a local disk and try again".into()),
            _ => Err("The system share sheet could not be opened".into()),
        };
        pending.finish(result);
    }

    struct CancelOnDrop(u64);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            if SHARES.revoke(self.0) {
                // The native cancel only looks up this ID and is a no-op once
                // the sheet has finished, so it is declared safe at the ABI.
                kiwix_share_cancel(self.0);
            }
        }
    }

    struct ScheduledShare(Option<u64>);
    impl ScheduledShare {
        fn show(mut self, origin: tauri::WebviewWindow, path: CString) {
            let Some(id) = self.0 else { return; };
            if !SHARES.is_live(id) {
                complete(id, 4, 0);
                return;
            }
            let Ok(host) = crate::apple_window::WindowRef::new(&origin) else {
                complete(id, 3, 0);
                return;
            };
            // The bridge retains the live window and copies the path before
            // returning; both proof-carrying values are borrowed for the call.
            if kiwix_share_present(id, host.window(), crate::ffi::CPath::new(&path), admitted, is_live, complete) {
                self.0 = None;
            } else {
                complete(id, 8, 0);
            }
        }
    }
    impl Drop for ScheduledShare {
        fn drop(&mut self) {
            if let Some(id) = self.0 {
                drop(SHARES.take(id));
            }
        }
    }

    pub(super) async fn present(
        window: tauri::WebviewWindow, offers: &[Offer], item: Item, permit: OwnedSemaphorePermit,
        commit: Commit, observe: Authority, cancel: CancellationToken,
    ) -> Result<(), String> {
        let path = CString::new(serde_json::to_string(&offers).map_err(|error| error.to_string())?)
            .map_err(|_| "Invalid shared path")?;
        let admit: Authority = Arc::new(move |_| commit(&mut || Ok(())));
        let (id, receiver) = SHARES.register(admit, Lease { _item: item, _permit: permit })?;
        let _guard = CancelOnDrop(id);
        let scheduled = ScheduledShare(Some(id));
        let origin = window.clone();
        window.run_on_main_thread(move || scheduled.show(origin, path))
            .map_err(|_| "Could not schedule the system share sheet")?;
        let mut checks = tokio::time::interval(Duration::from_millis(250));
        checks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let deadline = tokio::time::sleep(Duration::from_secs(120));
        tokio::pin!(receiver, deadline);
        loop {
            tokio::select! {
                result = &mut receiver => return result.map_err(|_| "Native share sheet stopped".to_string())?,
                _ = cancel.cancelled() => return Err("Kiwix stopped waiting. A sharing service you already chose may still finish".into()),
                _ = &mut deadline => return Err("Kiwix stopped waiting for the share. A sharing service you already chose may still finish".into()),
                _ = checks.tick() => {
                    if let Err(error) = observe(Phase::AfterHandoff) {
                        return Err(format!("{error}. A sharing service you already chose may still finish"));
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) async fn present_pdf(
    window: tauri::WebviewWindow, pdf: crate::native_pdf::OwnedPdf,
    commit: crate::native_pdf::Commit, observe: crate::native_pdf::Authority,
    cancel: CancellationToken,
) -> Result<(), String> {
    let permit = macos::reserve()?;
    let offered = macos::Offer::new(pdf.path().to_path_buf(), pdf.validated_file()?)?;
    let pdf = Arc::new(pdf);
    let commit: crate::native_pdf::Commit = Arc::new({
        let pdf = pdf.clone();
        move |action| {
            pdf.share_source_unchanged()?;
            commit(action)
        }
    });
    macos::present(window, &[offered], macos::Item::Pdf { _pdf: pdf }, permit, commit, observe, cancel).await
}

impl Service {
    /// Share a library archive through the platform share sheet. The path is
    /// resolved from the trusted store, never from renderer-supplied input.
    pub(super) async fn share_book(self: Arc<Self>, label: &str, args: Value) -> Result<Value, String> {
        self.writable()?;
        self.ensure_window_active(label)?;
        let generation = self.ensure_generation(&args)?;
        #[cfg(not(target_os = "macos"))]
        let _ = generation;
        let id = text(&args, "bookId")?.to_string();
        let installation = text(&args, "installationId")?.to_string();
        let book = self.store.book(&id)?.ok_or("The archive is no longer in the library")?;
        if installation != book.installation_id {
            return Err("The selected archive installation changed. Open it again before sharing".into());
        }
        if !book.available {
            return Err("The archive file is missing; relocate it before sharing".into());
        }
        let path = PathBuf::from(&book.path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("Reading the archive to share: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("Only a regular archive file can be shared".into());
        }
        #[cfg(target_os = "macos")]
        {
            use crate::native_pdf::{Authority, Commit};
            use tauri::Manager;
            let window = self.app.lock().map_err(|_| "Application handle unavailable")?
                .as_ref().and_then(|app| app.get_webview_window(label))
                .ok_or("The window you started sharing from has closed")?;
            let policy = self.privacy_policy_generation.load(std::sync::atomic::Ordering::SeqCst);
            let weak = Arc::downgrade(&self);
            let label = label.to_owned();
            let verify: Commit = Arc::new(move |action| {
                let service = weak.upgrade().ok_or("Sharing stopped because the app is closing")?;
                let _mode = service.privacy_gate.try_read().map_err(|_| "The browsing policy is changing")?;
                service.ensure_running()?;
                if service.privacy_generation.load(std::sync::atomic::Ordering::SeqCst) != generation
                    || service.privacy_policy_generation.load(std::sync::atomic::Ordering::SeqCst) != policy {
                    return Err("Your privacy settings changed during sharing".into());
                }
                let _window = service.active_window_guard(&label)?;
                service.store.with_current_installation(&id, &installation, action)
            });
            let permit = macos::reserve()?;
            let guard = verify.clone();
            let (source, permit) = blocking(move || {
                guard(&mut || Ok(()))?;
                let source = crate::archive_source::retain(&book)?;
                if source.open()?.metadata()?.id != book.metadata.id {
                    return Err("The archive at the selected path changed. Open it again before sharing".into());
                }
                if let Some(record) = &book.integrity {
                    if record.installation_id != book.installation_id || source.identity()? != record.identity {
                        return Err("The selected archive files changed. Verify them before sharing".into());
                    }
                }
                source.unchanged()?;
                guard(&mut || Ok(()))?;
                Ok((Arc::new(source), permit))
            }).await?;
            let offered = source.files().iter().map(|(name, file)| macos::Offer::new(path.with_file_name(name), file))
                .collect::<Result<Vec<_>, _>>()?;
            let observe: Authority = Arc::new({
                let verify = verify.clone();
                move |_| verify(&mut || Ok(()))
            });
            let commit: Commit = Arc::new({
                let source = source.clone();
                move |action| {
                    source.unchanged()?;
                    verify(action)
                }
            });
            macos::present(window, &offered, macos::Item::Archive { _source: source }, permit, commit, observe, CancellationToken::new()).await?;
            Ok(Value::Null)
        }
        #[cfg(any(target_os = "ios", target_os = "android"))]
        {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("The archive name is not UTF-8")?
                .to_string();
            // The native adapter copies into its own export scratch before the
            // system sheet appears, so the library file is never mutated.
            self.mobile_export(&path, "application/x-zim", &name, true, true).await?;
            Ok(Value::Null)
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let _ = &path;
            Err("System sharing is unavailable on this platform".into())
        }
    }
}
