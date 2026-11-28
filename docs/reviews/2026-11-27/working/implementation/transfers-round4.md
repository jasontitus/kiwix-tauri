# Lane `transfers` — round 4 (these two items only)

R9 is fixed and R10 is documented. `torrent:: acquisition::` is 221 passed, 0 failed, 3 ignored. Host clippy did not reach exit 0 in six attempts over about 15 minutes; every error was in another lane's file and none was in mine.

---

### TRANSFERS-R9 — shadowed `end` let a no-Range mirror pause unpaced at each range boundary
Disposition: **fixed**

Verification of the finding: confirmed by reading and then reproduced (see Tests).

Change: `src-tauri/src/torrent/web_seed.rs::fetch_ranges` — the outer variable is now `last_end`, and `pace.received` measures what remains up to `last_end`, so the bytes a whole-file answer carries between ranges are held to the pace too. No other behaviour changes.

Tests: `a_whole_file_answer_is_held_to_the_pace_between_the_ranges_it_repairs`
- Setup: first and third pieces missing; the mirror ignores Range, sends the first piece, then holds for 20 s before the gap bytes; window 400 ms.
- With the fix: the response is dropped with "Web seed is sending too slowly", the stage ends after 1 + 5 requests in well under 10 s, the repaired piece is kept and nothing else is touched.
- **Observed failing without the fix**: "ending the stage took 22.582568659s".
- How that was observed without touching the shared tree: inside a private mount namespace I bind-mounted an unfixed copy of `web_seed.rs` over the real path and put an overlay over the target directory, with its writable layer in my scratch directory. The shared sources and the shared target were not written to; the scratch layer is deleted. The two other pace tests still passed in that run.

Not exercised: the real 60 s window on the repair path.

---

### TRANSFERS-R10 — bound for "fast piece, then trickle", and what the floor cannot do
Disposition: **fixed (docs only); no stage deadline added**

Change: `docs/WEB-SEEDS.md`, Residual limits:
- The bound is stated per mirror: roughly 32 passes × (mirrors × 60 s), about 33 minutes for one mirror and about 97 minutes for three.
- It says plainly that a mirror keeping exactly to the minimum pace is never cut off and can hold the stage, and the peers behind it, for as long as the file takes at that rate; that no rate floor can prevent this; and that there is no deadline for the whole stage.
- The Verification list now mentions the repair-path trickle test.

---

## Commands run
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- torrent:: acquisition::` → `test result: ok. 221 passed; 0 failed; 3 ignored; 0 measured; 1270 filtered out; finished in 14.61s`
- Namespace run of the three pace tests against the unfixed file → `2 passed; 1 failed` (the new test, as above)
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 101 on all six attempts. The last attempt's complete error list:
  - `src/store.rs:2456` — method never used
  - `src/crash.rs:530` and `:820` — `is_multiple_of`
  - `src/service_tests.rs:6288` and `:6292` — `MutexGuard` held across an await

  These are lint-phase errors, so the whole crate was linted in that run, and the log contains no mention of `web_seed`, `src/torrent` or `src/acquisition`. I have no exit-0 run for this round; the last clean host and Windows-gnu runs were in round 3, before this one-variable change and its test.

## Files changed in this round
- `src-tauri/src/torrent/web_seed.rs`
- `docs/WEB-SEEDS.md`

New user-visible strings: none.

## Doubts
- The namespace check read the shared target directory through an overlay while other lanes may have been building into it. That can only have affected my private view, and the result (a 22 s wait, matching the mirror's 20 s hold) is what the defect predicts.