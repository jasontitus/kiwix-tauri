### CORE-8 (Swift half) — iOS crash redaction stopped a path or `file:` URL at the first space
Disposition: fixed

**The Swift change is syntax-checked only; not type-checked, not run** against an Apple SDK (`swiftc -parse` on both changed files and on the whole Swift set: clean). On Linux, the real function text runs against corelibs' `NSRegularExpression` and agrees with the Rust rule on every case I tried.

Verification of the finding: confirmed in `CrashIntake.swift`. `pathPattern` ended with `[^\s"<>`]+` and `urlPattern` with `[^\s"<>`]*`. Running the base text in the Linux harness on `open /var/mobile/Containers/Data/Application/0A1B/Documents/My Books/medical wikipedia.zim failed` gave `open [path] Books/medical wikipedia.zim failed`.

Change (`ios/Sources/CrashIntake.swift`):
- **`pathTail`** — a new shared pattern fragment, `(?>[^"<>`\r\n:\\]|\\\\|\\"|\\|:(?!\s))*+`. A path now runs until an unescaped `"`, `<`, `>`, a backtick, a line break, `:` followed by whitespace, or end of text. `'` does not end it. Backslashes are consumed in pairs, so `\"` continues and `\\"` ends, the same parity rule as `path_end`.
- **`pathPattern`** — same start boundary and the same four starts as before, followed by `pathTail`. The Unix start now requires a following non-`/`, non-space character, and the drive start no longer consumes its separator. Both mirror `path_at` and keep the backslash parity exact.
- **`urlPattern`** — a first alternative `file:(?=/)` + `pathTail`. Every other URL keeps the old whitespace rule.
- **`keepingSeparator`** — a small helper that puts back one space when a match ended in whitespace. Rust trims that whitespace off the path; this keeps the collapsed output identical.
- Backtrace handling is untouched. Swift has no separate `file:line` path; frames go through the same `redact`.
- Symbols used: `NSRegularExpression(pattern:)` on a raw string, a static `let` built from another, and `Character.isWhitespace` are all already in this file. Atomic groups and possessive quantifiers are ICU syntax and compile on corelibs' ICU.

Tests (`ios/Tests/CrashCapturePolicyTests.swift`; XCTest cannot run here):
- New `testReportTextKeepsNoPartOfAPathOrFileURLThatContainsSpaces`: 16 cases, each also checked for leaked name fragments.
  - Paths: iOS container path with spaces, `: cause`, Windows drive, quoted, escaped quote inside a quoted path, doubled backslashes, apostrophe, `~/`, `<` and backtick delimiters, line break.
  - `file:` URLs: plain, with cause, quoted, upper-case.
  - Two cases showing that other URLs still end at whitespace.
- The existing `testReportTextRedactsReaderLocationsAndNonHTTPURLs` needed one input edit: `path=…` and `relative=…` are now on separate lines. Under the new rule the first path would swallow the second on the same line. The expected output is unchanged.

Linux harness results:
- Both test bodies run verbatim against the real `CrashIntake` text: `ALL PASSED`.
- Against base `d98cac2` the new test fails (observed: 46 assertion failures, every spaced case leaking); the existing test passes on both.
- The nine inputs of the Rust test `redaction_keeps_no_part_of_a_path_that_contains_spaces` give its exact expected strings.
- Differential test against the real Rust code, compiled standalone with `rustc` (no cargo):
  - `redact_paths` against the Swift path step: 300,000 random strings, 0 mismatches (69,262 contained a path).
  - `replace_urls` match extents (with `safe_url` stubbed to `[url]`) against the Swift URL step: 300,000 strings, 0 mismatches (204,895 contained a URL).

Not exercised: Darwin's `NSRegularExpression`; a real uncaught exception on a device. The harness makes one substitution: corelibs declares the `enumerateMatches` block `@escaping`, so `replacing`'s closure parameter is marked `@escaping` there.

**Commands run**
- `swiftc -parse` on `CrashIntake.swift` and `CrashCapturePolicyTests.swift` → clean; on all `ios/Sources`, `ios/Tests`, `gen/apple/DownloadActivity` → clean.
- `cd /tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/apple/crash && python3 gen.py current cur <two test names> && swiftc -swift-version 5 -o cur/run shims.swift parity.swift cur/subject.swift cur/tests.swift main.swift && ./cur/run` → `ALL PASSED`.
- `rustc -O paths.rs` and `rustc -O urls.rs` (extracted from `crash.rs`), compared with `diff/paths_swift` on `diff/cases.txt` and `diff/url_cases.txt` → `mismatches: 0 of 300000` each.

**Files changed**
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashIntake.swift`
- `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/CrashCapturePolicyTests.swift`

New user-visible strings: none.

**Handoffs / contract changes**: none. The report text can now be shorter: words between a path and its delimiter are dropped, as in Rust.

**Doubts**
1. A path that directly follows `<` is not recognised as a path, in Swift and in Rust alike: `<` ends a path but is not a start boundary. `see </var/mobile/My Books/a.zim>` is left intact by both. My first draft of the test assumed otherwise. This is outside "match the Rust rule", so I left it; it is a residual leak for the startup lane to decide.
2. Swift still applies its steps in a different order from Rust (URLs, bearer, secrets, paths, tokens against URLs, paths, secrets, tokens). I did not change that. The differential test compared the path step and the URL step separately, not the whole pipeline.
3. ICU's `\s` and Rust's `is_whitespace` differ on vertical tab and NEL (U+0085). I did not include those in the random alphabet.