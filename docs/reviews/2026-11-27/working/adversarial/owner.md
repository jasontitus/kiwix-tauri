## Verdict for lane owner (docs, crash spec, Rust localization test): only non-blocking findings

Review base `d98cac2`, read at HEAD `e01a6b1` with a clean tree. Paths reviewed: the listed sections of `docs/STATE_MACHINES.md`, `docs/PLATFORM-GAPS.md` and `docs/RELIABILITY.md`, the listed parts of `src-tauri/tests/fixtures/crash-redaction-spec.md`, and the new `src/locales/extra-en-rust.json` and `scripts/test-rust-localized-strings.mjs`. I edited nothing in the repository. Scratch files are under `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-owner/`.

### OWNER-R1 — The new localization test misses three call shapes; 16 Rust-localized strings are not catalog sources and the test still passes
Severity: Should-fix
Fix under review: `scripts/test-rust-localized-strings.mjs`, `src/locales/extra-en-rust.json`
Evidence class: reproduced (a scratch extractor, `adv-owner/extract.mjs`, run against the current tree)
Location: `scripts/test-rust-localized-strings.mjs:44` (the regex is `/\.text\(\s*"…"/` only)
Trigger: run `node --test scripts/test-rust-localized-strings.mjs`. It passes, but these strings reach the Localizer and are in neither `messages/en.json` nor any `extra-en-*.json`:
- **`t("…")` closures** (`let t = |source| escape(&locale.text(source, &[]))`):
  - `sharing.rs` (2153, 2220, 2386, 2742, 2845): "Sharing PIN", "Open shared library", "Read-only sharing over an unencrypted local connection. Use only a trusted network.", "Library server · Kiwix", "Kiwix · Library server", "Open the OPDS catalog", "Available archives", "Next page", "Read-only library sharing. Anyone with a link can read these items until you stop sharing."
  - `sharing_reader.rs:123`: "Refine your search to see more results."
- **`.counted("…")`** in `sharing.rs` (2483, 2762, 2788): "Serving {count} archives from this device. Read-only; no application controls are exposed." and "{count} articles". These are in `plural-sources.json` in neither form; it only has " · {count} articles".
- **A variable source:** `lib.rs:428` gives `title = "Look up in Kiwix"`, and line 469 calls `.title(strings.text(title, &[]))`.

Other variable sources happen to be present today: the `mobile_labels(...)` titles, the `windows.rs:89` tabbing array, the `sharing.rs:3133` facet titles and "Import archives".

All of these gaps exist at base too, so the shared-library web pages are English in every language. But the header claim of the new test ("Every literal the Rust core passes to its Localizer") is false, and it gives a green result on exactly this defect. A smaller gap: if `label()` keeps its name but loses all its arms, the `seen > 10` floor still passes, because `.text(` alone yields 51.

Smallest correct fix:
1. In files that define `let t = |`, also extract `(?<![\w.])t\(\s*"…"`.
2. Extract `.counted\(\s*"…"` and check it against `plural-sources.json` keys.
3. Either make the known variable-source call sites take literals, or list their literal arguments in the test (`report_launch_outcome` titles, `mobile_labels(`, the `windows.rs` sources array, the facet-title match).
4. Add the missing sources (the plural ones to `plural-sources.json`), then run `generate`.

