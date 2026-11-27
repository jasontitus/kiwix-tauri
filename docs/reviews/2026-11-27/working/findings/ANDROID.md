ANDROID review, base d98cac2, read-only. Nothing was built or run; every finding is traced through current source. Two platform facts were checked against primary sources: `IpPrefix(InetAddress,int)` is "Added in API level 33" (developer.android.com), and Chromium `AwSettings` scales WebView text by the system `fontScale` by default.

### ANDROID-1 — A copy-import that fails mid-way leaves its staged multi-GB files in app-private storage forever
Severity: High        Category: reliability
Platforms: Android
Evidence class: source-traced
Location: `src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/KiwixMobilePlugin.kt:1467` (`fail`), `:1228-1232` (`launchWorker` catch) + `OwnedDocuments.kt:248-291` (`copy`), `src-tauri/src/mobile_service.rs:1229-1234` and `:1301-1305`, `src-tauri/src/library_files.rs:2104` (`mobile_directory`), `:948` (`remove_empty_import_directory`)
Trigger: "Import a managed copy" (or folder copy, or incoming "Save a copy") of a ZIM larger than free space. The same happens when an SD card/USB provider is pulled mid-copy, or when the second file of a multi-select is rejected (`NOT_AN_ARCHIVE_DOCUMENT`, `DUPLICATE_DOCUMENT_NAME`) after the first was fully copied.
Causal path:
- Rust creates a fresh `archives/mobile-imports-v1/<uuid>/` per attempt (`mobile_staging` → `mobile_directory`).
- `OwnedDocuments.copy` does `files.add(target)` and streams with no free-space check; `output.write` throws ENOSPC.
- `launchWorker` catch: the op is not cancelled, so `main.post { fail(op, error) }`.
- `fail()` only reports `op.staging?.retainedPaths()` in the rejection data and deletes nothing. Only the cancel path calls `cancelCleanup()`.
- Rust: `let result = self.mobile_documents(...).await?;` returns the error without touching `root`. Nothing in Rust or TypeScript reads the returned `paths`.
- The only sweeper is `remove_empty_import_directory`, whose own comment says "non-empty or foreign directories are never touched".
- Result: the partial file is not a library book, is invisible in the UI, and is never reclaimed. Each retry leaks another directory, and the leak is largest exactly when the disk is full.
Smallest correct fix:
- In `fail(op, …)` for staging kinds (`archives`, `directory`), run `op.staging?.cancelCleanup()` on `copyWorker` before rejecting. It removes only files this operation created with O_EXCL. Keep reporting anything that could not be removed.
- In Rust, on `Err` from `pickArchiveCopies` / `pickLibraryDirectoryCopy` / `copyIncomingArchives`, best-effort call `remove_empty_import_directory(&root)`.
- Optional: compare `OpenableColumns.SIZE` with `StatFs` before the first byte and reject with a typed "not enough space" code.
Regression test: a JVM test beside `DocumentCopyBudgetTest.kt` that drives `OwnedDocuments` with a stream throwing after N bytes and asserts the staging directory is empty after the failure path. A Rust test that `mobile_pick_archive_copies` with a failing native stub leaves no child under `mobile-imports-v1`.

