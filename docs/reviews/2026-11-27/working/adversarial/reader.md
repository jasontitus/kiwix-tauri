## Verdict for lane reader: only non-blocking findings

Base `d98cac2`. I reviewed `git diff d98cac2` for Reader.tsx, reader.css, reader-bridge.js, reader-storage.ts and the 8 test files (no untracked files in the lane), and read the surrounding code those changes rely on:
- **Bridge:** bootstrap 690-860 and the parent message listener 3880-3973.
- **Shell:** `receive`, `onLoad`, the open effect, `deactivateFrame`, the 20 s watchdog, ReaderHost (App.tsx), `retireDocuments`.
- **Rust:** server.rs bridge injection, `html_head_offset`, `archive_response` CSP/Cache-Control, `restricted_nonce`; service.rs `open_article` and `set_reader_restricted`; tauri.conf CSP.

No edits were made to the repository. Scratch files are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-reader/`.

### READER-R1 — An archive script can own the token the shell bound, so the READER-2 proof does not stop a hostile archive keeping a session across a real navigation
Severity: Should-fix. It would be Blocking if per-document re-sessioning is meant to be an authority boundary against the archive itself. I judge that it is not: both documents are the same archive origin, and sensor decisions are per installation.
Fix under review: READER-2
Evidence class: reproduced in Chromium 158 and WebKitGTK 2.52.6 (`xvfb-run`). The real `src/reader-bridge.js` ran under a stand-in shell that copies Reader.tsx's rules: hello sets `documentAnnounced`, first-writer `reading-pan-bind`, `documentReplaced`, probe/proof acceptance. Script: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-reader/attack.mjs`.
Location:
- src/reader-bridge.js:3880-3914 (message listener): `const message = event.data;` is read through the overridable `MessageEvent.prototype.data` getter. `stopImmediatePropagation` (3910) and `post("reading-pan-bind")` (3914) both run after that read.
- src/components/Reader.tsx:3559-3572: the first `reading-pan-bind` for a session wins and sets `documentAnnounced.current = data.token`.
- Reader.tsx:1962 (`documentReplaced`), 3458-3479 (proof acceptance), 6927-6929 (`onLoad`).

Trigger: any archive page runs this after the bridge:
```
Object.defineProperty(MessageEvent.prototype, 'data', {get(){ const d = orig.call(this); /* steal reader-init, post reading-pan-bind {sessionId:d.sessionId, token:X} */ return d; }})
```
It saves S and X in sessionStorage and navigates to doc2. Doc2's script re-posts `reading-gesture-hello {token:X}` after its own bridge's hello, and answers `document-probe` with `{nonce, token:X}`.

Causal path:
1. The getter runs synchronously inside the bridge's own reader-init handling, so the archive's bind for S reaches the shell first. `panAuthority = {S, X}`.
2. At doc2's load, the newest hello is X, so `documentReplaced` is false and `probeDocument` runs.
3. Doc2's script answers with X, which passes `data.token === authority.token`.

Observed result, identical in both engines:
- trace `hello,hello,load,probe,PROOF ACCEPTED`, `session unchanged=true`;
- doc2's bridge never gets a session (a `toc` command gets no reply);
- the same hook also read the photo secret.

The implementer's claim (a), "that token is only ever posted to the parent", is true of `readingPanToken` but irrelevant: the shell trusts whatever was bound first. Bridge-less documents (`document.open()`, `about:blank`, or a `javascript:` URL) that receive the re-session's reader-init can do the same with no hook at all.

Base comparison: base re-sessioned every load, but the same hook stole every new session and photo secret. So the archive gains no new capability, but the READER-2 safety argument and the comment "Archive scripts never see reader-init" are false. Pre-existing and outside this diff, but in this lane's file: the photo secret, which authorises the Windows camera/photo chooser, leaks to any archive page through this hook.

Smallest correct fix:
1. At bootstrap, capture native getters for `MessageEvent.prototype` `data`, `source` and `origin`, the same way `photoNatives` and `nativeGetter` already do, and use them in the parent listener.
2. For reader-init, call `stopImmediatePropagation` and post `reading-pan-bind` before any archive-reachable work. The re-session reset branch (`stopSpeech`, `querySelectorAll`, media calls) can currently be made to throw before the bind.
3. Restate in comments and docs that the probe tells benign same-document loads apart, and is not an authority boundary against same-origin archive script.
4. Add a frame-admission test in which an archive-authored bind wins the race.

