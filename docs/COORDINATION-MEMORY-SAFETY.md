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
| `src-tauri/src/fs_ext.rs` | `volume_space`, `try_flock_exclusive`/`try_flock_shared` (Unix arms) and their tests |
| `src-tauri/src/acquisition_tests.rs` | `cpu_seconds`, `bench_loopback_throughput` |
| `src-tauri/src/cpu_time.rs` | documentation and lint reason only |
| `docs/RUST-UNSAFE-REVIEW.md` | sections `## Follow-up implemented 2026-11-19` and the second batch |
| `docs/MEMORY-SAFETY-INVENTORY.md` | subsection `### Progress since the 2026-11-17 snapshot` |
| `CHANGELOG.md` | the bullet starting `- Remove 15 hand-written unsafe blocks` |

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

## Status

- Verified on macOS: `cargo check --lib`, `cargo clippy --lib -- -D warnings`, and the publication,
  exclusive, capture, rename, composer, `fs_ext::`, `instance::` and `library_root::` test filters.
- Verified for Android: `cargo check --lib --target aarch64-linux-android --features android-evaluation`
  with NDK 28 under Rust 1.99.1.
- Not run: iOS/Linux cross-target checks, device tests, sanitizer/Miri.
- First batch: 239 → 226 matches, 42 → 37 files. Second batch (`84fd93e7de`, `cdfdf64478`): 226 → 223.
  The four initial findings and two follow-up findings from the first batch are fixed and independently
  verified.
- The second batch is under independent review (statvfs parity qualification already returned; flock change
  included in the same review round).

## Not reserved (safe for the other session)

- `src-tauri/src/fs_ext.rs` `flock`, `src-tauri/src/instance.rs` `getpeereid`
- Windows rename arms (`storage::rename_into_directory_noreplace`, `MoveFileExW`)
- A SAFETY-comment sweep across the remaining blocks
- Inventory rows A1–A5, B1/B2/B4–B9, C1–C5, and the `unsafe impl Send/Sync` sign-offs
