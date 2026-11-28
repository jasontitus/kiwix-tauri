# Findings ledger (working notes; base d98cac2)

Status keys: TODO / FIXED / REVIEWED / DEFER(reason)

## Environment
- Baseline: npm build ok; node tests 399 pass/3 skip; py 182 ok; mcp clippy+9 tests ok; linux clippy ok; win-gnu clippy ok; win-security clippy ok.
- cargo test --lib (linux, no IPv6 kernel): 1329 pass, 5 fail (all torrent "error starting listeners" => OWN-1), 16 ignored.
- Env: export SHERPA_ONNX_ARCHIVE_DIR=$S/sherpa ; swiftc at $S/swift/swift-6.0.3-RELEASE-ubuntu24.04/usr/bin/swiftc (-parse only)
- No Android SDK/NDK. add_repo openzim/libzim denied (not needed).

## OWN
- OWN-1 TODO torrent engine binds [::] only; no IPv4 fallback on IPv6-less hosts (torrent/manager.rs engine()). Fix: probe, set ipv4_only on SessionOptions+ListenerOptions.

## APPLE (report received)
- APPLE-1 Med iOS linked archives rejected when fileResourceIdentifier changes (DocumentStorage.swift acquire ~1037)
- APPLE-2 Med macOS sandbox dead bookmark never retried (sandbox.rs restore/grant)
- APPLE-3 Med macOS ⌘M conflict (reader-toc vs Minimize), app menu only Quit; Settings F12, Fullscreen F11
- APPLE-4 Med Download Live Activity ignores isStale
- APPLE-5 Med PrivacyInfo.xcprivacy lacks UserDefaults category (CA92.1)
- APPLE-6 Low Read-aloud Live Activity hard-coded English + missing a11y labels
- APPLE-7 Low iOS hardware Escape swallowed outside reader (App.tsx 2846-2866)
- APPLE-8 Low persistFolderBookmark writes unused bookmark; failure fails a committed import
- APPLE-9 Low iOS XCTest suite not in CI

## SHELL (report received)
- SHELL-1 Med errorText regex rewrites any checksum/verification msg to "sender's archive" (UI.tsx:137)
- SHELL-2 Med errors from modal actions only go to global band under the modal
- SHELL-3 Med Catalog/Maps card-level errors rendered at page top off-screen
- SHELL-4 Med Android Back never handled by shell
- SHELL-5 Med Share as BitTorrent: 180 s poll cap false failure (Library.tsx:599-613)
- SHELL-6 Low download directory text field debounce opens confirm modal mid-typing (SettingsAutosave.tsx:164)
- SHELL-7 Low user cancellations painted as errors (update cancel; Download cancelled double)
- SHELL-8 Low 64 UI strings uncollected in localization inventory; no CI gate; plural "Used today: {count} requests"
- SHELL-9 Low mobile Settings/Help mention File menu/New window
- SHELL-10 Low UpdatesSettings listener leak if cleanup before listen resolves; installing never cleared
- SHELL-11 Low camera window dead-end on capture-complete reject; locale failure blocks start

## SERVE (report received)
- SERVE-1 High cached archive handle gone bad never dropped (service.rs archive_for_book ~1598; dispatch library_maintenance ~1842; archive_search Parts::unchanged). Fix: liveness probe on cache hit; invalidate+retire on availability flip.
- SERVE-2 Med Enter <120ms after typing: pending suggestion timer cancels submitted search (Library.tsx 396-411, 729-761, 793-829)
- SERVE-3 Med spelling cache dir never GC'd; .previous backups + .tmp linger (spelling.rs publish_index 1027-1083)
- SERVE-4 Low non-404 document navigation failures return uninstrumented text/plain (server.rs 790-796, 885-894, 976-983)
- SERVE-5 Low Library search ignores corrections/spellingStatus (Library.tsx 1454-1489)
- SERVE-6 Low dead libzim SDK glue native/mobile_sdk.rs, desktop_sdk.rs; docs/FORKS.md:30 stale
- SERVE-7 Low fuzz target never decodes cluster / no xapian target

## ACQ (report received)
- ACQ-1 High web-seed stage truncates valid partial torrent payload on restart after engine sized file (web_seed.rs:399-440,478-483)
- ACQ-2 Med segmented retry budget never resets on progress (segments.rs:213; cf transfer.rs:543)
- ACQ-3 Med iOS official-catalog downloads named <name>.zim => flavours collide (dispatch.rs:2332-2336)
- ACQ-4 Med web seed 3 attempts no backoff; stale `current` on Err arm (web_seed.rs:406-424,485-519)
- ACQ-5 = SHELL-1 (errorText regex)
- ACQ-6 Med Metalink mirror pinned; Retry never re-resolves (transfer.rs:69-83; queue_resume)
- ACQ-7 Low Catalog installedEdition compares raw date strings, ignores flavour (Catalog.tsx:83-97)
- ACQ-8 Low interrupted/failed rows hide progress (Downloads.tsx:1187-1189,1245)
- ACQ-9 Low Android foreground service dropped while torrent share "creating" (mobile_service.rs:2080)
- ACQ-10 Low complete single-stream part cannot finalize: 416 w/o validator (transfer.rs:287-300)
- ACQ-11 Low OPDS display length enforced as exact size for non-metalink (Catalog.tsx:418-422; manager.rs:776,875)

