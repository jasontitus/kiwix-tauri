# Rust `unsafe` usage review — 2026-11-12

For the current cross-platform replacement checklist, including native and transitive dependencies, see the [memory-safety inventory](MEMORY-SAFETY-INVENTORY.md).

Scope: hand-written Rust under `src-tauri/src` and `src-tauri/build.rs` at `3d289b1`. Vendored crates,
`target/`, generated Android projects and native C/C++/Objective-C++ sources are excluded from the Rust
keyword counts, although the C/C++ FFI boundary is the reason most `unsafe` exists.

This is an inventory and eliminability assessment, **not** a line-by-line proof that every `unsafe` block is
sound. No Miri, FFI sanitizer, fuzz run or new per-block UB audit was performed for this review.

## Inventory

```sh
rg -o 'unsafe\s*\{' src-tauri/src | wc -l          # 238 blocks
rg -l 'unsafe\s*\{' src-tauri/src | wc -l          # 42 files
rg -n 'unsafe (extern|impl|fn)' src-tauri/src | wc -l  # 50 items
rg -n 'unsafe impl' src-tauri/src | wc -l          # 6 items
rg -n 'libc::geteuid' src-tauri/src | wc -l        # 13 calls
```

| Metric | Count |
|---|---:|
| `unsafe { … }` blocks | **238** |
| Files containing unsafe blocks | **42** |
| `unsafe extern` / `unsafe fn` items | **50** |
| `unsafe impl` items | **6** |
| `libc::geteuid()` calls | **13** |
| Unsafe blocks in `*tests.rs` | **4** |

Top files by unsafe-block count:

| File | Blocks | Dominant boundary |
|---|---:|---|
| `localswarm/apple_transport.rs` | 22 | Apple C QUIC transport callbacks and raw handles |
| `localswarm/ios_discovery.rs` | 21 | DNS-SD C callbacks |
| `private_sync_identity.rs` | 16 | `geteuid`, Windows token/SID APIs |
| `native_pdf/linux.rs` | 15 | GTK printer enumeration and dialogs |
| `storage.rs` | 15 | Win32/NT file identity, rename and deletion |
| `native_pdf/apple.rs` | 12 | PDFKit/CoreGraphics |
| `native_pdf/windows_print.rs` | 11 | Win32 print dialog |
| `nearby_route.rs` | 11 | Apple networking primitives |
| `windows.rs` | 10 | Win32 windowing |
| `archive.rs` | 9 | libzim C ABI and `Handle` |

The six `unsafe impl` items are `Send`/`Sync` markers for raw handles: `archive::Handle`,
`localswarm::apple_transport::{RawConnection, RawListener}` and `power_activity::Token`.

## Findings

1. **Most `unsafe` is the native ABI boundary, not ordinary application logic.** libzim, Win32/NT, GTK,
   PDFKit/CoreGraphics, Network.framework/DNS-SD and Android JNI have no complete safe Rust API. The
   `unsafe` keyword is doing its intended job at that boundary.
2. **A smaller set is replaceable with safe OS wrappers.** The clearest examples are the 13 `geteuid`
   calls, the libc lock/volume/rename calls in `fs_ext.rs` and `library_root.rs`, peer-credential calls in
   `instance.rs`, and CPU-time calls used only by tests and diagnostics.
3. **The `unsafe impl Send`/`Sync` markers are the highest-consequence claims.** They assert that a raw
   native handle can cross threads safely. Those invariants are not derivable by the compiler and deserve
   explicit ownership/lifetime documentation and focused tests.
4. **There is no crate-wide unsafe lint policy.** The tree does not currently deny
   `unsafe_op_in_unsafe_fn`, and safe modules are not mechanically prevented from adding `unsafe`.
5. **The test-only unsafe is avoidable.** `getrusage`, `clock_gettime` and `GetThreadTimes` calls used for
   CPU-limit measurements can use safe crates or be isolated behind a test helper.

## Can it be eliminated?

**Not completely** without replacing libzim and the platform APIs with pure-Rust equivalents. Even using
`nix`, `rustix`, `fs2`, `fs4` or `windows` only moves the unsafe implementation into a dependency; the
native operations remain unsafe underneath. A crate-wide `#![forbid(unsafe_code)]` would require a
different native stack.

**A meaningful subset can be eliminated from this crate.** Candidate reductions, in priority order:

