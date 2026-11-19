# Required change and deployment review

Every future repository change requires adversarial review before it is called complete, installed on a user device, published, or released. This includes application code, tests, dependencies, build/CI configuration, generated resources, and documentation claims. Review cohesive batches; do not substitute an author self-check for independent review.

## Scope and independence

- During ongoing work, review each cohesive change with its affected callers and tests as soon as it can be exercised. For a change that spans several milestones, get independent review at each testable milestone before building further work on it. Carry findings and open questions into the next review.
- Record the reviewed base/snapshot and changed paths, including staged, unstaged, and new files. A Git diff alone omits untracked additions.
- Use an independent reviewer with the complete affected producer/consumer contract. Split genuinely independent subsystems when the read set is large; keep one integration owner.
- Read the current code and relevant existing patterns. Historical findings or a green suite do not establish that a current change is safe.
- Give reviewers the user-visible behavior, boundaries, known failures, and exercised verification. Never hide unresolved observations.

## Adversarial checks

- Exercise realistic state transitions and failure boundaries: cancellation, retries, process death, suspension/relaunch, privacy/read-only cutovers, revoked permissions, missing storage, low space, and concurrent work where affected.
- For filesystem changes, check exact ownership, no-follow traversal, source/replacement identity, commit durability, rollback, and preservation of unrelated files and reading state.
- For reader, sharing, and native handoffs, check installation/origin authority, stale callbacks, credential/route lifetime, frame identity, cancellation and cleanup. Metadata UUIDs, filenames, matching bytes, and UI state alone are not authority.
- Check the actual nontechnical interaction and accessible surface. Do not replace an actionable failure with silent success or merely hide an underlying defect.
- Keep compatibility, performance, localization provenance, packaging and documentation claims within observed evidence.

## Findings and closure

- Report evidence-backed findings with severity, exact symbol/path, consumer-visible trigger, causal path, and the smallest correct fix. Separate reproduced behavior, source-traced risks, inferences and unavailable hardware checks.
- Fix applicable findings before deployment/completion. Retain a meaningful behavioral regression when practical, and exercise the changed runtime path; tests or compilation alone are not runtime proof.
- Have the independent reviewer check each fix and any subsequent affected changes. A new edit invalidates the earlier review for its affected paths/contracts.
- Record review coverage, findings/dispositions, verification actually exercised, and residual blockers. A zero-findings review is not release or parity certification.
- Preserve user files, libraries, grants and settings. Obtain approval before disruptive device/volume operations. Withdraw test publishers and remove owned temporary device data and processes.

This is a mandatory contributor/assistant workflow gate, not a claim that branch protection or CI automatically performs independent review. Do not claim remote enforcement unless it has actually been configured and verified.

## Physical iPhone testing windows

**Standing authorization — dedicated sharing-test iPhone (owner confirmed 2026-11-07):** the iPhone 13 Pro whose UDID the owner supplies locally in `KIWIX_IPHONE_UDID` (never committed) is entirely dedicated to testing. If that variable is unset or does not exactly match the target UDID, no standing authorization applies. Assume yes for routine, non-destructive test installs, launches, Bluetooth/Local Network consent, Wi-Fi Join, log collection, and repeated prepared test windows. Do not wait for per-run approval or ask the owner to tap Join. Keep it signed out of a personal Apple Account. This does not authorize wiping it, deleting unrelated data, or unrelated account/security changes.

Prepare the application, signing, test runner, dependencies and independent reviews before starting a device window. Use `npm run iphone` rather than rebuilding or deploying through a hand-operated sequence. `--prepare-only` must not contact the phone.

Before requesting another physical comparison window, exercise the same real-Wikipedia/search/StreetZIM UI workflow in an explicitly owned iPhone simulator and fix observed runner defects there. Simulator UI proof does not certify physical-device profiling transport or memory. The next physical window must start with a bounded sampler-readiness check before installation or UI work; a transport failure must release the phone, not consume a reader-testing window.

For devices without standing authorization, ask for one explicit hands-off window only when the prepared artifacts are ready. The installation/test window is at most ten minutes, including cleanup. Do not compile, install host tools, fix selectors, debug transport or perform reviews while holding that window. Stop at the first unexpected state, complete bounded cleanup, and investigate off-phone.

Do not announce that the phone is released unless the matching run confirms that its owned automation has stopped. Preserve existing archives and settings; do not enable VoiceOver, speech or media playback for these tests. See `docs/PACKAGING.md` for the command and evidence limits.

Immediately after a device window ends, send a standalone user-facing **DEVICE RELEASED** message when cleanup is confirmed; do not rely on tool logs or bury the handoff in another progress report. Say what worked, what failed (including any bug or unexpected state), and what was not measured. Distinguish “done with the device” from “all testing passed.” If cleanup is unconfirmed, say that explicitly instead of announcing release. For devices without standing authorization, ask again before any further access.

