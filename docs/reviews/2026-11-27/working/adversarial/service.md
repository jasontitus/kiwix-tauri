## Verdict for lane service: BLOCKING findings exist

Reviewed `git diff d98cac2` for `service.rs`, `service/dispatch.rs`, `service_tests.rs`, `imports.rs`, `spelling.rs`, `server.rs`, `server_tests.rs`, `archive.rs`, `mobile.rs`, `mobile_service.rs` and `native_pdf.rs`. The working tree matched HEAD (`6b69d87`/`f4ed211`) for `service.rs` and `dispatch.rs` when I read them.

Other lanes' edits are intact. For every checkpoint commit since `d98cac2`, I listed each line it added to `service.rs`/`dispatch.rs` that is no longer in the current file. Every one of those lines was this lane's own superseded iteration (the `import_hook` type, earlier LIB-7 and flip drafts). No foreign edit was dropped. Regions that belong to other lanes are present: `load_notice`, `edition_discard_original`/`original_discarded`, nearby hotspot release, the move warning, the official-catalog name, `mobile_apply_theme` and the H2 `remove_dir`.

The `native_pdf.rs` export-time `sweep_stale` call in `render()` belongs to the features lane; this lane added only the `pub(crate) use` line.

### SERVICE-R1: Android and iOS no longer compile
Severity: Blocking
Fix under review: LIB-3
Evidence class: reproduced
Location: `src-tauri/src/service.rs:1775` (`registered_at_path`); `src-tauri/src/store.rs:2763`
Trigger: any `cfg(mobile)` build. I ran `scratchpad/android-clippy.sh`.
Causal path:
- `if !canonical.is_file() || !self.store.has_book_at_path(&canonical).unwrap_or(false)` calls a method declared `#[cfg(any(not(mobile), test))] pub(crate) fn has_book_at_path`.
- The result is `error[E0599]: no method named has_book_at_path found for struct Arc<store::Store>`, then `could not compile kiwix-workbench (lib)`, exit 101.
- The implementer's Android run predates this prefilter. Their report calls the edit "platform-neutral", which is wrong.
- iOS takes the same cfg branch.
- Mobile-only borrow-check errors behind this type error cannot be ruled out until it compiles.
Smallest correct fix: drop the `cfg(any(not(mobile), test))` gate on `Store::has_book_at_path`, which is a pure read. Or gate the prefilter with `#[cfg(not(mobile))]`. Then re-run the Android clippy script.

### SERVICE-R2: a file replaced by rename at the same path is not detected
Severity: Should-fix
Fix under review: SERVE-1, LIB-3
Evidence class: source-traced. The decisive primitive is reproduced: after `rename(new, path)`, `fstat` on the old descriptor shows the same size and mtime, `is_file(path)` stays true, and the inode differs. Script in `scratchpad/adv-service/demo`.
Location:
- `service.rs:1702-1710` (`archive_for_book` probe)
- `archive_search.rs:54` (`Parts::unchanged`)
- `service.rs:1771-1781` (`registered_at_path`)
- `dispatch.rs:1822-1834` (availability pass)
Trigger: a book is in the archive cache because it was opened or searched in this run. The file at the same path is then replaced by `mv`, rsync's default temp-then-rename, or a download tool that renames over the old file. The user then opens that file again (double-click, `kiwix file.zim`, drop, Scan folder).
Causal path:
- The probe is `archive.retained_unchanged()` → `file.metadata()` on the old, still-valid unlinked inode, so it returns `Ok`.
- The availability pass uses `Path::new(&book.path).is_file()`, which stays true, so there is no flip and no invalidate.
- In `registered_at_path`, `self.archive_for_book(book.clone()).is_err()` is false, so it returns the old book. The new file is never verified.
- If the UUID differs, the user is shown the old content and told the import succeeded. The new archive is not registered.
- If the UUID is the same (for example a damaged re-download), the file the integrity check would have refused is later served under the existing entry without ever being verified.
- The report's claim "a different file at that path still takes the full verified import" holds only when no slot is cached. Its test uses `fs::write`, which keeps the same inode, so this case is never exercised.
- The same gap covers SERVE-1 trigger (a) on Linux. A yanked USB keeps returning cached `fstat` data. If it is replugged before the next 15 s maintenance tick, there is no flip, and the dead handle is kept until restart.
Smallest correct fix:
- In `registered_at_path`, require `library_files::archive_content_identity(&book)? == archive.content_identity()?`. This compares a fresh retain of the path with the handle (dev, inode, size, mtime, ctime per part). On a mismatch, fall through to the verified import.
- In `library_maintenance`'s blocking availability pass, treat a cached slot whose identity differs from the path's identity like an availability flip: invalidate it and retire its readers. Keeping this in the maintenance pass keeps path stats off the hot `archive_for_book` path.
- Add a test that keeps a handle, renames a new-UUID file over the path, and imports it again.

