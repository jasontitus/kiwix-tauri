## CORE review — Rust application core (base `d98cac2`)

**Disclosure first.** I broke the read-only rule once, by accident: I ran `node scripts/generate-error-docs.mjs --help` expecting usage text, but the script ignores arguments and rewrote `docs/ERRORS.md`. I restored the file byte-exact from `HEAD` (blob `76ca4512…` verified with `git hash-object`; `git status --short` is empty again). No other repository file was touched. The regenerated copy is kept only in my scratch directory and is the evidence for CORE-10.

All findings are source-traced; nothing was compiled or run (no `cargo`). Line numbers are from the current tree.

### CORE-1 — Quit cannot complete while the profile cannot be written; the only dialog offers "Retry"
Severity: High        Category: usability
Platforms: macOS, Windows, Linux
Evidence class: source-traced
Location: `src-tauri/src/quit.rs:306` (`prepare`), `quit.rs:147-156` (`ack_quit_flush`), `src-tauri/src/lib.rs:1282-1307` (ExitRequested handler), `src-tauri/src/store.rs:2902-2904` (`update_inner`), `src/App.tsx:2020` (`flushSession`)
Trigger: The profile volume is full, removed or remounted read-only, or the store has latched a durability error ("further writes are disabled until restart", `store.rs:3486`). The user then chooses Quit.
Causal path:
1. Each shell's `flushSession` always calls `save_session` in normal mode. That reaches `self.store.update(...)` (`dispatch.rs:971`), which returns `Err(durability_error)` or the `persist` I/O error.
2. The shell acks with that error. `ack_quit_flush` records it ("A failed acknowledgement always cancels"), and `prepare` returns `Err`.
3. `lib.rs:1282-1287` only prints `Application Quit cancelled`, resets `exiting` and returns. Every later Quit repeats this.
4. Even if the flush passes, `crate::windows::capture(app, &service.store)?` at `quit.rs:306` is another durable store write and is fatal to `prepare`. The session snapshot two lines below is deliberately non-fatal.
5. If `Service::shutdown` fails instead (`flush_browsing` error, `service.rs:3643`), the dialog uses `MessageDialogButtons::OkCustom("Retry")`: one button, and the callback only retries.
6. Net effect: the latch tells the user to restart, but the app can only be left by force-kill. A self-update relaunch is blocked the same way.
Smallest correct fix:
- Make the `windows::capture` failure in `prepare` non-fatal (log it, like the session snapshot).
- When `quit::prepare` or `Service::shutdown` fails, show a two-button native dialog (Retry / Quit without saving).
- On "Quit without saving", set `authorized`, skip the flush and exit. This touches the §1.1 contract in `docs/STATE_MACHINES.md` (Preparing → Idle on flush error), which needs the new edge.
Regression test: `quit::tests` — extend the attempt state machine with a "forced" transition and assert a failed ack can be overridden. `store_tests.rs` — after `fail_next_directory_sync_for_test()` latches, assert the extracted quit decision helper returns "may exit unsaved" instead of an unconditional cancel.

### CORE-2 — Fatal desktop startup errors are shown to nobody
Severity: High        Category: usability
Platforms: Windows (worst), macOS, Linux
Evidence class: source-traced
Location: `src-tauri/src/lib.rs:1399-1405` (`run`), `lib.rs:1113` (setup error), `lib.rs:887-893`, `lib.rs:941-952`; `src-tauri/src/main.rs:6`; `tauri-2.12.5/src/app.rs:1443-1444`
Trigger: Any non-"busy" profile failure on a GUI launch, for example:
- `state.json` saved by another version (`store.rs:998-1003`), or a settings key this build does not know (`Settings` is `deny_unknown_fields`, `store.rs:465`);
- damaged or oversize `state.json`;
- data directory on an absent volume;
- home directory on NFS/SMB where `flock` is unsupported (`ProfileLockError::Lock`).
Causal path:
1. Errors before the builder return through `run()`: `eprintln!("Kiwix: {error}"); std::process::exit(1)`.
2. Release builds set `windows_subsystem = "windows"`, and Finder or desktop-entry launches have no visible stderr.
3. Errors inside setup return `Err(error.to_string().into())`. Tauri turns that into `panic!("Failed to setup app: {e}")`, so the process dies.
4. The store's carefully written recovery text ("Open it with that version, or choose another profile. Your saved file was not changed.") is never displayed. The app simply does not open, on every launch.
5. Only the busy-profile case has a dialog (`instance::show_native_message`, `profile_busy_in_setup`).
Smallest correct fix:
- In the setup `Err(error)` arm, reuse the `profile_busy_in_setup` pattern: close windows, show `app.dialog()` with the error, exit 1 in the callback. Never return `Err` to Tauri.
- In `run()`, show the error through a native message box before `exit(1)`. Generalise `instance::show_native_message` (osascript / zenity / kdialog) and add a Windows `MessageBoxW` arm, since the `windows` crate is already a dependency.
- Keep stderr output as well.
Regression test: `lib.rs` `launch_tests` — factor the reporting into `report_startup_failure(error) -> StartupReport` and assert a `StoreOpenError::Fatal` maps to "dialog + exit 1", never to a setup `Err`. Add a test that a version-3 `state.json` produces that report with the store's message intact.

