# Implementer brief (read fully before starting)

Repository: `/home/user/kiwix-tauri` (branch `claude/stoic-dirac-rs0ol3`, review base
`d98cac2`). "Kiwix Workbench": Tauri 2 app — Rust core in `src-tauri/src`, React/TS
frontend in `src`, Kotlin + Swift mobile plugin in
`src-tauri/plugins/tauri-plugin-kiwix-mobile`, Objective-C++ in `src-tauri/native`,
MCP relay in `kiwix-mcp`. It downloads, verifies, reads, searches and shares offline ZIM
archives on macOS, Windows, Linux, iOS and Android.

Independent reviewers produced findings (full text in
`/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/reports/<AREA>.md`).
You are the **implementer for one lane**: a named subset of those findings and the
files they live in. A separate, independent adversarial reviewer will examine your
changes afterwards, so leave the tree in a state you can defend line by line.

## How to work each finding

1. **Verify first.** Read the cited code and its callers/consumers in the current tree.
   The reviewers were careful but not infallible, and nothing was executed. If a finding
   is wrong, already handled elsewhere, or the proposed fix would break a deliberate
   defence, DO NOT change code for it: report "rejected" with the evidence.
2. **Smallest correct fix.** Fix the defect at its cause. Preserve existing contracts,
   lock ordering, error wording conventions and code style of the file. No drive-by
   refactors, renames, reformatting or dependency additions. Do not weaken a security,
   privacy, integrity or ownership check to make something work; if the only fix you can
   see does that, stop and report instead. When the reviewer's suggested fix is not the
   best one, choose better and say why.
3. **Never replace an actionable failure with silent success.** A fix that makes an
   error disappear without the underlying operation succeeding is a defect.
4. **Regression test** for every behavioural fix where practical, in the existing test
   module/harness for that file, asserting the user-visible behaviour (not the
   implementation). The test must fail without your fix — say for each test whether you
   actually observed that (e.g. by temporarily reverting the fix) or are inferring it.
5. **Exercise it.** Run the tests you added and the existing tests of every module you
   touched. Compilation alone is not proof. Paste the real result lines in your report.
   If something cannot be exercised here (device-only, other OS), say so plainly.

## Shared working tree — strict protocol

Several implementers edit this same working tree at the same time, each in different
files. There is no isolation, so:

- Edit ONLY the files your lane lists as owned, plus the specific small edits in shared
  files that your lane explicitly allows. If a correct fix needs a change in a file you
  do not own, do not make it: describe the exact change needed in your report
  ("handoff") and make your side compile and behave safely without it.
- NEVER run any state-changing git command (`commit`, `add`, `stash`, `checkout`,
  `restore`, `reset`, `clean`, `merge`, `rebase`, `worktree`). Read-only git (`diff`,
  `status`, `log`, `show`, `blame`) is fine. Never revert or "fix" another implementer's
  in-progress edits. The integration owner commits work-in-progress checkpoints of the
  whole tree from time to time (this does not touch your working files), so plain
  `git diff` may show only part of your work: to see your full change always diff
  against the review base, `git diff d98cac2 -- <your files>`.
- The tree may transiently fail to compile because another implementer is mid-edit. If
  `cargo`/`tsc` reports errors located ONLY in files you do not own, wait ~90 seconds
  and retry (up to 10 times). Do not touch those files. If it persists, report it.
- Keep your own intermediate states short: prefer making an edit and its dependent
  call-site edits back to back so the crate does not sit broken for long.
- Scratch files go only under
  `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/<lane>/`.

## Build and test environment (Linux x86_64 container, 4 cores, shared)