### SERVICE-R3: a monitored import cancelled by a privacy switch is remembered as a bad file
Severity: Should-fix
Fix under review: CORE-6
Evidence class: source-traced
Location: `service.rs:1916-1918` (`import_monitored_paths`), `service.rs:2045-2077` (`record_watch_result`), `watch.rs:159` (`FailureLog::record`)
Trigger: a new archive in a monitored folder is being verified. The user turns private or read-only mode on and later off again.
Causal path:
- `ImportCutover::begin` cancels the token, so `load_book_cancellable` returns `IMPORT_CANCELLED` as a per-path error.
- The watcher path and `library_maintenance` both pass the result to `record_watch_result`, which calls `failures.record(Path::new(path))` with the file's fingerprint.
- `should_skip` then skips that unchanged file in the live watcher and in every periodic scan until the file changes or the app restarts. The archive silently never appears; the only trace is an `eprintln`.
- Before this change the import waited and succeeded.
Smallest correct fix: in `record_watch_result`, skip entries whose `error == IMPORT_CANCELLED`. Alternatively, have `import_monitored_paths` map cancellation to `Ok(None)`, since the next scan retries anyway.

### SERVICE-R4: the "Import cancelled" message is dropped by the shell after a privacy switch
Severity: Should-fix (handoff: frontend lane)
Fix under review: CORE-6
Evidence class: source-traced
Location: `src/App.tsx:1907-1928` and `:1944-1950`; `src/api.ts:400` (`isPrivacyContextCurrent`)
Trigger: an `import_archives` or `scan_folder` call is verifying, and the user changes private or read-only mode in that window.
Causal path:
- Rust returns `{"errors":[{"error":"Import cancelled because … Add the archive again to import it."}]}`.
- The shell then returns early on `!isPrivacyContextCurrent(context)`, before `onError(...)`.
- The spinner stops, no book appears, and the actionable sentence is never shown.
- For Quit the message is moot. `lib.rs open_paths` relays it through `launch_failures`, which I did not trace further.
Smallest correct fix: either let the shell report `IMPORT_CANCELLED` errors even when the privacy context changed, or have the cutover call `note_startup_warning(IMPORT_CANCELLED)` when it cancelled a running import, which the shell already renders.

### SERVICE-R5: a reader is retired when its book becomes available again
Severity: Should-fix
Fix under review: SERVE-1
Evidence class: inference (timing-dependent); the code path itself is source-traced
Location: `dispatch.rs:1843-1871` (`flipped` includes `available` false→true)
Trigger:
- A book stored `available=false` from the last run is present now. Session restore or a link click in its tab opens a reader (`reader()` does not check `available`), and the next maintenance tick then flips it to true.
- Or: the volume is replugged, and the tab is used before the next 15 s tick.
Causal path:
- The availability pass pushes every changed entry: `if book.available != *available { flipped.push(...) }`.
- `retire_readers(&mut readers, Some(installation))` then shuts down a listener that was opened while the file was present, so it is valid. The tab's origin dies.
- A flip to available cannot leave behind a handle from before the outage: the earlier flip to unavailable already retired it, and with no observed outage there is no flip at all.
Smallest correct fix: retire readers only for flips to `available == false`. Invalidating the cache on either flip is harmless and can stay.

