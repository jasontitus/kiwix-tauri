// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Shared exclusion between publishing new transfer/migration work and an
//! updater's final active-work check. Acquire admission before privacy and
//! transfer locks, never recursively. Hold a read lease until worker/status
//! publication (or the entire migration); hold the install lease until failure
//! or process restart. Read-only UI commands do not acquire this gate.

use std::sync::Arc;

#[derive(Clone, Default)]
pub(crate) struct AdmissionGate(Arc<tokio::sync::RwLock<()>>);

impl AdmissionGate {
    pub(crate) async fn admit(&self) -> tokio::sync::OwnedRwLockReadGuard<()> {
        self.0.clone().read_owned().await
    }

    #[cfg(desktop)]
    pub(crate) async fn install(&self) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.0.clone().write_owned().await
    }
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn install_excludes_admission_until_failure_releases_lease() {
        let gate = AdmissionGate::default();
        let installation = gate.install().await;
        let admission = gate.admit();
        tokio::pin!(admission);
        assert!(matches!(futures_util::poll!(&mut admission), std::task::Poll::Pending));
        drop(installation);
        let _admitted = admission.await;
    }

    #[tokio::test]
    async fn install_waits_for_worker_publication_and_blocks_later_starts() {
        let gate = AdmissionGate::default();
        let published = AtomicBool::new(false);
        let first_start = gate.admit().await;
        let installation = gate.install();
        tokio::pin!(installation);
        assert!(matches!(futures_util::poll!(&mut installation), std::task::Poll::Pending));
        let later_start = gate.admit();
        tokio::pin!(later_start);
        assert!(matches!(futures_util::poll!(&mut later_start), std::task::Poll::Pending));
        published.store(true, Ordering::SeqCst);
        drop(first_start);
        let installation = installation.await;
        assert!(published.load(Ordering::SeqCst));
        assert!(matches!(futures_util::poll!(&mut later_start), std::task::Poll::Pending));
        drop(installation);
        let _later_start = later_start.await;
    }
}
