// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Device-wide ZIM discovery for Android. Native code reports only display
//! metadata and opaque open tokens; no URI or filesystem path crosses into the
//! UI. Candidates from MediaStore and granted SAF trees are normalized and
//! deduplicated by physical identity so the same file found through two
//! authorities appears once.
//!
//! Identity is the canonical location within a volume plus `(size,
//! modification time in seconds)`. The relative path/name is part of the
//! canonical identity: distinct archives can legitimately share a size and a
//! one-second timestamp (the acceptance fixtures do), while the same physical
//! file reached through MediaStore and a granted folder has the same relative
//! path and name in both sightings. Split archives are grouped by native code
//! and identified by the sum of their parts, so a split set seen through both
//! sources still collapses. Unknown size or modification time is never merged:
//! an entry without a usable identity keeps its own token as the key instead of
//! falsely collapsing unrelated files.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MAX_ENTRIES: usize = 512;
const MAX_LIMITATIONS: usize = 16;
const MAX_NAME: usize = 255;
const MAX_VOLUME: usize = 128;
const MAX_LOCATION: usize = 1024;
const MAX_PROBLEM: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundArchive {
    /// Physical identity, stable across a MediaStore and SAF sighting.
    pub id: String,
    pub name: String,
    pub names: Vec<String>,
    pub size: u64,
    /// Milliseconds since the Unix epoch as reported by the platform.
    pub modified: u64,
    pub volume: String,
    pub location: String,
    pub sources: Vec<String>,
    pub readable: bool,
    pub split: bool,
    /// Opaque native handle for in-place opening; never a path or URI.
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    pub in_library: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanResult {
    pub entries: Vec<FoundArchive>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Candidate {
    token: String,
    name: String,
    names: Vec<String>,
    size: u64,
    modified: u64,
    volume: String,
    location: String,
    sources: Vec<String>,
    readable: bool,
    #[serde(default)]
    problem: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeScan {
    entries: Vec<Candidate>,
    limitations: Vec<String>,
}

/// Validate the opaque native open token. Tokens are canonical UUIDv4 values,
/// never derived from a path, so a compromised UI cannot name a file directly.
pub fn validate_token(token: &str) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(token).map_err(|_| "The file list is out of date. Scan your device again")?;
    if id.get_version_num() != 4 || !id.to_string().eq_ignore_ascii_case(token) {
        return Err("The file list is out of date. Scan your device again".into());
    }
    Ok(())
}

fn clean(value: &str, limit: usize, what: &str) -> Result<String, String> {
    let value = clean_optional(value, limit, what)?;
    if value.is_empty() {
        return Err(format!("Invalid {what} in device scan"));
    }
    Ok(value)
}

/// `clean` for a field that may legitimately be empty: an archive at the top
/// level of a volume or of a granted folder has no location within it.
fn clean_optional(value: &str, limit: usize, what: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(format!("Invalid {what} in device scan"));
    }
    Ok(value.to_owned())
}

fn is_archive_part(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".zim") { return true; }
    let bytes = lower.as_bytes();
    bytes.len() >= 6 && &bytes[bytes.len() - 6..bytes.len() - 2] == b".zim"
        && bytes[bytes.len() - 2].is_ascii_alphabetic()
        && bytes[bytes.len() - 1].is_ascii_alphabetic()
}

fn source_name(source: &str) -> Option<&'static str> {
    match source {
        "media" => Some("media"),
        "storage" => Some("storage"),
        "tree" => Some("tree"),
        _ => None,
    }
}

fn identity(volume: &str, size: u64, modified: u64, location: &str, name: &str) -> Option<String> {
    // A zero size or timestamp cannot distinguish files; native tokens then
    // keep the entries separate instead of merging unrelated archives.
    if size == 0 || modified == 0 {
        return None;
    }
    Some(format!("{volume}:{size}:{}:{location}/{name}", modified / 1000))
}

