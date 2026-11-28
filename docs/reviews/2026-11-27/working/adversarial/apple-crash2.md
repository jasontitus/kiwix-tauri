## Verdict for lane apple (Swift crash-redaction port): only non-blocking findings — the port is closed against the spec

The Swift port agrees with the Rust reference on every input I threw at it, and CRASH-R1, R2 and R3 are all closed. What remains is one test-rule fix (the fixture test can silently skip) and two leak shapes that are gaps in the spec itself, identical in Rust. Nothing was built against an Apple SDK; the redaction file is pure standard library and ran unmodified on Linux.

### CRASH-R4 — The fixture XCTest skips, and so passes, whenever the fixture cannot be read
Severity: Should-fix
Fix under review: Swift port tests
Evidence class: source-traced
Location: `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/CrashCapturePolicyTests.swift:88-95` (`testRedactionMatchesTheCanonicalCasesSharedWithRust`)
Trigger: Run on a simulator or host where the `#filePath`-derived path does not resolve: the fixture is renamed or moved, or the tests were built on another machine or in a relocated checkout.
Causal path: `guard let data = try? Data(contentsOf: fixture) else { throw XCTSkip(…) }`. A skip is reported as success, and this test is the only link between the Swift port and the shared vectors. The path arithmetic itself is right today (five components up lands on `src-tauri`). A second loosening: `XCTAssertGreaterThanOrEqual(cases.count, 80, …)` while the fixture holds 228, so 148 vectors could vanish unnoticed.
Smallest correct fix: skip only on a physical device.
- Inside the `guard`, under `#if targetEnvironment(simulator) || os(macOS)`, call `XCTFail` with the path and return.
- In the `#else` branch, keep the `XCTSkip`.
- Raise the count floor to 228, or to whatever number the Rust test pins.

### CRASH-R5 — Two secret shapes leak under the spec, the same in Rust and Swift (implementer gap 2, plus one new)
Severity: Should-fix at spec level (owner decision; no change to the Swift port alone)
Fix under review: spec §2 step 2 / §4
Evidence class: reproduced on both the Rust reference and the Swift port
Location: `/home/user/kiwix-tauri/src-tauri/tests/fixtures/crash-redaction-spec.md` (value form 1 wrappers; item 4.1 quoted key; §4 residual list)
Trigger and causal path:
- **Wrapper followed by whitespace.**
  - `token=Some( "x y") z` gives `token=[redacted] "x y") z`. Written by hand like that it is unrealistic.
  - The realistic form is Rust's pretty debug (`{:#?}`), which puts a line break after `Some(`: `token: Some(\n        "abc def",\n    )` gives `token: [redacted] "abc def", )`. That is plausible in a Rust `panic!`/`expect` message.
  - On iOS it is close to impossible: Swift prints `Optional("…")` with no space, and that is redacted.
- **Swift dictionary keys.** `[AnyHashable("token"): abc def, AnyHashable("password"): hunter2]` comes out unchanged, because `)` sits between the closing quote and the `:`. This is how Swift prints any `[AnyHashable: Any]`, such as a `userInfo`. It only reaches this intake if code raises an `NSException` whose reason interpolates such a dictionary, so the likelihood is low. The Objective-C dictionary form (`token = abcdef;`) is redacted.
Smallest correct fix: at minimum add both to §4. To close them, change all three ports and the vectors together: let a wrapper's `(` be followed by whitespace before the quote, and let `)` follow the closing quote of a quoted key.

