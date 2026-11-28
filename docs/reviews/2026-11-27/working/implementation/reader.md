# Lane `reader` — report

All ten findings are fixed in owned files; READER-3 is partial by instruction. `tsc --noEmit` is clean and the lane's tests are 229/229. The full frontend suite has 2 failures, both generated-file checks the integration owner regenerates (details under Commands run).

Review base `d98cac2`. My files are already inside the WIP checkpoints, so use `git diff d98cac2 -- <file>` to see them.

Two things in READER-2 differ from the reviewer's proposal, and the independent reviewer should look at them first:
- **A hello flag alone is not safe or sufficient.** On WebKitGTK 2.52.6 a quickly loaded document's `load` event reaches the shell *before* its bootstrap hello (probe below). A boolean "hello since last load" would both miss real navigations and misfire on the session's own late hello.
- **I added a positive proof.** The session survives an unannounced `load` only if the bridge that holds it answers a nonce probe with its private bootstrap token. Everything else re-sessions as before.

---

### READER-1 — Restricted mode / "Reload article" do not load a new document when the URL has a fragment
Disposition: fixed

Verification of the finding: Read `toggleRestricted`, the Reload handler, the open effect (`reuseContext`), the layout effect (`node.src = …`), `service.rs::reader` / `set_reader_restricted`, and the `server.rs` CSP and `Cache-Control: no-store` for HTML. Reproduced the primitive with my own probe in both engines:
- Chromium 158: same frame, identical `src` with fragment → document requests stay 1, `load` fires; new frame element, same URL → requests 2, hello + load.
- WebKitGTK 2.52.6: identical result.

Change: `Reader.tsx::reopenFreshDocument` (new), used by `toggleRestricted` (both directions) and the Reload button.
- It deactivates the frame, calls `retireReaderDocuments({ restoreOnFailure: false })` (real pagehide and storage handoff), then `setFrame(null)` + `setReload`. The open effect then builds a new iframe, which is a new browsing context and therefore always a new document, fragment or not.
- `resolvedKey` is cleared up front so an interrupted reopen cannot settle on the old document.
- On an unconfirmed retirement it follows the `changeSensorDecision` precedent: quarantine storage, remove the frame, show the existing "could not confirm that the article finished saving" message, no reopen.
- The banner derives from `frame.location.restricted`, so it only changes with the replacement document; on failure there is no frame to carry it.
- `freshOrigin` is deliberately not set: restriction is a per-response CSP on the same reader server, and HTML is `no-store`.

Tests: new `scripts/test-reader-fresh-document.mjs`, 4 tests, `# pass 4 # fail 0`.
- Covers the toggle in both directions, Reload with and without registered storage, the failed-retirement path, and an overtaken reopen.
- Observed failing on base: I ran the same file against a `d98cac2` copy of the sources (extraction assert relaxed) and all 4 failed on behaviour, e.g. no `retire` event before reload.

Not exercised: the full app. No test drives the production shell in a real browser with a real archive server, so "banner never shown over an unrestricted document" is unit-tested plus primitive-probed, not end-to-end.

