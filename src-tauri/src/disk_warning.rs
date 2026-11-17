// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Advisory download admission, independent of every transport's hard space guards.
use cap_fs_ext::MetadataExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const THRESHOLD_BYTES: u64 = 1_000_000_000;
pub const WARNING_PREFIX: &str = "LOW_DISK_SPACE:";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceWarning {
    pub acknowledgement: String,
    pub directory: String,
    pub available_bytes: u64,
    pub remaining_bytes: u64,
    pub projected_free_bytes: u64,
    pub threshold_bytes: u64,
}

/// Unknown-length transfers have no projected size: they must keep their normal
/// runtime capacity guards and call this again if authoritative metadata arrives.
#[cfg(test)]
pub fn projected_free(available: u64, remaining: Option<u64>) -> Result<Option<u64>, String> {
    projected_free_with_outstanding(available, 0, remaining)
}

/// Reserve capacity for already-admitted work on the same volume before
/// projecting this request. `outstanding` excludes the current job.
pub(crate) fn projected_free_with_outstanding(
    available: u64,
    outstanding: u64,
    remaining: Option<u64>,
) -> Result<Option<u64>, String> {
    remaining.map(|remaining| {
        available
            .checked_sub(remaining)
            .map(|after| after.saturating_sub(outstanding))
            .ok_or_else(|| "Not enough free space for this download".to_string())
    }).transpose()
}

/// Resolve the actual chosen volume, never the profile directory. Missing final
/// components use their existing ancestor without creating or modifying anything.
fn destination(directory: &Path) -> Result<(PathBuf, PathBuf), String> {
    if !directory.is_absolute() {
        return Err("Download destination must be an absolute directory".into());
    }
    let mut missing = Vec::new();
    for ancestor in directory.ancestors() {
        match std::fs::metadata(ancestor) {
            Ok(metadata) if metadata.is_dir() => {
                let existing = std::fs::canonicalize(ancestor)
                    .map_err(|e| format!("Resolving download volume: {e}"))?;
                let mut resolved = existing.clone();
                for name in missing.iter().rev() { resolved.push(name); }
                return Ok((existing, resolved));
            }
            Ok(_) => return Err("Download destination ancestor is not a directory".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(ancestor.file_name().ok_or("Invalid download destination")?.to_os_string());
            }
            Err(e) => return Err(format!("Inspecting download volume: {e}")),
        }
    }
    Err("Download destination has no accessible filesystem ancestor".into())
}

/// The stable device identity of the volume that currently owns a destination
/// path. Missing final components use their existing ancestor; nothing is created.
pub(crate) fn volume_device(directory: &Path) -> Result<u64, String> {
    let (existing, _) = destination(directory)?;
    // Through an open directory handle, so Windows reports the real volume
    // serial number (std's path metadata has no device identity there).
    cap_std::fs::Dir::open_ambient_dir(&existing, cap_std::ambient_authority())
        .and_then(|directory| directory.dir_metadata())
        .map(|metadata| metadata.dev())
        .map_err(|e| format!("Inspecting download volume: {e}"))
}

/// The acknowledgement is scoped to the exact request/job, destination directory
/// identity and remaining bytes, not a permanent preference or quota bypass. Free
/// space and other jobs' outstanding bytes are intentionally not fingerprinted:
/// they change as unrelated work progresses, and including them would create an
/// endless confirm loop. Insufficiency is always checked anew.
pub fn check(
    directory: &Path,
    remaining: Option<u64>,
    identity: &str,
    acknowledgement: Option<&str>,
) -> Result<(), String> {
    check_with_available_and_outstanding(directory, remaining, 0, identity, acknowledgement, None)
}

/// Same admission policy with an explicit capacity. Production always passes
/// `None`; map admission tests pin the value so a concurrent build's disk use
/// cannot change which branch a deterministic scenario exercises.
pub(crate) fn check_with_available(
    directory: &Path,
    remaining: Option<u64>,
    identity: &str,
    acknowledgement: Option<&str>,
    available_override: Option<u64>,
) -> Result<(), String> {
    check_with_available_and_outstanding(directory, remaining, 0, identity, acknowledgement, available_override)
}

/// Reserve bytes for other admitted jobs on the same volume. The current job's
/// `remaining` is separate from `outstanding`; callers exclude the current ID.
pub(crate) fn check_with_outstanding(
    directory: &Path,
    remaining: Option<u64>,
    outstanding: u64,
    identity: &str,
    acknowledgement: Option<&str>,
) -> Result<(), String> {
    check_with_available_and_outstanding(directory, remaining, outstanding, identity, acknowledgement, None)
}