| Candidate | Approx. blocks | Approach | Risk |
|---|---:|---|---|
| `geteuid` checks | 13 | One safe helper using `rustix::process::geteuid` or `nix::unistd::geteuid` | Low |
| Test/diagnostic CPU time | 4–5 | `cpu-time`/`nix` safe APIs, or a narrow test helper | Low |
| `fs_ext` locks and volume space | 4 | `fs2`/`fs4`/`rustix` safe wrappers | Medium: lock semantics must stay advisory/compatible |
| `library_root` exclusive rename | 2 | `rustix::fs::renameat_with` where available | Medium: platform fallbacks |
| `instance` peer credentials | 2–3 | `nix`/`rustix` safe credential APIs | Medium: macOS `getpeereid` coverage |
| Safe-module isolation | — | `#![deny(unsafe_code)]` on modules with no native boundary | Low |

The remaining blocks should stay, but be made easier to review:

- keep all raw FFI behind narrow safe wrappers in dedicated modules;
- require a `SAFETY:` comment naming the pointer/lifetime/ownership invariant for every block;
- add `#![deny(unsafe_op_in_unsafe_fn)]` and `#![deny(unsafe_code)]` to safe modules;
- run `cargo-geiger` as a trend check, not a pass/fail quality target;
- exercise callback ownership, `Box::from_raw`, `Send`/`Sync` handles and error paths with focused tests;
- use sanitizers/fuzzing for the C/C++ side where feasible. Miri cannot validate these FFI calls.

Zero `unsafe` is therefore not the practical goal. The practical goal is **no unsafe in safe application
logic and a small, documented, tested native boundary**.

## Follow-up implemented 2026-11-12

This section records what changed after the review above. It is a static inventory plus the rationale for
what was kept; it is **not** a soundness proof, and no Miri, FFI sanitizer or fuzz run was added.

Recounted with the same commands (`rg` replaced by `grep -r` over `src-tauri/src`; baseline measured with
the same tool at `3d289b1`, current tree at the follow-up commit):

| Metric | Before | After |
|---|---:|---:|
| `unsafe { … }` blocks | 238 | **223** |
| Files containing unsafe blocks | 42 | **41** |
| `unsafe extern` / `unsafe fn` / `unsafe impl` lines | 56 (review said 50) | **57** (56 code lines + 1 `lib.rs` comment; 6 are `unsafe impl`) |
| `libc::geteuid()` calls | 13 | **0** |
| Unsafe blocks in `*tests.rs` | 4 | **3** |

This pass removed 17 blocks (all 13 `geteuid` calls across seven files, 3 raw CPU-clock reads, and the
Linux `SO_PEERCRED` block) and added one centralized Windows block. One documentation comment in `lib.rs`
quotes `unsafe {}` and is also matched by the text search, so the measured net delta is 15. The
`unsafe`-item line count is `grep` line-based and includes extern-block declarations; it is not a new
boundary.

**Eliminated from this crate**

1. **`geteuid` (all 13 calls).** `fs_ext::effective_uid() -> u32` wraps
   `rustix::process::geteuid().as_raw()`. Callers in `private_sync_identity.rs` (3), `mobile_rebase.rs` (3),
   `instance.rs` (3) and `localswarm/storage.rs` (1) now call it, as do `torrent/publication.rs`,
   `library_files.rs` and `torrent_tests.rs` after the same replacement by the agents that owned those
   files. Ownership and permission comparisons are unchanged: same effective UID, same error messages,
   same `#[cfg(unix)]` gates. The helper is `#[cfg(unix)]`, matching every call site.
2. **CPU-time reads (3 blocks).** `src/cpu_time.rs` owns the platform reads:
   - Unix uses `rustix::time::clock_gettime_dynamic(Known(ClockId::ThreadCPUTime | ProcessCPUTime))`. The
     fallible rustix API is deliberate: `rustix::time::clock_gettime` panics on failure, which would turn a
     failed diagnostic clock into an abort, while the previous code returned `None`.
   - Windows keeps `GetThreadTimes` with one SAFETY block that names the pseudo-handle and the four
     `FILETIME` outputs; kernel+user 100 ns ticks are converted to microseconds, preserving the previous
     semantics (per-tick granularity, wall-clock fallback).
   - `exports.rs`'s `CpuClock` and `android_evaluation.rs`'s `cpu_us` now call
     `thread_cpu_time_micros()` / `process_cpu_time_micros()`. The only observable change is that
     `CpuClock` reports whole microseconds instead of nanoseconds; the CPU budgets it enforces (50 ms and
     above) are far above that resolution. A deterministic unit test covers the `Timespec` → microsecond
     conversion (clamping and truncation); no timing-dependent assertion was added.
