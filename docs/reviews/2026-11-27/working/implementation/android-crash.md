# Lane `android`, round 4: Kotlin crash redaction ported to the canonical pipeline (spec v2.2, frozen)

The Kotlin port is finished against the frozen fixture: 228/228 vectors pass through the Gradle wiring, and `:tauri-plugin-kiwix-mobile:testDebugUnitTest` is green. Nothing ran on an Android device or emulator; all evidence is JVM unit tests on the host.

Reviewed base `d98cac2`. Fixture and spec as tested:
- `src-tauri/tests/fixtures/crash-redaction-cases.json`: 228 cases, sha256 `c3be6731…843f8440`, mtime 02:36:26
- `src-tauri/tests/fixtures/crash-redaction-spec.md`: sha256 `1d6f0886…5703cf9`, mtime 02:42:20

## CORE-8 handoff: canonical crash redaction, Kotlin side

**Disposition:** fixed (ported).

**Verification of the finding:** the round-3 Kotlin redaction was a set of regexes with UTF-16 `.take` cuts and `\s` whitespace. It did not follow the spec's step order or its scalar counts, and it had no e-mail step.

**Change:**
- New `CrashRedaction.kt` (`internal object CrashRedaction`) implements steps 1–8 in spec order: bound, secrets, URLs, e-mail, paths, opaque tokens, whitespace collapse, cut.
- It works on an `IntArray` of code points throughout, with the spec's explicit White_Space list and ASCII-only case folding.
- It uses no `java.net.URI`, no regex, no `equals(ignoreCase = true)` and no `Character.isWhitespace`.
- Every step is a single pass or uses precomputed per-line tables (`Lines.end`, `lastApostrophe`, `wordEnd`, `backslashAhead`).
- `CrashIntake.kt` now delegates and its old patterns are removed:
  - message: `CrashRedaction.redact(...)`
  - frames: `truncate(redactLine("class.method(file:line)"), 240)`, where `redactLine` is steps 1–5 with spacing kept
  - thread name: `truncate(redact(name), 64)`
  - all cuts are in scalars.

**Tests:** `CrashRedactionTest.kt` is new with 9 tests; `CrashCapturePolicyTest.kt` has its expectations updated to the canonical shapes.

- **Fixture, 228/228.** `redactionMatchesTheCanonicalCasesSharedWithRustAndSwift` walks up from the Gradle working directory to `src-tauri/tests/fixtures/crash-redaction-cases.json` and reads that file with Jackson. There is no copy and no hand-duplicated vector. It asserts each case is exactly `{input, expected}`, that there are at least 80 cases, and that every output matches verbatim.
- **Code-point bound.** `inputIsBoundedInScalarsBeforeAnyStepRuns` covers:
  - exactly 32,768 scalars are kept
  - a token straddling the bound is dropped whole: a reader URL across the bound gives `""`, not a partial `[reader-url]` or a leaked prefix
  - a partial path at the bound is dropped
  - surrogate-pair input is counted per scalar, not per UTF-16 unit
  - one unbroken run longer than the bound gives `""`.