### OWNER-R2 — The crash spec says every implementation runs steps 1 to 5 on frame text; Rust does not on its `at file:line` text
Severity: Should-fix
Fix under review: crash-spec "Notes for porters" (frame bullet), and the matching §3 sentence "Every platform must still run steps 1 to 5, in that order, on any frame text it keeps"
Evidence class: reproduced (Rust pipeline copied verbatim from `crash.rs` 262–287 and 419–1114 into `adv-owner/redact.rs`, then compiled and run)
Location: `src-tauri/src/crash.rs:275-281` (`sanitize_backtrace`)
Causal path: for the location line, Rust does `replace_urls(&location)` (step 3), then `file_name(...)`, then `redact_secret_values(&name)` (step 2). So steps 1, 4 and 5 never run there, and step 2 runs after step 3. Input `at C:\Users\Jane Doe\src\jane@example.org.rs:3:4` comes out as frame `1: bar at jane@example.org.rs:3:4`: the e-mail is kept. The symbol line does go through `redact_line` (steps 1–5).
- The porter note "Rust does this for its `at file:line` lines as described above" is false. §3's own Rust bullet is accurate, so the spec contradicts itself.
- The privacy impact is small, because these are compiler-supplied file names.
- The "every field cut keeps one scalar fewer than the limit and adds `…`" claim is correct in all three implementations: Rust `truncate_chars`, Swift `truncated` and Kotlin `truncate`. That includes Swift's extra 240-scalar `location` cut.

Smallest correct fix: reword both places. The rule should be that each platform runs steps 1–5 on the frame's symbol text, and that Rust's source location is reduced to its file name and then gets step 3 and step 2 only. Alternatively, run `redact_line` on the extracted file name in Rust, which needs a vector check.

### OWNER-R3 — STATE_MACHINES §1.2 step 2 is stale after the cutover-count and staged-copy rounds
Severity: Should-fix
Fix under review: §1.2 step 2 import-cutover wording
Evidence class: source-traced
Location: `service.rs:109-137` (`ImportCancel`, `ImportCutover`), 3316/3332, 2518-2541, 1876-1890, 2593 (`discard_cancelled_staged_copy`); `mobile_service.rs:386-388`, 1884-1886
What is wrong:
- **(a) When the fresh token arrives.** It is not installed "once the writer is held". `ImportCutover::drop` decrements `cutovers` and installs a fresh token only when the count reaches 0 and Stopping has not begun.
  - With overlapping cutovers (for example a background retirement waiting for its writer while a privacy save holds the writer), the token stays cancelled until the last cutover ends.
  - A cutover that ends early also replaces the token, with no writer held. One example is `self.nearby_retire().await?` returning an error between `begin` (3316) and the writer.
- **(b) Staged copies are deleted.** The step omits that a cancelled mobile import or intake discards its own staged copy under `archives/mobile-imports-v1/<uuid>/`, so a retry copies again. It also omits that a non-Quit cancellation of a manual import posts the "add it again" startup notice (`cancelled_imports`).
- **(c) Contradicting code comments.** These are not owner edits, but the doc has to be checked against them. `mobile_service.rs:1881-1883` says "nothing it staged is removed", and the `service.rs:2501-2503` doc comment says a failure "removes no copy". Both contradict `discard_cancelled_staged_copy`, which commit 862787c added after them.

Smallest correct fix: say the token is cancelled at `begin`, and a fresh one is installed when the last overlapping cutover ends (after its writer is held, or on early exit), never once Stopping has begun. Add that a cancelled mobile import or intake removes its own staged copy and the request stays pending. Fix the two stale code comments.

### OWNER-R4 — The RELIABILITY 2026-11-27 section cites a missing file and claims verification the final tree has not had yet
Severity: Should-fix
Fix under review: `docs/RELIABILITY.md`, "Review remediation — 2026-11-27"
Evidence class: source-traced (filesystem, LEDGER, scratch logs)
What is wrong:
- **(a) Missing file.** `docs/reviews/2026-11-27/PROJECT-REVIEW-REMEDIATION.md` does not exist. Only `working/` does, and `RESUME.md` says the record "will be" written.
- **(b) Windows and host checks are not current.** "cross-target type checks for Windows" and the host and browser test claims describe per-lane runs. The newest Windows clippy log is from 02:28. Startup round 5 (03:04), service round 4 (02:54) and buildhygiene changed code afterwards, and `RESUME.md` step 5 still lists the full suites and Windows/Android clippy as to-do.
- **(c) Uncompiled code is not mentioned.** The section omits that iOS- and macOS-only Rust was never compiled: no Apple Rust target is installed, and this includes the iOS `mobile_media_released` cutover and the macOS map power guard. It also omits that Swift outside `CrashRedaction.swift` was syntax-checked only (`swiftc -parse`).