3. **New dependency.** `rustix = { version = "1", default-features = false, features = ["process", "time", "net"] }`
   under `[target.'cfg(unix)'.dependencies]`. It is already in the graph at 1.1.6 through `cap-std` with the
   same lock entry, so only the root package's dependency list changed; `Cargo.lock` was updated by hand to
   add `rustix` there, and `cargo check` should confirm the lock is accepted as-is. Non-Unix targets never
   compile it. The `net` feature is used only for Linux `SO_PEERCRED`.
4. **Lint policy.**
   - `#![deny(unsafe_op_in_unsafe_fn)]` at both crate roots (`lib.rs`, `main.rs`). The 3 `unsafe fn` and 14
     `unsafe extern "C" fn` bodies in the tree already wrapped their unsafe operations, so no body needed
     rewriting.
   - `#![deny(unsafe_code)]` on modules verified to contain no `unsafe` token and no unsafe-producing
     macro: `casefold`, `crash`, `disk_warning`, `localization`, `migration`, `nearby_log`,
     `profile_paths`, `publisher`, `spelling`, `transfer_response`, `update_admission`, `user_error`,
     `zim_scheme`, `integrity`, `partial_donor`, plus the now-unsafe-free `exports`, `mobile_rebase` and
     `android_evaluation`, and the binary crate root (`main.rs`). Deliberately not added crate-wide:
     libzim/Win32/GTK/PDFKit/Network.framework/JNI remain.
5. **SAFETY documentation.** The six `unsafe impl Send`/`Sync` items (`archive::Handle`,
   `localswarm::apple_transport::{RawConnection, RawListener}`, `power_activity::Token`) now state the
   ownership, lifetime and thread invariant and who enforces it (private field + `Mutex`/`Arc`/native
   serial queue). The remaining `instance.rs` `getpeereid` block gained a SAFETY comment. No impl or check
   changed.

**Kept, with reasons**

- **`fs_ext.rs` flock/statvfs.** `rustix::fs` would move the call, not remove it, and the replacement was
  not verified against the current behavior on all five targets: `flock(2)` lock semantics, the
  `EWOULDBLOCK` → `WouldBlock` mapping and the Windows `ERROR_LOCK_VIOLATION` (33) mapping are load-bearing
  for `store::lock_failure`, and std's own `File::try_lock` is still `Unsupported` on Android. The four
  blocks keep their SAFETY comments.
- **`instance.rs` peer credentials.** Linux now uses
  `rustix::net::sockopt::socket_peercred` (safe, `net` feature) and compares the typed UID. rustix exposes
  no `getpeereid`, so the macOS/iOS/BSD block remains a single raw `getpeereid` call with a SAFETY comment;
  `nix` would be a new dependency. The owner check is unchanged.
- **`library_root.rs` exclusive rename and `maps/partials.rs` `renameatx_np`/`renameat2`.** Out of scope
  for this pass (owned elsewhere); still candidates for `rustix::fs::renameat_with` on Linux and a
  documented Apple-only path.
- **Test CPU time (`acquisition_tests.rs` `getrusage`, `maps.rs` `thread_cpu_us`).** Out of scope for the
  files this pass owned; both are the same class of read now centralized for the other callers.

## Follow-up implemented 2026-11-19

Scope: counts are for `942c36306b`, hand-written Rust under `src-tauri/src`. The NUL precheck and its
regression test were added in the corrective follow-up `cf24679f21`; the preserved NUL diagnostic described
below applies to `publication.rs` at that revision. Counts use
`git grep -o -e 'unsafe[[:space:]]*{'` over `src-tauri/src/*.rs`, so they include matches in comments and
are comparable only with the same command at the two revisions below.

| Metric | Before (`942c36306b^`) | After |
|---|---:|---:|
| `unsafe { … }` matches | 239 | **226** |
| Files containing them | 42 | **37** |
| `unsafe extern` / `unsafe fn` / `unsafe impl` lines | 58 | **58** |
| `getrusage` / `CLOCK_THREAD_CPUTIME_ID` call sites | 2 | **0** |

