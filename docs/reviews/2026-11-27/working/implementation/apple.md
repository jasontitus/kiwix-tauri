# Lane `apple` — report (review base `d98cac2`)

All five findings are addressed in code; two have residuals (APPLE-4 stale wording, APPLE-6 "% read"). Nothing was built or run against an Apple SDK. **Every Swift change below is syntax-checked only; not type-checked, not run** (`swiftc -parse` over all `ios/Sources`, `ios/Tests` and `gen/apple/DownloadActivity` Swift: clean). No `cargo`, no `npm run build`.

Extra evidence, which is not a substitute for a Mac build: the Linux toolchain ships swift-corelibs-foundation, so I compiled and ran the real text of the changed code on Linux with stand-ins for the Apple-only parts. Where I cite "Linux harness" below, that is what it means.

---

### APPLE-5 — Privacy manifest omits the UserDefaults required-reason category
Disposition: fixed

Verification of the finding: `ExternalDocuments` reads and writes `kiwix.external-document-references.v1` through `UserDefaults` (`DocumentStorage.swift`, `init(defaults: UserDefaults = .standard`). The manifest declared only FileTimestamp, DiskSpace and SystemBootTime. The new test failed on the pre-fix manifest (observed):
`src-tauri/PrivacyInfo.xcprivacy must declare these required-reason API categories: NSPrivacyAccessedAPICategoryUserDefaults (first used at …/DocumentStorage.swift:653)`.

Change:
- `src-tauri/PrivacyInfo.xcprivacy` — added `NSPrivacyAccessedAPICategoryUserDefaults` with `CA92.1`. Apple's reason page (fetched) describes CA92.1 as "information that is only accessible to the app itself", which matches the use.
- New `scripts/test-privacy-manifest.mjs` (Node test runner, no dependencies), four tests:
  1. The detector recognizes each of Apple's five families on sample lines and ignores comments.
  2. Every family used in `ios/Sources/*.swift` and `src-tauri/native/*.mm` is declared.
  3. Every declared category has valid, non-duplicated reason codes. The two SDK-only codes (0A2A.1, C56D.1) are rejected for an app manifest. All 17 codes were checked against Apple's page.
  4. `project.yml` still bundles the manifest as an app resource, and the widget extension's sources use no required-reason API.
- The test parses the plist structurally, so key order and whitespace do not matter.

Tests: `node --test scripts/test-privacy-manifest.mjs` → `# tests 4 / # pass 4 / # fail 0`.
- Observed failing before the fix (above).
- Not a one-trick test: on scratch copies of the manifest, dropping FileTimestamp fails (first use `BackgroundAcquisition.swift:33`), dropping SystemBootTime fails (`NativeSpeech.swift:441`), dropping UserDefaults fails, a wrong reason code fails, a duplicated category fails.
- Dropping DiskSpace does not fail. No scanned Swift or `.mm` uses a disk-space API; that entry covers Rust or libzim code, which the test does not scan.
- Plist well-formed: `python3 -c "import plistlib…"` → ok.

Widget extension manifest (found, not guessed):
- `project.yml` gives `KiwixDownloadActivity` only `DownloadActivity/` plus the two `*ActivityAttributes.swift` files, and no privacy manifest.
- Apple's "Describing use of required reason API" page (fetched) says: "For each executable or dynamic library in an app that uses a required reason API, the bundle that includes the executable or dynamic library needs to include a privacy manifest file that reports the API."
- Those four Swift files use none of the five families (asserted by test 4). So the extension needs none today, and the test fails if that changes.

Not exercised: an App Store Connect upload. Third-party frameworks and Rust are not scanned.

---

### APPLE-8 — `persistFolderBookmark` writes a bookmark nothing reads
Disposition: fixed

Verification of the finding: a repo-wide search for `kiwix-provider-bookmarks|persistFolderBookmark` (excluding node_modules, target, dist, .git) hit only the definition, its one call, and a historical review note under `docs/reviews/`. Nothing reads it. The call sat after `committed = true`, so a throw reached `fail(current, error, retainedDirectory:)` for a fully copied import.

Change:
- `KiwixMobilePlugin.swift` `processSelection` — removed the call. Nothing after `committed = true` in that branch can throw now.
- `DocumentStorage.swift` — removed `persistFolderBookmark`; added `removeUnreadFolderBookmarks(in:)`. It opens `Application Support/kiwix-provider-bookmarks` with `O_DIRECTORY | O_NOFOLLOW`, unlinks only direct `*.bookmark` entries with `unlinkat(fd, name, 0)`, then `rmdir`s. A sub-directory, any other file, or a symlink in place of the directory is left alone. It is best-effort and never throws.
- `KiwixMobilePlugin.load(webview:)` — `providerWorker.async { removeUnreadFolderBookmarks() }`.
- Precedents for each symbol: `providerWorker.async` (same file); `open(…, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`, `contentsOfDirectory(at:includingPropertiesForKeys:options:)`, `unlinkat(…, 0)` (`DocumentStorage.swift`); `FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first` (`NearbyBluetooth.swift:117`); `rmdir(path)` as a statement (`MobilePolicy.swift:119`).

