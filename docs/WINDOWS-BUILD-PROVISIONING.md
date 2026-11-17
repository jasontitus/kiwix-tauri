# Windows build provisioning

Use a native Windows 10/11 x64 machine or VM. WSL/Linux and Wine are not substitutes for the MSVC build.

## Install

From an elevated PowerShell, the repository script can install the non-SDK prerequisites:

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

The checker reports missing C++ tools, WebView2, Rust/Node, or the verified SDK. `provision-windows.ps1 -Install` installs VS Build Tools, Rustup, Node and Git; the checker remains read-only. Neither script has yet been executed on the Windows VM because it was left at its lock screen after the reference run.

## Libzim SDK

The build intentionally refuses a stock or unverified libzim. Provision the schema-2 Windows SDK described in `docs/PACKAGING.md`:

- libzim `9.8.2`, commit `26ec526f74e8342a40da5ab14e364546988e0e1a`;
- full-text Xapian enabled;
- `libzim-split-ranges.patch` and `libzim-retained-fd.patch` applied;
- all transitive DLLs, headers, import library and SHA-256 manifest present.

Set `KIWIX_DESKTOP_SDK_ROOT` to that directory. Do not invent hashes or relax `src-tauri/native/desktop_sdk.rs`.

## Build

```powershell
npm ci
npm run build
$env:KIWIX_DESKTOP_SDK_ROOT = 'C:\src\kiwix-sdk'
cargo check --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc
npm run tauri -- build --target x86_64-pc-windows-msvc --bundles nsis
```

Then run the W1–W15 matrix in `docs/WINDOWS-TEST-PLAN.md` on a clean standard-user VM.

## Disk expansion already performed (2026-11-10)

- UTM/qcow2 virtual disk: **40 GiB → 80 GiB** (`qemu-img resize`; integrity check before resize was clean).
- Windows sees Disk 0 as 80 GB with 40 GB unallocated.
- `C:` remains 39 GB; the 702 MB recovery partition sits immediately after it, so `diskpart extend` correctly refuses to grow `C:` without deleting/moving recovery.
- Instead, a non-destructive **`W:` Work volume (40 GB NTFS)** was created in the unallocated space.
- Rust was installed under `W:\cargo` / `W:\rustup`; Node 22 was downloaded under `W:\tools`.
- Use `W:` for the SDK, Visual Studio install/cache, build tree and cargo target to avoid filling `C:`.

## Current status

A native Windows 11 24H2 machine with MSVC, Rust, Node and a patched libzim 9.8.2 SDK built and exercised the app; the report is in [`WINDOWS-TEST-REPORT.md`](WINDOWS-TEST-REPORT.md). The local UTM reference VM is not used for build acceptance. The macOS host still cannot produce an MSVC artifact or replace native Windows runtime testing; its Windows GNU job remains a typecheck-only gate.
