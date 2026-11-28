## Verdict for lane transfers (round 4): no findings — lane closed, apart from host clippy, which still has no exit-0 run

R9 is fixed, the new test exercises the no-Range 200 repair path, and the R10 doc text is accurate. The lane's tests pass. Host clippy failed only in another lane's file.

**R9 — closed.**
- `fetch_ranges` now names the overall end `last_end` (`web_seed.rs:1032`).
- Both `pace.start(last_end.saturating_sub(at))` (`:1033`) and `pace.received(…, last_end.saturating_sub(at + chunk.len()))` (`:1057`) measure to it. The loop's own `end` is used only to bound each range's writes.
- So bytes owed between ranges are now held to the pace.

**The new test exercises the right path.** It is `a_whole_file_answer_is_held_to_the_pace_between_the_ranges_it_repairs`:
- The staged file is full length with pieces 0 and 2 zeroed, so the stage goes straight to hash and repair.
- `paced_mirror_with(5, 20 s, ranges = false)` makes the mirror answer with the whole file as a 200, through the `answer(BODY, "", None)` call. That is the no-Range 200 branch of `fetch_ranges`, with both ranges served from one answer.
- The first chunk (bytes 0..5) completes the first range; the mirror then holds for 20 s before the gap bytes.
- With the fix, 5 bytes are still owed within the 400 ms window, so the response is dropped. The stage then ends after 1 + 5 requests, in well under 10 s.
- Without the fix, the remaining count would be measured to 5, so nothing would be owed and the stage would wait out the 20 s hold. That matches the implementer's observed failure of 22.58 s. I did not repeat their revert.
- It also checks that the repaired piece is kept and that piece 2 is left untouched.

**The R10 doc text is accurate** (`docs/WEB-SEEDS.md`, around lines 89–97).
- The bound is stated per mirror: 32 × (mirrors × 60 s), about 33 minutes for one mirror and about 96–97 minutes for three including backoff. That matches the code: a pace failure is transient, the pass goes on to the next mirror, and a pass that added a piece resets the stall counters.
- The doc says a mirror keeping exactly to the floor is never cut off, can hold the stage and the peers behind it, and that there is no deadline for the whole stage. That matches the code: `Pace::received` re-arms the window whenever the quota is met, and nothing else limits a single response.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- torrent:: acquisition::` → `test result: ok. 221 passed; 0 failed; 3 ignored; 0 measured; 1284 filtered out; finished in 19.24s`. That includes `torrent::web_seed::tests::a_whole_file_answer_is_held_to_the_pace_between_the_ranges_it_repairs ... ok`.
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 101.
  - The only errors are in `src/crash.rs:530` and `:820` (manual `.is_multiple_of()`), which belong to another lane.
  - These are lint-phase errors, so the whole crate was linted, and the log has no mention of `src/torrent` or `src/acquisition`.
  - There is still no exit-0 host clippy run that includes this round's change.
- I did not re-run Windows-gnu or Android this round; the change is one variable and a test.

**Lane residuals for the owner**
- **Peers wait for the web-seed stage.**
  - A mirror that keeps exactly to the 128 KiB / 60 s floor (about 2 KiB/s) can hold the stage, and the swarm, for as long as the file takes at that rate.
  - A "fast piece, then trickle" mirror holds it for up to about 33 minutes per listed mirror.
  - Dead mirrors cost about 15 s to 4.5 minutes per start.
  - There is no deadline for the whole stage. Closing this needs an owner decision: a stage deadline, or peers running alongside the web seed.
- **Hash cost.** A full-length staging file is SHA-1 hashed in full by the stage on every start, then again by the engine, with no progress shown.
- **Split archives.** A direct `.zimaa` download is refused, and its message still suggests Retry, which cannot help.
- **Untested paths.**
  - The multi-file and BEP 47 padding repair paths are covered by reading only.
  - The real 60 s / 128 KiB values, the 120 s stall limit and the 15 s connect timeout are not exercised; tests use short windows.
- **Not compiled or run here.**
  - iOS and macOS builds: the ACQ-3 branch and the R5 `libc`/`nix` errno code.
  - Windows and Apple errno values at runtime.
  - Any host that has IPv6.
- **Not reproduced.** I never saw a real middlebox truncate a response; the R1 header check was exercised only by fixtures.
- **Integration work.**
  - `docs/ERRORS.md` needs `npm run docs:errors`; it is out of date across the whole tree because it is keyed by line number.
  - Host clippy currently fails in other lanes' files (`crash.rs`, and earlier `localswarm/mod.rs:24`, `store.rs`, `service.rs`). It needs one clean exit-0 run across all lanes before release.

Logs are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-transfers/`: `test4.log` and `clippy-host4.log`.