Tests: XCTest `testUnreadFolderBookmarkCleanupRemovesOnlyItsOwnFiles` (cannot run here). Its body passes verbatim in the Linux harness against the real function text. No test for the import path itself: `processSelection` is private picker flow and the throwing statement is gone.

Not exercised: on-device run; the plugin-load call site is not type-checked.

---

### APPLE-1 — Linked archives rejected when `fileResourceIdentifier` changes
Disposition: fixed differently. The reviewer's literal fix (accept on size + ZIM UUID) is rejected as a weakening.

Verification of the finding:
- Apple's page for `fileResourceIdentifierKey` (fetched): "The value of this identifier isn't persistent across system restarts."
- `acquire` threw `provider_archive_parts_changed` on any mismatch.
- A digest is only computed when `expected == nil && resource == nil`, so a record with an identifier never has a digest. Under the proposed fix, size plus the first part's UUID would be the only proof, and nothing but size for split parts 2+. In the Linux harness that mutation admits a same-size same-UUID replacement, an in-place rewrite and a replaced split part (12 tests fail).

What I did instead: the identifier is boot-scoped by contract, so the record now also keeps identity that is not.
- `PartIdentity` gains optional `volume: String?` from `URLResourceValues.volumeUUIDString`. Apple documents it as "The volume's persistent UUID as a string, or nil…", iOS 8+. This is the one new framework API in the lane; signature confirmed from Apple's page and it type-checks on corelibs.
- `inode` and `modified` were already recorded.

Evidence that admits a part, before and after. Unchanged in both: the reference resolves (in-session URL or bookmark), security scope, coordinated read, `O_NOFOLLOW` regular seekable descriptor, no-follow traversal for folders, equal size, digest satisfied on a stale resolution, then first-part ZIM magic + UUID and part count.
- **Before:** if the record has an identifier, the archived identifier bytes must be equal.
- **After:** identifier equal, **or** the record has volume UUID, inode and nanosecond mtime, and all three equal those of the file opened now (inode and mtime from `fstat` on the descriptor handed to Rust).

Why a different file at the same URL with the same size is still refused:
- On the same volume it has a different inode.
- On another volume the UUID differs.
- A record lacking any of the three never uses the fallback; a volume with no UUID never matches.
- On file systems with synthesized file numbers (FAT/exFAT), a same-slot replacement could carry the same number. It would equally carry the same in-boot identifier today, and it must now also match the mtime, which the old check never required.

This is file-object identity, not content proof — the same kind as the identifier it stands in for.

Healing:
- After all parts and the UUID validate, a part that proved itself is rebound to this boot's identifier and the record is persisted (`rebound` joins the existing `stale || legacy` write condition).
- A record from an earlier build gains the durable fields on its first successful same-boot reopen.
- Rebinding happens only while the provider still exposes an identifier, so a record can never degrade into a size-only one.
- The digest and the originally admitted mtime are never replaced.

Tests: seven new XCTests in `ios/Tests/ExternalDocumentsTests.swift` (cannot run here):
- `testLinkedOriginalReopensAfterItsResourceIdentifierChangesAcrossRestart`
- `testChangedResourceIdentifierCannotAdmitReplacementWithSameSizeAndArchiveUUID` (replacement is given the same mtime, so only the inode differs)
- `testChangedResourceIdentifierCannotAdmitSameSizeFileWithDifferentArchiveUUID`
- `testChangedResourceIdentifierCannotAdmitFileRewrittenInPlace`
- `testChangedResourceIdentifierCannotAdmitSameFileNumberOnAnotherVolume`
- `testRecordWithoutRestartProofIdentityRequiresReselectionUntilItIsCompleted`
- `testUnreadFolderBookmarkCleanupRemovesOnlyItsOwnFiles` (APPLE-8)

Linux harness results (real `ExternalDocuments` text, these test bodies verbatim, 14 existing tests ported, 10 extra scenarios):
- Current source: `ALL PASSED`, 31 ok, stable over repeated runs.
- Base source `d98cac2`: the two positive tests fail with `provider_archive_parts_changed` (observed); the rejection tests pass on both.
- Mutations of the new check, all caught: size + UUID only; drop volume; drop mtime; drop inode; rebind without an identifier; never persist the rebinding.
- With the real `volumeUUIDString` line (nil on this Linux volume), the fallback never engages.

