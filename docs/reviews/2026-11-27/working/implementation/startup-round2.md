# Lane `startup` — round 2 report (changes since round 1 only)

All six review fixes and the canonical redaction pipeline are done. Lane tests, host clippy, Windows-gnu clippy and the unsafe census all pass, this time with clean exits on both clippy targets. Nothing was shown as a real dialog; the dialog wiring and the `cfg(macos)` code remain unexercised here.

## Part A — review fixes

### STARTUP-R1 — Cancel could leave Kiwix running with no window; a second Quit was dropped
Disposition: fixed.
Verification of the finding: Confirmed by reading: the `Stay` arm only reset `exiting`, and the exit handler dropped any request that arrived while `exiting` was set.
Change:
- `quit.rs: settle_failed_quit(action, shell_window_remains, quit_requested_while_asking)` — pure. A `Stay` becomes `Retry` when no `quit::shell_label` window remains or a Quit was requested while the dialog was open. I chose the Retry path over re-showing the same dialog: it gives up nothing, exits if the cause is fixed, and otherwise asks again with the fresh error.
- `lib.rs: QuitFlags { exiting, authorized, requested_again }` replaces the two loose flags (an eighth argument would also have tripped clippy). The handler sets `requested_again` when it holds a request back. The dialog clears it when it opens and reads it when it closes, and re-checks once after clearing `exiting`.
- The dialog is now parented (`failed_quit_dialog_parent`: focused shell window, else `main`, else first) and that window is un-minimized first. Per rfd 0.17.2 source: Windows makes the TaskDialog owned by the window, macOS shows a sheet, and GTK ignores the parent (rfd passes null).
- `docs/STATE_MACHINES.md` §1.1 updated for the new edges and invariant.

Tests: `quit::tests::cancel_cannot_leave_kiwix_running_without_a_window_or_drop_a_second_quit` (passes; new function, fail-without-fix inferred).
Not exercised: the live handler, parenting and un-minimize on any OS.

### STARTUP-R2 — forced-quit test could not fail
Disposition: fixed.
Change: `launch_tests::quitting_without_saving_still_retires_the_service_when_the_profile_is_gone` now opens an article so a reader listener is live, and `retire_without_saving` is the first and only retirement before the assertions. Before the call it asserts a TCP connect to the reader port succeeds and `ensure_running()` is `Ok`. After the call it asserts the connect is refused and commands are rejected. Only then does it check that an ordinary `shutdown()` still reports "Writing the last reading position".
Tests: passes. I did not run it with an emptied `retire_without_saving`: the target directory is 18 GB with 22 GB free, so a tree copy was not feasible. The paired before/after assertions are what make it able to fail.

### STARTUP-R3 — label collision rule
Disposition: fixed as specified. Quit Without Saving needs its label to differ from Retry and from Cancel; Retry maps only when it differs from Cancel.
Tests: the collision block in `quit::tests::a_failed_quit_can_be_retried_left_open_or_overridden_only_by_the_explicit_button` was rewritten, including "Retry = Cancel still lets the destructive button work".

### STARTUP-R4 — unsafe docs
Disposition: fixed.
- `docs/RUST-UNSAFE-REVIEW.md`: one paragraph on the `MessageBoxW` site; the gate sentence now says 75 blocks in 20 files.
- `docs/MEMORY-SAFETY-INVENTORY.md`: row B8 names the site; a new paragraph gives the gate's own figures (75 in 20 files: 60 app, 15 `win-security`, 0 relay, from `--print`).
- I left the older "73 / 58" sentence alone and said it was not recounted, because I cannot verify its "production" definition.

### STARTUP-R6 — dismissed zenity opened kdialog as well
Disposition: fixed.
Change: `instance.rs: helper_showed_dialog(program, code)`. Any exit status from a spawned helper counts as shown, except zenity 255; a helper ended by a signal counts as not shown.
kdialog equivalent: I found none. From memory, Qt's parser exits 1 on a bad command line, the same as a dismissed dialog. Neither helper is installed here, so this is unverified. kdialog is last in the chain, so a wrong guess costs only the stderr copy.
Tests: `instance::tests::a_dismissed_dialog_counts_as_shown_so_the_message_is_not_repeated`.

