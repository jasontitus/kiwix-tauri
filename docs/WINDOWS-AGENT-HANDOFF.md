# Windows agent handoff — build and acceptance

Date: 2026-11-10
Status: **completed on native Windows**. The build/runtime report and residual list are in [`WINDOWS-TEST-REPORT.md`](WINDOWS-TEST-REPORT.md); this handoff is retained for reproduction and evidence. It records the native-SDK baseline: the app now reads archives with its Rust reader, so the libzim SDK and `KIWIX_DESKTOP_SDK_ROOT` steps below no longer apply. Use the current [build and test commands](WINDOWS-BUILD-PROVISIONING.md#build-and-test).

Repository: `https://github.com/jasontitus/kiwix-tauri`
Checkout: `main` at the current public commit (record the exact commit; this handoff is historical)
Goal: build our Windows app natively, run W1–W15 against the official Qt 2.5.1 reference, and return evidence. Do not modify source on the checkout; report findings.

## 1. Required environment

Native Windows 10/11 x64 only. WSL and Wine are not substitutes.

- Visual Studio 2022 Build Tools with Desktop C++ (MSVC v143), Windows 11 SDK, CMake/Ninja.
- Rust stable via rustup, target `x86_64-pc-windows-msvc`.
- Node 22 LTS + npm.
- Git for Windows.
- Evergreen WebView2 runtime.
- Verified schema-2 Windows libzim **9.8.2** SDK at commit `26ec526f74e8342a40da5ab14e364546988e0e1a`, Xapian enabled, with `kiwix-split-ranges-v1` and `kiwix-retained-fd-v1` patches, all DLLs/headers/import lib and `kiwix-sdk.json` SHA-256 manifest.

This manifest contract was checked at build time by `src-tauri/native/desktop_sdk.rs`, which was removed once the app stopped linking libzim. No current app build step or workflow reads or verifies a Windows libzim SDK or its `kiwix-sdk.json`; `docs/windows-agent-tools/assemble-sdk.py` writes one for the native comparison tools only. Do not invent hashes.

Current builds need no SDK (see [WINDOWS-BUILD-PROVISIONING.md](WINDOWS-BUILD-PROVISIONING.md#build-and-test)); for this historical baseline, if the machine lacks the SDK, stop after the compile-only gate and report the exact missing prerequisite; do not substitute stock libzim.

## 2. Build and local tests

From a Developer PowerShell for VS 2022:

```powershell
git clone https://github.com/jasontitus/kiwix-tauri C:\src\kiwix-tauri
cd C:\src\kiwix-tauri
git checkout main
git log --oneline -1
npm ci
npm run build
$env:KIWIX_DESKTOP_SDK_ROOT = 'C:\src\kiwix-sdk'
$env:PATH = "$env:KIWIX_DESKTOP_SDK_ROOT\bin;$env:PATH"
cargo check --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc 2>&1 | Tee-Object test-windows.log
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --features custom-protocol --target x86_64-pc-windows-msvc -- -D warnings
npm run tauri -- build --target x86_64-pc-windows-msvc --bundles nsis
```

Expected artifact: `src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\*.exe` (unsigned unless a signing identity is supplied). Record tool versions (`rustc -V`, `node -v`, `cl`, `winver`).

## 3. Reference build

Official ZIP: `kiwix-desktop_windows_x64_2.5.1.zip`
SHA-256: `56ad5b3a86010c2c74db63a717d087cfbdcdbe71a0d7fca0655f90ef1d80593f`
Size: 157,265,310 bytes; extracted 137 files / 328,844 KiB.

Reference evidence already observed on a native Windows 11 VM: official 2.5.1 launched and UI Automation exposed a `Go by Example - Kiwix` window/article for Zimit 3; toolbar included Back, Forward, Random Article, Home page, Search Options, Open file, Set fullscreen; three tab items. No usable visual capture was retained. Use the official build only as a behavior reference; do not count Wine results.

## 4. Windows acceptance matrix

Install the unsigned NSIS as a normal user on a clean standard-user VM. Run:

| ID | Check | Expected / evidence |
|---|---|---|
| W1 | Install, first launch, second launch | App starts; second launch focuses/restores existing window; one process tree |
| W2 | Open `.zim` by double-click and path | Existing instance opens archive; no blank window |
| W3 | 10+ `.zim` burst | All import/open; no error boxes |
| W4 | Reader TOC/find/links/back-forward | Matches macOS/Linux; no WebView2 navigation errors |
| W5 | Full-text + title suggestions | Snippets, paging, cancellation, selected-book scope |
| W6 | H.264/AAC and WebM | Playback/fullscreen/seeking; record codec limits |
| W7 | PDF save and system print | Native provider opens save path and Windows print UI; no hidden PDF window |
| W8 | HTTP + BitTorrent download; pause/resume/retry | Completion, verification, speed/ETA, no orphan jobs |
| W9 | LocalSwarm share; join from Android/iPhone | Library index/archive fetch; firewall prompts explained |
| W10 | Storage free/total; move/relink | Matches Explorer; survives restart |
| W11 | Profile lock with two `--data-dir` instances | Second reports busy; no corruption |
| W12 | Read-only/private; update check | Export/share blocked with clear copy; empty updater endpoint performs no download |
| W13 | Keyboard shortcuts + 100/150/200% DPI | Accelerators reach reader; no clipped controls |
| W14 | Sleep/resume + network loss during download | Clear recovery/error; reader returns |
| W15 | Uninstall/reinstall + per-user data | Library survives as designed; no stale lock |

Evidence: screenshots for UI rows, logs for failures, and for each failure the same check on `origin/main` where feasible.

## 5. LAN fixtures and device exchange

A Mac on the same LAN is serving fixtures at:

```
http://<mac-lan-ip>:8799/
```

Files: `walk-test.zim` (9 MB; 180 s audio, 120 s H.264 video, 30 s VP8/VP9), `media-test.zim` (1.3 MB), `hawaii.zim` (144 MB StreetZim), `README.txt`.
Verify with `curl.exe -I http://<mac-lan-ip>:8799/walk-test.zim`; ask the owner for the Mac's current LAN IP.

Current device state at handoff: the Samsung SM-A356U (`<samsung-serial>`) is actively sharing **12 archives** over Nearby/LAN and is kept awake. The iPhone 17 has the media/Hawaii fixtures staged and is waiting for a physical import tap; start Nearby on it after import if direct iPhone exchange is needed. Android devices are attached to the Mac, not the Windows machine. Exchange options, in order:

1. **HTTP:** download fixtures from the Mac URL; for returning the NSIS artifact, run `python -m http.server 8800 --bind 0.0.0.0 --directory <artifact-dir>` on Windows and give the Mac/Android/iPhone the `http://<windows-ip>:8800/...` URL.
2. **LocalSwarm/Nearby:** on Android/iPhone open Kiwix → Nearby → Share all (or share a selected archive); on Windows open the built app → Nearby, discover and join. Verify a small (1.3 MB) and a large (144 MB) transfer. If the devices are on a shared Wi-Fi subnet, no hotspot is needed; iPhone/Android app-owned AP paths are documented in `docs/LOCALSWARM.md`.
3. **ADB from the Mac:** if direct Android inspection is needed, ask the Mac owner to run `adb` commands; do not assume USB access from Windows.

Do not delete the shared fixture directory or device archives. Report transfer results and any firewall/interface failures.

## 6. Return

Return:
- exact commit, Windows edition/build, toolchain/SDK provenance;
- W1–W15 pass/fail/not run with evidence;
- official reference observations;
- Windows artifact path/hash;
- device-exchange results and any failures;
- blockers, including whether a clean-machine libzim/WebView2 closure was required.

Do not push source changes from the Windows agent; findings are fixed separately with independent review.
