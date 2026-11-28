## Verdict for lane transfers (round 3): only non-blocking findings

R7, R8 and the trickling-mirror fix are closed. I found one new small defect in the new pace code (R9), and the ~33-minute bound needs correcting for more than one mirror (R10). On your 128 KiB question: the reasoning holds for honest links, and it is no more exploitable than a piece-per-60 s floor would be.

### TRANSFERS-R9 — Pace in repair: a shadowed `end` lets a no-Range mirror pause unpaced at every range boundary
Severity: Nit (small, but a real defect in the new code)
Fix under review: trickling-mirror fix
Evidence class: source-traced
Location: `src-tauri/src/torrent/web_seed.rs`, `fetch_ranges`:
- `let end = ranges.last().map_or(at, |range| range.2);` is the overall end.
- `for &(_, start, end) in ranges` then shadows it with the current range's end.
- `pace.received(chunk.len() as u64, end.saturating_sub(at.saturating_add(chunk.len() as u64)))` uses the shadowed one.

Trigger: in repair, a mirror that ignores Range answers `200` for a file with several missing ranges.

Causal path:
- Inside the loop, "remaining" is measured to the end of the current range, not to the end of the last one. The comment just above says "Everything up to the end of the last range must arrive".
- When a chunk that meets the quota crosses the current range's end, `received(bytes, 0)` calls `start(0)` and sets `owed = 0`.
- The loop then reads gap bytes for the next range under `pace.missed()`, which with `owed == 0` is `pending()` for ever. Only the 90 s read timeout bounds that read.
- The pace resumes at the next chunk. So each range boundary allows one free silent read of up to about 90 s. A file with thousands of scattered missing ranges therefore allows thousands of such pauses.
- The `206` path and `download_file` are not affected.

Smallest correct fix: rename the outer variable (for example `last_end`) and pass `last_end.saturating_sub(...)` to `received`. A test with two ranges, a no-Range mirror and a hold in the gap would catch it.

### TRANSFERS-R10 — The "fast piece, then trickle" bound is about 33 minutes per mirror, not per stage
Severity: Nit (the figure is only in the round-3 report; `docs/WEB-SEEDS.md` does not state it)
Fix under review: trickling-mirror fix
Evidence class: source-traced
Location: `web_seed.rs`, the per-mirror loop in `download_once` and `repair`, and the pass accounting in `download_with`

Trigger: every mirror in the url-list alternates one fast piece with a trickle.

Causal path:
- A pace failure is `Transport`, so the same pass goes on to the next mirror. Each mirror can add a piece and then hold its connection for up to about 60 s.
- The pass counts as advancing, so the stall counter and the 120 s limit reset. Only `SEED_MAX_PASSES = 32` applies.
- The bound is therefore about 32 × (number of mirrors × 60 s) plus backoff: about 33 minutes for one mirror and about 97 minutes for three (Kiwix torrents list three).

Smallest correct fix: state the bound per mirror. If about 1.5 h behind three hostile mirrors is too long, add an absolute deadline for the whole stage.

**On the 128 KiB per 60 s choice (owner note)**
- **Honest links:** a fixed amount is right. A piece per 60 s would require 68 KiB/s with 4 MiB pieces, or about 1.1 MiB/s with the 64 MiB maximum the parser accepts, and would cut off honest slow mirrors.
- **Hostile mirrors:** any rate floor can be met by a mirror that wants to hold the stage. A mirror at exactly 2.1 KiB/s keeps one response, and the swarm, waiting for as long as the file lasts: about 1.4 years for 90 GB. With a piece-per-60 s floor it would be about 16 days. Neither is a real bound.
- **What would bound it:** an overall deadline for the stage, or starting peers alongside the web seed. That is an owner decision; it is already disclosed in WEB-SEEDS.md ("peers then wait for as long as the web seed takes").
- **Conclusion:** the threshold is not wrong; it closes the accidental trickle case it targets.

**Per-fix assessment**
- **R7 — closed.**
  - `ZimLength::EndsEarly if !snapshot.segmented && snapshot.expected_size != Some(downloaded)`: a Metalink or shared-library row with exactly its advertised size now gets `ZIM_LENGTH_MISMATCH`, and its total is kept.
  - Remaining cases where Retry is still offered but cannot help:
    - The first part of a `.zimaa` split archive (known, acceptable).
    - A direct link whose server-side file is itself truncated, with a Content-Length equal to the bytes received. Retry gets a 416 and fails either way, and the message's "if it stops early again, cancel it" covers it.
  - Optional refinement (Nit): offer Retry only when the response declared no length, which needs a flag recording that the total came from EOF.
- **R8 — closed.** "The stage ends at once when no mirror's failure in a pass was transient" matches `SeedError::after`.
- **Trickling mirror — closed, apart from R9.**
  - `Pace` is passed fresh per response on both paths: `download_file` (front to back) and `fetch_ranges` (repair).
  - `pace.missed()` sits in the same `select!` as `token.cancelled()` and `stream.next()`, so a pause or cancel returns at once.
  - Once `owed == 0` (no bytes expected), `missed()` is `pending()`, so a complete response is never dropped for a late close. If such a close is then cut by the read timeout, `download_once` still accepts the file, because it only checks `current >= file.length`.
  - The window starts at the body start; time to the first byte is still covered by the read timeout.
  - The doc figures are right: a trickle-only mirror costs 60 + 60 s, about 2 minutes, for one mirror; with three, 180 s already exceeds the 120 s limit, so about 3 minutes.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- torrent:: acquisition::` → `test result: ok. 220 passed; 0 failed; 3 ignored; 0 measured; 1270 filtered out; finished in 14.24s`.
- That includes `ok` for `an_object_of_the_exact_advertised_size_with_a_longer_header_is_not_offered_a_retry`, `a_mirror_that_only_trickles_is_dropped_and_ends_the_stage` and `a_slow_mirror_that_keeps_the_pace_is_not_interrupted`.
- I did not re-run clippy this round.

**What I could not verify**
- The real 60 s / 128 KiB values at runtime; the tests use a 400 ms window and 5-byte pieces.
- The pace on the repair path: there is no dedicated test, and that is where R9 sits.
- That R7's test would fail without the fix (the implementer infers it; I did not revert).

Log: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-transfers/test3.log`