### CORE-3 — `localswarm_discovery` takes the privacy read lease twice on mobile; a writer queued in between deadlocks every command
Severity: Medium        Category: reliability
Platforms: Android, iOS
Evidence class: source-traced
Location: `src-tauri/src/service/dispatch.rs:2099-2109` (`"localswarm_discovery"`), `src-tauri/src/nearby_service.rs:450` (`nearby_command`), `src-tauri/src/service.rs:2769-2777` (`operation`)
Trigger: Nearby/LocalSwarm discovery is enabled (fired when the panel opens, `LocalSwarmControls.tsx:222/239/586`) while one of these arrives before the arm reaches its Nearby step:
- `kokoro_delete`;
- any mutating `private_sync_*` command;
- `request_completion_notifications`;
- background `mobile_speech_released` / `mobile_media_released`.
These writers do not pass through `mobile.transfer_gate` first. The window is widened because `LocalSwarmManager::discovery` first waits on the LocalSwarm `operation` mutex (`localswarm/mod.rs:742`).
Causal path:
1. `operation` gives this command `_mode_guard = Some(self.privacy_gate.read().await)`; it is not in the lease-free list.
2. The arm runs `#[cfg(mobile)] self.clone().nearby_command("nearby_discover", args.clone())`.
3. With no live consumer, that reaches `let _mode = self.privacy_gate.read().await;` (`nearby_service.rs:450`).
4. Tokio's `RwLock` is write-preferring. A writer queued after the outer read blocks the inner read, and the writer waits for the outer read. Neither is ever released.
5. Every later command and lifecycle transition then hangs on the gate.
6. The code's own rule at `service.rs:2700` is "Never nest read guards behind Tokio's fair RwLock." Lifecycle transitions and `save_settings` are fenced by `transfer_gate`, which is why this has not been seen more often.
Smallest correct fix: Split the body after line 450 into `nearby_discover_leased(&self, label, …)`, which assumes the caller's lease. Call that from the `localswarm_discovery` arm. Keep `nearby_command` (the lease-free early-return path in `operation`) acquiring the lease itself. No contract change.
Regression test: `service_tests.rs`, compiled under `cfg(test)` — hold `update_privacy_gate()`'s read lease as the dispatch arm would, queue a `write()`, call the new leased helper, and assert it completes within a timeout. A second test asserts the public `nearby_command` path still takes exactly one lease.

