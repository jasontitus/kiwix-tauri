# Coordination: Rust memory-safety work, 2026-11-19

One session owns the items below. The other session should avoid these lines/files until this note is
removed or updated. Coordination happens through commits and this file; there is no other shared channel.

## Owner

Session working the Rust memory-safety worklist (`docs/MEMORY-SAFETY-INVENTORY.md`) rows **B3
(exclusive-rename subset)**, **B4 (provider descriptor duplication only)** and **B10 (diagnostic clocks)**.

## Reserved paths and symbols

| Path | Reserved part |
|---|---|
| `src-tauri/src/publication.rs` | `platform_publish` (both Unix arms), `platform_publish_in`, `check_path`, `publication::tests::nul_paths_*` and `dangling_symlink_*` |
| `src-tauri/src/maps/partials.rs` | `capture_partial` (Unix arm) |
| `src-tauri/src/library_files.rs` | `move_exclusive` (Unix arm) |
| `src-tauri/src/torrent/publication.rs` | `move_publication_entry` (Unix arm) |
| `src-tauri/src/provider_reference.rs` | native `retain` descriptor duplication |
| `src-tauri/src/localswarm/storage.rs` | `rename_directory_noreplace` (Unix arm) |
| `src-tauri/src/maps.rs` | `evaluation` module and `Control::chunk_sha256` clock reads |
| `src-tauri/src/fs_ext.rs` | `volume_space`, `try_flock_exclusive`/`try_flock_shared` (Unix arms) and their tests |
| `src-tauri/src/acquisition_tests.rs` | `cpu_seconds`, `bench_loopback_throughput` |
| `src-tauri/src/cpu_time.rs` | documentation and lint reason only |
| `docs/RUST-UNSAFE-REVIEW.md` | the 2026-11-19 follow-up sections (all batches) |
| `docs/MEMORY-SAFETY-INVENTORY.md` | subsection `### Progress since the 2026-11-17 snapshot` |
| `CHANGELOG.md` | the memory-safety bullet beginning `- Remove` |

## Commits

- `942c36306b` — removed 10 raw `renameatx_np`/`renameat2` blocks (four files, six functions) in favour of
  `rustix::fs::renameat_with(RenameFlags::NOREPLACE)`, and 3 diagnostic clock blocks in favour of
  `cpu_time` helpers.
- `cf24679f21` — restores the pre-rename interior-NUL rejection (`check_path`, safe byte scan) after review
  found the Linux fallback could misread rustix's `EINVAL`; adds a regression test; corrects the docs.
- `28a8d06b37` — restores the `dangling_symlink_is_an_existing_destination` tail (assertion + cleanup) that
  the regression-test insertion displaced, and scopes the doc section's revision claim.
- `84fd93e7de` — replaces the Unix `libc::statvfs` blocks with `rustix::fs::statvfs`; same fields, arithmetic
  and NUL diagnostic.
- `cdfdf64478` — replaces the Unix `libc::flock` helper with `rustix::fs::flock`; same non-blocking
  operations and `WouldBlock` mapping.
- `7b50d62763` — torrent publication rename now uses `rustix::fs::renameat_with`; provider descriptor
  duplication uses `rustix::io::fcntl_dupfd_cloexec` and `rustix::fs::tell` (one documented `borrow_raw`).

## Status

- Verified on macOS: `cargo check --lib`, `cargo clippy --lib -- -D warnings`, and the publication,
  exclusive, capture, rename, composer, `fs_ext::`, `instance::`, `library_root::`, torrent-publication and
  `provider_reference` test filters.
- Verified for Android: `cargo check --lib --target aarch64-linux-android --features android-evaluation`
  with NDK 28 under Rust 1.99.1.
- Not run: iOS/Linux cross-target checks, device tests, sanitizer/Miri.
- First batch: 239 → 226 matches, 42 → 37 files. Second batch (`84fd93e7de`, `cdfdf64478`): 226 → 223.
  Third batch (`7b50d62763`): 223 → 219 matches, 37 → 36 files. The four initial findings and two follow-up
  findings from the first batch are fixed and independently verified.
- The second batch is independently reviewed; two P3 documentation findings (batch count, reservation
  synchronization) are fixed in its follow-up commit.
- The third batch is independently reviewed; two P3 documentation findings (cumulative file count, Android
  seek-probe wording) are fixed in its follow-up commit.

## Not reserved (safe for the other session)

- `src-tauri/src/instance.rs` `getpeereid`
- Windows rename arms (`storage::rename_into_directory_noreplace`, `MoveFileExW`)
- A SAFETY-comment sweep across the remaining blocks
- Inventory rows A1–A5, B1/B2, the B4 remainder (lease lifetimes, revoke/regrant, backup flags, bookmark
  scope, Swift delegate chaining), B5–B9, C1–C5, and the `unsafe impl Send/Sync` sign-offs