- Always: `export SHERPA_ONNX_ARCHIVE_DIR=/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/sherpa`
  (the speech library's build script cannot download through the proxy otherwise).
- Rust, host (Linux): 
  - type-check: `cargo check --locked --manifest-path src-tauri/Cargo.toml --lib --tests`
  - tests: `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib <module_or_test_filter>`
    (the crate is one 140k-line library; each test build takes minutes and `cargo`
    serialises on a build lock across all implementers — batch your edits and compile
    sparingly; run targeted filters, NOT the whole suite; the integration owner runs
    the full suite).
  - lint (must be clean for your code, run once near the end):
    `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --lib --tests --features custom-protocol -- -D warnings`
  - Do not pass `--release`. Do not use a different `CARGO_TARGET_DIR`.
- Rust, Windows cfg type-check (only if you touched `cfg(windows)` code):
  `KIWIX_TYPECHECK_ONLY=1 CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ cargo clippy --locked --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-gnu --lib --tests --features custom-protocol -- -D warnings`
- Rust, Android cfg type-check (use when you touched `cfg(mobile)` / `cfg(target_os = "android")` code; it has its
  own build directory so it does not block others, takes ~3 minutes, run it once near the end):
  `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android-clippy.sh`
  (clippy `-D warnings` for `aarch64-linux-android --lib`; clean at baseline). This type-checks
  `cfg(mobile)` and Android code; it does not run anything.
- There is no macOS/iOS Rust target here: `cfg(target_os = "macos"|"ios")` code CANNOT be
  compiled in this container (and `cfg(mobile)` code is only type-checked for Android by the command above). Be extra conservative
  there: mirror existing compiled patterns exactly, keep edits minimal, and flag every
  such edit in your report as "not compiled here".
- Baseline facts: at `d98cac2` the host suite is 1329 passed / 5 failed / 16 ignored;
  the 5 failures are all `torrent::tests::*` failing with "error starting listeners"
  because this container's kernel has no IPv6 (a real defect, being fixed in the
  transfers lane). Linux and Windows-gnu clippy are clean at baseline.
- Frontend: `npx --no-install tsc --noEmit` (type-check), `node --test scripts/test-<name>.mjs`
  (targeted), `node --test scripts/test-*.mjs` (all, ~7 s; 399 pass / 3 skip at baseline).
  `npm run build` rewrites `dist/`, which Rust embeds — only the integration owner runs
  it. A headless Chromium is at `/opt/pw-browsers/chromium` (find the executable under
  it); `KIWIX_TEST_CHROME=<path> node --test scripts/test-frontend-surfaces.mjs` runs the
  real-browser surface tests (they need a prior `npm run build`; ask via your report if
  you need one rather than running it yourself, unless your lane says otherwise).
- Swift: syntax only — `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/swift/swift-6.0.3-RELEASE-ubuntu24.04/usr/bin/swiftc -parse <files>`.
- `node scripts/check-unsafe-census.mjs` must keep passing. Do not add `unsafe` unless
  the lane explicitly calls for it; if you must, follow the existing `// SAFETY:` comment
  convention and report it (the baseline file and docs need updating by the owner).
- `docs/ERRORS.md` and the files under `src/locales/` are GENERATED. Do not hand-edit
  them; the integration owner regenerates them at the end. If you add or change a
  user-visible frontend string (anything passed to `t(...)`), list it in your report.
- Rust user-facing error strings are plain English sentences that say what happened and
  what the person can do; follow the tone of neighbouring messages. Never include raw
  paths of user documents in crash/diagnostic output.

## Report (your final message is all that is returned)

For each finding in your lane:

```
### <ID> — <title>
Disposition: fixed | fixed differently (why) | rejected (evidence) | partial (what remains) | handoff (exact change needed elsewhere)
Verification of the finding: <what you read/ran that confirmed or refuted it>
Change: <file:symbol — what changed and why it is correct; contracts touched>
Tests: <test names; observed result line; whether you observed it fail without the fix>
Not exercised: <what could not be run here and why>
```

Then:
- **Commands run** with their final result lines (cargo test filters and counts, clippy,
  tsc, node tests).
- **Files changed** (exact list) and any **new user-visible strings**.
- **Handoffs / contract changes** other lanes must know (new JSON fields, commands,
  changed error text matched elsewhere).
- **Doubts**: anything you are not sure is right. Hiding a doubt is worse than stating it.
