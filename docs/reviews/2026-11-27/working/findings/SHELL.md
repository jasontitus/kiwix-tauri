## SHELL review: React shell and non-reader frontend (base d98cac2, read-only)

Eleven findings, most severe first. No High-severity defect found; the privacy/session/lifecycle machinery in `api.ts` and `App.tsx` held up to tracing. The real defects are in error presentation, Android Back, and a few smaller surfaces.

Evidence I generated: a Node snippet reproducing the `errorText` regex against real Rust messages, a read-only run of `node scripts/localization.mjs audit`, and a field-name cross-check of `types.ts` and component-local interfaces against Rust/Kotlin/Swift (no drift beyond two dead optional fields). No headless-browser or device run.

---

### SHELL-1 — `errorText` rewrites any message containing "checksum/corrupt/integrity/verification/source changed" into "The sender's archive failed verification…"
Severity: Medium        Category: usability
Platforms: all
Evidence class: reproduced (regex against the real Rust strings) + source-traced (render sites)
Location: `src/components/UI.tsx:137-138` (`errorText`)  [+ `src/components/Downloads.tsx:1388-1395`, `src/App.tsx:799`, `src-tauri/src/acquisition/persistence.rs:249-251`, `src-tauri/src/integrity.rs:135-137`, `src-tauri/src/acquisition/finalize.rs:29`, `src-tauri/src/kokoro.rs:1071`, `src-tauri/src/library_root.rs:480`, `src-tauri/src/edition_updates.rs:573`]
Trigger: Quit while an ordinary catalog (HTTPS) download is verifying, relaunch, open Downloads. Other triggers: an HTTP download whose SHA-256 fails; starting an integrity check while one runs; a Kokoro model download failing verification; Android library-location copy failing verification; "Catalog source changed; select this edition again".
Causal path:
- Rust sets `stored.job.status = "paused"`, `interrupted = true`, `error = "Verification was interrupted by application shutdown; resume when ready"`.
- The `downloads` reply carries that string as `job.error` (not an `Err`, so `user_error::plain` never sees it).
- `Downloads.tsx:1392` renders `errorText(job.error)`.
- `UI.tsx:137`: `if (/checksum|corrupt|source changed|integrity|verification/i.test(message)) return t("The sender's archive failed verification. Stop sharing and start again, or download it again from the catalog.");`
- The user is told a "sender's archive" is corrupt and to "stop sharing", when the right action is Resume. No sender exists.
- The same substitution hits every `errorText` call (82 sites) and the global band. All nine sampled non-Nearby Rust messages matched the regex.
Smallest correct fix: scope the branch to Nearby/LocalSwarm text, e.g. `/\bLocalSwarm\b.*(checksum|corrupt|source changed|verification)|Nearby publication changed during verification/i`. Those are the only producers that mean "sender" (`localswarm/network.rs:316,566,653,1458`, `localswarm/mod.rs:369,1314,1714`, `localswarm/import.rs:149`). Everything else passes through unchanged. No contract change.
Regression test: `scripts/test-error-visibility.mjs` already extracts `errorText` from `UI.tsx` via the TS AST and runs it in `vm`. Add a table asserting the non-Nearby strings above come back verbatim and the LocalSwarm strings still map to the sender sentence.

