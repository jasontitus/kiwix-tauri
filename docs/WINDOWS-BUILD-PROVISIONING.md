# Windows build provisioning

Use a native Windows 10/11 x64 machine or VM. WSL/Linux and Wine are not substitutes for the MSVC build.

## Install

From an elevated PowerShell, the repository script can install the prerequisites:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provision-windows.ps1 -Install
```

The equivalent manual commands are:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --accept-package-agreements --accept-source-agreements --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup --exact --accept-package-agreements --accept-source-agreements
winget install --id OpenJS.NodeJS.LTS --exact --accept-package-agreements --accept-source-agreements
winget install --id Git.Git --exact --accept-package-agreements --accept-source-agreements
rustup default stable
rustup target add x86_64-pc-windows-msvc
```

Open a **Developer PowerShell for VS 2022**, then verify:

```powershell
git clone https://github.com/jasontitus/kiwix-tauri C:\src\kiwix-tauri
cd C:\src\kiwix-tauri
powershell -ExecutionPolicy Bypass -File scripts/check-windows-build.ps1
```

The checker reports missing C++ tools, WebView2, Rust or Node. `provision-windows.ps1 -Install` installs VS Build Tools, Rustup, Node and Git; the checker remains read-only. Neither script has yet been executed on the Windows VM because it was left at its lock screen after the reference run.

## No libzim SDK

The app reads archives with its Rust reader, so the Windows build needs no libzim SDK and no `KIWIX_DESKTOP_SDK_ROOT`. The installer ships only the four pinned speech runtime DLLs and the agent connector. The patched libzim SDK described in `docs/PACKAGING.md` is used only by the native comparison tools and by earlier releases.

## Build and test

Use an LF checkout. `.gitattributes` pins LF, but a working tree checked out before that file existed keeps its CRLF files until they are checked out again, and the byte-exact tests then fail. `git ls-files --eol` shows them as `i/lf w/crlf`; only `.bat` and `.cmd` files should appear that way.

```powershell
npm ci
npm run build
node scripts/check-unsafe-census.mjs
node --test scripts/test-*.mjs
python -m unittest discover -s scripts -p 'test_*.py'
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --features custom-protocol --target x86_64-pc-windows-msvc -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml --features custom-protocol --target x86_64-pc-windows-msvc
cargo test --locked --manifest-path kiwix-mcp/Cargo.toml --target x86_64-pc-windows-msvc
```

- Run these one at a time. The app embeds `dist/` at compile time, and `npm run build` or `tauri build` empties it while it rebuilds, so a concurrent `cargo` run fails in `rust_embed`.
- The Rust test executables load the speech runtime DLLs from `src-tauri\target\x86_64-pc-windows-msvc\debug`; put that directory on `PATH`.
- The Python tests cover host tools for macOS and Linux. The ones that need process groups, POSIX signals or no-follow opens are skipped on Windows.
- `docs/windows-agent-tools/run-gate.ps1` runs all of the above and logs each step.

Build the installer with the commands in [`PACKAGING.md`](PACKAGING.md#guarded-windows-prerequisite-and-build). Then run the W1–W15 matrix in `docs/WINDOWS-TEST-PLAN.md` on a clean standard-user VM.

## Disk expansion already performed (2026-11-10)

- UTM/qcow2 virtual disk: **40 GiB → 80 GiB** (`qemu-img resize`; integrity check before resize was clean).
- Windows sees Disk 0 as 80 GB with 40 GB unallocated.
- `C:` remains 39 GB; the 702 MB recovery partition sits immediately after it, so `diskpart extend` correctly refuses to grow `C:` without deleting/moving recovery.
- Instead, a non-destructive **`W:` Work volume (40 GB NTFS)** was created in the unallocated space.
- Rust was installed under `W:\cargo` / `W:\rustup`; Node 22 was downloaded under `W:\tools`.
- Use `W:` for the Visual Studio install/cache, build tree and cargo target to avoid filling `C:`.

## Current status

A native Windows 11 24H2 machine with MSVC, Rust, Node and a patched libzim 9.8.2 SDK built and exercised the app; the report is in [`WINDOWS-TEST-REPORT.md`](WINDOWS-TEST-REPORT.md). The same machine built and tested the SDK-free app on 2026-11-20; that pass is the last section of the report. The local UTM reference VM is not used for build acceptance. The macOS host still cannot produce an MSVC artifact or replace native Windows runtime testing; its Windows GNU job remains a typecheck-only gate.
