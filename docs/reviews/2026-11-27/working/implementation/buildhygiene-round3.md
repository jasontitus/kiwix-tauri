## buildhygiene round 3: report

Both additions are done, and only docs changed. `node --test scripts/test-build-hygiene.mjs` still passes 5 of 5.

1. **`docs/DEPENDENCIES.md` size table.**
   - **Table:** every "Now" row is refreshed with the measured numbers and dated "(remeasured 2026-11-28)", the same as the Linux row:

     | Row | Now |
     |---|---|
     | Lockfile | 739 |
     | Linux | ~537 |
     | macOS arm64 | ~511 |
     | Windows x86_64 | ~506 |
     | Android arm64 | ~488 |
     | iOS arm64 | ~502 |

     The two npm rows are unchanged.
   - **Prose:** "Most of each platform's ~440–490 packages" now reads "~490–540 packages (2026-11-28)".
   - **Method note:** I removed my earlier "Only the Linux row was refreshed then". The note now says the per-platform "Before" figures were measured with `--all-features`, and that the Linux `--all-features` count is 565 against 537. It also says the lockfile row counts `name = ` entries in `src-tauri/Cargo.lock`.

2. **`docs/WINDOWS-TEST-BRIEF.md`.** The heading is now "## Phase 3: Rust tests on Windows (historical baseline: needs the SDK)". No docs link to a `#phase-3…` anchor, so the rename breaks nothing.

Two things for the owner:
- **The table now shows growth.** Every Rust "Now" figure except Linux is higher than its 2026-11-05 "Before" figure, and the lockfile went from 726 to 739. Any nearby wording that describes the table as a reduction may need softening; I did not change any.
- **Lockfile method.** I don't know how the old 686 was counted, so 739 may not be a like-for-like comparison.