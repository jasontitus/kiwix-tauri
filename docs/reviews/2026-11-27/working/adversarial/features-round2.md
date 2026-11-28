## Verdict for lane features (round 2, maps paths): only non-blocking findings

R1 is closed: nothing unproven can be unlinked, the demotion is now reversible, and Delete is honest about a failed removal. One place still draws a conclusion without the ownership proof (`record_vanished`); it only drops a record, never a file, and needs your decision. Lane tests pass here.

### FEATURES-R8 — `record_vanished` drops a proven record on path-only evidence, and Delete then reports everything gone
Severity: Should-fix (owner decision)
Fix under review: R1 / R5
Evidence class: reproduced for the drop (the lane's own test `deleting_a_map_whose_folder_was_replaced_says_so_once_and_reports_nothing_that_is_not_there` asserts `status()` is `None` while `fs::read(&moved) == prefix`; I ran it); the unmounted-volume trigger is inference
Location: `src-tauri/src/maps/partials.rs:589-596` (`record_vanished`), used at `src-tauri/src/maps/manager.rs:801` onward (`retry_recovery_records`, arms `Some(Err(_))` and `Some(Ok(_)) | None`)
Trigger: A kept record whose destination path now resolves to a different directory that lacks the record's names, then Delete. Two ways to get there:
- The user moved the folder and made a new one (benign; the user knows where the bytes are).
- The destination is a mount point whose directory persists when the volume is absent, or it sits under one and was recreated on the local disk by `create_dir_all(destination)` when another map was started there (`manager.rs`, `compose`). The proven multi-GB partial is on the absent volume.

Causal path:
- `record_vanished` opens `record.destination` and checks only that the names are absent. It never compares `directory.identity` with `record.directory_identity`.
- `reclaim_owned_file` makes its own "every file it named is gone" conclusion only after that identity check (`partials.rs:320-339`), so this is a weaker test than the one it imitates.
- The record is removed, `recovery_records` becomes empty, `history_dismissed` is set, and `delete_download` returns `Ok` with `map_status` null. The documented contract for that result is "everything is gone".
- If the record was demoted earlier by an unattended `cancel()`, this Delete shows no message at all.
- A dropped record with a publication name also gives up its monitored-path claim. That matters only in a very narrow crash-plus-unmount sequence.

This traces to my own R5 wording ("when the destination opens and none of the record's names exist, drop the record"), which omitted the identity precondition. The implementer followed it and flagged the doubt.

Smallest correct fix: require `directory.identity == record.directory_identity` in `record_vanished`. Pair it with treating a directory-identity mismatch as report-only in `retry_recovery_records` (like `Preserved`: kept, not `failed`), so Delete returns `Ok` with the warning instead of `Err` forever.

Trade-off for your decision: the path-only rule is the better outcome after a phone restore or migration, where the backup-excluded partial really is gone and the directory identity never matches again. The strict rule leaves a stale warning there until Dismiss exists (R3). The strict rule never forgets silently.

### FEATURES-R9 — The kept-file reason puts raw OS text and Debug-printed identities on the job card
Severity: Nit
Fix under review: R1 / R5
Evidence class: source-traced
Location: `manager.rs:730-775` (`reclaim_job_partial`, `Err(reason) => reason.trim_end()`), `maps.rs:459` (`append_recovery_warning_because`), `delete_download` `Err(format!(…))`
Trigger: Any failed reclaim on Delete or cancel.
Causal path:
- Nothing sensitive leaks: the text holds the path, an OS error, and for two reclaim errors a `{:?}` of `FileIdentity { device, inode }` (`partials.rs:387-393` and `:539-543`).
- What changed from base is where it lives. Base showed it once in the Delete dialog. It is now also written into `job.error`, saved in the journal, and rendered as a standing `role="alert"` on the card until the next Delete or Dismiss.
- The commonest case reads "…so it was kept at <path>. No such file or directory (os error 2)", which contradicts itself for a non-technical reader and gives no instruction. On Windows the OS part is in the system language.
- The moved-folder case appends "Delete the map download, then start it again" to a message produced by pressing Delete.
- With several records, the single reason follows the whole path list, so it is unclear which path it describes.

Smallest correct fix: map the three common outcomes to plain sentences before they reach `job.error` and the `Err` (destination not available: reconnect the drive and press Delete again; folder replaced; file in use). Send the raw reclaim text to stderr.

### FEATURES-R10 — Small items
Severity: Nit
Evidence class: source-traced
- The widening to earlier jobs' leftovers has no direct test. The two older tests that covered a carried proven record now use a foreign file. Add one case: a carried record with its real inode is removed by another job's Delete, and a foreign file beside it survives.
- `NOT_OUR_FILE` ("is not the one this download wrote") is also used for a never-proven reservation (`identity: None`), where the file may well be this download's. "could not be confirmed as" would be accurate.
- `MapJob::recovery_path` does an ambient `symlink_metadata` from `status()`, which the UI polls. It only runs for records with a capture; on a hung network mount it would block the `map_status` call.
- The implementer's "there is no retry" for mobile reconcile misses `dispatch.rs:1513` (settings save also reconciles). The conclusion is unaffected.

**Earlier findings**
- R1: closed. The call that demotes returns `Err` with path and reason; a later Delete retries the same proven reclaim; `cancel()` keeps demotion and `Ok`.
- R2: not closed because nothing in `service.rs` calls `sweep_stale` at startup; `native_pdf.rs:334` is still the only caller.
- R3: not closed because `src/` still has no caller of `map_forget_recovery` and no reader of `recoveryPaths`. It matters more now: a permanently missing folder makes Delete return `Err` every time until Dismiss exists.
- R4: closed. A flush or sync failure is `retain: true, checkpoint_current: false`; a journal-save failure keeps the old class. The end-to-end test would fail with the old classification.
- R5: closed for the reason and the capture-directory path; the "reports nothing that is not there" half is what R8 questions.
- R6: closed by deleting the test. FEAT-8 remains uncompiled and untested.
- R7: not closed because `docs/STATE_MACHINES.md:270` still says `MapJob` tracks `eta_seconds`, and `docs/PLATFORM-GAPS.md` item 5 still says map downloads never hold the guard on any platform.

**Your five questions**
1. **Unlink or drop without proof.**
   - Unlink: no. The only unlink is still inside `reclaim_owned_file`, after directory identity, inode identity, exclusive capture and re-check inside the capability. Records without a journaled inode never reach it. A recovery record must have a `.streetzim-<uuid>.part` name or reclaim refuses. A `Published` or `Preserved` outcome is kept and only reported.
   - Earlier jobs' leftovers: safe. They are hidden partials matching a journaled inode, and nothing can resume them once demoted.
   - Drop: yes, see R8.
2. **Err text.** No secrets; noisy and now persistent (R9).
3. **Cleanup-directory path.** Correct. The captured path is reported only when an entry is there, otherwise the journaled one. `forget_recovery` matching either spelling cannot hit the wrong record, because the captured spelling contains the record's own cleanup UUID.
4. **R4.** Correct, including the creation checkpoint (the fresh partial is still reclaimed by the armed guard) and the final sync. The hook injects before the real `sync_all`, so it tests the classification, not a real fsync failure.
5. **Adjusted older tests.** Intent kept: publication reclaim is saved before the older warning is reported, restart does not bring it back, and the warning follows the next job. The coverage that moved is in R10.

**`cancel()` evidence: verified.**
- `mobile_policy.rs:472` pushes the error into `internet_errors`; `NETWORK.acknowledge` at `:492` runs only when that list is empty.
- `retirement_pending()` (`:45`) then keeps `admitted()` false, `mobile_network_check` (`:400-418`) falls through to "The previous network connection is still being closed…", and `internet_admission_of` (`:177`) reports `Retiring`.
- `dispatch.rs:1396` uses `cancel().await?` inside the private/read-only switch.
- Keeping demotion with `Ok` there is right.

**Implementer's doubts**
- Repeated `Err`: you confirmed it. It needs R3 to be livable for a permanently missing folder.
- Delete removes earlier jobs' proven leftovers: acceptable.
- Vanished ignores directory identity: real, R8.
- Raw reason text: R9.
- `status()` stats the capture path: acceptable, noted in R10.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- maps:: kokoro native_pdf`: first attempt, `test result: ok. 132 passed; 0 failed; 3 ignored; 0 measured; 1316 filtered out`. All new and adjusted maps tests are in that run.

**What I could not verify**
- Anything on Windows, macOS, iOS or Android, including the sharing-violation trigger and a real unplugged or shadowed mount.
- A real failing `fsync`.
- The implementer's neutralization runs and clippy results; I did not mutate the tree or re-run clippy.