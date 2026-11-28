## Verdict for lane reader (round 2): only non-blocking findings

R1–R3 are closed, and the bridge hardening holds in both engines. One Low residual remains: a forged hello followed by `document.open()` still receives `reader-init` and the photo secret. No JS or HTTP fix closes it, but a cheap change confines it to Windows with restricted mode off (R4).

Base is `d98cac2`. I reviewed `git diff d98cac2` of Reader.tsx, reader-bridge.js, storage-bridge.js and reader-storage.ts at HEAD `94dda40` plus the working tree; the lane's files had no uncommitted changes. I also read `photo_capture.rs`, the photo path in `dispatch.rs`, the fullscreen accelerator in `menus.rs`, and the injection and service-worker refusal in `server.rs`. I made no repository edits.

### READER-R4 — The photo secret goes to every platform and to restricted mode, so the residual is wider than it needs to be
Severity: Should-fix (Low)
Fix under review: owner decision 2 (the residual)
Evidence class: reproduced in Chromium 158 and WebKitGTK 2.52.6 via `attack2.mjs` step 5 (`replacement script received: ["reader-init+photoSecret"]`); the scoping is source-traced.
Location:
- Reader.tsx:1988 (`initializeDocument`): `photoSecret: photoSecretFor(session.sessionId)` is sent unconditionally.
- Reader.tsx:3669-3703 (`photo-request` handler): it checks only `isTrusted`, the secret, and that no request is already open. There is no platform or restricted-mode check.

Trigger: an archive page posts a forged `reading-gesture-hello`, calls `document.open()`, and reads `reader-init`. It then posts `photo-request` with the secret.

What the secret authorises:
- Only one thing: the shell's modal "“{book}” asks for a photo." without a user activation in the page.
- Anything beyond that needs more user action: "Choose a file" opens the native `pick_image` dialog, and "Use camera" opens the app's `camera-capture` window, where the user still has to take the photo.
- No camera or file access happens without those clicks.
- Without the secret, a hostile page can get the same dialog from any single real click on a photo `<input>`. So the secret only protects "no unprompted dialog". "Use camera" has `autoFocus`, so a dialog timed to appear while the user is typing could open the camera window on Enter. That is still one step away from a photo.

Compared with base: base let any archive page steal the secret with a one-line accessor hook. Now it needs deliberately hostile page code (forged hello plus `document.open()`, a `javascript:` URL, or `document.open()` during the initial load). Strictly narrower, same payload.

Smallest correct fix:
1. Send `photoSecret` only when the photo chooser is enabled, i.e. the same expression as `photoChooser` (`isWindows && session.location.restricted !== true`).
2. Have the `photo-request` handler reject unless `isWindows && session.location.restricted !== true`.
3. Update "only the session being initialised keeps a photo secret" accordingly.

The bridge already refuses without `photoChooser`, so honest pages are unaffected. The residual then exists only on Windows with restricted mode off.