What is correct: the bold "None of them has been run on a phone, a Mac, Windows or an iPhone simulator" is accurate. No sentence claims a device, Mac, Windows or simulator run.

Smallest correct fix: write the record (or cite `working/LEDGER.md` until then). Run the integrated Windows and Android clippy plus the host suites before landing, or say the checks were per lane. Add one sentence on uncompiled Apple Rust and syntax-only Swift.

### OWNER-R5 — The §4 `_`-suffix residual is narrower than the actual gap
Severity: Nit
Evidence class: reproduced (scratch Rust harness)
Location: crash spec §4, the bullet on forms the left and right sides reject
Detail: the rule (3, first bullet) rejects any keyword followed by `_`, not only after a camelCase hump. All of these come out unchanged: `secret_value=abc`, `token_value=hunter2`, `access_token_value=hunter2`, `client_secret_value=abc`, `api_key_id=abc`, `password_hash=abc`. The vector "password_hint=x stay" pins this as intended.
Fix: "or any `_` suffix after a keyword (`accessToken_value=…`, `secret_value=…`, `token_value=…`)".

### OWNER-R6 — The STATE_MACHINES §5.1 reason for `RetainedMapJob.eta_seconds` is wrong
Severity: Nit
Evidence class: source-traced
Location: `maps/partials.rs:86-105` (`#[derive(Serialize)]` only); `maps/manager.rs:900`
Detail:
- `RetainedMapJob` is never deserialized. `retained_jobs()` builds it fresh for the shell, so it cannot be "kept so older saved jobs still load".
- Old journals load because `MapJob` has no `deny_unknown_fields`, so serde ignores an old `etaSeconds` (tested at `maps_tests.rs:857`).
- "Always `None`" is true.

Fix: "`MapJob` has no ETA; older journals carrying `etaSeconds` still load because the field is ignored. `RetainedMapJob.eta_seconds` is always `None` (serialized as `etaSeconds: null` in the download-row shape)."

### OWNER-R7 — Crash-spec header paths
Severity: Nit
Detail: the Rust path is repo-relative (`src-tauri/src/crash.rs`), but Swift and Kotlin are given as `plugins/tauri-plugin-kiwix-mobile/...`, which does not exist from the repo root. The correct path is `src-tauri/plugins/...`, which is what the Swift and Kotlin file headers themselves use.

### OWNER-R8 — Porter note on `\'`
Severity: Nit
Detail: "`\'` is accepted before a key" should say *after* a key, as the escaped closing quote of a quoted key (rule 4.1). The behaviour itself is confirmed:
- `\'token\'=\'abc def\' x` becomes `\'token\'=[redacted]`, because the token form is extended by `path_end`.
- `\'token\': hunter2 x` becomes `\'token\': [redacted] x`.

### OWNER-R9 — PLATFORM-GAPS wording
Severity: Nit
Location: `src/shell-dismiss.ts:38-46`, `src/App.tsx:3008-3012`, `power_activity.rs`
Detail:
- **(a) Back order.** The code is dialog → tab sheet → popover (shell pickers, menus and suggestions, plus reader overlays such as find bar, preview and overlaid panel) → search → reader history → library. The doc lists "dialog, picker, tab sheet or search".
- **(b) Contradicting App.tsx comment.** The comment says Back at the library "backgrounds the task". That contradicts the doc's correct "first walks the WebView's own page history". The vendored `AppPlugin.kt` calls `view.goBack()` when no listener is registered, and reader iframe navigations (`node.src = …`, in-article links) add joint-history entries. The "several presses" part is an inference, labelled as not checked on a device.
- **(c) Item 5.** "On every platform that has one" is ambiguous. iOS LocalSwarm takes the guard, but the iOS platform module is a no-op.

