## Verdict for lane startup, round 2: only non-blocking findings. Part B still lets secrets through and is ambiguous enough that the ports could drift, so settle B-1 to B-5 before the Swift and Kotlin ports pin the fixture.

I am not changing the expected value of any existing vector. Everything below is either an addition or an open question for the owner. I prototyped the B-1 and B-2 fixes in a scratch copy, and all 110 existing vectors still produced the same output.

## Part B: canonical redaction (`crash.rs:415-848`, `tests/fixtures/crash-redaction-cases.json`)

How I tested: I copied `crash.rs:415-848` into a scratch crate (`.../adv-startup/redact2`). It matches the fixture 110 out of 110. I then ran it on about 80 adversarial inputs and on 32K-character inputs for timing.

### STARTUP-R2-B1: secret keys inside escaped JSON or tuples are not recognised
Severity: Should-fix (before the ports pin the fixture). Evidence: reproduced.
Location: `crash.rs` `secret_at`, which only accepts a bare `"`/`'` as the key's closing quote, and `secret_value`.

| Input | Output (secret kept) |
|---|---|
| `{\"password\":\"hunter2\"}` | unchanged |
| `Err(Custom("{\"token\":\"abc123\"}"))` | unchanged |
| `[("authorization", "Basic dXNlcjpwYXNz")]` | unchanged |
| `(authorization, Basic dXNlcjpwYXNz)` | unchanged |

- The first two shapes are what Rust `{:?}` and Swift `String(reflecting:)` print for any error that carries JSON.
- The last two are how Rust, Kotlin and Swift print a tuple or `Pair`.
- Smallest fix:
  - Accept `\"` / `\'` (one backslash, then the quote) as a quoted key.
  - Treat a value that starts with `\"` as quoted up to the next `\"`.
  - Accept `,` as a separator after a quoted key, or after `authorization`.
- I tested the first two parts in scratch: they redact both escaped cases and leave all 110 vectors unchanged.

### STARTUP-R2-B2: camelCase and common unlisted keys leak
Severity: Should-fix. Evidence: reproduced.
- camelCase:
  - `accessToken=abc123 refreshToken=def456 authToken=z9` passes through unchanged, and so does `{"accessToken":"abc123"}`.
  - Only `apiKey` is caught, because it matches the listed key `apikey`.
  - Cause: the rule "the character before the key must not be an ASCII letter" rejects `sT` in `accessToken`.
- Keys not in the list at all, all passed through: `pwd=`, `pass=`, `passphrase=`, `private_key=`, `sessionid=`, `auth=`, `signature=`, and `;jsessionid=ABC123DEF` (which stays inside a kept URL path).
- Smallest fix:
  - Also allow a match when the previous character is an ASCII lowercase letter and the key's first character in the input is uppercase. I tested this in scratch: 0 of 110 vectors change.
  - Decide on the extra keys now, because ports pin whatever list exists.