### STARTUP-R7 — `open_paths` contract
Disposition: fixed. The comment is corrected; when delivery to a window fails after import, the `Err` text now ends with the per-file failures (`undelivered_launch_error`).
Tests: added to `launch_tests::unreadable_launch_archives_are_reported_and_readable_ones_still_open` (mixed launch with no window gives the error plus the broken file's line).

## Part B — canonical crash redaction

Disposition: implemented in `crash.rs`, pinned to `src-tauri/tests/fixtures/crash-redaction-cases.json`: 110 cases, exactly `{input, expected}`, ASCII-only with `\u` escapes.

Verification:
- **Vectors:** `crash::tests::redaction_matches_the_canonical_cases_shared_with_the_mobile_intakes` passes.
- **Fail-without-fix, in scratch copies:** against the 99 cases that existed at that point, the base pipeline differed on 63 and the round-1 pipeline on 37. The round-1 mismatches include the real leak `open /var/x password: "hunter2"` → `open [path]: "hunter2"`.
- **Leak search:** a scratch combinatorial run (31,080 inputs: 37 left contexts × 30 path/URL shapes × 14 right contexts, through both `redact` and `redact_line`) ends with 0 sentinel leaks. It found one leak I had introduced (an unterminated single-quoted path containing an apostrophe), which is fixed and pinned.

### The pipeline the ports must follow

Definitions:
- **Whitespace** is Unicode White_Space.
- **`path_end(start)`** scans forward and stops before the first of:
  - LF or CR;
  - `<`, `>` or a backtick;
  - a `"` not preceded by an odd run of backslashes (counted back no further than `start`);
  - a `:` whose next character is whitespace;
  - end of text.
  
  It then drops trailing whitespace from the range.
- **Path start shape** is one of:
  - an ASCII letter, `:`, then `\` or `/`;
  - `/` followed by a character that is neither `/` nor whitespace;
  - `~` followed by `/` or `\`;
  - `\\`;
  - `//` followed by a character that is neither `/` nor whitespace.
- **Boundary** is start of text, or a previous character that is whitespace or one of `= ( [ { " ' , ; : ` `` ` `` ` < > | “ ‘`.

Steps for a message field (`redact`):
1. **Bound.** Keep the first 32,768 characters.
2. **Secrets**, one left-to-right scan. At each position try the keys in this order, ignoring ASCII case: `authorization, access_token, refresh_token, api_key, api-key, apikey, password, passwd, cookie, bearer, token, secret, pin`.
   - The character before the key must not be an ASCII letter or digit (an underscore is allowed). The character after must not be an ASCII letter, digit or underscore.
   - Then, in order: an optional single `"` or `'` (a quoted key); any run of space, tab, CR, LF; a separator `=`, or `:` not followed by another `:`; then any spaces or tabs.
   - With no separator, only `authorization` and `bearer` match, and only if the key was not quoted and at least one whitespace character was skipped.
   - The value, starting at `v`:
     - `"` — from `v+1` to the first unescaped `"` on the line (escape run counted from `v+1`), else to the line end.
     - `'` — from `v+1` to the last `'` on the line, else to the line end. If that range contains `/` or `\`, the end is at least `path_end(v+1)`.
     - key `cookie` — from `v` to the line end.
     - otherwise — one token ending at space, tab, CR, LF or one of `, ; & ) ] }`. For `authorization` with a non-empty first token, skip spaces, tabs, CR and LF and include a non-empty second token. If the range contains `/` or `\`, the end is at least `path_end(v)`.
   - An empty value is no match; try the next key. Otherwise replace the value with `[redacted]` and continue at its end.
3. **URLs.** For each `:`, the scheme is the run of `[A-Za-z0-9+.-]` just before it; it must start with a letter. It is a URL if the tail starts with `//`, or if the scheme is one of `blob content data file mailto tauri tel zim` (any case) and the tail does not start with `:`.
   - `token_end` is the first whitespace, `"`, `<`, `>` or backtick.
   - `file` with a tail that does not start with whitespace: through `path_end(tail)`, output `[url]`.
   - `data` likewise: to the line end, output `[url]`.
   - Otherwise apply the existing `safe_url` to the token: `[reader-url]`, `[url]`, or a kept `scheme://host[:port]path`. If the tail starts with `//` and the result is a placeholder, the range ends at `max(token_end, path_end(tail))`.
