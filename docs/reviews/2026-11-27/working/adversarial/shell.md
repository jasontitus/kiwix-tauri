## Verdict for lane shell: only non-blocking findings

I found nothing Blocking. Every fix does what it claims, with two caveats: SHELL-7 is partial (the implementer says so), and SHELL-2 meets its "one owner per error" rule only for the App's own surfaces. Several components still paint the same failure inline. All tests pass: the lane's node tests, the whole node suite apart from two known regeneration gates, `tsc`, and the browser tests in 3 plain runs plus 1 run under load.

### SHELL-R1 — The new alert stacks on top of dialogs that already show the same failure inline (two owners, two announcements)
Severity: Should-fix
Fix under review: SHELL-2
Evidence class: source-traced
Location: `src/App.tsx` `onError` (`covered` and `errorSurface`). The callers that also paint inline:
- `src/components/Library.tsx:530-531` and `:561`: `setRemovalError(errorText(e)); onError(e);`, shown as an `ErrorNotice` with role=alert in the removal and detail Modals (`:1550`, `:1568`).
- `src/components/Downloads.tsx:807-808`: `DownloadCleanupDialog`, whose inline `<p role="alert">` is at `DownloadCleanupDialog.tsx:31`.
- `src/components/Downloads.tsx:881-882` (`setMapError`).
- `src/components/Maps.tsx:745-746` (cleanup dialog).

Trigger: Library → archive detail → Remove → `remove_book` fails.
Causal path:
- The catch paints the error inline inside the open Modal, then calls `onError(e)`, which defaults to blocking with `showGlobally = true`.
- `openDialogs(document)` finds the removal Modal, so `surface === "dialog"`, and a modal alertdialog now opens on top with the same text.
- Result: the message is painted twice and announced twice (inline role=alert, then the alertdialog), and the person needs an extra Dismiss before they can act in the dialog that already explained the failure.
- At base the band copy was behind the backdrop and inert. SHELL-2 makes the existing double owner intrusive. The browser test checks the bookmark editor only, which has no inline error.

Smallest correct fix: at those call sites use the existing parameter, `onError(e, "blocking", false)`: the failure is recorded and owned inline. Add one regression in `test-shell-surfaces.mjs`: a failing `remove_book` gives exactly one painted copy.

### SHELL-R2 — The confirmation-gated download directory shows "Saving…" until the field loses focus; quitting without leaving the field drops it silently
Severity: Should-fix
Fix under review: SHELL-6
Evidence class: source-traced
Location: `src/components/SettingsAutosave.tsx` `SettingsSaveStatus` (`pending = … edit.phase === "waiting"`) together with `input().onChange` (no timer when `needsConfirmation`), and `SettingsPolicyConfirmation`'s `available` effect.
Trigger (desktop): type a new download directory, do not leave the field, then quit (Cmd/Ctrl+Q from the menu) or close the window.
Causal path:
- The edit stays `waiting` with no timer, so the status reads "Saving…" indefinitely, although nothing is being saved.
- On quit, `flushSession` → `flushPendingPreferences` → `flushDrafts` → `commit` sets `phase = "confirming"`.
- `quitting` makes `available` false, and the effect does `edits.delete(key)`. The value the person typed is discarded without a prompt.
- At base the same loss window was 600 ms. Now it lasts until blur.

Smallest correct fix: in `SettingsSaveStatus`, count a `waiting` edit for which `needsConfirmation` is true as `confirm`, not as pending. For example: "Leave the field or press Enter to review this change." That way the status never claims a save is in progress.

### SHELL-R3 — A recoverable diagnostic replaces an open blocking alert
Severity: Nit
Fix under review: SHELL-2
Evidence class: source-traced
Location: `src/App.tsx` `onError`. `errorSurface("recoverable", covered)` → `"band"` → `setError({… surface: "band"})`.
Trigger: technical details on, a blocking failure is shown in the alert over an editor, then a recoverable diagnostic arrives (for example from the Android Back registration, or `mobile_apply_locale`).
Causal path: the alert unmounts and is replaced by a band under the still-open editor backdrop. That band is inert and cannot be dismissed, and the blocking message is gone.
Smallest correct fix: use a functional `setError` that keeps a current `surface === "dialog"` entry when the incoming severity is recoverable.

### SHELL-R4 — Focus falls to `<body>` when the alert's origin is gone or still disabled, even with a modal still open
Severity: Nit
Fix under review: SHELL-2
Evidence class: source-traced
Location: `src/App.tsx` focus-return effect: `if (!origin?.isConnected || origin.matches(":disabled") || …) return;`
Trigger: the control that failed is removed, or is still disabled, when Dismiss is pressed.
Causal path: the Modal cleanup's `previous` is `<body>` or disconnected, and the effect returns early, so focus stays on `<body>` while another modal is open.
Smallest correct fix: fall back to the top-most `dialog[open]` (its first enabled control) when `openDialogs(document).length > 0`.