### STARTUP-R2-B3: values the key rules do not reach
Severity: Nit to Should-fix (owner's call). Evidence: reproduced.

| Input | Output | What leaks |
|---|---|---|
| `password: Some("hunter two")` | `password: [redacted] two")` | ` two")` |
| `password\u00A0=hunter2` | unchanged | the value |
| `password=\r\nhunter2` | unchanged | the value |
| `password:\n  hunter2` | unchanged | the value |
| `'pin' => '1234'` | `'pin' =[redacted] '1234'` | `'1234'` |

- The `Some(...)` / `Optional(...)` wrappers are Rust and Swift debug output.
- Smallest fix:
  - Treat a value of the form `Name(` + a quoted string as a quoted value.
  - Skip Unicode whitespace, not just ASCII, around the separator.
  - Allow CR/LF after the separator.

### STARTUP-R2-B4: path starts that are still missed
Severity: Should-fix for the contract. Evidence: reproduced.
- These inputs pass through unchanged:
  - after `„`: `„/Users/Jane Doe/a.zim“` (German),
  - after `「`: `「/Users/Jane Doe/a.zim」` (CJK quote),
  - after `«` with no space: `«/Users/Jane Doe/a.zim»`,
  - after full-width `：`: `：/Users/Jane Doe/a.zim`,
  - after `@` or `#`: `@/home/jane doe/x.zim`, `#/home/jane doe/x.zim`,
  - after U+001F or U+200B: `x\u001F/home/jane doe/a.zim`, `x\u200B/home/jane doe/a.zim`,
  - single-backslash paths: `open \Device\HarddiskVolume3\Users\Jane Doe\a.zim`, `open \Users\Jane Doe\a.zim`.
- Smallest fix:
  - Define the boundary as "start of text, or a previous character that is not a letter, digit, `_`, `.` or `-`", instead of an allow-list.
  - Decide whether a single `\` followed by a name and a later `\` is a path start.
  - Re-run the fixture afterwards; I did not prototype this change.

### STARTUP-R2-B5: the spec is not precise enough to port
Severity: Should-fix. Evidence: source-traced, plus reproduced runtime facts.
1. **Unit.** Every count, scan and cut (32,768, 2,000, 240, the 40-character opaque rule) must be over Unicode scalar values.
   - Swift `Character` is a grapheme cluster, so `"\r\n"` is one character that equals neither CR nor LF. Its `count` is wrong for the cuts too.
   - Kotlin `substring` or `length` work on UTF-16 units and can split a surrogate pair. A lone surrogate then breaks JSON writing.
   - The spec should require `unicodeScalars` in Swift and code points in Kotlin.
2. **Whitespace.** Give the explicit White_Space code-point list.
   - Kotlin `isWhitespace` is true for U+001C–U+001F and false for U+0085.
   - Java and Kotlin regex `\s` is ASCII-only.
   - Rust redacts `x\u0085/home/jane doe/a.zim` to `x [path]`; a Kotlin port would leak it.
   - Pin vectors for U+0085, U+001F, U+2028, U+3000, U+200B (not whitespace) and CRLF.
3. **ASCII-only case folding** must be stated. Kotlin `equals(ignoreCase=true)` treats `ſ` as `s` and the Kelvin sign as `k`.
   - Pin `pasſword=hunter2` and `coo\u212Aie=secret`, both unchanged under the current rule.
4. **`safe_url` is not specified at all**, yet it decides "reader URL" versus "kept" using WHATWG parsing. Java `URI` and Swift `URL` parse these differently, so a port would keep the book and article path:
   - `http://127.1:8080/…`, `http://0x7f.0.0.1/…`, `http://2130706433/…`, `http://[::ffff:127.0.0.1]/…` and `http://LOCALHOST/…` all give `[reader-url]` in Rust.
   - Kept URLs are also normalised:
     - `https://Example.ORG/a/%C3%A4/../b?token=x` becomes `https://example.org/a/b`;
     - `https://example.org/ä b` becomes `https://example.org/%C3%A4 b`;
     - `:443` is dropped;
     - `http://example.org\@127.0.0.1/A` becomes `http://example.org/@127.0.0.1/A`.
   - None of these forms is pinned. Either specify a textual algorithm and pin it, or pin these cases.
5. **Smaller wording gaps:**
   - Step 2's value forms must be tried in the listed order (quoted forms before `cookie`), and the first key with a non-empty value wins.
   - Step 3: colons inside a range already replaced are skipped, and the scheme run cannot reach back past the previous replacement.
   - An empty `file:` tail becomes `[url]`. The report calls `file: ` "prose", but the output is `cannot open [url] [path]`. The fixture pins that, so the report's wording is what is wrong.
   - Step 6 must drop empty pieces.
   - Frame lines: the `at` line runs the URL step, then `file_name` (last `/` or `\` component), then the secret step. Frame detection and the 240 cut are unstated. The ports' stack formats differ anyway, so say that this part is per-platform.
6. **The 32K cut can split a reader URL.** In a 32K input of spaces followed by `http://12b34567.localhost:9/A/…` straddling character 32,768, the host is cut to `12b34567.loc`, which is kept. This is contrived; a Nit.

### STARTUP-R2-B6: worst-case cost is about 100 ms here and quadratic
Severity: Nit for Rust, Should-fix guidance for the ports. Evidence: measured (release build, x86, `redact` plus `redact_line` on 32,768 characters).

| Repeated unit | Time |
|---|---|
| `a://b<` | 98 ms |
| `file:x<` | 93 ms |
| `` zz://q` `` | 88 ms |
| `password="" ` | 40 ms |
| any other shape I tried | 2 ms or less |

- Causes:
  - `replace_urls` `path_like_end` collects the whole remaining tail into a new `Vec<char>` for every URL candidate.
  - `secret_value` computes `line_end` eagerly even when the quoted value turns out empty.
- A Swift port that walks `Character`s could be ten times slower or more on a phone, inside crash intake.
- Fix: share one char array across `path_end` and compute line ends lazily or once. The spec should require linear time.

Not leaks, for reference:
- URL userinfo and query are dropped, and `ftp://user:pass@…` and `smb://` become `[url]`.
- LAN sharing URLs end up as `http://192.168.1.5:[redacted]`, but only because the 40-character opaque rule happens to catch them. A `.local` host with an article path is kept (Nit).
- NSError and `FileNotFoundException` shapes and `Optional("<path>")` redact correctly.

## Part A

- **R1: closed.**
  - The `requested_again` handling is race-free by my trace. The handler sets the flag only while `exiting` is set. The callback swaps it before deciding and again after `exiting.store(false)`. A request arriving after that store starts its own attempt and never sets the flag, so nothing is lost or doubled. A window destroyed while the dialog closes is caught by one of the two swaps or by `shell_window_remains`.
  - The parent window removes my earlier macOS Escape doubt whenever a window exists: NSAlert sheet, and "Quit Without Saving" is not "Don't Save".
  - Cancel with no window becomes Retry on macOS too. That is consistent, because Tauri raises ExitRequested when the last window closes.
  - New Nit, source-traced: if the dialog cannot be displayed at all (`TaskDialogIndirect` or `CFUserNotificationDisplayAlert` fails and reports Cancel) and no window remains, Cancel→Retry→prepare→shutdown fails→dialog fails again. That is a CPU-spinning, log-spamming loop. Fix: after N immediate failures, or when the dialog returns within a few milliseconds, stop retrying and log instead.
- **R2: closed.** The test can now fail. With an empty `retire_without_saving`, `ensure_running()` would still be `Ok` and the TCP connect to the reader port would succeed. `flush_browsing` is the last stage, so every earlier stage provably ran.
- **R3: closed** (code read; the test was rewritten).
- **R4: closed.** Both documents updated; the census reports 75/75/2 in 20 files.
- **R6: fixed, but with a new finding.**
  - New finding (Should-fix, source-traced): the report says "the stderr line is still printed first on both callers". That is false for the busy-profile dialog. `instance.rs:124-130` `show_profile_busy` prints to stderr only when `!show_native_message(...)`.
  - Trigger: zenity exits 1 when GTK cannot open a set but unusable `DISPLAY` (for example a stale SSH `DISPLAY`). That now counts as "shown", so the busy message reaches no one.
  - kdialog exits 1 on a bad command line, which also counts as shown. A kdialog killed by a signal correctly counts as not shown.
  - Zenity's 255 for an unusable command line matches my reading of zenity's option parser, but I could not run it here.
  - Fix: always `eprintln!` in `show_profile_busy` before showing the dialog, as `exit_after_startup_failure` already does.
- **R7: closed.** `undelivered_launch_error` keeps the per-file lines and the comment is corrected.

## What I ran
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- crash:: quit:: launch_tests instance::` → `test result: ok. 34 passed; 0 failed; … 1459 filtered out`.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented.`
- Scratch pipeline copy: 0 of 110 fixture mismatches, the leak probes above, the timings above, and the B-1/B-2 prototype (0 of 110 vectors changed).
- I did not re-run either clippy this round.

## What I could not verify
- No dialog ran on any OS: parenting, un-minimize, the macOS sheet, and the zenity and kdialog exit codes.
- The Kotlin and Swift behaviour described in B-5 comes from documented runtime semantics, not from running ports.