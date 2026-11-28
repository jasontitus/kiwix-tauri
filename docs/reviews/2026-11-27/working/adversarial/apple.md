## Verdict for lane apple: BLOCKING findings exist

One blocking item (APPLE-R1), and it is an owner decision rather than a coding error: the APPLE-1 fallback is sound on internal storage but loses a guarantee on removable media. Everything else is non-blocking. I found no compile error by reading, but nothing here has been built against an Apple SDK.

### APPLE-R1 — Restart fallback silently re-admits a linked archive that was modified while its drive was away
Severity: Blocking (weakens a check in a case the old code refused; the owner may instead accept it in writing)
Fix under review: APPLE-1
Evidence class: reproduced in the Linux harness for the in-place case; inference for iOS FAT/exFAT file-number behaviour
Location: `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/DocumentStorage.swift:1061-1068` (`acquire`), `:751` (`identity`)
Trigger: Link an archive in place on a removable drive. Unplug it; elsewhere, rewrite the file keeping its size and first-part ZIM UUID and put the mtime back. Replug and open.
Causal path:
- The identifier differs after the remount, so `sameResource` is false.
- `sameNode = durable && expected.volume == observed.volume && expected.inode == observed.inode && expected.modified == observed.modified` is true, so `guard expected.size == observed.size, sameResource || sameNode, …` passes.
- No digest exists for identifier-bearing records (`if (expected == nil && resource == nil) || (stale && expected?.digest != nil)`), so content is never checked.
- The record is then rebound to the new identifier.
- Base `d98cac2` refuses every reopen after a restart, so this is a case where old refused and new admits. My harness scenario S1 printed `admitted` for both `stale=false` and `stale=true`.

