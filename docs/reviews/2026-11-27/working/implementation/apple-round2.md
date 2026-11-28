# Lane `apple` — update after adversarial review (changes since the first report only; diff base `d98cac2`)

All six owner decisions are implemented, including the optional R3. **Every Swift change is syntax-checked only; not type-checked, not run** against an Apple SDK (`swiftc -parse` over all `ios/Sources`, `ios/Tests`, `gen/apple/DownloadActivity`: clean). I did not run `cargo` (lane rule), so the two Rust lines for R3 are **not compiled by me**.

"Linux harness" below means what it meant before: the real text of the changed Swift compiled against swift-corelibs-foundation with stand-ins for the Apple-only parts. It shows logic, not Darwin behaviour.

### APPLE-R1 — Restart fallback only on built-in storage
Disposition: fixed (reviewer's fix adopted)

Verification of the finding: confirmed by reading `acquire`. On a remounted drive `sameNode` could admit a file rewritten elsewhere, where base always refused.

Change, `DocumentStorage.swift`:
- `ExternalDocuments.init` gains `volumeIdentity: (URL) -> (String?, Bool?)`. It is injected the same way as the existing `resolveBookmark` (same shape: stored `private let`, defaulted `@escaping` closure parameter).
- The default reads `.volumeUUIDStringKey` and `.volumeIsInternalKey` in one `resourceValues` call. `volumeIsInternalKey` / `volumeIsInternal: Bool?` is a second new framework API (iOS 5+, from memory); it type-checks on corelibs.
- `identity()` keeps the UUID only when `volumeIsInternal == true`. False and nil both give `volume = nil`, so:
  - a part linked on a removable, network or unidentified volume records no restart proof;
  - a part on such a volume never matches a recorded one.
- No schema change beyond the first round's optional `volume`. Refusals keep `provider_archive_parts_changed` and the existing "Choose Locate" message.

Tests (XCTest, cannot run here):
- `testChangedResourceIdentifierIsRefusedOnARemovableOrUnidentifiedVolume` — same UUID, inode and mtime, with the flag false and nil: refused; with true: admitted.
- `testArchiveLinkedOnARemovableVolumeKeepsRequiringReselectionAfterItsIdentifierChanges`.

Linux harness:
- Both test bodies pass verbatim.
- New scenario `removableDriveChangedWhileAwayIsRefused` is the reviewer's S1 (rewrite keeping size, UUID and mtime; `stale` false and true; flag false and nil): refused. The untouched file on a remounted drive is also refused, as at base.
- New scenario `proofRecordedOnBuiltInStorageIsRefusedOnARemovableVolume` passes.
- Mutations "any volume with a UUID" and "unknown counts as built in" are both caught.
- Observed failing without the fix: yes, by mutation in the harness.

Not exercised: whether iOS reports `volumeIsInternal == true` for the data volume, iCloud Drive and File Provider locations. If it reports nil or false there, APPLE-1 is fixed nowhere; it fails closed.

### APPLE-R2 — Provider that stops exposing an identifier
Disposition: fixed

Change: `sameNode` now also requires `observed.resource != nil`. Such a provider is refused as at base, and a re-link records a digest.

Tests: no XCTest (there is no seam to make a real provider drop its identifier). Harness scenario `providerThatStopsExposingAnIdentifierIsRefusedAsBefore`: refused for `stale` false and true, record unchanged, admitted again once the identifier returns. It passes on base too, so this case is at parity with base. Removing the clause is caught.

Note: the `observed.resource != nil` in the rebinding condition is now implied by the guard. I kept it as a stated invariant; removing it is no longer observable.

### Owner decision 3 — `digestSatisfied` ignored a differing recomputed digest
Disposition: fixed

Verification:
- The digest is SHA-256 over the whole part: `identity()` reads to EOF and requires `offset == st_size`.
- It is recomputed exactly when `stale && expected.digest != nil`, and at first admission from a provider with no identifier.
- So a recomputed digest that differs means different bytes. The read is already paid for, and the adjacent comment says device/inode/mtime "are not content proof".
- The `sameFile ||` short-circuit and the always-rehash condition arrived in the same commit (`35e220b`). I found no reason for the short-circuit.

Change: the return is now `expected?.digest == nil || digest == expected?.digest || (digest == nil && sameFile)`. A recomputed digest decides. When none was recomputed (non-stale, which `acquire` does not consult) the value is as before. The doc comment that described the old intent is corrected.

Tests: XCTest `testStaleResolutionRefusesRewrittenContentEvenWithItsModificationTimeRestored`. It gives the saved record the no-identifier shape with a real SHA-256, proves the unchanged file still opens on a stale resolution, then rewrites in place with the same inode, size, UUID and mtime and expects refusal. `import CryptoKit` added to the test file; `SHA256()`, `update(bufferPointer:)`, `finalize()` are the calls `DocumentStorage.swift` already makes.

Linux harness: the test body passes on current and **fails on base** (observed). Scenario `staleResolutionRefusesDifferentDigestDespiteMatchingMetadata` is the reviewer's S2d: refused on current, admitted on base; a moved unchanged part still resolves. Restoring the old expression is caught.

### APPLE-R5 — Read-aloud stale window
Disposition: fixed

Change: `ReadAloudLiveActivity.swift` `show` — `now.addingTimeInterval(600)`. The download activity stays at 120 s.

Tests: none; it is a constant. Not exercised on a device.

### APPLE-R6 — Privacy test minimum set
Disposition: fixed

Change: `scripts/test-privacy-manifest.mjs` — new test "the categories that unscanned code relies on stay declared", asserting FileTimestamp, DiskSpace, SystemBootTime and UserDefaults.

Tests: `tests 5, pass 5, fail 0`. Dropping DiskSpace on a scratch copy now fails (observed); it passed before.

### APPLE-R3 — Stale wording for the download activity
Disposition: fixed on the Swift side; Rust producer added but not compiled by me. APPLE-4 is no longer partial once the owner's host test run confirms the Rust lines.

`cfg` check: `live_activity.rs` is `#[cfg(any(target_os = "ios", test))]`. `snapshot` has no further gate, so the host `--tests` build compiles it and its unit tests. `android-clippy.sh` runs `--lib` without `--tests` and does **not** compile it.

Change:
- `live_activity.rs::snapshot` — `"staleLabel": locale.text("Open Kiwix to see progress", &[])`, the same call shape as `statusLabel` on the line above. One added assertion in `starts_updates_throttles_and_ends_one_activity`.
- `DownloadActivityAttributes.ContentState` — optional `var staleLabel: String?`. Earlier content still decodes; the existing five-argument initializer call in `finish` still compiles.
- `LiveActivityArgs.staleLabel` is passed into the content in `show`.
- `DownloadActivityStyle.presented` — `statusLabel = state.staleLabel ?? ""`. With no label it shows nothing rather than English. The wording also becomes the VoiceOver name of the symbol.

Tests:
- XCTest `testDownloadRequestCarriesTheStaleWordingAndEarlierContentStillDecodes` (cannot run here). Its body passes verbatim in the widget harness, which also checks the mapping with and without a label: `ALL PASSED`.
- Rust: not run. For the owner: `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib live_activity`.

Not exercised: the Rust build; the widget on a device.

## Commands run
- `swiftc -parse` on all three Swift directories → clean.
- Storage harness (same command as before) → `ALL PASSED`, 38 ok, stable over repeated runs.
- Storage harness on base source → the positive restart tests and the digest test fail; the removable-drive and vanished-identifier scenarios pass on both. The two new XCTests that use `volumeIdentity:` do not compile against base and were left out of that run.
- Storage harness with the real `resourceValues` line → compiles; Linux reports `nil nil`, so the fallback never engages.
- Mutations: four new and five of the six earlier ones caught. The earlier "rebind without identifier" mutation is no longer observable (see R2 note).
- Widget harness → `ALL PASSED`.
- `node --test scripts/test-privacy-manifest.mjs` → `tests 5, pass 5, fail 0`.
- `node --test scripts/test-*.mjs` → `tests 497, pass 484, fail 2, skipped 11`. The two failures are `docs/ERRORS.md` and the localization inventory, both generated files; neither failure output names my files.
- Plist check → ok. `check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented.`

## Files changed since the first report
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/DocumentStorage.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/ReadAloudLiveActivity.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/DownloadActivityAttributes.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/LiveActivity.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/ExternalDocumentsTests.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/LiveActivityReconciliationTests.swift`
- `/home/user/kiwix-tauri/src-tauri/gen/apple/DownloadActivity/DownloadActivityWidget.swift`
- `/home/user/kiwix-tauri/scripts/test-privacy-manifest.mjs`
- `/home/user/kiwix-tauri/src-tauri/src/live_activity.rs` (shared; 4 added lines)

Harness scratch is unchanged in location: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/harness/` and `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/widget/`.

## New user-visible string
- `Open Kiwix to see progress` — Rust, `live_activity.rs`, through `locale.text`. Shown on the Lock Screen and spoken by VoiceOver when the download activity is stale.

## Handoffs / contract changes
- The new string is not in the catalogs. Rust-only strings enter through `src/locales/extra-en-*.json`, which I may not edit. Until it is added and the catalogs are regenerated, `locale.text` returns the English source in every locale.
- Related gap, not changed: `Reading aloud`, `Fetching metadata`, `Resolving`, `Connecting` and `Checking` are also absent from `src/locales/manifest.json`. Those status labels, including the read-aloud one APPLE-6 relies on, are English in every locale today.
- The download Live Activity request gains `staleLabel`; `ContentState` gains optional `staleLabel`.
- `ExternalDocuments.init` gains a defaulted `volumeIdentity` parameter. No call site changes.
- `docs/ERRORS.md` records Rust line numbers. My insertions in `live_activity.rs` and `mobile_speech.rs` shift lines, so it needs regenerating.

## Doubts
1. R1 rests on `volumeIsInternal` being true for iOS built-in storage. In the Simulator the positive XCTests also need the Mac's test volume to be internal; they say so if it is not.
2. With R1, the original APPLE-1 trigger on external drives is deliberately not fixed. A replugged drive still needs Locate.
3. Accepted limit, shown by harness scenario `builtInStorageAdmitsTheSameFileObjectAsWithinOneBoot`: on built-in storage, a file rewritten in place with its mtime restored is admitted after a restart, exactly as it already is within one boot.
4. The digest fix changes behaviour for providers with no identifier: a stale resolution of rewritten content is now refused. A non-stale reopen there still checks only size and the first part's UUID (unchanged).
5. The stale wording is my choice. It names the action, not the cause; the transfer may still be running in the background.
6. A read-aloud session whose app died now keeps its live look for up to 10 minutes instead of 2.
7. The Rust lines are trivially shaped but unbuilt by me. If the host build fails on them, the fault is mine and is confined to `snapshot` and its one assertion.