Harness stand-ins:
- file coordination, security scope, bookmarks, CryptoKit SHA256;
- `st_mtimespec` → `st_mtim`;
- the identifier modelled as (boot, dev, ino);
- NSKeyedArchiver replaced by raw bytes, because corelibs' archiver is not byte-deterministic (4 of 200 archives of equal NSData differed in one probe).

Not exercised: Darwin; a real reboot or remount; whether iOS reports a volume UUID and stable inodes for exFAT/FAT drives or third-party providers. If it does not, the result is the old failure, not a wrong admission.

---

### APPLE-4 — Download Live Activity never renders as stale
Disposition: partial. Stale rendering is fixed with no new `ContentState` field; a localized "stale" wording is not added.

Verification of the finding: no `isStale` anywhere under `gen/apple/DownloadActivity`. `LiveActivity.swift` sets `staleDate = now + 120 s` for every non-paused status. Rust re-sends an active job every 2 s (`REFRESH` in `live_activity.rs`), so a live activity never expires. Apple's Live Activities article (fetched) reads `context.isStale` in both the `ActivityConfiguration` content closure and the Dynamic Island, exactly as done here. `isStale` is iOS 16.2+ and both targets deploy to 16.4, so no `#available` is needed; the download widget already uses 16.2-only `DownloadActivityAttributes` unguarded.

Change:
- `DownloadActivityWidget.swift` — `DownloadActivityStyle.presented(_:isStale:)` maps a stale, non-paused, non-terminal state to a widget-local status `"stale"` with an empty status label. That status shows:
  - no percentage and "…" in the compact region;
  - the outline symbol `arrow.down.circle`, gray tint, no spinner;
  - the bar at the last known fraction and the last known bytes;
  - no "Downloading" wording.
- `ReadAloudActivityWidget.swift` — the same applies (stale date set, never read), so the same mapping: gray tint, no status wording, no percentage or "% read"; paused sessions untouched.
- The synthetic status never crosses the Codable boundary. The decode contract with Rust is untouched.

Tests: no XCTest is possible; the widget extension has no test target and `project.yml` is not mine. In the Linux harness the real style functions type-check against stubs for `Color` and `ActivityAttributes`, and pass assertions for live, stale (nine Rust statuses), paused and terminal states: `ALL PASSED`.

Not exercised: SwiftUI view bodies; `context.isStale` on a device; the system re-rendering at the stale date.

Residual: there is no localized "stale" text, so the status row is simply empty. Plan: an optional `staleLabel: String?` on `DownloadActivityAttributes.ContentState`, sent by `live_activity.rs::snapshot` and `LiveActivityArgs`, shown where `presented` now blanks the label. That needs a new localized Rust string and catalog regeneration.

---

### APPLE-6 — Read-aloud Live Activity English labels and unlabeled glyphs
Disposition: partial. Control names are localized and compact regions are labeled; "% read" stays English.

Verification of the finding: the literals were as cited. Rust already builds a localized `labels` object for each session (`mobile_speech.rs:656`, keys `pause`/`resume`/`stop`/`previous`…) and sends it to `speechEngineStart`.

Change:
- `mobile_speech.rs`, start payload — one additive line, `"labels": labels,`. **Not compiled here** (`#[cfg(mobile)] mod transport`). `json!` borrows its values; the same block already reuses `title` and `name` after an earlier `json!`.
- `ReadAloudLiveActivity.swift` — `ReadAloudActivityArgs.labels: NativeSpeechLabels?` (existing compiled type); the four names are passed into the attributes.
- `ReadAloudActivityAttributes.swift` — four optional `var …Label: String?`. Activities started by an earlier build still decode, and the existing three-argument initializer call still compiles (checked in the harness).
- `ReadAloudActivityWidget.swift` — `controls` takes the attributes and uses `attributes.previousLabel ?? "Previous paragraph"` and so on, keeping the English fallbacks.
- Both widgets — compact leading and minimal symbols get `.accessibilityLabel` from the localized `statusLabel` (file name or article title when it is empty). The compact trailing glyph is spoken as the percentage, or the status when there is none.

Tests: XCTest `testReadAloudStartCarriesLocalizedControlNamesAndEarlierShapesStillDecode` in `LiveActivityReconciliationTests.swift` (cannot run here; its body passes verbatim in the Linux harness). No Rust test: the payload is built inline in mobile-only code.

Not exercised: VoiceOver on the Dynamic Island; the Rust line.

Residual:
- `"% read"` needs a new localized Rust string. Plan: add `progress` to the label object and an optional `progressLabel` attribute.
- `"Kokoro · "` is a product name and is left as is.
- The localized names are Rust's short forms ("Pause", "Resume", "Stop"), not "Pause reading".

---

