# Lane `shell` — report

All ten lane findings plus the two Android handoffs (H1, H5) are addressed in owned files; SHELL-7 is partial pending a two-line change in a file I do not own. `tsc` is clean, the lane's 50 node tests and all 6 real-browser tests pass. Nothing here ran on a device: Android Back and iOS Escape were exercised only against a simulated Tauri bridge in headless Chromium.

To see my full change: `git diff d98cac2 -- <file>`.

---

### SHELL-1 (= ACQ-5, FEAT-5) — `errorText` rewrote every "checksum/verification" message into the sender sentence
Disposition: fixed
Verification of the finding: ran the base `errorText` against real messages (`scratchpad/shell/base-errortext.mjs`). "Verification was interrupted by application shutdown; resume when ready", "Downloaded archive failed its advertised SHA-256 checksum; …" and "Archive verification queue is full" all came back as the sender sentence.
Change: `src/components/UI.tsx` — the unscoped regex is replaced by `senderVerificationFailure`, an explicit list of the four genuine producers:
- "LocalSwarm chunk checksum mismatch" (`localswarm/network.rs` ×3)
- "LocalSwarm source changed or returned corrupt bytes" (`network.rs`)
- "Verified LocalSwarm source changed" (`localswarm/mod.rs`)
- "LocalSwarm source parts must exactly cover the ZIM checksum boundary" (`mod.rs`)

I classified eight other Nearby messages as local, so they now pass through unchanged. This is narrower than the reviewer's suggested regex; see Doubts.
Tests (`scripts/test-error-visibility.mjs`):
- "only Nearby sender verification failures become the sender sentence" — table with every ACQ-5/FEAT-5 message.
- "the sender-verification list covers every Nearby producer, and no other Rust message is rewritten" — scans shipped Rust; a new Nearby message with those words fails until classified, and every non-Nearby literal must pass through.
- Observed failing: yes, by restoring the old regex in a scratch mirror (both fail).
Not exercised: nothing.

### NET-3 — native error codes arrive wrapped or prefixed; about 90 unmapped
Disposition: fixed
Verification of the finding: base `errorText` returns `[NEARBY_JOIN_TIMEOUT] - NEARBY_JOIN_TIMEOUT`, `Creating the Wi-Fi hotspot failed: HOTSPOT_UNSUPPORTED`, `BLE_POWERED_OFF` and `MOBILE_LOCAL_NETWORK_BLOCKED` unchanged. 94 of 147 produced Nearby-family codes were unmapped.
Change: `UI.tsx` — new `nativeCodeText` runs before the other mappings.
- Forms recognised: bare; `[CODE]`; `[CODE] - CODE`; `[CODE] - sentence`; `[sentence] - CODE`; and any of those after one single-line `Sentence:` prefix (the prefix is kept).
- New mappings for Bluetooth off / denied / waiting / unsupported, local network lost or blocked, tethering disallowed, join lost / cancelled / timed out, plus iOS/Android twins of codes already mapped.
- Any other `NEARBY_/BLE_/JOIN_/HOTSPOT_/LOCAL_NETWORK_` code becomes "Nearby could not connect. Try again. (CODE)".
- A whole-message code of any other family becomes "That action did not finish. Try again. (CODE)".
- Left alone: multi-line messages, sentences with an upper-case word, an unknown-family token after a sentence, and an unmapped code that already has a native sentence beside it.
- Output is stable when formatted twice (`ErrorNotice` re-applies `errorText`).

Tests (`test-error-visibility.mjs`):
- "no native status code produced by Kotlin, Swift or Rust is ever shown as the message" — collects literals from the Kotlin, Swift and Rust sources (with minimum-count guards) and checks bare, wrapped, code-only, padded and two prefixed forms; every code-shaped literal of any family is checked bare and wrapped.
- "native code families map to their instruction in every form they arrive in".
- "a sentence is never mistaken for a native code".
- Observed failing: yes, by mutation in the mirror (first two fail).
Not exercised: the real wire format on a device.