- **Message cut.** `aMessageIsCutToTwoThousandCodePointsWithoutSplittingOne` checks 1,999 scalars plus `…` and that no surrogate pair is split.
- **Linear time.** `everyStepIsLinearInItsInput` runs 24 adversarial units at 32,768 scalars and at 4× that, through `unboundedStepsForTiming` (steps 2–6 without the bound). It asserts `large <= max(small, 5 ms) * 10` and `small < 300 ms`. Measured in the final run:

  | Unit | µs at 32,768 | µs at 4× |
  |---|---|---|
  | `a://b<` | 3919 | 11701 |
  | `file:x<` | 2658 | 9619 |
  | ``zz://q` `` | 2141 | 8463 |
  | `password=""` | 1940 | 7618 |
  | `data:x` | 2004 | 8151 |
  | `/a:` | 2736 | 10926 |
  | `\a` | 2228 | 8894 |
  | `a:` | 2770 | 10953 |
  | `'password'` | 1912 | 7716 |
  | `token='x` | 257 | 826 |
  | `C:\a"` | 2484 | 10072 |
  | `pin=''` | 202 | 886 |
  | `token=\"\"` | 1957 | 7559 |
  | `\a<` | 2713 | 10967 |
  | `Some(Some(` | 2028 | 8018 |
  | `https://example.org/a` | 865 | 3373 |
  | `a@b.c` | 2572 | 10073 |
  | `a.b@` | 2587 | 10414 |
  | `@a.b-` | 2456 | 9806 |
  | `x@y.zz@` | 2519 | 10178 |
  | `TokenA` | 1914 | 7743 |
  | `passwordHash` | 1961 | 7862 |
  | `~jane` | 1892 | 7525 |
  | `~a~` | 2285 | 9122 |

  Every ratio is between about 3.0× and 4.4× for 4× the input.
- **Whitespace.** `whitespaceIsExactlyTheUnicodeWhiteSpaceProperty` checks every code point against the spec list and the JDK's `\p{IsWhite_Space}`. `whitespaceIsNotWhatThePlatformCallsWhitespace` pins the points where `Character.isWhitespace` disagrees.
- **Android shapes.** `androidStoragePathsAndProviderLinksKeepNoPart`, `stackFramesKeepTheirShape` and `onlyAPublicHttpHostIsKeptAndNeverItsPath`.

**Differential against the Rust reference:** 444,019 inputs gave 0 differences for both `redact` and `redactLine`. The Rust side was a scratch `rustc` build of the reference functions from `src-tauri/src/crash.rs` (mtime 02:36:40, unchanged since); the Kotlin side ran in an isolated Gradle copy. The inputs were the fixture inputs plus generated mutations and concatenations.

**Fail-without-fix, in isolated copies only:**
- Three rules broken together (per-candidate rescan instead of precomputed tables, bound without the partial-run drop, `Character.isWhitespace`): 5 tests fail — `everyStepIsLinearInItsInput`, `inputIsBoundedInScalarsBeforeAnyStepRuns`, the fixture test and both whitespace tests.
- Secrets moved after paths: 2 tests fail — the fixture test (`9 of 228 cases differ`) and `androidStoragePathsAndProviderLinksKeepNoPart`.
- Each copy was deleted afterwards. The shared tree was confirmed unmutated: step order intact at `CrashRedaction.kt:39-43`.
- The v2.1/v2.2 additive rules (e-mail step, letter-only `://` exception, `secret_key`/`access_key` family, camelCase suffix, `~user/`, punycode TLD) were not mutated one by one. They are covered by the fixture vectors and the differential only.

**Not exercised:**
- A real crash record written and read back on a device or emulator.
- R8-minified builds.
- The Swift side.
- The linear-time assertion on a loaded CI machine.

## Spec ambiguities hit

All are resolved the way the Rust reference and the vectors behave; none needs a fixture change. For some the prose could be tightened.

