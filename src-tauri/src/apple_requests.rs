// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]

use crate::native_pdf::{Authority, Phase};
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::oneshot;

pub(crate) type ResultReceiver = oneshot::Receiver<Result<(), String>>;

pub(crate) struct Pending<Lease> {
    authority: Option<Authority>,
    sender: oneshot::Sender<Result<(), String>>,
    _lease: Lease,
}

impl<Lease> Pending<Lease> {
    pub(crate) fn finish(self, result: Result<(), String>) {
        let Self { authority, sender, _lease } = self;
        drop(authority);
        drop(_lease);
        let _ = sender.send(result);
    }
}

struct State<Lease> {
    last_id: u64,
    pending: HashMap<u64, Pending<Lease>>,
}

impl<Lease> State<Lease> {
    fn authority(&self, id: u64) -> Option<&Authority> {
        let pending = self.pending.get(&id)?;
        if pending.sender.is_closed() {
            return None;
        }
        pending.authority.as_ref()
    }
}

pub(crate) struct Registry<Lease>(Mutex<State<Lease>>);

impl<Lease> Default for Registry<Lease> {
    fn default() -> Self {
        Self(Mutex::new(State { last_id: 0, pending: HashMap::new() }))
    }
}

impl<Lease> Registry<Lease> {
    pub(crate) fn register(&self, authority: Authority, lease: Lease) -> Result<(u64, ResultReceiver), String> {
        let (sender, receiver) = oneshot::channel();
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let id = state.last_id.checked_add(1).ok_or("Apple PDF request IDs exhausted")?;
        state.last_id = id;
        state.pending.insert(id, Pending { authority: Some(authority), sender, _lease: lease });
        Ok((id, receiver))
    }

    pub(crate) fn admitted(&self, id: u64) -> bool {
        let authority = self.0.lock().unwrap_or_else(|error| error.into_inner()).authority(id).cloned();
        if !authority.is_some_and(|authority| authority(Phase::BeforeHandoff).is_ok()) {
            return false;
        }
        self.0.lock().unwrap_or_else(|error| error.into_inner()).authority(id).is_some()
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn is_live(&self, id: u64) -> bool {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).authority(id).is_some()
    }

    pub(crate) fn revoke(&self, id: u64) -> bool {
        // Drop the authority outside the lock. Native completion still owns
        // the resource lease, even if its Rust waiter has gone away.
        let retired = self.0.lock().unwrap_or_else(|error| error.into_inner()).pending
            .get_mut(&id).and_then(|pending| pending.authority.take());
        retired.is_some()
    }

    pub(crate) fn take(&self, id: u64) -> Option<Pending<Lease>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).pending.remove(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_pdf::Files;
    use std::{fs, sync::Arc};

    fn lease() -> Files {
        let directory = tempfile::tempdir().unwrap().keep();
        Files::new(directory, Arc::new(|error| panic!("{error}")), None)
    }

    #[test]
    fn revoked_and_late_callbacks_cannot_affect_a_replacement() {
        let registry = Registry::default();
        let files = lease();
        let path = files.directory().to_path_buf();
        fs::write(path.join("article.pdf"), "owned").unwrap();
        let (first, receiver) = registry.register(Arc::new(|_| Ok(())), files).unwrap();
        assert!(registry.admitted(first));
        assert!(registry.revoke(first));
        assert!(!registry.revoke(first));
        assert!(!registry.admitted(first));
        drop(receiver);
        assert!(path.join("article.pdf").is_file());

        let (second, replacement) = registry.register(Arc::new(|_| Ok(())), lease()).unwrap();
        assert!(second > first);
        drop(registry.take(first).unwrap());
        assert!(!path.exists());
        assert!(registry.take(first).is_none());
        assert!(!registry.admitted(first));
        assert!(registry.admitted(second));
        registry.take(second).unwrap().finish(Ok(()));
        assert!(replacement.blocking_recv().unwrap().is_ok());
    }

    #[test]
    fn closed_waiters_and_policy_denials_fail_admission() {
        let registry = Registry::default();
        let (closed, receiver) = registry.register(Arc::new(|_| panic!("closed waiter evaluated")), lease()).unwrap();
        drop(receiver);
        assert!(!registry.admitted(closed));
        let (denied, _receiver) = registry.register(Arc::new(|_| Err("privacy".into())), lease()).unwrap();
        assert!(!registry.admitted(denied));
    }

    #[test]
    fn revocation_during_policy_evaluation_denies_admission() {
        let registry = Arc::new(Registry::default());
        let weak = Arc::downgrade(&registry);
        let (id, _receiver) = registry.register(Arc::new(move |_| {
            let registry = weak.upgrade().unwrap();
            assert!(registry.revoke(1));
            Ok(())
        }), lease()).unwrap();
        assert_eq!(id, 1);
        assert!(!registry.admitted(id));
    }

    #[test]
    fn exhausted_ids_fail_without_reusing_an_existing_id() {
        let registry = Registry(Mutex::new(State { last_id: u64::MAX - 1, pending: HashMap::new() }));
        let (last, _receiver) = registry.register(Arc::new(|_| Ok(())), lease()).unwrap();
        assert_eq!(last, u64::MAX);
        assert!(registry.register(Arc::new(|_| Ok(())), lease()).is_err());
        assert!(registry.admitted(last));
    }
}
