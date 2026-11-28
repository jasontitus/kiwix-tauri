# Lane `reader` — round 2 report (changes since the first report only)

All four reviewer items and the fullscreen handoff are done. The reviewer's attack now fails in both engines.

**One limit you need to know first.** Your decision 2 is implemented as specified, but it does not stop a determined hostile archive. A page that forges a hello and then calls `document.open()` on itself still receives `reader-init`, photo secret included. I reproduced this in both engines. It cannot be closed from JS in this lane; details are under "Still open".

`tsc --noEmit` is clean. Lane tests are 240/240. Real-browser surface tests are 22/22. The full suite has the same two generated-file failures as before.

---

### READER-R1 — archive script could steal `reader-init` and bind first
Disposition: fixed

Verification of the finding: Re-ran the reviewer's `attack.mjs` logic against the round-1 sources through my new unit tests. The hooked-accessor tests fail there and on `d98cac2`. Reading the code showed two more instances of the same hook class that the review did not list:
- The **descendant-channel listener** in `reader-bridge.js` is registered before the shell listener and read `event.data` through the hookable getter on every shell message.
- The **storage bridge's listener** (`storage-bridge.js`) is registered earlier still and read `event.source`, `event.origin` and `event.data` the same way.
- `window.event` exposes the message to any function the bridge calls while handling it, so native getters alone were not enough.

Change:
- **`reader-bridge.js` bootstrap.** Captures the native `MessageEvent.prototype` getters for `data`, `source`, `origin`, a pre-bound `hasOwnProperty`, a pre-bound `stopImmediatePropagation`, the real embedder window and the archive origin. `postToParent` is bound to the captured embedder.
- **Shell listener.** Before anything replaceable runs it:
  - reads source, origin and data natively;
  - reads `channel`, `version`, `type`, `sessionId`, `photoSecret` as own properties only;
  - deletes `sessionId` and `photoSecret` from the message object (the same object every later `event.data` or `window.event.data` returns);
  - stops propagation of every shell message.
- **`reader-init`.** Posts `reading-pan-bind` first, directly through the pre-bound post. The outgoing-session reset now runs in a `try`, so a page that makes it throw only breaks itself.
- **Descendant listener.** Checks origin natively first and returns on shell traffic before any hookable call.
- **`storage-bridge.js` listener.** Same native reads. A message whose own `type` is not one of the five storage types leaves through plain `!==` comparisons only.
- **`Reader.tsx` bind handler (defence in depth).** A second, different `reading-pan-bind` for the same session marks the binding contested (empty token). No proof or pan/zoom is accepted for it and its next load re-sessions. The bridge binds exactly once per session, so honest pages never trigger this.
- **Comments.** The "archive scripts never see reader-init" claims are replaced in both files. They now say secrecy holds only while the bridge's listener exists, and that the probe tells same-document loads from replaced documents for intact bridges and is not an authority boundary against archive script.

Other hook surfaces checked, no change needed:
- `crypto.randomUUID` for the token runs at bootstrap.
- `post()` builds a literal and calls the bound native.
- The photo secret leaves the closure only inside that literal.
- `uuid.test(nonce)` is hookable but sees only the nonce.
- JSON and Array methods are not on any secret path after the scrub.

Tests (all observed failing on the round-1 checkpoint `c0c5efc`, passing now):
- `test-reader-read-aloud.mjs`:
  - "an archive script that replaces the message accessors learns neither the session nor the photo secret"
  - "the bridge binds its token to a new session before anything an archive script can reach"
  - "replaced accessors cannot make the bridge accept a message that is not from the shell"
- `test-reader-storage-bridge.mjs`:
  - "a reader message passing through the storage listener is never handed to replaceable code"
  - "credit from the shell is still accepted after the listener's hardening"
- `test-reader-frame-admission.mjs`: "an archive-authored binding that wins the race is not trusted once the bridge's own arrives" (both orders).

Attack traces, real `reader-bridge.js`:

| Probe | Engine | Result |
|---|---|---|
| Reviewer's `attack.mjs`, unmodified | Chromium 158 | `archive script holds S=false photoSecret=false and the bound token=false`; trace `hello,load,RESESSION(replaced at load),bind,ready`; `session unchanged=false` |
| Reviewer's `attack.mjs`, unmodified | WebKitGTK 2.52.6 (`xvfb-run`) | identical |
| My `attack2.mjs`, steps 1–3 | both | hooks fired during `reader-init` handling (`window.event@querySelectorAll`, `proto.sensorStatus`, …) but held session id=false, photo secret=false; binding not the attacker's and not contested; same-document load `probe,PROOF ACCEPTED`; real navigation re-sessioned |
| `attack2.mjs`, step 4: `document.open()` with no announcement | both | `load,probe,BIND CONTESTED,BIND CONTESTED,RESESSION(unproven),init withheld`; the replacement received only `document-probe` |
| `attack2.mjs`, step 5: forged hello then `document.open()` | both | replacement received `reader-init+photoSecret` |

