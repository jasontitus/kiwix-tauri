// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(target_os = "android")]
use super::{kokoro, Service};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Take};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
#[cfg(target_os = "android")]
use std::sync::Arc;
#[cfg(target_os = "android")]
use tauri_plugin_kiwix_mobile::MobileExt;

#[derive(Default)]
pub(super) struct BundledVoice(tokio::sync::OnceCell<Option<ApkAsset>>);

impl BundledVoice {
    async fn locate<F: std::future::Future<Output = Result<Value, String>>>(
        &self,
        query: impl FnOnce() -> F,
    ) -> Result<Option<&ApkAsset>, String> {
        // Cache a confirmed standard or offline build, but allow Retry to ask
        // AssetManager again after a transient bridge or package lookup error.
        let lookup = self
            .0
            .get_or_try_init(|| async { ApkAsset::from_response(query().await?) });
        // A lost Activity reply must release the settings/shutdown mode guard.
        // The same deadline covers waiting behind another lookup and our own call.
        tokio::time::timeout(std::time::Duration::from_secs(3), lookup)
            .await
            .map_err(|_| "Checking the bundled voice model timed out. Try again.".to_owned())?
            .map(Option::as_ref)
    }
}

pub(super) struct ApkAsset {
    path: PathBuf,
    offset: u64,
    length: u64,
    device: u64,
    inode: u64,
}

impl ApkAsset {
    fn from_response(v: Value) -> Result<Option<Self>, String> {
        match v["available"].as_bool() {
            Some(false) => return Ok(None),
            Some(true) => {}
            None => return Err("Invalid bundled voice availability".into()),
        }
        let number = |field: &str| {
            v[field]
                .as_u64()
                .ok_or("Invalid bundled voice asset identity")
        };
        let path = PathBuf::from(
            v["path"]
                .as_str()
                .ok_or("Missing bundled voice APK location")?,
        );
        let asset = Self {
            path,
            offset: number("offset")?,
            length: number("length")?,
            device: number("device")?,
            inode: number("inode")?,
        };
        if !asset.path.is_absolute() || !(1..=256 * 1024 * 1024).contains(&asset.length) {
            return Err("Invalid bundled voice APK extent".into());
        }
        Ok(Some(asset))
    }

    fn reader(&self) -> Result<Take<File>, String> {
        let mut file = File::options()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
            .map_err(|e| format!("Cannot open the installed voice APK: {e}"))?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file()
            || meta.dev() != self.device
            || meta.ino() != self.inode
            || self
                .offset
                .checked_add(self.length)
                .is_none_or(|end| end > meta.len())
        {
            return Err("The installed voice APK changed. Restart the app and try again.".into());
        }
        file.seek(SeekFrom::Start(self.offset))
            .map_err(|e| e.to_string())?;
        Ok(file.take(self.length))
    }
}

#[cfg(target_os = "android")]
impl Service {
    async fn bundled_asset(&self) -> Result<Option<&ApkAsset>, String> {
        self.bundled_voice
            .locate(|| async {
                let app = self
                    .app
                    .lock()
                    .map_err(|_| "Application handle unavailable")?
                    .clone()
                    .ok_or("Application handle unavailable")?;
                app.kiwix_mobile().call("bundledVoicePack", json!({})).await
            })
            .await
    }

    pub(super) async fn voice_status_with_bundle(&self, mut value: Value) -> Value {
        match self.bundled_asset().await {
            Ok(Some(_)) => {
                value["bundled"] = json!(true);
            }
            Err(error) => {
                value["bundled"] = json!(true);
                value["bundleCheckFailed"] = json!(true);
                value["error"] = json!(error);
            }
            Ok(None) => value["bundled"] = json!(false),
        }
        value
    }

    // The caller holds the mode read guard only for admission. The worker
    // completes its cancellation notification before reacquiring that guard.
    pub(super) async fn start_bundled_voice(
        self: &Arc<Self>,
        explicit: bool,
    ) -> Result<Option<Value>, String> {
        let Some(asset) = self.bundled_asset().await? else {
            return Ok(None);
        };
        self.ensure_running()?;
        self.normal_writable()?;
        let reader = asset.reader()?;
        let manager = self.kokoro.clone();
        let control = if explicit {
            manager.begin_bundled_explicit()?
        } else {
            manager.begin_bundled()?
        };
        let Some(control) = control else {
            return Ok(Some(self.voice_status_with_bundle(manager.status()).await));
        };
        let root = self.store.data_dir().to_owned();
        let generation = self.launch_generation();
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = kokoro::install_bundled(&root, reader, control.clone()).await;
            service.finish_voice_job(&manager, control, generation, result).await;
        });
        Ok(Some(json!({"state": "downloading"})))
    }

    pub(crate) async fn prepare_bundled_voice(self: &Arc<Self>) {
        let _mode = self.privacy_gate.read().await;
        if self.normal_writable().is_ok() {
            if let Err(error) = self.start_bundled_voice(false).await {
                crate::mobile::error(self, "bundled-voice", error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[tokio::test]
    async fn retries_asset_lookup_errors_and_caches_only_confirmed_apk_layouts() {
        for bundled in [false, true] {
            let cache = BundledVoice::default();
            assert!(cache
                .locate(|| async { Err("Unable to read installed assets".into()) })
                .await
                .is_err());
            assert!(cache
                .locate(|| async { Ok(json!({ "available": true })) })
                .await
                .is_err());
            let response = json!({"available":bundled,"path":"/apk/base.apk","offset":2,"length":5,"device":1,"inode":2});
            assert_eq!(
                cache
                    .locate(|| async { Ok(response) })
                    .await
                    .unwrap()
                    .is_some(),
                bundled
            );
            assert_eq!(
                cache
                    .locate(|| async { Err("Unexpected second lookup".into()) })
                    .await
                    .unwrap()
                    .is_some(),
                bundled
            );
        }
    }

    #[tokio::test]
    async fn a_lost_android_reply_releases_waiters_and_permits_retry() {
        let cache = BundledVoice::default();
        let (first, next) = tokio::join!(
            cache.locate(std::future::pending),
            cache.locate(std::future::pending),
        );
        assert!(first.is_err() && next.is_err());
        assert!(cache
            .locate(|| async { Ok(json!({"available":false})) })
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn bounds_the_asset_and_rejects_missing_or_replaced_apks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("split.apk");
        std::fs::write(&path, b"header.model.tail").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let mut asset = ApkAsset {
            path: path.clone(),
            offset: 7,
            length: 5,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let mut retained = asset.reader().unwrap();
        std::fs::rename(&path, dir.path().join("old.apk")).unwrap();
        assert!(asset.reader().is_err());
        std::fs::write(&path, b"header.other.tail").unwrap();
        assert!(asset.reader().is_err());
        let mut bytes = Vec::new();
        retained.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"model");
        std::fs::remove_file(&path).unwrap();
        symlink(dir.path().join("old.apk"), &path).unwrap();
        assert!(asset.reader().is_err());
        asset.path = dir.path().join("old.apk");
        asset.length = u64::MAX;
        assert!(asset.reader().is_err());
    }
}
