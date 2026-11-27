Area FEAT review at `d98cac2`: 10 findings, ranked. Nothing in the repository was modified; no cargo/gradle/npm build was run. One Node snippet (regex check, in my scratch dir) was executed. tokio 1.54.1 and misaki-rs 0.9.0 sources were read from `~/.cargo/registry`.

### FEAT-1 — A resumable map whose partial cannot be reclaimed permanently blocks every map download
Severity: High        Category: reliability
Platforms: all (desktop most reachable)
Evidence class: source-traced
Location: `src-tauri/src/maps/manager.rs:397-403` (`MapManager::start` guard) [+ `manager.rs:832-886` `delete_download`, `manager.rs:712-737` `reclaim_job_partial`, `manager.rs:662-710` `cancel`, `manager.rs:72-101` `new`, `maps/partials.rs:320-325` and `:408-410` `reclaim_owned_file`, `src/components/Maps.tsx:754,962-980,1104-1107`]
Trigger: Pause a map (or let it fail after its first checkpoint). Then delete, move, or re-create the download folder, or have a sync/AV tool rewrite the hidden `.streetzim-*.part` so its inode changes. Try Resume, Delete download, then Get map on any region.
Causal path:
1. The job has `resume: Some` and `owned_partial: Some`.
2. Resume: `resume()` → `open_map_checkpoint` fails on `MapDirectory::open`, or on `directory.identity != record.directory_identity`.
3. Delete: `delete_download` persists `status="cancelled"`, then `self.reclaim_job_partial(job)?` runs `reclaim_owned_file`. That returns `Err` for a missing directory, `Err("This partial map download was moved or changed, so Kiwix kept it. Delete the map download, then start it again")` for a changed directory identity, or `Ok(Preserved)` for a foreign inode, which becomes `Err("Map file ownership changed; no bytes were deleted")`. `resume` and `owned_partial` stay set.
4. Get map: `start()` returns `"Resume, finish importing or delete the previous map download before starting another map"` because `previous.resume.is_some()`.
5. `remove_terminal` refuses with "Resume or delete retained map data first."; `cancel()` hits the same `reclaim_job_partial` error.
6. A restart goes through `new()`, which only sets `resumable=false` and keeps `resume`.
7. No path clears `resume`. The branch at `manager.rs:439-443` that would carry such a record into `recovery_records` is unreachable behind the guard at 398.
8. In the UI, Get map is disabled without explanation while the job is `paused` (`active` is truthy, `Maps.tsx:754/1106`). After the failed Delete the copy says "This download cannot be resumed. Get the map again to start a new download." and Get map is then refused.
9. The same copy/backend contradiction appears for a `resume.invalid` checkpoint (covered by `resume_rejects_changed_prefix…`), though there Delete works.
Smallest correct fix: In `delete_download` (and `cancel`), when `reclaim_job_partial` cannot reclaim (error or `Preserved`), demote instead of failing forever: move `owned_partial` into `recovery_records`, clear `resume`/`resumable`, persist, then return the existing "Map recovery warnings remain unresolved…" error. No bytes are deleted, which matches the header contract in `maps.rs:29-31`. Remove the dead branch at 439-443. Change the `Maps.tsx:972` copy to "Delete this download, then get the map again". Journal contract touched: `recoveryRecords` may then hold records with a proven identity.
Regression test: `maps_tests.rs`, using `paused_output`. Rename the output dir away and recreate it; assert `delete_download` errs once, a following `start(...)` for another region succeeds with `recovery_records.len()==1`, and the original bytes are intact. Add a second case with the partial replaced by a foreign file.

