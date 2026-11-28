## Verdict for lane reader (round 3): no findings

R4 and R5 are both closed. Lane tests and `tsc` pass. I reviewed `Reader.tsx` as it stands in the working tree against `d98cac2`; I did not re-run the browser attack scripts this round (reasons under "Not verified").

### Your questions
- **Secret issued and honoured only on Windows with restricted mode off — yes.**
  - `photoChooserOffered(location)` (Reader.tsx:574) returns `isWindows && location.restricted !== true`.
  - It is the only source of three things: the `photoChooser` flag (1986), whether `photoSecret` is put in `reader-init` (1999, otherwise the field is absent), and the `photo-request` gate (3692). When the check fails, the reply is `photo-cancel` even if the secret matches.
  - `photoSecretFor` is now called only from that conditional `reader-init` line.
- **Entering restricted mode drops the frame's secret — yes.**
  - Turning restricted mode on rebuilds the document in a new frame with a new session, and its `location.restricted` comes from `open_article`.
  - `initializeDocument` then clears the shell's whole secret map (1987) and sends no secret.
  - A `photo-request` from the old document carries the old session id, so `receive` drops it. Even if one carried the new id, there would be no stored secret to match and the restricted check would refuse it.
  - A child navigation inside restricted mode keeps `location.restricted`, so it gets no secret either.
- **Remaining leak paths — none beyond the known one.**
  - The secret appears only in `reader-init`, and the bridge strips it out before archive code runs. It leaves the bridge only in its own `photo-request` to the shell.
  - The known forged-hello plus `document.open()` case is now limited to Windows with restricted mode off. There its effect is the "asks for a photo" dialog appearing without a click in the page, as assessed last round.
  - Off Windows, and in restricted mode, there is no secret to steal.
- **Honest Windows photo flow — intact.** The bridge receives `photoChooser: true` and a non-empty secret. The shell's check passes, so the "Add a photo" dialog opens and the result flow is unchanged. Off Windows the bridge was already refusing to ask, because it needs both the flag and a secret, so nothing honest is lost.

### Per-fix
- **R4 — closed.**
- **R5 — closed.** Reader.tsx:4985 now reads "…Choose Reload article to open it again. Anything the page had not finished saving may be lost." That matches the button next to it and no longer claims a loss Kiwix cannot know.

### What I ran
- `node --test scripts/test-reader-*.mjs scripts/test-sensor-policy.mjs scripts/test-media-policy.mjs scripts/test-speech-preferences.mjs scripts/test-reference-transitions.mjs` → `# tests 242 # pass 242 # fail 0`
- `npx --no-install tsc --noEmit` → exit 0, no output

### Not verified
- Nothing ran in a real Windows webview.
- I did not re-run `attack2.mjs`: its stand-in shell does not model the platform flag. The scoping above is checked by reading the code and by the new unit tests.