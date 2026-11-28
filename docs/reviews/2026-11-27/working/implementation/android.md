# Lane `android` — implementer report

## Read this first

**Checkpoint `08f3094` (2026-11-28 00:07:53) contains six of my Kotlin files in a deliberately broken state.** I ran a mutation check in the shared tree (reverted eight decisions for about 46 s to watch the tests fail) and the checkpoint landed inside that window. The working tree was restored byte-for-byte (`cmp` against my pre-mutation copies) and is correct; the next checkpoint will carry the right content. Do not build or review from `08f3094`. Affected files: `KiwixMobilePlugin.kt`, `KiwixTransferService.kt`, `SpeechPlayback.kt`, `DeviceZimScan.kt`, `WindowTheme.kt`, `OwnedDocuments.kt`. `git diff 08f3094 -- <plugin android dir>` shows exactly the eight reverted lines. I should have mutated a copy, not the shared tree.

Nothing ran on Android itself (no emulator or device). Every lifecycle claim below is source reasoning plus JVM tests of the pure decisions.

Final state: Kotlin compiles (plugin + app module), 64/64 plugin unit tests pass (42 baseline + 22 new), Android Rust clippy is clean, `tsc` is clean, my node tests pass. Two node tests fail on generated files the integration owner regenerates (details under Commands).

---

### ANDROID-1 — Failed copy-import leaves staged files forever
Disposition: fixed (Kotlin); fixed differently (Rust)
Verification of the finding: Confirmed in `OwnedDocuments.kt` and `KiwixMobilePlugin.kt`. `fail()` only reported `retainedPaths()`; only the cancel path cleaned up. The ownership claim holds: a file is recorded only after `Os.open(O_CREAT|O_EXCL|O_NOFOLLOW)` succeeded, a directory only after `mkdir()` returned true, and `File.delete()` cannot remove a non-empty directory. Rust creates the per-attempt UUID root, so Kotlin never records or removes it.
Change:
- `KiwixMobilePlugin.kt: launchWorker` catch — for `archives`/`directory` kinds (`stagingFailureRemovesCopies`), calls `op.staging?.cancelCleanup()` on the copy worker before posting `fail()`. `fail()` still reports whatever could not be removed. The branch only runs when the task threw, so nothing is deleted after a result was delivered.
- `OwnedDocuments.kt` — the two tracking lists moved into `internal class StagedEntries` so the ownership rule is JVM-testable. Behaviour is identical.
- `mobile_service.rs` (both call sites, covering all three commands) — on `Err`, `let _ = std::fs::remove_dir(&root); return Err(error);`.
- **Why not `remove_empty_import_directory`:** it is a private `fn` in `library_files.rs`, which I do not own. `remove_dir` removes only an empty directory and `root` is the directory this call just created.
- Free-space pre-check: not done. `StatFs` is used nowhere in the plugin and providers often report no size.
Tests: `StagingCleanupTest` (4 tests). Observed failing with the fix reverted (3 of 4; the fourth guards a different property).
Not exercised: the real failure path (ENOSPC, provider pulled) on a device. The Rust edit is type-checked for Android but not run. The cancelled path still leaves the empty per-attempt directory, as before; that was outside my allowed edit.
Lifecycle reasoned: copy fails on worker → cleanup on worker → `main.post { fail }` → reject → Rust removes empty root. Activity replaced mid-copy: see ANDROID-5.

