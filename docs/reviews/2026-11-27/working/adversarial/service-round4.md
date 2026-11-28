## Verdict for lane service (round 4): no findings — the lane is closed

Both round-3 findings are fixed. I re-ran my notice reproduction against the new code, the node test, tsc and the lane tests, and all are clean. I did not re-run either clippy this round; the implementer reports both as clean.

**SERVICE-R3-1 (a re-added notice stayed hidden): closed.**
- I updated my reproduction script for the new API: `adv-service/notice-repro2.mjs`.
  - Both notices are shown and then dismissed; after the cancellation notice is withdrawn, nothing is visible.
  - When the same cancellation is noted again, the band shows it: `second cancellation visible: ["Import cancelled because …"]`. The old code printed `false` here.
  - A missing snapshot keeps the closed notices hidden.
- The band now lists only notices the person has not closed. The pruning effect returns the same array when nothing changes, so it causes no re-render loop.
- One narrow edge (Nit, needs no action): if a notice is withdrawn and noted again before the shell takes a new snapshot, it stays hidden. The shell refreshes after every import and every privacy switch, so this is unlikely.

**SERVICE-R3-2 (an unrelated import withdrew the notice): closed.**
- The `cancelled_imports` set records the archive's UUID (or its canonical path if the file no longer opens). The key is taken before the staged copy is discarded.
- Only books actually admitted are removed from the set, by UUID and by canonical path. The notice is withdrawn only when the set becomes empty.
- An unrelated import, or reopening a different registered book through the LIB-3 shortcut, leaves the notice in place.
- The key is computed inside `import_paths`, which already runs off the async runtime.

**What I ran**
- `node adv-service/notice-repro2.mjs`: output as described above.
- `node --test scripts/test-startup-notice.mjs`: `pass 3, fail 0`.
- `npx --no-install tsc --noEmit`: exit 0.
- Lane tests (`service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf:: store::`): `test result: ok. 484 passed; 0 failed; 6 ignored`.

**Lane residuals**
- **LIB-7:** if this run never saw a monitored folder with its archives in it, entries deleted while the app was closed are kept as unavailable; the person can remove them by hand. Windows behaves as before. A file imported from the directory underneath an unmounted mount point is retired once the volume covers it again.
- **Provider-backed books** get no detection of a file renamed over them, by design.
- **Cancelled staged copies:**
  - A copy that can no longer be opened is kept, because the app cannot prove which files belong to it (accepted leak).
  - If Android kills the process in the background, the incoming share's permission may be gone after the copy was discarded. The person then shares again; the original is untouched.
- **Mobile:** manual imports and incoming copies are cancelled when the app goes to the background or the screen locks (owner decision). The person adds the archive again, or the incoming copy is offered again.
- **Startup band:** the heading "Some saved items need attention." does not fit import cancellations (Nit).
- **Docs:** `docs/ERRORS.md` needs regenerating (its check fails repo-wide), and `docs/STATE_MACHINES.md` should mention import cancellation. `mobile_policy.rs:566` still lists the removed `localswarm_hotspot_start`, which is harmless.
- **Integrity test flake:** it failed once under load and has not been reproduced since. I have no panic line for it.
- **Not verified here:** no iOS compile, no device runs (lifecycle, provider descriptors, real mounts), and the band has not been seen rendered in a real shell.

This is a closed review of the lane's changes, not a release or parity certification.