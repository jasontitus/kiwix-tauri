Lane `android`, Kotlin crash port round 2: the three v2.3 rules are ported, the fixture is 238/238 through Gradle, and the differential against the current Rust reference shows 0 differences on 581,458 inputs.

Tested against fixture sha256 `3f77105c…ae5cf614` (238 cases), spec `445d3740…60ec6b17` (mtime 03:06:37) and `crash.rs` (mtime 03:00:00). Nothing ran on a device or emulator.

## 1. v2.3 rules
**Disposition:** fixed.

**Change** (`CrashRedaction.kt`, three edits mirroring `crash.rs`):
- `skipWrappers`: after each wrapper's `(`, skip White_Space (the spec list, line breaks included).
- `secretAt`: after a quoted key's closing quote, skip any number of `)`. Unquoted keys are unchanged.
- `secretValue`: the token, the `authorization` second token and the path extension are measured from after the wrappers (`inner`); the value still starts at the wrapper.

**Tests:**
- Fixture test: 238/238, read from the repo path.
- Differential, isolated copy: 581,458 inputs, 0 differences for `redact` and `redactLine`. The corpus is:
  - the earlier 444,019 inputs
  - the 238 fixture inputs
  - 150,000 generated combinations of key, key quote, `)` closers, separator, wrapper (with spaces, newlines, CRLF, NBSP, U+2028, nested), value and tail
  - each fixture input with whitespace added after every `(` and `)` added after every quote
  - the new shapes at offsets −8 to +8 around the 32,768 bound.
- A second run of 867 struct, array and map values inside wrappers also gave 0 differences. It covers the residual the spec gained at 03:06 ("only its first token goes"); that edit is prose only and changed no vector.
- Fail-without-fix, isolated copy: with the three edits reverted, 2 tests fail — the fixture test (`8 of 238 cases differ`) and the new `wrappedValuesAndWrappedKeysAreRedacted`. The copy was deleted and the shared tree is unmutated.
- Linear time: five units added to `everyStepIsLinearInItsInput`, including Rust's two (`pin=a( ` and `'pin')`). Measured µs at the bound / at 4×:

  | Unit | µs at 32,768 | µs at 4× |
  |---|---|---|
  | `pin=a( ` | 2135 | 8748 |
  | `'pin')` | 2623 | 10574 |
  | `"pin"))` | 2485 | 10085 |
  | `pin: Some(\n` | 2159 | 8645 |
  | `pin=a(a( b ` | 2639 | 10746 |

## 2. Residual test
**Disposition:** fixed. The `token=Some( "x y") z` line is removed from `theResidualsTheSpecNamesStayAsWritten`; the other residuals stay there. It is now asserted in a new test, `wrappedValuesAndWrappedKeysAreRedacted`, with the v2.3 output `token=Some( "[redacted]") z`. That test also covers:
- the pretty-printed quoted form, through `redact` and `redactLine`
- `pin: Some(\n    1234,\n) end` → `pin: [redacted], ) end`
- `pin=Some(1234) end` → `pin=[redacted]) end`
- an Android path with spaces inside a wrapper
- `[AnyHashable("password"): hunter2]`
- `("pin")) = 12`
- the negative `(pin) = 12`, which is unchanged because the key is unquoted.

## 3. Count floor
**Disposition:** fixed. The fixture test now requires `cases.size() >= 238`.

## Commands run
- `scratchpad/android-lane/g.sh compile` → `BUILD SUCCESSFUL in 18s`.
- `scratchpad/android-lane/g.sh test` (`:tauri-plugin-kiwix-mobile:testDebugUnitTest`, offline, `--rerun`) → `BUILD SUCCESSFUL in 18s`; 14 result files; 79 tests, 0 skipped, 0 failures, 0 errors; `CrashRedactionTest` 12 tests, 0 failures.
- `scratchpad/android-lane/diff/run.sh v23` → `inputs 581458 differ 0`; `run.sh v23b` → `inputs 867 differ 0`.
- No state-changing git command, no `cargo`, no `npm run build`.

## Files changed
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/CrashRedaction.kt`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/test/java/org/kiwix/mobile/CrashRedactionTest.kt`

## Handoffs and doubts
- ANDROID-R11 (no CI job runs the plugin's JVM tests) is outside my owned files and is not addressed; the 238/238 check remains local only.
- The spec file changed once more during this round (03:06:37). I tested against that version. If the startup reviewer changes a vector, the fixture test will show it on the next run.
- These edits have not had independent review yet.