### ANDROID-2 — Launch intent replayed after process death
Disposition: fixed, with one addition to the reviewer's rule
Verification of the finding: Confirmed. "Handled" is only an extra on the in-process `Intent`; `load()` and `onResume` re-offer `activity.intent` unconditionally. The generated `WryActivity.onNewIntent` never calls `setIntent`, so `activity.intent` is always the launch intent.
Change:
- `MainActivity.onCreate` passes `savedInstanceState != null` to `KiwixMobilePlugin.activityCreated(activity, restored)`.
- New pure `launchIntentConsumable(restored, flags, unofferedByPredecessor)`; non-consumable Activities are recorded and `launchIntent(owner)` returns null for them in `load()` and `onResume`. `onNewIntent` is untouched.
- **Addition:** a same-process recreation keeps the intent consumable only if no plugin has been offered it yet. Without this, a configuration change during a cold "Open with" start (before the plugin loads) would lose the file under the plain "restored ⇒ consumed" rule.
Tests: three new tests in `MobileIntakePolicyTest`; observed failing with the predicate reverted (2 of 3). `scripts/test-android-manifest.mjs` checks the wiring by source assertion (weak).
Not exercised: actual restore after process death, Recents relaunch, `onNewIntent`.
Lifecycle reasoned:
- Cold "Open with": not restored → offered once in `load()`; later resumes are deduped by the existing extra.
- Warm new intent: `onNewIntent`, unchanged.
- Process death then restore: restored and first in process → skipped.
- Back, then reopen from Recents: `FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY` → skipped.
- "Don't keep activities": predecessor was already offered → skipped.
Device test should confirm: no second "Add X to your library?" after a background kill; first Open-with still prompts once; a second Open-with while running still prompts.

### ANDROID-5 — Unlisted configuration changes recreate the Activity
Disposition: partial — `navigation` added, `layoutDirection` left out, failure code added; user-visible notice needs a handoff
Verification of the finding: Confirmed. On recreation the old Activity's `onDestroy` reaches `retireWindowUi()` first, which settled the operation as cancelled.
Change:
- Manifest: `configChanges` gains `navigation` only.
- **`layoutDirection` left out, as you allowed.** As I recall the platform and AppCompat source (not checked here), both attach `CONFIG_LAYOUT_DIRECTION` to every locale change, not just LTR↔RTL. Handling it would stop the recreation that today follows every in-app language switch; the app would then change language in place with a live WebView. On API 24–32 AppCompat's in-place path updates Resources only and does not re-resolve the decor view's direction. That path is untested and I cannot run it.
- `retireWindowUi(replaced)`: `bindActivity` passes true; `onDestroy`/`activityDestroyed` pass `!activity.isFinishing`; Rust's `retireWindow` keeps false. New pure `retirementOutcome(copying, viewerScratch, replaced)`. A replaced Activity settles a pending pick or copy as `ACTIVITY_RECREATED` instead of cancelled; a running worker still cleans up first.
Tests: `replacedActivityFailsAPendingOperationInsteadOfCancellingIt` (observed failing when reverted); manifest test asserts `navigation` handled and `fontScale`/`density` not handled.
Not exercised: any real recreation.
**Limit you should know:** when the Activity is recreated, the WebView that issued the command is destroyed too, so the rejection reaches nobody. The Kotlin change makes the outcome truthful and distinguishable in Rust, but the person sees a message only with the Rust handoff below (H2).

### ANDROID-7 — Transfer service can stop before `startForeground`
Disposition: fixed on both sides
Verification of the finding: Confirmed in `KiwixTransferService.onStartCommand` (stale-admission, pending-event and catch branches) against `SpeechPlayback.foreground`.
Change:
- Service: new pure `transferStartDecision(admitted, foreground, pendingEvents)`. When a delivery is about to stop a service that is not yet foreground, `enterForegroundBeforeStop(intent)` calls `ServiceCompat.startForeground` once with a minimal notification. It uses the title/channel labels the intent carried and falls back to the app's own label; it never renames an existing channel to the fallback. Refusal by the platform is caught.
- Plugin: `startTransferService` reserves `foregroundInvoke`, reads `TransferEvents.peek` on `worker` (it is disk-backed), and back on main refuses with `TRANSFER_CONTROL_PENDING` before `startForegroundService`. The Rust-visible code is the same as before.
Tests: `TransferStartDecisionTest` (4); observed failing when `foregroundFirst` is forced false (2 of 4).
Not exercised: a real service start, the Pause race, a real `ForegroundServiceDidNotStartInTimeException`.
Residual, not changed: `stopOwned` calls `context.stopService` while a start may still be pending. That is the same class of platform rule and was not in the finding.

### ANDROID-10 — System-TTS read-aloud ignores headphone unplug
Disposition: fixed
Verification of the finding: Confirmed; the receiver was inside `if (external)`. The pause route for non-Kokoro sessions exists in `mobile_speech_action`.
Change: `SpeechPlayback.start` registers the becoming-noisy receiver for every session via `speechAudioPolicy(external)`; audio focus stays external-only. Unregistration is unchanged and covers every path: `releaseAudio()` runs in `stop()` and at the top of every `start()`.
Tests: two tests in `SpeechAndProviderAuthorityTest`; the receiver one observed failing when reverted.
Not exercised: a real unplug. If receiver registration throws, a system-voice session now fails closed like the Kokoro path already did.