### ANDROID-2 — The launch intent is replayed after process death: the import prompt, deep link, selected-text search or pinned article runs again on every cold restore
Severity: Medium        Category: reliability
Platforms: Android
Evidence class: source-traced (the restore semantics are documented platform behaviour, not run here)
Location: `KiwixMobilePlugin.kt:342-360` (`load`), `:1793-1795` (`parseDocumentIntent`, `INTAKE_HANDLED_EXTRA`), `:1827/1939/1972`; `AppEntryPoints.kt:35,136`; `src-tauri/gen/android/app/src/main/java/org/kiwix/workbench/MainActivity.kt:36-39`
Trigger: With Kiwix not running, open a `.zim` from Files ("Open with"), or use text selection → Kiwix, a `kiwix://`/`zim://` link, a pinned article shortcut or the widget. Finish or dismiss it. Android later kills the backgrounded process. Reopen Kiwix from the launcher or Recents.
Causal path:
- "Handled" is only an extra put on the in-process `Intent` object (`intent.putExtra(INTAKE_HANDLED_EXTRA, true)`, `AppEntryPoints.markHandled`).
- After process death Android recreates the Activity with the original launch intent, so the extra is gone.
- `load()` unconditionally calls `receiveDocuments(owner.intent)` and `receiveEntries(owner.intent)`. `MainActivity.onCreate` ignores `savedInstanceState`, and nothing checks `FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`.
- The in-memory dedupe sets (`transientBatchUris`, `durableIntakeKeys`) are empty and an acknowledged batch is no longer in `INTAKE_BATCHES`, so a new batch or entry is admitted and Rust queues it.
- The user gets "Add X to your library?" again, or the stale search/article reopens over the restored session, at every cold restore until the task is swiped away.
- Download-action and completion-tap intents are immune because they have durable nonces/tombstones. Documents and generic entries do not.
- A raw 2026-11-12 review listed the entry half as low with no recorded disposition. The document half and this trigger are new.
Smallest correct fix: pass `savedInstanceState != null` from `MainActivity.onCreate` into `KiwixMobilePlugin.activityCreated(activity, restored)`. For a restored Activity, treat `activity.intent` as already consumed and skip `receiveDocuments` / `receiveEntries` / `receiveCompletionNotification` for it in `load()` and `onResume`. `onNewIntent` deliveries stay untouched. Also skip when the intent carries `FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`. No Rust contract changes.
Regression test: extend `MobileIntakePolicyTest.kt` with a pure predicate `launchIntentConsumable(restored, flags)`. Add an instrumented case in `AndroidEvaluationTest` that launches with ACTION_VIEW, recreates with saved state, and asserts no second `incoming-documents` batch.

### ANDROID-3 — System Back is not connected to the shell: it exits the app from Library/Settings/Catalog and ignores open dialogs and panels
Severity: Medium        Category: usability
Platforms: Android
Evidence class: source-traced
Location: `src-tauri/vendor/tauri-android-2.12.5/src/main/java/app/tauri/AppPlugin.kt:33-48`; `src/App.tsx:2846-2866` (existing escape/back plumbing); no `back-button` / `onBackButtonPress` listener anywhere in `src/`
Trigger: Press or swipe Back in Settings, Catalog, Downloads, Nearby or Library; or with a modal, TOC panel, find bar or mobile tab switcher open in the reader.
Causal path:
- `handleOnBackPressed`: `if (hasListener("back-button")) trigger(...) else if (view?.canGoBack() == true) view.goBack() else { isEnabled = false; current.onBackPressedDispatcher.onBackPressed() }`.
- The frontend never registers that listener, so only WebView session history counts. That history holds only reader-iframe navigations, which `Reader.tsx:3549` handles as native pops.
- In non-reader panes `ReaderHost` has unmounted the iframe, so `canGoBack()` is false and the Activity finishes (≤ API 30) or the task goes to the background (31+).
- With an overlay open in the reader, Back navigates the article underneath instead of closing the overlay.
Smallest correct fix: in `App.tsx` under `isAndroid`, register `onBackButtonPress` from `@tauri-apps/api/app` (present in the installed 2.12.4). Route it through the logic already at 2846-2866: close the top open `<dialog>` as the `"escape"` branch does; else `mobileReaderShortcut("escape")` if a reader overlay is open; else reader `"back"` when `tab.back.length`; else `setPane("library")` when not on Library. Only when nothing is left, send the task to the background (a tiny Kotlin `moveTaskToBack` command, or the existing exit path).
Regression test: a `scripts/test-frontend-surfaces.mjs` case that fires the registered handler with a dialog open, then with the Settings pane active, and asserts dialog close and pane change with no exit call.