## LIB (report received)
- LIB-1 High rename_noreplace_at no fallback on NFS/FUSE EINVAL (publication.rs:563-588 vs 669-695 platform_publish which has fallback); consumers finalize.rs:199, library_files.rs:2392 move_exclusive, library_root.rs:974 publish_exclusive
- LIB-2 High Android device scan aborts when ZIM at volume root (location "") or dot-named (device_scan.rs:92-101,155-158,181; DeviceZimScan.kt walk)
- LIB-3 Med opening already-imported archive re-runs full integrity (service.rs:1694-1721 import_paths)
- LIB-4 Med open-with/CLI errors only eprintln (lib.rs:668-690 open_paths)
- LIB-5 Med desktop move previous downloads: no space check; unfinishable journals retried forever (library_files.rs:1624-1807; dispatch.rs:1797-1816)
- LIB-6 Med superseded edition original cannot be deleted in app (edition_updates.rs:627-664; Downloads.tsx:1489-1509)
- LIB-7 Med monitored folder as empty mount point retires all books (service.rs:1968-2051)
- LIB-8 Med mobile managed-copy import of book whose existing entry unavailable deletes fresh copy; batch admitted unverified (service.rs:2179-2219; mobile_service.rs:1094-1121)
- LIB-9 Low Linux trash hidden prefix; cross-device copy to home trash; misleading "recoverable at" (library_files.rs:804,874-882)
- LIB-10 Low unsafe-name archives can't be deleted/revealed; unhelpful error (library_files.rs:172-209,556,746)

## CORE (report received)  [note: reviewer accidentally regenerated docs/ERRORS.md then restored; git status verified clean]
- CORE-1 High Quit cannot complete when profile unwritable; dialog only "Retry" (quit.rs:306,147-156; lib.rs:1282-1307)
- CORE-2 High fatal desktop startup errors shown to nobody (lib.rs:1399-1405,1113; main.rs windows_subsystem)
- CORE-3 Med localswarm_discovery nests privacy read lease on mobile => deadlock w/ queued writer (dispatch.rs:2099-2109; nearby_service.rs:450)
- CORE-4 Med skipped profile records permanently deleted by next write; no desktop warning (store.rs:911-938,1242,2888-3006; service.rs:746-765); mobile quarantines on version mismatch (store.rs:1798-1815)
- CORE-5 = LIB-7 (empty mount point retires all books) (service.rs:1968-2050)
- CORE-6 Med manual import verify uncancellable under privacy read lease; Quit freezes (service.rs:1694-1722; imports.rs:47-63)
- CORE-7 = LIB-4 (open_paths errors eprintln only) + macOS Opened handler (lib.rs:1353-1361)
- CORE-8 Low crash path redaction stops at first space (crash.rs:527-556 path_at)
- CORE-9 Low map_forget_recovery no caller => recovery record unclearable; 4 dead arms torrent_limits, sharing_status, localswarm_hotspot_start/stop (dispatch.rs:2640,2486,2813,2081/2090)
- CORE-10 Low docs/ERRORS.md stale; generator has no --check, writes on any args
- CORE-11 Low docs/STATE_MACHINES.md §1.2/§1.4 stale

## FEAT (report received)
- FEAT-1 High resumable map whose partial can't be reclaimed blocks all map downloads (maps/manager.rs:397-403 start guard; 832-886 delete_download; 712-737 reclaim_job_partial; 662-710 cancel; Maps.tsx:972 copy). Fix: demote to recovery_records, clear resume.
- FEAT-2 Med map compose: tokio File write errors deferred => checkpoint past bytes on disk (manager.rs:1400,1413,1428-1441; 1068-1079 checkpoint_output). Fix: flush after write_all.
- FEAT-3 Med article-pdf scratch never swept after crash (native_pdf/cleanup.rs; pdf_export.rs:218-230)
- FEAT-4 Med one unpronounceable chunk ends Kokoro read-aloud (kokoro.rs:679-692, 824-828)
- FEAT-5 = SHELL-1
- FEAT-6 Med map_forget_recovery no frontend caller (=CORE-9); delete_download always Err with recovery records
- FEAT-7 Low MapJob.speed/etaSeconds never shown, ETA dishonest (compose.rs:763-790)
- FEAT-8 Low map composition holds no macOS idle-sleep assertion (manager.rs:1166 run)
- FEAT-9 Low cancelling voice-model download shown as red error (kokoro.rs:428-456 finish_download)
- FEAT-10 Low mobile "Print / save as PDF" row calls window.print() which does nothing (Reader.tsx:4507-4511, 5890-5908)

## BUILD (report received)
- BUILD-1 = CORE-2 (startup errors invisible); also --help/--version invisible on Windows release
- BUILD-2 Med Windows self-update exits inside plugin, skipping quit flush (update.rs:972-1001; updater_for 451-470 on_before_exit)
- BUILD-3 Med real-browser UI tests skipped everywhere (KIWIX_TEST_CHROME) (test-frontend-surfaces.mjs:9, test-startup-navigation.mjs:9; checks.yml:51)
- BUILD-4 Med Rust unit tests never run on PRs; Windows tests never run; clippy lacks --tests on linux/android; test-linux-appimage.py not in discover
- BUILD-5 Med location_linux/android never compiled; zbus still built on Linux; DEPENDENCIES.md:96,127 false
- BUILD-6 Low doc contradictions PACKAGING/UPDATE/DEV-ITERATION/PLATFORM-GAPS (8 items)
- BUILD-7 Low Windows minimized window geometry overwritten (windows.rs:461-478 capture)
- BUILD-8 Low busy-profile dialog says "Activity Monitor" on Linux/Windows (instance.rs:101-114)
- BUILD-9 Low future lastChecked suppresses update checks (update.rs:370-384)
- BUILD-10 = CORE-10 (ERRORS.md stale, no --check)
- BUILD-11 Low floating rust toolchain / mutable action refs
- BUILD-12 Low Windows state.json replace no transient retry (store.rs:3551-3555; storage.rs retry_transient_sharing); update.rs:351-355 dead remove-then-rename