### SHELL-2 — Failures of actions started inside a modal are reported only to the global band underneath it
Severity: Medium        Category: usability
Platforms: all; worst on iOS/Android phones, where panels are near-full-height bottom sheets
Evidence class: source-traced (inference only for "assistive tech does not announce inert live regions")
Location: `src/components/Library.tsx:577-578` (`shareNearby`), `:487-488` (`setPinned`), `:693-694` (`random`), `:1683` (reveal)  [+ `src/components/SelectedStorageActions.tsx:79-80` (`relocate`), `src/components/Settings.tsx:314-315` (`switchLibraryLocation`), `:382-383` (`confirmExternal`), `src/components/Collections.tsx:249-250` (`save`), `src/components/IntegrityPanel.tsx:223,240`; band at `src/App.tsx:3489-3506`; `src/components/UI.tsx:290` (`showModal`); `src/platform.css:55-65`]
Trigger: iPhone: Library → archive Info sheet → Share, with Local Network permission denied. Or Android: Settings → Library location → "Copy library here" while a download is active. Or Reading list → Edit note → Save changes when the write is refused.
Causal path:
- Each handler's `catch` calls only `onError(e)`; `App.onError` does `setError(...)`.
- The band renders as a normal-flow `<div className="global-error">` inside `.main-shell`.
- The modal is a `<dialog>` opened with `showModal()`: top layer, rest of document inert, `::backdrop` over the page. On phones it is a sheet up to `100dvh - safe-area - 24px`.
- The modal stays open (`setDetail(null)` / `setOpen(false)` / `setPendingLocation(null)` run only on success). The button just leaves its busy label.
- The `role="alert"` is covered or dimmed, cannot be dismissed while the dialog is open, and sits in an inert subtree.
- The existing test "genuine storage failure remains visible" only proves `setError` was called.
Smallest correct fix: one central change. In `App.onError`, when `document.querySelector("dialog[open]")` is true at report time, also present the message in the top layer. For example, reuse the `confirmAction` root in `UI.tsx` for a one-button alert dialog, or render the band inside a `popover="manual"` element shown while a dialog is open. Per-surface alternative: give these modals a local error slot, as Catalog's detail already has (`Catalog.tsx:772`).
Regression test: `scripts/test-frontend-surfaces.mjs`: make fake IPC reject `save_bookmark`, click "Save changes" in the edit dialog, assert `dialog[open]` (or a top-layer element) contains the failure text, not only `.global-error`.

### SHELL-3 — Catalog and Maps card-level failures are painted at the top of the scrolled page, off-screen
Severity: Medium        Category: usability
Platforms: all (phones: one-column lists are almost always scrolled)
Evidence class: source-traced
Location: `src/components/Catalog.tsx:343-347` (`reportActionError`), `:471` (`{!detail && actionNotice}`), `:665-677`  [+ `src/components/Maps.tsx:796`, `:497-499`, `:544-546`, `:629-631`, `:1104-1112`; `src/styles.css:436-441`, `:1112-1123`]
Trigger: Discover → scroll down → tap Download on a card when `start_download` fails (metered network, low disk declined, duplicate, bad URL). Same for Maps → "Get map" on a grid card, or Pause/Resume on the map job panel.
Causal path:
- `download()` catches and calls `reportActionError`, which sets `actionError`.
- The only render site outside the detail dialog is `{!detail && actionNotice}`, the first child of `<section className="page catalog-page">`, above the header.
- `.main-content { overflow: auto }` is the scroller; `.error-notice` is in normal flow (not sticky). Nothing calls `scrollIntoView` or `focus`.
- The button flips "Starting…" → "Download" and nothing visible changes. Success is obvious (`focusDownload` switches to Downloads); failure is silent for sighted users.
- Maps is identical: `{!openedRegion && startError && <ErrorNotice …/>}` at the page top, the grid sits below Saved maps and the job panel.
- The frontend-surfaces test covers only the in-dialog path.
Smallest correct fix: when the action started from a card (no detail dialog), make the notice reachable. Either attach a ref and call `scrollIntoView({ block: "nearest" })` + `focus()` in an effect keyed on the error, or route card-level failures to the always-visible global band via the existing `onError` prop. Keep the in-dialog notice as is.
Regression test: `scripts/test-frontend-surfaces.mjs`: fixture with enough catalog entries to scroll, scroll `#main-content` to the bottom, click the last card's Download with `start_download` rejecting, assert the alert's bounding rect intersects the viewport (or `.global-error` contains the text).