### READER-2 — WebKit fires iframe `load` for in-page jumps and the shell re-sessions
Disposition: fixed differently (reviewer's hello-only rule is neither fail-closed nor correct on WebKit)

Verification of the finding: Read all of `receive`, `onLoad`, the bridge handshake, and both admission test files. Probe `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/reader/probe.mjs` runs the real `src/reader-bridge.js` cross-origin under a stand-in shell (not the production shell). Results are in `report-chromium.txt` and `report-webkit.txt` in the same directory.

| Probe step | Chromium 158 | WebKitGTK 2.52.6 |
|---|---|---|
| `location.hash`, anchor click | 0 frame loads | 1 frame load, 0 hellos, load arrives before any child message |
| `history.back()` between fragments | 1 frame load, 0 hellos | 1 frame load, 0 hellos |
| Probe of held session after each jump | proof with the bridge's hello token | same |
| Real navigation doc1→doc2 | `hello, load` | `load, hello` |
| New document probed for the old session | no answer | no answer |
| `document.open()` replacement | `load`, no hello; bridge listener erased; replacement script sees the probe but cannot produce the token | same |
| Archive script in the same document | sees `document-probe`, never `reader-init` | same |

Change:
- **Bridge** (`reader-bridge.js`, message listener): new `document-probe` → `document-proof {nonce, token: readingPanToken}`. Answered only for the session the bridge holds, from the parent, with a UUID nonce.
- **Shell** (`Reader.tsx`):
  - `documentAnnounced` holds the token of the newest hello. It is recorded before the `activeFrame` gate, tied to the frame's window and the displayed origin.
  - `documentReplaced(session)` is true when the session is bound and the newest hello token is not its token.
  - `onLoad`, initialised frame: replaced → `resessionFrame` (old behaviour); otherwise `probeDocument`.
  - `probeDocument`: an unbound session is never probed, it re-sessions at once. Otherwise it sends a nonce and waits `DOCUMENT_PROOF_TIMEOUT` (1000 ms); no proof → re-session.
  - A foreign hello while a probe is open → re-session immediately. This is the normal WebKit navigation path (load, then hello).
  - A proof is accepted only with the latest nonce, the session id, and the token bound to that session.
  - The first (shell-initiated) branch is unchanged, apart from using the shared `initializeDocument`.

Why a hostile archive cannot abuse it:
- **(a) Keep an old session across a real navigation.** Skipping the re-session requires the token that the old document's bridge generated before any archive script ran. That token is only ever posted to the parent, so the new document cannot know it. The new document's own bridge has no session and does not answer.
- **(b) Suppress a re-session.**
  - Hello before load: replaced → re-session at load.
  - Hello after load: re-session on the hello; if it never arrives, on timeout.
  - `document.open()`/`write`: no hello and no proof → timeout → re-session, exactly as today.
  - Navigate away and back: each fresh document announces a new token.
  - BFCache restore: no hello; the restored bridge holds a retired session, so it cannot answer the admitted one → timeout → re-session, after which it binds its token and behaves normally.
  - Forged hello: can only cause a re-session.
  - Forged proof: needs the token.
  - Forged bind: bindings are first-writer and the bridge binds synchronously inside `reader-init`, which archive scripts never see. This is why unbound sessions are never named in a probe.
- **Ordering.** Safety does not depend on message/load ordering; ordering only decides whether the re-session is immediate or waits ≤1 s.

Tests: `scripts/test-reader-frame-admission.mjs`, `# pass 32 # fail 0`.
- 12 new tests cover: proved anchor load, unproven timeout, seven proof forgeries, stale nonce, hello after load, forged hello, own late hello (WebKit order), load before hello, unbound session, hello during an in-flight re-session, BFCache restore, and a stale probe.
- Bridge side in `scripts/test-reader-read-aloud.mjs`: "the bridge proves an unchanged document only for the session it holds…".
- Existing child-navigation tests now model the real handshake (hello + bind).
- Observed failing on base: yes, but many fail because the base lacks the extracted callbacks rather than on an assertion.

Not exercised: the production shell inside WKWebView, WebView2, Android WebView or the Linux app. macOS and iOS remain inferred from WebKitGTK.

### READER-3 — Blink on in-archive navigation
Disposition: partial (as instructed: paint decoupling only; handshake-before-load left as follow-up)

Verification of the finding: source-traced `is-pending` ↔ `frameReady` and the re-session branch.

Change:
- `Reader.tsx` state `heldPaint` (a frameId). Set in `resessionFrame` only when the frame was ready; the class is `frameReady || heldPaint === frame.frameId`.
- Cleared on ready, failed or abandoned re-session, watchdog, `deactivateFrame`, and every shell-initiated open.
- Admission is untouched: `activeFrame` is null and controls stay disabled during the hold.
- `reader.css` comment updated to state the exception.

Tests: "a child navigation keeps the page painted while it re-sessions, and hides it if that fails" — ok.

Not exercised / remaining: clicks on external links before `load` are still swallowed, and shell-opened pages still wait for `load`. Both need the handshake to start from the hello.

### READER-4 — Desktop read-aloud cancelled when the window is hidden
Disposition: fixed

Verification of the finding: bridge `visibilitychange` → `stopSpeech()` → `speechSynthesis.cancel()`. Suspension is mobile-only and unmount sends `stop-speech` explicitly, so nothing relied on the visibility stop.

Change: the handler now only clears read-along paint. Stale comment in `stopSpeech` corrected.

Tests: "a hidden window keeps desktop read-aloud going and only drops its paint" — ok; failed on base.

Not exercised: real window minimise on macOS/Linux.

### READER-5 — Link previews for same-page anchors
Disposition: fixed

Verification of the finding: read `previewLink` and the shell `article-preview` handler.

Change: the bridge skips links with the same pathname and search. The shell also compares paths with the fragment removed.

Tests: `test-reader-preview.mjs` "footnote, section and back links…" and frame-admission "a preview request for the page on screen is dropped…" — ok; both failed on base.

Not exercised: nothing further.

### READER-6 — Unbounded history stacks and `photoSecrets`
Disposition: fixed

Verification of the finding: no trim existed in `navigate`, `travel`, or the ready/navigation push.

Change:
- `boundedHistory` (200 nearest entries) applied in all three places.
- `photoSecretFor` keeps only the session being initialised. It is called only from `initializeDocument`, when that session is the active one.

Tests: "a tab's history keeps only its nearest entries", "only the session being initialised keeps a photo secret" — ok; failed on base.

Not exercised: the Rust `save_session` size path.

### READER-7 — `tel:` raises a blocking error band
Disposition: fixed (conservative option; no new schemes; no Rust edits)

Verification of the finding: bridge allow-lists included `tel:`; shell `validExternal` did not.

Change:
- The bridge's click and middle-click lists are now http/https/mailto, matching the shell and native.
- Other schemes stay blocked. A trusted activation posts an address-less `unsupported-link`; `javascript:` and script-made clicks stay silent.
- The shell answers `unsupported-link`, and any invalid `external-link`, with `setNotice(...)` using the existing string instead of `report(...)`.

Tests: bridge "links the app cannot open stay blocked…" and shell "a link the app cannot open is answered with a quiet notice…" — ok; failed on base.

Not exercised: a real phone.

### READER-8 — Mobile print row / Ctrl+P only call `window.print()`
Disposition: fixed

Verification of the finding: `printArticle` → `!canPrintArticlePdf` → frame `print` on mobile. The row below is already Android's print dialog.

Change:
- `printArticle`: non-native → `window.print()`; desktop → native panel; Android → `exportArticle("pdf")`; iOS → a quiet notice.
- The separate print row renders only when `!isMobile`. Android's working row is directly below it, so I hid the duplicate on Android as well as iOS.
- The shortcuts list omits Print on iOS.

Tests: new `scripts/test-reader-print.mjs`, `# pass 5 # fail 0`; 4 of 5 failed on base.

Not exercised: Android and iOS devices. That `window.print()` is a no-op in mobile webviews remains unverified.

### READER-9 — Dead handlers
Disposition: fixed

Verification of the finding: nothing sends the `selection` command; nothing posts `kiwix-storage-closed`.

Change:
- Removed bridge `case "selection"` and the host `kiwix-storage-closed` branch.
- Rewrote the BFCache storage test to close the stream through hiding + successor ready + fence, and added a test that this path really closes it.

Tests: new `scripts/test-reader-message-contract.mjs` (command sets equal in both directions; storage message types matched in both directions, including the Rust retirement page) — `# pass 2`. Run against base sources it fails with exactly `selection` and `kiwix-storage-closed`. `test-reader-storage-admission.mjs` — `# pass 13`.

Not exercised: n/a.

### READER-10 — Blank TOC buttons
Disposition: fixed

Verification of the finding: `toc()` emitted every heading; the shell accepted empty titles.

Change:
- `toc()` skips headings that are empty, self-`hidden`, or `aria-hidden="true"`. Generated ids keep the full-list index so jumps still resolve.
- The shell filter drops blank titles.
- Headings inside hidden ancestors are deliberately kept: they may be collapsed sections the TOC jump reveals.

Tests: "the contents list names every entry and skips blank or hidden headings", "blank section titles never become contents entries" — ok; failed on base.

Not exercised: n/a.

---

## Commands run
- `npx --no-install tsc --noEmit` → no output (clean).
- `node --test scripts/test-reader-*.mjs scripts/test-sensor-policy.mjs scripts/test-media-policy.mjs scripts/test-speech-preferences.mjs scripts/test-reference-transitions.mjs` → `# tests 229 # pass 229 # fail 0`.
- `node --test scripts/test-*.mjs` → `# tests 489 # pass 476 # fail 2 # skipped 11`. Both failures are generated files:
  - `docs/ERRORS.md matches the error copy…`
  - `every UI string in the sources is collected in the localization manifest` — 88 uncollected keys, one of them mine.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented.`
- Probes: Chromium via puppeteer-core; WebKitGTK via the reviewer's `wk` under `xvfb-run`.
- No `cargo`, no `npm run build`.

## Files changed
- `src/components/Reader.tsx`
- `src/components/reader.css` (comment only)
- `src/reader-bridge.js`
- `src/reader-storage.ts`
- `scripts/test-reader-frame-admission.mjs`
- `scripts/test-reader-preview.mjs`
- `scripts/test-reader-read-aloud.mjs`
- `scripts/test-reader-storage-admission.mjs`
- `scripts/test-reader-video-capability.mjs` (harness refs only)
- New: `scripts/test-reader-fresh-document.mjs`, `scripts/test-reader-message-contract.mjs`, `scripts/test-reader-print.mjs`

New user-visible string (one): `"Printing is not available on this device. Save or share the article as a PDF instead."`

`"This type of external link is not permitted."` is reused; it now appears as a status notice instead of an error band.

## Handoffs / contract changes
- Run `node scripts/localization.mjs generate` to collect the new string.
- `docs/PLATFORM-GAPS.md` item 8 ("iOS print") is now stale. Android routes to its print dialog; iOS no longer offers the row and Cmd+P shows a notice. A native iOS print path is still missing.
- New bridge protocol messages, spoken only by `Reader.tsx` and `reader-bridge.js`: shell→frame `document-probe {sessionId, nonce}`; frame→shell `document-proof {nonce, token}` and `unsupported-link {}`.
- `reader-bridge.js` is `include_str!`-embedded by `server.rs`, so the Rust build picks up the bridge change. I found no Rust test asserting on the changed text.
- No changes needed in `App.tsx`, `UI.tsx` or `api.ts`.

## Doubts
- **Reload now waits for retirement.** If the retirement page cannot be confirmed within 5 s (e.g. dead reader server), the user gets the quarantine message and must click Reload again, which discards the quarantine and reopens. Before, Reload retried in place immediately. I kept the precedent's fail-closed path rather than auto-discarding; say if you want Reload softened.
- **Restricted toggle keeps its button busy for the retirement** (normally milliseconds, up to 5 s).
- **Proof window is 1000 ms.** A main thread janky beyond that degrades one anchor jump to the old re-session behaviour. Bridge-less replacements stay admitted up to 1 s longer than before; the old session was already honoured from navigation commit until `load`.
- **Pre-existing, not fixed.** `reader-init`, with the session id and photo secret, is still posted to whatever document is in the frame at `load`. That includes bridge-less ones: a `document.open()` replacement (confirmed in both engines) and, by source reading, a `blob:` HTML navigation. Closing it means only initialising a document that announced itself, which is the same follow-up as starting the handshake from the hello.
- **Held paint and bridge-less documents.** A bridge-less document reached by child navigation now stays visible until the 20 s watchdog and is then hidden with the existing error. Before, it was hidden at `load`.
- **Chromium also fires unannounced loads** for some history traversals (seen in the probe), so the proof path is live there too, not WebKit-only.
- **Cross-document message ordering** in out-of-process iframes is assumed only for speed, never for safety. A reviewer should check that claim against `documentAnnounced` being overwritten at bind and at proof.