### ANDROID-4 — Android native failures reach the user as raw `[CODE] - {"code":…}` or `java.lang.IllegalStateException: {…}` text
Severity: Medium        Category: usability
Platforms: Android
Evidence class: source-traced
Location: `KiwixMobilePlugin.kt:1481-1492` (`reject`), `ApplicationResources.kt:161-164,294,305`, `src-tauri/src/user_error.rs:81-101,147-158`, `src/components/UI.tsx:120-121,140-143`
Trigger: Pick a non-ZIM file in "Import a managed copy"; run out of space during a copy; a provider denies access; a foreground-transfer start fails; a busy native operation.
Causal path:
- Kotlin rejects with message `{"code":C,"data":{…}}` and code `C`.
- Tauri's `ErrorResponse` Display renders `[C] - {…}`.
- `user_error::plain` only parses envelopes that start with `{`, and `native_message` only knows the lowercase iOS codes, so the string passes through.
- `errorText` ends with `if (!isIOS) return message;`.
- On screen: `[NOT_AN_ARCHIVE_DOCUMENT] - {"code":"NOT_AN_ARCHIVE_DOCUMENT","data":{"paths":[],"detail":"NOT_AN_ARCHIVE_DOCUMENT"}}`.
- The JNI endpoint path wraps a second time: `java.lang.IllegalStateException: {"message":…}`.
- Two consumers show the drift. `UI.tsx:120` matches exactly `"VISIBLE_ACTIVITY_REQUIRED"`, which the bracketed form never equals. `UI.tsx:141` tests the lowercase iOS spelling of a code Kotlin emits in uppercase.
Smallest correct fix: in `user_error.rs`, make `envelope_code` also accept the `[CODE] - {…}` and `<java class>: {…}` shapes. Add an Android table to `native_message` covering at least `NOT_AN_ARCHIVE_DOCUMENT`, `NATIVE_OPERATION_BUSY`, `DOCUMENT_PERMISSION_DENIED`, `DUPLICATE_DOCUMENT_NAME`, `TEXT_DOCUMENT_TOO_LARGE`, `EXPORT_DESTINATION_NOT_EMPTY`, `STAGING_COPY_BUDGET_EXCEEDED`, the `PROVIDER_*_USE_COPY_IMPORT` family, `VISIBLE_ACTIVITY_REQUIRED`, `TRANSFER_FOREGROUND_START_*`, and `NATIVE_DOCUMENT_ERROR` with an ENOSPC detail. Unknown `[A-Z_]+` codes fall back to `TRY_AGAIN` and log the original. Rust `contains("VISIBLE_ACTIVITY_REQUIRED")` checks run before `plain`, so they are unaffected.
Regression test: `user_error.rs` tests asserting that `plain("[NOT_AN_ARCHIVE_DOCUMENT] - {\"code\":…}")` and the `java.lang.IllegalStateException: {…}` form return sentences with no `{`, `[` or uppercase code.

### ANDROID-5 — Configuration changes not listed in the manifest recreate the Activity, which silently cancels running imports and picker results and stops read-aloud
Severity: Medium        Category: reliability
Platforms: Android
Evidence class: source-traced for the consequences; inference for which devices raise each change
Location: `src-tauri/gen/android/app/src/main/AndroidManifest.xml:30`; `KiwixMobilePlugin.kt:377-389` (`bindActivity`), `:484-510` (`retireWindowUi`), `:1243-1271`; `src-tauri/src/mobile_service.rs:1232`
Trigger: `configChanges` lacks `navigation`, `density`, `layoutDirection` and `fontScale`. Examples: attaching or reconnecting a hardware keyboard with arrow keys (the app ships `HardwareShortcutKeys`); folding/unfolding or moving to a display with another density; switching the in-app language between LTR and RTL.
Causal path:
- Relaunch → `activityCreated` → `bindActivity` → `retireWindowUi()`.
- `speech.shutdown()` ends read-aloud.
- `op.cancellation.cancel()`: a running multi-GB copy unwinds through `finishCancelledCleanup` to `complete(op, cancelledResult(op.kind))`. A pending picker is completed as cancelled, and its later result is dropped at `documentResult`'s `!ownsUi(op)` check.
- Rust sees `cancelled: true` and returns `Ok([])`, so the UI shows nothing, as if the user had pressed Cancel.
- The WebView is destroyed and the shell reloads.
Smallest correct fix:
- Add `navigation|layoutDirection` to `android:configChanges`. Neither affects the web UI or the insets code. Add `density` only after an emulator check of WebView DPR handling. Keep `fontScale` recreating (see ANDROID-8).
- In `retireWindowUi`, when the cause is Activity replacement, finish a copying/picking op with `fail(op, NativeDocumentError("ACTIVITY_RECREATED"))` (mapped in ANDROID-4) instead of a cancelled result.
Regression test: a manifest assertion in an existing `scripts/test-*.mjs` that `configChanges` contains `navigation` and `layoutDirection`. A JVM test for a pure `retirementOutcome(kind, copying, replaced)` helper returning failure, not cancelled.