This closes the two candidates the 2026-11-12 section left open. It is again an inventory change, not a
soundness proof; no Miri, sanitizer or fuzz run was added.

**Eliminated from this crate**

1. **Exclusive rename (10 blocks, six functions in four files).** `publication::platform_publish` (both
   cfg arms), `publication::platform_publish_in`, `maps::partials::capture_partial`,
   `library_files::move_exclusive` (Unix arm) and `localswarm::storage::rename_directory_noreplace` now call
   `rustix::fs::renameat_with(…, RenameFlags::NOREPLACE)`, the same call `library_root::publish_exclusive`
   already used. rustix maps the flag to `RENAME_NOREPLACE` on Linux/Android and `RENAME_EXCL` on Apple, so
   the filesystem still enforces no-replace rather than a racy existence check. The raw `CString`/fd
   plumbing is gone; an interior NUL is still rejected with the same `InvalidInput` error and message
   before the rename, so the Linux fallback cannot misread it as an unsupported rename.
   Behavior notes:
   - Linux keeps the old fallback: `ENOSYS`, `EINVAL` or `EOPNOTSUPP` from the exclusive rename still falls
     back to atomic `link(2)` + unlink for regular files. rustix issues the `renameat2` syscall itself when
     libc lacks a wrapper (`weak_or_syscall!`), and its default Linux backend is the raw syscall.
   - Android uses rustix's libc backend, which also calls `SYS_renameat2` directly.
   - Apple uses `renameatx_np` through a weak symbol. A host without the symbol now gets `ENOSYS` at the
     call instead of a load-time link failure; every supported deployment target provides it.
   - Error text is unchanged: `Errno`'s `Display` delegates to `std::io::Error`, and the publication
     fallback still inspects `raw_os_error`.
2. **Diagnostic CPU clocks (3 blocks).** `maps::Control::chunk_sha256` (Android debug evaluation) now
   reads `cpu_time::thread_cpu_time_micros` directly instead of a module-local helper, and the ignored
   `bench_loopback_throughput` harness calls `cpu_time::process_cpu_time_micros` instead of `getrusage`
   plus `assume_init`. Both are the same clocks the 2026-11-12 pass centralized, at the same microsecond
   granularity. The bench keeps failing loudly if the process clock is unreadable, as the old
   `assert_eq!` did.

**Files that became unsafe-free:** `acquisition_tests.rs`, `library_files.rs`, `localswarm/storage.rs`,
`maps.rs`, `maps/partials.rs`.

**Kept, with reasons**

- `fs_ext.rs` flock/statvfs and `instance.rs` `getpeereid`: unchanged from the 2026-11-12 rationale.
- Windows rename arms (`storage::rename_into_directory_noreplace`, `MoveFileExW`): the handle-based NT
  rename is a different contract and was not part of this pass.
- The SAFETY-comment gap remains: an audit of the pre-change tree found 201 of its 238 same-line blocks
  with no `SAFETY:` comment within six lines. That sweep needs a dedicated review, not a mechanical
  comment insert.

**Verification actually exercised**

- `cargo check --lib` and `cargo clippy --lib -- -D warnings` pass on macOS (host, `aarch64-apple-darwin`).
- Targeted tests passed: `publication::` (9, including the new interior-NUL regression), `exclusive` (6,
  including `library_files::exclusive_move…`, `maps::cleanup_captures…`,
  `startup_recovers_a_split_archive…`), `capture` (24), `rename` (9), `composer` (5), `instance::` (4) and
  `library_root::` (35). The last two exercise `rename_entry_noreplace` and the library recovery/publication
  paths that call it.
- `cargo check --lib --target aarch64-linux-android --features android-evaluation` passes with NDK 28 under
  Rust 1.99.1. Toolchain roles: `CC_aarch64_linux_android=aarch64-linux-android24-clang`,
  `CXX_aarch64_linux_android=aarch64-linux-android24-clang++`, `AR_aarch64_linux_android=llvm-ar`,
  `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android24-clang`. This covers the Android rename
  arms, rustix's libc backend and the changed `android-evaluation`-gated hash caller.
- Not run: iOS/Linux cross-target checks (this host's toolchains lack those std targets or C cross
  toolchains), any device or installed-app test, and sanitizer/fuzz runs.