Why no complete fix exists now:
- **Server-attested per-document token:** a hostile archive can mint a valid token. It loads a same-origin nested iframe (the unrestricted CSP allows `frame-src` to the archive's own origin) and reads the bridge `<script>` attribute while `bridge.js` is still being fetched. It can do the same with a one-time bridge URL by fetching that URL first. Rust cannot tell a nested-frame request from the reader frame's own: both send `Sec-Fetch-Dest: iframe`, same-origin. And `document.open()` makes no request, so "was there a new document request" cannot be checked either.
- **`MessagePort` carried in the bridge's hello, with `reader-init` sent over that port:** this closes variant 2 (a genuine hello, then `document.open()` during the load). `document.open()` erases window and document listeners, not the bridge's private port. It cannot close variant 1, because a forged hello can carry the attacker's own port. Feasible but only partial; owner decision.
- **The real fix is a native per-frame channel or isolated world:** a `WKContentWorld` on Apple, a WebKitGTK script world, and a native frame channel on Android and WebView2. That is not a small change.

### READER-R5 — The new Reload failure text states a loss it cannot know
Severity: Nit
Fix under review: R3
Evidence class: source-traced
Location: Reader.tsx:4973
Trigger: retirement is unconfirmed on the Reload or restricted-toggle path.
Causal path: the text "Anything the page had not finished saving is lost." follows "Kiwix could not confirm…". The writes may have been saved.
Smallest correct fix: change "is lost" to "may be lost". The rest (the frame is closed, "Choose Reload article to open it again") matches the button and the code.

### Per-fix assessment
- **R1 — closed.**
  - Shell-message reads go through `MessageEvent` getters captured at bootstrap, and fields are read as own properties.
  - `sessionId` and `photoSecret` are deleted from the cached data object before anything replaceable runs. `delete` on an own data property calls nothing.
  - Propagation is stopped with a pre-bound `stopImmediatePropagation`.
  - The bind is posted first through the pre-bound `postToParent`. The reset work is wrapped in `try`.
  - In the storage and descendant listeners, a message whose origin is not the archive (descendant) or whose type is not storage (storage bridge) returns early with plain comparisons.
  - `bridge.js` is injected after only the doctype, comments, leading text and the `<html>` start tag, so no archive script or handler runs first.
  - Service-worker script requests are refused server-side, so a service worker cannot replace the bridge.
  - `window.onmessage` and handler attributes are registered after the bridge. `MessagePort`/`structuredClone` paths do not exist; the shell transfers only the photo ArrayBuffer.
  - After the scrub, `window.event` and `Object.prototype` getters see only non-secret fields.

  My round-1 `attack.mjs` now gives `archive script holds S=false photoSecret=false`, trace `hello,load,RESESSION(replaced at load),bind,ready`, in both engines.
- **R2 — closed.** A proof timeout re-sessions with `painted=false`. Held paint needs an announcement and is capped at 3 s. A stale cap timer cannot release a later hold, because each `resessionFrame` clears it. A forged hello followed by navigation to another `*.localhost` origin now shows that page for at most 3 s, and `reader-init` to it is dropped by the target-origin check.
- **R3 — closed** apart from the R5 wording nit.
- **Decision 2 — implemented, but not closed against forged-hello plus `document.open()`** (R4). Closed for documents that never announce: bridge-less ones, other origins, and `document.open()` after a completed handshake.

Honest-page deadlock pass, traced through `receive`, `onLoad`, `probeDocument`, `admitLoadedDocument`, `settlePendingInit` and the bind rule:
- **Initial open:** hello then load (Chromium) and load then hello (WebKit) both reach init.
- **Child navigation:** works in either order, and with a second navigation during the re-session.
- **Anchor jumps:** a jump while init is still pending is held; a jump before the bind waits for the bind, then is probed.
- **Busy bridge:** a bridge busy past 1 s is recovered by the late proof.
- **BFCache restore:** the `pageshow` hello is not treated as the document's own, so it re-sessions.
- **Same-document reopen:** this path depends on a `load` event. I measured it: assigning `src` to a different fragment, the same fragment, or no fragment each fires one frame `load` in both engines (`adv-reader/fraglaod.mjs`).
- **Contested bind:** honest bridges post exactly one bind per session; `initializeDocument` runs once per session.
- **Remaining dead ends:** documents that never vouch (a contested binding, a raw non-HTML entry, a document whose bridge was erased) end in the 20 s watchdog, as base did.

### Hostile page: new authority or a suppressed re-session?
None found.
- Keeping a session across a real navigation needs the bound token, which never leaves the bridge's closure except through the pre-bound post.
- Contesting a binding needs the session id, which is now scrubbed from every shell message while the bridge listener lives, so it only hurts the page itself.
- A forged hello only causes a re-session plus a lingering vouch, which is the R4 residual.

### Honest pages and their own `message` events
Only messages from the embedder at the shell origin with `channel: "kiwix-reader"` are stopped. Archive-to-archive and cross-frame messages, and storage messages, are untouched; the reader bridge returns before calling `stopImmediatePropagation` for them, in either at-target listener order.

### macOS fullscreen label
Correct. `⌃⌘F` matches `menus.rs:64` (`Ctrl+Cmd+F`, covered by a test at `:537`). The Reader's keyboard handler returns early on native desktop except for Escape, and the browser preview has `isMacos` false, so F11 shows there. Wrapping "⌃⌘F" in `t()` is harmless.

### Implementer's doubts
- **Complexity:** I did this second pass and found no deadlock or bypass beyond R4.
- **Self-inflicted dead end:** acceptable; only a page that forges a second bind can reach it.
- **Slow first handshake blinks:** acceptable; this is base behaviour plus a recovery.
- **Lingering forged vouch:** the same residual as R4; R4's fix narrows its payload.
- **Target-phase listener order not relied on:** confirmed safe in both orders.

### What I ran
- `node --test scripts/test-reader-*.mjs scripts/test-sensor-policy.mjs scripts/test-media-policy.mjs scripts/test-speech-preferences.mjs scripts/test-reference-transitions.mjs` → `# tests 240 # pass 240 # fail 0`
- `npx --no-install tsc --noEmit` → exit 0, no output
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-frontend-surfaces.mjs scripts/test-shell-surfaces.mjs scripts/test-interface-accessibility.mjs scripts/test-startup-navigation.mjs` → `# tests 22 # pass 22 # fail 0`. Caveat: `dist/` is from Nov 27 22:49 and was not rebuilt, so `test-startup-navigation` does not exercise the current Reader.tsx.
- `adv-reader/attack.mjs` (Chromium and WebKitGTK) → S, photo secret and token all false; `session unchanged=false`.
- `reader/attack2.mjs` (Chromium and WebKitGTK) → steps 1–4 hold in both: anchor `PROOF ACCEPTED`, real navigation re-sessions, `document.open()` without a hello gets only `document-probe`. Step 5 residual reproduced in both: `reader-init+photoSecret`.
- `adv-reader/fraglaod.mjs` (both engines) → each `src` fragment assignment fires `loads=1`.

### What I could not verify
- The production Reader.tsx in any real webview (WKWebView, WebView2, Android WebView, the Linux app). All attack shells are copies of the rules.
- A real iframe BFCache restore.
- Whether old Android System WebView builds use the legacy at-target listener order. The code is safe in both orders.
- R4's fix is proposed only, not exercised.

All my scripts are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-reader/`.