### SHELL-R5 — Two code shapes still reach the user raw
Severity: Nit
Fix under review: NET-3
Evidence class: reproduced in a scratch harness that uses the real `UI.tsx` parts (`adv-shell/errortext.mjs`)
Location: `src/components/UI.tsx` `nativeCodeText`: `else if (prefix) return null;` and `if (detail && detail !== code && !nativeCodeOnly.test(detail)) return null;`
Trigger and observed output:
- `Creating the Wi-Fi hotspot failed: [ACTIVITY_DESTROYED] - ACTIVITY_DESTROYED` is returned unchanged. It is produced by `KiwixMobilePlugin.kt:533` through `mobile_service.rs:2164`, but is probably seen only if the document outlives the Activity.
- `[NEARBY_FUTURE] - {"code":…}` (an unmapped shell-owned code inside the JSON envelope) is returned with the raw JSON. Kotlin's Nearby code rejects with `[CODE] - CODE` today, so this is latent.

Smallest correct fix: after a prefix, treat a self-identical `[CODE] - CODE` of any family as a code. Treat a `{…}` detail as no detail.

### SHELL-R6 — On iOS the shell's `*_RETIRED` mappings never run
Severity: Nit (pre-existing; combined behaviour with `user_error.rs`)
Fix under review: NET-3
Evidence class: source-traced
Location:
- `src-tauri/src/user_error.rs` `plain` → `has_internal_terms` (the "retired" term).
- iOS `NearbyBluetooth.swift:73,88,462`: `invoke.reject("BLE_OWNER_RETIRED")` and `"BLE_CONNECTION_RETIRED"`.

Causal path:
- A bare iOS code is not an Android failure, so `plain` reaches `has_internal_terms("ble_owner_retired")` → `TRY_AGAIN`.
- The shell's "Nearby sharing is no longer active. Start it again." is never shown for these codes on iOS. The NET-3 test checks only `errorText`, not `plain` followed by `errorText`.

The user still gets an actionable generic sentence. Optional fix: let `plain` pass shell-owned bare families (`shell_owned_code`) through before the internal-terms check.

### Per-fix assessment
- **SHELL-1 (and the ACQ-5 / FEAT-5 duplicates):** fixes it.
  - The four producers are the only Nearby messages that reach `VERIFICATION_WORDS`. A grep of the current `localswarm/` and `nearby*.rs`, including the networking lane's live edits, found no new unclassified producer. The scan test enforces this.
  - "Published LocalSwarm bytes no longer match their verified manifest" (`mod.rs:1616`) and "Manifest content address, totals or hashes are inconsistent" (`protocol.rs:182`) fall outside the scan words. Neither was mapped at base either.
  - Kotlin and Swift have no checksum/verification literals.
- **NET-3:** fixes it, but see R5 and R6.
  - Adversarial inputs pass through unchanged: ZIM, HTTP, PIN, URL, SHA-256, IPv4, file names, multi-line messages, `[x] - y`, `Download failed: HTTP_404`, the empty string, and output fed through a second time.
  - A bare whole-message SCREAMING_CASE token such as `MY_FILE` becomes the generic sentence. No real producer of such a token was found, and no TS consumer compares raw codes after `errorText` (only `SPEECH_TEXT_LIMIT` and `PROVIDER_*`, both handled earlier).
  - Combined with `user_error::plain`, nothing is mapped twice: Android `[NEARBY_*] - CODE` reaches the shell unchanged; `[sentence] - HOTSPOT_*` is left alone by Rust and mapped by the shell.
- **SHELL-2:** fixes it, but see R1, R3 and R4.
  - Checked and correct: nested dialogs; a dialog closing itself under the alert; a burst of errors (one alert, first origin kept); StrictMode; Escape and Enter reach only the alert.
  - Nothing becomes impossible to dismiss: the alert has Dismiss, ×, Escape and the backdrop. It is hidden only while quitting or suspended, and the recovery dialog renders `error.message` then.
  - `elementFromPoint` fails safe: a null result or an iframe falls back to document order, or to not moving focus.
  - I reproduced in Chromium 158 that Escape on an alert opened 6 s after the last user activation cancels only the alert, not the dialog under it.
- **SHELL-4 / ANDROID-3 / H5:** fixes it, with the documented residuals.
  - The API is used correctly. `unregister()` returns `invoke("plugin:app|remove_listener", {event, channelId})`, which is the same call `forget` makes. `Plugin.removeListener` always resolves and matches only `back-button` channels. Channel ids are random u32 values from `transformCallback`, so a stale or forged stored value can at worst remove this shell's own Back listener, which puts the platform default back. That would need same-origin script.
  - Without `localStorage`, a stale listener left by a recreated Activity cannot be cleared, so Back is a dead key until the process dies. This is documented and unlikely with DOM storage enabled.
  - The registration coalesces correctly, so rapid open/close leaks no listener.
  - The "swallowed once" press is the `leave` branch's re-read. Acceptable.
  - The shell never calls exit. A busy dialog's own cancel policy is kept, because it is the same path as Escape. The MutationObserver is Android-only and is disconnected (with its rAF cancelled) on unmount.
  - ACL: `core:default` includes `allow-register-listener` and `allow-remove-listener`.