4. **Paths.** At every boundary with a path start shape, replace through `path_end` with `[path]`. The `//` shape directly after `:` is skipped, so `https://example.org/a` is not mangled.
5. **Opaque tokens.** Runs of `[A-Za-z0-9_+/=.-]` of 40 or more characters that contain a digit and a letter become `[redacted]`.
6. **Whitespace.** Split on whitespace, join with one space.
7. **Cut** to 2,000 characters (1,999 plus `…`). No vector is that long.

Backtrace frame lines run steps 1–4 only. The `at file:line` line is unchanged (file name only). Panic locations such as `lib.rs:1234` and `token.rs:42` are pinned unchanged.

Beyond what you listed, I added the following, each pinned by vectors:
- quoted values end at their closing quote, not the first space;
- JSON-style quoted keys (`"password":"…"`);
- `authorization` takes two tokens (`Basic …`);
- `cookie` takes the rest of the line;
- key `api-key`;
- an underscore may precede a key (`client_secret=`);
- `::` is not a separator, so Rust symbols like `auth::token::rotate` survive;
- `data:` URLs go to the line end;
- placeholder `scheme://` URLs (reader, `content://`, `smb://`) go whole, like paths.

For `file:` I kept one exception to "any tail": `file: ` followed by a space is treated as prose, and the path after it is caught by step 4.

Known residuals, deliberately not pinned as "expected":
- a name containing `: `, `<`, `>`, a backtick or an unescaped `"`;
- relative paths (`My Books/a.zim`, `%USERPROFILE%\…`);
- text after a kept external URL that is written with raw spaces;
- a spaced unquoted secret with no slash.

## Commands run
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- launch_tests quit:: crash:: instance:: launch_target_tests maintenance_cadence_tests navigation_policy_tests` → `test result: ok. 37 passed; 0 failed; 0 ignored`. After the final fixture regeneration (escaping only), `-- crash::` → `ok. 8 passed`.
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 0, `Finished`. Earlier attempts failed only on `service.rs:211` `type_complexity`, another lane.
- Windows-gnu clippy (brief's command) → exit 0, `Finished`.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented.`

## Files changed this round
`src-tauri/src/crash.rs`, `src-tauri/src/quit.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/instance.rs`, `docs/STATE_MACHINES.md` (§1.1), `docs/RUST-UNSAFE-REVIEW.md`, `docs/MEMORY-SAFETY-INVENTORY.md`, and new `src-tauri/tests/fixtures/crash-redaction-cases.json`. No new user-visible strings.

## Handoffs / contract changes
- **iOS and Android lanes:** port the seven steps above and assert the fixture verbatim.
  - Output shapes that differ from today's ports: `Authorization: [redacted]` (separator kept, no `key=` normalisation); the opaque-token rule (dots allowed, a digit and a letter required); placeholders swallowing the rest of a clause.
  - The 32,768 bound is in characters. It is not observable in the vectors, so each port needs its own long-input test.
- **Crash record text** is more heavily redacted than in round 1 (a clause after a reader URL or path-like secret goes). Nothing parses it that I could find.
- **Desktop lane:** round 1's §1.5 note still stands. In addition, the exit handler's flags are now `QuitFlags`.

## Doubts
- **Cancel with no window quits on macOS too** if the retry succeeds. macOS apps can normally stay open without a window, but this app has no reopen handler, so I applied the rule everywhere.
- **Parenting** un-minimizes a window and, on macOS, turns the dialog into a sheet on a window that may be frozen showing "Quitting…". None of this was seen running.
- **zenity status 1** also covers "could not open display" when `DISPLAY` is set but unusable; that now counts as shown, so kdialog is not tried. The stderr line is still printed first on both callers.
- **Over-redaction grew:**
  - `unexpected </p> tag` becomes `unexpected <[path]> tag`;
  - prose after a reader URL or after `cookie:` is dropped;
  - an Objective-C selector such as `token:forUser:` loses its tail (as before).
- **Whitespace definitions differ across runtimes** (NEL, U+001C–001F). Only NBSP is pinned.
- **Worst-case cost** on adversarial 32k input is quadratic in a few scans (roughly tenths of a second by estimate, not measured).
