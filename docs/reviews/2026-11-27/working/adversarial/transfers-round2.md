## Verdict for lane transfers (round 2): only non-blocking findings

All of R1–R6 are closed; only two Nits remain, below. One question is left for the owner: a mirror that trickles data can still hold the swarm back with no time limit. That was already true at base.

### TRANSFERS-R7 — "Retry to fetch the rest" is shown where Retry cannot help
Severity: Nit
Fix under review: R1
Evidence class: source-traced
Location: `src-tauri/src/acquisition/transfer.rs` (`ZimLength::EndsEarly if !snapshot.segmented` arm, about line 752); `ZIM_ENDED_EARLY` in `finalize.rs`
Trigger, either of:
- A Metalink or shared-library row whose bytes equal the exact `expected_size`, but whose ZIM header declares a larger file (the mirror served the wrong object).
- The first part of a split archive (`.zimaa`) downloaded by direct URL.
Causal path:
- Both cases return `ZIM_ENDED_EARLY` and clear `job.total`, but `expected_size` still holds.
- Retry then fails again. Without a validator it refetches the same bytes; with one it gets a 416, and if the mirror sends no validator the user sees the technical "416 response does not prove…" error.
- No data is at risk. The text "if it stops early again, cancel it" eventually leads the user to the right action.
Smallest correct fix: when `expected_size == Some(downloaded)`, return `ZIM_LENGTH_MISMATCH` instead of `ZIM_ENDED_EARLY`.
- That covers the Metalink and shared-library case only. A `.zimaa` part has no exact size, so it would still get the Retry message.
- Refusing `.zimaa` itself is acceptable: `safe_name` forces a `.zim` destination, a lone first part cannot be opened (`validate_layout` rejects it), and the later parts already failed the magic check.

### TRANSFERS-R8 — "Any other answer ends the stage at once" is slightly too strong in the docs
Severity: Nit
Fix under review: R4
Evidence class: source-traced
Location: `docs/WEB-SEEDS.md` Flow step 5; `src-tauri/src/torrent/web_seed.rs` (`SeedError::after`)
Trigger: one mirror gives a definite refusal (for example 404) while another fails transiently.
Causal path: within a pass every mirror is still tried, and `SeedError::after` lets a transient failure win. A 404 from one mirror therefore does not end the stage when another mirror is retryable.
Smallest correct fix: change the doc to "…ends the stage once no mirror's failure was transient".

**Per-fix assessment**
- **R1 — closed.**
  - The rule matches `archive::validate_layout` exactly: `mime_list_pos != 72 && checksum_pos + 16 != size` is rejected.
  - It also matches libzim's checksum-position rule as I recall it; I did not open the libzim sources. Offsets 56..64 and 72..80 match zimru's `header.rs` table.
  - zimru's own `has_checksum()` is `checksum_pos != 0`, which differs. But an archive with `mime_list_pos` 80 and `checksum_pos` 0 is already rejected by the app's own `validate_layout` when opened, so no archive the app can open is newly refused.
  - Legitimate archives still pass:
    - Version-5 headers: the rule does not depend on the version.
    - `mime_list_pos == 72` is treated as `Whole`.
    - All arithmetic is u64 with `checked_add`, so a 4 GiB+ file on 32-bit Android is fine.
  - Coverage of publication paths:
    - Single-stream and segmented completion in `execute_transfer` run the check before the receipt shortcut and before hashing.
    - `finalize_durable_download` (startup and sweep, which also finishes iOS native adoption) runs it before the receipt shortcut, after its `total < 80` guard.
    - Shared-library downloads go through `execute_transfer`.
    - LocalSwarm already has its own header length check plus `check_integrity_cancellable` (`localswarm/mod.rs:1703-1708`).
    - Torrent publication does not need it: every byte is SHA-1 verified against the metadata, and an archive described short by its torrent is rejected when opened.
  - Clearing `job.total` on an early end leaves no later length unchecked:
    - A continued 206 is gated by If-Range and the validator.
    - A 200 truncates the kept bytes and is checked against its own Content-Length.
    - The final header check runs again.
    - `expected_size` (Metalink or shared-library) is never cleared.
  - The complete-part check seeks back to `downloaded` before the request path, at about line 193.
