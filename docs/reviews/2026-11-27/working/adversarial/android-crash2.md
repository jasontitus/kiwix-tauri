## Verdict for the Kotlin crash-redaction port, round 2 (spec v2.3): closed

The port is closed; I found no new problems. Kotlin agreed with the rebuilt Rust reference on every input I tried, including the v2.3 shapes, and every step still runs in linear time. One earlier finding stays open: ANDROID-R11, no CI job runs the plugin's JVM tests. The implementer notes it is outside their files.

### Per-item assessment
- **v2.3 rules: closed.**
  - The three rules are whitespace after a wrapper's `(`, any number of `)` after a quoted key, and an unquoted token measured from after the wrappers.
  - My differential showed 0 differences. I built it myself; none of it comes from the fixture, and it adds 151,020 inputs with the v2.3 shapes:
    - The combinations cover leading text, an optional key quote, keyword, key quote (`"`, `'`, `\"`, `\'`), 0–3 `)` closers and a separator.
    - Each is followed by 0–2 wrappers and a value. The wrappers include `Some( `, `Some(\n`, `Some(\r\n    `, `Some(` + NBSP, `Some(` + U+2028, nested, and `a(a( `. The values include quoted, empty-quoted, escaped, an Android path with spaces, `content://`, struct and array values, two-token `authorization` and e-mail.
    - Then a tail, sometimes followed by a second key and value.
    - 1,020 of the 151,020 place the first 60 combinations at offsets −8 to +8 around the 32,768 bound.
  - In the Rust output, 89,036 of those inputs contain `[redacted]`, so the new rules really ran.
- **Residual test moved: closed.** `token=Some( "x y") z` is now asserted as `token=Some( "[redacted]") z` in `wrappedValuesAndWrappedKeysAreRedacted`. It was removed from `theResidualsTheSpecNamesStayAsWritten`.
- **Fixture floor: closed.** The test requires `cases.size() >= 238`. The fixture has 238 cases (sha256 `3f77105c…ae5cf614`); the spec is sha256 `445d3740…60ec6b17`.
- **Linear time: closed.** My own units for the new rules, timed at 32k, 128k and 512k:

  | Unit | 32k | 128k | 512k |
  |---|---|---|---|
  | `pin=Some(     ` | 1.5 ms | 4.9 ms | 18.8 ms |
  | `"pin")))))` | 1.3 ms | 4.7 ms | 23.1 ms |
  | `pin=a(\n` | 1.3 ms | 4.5 ms | 17.7 ms |
  | `'pin'))) ` | 1.2 ms | 4.8 ms | 18.5 ms |
  | `pin=a( a( a( ` | 1.1 ms | 4.0 ms | 16.3 ms |
  | `pin: Some(` + U+2028 | 3.3 ms | 9.2 ms | 21.9 ms |

### What I ran
- Kotlin compile (plugin and app module) and plugin unit tests (Gradle, offline, `--max-workers=2`, `--no-daemon`, `--rerun`): `BUILD SUCCESSFUL in 9s`.
  - 15 result files: `tests=79 skipped=0 failures=0 errors=0`. `CrashRedactionTest` has 12 tests.
  - The compiled `CrashRedaction.class` (03:06:01) is newer than its source (03:05:27).
- Rust reference rebuilt from the current `src-tauri/src/crash.rs` (mtime 03:00:00) with scratch `rustc`. The Kotlin side was the compiled classes run directly under Java.

| Corpus | Inputs | Differences |
|---|---|---|
| Earlier non-fixture corpus `in1` + `in2` | 300,000 | 0 |
| Long inputs around the 32,768 bound | 1,346 | 0 |
| New v2.3 shapes | 151,020 | 0 |
| Lone-surrogate inputs (each compared with Rust given U+FFFD in the same place) | 40,000 | 0 |

Harness and corpora are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-android/crash/` (`ref23.rs`, `in23.json`, `L3.java` added this round). The repository was not modified.

### What I could not verify
- A crash record written on a device.
- R8-minified builds.
- Timing behaviour on a CI runner.