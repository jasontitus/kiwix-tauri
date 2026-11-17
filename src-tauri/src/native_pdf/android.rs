// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Authority, Commit, Destination, Files, Phase, Presentation};
use serde_json::json;
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use tauri_plugin_kiwix_mobile::MobileExt;
use tokio_util::sync::CancellationToken;
pub enum Completion { Complete, Cancelled }
struct NativeJob { app: tauri::AppHandle, id: String, files: Files }
impl Drop for NativeJob {
    fn drop(&mut self) {
        let app = self.app.clone(); let id = self.id.clone(); let files = self.files.clone();
        tauri::async_runtime::spawn(async move { let _ = app.kiwix_mobile().call("cancelArticlePdf", json!({"jobId": id})).await; drop(files); });
    }
}
// Shared platform-provider boundary; authority, commit, retained files and
// cancellation remain independent of untrusted article/presentation data.
#[allow(clippy::too_many_arguments)]
pub async fn render(app: tauri::AppHandle, _bootstrap: String, article: String, secret: String, output: PathBuf, presentation: Presentation, destination: Destination, width: u32, height: u32, authority: Authority, commit: Commit, handed_off: Arc<AtomicBool>, files: Files, cancel: CancellationToken) -> Result<Completion, String> {
    let name = presentation.name;
    let id = uuid::Uuid::new_v4().to_string();
    let admitted = authority.clone();
    let _admission = tauri_plugin_kiwix_mobile::android_pdf::admit(id.clone(), Arc::new(move |after| {
        let phase = if after { Phase::AfterHandoff } else { Phase::BeforeHandoff };
        admitted(phase).is_ok()
    }), Arc::new(move |action| {
        commit(&mut || {
            action()?;
            handed_off.store(true, Ordering::SeqCst);
            Ok(())
        })
    }))?;
    let _job = NativeJob { app: app.clone(), id: id.clone(), files };
    let render = app.kiwix_mobile().call("printArticlePdf", json!({
        "jobId":id,"url":article,"secret":secret,"suggestedName":name,
        "destination":if destination == Destination::Share { "share" } else { "print" },
        "output":output,"width":width,"height":height,"labels":presentation.labels
    }));
    let result = tokio::select! {
        result = render => result?,
        _ = cancel.cancelled() => return Err("Article PDF export was cancelled".into()),
    };
    if result.get("cancelled").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(Completion::Cancelled);
    }
    if destination != Destination::Share {
        if result.get("opened").and_then(serde_json::Value::as_bool) != Some(true) {
            return Err("Android did not open the native PDF print UI".into());
        }
        return Ok(Completion::Complete);
    }
    if result.get("rendered").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err("Android did not render the live article PDF".into());
    }
    authority(Phase::BeforeHandoff)?;
    let metadata = std::fs::symlink_metadata(&output).map_err(|error| format!("Reading article PDF: {error}"))?;
    if !metadata.is_file() || !(8..=64 * 1024 * 1024).contains(&metadata.len()) {
        return Err("Android article PDF is invalid or exceeds the 64 MiB limit".into());
    }
    let mut file = std::fs::File::open(&output).map_err(|error| error.to_string())?;
    let mut header = [0u8; 5];
    std::io::Read::read_exact(&mut file, &mut header).map_err(|error| error.to_string())?;
    if &header != b"%PDF-" { return Err("Android did not produce a PDF document".into()); }
    let share = app.kiwix_mobile().call("exportFile",
        super::presentation::android_share_payload(&output, &name, &id, presentation.labels));
    tokio::select! {
        result = share => {
            let cancelled = result?.get("cancelled").and_then(serde_json::Value::as_bool)
                .ok_or("Android PDF sharing returned no completion state")?;
            Ok(if cancelled { Completion::Cancelled } else { Completion::Complete })
        },
        _ = cancel.cancelled() => Err("Article PDF sharing was cancelled".into()),
    }
}