pub(crate) fn check_with_available_and_outstanding(
    directory: &Path,
    remaining: Option<u64>,
    outstanding: u64,
    identity: &str,
    acknowledgement: Option<&str>,
    available_override: Option<u64>,
) -> Result<(), String> {
    let Some(remaining) = remaining else { return Ok(()); };
    let (existing, resolved) = destination(directory)?;
    let capability = cap_std::fs::Dir::open_ambient_dir(&existing, cap_std::ambient_authority())
        .map_err(|e| format!("Opening download volume: {e}"))?;
    let before = capability.dir_metadata().map_err(|e| format!("Inspecting download volume: {e}"))?;
    let available = match available_override {
        Some(available) => available,
        None => crate::fs_ext::available_space(&existing)
            .map_err(|e| format!("Checking destination free space: {e}"))?,
    };
    let after = cap_std::fs::Dir::open_ambient_dir(&existing, cap_std::ambient_authority())
        .and_then(|directory| directory.dir_metadata())
        .map_err(|e| format!("Rechecking download volume: {e}"))?;
    if before.dev() != after.dev() || before.ino() != after.ino() {
        return Err("Download destination changed while checking its free space".into());
    }
    let projected = projected_free_with_outstanding(available, outstanding, Some(remaining))?
        .expect("known remaining size");
    if projected >= THRESHOLD_BYTES { return Ok(()); }
    let scope = serde_json::to_vec(&(
        identity, &resolved, before.dev(), before.ino(), remaining,
    )).map_err(|e| format!("Encoding download confirmation: {e}"))?;
    let fingerprint = hex::encode(Sha256::digest(scope));
    if acknowledgement == Some(fingerprint.as_str()) { return Ok(()); }
    let warning = SpaceWarning {
        acknowledgement: fingerprint,
        directory: resolved.to_string_lossy().into_owned(),
        available_bytes: available,
        remaining_bytes: remaining,
        projected_free_bytes: projected,
        threshold_bytes: THRESHOLD_BYTES,
    };
    Err(format!("{WARNING_PREFIX}{}", serde_json::to_string(&warning)
        .map_err(|e| format!("Encoding download warning: {e}"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_threshold_is_strict_and_unknown_is_not_zero() {
        assert_eq!(projected_free(1_100_000_000, Some(100_000_000)).unwrap(), Some(THRESHOLD_BYTES));
        assert_eq!(projected_free(1_099_999_999, Some(100_000_000)).unwrap(), Some(THRESHOLD_BYTES - 1));
        assert_eq!(projected_free(1_100_000_001, Some(100_000_000)).unwrap(), Some(THRESHOLD_BYTES + 1));
        assert_eq!(projected_free(0, None).unwrap(), None);
        assert!(projected_free(99, Some(100)).is_err());
        assert!(projected_free_with_outstanding(99, 50, Some(100)).is_err());
        assert_eq!(projected_free(u64::MAX, Some(u64::MAX)).unwrap(), Some(0));
    }

    #[test]
    fn outstanding_reservations_reduce_projection_without_volatile_acknowledgement() {
        assert_eq!(
            projected_free_with_outstanding(3_000_000_000, 1_000_000_000, Some(1_200_000_000)).unwrap(),
            Some(800_000_000)
        );
        let root = std::env::temp_dir().join(format!("kiwix-space-outstanding-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let error = check_with_available_and_outstanding(
            &root, Some(1_200_000_000), 1_000_000_000, "job-one", None, Some(3_000_000_000),
        ).unwrap_err();
        let warning: SpaceWarning =
            serde_json::from_str(error.strip_prefix(WARNING_PREFIX).unwrap()).unwrap();
        assert_eq!(warning.projected_free_bytes, 800_000_000);
        assert!(check_with_available_and_outstanding(
            &root, Some(1_200_000_000), 1_000_000_000, "job-one",
            Some(&warning.acknowledgement), Some(3_000_000_000),
        ).is_ok());
        // Outstanding bytes are volatile as other jobs progress; they change
        // the projection but not the stable request/destination/remaining scope.
        let changed = check_with_available_and_outstanding(
            &root, Some(1_200_000_000), 1_200_000_000, "job-one",
            None, Some(3_000_000_000),
        ).unwrap_err();
        let changed: SpaceWarning =
            serde_json::from_str(changed.strip_prefix(WARNING_PREFIX).unwrap()).unwrap();
        assert_eq!(changed.projected_free_bytes, 600_000_000);
        assert_eq!(changed.acknowledgement, warning.acknowledgement);
        assert!(check_with_available_and_outstanding(
            &root, Some(1_200_000_000), 1_200_000_000, "job-one",
            Some(&warning.acknowledgement), Some(3_000_000_000),
        ).is_ok());
        // The current request still fits, but other reservations exhaust the
        // projection: it must remain an acknowledgeable warning, not a hard
        // error. The per-job hard guard remains separate.
        let zero = check_with_available_and_outstanding(
            &root, Some(1_000_000_000), 1_500_000_000, "job-two",
            None, Some(2_000_000_000),
        ).unwrap_err();
        let zero: SpaceWarning =
            serde_json::from_str(zero.strip_prefix(WARNING_PREFIX).unwrap()).unwrap();
        assert_eq!(zero.projected_free_bytes, 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn actual_destination_and_job_changes_invalidate_confirmation() {
        let root = std::env::temp_dir().join(format!("kiwix-space-{}", uuid::Uuid::new_v4()));
        let first = root.join("chosen");
        let second = root.join("other");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let available = crate::fs_ext::available_space(&first).unwrap();
        let remaining = available.saturating_sub((available / 2).min(500_000_000));
        let error = check(&first, Some(remaining), "job-one", None).unwrap_err();
        let warning: SpaceWarning = serde_json::from_str(error.strip_prefix(WARNING_PREFIX).unwrap()).unwrap();
        assert_eq!(Path::new(&warning.directory), std::fs::canonicalize(&first).unwrap());
        assert!(warning.projected_free_bytes < THRESHOLD_BYTES);
        assert!(check(&first, Some(remaining), "job-one", Some(&warning.acknowledgement)).is_ok());
        assert!(check(&second, Some(remaining), "job-one", Some(&warning.acknowledgement))
            .unwrap_err().starts_with(WARNING_PREFIX));
        assert!(check(&first, Some(remaining), "job-two", Some(&warning.acknowledgement))
            .unwrap_err().starts_with(WARNING_PREFIX));
        assert!(check(&first, Some(u64::MAX), "job-one", Some(&warning.acknowledgement)).is_err());
        let old = root.join("replaced");
        std::fs::rename(&first, &old).unwrap();
        std::fs::create_dir(&first).unwrap();
        assert!(check(&first, Some(remaining), "job-one", Some(&warning.acknowledgement))
            .unwrap_err().starts_with(WARNING_PREFIX));
        std::fs::remove_dir_all(root).unwrap();
    }
}