`attack2.mjs` adds `storage-bridge.js` in production order, hooks on accessors, `Object.prototype` getters, `stopImmediatePropagation`, about 60 common methods peeking at `window.event`, `parent` redefinition and three kinds of listener. Its stand-in shell copies the current rules. It lives at `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/reader/attack2.mjs`; outputs are `attack2-chromium.txt` and `attack2-webkit.txt` beside it.

Not exercised: the production `Reader.tsx` in a real webview. Both attack shells are copies of the rules.

### Owner decision 2 — never post `reader-init` to an unannounced document
Disposition: fixed as specified, with one necessary addition. Partial against hostile script (see "Still open").

Change (`Reader.tsx`):
- **Vouch gate.** `bridgeVouched` is set by a trusted hello from the frame's own window at the displayed origin. It is cleared when `reader-init` is posted, when the shell assigns `src`, and on a new iframe. `admitDocument` posts init only when vouched; otherwise the session waits in `pendingInit` and is released by the next hello. This applies to both the first branch and the re-session branch.
- **Own late hello.** A document's own hello arriving after its init is cancelled at its bind, so it cannot vouch for whatever the frame holds next. This matters because WebKit delivers hello after load for fast documents.
- **Addition, to avoid a deadlock.** A same-document reopen (the shell re-assigns a fragment URL) makes no new hello. On that first load the shell probes the bridge already bound in this frame, same context and origin only, and a valid proof vouches.
- **Late proofs.** After a proof-window timeout, a late valid proof no longer keeps the old session; it only vouches so the replacement session reaches the still-live bridge. This keeps a bridge that was merely busy for over 1 s from dead-ending in the watchdog.
- **Unbound sessions.** A load for a session no bridge has bound yet now waits for the bind, then probes, instead of re-sessioning. Its id is still never named to the frame.
- **Contested or silent documents.** They stay unadmitted and hidden and end in the existing 20 s watchdog error.

Honest-page deadlock check:

| Flow | Outcome |
|---|---|
| New iframe, hello before load (Chromium) | init at load |
| New iframe, load before hello (WebKit) | init at hello |
| Child navigation, either order | re-session, then init |
| Same-document reopen | init after liveness proof |
| Hash set during load | waits for bind, then proved |
| Bridge busy longer than 1 s | recovers via late proof, with a blink |

BFCache:
- The bridge now repeats its hello on `pageshow` with `persisted`.
- With that hello, the restored document is re-sessioned at its load and handed a session of its own.
- Without it, nothing is posted until it speaks.
- If an engine restores without firing the frame's `load`, the shell does not notice until the next load. That is unchanged from base and belongs to the deferred "handshake from hello" work.
- I could not produce a real iframe BFCache restore in either engine; this is unit-tested only.

Tests (`test-reader-frame-admission.mjs`, 37 pass):
- "reader-init is never posted to a document no bridge has vouched for"
- "a withheld initialisation is not released to a retired frame"
- "an unannounced load nobody proves loses its session, is hidden, and is handed nothing"
- "a late proof cannot bring the session back; it only lets the bridge be handed the replacement"
- "a session no bridge has bound yet is never named in a probe"
- "a same-document reopen is initialised once the bridge already in the frame proves it is listening"
- the BFCache test, rewritten for both variants
- bridge side: "a document restored from the back-forward cache announces itself again"

Ten of these fail on `c0c5efc`.

### READER-R2 — held paint showed unadmitted documents
Disposition: fixed, plus a bound

Change:
- A proof-window timeout re-sessions with `painted = false`.
- Paint is held only when the replacement was announced: replaced at load, or a hello during the probe.
- Because a hello can be forged before navigating to a foreign page, the hold is also capped at `HELD_PAINT_LIMIT` (3 s). A real handshake finishes well inside that.

Tests: the paint test now also covers the silent load (hidden at timeout), an announced-but-stalled replacement (hidden at 3000 ms), and the WebKit late-announce path.

Not exercised: at runtime in a webview.

### READER-R3 — contradictory advice on the Reload path
Disposition: fixed by rewording. I did not auto-discard: it would hide a possible data loss.

Change: `reopenFreshDocument` now shows a string that matches the button beside it (below). `changeSensorDecision` keeps the old wording.

Tests: `test-reader-fresh-document.mjs` asserts the new advice and that "Close and reopen this window" is absent.