### SHELL-2 — failures inside a modal went only to the band underneath
Disposition: fixed
Verification of the finding: source-traced, then observed in the browser: on base `App.tsx` the new test fails waiting for any top-layer message.
Change:
- `src/error-visibility.ts` — `errorSurface(severity, coveredByDialog)` returns `"band" | "dialog"`.
- `src/App.tsx` `onError` — a blocking failure reported while a `dialog[open]` exists (other than the shell's quit and mobile-recovery dialogs) is owned by a nested `role="alertdialog"` modal. The band renders only `surface === "band"`, so there is exactly one owner.
- `UI.tsx` `Modal` — optional `role` prop.
- Focus: moves to Dismiss. On dismissal it returns to the originating control, including the case where the button disabled itself and had already lost focus. I added that after the browser test showed focus landing on `<body>`.
- A newer failure replaces the message in the same alert. Recoverable diagnostics stay in the band.
- No CSS added (reuses `.modal-confirmation` and `.confirm-message`).

Tests:
- `scripts/test-shell-surfaces.mjs` "a failure inside a modal is shown, announced and dismissed above it, with one owner" — real Chromium, dev bundle, StrictMode. It checks: one alertdialog, top-most, text painted once, no band, no nested live region, Chrome's accessibility tree exposes role/name/description, editor draft intact, Enter and Escape dismiss only the alert, focus returns to "Save changes", retry succeeds, and a three-deep stack (detail → confirmation → alert).
- `test-error-visibility.mjs` "the App reporter owns one surface per failure and paints no cancellation" and "the error surface is the band unless …".
- Observed failing: yes, on base `App.tsx` in the mirror.
Not exercised: real VoiceOver/TalkBack speech; WKWebView focus restoration (the fallback only acts when focus is left on `<body>`).

### SHELL-4 (+ H5) — Android system Back
Disposition: fixed differently — the listener is held only while there is something to dismiss.
Verification of the finding: no `onBackButtonPress` anywhere in `src/` at base. I read the vendored `AppPlugin.kt` and confirm the Android lane's H5 reading:
- No listener: `canGoBack()` → `goBack()`, otherwise the system's `onBackPressed()`.
- With a listener: the press is only delivered to the page. There is no hand-back call.
- The one exit command is `finishAffinity()`. Tauri's JS `exit` also runs the Rust quit pipeline, and `core:app:allow-exit` is not granted.
- Capability: `core:default` includes `core:app:default`, which grants `allow-register-listener` and `allow-remove-listener` (checked in `gen/schemas/acl-manifests.json`; `capabilities/main.json` has no platform restriction). No capability edit is needed.

Change:
- `src/shell-dismiss.ts` (new) — pure `backTarget` / `escapeTarget` and the DOM selectors. Order: dialog → tab sheet → popover → search → reader history → library → leave.
- `src/android-back.ts` (new) — `createBackRegistration`: serialised, coalescing register/unregister. It records the live channel id in `localStorage` and removes a listener left by a previous document. Without that, a stale channel would make Back a dead key, because the plugin keeps listeners for the process lifetime while the WebView is replaced per Activity.
- `App.tsx` — Android only. A `MutationObserver` (once per frame) tracks open dialogs and pickers. The listener is wanted when a surface is open, the tab sheet or search is open, or the pane is not the library, and never while quitting or suspended.
- The handler closes a dialog through its own `cancel` event (busy and consent policy kept), gives a picker a synthetic Escape, calls the reader's own "escape"/"back", or navigates to the library and focuses `#main-content`.

**What happens at the root:** no listener is registered and the shell calls nothing. Back is exactly the plugin default, as before this change.

Tests:
- `scripts/test-shell-dismiss.mjs` (11 tests) — order tables; registration under effect replay, bursts, late resolution, stale recovery, failed register/remove, no or broken storage; source checks that it is Android-only and never calls exit. Writing these found and fixed two real bugs in my first version (a leaked listener, and `""` parsed as channel 0).
- `test-shell-surfaces.mjs` "Android Back closes the top-most surface, returns to the library, and only then leaves to the system" — Android UA, phone viewport, simulated bridge using the real `onBackButtonPress` and `Channel` code.
- Observed failing: yes, on base `App.tsx`.

Not exercised — a device test must confirm:
1. The listener registers and unregisters through the real ACL and Kotlin bridge.
2. Back at the library root reaches the system. The plugin default first walks any WebView session history (`canGoBack`), so after reading articles the root may still take several presses before the task backgrounds. That is pre-existing; fixing it needs a native hand-back, which I was told not to add.
3. After the Activity is destroyed and recreated in a live process, Back still works (the stale-listener path).
4. Timing: a Back within roughly a frame plus an IPC round trip of opening a surface gets the default; within that window after the last surface closes it is swallowed once.
5. Back during a busy dialog does nothing.

### APPLE-7 — iOS hardware Escape dropped outside the reader
Disposition: fixed
Verification of the finding: source-traced; on base, the picker and tab sheet stay open in the browser test.
Change: `App.tsx` `kiwix-mobile-shortcut` listener. Escape now goes dialog → tab sheet (wherever focus is) → a synthetic cancelable Escape `keydown` on the focused shell element → the reader only if nothing used it. An archive frame is never targeted (`<body>` is used instead), and the Reader's own window key handler already ignores untrusted events.
Tests: `test-shell-surfaces.mjs` "a hardware Escape consumed by the native layer closes shell popovers outside the reader" and "the native Escape closes the phone's tab sheet wherever focus is" (iPhone UA, simulated bridge); `test-shell-dismiss.mjs` escape table. Observed failing on base: yes.
Not exercised: real UIKit key commands; the reader's history menu and library suggestion list (same mechanism, not driven in the browser).

### SHELL-6 — download-directory prompt opened 600 ms after the last keystroke
Disposition: fixed
Verification of the finding: on base the debounce is armed for `downloadDirectory` (test fails there).
Change: `src/components/SettingsAutosave.tsx` `input().onChange` — no debounce when `needsConfirmation(key, edit)`. Blur, form submit and the existing lifecycle flushes still commit and ask.
Tests: `scripts/test-settings-surface.mjs` — "a pause while typing a download directory neither saves nor opens the confirmation" (observed failing on base), plus two guards (submit commits; other fields still autosave).
Not exercised: not driven in the real browser (the field is desktop-native only).

### SHELL-7 — user cancellations painted as failures
Disposition: partial
Verification of the finding: confirmed in `UpdatesSettings.installNow`, `api.request` and `Downloads.start`.
Change:
- `error-visibility.ts` — `UserCancelledError` / `isUserCancelledError` next to `SuspendedError`.
- (a) `src/components/UpdatesSettings.tsx` — a rejected install is not reported when this user's Cancel was confirmed by `update_cancel` returning `true` (awaited, so reply order does not matter). No sentence matching. Any other stop is still reported.
- (b) `src/api.ts` + `App.tsx` — the low-storage prompt now returns `"accepted" | "declined" | "unavailable"`. Only `declined` (Cancel, ×, Escape, backdrop) throws `UserCancelledError`. A prompt that could not be shown, or was withdrawn by the lifecycle, stays an ordinary error. `App.onError` paints nothing for the type, so the band doubling is gone.

What remains: the declined prompt is still shown once, inline, by the owners that call `errorText` (Downloads "Download cancelled.", Catalog/Maps notices). I kept `errorText` returning the sentence because the batch flows in `AdvancedSettings` and `LocalSwarmControls` list per-item outcomes as "Could not add “X”: {error}", and an empty string there would be a regression in files I do not own. See Handoffs 1–2.
Tests: `scripts/test-updates-settings.mjs` (cancel accepted; rejection before cancel reply; three "still a failure" cases; stale cancel). `test-error-visibility.mjs`: "a cancellation the user chose is a type of its own…", "only the user's own answer to the low-storage prompt is a cancellation", and the App reporter test. Observed failing on base: yes.
Not exercised: a real updater download.

### SHELL-9 — mobile Settings/Help described windows, a File menu and New window
Disposition: fixed
Verification of the finding: both rendered unconditionally; `restorePreviousSession` is desktop-only.
Change: `src/components/Settings.tsx` — the paragraph and the New-window row are gated on `isDesktop`; other platforms get a variant without windows or the File menu.
Tests: `test-settings-surface.mjs` source-level gating check; `test-shell-surfaces.mjs` "desktop-only Settings copy is not shown where there are no windows or File menu". Both observed failing on base.
Not exercised: the desktop branch is not rendered in a browser (the preview is not `isDesktop`); the source check covers it.

### SHELL-10 — `UpdatesSettings` listener leak; `installing` never cleared
Disposition: fixed
Verification of the finding: both confirmed in source and by tests failing on base.
Change: `UpdatesSettings.tsx` — `keep()` unlistens immediately if cleanup already ran; `setInstalling(false)` after a resolved install.
Tests: `test-updates-settings.mjs` "update listeners registered after cleanup are removed, not leaked" and "a completed install re-enables the button when the app keeps running". Observed failing on base: yes.
Not exercised: a real install.

### SHELL-11 — camera window dead-end; locale failure blocked start
Disposition: fixed
Verification of the finding: confirmed in `src/camera.ts` and `photo_capture.rs` (the early returns leave the window open).
Change: `camera.ts`
- Capture rejection: resets `finished`, shows a localized message, focuses Cancel. Cancel/Escape then send `camera_capture_cancel` and fall back to `window.close()`.
- `setLocale` is wrapped (an unsupported locale threw at module load) and a `whenLocaleReady` rejection no longer skips `start()`.
- `camera.html` is unchanged.

Tests: `scripts/test-camera-window.mjs` (4 tests; 3 observed failing on base, 1 guard).
Not exercised: a real camera or WebView2.

### H1 (ANDROID-8) — system text scale effect on Android
Disposition: fixed
Change: `App.tsx` dynamic-type effect now starts with `if (!isIOS && !isAndroid) return;`. The other `if (!isIOS) return;` is untouched. The initial `api("mobile_dynamic_type_state")` fetch is in the same effect, so it runs on Android.
Tests: the Android browser test asserts `data-content-size-category="android"`, `data-large-text="true"` and `--system-text-scale: 1` for a simulated scale of 1.5. Observed failing with the line reverted in the mirror.
Not exercised: device.

---

## Commands run (final results)
- `npx --no-install tsc --noEmit` → exit 0.
- `node --test scripts/test-*.mjs` (no browser) → `tests 496, pass 483, fail 2, skipped 11`. The two failures are other lanes' gates waiting on the end-of-run regeneration: `docs/ERRORS.md matches…` and `every UI string … is collected in the localization manifest`.
- `node --test` on my five node files plus `test-provider-recovery.mjs` → `tests 50, pass 50, fail 0`.
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-shell-surfaces.mjs scripts/test-frontend-surfaces.mjs` → `tests 6, pass 6, fail 0`. `test-shell-surfaces.mjs` was run three more times (5/5 each).
- Not run: `test-startup-navigation.mjs` in a browser (needs `npm run build`), `cargo`, `npm run build`.

## Files changed
- Edited: `src/App.tsx`, `src/components/UI.tsx`, `src/error-visibility.ts`, `src/api.ts`, `src/components/Settings.tsx`, `src/components/SettingsAutosave.tsx`, `src/components/UpdatesSettings.tsx`, `src/camera.ts`, `scripts/test-error-visibility.mjs`.
- New: `src/shell-dismiss.ts`, `src/android-back.ts`, `scripts/test-shell-dismiss.mjs`, `scripts/test-shell-surfaces.mjs`, `scripts/test-settings-surface.mjs`, `scripts/test-updates-settings.mjs`, `scripts/test-camera-window.mjs`.
- Not touched: `scripts/test-frontend-surfaces.mjs` (I used a new file because other lanes edit that one), `camera.html`, CSS, `nativeMenuLabels`.

## New user-visible strings (for localization generate)
1. `Bluetooth is off. Turn on Bluetooth, then try again.`
2. `Allow Bluetooth access for Kiwix in the device settings, then try again.`
3. `Bluetooth access is still waiting for approval. Try again.`
4. `The local network connection was lost or is not available. Connect both devices to the same Wi-Fi, then try again.`
5. `Nearby could not connect. Try again. ({code})`
6. `That action did not finish. Try again. ({code})`
7. `Reader tabs and reading positions reopen at launch. Private and read-only profiles are never restored.`
8. `The photo could not be sent. Cancel and choose a file instead.`

Reused for the alert dialog: `The operation could not be completed.` and `Dismiss`.

## Handoffs / contract changes
1. **`src/diagnostics.ts` (not owned), to finish SHELL-7:** in `reportError`, change `if (isSuspendedError(error)) return false;` to `if (isSuspendedError(error) || isUserCancelledError(error)) return false;` and import `isUserCancelledError`. That stops Catalog and Maps painting or recording a declined prompt. Suggested test line: `assert.equal(reportError(log, new UserCancelledError("x"), "blocking", true), false)`.
2. **`Downloads.tsx` (F2 lane):**
   - `start()` — `if (!isUserCancelledError(e)) setStartError(errorText(e));` and drop the adjacent `onError(e)`, which is a real double owner for genuine errors.
   - Same double owner near line 880: `setMapError(errorText(value)); onError(value);`.
   - Batch flows may want their own wording for a declined prompt.
3. **API contract:** `registerDiskSpaceConfirmation` handlers now resolve `DiskSpaceDecision`, not `boolean`. `confirmDiskSpaceWarning()` still returns `boolean`. `App.onError` adds `surface` to the error state.
4. **Tests that extract `App.onError`:** it now references `openDialogs`, `errorSurface`, `quitDialog`, `mobileRecoveryDialog`, `HTMLElement`, `errorOrigin` and `lastFocus`, but only on the painted path. `test-provider-recovery.mjs` passes unmodified.
5. **Kotlin lane (information):** `LocalHotspot.kt` calls `invoke.reject("HOTSPOT_…", "sentence")`, which puts the code in the message and the sentence in the code, so Rust sees `[sentence] - CODE`. `errorText` now tolerates both orders.
6. **Docs owner:** `PLATFORM-GAPS.md` / `RELIABILITY.md` should record the Back behaviour and the residual at the root; `docs/ERRORS.md` and the locale inventory need regenerating.

## Doubts
- **SHELL-1 classification.** These pass through although the reviewer's regex would have mapped them: "LocalSwarm staged bytes changed after chunk verification", "LocalSwarm chunk verification stopped", "LocalSwarm verification stopped: …", "Nearby publication changed during verification; retry the import". I judged them local (staged files or a worker on this device), but their raw text is jargon. If you would rather have a friendlier sentence, they need their own string, not the sender one.
- **Some NET-3 mappings are judgment calls** from reading the native call sites:
  - `NEARBY_JOIN_BUSY` / `NEARBY_HOTSPOT_BUSY` → "still starting"
  - `NEARBY_ACTIVITY_RETIRED` → "Keep Kiwix open … while connecting"
  - `NEARBY_ROUTE_*` → the local-network sentence
  - `BLE_RESET` / `BLE_UNAVAILABLE` → "Turn Bluetooth off and on"
- **Remote sender codes.** "The sender could not join…: JOIN_USER_DENIED" keeps the prefix and appends an instruction that is really for the other device.
- **Fallbacks show the raw code** in parentheses, on purpose, for support requests.
- **Back and reader chrome.** Back closes the find bar, preview, or an overlaid (`position: absolute`) side panel before navigating history. That step is not in the specified order; a docked tablet panel is deliberately ignored.
- **Top-most dialog detection** uses `document.elementFromPoint(0, 0)`, relying on the modal backdrop covering the viewport. It works in Chromium; untested in WKWebView.
- **Recoverable diagnostics** reported under a dialog stay in the covered band by design.
- **SHELL-6 side effect.** While a typed download directory waits for blur, `SettingsSaveStatus` still says "Saving…".
- **SHELL-10 residual.** After an install whose Quit is then cancelled, the panel still offers the update; pressing it again produces the updater's own error.
- **SHELL-11.** The raw rejection text is no longer shown in the camera window (it goes to `console.warn`).