What holds and what does not:
- **Holds:** the implementer's "same kind of evidence" argument, wherever a file number is a persistent object identity and nobody can alter the medium outside this device (internal APFS, iCloud and File Provider files materialised on the data volume). There a different file has a different inode, and an on-device writer could already pass the old check without waiting for a restart.
- **Does not hold:** removable media. Any offline change necessarily involves a remount, so the old code always sent the user to Locate. On FAT/exFAT, file numbers are synthesised (my recollection of Apple's msdosfs, not observed on iOS), so a replaced file, not only a rewritten one, may match.
- **Coupled unknown:** on those volumes the fix either never engages (nil UUID or unstable inode, so APPLE-1 is not fixed there) or engages with this weakening.

Remaining checks on the fallback path are unchanged: size, first-part magic and UUID, part count, digest-when-recorded on a stale resolution, no-follow descriptor, security scope. `inode`, `modified` and `size` come from `fstat` on the descriptor later handed to Rust; `resource` and `volume` come from the URL (see R4).

Smallest correct fix: fetch `.volumeIsInternalKey` in the same `resourceValues` call as the UUID and require `volumeIsInternal == true` in `sameNode`, failing closed when nil. Removable and network volumes then keep today's "Choose Locate" message. Add a rejection test. If the owner wants replug to work for external drives, that needs device evidence and explicit acceptance, or a content digest recorded at link time for such volumes and re-checked once per identifier change.

### APPLE-R2 — A provider that stops exposing an identifier is admitted indefinitely without content binding (implementer doubt 5)
Severity: Should-fix
Fix under review: APPLE-1
Evidence class: reproduced in harness (`providerThatStopsExposingAnIdentifierIsNotDowngraded` is `admitted` on current, `provider_archive_parts_changed` on base)
Location: `DocumentStorage.swift:1063-1064`, `:1074`
Trigger: A record has `resource != nil`; the provider later returns no `fileResourceIdentifier`.
Causal path: `sameNode` admits, but rebinding requires `observed.resource != nil`, so the record never heals and every later open uses the fallback. A fresh link under the same provider would have recorded a digest (`expected == nil && resource == nil`).
Smallest correct fix: add `observed.resource != nil` to `sameNode`. That restores the old refusal, and the re-link then records a digest.

### APPLE-R3 — Stale download activity gives no explanation, and still draws frozen progress
Severity: Should-fix (residual the implementer acknowledged)
Fix under review: APPLE-4
Evidence class: source-traced
Location: `/home/user/kiwix-tauri/src-tauri/gen/apple/DownloadActivity/DownloadActivityWidget.swift:118-130`, `:94-99`, `:133-135`
Trigger: Start a download, lock the phone, wait more than 120 s.
Causal path: `presented` sets `statusLabel = ""`, so the Lock Screen row is `Text("")` on the left and the last bytes on the right. `bar` still draws the last fraction. `spoken` falls back to the file name, so VoiceOver hears nothing about state.
- Not a blank row, and the false "Downloading N %" is gone.
- Paused and terminal states pass through untouched.
- The only stale signal is a gray outline icon, which nothing announces.
Smallest correct fix: the planned optional `staleLabel` on `ContentState` from `live_activity.rs::snapshot`. Until then, treat APPLE-4 as partial.

### APPLE-R4 — Volume identity is read from the path while inode and mtime come from the descriptor
Severity: Nit
Fix under review: APPLE-1
Evidence class: source-traced
Location: `DocumentStorage.swift:751` against `:744`, `:753-754`
Trigger: A party controlling the location swaps the path between `open` and `resourceValues`.
Causal path: The same gap already existed for `fileResourceIdentifier` at `:745` and was easier to use there, so this is not weaker than before.
Smallest correct fix: none required now. If hardened later, derive the volume from the descriptor and confirm the API on a device.

### APPLE-R5 — Read-aloud can look stale while still reading at very slow rates (implementer doubt 8)
Severity: Nit
Fix under review: APPLE-4
Evidence class: inference from source
Location: `ReadAloudLiveActivity.swift:104`; `/home/user/kiwix-tauri/src-tauri/src/kokoro.rs` `chunks` (700-byte cap)
Trigger: Rate near 0.25×, one unpunctuated chunk longer than about 120 s of audio.
Causal path: Progress events arrive per chunk or sentence; `staleDate = now + 120`. The widget then shows gray with no percentage until the next event. Controls still work.
Smallest correct fix: lengthen the read-aloud stale window (for example 10 minutes) or re-send on a timer.

### APPLE-R6 — Privacy test would not notice a dropped DiskSpace entry
Severity: Nit
Fix under review: APPLE-5
Evidence class: reproduced (removing the DiskSpace entry on a scratch copy gave 4 pass)
Location: `/home/user/kiwix-tauri/scripts/test-privacy-manifest.mjs:178-189`
Trigger: Someone removes the DiskSpace entry that only Rust or libzim code needs.
Causal path: Only Swift and `.mm` sources are scanned; `PdfExportBridge.m` and `gen/apple/Sources` are not (I grepped them by hand: no required-reason use today).
Smallest correct fix: assert a fixed minimum set of categories as well.

**Per-fix assessment**
- **APPLE-1:** fixes it for same-volume restarts where the UUID and inode are stable, but see R1 and R2. Its effect on a replugged FAT/exFAT drive is unobserved. Healing is all-or-nothing: it happens after every part, the UUID and the count validate, inside the existing `locked` block with the revocation recheck. It never drops a digest and never writes a record weaker than it read. Older builds ignore `volume`.
- **APPLE-4:** fixes the false live claim, but only partly (R3, R5). Paused is unaffected.
- **APPLE-5:** fixes it. `CA92.1` is the correct reason. The test fails on the base manifest and when UserDefaults, FileTimestamp or SystemBootTime is removed; comments are ignored; CI's `node --test scripts/test-*.mjs` picks it up.
- **APPLE-6:** fixes the control names and the unlabeled glyphs, but "% read" stays English. The JSON shape matches:
  - Rust sends the label object or `null`; every `NativeSpeechLabels` field is optional; the extra `paused` key is ignored.
  - Updates omit `labels`.
  - Attributes from an earlier build still decode.
- **APPLE-8:** fixes it. The cleanup can only unlink names ending in `.bookmark` relative to the directory descriptor it opened with `O_NOFOLLOW`. Sub-directories and a symlinked directory survive, and `rmdir` removes only an empty directory. It runs on `providerWorker`, and after the first launch costs one failed `open`.

**Implementer's doubts**
1. No SDK type-check: real, see the unverified list below.
2. Device behaviour of UUID and inode: real; coupled to R1.
3. Legacy records heal only on a same-boot reopen: acceptable; the Locate message is already actionable. Archives not opened before the next restart or replug need Locate once.
4. Cloned volume: acceptable.
5. Identifier disappears: fix now (R2).
6. Rebinding write in read-only or private mode: owner decision. Same class as the existing stale-bookmark refresh; the record reveals only that the archive was opened since the last identifier change.
7. Existing weaknesses left alone: correct to leave in this lane, but one should become its own finding. `digestSatisfied` returns `expected?.digest == nil || sameFile || digest == expected?.digest`, so a recomputed digest that differs is ignored when device, inode and mtime match. My scenario S2d reproduced it (`admitted`); it is present at base line 771.
8. Read-aloud stale at slow rates: nit (R5).
9. Cleanup at every load: acceptable.
10. `isStale` availability from memory: I also recall iOS 16.2, and both targets deploy to 16.4. Unverified here.

**What I ran**
- `swiftc -parse` over all of `ios/Sources`, `ios/Tests` and `gen/apple/DownloadActivity`: exit 0.
- `node --test scripts/test-privacy-manifest.mjs`: `tests 4, pass 4, fail 0`, plus the scratch mutations above.
- `android-clippy.sh` (`-D warnings`, compiles `mobile_speech.rs`): `Finished`, exit 0.
- Implementer's storage harness, rebuilt in my scratch directory:
  - current source: `ALL PASSED`, 31 ok;
  - base source: the positive tests fail with `provider_archive_parts_changed`.
- The real `volumeUUIDString` line type-checks on corelibs. The widget style harness: `ALL PASSED`.
- Harness fidelity: the control flow is the real text, but the identifier, volume UUID, bookmarks, coordination and SHA-256 are stand-ins. A pass shows logic, not Darwin behaviour.
- Scratch: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-apple/`

**What I could not verify (look at these first on the Mac)**
1. Widget target build:
   - `context.isStale` at `DownloadActivityWidget.swift:22,26` and `ReadAloudActivityWidget.swift:15,24`;
   - `.accessibilityLabel(String)` on the compact and minimal Dynamic Island regions and on the `Link`s.
2. Plugin target build:
   - `DocumentStorage.swift:751`;
   - the bare `unlinkat` / `rmdir` statements at `:592-594`;
   - the seven-argument `ReadAloudActivityAttributes` call at `ReadAloudLiveActivity.swift:138-146`;
   - `KiwixMobilePlugin.swift:278`.
3. `xcodebuild test`: the seven new `ExternalDocumentsTests` and the new `LiveActivityReconciliationTests` case. I read them against the existing fixture helpers and found no compile problem.
4. Device: record `fileResourceIdentifier`, `volumeUUIDString`, `st_ino`, mtime and (for R1) `volumeIsInternal` before and after a reboot and a replug, for internal storage, iCloud Drive, exFAT, FAT32, external APFS and SMB.
5. Device: whether the system redraws at the stale date, and what VoiceOver says on the island.
6. Whether Rust's Locate flow compares anything beyond what Swift `select` does; I traced only the Swift side.