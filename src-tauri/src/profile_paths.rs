// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Pure schema inventory shared by mobile relocation and desktop portability.
//! Archive-content paths, provider URIs, identity evidence and relative SDK names
//! are deliberately not filesystem paths and must never be recursively rewritten.
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
pub enum RecordKind { State, Downloads, Torrents, LocalSwarm, Maps, Migration }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PathRole { Owned, ExplicitSource }

pub fn rewrite(
    kind: RecordKind,
    value: &mut Value,
    include_unmanaged: bool,
    mut path: impl FnMut(&mut Value, PathRole) -> Result<bool, String>,
) -> Result<bool, String> {
    fn field(value: &mut Value, key: &str, path: &mut impl FnMut(&mut Value, PathRole) -> Result<bool, String>) -> Result<bool, String> {
        let role = if key == "source_directory" { PathRole::ExplicitSource } else { PathRole::Owned };
        match value.get_mut(key) { Some(value) => path(value, role), None => Ok(false) }
    }
    fn array(value: &mut Value, key: &str, path: &mut impl FnMut(&mut Value, PathRole) -> Result<bool, String>) -> Result<bool, String> {
        let Some(values) = value.get_mut(key) else { return Ok(false); };
        let mut changed = false;
        for value in values.as_array_mut().ok_or("Persisted path list is not an array")? { changed |= path(value, PathRole::Owned)?; }
        Ok(changed)
    }
    fn job(value: &mut Value, path: &mut impl FnMut(&mut Value, PathRole) -> Result<bool, String>) -> Result<bool, String> {
        let changed = field(value, "destination", path)?;
        Ok(array(value, "completedPaths", path)? || changed)
    }
    let mut changed = false;
    match kind {
        RecordKind::State => {
            if let Some(books) = value.get_mut("books").and_then(Value::as_array_mut) {
                for book in books {
                    // Provider markers are opaque native authority, never relative
                    // or absolute filesystem paths to migrate with the profile.
                    if book.get("sourceReference").is_some_and(|source| !source.is_null()) { continue; }
                    if include_unmanaged || book.get("managed").and_then(Value::as_bool) == Some(true) {
                        changed |= field(book, "path", &mut path)?;
                    }
                }
            }
            // Edition records retain predecessors and unregistered verified
            // output. Walk only known filesystem fields: provider references,
            // content identities and archive article locations stay opaque.
            for collection in ["editionIntents", "editionTransactions"] {
                if let Some(records) = value.get_mut(collection) {
                    for record in records.as_array_mut().ok_or("Saved edition records are not an array")? {
                        if collection == "editionIntents" {
                            changed |= field(record, "destination", &mut path)?;
                            changed |= field(record, "downloadDirectory", &mut path)?;
                        }
                        for key in ["target", "verified", "original", "replacement", "rollbackBook"] {
                            if let Some(book) = record.get_mut(key).filter(|book| !book.is_null()) {
                                if book.get("sourceReference").is_some_and(|source| !source.is_null()) { continue; }
                                if include_unmanaged || book.get("managed").and_then(Value::as_bool) == Some(true) {
                                    changed |= field(book, "path", &mut path)?;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(settings) = value.get_mut("settings") {
                changed |= field(settings, "downloadDirectory", &mut path)?;
                changed |= array(settings, "monitorDirectories", &mut path)?;
                changed |= field(settings, "lastImportDirectory", &mut path)?;
                changed |= field(settings, "lastSaveDirectory", &mut path)?;
            }
        }
        RecordKind::Downloads => {
            for row in value.as_object_mut().ok_or("Download state is not an object")?.values_mut() {
                changed |= field(row, "destination", &mut path)?;
            }
        }
        RecordKind::Torrents => {
            for row in value.get_mut("jobs").and_then(Value::as_array_mut).ok_or("Torrent state has no jobs array")? {
                for key in ["staging", "directory", "metadata"] { changed |= field(row, key, &mut path)?; }
                if let Some(value) = row.get_mut("job") { changed |= job(value, &mut path)?; }
                // A share pins the library archive it seeds. Its path is the same
                // absolute filesystem path as job.destination and must move with
                // the profile; identity evidence stays untouched.
                if let Some(share) = row.get_mut("share") { changed |= field(share, "path", &mut path)?; }
            }
        }
        RecordKind::LocalSwarm => {
            for row in value.as_object_mut().ok_or("LocalSwarm state is not an object")?.values_mut() {
                if let Some(value) = row.get_mut("job") { changed |= job(value, &mut path)?; }
                if let Some(owned) = row.get_mut("owned") { changed |= field(owned, "parent", &mut path)?; }
            }
        }
        RecordKind::Maps => {
            if !value.is_null() {
                changed |= field(value, "path", &mut path)?;
                changed |= array(value, "monitoredPaths", &mut path)?;
                if let Some(decision) = value.get_mut("decision").filter(|value| !value.is_null()) {
                    changed |= field(decision, "destination", &mut path)?;
                    if let Some(target) = decision.get_mut("target").filter(|value| !value.is_null()) {
                        if !target.get("sourceReference").is_some_and(|source| !source.is_null())
                            && (include_unmanaged || target.get("managed").and_then(Value::as_bool) == Some(true)) {
                            changed |= field(target, "path", &mut path)?;
                        }
                    }
                }
                if let Some(published) = value.get_mut("published").and_then(|value| value.get_mut("record")) {
                    changed |= field(published, "destination", &mut path)?;
                }
                if let Some(records) = value.get_mut("retainedPublications") {
                    for saved in records.as_array_mut().ok_or("Retained map publications are not an array")? {
                        if let Some(record) = saved.get_mut("published").and_then(|value| value.get_mut("record")) {
                            changed |= field(record, "destination", &mut path)?;
                        }
                    }
                }
                if let Some(owned) = value.get_mut("ownedPartial").filter(|value| !value.is_null()) { changed |= field(owned, "destination", &mut path)?; }
                if let Some(records) = value.get_mut("recoveryRecords") {
                    for owned in records.as_array_mut().ok_or("A saved map download could not be read. Restart the app")? { changed |= field(owned, "destination", &mut path)?; }
                }
            }
        }
        RecordKind::Migration => {
            changed |= field(value, "source_directory", &mut path)?;
            changed |= field(value, "destination_directory", &mut path)?;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn edition_paths_rebase_retained_sources_without_rewriting_provider_or_identity() {
        let mut value = serde_json::json!({
            "editionIntents": [{
                "destination": "/old/downloads/edition.zim", "downloadDirectory": "/old/downloads",
                "target": {"path": "/old/original.zimaa", "managed": true},
                "verified": {"path": "/old/downloads/edition.zim", "managed": true},
                "originalIdentity": [{"inode": 7, "modifiedNs": "123"}]
            }],
            "editionTransactions": [{
                "original": {"path": "/old/original.zimaa", "managed": true},
                "replacement": {"path": "/old/downloads/edition.zim", "managed": true},
                "rollbackBook": {"path": "provider://opaque", "managed": true, "sourceReference": {"reference": "/old/opaque-provider-token"}}
            }]
        });
        let identity = value["editionIntents"][0]["originalIdentity"].clone();
        let provider = value["editionTransactions"][0]["rollbackBook"].clone();
        assert!(rewrite(RecordKind::State, &mut value, false, |path, _| {
            let text = path.as_str().unwrap();
            *path = text.replacen("/old/", "/new/", 1).into();
            Ok(true)
        }).unwrap());
        assert_eq!(value["editionIntents"][0]["destination"], "/new/downloads/edition.zim");
        assert_eq!(value["editionTransactions"][0]["original"]["path"], "/new/original.zimaa");
        assert_eq!(value["editionIntents"][0]["originalIdentity"], identity);
        assert_eq!(value["editionTransactions"][0]["rollbackBook"], provider);
    }
    #[test]
    fn retained_map_rebase_moves_only_owned_destinations_and_preserves_proofs() {
        let mut value = serde_json::json!({
            "retainedPublications": [{
                "id": "original-job", "regionId": "region", "language": "en",
                "published": {
                    "record": {
                        "destination": "/old/archives",
                        "directoryIdentity": {"device": 7, "inode": 8},
                        "name": "download.zim",
                        "identity": {"device": 7, "inode": 9},
                        "capture": {"name": ".streetzim-cleanup-exact", "identity": {"device": 7, "inode": 10}, "size": 11}
                    },
                    "size": 11, "sha256": [1, 2, 3]
                }
            }]
        });
        let original = value.clone();
        assert!(rewrite(RecordKind::Maps, &mut value, false, |path, role| {
            assert!(matches!(role, PathRole::Owned));
            assert_eq!(path.as_str(), Some("/old/archives"));
            *path = Value::String("/new/archives".into());
            Ok(true)
        }).unwrap());
        assert_eq!(value["retainedPublications"][0]["published"]["record"]["destination"], "/new/archives");
        value["retainedPublications"][0]["published"]["record"]["destination"] = "/old/archives".into();
        assert_eq!(value, original);
    }

    #[test]
    fn torrent_share_path_moves_with_the_profile_and_keeps_identity_evidence() {
        let mut value = serde_json::json!({
            "jobs": [{
                "directory": "/old/archives",
                "staging": "/old/archives/.kiwix-torrent-id",
                "metadata": "/old/data/torrents/id.torrent",
                "job": {"id": "torrent:id", "destination": "/old/archives/book.zim"},
                "share": {
                    "bookId": "book",
                    "installationId": "installation",
                    "path": "/old/archives/book.zim",
                    "identity": {"device": 1, "inode": 2, "size": 3, "modifiedNs": 4}
                }
            }]
        });
        let evidence = value["jobs"][0]["share"]["identity"].clone();
        assert!(rewrite(RecordKind::Torrents, &mut value, false, |path, role| {
            assert!(matches!(role, PathRole::Owned));
            let text = path.as_str().unwrap();
            if let Some(suffix) = text.strip_prefix("/old") {
                *path = Value::String(format!("/new{suffix}"));
                return Ok(true);
            }
            Ok(false)
        }).unwrap());
        assert_eq!(value["jobs"][0]["share"]["path"], "/new/archives/book.zim");
        assert_eq!(value["jobs"][0]["job"]["destination"], "/new/archives/book.zim");
        assert_eq!(value["jobs"][0]["directory"], "/new/archives");
        assert_eq!(value["jobs"][0]["share"]["identity"], evidence);
    }

    #[test]
    fn malformed_retained_map_path_inventory_fails_closed() {
        let mut value = serde_json::json!({"retainedPublications": {}});
        assert!(rewrite(RecordKind::Maps, &mut value, false, |_, _| Ok(false)).is_err());
    }
}
