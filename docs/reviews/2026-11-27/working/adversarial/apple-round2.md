## Verdict for lane apple (round 2): only non-blocking findings

R1, R2 and the digest bypass are closed in code and I found no new defect in the round-2 changes. Two things still stand between this and calling APPLE-1 and APPLE-4 fully done: the built-in-storage proof rests on an iOS value nobody has observed, and the new stale wording is not in the translation catalogs. No Swift has been built against an Apple SDK.

### APPLE-R7 — The restart proof now depends on `volumeIsInternal == true`, which is unobserved on an iPhone
Severity: Should-fix (verification gate before recording APPLE-1 as fixed)
Fix under review: APPLE-1 / R1
Evidence class: inference
Location: `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/DocumentStorage.swift:678-681` (default `volumeIdentity`), `:761-762` (`identity`)
Trigger: Link an archive on built-in storage or iCloud Drive, restart, open.
Causal path: `let volume = volumeIsInternal == true ? volumeUUID : nil`. Apple documents the key as nil when it cannot be determined. If iOS returns nil or false for the data volume, no record ever gets a `volume`, `durable` is always false, and behaviour is exactly base: refused, "Choose Locate". That is fail-closed, but APPLE-1 would then be fixed nowhere. The Simulator cannot settle this, because it reads the Mac's volume.
Smallest correct fix: none in code yet. On the test iPhone, log `volumeUUIDString`, `volumeIsInternal`, `st_ino` and `fileResourceIdentifier` for a file in On My iPhone, iCloud Drive and a third-party provider, before and after a restart. Until then, record APPLE-1 as "mitigation, unverified on device". If the value is nil, a descriptor-based signal is the next option.

### APPLE-R8 — "Open Kiwix to see progress" is English in every locale
Severity: Should-fix (handoff the implementer already flagged)
Fix under review: APPLE-4 / R3
Evidence class: source-traced
Location: `/home/user/kiwix-tauri/src-tauri/src/live_activity.rs:266-268`; absent from `src/locales/manifest.json`, `extra-en-shell.json` and `catalogs.json` (grep count 0 in each)
Trigger: A non-English user's download activity goes stale.
Causal path: `locale.text` falls back to the source string when it has no catalog entry. The row that was blank in round 1 now shows English text beside a localized file row, and VoiceOver speaks it.
Smallest correct fix: the integration owner adds the string to an `extra-en-*.json` file and regenerates the catalogs. Do not describe the stale wording as localized before that.

### APPLE-R9 — Expanded Dynamic Island still shows no stale wording
Severity: Nit
Fix under review: APPLE-4 / R3
Evidence class: source-traced
Location: `/home/user/kiwix-tauri/src-tauri/gen/apple/DownloadActivity/DownloadActivityWidget.swift:40-56`
Trigger: Long-press the island while the activity is stale.
Causal path: The expanded bottom region never rendered `statusLabel`, so it shows the gray icon, bar, file name and bytes only. The Lock Screen row and the compact and minimal VoiceOver labels do carry the wording.
Smallest correct fix: optional; add `Text(state.statusLabel)` to the bottom region when stale.

### APPLE-R10 — Positive restart XCTests depend on the host Mac's volume reporting internal
Severity: Nit
Fix under review: APPLE-1 tests
Evidence class: source-traced
Location: `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/ExternalDocumentsTests.swift` (`testLinkedOriginalReopensAfterItsResourceIdentifierChangesAcrossRestart`, `testRecordWithoutRestartProofIdentityRequiresReselectionUntilItIsCompleted`, and the `XCTUnwrap` at line 624)
Trigger: Run the suite with the temp directory on an external or virtual disk.
Causal path: Those tests use the default `volumeIdentity`. They fail with a clear message rather than silently, so this is only a portability point.
Smallest correct fix: inject `volumeIdentity: { _ in (uuid, true) }` in all but one of them.

**Per-fix lines for earlier findings**
- **R1:** closed. A removable, network or unknown volume records no `volume` and observes none, so `sameNode` is false. Harness: untouched and rewritten files after a restart are refused for `false` and `nil`, and a record made on removable storage stays refused even if the volume later reads as internal. The built-in restart case still passes in the harness. On a device it depends on R7.
- **R2:** closed. `sameNode` requires `observed.resource != nil`; my scenario S2 now returns `provider_archive_parts_changed` and leaves the record unchanged.
- **R3:** closed in code for the Lock Screen and VoiceOver. Older content without `staleLabel` decodes, and the five-argument `ContentState` call in `finish` still compiles by reading. See R8 and R9.
- **R4:** not closed (nit, unchanged). The UUID and the internal flag are both read by path; this is the same gap the identifier always had.
- **R5:** closed. The read-aloud stale window is 600 s; the download window stays 120 s.
- **R6:** closed. Removing DiskSpace on a scratch copy now fails test 3.
- **Doubt 7, digest bypass:** closed. My S2d is now refused. It cannot refuse a file whose bytes read the same:
  - a short read is looped to EOF;
  - a length mismatch already threw before this change;
  - iCloud files carry an identifier, so they have no digest and never take this path;
  - the read runs inside a coordinated read.
  The only newly refused case is same device, inode, mtime and size with different bytes read.
- **Doubt 7, non-stale reopen:** still open and outside this lane. A provider with no identifier is checked only on size and first-part UUID when the bookmark is not stale (my S2e: `admitted`). It should become its own finding.

**Accepted limits, restated so they are not lost**
- On built-in storage, a file rewritten in place with its mtime restored is admitted after a restart (my S1: `admitted`), exactly as it already is within one boot.
- A replugged external drive still needs Locate, by owner decision, so the original APPLE-1 trigger "drive unplugged and re-plugged" is deliberately not fixed.

**Compile reading of the new code**
- The `volumeIdentity` default closure and `values?.volumeIsInternal` type-check on corelibs with the real line. `.volumeIsInternalKey` / `volumeIsInternal: Bool?` are from memory for Darwin.
- Every test call site passes `resolveBookmark:` and `volumeIdentity:` with labels, so the added parameter cannot rebind a trailing closure.
- `LiveActivityArgs.staleLabel` and `ContentState.staleLabel` are optional. Their synthesised decoding accepts payloads without the key.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib live_activity`: `8 passed; 0 failed`, including the new `staleLabel` assertion.
- `swiftc -parse` over `ios/Sources`, `ios/Tests` and `gen/apple/DownloadActivity`: exit 0.
- `node --test scripts/test-privacy-manifest.mjs`: `tests 5, pass 5, fail 0`.
- Storage harness, current source, with my scenarios added: `ALL PASSED`, 38 ok.
- Widget harness: `ALL PASSED`.
- Scratch: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-apple/`

**What I could not verify**
- Any Apple SDK build of either target or of the test target. The first places to look are unchanged from round 1, plus `DocumentStorage.swift:678-681` and the six-argument `ContentState` call at `LiveActivity.swift:111-118`.
- `volumeIsInternal` on a device (R7).
- Stale redraw and VoiceOver on a device.
- The iOS-only parts of `live_activity.rs`. `snapshot` compiled under the host test build; the Android type-check does not compile this file.
- `docs/ERRORS.md` line references for `live_activity.rs` and `mobile_speech.rs`. That test already fails from other lanes' edits and needs regenerating by the integration owner.