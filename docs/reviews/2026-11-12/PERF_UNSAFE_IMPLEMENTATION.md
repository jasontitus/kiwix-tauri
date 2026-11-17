# Performance and unsafe review implementation — 2026-11-12

Base revision: `27767fa`. Reviewed worktree snapshot: 134 changed files, +5997/−1690, plus new
`src-tauri/src/cpu_time.rs`. The exact path list is reproducible with
`git diff --name-only 27767fa` plus that untracked file.

## Scope

- `docs/reviews/2026-11-12/PI_REVIEW_PERF_DEEPSEEK_V4_1_FLASH_1017_20261112.md` (triaged: 19 high,
  172 medium, 36 low retained). This change applies all 19 high-severity findings and a broad
  medium/low subset. Entries under "Refuted in triage" were not touched.
- `docs/RUST-UNSAFE-REVIEW.md`: lint policy, effective-UID and CPU-time helpers, SAFETY
  documentation, and the Linux peer-credential reduction.

## High-severity dispositions

| # | Finding | Disposition |
|---|---|---|
| 1 | Android `ExternalArchiveMove` N² hashing | Fixed: digest once per invocation, stat-identity reuse, target re-hashed before delete |
| 2 | iOS `NearbyBluetooth` per-packet log open/stat/flush | Fixed: persistent `O_APPEND` handle, buffered flush, rename rotation, failure backoff |
| 3 | Reader shortcut effect re-registers every render | Fixed: one registration, handlers via ref |
| 4 | Reader per-frame `publishTab` | Fixed: 250 ms trailing publish with final flush |
| 5 | `storage-bridge` full-store quota serialization | Fixed: incremental byte accounting; no per-mutation full scan |
| 6 | `IntegrityPanel` O(targets×books) | Fixed: indexed lookups and payload dedupe |
| 7 | `Store::update` double clone/pretty persist | Fixed: one clone, compact JSON; full validation and both durability syncs retained |
| 8 | `reading_data::capture` double validate/projection | Partial: one validation and per-touched-article baseline; atomic clone retained |
| 9 | `projection_ids.retain` O(bookmarks×projections) | Fixed: reverse ownership index |
| 10 | Torrent journal rewrite per created file | Partial: record clones/directory cache fixed; journal still rewritten per entry for crash safety |
| 11 | Torrent `record()` clone per file | Fixed: targeted cleanup lookup |
| 12 | Acquisition claims scan per file | Fixed: precomputed lexical key set; one claim snapshot per scan |
| 13 | Acquisition deletion hashes under global lock | Fixed: digest outside the lock; row revalidated before unlink |
| 14 | Sharing `available_books` clone + stat per request | Fixed: `Store::books()` + bounded availability cache |
| 15 | Library search opens every archive | Partial: selected-book search fixed; library-wide still resolves each archive once, 8-handle LRU |
| 16 | Library move re-hashes every part five times | Fixed: same-filesystem 5→2 full passes; cross-volume 13→3 |
| 17 | New `reqwest::Client` per catalog fetch | Fixed: shared policy-keyed clients |
| 18 | Maps `selections` rebuilt per render | Fixed: memoized selection map and canonical cache |
| 19 | Maps `languageOptions` per card | Fixed: per-region+locale option cache |

## Other fixes

- Rust: store/reading-sync accessors and persistence, torrent gates/record handling, acquisition
  journal/deletion/transfer/segment/finalize paths, sharing availability and provider-reference
  exclusion, catalog clients/indexes, library move proofs and journal batching, LocalSwarm
  checkpoint ordering and state readers, maps candidate identity, portable parsing, spelling caches,
  mobile lifecycle cadence, private-sync idle skip, notifications, watcher directory refresh, search
  bounds, loopback storage mutation, integrity indexes, diagnostics gate, nearby log handle.
- Frontend: Reader/App effect and scroll publishing, memoized indexes across Library, Collections,
  Downloads, Catalog, Maps, IntegrityPanel, SharingControls, UnifiedSearch, UI, i18n, speech,
  reference transitions; storage-bridge incremental quota/key cache; reader-storage head-cursor
  queue and early size cap; platform resize debounce; native-speech dependency fix.
