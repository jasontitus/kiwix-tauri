# Resuming the 2026-11-27 review remediation

This folder is working state, not the final review record. It exists so the work can be
picked up if the cloud session that ran it is lost. The final record will be
`docs/reviews/2026-11-27/PROJECT-REVIEW-REMEDIATION.md`; this `working/` folder is to be
removed or condensed into it before the work is called complete.

## What was asked

A deep review of maintainability, reliability and usability across all platforms, with
every issue found fixed and every change adversarially reviewed (see `AGENTS.md`).

## State of the work

- **Review base:** `d98cac2`. Every commit after it on `claude/stoic-dirac-rs0ol3` titled
  `wip: checkpoint …` is an automatic, unreviewed snapshot of a shared working tree that
  several implementers were editing at once. A checkpoint may not build.
- **Findings:** `findings/<AREA>.md`, eleven independent reviews (APPLE, SHELL, SERVE, ACQ,
  LIB, CORE, FEAT, BUILD, NET, READER, ANDROID).
- **Plan, lanes and status:** `LEDGER.md`. Each lane owns a set of files and a list of finding
  IDs. A lane's work so far is `git diff d98cac2 -- <its owned files>`.
- **Lane reports:** `implementation/<lane>.md`, written when a lane finished. A lane with no
  report here was still in progress when the session ended. Its partial edits are in the
  newest checkpoint and must be re-verified from scratch, not trusted.
- **Adversarial reviews:** `adversarial/<lane>.md`, when written.
- **Briefs used for each role:** `REVIEW-BRIEF.md`, `IMPLEMENT-BRIEF.md`,
  `ADVERSARIAL-BRIEF.md`. `ANDROID-ENV.md` records how Kotlin was compiled and tested in the
  container. The SDK paths in it were in temporary scratch space and will be gone.

## To resume in a new session

1. Check out `claude/stoic-dirac-rs0ol3`. Read `LEDGER.md`, then this file.
2. For each lane with no `implementation/<lane>.md`: diff its owned files against `d98cac2`,
   compare with its findings, and either finish it or revert the partial edits to `d98cac2`.
3. Every lane, finished or not, still needs its adversarial review, its fixes, and a
   re-review before anything is called done.
4. Rebuild the environment the briefs assume:
   - `SHERPA_ONNX_ARCHIVE_DIR` holding the sherpa-onnx v2.0.1 archives, because its build
     script cannot download through the proxy;
   - GTK/WebKit dev packages and the MinGW cross compiler, for the Linux and Windows checks;
   - the Android SDK and NDK 28.2, for Kotlin and Android Rust checks;
   - a Swift toolchain, for `swiftc -parse`.
5. Finish: full suites (Rust lib, Node, Python, Kotlin unit tests, Linux, Windows and Android
   clippy, real-browser surface tests); regenerate `docs/ERRORS.md` and the localization
   inventory; write the final record; replace the `wip:` checkpoints with reviewed commits.