- **R2 — closed.** A part counts as complete only if it is at least 80 bytes, has the magic and passes `zim_length == Whole`. Anything else takes the ordinary If-Range or full-request path. The test covers a page with and without a validator, and a 20-byte page.
- **R3 — closed, but trickle is a residual for the owner.**
  - The pass cap of 32 bounds the stage across passes; `stalled_for`, the 120 s limit, excludes backoff waits and the full hash pass.
  - The arithmetic in `before = state.sequential - state.gained` cannot underflow.
  - The delay figures in the docs follow from the timeouts. For example, three mirrors that drop connection attempts silently: 45 s per pass, the limit is reached after three passes, plus 1 + 2 s of backoff ≈ 138 s.
  - On your question: the trickle case really is unbounded within one pass. reqwest 0.12.28 and 0.13.5 document `read_timeout` as "applies to each read operation, and resets after a successful read". A mirror sending one byte every 89 s keeps a request alive for as long as the file lasts, and peers wait the whole time.
  - This is unchanged from base and is documented in WEB-SEEDS.md.
  - If you want it closed: abort a response that delivers less than `min(piece_length, remaining)` within a fixed window (for example 60 s) and count that pass as stalled.
- **R4 — closed**, apart from the R8 wording.
- **R5 — closed.**
  - In the vendored crate, `nix::errno::Errno::{EAFNOSUPPORT, EPROTONOSUPPORT} as i32` sits under `cfg(unix)`, matching its `cfg(unix)` nix 0.32 dependency. `nix::errno` is not behind a feature flag. Windows uses `raw_os_error() == Some(10047)`, which is `WSAEAFNOSUPPORT`. There is also a `not(any(unix, windows))` arm.
  - In the app, `libc` is an unconditional dependency and its two constants exist on Linux, Android and Apple. Windows uses 10047.
  - The probe now only creates the sockets.
  - My first runs of host, Windows-gnu and Android clippy each showed exactly one error, the unused import in `localswarm/mod.rs:24`. That is a late lint, so these files passed type-checking and linting on all three targets.
  - Not compiled for Apple.
- **R6 — closed.** The test uses a 30 s wait and asserts the call returns in under 10 s, so a wait that ignored cancellation would fail it. The implementer reports watching it fail that way (30.001 s); I did not re-run that revert.

**Implementer's doubts**
- **Dead-mirror timings:** acceptable. They are derived from the timeouts, not measured, and the docs say so.
- **Trickle without closing:** needs an owner decision (see R3). It is real and was already true at base.
- **Stage ends after 32 passes and waits for the next start:** acceptable; the bytes are kept.
- **R1 trusts the header's own `checksum_pos`:** acceptable. Without an advertised digest nothing else can catch a header altered to match.
- **`EPROTONOSUPPORT`:** harmless to include.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- torrent:: acquisition::` → `test result: ok. 217 passed; 0 failed; 3 ignored; 0 measured; 1231 filtered out; finished in 17.81s`
- The full suite, `cargo test --locked … --lib` → `test result: ok. 1435 passed; 0 failed; 16 ignored; 0 measured; 0 filtered out; finished in 127.79s`. This confirms the header rule breaks no other module's fixtures.
- **Windows-gnu clippy**, built from scratch in my own target dir (`…/scratchpad/adv-transfers/target-win`), 509 crates compiled:
  - Exit 101 on one error only: `unused import: reachable_addresses` (`localswarm/mod.rs:24`, another lane).
  - A retry also hit `update.rs:662` E0382 (another lane, mid-edit).
- **Host clippy** `--lib --tests -D warnings`:
  - First run: exit 101 on the same single localswarm error.
  - Later retries also failed in `store.rs:3074` (`state_bytes`), `server.rs:163` and `service.rs` (other lanes, mid-edit).
- **Android type-check**:
  - First run: exit 101 on the single localswarm error.
  - A retry also failed on `dispatch.rs:1915` `sweep_spelling_cache` (another lane's hunk, not the iOS branch).
- None of these clippy runs reported anything in `torrent/`, `acquisition/`, `mobile_service.rs` or the iOS `start_download` branch.
- I retried five times over roughly 25 minutes. The localswarm unused import was present in every run, including committed checkpoints, so it is a real defect in the net lane, not a transient edit.
- **What you need to know:** I have no clean exit-0 clippy run on any target. My evidence is the late-lint reasoning above, not exit codes.

**What I could not verify**
- iOS and macOS compiles of the R5 `libc` and `nix` code, and the ACQ-3 branch.
- Windows and Apple errno values at runtime.
- The 120 s stall limit and the 15 s connect timeout at runtime.
- A real truncation by a middlebox.

**Integration owner**
- `node --test scripts/test-error-docs.mjs` fails tree-wide: 3659 rows missing and 3191 stale, because `docs/ERRORS.md` is keyed by line number. It needs `npm run docs:errors` at integration and is not specific to this lane.
- Logs are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-transfers/`: `test2.log`, `testfull.log`, `win.log`, `win2.log`, `clippy-host.log` to `clippy-host3.log`, `android3.log`, `android4.log`.