### ANDROID-6 — "Move: remove originals" can never succeed for opened-in-place archives because no persisted write grant is ever taken
Severity: Medium        Category: usability
Platforms: Android
Evidence class: source-traced
Location: `ExternalArchiveMove.kt:18-22` (`requireGrant`), `OwnedDocuments.kt:555-556`, `KiwixMobilePlugin.kt:1147,1168`, `src-tauri/src/library_root.rs:495`, `src-tauri/src/provider_reference.rs:118-122`, `src/components/SelectedStorageActions.tsx:102-108`
Trigger: Library → select a provider-backed archive → "Copy or move selected archives" → tick "Move: remove originals…" → Move.
Causal path:
- `relocate_selected` → `prepare_provider_move` → `prepareMove` → `requireGrant`, which needs a persisted grant with `isReadPermission && isWritePermission`.
- Every `takePersistableUriPermission` in the plugin passes only `FLAG_GRANT_READ_URI_PERMISSION` (three call sites), so `isWritePermission` is never true.
- The call always throws `PROVIDER_MOVE_REQUIRES_PERSISTED_WRITE_GRANT`. The `?` at `library_root.rs:495` aborts the whole operation, including the copy.
- The user reads "The provider has not authorized moving this archive: java.lang.IllegalStateException: {…}", which blames the provider.
- `ExternalArchiveMove.delete`, `ProviderMoveIdentity.kt` and the Rust delete-receipt journaling are unreachable on a shipped device, though they have unit tests.
Smallest correct fix: in `SelectedStorageActions.tsx`, disable the Move checkbox with an explanatory line when any selected book has `sourceReference`, and reword the hint. Leave the native delete path alone until a deliberate re-pick-with-write flow exists; widening every picker to persist WRITE would be a security-posture change, not a bug fix.
Regression test: a frontend surface test that, with a `sourceReference` book selected, the Move option is disabled and `library_files_relocate` is called with `removeOld:false`.

### ANDROID-7 — `KiwixTransferService` can stop itself before calling `startForeground` after a `startForegroundService` start, which the platform punishes by killing the app
Severity: Medium        Category: reliability
Platforms: Android 8+
Evidence class: source-traced (platform crash rule is documented, not reproduced; main trigger is a narrow race)
Location: `KiwixTransferService.kt:168-183` (admission-null and pending-event branches), `:226-233` (catch → `finish()`); contrast `SpeechPlayback.kt:197-206`
Trigger: Tap "Pause" on the transfer notification while the app is visible, within a few milliseconds of the 3 s Rust tick in `mobile.rs:736`.
Causal path:
- Pause calls `finish()`, so `running` becomes false.
- Rust's `foreground_transfers` is still true until the async event arrives, so `transferState(active=true)` reaches `startTransferService` → `ContextCompat.startForegroundService`.
- `onStartCommand` sees `TransferEvents.peek(this).length() != 0` and calls `finish()` (`stopForeground` + `stopSelf()`) without ever calling `startForeground`.
- Android then raises `ForegroundServiceDidNotStartInTimeException` ("did not then call Service.startForeground()") and the process dies.
- The admission-null branch (`stopSelf(startId)`) and the catch branch have the same shape.
- `SpeechPlayback.foreground` already documents and follows the rule: "The platform contract requires one startForeground call even when the service is about to stop."
Smallest correct fix:
- In `startTransferService`, reject with `TRANSFER_CONTROL_PENDING` before `startForegroundService` when `TransferEvents.peek()` is non-empty.
- In the three early-exit branches of `onStartCommand`, call `ServiceCompat.startForeground` with a minimal notification before `finish()` / `stopSelf`, mirroring `SpeechPlayback.foreground`.
Regression test: extract the branch decision into a pure function (`UPDATE` + pending events → `ForegroundThenStop`) and assert it in a JVM test. A plugin-level test that `startTransferService` rejects without starting the service when an event is pending.

