READER review: 10 findings, two High. Reader.tsx and reader-bridge.js were read end to end; the storage bridge, most of reader.css and two Rust files were not (see Not covered).

Both High findings rest on one browser behaviour I reproduced in headless Chrome 158 and WebKitGTK 2.52.6 (the Linux app's engine): an iframe `load` event does not mean a new document. The probe scripts are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/READER/` (`srv2.mjs`, `srv3.mjs`, `srv4.mjs`, `wk.c`, `anchor-preview.mjs`). I did not run the full app; shell-side paths are source-traced.

Two candidates were dropped: I first suspected fragment-only `src` navigation would hang the reader, and testing showed `load` does fire, so it works. Ctrl+wheel zoom dying after the first navigation is already documented in `docs/FEATURES.md:373`.

### READER-1 — Restricted mode and "Reload article" do not reload the page when the article URL has a fragment; the restricted banner is shown over a document that still runs scripts
Severity: High        Category: usability (security-feature false assurance)
Platforms: all
Evidence class: reproduced (browser primitive, Chrome 158 and WebKitGTK 2.52.6) + source-traced (shell path)
Location: src/components/Reader.tsx:4491 (`toggleRestricted`), :5646 ("Reload article" handler), :2140-2146 (`reuseContext`), :2005 (`node.src = session.location.url`); src-tauri/src/service.rs:2239 (`set_reader_restricted`); src-tauri/src/server.rs:1870 (per-response CSP)
Trigger: Open an article from the shell (tab restore, search hit, Home, history reopen). Click a TOC entry or footnote so the URL becomes `…/A#History`, or use a hash-routed archive. Click "Open in restricted mode", or "Reload article" in an error band.
Causal path:
1. `toggleRestricted` calls `setReaderRestricted`, which only flips an atomic flag on the per-window/book reader server (`reader.set_restricted(restricted)`), then runs `publishTab(...); setReload(v+1)`.
2. The open effect keeps the same iframe: `reuseContext = retained !== null && retained.location.bookId === tab.bookId && … && !freshOrigin.current`.
3. `open_article` returns `url = base_url + encode_path(entry.path) + suffix`, which is byte-identical to the live document URL including `#History`.
4. The layout effect runs `node.src = session.location.url`. With an identical URL that has a fragment this is a same-document navigation. `srv2.mjs` output in both engines: "assign identical src A#s1 → iframe load events=1", and no new document. Without a fragment the same assignment does produce a new document.
5. `onLoad` still fires, takes the `initializedFrame.current !== frame.frameId` branch and sends `reader-init`. The old document's bridge accepts the new session and answers `ready`.
6. `restricted = frame?.location.restricted === true` is now true, so the banner "Restricted mode blocks archive scripts, forms, workers and remote network requests" and the active toggle are shown.
7. Restriction exists only as the CSP on a newly served document (`script-src 'nonce-…'; form-action 'none'; worker-src 'none'`). The live document keeps its original unrestricted policy and its scripts keep running until the next cross-document navigation.
8. Leaving restricted mode is equally ineffective, and "Reload article" (`leaveDocument(); resolvedKey.current = ""; setReload`) silently reloads nothing.
Smallest correct fix: Make both actions navigate the browsing context for real before reopening, using the handoff `changeSensorDecision` already uses: `await retireReaderDocuments({ restoreOnFailure: false })`, then `setFrame(null)`, then `setReload`. That keeps the genuine-pagehide storage contract. Only these two "must be a fresh document" callers need it; ordinary navigation can keep context reuse.
Regression test: A browser-driven test next to `scripts/test-startup-navigation.mjs`: open an article, change the frame hash, toggle restricted mode, assert the archive server received a second document request and an inline archive script no longer runs. A unit test can assert `toggleRestricted` and the Reload handler call `retireReaderDocuments` before `setReload`.

### READER-2 — On WebKit every in-page anchor, TOC or footnote jump fires the iframe `load` event, so the shell tears down and rebuilds the reader session
Severity: High        Category: reliability
Platforms: Linux (reproduced engine); macOS and iOS inferred (same WebCore). Chromium (Windows, Android) does not fire this event.
Evidence class: reproduced (WebKitGTK 2.52.6 primitive) + source-traced; macOS/iOS inference
Location: src/components/Reader.tsx:6547-6669 (iframe `onLoad`, re-session branch from :6587); src/reader-bridge.js:3886-3896 (session-replacement branch of `reader-init`); src/components/reader.css:139 (`.is-pending { visibility: hidden }`)
Trigger: On Linux (and, by inference, macOS/iOS), click a TOC panel entry, an in-page `#anchor` link or a footnote marker. The same happens for `location.hash = …` and `history.back()` between fragments.
Causal path:
1. `srv3.mjs` on WebKitGTK: in-frame `location.hash`, anchor click and `history.back()` each give "iframe load events=1"; Chrome gives 0. `srv4.mjs` shows the parent's load handler runs before the child's scroll and hashchange messages.
2. `onLoad` sees `initializedFrame.current === frame.frameId` and treats it as a child navigation: `cancelSensors(active.sessionId); … activeFrame.current = null; setFrameReady(false); setLoading(true)`, then `sensor_deactivate`, `sensor_activate`, and a new `reader-init`.
3. The bridge in the same document hits `if (sessionId && sessionId !== message.sessionId) { resetDocumentHistory(); stopSpeech(); resetReadAloud(); pauseAllMedia(); retireMediaChildren(); … revoke("camera"); revoke("location"); }`.
4. Consequences per anchor click:
   - the article is blanked (`visibility:hidden` plus "Loading article…") for the IPC round trip;
   - playing audio/video is paused and PiP exited;
   - camera streams and location watches are revoked;
   - the media checkpoint is cleared (`clearMediaCheckpoint()`);
   - the bridge's `navigation` and `scroll` posts are dropped because `activeFrame` is null, so `tab.scroll` stays 0 until the next scroll;
   - the bridge's live history is wiped, so Back from an anchor is always refused (`documentHistory.delta` returns null) and falls back to `reopenHistoryTraversal`, a full article reload.
5. The existing test "child navigation withholds readiness until the replacement session authenticates its changed document" (`scripts/test-reader-frame-admission.mjs`) confirms that any second `load` re-sessions.
Smallest correct fix: Re-session only when a new document actually announced itself. The bridge posts `reading-gesture-hello` at bootstrap of every real document, before archive scripts. In `receive`, record a `documentAnnounced` ref for any trusted hello from the frame window at the displayed frame's origin; do this before the `activeFrame` gate and independent of `awaitingGestureHello`. In the re-session branch of `onLoad`, return early when no hello arrived since the last handled load, and clear the ref after each handled load. Leave the first branch (shell-initiated open) unchanged. A forged hello only reproduces today's behaviour; a `document.open()` replacement no longer receives a fresh `reader-init` with the photo secret.
Regression test: In `scripts/test-reader-frame-admission.mjs`: after `r.load(); r.ready();`, a second `r.load()` with no intervening hello leaves `frameReady === true`, issues no `sensor_deactivate`, and keeps the session id; with a hello in between, the existing re-session expectations still hold.

### READER-3 — After an in-archive link click the new page is shown, blanked at `load`, then shown again; clicks on external links before `load` are silently swallowed
Severity: Medium        Category: usability
Platforms: all
Evidence class: source-traced (how visible the blink is depends on IPC latency: inference)
Location: src/components/Reader.tsx:6587-6594 and :6537 (`is-pending` tied to `frameReady`); src/reader-bridge.js:849-855 (`post` is a no-op without `sessionId`), :3980-3982
Trigger: Click an ordinary internal link in an article (native navigation). It is most noticeable on pages whose `load` is slow (many images, shared or LAN archive, slow storage).
Causal path:
1. `frameReady` stays true from the previous document, so the new page paints progressively.
2. Its bridge has `sessionId === ""` until `reader-init`, which is sent only from the iframe `load` handler.
3. In that window the external-link and modifier-click handler does `event.preventDefault(); post("external-link", …)` and `post` returns on `if (!sessionId) return;`. Nothing happens and nothing is shown. Shell commands (Find, TOC, zoom) carry the old session id and are dropped; the toolbar shows the previous title and TOC.
4. At `load` the shell sets `setFrameReady(false); setLoading(true)`, so the already-visible page becomes `visibility:hidden` for `sensor_deactivate` + `sensor_activate` + `reader-init` + `ready`, then reappears.
5. For shell-opened articles the same rule means first paint waits for the full `load`, not DOMContentLoaded. The 20 s watchdog can fire on a page that is merely slow.
Smallest correct fix: Decouple paint from `frameReady` during the re-session only: keep the frame visible when it was ready for the previous document in the same context, falling back to pending if the re-session fails. Controls stay disabled and `activeFrame` stays null. This removes the blink without changing admission. Closing the pre-`load` gap would mean starting the handshake from the bootstrap hello instead of `load`; I would leave that as a follow-up.
Regression test: Frame-admission harness: after a child-navigation `load`, assert the rendered iframe class does not include `is-pending` while `frameReady` is false, and that it does after a failed activation.

### READER-4 — Desktop system-voice read-aloud is cancelled whenever the window becomes hidden
Severity: Medium        Category: usability
Platforms: macOS and Linux (window minimised, hidden or fully occluded). Windows probably unaffected: the vendored wry does not mark the WebView hidden on minimise.
Evidence class: source-traced; platform visibility semantics are inference
Location: src/reader-bridge.js:4229-4232 (`if (document.hidden) stopSpeech();`), :1432-1440 (`stopSpeech`)
Trigger: Start "Read article" with a system voice on desktop, then minimise the window, hide the app or switch Space.
Causal path: Desktop non-Kokoro reading is a `SpeechSynthesisUtterance` inside the archive frame. The bridge's `visibilitychange` handler calls `stopSpeech()`, which runs `speechSynthesis.cancel()` and posts `speech-state {speaking:false}`. Reading ends with no way to resume; the next Play restarts from the top. Native speech (mobile, desktop Kokoro) deliberately survives backgrounding. The comment in `stopSpeech` and the test "background visibility clears paint but keeps the mapping for return" show the hidden handler was meant to clear read-along paint for native speech, not to end desktop reading.
Smallest correct fix: In the visibility handler, clear paint only (`clearReadAloudPaint()`) and do not cancel a live `activeSpeech` utterance. Explicit stop, navigation and pagehide already stop it.
Regression test: `scripts/test-reader-read-aloud.mjs`: start `speak`, emit `visibilitychange` with `document.hidden = true`, assert `speechSynthesis.cancel` was not called and no `speech-state {speaking:false}` was posted. Keep the existing paint-cleared assertion for the native path.

### READER-5 — Link previews fire for same-page anchors, opening a sticky "Article preview" of the article already on screen
Severity: Low        Category: usability
Platforms: all (link previews default on; mostly pointer and keyboard users)
Evidence class: reproduced (`anchor-preview.mjs`, using the production `previewLink`, `articlePath` and `canonicalLocation`)
Location: src/reader-bridge.js:4037-4061 (`previewLink`); src/components/Reader.tsx:3443-3458 (`article-preview` handler)
Trigger: Hover or keyboard-focus a footnote marker `[1]`, an in-page TOC link or a "↑" backlink for 350 ms.
Causal path: The bridge skips only `url.href === location.href`, so `…/A#cite_note-1` is posted. The shell skips only `path === tabRef.current.path`, and "A#cite_note-1" ≠ "A". `article_preview` then resolves the entry with the fragment stripped (`decode_location_path`) and returns the current article's title and first paragraph. The card has no auto-dismiss and blocks mobile chrome hiding (`!preview`). Scratch output: `bridge posted: …/A/Earth#cite_note-1 … shell dispatches article_preview: true`.
Smallest correct fix: In `previewLink`, return when `url.pathname === location.pathname && url.search === location.search`. In the shell handler, also return when the path with its fragment removed equals the tab path with its fragment removed.
Regression test: `scripts/test-reader-preview.mjs`: `r.focus(r.anchor("#section")); r.advance(500); assert.deepEqual(r.events, [])`.

### READER-6 — Per-tab back/forward stacks grow without bound
Severity: Low        Category: reliability
Platforms: all
Evidence class: source-traced
Location: src/components/Reader.tsx:2406 (`navigate`), :3542-3547 (ready/navigation push), :2498-2507 (`travel`), :5237 and :5267 (history menus render every entry); src-tauri/src/service/dispatch.rs:920 (`save_session` accepts any stack length); src-tauri/src/store.rs:371 and :3528 (`MAX_SESSION_BYTES`, 8 MiB)
Trigger: A long-lived, session-restored tab. Every link click and every hash jump appends an entry.
Causal path: Nothing in TS or Rust trims `tab.back` or `tab.forward`; only the bridge caps a single traversal at 200 and its own live history at 512. The whole stack is re-serialised into every debounced `save_session`, both history `<details>` menus render one button per entry on each Reader render, and the previous-session snapshot refuses to write past 8 MiB ("The saved session exceeds the supported size…"), with no remedy except closing tabs. `photoSecrets` (Reader.tsx:958) also gains one never-pruned entry per session.
Smallest correct fix: Cap both stacks where they are produced (the bridge's 200 is a natural number) by slicing in `navigate`, the ready/navigation push and `travel`. Delete the outgoing session's `photoSecrets` entry on re-session.
Regression test: Frame-admission harness: feed 250 changed `ready` messages and assert `tabRef.current.back.length <= 200` with the newest entries retained.

### READER-7 — Tapping a `tel:` link raises a blocking red error band with a "Reload article" button
Severity: Low        Category: usability
Platforms: all; most visible on phones (travel-guide phone numbers)
Evidence class: source-traced
Location: src/reader-bridge.js:3971 and :4119 (bridge forwards `tel:`); src/components/Reader.tsx:3766-3769 and :556 (`validExternal`); src-tauri/src/service.rs:3788
Trigger: Tap a phone-number link in an article.
Causal path: The bridge's allow-list includes `tel:` and posts `external-link`. The shell's `validExternal` allows only http, https and mailto, so it calls `report(new Error(t("This type of external link is not permitted.")))`. That is a `role="alert"` band whose only action is "Reload article". Native `external_link_target` rejects `tel:` too, so the bridge allow-list and the shell/native policy have drifted. `sms:` and `geo:` are swallowed with no feedback at all.
Smallest correct fix: Either support `tel:` end to end behind the same confirmation dialog (shell and native allow-lists), or treat unsupported schemes as a quiet `setNotice(...)` and drop `tel:` from the bridge allow-list so the two sides agree.
Regression test: `scripts/test-reader-external-links.mjs`: a `tel:` external-link message must not call `report` with blocking severity (or must reach `open_external`, if supported).

### READER-8 — Mobile "Print / save as PDF" row and Ctrl/Cmd+P only call `window.print()` in the frame
Severity: Low        Category: usability
Platforms: Android, iOS
Evidence class: source-traced; the WebView no-op is inference and already listed as unverified in docs/PLATFORM-GAPS.md:63
Location: src/components/Reader.tsx:4507-4516 (`printArticle`), :5891-5908 (row enabled on mobile), :4646-4651 (shortcut); src/api.ts:38 (`canPrintArticlePdf = isDesktop`)
Trigger: On a phone, open Reading appearance → Print and export → "Print / save as PDF".
Causal path: `!canPrintArticlePdf` leads to `send({ command: "print" })`, then `window.print()` in a sandboxed cross-origin iframe. Nothing visible happens and no error is shown, while the row directly below (`exportArticle("pdf")`) already opens the Android system print dialog. This is a documented gap; I list it because the Android half is a small source fix.
Smallest correct fix: On mobile, route `printArticle` to `exportArticle("pdf")` on Android and hide the row on iOS until a native print path exists. Keep `window.print()` only for the non-native browser preview (`!isNative`).
Regression test: `scripts/test-interface-accessibility.mjs` or a small AST test: with `isMobile`, `printArticle` must not emit the `print` reader command.

### READER-9 — Dead message handlers, one kept alive only by a test
Severity: Low        Category: maintainability
Platforms: all
Evidence class: source-traced
Location: src/reader-bridge.js:3767-3769 (`case "selection"`); src/reader-storage.ts:690-695 (`kiwix-storage-closed`); scripts/test-reader-storage-admission.mjs:248-251
Trigger: n/a
Causal path: `ReaderCommand` has no `selection` member and nothing in `Reader.tsx` sends it, so the bridge case is unreachable. `storage-bridge.js` never posts `kiwix-storage-closed` (it sends ack, batch, credit, drain, drained, failed, hiding, ready, resume), yet the host handles it, and the BFCache admission test simulates stream closure with that message. That test therefore exercises a path production cannot take. All other message and command names match across the four files.
Smallest correct fix: Delete both handlers. Rewrite the BFCache test to close the stream the way production does (hiding plus successor ready and fence).
Regression test: A static test that every `case "<command>"` in the bridge appears in the `ReaderCommand` union and every `kiwix-storage-*` type handled by the host is posted by `storage-bridge.js` or the retirement page.

### READER-10 — TOC panel can contain blank, unnamed buttons
Severity: Low        Category: usability
Platforms: all
Evidence class: source-traced
Location: src/reader-bridge.js:1409-1420 (`toc`); src/components/Reader.tsx:3588-3602 (filter), :5731-5741 (render)
Trigger: An article with empty or image-only headings, or headings inside hidden or collapsed chrome.
Causal path: `toc()` emits every `h1`–`h6` with `title: heading.textContent.trim()`. The shell filter accepts `text(entry.title, 4096)`, which includes the empty string, and renders `<button>{entry.title}</button>`. The result is an empty button with no accessible name; hidden headings also appear.
Smallest correct fix: Skip entries whose trimmed title is empty in `toc()` (and in the shell filter), and skip headings that are `hidden` or have `aria-hidden="true"`.
Regression test: Bridge unit test in the read-aloud DOM harness: a document with `<h2></h2>` and `<h2 hidden>X</h2>` yields no TOC entries for them.

**Covered**
- `src/components/Reader.tsx`: all 7,255 lines.
- `src/reader-bridge.js`: all 4,275 lines.
- `src/native-speech.ts`, `reader-media-checkpoint.ts`, `reader-search-status.ts`, `reference-transitions.ts`, `media-policy.ts`, `sensor-policy.ts`, `speech-preferences.ts`, `speech-voice-filter.ts`, `speech-voice-ranking.ts`, `speech-voice-selection.ts`: in full.
- `src/reader-storage.ts`: lines 1-520 and 776-1075 (registration, admission, quarantine, retirement entry).
- Message and command name cross-check across `reader-bridge.js`, `storage-bridge.js`, `Reader.tsx` and `reader-storage.ts`.
- `src/components/reader.css`: targeted only (pending and loading states, status bar, find bar, preview card, compact media query, physical-direction scan).
- Rust contracts: `Service::reader` / `open_article`, `set_reader_restricted`, `external_link_target`, the `article_preview`, `open_external` and `save_session` arms; in `server.rs` the bridge injection and boot appearance, `archive_response` CSP, and location encode/decode helpers.
- `src/App.tsx`: `ReaderHost` and the Reader mount site only.
- Docs: the two newest review records, `PLATFORM-GAPS.md`, reader rows of `FEATURES.md`.
- Tests: read the frame-admission, document-identity, external-links and preview harnesses; ran frame-admission, history, preview, read-aloud and external-links (55 pass).

**Not covered**
- `src-tauri/src/zim_scheme.rs` and `src-tauri/src/sharing_reader.rs`: not read.
- `src/storage-bridge.js`: body not read beyond the pagehide/hiding snippet and message names.
- `src/reader-storage.ts`: lines 520-775 (message intake) and 1075-1361 (retire, flush, freeze) only grepped.
- Most of `reader.css` (no systematic large-text, high-contrast or small-screen pass).
- Rust sensor, photo, speech, bookmark, spelling and `reader_storage_write` arms; the rest of `server.rs`.
- Native mobile speech engines.
- Contents of `test-reader-video-capability`, `-theme-boot`, `-sensor-permissions`, `-storage-*`, `-live-export`, `-resource-context`, `test-mobile-voice-search`.
- No real-app runtime: nothing was exercised in WKWebView, WebView2 or Android WebView, and READER-2 was not reproduced on macOS or iOS.
- Nothing meeting the bar was found in the speech-preference/voice-selection modules, `reference-transitions.ts`, `media-policy.ts`, `sensor-policy.ts` or `reader-search-status.ts`.