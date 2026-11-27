# Adversarial change-review brief (read fully before starting)

Repository: `/home/user/kiwix-tauri` (Tauri 2 app "Kiwix Workbench": Rust core in
`src-tauri/src`, React/TS frontend in `src`, Kotlin + Swift mobile plugin, Objective-C++
bridges). Review base commit: `d98cac2`. Implementers have been fixing findings from a
deep review directly in this working tree; the tree is shared and OTHER lanes are still
being edited while you read, and the integration owner commits work-in-progress
checkpoints, so always look at changes as `git diff d98cac2 -- <paths>` (never plain
`git diff`).

You are the **independent adversarial reviewer** of ONE lane's changes. You did not write
them. The project's standing rule (`AGENTS.md`, read its "Adversarial checks" and
"Findings and closure" sections) is that no change is complete until someone other than
its author has tried hard to break it. Your job is to find what is wrong with these
changes — not to confirm they look reasonable. Assume the implementer's report is
optimistic: check its claims against the code and against what you can run.

## What you are given
- The original findings the lane was meant to fix (`.../scratchpad/reports/<AREA>.md`).
- The implementer's own report (`.../scratchpad/impl/<lane>.md`), including its stated
  doubts and handoffs.
- The changed paths.

## What to do
1. Read the complete diff for the lane's paths, then read the surrounding code and every
   caller/consumer of anything whose behaviour or contract changed (other languages
   included: Rust ⇄ TypeScript ⇄ Kotlin ⇄ Swift JSON field names, command names, error
   strings matched elsewhere, persisted formats).
2. For each fix ask, and answer from the code:
   - Does it actually fix the reported defect at its cause, on every platform the finding
     named? Or only the reviewer's example?
   - What did it break? New panics, deadlocks or lock-order changes, awaits while holding a
     lock, unbounded waits/loops/allocations, leaked files/threads/listeners, races across
     cancellation / retry / pause-resume / process death and relaunch / suspension /
     privacy or read-only cutover / permission revocation / low disk / missing volume.
   - Did it weaken an integrity, ownership, privacy, origin or authority check, or turn an
     actionable failure into silent success? Did it delete or overwrite anything it has
     not proven it owns?
   - Persisted data: does a profile/journal written by the previous build still load, and
     what does the previous build do with data written by the new one?
   - Is the user-visible result right — the exact message, where it appears, whether a
     keyboard and a screen reader reach it, whether the person can act on it?
   - Does each new test assert the user-visible behaviour, and would it fail without the
     fix? A test that cannot fail, or that tests the mock, is a finding.
   - Code that could not be compiled or run here (macOS/iOS/Swift; anything device-only):
     read it with extra suspicion for typos, wrong API names/signatures, missing
     availability or SDK-level guards, Send/Sync and threading mistakes. Say clearly what
     remains unverified.
3. Run what you can (see below) to confirm or refute — do not rely on the implementer's
   pasted results.
4. Also flag scope creep: edits outside the lane's stated ownership, drive-by refactors,
   or behaviour changes the findings did not call for.

## Rules
- You are READ-ONLY with respect to the repository: do not edit, create or delete files in
  it; no state-changing git commands. Scratch files only under
  `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-<lane>/`.
  (If you want to prove a test cannot fail, reason from the code, or copy the relevant
  function into a scratch file — do not mutate the tree.)
- You MAY run: targeted Rust tests
  (`export SHERPA_ONNX_ARCHIVE_DIR=/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/sherpa; cargo test --locked --manifest-path src-tauri/Cargo.toml --lib <filter>`
  — the crate is one large library, each build takes minutes and cargo serialises on a
  lock shared with implementers, so run few, targeted invocations; if the build fails with
  errors ONLY in files outside your lane, another lane is mid-edit: wait ~90 s and retry,
  and do not report those errors as findings); `npx --no-install tsc --noEmit`;
  `node --test scripts/test-<name>.mjs`; the real-browser surface tests
  (`KIWIX_TEST_CHROME=/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/chrome-nosandbox.sh node --test scripts/test-frontend-surfaces.mjs`);
  the Android Rust type-check
  (`/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android-clippy.sh`);
  Kotlin compile/unit tests per `.../scratchpad/ANDROID-ENV.md`; Swift syntax check
  (`.../scratchpad/swift/swift-6.0.3-RELEASE-ubuntu24.04/usr/bin/swiftc -parse <files>`).
  Do NOT run `npm run build`, `npm install`, or cargo with `--release`.
- Environment facts: Linux x86_64 container, no IPv6 in the kernel, no macOS/iOS/Windows
  runtime, no devices. At base, the host Rust suite was 1329 passed / 5 failed (the 5 are
  torrent tests needing an IPv6 listener; a transfers-lane fix targets them).
- Report only what you have traced in the actual code. A wrong finding makes someone break
  working code; if you are unsure, say it is a doubt and why. Equally, do not soften a real
  problem. "No findings" is an acceptable result only if you can say what you checked.

## Output (your final message is all that is returned)

```
## Verdict for lane <lane>: <BLOCKING findings exist | only non-blocking findings | no findings>

### <LANE>-R<n> — <title>
Severity: Blocking | Should-fix | Nit
Fix under review: <finding ID(s)>
Evidence class: reproduced | source-traced | inference
Location: <path>:<line> (<symbol>)
Trigger: <concrete sequence>
Causal path: <through the changed code, quoting decisive lines>
Smallest correct fix: <specific>
```

"Blocking" = would lose or corrupt data, hang, crash, weaken a safety/authority check,
mislead the user about success, not compile on a platform that ships, or fails to fix the
High/Medium finding it claims to fix. Then:
- **Per-fix assessment**: one line per original finding ID — "fixes it" / "does not fix
  it because …" / "fixes it but …".
- **Implementer's doubts**: for each doubt the implementer listed, your judgement
  (real problem to fix now / acceptable and why / needs owner decision).
- **What I ran** (commands and result lines) and **what I could not verify**.