## Commands run
- `swiftc -parse` on all `ios/Sources/*.swift`, `ios/Tests/*.swift`, `gen/apple/DownloadActivity/*.swift` → clean.
- `node --test scripts/test-privacy-manifest.mjs` → `tests 4, pass 4, fail 0`.
- `node --test scripts/test-*.mjs` → last run `tests 455, pass 447, fail 5, skipped 3`. The five failures are in other lanes' in-progress work: `test-error-docs`, `test-localization-inventory`, and three reader navigation tests. An earlier run failed five different frontend tests (`isUserCancelledError is not defined`, `displayedFrame is not defined`). None touch my files.
- `python3 -c "import plistlib…" src-tauri/PrivacyInfo.xcprivacy` → ok.
- `node scripts/check-unsafe-census.mjs` → `74 matches / 74 blocks / 2 impls in 19 files; every item documented.`
- Linux harness, storage: `cd …/scratchpad/apple/harness && python3 gen.py current harness cur $(cat old.txt) $(cat new.txt) && swiftc -swift-version 5 -o cur/run shims.swift extra.swift cur/subject.swift cur/tests.swift cur/main.swift && ./cur/run` → `ALL PASSED`. Use `base` in place of `current` for the pre-fix source.
- Linux harness, widgets: `cd …/scratchpad/apple/widget && python3 gen.py && swiftc -swift-version 5 -o run subject.swift main.swift && ./run` → `ALL PASSED`.

## Files changed
- `/home/user/kiwix-tauri/src-tauri/PrivacyInfo.xcprivacy`
- `/home/user/kiwix-tauri/scripts/test-privacy-manifest.mjs` (new)
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/DocumentStorage.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/KiwixMobilePlugin.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/ReadAloudActivityAttributes.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/ReadAloudLiveActivity.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/ExternalDocumentsTests.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/LiveActivityReconciliationTests.swift`
- `/home/user/kiwix-tauri/src-tauri/gen/apple/DownloadActivity/DownloadActivityWidget.swift`
- `/home/user/kiwix-tauri/src-tauri/gen/apple/DownloadActivity/ReadAloudActivityWidget.swift`
- `/home/user/kiwix-tauri/src-tauri/src/mobile_speech.rs` (shared file, 2 added lines)

New user-visible strings: none. No frontend `t(...)` key and no new Rust error text.

Scratch (for the reviewer): `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/harness/` (`gen.py`, `shims.swift`, `extra.swift`) and `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/widget/` (`gen.py`, `main.swift`).

## Handoffs / contract changes
- Read-aloud "start" payload now carries `labels` (the object already sent to `speechEngineStart`). The Swift side ignores unknown keys; Android does not consume this payload.
- `ReadAloudActivityAttributes` has four new optional fields. Both compiled copies come from the one shared file.
- The saved external-reference record (`UserDefaults`, JSON) gains optional `volume` per part. Older builds ignore it; older records decode.
- `package.json` has no `test:privacy-manifest` alias (not my file). CI runs `node --test scripts/test-*.mjs`, so the new test is picked up.
- `docs/RELIABILITY.md` lists "Physical reboot" as untested. That remains true; nothing here was observed on a device.

## Doubts
1. No Swift was type-checked against Apple SDKs. Not covered even by the harness: SwiftUI view bodies (`.accessibilityLabel(String)`, `context.isStale`), the `load(webview:)` call site, the `Activity.request` call, and Darwin's `st_mtimespec` path.
2. APPLE-1 depends on device behaviour I could not observe: a non-nil volume UUID and stable inodes on the providers that matter (exFAT/FAT especially). Failure is closed, not open.
3. Existing records only become restart-proof after one successful same-boot reopen on the new build. A user already locked out must use Locate once. I did not change the Rust message, which already says "Choose Locate in the library, or import a copy."
4. The restart proof is metadata. A volume cloned bit-for-bit with the same UUID is admitted.
5. Behaviour change: a provider that stops exposing an identifier is now admitted when volume, inode and mtime match; before, it was refused.
6. The rebinding adds one `UserDefaults` write per reference, including while Rust is in read-only mode. It is the same class of write as the existing stale-bookmark refresh.
7. Pre-existing behaviour I observed and left alone, for the reviewer:
   - For providers with no identifier, a non-stale reopen checks only size and the first part's UUID.
   - `digestSatisfied` accepts `sameFile` even when the re-hash differs, contradicting the adjacent comment.
   - Within one boot, an in-place rewrite keeping size and UUID is admitted.
8. Read-aloud progress updates arrive per sentence. One sentence longer than 120 s (very slow rate) would briefly show the stale look while audio plays. The stale look also leaves the pause/stop links in place.
9. The cleanup runs at every plugin load. It is one failed `open` after the first time.
10. I used WebFetch on Apple's public documentation to confirm the reason codes, the `volumeUUIDString` signature, the identifier's non-persistence, and the `context.isStale` usage. I could not fetch the `isStale` reference page itself (404), so its 16.2 availability is from memory plus Apple's Live Activities article.