# Lane `transfers` — round 3 (these items only)

All three items are done. `torrent:: acquisition::` is 220 passed, 0 failed, 3 ignored; host clippy and Windows-gnu clippy both exit 0.

One point departs from your wording: the pace is a fixed 128 KiB per 60 s, not a whole piece per 60 s. Kiwix torrents use 4 MiB pieces, so a piece a minute would demand about 68 KiB/s and cut off honest mirrors on slow links. Details are under item 3.

---

### TRANSFERS-R7 — "Retry to fetch the rest" shown where Retry cannot help
Disposition: **fixed**

Verification of the finding: confirmed by reading. With an exact size, every promised byte has arrived, so a header that declares more means the mirror holds the wrong object.

Change: `acquisition/transfer.rs` — the early-end arm now also requires `snapshot.expected_size != Some(downloaded)`. A row with an exact size gets `ZIM_LENGTH_MISMATCH` ("…Cancel it and start a new download") and its total is left alone.

Tests: `an_object_of_the_exact_advertised_size_with_a_longer_header_is_not_offered_a_retry` — a Metalink download of the exact advertised size and digest, whose header declares a longer file, fails with the cancel message and keeps its total. Passes. Failing without the fix is **inferred, not observed** (the assertion is on the message constant the arm selects).

Not covered, as the reviewer noted: a `.zimaa` first part has no exact size and still gets the Retry message.

---

### TRANSFERS-R8 — docs wording on when the stage ends
Disposition: **fixed**

Change: `docs/WEB-SEEDS.md` Flow step 5 now says each pass tries every mirror, and "The stage ends at once when no mirror's failure in a pass was transient."

---

### Trickling mirror — closed
Disposition: **fixed, with a different threshold from the one suggested**

Verification of the finding: confirmed. The 90 s read timeout restarts on every read, so nothing ended such a response.

Change: `torrent/web_seed.rs`
- New `Pace`, applied to every response body in both the front-to-back download and the in-place repair. Once a body starts, the mirror owes `min(128 KiB, piece_length, bytes remaining)` in each 60 s window; meeting it starts the next window.
- Falling behind drops the response with the transient error "Web seed is sending too slowly". The next mirror is then tried in the same pass.
- The wait is in the same `select!` as the cancellation token, so a pause or cancel ends it at once.
- Nothing is owed once no more bytes are expected, so a complete response is never dropped for a late close.

Why 128 KiB and not a whole piece:
- I read the piece length from three public Kiwix `.torrent` files (17 MB, 52 MB and 124 GB archives): all three are 4 MiB.
- A piece per 60 s would therefore be a floor of about 68 KiB/s (about 560 kbit/s). The parser accepts pieces up to 64 MiB, where it would be about 1.1 MiB/s.
- 128 KiB per 60 s is a floor of about 2.1 KiB/s whatever the piece size. Below that a download of any useful size is not practical.

Why 60 s: it is shorter than the 90 s read timeout, so it catches what that timeout cannot. Two missed windows reach the existing 120 s stall limit, and it is long enough to sit out a short stall that recovers.

Resulting bounds (derived, not measured):
- A mirror that only trickles: about 2 minutes for one mirror, about 3 for three, then peers start.
- A pass that had already added a whole piece before it fell behind still counts as advancing, as it does after a dropped connection. A mirror that alternates one fast piece with a trickle is therefore bounded by the 32-pass cap, roughly 33 minutes. I kept one definition of "advancing" rather than ending the stage on a brief slump after hours of good transfer.
- A mirror that stays at or above the floor is never cut off, so peers wait as long as the web seed takes. That is by design and now stated in the docs.

Tests:
- `a_mirror_that_only_trickles_is_dropped_and_ends_the_stage` — with a 400 ms window, a mirror that sends one byte then holds for 20 s ends the stage in five requests with the slow error; the bytes are kept. It also shows a stop inside a response returns at once.
- `a_slow_mirror_that_keeps_the_pace_is_not_interrupted` — one piece every 100 ms against a 400 ms window completes in a single request.
- Fail-without-fix: no revert was done in the shared tree or a copy. Instead the trickle test contains its own control: the same production function with an effectively unlimited window is still waiting when the test gives up on it (observed passing). That the full stage would overrun without the pace is inferred from that.

Docs: `docs/WEB-SEEDS.md` now describes the pace, lists the two new tests, and replaces the "trickle is unbounded" residual with the figures above.

Not exercised: the real 60 s window and 128 KiB amount (tests use a short window and 5-byte pieces); the repair path's pace (same code, no dedicated test).

---

## Commands run
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- torrent:: acquisition::` → `test result: ok. 220 passed; 0 failed; 3 ignored; 0 measured; 1261 filtered out; finished in 14.87s`
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → EXIT 0 on the fourth attempt. Earlier attempts failed only in other lanes' files (`store.rs:3074`, `library_files_tests.rs`, then `service.rs:211`).
- Windows-gnu clippy (the brief's command) → EXIT 0, first on my third attempt and again on the fourth.

## Files changed in this round
- `src-tauri/src/acquisition/transfer.rs`
- `src-tauri/src/acquisition_tests.rs`
- `src-tauri/src/torrent/web_seed.rs`
- `docs/WEB-SEEDS.md`

New user-visible string: none in the frontend. "Web seed is sending too slowly" reaches only the log line the manager already writes.

## Doubts
- The 4 MiB figure rests on three torrents from one host; StreetZim map torrents were not checked. The pace does not depend on it, which is why I fixed the byte count.
- About 2 KiB/s is a judgement. A peerless torrent on a link slower than that will now stop using its web seed after two minutes, where before it would have crawled on.