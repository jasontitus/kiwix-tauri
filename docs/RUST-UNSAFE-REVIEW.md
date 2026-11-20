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

## First memory-safety batches integrated 2026-11-19

The merge combines private parent `696a3b82554575bee78d48542de6774d18f18169`
with coordination tip `359fbc16f6c4a3c498fcc03fccc9de997e6a0ada`.
Their common base is `f2875d097225509c75a884739960cdf0951ef9fd`. Review covered the 18 incoming
paths, the 11 conflicts and the final seven changed Rust files. The Rust file manifest has SHA-256
`9f517231597570bcd62b2d2fa1a2975f8fc3c9274a8e90948f5bb3511f4d1504`.

- The private parent already used typed Unix exclusive rename, volume queries, advisory locks and
  provider descriptor duplication. Integration keeps the shared directory-relative rename helper,
  newer library and map handling, Windows whole-file locks, and provider lease and retry checks.
  The Unix volume error now reports an embedded NUL explicitly. Tests cover that diagnostic and
  confirm that an invalid publication name returns the path error before any Linux fallback.
- The Android map profiler and ignored loopback throughput harness now use the existing Rust clock
  helpers. The profiler keeps thread time and the benchmark keeps process time.
- The existing provider helper now calls `rustix::fs::tell` for its regular-file seekability check.
  The pinned rustix backends issue the same zero-offset `SEEK_CUR` operation. The duplicate still
  retains close-on-exec and the original provider lease remains live through duplication.
- Two comments for the iOS PDF export handoff describe the locked callback registration and
  synchronous token borrow. The newer Apple sharing, rendering, printing and notification
  implementations keep their request ID and payload lifetime handling.
- The merged source census identifies 131 app and 13 relay production Rust blocks and 36 declarations.
  The comparable pre-merge census had 145 blocks. The total excludes test-only source and dependencies
  and includes the Android debug evaluation code. It counts explicit Rust syntax across targets.

Independent filesystem and native-boundary reviews rechecked the exact resolved Rust files. The
filesystem review also ran six focused tests from the merged Mac test executable. The integration
review checked the disposition of the historical counts, callback comments and platform claims.
See the coordination-branch status below for the separate first-round review findings.

One inherited P2 finding remains in `exports::PreparedOutput::publish` on Linux and Android.
When the filesystem rejects an exclusive rename, the ambient publisher can hard-link the
temporary file and then remove the source by name. If that name changes between the two operations
in a writable export folder, the removal can affect a replacement file. `exports::Temporary::drop`
has a separate identity-check-to-unlink gap. Review traced both cases in source. The merge preserves
the existing publication implementation, including the fallback. A separate change needs to
address publication and cleanup ownership together and exercise old-kernel and removable-volume
failures. The verified directory-relative publisher avoids that fallback.

### Verification on the merged sources

- **macOS ARM64.** The locked debug test binary passed strict signature verification.
  The serial suite passed 1,281 tests and skipped 25. One PDF bridge test failed because the scratch
  checkout lacked the locked TypeScript dependency. It passed on the same binary after supplying
  that dependency. The final focused run passed 100 filesystem, provider, clock and callback tests.
  The ignored 256 MiB loopback test separately completed a verified download and reported process
  CPU time. The package configuration scripts passed four checks and skipped the Linux-only check.
- **Linux x86-64.** The locked debug app test binary in the configured Linux build container
  passed 96 focused tests, including exclusive publication, volume errors, provider descriptor
  ownership and capture recovery. The first run lacked the same TypeScript fixture. After supplying
  it, the focused tests and the 256 MiB loopback clock test passed.
- **Mobile.** Locked app-library checks passed for ARM64 iOS device, iOS simulator and Android.
  The Android check enabled `android-evaluation` and used NDK 28. The iOS checks reported an existing
  unused import in the unchanged Apple PDF module. These were compile checks without device
  execution, document-provider access or UI presentation. No fresh libzim comparison or release
  packages were produced for this merge.

### Historical coordination-branch evidence

The four sections below describe the coordination branch through `359fbc1`. Their counts and
native callback descriptions are scoped to those revisions. The combined source census and retained
implementations are recorded above.

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

