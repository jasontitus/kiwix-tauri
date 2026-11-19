# Coordination: Rust memory-safety work, 2026-11-19

One session owns the items below. The other session should avoid these lines/files until this note is
removed or updated. Coordination happens through commits and this file; there is no other shared channel.

## Owner

Session working the Rust memory-safety worklist (`docs/MEMORY-SAFETY-INVENTORY.md`) rows **B3
(exclusive-rename subset)** and **B10 (diagnostic clocks)**.

## Reserved paths and symbols

| Path | Reserved part |
|---|---|
| `src-tauri/src/publication.rs` | `platform_publish` (both Unix arms), `platform_publish_in`, `check_path`, `publication::tests::nul_paths_*` and `dangling_symlink_*` |
| `src-tauri/src/maps/partials.rs` | `capture_partial` (Unix arm) |
| `src-tauri/src/library_files.rs` | `move_exclusive` (Unix arm) |
| `src-tauri/src/localswarm/storage.rs` | `rename_directory_noreplace` (Unix arm) |
| `src-tauri/src/maps.rs` | `evaluation` module and `Control::chunk_sha256` clock reads |
| `src-tauri/src/acquisition_tests.rs` | `cpu_seconds`, `bench_loopback_throughput` |
| `src-tauri/src/cpu_time.rs` | documentation and lint reason only |
| `docs/RUST-UNSAFE-REVIEW.md` | section `## Follow-up implemented 2026-11-19` |
| `docs/MEMORY-SAFETY-INVENTORY.md` | subsection `### Progress since the 2026-11-17 snapshot` |
| `CHANGELOG.md` | the bullet starting `- Remove 13 hand-written unsafe blocks` |

## Commits

- `942c36306b` — removed 10 raw `renameatx_np`/`renameat2` blocks (four files, six functions) in favour of
  `rustix::fs::renameat_with(RenameFlags::NOREPLACE)`, and 3 diagnostic clock blocks in favour of
  `cpu_time` helpers.
- `cf24679f21` — restores the pre-rename interior-NUL rejection (`check_path`, safe byte scan) after review
  found the Linux fallback could misread rustix's `EINVAL`; adds a regression test; corrects the docs.
- Follow-up pending: restore the `dangling_symlink_is_an_existing_destination` tail (assertion + cleanup)
  that the regression-test insertion displaced, and scope the doc section's revision claim.

## Status

- Verified on macOS: `cargo check --lib`, `cargo clippy --lib -- -D warnings`, and the publication,
  exclusive, capture, rename, composer, `instance::` and `library_root::` test filters.
- Verified for Android: `cargo check --lib --target aarch64-linux-android --features android-evaluation`
  with NDK 28 under Rust 1.99.1.
- Not run: iOS/Linux cross-target checks, device tests, sanitizer/Miri.
- Independent review completed; two follow-up findings are being fixed in the next commit.

## Not reserved (safe for the other session)

- `src-tauri/src/fs_ext.rs` flock/statvfs, `src-tauri/src/instance.rs` `getpeereid`
- Windows rename arms (`storage::rename_into_directory_noreplace`, `MoveFileExW`)
- A SAFETY-comment sweep across the remaining blocks
- Inventory rows A1–A5, B1/B2/B4–B9, C1–C5, and the `unsafe impl Send/Sync` sign-offs
