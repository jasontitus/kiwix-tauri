### Swift crash redaction — port to the canonical spec (v2.2, frozen)
Disposition: fixed (ported). The Swift port reproduces all 228 vectors and agrees with the Rust reference on every differential case.

**Every Swift change is syntax-checked only against Apple; not type-checked or run on an Apple SDK.** `swiftc -parse` over all of `ios/Sources`, `ios/Tests` and `gen/apple/DownloadActivity` is clean. The new redaction file uses the Swift standard library only, and it compiles and runs **unmodified** on the Linux toolchain; that is where every count below comes from.

**Verification of the finding**
- The old code was a regex pipeline with `URLComponents`, in a different step order from Rust.
- The reviewer's CRASH-R1 (a long line fails open) applied to it.

**Change**
- New `ios/Sources/CrashRedaction.swift` (`enum CrashRedaction`): `redact` (steps 1–8), `redactLine` (steps 1–5), `truncate`.
  - One `[Unicode.Scalar]` array per step, plain loops.
  - No `NSRegularExpression`, no `URLComponents`/`URL`, no `Character`, no platform whitespace or case functions.
  - White_Space is the spec's explicit list; folding is ASCII-only.
  - Line ends, last apostrophes and word ends are precomputed once per secret step.
- `ios/Sources/CrashIntake.swift`: the regex statics and helpers are removed and `redact` forwards to `CrashRedaction`. In `writeRecord`:
  - frames use `redactLine` and a 240-scalar cut (before: the full message pipeline, which collapsed spacing and replaced long mangled symbols);
  - the thread name and location are cut by scalars, no longer by `Character` (`String.prefix`).
- The spec's header names `CrashIntake.swift` as the Swift implementation; the code is in `CrashRedaction.swift`. That line of the spec needs updating (not my file).

**Tests** (`ios/Tests/CrashCapturePolicyTests.swift`; XCTest cannot run here, but each body below runs verbatim in the Linux harness and passes)
- `testRedactionMatchesTheCanonicalCasesSharedWithRust` reads the fixture file itself through `#filePath` (never a copy), compares as scalars, and throws `XCTSkip` if the file is unreadable, as it would be on a physical device.
- `testRedactionInputIsBoundedBeforeAnyStepRunsAndCountsScalars`:
  - a run four times the bound leaves nothing;
  - a reader URL straddling scalar 32,768 is dropped whole (the partial token), and the same URL whole gives `[reader-url]`;
  - a path cut by the bound keeps none of its parts;
  - cuts on either side of a space keep every whole word;
  - 1,000,000 colons after `/`, a 1.2M-scalar `file:` URL and a 1.8M-scalar spaced path on one line (the CRASH-R1 shapes) do not pass through;
  - scalar counting holds for 2-byte, astral, combining and CR LF text;
  - the 2,000 and 240 cuts.
- `testEveryRedactionStepIsLinearInItsInput`: 21 adversarial units, a quarter of the bound against the full bound. It asserts at most 10× with a 5 ms floor, and under 2 s.
- `testStackFrameLineKeepsSpacingAndLongSymbols`.
- **CRASH-R3:** `path=/book-uuid/A/Private relative=/.kiwix/token/A/Private` → `path=[path]` is asserted in `testReportTextKeepsNoPartOfAPathOrFileURLThatContainsSpaces` (it is also a fixture vector).
- The two older tests are updated to v2 outputs: a kept URL is `scheme://host` only, and a placeholder URL takes its clause.
- Fail without the port: not observed as a run. The old code cannot produce these outputs; the reviewer measured the long-line failure on it.

**Observed counts**
- Fixture: `228 cases, 228 match, 0 differ`.
- Bound and scalar checks: 23 of 23 pass.
- Linear time, 55 adversarial units at 32,768 scalars, `redact` plus `redactLine`:

  | Build | Slowest | Growth for 4× the input |
  |---|---|---|
  | `-O` | 9.0 ms | at most ×4.3 |
  | `-Onone` | 190 ms | at most ×4.2 |

  These are x86 Linux numbers, not a phone.
- Differential against the real `crash.rs` (functions extracted, built with plain `rustc`, no cargo), comparing `redact`, `redact_line` and the 240 frame cut:
  - 1,200,000 mixed cases;
  - 600,000 URL-classification cases;
  - 400 inputs around the 32,768, 2,000 and 240 limits;
  - **0 mismatches** in all.
- A correction to an interim figure: my first v2 differential runs used a generator that had silently missed the v2 shapes (a chained command skipped the edit). I fixed the generator and reran; the counts above are all from the corrected one.

**Not exercised**
- Any Apple build or `xcodebuild test`.
- A real uncaught exception on a device.
- What the redaction does to real `callStackSymbols` lines.

**Spec ambiguities and gaps I hit** (I followed Rust in each; none changes a vector)
1. **Empty quoted value.** When value form 1 applies and the quoted value is empty (`password=""`, `Some("")`), the keyword does not match; it does not fall through to the cookie or token form. No vector pins this. One consequence: `cookie="" a=b` comes out unchanged, so the other pairs on that cookie line stay.
2. **Wrapper followed by whitespace.** `token=Some( "x y") z` becomes `token=[redacted] "x y") z`, so the value leaks. §4 does not list this residual. Rust and Swift behave the same.
3. **`classify` order.** The reader-URL test (step 5) comes before the port check (step 6), so `http://localhost:abc` is `[reader-url]`. This is as written, but easy to get backwards.
4. **Kept host.** It is written without the trailing dot. The spec implies this but does not say it.
5. **Field limits.** "64 scalars for a thread name" does not say whether the cut adds an ellipsis. Rust does (63 and `…`), and I do the same for thread, location and frames.
6. **Frames on iOS.** The spec leaves frames per platform. I run steps 1–5 on every `callStackSymbols` line, cut at 240 scalars, keep at most 24. Under the camelCase rule, Objective-C selectors such as `…WithToken:completion:` lose their tail.

**Commands run**
- `swiftc -parse` on all three Swift directories → clean.
- `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/canon/run-all.sh` → fixture 228/228, `bound: ALL PASSED`, six XCTest bodies `ALL PASSED`, seven differential runs each `mismatches: 0`. The script rebuilds the Rust reference and the Swift port, then runs everything.
- `./swift_port bound` built with `-O` and with `-Onone` → the timing table above.

**Files changed**
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashRedaction.swift` (new)
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashIntake.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/CrashCapturePolicyTests.swift`

New user-visible strings: none.

Harness: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/canon/` (`run-all.sh`, `mkref.sh`, `main.swift`, `gencases.py`, `genurls.py`, `compare.py`, `xct/`).

**Handoffs / contract changes**
- The iOS crash record's frame lines now keep their spacing and long symbols, and are redacted by steps 1–5.
- The spec's Swift path line needs `CrashRedaction.swift`.
- `CrashRedaction.swift` is picked up by the Swift package automatically (`path: "Sources"`); no project file changes.

**Doubts**
1. The file has never met Apple's compiler. It is pure standard library and compiled first time on Swift 6.0.3 for Linux in Swift 5 mode, but the call sites in `CrashIntake.swift` are only parsed.
2. The linear-time XCTest is a timing test. Its thresholds mirror Rust's and leave about 10× headroom over the unoptimized Linux numbers; a very slow or busy simulator could still trip it.
3. Agreement with Rust means the two ports match each other, including on gaps 1 and 2 above. It does not mean those outputs are right.