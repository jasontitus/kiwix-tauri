# Windows build-and-test brief for kiwix-tauri

Paste this file into the session that can reach the Windows machine, or have it read `docs/WINDOWS-TEST-BRIEF.md` after checking out the branch.

## Goal

Build and test branch **`deps/trim-direct-dependencies`** of `github.com/jasontitus/kiwix-tauri` (commit `edbd0f2` or later) **natively on Windows**. Nothing in this repository has been compiled with MSVC or run on Windows in this round.

**Do not build inside WSL.** A WSL build is a Linux build, and the code under test only compiles in a Windows build: the single-instance plugin, window focus and the hidden PDF-render window. Use PowerShell on the Windows side. You can reach it over ssh from WSL with `powershell.exe -Command ...`, or ssh directly into Windows.

Clone to a Windows path such as `C:\src\kiwix-tauri`, not `\\wsl$\...`. Builds on the WSL share are slow and can hit permission problems.

## Prerequisites (Windows side)

- Visual Studio 2022 Build Tools with "Desktop development with C++", including the MSVC v143 tools and a Windows 10/11 SDK.
- Rust stable, via rustup: `rustup target add x86_64-pc-windows-msvc`.
- Node 22 and Git for Windows.
- The WebView2 runtime. It ships with Windows 11 and current Windows 10.

Record the versions: `rustc -V`, `node -v`, `cl` (from a "Developer PowerShell for VS 2022"), and `winver`.

## Phase 1: type-check and lint (no libzim needed, about 15 minutes)

Run this from a Developer PowerShell for VS 2022:

```powershell
git clone https://github.com/jasontitus/kiwix-tauri C:\src\kiwix-tauri
cd C:\src\kiwix-tauri
git checkout deps/trim-direct-dependencies
git log --oneline -1
npm ci
npm run build                      # creates dist/, which the crate embeds
$env:KIWIX_TYPECHECK_ONLY = '1'    # skips libzim; any real link fails on purpose
cargo check  --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc -- -D warnings
```

**Expected:**
- `cargo check` passes.
- On the `deps/trim-direct-dependencies` checkout above, `clippy` fails with **13 errors that existed on `main` before #6**, so they are not regressions:
  - unused import `File` in `src/spelling.rs`
  - "variable does not need to be mutable" at nine `let mut builder` sites
  - `forward_launch` is never used
  - `new_with_claimed_lock` is never used, twice
- On a checkout that includes jasontitus/kiwix-tauri#6, those 13 are fixed and
  `clippy` is expected to pass. The `windows-typecheck` job in
  `.github/workflows/checks.yml` verifies that only for the
  `x86_64-pc-windows-gnu` target; this MSVC run has not been observed.
- A warning in the vendored `librqbit-dualstack-sockets` (`unused variable: name`
  in `bind_device.rs`) is known and does not fail the run.
- Report any other error or warning in full. Something new is a finding.

Optionally, if `cargo-deny` is installed:

```powershell
cd src-tauri
cargo deny --all-features --target x86_64-pc-windows-msvc check
```

It should report `advisories ok, bans ok, licenses ok, sources ok`. One warning is expected: `advisory-not-detected` for RUSTSEC-2024-0429, because glib is Linux-only.

## Phase 2: the Windows libzim SDK (needed for anything that links)

The build needs an MSVC build of **libzim 9.8.2 at commit `26ec526f74e8342a40da5ab14e364546988e0e1a`** with full-text search (Xapian), release mode, x86_64. It must include:
- the two patches in `src-tauri/native/`: `libzim-split-ranges.patch` and `libzim-retained-fd.patch`;
- all transitive DLLs (zstd, xz/lzma, ICU, Xapian, and so on);
- a schema-2 `kiwix-sdk.json` manifest.

The exact manifest contract is in `docs/PACKAGING.md`, section "Guarded Windows prerequisite and build". `src-tauri/native/desktop_sdk.rs` enforces it at build time. **There is no script for this in the repository.**

