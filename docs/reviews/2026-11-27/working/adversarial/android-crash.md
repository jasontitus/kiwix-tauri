## Verdict for the Kotlin crash-redaction port: closed. One non-blocking note.

The Kotlin output matched the Rust reference on every input I tried, including 301,346 inputs I generated myself and did not derive from the fixture. I found no leak, no split surrogate and no super-linear step. The only remaining item is that CI never runs these tests (ANDROID-R11). Nothing ran on an Android device or emulator.

### ANDROID-R11 — no workflow runs the plugin's JVM tests, so the fixture check is local only
Severity: Should-fix (process; not a defect in the port, and it was already true at `d98cac2`)
Fix under review: CORE-8 Kotlin port (fixture test)
Evidence class: source-traced
Location: `.github/workflows/build.yml`, `checks.yml`, `dependencies.yml`. None of them runs `gradle` or `testDebugUnitTest`. `android-typecheck` builds Rust only.
Trigger: a later edit to `CrashRedaction.kt`, or to the fixture or spec, that breaks Kotlin agreement with Rust.
Causal path: `CrashRedactionTest.redactionMatchesTheCanonicalCasesSharedWithRustAndSwift` is the only automated check that Kotlin still agrees with Rust, and it runs only when someone runs Gradle locally. The fixture loader itself is fine:
- It walks up from the Gradle module directory and finds `src-tauri/tests/fixtures/crash-redaction-cases.json`, taking the nearest match first.
- If the file is missing it throws an `AssertionError`; it does not skip.
- It also requires at least 80 cases and exactly `{input, expected}` per case.

Smallest correct fix: add a CI job that runs `:tauri-plugin-kiwix-mobile:testDebugUnitTest`. Until then, no one should claim the parity check is enforced remotely. Note that the timing test (`everyStepIsLinearInItsInput`, which requires `small < 300 ms`) would then run on shared CI runners.

### Assessment of the points you asked about
1. **Leaks that differ from Rust: none found.** I built a Rust binary from the current `src-tauri/src/crash.rs`, `file_name` through the start of the tests module. I ran the compiled `CrashRedaction` classes directly under Java, checking that the class file was newer than the source. Both sides computed `redact` and `redactLine`:

   | Corpus | Inputs | Differences |
   |---|---|---|
   | Two seeded random corpora (described below) | 300,000 | 0 |
   | Long inputs around the 32,768 bound (described below) | 1,346 | 0 |

   - The random corpora mix token fragments with random code points from every plane.
   - The fragments include Android storage paths (`/storage/emulated/0/Download/My Books/`, `/data/user/0/...`), `content://com.android.externalstorage.documents/tree/primary%3A…` URIs, SAF IDs (`primary:…`, `msf:12`, `raw:/…`) and keywords in mixed case.
   - They also include `Some(`, quotes and escaped quotes, CRLF, lone CR, NEL, U+2028/2029, NBSP, U+200B, U+FEFF, `ſ`, the Kelvin sign, `İ`/`ı`, emoji and astral characters.
   - In the first corpus the outputs contained 51,594 `[path]`, 55,189 `[url]`, 20,184 `[redacted]`, 9,249 `[email]`, 144 `[reader-url]` and 17 `…`, and 332 outputs kept an `http(s)` host. So the matching was not just pass-through.
   - The long inputs place each of ten sensitive tails at offsets −6 to +6 around position 32,768, with five fillers, one of them a surrogate pair. They also include repeated adversarial units at 40,000 scalars and the 2,000-scalar cut with surrogate pairs at the boundary.
2. **Code-point handling: correct.**
   - An unpaired surrogate in Kotlin gave the same result as Rust given U+FFFD at the same place, on 40,000 inputs with lone surrogates inserted and never adjacent: 0 differences. That is the spec's "one opaque non-whitespace code point" rule.
   - No step deletes text between two lone halves without putting something in its place, so no step can join them into a pair.
   - Cuts, checked directly:
     - `truncate(😀×300, 240)` gives 240 scalars ending in `…`, with no split pair.
     - The thread cut gives 64 scalars (95 UTF-16 units).
     - The 2,000 cut gives 2,000 scalars ending in `…`.
     - The 32,768 bound runs on code points, and the fixture and the long corpus agree with Rust on dropping a partial run.
3. **Linear time: confirmed.** I timed `unboundedStepsForTiming` at 32,768 and at 4× that on 33 units of my own:
   - The units are `~a-`, `pass `, `bearer\t`, `"password"\t`, `Some(`, `x@a.`, `\a/`, `http://a@`, `http://[`, `1://`, `a:/`, `authorization `, `cookie`, `token=\"\\"`, `'`, `password='/`, `https://a.b:`, `@a-`, `%@`, `Bearer`+U+2028, `~é/`, `//x`, `\\`, `C:/`, `password  :`, `pin,`, `"token",`, `::`, `file:`, `data:\r`, `zim:x"`, `a@b.cc.` and `xn--@a.xn--`.
   - Every ratio was 3.2–4.7×, except `Some(` at 6.1×.
   - Rerun at 32k, 128k and 512k, `Some(` scaled at 1.9 ms → 8.3 ms → 27.5 ms, which is linear.
   - Other checks: `password=Some(` at 512k took 4.0 ms, and 2,000 `password` + 50 spaces with no separator took 3.6 ms.
   - The bounded `redact` of each 4× input took at most 2.1 ms.
4. **`CrashIntake.kt` integration: every text field goes through the redaction pipeline.**
   - message: `redact`, which applies all 8 steps
   - location: `redact`
   - thread name: `redact`, then cut to 64
   - each of the ≤24 frames: `redactLine` (steps 1–5, as the spec requires for frames), then cut to 240
   - The other fields are app-controlled and contain no user text: schema, timestamp, `versionName`, `os.arch`, phase and operation.
   - The cause chain and suppressed exceptions are not written at all, so they cannot leak (the record does lose that diagnosis).
   - `CrashIntake` has no `Log.*` call and writes no other file.
   - Outside this intake's control: the previous default handler, the platform's own, still prints the raw "FATAL EXCEPTION" to logcat.
5. **Fixture loading:** robust and loud, as described under ANDROID-R11. CI does not run it.

The current spec (sha256 `d852cdd2…`) is consistent with the code. That includes the "Notes for porters" (only `\"` opens an escaped quoted value, reader host is checked before port, `:000` is kept, every cut is limit−1 plus `…`, the unpaired-surrogate rule) and the two new residuals. The two new tests, `anUnpairedSurrogateIsOneOpaqueCodePoint` and `theResidualsTheSpecNamesStayAsWritten`, assert exactly those rules. The fixture is unchanged (sha256 `c3be6731…`, 228 cases).

### What I ran
- Kotlin compile of the plugin and app module plus plugin unit tests (Gradle, offline, `--max-workers=2`, `--no-daemon`, `--rerun`): `BUILD SUCCESSFUL in 9s`. 15 result files: `tests=78 skipped=0 failures=0 errors=0`, of which `CrashRedactionTest` is 11 tests.
- Differential, Rust reference against Kotlin classes: 301,346 inputs, 0 differences. Lone-surrogate equivalence run: 40,000 inputs, 0 differences.
- The cut checks and timing runs listed above.

My harness and corpora are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-android/crash/` (`gen.py`, `genlong.py`, `ref.rs`, `Run.java`, `L.java`, `L2.java`, `cmp.py`). The repository was not modified.

### What I could not verify
- A real crash record written and read back on a device.
- R8-minified builds.
- How the timing test behaves on a loaded CI runner.