/// Validate one native candidate. An error rejects only this candidate.
fn admit(candidate: Candidate, library_names: &HashSet<String>) -> Result<FoundArchive, String> {
    let name = clean(&candidate.name, MAX_NAME, "archive name")?;
    if name.starts_with('.') || name.contains('/') || name.contains('\\') {
        return Err("Invalid archive name in device scan".into());
    }
    if candidate.names.len() > 676 {
        return Err("Device scan archive has too many parts".into());
    }
    if candidate.problem.is_none() {
        crate::provider_reference::validate_parts(
            &candidate.names.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
    } else {
        // A broken or incomplete set is reported as unreadable, not as a
        // scan failure: the user still needs to see that it exists.
        for part in &candidate.names {
            let part = clean(part, MAX_NAME, "archive part")?;
            if part.starts_with('.') || part.contains('/') || part.contains('\\')
                || !is_archive_part(&part) {
                return Err("Invalid archive part in device scan".into());
            }
        }
    }
    if candidate.names.first() != Some(&name) {
        return Err("Device scan archive name does not match its parts".into());
    }
    let volume = clean(&candidate.volume, MAX_VOLUME, "volume")?;
    // Empty for an archive at the root of a volume or of a granted folder.
    let location = clean_optional(&candidate.location, MAX_LOCATION, "location")?;
    let mut sources: Vec<String> = candidate
        .sources
        .iter()
        .map(|source| {
            source_name(source)
                .map(str::to_owned)
                .ok_or_else(|| "Unknown device scan source".to_string())
        })
        .collect::<Result<_, _>>()?;
    sources.sort();
    sources.dedup();
    if sources.is_empty() {
        return Err("Device scan entry has no source".into());
    }
    let problem = candidate
        .problem
        .as_deref()
        .map(|problem| clean(problem, MAX_PROBLEM, "problem"))
        .transpose()?;
    let split = candidate.names.len() > 1;
    let key = identity(&volume, candidate.size, candidate.modified, &location, &name)
        .unwrap_or_else(|| format!("token:{}", candidate.token));
    Ok(FoundArchive {
        id: key,
        name: name.clone(),
        names: candidate.names,
        size: candidate.size,
        modified: candidate.modified,
        volume,
        location,
        sources,
        readable: candidate.readable && problem.is_none(),
        split,
        token: candidate.token,
        problem,
        in_library: library_names.contains(&name.to_ascii_lowercase()),
    })
}

/// Validate, normalize and deduplicate a native scan response. `library_names`
/// are the file names already represented in the library (lowercased); they
/// only drive the advisory `inLibrary` flag, never admission.
pub fn scan(raw: Value, library_names: &HashSet<String>) -> Result<ScanResult, String> {
    let native: NativeScan = serde_json::from_value(raw)
        .map_err(|error| format!("Invalid device scan response: {error}"))?;
    if native.entries.len() > MAX_ENTRIES {
        return Err("Device scan returned too many archives".into());
    }
    let mut limitations = Vec::new();
    for limitation in native.limitations {
        if limitations.len() == MAX_LIMITATIONS {
            return Err("Device scan returned too many limitations".into());
        }
        limitations.push(clean(&limitation, 512, "limitation")?);
    }

    let mut entries: Vec<FoundArchive> = Vec::new();
    let mut by_identity: HashMap<String, usize> = HashMap::new();
    let mut tokens = HashSet::new();
    let mut skipped = 0usize;
    for candidate in native.entries {
        validate_token(&candidate.token)?;
        if !tokens.insert(candidate.token.clone()) {
            return Err("The file list is out of date. Scan your device again".into());
        }
        // Names, folders and provider text come from whatever is on the
        // device. One entry that cannot be shown safely is left out and
        // reported; it must not hide every other archive that was found.
        let entry = match admit(candidate, library_names) {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let key = entry.id.clone();
        match by_identity.get(&key).copied() {
            None => {
                by_identity.insert(key, entries.len());
                entries.push(entry);
            }
            Some(index) => merge(&mut entries[index], entry),
        }
    }
    if skipped > 0 {
        limitations.push(if skipped == 1 {
            "1 ZIM file was found but could not be listed, so it was left out.".to_owned()
        } else {
            format!("{skipped} ZIM files were found but could not be listed, so they were left out.")
        });
    }
    entries.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(ScanResult {
        entries,
        limitations,
    })
}

/// One physical file seen through several authorities. Keep the first display
/// identity, union provenance, and prefer a readable SAF-tree token (it carries
/// a persistable grant) over a MediaStore token when the two disagree.
fn merge(current: &mut FoundArchive, other: FoundArchive) {
    let current_tree = current.sources.iter().any(|source| source == "tree");
    let other_tree = other.sources.iter().any(|source| source == "tree");
    // Prefer a readable authority, then a persistable SAF tree, so the token
    // that survives the scan is the one most likely to open durably.
    let prefer_other = (other.readable && !current.readable)
        || (other.readable == current.readable && other_tree && !current_tree);
    let mut sources = current.sources.clone();
    sources.extend(other.sources.iter().cloned());
    sources.sort();
    sources.dedup();
    let in_library = current.in_library || other.in_library;
    if prefer_other {
        *current = other;
    }
    current.sources = sources;
    current.in_library = in_library;
    if current.readable {
        current.problem = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn candidate(token: &str, name: &str, size: u64, modified: u64) -> Value {
        json!({
            "token": token,
            "name": name,
            "names": [name],
            "size": size,
            "modified": modified,
            "volume": "primary",
            "location": "Download/kiwix",
            "sources": ["media"],
            "readable": true
        })
    }

    fn uuid(index: u8) -> String {
        format!("00000000-0000-4000-8000-{index:012}")
    }

    #[test]
    fn deduplicates_the_same_file_across_media_and_tree() {
        let token = uuid(1);
        let tree_token = uuid(2);
        let raw = json!({
            "entries": [
                candidate(&token, "wikipedia.zim", 1024, 1_700_000_000_123),
                {
                    "token": tree_token,
                    "name": "wikipedia.zim",
                    "names": ["wikipedia.zim"],
                    "size": 1024,
                    "modified": 1_700_000_000_999_i64,
                    "volume": "primary",
                    "location": "Download/kiwix",
                    "sources": ["tree"],
                    "readable": true
                }
            ],
            "limitations": []
        });
        let result = scan(raw, &HashSet::new()).unwrap();
        assert_eq!(result.entries.len(), 1);
        let entry = &result.entries[0];
        assert_eq!(entry.sources, vec!["media", "tree"]);
        assert_eq!(entry.token, tree_token, "persistable tree authority wins");
        assert_eq!(entry.id, "primary:1024:1700000000:Download/kiwix/wikipedia.zim");
    }

    #[test]
    fn keeps_distinct_names_with_equal_size_and_timestamp() {
        let raw = json!({
            "entries": [
                candidate(&uuid(9), "notify-a.zim", 36196, 1_700_000_000_500),
                candidate(&uuid(10), "notify-b.zim", 36196, 1_700_000_000_900)
            ],
            "limitations": []
        });
        let result = scan(raw, &HashSet::new()).unwrap();
        assert_eq!(result.entries.len(), 2, "equal size/second must not collapse distinct names");
    }

    #[test]
    fn keeps_distinct_files_with_unknown_identity() {
        let raw = json!({
            "entries": [
                candidate(&uuid(3), "a.zim", 0, 0),
                candidate(&uuid(4), "b.zim", 0, 0)
            ],
            "limitations": []
        });
        let result = scan(raw, &HashSet::new()).unwrap();
        assert_eq!(result.entries.len(), 2);
        assert_ne!(result.entries[0].id, result.entries[1].id);
    }

    #[test]
    fn rejects_unsafe_names_and_tokens() {
        // An unsafe name never becomes an entry; it is counted, not shown.
        let unsafe_name = json!({
            "entries": [candidate(&uuid(5), "../escape.zim", 10, 1)],
            "limitations": []
        });
        let result = scan(unsafe_name, &HashSet::new()).unwrap();
        assert!(result.entries.is_empty());
        assert_eq!(result.limitations.len(), 1);
        assert!(!result.limitations[0].contains("escape"), "{}", result.limitations[0]);
        // Tokens are minted by native code, never taken from the device: a bad
        // or repeated one means the response itself cannot be trusted.
        let unsafe_token = json!({
            "entries": [candidate("not-a-uuid", "ok.zim", 10, 1)],
            "limitations": []
        });
        assert!(scan(unsafe_token, &HashSet::new()).is_err());
        let repeated_token = json!({
            "entries": [candidate(&uuid(5), "a.zim", 10, 1), candidate(&uuid(5), "b.zim", 10, 1)],
            "limitations": []
        });
        assert!(scan(repeated_token, &HashSet::new()).is_err());
    }

    #[test]
    fn an_archive_at_a_volume_root_has_an_empty_location() {
        let mut root = candidate(&uuid(11), "wikipedia.zim", 1024, 1_700_000_000_123);
        root["location"] = json!("");
        let mut tree = candidate(&uuid(12), "wikipedia.zim", 1024, 1_700_000_000_456);
        tree["location"] = json!("  ");
        tree["sources"] = json!(["tree"]);
        let raw = json!({ "entries": [root, tree], "limitations": [] });
        let result = scan(raw, &HashSet::new()).unwrap();
        assert!(result.limitations.is_empty(), "{:?}", result.limitations);
        // Both sightings of the root-level file still collapse into one entry.
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].location, "");
        assert_eq!(result.entries[0].sources, vec!["media", "tree"]);
        assert_eq!(result.entries[0].id, "primary:1024:1700000000:/wikipedia.zim");
        // The location is still bounded and may not carry control characters.
        for location in ["Download\u{0}kiwix".to_owned(), "x".repeat(MAX_LOCATION + 1)] {
            let mut unsafe_location = candidate(&uuid(13), "other.zim", 10, 1);
            unsafe_location["location"] = json!(location);
            let result = scan(json!({ "entries": [unsafe_location], "limitations": [] }), &HashSet::new()).unwrap();
            assert!(result.entries.is_empty());
            assert_eq!(result.limitations.len(), 1);
        }
    }

    #[test]
    fn one_unusable_candidate_does_not_hide_the_others() {
        let mut no_volume = candidate(&uuid(16), "volume.zim", 10, 1);
        no_volume["volume"] = json!("");
        let raw = json!({
            "entries": [
                candidate(&uuid(14), "good.zim", 10, 1_000),
                // A granted folder can list a hidden trashed copy.
                candidate(&uuid(15), ".trashed-1700000000-old.zim", 10, 2_000),
                no_volume,
                candidate(&uuid(17), "also-good.zim", 20, 3_000)
            ],
            "limitations": ["Android 13 and later requires folder access"]
        });
        let result = scan(raw, &HashSet::new()).unwrap();
        let names: Vec<_> = result.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["also-good.zim", "good.zim"]);
        // One summary line for everything that was left out, after the
        // platform's own limitations; no rejected text is repeated in it.
        assert_eq!(result.limitations.len(), 2);
        assert!(result.limitations[1].starts_with("2 ZIM files were found but could not be listed"), "{}", result.limitations[1]);
        assert!(!result.limitations[1].contains("trashed"));
    }

    #[test]
    fn reports_incomplete_split_parts_as_unreadable() {
        let raw = json!({
            "entries": [{
                "token": uuid(6),
                "name": "split.zimaa",
                "names": ["split.zimaa", "split.zimac"],
                "size": 10,
                "modified": 1,
                "volume": "primary",
                "location": "Download",
                "sources": ["tree"],
                "readable": true,
                "problem": "Split archive is incomplete or has extra parts; import a copy."
            }],
            "limitations": []
        });
        let result = scan(raw, &HashSet::new()).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert!(!result.entries[0].readable);
        assert!(result.entries[0].problem.is_some());
    }

    #[test]
    fn rejects_invalid_split_names_even_when_unreadable() {
        let raw = json!({
            "entries": [{
                "token": uuid(6),
                "name": "split.zimaa",
                "names": ["../split.zimaa"],
                "size": 10,
                "modified": 1,
                "volume": "primary",
                "location": "Download",
                "sources": ["tree"],
                "readable": false,
                "problem": "broken"
            }],
            "limitations": []
        });
        let result = scan(raw, &HashSet::new()).unwrap();
        assert!(result.entries.is_empty());
        assert_eq!(result.limitations.len(), 1);
    }

    #[test]
    fn marks_archives_already_in_the_library() {
        let library: HashSet<String> = ["already.zim".into()].into_iter().collect();
        let raw = json!({
            "entries": [candidate(&uuid(7), "Already.zim", 10, 1)],
            "limitations": ["Android 13 and later requires folder access"]
        });
        let result = scan(raw, &library).unwrap();
        assert!(result.entries[0].in_library);
        assert_eq!(result.limitations.len(), 1);
    }

    #[test]
    fn unreadable_candidate_keeps_its_problem() {
        let raw = json!({
            "entries": [{
                "token": uuid(8),
                "name": "stream.zim",
                "names": ["stream.zim"],
                "size": 10,
                "modified": 1,
                "volume": "primary",
                "location": "Download",
                "sources": ["tree"],
                "readable": false,
                "problem": "Provider exposes a stream"
            }],
            "limitations": []
        });
        let result = scan(raw, &HashSet::new()).unwrap();
        assert!(!result.entries[0].readable);
        assert_eq!(result.entries[0].problem.as_deref(), Some("Provider exposes a stream"));
    }
}