### SHELL-4 — Android system Back is never handled by the shell
Severity: Medium        Category: usability
Platforms: Android
Evidence class: source-traced (no handler exists; default path read in vendored Tauri/wry). Runtime corroborated by the repo's emulator note at `docs/RELIABILITY.md:271-273`; exact WebView history behaviour is inference.
Location: `src/App.tsx` (no `onBackButtonPress` anywhere in `src/`; nearest are the hardware-key handler at `:2827-2876` and the Escape-only sheet close at `:3231-3236`)  [+ `src/components/UI.tsx:309-312`; `src-tauri/gen/android/.../MainActivity.kt`; `~/.cargo/registry/src/*/tauri-2.12.5/mobile/android/src/main/java/app/tauri/AppPlugin.kt:27-45`; `src-tauri/plugins/tauri-plugin-kiwix-mobile/android/.../HardwareShortcutKeys.kt:43-68`]
Trigger: on Android, press or swipe Back with any modal sheet open, with the reader-tabs sheet open, with a facet popover open, or on Settings/Downloads/Discover.
Causal path:
- `TauriActivity` sets `handleBackNavigation = false`; `MainActivity` adds no callback.
- Tauri's `AppPlugin` owns Back: `if (!hasListener("back-button")) { if (webView.canGoBack()) webView.goBack() else activity.onBackPressed() }`.
- The frontend registers no `back-button` listener. `HardwareShortcutKeys.kt` maps only keyboard keys (Escape, Alt+arrows), not `KEYCODE_BACK`.
- So Back either walks WebView session history (reader iframe navigations; no effect on the shell) or leaves the app. It never closes the `<dialog>`, the tabs sheet, a `FacetPicker`, or returns to the previous pane.
- The repo's own emulator note: "Back at the article scrolled it to the top; three further Back presses backgrounded the task".
- `docs/audit/android-all-closed-*.md` marks upstream back-press fixes (#569, #673, #1052) "not-applicable … replaced by the Tauri shell", but the shell has no replacement. `docs/PLATFORM-GAPS.md` does not list it.
Smallest correct fix: in `App.tsx`, under `isAndroid`, register `onBackButtonPress` from `@tauri-apps/api/app` (already in `node_modules`; `core:default` grants `allow-register-listener`) and dismiss the top-most transient surface in order:
- open `dialog[open]`: dispatch the same cancelable `cancel` event the `"escape"` branch at `App.tsx:2848-2856` uses
- `mobileTabsOpen` → `dismissMobileTabs()`
- `globalSearch` → close
- reader pane → `mobileReaderShortcut.current?.("back")`
- non-library pane → `navigate("library")`
- otherwise leave the app (which call backgrounds the task is unverified here: the plugin's `exit` runs `finishAffinity()`).
Needs an emulator pass; this is not a one-liner.
Regression test: extract the ordering into a pure `backTarget(state)` helper with a `scripts/test-*.mjs` table test; add one emulator acceptance line (Back closes a sheet first).

### SHELL-5 — "Share as BitTorrent" reports failure after 180 s while the torrent is still being created, then refuses the retry it suggests
Severity: Medium        Category: reliability
Platforms: desktop and mobile; any archive whose piece hashing exceeds ~3 minutes (large ZIMs on HDD/SATA or phone storage)
Evidence class: source-traced
Location: `src/components/Library.tsx:599-613` (`shareBook`), `:1722-1727`  [+ `src-tauri/src/torrent/manager.rs:438-448`, `:505-515`]
Trigger: Library → archive details → "Share as BitTorrent" on a large archive.
Causal path:
- Rust `share()` returns a job with `status: "creating"` and comments "large ones return the live job immediately and the UI follows `torrent_job`".
- The UI polls `for (let attempt = 0; attempt < 240 && … && !job.magnet && !["error","completed","paused","cancelled"].includes(job.status); attempt++)` with a 750 ms sleep, i.e. a hard cap of about 180 s.
- When the cap expires with `status` still `"creating"`: `if (!job.magnet) setShareError(job.error ? … : t("Torrent metadata was not created. Try sharing again."))`.
- That is a false failure: the worker is still hashing and will finish (the job appears in Downloads).
- Following the advice calls `share()` again, which hits `return Err("This archive is already being shared")`.
- The modal keeps rendering `<Loading …/>` whenever `!sharing.magnet`, so a real terminal error shows next to a permanent spinner.
Smallest correct fix: drop the attempt cap. The loop is already bounded by `alive.current`, `shareRequest`, privacy currency and the terminal-status list; closing the modal bumps `shareRequest`. Only set the "not created" error when the job is terminal without a magnet. Render the spinner only while the job is non-terminal and `!shareError`.
Regression test: `scripts/test-collections-async.mjs`-style handler extraction for `shareBook`, with a fake `torrent_job` that returns `creating` for 300 polls then a magnet (stub the timer). Assert `shareError` stays empty and the final `sharing.magnet` is set; a second case with `status: "error"` asserts the error is set and no spinner flag remains.

### SHELL-6 — Typing a download directory opens the confirmation modal 600 ms after the last keystroke; Cancel discards the typed path
Severity: Low        Category: usability
Platforms: macOS, Windows, Linux (the text field is desktop-only)
Evidence class: source-traced
Location: `src/components/SettingsAutosave.tsx:164-173` (`input().onChange`), `:57-70` (`needsConfirmation`/`commit`), `:271-281` (`finish`)  [+ `src/components/Settings.tsx:804-812`]
Trigger: Settings → Files & downloads → type or paste-and-edit a path in "Download directory", pause for more than 0.6 s mid-path.
Causal path:
- `onChange` arms `edit.timer = window.setTimeout(() => commit(key, edit), 600)` for every text key.
- `commit` hits `needsConfirmation` (`key === "downloadDirectory"`) and sets `edit.phase = "confirming"`.
- The App-level `SettingsPolicyConfirmation` then `showModal()`s "Change the download directory? … <partial path>" and focuses Cancel while the user is still typing.
- The next Space/Enter keystroke activates Cancel; `finish(false)` runs `edits.delete(key)` and the field snaps back to the saved directory.
- Accepting instead saves the half-typed path.
Smallest correct fix: do not arm the debounce for confirmation-gated keys. In `input().onChange`, skip the timer when `needsConfirmation(key, edit)` is true. Blur (`onBlur → commit`) and Enter (`form onSubmit → flush`) already commit. Privacy toggles go through `change()` and are unaffected.
Regression test: extract `input`/`commit` in a `vm` harness (as `test-collections-async.mjs` does) with a fake timer. Assert that after `onChange("/tmp/x")` and a 600 ms tick the edit is still `waiting`, and that `onBlur` moves it to `confirming`.

### SHELL-7 — User-initiated cancellations are painted as blocking failures (and twice in Downloads)
Severity: Low        Category: usability
Platforms: all (updater: desktop)
Evidence class: source-traced
Location: `src/components/UpdatesSettings.tsx:149-154` (`installNow` catch)  [+ `src-tauri/src/update.rs:775`, `:909-913`; `src/api.ts:1161-1162`; `src/components/Downloads.tsx:626-630`; `src/components/Catalog.tsx:426-427`]
Trigger: (a) Settings → Install update → Cancel during download. (b) Start a download from a URL, get the low-storage dialog, choose Cancel.
Causal path:
- (a) `update_cancel` makes `install()` return `Err("The update download was cancelled.")`. `installNow`'s catch calls `onError(error)`, and the blocking band shows "The operation could not be completed. The update download was cancelled."
- (b) `request()` throws `new Error(t("Download cancelled."))` when consent is declined. `Downloads.start` does `setStartError(errorText(e)); onError(e);`, so there is an inline alert plus the global band, both for the user's own choice.
- The band persists until dismissed. The doubling contradicts the "single error ownership" recorded in the 2026-11-23 remediation.
Smallest correct fix: add a typed cancellation next to `SuspendedError` in `error-visibility.ts` (e.g. `UserCancelledError`). Throw it from `api.request()` on declined consent, and map the updater's cancel message to it at the `dispatchOnce` boundary (or have `update_install` resolve `{ cancelled: true }`). `reportError`/`errorText` treat it like `suspended`: not recorded as a failure, no band. In `Downloads.start`, keep only one owner (inline).
Regression test: `scripts/test-error-visibility.mjs`: `shouldRenderError`/`reportError` return false for the cancellation type; a handler-level test that `start()` with a declined disk prompt produces no band and no inline error.

### SHELL-8 — 64 UI strings used in code are absent from the localization inventory; the audit that would catch it is not a CI gate
Severity: Low        Category: maintainability
Platforms: all (every non-English locale; one English plural)
Evidence class: reproduced (`node scripts/localization.mjs audit` exits 1 with 64 "Uncollected UI key" failures)
Location: `src/locales/manifest.json` (`sources`: 1552, generated 2026-11-19) vs `scripts/localization.mjs:239-343` (`inventory`), `:940-947` (`audit`)  [+ `.github/workflows/checks.yml:52`; `src/components/AgentConnectorSettings.tsx:115`]
Trigger: any non-English locale opening the agent connector, natural-voice prompts, the Windows camera window, "Replace download(s)" confirmations, "Search this book", and similar. In English: the first MCP request of the day.
Causal path:
- `t()` falls back `catalogs[locale]?.[source] ?? catalogs.en?.[source] ?? source`.
- 61 of the 64 keys are in no `messages/*.json` at all and 3 are English-only, so they are neither translated nor tracked in `missingNew`.
- `"Used today: {count} requests"` has no plural entry in `plural-sources.json` or `en.json`, so `count = 1` renders "Used today: 1 requests".
- CI runs `node --test scripts/test-*.mjs`; `test-localization-resources.mjs` never calls `inventory()`.
- `audit` cannot be used as a gate today: it also reports the documented English-fallback gaps as failures (43,636 "missing new copy" lines).
- It also flags two literal JSX labels, "Zimru" and "zimfile-xapian", at `Settings.tsx:1007,1011`.
Smallest correct fix:
- Run `node scripts/localization.mjs generate` to re-inventory (adds the 64 to `manifest.sources`/`missingNew` and the plural source).
- Add an English plural for "Used today: {count} requests".
- Add a node test (or a `--inventory-only` audit mode) that fails only on "Uncollected UI key", "Dynamic t() source" and "Untranslated JSX", excluding the documented fallback class.
- Wrap or allow-list the two product-name `<dt>`s.
Regression test: new `scripts/test-localization-inventory.mjs` importing `inventory()` and asserting every source is in `manifest.sources`.

### SHELL-9 — Mobile Settings and Help describe a File menu, windows and a New-window shortcut that do not exist on iOS/Android
Severity: Low        Category: usability
Platforms: iOS, Android
Evidence class: source-traced
Location: `src/components/Settings.tsx:527-529`, `:1088-1093`  [+ `src/App.tsx:1692-1693`, `:2859`]
Trigger: open Settings (or Help & about) on a phone or tablet.
Causal path:
- The paragraph "Windows, reader tabs and reading positions reopen at launch. File → Restore previous session reopens the last saved session on demand…" renders unconditionally.
- `restorePreviousSession` begins `if (!isDesktop) return;` and there is no menu on mobile.
- Help lists "New window ⌘ / Ctrl + Shift + N" for all platforms, while the mobile shortcut handler explicitly excludes it (`action !== "new-window"`).
Smallest correct fix: gate the sentence and the New-window row on `isDesktop`, with a mobile variant of the paragraph that omits windows and the File menu.
Regression test: source-level assertion in `scripts/test-interface-accessibility.mjs` style, or a frontend-surfaces case run with `?platform=android` asserting the strings are absent.

### SHELL-10 — `UpdatesSettings` never unregisters its Tauri listeners when cleanup runs before `listen()` resolves; `installing` is never cleared on a resolved install
Severity: Low        Category: reliability
Platforms: desktop
Evidence class: source-traced
Location: `src/components/UpdatesSettings.tsx:70-89` (listener effect), `:136-155` (`installNow`)
Trigger: (a) React StrictMode double-mount in dev, or leaving Settings before the two `listen()` round trips resolve. (b) macOS/Linux: install succeeds, `app.exit(0)` starts the Quit flush, the flush is cancelled (`kiwix-quit-cancelled`), the app keeps running.
Causal path:
- (a) `void listen(...).then((unlisten) => unlisteners.push(unlisten))`. Cleanup iterates `unlisteners` immediately, so a later-resolving `unlisten` is pushed and never called. Every other listener in the shell uses a `keep()`/`if (cancelled) unlisten()` guard.
- (b) `await api("update_install")` resolves `null` on success and only the `catch` resets `installing`. The "Install update" button stays disabled with "Installing…" until Settings is remounted.
Smallest correct fix: (a) `.then((unlisten) => { if (disposed) unlisten(); else unlisteners.push(unlisten); })`. (b) Reset `installing` when a `kiwix-update-progress` `null` arrives (Rust emits `set_progress(&app, None)` at the end), or in a `finally`.
Regression test: handler-extraction test with a deferred fake `listen`: run cleanup before resolving and assert `unlisten` is called once.

### SHELL-11 — Camera window dead-ends if completing the capture is rejected; a locale-chunk failure prevents the camera from ever starting
Severity: Low        Category: reliability
Platforms: Windows (the only platform that opens `camera.html` today)
Evidence class: source-traced; the trigger is narrow
Location: `src/camera.ts:100-109` (`capture`), `:112-114` (`close`), `:136` (`whenLocaleReady().then`)  [+ `src-tauri/src/photo_capture.rs:309-317`]
Trigger: `camera_capture_complete` rejects without Rust closing the window. That happens on the early returns "This camera session has ended" / "Camera unavailable" / "Not available", or a dispatch-level refusal. Separately: the selected locale chunk fails to load.
Causal path:
- `capture()` sets `finished = true`, stops the stream, then `catch (error) { say(String(error)); }`.
- Cancel and Escape call `close()`, whose first line is `if (finished) return;`. Take photo is disabled, so no in-page control works and the raw error text stays until the user finds the title-bar X.
- `void whenLocaleReady().then(() => { …; void start(); })` has no rejection handler, so on a chunk failure `start()` never runs: black stage, disabled Take photo, no message.
Smallest correct fix: in the `capture` catch, reset `finished = false`, show a localized message and focus Cancel (or call `close(String(error))`). Change the boot to `whenLocaleReady().catch(() => undefined).then(...)` so the camera starts in English.
Regression test: jsdom-free handler test in the `vm` style with a fake `invoke` that rejects: assert a subsequent `close()` still sends `camera_capture_cancel`.

---

**Covered** (read end to end unless noted)
- `src/api.ts`, `types.ts`, `i18n.tsx`, `error-visibility.ts`, `platform.ts`, `main.tsx`, `system-text-scale.ts`, `startup-ready.ts`, `display-path.ts`, `byte-units.ts`, `sharing-name.ts`, `recent-history.ts`, `tab-stack.ts`, `library-pins.ts`, `camera.ts`, `camera.html`.
- `src/App.tsx`: all 3,972 lines read. The mobile lifecycle reconcile and quit/close flows were read for structure, not exhaustively re-proved.
- Components in full: `UI.tsx`, `Library.tsx`, `Downloads.tsx`, `Catalog.tsx`, `IntegrityPanel.tsx`, `SwipeRow.tsx`, `SelectedStorageActions.tsx`, `UnifiedSearch.tsx`, `UpdatesSettings.tsx`, `Settings.tsx`, `SettingsAutosave.tsx`.
- Components in part: `Maps.tsx` (230–1285), `Collections.tsx` (60–592), `AdvancedSettings.tsx` (100–400), `PrivateReadingSync.tsx` (100–352), `SharingControls.tsx` (150–300), `AgentConnectorSettings.tsx` (100–122).
- Rust/native contracts opened: `acquisition/persistence.rs`, `update.rs` (install/cancel), `torrent/manager.rs::share`, `photo_capture.rs`, `user_error.rs`, `store.rs` `Settings`, Tauri `AppPlugin.kt`/`TauriActivity.kt`, wry `WryActivity.kt`, `MainActivity.kt`, `HardwareShortcutKeys.kt`, capabilities.
- Tests read: `test-frontend-surfaces.mjs`, `test-error-visibility.mjs`, `test-localization-resources.mjs`; test titles of the other frontend suites.
- Docs: both newest remediation records, `PLATFORM-GAPS.md`, relevant `RELIABILITY.md` passages.

**Not covered**
- Not read: `LocalSwarmControls.tsx`, `CrashReport.tsx`, `IncomingArchiveDialog.tsx`, `DownloadCleanupDialog.tsx`, `KokoroDownloadPrompt.tsx`, `SpeechModels.tsx`, `MapBlockProgress.tsx`, `useVoiceModelStatus.ts`.
- Not read (each has a dedicated test): `diagnostics.ts`, `download-state.ts`, `edition-update-queue.ts`, `suspended-retry.ts`, `shared-library-batch.ts`, `catalog-languages.ts`, `private-sync-api.ts`.
- CSS (`styles.css`, `platform.css`, `tabs.css`, component CSS): targeted greps only (modal/sheet, safe-area, sticky, focus, reduced-motion). No systematic touch-target, RTL, large-text or contrast review, so I have no findings there.
- `index.html`: head only.
- No headless-Chrome run of the surface tests and no device or emulator run. SHELL-2/3/4/6 are source-traced, not observed in a browser.
- Two trivial dead fields seen but not written up: `SharingStatus.discoveryError` (`types.ts:226`) and `LibraryRootResult.removedParts` (`Settings.tsx:72`) are produced by no native code and read by no component.