- **If the machine already has such an SDK:** set `LIBZIM_SDK_ROOT` to it and continue.
- **If not:** building one is the expensive, uncertain step. Expect hours: meson plus MSVC, and libzim's dependencies.
  - For a **local test build only**, it is fine to build it yourself. Write the manifest from SHA-256 hashes of the files you actually installed, and say in your report that it is self-built and not release provenance.
  - Never invent hashes, and never relax the checks in `desktop_sdk.rs`.
  - If this would take more than a reasonable effort, **stop after Phase 1 and report back**. Phase 1 is still valuable on its own.

## Phase 3: Rust tests on Windows (needs the SDK)

This suite has never run on Windows, so failures may predate this branch.

```powershell
$sdk = (Resolve-Path $env:LIBZIM_SDK_ROOT).Path
$env:KIWIX_DESKTOP_SDK_ROOT = $sdk
$env:LIBZIM_INCLUDE_DIR = Join-Path $sdk 'include'
$env:LIBZIM_LIB_DIR = Join-Path $sdk 'lib'
$env:PATH = (Join-Path $sdk 'bin') + ';' + $env:PATH
Remove-Item Env:KIWIX_TYPECHECK_ONLY -ErrorAction SilentlyContinue
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc 2>&1 | Tee-Object test-branch.log
```

For **every failing test**, run the same test on `origin/main` to classify it as new or existing:

```powershell
git worktree add ..\kt-main origin/main
cd ..\kt-main
npm ci
npm run build
cargo test ... -- <test name>
```

These tests matter most for this branch:
- `fs_ext`: file locks and free space, which now use our own LockFileEx and GetDiskFreeSpaceExW code instead of `fs2`.
- `store`: the profile lock.
- `torrent`: the pinned SHA-1 info hash.
- `native_pdf`, `exports` and `search`: the HTML/CSS parser port.
- `launch_target_tests`.

The test `bridge_audio_captures_prepare_with_the_real_pdf_sanitizer` needs `node_modules`, which `npm ci` provides.

## Phase 4: build the installer and check behaviour by hand (needs the SDK)

Follow the PowerShell block in `docs/PACKAGING.md`, under "For a local Windows build". It produces `src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\*.exe`, which is unsigned. Install it as a normal user, then check:

1. **Relaunch focuses the window.**
   - Start Kiwix and click into another app.
   - Start Kiwix again from the Start menu.
   - Expected: the existing window comes to the front. The second process exits; Task Manager shows one Kiwix process tree.
2. **Relaunch restores a minimized window.**
   - Minimize Kiwix, then relaunch it.
   - Expected: the window is restored and focused.
3. **Opening a file goes to the running instance.**
   - With Kiwix running, double-click a `.zim`, or run `"C:\...\Kiwix.exe" C:\path\file.zim`.
   - Expected: it opens in the existing window, which comes forward.
4. **The hidden PDF window never appears** (this is the bug the fix in `edbd0f2` addresses).
   - Open a second reader window, then close the main window.
   - Start a PDF export of an article.
   - While the save dialog is open, launch Kiwix from the Start menu.
   - Expected: no blank or odd window appears, and the remaining reader window comes forward.
5. **Bursts of files.**
   - Select 10 or more `.zim` files in Explorer and press Enter.
   - Expected: all of them open and no error boxes appear.
6. **Dependency swaps on Windows.**
   - **Settings, sharing:** the network interface list appears. It now comes from `if-addrs`. On Windows that crate leaves out IPv6 addresses that are not "preferred", for example deprecated temporary ones. That is expected.
   - **Profile lock:** start two instances with the same `--data-dir`. The second must report that the profile is busy and must not corrupt anything.
   - **Free space:** the storage settings show a correct free and total size for the library drive. Compare with Explorer.
   - **Downloads:** download a small ZIM over HTTP and one over BitTorrent. Both must complete and verify.
   - **Exports:** PDF-export an article that has SVG images and math. MHTML/HTML-export an article that uses stylesheets.
   - **Search:** full-text search snippets and article previews show text.

## Report back

Report the following:
- the commit tested;
- the Windows build and edition;
- toolchain versions;
- for each phase and each numbered check: pass, fail or not run, with evidence (log excerpts, plus screenshots for Phase 4);
- for every failure, whether `origin/main` fails the same way;
- whether the SDK was pre-existing or self-built.

Don't change code on the branch. Report findings, and they will be fixed with an independent review, as `AGENTS.md` requires.
