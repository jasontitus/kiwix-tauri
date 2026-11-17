# Windows test plan (2026-11-09)

Goal: verify the current `kiwix-tauri` Windows build against the pinned Qt `kiwix-desktop` 2.5.1 reference and the feature contract in `docs/FEATURES.md`.

Two separate activities, with different evidentiary value:

1. **Wine preflight on this Mac** — reference behavior and UI inventory only. It cannot certify our Windows app.
2. **Native Windows run** — the only run that can accept the Windows build.

## Phase 0 — Wine preflight (reference only)

Wine 10.6 (Staging) is installed and initializes a prefix on this machine. The upstream Qt build for Windows is the right subject for Wine because it bundles Qt WebEngine; our Tauri build does not bundle a web engine and requires Microsoft's WebView2 runtime.

Attempt recorded 2026-11-09: the official ZIP is at `~/Downloads/kiwix-desktop_windows_x64_2.5.1.zip` (SHA-256 `56ad5b3a86010c2c74db63a717d087cfbdcdbe71a0d7fca0655f90ef1d80593f`, 157,265,310 bytes). Extracted: 137 files and 328,844 KiB; it bundles Qt6 WebEngine (`Qt6WebEngineCore.dll`, 133,659,280 bytes), Qt6, aria2c and the VC runtime.

macOS Wine 10.6: the official `--help`/`--version` path exposes the Qt help dialog, but launching the library or opening any ZIM—including a clean start with no archive—ends in Wine's **Program Error** dialog. The retained evidence is that dialog, not a Wine backtrace; it does not establish the app's native behavior. A Linux Wine 8 + Xvfb fallback (amd64 under QEMU) aborted with an x86 AVX-state assertion in the same exploratory run.

Native Windows 11 VM (UTM): the official 2.5.1 build launched. UI Automation exposed a `Go by Example - Kiwix` window and article text; it included toolbar controls `Back`, `Forward`, `Random Article`, `Home page`, `Search Options`, `Open file`, `Set fullscreen`, and a three-item tab bar. UI Automation also exposed content from the current Zimit 3 `gobyexample.com` archive, showing the reference reader opened and parsed it. No usable visual capture was retained, so painted rendering is not claimed. The StreetZim Hawaii archive was added to the reference library/session, but its rendered view was not confirmed; no reference PDF/print/download test was completed.

Commands:

```sh
mkdir -p /tmp/kiwix-win && cd /tmp/kiwix-win
curl -fL -o kiwix.zip https://download.kiwix.org/release/kiwix-desktop/kiwix-desktop_windows_x64_2.5.1.zip
unzip -q kiwix.zip
find . -maxdepth 2 -iname '*.exe'
WINEDEBUG=-all wine kiwix-desktop.exe 2>&1 | tee wine.log
```

What to record if it launches:
- whether the main window, library, and reader render (Qt WebEngine under Wine is the likely failure point);
- search, bookmarks, downloads, PDF export, and file-open behavior;
- Qt/Chromium errors in `wine.log` (missing DLL, sandbox, GPU, WebEngine process crash).

What Wine cannot establish, even when the Qt app runs:
- our Tauri/WebView2 rendering, PDF, printing, and single-instance behavior;
- NSIS installer, file associations, shell integration, taskbar, DPI, and Windows 10/11 differences;
- WebView2 availability/version behavior.

Do not mark any Windows acceptance row passed from Wine alone.

## Phase 1 — native Windows: compile-only gate

Use `docs/WINDOWS-BUILD-PROVISIONING.md` and `scripts/check-windows-build.ps1` for prerequisite setup, and `docs/WINDOWS-TEST-BRIEF.md` for the MSVC commands, but test the current `main` commit rather than the older `deps/trim-direct-dependencies` checkout named there. Record `rustc -V`, `node -v`, `cl`, and `winver`.

From a Developer PowerShell for VS 2022, with `KIWIX_TYPECHECK_ONLY` set so the intentionally absent libzim SDK is not linked:

```powershell
npm ci
npm run build
$env:KIWIX_TYPECHECK_ONLY = '1'
cargo check --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol --target x86_64-pc-windows-msvc -- -D warnings
```

Optionally run `cargo deny` from `src-tauri`. Remove `KIWIX_TYPECHECK_ONLY` before any linked test or build.