### CORE-4 — Records skipped while loading a profile are permanently deleted by the next ordinary write; desktop users are never told
Severity: Medium        Category: reliability
Platforms: all (mobile has an additional variant, below)
Evidence class: source-traced
Location: `src-tauri/src/store.rs:911-938` (`decode_records`), `store.rs:1242`, `store.rs:1416-1417` (field doc), `store.rs:2888-3006` (`update_inner`), `src-tauri/src/service.rs:746-765` (`from_store`), `store_tests.rs:1175`
Trigger: A profile holds a record this build cannot decode. `Bookmark`, `Highlight`, `HistoryEntry` (`store.rs:175/221/242`) and `IntegrityRecord` (`integrity.rs:79`, embedded in every `Book`) are `deny_unknown_fields`, while the schema stays at version 2. So a downgrade after any later build adds an optional field skips every such record. A hand-edited or partly damaged file does the same. Then the app runs normally.
Causal path:
1. `decode_records` drops the record and pushes a content-free warning.
2. `state_from_value` returns `normalized && warnings.is_empty()`, so nothing is rewritten at load. The tests assert exactly that ("A skipped record is retained on disk").
3. The first `Store::update` (session autosave, `record_visit`, window capture on close) clones `inner.state`, which no longer has the records, and `persist` renames it over `state.json`. No copy of the original is kept.
4. The comment at `store.rs:1416` says "never persisted back over the original"; that only holds until the first update.
5. On desktop `startup_warnings` is `Vec::new()` (`service.rs:755-756`). The only trace is `diagnostics::record`, which is off by default (`diagnostics_enabled: false`).
6. Mobile variant: `load_mobile_state` (`store.rs:1798-1815`) quarantines the whole `state.json` and starts empty for any decode error without sync metadata. That includes "saved by a different version" and an unknown settings key, which are not corruption.
Smallest correct fix:
- In `open_profile` / `load_mobile_state`, when warnings are non-empty and the profile is writable, write the original bytes once to `state.skipped-<uuid>.json` (`create_new`, 0600, fsync) before returning the `Store`.
- Surface one desktop startup warning naming that file.
- In `load_mobile_state`, return the error instead of quarantining when the failure is the version check.
- Correct the field doc.
Regression test: `store_tests.rs` — extend `unreadable_book_records_are_skipped_and_reported`: after reopening, call `store.update(|_| Ok(()))` and assert a preserved file still contains `"broken-book"`. Add a mobile-path test that a `"version": 3` file is not renamed.

### CORE-5 — The folder watcher removes every library entry in a monitored folder when its volume is unmounted but the mount-point directory remains
Severity: Medium        Category: reliability
Platforms: Linux (fstab / manual / NAS mount points, WSL `/mnt/x`); macOS only when a stale empty `/Volumes/<name>` is the monitored folder
Evidence class: source-traced
Location: `src-tauri/src/service.rs:1968-2050` (`reconcile_watched_directory`), `service.rs:3834-3858` (`scan_watched_directory`), `service.rs:2056-2101` (`retire_missing_watched_book`), `src-tauri/src/watch.rs:380-386` (RESCAN)
Trigger: The monitored folder is the mount point itself (for example `/mnt/library`). The drive or share is not mounted at launch, or is unmounted while Kiwix runs.
Causal path:
1. The watcher observes the directory at start (`watch.rs:446`) and every 30 s.
2. `reconcile_watched_directory` treats only `canonicalize` or `read_dir` failure as "unreachable". An empty underlying mount point canonicalises and lists fine, giving `present = {}`.
3. For every known book whose parent is that directory and is not open in a tab, `retire_missing_watched_book` runs `state.books.retain(...)`, `state.reader_storage.remove(&installation_id)` and drops the pin.
4. On remount each file is re-imported under a new installation ID after a full integrity pass. Per-archive reader storage, pins and integrity records are lost.
5. This contradicts the function's own comment ("must never be read as 'everything was deleted'") and `store.rs:2` ("Missing archives are retained; only explicit service commands remove books").
6. The periodic `library_maintenance` pass would only have marked the books `available=false`.
Smallest correct fix: In `reconcile_watched_directory`, do not retire when none of the directory's known books is present. Leave them to `library_maintenance`'s `available=false`. Optionally, on Unix, also require the directory's `st_dev` to equal the one remembered from the last pass in which a known book was present. Retirement of an individual vanished file among surviving siblings stays as it is.
Regression test: `service_tests.rs`, next to the `collect_watched_entries` tests — register two books in a temp monitored directory, replace it with an empty directory of the same name, run `reconcile_watched_directory`, and assert both books and their `reader_storage` remain. Assert that deleting one of two files still retires just that one.

