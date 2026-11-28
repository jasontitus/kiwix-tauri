## Verdict for lane features (round 4): no blocking findings; the lane is closed

R11, R12, the twin-name handoff and FEAT-9 are closed. One Nit is new. Lane tests pass.

### FEATURES-R14 — The leftover name is removed before the captured entry is confirmed as ours
Severity: Nit
Fix under review: twin-name handoff, step 2
Evidence class: source-traced
Location: `src-tauri/src/maps/partials.rs:500-519` (twin handling) runs before `:522-537` (identity, size and link-count check)
Trigger: A record that already has a journaled capture, whose public name was later replaced by a different file that itself has a `.kiwix-renamed-<its device>-<its inode>` second name in the same folder.
Causal path: With a capture journaled, the pre-capture ownership check is skipped. The file is moved into the cleanup folder, step 2 unlinks its `.kiwix-renamed-…` name, and only then does the identity check refuse it and put it back. No bytes are lost, because the file keeps its main name, and the removed name can only be one Kiwix made for that same file. It is still an unlink performed for an entry the code then declares unconfirmed.
Smallest correct fix: check `is_file` and `FileIdentity == record.identity` on the captured entry before the twin block. Leave the link-count and size checks after it.

**Your four focus areas**

1. **`PartialRecovery::Refused`: closed.**
   - Every match site treats it as "keep":
     - `OwnedPartialGuard` (`compose.rs:47`) and `start()` (`manager.rs:474`) act only on `Reclaimed`.
     - `new()` (`manager.rs:161`) keeps its old error text.
     - `reclaim_published_map` returns the carried text as `Err`, as before.
     - `reclaim_job_partial` and `retry_recovery_records` classify it as declined.
   - A refusal cannot lead to a deletion of the entry. Six of the seven returns come before any capture; the seventh returns before the unlink and restores the entry. The only unlink that can precede a refusal is the leftover-name one in R14.
   - A refused record cannot be dropped while bytes remain. `record_vanished` still needs the journaled folder to hold none of the partial, publication and cleanup-folder names, and every refusal with bytes present leaves at least one of them.
   - The extra classification (post-capture "changed identity or size") is right. It fails identically on every attempt, and the record is still retried on each Delete, so removing a stray hard link lets a later Delete finish.
   - The Delete error for a file at the map's final name is now "Kiwix did not remove <path>…", with no "partial map download".

2. **Twin-name healing: closed.**
   - Name format matches `exclusive_rename_fallback` (`publication.rs:759`): `.kiwix-renamed-{device:x}-{inode:x}` of the linked file, same widths and lowercase hex.
   - The `-1` suffix applies only when the source or destination name equals the private name. That cannot happen here: names are `.streetzim-<uuid>.part` or a non-dot `.zim`.
   - The same-entry guard does not apply; source and destination are in different folders.
   - Step 1, traced against the real fallback: link fails with "exists", the destination is the same file, the old name is renamed aside and unlinked. Correct.
   - Step 2 unlinks only a regular file at that exact name with the same device and inode as the captured entry. It is check-then-unlink on a private name, the same pattern the primitive uses for its own leftover (`publication.rs:765-771`). The bystander `.kiwix-renamed-0-0` test covers a different file.
   - Step 3 requires a link count of 1 before any unlink, published or not.
   - The pre-capture `nlink != 1` check does not affect normal files. Resume already required a link count of 1 (`open_map_checkpoint`). An interrupted capture always has its capture journaled first, so it skips this check and reaches the healing.

3. **R12 wording: closed.** The Dismiss control is rendered by `MapRecoveryPaths` in `MapBlockProgress.tsx`, used by both `Maps.tsx:993` and `Downloads.tsx:1208`, outside the disk-space branch. Two places show the sentence without it:
   - the Delete dialog, where the user closes the dialog and finds Dismiss on the card;
   - private or read-only mode, where `recoveryPaths` is absent and Delete is disabled too.

4. **FEAT-9: closed.** `useVoiceModelStatus.ts:64` is the only listener and refreshes on any state other than "downloading". Nothing in `src/`, the plugins or scripts matches on "error". The `service.rs` change is the two lines at `:3781-3782`. The emitted state has no test.

**Residuals for the lane**
- R14 above.
- Step 1 on a filesystem that does have an exclusive rename returns "file exists", so Delete fails on each press. That state needs a hand-made hard link; the implementer flagged it.
- A crash during the publication move on a filesystem without exclusive rename leaves the partial name and the final `.zim` as one file with two names. Nothing heals it: Resume refuses and Delete reports it as unconfirmed. No bytes are lost. This became reachable with the files lane's fallback.
- A leftover name inside the cleanup folder (interrupted put-back) is healed only as a side effect of the next put-back, so it takes two Deletes.
- `partials.rs` duplicates the `.kiwix-renamed-…` format from `publication.rs`. If that changes, step 2 stops matching and the entry is refused, not leaked. A shared helper would remove the coupling.
- On iOS the path redaction in `UI.tsx` still garbles the parenthesised reason (R13, panes lane).
- `status()` still stats the capture path for records with a capture.
- Not exercised anywhere: the real link-then-unlink fallback, Windows, macOS, iOS, Android, real removable media, the Kokoro `run` loop with a model, and FEAT-8 (never compiled).

**Earlier handoffs:**
- R2: the startup sweep is now called from `service.rs:2665`; I did not review that call.
- R7: both doc lines are updated.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- maps:: kokoro native_pdf`: first attempt, `test result: ok. 140 passed; 0 failed; 3 ignored; 0 measured; 1365 filtered out`.

**What I could not verify**
- Step 1 against the real primitive; its test uses an injected capture that models it.
- The implementer's clippy and fail-without-fix runs.