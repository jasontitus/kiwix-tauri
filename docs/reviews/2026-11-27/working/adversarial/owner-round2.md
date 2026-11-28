## Re-review of owner fixes R1–R9: R1–R3 and R5–R9 closed, R4 still open, no new findings

I read the changes as `git diff e01a6b1` at HEAD `01bf80a`. I edited nothing in the repository. The only things outside the R1–R9 files that are uncommitted are `docs/WINDOWS-*`, which belong to another lane.

**The localization test passes:** `node --test scripts/test-rust-localized-strings.mjs` gives pass 1, fail 0.

To check it is not a test that cannot fail, I copied `scripts/`, `src/locales` and `src-tauri/src` into a scratch tree. There I removed "Sharing PIN" and "Look up in Kiwix" from `extra-en-rust.json` and "{count} articles" from `plural-sources.json`. The test then failed and named exactly those three entries, one per new call shape (`t()`, `counted()`, variable source). I deleted the scratch tree afterwards.

### Each finding

- **R1: closed.**
  - The rewritten test covers `.text` literals, `t(` literals in files with a `let t = |…| ….text(` closure, `.counted(` checked against `plural-sources.json`, `mobile_labels(` literals, the `label()` arms, and `VARIABLE_SOURCES`. Each shape has a floor.
  - I grepped for every Localizer `.text`/`.counted` call whose source is not a literal. Each one is covered:
    - the `t` closures;
    - the `lib.rs` launch title;
    - `mobile_service.rs:410` "Share"/"Save export";
    - the `windows.rs` tabbing array;
    - the `sharing.rs` facet titles;
    - the `sharing.rs:2476` search-note plurals;
    - `live_activity` `label()`.
  - The new plural entries pass the placeholder rule in `localization.mjs` `generate`: every form has `{count}`.
  - Residual: `VARIABLE_SOURCES` only proves each listed literal is still in its file. It cannot find a new variable-source call site. The test header says so honestly.
- **R2: closed.** §3 and the porter frame bullet now say steps 1–5 apply to each frame's symbol text. They say Rust's `at` location gets step 3, the reduction to a file name, then step 2, and they give the `jane@example.org.rs` example. This matches `crash.rs` `sanitize_backtrace`, and the Swift and Kotlin descriptions match `CrashIntake.swift:54-55` and `CrashIntake.kt:62-63`.
- **R3: closed.** §1.2 step 2 now matches `ImportCutover` (count, early end, never once Stopping has begun). It also matches the `discard_cancelled_staged_copy` scope: mobile only, under `mobile-imports-v1/<uuid>/`. The pending intake and the startup notice (only when not stopping, withdrawn once the archive is admitted) match too. Both stale code comments now agree with the code.
- **R4: still open, blocked on the record.** `docs/reviews/2026-11-27/PROJECT-REVIEW-REMEDIATION.md` still does not exist; only `working/` does. The section now says, in the present tense, that this record lists the final-tree verification, so the sentence is false until you write the record and run the final-tree suites. The added statements are accurate:
  - macOS/iOS Rust was not compiled;
  - the other Swift code had `swiftc -parse` only;
  - Clippy for Windows used the MinGW target, `x86_64-pc-windows-gnu`, which is the installed one.

  I'll check the citation and its verification list once the record exists.
- **R5: closed.** The §4 wording now says "any `_` suffix after a keyword". I reproduced each example earlier: `secret_value`, `token_value` and `password_hash` all stay.
- **R6: closed.** §5.1 now says older journals load because `MapJob` ignores unknown fields (tested at `maps_tests.rs:857`). It also says `RetainedMapJob.eta_seconds` is always `None` and is serialized as `etaSeconds: null`.
- **R7: closed.** The header paths now start with `src-tauri/plugins/...`.
- **R8: closed.** The note now says `\'` is "the escaped closing quote of a quoted key (rule 4.1)".
- **R9: closed.**
  - The Back order now matches `backTarget`: dialog, tab sheet, popover (including reader overlays), search, reader history, library.
  - Item 5 now says the guard works on macOS and Android and does nothing on iOS, Windows and Linux, which matches the `power_activity.rs` cfg modules.
  - The `App.tsx` comment now says the WebView walks its history first, which agrees with the doc and the vendored `AppPlugin.kt`.

### What I could not verify
- `localization.mjs generate` was not run. The `inventory` check will stay red until it is.
- I did not run the final-tree Windows/Android clippy or the full suites.
- Nothing on a device, a Mac or a simulator.