### FEAT-2 — Disk-full during composition records a resume checkpoint past the bytes on disk; Resume is then refused and the whole prefix must be deleted
Severity: Medium        Category: reliability
Platforms: all
Evidence class: source-traced (including tokio 1.54.1 `fs/file.rs`)
Location: `src-tauri/src/maps/manager.rs:1400,1413,1428-1441` (`compose` write loop) [+ `:1068-1079` `checkpoint_output`, `:1473-1492` error-path checkpoint, `maps/partials.rs:155` `open_map_checkpoint`]
Trigger: The destination volume fills, or a write fails, while a map is composing. Another download on the same volume is enough. Then free space and press Resume.
Causal path:
1. `tokio::fs::File::poll_write` returns `Ok(n)` as soon as it has spawned the blocking write (`file.rs:827-849`). A failure surfaces only on the next write or on `flush`.
2. Every `output.write_all(slice)` therefore returns with its last sub-write (up to 2 MiB) unverified. The code then immediately runs `md5.consume(slice); prefix_sha256.update(slice); chunks.written += count`.
3. `checkpoint_output` calls `output.sync_all()`. Tokio's `sync_all` runs `complete_inflight()`, which stores the error in `last_write_err` and discards it (`file.rs:317-323,1194-1203`), then fsyncs successfully. The periodic checkpoint persists `written = chunks.written`.
4. The error path does the same: `output.into_std().await` swallows the error, then `checkpoint_resume(written: chunks.written, …)` at 1487.
5. The journal now claims more bytes than the file holds. Resume fails at `metadata.len() < written` with "Map checkpoint file changed identity or length; no bytes were modified". After a restart, `new()` sets `resumable=false`.
6. The earlier valid checkpoint was overwritten, so the only way out is Delete and re-download. The published output is never corrupted (`VerifiedFile::new` checks the length).
7. Every slice of 2 MiB or less hits this when its write fails; for full 16 MiB slices it is one sub-write in eight.
Smallest correct fix: After each `write_all` at 1400/1413/1428, `output.flush().await.map_err(|e| format!("Map write failed: {e}"))?` before hashing and counting; a small `write_durable` helper would do. Also flush before `sync_all()` in `checkpoint_output`.
Regression test: `maps_tests.rs` or a helper unit test. Take a `tokio::fs::File::from_std` of a file opened read-only: the helper must return `Err` from the same call (plain `write_all` returns `Ok`), and no counter may advance.

### FEAT-3 — Article-PDF scratch (full article HTML with entered form text, plus the PDF) is never removed after a crash or kill, including private-mode exports
Severity: Medium        Category: reliability
Platforms: all
Evidence class: source-traced
Location: `src-tauri/src/native_pdf/cleanup.rs:17-40,71-88` (`Owner::drop`, `retry_cleanup`) [+ `pdf_export.rs:218-230`, `native_pdf.rs:72-78`, `native_pdf/live.rs:42-47`, `service.rs:3639`]
Trigger: The process dies while a save/share/print PDF job exists. Print and Android jobs live up to 16 minutes, and iOS/Android kill backgrounded apps while a share sheet or print UI is up.
Causal path:
1. `OwnedPdf::create` makes `<app cache>/article-pdf/pdf-<uuid>/`.
2. `Snapshot::document` writes the sanitized live article to `live-document`. The provider writes `article.pdf` (and a WebView2 profile on Windows).
3. Removal exists only in `Owner::drop` and the in-memory `PENDING` set, retried at orderly shutdown. `cleanup.rs:69` says "Never scans arbitrary directories".
4. Nothing sweeps `article-pdf` at startup; the only reference is `pdf_export.rs:219`.
5. `admit_pdf` allows private mode ("Private mode is allowed", `pdf_export.rs:105`), so a private session's article text can persist in the cache indefinitely.
Smallest correct fix: Add `native_pdf::sweep_stale(root)`. Without following links, remove children of `<cache>/article-pdf` named exactly `pdf-<uuid>` whose mtime is older than the job ceiling (for example 1 hour) and that are not in `PENDING`. Run it off the main thread at service start and before `OwnedPdf::create`. The same age-cutoff pattern already exists for `kiwix-mobile-exports` in `KiwixMobilePlugin.kt:369-372`.
Regression test: `native_pdf/cleanup.rs`. Assert an old `pdf-<uuid>` dir is removed, a fresh one is kept, and a non-matching name or a symlink is kept.