## NET (report received)
- NET-1 High default web-sharing binding picks docker/WSL/VPN adapter (sharing.rs:194-205; localswarm/network.rs:1135-1155; transfer_policy.rs:159-185; hotspot.rs:148-164) three diverged interface filters
- NET-2 High Bluetooth off/denied: Nearby fails silently (nearby_service.rs:356-385,459; LocalSwarmControls.tsx:514,550,580; types.ts:521)
- NET-3 Med native error-code contract drift: "[CODE] - CODE" wrapping, prefixes, ~90 unmapped codes (UI.tsx:54-136)
- NET-4 Med Share all stops silently when shared archive changes; Android hotspot left up (localswarm/library.rs:114-132; mod.rs:797-864,1482)
- NET-5 Med sharing addresses start-time snapshot; no addressLost (sharing.rs:1705-1845)
- NET-6 Med after discovery failure "Find nearby" cannot restart (ios_discovery.rs:449-603; discovery.rs:175; mod.rs:741-747)
- NET-7 Med private sync invitation cancelled silently on mobile background (PrivateReadingSync.tsx:218,277)
- NET-8 Low closing connect dialog doesn't cancel pending BT/WiFi join (LocalSwarmControls.tsx:243-252)
- NET-9 Low shared-library receipt TTL 5 min => false "changed or disconnected" (shared_library.rs:24,160)
- NET-10 Low directConnection dead setting