- **APPLE-7:** fixes it.
  - A synthetic Escape is never sent into an archive frame: an iframe target is swapped for `<body>`.
  - `Reader.shortcutKeyboard` returns on `!event.isTrusted`. The Reader handlers that do accept untrusted Escape (history menu `:676`, workspace `:5810`, preview `:6939`) only close their own surface. The same applies to the UI picker, Library suggestions and SwipeRow.
- **SHELL-6:** fixes the premature prompt. Blur, form submit (`flush`), unmount (`flushDrafts`), `visibilitychange`, `pagehide` and `flushPendingPreferences` all still commit. The status text and quit-time loss are in R2.
- **SHELL-7:** fixes it in part (acknowledged).
  - Every caller of `confirmDiskSpaceWarning` (Maps `:587`, Downloads `:568`, `:855`) still gets a boolean. `registerDiskSpaceConfirmation` has one registrant (App). No boolean is left treating the new string as truthy.
  - `UserCancelledError` is thrown only for `"declined"`. That comes only from the Modal's `onClose` (Cancel, ×, a cancel event, a backdrop click) or Cancel, all of which are user gestures. Android Back and iOS Escape count as user gestures here too.
  - Remaining: `App.onError` still logs the decline through `reportError` before its type check. Catalog, Maps and Downloads still paint "Download cancelled." (handoffs 1–2 are not yet applied; `diagnostics.ts` is unchanged).
- **SHELL-9:** fixes it.
- **SHELL-10:** fixes it.
- **SHELL-11:** fixes it.
  - During delivery `finished = true` blocks Cancel and Escape. After a rejection the session is either gone (cancel → `Err` → `window.close()`) or Rust has already closed the window. So the fallback cannot run while a photo is still being delivered.
  - Unverified: whether `window.close()` actually closes a Tauri top-level window on WebView2. This fallback existed at base.
- **H1 (ANDROID-8):** fixes it. `mobile_dynamic_type_state` is now routed on Android to the Kotlin `dynamicTypeState` command.
- **Scope:** no creep beyond the H1 handoff. I ignored the `nativeMenuLabels` hunk as instructed.

### Implementer's doubts
- **SHELL-1 classification:** acceptable. The "local" messages are about this device. Friendlier copy is an owner decision, not a reason to use the sender sentence.
- **NET-3 mapping judgement calls:** acceptable as they are.
- **Remote sender codes:** a real copy issue. "…join the temporary Wi-Fi network: Allow Wi-Fi and location access…" sends the person to the wrong device. Owner decision; low.
- **Raw code in parentheses:** acceptable.
- **Back closing reader chrome:** acceptable. Side effect: `dismissReaderChrome` also clears a docked panel when Back closes the find bar. Nit.
- **elementFromPoint:** fails safe in code. WKWebView is unverified.
- **Recoverable diagnostics staying in the band:** acceptable in itself, but see R3.
- **SHELL-6 "Saving…":** real, fix now (R2).
- **SHELL-10 residual:** acceptable.
- **SHELL-11 console-only raw text:** acceptable.

### What I ran
- `node --test` on the five node test files: error-visibility 20/20, shell-dismiss 11/11, settings-surface 4/4, updates-settings 5/5, camera-window 4/4. All extract or run the real shipped code (AST extraction of `onError`, `errorText`, `installNow` and the effects; `SettingsAutosave` and `camera.ts` transpiled whole).
- `node --test scripts/test-*.mjs`: `tests 499, pass 486, fail 2, skipped 11`. The failures are the known regeneration gates `docs/ERRORS.md matches…` and `every UI string … localization manifest`.
- `npx --no-install tsc --noEmit`: exit 0.
- `KIWIX_TEST_CHROME=… node --test scripts/test-shell-surfaces.mjs scripts/test-frontend-surfaces.mjs`, three times: `tests 6, pass 6, fail 0` each (about 31–34 s). A fourth run while the full node suite and a second shell-surfaces run were going: 6/6, and 5/5 for the second run.
- Scratch harnesses in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-shell/`: `errortext.mjs` (adversarial `errorText` cases) and `closewatcher.mjs` (Chromium 158 nested-dialog Escape check).

### What I could not verify
- Real Android: the ACL and Kotlin bridge, Activity recreation and stale-listener recovery, `localStorage` persistence, Back at the root walking WebView history.
- iOS: UIKit key commands, WKWebView `elementFromPoint` and focus restore.
- Windows: the WebView2 `window.close()` fallback.
- TalkBack and VoiceOver announcements.
- A real updater download.
- I ran no Rust tests. The `user_error` behaviour above is traced from source.