# Project review and remediation — 2026-11-27

| | |
|---|---|
| **Review base** | `d98cac2` (clean tree) |
| **Branch** | `claude/stoic-dirac-rs0ol3` |
| **Tree this record describes** | the reviewed state named in [§7](#7-how-the-work-is-recorded): the commit that removes `working/`. Its Rust sources are those verified in [§5.4](#54-final-tree-integration-verification). |
| **Scope** | maintainability, reliability and usability across macOS, Windows, Linux, iOS and Android |
| **Method** | 11 independent read-only area reviews, then 15 lanes (14 implementation lanes and the owner's integration work) plus two crash-redaction port lanes, each followed by independent adversarial review and re-review until the reviewer closed it |
| **Replaces** | `docs/reviews/2026-11-27/working/` (ledger, briefs, area reports, implementer and adversarial reports, final logs), kept in the evidence commit and removed in the reviewed state; see [§7](#7-how-the-work-is-recorded) |

This file is written only from the working record: the findings ledger, the 11 area reports, the implementer reports and the adversarial reports. Where those sources disagree or leave a gap, this record says so (see [§8](#8-source-conflicts-and-gaps-in-the-record)). Apart from §5.4, which records the integration owner's final runs, nothing in this file was re-measured while writing it.

---

## 1. Summary

### What was reviewed and how

- **Area review.** Eleven independent reviewers each read one subsystem at `d98cac2`, read-only, with no `cargo`, `gradle` or `npm run build`. They produced 113 numbered findings (APPLE 9, SHELL 11, SERVE 7, ACQ 11, LIB 10, CORE 11, FEAT 10, BUILD 12, NET 10, READER 10, ANDROID 12). Eight duplicate a finding in another report (ACQ-5 and FEAT-5 = SHELL-1, CORE-5 = LIB-7, CORE-7 = LIB-4, BUILD-1 = CORE-2, BUILD-10 = CORE-10, ANDROID-3 = SHELL-4, FEAT-10 = READER-8), and FEAT-6 overlaps CORE-9. The integration owner added one finding from the baseline run (OWN-1), and the ANDROID report carried one unverified note (an R8 keep rule).
- **Implementation lanes.** The findings were split into lanes with exclusive file ownership in one shared working tree: apple, shell, reader, android, cidocs, transfers, files, net, features, desktop, startup, service, panes and buildhygiene, plus the owner's own documentation, localization and generated-file work ("owner"). Two further lanes ported the canonical crash redaction to Swift (`apple-crash`) and Kotlin (`android-crash`). A change needed outside a lane's files was handed off to the owning lane, not made in place.
- **Adversarial review.** After each implementation round, an independent reviewer (not the implementer) attacked the change with its callers and consumers, reported findings with severity (Blocking / Should-fix / Nit), and stated what it ran and what it could not verify. The owner recorded a decision on each finding; the implementer fixed it; the same reviewer re-reviewed. This repeated until the reviewer's last round stated closure. Closed lanes took between 2 and 5 adversarial rounds.

### Outcome

- Every one of the 113 area findings, OWN-1 and the R8 note has a recorded disposition in [§3](#3-findings-and-dispositions). None was rejected outright. Most are fixed; some are fixed differently from the reviewer's proposal, some are partial, and some are accepted residuals by owner decision.
- The adversarial rounds raised six **Blocking** findings (FEATURES-R1, APPLE-R1, TRANSFERS-R1 after the owner raised it, FILES-R1, SERVICE-R1, CIDOCS-R1). All six were fixed and closed by their reviewers.
- Thirteen lanes and both crash ports closed in their reviewers' final reports. Two closed on a recorded condition:
  - **apple:** the reviewer's last round raised no blocking item but kept APPLE-1 and APPLE-4 short of "fully done" until the device check APPLE-R7 runs; the owner closed the lane on that basis (LEDGER), with APPLE-R7 open as a device gate.
  - **buildhygiene:** the reviewer closed it once `DEPENDENCIES.md:3` was reworded; the owner applied the reviewer's text, and this record's review confirmed it.
- **owner:** R1–R3 and R5–R9 closed in owner review r2. OWNER-R4 (the `docs/RELIABILITY.md` section citing this record) and the generated outputs were checked in this record's review, whose findings were applied and re-checked.
- The final-tree integration verification is recorded in [§5.4](#54-final-tree-integration-verification): Rust library tests 1493 passed / 0 failed, clippy clean on Linux, Windows and Android, 544 of 545 Node tests passing with 1 skipped, Kotlin 79/79, Swift crash redaction 238/238.

### Limits of this record

- **A review with zero findings is not release, device or parity certification.** A closed lane means its reviewer found nothing further to fix in the changed code it read and ran. It does not certify a release, behaviour on any physical device, a Mac, Windows, an iPhone simulator or Android, or parity between platforms.
- **No remote enforcement exists.** The review gate in `AGENTS.md` is a contributor and assistant workflow. No branch protection, required check or CI job performs or enforces independent review, and none was configured or verified during this work.
- **No device, Mac, Windows, simulator or Apple-compiler run happened.** All verification ran in one Linux x86_64 container (see [§5.1](#51-what-was-compiled-and-run-and-what-was-not)).

---

## 2. Coverage

### 2.1 Lanes

Rounds are counted as implementer reports / adversarial reports. "Closed" means the lane's last adversarial report states closure.

| Lane | Files owned (brief) | Findings handled | Rounds (impl / adv) | Closure |
|---|---|---|---|---|
| transfers | `torrent/manager.rs`, `torrent/web_seed.rs`, `torrent_tests.rs`, `acquisition*.rs` (transfer, segments, http_meta, manager, finalize), `acquisition_tests.rs`; one-line edits in `dispatch.rs` (iOS) and `mobile_service.rs`; from round 2 `vendor/librqbit-dualstack-sockets/src/socket.rs`, `docs/WEB-SEEDS.md`, `docs/FORKS.md` | OWN-1, ACQ-1, 2, 3, 4, 6, 9, 10, 11 | 4 / 4 | Closed (adv r4), "apart from host clippy, which still has no exit-0 run" for round 4 |
| features | `maps.rs`, `maps/{manager,compose,partials}.rs`, `maps_tests.rs`, `kokoro.rs`, `native_pdf.rs`, `native_pdf/cleanup.rs`, `MapJob` in `src/types.ts`; two lines in `service.rs` (round 4) | FEAT-1, 2, 3, 4, 6 (backend), 7 (backend), 8, 9; files-lane handoff (`partials.rs` second name) | 5 / 5 | Closed (adv r5) |
| files | `publication.rs`, `library_files.rs`, `library_files_tests.rs`, `library_root.rs`, `device_scan.rs`, `edition_updates.rs`, `edition_service.rs`; listed edits in `dispatch.rs`, `service.rs`, `store.rs`, `src/types.ts` | LIB-1, 2 (Rust), 5, 6 (backend), 9, 10 | 3 / 3 | Closed (adv r3) |
| shell | `src/App.tsx`, `UI.tsx`, `error-visibility.ts`, `api.ts`, `Settings.tsx`, `SettingsAutosave.tsx`, `UpdatesSettings.tsx`, `camera.ts`, `diagnostics.ts`; new `shell-dismiss.ts`, `android-back.ts`; shell tests | SHELL-1 (= ACQ-5, FEAT-5), 2, 4 (= ANDROID-3), 6, 7, 9, 10, 11, NET-3, APPLE-7, ANDROID-8 handoff H1, H5 | 3 / 3 | Closed (adv r3) |
| reader | `Reader.tsx`, `reader.css`, `reader-bridge.js`, `storage-bridge.js`, `reader-storage.ts`, reader tests | READER-1 to 10 (READER-8 = FEAT-10), desktop handoff (⌃⌘F label) | 3 / 3 | Closed (adv r3) |
| apple | `PrivacyInfo.xcprivacy`, iOS plugin Swift (`DocumentStorage`, `KiwixMobilePlugin`, `LiveActivity`, `DownloadActivityAttributes`, `ReadAloud*`), iOS tests, `gen/apple` widgets, `test-privacy-manifest.mjs`; a few lines in `mobile_speech.rs` and `live_activity.rs` | APPLE-1, 4, 5, 6, 8 | 2 / 2 | Closed (adv r2) with APPLE-1 recorded as "mitigation, unverified on device" |
| apple-crash | `CrashIntake.swift`, new `CrashRedaction.swift`, `CrashCapturePolicyTests.swift` | CORE-8 (Swift half), canonical spec port | 3 / 3 | Closed (adv apple-crash3) |
| android | `AndroidManifest.xml`, `MainActivity.kt`, `consumer-rules.pro`, plugin Kotlin (`KiwixMobilePlugin`, `KiwixTransferService`, `SpeechPlayback`, `OwnedDocuments`, `MobilePolicy`, `NearbyRoutes`, `LocalHotspot`, `DeviceZimScan`, new `WindowTheme`), Kotlin tests, `user_error.rs`, limited `mobile_service.rs`, `system-text-scale.ts`, `platform.ts`, `platform.css` | ANDROID-1, 2, 4 (Rust), 5, 7, 8, 9, 10, 11, 12 (part), LIB-2 (Kotlin), R8 note, SHELL-R6 | 3 / 3 | Closed (adv r3) except the crash port, which moved to android-crash |
| android-crash | `CrashIntake.kt`, new `CrashRedaction.kt`, `CrashRedactionTest.kt`, `CrashCapturePolicyTest.kt` | CORE-8 (Kotlin half), ANDROID-R10, canonical spec port | 2 / 2 | Closed (adv android-crash2); ANDROID-R11 open as a process residual |
| net | `sharing.rs` (+ `sharing_tests.rs`, `sharing_interop_tests.rs`), `shared_library.rs`, `hotspot.rs`, `transfer_policy.rs`, `nearby_service.rs`, `localswarm/{mod,network,discovery,tests,cleanup}.rs`, `SharingControls.tsx`, `LocalSwarmControls.tsx`, `PrivateReadingSync.tsx`, `localswarm-controls.css`; two `dispatch.rs` arms; additive `src/types.ts` | NET-1, 2, 4, 5, 6, 7, 8, 9, 10, CORE-3; files-lane handoff (`localswarm/cleanup.rs` placeholder) | 3 / 3 | Closed (adv r3) |
| startup | `lib.rs`, `quit.rs`, `instance.rs`, `crash.rs`, `docs/STATE_MACHINES.md` §1.1, `unsafe-census-baseline.json`, `RUST-UNSAFE-REVIEW.md`, `MEMORY-SAFETY-INVENTORY.md`, `tests/fixtures/crash-redaction-cases.json` and `-spec.md` | CORE-1, CORE-2 (= BUILD-1), CORE-7 (= LIB-4), CORE-8, BUILD-8; canonical crash-redaction spec | 6 / 5 (rounds 1, 2, 3, the v2.1 e-mail addendum, 4, 5) | Closed (adv r4 spec final; adv r5 spec final v2.3) |
| cidocs | `.github/workflows/{checks,build}.yml`, docs (`PACKAGING`, `UPDATE`, `DEV-ITERATION`, `PLATFORM-GAPS`, `DEPENDENCIES`, `SANITIZERS-AND-FUZZING`, `STATE_MACHINES` §1.2/§1.4), scripts (`generate-error-docs`, `localization`, `error-inventory`, `make-update-manifest`, `fuzz.sh`), new tests, `package.json`, `src-tauri/fuzz/*` | BUILD-3, 4, 6, 11, CORE-10 (= BUILD-10), CORE-11, SERVE-7, APPLE-9, SHELL-8 (gate) | 2 / 2 | Closed (adv r2) |
| desktop | `store.rs` (load path), `store_tests.rs`, `update.rs`, `windows.rs`, `sandbox.rs`, `menus.rs`, `nativeMenuLabels` in `App.tsx` | CORE-4, BUILD-2, 7, 9, 12, APPLE-2, APPLE-3 | 3 / 3 | Closed (adv r3) except DESKTOP-R11's service side, which the service lane applied in its round 3 |
| service | `service.rs`, `service/dispatch.rs`, `imports.rs`, `spelling.rs`, `server.rs` (+ tests), `archive.rs`, `mobile.rs` (H3), `mobile_service.rs` (H2), three `store.rs` edits (round 3), new `src/startup-notice.ts` + test | SERVE-1, 3, 4, LIB-3, LIB-7 (= CORE-5), LIB-8, CORE-6, CORE-9; handoffs FEAT-3 startup sweep, FEAT-9, APPLE-2 revive call, ANDROID H2/H3, DESKTOP-R11/R8 | 4 / 4 | Closed (adv r4) |
| panes | `Library.tsx`, `Downloads.tsx`, `Catalog.tsx`, `Maps.tsx`, `MapBlockProgress.tsx`, `SelectedStorageActions.tsx`, `bareFilesystemPath` in `UI.tsx`, pane tests | SHELL-3, 5, SERVE-2, 5, ACQ-7, 8, ANDROID-6, LIB-6 (UI), FEAT-1/6/7 (UI), SHELL-R1 double owners, FEATURES-R13 | 5 / 5 | Closed (adv r5) |
| buildhygiene | deleted `src-tauri/native/{mobile_sdk,desktop_sdk}.rs`; `lib.rs` (mod removal), `Cargo.toml`, `build.rs`, `mobile_policy.rs` (test), CI steps, docs (`FORKS`, `PACKAGING`, `DEPENDENCIES`, `FEATURES`, `DEV-ITERATION`, `WINDOWS-*`), `linux-appimage.py`, new `test-build-hygiene.mjs` | SERVE-6, BUILD-5, stale `mobile_policy.rs:566` | 3 / 2 | Closed (adv buildhygiene r2, covering rounds 2 and 3); its one should-fix (`DEPENDENCIES.md:3`) applied by the owner with the reviewer's exact text |
| owner | `STATE_MACHINES.md` (§1.1/§1.2 import cancellation, §5.1 MapJob ETA), `PLATFORM-GAPS.md` (items 5 and 8, print row, Android Back), `RELIABILITY.md` dated section, crash-spec edits, `extra-en-rust.json`, `plural-sources.json`, `test-rust-localized-strings.mjs`, two stale code comments, `App.tsx` Back comment; regenerated `docs/ERRORS.md` and `src/locales/**`; the initial OWN-1 vendor patch; the desktop handoff assertion in `service_tests.rs` | integration and handoffs | — / 2 (+ this record's review) | R1–R3, R5–R9 closed (adv owner r2); OWNER-R4 and the generated outputs (`docs/ERRORS.md`, `src/locales/**`) are covered by the independent review of this record |

### 2.2 Changed paths

`git diff --name-status d98cac2 08528ee`, excluding `docs/reviews/2026-11-27/working/` (124 files at that commit), lists 273 paths: 236 modified, 35 added, 2 deleted. 71 of the modified paths are `src/locales/messages/*.json` from the localization generation.

- **Deleted:** `src-tauri/native/desktop_sdk.rs`, `src-tauri/native/mobile_sdk.rs`.
- **Added:** `src/android-back.ts`, `src/shell-dismiss.ts`, `src/startup-notice.ts`, `src/locales/extra-en-rust.json`; `src-tauri/tests/fixtures/crash-redaction-cases.json` and `crash-redaction-spec.md`; `src-tauri/fuzz/extract-index-seeds.py`, `src-tauri/fuzz/fuzz_targets/search_index.rs`; Kotlin `CrashRedaction.kt`, `WindowTheme.kt` and tests `CrashRedactionTest`, `DeviceScanPolicyTest`, `NetworkPrefixTest`, `StagingCleanupTest`, `TransferStartDecisionTest`, `WindowThemeTest`; Swift `CrashRedaction.swift`; 18 new Node tests (`test-android-manifest`, `-build-hygiene`, `-camera-window`, `-error-docs`, `-localization-inventory`, `-nearby-sharing-surfaces`, `-pane-actions`, `-pane-surfaces`, `-privacy-manifest`, `-reader-fresh-document`, `-reader-message-contract`, `-reader-print`, `-rust-localized-strings`, `-settings-surface`, `-shell-dismiss`, `-shell-surfaces`, `-startup-notice`, `-updates-settings`).
- **Modified, by area:** Rust core (61 changed files under `src-tauri/src`, including `service.rs`, `dispatch.rs`, `store.rs`, `crash.rs`, `lib.rs`, `sharing.rs`, `localswarm/*`, `maps/*`, `acquisition/*`, `torrent/*`); the vendored `librqbit-dualstack-sockets/src/socket.rs`; `Cargo.toml`, `build.rs`, the fuzz crate; Android manifest, `MainActivity.kt` and plugin Kotlin; iOS plugin Swift and the `gen/apple` widgets; `PrivacyInfo.xcprivacy`; 31 frontend files under `src/` outside `src/locales`; both workflows; 16 documents under `docs/`; localization sources and generated catalogs.

---

## 3. Findings and dispositions

Dispositions: **fixed**, **fixed differently** (the defect is fixed by another mechanism than the reviewer proposed), **partial**, **rejected**, **handoff** (to another lane; the row names the lane that finished it), **accepted residual** (owner decision; not fixed). A duplicate takes the disposition of the finding it duplicates. "Closing review" names the adversarial report that closed the item, as `adv <lane> r<n>`.

None of these dispositions has been checked on a device, a Mac, Windows or a simulator. Code that is never compiled here (macOS/iOS Rust, Swift beyond parsing) is marked.

### 3.1 APPLE (iOS and macOS platform layers)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| APPLE-1 | Med | Linked iOS archives are rejected once `fileResourceIdentifier` changes | **Fixed differently**: after a restart the record is accepted on volume UUID + inode + mtime, only when `volumeIsInternal == true`, and then healed; removable, network and unknown volumes still need Locate. Recorded as "mitigation, unverified on device" (APPLE-R7) | apple | adv apple r2 |
| APPLE-2 | Med | macOS sandbox: a bookmark that fails at launch is never retried | **Fixed**: `Registry::revive` at `grant()` and from `library_maintenance` (rate-limited; call verified at `dispatch.rs`). macOS code never compiled | desktop, service | adv desktop r3; adv service r1 ("Desktop handoff 2: fine") |
| APPLE-3 | Med | ⌘M claimed twice; app menu has only Quit | **Fixed**: Table of contents ⌥⌘T, Settings ⌘,, Fullscreen ⌃⌘F, app-menu items, five labels; Reader shortcut table shows ⌃⌘F. Never compiled for macOS | desktop, reader | adv desktop r3; adv reader r2 |
| APPLE-4 | Med | Download Live Activity never looks stale | **Fixed** in round 2: stale presentation plus an optional `staleLabel` ("Open Kiwix to see progress") from `live_activity.rs`. The expanded Dynamic Island still shows no stale wording (APPLE-R9, accepted Nit). The new string is English in every non-English locale until translated | apple, owner | adv apple r2 |
| APPLE-5 | Med | Privacy manifest omits the UserDefaults category | **Fixed** (`CA92.1`) with `test-privacy-manifest.mjs` | apple | adv apple r1, r2 (R6) |
| APPLE-6 | Low | Read-aloud Live Activity: hard-coded English and VoiceOver labels | **Partial**: control names localized, compact regions labelled; "% read" stays English | apple | adv apple r1 |
| APPLE-7 | Low | iOS hardware Escape swallowed outside the reader | **Fixed** | shell | adv shell r1 |
| APPLE-8 | Low | `persistFolderBookmark` writes an unused bookmark and its failure fails a finished import | **Fixed** (removed; stale bookmarks folder cleaned with no-follow) | apple | adv apple r1 |
| APPLE-9 | Low | iOS XCTest suite not run by CI and cannot resolve from a clean checkout | **Accepted residual**: documented as local-only in `docs/SANITIZERS-AND-FUZZING.md`; the run instructions were not executed | cidocs | adv cidocs r1 |

### 3.2 SHELL (React shell and non-reader frontend)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| SHELL-1 | Med | `errorText` rewrites any checksum/verification message into the sender sentence | **Fixed**: explicit list of the four Nearby sender producers, plus a scan test over shipped Rust | shell | adv shell r1 |
| SHELL-2 | Med | Failures of actions inside a modal go only to the band beneath it | **Fixed**: alert dialog over the modal; follow-ups SHELL-R1 (double owners, finished by panes), R3, R4 | shell, panes | adv shell r2 |
| SHELL-3 | Med | Catalog and Maps card failures painted off-screen at page top | **Fixed** | panes | adv panes r1 |
| SHELL-4 | Med | Android system Back never handled by the shell | **Fixed differently**: listener held only while something can be dismissed; at the root the plugin default applies | shell | adv shell r1 |
| SHELL-5 | Med | "Share as BitTorrent" false failure after 180 s | **Fixed**: polls until settled; retired on close, unmount, privacy change or backend error. The 750 ms poll has no cap while the dialog is open (accepted) | panes | adv panes r1 |
| SHELL-6 | Low | Download-directory confirmation opens mid-typing | **Fixed**, with SHELL-R2/R2b (typed path never lost or applied without confirmation); switch edits still revert on Quit (accepted deviation) | shell | adv shell r3 |
| SHELL-7 | Low | User cancellations painted as errors | **Partial**: `UserCancelledError` for the updater cancel and the declined low-storage prompt; `reportError` ignores it. A declined prompt is still shown once inline by owners that call `errorText` | shell, panes | adv shell r2 |
| SHELL-8 | Low | 64 UI strings missing from the localization inventory; no CI gate; plural | **Fixed**: inventory gate and allow-list (cidocs); plural source and catalog data by the owner; `localization.mjs generate` run at integration | cidocs, owner | adv cidocs r2. The generated catalogs were not adversarially reviewed |
| SHELL-9 | Low | Mobile Settings and Help mention a File menu and New window | **Fixed** | shell | adv shell r1 |
| SHELL-10 | Low | `UpdatesSettings` listener leak; `installing` never cleared | **Fixed** | shell | adv shell r1 |
| SHELL-11 | Low | Camera window dead end; locale failure blocks start | **Fixed** | shell | adv shell r1 |

### 3.3 SERVE (archive serving, search, spelling)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| SERVE-1 | High | A cached archive handle that has gone bad is never dropped | **Fixed**: liveness probe on cache hit, extended to reader listeners; content-identity check for a rename-over (SERVICE-R2); readers retired only on a flip to unavailable (SERVICE-R5) | service | adv service r2 |
| SERVE-2 | Med | Enter within 120 ms lets the suggestion timer cancel the search | **Fixed** (differently from the proposed `busyRef`) | panes | adv panes r1 |
| SERVE-3 | Med | Spelling cache directory never garbage-collected | **Fixed**: hourly no-follow sweep of exact cache names; skipped while any book is unavailable | service | adv service r1 |
| SERVE-4 | Low | Non-404 document failures return uninstrumented `text/plain` | **Fixed** (escaped static HTML page with the bridge) | service | adv service r1 |
| SERVE-5 | Low | Library search never shows spelling corrections | **Fixed** (corrections only) | panes | adv panes r1 |
| SERVE-6 | Low | Dead libzim SDK glue; docs still say the app links libzim | **Fixed**: both SDK files and the `cc` dev-dependency deleted, docs corrected | buildhygiene | adv buildhygiene r1; doc follow-ups closed in adv buildhygiene r2 |
| SERVE-7 | Low | Fuzz target never decodes a cluster; no search-index target | **Fixed**, type-checked only (`cargo check` of the fuzz crate). No fuzzing was run | cidocs | adv cidocs r1 |

### 3.4 ACQ (downloads and acquisition)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| ACQ-1 | High | Web-seed stage truncates a valid partial torrent payload on restart | **Fixed differently**: in-place piece-range repair, no truncation | transfers | adv transfers r1 |
| ACQ-2 | Med | Segmented retry budget never resets on progress | **Fixed** | transfers | adv transfers r1 |
| ACQ-3 | Med | iOS names every catalog download `<name>.zim`, so flavours collide | **Fixed**; never compiled for iOS | transfers | adv transfers r1 |
| ACQ-4 | Med | Web seed: three attempts, no backoff, stale offset | **Fixed**, with TRANSFERS-R3 (32-pass cap, 120 s stall limit, 15 s connect timeout), R6 (cancellable test) and the pace rule (see [§4](#4-owner-decisions-and-accepted-deviations)) | transfers | adv transfers r3, r4 |
| ACQ-5 | Med | = SHELL-1 | Fixed with SHELL-1 | shell | adv shell r1 |
| ACQ-6 | Med | Metalink mirror pinned; Retry never re-resolves | **Partial**: a job with nothing durable re-resolves and moves to the next mirror at each start; once bytes are kept the mirror stays pinned (by design: no cross-server validator). See [§8](#8-source-conflicts-and-gaps-in-the-record) | transfers | adv transfers r1 ("fixes it") |
| ACQ-7 | Low | Catalog "installed" compares raw dates and ignores flavour | **Fixed**; `exact` follows Rust `series_compatible` (PANES-R1, R6) | panes | adv panes r3 |
| ACQ-8 | Low | Interrupted and failed rows hide progress | **Fixed** | panes | adv panes r1 |
| ACQ-9 | Low | Android drops the foreground service while a share is "creating" | **Fixed**; Android type-check clean | transfers | adv transfers r1 |
| ACQ-10 | Low | Complete single-stream part cannot finalize on a validator-less 416 | **Fixed**, with TRANSFERS-R2 (a part counts as complete only with the ZIM signature and header length) | transfers | adv transfers r2 |
| ACQ-11 | Low | OPDS display length enforced as an exact size | **Fixed** (Rust side), with TRANSFERS-R1 (header length check before publication) | transfers | adv transfers r2 |

### 3.5 LIB (library intake, move, delete, editions, export)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| LIB-1 | High | Exclusive rename has no fallback on NFS/FUSE | **Fixed differently**: capture-then-unlink hard-link fallback, refusal where neither exclusive rename nor hard links exist, healing of leftovers (FILES-R4). On Android `linkat` is refused, so the move fails with a clear message (accepted residual) | files | adv files r2, r3 |
| LIB-2 | High | Android device scan aborts on a root-level or dot-named ZIM | **Fixed** (Rust half: files; Kotlin walk: android, incl. `hiddenScanName` at every site) | files, android | adv files r1; adv android r2 |
| LIB-3 | Med | Opening an already-imported archive reruns a full integrity pass | **Fixed** (fast path only when content identity matches; provider books skipped) | service | adv service r3 |
| LIB-4 | Med | "Open with", CLI and second-instance failures only on stderr | = CORE-7, fixed | startup | adv startup r2 |
| LIB-5 | Med | Desktop "move previous downloads": no space check; endless retries | **Fixed**, with FILES-R1 (Blocking), R2, R3, R5 | files | adv files r2 |
| LIB-6 | Med | Superseded edition original cannot be deleted in the app | **Fixed**: backend `edition_discard_original` (files, FILES-R6/R9) and the card button (panes) | files, panes | adv files r3; adv panes r1 |
| LIB-7 | Med | Monitored folder that is an empty mount point retires all its books | **Fixed differently**: device-id rule; a folder never seen with its archives in this run, and Windows, keep missing entries as unavailable (accepted residual) | service | adv service r2 |
| LIB-8 | Med | Mobile managed copy of an unavailable entry deletes the fresh copy; batch unverified | **Fixed** (relink unusable entries; verify the batch); Android type-check only, iOS not compiled | service | adv service r2 |
| LIB-9 | Low | Linux Trash: hidden prefix, cross-device home-Trash copy, misleading message | **Partial**: prefix and message fixed; cross-device copy into the home Trash not prevented | files | adv files r1 |
| LIB-10 | Low | "Unsafe"-named archives cannot be deleted or revealed | **Fixed** | files | adv files r1 |

### 3.6 CORE (Rust application core)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| CORE-1 | High | Quit cannot complete while the profile is unwritable; only "Retry" | **Fixed**: Retry / Quit Without Saving / Cancel, no headless zombie (STARTUP-R1), at least 1 s between no-window retries | startup | adv startup r4 |
| CORE-2 | High | Fatal desktop startup errors shown to nobody | **Fixed**: native dialog and exit 1; Windows `MessageBoxW` adds one documented `unsafe` block (census baseline 75) | startup | adv startup r2 |
| CORE-3 | Med | `localswarm_discovery` nests the privacy read lease on mobile | **Fixed**; the test now drives the real entries (NET-R3). The `#[cfg(mobile)]` arm is type-checked for Android only | net | adv net r2 |
| CORE-4 | Med | Records skipped at load are deleted by the next write; desktop not told | **Fixed**: preserved sibling copy plus notice; mobile refuses a foreign profile version (DESKTOP-R4); read-only → writable keeps the copy (DESKTOP-R5); notice follows the store (DESKTOP-R11) | desktop, service | adv desktop r3; adv service r3 |
| CORE-5 | Med | = LIB-7 | See LIB-7 | service | adv service r2 |
| CORE-6 | Med | Manual import verification uncancellable under the privacy read lease | **Fixed**: `import_cancel` token, `ImportCutover`, cancelled on Quit, privacy switch and (mobile) backgrounding | service | adv service r3 |
| CORE-7 | Med | = LIB-4, plus the macOS Opened handler | **Fixed** | startup | adv startup r2 |
| CORE-8 | Low | Crash path redaction stops at the first space | **Fixed differently**: replaced by one canonical redaction rule for Rust, Swift and Kotlin (spec v2.3, 238 vectors) | startup, apple-crash, android-crash | adv startup r5; adv apple-crash3; adv android-crash2 |
| CORE-9 | Low | `map_forget_recovery` has no caller; four dead dispatch arms | **Fixed** (dead arms removed; recovery Dismiss wired in the UI) | service, features, panes | adv service r1; adv panes r1 |
| CORE-10 | Low | `docs/ERRORS.md` stale; generator has no `--check` | **Fixed**: `--check`, deterministic output, CI gate; `docs/ERRORS.md` regenerated by the owner | cidocs, owner | adv cidocs r2 |
| CORE-11 | Low | `docs/STATE_MACHINES.md` §1.2 and §1.4 stale | **Fixed** (cidocs), later sections updated by the owner (OWNER-R3) | cidocs, owner | adv cidocs r2; adv owner r2 |

### 3.7 FEAT (maps, read-aloud, PDF export)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| FEAT-1 | High | A map partial that cannot be reclaimed blocks all map downloads | **Fixed**: demoted to a recovery record; proven records retried on each Delete, which reports an error while a reclaim fails (FEATURES-R1, R8, R11, R14) | features, panes | adv features r5 |
| FEAT-2 | Med | Disk-full composition checkpoints past the bytes on disk | **Fixed**, with FEATURES-R4 (a failed sync does not checkpoint) | features | adv features r2 |
| FEAT-3 | Med | Article-PDF scratch never swept after a crash | **Fixed**: export-time sweep plus a once-per-process startup sweep from `library_maintenance`, `STALE_AFTER` 1 h. Desktop read-only mode relies on the export-time sweep | features, service | adv service r1 |
| FEAT-4 | Med | One unpronounceable chunk ends Kokoro read-aloud | **Fixed** (forward and backward skip). The real Kokoro `run` loop with a model was not exercised | features | adv features r1 |
| FEAT-5 | Med | = SHELL-1 | Fixed with SHELL-1 | shell | adv shell r1 |
| FEAT-6 | Med | `map_forget_recovery` has no frontend caller; Delete always fails | **Fixed** (Rust: features; Dismiss control: panes) | features, panes | adv features r1 (backend), r2 (Delete); adv panes r1 |
| FEAT-7 | Low | `MapJob.speed`/`etaSeconds` never shown; ETA dishonest | **Fixed differently**: ETA dropped; rate shown in the UI | features, panes | adv features r1; adv panes r1 |
| FEAT-8 | Low | Map composition holds no macOS idle-sleep assertion | **Fixed in source, never compiled, no test** (the test that could not fail was deleted, FEATURES-R6) | features | adv features r2 |
| FEAT-9 | Low | Cancelling the voice-model download shown as a red error | **Fixed** (`kokoro.rs` and two lines in `service.rs`) | features | adv features r4 |
| FEAT-10 | Low | Mobile "Print / save as PDF" row calls a no-op `window.print()` | = READER-8, fixed | reader | adv reader r1 |

### 3.8 BUILD (Windows/Linux desktop, updater, build, CI, docs)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| BUILD-1 | High | = CORE-2, plus `--help`/`--version` invisible on Windows release | **Fixed** (both) | startup | adv startup r2 |
| BUILD-2 | Med | Windows self-update exits inside the plugin, skipping the Quit flush | **Partial**: the installer hook captures geometry, session snapshot and browsing, calls the plugin's `cleanup_before_exit` (DESKTOP-R1) and keeps the recorded version until the outcome is known (DESKTOP-R2). The shell's in-memory session flush is not run (about 600 ms of shell changes can be lost; accepted). The real hook never ran | desktop | adv desktop r3 |
| BUILD-3 | Med | Real-browser UI tests skipped in every workflow | **Fixed**: CI Chrome wrapper; the tests fail rather than skip under `CI`. Not run on GitHub | cidocs | adv cidocs r2 |
| BUILD-4 | Med | Rust tests never run on PRs; Windows tests never run; clippy lacks `--tests` | **Fixed**: `linux-tests` job, Linux clippy `--lib --tests`, AppImage test step, Windows test step in `build.yml`. Android `--tests` deliberately not added. None of it ran on GitHub; the Windows step is entirely unexercised | cidocs | adv cidocs r2 |
| BUILD-5 | Med | Linux/Android location providers never compiled; `zbus` built anyway | **Fixed**: opt-in `experimental-location` feature + env var, CI opt-in type-checks; zbus absent from default trees on all nine targets | buildhygiene | adv buildhygiene r1 |
| BUILD-6 | Low | Eight documentation contradictions | **Fixed** | cidocs | adv cidocs r1 |
| BUILD-7 | Low | Windows minimized window geometry overwritten | **Fixed** (skip only on Windows; DESKTOP-R6, R10) | desktop | adv desktop r3 |
| BUILD-8 | Low | Busy-profile dialog says "Activity Monitor" on Linux/Windows | **Fixed** | startup | adv startup r1 |
| BUILD-9 | Low | A future `lastChecked` suppresses update checks | **Fixed** | desktop | adv desktop r1 |
| BUILD-10 | Low | = CORE-10 | See CORE-10 | cidocs, owner | adv cidocs r2 |
| BUILD-11 | Low | Floating Rust toolchain and mutable action refs | **Accepted residual**: documented, nothing pinned | cidocs | adv cidocs r1 |
| BUILD-12 | Low | Windows `state.json` replace has no transient retry | **Fixed** (`retry_transient_sharing`; dead remove-then-rename branch deleted). Not exercised on Windows | desktop | adv desktop r1 |

### 3.9 NET (peer-to-peer and local network)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| NET-1 | High | Web sharing binds a Docker/WSL/VPN adapter | **Fixed** on Linux, Android and Apple names: shared LAN classifier plus routed-address preference. On Windows the routed preference is disabled (NET-R7) and a full-tunnel VPN that sorts first is still chosen (accepted residual) | net | adv net r3 |
| NET-2 | High | Bluetooth off or denied: Nearby fails silently | **Fixed** | net | adv net r1 |
| NET-3 | Med | Native error-code contract drift (`[CODE] - CODE`, ~90 unmapped) | **Fixed** (`nativeCodeText`; `user_error.rs` table; SHELL-R5/R5b/R6) | shell, android | adv shell r3; adv android r2 |
| NET-4 | Med | "Share all" stops silently; Android hotspot left up | **Fixed** (stop reason slot; Android hotspot waiter). Android backgrounding and iOS admission still stop sharing silently (pre-existing residual) | net | adv net r1 |
| NET-5 | Med | Sharing addresses are a start-time snapshot | **Fixed** (`addressLost`, `localOnly`, refresh). Non-Apple mDNS A records still carry listen-time addresses | net | adv net r3 |
| NET-6 | Med | "Find nearby" cannot restart after a discovery failure | **Fixed** | net | adv net r1 |
| NET-7 | Med | Pairing invitation cancelled silently on mobile background | **Fixed** (frontend; foreground-only policy kept) | net | adv net r1 |
| NET-8 | Low | Closing the connect dialog does not cancel a pending join | **Fixed** (with a backend guard; NET-R5) | net | adv net r2 |
| NET-9 | Low | Shared-library list reports "changed or disconnected" after 5 minutes | **Fixed** (receipt TTL, conditioned on list age: NET-R2) | net | adv net r2 |
| NET-10 | Low | `directConnection` is a dead setting | **Accepted residual** until it can be tested on two Apple devices | net | adv net r3 |

### 3.10 READER (reader and bridge)

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| READER-1 | High | Restricted mode / Reload do not reload with a fragment URL | **Fixed** | reader | adv reader r1 |
| READER-2 | High | WebKit fragment navigation re-sessions the reader per anchor click | **Fixed differently**: per-document proof with the bridge bootstrap token; a hello is required before any `reader-init`; hostile-script hardening (READER-R1) | reader | adv reader r2 |
| READER-3 | Med | Blink on child navigation; clicks before `load` swallowed | **Partial**: paint decoupling only; the handshake-before-load follow-up is not done, and external-link clicks before `load` are still swallowed | reader | adv reader r2 (READER-R2 against this fix closed there) |
| READER-4 | Med | Desktop system-voice read-aloud cancelled when hidden | **Fixed** | reader | adv reader r1 |
| READER-5 | Low | Link preview fires for same-page anchors | **Fixed** | reader | adv reader r1 |
| READER-6 | Low | Per-tab history stacks unbounded; `photoSecrets` never pruned | **Fixed** | reader | adv reader r1 |
| READER-7 | Low | `tel:` link raises a blocking error | **Fixed** (status notice; no new schemes) | reader | adv reader r1 |
| READER-8 | Low | Mobile print row / Ctrl+P only call `window.print()` | **Fixed**: row hidden on mobile; notice "Printing is not available on this device…" | reader | adv reader r1 |
| READER-9 | Low | Dead message handlers | **Fixed** | reader | adv reader r1 |
| READER-10 | Low | Blank unnamed TOC buttons | **Fixed** | reader | adv reader r1 |

### 3.11 ANDROID

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| ANDROID-1 | High | Failed copy-import leaves staged multi-GB files forever | **Fixed** (Kotlin removes only what it created); **fixed differently** in Rust (empty per-attempt UUID directory removed) | android | adv android r1 |
| ANDROID-2 | Med | Launch intent replayed after process death | **Fixed** (one addition to the reviewer's rule) | android | adv android r1 |
| ANDROID-3 | Med | = SHELL-4 | See SHELL-4 | shell | adv shell r1 |
| ANDROID-4 | Med | Raw `[CODE] - {json}` / Java exception text reaches the user | **Fixed** (Rust sentence table; unknown codes pass as bare codes to the localized frontend fallback, ANDROID-R5). The Rust sentence table is English-only (accepted) | android | adv android r2 |
| ANDROID-5 | Med | Unlisted configuration changes recreate the Activity and cancel imports | **Partial**: `navigation` added; `layoutDirection` not; new `ACTIVITY_RECREATED` code with a startup warning (H2, service lane) | android, service | adv android r2; adv service r1 |
| ANDROID-6 | Med | "Move: remove originals" can never succeed for provider archives | **Fixed** (UI no longer offers it) | panes | adv panes r1 |
| ANDROID-7 | Med | `KiwixTransferService` may stop before `startForeground` | **Fixed**, with ANDROID-R1, R4, R8 | android | adv android r3 |
| ANDROID-8 | Low | `fontScale` → `data-large-text` wiring | **Fixed** (native, TS and CSS; `App.tsx` H1 by the shell lane). Rendering on Android not seen | android, shell | adv android r1 |
| ANDROID-9 | Low | Status/navigation bar strips ignore the in-app theme | **Fixed** | android | adv android r1 |
| ANDROID-10 | Low | System-TTS read-aloud not paused on headphone unplug | **Fixed** | android | adv android r1 |
| ANDROID-11 | Low | `IpPrefix` constructor API 33 unguarded; dead nearbyRoute commands | **Partial**: constructor fixed; dead commands kept (accepted residual) | android | adv android r1 |
| ANDROID-12 | Low | Kotlin-composed English sentences bypass localization | **Partial**: notification-channel half fixed (with ANDROID-R6); device-scan half (handoff H4) not done | android | adv android r1 |
| (note) | — | Unverified note: R8 keep rule for native methods in `consumer-rules.pro` | **Fixed** (keep rule added). R8-minified builds not verified | android | adv android r1 |

### 3.12 Owner-found

| ID | Sev | Title | Disposition | Lane | Closing review |
|---|---|---|---|---|---|
| OWN-1 | (none assigned) | Torrent engine binds `[::]` only; no IPv4 fallback on IPv6-less hosts (5 baseline test failures) | **Fixed**: app probe in `torrent/manager.rs`; vendored `librqbit-dualstack-sockets` falls back to IPv4 only for address-family errors (TRANSFERS-R5), noted in `docs/FORKS.md`. Verified by the 5 previously failing tests passing. No host with IPv6 was available | transfers, owner | adv transfers r1, r2 |

### 3.13 Findings raised by the adversarial rounds

The adversarial rounds raised findings named `<LANE>-R<n>` (and `CRASH-R<n>`, `STARTUP-R2-B<n>`, `R3-B<n>`, `SERVICE-R2-<n>`, `SERVICE-R3-<n>`). Nits were all fixed or accepted by the owner; they are not listed.

#### Blocking

| ID | Lane | Finding | How it was closed | Closing review |
|---|---|---|---|---|
| FEATURES-R1 | features | Demotion to a recovery record made every retryable reclaim failure permanent, and Delete reported success | Owner adopted the reviewer's fixes 1–3: proven records are re-reclaimed on each explicit Delete; the demoting call and Delete return an error while the reclaim fails. Ownership refusals were later classified separately (R11) | adv features r2 |
| APPLE-R1 | apple | The restart fallback re-admitted a linked archive modified while its removable drive was away | Owner: require `volumeIsInternal == true` (fail closed); removable drives keep Locate. The fix is unobserved on an iPhone (APPLE-R7) | adv apple r2 |
| TRANSFERS-R1 | transfers | ACQ-11 removed the only truncation guard: a close-delimited `200` cut short was published as a complete archive (reviewer: Should-fix, borderline; **owner raised it to Blocking**) | ZIM header length check (`checksum_pos + 16 == size`) before every publication path, including startup finalize and native adoption; Retry fetches the rest. Exercised only with fixtures | adv transfers r2 |
| FILES-R1 | files | Abandoning a move deleted a verified staged copy without proving the source still existed, then said the archive "is still in" a missing folder | `source_intact` proof before any staged file, folder or journal is removed | adv files r2 |
| SERVICE-R1 | service | Android and iOS no longer compiled (`has_book_at_path` is desktop/test-only) | Prefilter gated `cfg(not(mobile))`; Android clippy exit 0. iOS was never compiled | adv service r2 |
| CIDOCS-R1 | cidocs | The new `linux-tests` job failed every run: a Rust test needs Node with `typescript`, and the job never ran `npm ci` | `setup-node` and `npm ci` added to the job. Simulated locally, not run on GitHub | adv cidocs r2 |

No other adversarial finding was graded Blocking.

#### Should-fix

| ID | Lane | Finding (short) | How it was closed |
|---|---|---|---|
| APPLE-R2 | apple | A provider that stops exposing an identifier was admitted without content binding | Fixed; `sameNode` requires an observed identifier (adv apple r2) |
| APPLE-R3 | apple | Stale download activity gave no explanation | `staleLabel` added (adv apple r2) |
| APPLE-R7 | apple | Restart proof depends on `volumeIsInternal == true`, never observed on an iPhone | **Open as a device verification gate**; APPLE-1 recorded as mitigation, unverified |
| APPLE-R8 | apple | "Open Kiwix to see progress" English in every locale | Owner added the Rust-only strings to `extra-en-rust.json` and regenerated; still untranslated |
| CRASH-R1 | apple-crash | Swift regex redaction failed open above ~0.5 M characters on one line | Input bounded to 32,768 scalars in the canonical spec, all three implementations (adv apple-crash2) |
| CRASH-R2 | apple-crash | A path after `<` or a typographic quote not recognised | Path start set widened in Rust, Swift and Kotlin together (spec) |
| CRASH-R4 | apple-crash | Fixture XCTest skipped (and so passed) when the fixture was unreadable | Fails on simulator/macOS, skips only on device; floor 238 in Swift and Kotlin (adv apple-crash3). The `XCTFail` branch has not met Apple's compiler |
| CRASH-R5 | apple-crash | Two secret shapes leaked in Rust and Swift (wrapper + whitespace; `)` after a quoted key) | Owner: fix in all three; spec v2.3 (adv startup r5, apple-crash3, android-crash2) |
| ANDROID-R1, R2, R4, R5 | android | False 4 s start timeout; export file leaked on recreate; Pause stop under queued start; unknown codes skipped the localized fallback | Fixed (adv android r2) |
| ANDROID-R8 | android | `stopOwned` called `stopService` while a foreground start was queued (pre-existing) | `queuedForegroundStarts` count (adv android r3) |
| ANDROID-R10 | android | Kotlin redacted secrets before paths, so a spaced path tail survived | Superseded by the canonical pipeline port (adv android-crash) |
| ANDROID-R11 | android-crash | No CI job runs the plugin's JVM tests | **Open process residual** (see [§6.9](#69-process-residuals)) |
| BUILDHYGIENE-R1 | buildhygiene | `FORKS.md` still described the zimru migration as a future plan | Fixed in round 2; closed (adv buildhygiene r2) |
| BUILDHYGIENE-R2 (unnumbered should-fix) | buildhygiene | `DEPENDENCIES.md:3` described the 2026-11-28 remeasurement as the post-reduction snapshot | Owner applied the reviewer's three-column table and replacement sentence; checked in this record's review |
| CIDOCS-R2, R3, R4 | cidocs | Plural order depends on the Node version; error-docs gate depends on the path separator; §1.2 wrong about `map_recipe_metadata` | Fixed (adv cidocs r2) |
| DESKTOP-R1, R2, R4, R5 | desktop | Hook replaced `cleanup_before_exit`; recorded version cleared too early; mobile reset rewrote a newer profile; read-only → writable dropped skipped records | Fixed (adv desktop r2) |
| DESKTOP-R3 | desktop | APPLE-2 not revived from the availability pass | Service lane added the `sandbox::revive()` call; owner verified it present (`dispatch.rs`) |
| DESKTOP-R7 | desktop | Localization manifest and `docs/ERRORS.md` checks fail | Owner regenerated both at integration |
| FEATURES-R2, R3 | features | Crash scratch not swept at startup; FEAT-6 not user-reachable | Handoffs to service (startup sweep) and panes (Dismiss UI) |
| FEATURES-R4, R8, R11 | features | Failed sync still checkpointed; record dropped on path-only evidence; ownership refusals classed as busy | Fixed (adv features r2, r3, r4) |
| FILES-R2 to R6 | files | Orphan journal dropped; flaky single-slot test hooks; fallback leftovers; bind-mount re-hash; unplugged drive treated as missing | Fixed (adv files r2) |
| NET-R1, R2, R3, R4 | net | Hyper-V switch regression; old receipt admitted indefinitely; CORE-3 test could not fail; Windows VPN/VirtualBox adapters | Fixed or fixed differently (R1 narrowed to vEthernet; R2 age-conditioned); R4 partial → R7 (adv net r2) |
| NET-R7 | net | Routed preference picked an unflagged Windows VPN | Routed preference disabled on Windows; IfType gating declined (adv net r3) |
| OWNER-R1, R2, R3 | owner | Localization test missed three call shapes; spec frame note wrong; §1.2 stale | Fixed (adv owner r2) |
| OWNER-R4 | owner | `RELIABILITY.md` section cites a missing file and claims final-tree verification not yet run | **Open** until this record and the final-tree verification exist and are re-reviewed |
| PANES-R1, R6 | panes | Catalog `exact` match could mark another flavour or variant as installed | `exact` follows Rust `series_compatible`; badge uses the looser rule (adv panes r3) |
| PANES-R8, R9, R10 | panes | iOS display redaction leaked folder tails; lookbehind could break the whole UI on old WebKit; extension gate fired on abbreviations | Lookahead-only regex gated on `.part`/`.zim(aa)`, compared in V8 and JSC (adv panes r5) |
| READER-R1 | reader | An archive script could take the token and session the shell bound | Native accessors captured at bootstrap, secret scrubbed, hello required before any `reader-init` (adv reader r2) |
| READER-R2 | reader | Held paint showed an unadmitted document for ~21 s | Held paint needs an announcement, capped at 3 s (adv reader r2) |
| READER-R4 | reader | Photo secret sent on every platform and in restricted mode | Sent and honoured only on Windows with restricted mode off (adv reader r3) |
| SERVICE-R2 to R6 | service | Rename-over undetected; cancelled monitored import recorded as bad file; cancellation notice dropped; readers retired on flip to available; verification blocks background writer | Fixed (adv service r2); R6 by cancel-on-background |
| SERVICE-R2-1, R2-2, R2-3 | service | Provider books re-acquired every pass; notice could stay hidden or stale; cancel on background orphaned a staged copy | Fixed (adv service r3); R2-3: remove own `<uuid>` staging on `IMPORT_CANCELLED` |
| SERVICE-R3-1 | service | A dismissed notice that came back stayed hidden | Dismissed set pruned; only undismissed notices rendered (adv service r4) |
| SHELL-R1, R2, R2b | shell | New alert duplicated inline owners; gated directory showed "Saving…" and Quit dropped it; blur on hide bypassed the gate | Double owners removed by panes; single gate in `commit()` (adv shell r3) |
| STARTUP-R1 | startup | Failed-Quit dialog could leave a windowless Kiwix holding the profile | Fixed (adv startup r2) |
| STARTUP-R2-B1 to B5, R3-B1, R3-B2 | startup | Canonical redaction leaks (escaped JSON keys, camelCase keys, wrappers, path starts, spec precision, `://` exception, common key names) | Spec v2 → v2.3, vectors 110 → 238; Rust leak searches 0 leaks; B6 (quadratic cost) fixed to linear (adv startup r3–r5) |
| TRANSFERS-R2, R3, R4 | transfers | Retry after a non-ZIM answer never contacted the server; tiny progress reset the stall counter; `WEB-SEEDS.md` contradicted the code | Fixed (adv transfers r2) |

---

## 4. Owner decisions and accepted deviations

Each line is a decision recorded in the ledger or in a review report. "Accepted" means the owner chose not to change code further.

### 4.1 Decisions that set behaviour

| Area | Decision | Source |
|---|---|---|
| iOS linked archives (APPLE-1/R1) | The restart fallback applies only on internal volumes (`volumeIsInternal == true`), fail closed. Removable, network and unknown volumes keep "Choose Locate", so the original "drive unplugged and re-plugged" trigger is deliberately not fixed | LEDGER (adv-apple); adv apple r2 |
| iOS digest (APPLE doubt 7) | `digestSatisfied` refuses a recomputed digest that differs, even for the same device/inode/mtime/size | LEDGER; adv apple r2 |
| iOS rebind write (APPLE doubt 6) | The record rewrite in read-only or private mode is accepted, same class as the existing stale-bookmark refresh | LEDGER |
| Read-aloud Live Activity (APPLE-R5) | Stale window lengthened to 600 s (10 min); the download activity stays at 120 s | LEDGER; adv apple r2 |
| Reader photo secret (READER-R4) | Issued and honoured only on Windows with restricted mode off | LEDGER; adv reader r3 |
| Reader admission (READER-R1) | A hello is required before any `reader-init`; a document that never announces is not admitted. A MessagePort-based partial fix was declined; the forged-hello + `document.open()` case stays a residual (needs a native per-frame channel) | LEDGER; adv reader r2 |
| Web-seed pace (TRANSFERS trickle, R10) | Minimum progress 128 KiB per 60 s window (accepted "provisionally"). No deadline for the whole web-seed stage: a mirror that keeps exactly to the floor can hold the stage, and the peers, for as long as the file takes; this is disclosed in `docs/WEB-SEEDS.md` | LEDGER; adv transfers r3, r4 |
| Truncated downloads (TRANSFERS-R1) | Raised to Blocking; fixed by the ZIM header length rule | LEDGER |
| Windows web-sharing adapter (NET-R7) | Routed preference disabled on Windows. Reading the adapter IfType (new `unsafe` via `windows-sys`) was declined; the residual stays | LEDGER; adv net r3 |
| Windows Hyper-V and receipts (NET-R1, R2) | The implementer's narrower fixes (vEthernet only; receipt TTL conditioned on list age) accepted as deviations | LEDGER; adv net r2 |
| Loopback-only sharing (NET-R6) | Reported through an `addressLost`-style `localOnly` flag with a two-observation debounce | LEDGER |
| `directConnection` (NET-10) | Kept as a residual until it can be tested on two Apple devices | LEDGER; adv net r3 |
| Mobile imports (SERVICE-R6) | Manual imports and incoming copies are cancelled when the app goes to the background or the screen locks; the person adds the archive again or the copy is offered again | LEDGER; adv service r4 |
| Cancelled staged copies (SERVICE-R2-3) | On `IMPORT_CANCELLED` the app removes its own `mobile-imports-v1/<uuid>/` staging; a copy it cannot prove is kept (accepted leak) | LEDGER; adv service r3 |
| macOS Table of contents | ⌥⌘T accepted (⌘M left to Minimize) | LEDGER; adv desktop r3 |
| PDF scratch sweep (FEAT-3) | `STALE_AFTER` stays at 1 h | LEDGER |
| Map power guard (FEAT-8) | The existing option set kept (same as LocalSwarm); untested residual | LEDGER |
| Map recovery Delete (FEATURES-R1, R11) | Proven records are retried on each Delete and the error is repeated while the reclaim fails; `cancel()` keeps demotion and returns Ok. Ownership refusals became a typed `Refused` outcome rather than "busy" | LEDGER; adv features r4 |
| Map `record_vanished` (FEATURES-R8) | Strict rule: a record is dropped only when the journaled folder holds none of its names; an identity mismatch is report-only | LEDGER |
| LIB-7 | A folder never seen with its archives in this run, and Windows, keep missing entries as unavailable (removable by hand) | LEDGER; adv service r2, r4 |
| LIB-1 on Android | `linkat` refusal accepted: the move fails with a clear message | LEDGER; adv files r1 |
| Catalog matching (PANES-R1, R6) | `exact` (which hides Download) only when flavours and variants are compatible under Rust `series_compatible`; the "Installed edition" badge keeps the looser rule | LEDGER; adv panes r3 |
| Startup failure dialog | Dialog failure → backoff; at least 1 s between automatic retries with no window | LEDGER; adv startup r4 |
| CI (CIDOCS-R8) | Frontend build split into its own job so a failing JS or browser test no longer skips the Rust jobs | LEDGER |
| Kotlin unit tests in CI (ANDROID-R11) | Not added (needs a generated `tauri.settings.gradle`, unverifiable here); residual, local command to be documented | LEDGER |
| Desktop updater (BUILD-2) | Shell changes in the ~600 ms before the Windows installer hook are not saved | adv desktop r3 |

### 4.2 Canonical crash-redaction rule

One rule now governs the text the three crash intakes write to disk: `src-tauri/tests/fixtures/crash-redaction-spec.md` (final at v2.3) with `crash-redaction-cases.json` (238 vectors). Rust (`crash.rs`), Swift (`CrashRedaction.swift`) and Kotlin (`CrashRedaction.kt`) must each reproduce every vector.

- **Origin.** The startup lane fixed CORE-8 in Rust; the Swift and Kotlin halves were found to stop at whitespace too. The Apple reviewer found that Rust's paths-before-secrets order let a secret value survive (CRASH-R1/R2); the Android reviewer found that Kotlin's secrets-before-paths order leaked a spaced path tail (ANDROID-R10). The owner decided on one pipeline, written as a spec, with shared vectors and a differential test across all three.
- **Steps.** Bound the input to 32,768 scalars (dropping a trailing partial token), secrets, URLs, e-mail addresses (`[email]`, from v2.1), paths, opaque tokens, whitespace, length.
- **Kept URLs.** Only `http` and `https` URLs whose host is a public DNS name are kept, as the lowercased scheme, `://` and the host, plus a non-default port. Path, query, fragment and userinfo are dropped (the owner's decision was "keep `scheme://host[:port]` only, drop external paths, Rust too"). Every other URL becomes `[url]`; loopback reader URLs become `[reader-url]`.
- **Accepted gaps (spec §4).** A name containing `: `, `<`, `>`, a backtick or an unescaped `"` ends redaction early; paths without a start shape (relative paths, `%USERPROFILE%`, `\Program Files\…`, `D:Users\…`); the kept external host, which can itself be personal; `git@host:account/repo`; an unquoted secret with spaces, or a struct/array/map value, loses only its first token; unlisted keywords, `OAuth=`, no-separator forms and any `_` suffix after a keyword (`accessToken_value=`); an empty quoted value; non-ASCII, dotless or quoted-local-part e-mail addresses.
- **Accepted over-redaction (spec §5).** Words after a path up to the next stop; `/` after punctuation or a non-ASCII letter; escape runs; words after keywords in prose and camelCase type names; `~5/10`-style text.
- **Platform residual.** Android's default handler still writes a raw `FATAL EXCEPTION` to logcat (adv android-crash).

### 4.3 Accepted deviations from a reviewer's proposal

- **SHELL-R2b:** a typed download directory is never lost or applied without confirmation, but a switch edit withdrawn by Quit reverts to the confirmed state (adv shell r3).
- **DESKTOP-R9:** preserved profile copies are never cleaned up and keep reading history after Clear history (adv desktop r3).
- **Desktop comparator:** two pre-releases of the same version are not ordered, so a running rc.2 can still mention a recorded rc.1 (cosmetic; adv desktop r3).
- **PANES:** kept-file map paths appear both in the alert text and the recovery list (accepted Nit; adv panes r5).
- **FILES R2 proxy:** the orphan-journal rule uses "at least one non-session book" as a stand-in for fresh state (adv files r3).
- **APPLE-R4, R9, R10:** volume identity read by path; no stale text on the expanded island; positive restart XCTests assume the host volume reports internal (adv apple r2).
- **ACQ-6:** kept bytes keep their mirror; no cross-mirror resume.
- **SHELL-1:** narrower than the reviewer's suggested regex (an explicit list of four producers, enforced by a scan test).

---

## 5. Verification actually exercised

### 5.1 What was compiled and run, and what was not

All commands ran in one shared Linux x86_64 container (4 cores, no IPv6 in the kernel).

| Target | What happened |
|---|---|
| Linux host Rust | Compiled. `cargo test --lib` with per-lane filters; full-suite runs by reviewers; `cargo clippy --lib --tests --features custom-protocol -- -D warnings` |
| Windows Rust | **Type-check only**: clippy for `x86_64-pc-windows-gnu` (MinGW), `--lib --tests`. Nothing ran on Windows; the MSVC target was not built |
| Android Rust | **Type-check only**: `android-clippy.sh` (clippy for `aarch64-linux-android`, plus the opt-in location variant). No `--tests`, nothing ran |
| Android Kotlin | Compiled with Gradle (offline) for the plugin and app module; the plugin's **JVM unit tests** ran (final count 79 tests, 0 failures). No emulator or device |
| macOS and iOS Rust | **Never compiled.** Every `cfg(target_os = "macos")`/`cfg(target_os = "ios")` change (menus, `sandbox::revive`, FEAT-8, ACQ-3, iOS-only parts of `live_activity.rs` and `mobile.rs`) is unverified by any compiler |
| Swift | **Syntax only** (`swiftc -parse` of `ios/Sources`, `ios/Tests`, `gen/apple/DownloadActivity`). The Linux Swift 6.0.3 toolchain also compiled and ran `CrashRedaction.swift` unmodified, and harnesses built from extracts of `DocumentStorage.swift` and the widget code with shims (see [§8](#8-source-conflicts-and-gaps-in-the-record)). No Apple SDK, no `xcodebuild`, no XCTest run |
| Frontend | `tsc --noEmit`; Node tests; headless-browser tests through a Chromium `--no-sandbox` wrapper (`KIWIX_TEST_CHROME`) |
| Other engines | Reader attack scripts ran in Chromium and in WebKitGTK under `xvfb-run` against copies of the admission rules (not the app). The panes redaction regex was compared in V8 and JavaScriptCore 2.52.6 |
| Fuzzing | Fuzz crate type-checked (`cargo check`); **no fuzzing was run** |
| CI | Workflow YAML parsed and the `paths` filter simulated locally. **No GitHub Actions run** of any changed workflow |

No device, Mac, Windows machine, iPhone simulator or Android emulator was used, and no Apple compiler ran.

### 5.2 Baseline at `d98cac2` (LEDGER)

- `npm run build` ok; Node tests 399 pass / 3 skip; Python unittest 182 ok; `kiwix-mcp` clippy and 9 tests ok; Linux clippy ok; Windows-gnu clippy ok; `win-security` clippy ok.
- `cargo test --lib` on Linux: 1329 pass, **5 fail** (all torrent "error starting listeners", the OWN-1 tests), 16 ignored.

### 5.3 Per lane, as the reports state

The lines below are the final-round results each lane's implementer (I) or adversarial reviewer (A) reported. Many runs failed first on other lanes' mid-edit files and were retried; the reports say so and those intermediate failures are not repeated here. These results describe the tree at the time of each run, not the final tree.

**transfers**
- I r4: `cargo test --lib -- torrent:: acquisition::` → `ok. 221 passed; 0 failed; 3 ignored`; unfixed-file namespace run of the pace tests → `2 passed; 1 failed` (the new test, "ending the stage took 22.58 s"). Host clippy exit 101 in all six attempts, errors only in other lanes' files (`store.rs:2456`, `crash.rs:530/820`, `service_tests.rs:6288/6292`).
- A r4: same filter → `ok. 221 passed; 0 failed; 3 ignored`; host clippy exit 101 (only `crash.rs`). "There is still no exit-0 host clippy run that includes this round's change."
- A r2: full suite `cargo test --locked … --lib` → `ok. 1435 passed; 0 failed; 16 ignored` (about 01:00 UTC; the newest full-suite result in the record).
- I r2: host clippy and Windows-gnu clippy exit 0; revert run `2 passed; 5 failed` (new tests).

**features**
- I r5: `cargo test --lib -- maps:: kokoro native_pdf` → `ok. 141 passed; 0 failed; 3 ignored`; host clippy exit 0; check-disabled copy → `74 passed; 1 failed`. Windows-gnu clippy last exit 0 in round 4.
- A r5: same filter → `ok. 141 passed; 0 failed; 3 ignored`.

**files**
- I r3: `library_files::` 52 passed; `publication::` 24 passed; `edition` 25 passed; host and Windows-gnu clippy clean; Android clippy failed on four other-lane lints only. I r2: Android clippy clean; `library_files::` binary 20 runs `pass=20 fail=0`.
- A r3: `cargo test --lib -- library_files:: publication:: edition` → `ok. 101 passed; 0 failed`.

**shell**
- I r3: `tsc` exit 0; `node --test scripts/test-*.mjs` → 528 tests, 512 pass, 2 fail (the two regeneration gates), 14 skipped; lane files 57/57; browser tests 6/6 three times.
- A r3: lane node files (error-visibility 21, shell-dismiss 11, settings-surface 10, updates-settings 5, camera-window 4, provider-recovery 6) all pass; `tsc` exit 0; `test-shell-surfaces` + `test-frontend-surfaces` with Chrome → 6/6.

**reader**
- I r3: reader and policy tests → 242/242; `node --test scripts/test-*.mjs` → 525, 509 pass, 2 fail (regeneration gates), 14 skipped; fail-without-fix in a scratch copy → 3 failures.
- A r3: reader and policy tests 242/242; `tsc` exit 0. A r2 also ran Chromium/WebKitGTK attack scripts and 22/22 browser tests (`dist/` not rebuilt, so `test-startup-navigation` did not exercise the current `Reader.tsx`).

**apple**
- I r2: `swiftc -parse` clean; storage harness `ALL PASSED` (38 ok); widget harness `ALL PASSED`; `test-privacy-manifest.mjs` 5/5; unsafe census 75/75.
- A r2: `cargo test --lib live_activity` → `8 passed; 0 failed`; `swiftc -parse` exit 0; privacy test 5/5; harnesses all passed.

**apple-crash**
- I r3: fixture `238 cases, 238 match, 0 differ`; bound checks passed; differential 0 mismatches.
- A crash3: 4,159,000 differential cases against a Rust reference rebuilt from `crash.rs`, 0 mismatches; fixture 238/238; slowest full-length input 8.5 ms; `swiftc -parse` exit 0.

**android**
- I r3: Gradle compile (plugin + app) `BUILD SUCCESSFUL`; plugin unit tests 70, 0 failures; five-decision revert → 6 failed.
- I r2: `cargo test --lib user_error` → `18 passed`; `android-clippy.sh` exit 0; `test-system-text-scale` + `test-android-manifest` 11/11.
- A r3: Gradle compile and tests → 70 tests, 0 failures.

**android-crash**
- I r2: 79 tests, 0 failures (`CrashRedactionTest` 12); differential `inputs 581458 differ 0`.
- A crash2: Gradle `BUILD SUCCESSFUL`, 79 tests, 0 failures; differential against rebuilt Rust: 300,000 + 1,346 + 151,020 + 40,000 inputs, 0 differences.

**net**
- I r3: lane tests → `ok. 215 passed; 0 failed; 5 ignored`; host clippy exit 0; Windows-gnu clippy exit 0 (cargo judged it fresh); Android clippy exit 0; `tsc` exit 0; `test-nearby-sharing-surfaces` 3/3.
- A r3 (at `618b809`): lane tests 215 passed; host clippy exit 0; `android-clippy.sh` exit 0; Windows-gnu clippy exit 0 with a real recheck; `tsc` exit 0.

**startup**
- I r5: `cargo test --lib -- crash::` → `ok. 10 passed` (238-vector fixture and linear-time tests); host clippy exit 0; leak search `0 leaks in 224048`.
- A r5: `crash::` → `ok. 10 passed`; scratch pipeline reproduces 238/238; 38 adversarial units at 32,768 scalars ≤ 5.8 ms. A r4: `crash:: quit:: launch_tests` → `32 passed`.
- I r1: Windows-gnu clippy never fully clean in round 1 (other lanes' test lints); I r2 and r3: Windows-gnu clippy exit 0.

**cidocs**
- I r2: YAML parse ok; Chrome step script extracted and run → exit 0; six Node test files → 37 tests, 33 pass, 2 fail (regeneration gates), 2 skipped; with Chrome and `CI=true` → 7/7.
- A r2: YAML parse; `paths` simulation; in a scratch copy, `generate` on Node 22 and Node 24 byte-identical, error-docs write then `--check` exit 0; repository read-only: 16 pass, 2 fail (regeneration gates).

**desktop**
- I r3: `cargo test --lib -- store:: update:: windows::` → `109 passed; 0 failed; 1 ignored`; host and Windows-gnu clippy `Finished`. I r2 also: Android clippy `Finished`.
- A r3: same filter → `ok. 109 passed; 0 failed; 1 ignored`.

**service**
- I r4: lane tests (`service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf:: store::`) → `ok. 484 passed; 0 failed; 6 ignored`; host clippy `Finished`; Android clippy exit 0; `tsc` exit 0; `test-startup-notice.mjs` 3/3. I r3 also: Windows-gnu clippy `Finished`; the lane tests twice with no flake; revert copy → 4 failed.
- A r4: notice reproduction script as expected; `test-startup-notice` 3/3; `tsc` exit 0; lane tests 484 passed.

**panes**
- I r5: `tsc` exit 0; nine pane node files 116/116; `node --test scripts/test-*.mjs` → 531, 515 pass, 2 fail (regeneration gates), 14 skipped; `test-pane-surfaces` with Chrome 3/3 three times; JSC harness.
- A r5: `tsc` exit 0; pane node tests 116/116; `test-pane-surfaces` 3/3; V8 vs JSC on 3,021 inputs, 0 differences.

**buildhygiene**
- I r1: `cargo metadata --locked` ok; `Cargo.lock` unchanged; host clippy default and opt-in clean; Windows-gnu clippy exit 0; Android default and opt-in clippy exit 0; `cargo test --lib permissions` 10 passed, `mobile_policy` 17 passed; `test-build-hygiene.mjs` 5/5.
- A r1: `cargo tree` for zbus on nine targets → none by default; opt-in builds receive `--cfg native_location`; `permissions mobile_policy` 27 passed; with the opt-in, `location_linux permissions` 15 passed; base checkout fails the new checks.
- I r2: `test-build-hygiene.mjs` 5/5; host clippy default and opt-in exit 0; `test-linux-appimage.py` OK. Android and Windows-gnu not re-run.
- I r3 (docs only): `test-build-hygiene.mjs` 5/5.
- A r2 (rounds 2 and 3): `test-build-hygiene.mjs` 5/5; `cargo tree` remeasurement of macOS, iOS and Windows counts; `Cargo.lock` 739 entries now and at base.

**owner**
- A r2: `node --test scripts/test-rust-localized-strings.mjs` → pass 1; a scratch copy with three entries removed fails naming exactly those three.
- Integration steps the LEDGER records (owner, not reviewed):
  - `docs/ERRORS.md` regenerated: `check:errors` OK, `test-error-docs` 5/5; plural source "Used today: {count} requests" added.
  - `localization.mjs generate`: sources 1552 → 1669; `check:localization` failures `[]`; coverage about 0.80 → about 0.74.
  - `npm run build` OK.
  - Node suite with Chrome: 545 tests, 544 pass, 0 fail, 1 skip (macOS-only).
  - Python unittest 182 OK (2 skipped); `test-linux-appimage.py` OK.

  The LEDGER places these after buildhygiene round 2 and before round 3's documentation edits; they are not the final-tree run.

### 5.4 Final-tree integration verification

All of the following ran on the integrated tree after the last code change of every lane had been made. The Rust sources, `build.rs`, `Cargo.toml` and `Cargo.lock` are identical from checkpoint `fbcb420` to the final commit. After the first runs, the record review found two uncollected plural sources (§6.9); the catalogs were regenerated, and the Rust library tests, Node, Python, build, localization and error checks were run again on that final data. The rows below give the final run. Clippy, the unsafe census, Kotlin and Swift do not read the catalogs and were not re-run. Every log, with the command it ran, is kept as `verification/*.log` in the evidence commit named in §7. Commands are run from the repository root unless noted; `SHERPA_ONNX_ARCHIVE_DIR` pointed at locally downloaded sherpa-onnx v2.0.1 archives because the speech library's build script cannot download through this container's proxy.

| Check | Command | Result |
|---|---|---|
| Rust library tests, host (Linux x86_64) | `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib` | `ok. 1493 passed; 0 failed; 16 ignored` (base `d98cac2`: 1329 passed, 5 failed, the 5 being the IPv6-less torrent listener defect fixed here) |
| Clippy, host | `cargo clippy --locked … --lib --tests --features custom-protocol -- -D warnings` | exit 0 |
| Clippy, Windows type-check | same, `--target x86_64-pc-windows-gnu` with `KIWIX_TYPECHECK_ONLY=1` and MinGW | exit 0 |
| Clippy, Android type-check | `aarch64-linux-android`, NDK 28.2, `--features custom-protocol,android-evaluation` | exit 0 |
| Clippy, opt-in location providers | host, `KIWIX_EXPERIMENTAL_LOCATION=1 KIWIX_TYPECHECK_ONLY=1 … --features custom-protocol,experimental-location` | exit 0 |
| Unsafe census | `node scripts/check-unsafe-census.mjs` | `75 matches / 75 blocks / 2 impls in 20 files; every item documented` |
| Lockfile | `git diff --quiet d98cac2 -- src-tauri/Cargo.lock` | unchanged |
| Error inventory | `npm run docs:errors`, then `npm run check:errors` | `docs/ERRORS.md matches the current sources.` |
| Localization inventory | `node scripts/localization.mjs generate`, then `npm run check:localization` | `"failures": []`; English sources 1552 → 1669 |
| Frontend build | `npm run build` | succeeded (existing chunk-size and dynamic-import warnings only) |
| Node tests, with a real headless Chromium | `KIWIX_TEST_CHROME=<no-sandbox wrapper> node --test scripts/test-*.mjs` | 545 tests: 544 pass, 0 fail, 1 skipped (a macOS-only finalizer test); the same result before and after the catalog fix |
| Python tests | `python3 -m unittest discover -s scripts -p 'test_*.py'`; `python3 scripts/test-linux-appimage.py` | 182 OK (2 skipped); 1 OK |
| Kotlin JVM unit tests | from `src-tauri/gen/android`, with `ANDROID_HOME`, `JAVA_HOME` (JDK 21) and `GRADLE_USER_HOME` set: `gradle --no-daemon --offline --max-workers=2 --console=plain :tauri-plugin-kiwix-mobile:testDebugUnitTest --rerun` (Gradle 9.6.1; `src-tauri/gen/android/tauri.settings.gradle` is generated and git-ignored, and was hand-written here to point at the Tauri and plugin Android projects). Results are the JUnit XML files under `src-tauri/plugins/tauri-plugin-kiwix-mobile/android/build/test-results/testDebugUnitTest/` | 79 tests, 0 failures, 0 errors, 0 skipped |
| Swift crash redaction | Linux Swift 6.0.3 harness over `CrashRedaction.swift` and a `rustc` build of `crash.rs`'s redaction functions | fixture 238/238; bound checks all passed; 1,600,500 differential cases, 0 mismatches |
| Swift syntax | `swiftc -parse` over `ios/Sources/*.swift` and `ios/Tests/*.swift` | clean |

Not run on the final tree: anything listed in §5.5, `cargo deny`, `npm audit`, and the GitHub Actions workflows themselves.

### 5.5 Not run anywhere

- Any physical device, Mac, Windows machine, iPhone simulator or Android emulator; any Apple SDK build or XCTest run; any macOS or iOS Rust compile.
- **The app itself was not launched on any platform, Linux included.** Only library-level tests, the Node and Python suites, and headless-Chromium tests of the built frontend (`dist/`) ran; no Tauri window, WebKitGTK, WebView2 or WKWebView ever loaded it.
- Any GitHub Actions run of the changed workflows, the path filter, the artifact hand-off, the CI Chrome step or the Windows test step.
- Real NFS/FUSE, removable media, mount/unmount, USB replug, the real Trash, real sleep/wake address changes, a host with IPv6, a middlebox truncating a response, the real 60 s / 128 KiB pace window, the 120 s stall limit and 15 s connect timeout.
- The real failed-Quit dialog and zenity/kdialog exit codes, the Windows installer hook, IME `keyCode 229` in WKWebView/WebKitGTK, a real updater download, real Kokoro playback with a model, and fuzzing.

---

## 6. Residuals and blockers

From each lane's final adversarial report and the LEDGER. None of these blocks closure of its lane; together with [§5.5](#55-not-run-anywhere) they bound what this work shows.

### 6.1 Apple (iOS and macOS)

- **APPLE-1 is a mitigation, unverified on device.** It depends on `volumeIsInternal == true` for the iPhone data volume, which nobody has observed (APPLE-R7). If iOS reports nil or false, behaviour is exactly the base (Locate). Next step: on the test iPhone, log `volumeUUIDString`, `volumeIsInternal`, `st_ino` and `fileResourceIdentifier` for files in On My iPhone, iCloud Drive and a third-party provider, before and after a restart.
- A replugged external drive still needs Locate (owner decision). On built-in storage a file rewritten in place with its mtime restored is admitted after a restart, as within one boot.
- Non-stale reopen for identifier-less providers checks only size and the first part's UUID (pre-existing; a rehash per open was judged too costly). The ledger says this should become its own documented finding; no document in the tree records it yet.
- APPLE-6 "% read" and the new "Open Kiwix to see progress" are English in non-English locales; the expanded Dynamic Island shows no stale text.
- No Apple SDK build of the plugin, the widgets or the test target; the `XCTFail` branch of the crash fixture test has not met Apple's compiler.
- macOS-only Rust never compiled: menus (⌥⌘T, ⌘,, ⌃⌘F), `sandbox::revive`, FEAT-8 power assertion.

### 6.2 Android

- `onNewIntent` before the plugin loads still loses the intent (pre-existing); a file is lost if the process dies before the first offer.
- `layoutDirection` configuration changes still recreate the Activity (ANDROID-5 partial).
- The Rust sentence table for native codes is English-only (better than raw JSON).
- Dead nearbyRoute commands kept (ANDROID-11); device-scan sentences still composed in Kotlin (ANDROID-12, H4).
- If the process dies between `startForegroundService` and delivery, a restarted service could be admitted without a Rust owner (milliseconds; same as base).
- Logcat `FATAL EXCEPTION` from the default handler stays raw (platform).
- Never run: service start/stop, the platform rules for redelivering a queued start, TalkBack, rendering, R8-minified builds, a crash record written on a device.

### 6.3 Desktop, startup and updater

- BUILD-2 partial: the Windows installer hook does not run the shell's in-memory session flush.
- DESKTOP-R9: preserved profile copies are never cleaned up and keep history after Clear history.
- Updater comparator does not order two pre-releases of the same version (cosmetic).
- Never run: the real Windows installer hook, `cleanup_before_exit`, minimized rectangles, the App Sandbox `lstat` behaviour `present()` relies on, the live failed-Quit dialog and its window parenting, the backoff sleeps, zenity/kdialog exit codes.

### 6.4 Library, files and service

- LIB-7: entries in a folder never seen with its archives in this run, and on Windows, are kept as unavailable; a file imported from under an unmounted mount point is retired once the volume covers it again.
- LIB-9: cross-device copy into the home Trash still not prevented. LIB-1 on Android: refusal, not a working move.
- An interruption between capture and unlink still needs the person to delete the hidden `.kiwix-renamed-…` file (the message points at it). Discard can report `missing` if a leftover mount point is not empty. A device number that changes between mounts keeps the record open.
- After FILES-R5, a same-volume move that Windows startup recovery cannot finish (archive open at launch) is cancelled, not retried; the comment above it in `dispatch.rs` was corrected in service round 3. The person has to move the archive again.
- Provider-backed books get no rename-over detection (by design).
- A cancelled staged copy that cannot be proved is kept (accepted leak). After Android kills the process in the background, an incoming share's permission may be gone once the copy was discarded.
- Mobile imports are cancelled on background or screen lock (owner decision).
- Startup band heading "Some saved items need attention." does not fit import cancellations (Nit). A notice withdrawn and re-noted before the next snapshot stays hidden (Nit).
- `mobile_policy.rs:566`: the service lane left it; the buildhygiene lane removed the stale entry (round 1, reviewed).

- The FILES-R2 orphan rule is a stand-in ("the library lists at least one other book"), so a library whose only book was removed mid-move keeps showing its move warning at each launch.
- The startup notice band (SERVICE-R3-1) was exercised only in a Node test of its rule, never seen rendered in a real shell.

### 6.5 Maps, read-aloud and PDF

- A crash during the publication move on a filesystem without exclusive rename leaves the partial and the final `.zim` as one file with two names; nothing heals it (no bytes lost).
- On a filesystem that has exclusive rename, twin-name step 1 returns "file exists", so Delete fails on each press (needs a hand-made hard link).
- A leftover name inside the cleanup folder heals only on the next put-back (two Deletes).
- `partials.rs` duplicates the `.kiwix-renamed-…` name format from `publication.rs`.
- `status()` still stats the capture path.
- FEAT-8 never compiled and has no test. The real link-then-unlink fallback, the Kokoro `run` loop with a model, Windows, macOS, iOS, Android and removable media were not exercised.

### 6.6 Downloads and torrents

- Peers wait for the web-seed stage: a floor-keeping mirror can hold the stage as long as the file takes at about 2 KiB/s; a "fast piece, then trickle" mirror up to about 33 minutes per listed mirror; dead mirrors about 15 s to 4.5 minutes per start. No stage deadline (owner decision).
- A full-length staging file is SHA-1 hashed in full by the stage and again by the engine on every start, with no progress shown.
- A direct `.zimaa` download is refused and its message still suggests Retry.
- After a web seed gives up and no peer is present, the row shows 0 B/s with no error.
- The multi-file and BEP 47 padding repair paths are covered by reading only. iOS/macOS compile of ACQ-3 and of the R5 errno code, Windows/Apple errno values at runtime, and an IPv6 host were not available.

### 6.7 Sharing and Nearby

- NET-10 `directConnection` has no control (owner decision).
- Windows one-click Web sharing: a full-tunnel VPN whose private address sorts before the LAN is still chosen; the routed tie-break is not used on Windows (IfType declined).
- CORE-3's mobile dispatch arm cannot be caught by a host test (Android type-check only).
- NET-5: non-Apple LocalSwarm mDNS A records carry listen-time addresses. NET-4: Android backgrounding and iOS admission still stop sharing silently.
- Never exercised: Windows Hyper-V switch and VPN adapter names, Android hotspot waiter and Bluetooth off/denied, Apple interface tables and Local Network, sleep/wake address change, a 5-minute sender-gone receipt, a crash between `mkdir` and `rename` on a volume without `RENAME_NOREPLACE`.

### 6.8 Reader, shell and panes

- **Reader:** on Windows with restricted mode off, a forged hello plus `document.open()` can still receive `reader-init` with the photo secret, so the "Add a photo" dialog can appear without a click in the page; closing it needs a native per-frame channel or isolated world. READER-3 handshake-before-load is not done; external-link clicks before `load` are swallowed. The admission state machine is only harness-tested (no real webview).
- **Shell:** real desktop Quit, window close, Dock quit, logoff and the native failed-Quit dialog are untested; IME and blur timing in WKWebView/WebKitGTK unverified; `test-frontend-surfaces.mjs` keeps puppeteer's 30 s defaults. SHELL-7: a declined low-storage prompt is still shown once inline.
- **Panes:** iOS display redaction (exports keep raw text) still shows a tail for nested parentheses in a folder name and a stray `)`; an upper-case or unlisted extension before ". Capital" eats the sentence tail (nothing revealed). Two empty languages match `exact` in the frontend where Rust refuses. Suggestions return only on the next keystroke. A successful rollback drops focus to the page body. The share poll has no cap while open. Android rendering and TalkBack, the iOS display and the phone keyboard, the edition-original outcomes "trashed", "missing" and "external", and a real failing map Delete were not rendered; the Rust handlers for these were traced by reading.

- **Panes, further:** a Get map started from a card is dropped silently if another area's dialog opens while it is being prepared (base behaviour, caused by the person's own navigation); test 9 does not cover a reason sentence that itself contains a path; a real long torrent hash was never rendered.

### 6.9 Process residuals

- **ANDROID-R11:** no CI job runs the Kotlin plugin unit tests (pre-existing). The crash-redaction fixture check for Kotlin, and all the new Kotlin tests, run only when someone runs Gradle locally. A CI job needs the generated `tauri.settings.gradle`, which could not be verified here. The owner decided to keep this as a residual; the local command is in [§5.4](#54-final-tree-integration-verification).
- **Integrity test flake not reproduced:** `integrity::tests::real_valid_and_checksum_corrupt_results_survive_restart` failed once under load during service round 2 and passed 3 of 3 in isolation; 12 loaded runs in service round 3 passed. No panic line was kept, and `integrity.rs` is unchanged since base. A suspected (unconfirmed) cause is the store instance lock still held when it reopens after `drop(store)`.
- **New strings untranslated:** the localization generation collected the new strings (sources 1552 → 1669), so non-English locales show English for them; coverage fell from about 0.80 to about 0.74. Machine translation (`localization.mjs translate`) was deliberately not run. The generated catalogs were checked in this record's review: `generate` reproduces them byte for byte, and each sampled locale lost only keys whose English source was removed.
- **CI not run on GitHub:** the `paths` filter (simulated against GitHub's documented ordered-pattern semantics), the `frontend-build` split and artifact hand-off, the `linux-tests` job, the CI Chrome step, the opt-in location type-checks and the Windows test step in `build.yml` have not run on GitHub. If branch protection requires checks by name, the new "Frontend type-check and build" check is not in that list; the protection settings could not be read.
- **This record** was adversarially reviewed (record review r1: no blocking findings; all generated catalogs and `docs/ERRORS.md` regenerate byte for byte; no translation lost). Its should-fix items were applied and re-checked in record review r2.
- **Rust localization test limits:** `scripts/test-rust-localized-strings.mjs` finds literal call shapes itself, but call sites whose source is a variable are a hand-kept list (`VARIABLE_SOURCES`); a new variable-source call site is not found automatically.
- **Shared-library plurals fixed late:** the record review found that `{count} articles` and `Serving {count} archives…` were plural sources but not collected, so the Rust Localizer fell back to the raw English template ("1 articles") in every locale; this predates the work. Both are now collected (`extra-en-rust.json`), present in `messages/en.json` with their forms, and the test now requires every Rust plural source to be in the shipped catalog. The other locales show English for them until translated.
- **Unused plural sources:** five entries in `src/locales/plural-sources.json` (`{size} · {count} sources`, `{size} · {count} files`, `{count} connected peers · {size} uploaded`, `Another app sent {count} documents…`, `{count} bytes copied`) are used nowhere in the code; they predate this work and ship nothing.
- **Error inventory heuristic:** `docs/ERRORS.md` has no row for some user-facing Rust strings, for example the `IMPORT_CANCELLED` text and "Kiwix could not start.", because `error-inventory.mjs` collects strings by marker context. This is the extractor's existing behaviour, not a regeneration defect.

### 6.10 Build hygiene and location providers

- The Android opt-in location type-check covers aarch64 only; armv7 never type-checked the provider.
- Neither unreleased location provider (GeoClue on Linux, LocationManager on Android) has had a runtime test.
- A docs-only pull request that edits Markdown other than `docs/FORKS.md` skips the libzim-claim check in `test-build-hygiene.mjs` until the next pull request that is not docs-only (stated in the test's header).
- The historical "31 packages" figure for `tauri-plugin-single-instance` cannot be verified because the earlier history is squashed.

---

## 7. How the work is recorded

- **Reviewed state.** The reviewed state is the commit that removes `docs/reviews/2026-11-27/working/` and fills in `EVIDENCE_COMMIT` below. It is the child of the evidence commit and the first commit after `d98cac2` whose title does not start with `wip:`.
- **Evidence commit `EVIDENCE_COMMIT`.** The last `wip:` checkpoint. It holds `docs/reviews/2026-11-27/working/` complete: the LEDGER, the review, implementation and adversarial briefs, `RESUME.md`, the 11 area reports (`findings/`), every implementer report (`implementation/`) and adversarial report (`adversarial/`, including this record's own reviews), and the final-tree logs with their commands (`verification/`). Read any report cited here as `adv <lane> r<n>` or `impl <lane> r<n>` with `git show EVIDENCE_COMMIT:docs/reviews/2026-11-27/working/<folder>/<file>`.
- **Keep the history.** The evidence commit stays reachable only while this branch's history is kept. Merge without squashing or rebasing, or, if a squash merge is required, first tag `EVIDENCE_COMMIT` or copy `working/` somewhere durable.
- **The `wip:` commits are unreviewed snapshots.** Every commit from `d98cac2` up to and including the evidence commit is titled "wip: checkpoint in-progress review remediation (unreviewed)". They exist so in-flight work survived the session container. Apart from the evidence commit's role as an archive of the record, none of them is a reviewed state, and none should be released, bisected to as known-good, or cited as evidence for behaviour. The ledger notes one (`08f3094`) that captured a Kotlin file while it was mutated for a test.
- **Scratch evidence is not durable.** The harnesses, differential corpora and attack scripts that reviewers cite live in the session scratch directory, not in the repository, and may no longer exist; only the final-tree logs were copied into the evidence commit. The results quoted here are as the reports state them.
- **Base and changed paths.** The base is `d98cac2`; the changed paths are summarized in [§2.2](#22-changed-paths). `git diff --name-status d98cac2 <final revision>` gives the exact list, including files that were untracked during the work and committed by the checkpoints.

---

## 8. Source conflicts and gaps in the record

These are places where the sources disagree, are unclear, or do not support a claim. They are listed so they are not smoothed over.

1. **buildhygiene closed after this record was drafted.** adv buildhygiene r2 reviewed rounds 2 and 3 together: R1–R8 closed, and one should-fix (`docs/DEPENDENCIES.md:3` described remeasured figures as the post-reduction snapshot). The owner applied the reviewer's exact three-column table and replacement sentence; the reviewer said that docs-only fix needed no further Rust review. This record's review covers that edit.
2. **Generated outputs and late owner edits lack independent review.** `docs/ERRORS.md` was regenerated before the owner review but is not in its reviewed paths; `localization.mjs generate` ran after owner review round 2, whose report says "`generate` output (deliberately not run)". The LEDGER's integration line records the results of these steps, but no reviewer checked the outputs. `AGENTS.md` requires review of generated resources. Their generators' checks pass on the final tree (§5.4), and the independent review of this record covers the generated outputs.
3. **OWNER-R4.** `docs/RELIABILITY.md` says this record lists the final-tree verification; §5.4 now does.
4. **Integration-stage results.** The integration results quoted in earlier drafts predate the last documentation edits; §5.4 replaces them with runs on the final Rust tree. The Node and Python runs predate only documentation edits (buildhygiene round 3 and the `docs/DEPENDENCIES.md` table), which no test reads except `test-build-hygiene.mjs`, re-run after them (5/5).
5. **Swift verification is wider than "syntax only".** The brief for this record says Swift was syntax-only apart from `CrashRedaction.swift` on the Linux toolchain. The apple lane and its reviewer also compiled and ran harnesses on the Linux Swift toolchain built from generated extracts of `DocumentStorage.swift` and the widget code with corelibs shims (`ALL PASSED`, 38 checks). That is not a compile of the shipped files against Apple frameworks, so the statement in `docs/RELIABILITY.md` ("the other Swift changes were only syntax-checked") is accurate for the shipped files.
6. **Kept crash-report URLs keep a non-default port.** The owner's decision was "`scheme://host[:port]` only"; spec v2.3 keeps the lowercased scheme and host plus a non-default port, and only for `http`/`https` public DNS hosts. A summary that says "scheme and host only" omits the port.
7. **ACQ-6 disposition.** The implementer and the LEDGER call it partial (re-resolution only when nothing is kept); the round-1 reviewer wrote "ACQ-6: fixes it" for that case. This record uses partial, because the reported trigger with kept bytes stays pinned by design.
8. **LIB-6 frontend button.** The files reviewer's round 3 lists "the button for `edition_discard_original` is still missing" as a handoff; the LEDGER says the button already exists (`Downloads.tsx:1048`, panes lane) and the handoff is stale. The tree has the call at `src/components/Downloads.tsx:1048`, and adv panes r1 reviewed the edition-originals UI.
9. **APPLE-2 revive call.** Desktop round 3 says APPLE-2 is "not closed until" `sandbox::revive()` is called from `library_maintenance`; the LEDGER records it as verified present and the tree has the call in `src-tauri/src/service/dispatch.rs`. The adv service r1 report reviewed the handoff ("fine, `revive` is rate-limited"). The macOS code was never compiled.
10. **Transfers clippy.** The transfers reviewer closed round 4 with no exit-0 host clippy run containing that round's change; later lanes recorded exit-0 host clippy runs (adv net r3 at `618b809`, service rounds 3–4), which postdate it by ledger order but are not tied to that change in any report.
11. **APPLE identifier-less reopen.** adv apple r2 says this "should become its own finding" and the LEDGER says "→ document"; no document in the tree was found to record it. It is recorded in [§6.1](#61-apple-ios-and-macos).
12. **`dispatch.rs` Windows recovery comment.** Corrected in service round 3 (its report lists "DESKTOP-R8 comments … `dispatch.rs` Windows recovery comment"); the comment above `#[cfg(windows)] let moving` now says a same-volume move pinned by a launch-time open is cancelled, not retried.
13. **ANDROID-R11 local command.** Documented here, in §5.4 (the Kotlin row): the plugin's JVM tests, including the 238-vector parity check against Rust, run only locally. Gradle needs a `tauri.settings.gradle`, which is generated and git-ignored; no workflow runs these tests.