### FEAT-4 — One unpronounceable chunk ends Kokoro read-aloud for the whole article
Severity: Medium        Category: usability
Platforms: desktop, iOS, Android (wherever Kokoro runs)
Evidence class: source-traced (misaki-rs 0.9.0 source and its own tests); the whitespace-only case is inference
Location: `src-tauri/src/kokoro.rs:679-682,690-692` (`synthesize`) [+ `:824-828` `run`, `:573-578` worker error event, `src/native-speech.ts:349-354`]
Trigger: An English article read with a Natural voice contains a line of at least 20 bytes that misaki cannot phonemize as a single token. Examples: seven or more CJK characters in their own block, a long out-of-vocabulary word as a heading, or a bare URL line.
Causal path:
1. `Cargo.toml:108` sets `misaki-rs = { default-features = false }`, so there is no espeak fallback. Unresolved tokens become `❓` (`g2p.rs:1414`; the crate's test `test_hyphenated_oov_preserves_unknown_marker_without_fallback`).
2. `chunks()` emits each block line of 20 bytes or more as its own chunk.
3. `synthesize` filters out `❓`. If the result is empty it returns `Err("The natural voice found no pronounceable text")`.
4. `run` propagates this with `?` at 824/827. The worker sends `{speaking:false, error}`.
5. `native-speech.ts` ends the session and reports the error. There is no skip and no system-voice fallback.
6. The same chunk fails on every retry, so the article cannot be read past that point with Kokoro.
7. A chunk whose tokens are all unresolved but space-separated passes only spaces to the engine. If `generate_with_config` returns `None` there, the same abort occurs through "The natural voice produced no audio" (inference).
Smallest correct fix: Have `synthesize` return a distinct "nothing to speak" outcome (for example `Result<Option<File>, String>`) for empty phonemes or no audio when not cancelled. In `run` and `Prefetch::finish`, advance to the next chunk (`index += 1; position = next start`) instead of returning `Err`.
Regression test: `kokoro.rs` tests. A pure test over the chunk/phoneme decision asserting that a `"中华人民共和国"` chunk between two English chunks yields "skip", not an error. `chunks()` and `transport_position` tests already exist to extend.

### FEAT-5 — `errorText` rewrites map and import failures into the Nearby-sharing message
Severity: Medium        Category: usability
Platforms: all
Evidence class: reproduced (regex extracted from `UI.tsx` and run against the literal backend strings in Node)
Location: `src/components/UI.tsx:137-138` (`errorText`) [+ `Maps.tsx:949,954`, `Downloads.tsx:1100`, `maps/partials.rs:196,209,236`, `imports.rs:35,150,730`]
Trigger: Any map or import error whose text contains checksum, verification, integrity, corrupt or "source changed".
Causal path:
1. `if (/checksum|corrupt|source changed|integrity|verification/i.test(message)) return t("The sender's archive failed verification. Stop sharing and start again, or download it again from the catalog.")`.
2. The check was written for LocalSwarm but runs for every caller.
3. These strings are all replaced by the "sender's archive… Stop sharing" sentence:
   - "Saved map prefix failed its checksum; delete the retained download or get the map again…"
   - "Map checkpoint moved or changed identity during verification…"
   - "Map file length changed during ownership verification"
   - "Archive verification queue is full"
   - "Archive verification was cancelled"
   - "Archive changed after verification; retry the import"
   - "Downloaded archive failed its advertised SHA-256 checksum; cancel and start a new download"
4. The user loses the actionable instruction (delete or retry) and is told to stop a share that does not exist. The map card renders "Map import failed: The sender's archive failed verification…".
Smallest correct fix: Scope the rewrite to LocalSwarm/Nearby call sites: either an explicit option or wrapper used by the sharing components, or require a LocalSwarm marker in the message. Leave other messages untouched.
Regression test: `scripts/test-frontend-surfaces.mjs` or a small `errorText` unit test asserting the map-prefix and "queue is full" strings pass through unchanged while a LocalSwarm verification error is still mapped.

### FEAT-6 — `map_forget_recovery` has no frontend caller: a recovery warning can never be dismissed and Delete always reports failure
Severity: Medium        Category: usability
Platforms: all
Evidence class: source-traced
Location: `src-tauri/src/service/dispatch.rs:2640-2647` (`map_forget_recovery`) [+ `maps/manager.rs:878-880`, `:900-903`, `:648-660`, `:915-935`; no reference in `src/`]
Trigger: Any job that carries `recovery_records`. Today that means a preserved unprovable partial, per `a_preserved_unprovable_partial_can_be_forgotten_without_deleting_it`. It would also be every case demoted by the FEAT-1 fix.
Causal path:
1. `status()` sets `can_remove_from_list=false` while `recovery_records` is non-empty.
2. `remove_terminal` refuses.
3. `delete_download` deletes what it owns, then always returns `Err("Map recovery warnings remain unresolved; this download stays visible and no unproven files were deleted")`.
4. The warning "Some earlier map downloads could not be cleaned up. The files were kept here: …" follows every later job.
5. The only exit is `forget_recovery`, exposed over IPC and tested in Rust, but `grep -rn map_forget_recovery src` finds nothing. The TypeScript `MapJob` type has no recovery-record field.
Smallest correct fix: Have `status()` expose `recoveryPaths: string[]` (the same `destination.join(name)` that `forget_recovery` compares). In the `Maps.tsx` and `Downloads.tsx` map cards, list each path with a "Dismiss (keeps the file)" button calling `map_forget_recovery {path, privacyGeneration}`. Make `delete_download` return `Ok` once everything it owns is gone.
Regression test: Frontend-surfaces test with fake IPC: a job with `recoveryPaths` renders the button and invokes `map_forget_recovery`. Rust test: after `forget_recovery`, `delete_download` returns `Ok` and `status()` is `None`.

### FEAT-7 — `MapJob.speed` / `etaSeconds` are computed and serialized but never shown, and the ETA is not an honest estimate
Severity: Low        Category: maintainability
Platforms: all
Evidence class: source-traced
Location: `src-tauri/src/maps/compose.rs:763-790` (`fetch_range` progress closure) [+ `maps.rs:263-266`, `src/types.ts:572-573`, `Maps.tsx:880-913`, `Downloads.tsx:1040-1070`]
Trigger: Any multi-GB map download.
Causal path:
1. The 2026-11-25 record lists "throttled `MapJob.speed` and `eta_seconds`" as delivered.
2. The only reader of `etaSeconds`/`speed` in `src/` is `downloadLiveSummary(job: DownloadJob)` at `Downloads.tsx:1187`, which is never applied to the map job.
3. The map card shows percent and block counts only.
4. The value itself is `eta = (total − written − incoming) / speed`: remaining output bytes divided by the current block's network speed. With a reusable local base, most output is copied locally, so the figure is far too high.
5. `speed`/`eta` are reset only on error (`compose.rs:674-679`), not when later blocks come from cache or a local candidate (`:553`, `:612`), so they go stale.
Smallest correct fix: Either render the rate (and ETA) on both map cards while `incomingTotal > 0`, and make it honest by clearing `speed`/`eta_seconds` when a block is served locally and deriving ETA from remaining blocks to download. Or delete the two fields from `MapJob`, `types.ts` and the review record.
Regression test: `maps_tests.rs`: after a locally served block, `status().speed == 0 && eta_seconds.is_none()`. Frontend-surfaces: the map card shows a rate when `speed > 0`.

### FEAT-8 — Map composition never holds the existing idle-sleep assertion; with a 3-second retry budget it fails when a Mac idle-sleeps
Severity: Low        Category: reliability
Platforms: macOS
Evidence class: source-traced. The gap is already listed in `docs/PLATFORM-GAPS.md` item 5; I am reporting it because a one-line source fix exists.
Location: `src-tauri/src/maps/manager.rs:1166` (`MapManager::run`) [+ `power_activity.rs:87`, `maps/compose.rs:635-690`]
Trigger: Start a multi-GB map on an unattended MacBook and let it idle-sleep.
Causal path:
1. `power_activity::Guard::acquire` is taken only in `localswarm/mod.rs:1074` and `localswarm/network.rs:1100`.
2. `run` holds only `MapWorkActivity`, an AtomicBool for iOS.
3. On wake, the in-flight range request errors. `ChunkReader::get` retries three times with 1 s and 2 s sleeps, then returns the raw reqwest error.
4. The job becomes `failed` (resumable) and waits for a manual Resume.
Smallest correct fix: `#[cfg(target_os = "macos")] let _power = crate::power_activity::Guard::acquire("Composing an offline map");` beside `_activity` in `run`. It releases on pause, cancel, finish or panic through RAII. Windows and Linux stay as documented.
Regression test: Expose a `#[cfg(test)]` holder count for `PROCESS`. A maps test asserts it is 1 while a composition is parked on the `before_worker`/`pause_after` hook and 0 after `finish`.

### FEAT-9 — Cancelling the voice-model download is shown as a red error alert
Severity: Low        Category: usability
Platforms: all with Kokoro
Evidence class: source-traced
Location: `src-tauri/src/kokoro.rs:428-456` (`finish_download`) [+ `:1093`, `service.rs:3306-3323`, `src/components/SpeechModels.tsx:94,176`, `KokoroDownloadPrompt.tsx:63,103`]
Trigger: Press "Cancel download" in Settings or the prompt, or switch to private/read-only mode during the download (`drain_download`).
Causal path:
1. `kokoro_cancel` cancels the token.
2. `download()` returns `Err("The voice model download was cancelled")`.
3. `finish_download` stores it: `state.error = Some(error.clone())`.
4. `status().error` is rendered by both components as `<p className="inline-error" role="alert">`.
5. The prompt filters only the thrown error (`/cancel/i`), not `status.error`.
6. The alert stays until the next download attempt or an app restart.
Smallest correct fix: In `finish_download`, do not record `state.error` when `download.cancel.cancel.is_cancelled()`. In `finish_voice_job`, emit `state: "cancelled"` instead of `"error"` for that case.
Regression test: `kokoro.rs` tests: `begin_download` → `cancel_download` → `finish_download(Err(cancelled))` → `status()["error"].is_null()`.

### FEAT-10 — The mobile "Print / save as PDF" row only calls `window.print()` in the article iframe, which the mobile webviews do not implement
Severity: Low        Category: usability
Platforms: Android, iOS
Evidence class: inference (webview behaviour not executed). The gap is already listed in `docs/PLATFORM-GAPS.md` item 8; I am reporting it because a one-line source fix exists.
Location: `src/components/Reader.tsx:4507-4511` (`printArticle`) [+ `Reader.tsx:5890-5908`, `src/reader-bridge.js:3770-3771`, `src/api.ts:38`]
Trigger: On a phone, open Reader → Print and export → "Print / save as PDF — Choose a printer or PDF in the system dialog."
Causal path:
1. `canPrintArticlePdf = isDesktop`, so on mobile `printArticle` sends `{command:"print"}`.
2. The bridge runs `window.print()`.
3. The vendored wry 0.57.1 has no iOS print UI delegate and no Android print hook; the only print code is macOS `printOperationWithPrintInfo` in `wkwebview/mod.rs:975-993`.
4. Nothing opens and no message is shown.
5. The row directly below already reaches the native path: Android system print, iOS document picker.
Smallest correct fix: Render the first row only when `canPrintArticlePdf || !isNative`, so desktop and the browser preview keep it and mobile is left with the working row.
Regression test: `scripts/test-frontend-surfaces.mjs`: with a mobile platform flag, the Print and export section has no button whose handler posts `print`.

**Covered** (read end to end unless noted)
- Maps: `maps/manager.rs`, `maps/compose.rs`, `maps/partials.rs`, `maps.rs` (evaluation module skimmed), `map_chunk_cache.rs` lines 1-330, `Maps.tsx`, `MapBlockProgress.tsx`, the `map_*` and `prepare_download_cleanup` dispatch handlers, the map handlers in `Downloads.tsx`, the `maps_tests.rs` test list and about ten tests.
- Kokoro/speech: `kokoro.rs` (non-test part), `kokoro/audio_cache.rs`, `service/android_voice_pack.rs`, the `kokoro_*` handlers, `SpeechModels.tsx`, `KokoroDownloadPrompt.tsx`, `useVoiceModelStatus.ts`, `native-speech.ts`, `speech-*.ts`, `mobile_speech.rs` (state machine, mobile start/event/action, desktop transport), speech extraction in `reader-bridge.js`.
- PDF/print/export/share: `native_pdf.rs`, `native_pdf/cleanup.rs`, `native_pdf/linux.rs`, `native_pdf/windows.rs`, `live.rs::document`, `pdf_export.rs` lines 1-330, `share.rs`, the output/publish section of `exports.rs`, the export/print handlers in `Reader.tsx`, the save/share/print functions in `api.ts`, `photo_capture.rs`, `camera.ts`.
- Location: `permissions.rs` location section, `location_apple.rs`, the sensor request flow in `Reader.tsx`, `sensor-policy.ts`, the `native_location` gating in `build.rs`. Nothing met the bar at the depth read. The Linux and Android providers are still not compiled, as PLATFORM-GAPS documents.
- Notifications/power/disk: `power_activity.rs`, `notifications_service.rs`, `notifications.rs` lines 1-140, `disk_warning.rs` lines 1-140. Nothing met the bar beyond FEAT-8; the permission-denied path is handled in `Settings.tsx`.

**Not covered**
- `maps/recipe.rs`.
- `kokoro/install.rs` (function and test list only).
- `native_pdf/apple.rs`, `android.rs`, `handoff.rs`, `presentation.rs`, `windows_print.rs`, `origin_tests.rs`; the `live.rs` sanitizer; `pdf_export.rs` after line 330 (iOS picker retirement).
- MHTML/HTML reference parsing in `exports.rs`.
- Bodies of `location_linux.rs`, `location_android.rs`, `location_windows.rs`.
- The rest of `notifications.rs`, and all Objective-C++/Swift/Kotlin counterparts (`native/*.mm`, speech, PDF, location, notification plugins).
- Second half of `disk_warning.rs`; `camera.html`.
- `docs/STREETZIM-*.md`, `docs/KOKORO.md` (grep only), `docs/PHOTO-CHOOSER.md`, `docs/ANDROID-PDF-SHARE-TEST.md`.
- Tail and tests of `map_chunk_cache.rs`.
- No Rust test was executed, so FEAT-1, 2, 3, 4, 6, 7, 8 and 9 rest on source tracing, not a run.