- Android: metadata-only scan probes when stat is unavailable, digest cache and destructive-target
  re-hash, executor split, off-main durability writes, provider reference rollback, notification and
  PendingIntent caches, lifecycle-driven Nearby retirement, PDF off-main write, voice/recognizer TTL
  caches, logging gates.
- iOS: shared nearby log append/rotation/backoff, bounded background retirement, reconcile error
  aggregation, digest authority for stale resource-less provider parts, export retention cleanup,
  lifecycle polling, voice/regex caches.

## Unsafe review

- `#![deny(unsafe_op_in_unsafe_fn)]` at both crate roots.
- `#![deny(unsafe_code)]` on 19 modules verified to contain no unsafe operations.
- All 13 `libc::geteuid()` calls replaced with `fs_ext::effective_uid()` using
  `rustix::process::geteuid`.
- Linux `SO_PEERCRED` replaced with `rustix::net::sockopt::socket_peercred`; macOS/iOS/BSD
  `getpeereid` remains raw with a SAFETY comment.
- CPU-time reads centralized in `src-tauri/src/cpu_time.rs`.
- SAFETY documentation added to the six `Send`/`Sync` impls.
- Inventory after the pass: 223 `unsafe` blocks (from 238), 41 files (from 42), 0 `geteuid` calls.
- Kept raw with reasons: `fs_ext` flock/statvfs, macOS/iOS/BSD `getpeereid`, `library_root` exclusive
  rename, `maps/partials` `renameatx_np`/`renameat2`.

## Independent review

Ten independent reviewers covered disjoint subsystems: store/reading-sync, torrent/acquisition,
library/LocalSwarm, sharing/network/mobile, Reader/App, frontend components/storage, Android, iOS,
unsafe, and cross-cutting integration. Findings were fixed and re-reviewed through multiple rounds;
the final round returned no open findings.

`RustAcqHot` failed at the network boundary after writing its changes; 7 of its 8 assigned findings
were verified in the tree. `DownloadManager::list()`/`list_snapshots()` still clone retained rows per
status query (medium residual). `ReviewLibrarySwarm` also failed once on the same network error and
was retried successfully. No agents were running when this record was written.

## Verification exercised

- `scripts/dev.sh check` — pass.
- `scripts/dev.sh test` — 1086 passed, 0 failed, 19 ignored.
- `cargo clippy --lib -- -D warnings` — pass (only the pre-existing vendor `wry` cfg warning).
- `npm run build` — pass.
- `node --test scripts/test-*.mjs` on Node 22.23.2 — 315 passed, 0 failed.
- `swiftc -parse` on changed Swift files — pass.
- Desktop smoke with an isolated `HOME`: `KIWIX_INSTANCE_READY`, `KIWIX_DESKTOP_READY`, profile
  files created, clean SIGTERM.
- Python suite — 177/178; the one failure is environmental (`xcrun: unable to find utility
  "devicectl"`).
- Kotlin was not compiled: the generated Android project lacks `.tauri/tauri-api`; Kotlin changes
  were source-reviewed only.
- No physical-device window was opened. The iPhone 17 Pro Max test device is visible over Network, but no
  iOS SDK, signing source, or `.native/iphone-config.json` is present, and the worktree was
  uncommitted at build time.

## Residual blockers and limits

- `shared-library-batch.ts` sequential admission and per-book progress are pinned by existing tests;
  changing them would allow N−1 already-admitted books after a failure.
- `api.ts` `saveBytes` and `structuredClone` findings were refuted with measurements/contract
  analysis.
- Library move proof uses rename-stable device/inode/size/mtime; an in-place rewrite that restores
  mtime is a documented residual in the owner-only staging window.
- Android rows whose raw path stats successfully still advertise readability without the
  `startOffset`/`declaredLength` check; `open(token)` safely rejects them.
- Library-wide sharing search still opens each archive once; LocalSwarm checkpoints and torrent
  journals still rewrite full documents where the recovery format requires it.
- Not addressed: `archive.rs` integrity-under-archive-mutex, `maps/partials.rs` incremental prefix
  hashing (hasher state unavailable), `fs_ext` flock/statvfs, and `renameatx_np` residuals.
- New sharing strings (`Next page`, `Refine your search…`) fall back to English; no locale entries
  were added.
- `DownloadManager::list()`/`list_snapshots()` row cloning remains.