### CORE-6 — Manual import verifies the whole archive uncancellably under the privacy read lease, so Quit or a policy change freezes the app until it finishes
Severity: Medium        Category: reliability
Platforms: all (the Quit symptom is desktop)
Evidence class: source-traced
Location: `src-tauri/src/service.rs:1694-1722` (`import_paths`), `src-tauri/src/imports.rs:47-63` (`load_book`), `src-tauri/src/service/dispatch.rs:452-459`, `service.rs:2755` and `service.rs:3553`
Trigger: The user imports or scans a large ZIM (tens of GB; many minutes on HDD, USB or network storage). During the import they Quit, switch private or read-only mode, change the per-book zoom (`bookZoom` is not a presentation key, `service.rs:550-574`), or delete the voice model.
Causal path:
1. `import_archives` runs `blocking(move || service.import_paths(...))` while `operation` holds `_mode_guard` (read) and the update-admission read.
2. `import_paths` calls `load_book(&path, managed, true)`, which calls `archive.check_integrity()`. That is the non-cancellable variant, although `load_book_inner` already supports a token.
3. `Service::shutdown` reaches `self.privacy_gate.write().await` and waits for the whole checksum, with shells already frozen by the Quit flush.
4. Any other queued writer (non-presentation `save_settings`, `kokoro_delete`) also makes every later reader (`snapshot`, `open_article`, `record_visit`) queue behind it. The whole UI stalls with no progress indication.
5. The comment at `service.rs:2711-2713` acknowledges the hazard, but only presentation saves were exempted.
Smallest correct fix:
- Give `Service` an import cancellation token.
- Cancel it in `shutdown()` before taking the writer, and in `operation` where `retire_private_work` is computed for a policy change. Replace it afterwards.
- Pass it via `load_book_inner(path, managed, true, Some(&token))` from `import_paths` and `import_monitored_paths`.
- A cancelled path becomes a per-path error ("Import cancelled").
Regression test: `service_tests.rs` — use the existing `archive_open_hook` / mutation gate to block an `import_archives`, call `shutdown()`, and assert it returns within a bound with the import reporting a cancelled error and no book registered.

### CORE-7 — Archives opened from the OS that fail to import produce no visible message
Severity: Medium        Category: usability
Platforms: macOS, Windows, Linux
Evidence class: source-traced
Location: `src-tauri/src/lib.rs:594-692` (`open_paths`, especially 668-672 and 688-690), `lib.rs:1353-1361` (`RunEvent::Opened`)
Trigger: Double-clicking or passing a truncated or corrupt `.zim`, a file the sandbox cannot read, or an unregistered archive in read-only mode. On macOS, also any Open With while private mode, a pending Quit or a privacy transition refuses the import.
Causal path:
1. `import_paths` reports per-file failures inside `Ok({"books":[],"errors":[…]})`.
2. `open_paths` then does `eprintln!("Archive open errors: {}", imported["errors"]); return Ok(())`.
3. `spawn_secondary_paths` shows a dialog only on `Err`, and startup calls `note_startup_warning` only on `Err`. So the window comes forward and nothing else happens.
4. The macOS `Opened` handler is weaker still: even a hard `Err` is only `eprintln!("Opening document: {error}")`.
5. The in-app picker path shows the same `errors` array through `onError` (`App.tsx:1806-1812`), so the two entry points have diverged.
Smallest correct fix: Have `open_paths` return the per-path errors: `Ok(Vec<(path, error)>)`, or `Err` when nothing opened and errors exist. Route them through the dialog in `spawn_secondary_paths`, through `note_startup_warning` plus `kiwix-open` at startup, and use the same dialog helper in the `Opened` handler.
Regression test: `lib.rs` `launch_tests` — call `open_paths` with a non-ZIM temp file and assert the returned errors are non-empty and carry the path. Extend `incompatible_secondary_launches_reject…` to assert a corrupt-file launch yields a reportable error, not `Ok(())`.

### CORE-8 — Crash-report path redaction stops at the first space, leaking the rest of the path
Severity: Low        Category: reliability
Platforms: all; most visible on Windows profiles such as `C:\Users\Jane Doe\…`
Evidence class: source-traced (a line-for-line Python port of `path_at` reproduces the outputs below; the Rust code was not run)
Location: `src-tauri/src/crash.rs:527-556` (`path_at`), used by `redact` and `redact_line`
Trigger: Any panic whose message contains a path with a space. The setup failure in CORE-2 is one: Tauri panics with the store's `Cannot parse <profile path>…` after `crash::install` has armed capture.
Causal path:
1. `path_at` ends the match at `chars[end].is_whitespace()`.
2. `…Cannot parse C:\Users\Jane Doe\AppData\Roaming\kiwix\…\state.json` becomes `[path] Doe\AppData\Roaming\kiwix\…\state.json`.
3. `/Volumes/Family Backup/Health Records/medical_wikipedia.zim` becomes `[path] Backup/Health Records/medical_wikipedia.zim`.
4. The remainder does not start at a path boundary, so no later pass catches it.
5. The module header promises the record "never contains … library paths, user document paths". The existing tests only use space-free paths.
Smallest correct fix: Once a path start is recognised, extend the redaction to the end of the path-bearing clause: stop only at `"`, `'`, `<`, `>`, a backtick, a newline, the two-character sequence `": "`, or end of text. Over-redaction is acceptable here. Leave `file_name()` handling of backtrace locations unchanged.
Regression test: `crash::tests::redaction_drops_paths_url_authority_and_secrets` — add the two strings above and assert the output contains none of `Doe`, `Health`, `medical_wikipedia`.

