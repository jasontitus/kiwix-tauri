# Lane `features` — round 5 report (R14 only)

### FEATURES-R14 — leftover name removed before the captured entry is confirmed as ours
Disposition: fixed.

Verification of the finding: Confirmed by reading `reclaim_owned_file`. With a capture already journaled, the pre-capture ownership check is skipped. The entry was moved into the cleanup folder and its `.kiwix-renamed-<dev>-<ino>` name unlinked before the identity check refused it.

Change: `src-tauri/src/maps/partials.rs`, in the reclaim closure. Right after the captured entry is stat'ed, and before the twin-name block, it must be a regular file with `FileIdentity == record.identity`. Otherwise the reclaim is refused and the entry is put back, with no other name touched. The link-count and size checks stay after the twin block, unchanged.

Tests: `a_leftover_name_of_a_file_that_is_not_ours_is_never_removed` in `maps_tests.rs`, built as you described: a journaled capture, the public name now holding a different file with its own `.kiwix-renamed-<dev>-<ino>` hard link. It asserts `Ok(Refused)`, both names of that file intact with link count 2, and the record's capture still journaled. It passes, and I observed it fail in a private copy with the new check disabled (the twin was gone).

Not exercised: Windows, macOS, iOS, Android; the real link-then-unlink fallback.

## Commands run
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- maps:: kokoro native_pdf` → `test result: ok. 141 passed; 0 failed; 3 ignored`
- Host clippy with `-D warnings` → exit 0, first attempt.
- Private copy, same target directory, check disabled → `FAILED. 74 passed; 1 failed`. The copy and its artifacts are removed.
- Windows-gnu clippy was not re-run; you asked for host only, and the change is platform-neutral.

## Files changed
`src-tauri/src/maps/partials.rs`, `src-tauri/src/maps_tests.rs`. No new strings, and no change to the `map_delete_download` contract.

## Doubts
None new.