### ANDROID-8 — The open `fontScale` → `data-large-text` follow-up: what is missing, and why the obvious wiring double-scales text
Severity: Low        Category: usability
Platforms: Android
Evidence class: source-traced (plus Chromium `AwSettings` source)
Location: `src/App.tsx:803-826` (`if (!isIOS) return`), `src-tauri/src/mobile_service.rs:1125-1128` (`#[cfg(target_os = "ios")] "mobile_dynamic_type_state"`), `src/system-text-scale.ts`, `src/platform.css:148-151` and `:1129-1205`, `src-tauri/vendor/wry-0.57.1/src/android/kotlin/RustWebView.kt` (no `textZoom`)
Trigger: Set Android Font size to Large or larger (up to 200% on Android 14+) and open Kiwix.
Causal path:
- Nothing sends `fontScale` to JavaScript. There is no Kotlin command, the Rust arm is iOS-only, the effect returns early on Android, and every large-text selector is `[data-platform="ios"]`. So `--system-text-scale` stays 1 and `data-large-text` is never set.
- Text is nonetheless already enlarged. Chromium's `AwSettings` does "By default, scale the text size by the system font scale factor" (`textSizePercent = fontScale*100`), and neither wry nor the plugin overrides it. This applies to shell and archive frames alike.
- The real gap is therefore geometry: fixed `--control-height` / `--field-height` and the missing large-text reflows while glyphs are 1.3–2× larger.
- Feeding `fontScale` into `applySystemTextScale` as on iOS would multiply `calc(15px * var(--system-text-scale))` by Chromium's zoom again, giving fontScale².
Smallest correct fix:
1. Kotlin: add a `dynamicTypeState` command returning `{category:"android", scale: applicationContext.resources.configuration.fontScale}`.
2. Rust: widen the cfg on `"mobile_dynamic_type_state"` to Android.
3. `App.tsx`: allow `isAndroid`. In `applySystemTextScale`, on Android set only `dataset.largeText` / `contentSizeCategory` and leave `--system-text-scale` at 1.
4. `platform.css`: change the large-text selectors to `:root:is([data-platform="ios"],[data-platform="android"])[data-large-text="true"]`. Give the two height tokens an Android large-text override.
No change event is needed: `fontScale` is not in `configChanges`, so a change recreates the Activity and WebView and the value is re-read at mount.
Regression test: extend the existing `applySystemTextScale` unit test with an Android case (scale 1.5 → `largeText="true"`, `--system-text-scale` unchanged). A CSS assertion that the large-text selectors match `data-platform="android"`.

### ANDROID-9 — Status and navigation bar strips ignore the in-app theme
Severity: Low        Category: usability
Platforms: Android
Evidence class: source-traced for the theme mismatch; inference for staleness after a system dark-mode toggle
Location: `MainActivity.kt:38-59`, `src-tauri/gen/android/app/src/main/res/values/themes.xml`, `src/platform.ts:55-60` (`if (!isDesktopApp) return`), `src/App.tsx:1250-1256`
Trigger: Settings → Theme "Dark" while the system is light, or the reverse.
Causal path:
- `enableEdgeToEdge()` plus the insets listener pads `android.R.id.content` by the system-bar insets, so the bar areas show the window background.
- That background comes from `Theme.Material3.DayNight`, which follows the system night mode fixed at Activity creation.
- The shell theme lives only in `data-theme`, and `applyWindowTheme` returns early off desktop, so native never learns it.
- Result: a white top strip with dark icons over a dark app, or the inverse.
- Because `uiMode` is in `configChanges`, a system toggle while running does not recreate the Activity, so the strips and icon contrast probably stay stale too.
Smallest correct fix: add an Android branch to `applyWindowTheme` calling a small plugin command (`applyTheme {dark}`) that sets `window.decorView.setBackgroundColor(...)` and `WindowInsetsControllerCompat.isAppearanceLightStatusBars` / `isAppearanceLightNavigationBars`. Re-apply it from the plugin's `onConfigurationChanged` for the "system" setting.
Regression test: a JVM test of a pure `barAppearance(setting, systemNight)` mapping. A frontend test that `applyWindowTheme("dark")` issues the mobile command on Android.