### SERVICE-R6: whole-archive verification still blocks some writers, now including mobile backgrounding during intake
Severity: Should-fix (needs owner decision)
Fix under review: CORE-6, LIB-8
Evidence class: source-traced
Location:
- `service.rs:3151-3166`: only `retire_private_work` begins an `ImportCutover`.
- `mobile_service.rs:1877` (background retirement) and `:384`: `privacy_gate.write()` with no cancellation.
- `service.rs:2393-2410`: intake now runs `load_book_cancellable(..., verify=true)`.
Trigger:
- A large manual import while a non-presentation `save_settings` (CORE-6 named `bookZoom`) or `kokoro_delete` queues a writer.
- On mobile, sharing a multi-GB ZIM into Kiwix and then switching apps.
Causal path:
- Those writers never cancel the token, so they and every later reader wait for the whole MD5 pass, which was CORE-6 trigger 4.
- LIB-8 made intake verify under the read lease. The mobile background writer then waits for it, so the background checkpoint can expire (`CHECKPOINT_EXPIRED`) before it is durable.
Smallest correct fix:
- Begin an `ImportCutover` in the mobile background-retirement writer path. The intake then fails with `IMPORT_CANCELLED` and stays pending for the next foreground, which the "any failure admits nothing" design already supports.
- Make per-book zoom a presentation-only save.

### SERVICE-R7: files written under an unmounted mount point bypass the LIB-7 device guard
Severity: Nit
Fix under review: LIB-7/CORE-5
Evidence class: source-traced
Location: `service.rs:2007-2010` (`SettledFile`: `remember_watched_device(parent, …)` with no condition); `service.rs:2161-2172`
Trigger: the monitored folder is a mount point and the volume is not mounted. A ZIM is written into the underlying directory, for example by a download or a copy.
Causal path:
- The settled event records the root filesystem's device, and the file is imported.
- On the next reconcile, `known` includes the new book, which is present, so `any_known_present` is true.
- The device is remembered and every missing volume book is retired.
- The import-path guard (`imported && (known.is_empty() || unchanged)`) is inconsistent with the unconditional record in `SettledFile`.
Smallest correct fix: in `SettledFile`, record the device only when nothing is recorded for the folder and it has no missing known books, or when the recorded device equals the current one. In reconcile, count as "present" only known books that were seen on the recorded device.

### SERVICE-R8: the spelling "keys match" assertion cannot fail
Severity: Nit
Fix under review: SERVE-3
Evidence class: source-traced
Location: `service_tests.rs:6383-6385`
Causal path:
- The dictionary was built in the same process, so `sweep` keeps it through `keep.extend(inner.statuses.keys())` whatever key `Identity::from_content` derives.
- "The key derived without opening the archive matches the build's key" is therefore never actually tested.
- By reading the code, I believe the keys do match: both use `ContentIdentity::from_handles` and the stored `book.metadata`.
Smallest correct fix: restart the service without a relink, run `library_maintenance`, and assert the `.words` file survives.

### SERVICE-R9: iOS now keeps no raw native error text anywhere
Severity: Nit
Fix under review: Android H3
Evidence class: source-traced
Location: `mobile.rs:187-193`
Causal path:
- The raw text is logged only under `#[cfg(target_os = "android")] eprintln!`. `plain()` maps native codes to sentences and `TRY_AGAIN`.
- User-actionable codes are preserved: the suspended envelope and shell-owned codes pass through. No user-facing actionable information is lost.
- The raw text is lost for diagnosis.
Smallest correct fix: call `crate::diagnostics::record(&service.store, operation, Some(&message))` before emitting.

### SERVICE-R10: the intake doc comment overstates atomicity
Severity: Nit
Fix under review: LIB-8
Evidence class: source-traced
Location: `service.rs:2383-2389`, `:2421-2433`
Causal path: if relink #1 commits and relink #2, or `admit_incoming_books`, then fails, relink #1 stays committed. It is harmless: no file is deleted, and the retried request resolves that path as a non-duplicate. Only the wording "admits nothing new" is too strong.
Smallest correct fix: correct the comment.