### Desktop handoff — macOS fullscreen shortcut
Disposition: fixed

Change: the shortcuts table shows `⌃⌘F` when `isMacos` and `F11` elsewhere.

Handler decision: I added no `Ctrl+Meta+F` mapping to the Reader's keyboard handler.
- On native desktop the handler returns early for everything except Escape (`if (isDesktop && name !== "escape") return;`), so the menu owns the accelerator. `isMacos` implies `isDesktop`.
- In the non-native preview the browser owns Ctrl+Cmd+F, and `isMacos` is false there.

No test added; it is one conditional label.

---

## Still open
`reader-init` is posted to the window, so whoever listens there reads it. Two ways a page can receive it without a live bridge, neither closable in this lane:
1. **Forged hello, then `document.open()` on itself.** Reproduced in both engines (step 5 above). The shell cannot tell a forged hello from a real one: both come from the frame's window at the archive origin.
2. **Announce, then `document.open()` during the initial load.** The hello is genuine; the listener is gone by the time init arrives. Needs no forgery.

What was closed: pages with an intact bridge (the R1 hook class) and documents that never announce (bridge-less, foreign origin, `document.open()` after a completed handshake).

Closing the rest needs a server-attested per-document identity (Rust). I considered delivering `reader-init` over a `MessagePort` carried by the bridge's hello and did not build it: by my analysis it closes variant 2 for shell-opened frames but still loses to a port parked in a SharedWorker for child navigations. That is reasoning, not something I tested. I recommend treating the photo secret as defence against non-hostile misuse until then.

## Commands run
- `npx --no-install tsc --noEmit` → no output.
- `node --test scripts/test-reader-*.mjs scripts/test-sensor-policy.mjs scripts/test-media-policy.mjs scripts/test-speech-preferences.mjs scripts/test-reference-transitions.mjs` → `# tests 240 # pass 240 # fail 0`.
- `node --test scripts/test-*.mjs` → `# tests 514 # pass 501 # fail 2 # skipped 11`. The two failures are the generated `docs/ERRORS.md` check and the localization manifest check.
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-frontend-surfaces.mjs scripts/test-shell-surfaces.mjs scripts/test-interface-accessibility.mjs scripts/test-startup-navigation.mjs` → `# tests 22 # pass 22 # fail 0`. `test-startup-navigation` runs against the existing `dist/`, which I did not rebuild.
- `node adv-reader/attack.mjs` (Chromium), the same under `xvfb-run ../READER/wk` (WebKitGTK), and `attack2.mjs` in both. Traces above.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented.`

## Files changed this round
- `src/reader-bridge.js`
- `src/storage-bridge.js` (new to the lane's diff)
- `src/components/Reader.tsx`
- `scripts/test-reader-frame-admission.mjs`
- `scripts/test-reader-read-aloud.mjs`
- `scripts/test-reader-storage-bridge.mjs`
- `scripts/test-reader-fresh-document.mjs`
- `scripts/test-reader-video-capability.mjs` (harness stubs only)

New user-visible strings, to be collected with `node scripts/localization.mjs generate`:
- `"Kiwix could not confirm that this page finished saving, so it was closed. Choose Reload article to open it again. Anything the page had not finished saving is lost."`
- `"⌃⌘F"`
- From round 1, still uncollected: `"Printing is not available on this device. Save or share the article as a PDF instead."`

## Handoffs / contract changes
- The bridge now stops propagation of every `kiwix-reader` message from the shell and removes `sessionId` from it. Archive listeners no longer see shell traffic. Nothing in the repo depended on that.
- `storage-bridge.js` is embedded by `server.rs` through `include_str!`, like the reader bridge. No Rust edits.
- `docs/PLATFORM-GAPS.md` item 8 is still stale from round 1.

## Doubts
- **Complexity.** The admission state machine is now larger: `documentAnnounced`, `bridgeVouched`, `pendingInit`, and three probe modes. Every path I could think of is tested, but only in the extraction harness. A second adversarial pass on `receive`, `onLoad`, `probeDocument` and `admitLoadedDocument` is warranted.
- **Self-inflicted dead end.** A page that forges a second bind strands itself: contested binding, then re-session, then no vouch, then watchdog error. Honest pages cannot trigger it.
- **Slow first handshake.** A bridge slower than 1 s after an unannounced load now blinks: hidden, then recovered by the late proof. Before round 2 it was re-sessioned immediately.
- **Lingering forged vouch.** A forged hello leaves `bridgeVouched` true until the next init. This is the same residual as variant 1 above, not a separate hole.
- **Target-phase ordering not relied on.** I hardened the storage and descendant listeners rather than depending on capture listeners firing first at the target. I did not test an engine old enough to order them differently.