### ANDROID-10 — Read-aloud with the system TTS engine keeps playing through the phone speaker when headphones are unplugged
Severity: Low        Category: usability
Platforms: Android
Evidence class: source-traced
Location: `SpeechPlayback.kt:112-130` (`start`)
Trigger: Start read-aloud with a system voice on wired or Bluetooth headphones, then disconnect them.
Causal path:
- The `ACTION_AUDIO_BECOMING_NOISY` receiver is registered only inside `if (external) { … }`, which is the Kokoro path.
- `NativeSpeech.start` calls `SpeechPlayback.start(context, id, labels, speaking = true, canPrevious = false)` with `external` defaulting to false, so nothing pauses the engine.
- The pause route already works for both engines: `dispatchAction(id,"pause")` → Rust `mobile_speech_action` → `speechPause`.
Smallest correct fix: register the noisy receiver for every session by moving it out of the `external` guard. Keep audio-focus acquisition external-only: the system TTS service requests transient-duck focus per utterance, and the existing `change < 0 → pause` listener would then pause its own speech.
Regression test: split `start` so a pure `needsNoisyReceiver(external)` returns true for both, and assert it in a JVM test. An instrumented check that broadcasting `ACTION_AUDIO_BECOMING_NOISY` yields a `speech-action` pause.

### ANDROID-11 — `IpPrefix(InetAddress,int)` is API 33 but called unguarded with `minSdk 24`, and the surrounding `NearbyRoutes` commands have no Rust caller
Severity: Low        Category: maintainability
Platforms: Android 8–12L
Evidence class: source-traced (API level verified; runtime outcome on API 28 is inference)
Location: `LocalHotspot.kt:90`, `NearbyRoutes.kt:62` (`NearbyRouteChecks.onLink`), `MobilePolicy.kt:155` (comment), plugin `build.gradle.kts` (`minSdk = 24`), `KiwixMobilePlugin.kt:221,353,715-716`, `src-tauri/plugins/tauri-plugin-kiwix-mobile/src/android.rs:73`
Trigger: Android hosting the temporary Wi-Fi while a peer endpoint is bound (`hotspotQuicRoute` with `address`, sent from `nearby_route.rs:77`) on a pre-13 device.
Causal path:
- Both sites construct `android.net.IpPrefix(address, prefix)`. That constructor is public SDK only from API 33; before that it is a non-SDK member.
- `MobilePolicy.inPrefix` was written to avoid exactly this ("Prefix containment without hidden IpPrefix constructors (Android 9 safe)"), but the other two sites were not converted.
- If linkage is refused, the result is `NoSuchMethodError`, a `java.lang.Error`. The surrounding `catch (error: Exception)` blocks do not catch it, so it would kill the main thread.
- Separately, `nearbyRouteBindSocket` / `nearbyRouteState` appear only in the `owns()` list and are never called from Rust. `load()` still runs `nearbyRoutes.install()`, registering a process-lifetime `NetworkCallback` for an unused table.
Smallest correct fix: replace both constructor uses with the existing byte-wise prefix test (move `inPrefix` to a shared internal function). Delete the two dead commands, `NearbyRoutes`' route table and the `install()` call, keeping `NearbyRouteChecks`.
Regression test: a JVM test for the shared `inPrefix` covering /24, /31, /32 and non-octet prefixes. A script check that every Kotlin `@Command` name appears in a Rust call site.

### ANDROID-12 — User-visible sentences composed in Kotlin bypass localisation
Severity: Low        Category: usability
Platforms: Android
Evidence class: source-traced
Location: `DeviceZimScan.kt:80-87,185-187,210-212,271-274,365,496`; `src/components/Library.tsx:1111,1138-1139`; `SpeechPlayback.kt:346-355` (`ensureChannel`)
Trigger: Use Library → "On this device" → Scan in any non-English locale; or change the app language after the first read-aloud.
Causal path:
- Scan `limitations` and per-entry `problem` are English literals built in Kotlin. One of them is added on every Android 13+ scan.
- `device_scan.rs` only length-checks them, and `Library.tsx` renders `{limitation}` / `{entry.problem}` without `t()`. The strings are absent from `src/locales`.
- Some embed raw exception text: "MediaStore could not be searched: ${error.message}".
- The read-aloud channel is created only `if (manager.getNotificationChannel(CHANNEL) == null)`, so its name stays in the first language used. The transfer and completion channels re-create with the current label on each use.
Smallest correct fix: emit stable codes from Kotlin (for example `ANDROID13_FOLDER_ACCESS`, `NO_FOLDER_ACCESS`, `CANDIDATE_CAP`, `FOLDER_ACCESS_EXPIRED`, `NOT_READABLE_IN_PLACE`, `SPLIT_INCOMPLETE`) with optional parameters, and map them through `t()` in `Library.tsx`. In `ensureChannel`, always call `createNotificationChannel`; it is idempotent and updates the name.
Regression test: a script test that every limitation/problem code emitted in `DeviceZimScan.kt` has an entry in the English catalog. A JVM test that the channel label passed after a locale change is the new one.

