## Verdict for lane features (round 3, maps paths): only non-blocking findings

R8, R9 and R10 are closed. One new Should-fix remains: the "could not be removed just now" class also swallows cases where Kiwix declined on ownership grounds, so Delete can fail on every press with advice that cannot work. Lane tests pass and the shared build is intact.

### FEATURES-R11 — Ownership refusals are classified as "busy", so Delete fails forever with the wrong advice
Severity: Should-fix (narrow triggers)
Fix under review: R9 (`Kept`), R8 ("never `Err` forever")
Evidence class: source-traced
Location: `src-tauri/src/maps/partials.rs:622-635` (`Kept::of`, `Kept::retryable`); `src-tauri/src/maps/manager.rs:843` and `:855` (`retry_recovery_records`)
Trigger: A kept record whose journaled folder is intact, where the reclaim returns one of its refusal strings instead of `Ok(Preserved)`:
- "Map publication remains unresolved…; all entries were preserved" (`partials.rs:386-394`): the record carries a publication name, the partial is gone, and a different file sits at the final `.zim` name.
- "Map cleanup capability disappeared…" or "…was replaced; nothing was removed" (`:457`, `:465`): for example the user removed the empty hidden `.streetzim-cleanup-<uuid>` folder left by an earlier failed capture.
- "…has an unexpected name" or "Invalid saved map publication filename" (`:318`, `:349`): a damaged journal.

Causal path:
- `Kept::of(record, false)` returns `Busy` for every `Err` once the folder opens and matches. `Busy.retryable()` is true, so `failed` is set and `delete_download` returns `Err` on every press.
- The sentence says "another program may be using it. Close anything that has it open, then press Delete again". None of these states can be fixed that way; only Dismiss ends it.
- In the publication case `recovery_path` is the final `.zim` name. The `Err` then calls a file that is not Kiwix's "a partial map download … kept at <map>.zim", which may be a good installed map.
- Round 2 had the same permanent `Err` with the raw text; round 3 made the wording misleading.

Smallest correct fix: make refusals distinguishable without matching text. Either return `Ok(PartialRecovery::Preserved)` from those branches of `reclaim_owned_file`, or give its error a `refused` flag. Classify them `Unconfirmed` (report-only, Delete returns `Ok`). I checked the other callers of the `Preserved` route: `new()`, `start()`, `reclaim_published_map` and `OwnedPartialGuard` all treat anything other than `Reclaimed` as "keep".

### FEATURES-R12 — Two sentences promise more than the code can know
Severity: Nit
Fix under review: R9
Evidence class: source-traced
Location: `partials.rs:638-650` (`Kept::sentence`)
- `Unavailable` is any failure to open the destination. "Reconnect the drive that holds it" is wrong when the folder was deleted or moved on an internal disk (FEAT-1's original trigger), or when access is denied. That state is permanent and Delete keeps returning `Err`. Suggested: "Its folder cannot be opened right now. If it is on a drive that is not connected, reconnect it and press Delete again. If the folder is gone, dismiss this warning."
- `Busy` also covers a full disk (creating the cleanup folder fails) and a journal that cannot be saved. "Another program may be using it" then points the wrong way. Suggested: drop the cause and keep "It could not be removed just now. Press Delete again; if it keeps failing, free some space or close programs using that folder."

### FEATURES-R13 — Display of the new text (for the panes lane)
Severity: Nit
Evidence class: reproduced for the iOS part (I ran the `UI.tsx` redaction regexes in Node against the four sentences); source-traced for the note
Location: `src/components/UI.tsx:40-44, 249-253`; `src/components/MapBlockProgress.tsx:59`
- On iOS the path redaction swallows the start of the reason and leaves a stray bracket: "…kept here: [File] is not available right now. Reconnect the drive that holds it, then press Delete again), [File]". The `Err` form reads "kept at [File] is not available right now…". Readable, but untidy.
- The list's note says Kiwix kept the files "because it could not prove they were its own to delete. Remove them yourself…". Under round 3 some entries are proven and removable by pressing Delete again, and the alert above says so. The two texts disagree for those entries.

**Earlier findings**
- R8: closed. `record_vanished` returns false unless the opened directory has the journaled identity. A mismatch is `FolderReplaced`: not retryable, so never `Err` after the demoting call, and never dropped. Both folder tests pin `Err` once, then `Ok` with the record still listed, then removal once the real folder is back.
- R9: closed. No OS text or identity reaches `job.error` or the `Err`; each reason sits beside its own path. Residue is R11 and R12.
- R10: closed. The carried-leftover test exists and passes; leaving the `status()` probe is acceptable.

**Your five questions**
1. **R8.** Yes on both counts, as above. One more drop path exists and is correct: a never-proven record is dropped only when the journaled folder itself holds none of its names.
2. **R9 sentences.** `FolderReplaced` and `Unconfirmed` are accurate. `Unavailable` and `Busy` are covered in R12; `Busy` also covers refusals (R11). The implementer's two doubts are both real but small.
3. **stderr.** Not a privacy finding inside the app.
   - `crash.rs` writes only the panic message, location and backtrace (`capture_panic`, `:193-217`).
   - `diagnostics.rs` stores a category derived from the error and exports only time, level, category and a whitelisted operation name (`export_events`, `:21-37`).
   - Nothing in `src-tauri` redirects or captures stderr.
   - Existing `eprintln!` calls already print download and library paths (`localswarm/mod.rs:1648`, `store.rs:1827`, `sandbox.rs:294`).
   - Not verified: where each operating system sends an app's stderr.
4. **Frontend.** Works with the round-3 replies.
   - `Ok` with the job still listed: `Maps.tsx` re-reads `map_status` and `Downloads.tsx` calls `load()`, so the card stays with its Dismiss buttons.
   - `Err`: the dialog shows the text and the job refreshes; a second press in the same dialog is accepted once the job is already `cancelled`.
   - Nothing in `src/` parses the warning sentence, so the parenthesised reason is only displayed.
   - `recoveryPaths` strings pass through unchanged to `map_forget_recovery`.
   - Cosmetic issues are in R13.
5. **Build directory.** The shared target builds and the lane tests ran on the first attempt.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- maps:: kokoro native_pdf`: `test result: ok. 135 passed; 0 failed; 3 ignored; 0 measured; 1355 filtered out`.
- A Node script in my scratch directory applying the `UI.tsx` redaction regexes to the new messages.

**What I could not verify**
- Anything on Windows, macOS, iOS or Android.
- Whether a directory's identity survives unplugging and replugging real removable media. On filesystems where it does not, a reconnected drive would read as "moved or replaced" and the partial would stay report-only, despite the "Reconnect the drive… press Delete again" advice. That limit comes from the existing identity design, not from this round.
- The implementer's fail-without-fix runs and clippy results.