## APPLE review — iOS and macOS platform layers (base `d98cac2`, read-only)

Nine findings, most severe first. Nothing was compiled or run on Apple hardware; every finding is traced through the current source, and the two that depend on OS behaviour I could not observe are marked.

### APPLE-1 — Linked iOS archives are rejected once the persisted `fileResourceIdentifier` changes
Severity: Medium        Category: reliability
Platforms: iOS / iPadOS
Evidence class: source-traced (code path proven; the change of value across a real reboot or remount is Apple's documented contract, not observed on a device)
Location: `src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/DocumentStorage.swift:734-737` (`ExternalDocuments.identity`), `:1037-1043` (`acquire`), `:823-826` (`publish`); message at `src-tauri/src/provider_reference.rs:85-87`
Trigger: The user links an archive in place ("keep original", persistent) from Files or an external drive. Later the device restarts, iOS is updated, or the drive is unplugged and re-plugged. They open the archive.
Causal path:
1. First admission: `identity()` archives `URLResourceValues.fileResourceIdentifier` with `NSKeyedArchiver` into `PartIdentity.resource`. `publish()` then writes the record to `UserDefaults` (`stored[reference] = try JSONEncoder().encode(finalRecord)`).
2. Apple documents this key as "not persistent across system restarts".
3. On a later launch, `acquire()` re-reads the identifier and runs `guard expected.size == checked.identity.size, expected.resource == nil || expected.resource == checked.identity.resource, … else { throw MobileFailure("provider_archive_parts_changed") }`.
4. That check runs regardless of `stale`, and no digest was recorded for providers that expose an identifier (`if (expected == nil && resource == nil) || …`), so there is no fallback.
5. Rust maps the code to the non-retryable "This archive could not be opened from its document provider. Choose Locate in the library, or import a copy." The user must re-locate every linked archive although nothing changed.
6. `docs/RELIABILITY.md:2028` lists "Physical reboot" as an untested gate; `ExternalDocumentsTests` only reopens within one boot.
Smallest correct fix: In `acquire`, treat a resource-identifier mismatch as "same file not proven", not as "parts changed". Accept when size and the first part's archive UUID match (the post-loop guard already checks the UUID) and, where a digest was recorded, the re-hash matches (force the hash on mismatch as for `stale`). Then store `checked.identity` instead of `expected` and force the `encoded` rewrite so the record heals. No Rust contract changes.
Regression test: `ios/Tests/ExternalDocumentsTests.swift` — select persistently, rewrite the stored record's `resource` bytes in the injected `UserDefaults`, construct a new `ExternalDocuments(defaults:)`, assert `acquire` succeeds and rewrites the record. Assert a same-size file with a different ZIM UUID still fails.

### APPLE-2 — macOS sandbox: a bookmark that fails at launch is never retried in that process
Severity: Medium        Category: reliability / usability
Platforms: macOS (sandboxed build, `src-tauri/entitlements.plist`)
Evidence class: source-traced
Location: `src-tauri/src/sandbox.rs:258-262` (`Registry::restore` → `dead.insert`), `:266-303` (`Registry::grant`), `:133-143` (`restore`, called once at `src-tauri/src/lib.rs:864-865`); message at `sandbox.rs` `explain_folder_error`
Trigger: The library holds an in-place archive or watched folder on an external or network volume. Kiwix is launched with the volume disconnected, then the user plugs it in.
Causal path:
1. `restore()` runs `native.activate()` once per saved bookmark. A failure is recorded as `dead` with the comment "Keep the bookmark (an unmounted volume may return later)".
2. `activate` is the only place that calls `startAccessingSecurityScopedResource`, and nothing calls it again after launch.
3. `grant()` handles a dead key only by `if !native.accessible(&root.path) { continue; }`. The mounted folder is not accessible without an active scope, so the valid stored bookmark is skipped on every save.
4. The archive stays unavailable, and the user sees "Kiwix does not have access to the folder … Add that folder to the library, or add the archive again…". Only a relaunch or re-picking the folder restores access.
Smallest correct fix: Add `Registry::revive(native)` that retries `native.activate(decode(bookmark))` for each key in `dead`. On success, remove it from `dead`, apply `refreshed` and set `dirty`. Call it at the start of `grant()` and from `folders_needing_access` and the library availability pass via `with_registry`, rate-limited to once every few seconds. No change to the file format.
Regression test: `sandbox.rs` tests — give `Fake` a switchable `broken` flag; restore with it set (key in `dead`), clear it, call `grant`/`revive`, and assert `activated` grew, `dead` is empty and `created` is unchanged.

### APPLE-3 — macOS menu: two items claim ⌘M, and the application menu has only Quit
Severity: Medium        Category: usability
Platforms: macOS
Evidence class: source-traced
Location: `src-tauri/src/menus.rs:47` (`("reader-toc", "Table of contents", "CmdOrCtrl+M")`), `src-tauri/native/tabbing.mm:176` (`tabItem(@"Minimize", @selector(performMiniaturize:), @"m", NSEventModifierFlagCommand)`), `menus.rs:124-125` (`application.append(&quit)` is the only app-menu item), `menus.rs:21` (Settings = `F12`), `:49` (Fullscreen = `F11`)
Trigger: Any Mac user presses ⌘M, ⌘H or ⌘, or looks for About or Settings in the "Kiwix" menu.
Causal path:
1. The main menu is `[Kiwix, File, Edit, Tabs]` plus the Window menu appended by `kiwix_tabbing_install_menu`.
2. Both "Table of contents" (Tabs) and "Minimize" (Window) carry key equivalent `m` + Command. AppKit resolves key equivalents in menu order, so ⌘M toggles the table of contents and the Minimize item shows a shortcut that never fires.
3. `macos_menu_keys::normalize` only rewrites Shift items, so it does not resolve this.
4. The application submenu contains only Quit. There is no Hide (⌘H), Hide Others, Show All, About or Services; Settings sits in File on F12, and Fullscreen is F11 (macOS's default "Show Desktop" key).
Smallest correct fix: Under `cfg(target_os = "macos")`:
- give `reader-toc` an accelerator that is not system-reserved;
- add `PredefinedMenuItem::{about, services, hide, hide_others, show_all}` and a Settings item with `CmdOrCtrl+,` to the application submenu, and use `Ctrl+Cmd+F` for fullscreen;
- this touches the label contract: `MenuLabels` is `deny_unknown_fields`, so new localized labels must be added to both `menus.rs` and `nativeMenuLabels` in `src/App.tsx`.
Regression test: `menus.rs` unit test asserting no macOS accelerator in `ACTIONS` is in the reserved set {⌘M, ⌘H, ⌘Q, ⌘,} and that accelerators are unique.

### APPLE-4 — Download Live Activity never looks stale; it shows a frozen "Downloading N %" while the app is suspended
Severity: Medium        Category: usability (misleading progress; comment contradicts code)
Platforms: iOS / iPadOS
Evidence class: source-traced
Location: `src-tauri/gen/apple/DownloadActivity/DownloadActivityWidget.swift:21` (`DownloadActivityLockScreenView(state: context.state)`) and the Dynamic Island closures; `ios/Sources/LiveActivity.swift:117-119`
Trigger: The user starts an official-catalog download and locks the phone or leaves the app.
Causal path:
1. The transfer continues in the OS background `URLSession`. Rust keeps the row at `downloading` (`shutdown_workers` skips `retain_native` rows, `src-tauri/src/acquisition/manager.rs:1274-1277`), and the suspended process can no longer call `liveActivity`.
2. `show()` sets `staleDate = now + 120 s` with the comment "anything else that stops being refreshed should visibly go stale rather than claim live progress".
3. ActivityKit only flips `activityState`/`context.isStale`; it does not change the rendering. Neither widget file reads `isStale` (no match in `gen/apple/DownloadActivity`).
4. The Lock Screen and Dynamic Island keep showing the last percentage with the "Downloading" label, possibly for hours and even after the background download has finished or failed, until the app is opened.
Smallest correct fix: Pass `context.isStale` into `DownloadActivityLockScreenView` and the island regions. When stale, drop the percentage, use an indeterminate or dimmed bar and a neutral symbol, and show a Rust-localized `staleLabel` (new optional field on `DownloadActivityAttributes.ContentState`, supplied by `live_activity.rs::snapshot`). The same applies to `ReadAloudActivityWidget`.
Regression test: Move the mapping into a pure `DownloadActivityStyle.presentation(state, isStale:)` and assert in XCTest that a stale `downloading` state yields no percentage. Add a `live_activity.rs` test that `snapshot` carries `staleLabel`.

### APPLE-5 — Privacy manifest omits the UserDefaults required-reason category
Severity: Medium        Category: maintainability / reliability (store submission)
Platforms: iOS / iPadOS
Evidence class: source-traced
Location: `src-tauri/PrivacyInfo.xcprivacy` (declares only FileTimestamp, DiskSpace, SystemBootTime); `ios/Sources/DocumentStorage.swift:653-657, 680, 795, 961, 1069, 1090` (`UserDefaults.standard`)
Trigger: Uploading the iOS build to App Store Connect or TestFlight, which `docs/UPDATE.md:166` names as the iOS update channel.
Causal path: `ExternalDocuments` reads and writes `kiwix.external-document-references.v1` in `UserDefaults.standard`. UserDefaults is a required-reason API, and the manifest bundled by `project.yml` has no `NSPrivacyAccessedAPICategoryUserDefaults` entry, so Apple reports ITMS-91053 "Missing API declaration".
Smallest correct fix: Add a dict with `NSPrivacyAccessedAPICategoryUserDefaults` and reason `CA92.1` to `PrivacyInfo.xcprivacy`.
Regression test: A new `scripts/test-privacy-manifest.mjs` that scans `ios/Sources/*.swift` and `native/*.mm` for required-reason APIs (`UserDefaults`, `systemUptime`, file timestamps, `statfs`/volume capacity) and asserts each category is in the plist.

### APPLE-6 — Read-aloud Live Activity has hard-coded English text and VoiceOver labels
Severity: Low        Category: usability (localization / accessibility)
Platforms: iOS / iPadOS
Evidence class: source-traced
Location: `src-tauri/gen/apple/DownloadActivity/ReadAloudActivityWidget.swift:132` (`"…% read"`), `:137` (`"Kokoro · "`), `:169`, `:173`, `:177` (`.accessibilityLabel("Previous paragraph" / "Resume reading" / "Pause reading" / "Stop reading")`); `DownloadActivityWidget.swift:130-131` and `ReadAloudActivityWidget.swift:125` (compact "❚❚" / "!" with no accessibility label)
Trigger: A user of any non-English app locale, or a VoiceOver user, starts Kokoro read-aloud and looks at the Lock Screen or Dynamic Island.
Causal path: `statusLabel` arrives already localized from Rust, but the control labels and "% read" are Swift literals. The extension target bundles no string catalog (`project.yml` gives `KiwixDownloadActivity` only its sources; `native-locales` goes to the app), so they stay English. The compact island glyphs are read literally by VoiceOver.
Smallest correct fix: Carry the localized strings through ActivityKit like `statusLabel`: add `previousLabel`, `pauseLabel`, `resumeLabel`, `stopLabel` and `progressLabel` to `ReadAloudActivityAttributes` (immutable per session). Fill them in `mobile_speech.rs` where the `"start"` payload is built (~line 734), decode them in `ReadAloudActivityArgs`, and add `.accessibilityLabel(state.statusLabel)` to the compact and minimal regions of both widgets.
Regression test: A Rust test on the read-aloud start payload asserting the label fields are present and localized for a non-English locale.

### APPLE-7 — iOS hardware-keyboard Escape is swallowed for non-dialog popovers outside the reader
Severity: Low        Category: usability (keyboard)
Platforms: iPadOS / iOS with a hardware keyboard
Evidence class: inference (relies on UIKit consuming the key, which `src/App.tsx:2849` itself states: "Native capture consumes Escape before DOM dispatch")
Location: `ios/Sources/HardwareShortcuts.swift:63` (`add(UIKeyCommand.inputEscape, [], action: "escape", …)`); `src/App.tsx:2846-2866` (the `kiwix-mobile-shortcut` listener)
Trigger: On an iPad with a keyboard, in Library, Downloads or Settings, open a `<details>` picker (`src/components/UI.tsx:880-885`), the library search suggestions (`src/components/Library.tsx:880`) or the mobile tab strip (`src/App.tsx:3232`), then press Escape.
Causal path: The key arrives only as action `"escape"`. The listener handles an open `<dialog>`; otherwise `isShellShortcut("escape")` is false and the remaining branch requires `pane === "reader"`, so the action is dropped. The DOM `keydown` handlers that close these popovers never run. In the reader, `HistoryDisclosure`'s document keydown handler (`Reader.tsx:665`) is bypassed the same way.
Smallest correct fix: In the listener, when `action === "escape"` and no dialog is open, first dispatch a cancelable `new KeyboardEvent("keydown", { key: "Escape", bubbles: true })` on `document.activeElement ?? document.body`. Forward to `mobileReaderShortcut` only if it was not `defaultPrevented`. Shell DOM only; never forward into archive frames.
Regression test: `scripts/test-frontend-surfaces.mjs` — with the mobile flag and a picker open in the Library pane, emit `kiwix-mobile-shortcut {action:"escape"}` and assert the picker closes.

### APPLE-8 — `persistFolderBookmark` writes a bookmark nothing reads, and its failure fails a completed folder import
Severity: Low        Category: maintainability / reliability
Platforms: iOS / iPadOS
Evidence class: source-traced
Location: `ios/Sources/DocumentStorage.swift:574-589` (`persistFolderBookmark`); `ios/Sources/KiwixMobilePlugin.swift:890-892` and `:935-938` (`processSelection`)
Trigger: "Import a folder copy" from a provider whose folder URL cannot produce bookmark data, or any I/O error creating `Application Support/kiwix-provider-bookmarks/<uuid>.bookmark`.
Causal path:
1. After `destination.finish()` and `committed = true`, `if folder { try persistFolderBookmark(urls[0], destination: destination.url) }` runs.
2. A throw lands in the `catch` branch `if committed …` and calls `fail(current, error, retainedDirectory:)`.
3. Rust's `mobile_documents("pickLibraryDirectoryCopy").await?` (`src-tauri/src/mobile_service.rs:1231`) returns the error before `mobile_import_paths`, so the fully copied and fsynced staging directory is left unimported.
4. The file it writes is never read: `kiwix-provider-bookmarks` has no other reference in `src-tauri/src`, the plugin sources, `src`, `scripts` or `docs`. Each folder import also leaves a permanent bookmark blob that no privacy or cleanup path removes.
Smallest correct fix: Delete `persistFolderBookmark` and its call, and remove the `kiwix-provider-bookmarks` directory once at plugin load. If a future feature needs it, make it best-effort (`try?`) and add a reader.
Regression test: None needed if removed. Otherwise a Swift test that a folder import still resolves with `paths` when bookmark creation throws.

### APPLE-9 — The iOS plugin's XCTest suite is not run or built by any CI job and cannot resolve from a clean checkout
Severity: Low        Category: maintainability
Platforms: iOS (CI)
Evidence class: source-traced
Location: `src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/*.swift` (8 files, 1,345 lines); `ios/Package.swift` (`.package(name: "Tauri", path: "../.tauri/tauri-api")`); `.github/workflows/checks.yml`, `build.yml`
Trigger: Any change to the Swift plugin.
Causal path: No workflow or script references `KiwixMobileTests`, `swift test` or an iOS `xcodebuild`. The only native test CI runs is `scripts/test-native-sanitizers.sh`, which builds `localswarm_tests.mm` alone. `dependencies.yml` uses the iOS target for cargo-deny only. The package dependency `../.tauri/tauri-api` is absent from the repository (it is generated by a Tauri iOS build), so the suite cannot resolve standalone. The 2026-11-25 record confirms the Swift edits were checked only with `swiftc -parse`.
Smallest correct fix: Add a macOS CI job (or extend `native-sanitizers`) that materializes `.tauri/tauri-api` and runs `xcodebuild test -scheme KiwixMobile -destination 'platform=iOS Simulator,name=…'` in `ios/`. Until then, state in `docs/SANITIZERS-AND-FUZZING.md` that these tests are local-only.
Regression test: The CI job itself.

---

**Covered (read end to end)**
- Swift: `BackgroundAcquisition.swift`, `KiwixMobilePlugin.swift`, `DocumentStorage.swift`, `LiveActivity.swift`, `ReadAloudLiveActivity.swift`, both `*ActivityAttributes.swift`, `NativeSpeech.swift`, `SpeechNowPlaying.swift`, `ReaderAudioSession.swift`, `HardwareShortcuts.swift`, `MobileIdleTimer.swift`, `CrashIntake.swift`, `Package.swift`, both widget files.
- App project and config: `project.yml`, all three `Info.plist` files, both entitlements files, `PrivacyInfo.xcprivacy`, `tauri.ios.conf.json`, `tauri.macos.conf.json`, `main.mm`, `ExportOptions.plist`, `Podfile`, `scripts/bundle-macos.mjs`.
- Objective-C++: `localswarm.mm`, `location.mm`, `share.mm`, `print.mm`, `pdf.mm`, `tabbing.mm`.
- Rust: `ios_background.rs`, `live_activity.rs`, `apple_replies.rs`, `apple_requests.rs`, `apple_window.rs`, `share.rs`, `macos_menu_keys.rs`, `sandbox.rs` (logic, not its tests), `menus.rs` (to line 330), `localswarm/apple_transport.rs` (to line 420).
- Rust, targeted paths only: the `mobile.rs` maintain loop, iOS bootstrap and `Opened` handler; `acquisition/manager.rs::reconcile_native` and the native worker loop in `acquisition.rs`; the reader-audio and speech-engine transport in `mobile_service.rs` / `mobile_speech.rs`.

**No finding met the bar in**
- Background `URLSession` reconciliation: the completion handler is always chained and released, and receipts are journaled, recovered and acknowledged exactly once.
- `beginBackgroundTask` / expiry balance, and security-scope start/stop balance on every `ExternalDocuments`, `IncomingOriginalAuthority` and picker path.
- Objective-C ownership and exceptions across FFI, and main-thread confinement in the `.mm` bridges.
- Audio-session interruption and route handling.
- Usage strings and entitlements versus the APIs called. One note: `NSBluetoothAlwaysUsageDescription` in the macOS `src-tauri/Info.plist` has no macOS caller, so it is harmless dead configuration.

**Not covered / partial**
- Partial: `NearbyBluetooth.swift` (first ~330 lines), `MobilePolicy.swift` (first ~200 lines), `notifications.mm` (head and tail only).
- Not read: `NearbyJoin.swift`, `RouteIPAddress.swift`, `localswarm_tests.mm`, `LaunchScreen.storyboard`, the bodies of the Swift tests (names and signatures only).
- Not read (Rust): `location_apple.rs`, `native_pdf/apple.rs`, `native_tabs.rs`, `webkit_backdrop.rs`, `notifications.rs`, `notifications_service.rs`, `power_activity.rs`, `mobile_policy.rs`, `localswarm/ios_discovery.rs`, the remainder of `mobile_service.rs` and `mobile_speech.rs`.
- Not done: the three-way policy comparison against `MobilePolicy.kt`; frontend `-webkit-` / `ios` / `macos` branches beyond safe-area usage and the shortcut listener; `docs/PLATFORM-INTEGRATIONS.md` and `PLATFORM-GAPS.md` were grepped, not read through.