**Per-fix assessment**
- SERVE-1: fixes it but… It fixes in-place modification and deletion, and an outage that spans a maintenance tick. It misses replacement by rename and a USB replug within 15 s on Linux (R2). It also retires valid readers on a flip back to available (R5). The probe's race handling is correct: `Arc::ptr_eq` removal plus re-admission under the cache lock never drops a fresh slot or creates two. I found no lock-order problem: maintenance takes `localswarm_sources` → `readers`, the same order as relink and remove, and `reader()` probes under `readers` only.
- SERVE-3: fixes it. No-follow `Dir`, exact names, regular files only, `lifecycle` held, and a skip while a build or load is active or any book is unavailable. A live key and its `.previous` are kept. Note: a cold load does not take `lifecycle`, so a load whose key is in neither `live` nor memory could race. That is only reachable with a key mismatch (R2 / provider-backed books), so it is inference only. Test nit R8.
- SERVE-4: fixes it. All text is static and passed through `escape_attribute`. The page carries `bridge_injection`. Non-document responses are unchanged, apart from `is_document_request` moving earlier, which is pure.
- LIB-3: does not compile on mobile (R1). Its identity argument is incomplete (R2).
- LIB-7/CORE-5: fixes the reported empty-mount trigger, with a residual bypass (R7).
- LIB-8: fixes it on host. Relink replaces only unusable entries, removes no user file, and duplicate removal still proves the UUID. Mobile build broken (R1); mobile backgrounding concern (R6).
- CORE-6: fixes Quit and privacy cutovers. The token race with Quit is correct: `stopping` is checked under the token mutex, and overlapping cutovers are serialized by `settings_writes`. A cancelled import registers nothing and copies no file on desktop. Regressions R3 and R4; residual R6.
- CORE-9 + FEAT-6: fixes it. A whole-repo grep finds no caller in `src/`, `scripts/`, Kotlin, Swift, capabilities or `kiwix-mcp`. What remains is the harmless `mobile_policy.rs:566` test list and the stale `docs/ERRORS.md:2502`. `ERRORS.md` is wholesale out of date for reasons outside this lane: `test-error-docs.mjs` fails with 3650/3191 row differences.
- FEAT-3: fixes it. It runs once, detached, through `sweep_stale` with `STALE_AFTER` = 1 h, longer than the 16 min job ceiling. In-process `LIVE`/`PENDING` and the age rule protect another instance's live export. The Android `cachePath` is used without canonicalization, which is fine because leases match by file name. Not run in desktop read-only mode.
- Desktop handoff 2: fine (`revive` is rate-limited).
- Android H2: fine. It removes only the per-attempt UUID directory with `remove_dir`.
- Android H3: fine, but see R9.

**Implementer's doubts**
- LIB-3 skipping re-verification, including edition originals and replacements: acceptable for the same registered file. It returns an existing entry and grants no new authority; refusing to open a registered edition original was arguably a bug. It is a real problem when the path now names a different file (R2): fix now.
- Retiring readers on an availability flip: acceptable when the flip is to unavailable. A flip back to available should not retire (R5).
- Spelling sweep and in-memory keys / provider identities: acceptable. Removal is merely deferred to a later run. For provider-backed books with unstable per-open identities, the dictionary was not reusable across opens anyway.
- LIB-7 on Windows, and folders whose device was never recorded: needs an owner decision. Books deleted while the app was closed stay as unavailable entries indefinitely; they are visible and the user can remove them. Weigh this against the old behaviour of retiring them.
- Whole-file Python rewrites of `service.rs`: checked as described at the top; no foreign edit lost.

**What I ran**
- `cargo test --locked --lib -- service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf::` with `SHERPA_ONNX_ARCHIVE_DIR` set: `test result: ok. 403 passed; 0 failed; 6 ignored`.
- `scratchpad/android-clippy.sh`: `error[E0599] … has_book_at_path … src/service.rs:1775`, exit 101 (R1). Log at `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-service/android-clippy.log`.
- `node --test scripts/test-error-docs.mjs`: 4 passed, 1 failed (repo-wide `ERRORS.md` drift).
- Python `fstat`-after-rename demonstration (R2) in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-service/demo`.
- Repo-wide grep for the removed commands, and the per-commit check for dropped lines in `service.rs`/`dispatch.rs`.

**What I could not verify**
- Any iOS or Swift compile.
- Mobile code behind the R1 compile error.
- Real mount and unmount, USB or NFS.
- The provider-backed `retain`.
- The shell's actual reaction to a retired listener (R5) and the rendered SERVE-4 page.
- Reverted-fix failure of each new test: I reasoned about it from the code instead; R8 is the one test that cannot fail.
- I did not re-run Linux or Windows clippy.