### Per-claim assessment
- §5.1 MapJob has no ETA, `eta_seconds` always `None`: correct, but the reason given is wrong (R6).
- §1.1 Stopping cancels imports, and the token is never replaced after Stopping: correct.
  - `shutdown()` sets `stopping` and then calls `cancel_imports()`.
  - `Drop` checks `stopping` under the same mutex, so a racing `Drop` that installs a token is followed by its cancellation.
  - The iOS `mobile_begin_shutdown` sets `stopping` just before `shutdown()` cancels.
  - Cancelled monitored imports are skipped by `failures.record`.
  - Android never enters Stopping. That is consistent, since §1.1 is the terminal path.
- §1.2 step 2: stale (R3).
- PLATFORM-GAPS Print row and item 8: correct.
  - Android goes through `exportArticle("pdf")`, then `mobile_save_pdf`, then `NativeArticlePdf.kt` `PrintManager.print`.
  - The `!isMobile` row is hidden.
  - On iOS, Cmd+P (`HardwareShortcuts.swift:45` "p" → "print") shows the notice, but only with an HTML article open and not in read-only mode.
- Item 5: correct apart from the wording in R9(c). The guard is acquired only at `localswarm/network.rs:1100`, `localswarm/mod.rs:1118` and `maps/manager.rs:1347` (macOS only).
- Android Back paragraph: substantively correct (R9).
- RELIABILITY: R4.
- Crash spec:
  - Empty-quoted residual: confirmed. `cookie="" a=b`, `password=""`, `password=Some("")` and `password=''` are all unchanged.
  - Struct, array and map residual: confirmed. `token=Some(Token { value: "abc def" })` becomes `token=[redacted] { value: "abc def" })`, and `token=Ok(["a b"])` becomes `token=[redacted] b"])`.
  - Field-cut note: correct.
  - classify and port notes: correct. `http://localhost:abc/x` becomes `[reader-url]`; `:080` and `:0443` are dropped; `:000` is kept.
  - Surrogate note: not re-verified here.
  - Frame note: wrong (R2).
- `localization.mjs` `generate` and `inventory` both pick the new file up through the `^extra-en-.*\.json$` glob (lines 255-258), and `inventory()` feeds `generate` at line 557. `error-inventory.mjs` `localeSource` globs it too. The test runs in CI through `node --test scripts/test-*.mjs`. Stale patterns fail loudly (`seen > 10`, with 51 found; a missing `label()` asserts), except the arm-loss case in R1.
- Not covered by the test, and deliberately English with documented reasons: the pre-runtime dialogs `STARTUP_FAILURE_TITLE` and the busy-profile title and text in `instance.rs`, the boot-guard panel action labels in `mobile.rs:759`, and the "Kiwix — Camera" window title. I did not count these as findings.

### Implementer's doubts
No implementer report exists. The LEDGER asked for these to be checked: the §1.1/§1.2 wording against the cutover count and staged-copy removal (stale, see R3), and the spec header, §4 and porter notes (see R2, R5, R7, R8).

### What I ran
- `node --test scripts/test-rust-localized-strings.mjs`: pass 1, fail 0.
- `node scripts/localization.mjs inventory` (read-only): fails with many "Uncollected UI key" entries from every lane, including the new extras. That is expected while `generate` is pending, and it must be run before landing.
- Scratch rustc build of the verbatim `crash.rs` redaction pipeline (`adv-owner/redact.rs`, `adv-owner/redact2.rs`): the outputs quoted above.
- Scratch extractor `adv-owner/extract.mjs`: the R1 list.

### What I could not verify
- Any device, Mac, Windows or simulator behaviour, including the Android Back "several presses" inference and iOS Cmd+P.
- iOS and macOS Rust compilation.
- A current Windows or Android clippy run on the final tree (not run, and not in my allowed commands for Windows).
- The Swift and Kotlin frame paths beyond reading the code.
- `generate` output (deliberately not run).
