# kiwix-tauri — native Windows build & test report (2026-11-10)

Base: `3eb971a` (main). Work branch: `fix/windows-confined-rename-and-lock`.

## Environment
- Windows 11 Pro 24H2, build 26100, x64.
- MSVC 19.44, from VS 2022 Build Tools at `E:\BuildTools2022`. Windows SDK 10.0.26100.
- Rust 1.99.1, stable `x86_64-pc-windows-msvc`, in `E:\dev\rustup` and `E:\dev\cargo`. Node 22.22.0 in `E:\dev\nodejs22`. WebView2 Evergreen 157.
- Builds and tests ran in an LF-only worktree (`E:\kiwix-win\lf-check`). The system git sets `core.autocrlf=true`, which breaks byte-exact tests; a new `.gitattributes` now pins LF.
- Everything lives on `E:` (toolchains, caches, SDKs, `TEMP`, the app's data dir and library).

## libzim SDK: self-built
- Location: `E:\kiwix-win\sdk-next\x86_64-pc-windows-msvc`. It was built with kiwix-build and meson and is **not** release provenance.
- Contents: libzim 9.8.2 at `26ec526f`, plus the repo's split-ranges patch and the **updated** `libzim-retained-fd.patch` (SHA-256 `cefe049c…fdb17f80`).
- Dependencies: zstd, zlib, Xapian 1.4.23 (static), ICU 74.1, xz.
- **Every existing SDK must be rebuilt, on all platforms.** The patch hash changed, and SDK manifests pin it.
  - Linux (WSL Ubuntu 24.04): `scripts/build-desktop-sdk.py` builds with the final patch, and libzim's tests pass 31/31.
  - Android and iOS: their patch stacks apply with zero fuzz.
  - macOS, Android and iOS were not built here.

## Gate (final branch state, `6953ae0`, re-run after the last change)
- `cargo clippy --all-targets --features custom-protocol --target x86_64-pc-windows-msvc -- -D warnings`: clean. The only warning is in vendored `librqbit-dualstack-sockets`.
- `cargo test --lib`, same target: **1009 passed, 0 failed, 9 ignored**. Before the fixes: 800 passed, 191 failed.
- JS: **315/315**, plus `tsc --noEmit` clean.
- libzim on Windows: 36/36.

## Fixed (on the branch)
| # | Was | Fix |
|---|---|---|
| F1 | The retained-fd patch did not compile with shared MSVC (C2280: dllexport of `FileCompound`) | Export only the out-of-line members; out-of-line destructor |
| F2 | **Every confined rename failed with os error 87.** `FileRenameInfo` + `RootDirectory` is invalid in Win32, so no download, map, torrent or library move could publish | `storage::rename_into_directory_noreplace` calls `NtSetInformationFile(FileRenameInformation)`; verified with a C probe |
| F3 | A busy profile (error 33) was reported as "NFS/SMB lock failure" | `fs_ext::is_lock_contention` |
| F4 | An empty stray part (`.zimac`) failed the import | Name discovery stops at an empty part; explicit part sets are unaffected |
| F5 | `\\?\` include paths broke `cl.exe` (C1083) | `tool_path` strips the prefix when the path is Win32-safe |
| F6/F7 | Lib tests did not compile or start on Windows (POSIX-only APIs; comctl32 v6 entry point) | Windows twins or cfg gates; `build.rs` embeds `windows-app-manifest.xml` |
| F8 | ~40 sharing violations (os error 32/5/33) across acquisition, LocalSwarm, torrent, maps, publication, library files and cleanup: POSIX unlink-while-open assumptions | Identity-bound release-then-act patterns; shared `remove_empty_directory_verified` (POSIX-semantics delete, with a legacy fallback); bounded retry on transient 5/32 |
| F9 | clippy `-D warnings` failed on MSVC | Fixed |
| F12 | **Deleting a book failed with os error 32** when it was open anywhere: a reader, the cache, a spelling build, or a whole-library web share. (Rolled back safely.) | On Windows, removal releases each holder first and refuses while an integrity check is running. Library moves raise an "archives are being moved" gate and refuse while sharing or checking. Raw 32/5 errors are explained |
| F13 | PDF/MHTML export failed when a page stylesheet was unreadable (SecurityError) | That sheet is skipped |
| F14 | **PDF/MHTML export failed on every DevDocs article**: "A custom rendered surface has no readable shadow DOM", caused by an empty, never-registered `<devdocs-navbar>`. Platform-independent | A custom element that is undefined, empty and unrendered is left out; anything that could hide content still fails closed |
| F15 | Paths showed as `\\?\E:\…` in the UI | `displayPath` (TS) and `display_path` (Rust) |
| F17 | **"Move previous downloads" never moved anything on Windows.** Library records store `\\?\E:\…` paths; the settings directory is `E:\…`; `Path::starts_with` treats the two prefixes as different | `move_archives` also matches the canonical (verbatim) spelling of the source directory (`library_files.rs`). The other path-prefix checks were audited: `is_managed_path` falls back to a marker walk, and the remaining check is mobile-only |
| F16 | **Closing the native Print dialog aborted the app** (see the runtime table). **Verified fixed at runtime** with the runtime change alone: the dialog closes, the app keeps running, no crash record | Vendored `tauri-runtime-wry`: the removed window is dropped only after the `windows` borrow ends (`Destroyed`; `on_window_close` too, defensively). App: the final `Files` (PDF scratch) drop is moved off the UI thread / WebView2 callbacks (`OffUiFiles`) |
| — | Tests sent real entries to the Recycle Bin | Under `cfg(test)`, `delete_book_impl` refuses the real Trash |

Every change passed at least one independent adversarial review round (six rounds in total). The last rounds found only best-effort races that fail safely (one book's move fails and rolls back).

## Runtime results (installed NSIS build, data dir `E:\kiwix-win\appdata`, driven over WebView2 CDP)
| Area | Result |
|---|---|
| Launch, single-instance forwarding, 12-file burst, reader/TOC/find, title and full-text search, H.264/VP8/VP9 playback, storage free/total, updater, uninstall/reinstall | **Pass** |
| HTTPS download → library | **Pass** (failed with os error 87 before F2) |
| Delete a book to the Recycle Bin while it is open in a tab | **Pass**: lands as `kiwix-deleted-…` (failed with os error 32 before F12) |
| Delete a book while the web share is serving it | **Pass**: the share then answers 410 Gone |
| BitTorrent ZIM via URL (`devdocs_en_eslint….zim.torrent`) | **Pass**: done in about 16 s, opens |
| BitTorrent ZIM via the Discover catalog button (Sanctuary Type Classes) | **Pass** |
| BitTorrent **map** (StreetZim `recipes/bahrain/en.torrent`, 50.8 MB) | **No progress (0%).** The torrent's only source is an HTTP web seed (`url-list`). The web seed works (HTTP 206), but librqbit 9.0.1 has **no web-seed support**, and the swarm has no peers. Not Windows-specific. The Maps page itself downloads maps over HTTP only |
| Map download over HTTP from the Maps page (Bahrain, English, 50.8 MB) | **Pass**: completed in about 10 s, opens and renders offline, with search over 36,679 places |
| Settings → change download directory → "Move previous downloads" | **Failed, then fixed and runtime-verified (F17)**. Before: "0 archives moved, 19 left unchanged" although 4 app downloads were in the old folder. After: "4 archives moved, 16 left unchanged", including the map that was open in a reader tab. The files are in the new folder and the app shows the new paths |
| Nearby / LocalSwarm: receive from Android | **Pass**: device discovered with 12 archives. 60 kB and 201 MB archives received; the 201 MB one took under 10 s, opens and renders |
| Nearby: "Share all" from Windows | Starts and advertises "Windows · 18 archives". Remote peers were initially blocked by Windows Firewall (see below) |
| Nearby: **iPhone downloads from Windows** (after the firewall was allowed) | **Pass**: the iPhone found "Windows", browsed it, and downloaded two small archives; the PC reports 669.9 kB sent. A first attempt failed on the phone with "The selected Nearby route changed; browse again". That is iOS-side route pinning (`localswarm/library_route.rs:110`): the phone's status bar showed 5G and no Wi-Fi, so it changed networks between browsing and downloading. The PC saw no connection |
| Web sharing (HTTP/OPDS) | **Pass** on this PC: catalog lists 18 archives; downloads are byte-identical (SHA-256), ranges return 206 |
| iPhone, Mac | Not discoverable: neither is advertising a Kiwix share. The Mac answers ping. Needs someone at those devices to open Nearby → Share all |
| PDF export ("Save article as PDF") on a DevDocs article | **Fixed by F14**: now reaches the native Save As dialog. Filling that dialog could not be automated from this session; it hangs UI Automation |
| Print ("Print / save as PDF") | The native Print dialog opens. **Closing it (WM_CLOSE, the same as the X button) crashed the app**: fail-fast 0xc0000409. The crash record says `RefCell already mutably borrowed` at vendored `tauri-runtime-wry-2.12.3/src/lib.rs:3643` (`windows.0.borrow()`), then "panic in a function that cannot unwind". Likely cause: `TaoWindowEvent::Destroyed` (`lib.rs:4257`) drops the removed `WindowWrapper`, and with it the hidden PDF view's WebView2, while the `RefMut` temporary is still alive. Teardown callbacks then re-enter the runtime on the main thread. The cancel path's `NativeJob::drop` destroys that hidden window |

## Needs the user (not changed by the agent)
- **Windows Firewall:** `kiwix-workbench.exe` originally had inbound **Block** rules on the Public profile (created when the first-run prompt went unanswered), and this Ethernet network is classified as **Public**. **The owner later allowed it**: the rules are now inbound Allow for TCP and UDP. Downloads *from* this PC by another device were still not exercised. A WSL probe is not a valid outside client here, because this WSL shares the host's network.
- Clean-up decisions:
  - Recycle Bin entries from earlier test runs: `kiwix-deleted-*` from `E:\kiwix-win\tmp-private\…`.
  - Worktrees `E:\kiwix-win\wt\*` and `E:\kiwix-win\lf-check`.
  - Old SDK `E:\kiwix-win\sdk`.
  - VS package cache on `C:` (needs admin).
  - An empty torrent staging folder `E:\kiwix-win\library\.kiwix-torrent-059af196…`, left by the pre-fix failed torrent.

## Resuming on another Windows machine
The scripts that drove this work are in `docs/windows-agent-tools/`. They assume everything lives on `E:`; edit the paths at the top of each script for another layout.

| Script | Purpose |
|---|---|
| `env.ps1` | MSVC environment, `RUSTUP_HOME`/`CARGO_HOME`, private `TEMP` |
| `build-sdk.ps1` + `assemble-sdk.py` | kiwix-build dependencies, then libzim with meson, then a schema-2 `kiwix-sdk.json`. Only for the native comparison tools: the app no longer links libzim |
| `regen_patch.py` | Regenerates `libzim-retained-fd.patch` from HEAD's patch plus ordered edits. Verifies that pre-image + patch equals post-image |
| `run-gate.ps1` | The full gate for one checkout (`-Source`, default: the checkout holding the script): frontend build, unsafe census, JS and Python tests, clippy with `-D warnings` and the Rust tests for the app, `kiwix-mcp` and `win-security`, on `x86_64-pc-windows-msvc`. One log per step |
| `run-cargo.ps1` | One cargo command in the `lf-check` worktree, logged |
| `build-nsis.ps1`, `launch-app.ps1` | Build the installer. Launch the installed app with `--data-dir` and CDP on port 9222 |
| `ui/*.mjs`, `ui/save-dialog.ps1` | Drive the app's WebView2 over CDP (puppeteer-core), and native dialogs over UI Automation |
| `linux/*.sh` | WSL Ubuntu SDK build and libzim tests, without sudo |
| `probe/rename_probe.c` | Proves the os error 87 cause and the `NtSetInformationFile` fix |

Open items when this was written:
1. Re-test PDF export and print on a DevDocs article with the F14 fix installed.
2. Look again at the map torrent. With no peers it is blocked by librqbit's missing web-seed support.
3. Remote-device sharing. It needs the firewall change above, plus Share all on the iPhone and Mac.
4. Rebuild the release SDKs (desktop, Android, iOS) for the new patch hash.

## Residual (documented, not fixed)
- FAT/exFAT file IDs are unstable, so identity checks on those volumes fail closed (not testable without a FAT volume).
- ReFS 128-bit IDs are truncated to 64 bits.
- Mtime-only rewrite detection during the release gap.
- Trash is path-based.
- `archive.cpp` lease vs libzim handle identity (`DefineDosDevice` window).
- The WebView2 profile is not under `--data-dir` (by design).
- No service-level test drives `move_archives` on Windows, and no test covers a plain source directory with `\\?\` book paths (F17).
- UX, all platforms: a completed download whose book the user deleted through the Library shows **Failed** ("Completed archive is missing or has changed length") after a restart, but "Unavailable" before it (`acquisition/persistence.rs:142-146`, deliberate history). Clear or relabel the download record when its book is removed. "Delete download" clears the row.
- Map crispness on Windows (owner noticed StreetZim maps look slightly soft). Checked on a 3840×2160 panel at 150% scaling with physical-pixel captures:
  - The MapLibre canvas renders at full device resolution: 1-px roads stay 1–2 device pixels wide, with no 1.5× upscale blur.
  - The softness is grayscale antialiasing plus label halos, against ClearType sub-pixel UI text, at a fractional 1.5× scale and about 160 ppi.
  - Possible StreetZim-side improvement: MapLibre `pixelRatio: Math.max(2, devicePixelRatio)`, at some GPU cost.
  - Not verified: `devicePixelRatio` inside the map WebView2, which is not reachable over CDP.
- F17 review (all Low):
  - A race between opening `from` and `canonicalize`: filter the alias by directory identity.
  - The containment check compares mixed spellings: canonicalize both sides.
  - Moved books are recorded with plain (not `\\?\`) paths: canonicalize the destination.
- Windows print: `PrintDlgW`'s modal loop runs inside the hidden view's `NavigationCompleted` callback. It would be safer for the callback only to record readiness and run the dialog from `run_on_main_thread`.
- Windows PDF profile cleanup: the crash implies the per-job WebView2 profile directory was still locked when `Files` dropped. The `BrowserProcessExited` handler may be released before the browser exits, so "process exit owns cleanup" may not hold. **Confirmed at runtime**: after a cancelled print, "Could not remove private article PDF temporary files … (os error 32)". **Fixed and runtime-verified**: on Windows, cleanup retries sharing violations on a background thread for up to about 18 s before reporting (`native_pdf/cleanup.rs`). After a cancelled print there is now no warning, and the job's `pdf-*` scratch folder is gone. Two `pdf-*` folders from the pre-fix runs remain under `%LOCALAPPDATA%\org.kiwix.workbench\article-pdf` (that cache is not under `--data-dir`, and nothing sweeps it at launch).

## Follow-up — 2026-11-15 (releases v0.0.7 and v0.0.8)
Same machine and toolchain. PRs #21–#23 (desktop Kokoro, Look up, AI assistant connector) are merged; #24 (finished downloads no longer drop below the fold), #25 (Windows ARM64) and #26 (photo chooser) are stacked on `main` and built at `e6eca67`.

### Gate at `e6eca67`
- `cargo test --lib` on `x86_64-pc-windows-msvc`: **1073 passed, 0 failed, 10 ignored**.
- clippy `-D warnings`: fails only on lints already on `main` (`torrent/web_seed.rs`, `kokoro.rs`, `mobile_rebase.rs`, `store_tests.rs`, `acquisition_tests.rs`); none in the new code.
- JS: all 45 suites pass, including the 21-test `test:download-state`.

### Runtime results (installed NSIS build)
| Area | Result |
|---|---|
| Kokoro neural voices on desktop: play, pause, resume, stop, engine switch, private mode, a second window retiring the first, closing the reading window | **Pass** |
| Look up: cold start, warm link, `--search`, a Persian query keeping its ZWNJ, rejected links, a 20-link flood, HKCU key on install/uninstall, a foreign handler preserved | **Pass** |
| AI assistant connector (`kiwix-mcp`): discover, list, search, read, paging, legacy handshake, toggling and reconnecting, private mode, app closed, a pipe squatter refused | **Pass** |
| Photo chooser, Use camera (Logitech C930e, StreetZim Bahrain) | **Pass**: 1920×1080 preview; the 258,756-byte JPEG reached the page and its photo search ran; the camera window and chooser closed |
| Photo chooser, Choose a file (a "MANAMA" sign image) | **Pass**: StreetZim found Manama (Map 7, Results 8) |
| Photo chooser, Cancel; in-place reload | **Pass**: the page got `cancel`; no archive camera grant was created; the chooser still worked after reload |
| Downloads order | **Pass**: today's downloads first (php.net, Wikipedia top, Athens), then history; the unstamped legacy Baltics map card last |
| ARM64 installer | **Built** at `e6eca67`; all 22 shipped exes/DLLs are `0xAA64`. **Not run**: no ARM64 hardware or CI runner |

### Releases
- v0.0.7 (prerelease): x64 installer.
- v0.0.8 (prerelease, target `e6eca67`): `Kiwix_0.1.0_x64-setup.exe` (39,825,138 bytes, SHA-256 `A9F44A04…2F6270D`) and `Kiwix_0.1.0_arm64-setup.exe` (37,841,400 bytes, SHA-256 `EC6902A5…66E8126`). Both unsigned.

### Open (low, from the adversarial reviews)
- Photo chooser: a page's `cancel` handler can reopen the chooser within the same user activation; the camera window could call the app's few non-`dispatch` IPC commands; new shell and camera strings are English-only.
- ARM64 SDK provenance: `kiwix-sdk.json` still says "kiwix-build deps", the ICU zip's `.asc` is not verified, and the revision is hardcoded; libzim's tests are disabled in the cross build.
- Downloads: legacy torrents loaded in private or read-only mode are not backfilled with `created_at`.
- Clean-up still awaiting the owner: Recycle Bin test entries, the old SDK `E:\kiwix-win\sdk`, the two `pdf-*` folders above. (Worktrees `E:\kiwix-win\wt\A`–`E` were removed on 2026-11-15; their uncommitted edits, already merged via #18, are saved as patches in `E:\kiwix-win\wt-backup\`.)

### Final pass before handing back the machine (2026-11-15, installed v0.0.8 from the GitHub release)
| Area | Result |
|---|---|
| Release assets | **Pass**: both downloaded from GitHub match the published SHA-256s |
| Upgrade v0.0.7 → v0.0.8 (silent `/S`, fresh profile `E:\kiwix-win\appdata-upgrade`) | **Pass**: the book, `agentConnector: true` and the running connector survive; the `kiwix:` handler stays registered |
| Map over BitTorrent with only a BEP 19 web seed (StreetZim Bermuda, 30.7 MB) | **Pass**: completed in about 8 s (it was stuck at 0% before web seeds), byte-identical to the web-seed file, matches the catalog checksum (the catalog "md5" is the ZIM's internal checksum), opens and renders; it stays at the top of Downloads |
| Save article as PDF on a DevDocs article | **Pass**: a valid 25,584-byte PDF |
| Print, then close the Print dialog | **Pass**: no crash (F16), and the job's scratch folder is cleaned up |
| Camera: close the camera window | **Pass**: the chooser returns (Choose a file / Cancel); Cancel delivers `cancel` |
| Camera: private mode during a capture | **Superseded** (see 2026-11-16): this run switched private mode through raw IPC, which the UI never does |
| Camera: page reload during a capture; a second activation while the camera is open | **Superseded** (see 2026-11-16) |
| Hidden "Could not connect to the local library service" alert | **Fixed** (`58887cf`): it came from the mobile-only recovery dialog, which was mounted (closed) on desktop; it was never shown. No real connection error appeared in any test |

Test-tool note: sending WM_CLOSE to the app's untitled 15×15 "Tao Thread Event Target" window breaks later window creation ("the underlying handle is not available" when opening the camera). Only a test tool can do this; target windows by title.

### Follow-up pass (2026-11-16, installed v0.0.8, then a branch build)
| Area | Result |
|---|---|
| Private reading on → off through Settings (the real toggle, with its confirmation) on the StreetZim Bermuda map | **Pass**: no error; the reader reopens; the photo chooser works after the round trip |
| The 2026-11-15 "chooser stops working after private mode" failure | **Test-harness artefact, not a product bug.** The script switched private mode by calling `dispatch save_settings` directly. That skips the shell's `api()` wrapper, which freezes and retires every archive document *before* the native cutover. The native side then shut the reader down first, so the retirement navigation to `retire.html` was cancelled (its origin was no longer an allowed reader origin), the 5 s retirement timer raised "The archive could not finish closing its document", and quitting later asked to close without saving the session. v0.0.7 behaves identically under the same raw call. Only the app shell can make that call |
| Camera: page reload during a capture | **Pass**: the camera window and chooser close |
| Camera: a second activation while the camera is open | **Pass**: no second camera window; the chooser stays on "Take the photo in the camera window"; Cancel in the camera window returns to the chooser, then the page gets `cancel` |
| Camera: private mode during a capture | **Not reachable from that window**: the chooser is modal, so Settings cannot be opened while it is up |
| Portable mode (`.portable` marker beside a copy of the installed app) | **Pass**: the camera opens with a live preview; its WebView2 profile is `data\webview\camera-capture`, the shell's is `data\webview\main`; nothing under `%LOCALAPPDATA%` changed during the run |
| Downloads order and torrent `createdAt` across a restart | **Pass**: the same order before and after (Bermuda torrent newest; the unstamped legacy Baltics map last); the Bermuda job's `createdAt` is persisted in `torrents\jobs.json` |
| AI assistant connector on v0.0.8 (`kiwix-mcp` against the main profile) | **Pass**: discover, list, search (finds "Hamilton" in the new Bermuda map), read, error paths, legacy handshake |
| Look up: `kiwix://search?q=Hamilton` on v0.0.8 | **Pass**: Library search opens with about 8,235 results |
| Search: title search, full-text search, table of contents, find in article (Bower Docs) | **Pass** |
| Read aloud with Kokoro (voice Adam), from the panel and from the header play button, with and without the search panel open | **Pass**: audio reaches the endpoint (peaks 0.67–0.78). A selected word is read on its own, as designed |
| HTTP download with a 500 kB/s limit: pause holds the byte count, resume completes; checksum | **Pass**: devdocs_en_cpp (8 MB) matches download.kiwix.org's published SHA-256 |
| Cancel an active download (20 kB/s limit) through its × and the confirmation | **Pass**: row shows Cancelled; no partial file left |
| Reading list: save from the reader, list, export JSON and XML; history export | **Pass**: valid files (338 B, 555 B, 7.8 kB) |
| Save article as MHTML on a DevDocs article | **Failed, fixed** (this pass): "Cannot package referenced asset …/assets/sprites/docs-….webp: Entry not found". DevDocs styles name a sprite the ZIM never contained, and the export refused the whole article. A non-root reference the archive does not contain is now left unresolved, as the reader shows it; the article itself and every other read error still fail. Regression test on `preview.zim`'s `fallback.html`. Verified on the rebuilt installer: the Bower article exports (497 KB, 4 parts) |
| Save original HTML | **Pass** |
| Strict read-only mode through Settings (with confirmation) | **Pass**: Start download is disabled, a bookmark save is refused ("The application is in strict read-only mode"), reading continues, turning it off works |
| Restricted mode on a DevDocs article | **Pass**: an injected inline script is blocked in restricted mode and runs after leaving it; remote requests stay blocked in both |
| Remove from library (keep file), then open the file again | **Pass**: 28 → 27 → 28 books; the file stays on disk |
| Location request from the Bermuda map | **Superseded by a Windows provider**: this pass recorded code 1 because only macOS/iOS had an app-owned provider. A `Windows.Devices.Geolocation` provider is now built and gated on Windows; the earlier code-2 fallback remains for platforms without one. First-use consent, a real fix and the Settings-denied message still need a fresh Windows runtime pass. Archives never get the webview's own location. |
| Native menu shortcuts (Ctrl+T, Ctrl+W, Ctrl+Shift+N, F12, Alt+F) with a page focused | **Failed, fixed** (this pass): nothing happened. Keys typed into WebView2 go to the browser's window and never through the message loop where Tauri translates menu accelerators. A second cause: a single-webview window reports itself unfocused while its page has focus, so the menu dropped actions even when translated. The vendored wry now forwards Ctrl key-downs (not the Ctrl+A/C/V/X/Y/Z editing keys, which the browser handles and muda re-synthesizes, nor Ctrl+Alt/AltGr or auto-repeat), the menu's Alt keys (0–9, Home, Left, Right) and function keys except F10 to the top-level window, and `menus.rs` falls back to the foreground reader window. Verified on the rebuilt installer with real keystrokes: Ctrl+T opens a tab, Ctrl+W closes it, F12 opens Settings, Alt+1 selects the first tab, Ctrl+Shift+N opens a window, Alt+F4 closes exactly that window, and in a text field Ctrl+V pastes once and Ctrl+Z undoes once. AltGr dead keys on non-US layouts are untested |
| Camera window menu and title | **Fixed** (this pass): the capture window inherited the app's menu bar (Kiwix / File / Edit / Tabs), whose commands only reach reader windows, and its native title was just "Kiwix". It is now created hidden, its menu is removed on Windows and Linux, then it is shown, titled "Kiwix — Camera". If showing fails, the session is cleared and the window closed. Verified on the rebuilt installer: the window is found by its new title and has no menu |

Gate after this pass: `cargo clippy --all-targets -- -D warnings` is clean for the first time on this branch. The 13 lints from the newer toolchain (in `torrent/web_seed.rs`, `kokoro.rs`, `mobile_rebase.rs`, `store_tests.rs` and `acquisition_tests.rs`) were fixed mechanically. All 42 JS test files pass, and `tsc --noEmit` is clean.

Observed, not changed: download.kiwix.org's `.magnet` files carry doubled-scheme web seeds (`ws=https://https://ny.mirror…`). The app takes web seeds only from a torrent's `url-list`, which is correct in the `.torrent` files, so this has no effect today; it is an upstream data issue.

Still to test:
1. Owner-only: the SmartScreen first run of a browser-downloaded installer; the camera unplugged or blocked in Windows privacy settings; sharing from this PC to the iPhone/Mac.
2. The ARM64 installer on real ARM64 hardware.

## Follow-up — 2026-11-20 (public history, SDK-free build)
Same machine and toolchain (Rust 1.99.1, MSVC 19.44, Node 22.22.0, Python 3.13.9). Base: `8bc3758` on the rewritten public `main`. Work branch: `fix/windows-test-gate`. This pass built the app and ran every test suite; it did not repeat the installed-app runtime matrix.

The app now reads archives with its Rust reader. No libzim SDK was built, set or linked, and the installer carries no SDK DLL.

### What failed at `8bc3758`
| # | Was | Cause | Fix |
|---|---|---|---|
| G1 | **Removing a voice model could fail with "Access is denied" (os error 5)**, and then keep failing. `two_records_with_missing_members_do_not_block_explicit_repair_or_delete` failed | A product bug. `mark` reopened an existing marker read-only and flushed it; Windows flushes only through a handle with write access. Markers are never deleted, so any second write of one failed: a second preserved install record (then every later install or removal failed too); a repeated removal with the legacy model folder present; resuming an interrupted removal; clearing voice audio twice with foreign files left behind. A plain remove, reinstall, remove did not reach it: the bundled-voice marker is written only on Android | `kokoro/install.rs`: on Windows an existing marker is reopened with append access, bound by file identity to the checked handle, then flushed. Nothing is written. New test `a_repeated_marker_is_accepted_and_a_changed_one_is_refused`; both tests fail without the fix |
| G2 | Three tests failed with os error 32: `deletion_and_listing_prove_identity_without_reading_archive_bytes`, `library_archive_identity_is_rechecked_for_every_request`, `library_server_serves_index_opds_and_articles_then_closes_every_port` | Test only. Each deleted an archive while it still held a reader open. The Rust reader leases every part without delete sharing on Windows, and on Windows the service releases its holders before it removes a book | Under `cfg(windows)` the tests release the handle first: the first drops its own reader, the other two use the same cache eviction as the service. The sharing test first proves, with the entry still cached, that a relinked record is refused. Other hosts run the tests unchanged |
| G3 | `failed_install_preserves_same_inode_edits_and_removes_unchanged_live_writes` failed with os error 32 | Test only. It rewrote a staging file while its own handle, which denies other writers on Windows, was still open | The handle is closed before the rewrite; the file edited is the same one |
| G4 | `desktop_entries_accept_a_bare_file_or_search_url` failed | Test only. `file:///tmp/…` has no drive letter, so it is not a local path on Windows | The test uses `file:///C:/tmp/…` on Windows |
| G5 | 53 of 182 Python host-tool tests failed or errored (68 results, counting subtests), and left child processes that kept the run's output pipe open. Four more passed without testing anything: the POSIX-only failure was the error they expected, or was swallowed as a logging failure | The iPhone, Mac–Android, pairing and timing tools are written for macOS and Linux hosts: process groups, POSIX signals, signal masks, `O_NOFOLLOW` directory opens, `select` on pipes, unprivileged symlinks, POSIX container paths | Those 57 tests are skipped unless `os.name == "posix"`. A trace of the remaining Windows passes found no other swallowed POSIX-only failure that decides a result. On WSL Ubuntu the six edited modules apply none of these skips: 110 tests run, 108 pass and 2 need macOS. Editing `test_android_pair.py` changes an input that `android-pair.py` pins, so a prepared Android pairing window must be prepared again |
| G6 | One JS test (`vendored upstream files are byte-identical…`) failed in the existing checkout | Not a repository defect. 628 files in a working tree older than `.gitattributes` still had CRLF | The files were checked out again. `WINDOWS-BUILD-PROVISIONING.md` now says how to spot this |
| G7 | clippy failed once in `rust_embed` ("File should be readable") | Not a repository defect. An installer build ran `vite build` at the same time and emptied `dist/` | Run the frontend build first and nothing concurrently; `run-gate.ps1` does |

### Gate after the fixes (`docs/windows-agent-tools/run-gate.ps1`, all steps exit 0)
- `npm ci`, `tsc --noEmit` and `vite build`: clean.
- Unsafe census: 73 blocks in 18 files, every one documented.
- JS: **384 passed, 0 failed, 3 skipped** (387).
- Python: **123 passed, 0 failed, 59 skipped** (182): the 57 above and 2 that need macOS.
- clippy `--all-targets -D warnings` on `x86_64-pc-windows-msvc`: clean for the app, `kiwix-mcp` and `win-security`.
- `cargo test` for the app (lib, binary and doc tests): **1169 passed, 0 failed, 16 ignored** (1185). Before the fixes: 1162 passed, 6 failed.
- `kiwix-mcp`: 8 passed. `win-security`: no tests.

### Independent review
One independent, read-only adversarial review covered the whole change set against `8bc3758`, followed by a check of the fixes. It found nothing of high severity.
- G1 was judged sound by source trace: the Windows reopen cannot create, truncate or overwrite; a swapped symlink or directory fails; the identity comparison is by handle; other hosts compile the previous code. Every other `sync_all`/`sync_data` site flushes a handle opened for writing, or is a directory sync compiled only on Unix.
- Fixed after the review:
  - `run-gate.ps1` recorded a tool that could not be started as a pass, because it read the previous command's exit code. It now fails that step.
  - Four Python tests passed on Windows without testing anything; they are now skipped (part of G5).
  - One G2 edit had dropped the reader on every host; it now applies only on Windows, and the sharing test gained its cached-relink proof.
  - The G1 triggers were described too widely.
  - Stale SDK text in the Windows documents.
- Open, low:
  - Present before this pass: a whole-library web share keeps its cached reader after a book is relinked, or removed without deleting its file. On Windows that file then cannot be deleted or renamed until the share stops or the entry leaves the cache.
  - On Windows, `library_server_serves_index_opds_and_articles_then_closes_every_port` evicts before it deletes, so "refused while still cached" is proven for a deleted archive only on other hosts. Windows cannot delete a leased archive at all.

### Not covered in this pass
- The installed-app runtime matrix above was not repeated on the Rust reader build. G1 was verified by tests, not by removing a voice in the installed app.
- The fuzz targets and the native sanitizer script need Unix tooling and were not run.
- No macOS, Linux, Android or iOS build was run, and no Rust test ran on another host: WSL here lacks the WebKitGTK development packages. The G1 reopen is compiled only on Windows; other hosts compile a wrapper with the previous behaviour. Of the test edits, the new marker test, the reordered close in G3 and the URL test in G4 also compile on other hosts and were not executed there.
- ARM64 has no test run: an x64 host cannot execute ARM64 binaries.