Stop and report after this phase if the MSVC libzim SDK cannot be produced within the agreed effort.

## Phase 2 — native Windows: build and test matrix

Build the unsigned NSIS installer, install as a normal user, then run these checks. Branch/version: record the exact commit under test.

| ID | Check | Expected / evidence |
|---|---|---|
| W1 | Install, first launch, second launch | App starts; second launch focuses/restores the existing window; one process tree |
| W2 | Open `.zim` by double-click and by passing a path | Existing instance opens the archive; no blank window; correct reading-list/library state |
| W3 | Single-instance bursts (10+ `.zim` files) | All import/open; no error boxes; focus stays on the visible window |
| W4 | Reader: open article, TOC, find, links, back/forward | Matches the Linux/macOS contract; no WebView2 navigation errors |
| W5 | Search: full text and title suggestions | Snippets, paging, cancellation, selected-book scope |
| W6 | Media: H.264/AAC MP4 and WebM in a real archive | Playback, fullscreen, seeking; record codec limits |
| W7 | PDF save and system print | Native provider opens the save path and the Windows print UI; no hidden PDF window appears |
| W8 | Download over HTTP and BitTorrent; pause/resume/retry | Completion, verification, speed/ETA, no orphan jobs |
| W9 | LocalSwarm/share: start sharing, join from another device | Library index and archive fetch; firewall prompts explained |
| W10 | Storage: free/total on the library drive; move/relink | Values match Explorer; move survives restart |
| W11 | Profile lock with two instances using the same `--data-dir` | Second reports busy; no corruption |
| W12 | Read-only and private modes; update check | Export/share blocked with clear messages; private transition clears reader state. The manual update check must report that this checked-in build has no configured installable update endpoint (empty `tauri.conf.json` updater values) and must not download or restart |
| W13 | Keyboard shortcuts and DPI scaling (100/150/200%) | Accelerators reach the reader; no clipped controls |
| W14 | Sleep/resume and network loss during download | Download recovers or errors clearly; reader returns |
| W15 | Uninstall/reinstall and per-user data | Library survives as designed; no stale lock |

Evidence: screenshots for every UI row, logs for failures, and for each failure the same check on `origin/main` where feasible.

## Report

Return: commit, Windows edition/build, toolchain versions, Wine result (or explicitly "not run"), each W-row pass/fail/not run, and any new findings. Nothing here authorizes changing code during the Windows run; findings are fixed in a separate change.


## Reference comparison recorded 2026-11-09

| Dimension | Official kiwix-desktop 2.5.1 Windows | Our Windows target |
|---|---|---|
| Package | 137 files, 328,844 KiB extracted; bundled Qt6WebEngine/Qt/aria2c and the VC++ redistributable installer | Expected NSIS + system WebView2 + patched libzim DLLs; no Windows artifact exists in this checkout |
| Web engine | Qt6 WebEngine (bundled) | Evergreen WebView2 146.x present in the test VM |
| CLI | `-?, -h, --help`, `--help-all`, `-v, --version`, optional `zimfile` | `--article`, `--data-dir`, `--private`, `--read-only`, `--headless`, `--port`, `--version`, multiple archives |
| UI observed | Qt toolbar: Back, Forward, Random Article, Home page, Search Options, Open file, Fullscreen; three tabs | Tauri shell with Library/Reader/Downloads/Maps/Nearby/Settings, reader tabs, sessions, restricted reading |
| ZIM observed | Opened Zimit 3 `gobyexample.com`; UI Automation exposed article content; no visual capture | Same Zimit shape is source/HTTP-test readable through Wombat; packaged WebView2 not verified |
| Download engine | aria2c child process | Native Rust HTTP/BitTorrent |
| Windows acceptance | Native VM launch and one ZIM opened/UI-Automation-exposed only | No MSVC build; the `KIWIX_TYPECHECK_ONLY` cross-check failed in `aws-lc-sys` with `windows.h` not found (Windows SDK/toolchain absent; no retained log file); `.native` contains macOS and Linux SDK manifests but no Windows SDK manifest |

The Windows VM has a WebView2 installation. It does **not** have Rust, Cargo, MSVC, or a Windows libzim SDK, so our app cannot be built there; exercising it would require a prebuilt Windows artifact, which this checkout does not contain. A native Windows runner with the MSVC libzim SDK remains required.
