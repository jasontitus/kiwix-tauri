# Bug triage verification — 2026-11-12

Base snapshot for the triage: `3d289b1`. Verification snapshot: `0f6c160` plus this change. The
triage document is
`docs/reviews/2026-11-12/PI_REVIEW_BUGS_DEEPSEEK_V4_1_FLASH_1017_20261112.md` (879 findings: 5 high,
113 medium, 761 low).

## Method

Eight independent reviewers verified every finding against the current code, not the pre-fix
snapshot. Each finding was classified as `real` (still present), `fixed` (already addressed by
`0f6c160` or this change), `refuted` (premise false/unreachable), or `uncertain` (needs a runtime
check). Reviewers were instructed not to edit code or run builds. A second parallel wave implemented
the verified medium findings in disjoint file groups.

## Counts at verification

| Batch | Real | Fixed | Refuted | Uncertain |
|---|---:|---:|---:|---:|
| High + medium (118) | 53 | 12 | 46 | 7 |
| Android low (42) | 21 | 2 | 19 | 0 |
| iOS low (29) | 8 | 4 | 16 | 1 |
| Rust low A (197) | 83 | 7 | 99 | 8 |
| Rust low B (153) | 88 | 9 | 51 | 5 |
| Frontend low (104) | 33 | 7 | 60 | 4 |
| Scripts low (184) | 86 | 1 | 87 | 10 |
| Native/build low (47 + 5 unassigned) | 8 | 0 | 44 | 0 |
| **Total** | **380** | **42** | **422** | **35** |

The 5 high findings were all real and are fixed:

- `DeviceZimScan.kt` — `Cursor#getExtras` is API 29+; guarded for minSdk 24.
- `imports.rs` — verifier `unwrap()` after `unretire_acquisition`; replaced with a guarded lookup.
- `lib.rs` — forwarded `--help`/`--version` exited the running instance; forwarded paths now reject
  them before parsing.
- `store.rs` — skipped records were persisted out of `state.json`; warnings now block normalization.
- `torrent/manager.rs` — `interrupt_network_workers` left a stopped `OnceCell`; it now swaps in a
  fresh session before stopping the old one.

## Medium findings

All 53 real medium findings were addressed:

- **47 fixed** across catalog/export/parser bounds, `open_paths`, invalid-journal quarantine,
  library-marker and temporary-file symlinks, map-prefix 32-bit bounds, bookmark URL parsing, PDF
  read-only admission and post-cancel drain, saved-read-only reset, legacy publication recovery,
  virtual-interface classification, LocalSwarm discovery/ACK/pipeline, nearby stop deadlines,
  notification foreground deadline, catalog/map admission, PIN lockout, spelling filtering, Android
  startup window/exit, mobile policy acknowledgement, mobile rebase robustness, speech payload
  validation, torrent retirement/journal/worker/gate ordering, Android completion journal bounds,
  document-copy budget, operator-tooling safety, and the Library command/share button behavior.
- **6 partial** with documented residuals:
  - **57 LocalSwarm native completion** — the fail-closed change was reverted because the Apple
    server never sends a stream-complete signal; the prior native trailing-read tolerance remains.
    A real fix needs an explicit native finish/close-reason probe.
  - **61/63 LocalSwarm PIN exchange** — both peers now use a nonce-bound HMAC proof and the client
    never sends a deterministic PIN token; the seeder still accepts the legacy token. A new client
    cannot authenticate to an older build or the reference Swift/Go engines, and an active on-link
    peer can still offline-guess a low-entropy PIN from one proof. A versioned negotiation and a
    PAKE are required before cross-version release.
  - **67 maps `clear_cache`** — cache clearing now reserves under the mutex and drops it for
    filesystem work. `delete_download` still holds the lock across reclamation/persistence; dropping
    it needs a reservation honoured by every state-taking command.
  - **83/84 mobile speech** — all fallible payload construction now precedes `begin_start`; the
    secondary claim that a failure leaves a `Starting` token was refuted because the only caller
    already retires owned starts on any error.
- **Residual observations from fix review:** the strict catalog client's DNS fence can reject a
  system proxy whose own host resolves privately; the Android early-window fix can briefly show the
  page before the Service is managed on very slow startup (manual Retry remains); and the torrent
  journal-gate change can lose one tick of transient progress/status fields. These are documented
  for follow-up rather than silent claims of completion.

## Low-severity result

327 low findings were real at verification, 30 were already fixed, 376 were refuted and 28 were
uncertain. They are concentrated in operator tooling (`scripts/`, `docs/windows-agent-tools/`),
defensive hardening and test-only paths. They were not individually fixed in this change; the
reviewer dispositions are the actionable backlog.

## Verification

- `scripts/dev.sh check` — pass.
- `scripts/dev.sh test` — 1103 passed, 0 failed, 19 ignored.
- `cargo clippy --lib -- -D warnings` — pass (only the pre-existing vendor `wry` cfg warning).
- `npm run build` — pass.
- `node --test scripts/test-*.mjs` on Node 22.23.2 — 315 passed, 0 failed.
- No physical device was contacted for this review.