### READER-R2 — Held paint shows a document the shell never admitted, including another localhost origin, for about 21 s
Severity: Should-fix
Fix under review: READER-3
Evidence class: source-traced
Location: Reader.tsx:1983 (`setHeldPaint(painted ? active.frameId : "")`); the probe timeout at 2085-2091 passes `probe.painted` (true); the watchdog clears it at about 5163.
Trigger: archive script runs `location.href = "http://anything.localhost:<port>/"` (allowed by the shell's `frame-src http://*.localhost:*`), or navigates to another archive's reader server, or to a bridge-less document.
Causal path:
1. The load is unannounced, so a probe is posted to the old origin; the browser drops it.
2. After 1 s, `resessionUnproven` calls `resessionFrame` with `painted = true`.
3. `reader-init` to the old origin is dropped too, so `ready` never arrives.
4. The foreign or bridge-less page stays visible and clickable under "Article: <previous title>" until the 20 s watchdog.

Base set `is-pending` (`visibility: hidden`) at that load. Messages stay origin-gated, so nothing leaks. The regression is presentation and clickjacking surface for local web UIs.
Smallest correct fix: hold paint only when the replacement was announced by a same-origin bridge, that is when `documentReplaced` is true at load or a hello arrives during the probe. When the proof window times out, re-session with `painted = false`.

### READER-R3 — Reload with an unreachable retirement page ends in contradictory advice
Severity: Nit (owner decision)
Fix under review: READER-1
Evidence class: source-traced
Location: Reader.tsx:4785-4818 (`reopenFreshDocument`); reader-storage.ts about 1114 (5 s retirement timer)
Trigger: an error band whose cause leaves the reader origin unable to serve `retire.html`; the user clicks "Reload article".
Causal path: `retireReaderDocuments` waits 5 s for `kiwix-storage-retired`, then rejects. `setError("…Close and reopen this window to continue.")` is shown next to a Reload button that works on the second click, and that click discards the quarantine the first one created. Base reopened immediately.
Smallest correct fix: on the Reload path, word the failure as "click Reload again to discard and reopen", or let Reload discard the quarantine itself.

### Per-fix assessment
- **READER-1:** Fixes it. `setFrame(null)` plus `setReload` renders with `displayedFrame = null`, so `reuseContext` is false and a new iframe element (new browsing context) is created. HTML is always served as `text/html` with `no-store`, and the nonce/CSP is computed per request. Cases checked:
  - Toggling twice is guarded by `restrictedBusy`.
  - Toggling mid-load aborts the in-flight open through `openSequence`.
  - Quarantine or failed retirement removes the frame, so no banner remains.
  - A tab switch remounts Reader (keyed by tab id) through ReaderHost retirement.
  - `open_article` only ever sets restricted on (`Some(true)`).
  - I found no sequence where the banner sits over a document served unrestricted. See R3 for the copy issue.
- **READER-2:** Fixes the benign WebKit re-session. The fail-closed paths hold for honest pages:
  - `document.open()`, BFCache and bridge-less documents re-session at the timeout.
  - Floods only cause re-sessions or rejected proofs.
  - Source, origin, `isTrusted`, session, latest nonce and bound token are all checked (3425-3432, 3464-3473).
  - Timers are cleared on deactivate, open, unmount and re-session.

  But it does not hold against hostile archive script (R1).
- **READER-3:** Fixes the blink, but see R2.
- **READER-4:** Fixes it. Only paint is cleared on hide. Mobile and desktop Kokoro use native speech and do not go through this handler. Pagehide, navigation and stop still end speech.
- **READER-5:** Fixes it.
- **READER-6:** Fixes it. Stacks keep the newest entries at the end; photo secrets are pruned.
- **READER-7:** Fixes it. http, https and mailto behave as before. `javascript:` links were already blocked by `preventDefault` in base. Synthetic clicks are silent. A script can post `unsupported-link` itself, but that only produces a 5 s status notice.
- **READER-8:** Fixes it.
  - The browser preview (`!isNative`) keeps `window.print()`.
  - Desktop keeps the native print panel.
  - The print row is shown whenever `!isMobile`, so desktop and preview are unchanged.
  - Android Ctrl+P comes only from a trusted shell keydown.
- **READER-9:** Fixes it. No producer of `kiwix-storage-closed` exists: storage-bridge.js posts ack, batch, credit, drain, drained, failed, hiding, ready and resume, and server.rs posts only `kiwix-storage-retired`. No sender of the `selection` command exists.
- **READER-10:** Fixes it.

### Implementer's doubts
- **Reload waits for retirement:** acceptable, but fix the copy (R3).
- **Restricted button busy during retirement:** acceptable.
- **1000 ms proof window:** acceptable. A miss degrades to base behaviour, and the extra up-to-1 s of admission grants nothing new because messages stay origin-gated.
- **reader-init sent to bridge-less documents:** a real, pre-existing problem, and it combines with R1. Needs an owner decision for the follow-up.
- **Held paint for bridge-less documents:** a real problem to fix now (R2).
- **Chromium also fires unannounced loads:** consistent with what I observed.
- **Message ordering:** safety does not depend on it for benign pages. Overwrites at bind or proof can only fall back to the timeout. It is undermined by R1, not by ordering.

### What I ran
- `node --test scripts/test-reader-*.mjs scripts/test-sensor-policy.mjs scripts/test-media-policy.mjs scripts/test-speech-preferences.mjs scripts/test-reference-transitions.mjs` → `# tests 229 # pass 229 # fail 0`
- `npx --no-install tsc --noEmit` → no output, exit 0
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-frontend-surfaces.mjs` → `# pass 1 # fail 0`
- Same with `scripts/test-shell-surfaces.mjs scripts/test-interface-accessibility.mjs scripts/test-startup-navigation.mjs` → `# tests 21 # pass 21 # fail 0`
- `node adv-reader/attack.mjs` (Chromium 158) and `xvfb-run ../READER/wk` (WebKitGTK, UA Version/60.5) → both `PROOF ACCEPTED`, `session unchanged=true`, archive holds S, photoSecret and the bound token.

### What I could not verify
- The production Reader.tsx inside a real webview (WKWebView, WebView2, Android WebView, the Linux app). The probe's shell is a copy of the rules, not the component.
- R2 and R3 at runtime.
- Whether `*.localhost` resolves to loopback in every shipped webview.
- No cargo run: the lane touches no Rust, and the bridge is pulled in through `include_str!`.