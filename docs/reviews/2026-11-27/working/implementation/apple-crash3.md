### Swift crash redaction — v2.3 rules, CRASH-R4, scalar-exact driver
Disposition: fixed. 238/238 on the fixture and 0 mismatches against the rebuilt Rust reference.

**The Swift changes are syntax-checked only against Apple; not type-checked or run on an Apple SDK.** `swiftc -parse` over `ios/Sources`, `ios/Tests` and `gen/apple/DownloadActivity` is clean. `CrashRedaction.swift` is pure standard library and compiles and runs unmodified on the Linux toolchain, which is where the counts below come from.

**Change — `ios/Sources/CrashRedaction.swift`**
- **Wrapper + whitespace:** `skipWrappers` skips White_Space (line breaks included) after each `(`.
- **Quoted key + `)`:** in `secretAt`, any number of `)` may follow a quoted key's closing quote.
- **Unquoted value inside a wrapper:** in `secretValue`, the token, the `authorization` second token and the path extension are measured from after the wrappers; the value still starts at the wrapper.
- All three scans cover only whitespace or `)` directly after one wrapper or one key, so the step stays linear.

**Change — `ios/Tests/CrashCapturePolicyTests.swift` (CRASH-R4)**
- An unreadable fixture is now `XCTFail` with the path under `#if targetEnvironment(simulator) || os(macOS)`; the `XCTSkip` remains only for a physical device.
- The count floor is 238, with `>=`.
- Four units for the new rules were added to the linear-time test.
- `targetEnvironment(simulator)` has no earlier use in this target; it is a standard compilation condition, but that line has not met Apple's compiler.

**Change — scratch harness**
- The differential driver now decodes and encodes line breaks scalar by scalar and splits input on the LF byte, so no grapheme-level `String` handling is left.
- The generators now emit combining marks, including directly after LF, CR, `/`, `:`, `"`, `@`, `=` and `\`, plus the v2.3 shapes. I counted them in a generated corpus of 250,000: 42,993 lines with a combining mark, 5,359 with LF + mark, 6,653 with wrapper + line break, 23,985 with quoted key + `)`.

**Results**
- Fixture, loaded from the file: `238 cases, 238 match, 0 differ`. Before the port it was 230 of 238; the 8 that differed were new vectors.
- Bound and scalar checks: `bound: ALL PASSED`.
- Timing (61 units, `-O`, x86 Linux): slowest 8.6 ms at 32,768 scalars; at most ×4.0 for 4× the input.
- XCTest bodies run verbatim in the harness: 6 of 6 pass. The harness takes the device (`XCTSkip`) branch of the `#if`, so the new `XCTFail` branch is unexercised.
- Differential against the Rust reference rebuilt from the current `crash.rs` (plain `rustc`):
  - 1,000,000 mixed cases;
  - 600,000 URL cases;
  - 500 inputs around the 32,768 / 2,000 / 240 / 64 limits;
  - **0 mismatches** in all.

**Commands run**
- `swiftc -parse` on all three Swift directories → clean.
- `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/canon/run-all.sh` → the results above.

**Files changed**
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashRedaction.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/CrashCapturePolicyTests.swift`

New user-visible strings: none. No contract changes.

**Doubts**
1. One consequence of the unquoted-wrapper rule that no vector pins: an empty wrapper is now a non-empty value, so `pin: Some()` becomes `pin: [redacted])`. Rust and Swift agree.
2. The fixture floor is a literal 238. Each time the fixture grows it should be raised, or it stops guarding the new vectors.
3. If the startup reviewer changes a vector, the fixture test follows the file automatically; only the rule code would need touching.