### CORE-9 — `map_forget_recovery` has no caller, so a preserved map recovery record can never be cleared; four more arms are dead
Severity: Low        Category: maintainability
Platforms: all
Evidence class: source-traced (both sides searched: `src/**`, `kiwix-mcp`, `scripts`, native plugins)
Location: `src-tauri/src/service/dispatch.rs:2640-2647`; `src-tauri/src/maps/manager.rs:915-934` (`forget_recovery`), `manager.rs:878-880`, `manager.rs:900-902`; dead arms at `dispatch.rs:2486` (`torrent_limits`), `:2813` (`sharing_status`), `:2081/:2090` (`localswarm_hotspot_start/stop`)
Trigger: A map download's partial cannot be reclaimed when the next map starts (destination volume offline, permission denied, identity changed). The record moves into `recovery_records` (`manager.rs:426-455`).
Causal path:
1. `recovery_records` only shrinks in `forget_recovery`, which is reachable only through `"map_forget_recovery"`.
2. No file in `src/` contains that string. `src/api.ts` and `Maps.tsx` only display `recoveryWarning`.
3. From then on every map job carries "Some earlier map downloads could not be cleaned up…".
4. `remove_terminal` refuses with "Resume or delete retained map data first.", and delete refuses with "Map recovery warnings remain unresolved; this download stays visible…". There is no UI action that resolves it.
5. The other four arms are simply unreferenced: limits go through `save_settings`, sharing status travels in `snapshot`, and the hotspot is driven natively. They still appear in the admission and mutation lists (`service.rs:2673-2678`, `3727-3740`; `mobile_policy.rs:566`; `mobile_service.rs:205-216`).
6. No frontend `api()` call lacks a handler; the `mobile_*`, PDF, `reader_backdrop`, `update_cancel` and notification commands are handled in `operation` or `mobile_command`.
Smallest correct fix: Add a "Forget" action beside the recovery warning in `Maps.tsx` that calls `api("map_forget_recovery", { path, privacyGeneration })` per listed path. This requires exposing the paths as a field instead of only a sentence. Delete the four unused arms and their entries in the admission and mutation lists.
Regression test: a `scripts/test-command-contract.mjs` that extracts dispatch arm names and frontend command literals and fails on an arm with no caller outside an explicit allow-list (native/background commands). Also a `maps_tests.rs` case asserting a job with a recovery record becomes removable after `forget_recovery`.

### CORE-10 — `docs/ERRORS.md` says "Generated from current sources" but is about 780 entries out of date, and the generator has no check mode
Severity: Low        Category: maintainability
Platforms: n/a
Evidence class: reproduced (I regenerated the file — by accident, see the disclosure — and compared it with the committed one)
Location: `docs/ERRORS.md` (last generated 2026-11-19, commit `35e220b`); `scripts/generate-error-docs.mjs:73`; `package.json:25`
Trigger: Any change to error strings since 2026-11-19 (the 11-23 and 11-25 remediations and `d98cac2`).
Causal path:
1. The committed inventory has 3191 rows; current sources produce 3623.
2. 175 documented literals no longer exist. Examples: `store.rs` "Cannot parse {}: … not an empty replacement." under its old location, `service.rs` "Reading saved state: {e}", `dispatch.rs` "Native location requires an active Apple application event loop…".
3. 607 current literals are undocumented. Examples: "Could not open startup archive: {error}", "Writing the last reading position: {error}", "Pinned archives require the dedicated pin action", "The activation endpoint belongs to another account".
4. 2896 rows carry stale line numbers.
5. Nothing in `package.json` or CI compares the file with its generator. The script also writes unconditionally whatever arguments it is given, `--help` included.
Smallest correct fix: Regenerate with `npm run docs:errors`. Add a `--check` mode to `generate-error-docs.mjs` (build the document, compare with the file, exit 1 on difference, never write) and run it in the existing Node test or CI step. Make unknown arguments an error.
Regression test: `scripts/test-error-visibility.mjs` (or a new `test-error-docs.mjs`) invoking the `--check` mode.

