# Lane `startup` — round 3 report (changes since round 2 only)

Both Part A items are fixed and Part B v2 is implemented, written down as a spec in the repo, and pinned by 202 vectors. Lane tests, host clippy, Windows-gnu clippy and the unsafe census all pass. As before, no dialog was shown on any OS and no macOS code was compiled.

## Part A

### R6 (new) — busy-profile message could reach no one
Disposition: fixed.
Verification of the finding: Confirmed. `show_profile_busy` printed only when the helper reported "not shown", and round 2 made a zenity exit 1 count as shown. My round-2 claim that both callers print first was wrong for this one.
Change: `instance.rs: show_profile_busy` now always `eprintln!`s the full message, then tries the dialog.
Tests: none added (one unconditional line). The existing `instance::` tests pass.

### Dialog-failure loop — Cancel→Retry could spin
Disposition: fixed.
Verification of the finding: Confirmed by reading: with no window and an undisplayable dialog, `settle_failed_quit` turned every instant Cancel into an immediate retry.
Change:
- Neither the plugin nor rfd exposes an error path (a failed `TaskDialogIndirect` returns plain Cancel), so I used the timing rule.
- `quit.rs: failed_quit_dialog_was_shown(answer, elapsed)` — a Cancel that arrives in under 50 ms counts as not shown. Retry and Quit Without Saving always count as answers, because only their buttons produce them.
- `quit.rs: unshown_dialog_retry_delay(n)` — 0, then 1, 2, 4, 8, 16, 32, 60, 60… seconds.
- `lib.rs`: `QuitFlags.unshown_dialogs` counts consecutive unshown dialogs and resets when one is shown. A resulting Retry waits the delay first (with `exiting` still set) and logs one line per occurrence. A resulting Stay logs once. Nothing in this path can reach Quit Without Saving.
- `docs/STATE_MACHINES.md` §1.1 has a new row for this.

Tests: `quit::tests::a_dialog_that_cannot_be_shown_is_retried_with_backoff_and_never_quits` (pure; passes; fail-without-fix inferred, the functions are new).
Not exercised: the live handler and the sleep.

## Part B v2 — canonical redaction

Disposition: B1–B6 adopted as decided. The implementation in `crash.rs` was rewritten over one scalar array per step, and `crash.rs` now points to the spec.

### What was produced
- **Spec:** `src-tauri/tests/fixtures/crash-redaction-spec.md`. It covers:
  - the unit (scalars, with the Swift and Kotlin instructions);
  - the explicit White_Space list;
  - ASCII-only folding;
  - `path_end`;
  - the seven steps, including the textual URL rule;
  - the frame-line handling as Rust does it, marked per-platform;
  - the linear-time requirement;
  - the known residuals and over-redactions.