- `fs_ext.rs` flock/statvfs and `instance.rs` `getpeereid`: unchanged in this batch; the first two were
  replaced in the second batch below.
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

## Follow-up implemented 2026-11-19 (second batch)

`84fd93e7de` and `cdfdf64478` replace the three remaining Unix `unsafe` blocks in `fs_ext`:

- **Volume space (`84fd93e7de`).** `volume_space` calls `rustix::fs::statvfs`. Apple and Android still reach
  the libc call; on Linux rustix derives `statvfs` from `statfs` with the same `f_frsize`→`f_bsize` fallback
  glibc uses. rustix widens every field to `u64`, so the platform-width casts are gone; the arithmetic stays
  `f_frsize × f_bavail` and `f_frsize × f_blocks` with saturating multiplication. An interior NUL is still
  rejected with the original `InvalidInput` message before the call. One narrow exception is source-traced,
  not reproduced: on 32-bit glibc with narrow block counters, the old `statvfs` could return `EOVERFLOW`
  where rustix's `statfs64` path reports the capacity. Neither packaged Linux architecture is affected.
- **Advisory locks (`cdfdf64478`).** `try_flock_exclusive`/`try_flock_shared` call
  `rustix::fs::flock` with the non-blocking operations, the same `flock(2)` request as before. `EWOULDBLOCK`
  still becomes `io::ErrorKind::WouldBlock`, which `is_lock_contention` relies on.

| Metric | Before | After |
|---|---:|---:|
| `unsafe { … }` matches | 226 | **223** |
| Files containing them | 37 | **37** |
| `unsafe extern` / `unsafe fn` / `unsafe impl` lines | 58 | **58** |

The Windows `LockFileEx` and volume queries remain; `fs_ext` now has three blocks, all Windows-only. The
volume-space test proves `total > 0` and `available <= total`, so a zero fragment or block count fails, but
it does not independently prove the byte scale of `f_frsize`.

**Verification exercised:** `cargo check --lib`, `cargo clippy --lib -- -D warnings`, `fs_ext::` (2 tests,
including the volume-space and NUL assertions) and `library_root::` (35 tests) on macOS; the Android lib
check with `--features android-evaluation`. Not run: iOS/Linux cross-target checks, device tests,
sanitizer/Miri.

## Follow-up implemented 2026-11-19 (third batch)

`7b50d62763` removes four more blocks in two files:

- **Torrent publication rename (2 blocks).** `torrent::publication::move_publication_entry` was the last
  `renameatx_np`/`renameat2` site left out of the first batch. It now uses the same
  `rustix::fs::renameat_with(…, NOREPLACE)` call, and the file becomes unsafe-free.
- **Provider descriptor duplication (3 → 1 block).** `provider_reference`'s native `retain` duplicates the
  lease-owned descriptor with `rustix::io::fcntl_dupfd_cloexec` (the same `F_DUPFD_CLOEXEC` request), turns
  the resulting `OwnedFd` into a `File` without `from_raw_fd`, and probes seekability with `rustix::fs::tell`
  (the same `SEEK_CUR` check; rustix uses `lseek64` on Android, so large offsets are reported where the
  32-bit `libc::lseek` could not). One documented `BorrowedFd::borrow_raw` claim remains: the native
  lease owns the raw descriptor for the call and the duplicate is owned only by the new `File`.

| Metric | Before | After |
|---|---:|---:|
| `unsafe { … }` matches | 223 | **219** |
| Files containing them | 37 | **36** |
| `unsafe extern` / `unsafe fn` / `unsafe impl` lines | 58 | **58** |

**Verification exercised:** macOS `cargo check --lib`, `cargo clippy --lib -- -D warnings`,
`torrent::tests::publication` (16 tests) and `provider_reference` (9 tests). The provider `native` module is
`cfg(mobile)`, so the host build does not compile it; `cargo check --lib --target aarch64-linux-android
--features android-evaluation` compiles it. Not run: iOS/Linux cross-target checks, device tests,
sanitizer/Miri. The descriptor path itself was not exercised against a real document provider.