### CORE-11 — `docs/STATE_MACHINES.md` §1.2 and §1.4 no longer describe the code
Severity: Low        Category: maintainability
Platforms: n/a
Evidence class: source-traced
Location: `docs/STATE_MACHINES.md` §1.2 and §1.4; `src-tauri/src/service.rs:577-579`, `service.rs:2773`; `src-tauri/src/store.rs:2942-2954`
Trigger: Reading the document to reason about epochs or durability.
Causal path:
1. §1.2 lists three epochs and says "`launch_generation` belongs to the launch queue … All three are bumped". There is no such field: `launch_generation()` returns `self.privacy_generation`.
2. §1.2's lease-free command list omits `catalog_facets`, `catalog_torrent`, `map_recipe_metadata` and `update_install`, which the code also runs without `_mode_guard`.
3. §1.4 says "Every change is made on a clone and persisted atomically, then published in memory". Since the coalesced browsing path, `update_inner` publishes a telemetry-only change with `browsing_pending = true` and no write for up to `BROWSING_COALESCE_INTERVAL`.
4. Neither section records the two rules that CORE-3 depends on: `transfer_gate` before privacy, and never nesting a read lease.
Smallest correct fix: Edit §1.2 (two epochs; full lease-free list; a "lock order" line: update admission → `settings_writes` → `mobile.transfer_gate` → privacy; no nested read) and §1.4 (coalesced telemetry and its flush points). Documentation only.
Regression test: none practical; covered by review.

**Covered** (read end to end unless noted)
- `store.rs`: record decoding, validation, profile open (desktop and mobile), runtime mode, `update_inner`, `flush_browsing`, `persist`, session snapshot, startup hint. Not the acquisition-path comparison helpers (about 1425-1616).
- `quit.rs`, `crash.rs`, `disk_warning.rs`, `user_error.rs`, `diagnostics.rs`, `update_admission.rs`, `os_open.rs`, `main.rs`.
- `instance.rs`, and `instance/socket.rs` up to the stale-socket probe.
- `watch.rs` (non-test code), `fs_ext.rs` (non-test code).
- `lib.rs`: option parsing, launch/open paths, `maintenance`, `run_application` (setup, window events, exit handling). Not its tests.
- `service.rs`: `from_store`, launch queues, `operation`, `dispatch`, update leases, settings validation, import paths, folder-watch application and reconciliation, `shutdown`.
- `service/dispatch.rs`: entry gates, `snapshot` head, `save_session`, `record_visit` head, import arms, `save_settings`, catalog / `shared_library_browse` / `map_prepare` / `localswarm_discovery` arms, `library_maintenance`, tail. All 148 arm names were cross-checked mechanically against frontend command strings.
- Every `privacy_gate` acquisition in the crate was located and its caller checked for nesting.
- `docs/STATE_MACHINES.md`, `docs/PLATFORM-GAPS.md`, and the 11-23 and 11-25 review records in full.
- No finding in the platform-stub lens beyond what `PLATFORM-GAPS.md` already documents (for example the `power_activity.rs` no-op on Windows/Linux).

**Not covered**
- `migration.rs`, `storage.rs`, `portable.rs`, `profile_paths.rs`, `permissions.rs`, `sandbox.rs`, `reading_data.rs`, `menus.rs`, `macos_menu_keys.rs`, `native_tabs.rs`, `notifications.rs`, `notifications_service.rs`, `desktop_workflow.rs`, `launch_search.rs`, `localization.rs`, `timestamps.rs`, `cpu_time.rs`; `power_activity.rs` beyond its header and stubs.
- Most individual dispatch arms: bookmarks, history, downloads, torrents, maps, sensors, exports, crash-report commands.
- `service.rs` reader, restore-session and Android-window sections.
- The test modules, except `store_tests.rs:1130-1240`.
- `docs/RELIABILITY.md` was searched, not read through.
- The remaining slot was left empty; nothing else met the bar.