- **Vectors:** `crash-redaction-cases.json` now has 202 cases (110 kept, 92 added), ASCII-only with `\u` escapes.
- **Tests in `crash.rs`:**
  - the fixture test;
  - `whitespace_is_exactly_the_unicode_white_space_property` (every scalar against the spec's list);
  - `redaction_input_is_bounded_before_any_step_runs` (generated inputs for B5.6, including a reader URL straddling scalar 32,768);
  - `every_redaction_step_is_linear_in_its_input`.

### Decisions as implemented, with my additions
- **B1.**
  - Key quote may be `\"` or `\'`.
  - A `\"` value ends at the next `"` with exactly one backslash before it, so an embedded `\\\"` does not close it early.
  - `,` separates after a quoted key or `authorization`.
  - I limited this to one backslash: with two or more, the text already starts with `\\` and is taken as a share path (pinned: `"{[path]"`).
- **B2.**
  - The camelCase rule is implemented. I also allow a digit before the hump (`oauth2Token`).
  - Keys added exactly as listed.
  - `::` is still excluded.
- **B3.**
  - `Name(` wrappers before a quoted value (nesting allowed).
  - White_Space around the separator, line breaks included.
  - `=>`.
  - A token now also ends at any White_Space, not only the four ASCII ones.
- **B4.**
  - Boundary and rooted `\Name\…` start as decided.
  - Addition, needed once `/` became a boundary: neither slash of `://` starts a path. Without it the kept `https://example.org` was mangled.
- **B5 URL rule** as decided, with three choices of mine:
  - One trailing dot is removed from the host. This keeps the existing `…localhost.:9` vector as `[reader-url]`.
  - Every host whose last label is `local`, `localdomain`, `lan`, `home`, `internal` or `arpa` is `[url]`, with or without a path. The decision named only `.local` hosts with paths; the device name itself is personal.
  - A kept URL that has a path, query or fragment consumes the rest of its clause, like a placeholder. Otherwise the tail of an unencoded article title after the first space would survive the "path is dropped" rule.
- **B5.5 wording** is in the spec:
  - value forms in order;
  - replaced ranges skipped;
  - empty `file:` → `[url]`;
  - step 6 drops empty pieces;
  - frame lines per platform.
- **B5.6** as decided.
- **B6.** Linear: line ends and last apostrophes are precomputed once per step, and every scan covers text the match consumes.
- **Found by the leak search, not in your list:** a scheme run that started with a non-letter (`+https://…`, `1.http://…`) was not treated as a URL at all. The scheme now starts at the run's first letter.

### Vectors changed (8, all from the URL decision)

| Input | Was | Now |
|---|---|---|
| `mixed file:///Users/Jane Doe/a.zim: denied, see https://example.org/help` | `mixed [url]: denied, see https://example.org/help` | `mixed [url]: denied, see https://example.org` |
| `bridge https://example.org/book-uuid/%2Ekiwix/token/A/Private` | `bridge [reader-url]` | `bridge https://example.org` |
| `adjacent https://example.org/a,http://book-uuid.localhost/A/Private` | `adjacent [reader-url]` | `adjacent https://example.org` |
| `fetch https://user:pass@example.org/a/b?token=secret#x failed` | `fetch https://example.org/a/b failed` | `fetch https://example.org` |
| `see https://example.org/docs/a for help` | unchanged | `see https://example.org` |
| `see https://example.org//double/slash kept` | unchanged | `see https://example.org` |
| `https://example.org/cb?token=abc/def&state=xyz#frag done` | `https://example.org/cb` | `https://example.org` |
| `open /Users/alice/…?token=abc failed: https://example.org/read?token=secret&q=book#frag` | `open [path]: https://example.org/read` | `open [path]: https://example.org` |

- Rows 2 and 3 change placeholder because reader detection is now by host only; path-based detection (`/.kiwix/`, nested `://`) is gone with the path.
- The trailing words go in rows 4–6 because a URL with a path now consumes its clause.
- No vector was removed.

### Vectors added (92)
The complete list, as `input → expected` JSON strings, is in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/startup/vectors-added-v2.txt`. By group:

- **B1 (7):**
  - `{\"password\":\"hunter2\"}`
  - `Err(Custom("{\"token\":\"abc123\"}"))`
  - `[("authorization", "Basic dXNlcjpwYXNz")]`
  - `(authorization, Basic dXNlcjpwYXNz)`
  - the double-escaped form (→ `"{[path]"`)
  - an escaped value containing `\\\"hi\\\"`
  - `("token", "abc def") and ["cookie", "sid=1"]`
- **B2 (6):**
  - `accessToken=… refreshToken=… apiKey=… authToken=…`
  - `{"accessToken":"abc123","sessionId":"s1"}`
  - `pwd= pass= passphrase= private_key= sessionid= auth= signature=`
  - `private-key: privateKey= session_id= jsessionid= credential= credentials: client_secret=`
  - `auth::login failed and oauth2Token=abc but author=jane stays`
  - `https://example.org/app;jsessionid=ABC123DEF?x=1`
- **B3 (7):**
  - `password: Some("hunter two")`
  - `token: Optional("abc def") rest`
  - `password\u00a0=hunter2`
  - `password=\r\nhunter2`
  - `password:\n  hunter2`
  - `'pin' => '1234'`
  - `{"password"=>"hunter2"}`
- **B4 (17):**
  - paths after `„`, `「`, `«`, `：`, `@`, `#`, U+001F and U+200B
  - `\Device\HarddiskVolume3\Users\Jane Doe\a.zim`
  - `\Users\Jane Doe\a.zim`
  - prose that stays: `and/or n/a 1/2 2024/11/28 I/O w/o read_only/x v1.2/x a-b/c`
  - prose that goes: `tolerance +/- 5 percent`, `café/menu und/oder`, `rate 50%/year`, `regex \d+\.\d+ fails`
  - `line1\nline2 and a \n b stay`
  - `kiwix:///Private Page`
- **B5 whitespace and case (9):**
  - U+0085 and U+3000 before a path
  - `password=hunter2\u2028rest`
  - `token=a\u001fb next`
  - CRLF after a Windows path
  - a mixed-whitespace string
  - `pasſword=hunter2` and `coo\u212aie=secret` (both unchanged)
  - `cooKie=s1 PASSWORD=p2 SeCrEt=s3`
- **B5 URLs (31):**
  - `127.1:8080`, `0x7f.0.0.1`, `2130706433`, `[::ffff:127.0.0.1]`, `LOCALHOST`, `localhost.`
  - `Example.ORG/…?token=x`, `/ä b`, `user:pass@`, `:443`, `:80`, `:8443`
  - `example.org\@127.0.0.1/A`
  - a bare kept URL in prose, `HTTPS://EXAMPLE.ORG.`
  - a punycode host, a non-ASCII host, a bad port
  - `wss://`, `ftp://`, a single-label host
  - two `.local` cases
  - the three LAN sharing, catalog and nearby URLs
  - `see file:`, `file: /home/…`
  - `+https://…`, `1.http://…`
  - `mailto:`/`tel:`
- **Reviewer's remaining shapes (15):**
  - `Authorization failed for jane`
  - `bearer token missing`
  - `token\u00a0hunter2`
  - `"token" : "x y z"`
  - `smb://…: denied`
  - the Android `content://` tree URI
  - `token=abc;user=jane;password=hunter2`
  - `secret:42 token.rs:42 lib.rs:1234`
  - the `panicked at` message
  - the three-header line
  - the Debug-escaped path
  - `Optional("/var/mobile/…")`
  - the Android `file:///storage/…` URL
  - the NSError with curly quotes
  - the Java `FileNotFoundException`

### Over-redaction from B4 (as asked)
- **Goes:**
  - `+/- 5 percent` → `+[path]`
  - `café/menu …` → `café[path]`
  - `50%/year …` → `50%[path]`
  - `</p>` → `<[path]>`
  - several escapes in a row (`\d+\.\d+`, `"\nfoo\tbar"`) → `[path]`
- **Stays**, because an ASCII letter or digit precedes the `/`: `and/or`, `n/a`, `1/2`, `2024/11/28`, `I/O`, `w/o`.
- **From the new keys:** a word after `pass:`, `auth:`, `signature:` or `credentials:` in prose is redacted.

### Verification
- **Reviewer's probes:** all 66 inputs from `adv-startup/redact2` were run through a scratch copy of the pipeline and read one by one. What remains visible is only what the spec lists as not caught:
  - the two non-ASCII-folded keys (pinned unchanged by decision);
  - `token<NBSP>hunter2` with no separator;
  - a bare e-mail address;
  - `/home/jane/Notes: 2024/Private/a.zim` (a name containing `: `).
- **Leak search:** rerun with the v2 shapes and contexts: 142,500 inputs (75 left contexts × 50 path/URL shapes × 19 right contexts, through `redact` and `redact_line`), 0 sentinel leaks. The first run found 760, all the `+https://` scheme gap above.
- **Timing, scratch debug build:**

  | 32,768 scalars of | Round 2 | Now |
  |---|---|---|
  | `a://b<` | 1,032 ms | 21 ms |
  | `file:x<` | 870 ms | 15 ms |
  | `password="" ` | 406 ms | 12 ms |

  Round 2 grew ×15 for four times the input. Now 32k → 262k scalars takes ×6 to ×9 for an eightfold input.
- **Timing test bound I chose:** best of three runs. Four times the input may take at most ten times as long (linear is 4, quadratic is 16), with a 5 ms floor. The 32k run must also be under 300 ms. The round-2 code would fail both.
- **Old pipeline against the new vectors**, in a scratch copy: 64 of 202 differ.

## Commands run
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- crash:: quit:: launch_tests instance::` → `test result: ok. 37 passed; 0 failed; 0 ignored` (2.3 s).
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 0. My first run failed on two `manual_is_multiple_of` lints in `crash.rs`; fixed.
- Windows-gnu clippy (brief's command) → exit 0, `Finished`.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented.`

## Files changed this round
`src-tauri/src/crash.rs`, `src-tauri/src/quit.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/instance.rs`, `docs/STATE_MACHINES.md` (§1.1), `src-tauri/tests/fixtures/crash-redaction-cases.json`, and new `src-tauri/tests/fixtures/crash-redaction-spec.md`. No new user-visible dialog strings; two new stderr lines for the unshown-dialog case.

## Handoffs / contract changes
- **iOS and Android lanes:** port from `crash-redaction-spec.md` and assert all 202 vectors. Each port also needs its own tests for the 32,768 bound (including the dropped partial token) and for linear time, since neither is in the fixture.
- **Crash record text** changes for anything that shows records: an external URL now keeps only scheme and host, and LAN and IP URLs become `[url]`.
- `crash.rs` no longer uses the `url` or `percent-encoding` crates. Both are still used elsewhere in the crate.

## Doubts
- **The 50 ms rule** treats a Cancel from a held-down Escape key as "not shown". The effect is a log line, plus a 1 s wait if no window remains. Nothing is discarded.
- **Private-suffix list** (`lan`, `home`, `internal`, `localdomain`, `arpa` beyond `.local`) is my extension. `.home` and `.internal` could in principle be public names; they become `[url]`.
- **Prose after any URL with a path is now dropped**, for example `GET https://example.org/api failed with 500` → `GET https://example.org`. A `: ` still stops it, so `…/api: timeout` keeps `: timeout`.
- **Still not caught, by design:**
  - `OAuth=…` (uppercase before the hump);
  - keywords outside the list;
  - bare e-mail addresses;
  - relative paths;
  - names containing `: `, `<`, `>`, a backtick or an unescaped `"`.
- **The linear-time test is time-based.** The margins are wide, but a badly stalled container could still fail it.
- **The spec's Swift and Kotlin notes** (scalar views, whitespace functions) come from documented behaviour; I ran no port.