## SAFETY documentation added 2026-11-19

`dd2c6738f5` adds 25 `SAFETY:` comments to Apple native boundaries that previously had none. No code,
control flow or error text changed.

| File | Comments | Invariants now named |
|---|---:|---|
| `native_pdf/apple.rs` | 12 | exactly-once completion reclaims the `Box` from `into_raw`; authority hooks borrow only before completion; native copies input C strings synchronously; `print_start` false means no ownership; cancel of a finished token is a no-op |
| `notifications.rs` | 8 | `strdup` reply ownership and single free; callback context is the unique `Box<oneshot::Sender>` and native calls back exactly once; stored waker has a `'static` lifetime; installer is idempotent but must run at startup before concurrent readers |
| `share.rs` | 3 | token is an opaque integer, never dereferenced; path copied synchronously; completion message valid only for the callback |
| `native_pdf/handoff.rs` | 2 | installed function pointer is `'static` with no captures; token string is borrowed only for the synchronous lookup |

An audit with an eight-line lookback now finds **161 of 218** remaining code blocks without a `SAFETY:`
comment, occupying 160 of 217 code lines (`native_pdf/windows_print.rs:29` has two blocks on one line). The
earlier 201/238 figure used a six-line lookback on the pre-change tree. Largest remaining gaps:
`localswarm/apple_transport.rs` (22), `localswarm/ios_discovery.rs` (21), `agent_connector.rs` (16),
`native_pdf/linux.rs` (15), `private_sync_identity.rs` (13), `native_pdf/windows_print.rs` (12),
`nearby_route.rs` (11), `windows.rs` (10), `localswarm/library_route.rs` (9), `storage.rs` (7).

**Verification exercised:** `cargo check --lib`, `cargo clippy --lib -- -D warnings`, `notifications::`
(2 tests), `native_pdf` (22) and `share` (28) on macOS. Comment-only change; the callback paths themselves
were not exercised against a live share sheet, print panel or notification center.

## Final first-party pass — 2026-11-20

The merged branch plus this pass leaves **129 `unsafe` matches in 27 files** (136/30 before this pass).
This pass removed blocks that had a safe alternative available on this host:

- `sandbox::real_downloads_dir` uses `nix::unistd::User::from_uid` instead of a raw `getpwuid_r` record and
  buffer.
- The torrent publication regression creates its FIFO with `nix::unistd::mkfifo` instead of `libc::mkfifo`.
- `kiwix_pdf_cancel`, `kiwix_pdf_print_cancel` and `kiwix_share_cancel` are declared `safe fn` in their
  `unsafe extern` blocks: each native implementation only looks up an opaque integer ID and is a no-op once
  the request has finished. The calls no longer need an `unsafe` block; the ABI and native code are unchanged.
- `kiwix_mobile_bind_local_socket` is declared `safe fn`: it takes an integer descriptor, rejects invalid
  descriptors through checked socket calls, binds under its own lock and retains nothing. Pointer-taking
  bridges stay `unsafe` because a safe caller could pass a dangling or unterminated pointer.
- The iOS PDF scratch exclusion and three archive-policy imports now take a private `ffi::CPath` that borrows
  a live `CStr`. The native bodies read the path synchronously and retain no pointer, so the imports are
  `safe fn`; this removes four call-site blocks. Independent review confirmed the transparent ABI and lifetimes.
- The Apple PDF start paths, the iOS incoming URI check and the Android logcat tag/message also take `CPath`.
  Their native bodies copy or read the strings synchronously; independent review confirmed the ABI and lifetimes.
- `kiwix_swarm_read` is `safe fn`: it takes a static callback and lends dispatch data only for the call, and the
  Rust callback copies the bytes before returning. `start`, `write` and `listen` stay `unsafe` because their
  blocks can capture non-`Send` data and native invokes them on another queue.
- `WindowRef` retains and main-thread-checks the AppKit window; the share and print bridges borrow it and
  native takes its own reference. `webkit_backdrop` builds the appearance-name strings instead of reading
  AppKit's unsafe static pointers. `winapi-util::file::information` replaces the Windows file-identity call.