1. v1 `safe_url` was defined by the Rust `url` crate (IDN, percent hosts, `^` encoding). My v1 port differed on 1,914 of 109,085 inputs. v2's textual `classify` resolves this.
2. "Kept: scheme://host" means the host after ASCII lowercasing and removal of one trailing dot (vector `HTTPS://EXAMPLE.ORG.`). The scheme is also lowercased.
3. `classify` tests reader hosts before port validity, so `http://localhost:port/x` is `[reader-url]`, not `[url]`.
4. Only `\"` is an escaped value quote. `\'` at a value start is not a quoted form (Rust `if !escaped`). The spec lists `\'` for keys but is silent for values.
5. The default-port comparison ignores leading zeros (`:080` and `:0443` are dropped). `:000` is not a default and is kept as written.
6. The second slash of the `://` path exception is identified by position only: previous scalar `/`, `:` two back, ASCII letter three back.
7. The camelCase suffix skip runs to the word end (letters, digits, `_`). A suffix disables the no-separator `authorization`/`bearer` form.
8. Frame handling is per platform and not in the spec. Kotlin applies steps 1–5 to `class.method(file:line)`, then cuts frames to 240 scalars and thread names to 64. The spec could state which steps frames get.
9. Rust's rooted-path look-ahead (`\` + alnum … a later `\`) is a forward scan. Kotlin precomputes `backslashAhead` to stay linear. The outputs are identical; the Rust scan's worst case is a matter for the startup lane.
10. Kotlin strings can hold unpaired surrogates, which are not scalar values and cannot occur in a Rust `&str`. The port carries each as one code point and never splits or drops it. The spec does not say what to do; the Swift lane may want the same rule.
11. The last spec edit (02:42:20) arrived after the fixture froze. You described it as one §4 wording line naming the `accessToken_value` residual. The implementation matches that: `_` after the keyword is not a camelCase suffix, so that form is not treated as a key. My local snapshot is identical to the current spec file.

## Commands run
- `scratchpad/android-lane/g.sh compile` → `BUILD SUCCESSFUL in 8s`, `27 actionable tasks: 2 executed, 25 up-to-date`
- `scratchpad/android-lane/g.sh test` (`gradle --no-daemon --offline … :tauri-plugin-kiwix-mobile:testDebugUnitTest`) → `BUILD SUCCESSFUL in 19s`; 14 result files; 76 tests, 0 skipped, 0 failures, 0 errors; `CrashRedactionTest` 9 tests, 0 failures. The fixture was byte-identical to the frozen 228-case file at that run.
- Isolated copy, `:tauri-plugin-kiwix-mobile:testDebugUnitTest --rerun` with the step-order mutation → `76 tests completed, 2 failed`, `BUILD FAILED in 1m 1s` (expected).
- `scratchpad/android-lane/diff/run.sh` (scratch `rustc` and isolated Gradle copy) → 444,019 inputs, 0 differences.
- No state-changing git command, no `cargo`, no `npm run build`.
- `android-clippy.sh`, `tsc` and the node tests were not rerun: no Rust, TypeScript or script file was touched this round.

## Files changed this round
All under `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/android/`; `git diff d98cac2 --stat` shows 4 files, +946 −52.
- `src/main/java/org/kiwix/mobile/CrashRedaction.kt` (new, 690 lines)
- `src/main/java/org/kiwix/mobile/CrashIntake.kt` (delegates; regex helpers removed)
- `src/test/java/org/kiwix/mobile/CrashRedactionTest.kt` (new, 235 lines)
- `src/test/java/org/kiwix/mobile/CrashCapturePolicyTest.kt` (expectations updated)

## Handoffs
- **Startup lane / spec owner:** ambiguities 4, 8 and 10 are worth one line each in the spec. They are optional and change no vectors.
- **Startup lane:** item 9 — check the Rust rooted-path forward scan for a quadratic worst case on input such as repeated `\a` with no later backslash. I did not measure the Rust side.
- **Independent review:** this port has had no independent adversarial review yet (AGENTS.md gate). The reviewer needs `CrashRedaction.kt`, `CrashIntake.kt`, the two test files, the spec and the fixture.
- Earlier handoffs (H1–H5, H7 and the notes from rounds 1–3) are unchanged.

## Doubts
- The linear-time test is timing-based. The 10× allowance against 4× input and the 5 ms floor make a false failure unlikely, but a heavily loaded machine could still trip `small < 300 ms`.
- The White_Space test compares against the JDK's `\p{IsWhite_Space}` table as well as the spec list. On a JDK with a different Unicode version the second comparison could disagree; the implementation itself uses only the spec list.
- The differential corpus is derived from the fixture inputs plus mutations. It is strong evidence of agreement with the Rust reference, not proof for every input.
- The Kotlin frame format `class.method(file:line)` goes through steps 1–5. A class or file name that looks like a path or URL is redacted inside the frame, which is intended but can make a frame less readable.