### CRASH-R6 — Frame lines lose selector text after a keyword or after `data:` / `file:`
Severity: Nit (diagnostic loss only; the spec's steps 1–5 on frames; implementer gap 6 covers the keyword half)
Fix under review: `CrashIntake.swift` frames
Evidence class: reproduced
Location: `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashIntake.swift:54-56`
Trigger: A frame such as `-[KiwixSession setAuthorization:forHost:] + 88` or `+[NSData(Kiwix) data:encoding:] + 88`.
Causal path: The first becomes `setAuthorization:[redacted]] + 88`. The second becomes `+[NSData(Kiwix) [url]`, losing the offset too. Ordinary frames, Swift and Rust mangled symbols and `<redacted>` come out unchanged with their spacing.
Smallest correct fix: none needed; note it next to §3 so nobody is surprised when reading a record.

**Earlier findings**
- **CRASH-R1:** closed. There is no pattern engine any more and the input is bounded at 32,768 scalars before any scan. The million-character shapes that failed open before now give `[path]`, `[url]` or nothing.
- **CRASH-R2:** closed. `NSBundle </private/var/…/Kiwix.app>` gives `<[path]>`, and `“/var/mobile/My Books/a.zim”` gives `“[path]`.
- **CRASH-R3:** closed. The same-line case is asserted at `CrashCapturePolicyTests.swift:59` and is also a fixture vector.

**Answers to the six points**
1. **Leaks that differ from Rust:** none found. My own corpus was 2,259,000 cases, 0 mismatches, in four groups:
   - random Unicode: every White_Space member and near-misses, case-fold traps, astral and combining scalars, CR/LF/NEL/U+2028;
   - mutated iOS shapes: container and `/private/var` paths, `file:///`, `NSError` userInfo dumps, `Optional(…)`, `callStackSymbols`;
   - keyword forms;
   - inputs straddling the limits.

   Absolute outputs on unmutated iOS shapes kept no part of a path or URL. A bare name with no start shape (`My Books/a.zim`, or the `“medical wikipedia.zim”` inside an `NSError` description) stays; that is §4.
2. **Scalar handling:** correct at 32,768, 2,000, 240 and 64, including astral scalars, a cut inside a grapheme cluster, and CR LF. The thread name comes out at exactly 64 scalars (63 and `…`).
3. **Linear time:** by reading, every forward scan is either consumed by its match or cannot be repeated over the same text. Measured on x86 Linux, optimised:
   - slowest full-bound input, about 8.8 ms;
   - 1,500 random compositions of adversarial units, worst growth ×7 for 4× the input, all in single-digit milliseconds.

   The bound caps the cost whatever the growth.
4. **Integration:** every text field written goes through the pipeline:
   - `message` (the reason, or the name as fallback): all eight steps;
   - `thread`, and `location` (the exception name): eight steps plus a scalar cut;
   - `stack`: steps 1–5 plus a 240-scalar cut, at most 24 frames.

   `userInfo` is not written at all, and `NSException` has no cause chain here. The other fields are constants, the app version or a timestamp. Neither file contains `NSLog`, `os_log`, `print` or a `Logger`.
5. **XCTest skip:** see CRASH-R4.
6. **Gap 2:** see CRASH-R5.

**One harness note for the implementer**
The differential driver in `scratchpad/apple/canon/main.swift` decodes line breaks with `replacingOccurrences`, which does not match when a combining mark follows. With my corpus that produced about 3,900 false mismatches. A scalar-exact driver gives 0. Their own generator has no combining marks, so their counts are not wrong, but that case was never exercised on the Swift side.

**What I ran**
- `swiftc -parse` over `ios/Sources`, `ios/Tests` and `gen/apple/DownloadActivity`: exit 0.
- Rebuilt the Rust reference from the current `crash.rs` and the Swift port in my scratch directory: fixture `228 cases, 228 match, 0 differ`; `bound: ALL PASSED`.
- My differential (`mygen.py`, `d2/main.swift`), probes (`d3`, `d4`) and timing, all in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-apple/canon/`.
- I did not re-run the implementer's `run-all.sh`. Its saved output shows the six XCTest bodies passing and its differentials at 0 mismatches.

**What I could not verify**
- Any Apple build or `xcodebuild test`; the `CrashIntake.swift` call sites are parsed only.
- A real uncaught exception on a device, and real `callStackSymbols` text.
- The Kotlin port.
- Handoff: the spec header still names `CrashIntake.swift` as the Swift implementation; the code is in `CrashRedaction.swift`.