## PHASE 2 — implementation lanes (shared tree, file ownership; brief = scratchpad/IMPLEMENT-BRIEF.md; reports in scratchpad/reports/<AREA>.md)
Launched (wave 1, Rust):
- transfers: OWN-1, ACQ-1,2,3,4,6,9,10,11  (owns torrent*, acquisition*)
- features: FEAT-1,2,3,4,6(backend),7(backend),8,9 (owns maps*, kokoro*, native_pdf*, pdf_export)
- files: LIB-1,2(rust),5,6(backend),9,10 (owns publication, library_files, library_root, device_scan, edition_*)
Background: Android SDK/unit-test setup agent (scratchpad/android/)
Pending reviewer reports: READER, ANDROID
Planned (wave 2, Rust): 
- startup/quit: CORE-1, CORE-2/BUILD-1, CORE-7/LIB-4, CORE-8, BUILD-8 (lib.rs, quit.rs, instance.rs, crash.rs)
- desktop-misc: CORE-4, BUILD-12 (store.rs), BUILD-2, BUILD-9 (update.rs), BUILD-7 (windows.rs), APPLE-2 (sandbox.rs), APPLE-3 (menus.rs + App.tsx labels)
- service: SERVE-1, SERVE-3, SERVE-4, LIB-3, LIB-7/CORE-5, LIB-8, CORE-6, CORE-9 dead arms (service.rs, dispatch.rs, imports.rs, spelling.rs, server.rs)
- net: NET-1,2(backend),4,5,6,9,10, CORE-3 (sharing.rs, localswarm/*, nearby_service.rs, shared_library.rs, transfer_policy.rs, hotspot.rs)
Planned (frontend, no cargo):
- F1 shell: SHELL-1/ACQ-5/FEAT-5, NET-3, SHELL-2,3,4,6,7,9,10,11, APPLE-7
- F2 panes: SHELL-5, SERVE-2, SERVE-5, ACQ-7, ACQ-8, LIB-6 UI, FEAT-1 copy, FEAT-6 UI, FEAT-7 UI, FEAT-10, NET-2/5/7/8 UI
Planned (native): Swift: APPLE-1,4,5,6,8 ; Kotlin: LIB-2 walk + ANDROID findings
Planned (CI/docs, me or agent): BUILD-3,4,5,6,10/CORE-10,11, CORE-11, SERVE-6,7, APPLE-9, SHELL-8 (localization generate at END)
After each wave: independent adversarial review per lane -> fix -> re-review. Then full suites, review record docs/reviews/2026-11-27/, commit, push.

## READER (report received; reports/READER.md)
- READER-1 High restricted mode / Reload article don't reload when URL has fragment (Reader.tsx:4491,5646,2140,2005). Fix: retireReaderDocuments + setFrame(null) before setReload (like changeSensorDecision)
- READER-2 High WebKit fires iframe load on fragment nav => re-session per anchor click (Reader.tsx:6547-6669; reader-bridge.js:3886). Fix: re-session only when reading-gesture-hello seen since last load
- READER-3 Med blink on child navigation (is-pending at load); clicks before load swallowed
- READER-4 Med desktop system-voice read-aloud cancelled on visibility hidden (reader-bridge.js:4229)
- READER-5 Low link preview fires for same-page anchors (reader-bridge.js:4037; Reader.tsx:3443)
- READER-6 Low per-tab back/forward stacks unbounded (Reader.tsx:2406,3542,2498); photoSecrets never pruned
- READER-7 Low tel: link raises blocking error (bridge allow-list vs shell validExternal drift)
- READER-8 = FEAT-10 mobile Print row calls window.print()
- READER-9 Low dead handlers: bridge case "selection"; kiwix-storage-closed (reader-storage.ts:690) + test
- READER-10 Low TOC blank unnamed buttons (reader-bridge.js:1409; Reader.tsx:3588)
Wave status: launched also `shell` (frontend F1) and `apple` (Swift) lanes.

## ANDROID (report received; reports/ANDROID.md) — ALL 11 REVIEW REPORTS NOW IN
- ANDROID-1 High failed copy-import leaves staged multi-GB files forever (KiwixMobilePlugin.kt:1467 fail, 1228 launchWorker; OwnedDocuments.kt:248; mobile_service.rs:1229,1301; library_files.rs:948 remove_empty_import_directory)
- ANDROID-2 Med launch intent replayed after process death (KiwixMobilePlugin.kt:342-360 load; MainActivity.kt:36)
- ANDROID-3 = SHELL-4 Android Back
- ANDROID-4 Med raw "[CODE] - {json}" / java exception text reaches user (user_error.rs:81-101,147-158; UI.tsx:120,140)
- ANDROID-5 Med configChanges lacks navigation|layoutDirection => recreate cancels imports silently (AndroidManifest.xml:30; retireWindowUi 484-510)
- ANDROID-6 Med "Move: remove originals" can never succeed for provider archives (SelectedStorageActions.tsx:102)
- ANDROID-7 Med KiwixTransferService may stop before startForeground (KiwixTransferService.kt:168-183,226)
- ANDROID-8 Low fontScale->data-large-text wiring (avoid double scaling)
- ANDROID-9 Low status/nav bar strips ignore in-app theme
- ANDROID-10 Low system-TTS read-aloud not paused on headphones unplug (SpeechPlayback.kt:112-130)
- ANDROID-11 Low IpPrefix ctor API 33 unguarded (LocalHotspot.kt:90, NearbyRoutes.kt:62); dead nearbyRoute commands
- ANDROID-12 Low Kotlin-composed English sentences bypass localisation (DeviceZimScan.kt; SpeechPlayback ensureChannel)
- (unverified note) R8 keep rule for native methods in consumer-rules.pro
Env additions: Kotlin compile+42 unit tests: see scratchpad/ANDROID-ENV.md. Android Rust type-check: scratchpad/android-clippy.sh (clean on in-progress tree at 23:30). Real-browser tests: KIWIX_TEST_CHROME=scratchpad/chrome-nosandbox.sh (7 pass baseline). Checkpoint helper: scratchpad/checkpoint.sh (stop hook requires commit+push of WIP).
Lanes running: transfers(a10ec…), features(a413…), files(a5327…), shell(a5b1f…), apple(a53fb…), reader(a5578…), android(afedc…).
Launched wave 1b: net (a5a56…: NET-1,2,4,5,6,7,8,9,10 + CORE-3; owns sharing*, localswarm/*, nearby*, private_sync*, transfer_policy, hotspot, SharingControls/LocalSwarmControls/PrivateReadingSync), startup (a38ef…: CORE-1, CORE-2/BUILD-1, CORE-7/LIB-4, CORE-8, BUILD-8; owns lib.rs, quit.rs, instance*, crash.rs, STATE_MACHINES §1.1)
android lane (afedc…): ANDROID-1,2,4(rust),5,7,8(native+ts; App.tsx handoff),9,10,11,12(part), LIB-2 kotlin, R8 note
STILL TO LAUNCH: 
- service lane (after files lane done): SERVE-1, SERVE-3, SERVE-4, LIB-3, LIB-7/CORE-5, LIB-8, CORE-6, CORE-9 dead arms, FEAT-3 startup sweep handoff (service.rs, dispatch.rs, imports.rs, spelling.rs, server.rs, archive.rs)
- desktop lane (after files lane done): CORE-4, BUILD-12 (store.rs), BUILD-2, BUILD-9 (update.rs), BUILD-7 (windows.rs), APPLE-2 (sandbox.rs), APPLE-3 (menus.rs + App.tsx nativeMenuLabels)
- panes lane F2 (after shell+features+files done): SHELL-3, SHELL-5, SERVE-2, SERVE-5, ACQ-7, ACQ-8, LIB-6 UI, FEAT-1 copy/FEAT-6 UI/FEAT-7 UI, ANDROID-6, ANDROID-12 UI, ANDROID-8 App.tsx handoff
- CI/docs lane: BUILD-3,4,5,6,11; CORE-10/BUILD-10 (--check), CORE-11, SERVE-6, SERVE-7, APPLE-9, SHELL-8 gate; then I regenerate ERRORS.md + localization
Launched cidocs lane (ae9fa…): BUILD-3,4,6,11(doc only), CORE-10, CORE-11, SERVE-7, APPLE-9, SHELL-8 gate. Owner-at-end: BUILD-5, SERVE-6, regenerate ERRORS.md, localization generate.

## PHASE 3 — adversarial reviews (brief: scratchpad/ADVERSARIAL-BRIEF.md; impl reports: scratchpad/impl/<lane>.md; save with scratchpad/save-report.py <taskid> <out>)
- features lane DONE (impl/features.md): FEAT-1,2,3,4,6,7(ETA dropped),8,9 fixed. Handoffs: service.rs startup sweep_stale call (FEAT-3); finish_voice_job emit "cancelled" (optional); docs PLATFORM-GAPS item 5, STATE_MACHINES:255 eta; frontend: recoveryPaths Dismiss button, re-read map_status after Delete (Maps.tsx setJob(null)), rate display, Maps.tsx:972 copy. Removed error strings. Doubts: demotion on non-user cancel paths; FEAT-3 cross-instance; FEAT-4 backward skip unexercised.
  -> adversarial reviewer launched (a43b0…)
- apple lane DONE (impl/apple.md): APPLE-5 fixed (+test-privacy-manifest.mjs), APPLE-8 fixed (+cleanup of stale bookmarks dir), APPLE-1 fixed DIFFERENTLY (fallback identity = volume UUID+inode+mtime when resource identifier differs; heals record) — authority-check change, needs hard review; APPLE-4 partial (isStale presentation, no localized stale label), APPLE-6 partial (labels via attributes; "% read" English). mobile_speech.rs +"labels" line. Swift syntax-only (+ Linux corelibs harness in scratchpad/apple/).
  -> adversarial reviewer launching
- desktop lane launched (af117…): CORE-4, BUILD-12, BUILD-2, BUILD-9, BUILD-7, APPLE-2, APPLE-3 (owns update.rs, windows.rs, sandbox.rs, menus.rs; store.rs load path; App.tsx nativeMenuLabels). Model switched to Opus 5.5 by user; checkpoint attribution updated.
- DURABILITY: planning state synced to docs/reviews/2026-11-27/working/ on every checkpoint (sync-working.sh); RESUME.md written; auto-checkpoint loop every 10 min (bg b2pot9yel, 2h limit; re-arm when it ends). Save adversarial reports to scratchpad/adv/<lane>.md.
- 23:48 UTC: rate limit (Fable) stopped lanes: cidocs (ae9fa…), files (a5327…), startup (a38ef…), reader (a5578…). Resume each via SendMessage after 00:00 UTC reset. Others may follow.
- also stopped: adv-apple (a3939…), android (afedc…).
- also stopped: shell (a5b1f…).
- also stopped: net (a5a56…).
- also stopped: transfers (a10ec…).
- also stopped: adv-features (a43b0…). Only desktop (af117…, launched after model switch) still running.
- 00:02 UTC: all 10 stopped agents resumed via SendMessage (transfers, files, startup, reader, android, shell, net, cidocs, adv-apple, adv-features). desktop still running.
- adv-features DONE (adv/features.md): BLOCKING R1 (demotion irreversible on retryable failures; Delete Ok). Should-fix R2 (startup sweep handoff -> service lane), R3 (UI half -> panes lane), R4 (sync failure should not checkpoint current). Nits R5 (carry reclaim error; drop record if no file), R6 (FEAT-8 test can't fail), R7 docs (STATE_MACHINES:255 eta, PLATFORM-GAPS item 5 -> owner at end).
  Owner decisions: R1 adopt reviewer fix 1+2+3 (re-reclaim proven recovery records on explicit Delete; Err on the demoting call; keep error). FEAT-3 STALE_AFTER keep 1h. FEAT-8 option set keep (same as localswarm), residual.
  -> features implementer resumed for R1,R4,R5,R6
- reader lane DONE (impl/reader.md): all 10; READER-2 fixed DIFFERENTLY (nonce probe/document-proof with bridge bootstrap token; 1s timeout); READER-3 partial; READER-8 hides print row on all mobile; new string "Printing is not available on this device. Save or share the article as a PDF instead."; handoffs: localization generate; PLATFORM-GAPS item 8 stale. Pre-existing residual: reader-init posted to bridge-less doc at load.
  -> adversarial reviewer launching
- cidocs lane DONE (impl/cidocs.md): BUILD-3 (CI chrome wrapper; CI fails if missing), BUILD-4 (linux-tests job; clippy --tests; appimage test; Windows test step unexercised), CORE-10 (--check, line numbers dropped, sorted), SHELL-8 (inventory gate + allow-list; i18n.tsx literals now collected), BUILD-6 (8 doc fixes), CORE-11 (lock order: update_admission -> transfer_gate -> settings_writes -> privacy_gate), SERVE-7 (fuzz targets, type-checked only), APPLE-9 doc, BUILD-11 documented residual.
  END-OF-RUN (owner): plural-sources.json add "Used today: {count} requests" one/other; npm run docs:errors; localization generate (plural key reorder across Node versions! try other node to match committed order); check:errors/check:localization; npm run build.
  Watch: test-shell-surfaces.mjs / test-nearby-sharing-surfaces.mjs fail 7/8 with browser (lanes in progress / load ~10) — now gating.
  -> adversarial reviewer launching
- adv-apple DONE (adv/apple.md): BLOCKING R1 (fallback re-admits archive modified while removable drive away). Should-fix R2 (provider stops exposing identifier), R3 (stale label). Nits R4 (accept), R5 (read-aloud stale window), R6 (privacy test min set). Pre-existing: digestSatisfied ignores differing digest when sameFile.
  Owner decisions: R1 require volumeIsInternal==true (fail closed; removable keeps Locate). R2 adopt. R3 add optional staleLabel if Rust side compiles somewhere; else keep partial. R5 lengthen read-aloud stale window to 10 min. R6 adopt. digestSatisfied: refuse when a recomputed digest differs (strengthening) after verifying. Doubt 6 (rebind write in read-only/private) accepted, same class as stale-bookmark refresh.
  -> apple implementer resumed
- transfers lane DONE (impl/transfers.md): OWN-1 app fix + VENDOR handoff applied by owner (vendor/librqbit-dualstack-sockets/src/socket.rs IPv4 fallback for dualstack [::] when IPv6 socket cannot be created; docs/FORKS.md note). ACQ-1 fixed differently (piece-range repair, no truncation), ACQ-2,4,6(partial),10,11 fixed; ACQ-3,9 not compiled. Handoff: docs/WEB-SEEDS.md stale (flow 4-5, residual bullet) -> owner end. Residual: peerless after web seed gives up shows 0 B/s no error.
  -> adversarial reviewer launching (includes vendor patch)
- android lane DONE (impl/android.md): ANDROID-1,2,7,9,10 fixed; ANDROID-4 rust half (user_error.rs table; tests written, owner must run cargo test user_error); ANDROID-5 partial (navigation only; ACTIVITY_RECREATED code); ANDROID-8 native+ts+css (App.tsx H1); ANDROID-11 ctor fixed, dead cmds kept; ANDROID-12 channel half; LIB-2 kotlin; R8 keep rule. NOTE checkpoint 08f3094 had mutated Kotlin (fixed since).
  Handoffs: H1 App.tsx isAndroid dynamic-type (-> shell lane), H2 ACTIVITY_RECREATED startup warning (mobile_service.rs -> service lane/owner), H3 kiwix-mobile-error plain() (mobile.rs -> service lane/owner), H4 device-scan codes (3-part; residual unless panes lane), H5 moveTaskToBack for Back root (-> shell lane decision), H6 info, H7 optional.
  -> adversarial reviewer launching
- shell lane DONE (impl/shell.md): SHELL-1 (explicit list of 4 sender producers), NET-3 (nativeCodeText), SHELL-2 (alertdialog over modal), SHELL-4+H5 (listener only while something to dismiss; root = plugin default), APPLE-7, SHELL-6, SHELL-7 partial, SHELL-9, 10, 11, H1. 8 new t() strings. New files src/shell-dismiss.ts, src/android-back.ts, tests test-shell-dismiss/-shell-surfaces/-settings-surface/-updates-settings/-camera-window.
  Handoffs -> panes lane: diagnostics.ts reportError ignore UserCancelledError; Downloads.tsx start() single owner + map error double owner (~880). Docs owner: PLATFORM-GAPS/RELIABILITY Back behaviour.
  -> adversarial reviewer launching
- apple round 2 DONE (impl/apple-round2.md): R1 volumeIsInternal, R2, digestSatisfied fix, R5 600s, R6, R3 staleLabel ("Open Kiwix to see progress" via live_activity.rs, cfg(any(ios,test)) -> owner run cargo test live_activity). END-OF-RUN: add Rust-only strings to src/locales/extra-en-*.json (Open Kiwix to see progress; status labels Reading aloud, Fetching metadata, Resolving, Connecting, Checking).
  -> adv-apple re-review requested
- adv-cidocs DONE (adv/cidocs.md): BLOCKING R1 (linux-tests job lacks npm ci; live.rs test needs node+typescript). Should-fix R2 (CLDR plural order sort), R3 (error-inventory Windows separators), R4 (map_recipe_metadata re-check doc). Nits R5, R6, R7 (paths include docs/ERRORS.md), R8 (shell surface test flaky -> shell lane; split frontend build job optional).
  Owner: adopt all R1-R7; R8 split frontend build into own job = yes; flaky test -> shell lane.
  -> cidocs implementer resumed
- adv-reader DONE (adv/reader.md): no blocking. R1 should-fix (archive script hooks MessageEvent.prototype.data -> steals reader-init session + photo secret, binds own token; pre-existing leak of photo secret). R2 held paint for unadmitted/foreign doc 21s. R3 Reload copy.
  Owner: R1 fix all 4 items + require a hello before ANY reader-init (never init a bridge-less doc; timeout w/o hello = not admitted, hidden). R2 adopt. R3 reword failure on Reload path.
  -> reader implementer resumed
- adv-apple round 2 (adv/apple-round2.md): no blocking. LANE apple CLOSED with residuals:
  APPLE-1 = mitigation, unverified on device (depends on volumeIsInternal==true on iPhone data volume; fail-closed); external drive replug deliberately still needs Locate. R8 localization of "Open Kiwix to see progress" (end-of-run extra-en). Nits accepted: R4, R9 (expanded island no stale text), R10 (positive XCTests assume host volume internal). New residual finding: non-stale reopen for identifier-less providers checks only size + first-part UUID (pre-existing; rehash per open too costly) -> document.
- adv-transfers DONE (adv/transfers.md): OWN-1 VERIFIED (all 5 torrent tests pass; torrent+acquisition 210/0). R1 (owner raises to BLOCKING: truncated close-delimited body published complete; fix ZIM header checksum_pos+16==size before publish), R2 (complete_part w/o magic), R3 (web-seed stall reset by tiny progress; connect timeout), R4 WEB-SEEDS.md, R5 errno-specific fallback (vendor + probe), R6 test can't fail.
  -> transfers implementer resumed (granted docs/WEB-SEEDS.md + vendor socket.rs errno edit)
- cidocs round 2 DONE (impl/cidocs-round2.md): R1-R8 fixed (frontend-build job split; paths filter). -> adv-cidocs re-review requested
- net lane DONE (impl/net.md): NET-1 (lan_interface classifier + routed probe), NET-2 (finder error top-level; publisher in share card), NET-4 (library_stopped slot; Android hotspot waiter), NET-5 (addressLost; wildcard urls refresh; 4s poll), NET-6, NET-7, NET-8 (pendingOnly stop), NET-9, CORE-3 fixed; NET-10 residual (owner decision: keep residual until testable on Apple). 9 new t() strings + 1 Rust string. new test-nearby-sharing-surfaces.mjs.
  -> adversarial reviewer launching
- adv-cidocs round 2 (adv/cidocs-round2.md): R1-R8 closed. LANE cidocs CLOSED. Nits accepted: R9 (call-site not asserted), R10 (job name kept for possible required check). Unverified: Actions run, paths filter on GitHub, Windows host.
- adv-android DONE (adv/android.md): no blocking. Should-fix R1 (peek on worker -> false 4s timeout pauses all), R2 (export recreated leaks prepared file), R4 (stopSelf(startId)), R5 (unknown codes -> bare code so localized frontend fallback), R3/R6 nits adopt; R7 nits: use hiddenScanName at inline sites only.
  Owner: adopt R1-R6 (+R7 hiddenScanName). Residuals: onNewIntent-before-plugin-load loss (pre-existing), layoutDirection not handled, Rust sentence table English-only (better than raw JSON), dead nearbyRoute commands.
  -> android implementer resumed
- adv-shell DONE (adv/shell.md): no blocking. R1 (double owner inline+alert at Library.tsx:530,561; Downloads.tsx:807,881; Maps.tsx:745 -> PANES lane: onError(e,"blocking",false)), R2 (gated dir field "Saving..." + quit drops typed value), R3, R4, R5 -> shell lane; R6 (plain() internal-terms swallows iOS BLE_*_RETIRED) -> android lane (user_error.rs). diagnostics.ts SHELL-7 handoff -> shell lane now.
- desktop lane DONE (impl/desktop.md): CORE-4 (+mobile version refusal), BUILD-12, BUILD-2 partial (hook: capture, session snapshot, flush_browsing, clear state; no shell flush), BUILD-9, BUILD-7, APPLE-2, APPLE-3 (⌥⌘T toc, ⌘, settings, ⌃⌘F fullscreen; app menu items; 5 new labels).
  Handoffs: (1) service_tests.rs assertion — APPLIED by owner. (2) dispatch library_maintenance sandbox::revive() -> service lane. (3) Reader.tsx shortcut table ⌃⌘F -> reader lane. (4) optional quit::abandon for Windows hook shell flush -> residual unless startup lane cheap.
  Doubts: read-only->writable same session drops skipped records (residual); mobile Reset window state on newer-version profile.
  -> adversarial reviewer launching
- android round 2 DONE (impl/android-round2.md): R1-R7 + SHELL-R6 fixed. Residual: stopOwned->stopService under pending start (same platform rule) ; R2 replaced save silent (= base). -> adv-android re-review
- features round 2 DONE (impl/features-round2.md): R1 (retry proven records on Delete; Err on demote & while reclaim fails — owner CONFIRMS repeated-Err reading), R4, R5, R6 (test deleted; FEAT-8 untested). cancel() keeps demotion + Ok (evidence: mobile retirement pending blocks all transfers). -> adv-features re-review
- startup lane DONE (impl/startup.md): CORE-2/BUILD-1 (dialog + exit 1; setup wrapper; Windows MessageBoxW = +1 unsafe in lib.rs, census baseline 75), CORE-1 (Retry/Quit Without Saving/Cancel; §1.1), CORE-7/LIB-4, CORE-8 (path_end; file: URLs), BUILD-8.
  Handoffs: CrashIntake.kt:37,29-30 & CrashIntake.swift:21-27 path/URL redaction stop at whitespace -> mobile mini-round later; desktop: §1.5 + update.rs RESTART_AFTER_QUIT comment stale (relaunch now cleared only on Cancel; relaunch point lib.rs finish_authorized_quit) -> desktop lane after adv; owner docs: RUST-UNSAFE-REVIEW.md ~446 & MEMORY-SAFETY-INVENTORY.md:11 counts.
  WINDOWS CLIPPY blockers in other lanes' tests: library_files_tests.rs:1252 single_fixture unused (files), sharing_interop_tests.rs:541 (net), store_tests.rs:1385 (desktop), acquisition/transfer.rs:748 blocks_in_conditions (transfers) -> must be clean at integration.
  -> adversarial reviewer launching
- adv-net DONE (adv/net.md): no blocking. Should-fix R1 (Hyper-V external switch regression), R2 (receipt for vanished sender), R3 (CORE-3 test can't fail), R4 (Windows VPN/VirtualBox/ZeroTier/wwp). Nits R5, R6. Owner: adopt R1-R6 (R6 loopback-only => addressLost-style localOnly; 2-observation debounce; clear finder error when off). Plus Windows clippy sharing_interop_tests.rs:541. -> net implementer resumed
- transfers round 2 DONE (impl/transfers-round2.md): R1 header length check on all publish paths, R2, R3 (32 pass cap + 120s stall + 15s connect), R4 docs, R5 errno-specific (vendor uses nix Errno), R6. torrent+acquisition 217/0. -> adv-transfers re-review
- adv-android round 2 (adv/android-round2.md): R1-R7, SHELL-R6 closed. R8 (stopOwned stopService under pending start; pre-existing; timeout path worst) -> android round 3 + CrashIntake.kt redaction handoff. R9 nit accepted.
- files lane DONE (impl/files.md): LIB-1 (capture-then-unlink fallback; no-hardlink refusal), LIB-2 rust, LIB-5 (+ residual same-volume journals), LIB-6 backend (edition_discard_original contract), LIB-9 partial, LIB-10. No fail-without-fix observed (inferred).
  Handoffs: panes lane: LIB-6 UI + api.ts privacyGeneration auto-inject; maps partials.rs comment (inherit fallback). Doubt: LIB-1 crash leaves .kiwix-renamed-<uuid> second name.
  -> adversarial reviewer launching. Launching service lane + panes lane now.
- launched service lane (a8261…) and panes lane (af2c7…). files adv review (a3e6f…).
- adv-features round 2 (adv/features-round2.md): R1,R4,R5,R6 closed; R2->service lane, R3->panes lane, R7 docs owner. New R8 (record_vanished lacks dir identity; owner: STRICT rule, mismatch report-only), R9 (plain sentences), R10 nits (test + NOT_OUR_FILE wording). -> features round 3
- reader round 2 DONE (impl/reader-round2.md): R1 hardened (native accessors, scrub secret, storage-bridge too), vouch gate, R2 (+3s cap), R3 reword, ⌃⌘F. STILL OPEN: forged hello + document.open() still receives reader-init+photoSecret (needs server-attested per-document identity). -> adv-reader re-review (assess severity/what photo secret authorises; feasible Rust fix?)
- apple crash-redaction (impl/apple-crash.md): Swift pathTail matches Rust path_end (300k differential, 0 mismatch). Residual (both Rust+Swift): path right after '<' not recognised -> startup lane adv/round. -> adv-apple quick review
- adv-desktop DONE (adv/desktop.md): no blocking. Owner: adopt R1 (call cleanup_before_exit), R2 (semver hide stale version, don't clear in hook), R4, R5, R6 (gate to Windows/parked frame), R8 (store_tests name, update.rs log; service.rs/service_tests comment -> service lane), + startup handoff (§1.5 & RESTART_AFTER_QUIT comment). R3 -> service lane handoff 2 (already assigned). R7 owner end. R9 accepted residual (document). ⌥⌘T accepted. -> desktop round 2
- android round 3 DONE (impl/android-round3.md): R8 (queuedForegroundStarts; no stopService while queued), CrashIntake.kt pathEnd port. 70/70. -> adv-android re-review
- adv-apple crash (adv/apple-crash.md): CRASH-R1 (Swift regex fails open >~0.5M chars on one line -> bound input 32K at top of redact), CRASH-R2 (add '<', '“', '‘' to path start set in Rust+Swift+Kotlin together), CRASH-R3 test nit; Rust redacts paths before secrets (password value may survive) -> startup lane.
  PLAN: crash-redaction round after startup adv returns: startup(Rust starts + secret order), apple(Swift bound + starts + R3), android(Kotlin starts).
- adv-android round 3 (adv/android-round3.md): R8 CLOSED. ANDROID lane CLOSED except crash port: R10 Kotlin secrets-before-paths leaks spaced path tail. CONFLICT with apple finding: Rust paths-before-secrets lets secret value survive. => joint crash round must define ONE pipeline for Rust/Swift/Kotlin: secrets first, where a secret value that begins with a path start (or quote) extends via path_end; then URLs/paths; differential test across all three.
- shell round 2 DONE (impl/shell-round2.md): R2 (not-saved status, Enter review, Quit keeps unreviewed dir unapplied), R3, R4, R5, diagnostics.ts, test robustness. -> adv-shell re-review. Panes: widen Downloads onError type.
- adv-startup DONE (adv/startup.md): no blocking. R1 (Cancel with no window left => headless zombie), R2 test can't fail, R3 label collision, R4 unsafe docs, R5 file: non-slash tail + //UNC + boundaries, R6 zenity exit 1 => kdialog dup, R7 comment. -> startup round 2 incl. JOINT CRASH PIPELINE spec + shared vectors; then apple+android port.
- adv-reader round 2 (adv/reader-round2.md): R1-R3 closed; decision 2 closed except forged-hello+document.open residual. Owner: adopt R4 (send/accept photo secret only when isWindows && !restricted), R5 wording. MessagePort partial fix declined (partial; residual documented: needs native per-frame channel/isolated world). -> reader round 3
- adv-shell round 2 (adv/shell-round2.md): R3,R4,R5,diag closed; R2 NOT closed (R2b blur on hide commits+deletes). Nits R7 (IME keyCode 229), R8 (boot-fail fast), R5b (JSON after prefix). Owner: adopt all. -> shell round 3
- reader round 3 DONE (impl/reader-round3.md): R4 photoChooserOffered gate, R5 wording. -> adv-reader quick check
- adv-reader round 3: R4, R5 closed. LANE reader CLOSED. Residuals: forged-hello+document.open() photo-secret on Windows unrestricted (prompt without click; needs native per-frame channel); READER-3 handshake-before-load follow-up; external-link clicks before load swallowed; admission state machine only harness-tested (no real webview).
STATUS 01:05: CLOSED: apple(+crash pending joint), cidocs, reader, android(except joint crash). Pending: transfers re-review, features r3, net r2, desktop r2, shell r3, startup r2(+canonical crash), files adv, service impl, panes impl.
- shell round 3 DONE (impl/shell-round3.md): R2b (typed only; switches still revert - owner ACCEPTS deviation), R7, R8, R5b. -> adv-shell quick check
- adv-transfers round 2 (adv/transfers-round2.md): R1-R6 closed. FULL SUITE 1435 passed / 0 failed / 16 ignored (01:0x). Nits R7, R8; trickle unbounded (owner: FIX with 60s min-progress window). localswarm/mod.rs:24 unused import blocks all clippy (net lane). -> transfers round 3