**Covered** (read end to end unless marked partial)
- Kotlin: `KiwixMobilePlugin.kt` (all 2,400 lines), `KiwixTransferService.kt`, `DownloadNotifications.kt`, `AndroidCompletionNotifications.kt`, `ApplicationResources.kt`, `SpeechPlayback.kt`, `KiwixSpeechService.kt`, `NativeSpeech.kt`, `SpeechPosition.kt`, `SpeechRecognition.kt`, `OwnedDocuments.kt`, `DeviceZimScan.kt`, `LocalHotspot.kt`, `NearbyJoin.kt`, `NearbyRoutes.kt`, `NearbyNative.kt`, `MobilePolicy.kt`, `ReaderLocation.kt`, `AppEntryPoints.kt`, `AppShortcuts.kt`, `KiwixSearchWidget.kt`, `HardwareShortcutKeys.kt`, `ExternalArchiveMove.kt`, `NativeArticlePdf.kt`, `AppLocale.kt`, `StorageLocations.kt`, `BundledKokoro.kt`, `ExportScratch.kt`, `KiwixExportChoiceReceiver.kt`.
- Kotlin, partial: `CrashIntake.kt` (all but the tail of `safeUrl`); `NearbyBluetooth.kt` (first 330 lines, plus an API-level scan of the rest).
- App project: both `AndroidManifest.xml` files, `MainActivity.kt`, `app/build.gradle.kts`, plugin `build.gradle.kts`, `proguard-rules.pro`, `consumer-rules.pro`, `gradle.properties`, backup and data-extraction rules, `file_paths.xml`, themes, widget info and export paths.
- Rust: plugin `android.rs`, `lib.rs`, `android_pdf.rs`; `mobile.rs` (all); `mobile_service.rs` lines 225-300, 404-445, 522-545, 565-865, 1094-1316, 1560-2310; `user_error.rs`; relevant parts of `provider_reference.rs`, `library_root.rs`, `library_files.rs`, `nearby_route.rs`.
- Vendored substrate: tauri-android `PluginManager.kt`, `AppPlugin.kt`, parts of `Plugin.kt` and `Invoke.kt`; wry `WryActivity.kt`, `RustWebView.kt`.
- Frontend Android branches: `App.tsx` (dynamic type, locale, incoming import, shortcut handler), `UI.tsx` `errorText`, the `Library.tsx` device-scan block, `SelectedStorageActions.tsx`, `IncomingArchiveDialog.tsx`, `system-text-scale.ts`, `platform.ts`, the large-text rules in `platform.css`.
- Docs: the 2026-11-25 record in full; the 2026-11-23 record and `PLATFORM-GAPS.md` for Android items.

**Not covered**
- Rust: `mobile_policy.rs`, `mobile_rebase.rs`, `mobile_backup.rs`, `android_notifications_service.rs`, `android_tls.rs`, `android_evaluation.rs`, `location_android.rs`, `service/android_voice_pack.rs`; `mobile_speech.rs` beyond payload spot-checks and `mobile_speech_action`; `device_scan.rs` beyond the limitation/problem pass-through; `native_pdf/android.rs` beyond its call sites.
- Kotlin/Gradle: `ProviderMoveIdentity.kt`, the second half of `NearbyBluetooth.kt` in detail, `buildSrc` tasks, `res/` layouts and translated strings, the plugin's unit tests, `androidTest`.
- Config and docs: `network_security_config.xml` beyond its header, `docs/ANDROID_PERFORMANCE.md`, Android rules in `platform.css` other than large-text.
- R8: I could not establish whether AGP 9's `optimization { enable = true }` (which replaced the stock `getDefaultProguardFile(...)`) still applies the default rule that keeps `native` method names. `consumer-rules.pro` does not itself keep `NativeArticlePdf.authorityCurrent` / `commitHandoff`, which Rust resolves by JNI symbol name. This is unverified and not reported as a finding; an explicit `-keepclasseswithmembernames class org.kiwix.mobile.** { native <methods>; }` would remove the doubt.
- Nothing meeting the bar was found in the BLE transport, the read-aloud state machine, the durable intake queue, notification tap admission, SAF reference bookkeeping or the network-policy callbacks beyond the items above.