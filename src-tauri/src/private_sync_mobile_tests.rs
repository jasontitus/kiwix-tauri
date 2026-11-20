// SPDX-License-Identifier: GPL-3.0-or-later
//! Runs on native mobile test targets: real lifecycle state, no transport mocks.
#![forbid(unsafe_code)]
use super::*;
struct Profile(PathBuf);
impl Profile { fn new() -> Self { Self(std::env::temp_dir().join(format!("kiwix-private-mobile-{}", Uuid::new_v4()))) } }
impl Drop for Profile { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
#[tokio::test]
async fn queued_private_mutations_recheck_inactive_without_generation_change() {
    for command in ["private_sync_configure", "private_sync_rebootstrap"] {
        let profile = Profile::new(); let service = Service::new(profile.0.clone(), false, false).unwrap();
        assert!(service.mobile_active());
        let before = service.store.read().unwrap().reading_sync;
        let generation = service.privacy_generation.load(Ordering::SeqCst);
        let held = service.privacy_gate.read().await;
        let operation = service.clone();
        let queued = tokio::spawn(async move { operation.dispatch(command, json!({"privacyGeneration":generation,"enabled":true,"categories":{"bookmarks":true,"positions":true},"confirmed":true})).await });
        tokio::time::timeout(Duration::from_secs(3), async {
            loop { if service.privacy_gate.try_read().is_err() { break; } tokio::task::yield_now().await; }
        }).await.unwrap();
        service.mobile_note_inactive();
        assert_eq!(service.privacy_generation.load(Ordering::SeqCst), generation);
        drop(held);
        assert!(queued.await.unwrap().is_err());
        assert_eq!(service.store.read().unwrap().reading_sync, before);
        assert!(!profile.0.join("private-reading-sync-v1").exists());
    }
}