### ANDROID-11 — `IpPrefix(InetAddress,int)` on API < 33
Disposition: constructor fixed; dead commands left in place
Verification of the finding: Both constructor uses confirmed. `nearbyRouteBindSocket`/`nearbyRouteState` have no caller anywhere in `src-tauri/src` or the plugin's `src/`; the only Rust mention is the `owns()` list.
Change: `inPrefix` is now a top-level internal function in `MobilePolicy.kt`, used by `NearbyRoutes.kt` and `LocalHotspot.kt`. No `android.rs` edit.
- **Why I did not remove the commands:** they are verified dead, but removal means an edit in Rust I could not compile when I made that call, iOS keeps same-named commands, and the actual defect is fixed without it. Removal list if you want it: the two `@Command`s, the `nearbyRoutes` field and `install()` call, the `ApplicationResources` parameter and four lines, the `NearbyRoutes` class (keep `NearbyRouteChecks`), and the two `owns()` names.
Tests: `NetworkPrefixTest` (4). Not a fail-without-fix test: the function was moved, not changed.

### ANDROID-4 (Rust half) — Raw native failures shown to the user
Disposition: fixed
Verification of the finding: Confirmed from Tauri's `ErrorResponse` Display (`[code] - message`) and `ApplicationResources.call` (`java.lang.IllegalStateException: {…}`). Every mapped code was checked against the Kotlin that throws it.
Change (`user_error.rs`): `plain` recognises a whole message in either Android shape (`[UPPER_CODE]…` or `<java throwable>: …`, with bounded nesting) and returns:
1. the existing speech sentence for `SPEECH_*` codes;
2. a table sentence (below), or the no-space sentence when the detail contains `ENOSPC`, `EDQUOT` or `No space left`;
3. for `NEARBY_`/`BLE_`/`JOIN_`/`HOTSPOT_`/`LOCAL_NETWORK_` codes: the bracketed form unchanged (the frontend's new `nativeCodeText` reads it), or the bare code when Java-wrapped;
4. otherwise `TRY_AGAIN`, logging only the code (the payload can name a person's file).

Deliberately untouched: lower-case iOS bracket codes (`UI.tsx` matches one), bare codes, Rust-prefixed sentences, and Rust's own `{…}` envelopes. That last one matters: the provider retry envelope carries `PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT` and the frontend parses it.

Ordering of earlier checks: `mobile_service.rs:~2014` (`VISIBLE_ACTIVITY_REQUIRED`), `nearby_service.rs:280`, `provider_reference.rs` and `location_android.rs` all match before the dispatch boundary in `lib.rs` calls `plain`. Unaffected.

Codes and exact output:

| Code(s) | Sentence |
|---|---|
| `NOT_AN_ARCHIVE_DOCUMENT` | That file is not a ZIM archive. Choose a file whose name ends in .zim. |
| `DUPLICATE_DOCUMENT_NAME` | Two of the chosen files have the same name. Import them one at a time. |
| `NATIVE_OPERATION_BUSY` | Another file action is still open. Finish or cancel it, then try again. |
| `DOCUMENT_PERMISSION_DENIED`, `PROVIDER_PERMISSION_DENIED` | Android did not let Kiwix read that file. Choose it again. |
| `TEXT_DOCUMENT_TOO_LARGE` | That file is too large to import. Choose a smaller file. |
| `EXPORT_DESTINATION_NOT_EMPTY` | Kiwix does not overwrite an existing file. Save it under a new name. |
| `EXPORT_CANCELLED_PARTIAL` | Saving stopped before the file was complete. Delete the unfinished file, then save again. |
| `STAGING_COPY_BUDGET_EXCEEDED` | That is too much to import at once. Import fewer files at a time. |
| `STAGING_DURABILITY_ERROR`, `EXPORT_DURABILITY_ERROR` | The file could not be saved completely. Check the free space on this device, then try again. |
| `PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT` | That file is not available right now. Reconnect its storage and try again, or import a copy. |
| `PROVIDER_UNAVAILABLE_USE_COPY_IMPORT`, `PROVIDER_NOT_SEEKABLE_USE_COPY_IMPORT`, `SCANNED_ARCHIVE_UNAVAILABLE` | That file cannot be read where it is. Import a copy into Kiwix instead. |
| `PROVIDER_CANNOT_REMEMBER_USE_COPY_IMPORT` | Android will not let Kiwix keep access to that location. Import a copy into Kiwix instead. |
| `PROVIDER_INCOMING_CHOOSE_ORIGINAL_OR_COPY` | This file cannot be opened from its current location. Add a copy to your library, or choose the original file in Files. |
| `DOCUMENT_PICKER_FAILED`, `DOCUMENT_PICKER_EMPTY_RESULT`, `DOCUMENT_URI_REQUIRED`, `DOCUMENT_METADATA_UNAVAILABLE`, `DOCUMENT_STREAM_UNAVAILABLE`, `DOCUMENT_TREE_UNAVAILABLE` | The file could not be opened. Choose it again. |
| `DOCUMENT_PROVIDER_STILL_LOADING` | That folder is still loading. Wait a moment, then try again. |
| `SCAN_RESULT_EXPIRED` | That scan result is out of date. Scan again, then choose the file. |
| `SCAN_ALREADY_RUNNING` | A scan is already running. Wait for it to finish. |
| `VISIBLE_ACTIVITY_REQUIRED` | Bring Kiwix back to the screen, then try again. |
| `ACTIVITY_RECREATED` (new) | Kiwix restarted its window before this finished. Start it again. |
| `ACTIVITY_DESTROYED`, `STALE_ACTIVITY_GENERATION` | Kiwix was closed before this finished. Open Kiwix and try again. |
| `TRANSFER_FOREGROUND_START_FAILED`, `TRANSFER_FOREGROUND_START_TIMEOUT` | Android did not let the download keep running in the background. Keep Kiwix open, then try again. |
| `TRANSFER_START_PENDING`, `TRANSFER_START_CANCELLED`, `TRANSFER_CONTROL_PENDING` | Downloads were just paused or are still starting. Wait a moment, then try again. |
| any code with an ENOSPC/EDQUOT detail | There is not enough free space on this device. Free some space, then try again. |
| `NATIVE_DOCUMENT_ERROR` without that detail, any unknown code, code-less Java exception text | That did not work. Try again, and restart Kiwix if it keeps happening. |

Tests: seven new tests in `user_error.rs`, **written, not run by me under cargo.** I did run them outside cargo: a copy of the file with only `disk_warning` stubbed and its two dependent tests removed, compiled with plain `rustc --test` against existing rlibs — 13 passed. With the Android branch disabled, 5 of the new tests fail. The integration owner should still run `cargo test --lib user_error`.
Not exercised: real device error strings.

### ANDROID-8 — System font scale to large-text layout
Disposition: steps 1, 2, 4 and the `system-text-scale.ts` part of 3 done; `App.tsx` is handoff H1; the height-token override was not needed
Verification of the premise: no `textZoom`/`setTextZoom`/`fontScale` anywhere in vendored wry, tauri-android, tauri-runtime-wry, the plugin or the app sources; `RustWebView.kt` sets six unrelated settings. That Chromium multiplies text by `fontScale` by default is the reviewer's primary-source claim; I did not re-verify it and it is not confirmed on a device.
Change:
- Kotlin `@Command dynamicTypeState` returns `{category: "android", scale: <fontScale as double>}`, the same field names and types as Swift's `dynamicTypeSnapshot`.
- Rust: the iOS-only cfg on `"mobile_dynamic_type_state"` is removed (the module is already mobile-only). Routing: the command is not in `owns()`, so it goes through `run_mobile_plugin_async` to the Kotlin `@Command`; no `android.rs`/`lib.rs` edit needed.
- `system-text-scale.ts`: for category `"android"` (exported `ANDROID_TEXT_CATEGORY`), `--system-text-scale` stays `1`; only `data-large-text` (scale > 1.35) and the category are set.
- `platform.css`: all 16 large-text selectors now read `:root:is([data-platform="ios"], [data-platform="android"])[data-large-text="true"]` — same specificity as before.
- **Height tokens not overridden:** `--control-height` and `--field-height` are consumed only as `min-height` (three sites), so larger text already grows the controls.
Tests: four new tests in `scripts/test-system-text-scale.mjs` (Android scale, plugin category string, CSS selector coverage, `fontScale` not in `configChanges`). All pass.
Not exercised: how the reflow rules look on Android. They were designed for iOS and I did not render them; they need a build plus the surface tests or a device at 150–200 % font size.

### ANDROID-9 — System-bar strips ignore the in-app theme
Disposition: fixed
Verification of the finding: Confirmed. `enableEdgeToEdge()` plus inset padding shows the window background, which follows the system night mode at creation; `applyWindowTheme` returned early off desktop.
Change:
- New `WindowTheme.kt`: pure `barAppearance(setting, systemNight)` and `WindowTheme.apply`. It sets the window background to the Android palette's `--paper` (`#F8FAFD` light, `#131314` dark) and both light-bar flags via `WindowInsetsControllerCompat`.
- On API 26–28 only, it also sets the navigation scrim. `enableEdgeToEdge` fixed that scrim from the system mode, and flipping the icon colour alone would give white icons on a white scrim.
- I used `window.setBackgroundDrawable` rather than `decorView.setBackgroundColor` (the documented window API).
- Plugin: `@Command applyTheme {theme}`, and `onConfigurationChanged` re-applies so a "system" theme follows a live dark-mode toggle.
- Rust: new Android-only arm `"mobile_apply_theme"` (validates `system|light|dark`).
- `platform.ts`: Android branch calls `api("mobile_apply_theme", { theme })`.
Tests: `WindowThemeTest` (3); observed failing when the mapping is reverted (2 of 3). No frontend unit test: `platform.ts` cannot be imported in node (`import.meta.env`).
Not exercised: actual strip colours and icon contrast on any API level. The bottom strip uses `--paper` while the compact nav bar is `--sidebar`, so a slight tone step at the bottom is expected. A recreated Activity shows default strips until the shell reloads and calls again.

### ANDROID-12 — Kotlin-composed sentences bypass localisation
Disposition: channel half fixed; device-scan half is handoff H4
Change: `SpeechPlayback.ensureChannel` always calls `createNotificationChannel`, so the channel name follows the current label. No test: `NotificationChannel` cannot be constructed in the JVM test setup.
Device-scan: Kotlin strings left as they are. `device_scan.rs` declares `NativeScan` and `Candidate` with `deny_unknown_fields`, so any new field fails the whole scan unless that file changes in the same commit.

### LIB-2 (Kotlin half) — SAF walk includes dot-named entries
Disposition: fixed
Change: `DeviceZimScan.walk` skips names for which `hiddenScanName(name)` is true, files and folders alike, matching the shared-storage walk. A granted tree root that is itself dot-named is still walked.
Tests: `DeviceScanPolicyTest` (observed failing when reverted). It tests the predicate, not the walk.

### R8 note — native method names
Disposition: keep rule added (harmless and clarifying)
Finding: Rust resolves `Java_org_kiwix_mobile_NativeArticlePdf_authorityCurrent`, `…_commitHandoff` and `…KiwixMobilePlugin_installServiceEndpoint` by symbol name. `consumer-rules.pro` kept only `KiwixMobilePlugin`. From AGP 9.3.1 bytecode in the Gradle cache, `optimization.keepRules.includeDefault` defaults to true and the default rule file contains the generic native-names keep, so a release build probably already keeps them.
Change: added `-keepclasseswithmembernames,includedescriptorclasses class org.kiwix.mobile.** { native <methods>; }` to the plugin's `consumer-rules.pro`.
Not exercised: no minified build can be produced here. `proguard-rules.pro` unchanged.

---

## Commands run (final results)

- Kotlin compile, plugin + app: `BUILD SUCCESSFUL in 41s`; 5 warnings, all existing deprecations.
- Plugin unit tests: `BUILD SUCCESSFUL in 18s`; 13 result files, 64 tests, 0 failures, 0 errors.
- Mutation run (eight decisions reverted): `64 tests completed, 12 failed`; files then restored and re-verified.
- `android-clippy.sh`: `Finished dev profile … exit=0`; warnings only from the vendored `cesu8` crate. This covers my `mobile_service.rs` arms and `user_error.rs` library code for `aarch64-linux-android`, without tests.
- Scratch `rustc --test` of `user_error.rs`: `13 passed; 0 failed`; with the Android branch disabled, `8 passed; 5 failed`.
- `rustfmt --edition 2021 --check src/user_error.rs`: clean (baseline was clean too).
- `npx --no-install tsc --noEmit`: exit 0.
- `node --test scripts/test-system-text-scale.mjs`: 7 pass. `node --test scripts/test-android-manifest.mjs`: 4 pass.
- `node --test scripts/test-*.mjs`: 497 tests, 484 pass, 2 fail, 11 skipped. The two failures:
  - "docs/ERRORS.md matches the error copy" — the file is generated; my new Rust sentences contribute.
  - "every UI string … localization manifest" — I added no `t()` strings; the listed keys are other lanes'.
- `node scripts/check-unsafe-census.mjs`: passes; no `unsafe` added.
- Not run: host `cargo check/test/clippy` (per lane rule), `npm run build`.

## Files changed

- `src-tauri/gen/android/app/src/main/AndroidManifest.xml`
- `src-tauri/gen/android/app/src/main/java/org/kiwix/workbench/MainActivity.kt`
- `src-tauri/plugins/tauri-plugin-kiwix-mobile/android/consumer-rules.pro`
- Plugin main sources: `KiwixMobilePlugin.kt`, `KiwixTransferService.kt`, `SpeechPlayback.kt`, `OwnedDocuments.kt`, `MobilePolicy.kt`, `NearbyRoutes.kt`, `LocalHotspot.kt`, `DeviceZimScan.kt`, `WindowTheme.kt` (new)
- Plugin tests: `MobileIntakePolicyTest.kt`, `SpeechAndProviderAuthorityTest.kt`, and new `StagingCleanupTest.kt`, `TransferStartDecisionTest.kt`, `NetworkPrefixTest.kt`, `WindowThemeTest.kt`, `DeviceScanPolicyTest.kt`
- `src-tauri/src/user_error.rs`
- `src-tauri/src/mobile_service.rs` (only: cleanup at the two copy call sites, the dynamic-type cfg, the new theme arm)
- `src/system-text-scale.ts`, `src/platform.ts`, `src/platform.css`
- `scripts/test-system-text-scale.mjs`, `scripts/test-android-manifest.mjs` (new)

No edits to `android.rs`, `lib.rs` (plugin), `App.tsx`, `src/components/*`, `device_scan.rs`, vendored or generated Gradle files.

New user-visible strings: the Rust sentences in the table above, plus `"Invalid theme"` in `mobile_service.rs` (reachable only by a programming error). No new `t()` strings and no new Kotlin text: the pre-stop notification reuses the labels the start carried, or the app label.

New/changed commands and codes: Kotlin `dynamicTypeState`, `applyTheme`; Rust `mobile_apply_theme` (Android), `mobile_dynamic_type_state` now on Android; `activityCreated(activity, restored)` signature; new code `ACTIVITY_RECREATED`.

## Handoffs

- **H1 — `App.tsx` (ANDROID-8).** In the effect that listens for `"kiwix-mobile-dynamic-type"` and calls `applySystemTextScale` (currently about line 902), change `if (!isIOS) return;` to `if (!isIOS && !isAndroid) return;`. `isAndroid` is already imported. Do not change the other `if (!isIOS) return;` near line 812. Until this lands, the Android pieces are inert.
- **H2 — make ANDROID-5 visible (`mobile_service.rs`).** In the two `Err(error)` arms I added, before `return Err(error)`: `if error.contains("ACTIVITY_RECREATED") { self.note_startup_warning(crate::user_error::plain(&error)); }`. The reloaded shell already renders `startupWarnings`; I have not checked that it does so on mobile.
- **H3 — event-borne errors (`mobile.rs: error()`).** `kiwix-mobile-error` carries raw native text and never passes through `user_error::plain`. Suggested: emit `crate::user_error::plain(&message)` and keep the raw text in the `eprintln!`.
- **H4 — device-scan code contract (ANDROID-12).** All three parts must land together.
  1. Kotlin adds `limitationCodes: [{code, params}]` beside `limitations`, and `problemCode` beside `problem`, keeping the English strings as fallback.
     - Limitation codes: `ANDROID13_FOLDER_ACCESS`, `NO_FOLDER_ACCESS`, `CANDIDATE_CAP {count}`, `SHARED_STORAGE_DENIED`, `MEDIA_SEARCH_FAILED`, `STORAGE_SEARCH_FAILED`, `FOLDER_ACCESS_EXPIRED {provider}`, `FOLDER_SCAN_FAILED {folder}` (drop raw exception text from the two `*_FAILED`).
     - Problem codes: `NOT_READABLE_IN_PLACE`, `SPLIT_INCOMPLETE`.
  2. `device_scan.rs` adds `#[serde(default)]` fields to `NativeScan`/`Candidate`, validates codes as `[A-Z0-9_]{1,64}` with bounded params, and passes them through.
  3. `Library.tsx` maps codes through `t()` and falls back to the string.
- **H5 — Back button (ANDROID-3).** The plugin has no `moveTaskToBack`; the only native exits are Tauri's `exit` and `finishActivity(generation)`, both of which finish the Activity. Once a `back-button` listener is registered, the default dispatcher fallback no longer runs, so matching the Android 12+ root behaviour (background, keep the Activity) needs a new `moveTaskToBack(true)` command. Not added. If the root case finishes the Activity instead, ANDROID-2 already stops a Recents relaunch from replaying the old intent.
- **H6 — frontend contract after ANDROID-4.** After `plain`, an Android native failure is one of: an English sentence; an unchanged `[NEARBY_|BLE_|JOIN_|HOTSPOT_|LOCAL_NETWORK_…]` rejection; or a bare code of those families when it arrived Java-wrapped. `[CODE] - {json}` and `java…Exception: …` no longer reach the shell. Unknown non-family codes become `TRY_AGAIN` in Rust, so the shell's localized "That action did not finish. Try again. ({code})" fallback is not reached for them. If you prefer that fallback, change `android_plain`'s last line to return the bare code.
- **H7 — optional.** Make `library_files::remove_empty_import_directory` `pub(crate)` and call it in place of my `std::fs::remove_dir(&root)`.

## Doubts

1. **Checkpoint `08f3094`** — see the top.
2. **Lost new intent on a restored task (existing, adjacent to ANDROID-2).** After process death, a new "Open with" or notification tap reaches the restored Activity through `onNewIntent` just before its first `onResume`, most likely before the plugin is loaded, and `PluginManager.onNewIntent` then has nobody to deliver to. My change does not cause or worsen this, but with the stale launch intent now ignored it is the remaining way an intent goes missing. A small buffer in the companion, drained in `load()`, would close it. I did not add untestable lifecycle code beyond the finding.
3. **Unanswered transient "Open with" across process death.** The old replay accidentally re-offered a file the person had not yet answered. Now they get the designed "Re-open the file to import it." notice. I think that is the intended trade.
4. **`layoutDirection`** — my reasoning is from memory of platform and AppCompat source, not checked here.
5. **Rust-prefixed native errors are not normalised.** Examples: "Creating the Wi-Fi hotspot failed: [HOTSPOT_…] - …" and "The provider has not authorized moving this archive: java.lang.IllegalStateException: {…}". Only whole-message shapes are handled.
6. **iOS side effects of the Rust change.** Upper-case bracketed speech codes on iOS (`[SPEECH_X] - SPEECH_X`) now map to their sentences, and unknown ones become `TRY_AGAIN`. Separately, the existing iOS lower-case table looks unreachable in production, because iOS rejections also arrive bracketed; I left it alone since `provider_error` carries a localized system description that the generic sentence would hide.
7. **`is_java_throwable` breadth.** Any whole message starting `lower.dotted.NameException: ` or `…Error: ` is treated as native and replaced by `TRY_AGAIN`. Rust paths use `::`, so I expect no false positives.
8. **`replaced = !isFinishing`** also covers the system reclaiming a background Activity in a live process. I believe that is correct but it is a judgement.
9. **Transfer start timing.** The 4 s start timeout now also spans one hop to `worker`.