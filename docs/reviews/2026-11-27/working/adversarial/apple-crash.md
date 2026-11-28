## Verdict for lane apple (crash-redaction follow-up): only non-blocking findings

The change does what it claims: a path or `file:` URL with spaces is now fully redacted, stack frames are untouched, and cost is linear. One new weakness is worth a one-line fix: on a single line longer than about half a million characters the new pattern stops matching and the path is left in clear. Nothing was built against an Apple SDK; the regex ran on corelibs' ICU, not Darwin's.

### CRASH-R1 — Above roughly 0.5–1 M characters on one line, the new path and `file:` patterns fail open
Severity: Should-fix (very unlikely input, but a redactor should not fail open, and base did not on this input)
Fix under review: CORE-8 (Swift half)
Evidence class: reproduced on corelibs ICU; inference that Darwin's limit is similar
Location: `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashIntake.swift:25` (`pathTail`), `:155-168` (`redact`), `:69-71` (callers redact before truncating)
Trigger: An exception reason containing a path start followed by about 1,000,000 UTF-16 units (about 500,000 after `file:/`) with no `"`, `<`, `>`, backtick, line break or `: `.
Causal path:
- `(?>…)*+` discards its saved state only when the loop ends, so ICU keeps one state per character until then.
- Past ICU's backtrack-stack limit the match is abandoned, `enumerateMatches` reports nothing, and `replacing` copies the text through.
- Measured: redaction works at 200,000 units and stops at about 1,000,750 for a path and about 500,875 for a `file:` URL.
- Base's single character-class loop has no such limit: a 1,000,000-colon run after `/` gives `[path]` on base and is left intact now.
- `writeRecord` calls `redact(...)` on the whole reason and only then takes `.prefix(maxMessage)`, so the first 2,000 characters of the unredacted text would be stored.
Smallest correct fix: bound the input at the top of `redact`, for example `let value = String(value.prefix(32_768))`. That is far above the 2,000 and 240 character output caps and far below the limit.

### CRASH-R2 — A path directly after `<` or a typographic quote is still not recognised (implementer doubt 1)
Severity: Should-fix, pre-existing, needs the Rust rule changed in step
Fix under review: CORE-8
Evidence class: reproduced in harness, identical on base and current
Location: `CrashIntake.swift:34` (start boundary `(?<![^\s=(\[{"',;:`])`); `/home/user/kiwix-tauri/src-tauri/src/crash.rs:533-539` (`path_at`)
Trigger: A reason that puts a path straight after `<` or `“`.
Causal path:
- AppKit and UIKit do produce the `<` shape: `Could not load NIB in bundle: 'NSBundle </private/var/containers/Bundle/Application/<UUID>/Kiwix.app> (loaded)'`. That one comes out as `</[redacted].app>`, but only because the 40-character token rule catches a long run without spaces.
- A path with spaces is not caught: `at </var/mobile/…/File Provider Storage/My Books/a.zim>` leaves `Provider Storage/My Books/a.zim`.
- `“/var/mobile/My Books/a.zim”` is left whole.
- So this is a real leak shape, though I know no system message that puts a user's document path there.
Smallest correct fix: add `<` (and `“`, `‘` if wanted) to the start set in Swift and in `path_at` together, then extend the differential test. Do not change one side alone.

### CRASH-R3 — The test edit avoids, rather than asserts, the new same-line behaviour
Severity: Nit
Fix under review: CORE-8 test
Evidence class: reproduced
Location: `/home/user/kiwix-tauri/src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests/CrashCapturePolicyTests.swift:11-12`
Trigger: `path=/book-uuid/A/Private relative=/.kiwix/token/A/Private` on one line.
Causal path: This now gives `path=[path]`. Nothing leaks; the `relative=` label is lost. Splitting the line keeps the old expectation green, so no privacy regression is hidden, but the swallow is no longer exercised.
Smallest correct fix: add the one-line input with expected `path=[path]` to the new test's cases.

**Answers to the four questions**
1. **Regex as ICU reads it:** it compiles on ICU and I found no syntax Darwin would reject.
   - The atomic group makes each step deterministic (`\\\\` before `\\"` before `\\` gives the backslash parity).
   - **Leaks still present, all the same on base and in Rust:** after `<` or curly quotes (R2); `//server/share`; and the text after `: ` inside a name (`/var/mobile/Notes: Private Diary/a.zim` leaves `Private Diary/a.zim`).
   - **Over-redaction of frames:** none. Realistic `callStackSymbols` lines, including `<redacted>` and Objective-C selectors, come out identical to base. Swift frames carry no `file:line`.
   - **Diagnostic loss:** message text after a path up to the next delimiter is dropped, for example `NSFilePath=[path] "No such file or directory"}}` loses the underlying error's domain and code. That is the adopted rule.
2. **Backtracking and cost:** no catastrophic backtracking. Nothing follows the possessive tail in the path pattern, failed starts cost one character, and matches do not overlap. Measured: 1.4 M characters of short paths in 0.65 s, 1 M backslashes in 0.04 s. The constructs bound time but not ICU's state stack; that is R1.
3. **Test split:** see R3.
4. **`<`:** see R2.

One observation for the core lane, read from source and not run: Rust redacts paths before secrets (`crash.rs:426-428`), so `open /var/x password: "hunter2"` would lose the `password` key to the path and may leave the value. Swift redacts secrets first and gives `open [path]`.

**What I ran**
- `swiftc -parse` over `ios/Sources`, `ios/Tests` and `gen/apple/DownloadActivity`: exit 0.
- Implementer's harness: current `ALL PASSED`; on base the new test and the Rust-parity cases fail (62 failures).
- My probes and the length search, in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-apple/c/` (`probe.swift`, `th/main.swift`).
- I did not re-run the 300,000-case differential against Rust.

**What I could not verify**
- Darwin's `NSRegularExpression`: that the pattern compiles (it is `try!`, so a failure would trap on first use), where its stack limit sits, and whether an abandoned match behaves as on corelibs.
- A real uncaught exception on a device.
- ICU `\s` against Rust `is_whitespace` for vertical tab and NEL.