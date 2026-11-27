# Deep review brief (read fully before starting)

Repository: `/home/user/kiwix-tauri`, branch `claude/stoic-dirac-rs0ol3`, base commit
`d98cac2` (clean tree). It is "Kiwix Workbench": a Tauri 2 application (Rust core in
`src-tauri/src`, React/TypeScript frontend in `src`, a Kotlin + Swift mobile plugin in
`src-tauri/plugins/tauri-plugin-kiwix-mobile`, Objective-C++ bridges in
`src-tauri/native`, a standalone MCP relay in `kiwix-mcp`, and a Windows ACL crate in
`win-security`) that downloads, verifies, reads, searches and shares offline ZIM archives
on macOS, Windows, Linux, iOS and Android.

## What the owner asked for

A deep review of **maintainability, reliability and usability across all platforms**,
with every issue found then fixed. You are one of several independent reviewers, each
with a subsystem. You are **read-only**: find and prove defects; do not fix them.

## Ground rules

- Do NOT modify, create or delete any file inside the repository. Do not run `git`
  commands that change state. Scratch files go only in
  `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/<your-area>/`.
- Do NOT run `cargo`, `gradle`, `npm ci`/`npm install` or `npm run build` (the machine
  has 4 cores and other work is using the toolchain and `dist/`). You MAY run individual
  existing Node tests (`node --test scripts/test-<name>.mjs`), small throwaway Node/Python
  snippets from your scratch directory, and read-only shell commands (`grep`, `rg`, `wc`,
  `git log`, `git show`, `git blame`).
- Read the current code, including the callers and consumers of anything you flag. A
  finding you have not traced through the actual code path is not a finding. Never
  report something on the strength of a name, a comment or a pattern match alone.
- This code base has been reviewed repeatedly (see `docs/reviews/2026-11-17/`,
  `docs/reviews/2026-11-23/`, `docs/reviews/2026-11-25/`, and `docs/STATE_MACHINES.md`,
  `docs/PLATFORM-GAPS.md`, `docs/RELIABILITY.md`). Skim the two newest review records
  first. Many defences that look odd are deliberate. Do not re-report a residual limit
  those records already document (updater buffers the artifact in memory, Windows
  ChangeTime is not proof, no physical-device runs, etc.) unless you have a concrete
  source fix that is feasible now.
- Hardware-only issues (need a physical phone, signed bundle, real Windows box) are in
  scope only when the defect is provable from source and fixable in source.
- Prefer depth over breadth: a small number of real, well-traced defects is far more
  valuable than a long list of maybes. A wrong finding costs more than a missed one,
  because someone will change working code on the strength of it.

## What to look for

- **Reliability:** data loss or corruption, lost or stuck work, deadlocks, lock-order
  inversions, unbounded waits/reads/allocations, leaked threads/timers/listeners/file
  handles, races across cancellation / retry / suspension / relaunch / privacy cutover /
  permission revocation / low disk / missing volume, stale callbacks acting on a
  successor's state, panics on untrusted input, errors swallowed into silent success.
- **Usability:** an action that fails with no visible or actionable message; a state the
  user cannot get out of; missing loading/empty/error states; controls unreachable by
  keyboard or screen reader, missing accessible names, broken focus return; touch
  targets or layouts that break on small screens, large text, RTL or dark/high-contrast;
  platform conventions violated (menus, shortcuts, back gesture, safe areas);
  untranslated or internal identifiers shown to people; misleading progress.
- **Maintainability:** logic duplicated in two places that has already diverged; dead
  code or dead configuration that misleads; tests that exist but are not run by CI or
  that cannot fail; documentation or comments that contradict the code; cfg-gated code
  for one platform that cannot compile or is never type-checked; brittle string/JSON
  contracts between Rust ⇄ TypeScript ⇄ Kotlin ⇄ Swift that have drifted. Do NOT report
  "this file is too long" or style preferences — only concrete, low-risk, fixable items.

## Output format (your final message is the only thing returned)

Return at most **12** findings, ranked most severe first. For each:

```
### <AREA>-<n> — <one-line title>
Severity: High | Medium | Low        Category: reliability | usability | maintainability
Platforms: <which>
Evidence class: reproduced | source-traced | inference
Location: <path>:<line> (<symbol>)  [+ other key locations]
Trigger: <the concrete user/system sequence that hits it>
Causal path: <step by step through the real code, quoting the decisive lines>
Smallest correct fix: <specific: which function, what change; note any contract it touches>
Regression test: <where a test could live and what it would assert>
```

Then two short lists: **Covered** (files/flows you actually read end to end) and
**Not covered** (in-scope files you did not get to). Be honest in both. If you found
nothing meeting the bar in some part of your scope, say so plainly rather than padding.