- The Windows lib now type-checks with mingw after removing the parent `forbid` that covered native print
  children and fixing the deletion path's spelling cancellation. It is a cross-check, not a Windows run.
- Windows audit fixes: ACE header/type/size/SID bounds before the typed borrow, explicit owner alongside the DACL,
  NUL-bounded printer names, dmSize-prefixed DEVMODEW reads, deferred print dialog after the WebView2 callback,
  UI-thread cancellation, documented NT rename length, COM-init failure propagation and absolute volume paths.
  The remote-share/mounted-folder capacity query and synchronous GTK file-printer enumeration remain open; the latter can delay the export deadline if a print backend never completes enumeration.

| Metric | Before | After |
|---|---:|---:|
| `unsafe { … }` matches (app + relay) | 136 | **127** |
| Files containing them | 30 | **23** |
| `unsafe impl` items | 2 | **2** |
| Code blocks without a `SAFETY:` comment (eight-line lookback) | 97 | **0** |

Every one of the 127 remaining code blocks now names its pointer, handle, lifetime or exactly-once
invariant. The audit was completed file by file against the native counterparts; the Windows lib now
cross-checks with mingw, while Linux comments remain source-reviewed but not compiled on this host.

The remaining matches are concentrated in the Windows, Linux and Apple native ABI boundaries, with a
small Android-only set and one prose `unsafe {}` in `lib.rs`. `storage.rs` spans the Windows and Apple
gates; `provider_reference.rs` has one block under the shared mobile gate. All are native ABI boundaries.
`acquisition::file_identity` now uses the already-locked `winapi-util::file::information`, and the Windows lib
type-checks with mingw. Windows runtime behavior, Linux compilation/runtime and physical devices remain
unverified.

`npm run check:unsafe` (also run by the Checks workflow) enforces this census: it fails on an undocumented
`unsafe` block or `unsafe impl`, or any per-file count above `scripts/unsafe-census-baseline.json`
(127 blocks in 23 code files plus 2 impls). The baseline is a ceiling: reductions pass and leave headroom, so
update it in the same change when reducing to keep the ceiling tight.

`#![forbid(unsafe_code)]` is now on 140 safe modules, including archive source and search, map
composition/manager/recipe, LocalSwarm protocol/network/library/import, acquisition HTTP metadata, updates,
reading data, catalog indexing, connector sockets, command dispatch and timestamps. New safe modules should
add the attribute; the census gate covers the remaining native-boundary modules.

The current build's paired reader comparison at the app's 32 MiB cache is recorded in
[the 2026-11-20 OSM runs](benchmarks/2026-11-20-osm-reader-current.md): open plus first article and the
24-article random batch were faster than libzim on all three archives, with sub-millisecond differences in
the two tiny-read phases. That is three macOS archives, not the full corpus or mobile targets.

Cross-target check: `cargo check --lib --target aarch64-apple-ios` passes under Rust 1.99.1 with
`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`. This compiles the iOS-only DNS-SD discovery,
Apple transport, provider-descriptor and mobile-backup modules. The host `xcode-select` points at
CommandLineTools, so the developer directory is required. This is a type check, not a device run.

### Thread-safety assertion sign-off

Only one type still carries a handwritten thread-safety assertion, `agent_connector::Descriptor` (`Send` and
`Sync`, two items). The native object is a `SECURITY_DESCRIPTOR` allocated once by
`ConvertStringSecurityDescriptorToSecurityDescriptorW`; the field is private to the Windows connector module,
no method mutates it after construction, `CreateNamedPipeW` reads it synchronously and copies what it needs,
and `Drop` releases it with `LocalFree`, which is safe from whichever thread drops the last owner. The
assertions therefore add no concurrent-mutation permission: there is no `&mut` path to the descriptor.
They are exercised by the Windows connector tests only, which were not run here.

The other six assertions recorded in the 2026-11-12 inventory (`archive::Handle`,
`localswarm::apple_transport::{RawConnection, RawListener}` and `power_activity::Token`) are gone: a
repository search finds no `unsafe impl` outside `agent_connector.rs`.
