# Changelog

This changelog records source changes, not release certification. Only unsigned Windows test prereleases have been published from this independent build (v0.0.7, and v0.0.8 with x64 and ARM64 installers); none is a certified release. See [feature coverage](docs/FEATURES.md), [issue dispositions](docs/ISSUES.md), and [packaging](docs/PACKAGING.md) for boundaries.

## Unreleased — 0.1.0 implementation

### Latest implementation and adversarial corrections

- Build spelling suggestions in a checked Rust file format tied to the actual installed archive. Existing SQLite spelling files are preserved. A user can rebuild suggestions from the archive's titles after a zero-result search. [Storage, verification and measurements](docs/benchmarks/2026-11-18-spelling-index.md)

- Preserve unrelated files when an interrupted library move resumes or rolls back. A downloaded archive remains available when only its final history update fails, and Downloads shows the warning separately.
- Cancel stale reader exports when the archive installation or privacy context changes. Reading history and search now show 50 results per section while continuing to search the entire collection.
- Harden local connector startup and cleanup, release signing, and Windows installer resource checks. The November 17 review records the fixes, independent checks, and remaining platform and dependency limits. [Project review](docs/reviews/2026-11-17/PROJECT-REVIEW.md)

- Offer **Use camera** or **Choose a file** on Windows when an archive's image file input is activated (StreetZim photo search). The camera runs in an app-owned `camera-capture` window that alone is granted camera, only while a user-started capture is open; archives gain no camera access, WebView2 no longer saves permission decisions in the profile, requests carry a per-session secret hidden from archive scripts, and picked files are read bounded and content-sniffed. Three adversarial reviews (two design, one implementation); installed Windows 11 build verified with a USB webcam. Design and limits: [PHOTO-CHOOSER](docs/PHOTO-CHOOSER.md).
- Fix a finished download dropping below the fold: the Downloads list is ordered by start time whatever each job's state, so a download keeps its place when it completes, and Resume/Retry moves a job back to the top. Torrent and map jobs now record `created_at` (legacy torrents backfill from metadata mtime), the map card joins the same list, and retained original editions move below it.
- Add a Windows ARM64 build: cross-built libzim 9.8.2 SDK (meson zlib/zstd/static Xapian, official ICU 74.2 WinARM64), an installer script that pins the sherpa-onnx ARM64 DLLs, bundles the ARM64 VC++ runtime app-locally and checks every shipped binary is `0xAA64`; sha2's asm backend is disabled on Windows ARM64 because sha2-asm's GNU assembly cannot build with MSVC. Not yet run on ARM hardware. See [PACKAGING](docs/PACKAGING.md#windows-arm64).
- Add Look up (`--search`, bare desktop-entry URLs, `kiwix://search?q=`) and the opt-in read-only AI assistant connector (`kiwix-mcp` relay, MCP 2026-07-28 stateless plus the legacy handshake), and run Kokoro neural voices on desktop. Windows uses a per-user named pipe and installer registration; macOS declares the `kiwix` URL scheme and `RunEvent::Opened`, and Linux uses `%U` desktop entries with `x-scheme-handler/kiwix`. macOS/Linux use a same-UID Unix socket at `<profile>/instance/`, or the container temp directory in the macOS sandbox, with the relay bundled as a Tauri external binary. The packaged sandboxed macOS app and bundled relay passed end-to-end list/search/read and private-mode stop on 2026-11-16; the relay compiles for Linux and has socket-path tests, with a Linux desktop app build and runtime run still required. See [PLATFORM-INTEGRATIONS](docs/PLATFORM-INTEGRATIONS.md) and [AGENT-CONNECTOR](docs/AGENT-CONNECTOR.md).
- Refresh the frontend and Rust lockfiles. Vite 8.3.8, `@vitejs/plugin-react` 6.3.0, `lucide-react` 1.72.0 and `puppeteer-core` 25.17.0 are current; TypeScript stays 5.9.3 because TypeScript 7 breaks the compiler API used by the JS test harness. Move ASCII bundle output to Rolldown’s `output.charset`. Rust updates are compatible minors/patches; `notify-types` stays 2.2.0 to preserve the documented Rust 1.90 minimum. Build, 340 JS tests, 1116 Rust tests, Android emulator import/map, and the iPhone launch smoke passed.
- Keep the debug-only Android evaluation JNI bridge buildable under current Rust: use `#[expect(unsafe_code, reason = "JNI export requires no_mangle")]` on exactly the two exports, no wider module allowance. The `AndroidEvaluationTest` hardware/APK admission probe passed on the emulator; the fixture-backed workload was not run.
- Add a confirmed bulk **Update all** for archive editions. Native `find_updates` marks only a bound series with exactly one newer candidate as bulk-eligible; the settings panel shows the eligible count, queues prepared full downloads through the existing edition flow with bounded concurrency, reports per-archive failures, stops cleanly on request, and still leaves ambiguous lineage and every verified Replace decision explicit.
- Declare and request Android 17's `ACCESS_LOCAL_NETWORK` runtime permission. The Android plugin manifest now declares it, and every LocalSwarm LAN command (discovery, inspect, download, seed, share-all, shared-library browse/download, `sharing_start` and LocalSwarm resume) plus private reading sync's configure/pair/status/confirm/sync commands is admitted through a native permission check before the first socket opens. Loopback-only sharing (the `loopback` binding or a loopback `interface` address) is exempt. The permission is in the existing `NEARBY_DEVICES` group, so a prior Bluetooth or nearby-Wi-Fi grant resolves without a second sheet; a denied request fails with `LOCAL_NETWORK_PERMISSION_DENIED`, and a result delivered before `onResume` is retried through the existing suspension gate. The two-second private-sync status poll checks first and shows the sheet at most once per process, so a denial cannot turn refresh into a prompt loop. The `arm64` debug APK assembles and its merged manifest contains the permission.
- Target Android 17 (API 37): the generated Android application now declares `compileSdk = 37` and `targetSdk = 37` with the pinned AGP 9.3.1 / Kotlin 2.2.10 / Gradle 9.6.1 toolchain, and the arm64 debug APK assembles against SDK platform 37.0 (package `platforms;android-37.0`). The vendored `:tauri-android`, `:rustls-platform-verifier` and plugin modules keep `compileSdk = 36` and still resolve platform 36; AGP's default Build-Tools 36.0.0 is unchanged because no module pins `buildToolsVersion`.
- Compile the Android plugin module again. `DocumentsContract.PATH_TREE` and `PATH_DOCUMENT` are hidden platform constants absent from every SDK `android.jar`, so tree-child validation compares the documented `tree`/`document` path segments instead of failing the Kotlin compile.
- Move the Android and desktop SDK recipes from Xapian 1.4.23 to 1.4.32, checking the official `oligarchy.co.uk` tarball SHA-256 (`c4fd64e8…7aece`) and re-applying the unchanged `xapian-fd-offset.patch` with offsets. The iOS route still takes Xapian 1.4.23 from its pinned kiwix-build checkout.
- Add an explicit Export history action beside Clear history. It writes a versioned, lossless JSON document (`version` 1 plus every stored `id`, `bookId`, `path`, `title`, `visitedAt` and `scroll`) newest first with deterministic identity tiebreakers; it never imports, mutates or clears history, uses the same save picker/cancellation and read-only gates as reading-list export, and is covered by focused Rust export tests.
- Highlight the sentence and word being spoken during Read Aloud without mutating archive content or saved annotations. Desktop Web Speech boundary events and native iOS/Android word-range callbacks now feed the same deterministic text extraction the bridge already used for native speech; a run map resolves UTF-16 offsets to live text nodes, CSS Custom Highlight paints the sentence and word, and older WebViews get an `aria-hidden`, `pointer-events:none` overlay. Paint clears on pause, stop, navigation, pagehide and privacy/session replacement, and scrolls only when the active sentence is fully out of view. Boundary callbacks are engine-advisory; a voice that emits no ranges may leave the starting sentence painted, and physical audible alignment remains unverified.
- Native Windows MSVC build and runtime acceptance. Confined publish, move and delete now use capability-relative `NtSetInformationFile` rename and verified release-then-act deletion instead of invalid Win32 root-handle renames; handle-based volume/file-index receipts, sharing-violation classification, verbatim-path display, PDF scratch cleanup, and the print-cancel WebView2 runtime abort are corrected. The Windows agent passed 1009 lib tests and the installed NSIS matrix; residuals and exact evidence are in `docs/WINDOWS-TEST-REPORT.md`.
- Give reader-storage durability the native background window instead of a 1.8-second write deadline. A document picker, lock or app switch can leave a hydrated reader flush in flight longer than a UI frame; the checkpoint now waits up to 20 seconds while a foreground return still supersedes it and the iOS task remains the hard deadline.
- Add a session-scoped restricted reading mode for untrusted archives. The
  reader keeps its origin and instrumented bridge but serves archive HTML with
  a per-response nonce and blocks archive scripts, forms, workers, frames and
  network requests; a reader toolbar/banner toggles it and reloads the article.
  A pre-open Library choice applies the mode before the first archive request, including when a normal reader already exists, and the flag is preserved in the saved tab session. Service regressions verify the nonce CSP, pre-open selection, unrestricted reversal, and the normal-mode CSP still permitting WASM, blob Workers and camera policy.
- Compact live-page PDF captures with shared, declaration-ordered computed CSS
  chunks for elements and both pseudo surfaces, preserving inline geometry
  overrides and the 16 MiB limit. Escape literal closing-style text when
  serializing generated stylesheets.
- Cancel a pending link-preview dwell when trusted Escape dismisses reader
  chrome, and only request Library title suggestions while the search input is
  focused, so a blurred field cannot reopen a popup over another control.
- Initialize a genuinely fresh Nearby test installation through ordinary app
  startup before capturing its real preservation baseline. Failed inventory,
  authorization and state-copy operations still fail the run. Remove unscoped
  Cancel/Resume actions that could affect an unrelated download.
- Validate retained local-only Wi-Fi snapshots consistently when the OS marks
  their internet path unsatisfied. Withheld Wi-Fi information no longer bypasses
  bound-interface/address preservation or clears previously observed BSSID
  authority. Independent socket endpoint/on-link checks remain mandatory.
- Share iOS idle-timer ownership across discovery, publication, joining and
  active transfers; retiring one owner must not allow sleep during another.
- Tie Android screen wakefulness to the existing aggregate transfer lifetime,
  including publisher retirement on private/read-only cutovers. Stale Activity
  callbacks cannot modify replacement windows or pause already-admitted
  application-owned background transfers.
- Advertise actual IPv4 listener addresses through Apple's DNS A-record
 adapter without rejecting or rewriting a valid dual-stack LAN descriptor.
- Print a shell-quoted prepared iPhone run command with its selected target
  and configuration, rather than falling back to a different default device.
- Capture retained Android native crash records without launching the UI, and
  bind reviewed report identities and paired-tool receipts to their exact bytes.
- Reject moving Android test controls and verify exact native crash-dialog
  targets before input; preserve the reviewed report instead of hiding new crashes.
- Add prepared Mac↔iPhone Apple LAN acceptance with frozen artifacts, isolated
  Mac/browser profiles, source-generation binding and managed-reader proof gates.
- Acknowledge retained iOS Join failures once at sharing-test startup, without
  retrying stale Join requests or suppressing new association failures.
- Preserve the actual Android peer failure in paired LAN acceptance instead of
  misreporting every failed transfer as missing cleanup/preservation evidence.
- Add prepared two-iPhone sharing acceptance with independently observed source
  sessions, frozen device artifacts and a shared bounded cleanup window.
- Distinguish a receiver that never launched from a failed cleanup; partial
  launch still requires exact-device cleanup proof and preserves the run failure.
- Freeze the imported receiver helper in paired LAN preparation, rejecting
  dependency drift before helper import or device contact.
- Retire each native library request's owning QUIC group after its handler
  drains, and recognize exact remote no-error closes without accepting local
  errors, cancellation or stream EOF as successful completion.
- Add per-iteration build, ADB, device/UI and cleanup timing, with durable
  whole-loop markers, overlap-aware coverage and explicit unmeasured gaps.
  Freeze timing dependencies without invalidating unchanged production-app
  builds; never report residual wall time as measured inference.
- Expose a completed download's verified publication identity (size and
  SHA-256) with its job, so a receiver can prove the bytes it imported rather
  than only the file name. Keep the value unset until the final publication
  actually commits.
- Require explicit idle UI and native hotspot retirement after failed Android
  publisher starts. Normalize cleanup receipts without weakening identity or
  overwriting an earlier product failure with later cancellation.
- Bind shared-library sockets with plain prefix arithmetic instead of hidden
  `IpPrefix` constructors, so Android 9 receivers can connect over the LAN
  instead of failing with a generic Java exception. Surface the exact native
  bind failure in the command error.
- Make Android `Share all` LAN-only: a device without usable Wi-Fi/Ethernet
  now fails with `NEARBY_LAN_REQUIRED` and points at `Use temporary Wi-Fi`
  instead of silently starting an access point mid-publication.
- Keep test teardown from crashing or stranding the app: if `Instrumentation.finish()`
  cannot disconnect automation, the harness emits a retirement receipt and kills
  only its own target process. The Mac peer waits for the discovery card instead
  of sampling it once.
- Preserve a running temporary hotspot on repeated setup, verify iOS joins
  against the exact association instead of an arbitrary Wi-Fi interface, and
  refresh harmless route-generation changes without discarding a valid lease.
- Upgrade a nameless BLE discovery row when its later scan response supplies
  the publisher hint, retaining its identity across the native/Rust handoff.
- Accept CoreBluetooth's standard Extended Properties bit on the required BLE
  write/indicate channel, without accepting missing operations or alternate
  notification/write modes. Android previously rejected the physical iPhone's
  valid `0xA8` channel before credentials could be exchanged.
- Retain an Android GATT server through an outstanding service addition during
  Stop. Its exact late completion performs cleanup only; a missing completion
  cannot silently release ownership or permit a replacement peripheral.
- Keep Nearby privacy and sharing-admission generations separate. A reverse
  iPhone sender now retains the admission epoch captured when sharing started,
  instead of rejecting a successful Wi-Fi join using the privacy counter.
  Explicit Stop still cancels that owner's inherited admission token.
- Backpressure parallel library ranges against bounded per-peer admission and
  finish framing/ACK for consumers that stop at Content-Length. Drain source
  work and release server admission before announcing connection closure.
- Add bounded Nearby device acceptance to the existing iPhone tool: exact
  source selection, completed download, readable active archive and received
  file hash. Remove the broad download-cancellation/reset script and permanent
  scheme authorization. Offline runner preparation pins Apple-issued profile
  bytes; an Apple Account on the test phone is not required. Physical
  routerless completion and phone throughput remain separate acceptance gates.
- Pair iPhone sender acceptance with same-certificate Android instrumentation
  against the optimized release. Require disconnected station Wi-Fi, a fresh
  product-owned reverse hotspot, managed received bytes and the active reader;
  retain screenshots for content review rather than treating an iframe as proof.
  Preserve an already-imported managed sender fixture on retries. Bind both
  runners to one integer deadline, recognize Samsung's current radio fields,
  and restore the explicit normal Android activity after instrumentation.
  Verify native keyboard occlusion has ended before tapping Download; DOM
  hit-testing alone cannot detect a soft keyboard over the WebView.
- Automate Android publication in the paired iPhone receiver workflow as well:
  require initial disconnected radios, real product-owned AP readiness, scoped
  publication stop, preserved data and final OS evidence that the AP is gone.
  Reject stale Android tooling during off-phone preparation.
- Wait for native listener readiness before automatic Nearby discovery and
  initialize Bluetooth even when LAN browsing is already active. Keep startup
  errors visible and respect an explicit Find/Stop instead of restarting it.
- Remove a duplicate Unix descriptor metadata read from content-identity
  validation; retain fresh before/after size, inode, device, mtime and ctime checks.
- Add launch-only iPhone updates without reader fixtures, navigation or radio
  changes. Preserve existing archives, settings, bookmarks and tabs, and verify
  the exact profile and certificate after Xcode-managed runner signing.
- Make same-LAN sharing work across Mac, Android and iPhone from one
  **Share all** action: the Mac and Android advertise their live library over
  DNS-SD, the receiver lists the peer with its archive count, and **Choose**
  opens the shared catalog for selective or full download.
- Give every restarted library session a fresh DNS-SD identity and keep an open
  catalog across transient discovery gaps, so stopping and restarting a share
  cannot leave a receiver stuck on a stale session.
- Preserve the explicit share snapshot: archives received through Nearby are
  never added to an already-running share automatically.
- Add **Use temporary Wi-Fi** on Android for senders and receivers on different
  networks. The app-owned hotspot retries while Android publishes its AP
  interface, recognizes a reused soft-AP interface, and lets local transfers
  start without Internet admission. iOS users get an actionable prompt to
  approve the Wi-Fi join and Local Network access.
- Reuse unfinished bytes between LocalSwarm and BitTorrent acquisitions of
  the same archive. A LocalSwarm receive adopts chunks from a same-sized
  torrent partial only when each matches its manifest SHA-256, and requests
  only the rest; a first-run torrent is seeded with an unfinished LocalSwarm
  receive's verified ranges, which the engine's initial piece check must
  accept. Donors are only read. Exercised: unit tests for range coverage, no-follow donor opening, chunk
  adoption, different-content abandonment and torrent prefill journaling, plus
  a loopback end-to-end receive that fetched only the chunks missing from a
  saved torrent partial. No physical-device or live-swarm BitTorrent run.
- Accept an optional advisory `zim.uuid` in selected-file LocalSwarm
  manifests and ignore unknown top-level manifest keys for forward
  compatibility. The claim is validated, excluded from the content address and
  never trusted; duplicate detection still uses verified bytes. Exercised by a
  protocol unit test; no reference sender was run against Kiwix.
- Drop a failed Nearby peer instead of keeping a stale advertisement. iOS and
  Android emit a `lost` event for the candidate whose connection failed, the
  Rust discovery removes it from the list, and the friendly error asks for a
  retry. This removes the nameless stale row that produced repeated
  `BLE_CONNECT_TIMEOUT` dialogs even when the named Android peer connected.
- Give `.button-row` a wrapping flex layout with an 8 px gap, so the Settings
  **Verify available archives** and **Review diagnostic report** actions no
  longer touch or overlap on narrow phones.
- Translate BLE/Nearby failure codes (`BLE_CONNECT_TIMEOUT`,
  `NEARBY_BLUETOOTH_*`, stale peers, permissions, foreground requirements) into
  actionable sentences instead of showing the raw code in the dialog.
- Recover a blank MapLibre canvas after WebView WebGL context loss or a
  persisted `pageshow`: the reader bridge reloads the map once, rate-limited to
  one reload per 15 s, instead of leaving an empty page after navigating back
  from an article.
- Name the cause when an archive cannot be opened: missing/moved, permission
  denied (including file-level denial), empty, invalid or truncated, or an
  embedded-checksum failure each get a distinct actionable sentence; raw
  native/libzim text is no longer shown (it goes to stderr only).
- Add a persisted **Reading width** control (Original / Comfortable / Full
  width) in the reader appearance panel. It is applied before first paint and
  in nested archive documents; Comfortable centres a 52rem measure, Full width
  removes common wrapper limits, and XHTML/text resources are included.
- Make reader search announce its state: a live region reports searching,
  `1–50 of 500` ranges while paging, no results, empty-query validation, and
  failures once; opening focuses the field, Escape or closing returns focus to
  **Search books**. Translation keys stay statically inventoried.
- Block webview navigation from archive content to external origins. Every
  desktop window now allows only the shell, the exact origins of open readers,
  and `tauri`/`about:blank`/origin-bound `blob:` hand-offs; external `http(s)`,
  `data:`, `file:` and arbitrary loopback ports are refused before a request.
  WebView2 frame navigations use the same policy, the shell CSP adds
  `frame-ancestors 'none'`, and archive responses send
  `X-DNS-Prefetch-Control: off`. The headless API strips `__windowLabel` and
  rejects a `parentOrigin` that is not the printed shell origin.
- Document the authenticated loopback headless API in [docs/API.md](docs/API.md):
  startup, token/Host/Origin rules, request/response codes, verified commands,
  security boundaries, and token-safe curl examples.
- Format byte sizes without ever printing 1000 of the smaller unit. Unit
  selection now bumps when one-decimal rounding would reach 1000, so 999,999
  bytes reads **1 MB** instead of "1000 kB" and 999,950,000 bytes reads **1 GB**
  instead of "1000 MB" (Android #5246). The selector is a pure module covered by
  `scripts/test-format-bytes.mjs`.
- Paint the startup canvas with the platform paper instead of hardcoded black.
  The pre-CSS boot rules now use `var(--paper)`/`var(--ink)` (falling back to the
  old colours), so the macOS dark canvas is `#1e1e1e` and the light canvas is
  `#f6f6f6`, matching the native window background and the library surfaces.
  The native light window background now also matches each desktop skin
  (macOS `#f6f6f6`, Windows `#f3f3f3`, Linux `#fafafb`). Verified in Chromium
  and WKWebView: dark `rgb(30, 30, 30)`, light `rgb(246, 246, 246)` for both
  `html` and `body` (desktop #1564).
- Reconcile archive availability on the first maintenance cycle instead of the
  fifth. A file deleted while the app was closed now flips to unavailable
  **0.108 s** after startup rather than after 15 s; a restored file returns on
  the next periodic pass. Headless startup on the current build measured
  **0.99 s** median cold (fresh profile plus a 333 MB archive argument) and
  **0.031 s** warm, with no network dependency.
- Show installed editions in Discover. A catalog entry whose name and date
  match a library book now reads **Installed** with an **Open** action instead
  of offering a duplicate download; another date of the same archive names the
  installed edition and keeps the separate-edition download note. Verified in
  Chromium with a controlled catalog page against the real Wikipedia fixture:
  Open reached the installed archive's main page in a new reader tab.
- Re-verify the desktop, Apple and Android issue/PR audits against current
  source. Portable mode, archive-backup exclusion, cellular/metered waiting and
  reconnect resume, the 1 GB low-space warning, empty-TOC/theme/launch
  behaviour and bookmark-import deduplication are implemented; the remaining
  rows are device or Windows acceptance. See
  [issue coverage](docs/ISSUES.md#upstream-parity-re-verification--2026-11-06).
- Add regressions for two destructive upstream fixes: a malformed archive path
  (empty or a directory) can never widen deletion to its neighbours, and a
  cancel arriving after completion cannot erase the finished archive.
- Hide reader controls while reading on a phone and restore them reliably. A
  genuine downward pan that also scrolls the document removes the reader
  toolbar, the shell navigation and the workspace bar, and gives the article
  their space; an upward pan, the top of the article, a visible reveal button,
  a key press, focus, a dialog, navigation, a tab change, suspension or a
  viewport resize bring them back. Horizontal map drags, pinch, selection,
  editing and programmatic scrolling never trigger it. Actual Chromium at
  390×844 with the real Wikipedia and Hawaii archives measured the reader area
  growing from 736 to 844 CSS px, and programmatic scroll, touch cancellation
  and forged pre-head messages left the controls visible. Not exercised on a
  physical iPhone or in WebKit. See [reliability](docs/RELIABILITY.md#reader-controls-and-document-bootstrap--2026-11-05).
- Place the reader and storage bridges before any archive script. The previous
  insertion point was the first raw `<head`, which archive markup can precede.
  The new bounded, allocation-free prolog scan keeps leading text, comments,
  doctypes and the authored `<html …>` tag in front of the scripts, so an
  authored root style still merges and the document keeps its parsing mode.
  A doctype whose quoted system identifier contains `>` previously let an
  archive script run before the bridge; an actual browser run now reports the
  bridge first. Six unit fixtures and an owned browser fixture cover pre-head
  scripts, fake `<head>` text, abrupt comments, quoted/`SYSTEM` doctypes,
  authored root styles and a streamed 17 MB article.
- Make three more segmented-download tests deterministic. They raced a
  throttled body and failed on a fast CI runner with
  `pause happened after the whole transfer`; the gated test server now holds
  each range after its durable checkpoint until the test releases it.
- Stop the macOS developer bundle from looking up release signing
  credentials. The unsigned branch inherited `APPLE_SIGNING_IDENTITY` and
  failed with `The specified item could not be found in the keychain`; it now
  builds ad-hoc and lets `bundle-macos.mjs` re-sign with the app entitlements.
- Integrate the dependency-reduction, Android HTTPS-verifier, Windows cfg/CI,
  bounded-HTML, macOS sandbox and Apple SDK compatibility work. Correct
  review findings in privacy authority across folder prompts, normalized
  no-follow directory admission, bookmark-write retry, and parser EOF work,
  cancellation and shared-export accounting. Do not bookmark-probe the real
  Downloads directory: its persistent sandbox authorization comes from the
  existing entitlement. Actual isolated Mac startup, folder grant/relaunch,
  original-archive reads and Downloads export passed; this is not broad
  sandbox release certification.
- Keep ordinary iOS archive opening in place. An absent or false incoming
  authority hint now opens the original-file picker instead of selecting
  managed copying. Copying remains a separate explicit storage action;
  provider errors/cancellation never silently choose it. Existing library
  copies and originals are not moved or deleted. Browser UI behavior is
  exercised; this change has not been installed on the user's iPhone.
- Use the first readable paragraph for article previews, skipping empty
  leading paragraphs. The real Wikipedia Animal excerpt now appears;
  real MHTML export preserves all 611,348 original article bytes.
- Keep BitTorrent's blocking certificate checks off the app's shared
  runtime. librqbit's HTTPS trackers verify with blocking platform calls
  (JNI on Android, where a CRL fetch took 5–8 s on an emulator). Its calls
  that start tasks now run on a dedicated two-thread runtime. Pause, cancel,
  suspend and privacy cutovers no longer await routed engine work under
  manager gates. Abandoned adds are reaped before a subsequent worker re-adds;
  prompt command return does not mean cleanup has already finished.
  The merged host suite passed 1,001 tests; actual 150.6 MB Hawaii transfer,
  pause/resume, matching SHA-256 and asynchronous import passed.
  See [reliability](docs/RELIABILITY.md#android-https-tracker-acceptance).
- Make two segmented-download tests deterministic. Under load they raced a
  throttled transfer and failed intermittently on `main`. A gated test server
  now holds each range after a durable checkpoint. Locally under CPU
  contention, the original test failed 4/15; the gated versions passed 40/40.
- Fix BitTorrent HTTPS trackers on Android. librqbit's HTTPS client verifies
  through the Android platform verifier, which was never initialised. On an
  emulator, adding a magnet with an `https://` tracker panicked in librqbit's
  `add_torrent`. The torrent engine now initialises the verifier from the
  application Context before it starts, and the Kotlin component is built
  from pinned vendored source. If the component is missing, BitTorrent reports
  "Reinstall Kiwix" instead of panicking. It also permits cleartext to
  upstream's CA revocation-list hosts, which CRL-only certificates need. An
  API 24 emulator probe reproduced the panic, then a successful real librqbit
  HTTPS announce. The full app, physical devices and public trackers were not
  exercised; see [the probe](scripts/android-https-tracker-probe/README.md).
- Run bundled macOS builds in the App Sandbox. Archives are read through
  their folder, so Kiwix asks for access to the folder of archive files that
  are added, dropped or opened with Finder's Open With, and keeps folder access (in-place archive folders, watched folders,
  current and recent download folders, folders named by saved edition
  updates) as security-scoped bookmarks. New bookmarks are recorded before the
  profile is committed and unneeded ones dropped only after; failed or dead
  bookmarks are retried whenever the folder is accessible again. Directories
  inside the sandbox are opened directly instead of being walked from `/`,
  which the sandbox does not allow; when `F_GETPATH` reports another spelling,
  every ancestor is checked for links and the opened directory's identity is
  compared. The default download folder is the real `~/Downloads`. Exports are
  prepared in the system replacement directory. The second-instance socket
  moves to the container's temporary directory, where its path fits. Portable
  mode, self-update and automatic update checks (and their settings text)
  are disabled in the sandbox.
  Files, folders and links open through NSWorkspace. Existing unsandboxed
  macOS profiles are not migrated. The initial branch was type-checked only;
  the isolated native startup, directory/panel, bookmark-restore and Downloads
  export acceptance above was added during integration. Deletes, HTTP/torrent
  downloads, reveal/attachment opening, drag and drop, printing, camera,
  location and LAN sharing remain unexercised inside the sandbox.
- Evaluate application-package size optimization with unchanged dependencies,
  SDK, features and frontend. The strip-controlled trial saves 745,472 APK
  bytes, but establishes neither a whole-app memory win nor faster startup;
  it is not promoted and the default optimization-level-3 APK is restored.
  The initial symbol-retaining artifact is an explicit confounded trial,
  not an optimizer-size regression. Both complete 32-sample startup-enabled
  cohorts remain separate in the [receipt](docs/benchmarks/2026-11-05-android-root-size.json).
- Stop archive-storage callbacks retaining the complete initial bootstrap payload;
  preserve scalar read-only/generation authority. Fresh-context explicit-GC
  reachability changed from retained to collectible with clear/set sequencing
  intact; no payload-byte saving is inferred.
- Defer independent acquisition/map HTTP/TLS clients until actual manager network
  operations, preserving journal recovery, transport policies and cancellation.
  Current native six-case real-Wikipedia recovery and real HTTPS Hawaii
  composition/import passed; physical production Animal/Hawaii/Hilo smoke passed.
  The **67,145,771-byte APK** is 4.58% below official ARM64, but its new
  32-sample cohort remains worse: **257.03 versus 247.26 MiB PSS (+3.95%)**,
  all four pairs unfavorable. See the [scoped receipt](docs/benchmarks/2026-11-05-android-idle-memory.json);
  this is not a demonstrated whole-app memory or speed win.
- Extend the Android profiler with opt-in article-reference pixel-ready startup
  bounds and retained Home/loading/match frames. Its actual four-launch
  calibration completed without manual intervention; coarse screenshot timing
  intervals overlap, so no faster app is claimed.
- Emit Vite bundles as ASCII and escape the reader bridge's four non-Latin-1
  symbols without changing translated/runtime values. Actual physical WebView
  heap graphs measure **810,334 bytes less backing storage for two retained
  script sources only**, not a whole-app memory saving. The installed APK is
  **67,137,579 bytes**, 4.59% below official ARM64 but 151,552 bytes above its
  predecessor; a separate 16-sample Animal run still uses **253.30 versus
  245.56 MiB PSS (+3.15%)**. All 17 existing live-export tests passed
  (parent-observed host result; synthetic inert audio representation, no
  playback). Actual Persian RTL/English restoration and Hawaii/Hilo map
  rendering/search/selection were observed; no complete parity or matched-map
  performance certification. See the [bounded receipt](docs/benchmarks/2026-11-05-android-ascii-memory.json).
- Add a single-command physical Android memory profiler with unattended
  process-cold ABBA launches, prepared-article screenshot-reference checks,
  exclusive process/generation attribution, raw memory breakdowns and bounded
  cleanup. Its exercised 16-sample run finished in 188 seconds and still favored
  official Kiwix; it does not establish a memory improvement.
- Retire a completed compressed cluster's decoder workspace while retaining
  every decoded blob for subsequent random reads. Synchronize memory-cost
  inspection with retirement; preserve partial/failed reads, raw-reader
  ownership and LP32 oversized-blob handling. Correct Zstd encoder/decoder
  memory accounting without reducing the nominal 16 MiB cluster-cache budget.
  Strict patch composition and eight focused host-native cases passed,
  including failing-before/passing-after retirement and concurrency regressions.
  Android now requires both corrections in its verified reader SDK; iOS and
  desktop SDK requirements are unchanged. Device results remain separately
  scoped in the [comparison](docs/PLATFORM_COMPARISON.md).
- Remove the compact Android navigation bar's accidental vertical scroll
  container without clipping its controls or changing tablet-rail scrolling.
- Route Android download-notification updates through the retained application
  endpoint instead of an Activity-bound adapter. Keep background updates and
  empty-list clearing rather than suppressing the callback after Activity exit.
- Allocate annotation observers and paint resources only after a validated,
  nonempty highlight stream. Disconnect them on replacement/reset/navigation;
  reconnect retained highlights after BFCache restoration. Empty documents no
  longer create an unused highlight layer or observe every DOM mutation.
  Actual stored-highlight creation, text mutation/recovery, fallback painting,
  removal and real browser BFCache restoration passed; 216 JavaScript tests
  and TypeScript checking passed. An isolated, instrumented Android emulator
  ABBA experiment reduced attributed PSS by **9,339 KiB (4.48%)**. That is not
  a physical-phone gain: the reviewed rebuilt APK was installed and exercised
  on the Samsung, but its separate eight-block comparison still used more
  attributed PSS than official Kiwix. The physical memory objective remains open.
- Replace the ambiguous mobile incoming-file choices with a filename-specific
  **Add to library / Cancel** dialog and plain storage consequences. Offer
  in-place reading only when the current UIKit handoff attests it; otherwise
  offer an explicit managed copy or actual system-file reselection. Native
  authority is still checked when acting. Private/read-only admission stays
  session-only. Humanize the known provider-reselection error without hiding
  other diagnostics. Actual component interactions, 17 iOS document tests and
  strict Android/iOS production Clippy passed. The reviewed iPhone update
  preserved captured existing library/settings and admitted the real Wikipedia
  archive through an automated warm-handoff Add. Article/search benchmarks
  remain separate: the automation stopped at an incorrect archive-title
  selector, not a failed import.
- Share validated HTML buffers across bridge-injection boundaries instead of
  copying each complete document. Scan the bounded head prefix case-insensitively
  without a lowercase allocation; HEAD no longer constructs a discarded body.
  Preserve response bytes, Content-Length, UTF-8 checks, CSP and retirement.
  Real Wikipedia/StreetZim constructor probes and actual HTTP/browser/native
  reading passed. Allocation/constructor gains and faster warm **debug** HTTP
  delivery are scoped in the [response-path receipt](docs/benchmarks/2026-11-04-response-path.json);
  they do not establish lower settled RSS or faster shipping-phone startup/search.
  Rebuilt Android/iPhone updates were installed in place, with independently
  reviewed preservation boundaries and successful normal launches. The APK
  remains below the official ARM64 baseline; no reader functionality was removed.
- Reduce the ARM64 APK below the pinned official Kiwix Android APK without
  removing reader features or locales: **66,981,931 bytes**, versus
  **70,366,062** official and **77,029,419** in our preceding integrated build.
  The Android reader SDK removes only unused ICU currency/time-zone/unit/rule-
  based-number-formatting data trees, preserving every retained member's bytes
  and recording dependency/provenance checks. A fresh SDK build, multiscript
  normalization and real English Wikipedia/StreetZim pathname/retained-FD/
  title/full-text/article probes passed.
  Native exports, signatures and 16 KiB alignment remain verified.
- Share concurrent same-key archive opens through one pending cache slot;
  failed opens remain retryable and invalidation cannot be undone by a late
  initializer. Deterministic regressions fail on the preceding implementation.
  Reserve read buffers only after acquiring their native-handle lock, avoid
  whole-profile copies during reader bootstrap, and cache native search-end
  iterators rather than reallocating them on each loop comparison.
- Load six secondary frontend surfaces on demand while keeping Library, reader
  and authority/persistence services eager. The targeted Android entry shrank
  from 896,336 to 688,286 bytes. Actual browser/native surface and real-archive
  checks passed; **946 Rust tests/16 ignored**, 211 JavaScript tests, platform
  frontend builds and strict host/Android/iOS production Clippy passed.
- Install the optimized Android and signed iPhone updates in place. Android's
  captured profile/native preferences/document grants/runtime permissions were
  unchanged; iPhone's captured state/preferences and archive metadata survived
  explicit OS container/attribute changes. Both launch commands succeeded.
  Neither archive-byte integrity nor fresh physical-reader rendering is inferred
  from these deployment checks; VoiceOver was left off and untouched.
- Retain negative performance evidence: bounded-observer real Wikipedia and
  StreetZim memory cohorts, coarse content-ready starts and a same-SDK native
  search microbenchmark **did not establish a reliable memory or speed win**.
  Those improvement objectives remain open; see the
  [comparison](docs/PLATFORM_COMPARISON.md) and
  [scoped receipt](docs/benchmarks/2026-11-04-reader-efficiency.json).
- Use one release codegen unit without changing features, optimization level or
  unwinding. An isolated ARM64 rebuild reduced the signed APK by 6,766,592 bytes
  (8.07%), from 83,808,299 to 77,041,707; real Wikipedia search/navigation/restart
  and production Hawaii map search/zoom/pan passed. This is not a measured
  startup or whole-app memory improvement; see the scoped comparison receipt.
  The final integrated APK is 77,029,419 bytes (8.09% smaller than the preceding
  optimized build), with the same real-archive checks passing separately.
- Remediate the Rust Clippy backlog without a crate-wide suppression. Preserve
  lock/resource lifetimes and failure precedence, avoid per-worker/per-peer
  context copies, retain non-truncating profile locks and provider/PDF cleanup,
  and match platform/test APIs to their actual consumers. Strict host
  all-targets Clippy (including profiling), Android production and iOS production
  Clippy passed with warnings denied; 943 Rust tests passed, 16 ignored.
  Dependency/native compiler warnings remain separate from this Rust result.
- Defer native full-text database initialization until its first use across
  pathname, retained-descriptor and Windows leased-path opens. Preserve indexes,
  title/full-text results, snippets and source ownership. Twenty-four real
  Wikipedia/StreetZim native probe runs retained identical article hashes and
  full-text results; opening RSS was lower, but post-search RSS/timing was mixed.
  The rebuilt Android release passed real title/full-text navigation/restart
  and map search/zoom/pan. These are not measured Android performance savings.
- Strip non-runtime symbols from release binaries without changing optimization,
  panic behavior or runtime checks. Actual ARM64 APK size fell from 102,420,523
  to 83,808,299 bytes; required dynamic/JNI exports, native reading and restart
  passed. Debug symbols remain; release symbolication needs exact-build artifacts.
- Keep replay-engine links offline: capture native anchor/area URL getters before
  Wombat replaces them. Primary/middle click, previews, context and descendant
  containment use the actual browser target without relaxing origin/CSP/consent
  checks. Real Chrome and Android failed before and passed after; native Back
  and genuine external-link confirmation remain intact. All 211 JavaScript
  tests and TypeScript checking passed.
- Keep optional mobile speech-discovery failures out of article-load errors.
  The speech panel retains the unavailable reason and disables Start; diagnostics
  remain recoverable. Actual Android first-open, contextual controls, article
  return and saved-article restart passed without requesting playback. Current
  optimized comparison builds include it; physical phone installations predate it.
- Replace the mistaken cross-target report with a pinned comparison against
  existing Apple, Android and Qt releases. Record APK, installed code/data,
  expanded app/distribution sizes and real Wikipedia/production StreetZim
  reading, search and map interactions. Wikipedia Activity Manager launch
  median was 287/543 ms; main-plus-attributed-renderer PSS was 10.0% higher
  for Wikipedia and 18.8% higher for a rendered Hawaii overview. No first-paint,
  whole-system-memory or matched map-restoration ranking; official iOS installed
  storage and Apple/Windows matched runtimes remain unavailable.
- Paint the iOS app through the bottom home-indicator region with
  `viewport-fit=cover`; reserve safe-area space once at each chrome owner.
  Native simulator portrait geometry, dark/light snapshots and trusted keyboard
  editing passed. The signed correction was installed in place on the iPhone.
  After owner unlock, normal launch and the physical portrait-reader screenshot
  confirmed no light bottom strip; VoiceOver was untouched. Post-launch title
  and migration-source differences remain explicit in the preservation receipt.
- Add offline raster artwork to article-link previews. Resolve image paths
  against the redirected article and document base, reject external sources,
  and cap each image at 256 KiB. Actual ZIM/browser rendering and narrow-layout
  checks passed; malformed, missing, SVG and oversized candidates retain a
  text-only fallback. Updated checks passed 943 Rust tests/16 ignored,
  211 JavaScript tests, TypeScript/Vite, Mac and signed iOS device builds.
- Save the selected video, audio or image from its reader context menu as the
  original resource, not an article PDF. Trusted owned-browser downloads retained
  exact names and source hashes; native Service exports also preserved PDF bytes.
- Add the persisted article-link-preview preference and stop losing the next
  focused link inside the former bridge cooldown. Disabling retires queued and
  pending previews; actual opt-out/restart/re-enable and keyboard dismissal passed.
- Report export directory-sync failure honestly after publication, retain the
  completed file, and relinquish temporary-name cleanup ownership immediately.
  Keep the warning intact through error presentation regardless of filename.
  Final checks passed 942 Rust tests/16 ignored, 211 JavaScript tests, frontend,
  Mac build and iOS-simulator library check. Strict warnings-as-errors Clippy
  remains failing on the existing backlog; see [RELIABILITY](docs/RELIABILITY.md).
- Verify Samsung→ARM64 Linux and Mac→ARM64 Linux Nearby on a real bridged LAN:
  genuine discovery, selected archive QUIC transfer, source SHA-256, library
  import and reader HTML serving. Preserve the phone's recorded original fields
  and remove the temporary VM. These are retained-build directional checks,
  not bulk/reverse transfer, Raspberry Pi or release acceptance.
- Preserve live archive-document state during owned same-document Back/Forward
  traversal, including query/path SPA routes, repeated history locations and
  reentrant router replacements. Unowned history falls back to normal reopening;
  an unanswered traversal retires the old document before fresh-origin fallback.
  Actual owned headless-browser scenarios passed; native/mobile acceptance remains separate.
- Refuse archive removal after the profile has latched a post-replacement
  durability failure, before touching file bytes. Active-transfer Pause/Cancel
  remain available. This does not make deletion atomic against a new failure
  or process death.
- Try later numeric Nearby addresses after a connected candidate fails the
  on-link check, closing the rejected connection without weakening locality
  or route authority. A real isolated ARM64 Linux run failed before and passed
  inventory/full-fixture SHA256 afterward; off-link-only access remains denied.
  This is not Linux–Mac interoperability or Raspberry Pi acceptance.
- Reject overflowing segmented-download ranges during history restoration,
  clearing the unusable plan without deleting the retained partial archive.
- Propagate preserved-history synchronization errors before replacing an
  unreadable download journal. A forced backup-reopen failure now retains the
  original malformed journal rather than resetting it.
- Repair the desktop notification-permission compile path without suppressing
  Clippy or changing the mobile foreground/authority wait contract.
- Document project-matched Rust/TypeScript language-server setup and native
  Clippy, test and build commands.
- Correct stale reader/parity wording: Windows/Linux native printer providers
  are already wired through the desktop export path. Current Linux native
  Ctrl+P, Print to File selection, Cancel, reader navigation and reopening a
  second article's dialog passed. No job was submitted; physical output and
  Windows runtime remain unverified. The isolated app was stopped and original
  profile, AT preferences and application focus were preserved.
- Add bounded source-scoped incremental catalog reconciliation across configured
  OPDS sources, preserving meaningful source queries and opaque entry IDs.
  Report partial/stale traversal honestly; omission is not archive deletion.
  Add explicit lineage association, native full-edition preparation and verified
  separate downloads, reviewed replacement and retained-original rollback.
  Migrate live/saved reading references, per-book zoom and unsaved drafts while
  assigning fresh installation principals; do not migrate website/sensor/sharing
  authority. Real official HTTPS replacement/read/rollback passed on the host.
- Add separate **Connect my devices** private reading-data sync using pinned
  invitations, Snow Noise XX and matching-code confirmation on both devices.
  Keep bookmark/annotation and per-device-position consent explicit; public
  archive sharing never carries this data. Causal merges retain concurrent
  versions and deletion/recreation history; explicit resolution checks the exact
  presented alternatives and confirms discarding other versions. Bound sync
  history and provide confirmed reset/position-recovery actions without silently
  dropping active reading data. Real native two-process and two-browser
  pairing/manual/automatic transfer/conflict-consent flows passed. A rebuilt
  iOS simulator also passed native consent, matching-code confirmation and
  private-note receipt from the desktop peer without granting archive authority;
  broader mobile and assistive-technology acceptance remain separate.
- Replace explicit frame/ancestor `inert` hiding in the reader and app shell with
  hidden/display/visibility gating while retaining genuine retirement callbacks.
  Owned WebView controls isolated `inert`-specific native accessibility loss.
  The rebuilt Android product passed the previously failing TalkBack map/detail
  return and reader-tab-panel dismissal paths; rebuilt iOS passed a native
  internal-link scenario. Current Linux passed native map/detail/back reading,
  genuine HTML-dialog announcement/dismissal/focus return and subsequent article
  speech with isolated unmodified Orca 51.1 and AT-SPI 2.62.1, using fresh owned
  profiles/defaults. Older Orca stacks retain counterexamples; a 49.8/newer-client
  comparison ended in a WebKit SIGABRT without an established cause. VoiceOver,
  broader modal and suspend/resume acceptance remain open. No global AT packages
  or production workaround were installed; the original reader was restored.
- Give iOS its own `com.tiltastech.zimreader` application identity and matching
  Live Activity/background-acquisition identifiers; retain desktop/Android IDs.
  Generate both Wi-Fi entitlements from the canonical Xcode project source.
  A real authorized profile, app/widget signatures and physical iPhone launch
  were independently checked; this is not physical Bluetooth/join certification.
- Correct reader Find response correlation and navigation reset, restore live
  panel/dialog focus without stealing intervening focus, keep keyboard-selected
  and surviving tabs visible, and bind history expansion to each mounted
  disclosure. Keyboard and scoped native accessibility evidence do not establish
  complete assistive-technology acceptance.
- Replace competing Nearby library modes with a single public encrypted-QUIC
  **Share all → choose sender → Download all/Download selected** workflow.
  Keep browser/OPDS HTTP separately enabled in Settings, with a visible
  unencrypted-network warning. Remove obsolete invitation-code intake.
- Accept numeric local ULA IPv6 hints in public Nearby BLE connection metadata,
  and publish at most 16 supported hints instead of rejecting a reachable
  dual-stack library. Keep global/remote, mapped, scoped and multicast hints
  rejected. Require actual on-link prefix proof for numeric library connections
  and retained route rechecks rather than treating private address class as
  authority. Samsung Share all/advertising, real inventory browsing and verified
  archive transfer passed after the in-place update. A later owner-authorized
  Samsung→iPhone transfer completed with source-matching bytes and durable
  `quic-library` provenance; its owned test copy was removed with original
  libraries/reading state/settings unchanged. Historical throughput cause,
  reverse-direction and routerless joining remain unproven.
- Persist actual acquisition transport provenance instead of labeling every
  library job HTTP. New QUIC-library jobs retain `quic-library` through pause,
  completion and restart; older synthetic library history stays `unknown`
  without proof. Capture provenance with each job under one lock, preserving
  notification/cleanup behavior without a concurrent-removal lookup race.
- Default mobile Add files/Add folder to durable read-only original references,
  including whole/split archives; keep managed copying explicitly opt-in.
  Preserve actual incoming `.openInPlace` attestation, reference-count duplicate
  handoffs, release rejected handoff authority, and retain source-scoped recovery.
- Repair native route-text ownership, numeric client route receipts, cross-platform
  Apple discovery parsing, selected-interface lifetime, exact-owner retirement,
  and shared hotspot release. Preserve configured Web-sharing scope on both
  enable actions. Runtime acceptance and phone deployment remain separate gates.
- Add **Download all** to Nearby shared-library inventories. Queue missing ZIMs
  through the existing durable download/import path, skip installed and exact
  active-session copies, and report missing library files and partial results.
  Stop adding or closing the view prevents further admissions without deleting
  accepted work; existing consent, route, privacy and Stop all gates still apply.
- Add optional Nearby publisher nicknames with privacy-preserving platform
  defaults, bounded Unicode metadata and visible publisher/discovery codes in
  receiver rows, Choose's accessible label and the selected inventory.
  Equal names never merge publishers or become transfer authority.
  Save only explicitly submitted names; retain failed writes for retry, preserve
  newer typing across delayed completion/navigation, and order new publications
  after accepted name saves without letting Stop all revive deferred starts.
- Restore inline-video position after background quiet-origin retirement using
  one bounded, ephemeral current-tab checkpoint. Reauthenticate the replacement
  reader and match policy, installation/source, article and descendant occurrence
  before restoring paused; never automatically restart playback or reuse native
  authority. Recognize native WebKit picture-in-picture presentation.
- Bind Android provider-move deletion to the digests of the copied parts, not
  just coarse timestamps and file identity. Preserve the original and durable
  receipt when bytes change; refuse later digest rebinding.
- Admit resolved iOS official-download sizes against native staging and final
  copy capacity before payload transfer. Keep completed-receipt adoption local,
  preserve recoverable post-rename stages, and acknowledge escalated cancellation
  before reporting completion. Report native download rate/ETA; this is not a
  claim that slow observed iOS background throughput has been resolved.
- Honor Rust-shaped Android article-PDF cancellation by its exact `jobId`,
  retiring only that renderer/copy. Omit non-rendering audio and capture visible
  audio controls as inert, source-free content without weakening the native
  export sanitizer.
- Keep voice-recognition continuation independent of lifecycle reconciliation
  and preserve exact-owner native speech on valid foreground return.
- Report temporarily unavailable provider storage locally with reconnect/reload
  guidance; clear only the matching transient notice after authenticated reading
  recovers. Preserve genuine grant-revocation and unrelated failure notices.
- Resolve updater trust keys inside admitted release steps, and invoke Tauri's
  declared JavaScript signer through Node rather than spawning a Windows `.cmd`
  shim. Disposable-key signing/staging is exercised on macOS; Windows execution
  and protected GitHub environment acceptance remain separate gates.
- Keep valid hard-linked archives readable and searchable through retained
  descriptors, without granting relocation or deletion authority over shared
  inodes. Preserve all links and unrelated installations when refusing cleanup.
- Honor supported bandwidth limits during Nearby archive-identity preflight
  without counting intentional budget waits as stalled I/O. Try validated
  advertised library addresses and bind grants/downloads to the successful route;
  retain cancellation, session withdrawal and unadvertised-destination guards.
- Enable RustCrypto's runtime-checked ARM SHA-256 backend and optimize that
  dependency in debug builds. Profiling found software hashing dominating
  large-download cleanup; checksum, ownership, private-capture and tamper checks
  remain unchanged.
- Accept complete uppercase/mixed-case split archives through Android's external
  folder picker while preserving exact provider names/URIs. Keep rejection of
  gaps, duplicate case aliases and different archive bases; preserve reading
  state through missing-file and revoked-grant recovery.
- Repair iPhone archive sharing rejecting its own export scratch directory with
  `outside_app_storage`: compare lexical descendants against OS-owned app roots
  and their POSIX spellings, not existence-dependent Foundation normalization.
  Keep descriptor-relative no-follow traversal and identity-checked cleanup.
  Do not claim a prepared export was retained when sharing an original archive.
- Keep one DNS-SD library instance per publisher across share-session restarts,
  instead of allocating another discoverable name each time. Session credentials
  still rotate. Give each announcement its own withdrawal handle so a delayed
  Stop/cancellation cannot remove its replacement. Different publishers remain
  separate; identical display labels are not used as a deduplication key.
- Check Nearby archive identities before creating a transfer or staging files.
  Whole-library inventories show **In library → Open**; selected-file shares
  inspect bounded verified ZIM-header chunks first. Unavailable existing entries
  lead back to Library rather than copying an archive that cannot be imported.
  Native start-time checks remain independent of the UI, and final UUID-collision
  rejection remains authoritative for concurrent imports or misleading peers.
- Reduce redundant download-cleanup verification without dropping ownership
  checks: empty library-removal claims defer byte verification to final deletion;
  HTTP cleanup verifies the full checksum after private capture instead of also
  hashing the public path. Bound growing-file checksum reads and check LocalSwarm
  retirement between chunks. Show working feedback while cleanup is active.
  Actual Android receive/import/read/delete preserved an unrelated installed
  archive; these changes do not establish the cause of the reported Hawaii delay.
- Make the normal nearby flow **archive Details → Share**, then receiver
  **Nearby → Download → Open**. Opening Nearby starts receive-only discovery;
  restored surfaces/restart/private/read-only do not manufacture that intent.
  Keep explicit Stop; collapse library URLs/IPs/QR and Apple route-order preference
  into Manual connection details. Apple AWDL discovery was already enabled with
  the old switch off. Initialize Android's observed no-network state as offline
  and give actionable Wi-Fi/hotspot guidance instead of a download-status error.
  Actual two-instance Share/discover/QUIC/download/import/read/hash/restart passed;
  native Android exercised radio-off/re-entry/Stop/privacy and the Share action.
  BLE credential bootstrap and automatic maximum-speed route choice are not claimed.
  Active offers/incoming archives precede setup; native 320 px acceptance showed
  the selected archive and Stop sharing on the first screen without overflow.
- Add one-click **Nearby → Share all**: explicit open whole-library sessions,
  automatic physical LAN IPv4/port, one library advertisement, native browse/
  download/import, optional selected installations/PIN, browser Download links
  and clear Stop. No automatic startup sharing, global LAN HTTP exception,
  redirect following or BitTorrent cutover. Local grants bind exact discovered
  session/peer/route and retire with acquisition ownership; persisted parts need
  reauthorization. Android retains explicit sharing through its foreground
  service without demanding notification-display permission; iOS keeps open
  library sessions/receives awake in the foreground and retires HTTP on background.
  Actual two-instance open/protected/selected acquisition and reading, wrong PIN,
  browser links and Stop/withdrawal passed. Nonfatal discovery failure keeps an
  actionable access-link notice; raw details require Show technical errors.
  Physical phone-pair/routerless/AP-only and background acceptance remain distinct.
- Expose BitTorrent beside normal catalog downloads, move URL/magnet entry to the
  top of Downloads, and keep torrent upload statistics and seeding controls
  outside Details. Normal downloads remain HTTP; continued seeding stays opt-in.
  Official catalog entries lazily resolve publisher `.magnet` responses through
  bounded, provenance-checked HTTPS requests with privacy/lifecycle cancellation.
  Actual one-click official acquisition downloaded a 317,149-byte ZIM, imported
  it and opened readable documentation. Controlled real-engine magnet/.torrent,
  pause/resume and seeding confirmation checks also passed.
- Canonicalize the trusted desktop profile root after acquiring its process
  lock so `/tmp` aliases can publish torrent metadata. Descendant symlinks remain
  rejected; the regression exercises both successful metadata publication and
  refusal to write through an escaping child link.
- The sharing review reproduced missing whole-library discovery and rejection of
  LAN HTTP acquisition; the scoped library-session implementation above repairs
  that producer/consumer gap without weakening ordinary Internet acquisitions.
  Local BitTorrent was not adopted: no measured speed/ease benefit justifies its
  additional discovery, network-binding and public-announcement constraints.
- Repair the 26 findings from the 2026-11-01 review: exact migration rollback
  ownership; full replacement verification before original deletion; preservation
  after committed Store errors; saturated LocalSwarm progress; locked reconnect
  admission; retired deleted-download import authority; atomic torrent metadata;
  durable manual pauses; joined verification sweeps; correct map resume/active
  states; missing-entry document identity; cross-archive suggestions; original
  font restoration; iOS speech transports and stale Live Activity cleanup;
  read-only marker/crash policy; complete watcher observations; serialized power
  transitions; valid OPDS XML; unique, freshly authenticated update artifacts;
  updater/transfer exclusion; archive Home shortcut; nested media ownership.
- Add official-catalog-only iOS background URLSession with backend provenance,
  offline receipt adoption, authoritative pause/cancel and joined privacy policy.
  This restricted path uses user-approved OS redirect policy; custom URLs keep
  strict foreground transport. Physical OS-relaunch delivery is not certified.
- Add safe stale Apple bookmark refresh, title-inclusive default search, native
  shell text scaling, persisted macOS native window groups, current-page PDF
  capture, Android selected-file copy/move, external archive entrypoints,
  forward/timed speech, per-app/widget language, Android PDF sharing, LAN article
  search and split/provider ranged acquisition, and Windows/Linux native print
  selection. Android drawing-based PDFs may rasterize and do not claim native
  print-engine semantics. Platform acceptance remains separate.
- Stage release artifacts by target/version, verify exact signed source bytes
  when not signing, and provide a protected production release workflow. Base
  updater configuration is intentionally unconfigured until real release keys
  and channel inputs are supplied. No production distribution is claimed.
- Expand offline catalogs to 1,296 sources across 74 locales: 59/74 catalogs
  are string-complete. The 15 incomplete locales are `dag`, `gsw`, `ie`, `igl`,
  `io`, `kab`, `nqo`, `sc`, `sdc`, `skr-Arab`, `sms`, `sro`, `zgh`, `zza`,
  and `roa-Tara`; English fallback remains pending competent exact-language
  translation and native review.
- Bind finite durability checkpoints to exact ID/generation and a 1,800 ms
  deadline. Terminal `KIWIX_SUSPENDED` (`retry:false`) cancels without enqueue,
  replay or stale banners; genuine persistence failures remain visible.
  Reader navigation awaits cancellable actual storage admission. Android
  `onPause` reports inactive; `onStop` suspends; foreground differs from visible.
- Bind PDF capture to live URL identity with equivalent percent-dot spelling,
  corrected descriptor ownership and localized controls. Android has a visible
  cancellable preview. Android actual picker/admission/read/lifecycle checks,
  PDF recipient rendering and typed preview cancellation passed. Latest iOS
  simulator install/launch and maximum-Dynamic-Type
  navigation passed on artifact `22088`: all seven destinations reached by
  native taps/horizontal navigation swipes, without error bands. Physical
  hardware and assistive-technology acceptance remain separate; see RELIABILITY.
- Android native PDF preview/write completed in an actual Share attempt, but
  export handoff failed with `PATH_OUTSIDE_APP_STORAGE`. Android cache bind
  aliases are not unified by canonicalization alone: obtain `cachePath` from
  trusted native `storageRoot` metadata, then create/validate/canonicalize the
  scratch directory there. Native `prepareViewer` uses its canonical cache
  producer; strict export ownership validation is unchanged. Actual emulator
  picker, admission/read, Back, widget and background-return checks passed
  without new stale checkpoint/storage bands; the initial PDF error remained
  visible until the repaired artifact was installed.
- Actual iOS simple-fixture PDF sharing reached Apple Preview with correct
  Main page/H1 Main (`/tmp/kiwix-final-ios-pdf-recipient.png`). Complex long-page
  fidelity and cancellation remain unverified. After the last Android root
  producer repair, focused host `live_pdf_` tests passed 2/2 (artifact `22165`)
  and the full integrated host suite passed 771, nine ignored (artifact `22172`).
  Latest iOS arm64 device build/IPA export passed (artifact `22173`), without
  physical installation. Android recipient rendering passed on `22209` without
  baked-in native scrollbars; final APK `22227` typed cancellation passed with
  no write/chooser/handoff, no error bands and a readable reader afterward.
  Native localized Cancel was visible. Genuine render/policy/cleanup failures
  remain errors. Evidence: `/tmp/kiwix-android-finalqa-5586/` and RELIABILITY.
  Final desktop asset restoration build passed (`22230`). These checks do not
  certify complex-page fidelity, physical hardware or assistive technology.
- Repair the actual iOS fresh-launch crash from reassignment of non-retained
  `UIApplication.delegate` with a class-level pre-`UIApplicationMain` handoff.
  Corrected simulator runtime acceptance exercised launch, picker import,
  read/Back, background/return, cold restart and native speech controls.
  Evidence: `local://AcceptIOS-acceptance.md` and
  `/tmp/kiwix-accept-ios-evidence/`; this is not hardware audio certification.
  That simulator acceptance preceded the physical-phone deployment recorded below.
- Current recorded verification: **795 Rust passed, nine ignored; 107 JavaScript
  passed; 22 native iOS XCTest and 18 Android JVM tests passed**. Prior evidence does not certify later
  changes. Signing/distribution credentials, physical iOS URLSession
  suspended/relaunch delivery and Windows/Linux printers remain external gates.
  Simulator acquisition failed with unavailable `nsurlsessiond`; successful
  official native download completion is not claimed.
  Verified post-BitTorrent Android APK and iOS IPA builds (`22344`) were then
  installed in place on both phones. Android process launch passed; normal iOS
  Locked policy blocked launch. The official torrent runtime check used the
  native headless host/common UI, not physical iOS background acquisition.


- Fix repeated unmuted iPhone video pauses by leaving archive audio activation to WebKit. The app no longer calls `AVAudioSession.setActive` after media-state reports, reasserts activation on route/category notifications, or sends a second document pause after native interruption notifications. The startup `.playback` category remains; explicit activation is reserved for native read-aloud, whose claim is released only after speech and media are quiet. A failed speech activation stays retryable. Physical iPhone TED WebM playback was confirmed working by the user after this change. Temporary pause hooks, on-page debug overlays and raw media logging were removed after capture. Lock-screen/background audio and headset transitions remain separate, unverified acceptance checks.

- Add a signed desktop update channel. `tauri-plugin-updater` is initialized on macOS, Windows and Linux with a public key and HTTPS endpoint in `plugins.updater`; `requireSignedVersion` binds each artifact to the version its signature names. Settings now has an **Updates** section with the current version, stable-channel note, last/next check, an on-demand check, release notes, explicit install confirmation and download progress. A background check runs at most once a day and never downloads; private/read-only profiles, active HTTP/torrent/LocalSwarm/map transfers, archive moves and a concurrent update operation are refused with plain explanations. `scripts/make-update-manifest.mjs` packages a macOS `.app`, signs every artifact with `tauri signer sign --app-version`, and writes the static manifest. The old “check upstream Qt releases” button is gone; the remaining section checks archive editions. iOS and Android state that they update through the App Store/Play and do not show a dead button. A loopback proof uses a throwaway key to check, download with progress, install into a temporary `.app`, reject a tampered artifact and persist the last-check timestamp; production keys, notarization, a signed Windows installer and the final real-bundle restart remain distributor work (`docs/UPDATE.md`).
- Declare iOS background audio and retain reader servers, privacy generation and archive cache while the trusted root reports audible playback or PiP; retire those resources when playback ends. WebKit owns archive playback activation. `ReaderAudioSession` installs `.playback` before media starts and explicitly activates only native speech; interruption notifications update shell bookkeeping without issuing a second document pause. The bridge pauses non-PiP video on visibility loss and all media on teardown. Physical foreground unmuted TED playback is confirmed; lock-screen/background continuation is not yet certified.

- Put the managed library on a removable card without leaving app-owned serving. Android Settings now asks the native plugin for app-specific storage locations (`getExternalFilesDirs`), shows internal app storage plus each external/removable folder with free and total space, and marks the active root. Choosing another root calls `mobile_library_root_set`, which re-reads that native list before accepting the path. A new `library_root` journal verifies every managed archive through the existing libzim gate, copies the exact part set into a private staging directory on the destination volume, verifies the staged archive again, publishes each part without replacing an existing name, and only then updates every book path and `settings.downloadDirectory` in one atomic Store transaction. The old root is never touched before that commit. Old copies are kept by default; the user can opt in to remove them only after the new copies verify. An interrupted copy or publish rolls back on the next writable start while the old root is still active; an interrupted cleanup finishes when the Store already points at the new root. The switch refuses with “Pause or finish your downloads, then switch storage” while any queued/downloading/verifying/paused transfer or active map job still targets the old root, so no job can keep a destination below a root the store no longer uses. Free space is checked before copying, and private/read-only modes refuse the switch. The architectural boundary is explicit: archives are still served from app-owned paths, so the removable location is this app's own `Android/data/<package>/files/archives` folder on that volume, never a raw SD-card path. The storage summary reports the active root and its volume, and the Android emulator receipt covers selection, copy, reopening an archive and the summary; a physical SD card remains unverified.

- Show the active download on the iPhone lock screen and Dynamic Island. A new `KiwixDownloadActivity` widget extension (declared in `src-tauri/gen/apple/project.yml`, bundle id `org.kiwix.workbench.DownloadActivity`, deployment target 16.4, embedded into the app, `NSSupportsLiveActivities`) renders one `ActivityConfiguration`: a lock-screen card plus expanded, compact and minimal Dynamic Island presentations with the archive name, status, percent, byte progress and a status-coloured progress bar. The content is a projection, never a second source of truth: `src-tauri/src/live_activity.rs` reads the existing `DownloadManager::list()` snapshot once per second from the mobile maintenance tick (also during the background-retirement grace period), follows the most recently updated `queued`/`downloading`/`verifying`/`paused` row when several transfers run, updates on status/byte changes, ends the activity with a final state when the tracked row reaches `complete`/`failed`/`cancelled`, and dismisses it immediately when the row leaves the Downloads list. Private mode, read-only mode and unavailable native notifications clear the surface and publish nothing; the ActivityKit bridge (`plugins/tauri-plugin-kiwix-mobile/ios/Sources/LiveActivity.swift`, called over the existing Rust-only plugin transport) additionally refuses to start when notification permission is denied and reconciles from `Activity.activities`, so a request after a process restart still updates or retires what the system shows. ActivityKit needs iOS 16.1+ and the app already requires 16.4, so no runtime fallback is added; only island-capable iPhones get the Dynamic Island presentation, while iPads and older phones keep the lock-screen card. Verified: the simulator app plus extension build through `npx tauri ios build --debug --target aarch64-sim --no-sign --archive-only`, the embedded `.appex` reports the matching 16.4 minimum, and a real throttled loopback download started and updated the activity (`ActivityKit` `active`, `WidgetRenderer-Activities` session with Jindo island metrics, `os_log` receipts for every start/update). The projection's selection/terminal/dismiss/suppression/throttle rules have seven host unit tests. Not verified: the visual island or lock-screen capture (this simulator does not composite the island overlay into `simctl` screenshots), the completion/dismiss presentation in a running app, and physical-device rendering/signing of the extension.

- Finish the Android platform entry points with a home-screen search widget, pinned article shortcuts and permissionless voice search. The widget is an `AppWidgetProvider` whose search, **Library** and **Get content** buttons reuse the existing native entry queue, so tapping search opens the library article search from a cold start; the shell publishes only its latest archive-title summary through `mobile_update_search_widget`, and the provider reads that snapshot in the launcher broadcast, never starting the WebView. Private and read-only sessions publish an empty title set (and the provider defaults to hidden), so a previous session’s archive names cannot remain on the home screen. Pinned shortcuts call `ShortcutManager.requestPinShortcut` with a `zim://<uuid>/<path>` link; `AppEntryPoints` queues it, Rust validates/canonicalizes it through the existing `zim_scheme` parser, and the shell opens the article or, when the archive was deleted, lands on the library with a notice. Voice search launches an installed `ACTION_RECOGNIZE_SPEECH` activity and fills/runs the existing privacy-checked search; there is no `RECORD_AUDIO` permission, no in-app microphone session and no custom recognizer, and the action is hidden when package visibility finds no handler. No new permissions were added and LocalSwarm/torrent/maps/hotspot/sharing code was not changed. Android emulator receipts and physical-device limits are recorded in the deployment evidence for this build.

- Treat the mobile suspension refusal as a lifecycle gate instead of a failure. `mobile_admit`, the Android notification launch queue and the notification foreground guard now return `{"code":"KIWIX_SUSPENDED","message":…}`; `user_error::plain` passes that envelope through untouched. `src/api.ts` translates it once into a `SuspendedError`, waits for the native `kiwix-mobile-active` event (with a visibility fallback), and retries the command exactly once when the foreground gate is open. `reportError` neither records nor renders a suspended command, component error text is empty for it, and `LocalSwarmControls` checks the typed error instead of matching the sentence. A request whose privacy generation changed while waiting is dropped quietly, and the cold-launch snapshot no longer races its own resume listener.

- Download ZIMs over parallel HTTP ranges and stop paying per-16-KiB overhead. Measurement first, on this Mac against a local 512 MB fixture and the real `download.kiwix.org` mirror: `curl` over loopback carried 3.5–4.1 GB/s, but the release `DownloadManager` reached only **210 MiB/s** (4.63 CPU-s/GiB) while one remote stream sat at **4.11 MiB/s average / 5.06 median** — the same ceiling the iPhone reports. The transfer now reads 256 KiB at a time, batches writes through a 1 MiB `BufWriter`, bypasses the limiter lock entirely when the limit is zero, and keeps speed in memory every second while syncing durable progress every five. When the server advertises `Accept-Ranges: bytes` (or answers a resume with a 206), supplies a strong ETag or Last-Modified, and the object is at least 4 MiB, the transfer is split into four `Range` requests written through independent cursors into the one locked part file. The layout and each segment's synced byte count live in the job journal; every segment sends `If-Range`, so a changed validator or a 200 response restarts from zero rather than mixing representations, and 429/503 responses retry with `Retry-After`/exponential backoff (the mirror rate-limits concurrent connections). A server that ignores ranges falls back to one stream; cancel still deletes only the owned inode; the bandwidth budget is shared across segments; the process lock and ownership journal are unchanged. After: **230–249 MiB/s** loopback (3.68–3.93 CPU-s/GiB) and **12.13 MiB/s average / 14.58 median** remote, 257,669,919 bytes in 20.3 s — about 3× the previous remote rate. Exercised: Metalink SHA-256 + byte-exact segmented publication, pause/resume at durable mid-segment offsets, a validator change restarting dirty ranges, cancel removing only the owned part, four segments sharing a 256 KiB/s cap, and both advertised-but-ignored and absent range support falling back to one stream. The physical iPhone still needs the user's own run: build `npm run tauri build` (or the existing iOS lane) with this change and download a large archive over Wi-Fi and cellular; the Mac numbers cannot certify the phone's radio.

- Never leave a fully downloaded archive at “Downloading 100%”. A new durable `verifying` row state is written before the checksum pass, shown as **Verifying…** with a full bar, and accepted by restart validation. Once every journaled byte is synced, a foreground/background transition no longer interrupts the local SHA-256/publish pass (explicit pause, cancel and privacy/read-only revocation still do), and a bounded background wait never restarts the pass. The post-hash dev/ino/size/mtime/link identity plus digest and outcome are persisted as a verification receipt; a restart reuses it only while the part inode is unchanged, otherwise it hashes again. Startup and a five-second live sweep (plus foreground resume) finalize a fully durable row whose worker is gone, or mark it failed with resume/cancel; a changed identity is never published from a receipt. Covered by projection tests and Rust tests including the exact wedged device row, a real segmented transfer backgrounded mid-finalize, and a one-hash-pass counter. iOS simulator backgrounding during finalize was not re-run in this lane.

- Stop serving an archive the library no longer contains, and close the retry/release gaps around the secondary transfer surfaces. Selected-scope LAN sharing keeps a strong archive handle, so `remove_book`/`relink_book` now refuse while that archive is shared (matching the existing “Stop sharing this archive” rule) instead of leaving the file readable to LAN clients after removal or relinking. Mobile duplicate-intake cleanup deletes only the duplicate’s exact staged path or its checksum-bounded part cohort after re-checking the archive identity, so a split set or differently cased copy admitted in the same batch is never destroyed and a `.ZIM` duplicate is no longer leaked. Forgetting a terminal LocalSwarm row releases its finished worker slot, so the next Stop or private/read-only switch no longer fails with “LocalSwarm job not found”. A map partial whose ownership was never proven is reclaimed once its named file is gone; preserved records move to the next job’s recovery warning list and no longer veto every later map, and `map_forget_recovery` can drop an unprovable warning without touching the file. A hotspot start that returns unusable credentials now releases the native reservation it still owns, `sharing_stop` always releases the hotspot even when the listener shutdown fails, and hotspot address selection ranks the AP link above the client Wi-Fi link by excluding the default route (and preferring `wlan1`+ over `wlan0` when route evidence is unavailable).

- Resolve the macOS bundle from the cargo target directory that was actually used. `npm run bundle:macos` passed the hardcoded `src-tauri/target/release/bundle/macos/Kiwix.app`, so a build with a private `CARGO_TARGET_DIR` finished successfully and then failed in the packager with `ENOENT … lstat`. The script now defaults to `Kiwix.app` under `CARGO_TARGET_DIR` when set (a relative value is tried from the current directory and from `src-tauri/`) and otherwise under `src-tauri/target`, and names every path it tried when the bundle is genuinely absent. An explicit `APP.app` argument still wins.
- Anchor a repository-relative `KIWIX_DESKTOP_SDK_ROOT` at the repository root. Build scripts run with `src-tauri/` as their working directory, so the documented relative form (`.native/desktop-retained/<target>`) resolved below `src-tauri/` and failed with `Matched patched desktop SDK … No such file or directory` even though the SDK existed. Absolute values are unchanged; the error now says where a relative value is resolved.
- Remove a native-import directory once permanent deletion empties it. Imported archives live in `archives/mobile-imports-v1/<uuid>/`; deleting the last book left the empty directory behind on every import/remove/delete cycle. A successful permanent delete now removes that one empty, canonical-UUID directory and nothing else: non-empty directories, other names and anything outside that collection are left untouched, and trash moves never trigger the cleanup.
- Stop offering a delete that the app cannot perform, and say why. An imported archive is outside app-managed storage, yet its details panel still showed the red delete action with no explanation of what deleting an unmanaged original means. For `managed: false` archives that action is now replaced by a visible note: remove the archive from the library, then delete the original in the file manager. Managed downloads and trashed files keep their existing delete flow, and the delete command itself is unchanged.
- Say that removing a book closes its tab. Confirming **Remove from library** also closed the book's reader tab without mentioning it; the confirmation now adds “and any open tab for it will close”. No tab, store or reader behaviour changed.
- Reduce blank startup windows with an opaque, theme-aware "Starting Kiwix…" placeholder before modules/styles load, replaced on React's first commit. `frontend_ready` acknowledges a usable snapshot; native 10-second and page 9-second watchdogs present diagnostics when readiness stalls. Native window backgrounds follow the saved boot theme. Saved window fields migrate individually to defaults and persist once; genuinely lost records produce a quiet Troubleshooting notice. Snapshot assembly skips/reports individual unreadable records, and Maps has an empty-registry state. These safeguards do not guarantee that every launch succeeds.

These entries describe source changes. The verification section identifies exercised scenarios and remaining boundaries; a passing earlier build does not certify later edits.

- Open and stay inside every readable archive shape. A ZIM with no main page (`getMainEntry()` throws `EntryNotFound: No main page`, common for media- and dataset-only archives) now opens a random article with a quiet "no main page" notice instead of a blocking `Entry not found`; an archive with no articles at all reports the translated, recorded "This archive has no articles to open." notice instead of exposing a libzim metadata entry, and "Random article" uses the same recoverable path. A stored location with a malformed percent escape is treated as a navigation miss, so the user lands on the fallback document with the existing notice instead of a permanent error tab. In-frame navigation to an absent entry no longer freezes the reader for 20 s: document destinations get a `307` to the fallback page with a `kiwix-miss=1` marker that the shell turns into a quiet notice (media/fetch requests still get `404 Entry not found`; other archive failures now return a generic `500` and are recorded in diagnostics instead of masquerading as a missing entry). HTML documents above the 16 MiB single-read limit are streamed: the bridge is injected from the first 64 KiB and the remainder follows in 1 MiB archive ranges, with a bounded prefix buffer and unchanged per-read caps. The reader clears the connection-watchdog band when the same frame finally reports `ready`. Explicit **Restore previous session** now pushes saved tabs into open-but-empty native windows (an event, protected from a racing empty `save_session`) instead of silently skipping them, and surfaces failed windows as a blocking error while skipped windows are recorded as recoverable.
- Make error surfacing a user setting instead of painting a red band for every partial failure. Failures now carry a call-site severity: **blocking** (unreadable or corrupt archive, storage handoff/acknowledgement, privacy/authority refusal, disk full, a download/import the user started, quit/close) always renders; **recoverable** (a navigation target that is not in the archive, visit/search bookkeeping, optional metadata/illustration/suggestion refresh, remembering a dialog directory, sensor/capability activation the reader can retry, a subresource the archive server answers 404 to) renders only when the new default-off **Show technical error details** setting is on. Recoverable failures are still recorded in a bounded 50-entry in-memory list with a copy action under Settings → Troubleshooting; nothing is written to disk, sent over the network, or persisted in private mode. `archive::is_entry_not_found` keeps the Rust taxonomy for the one structured case: a stale tab path or `zim://` location opens the archive's main document with a quiet inline notice instead of `The operation could not be completed.` over a blank frame. Severity is never inferred by matching user-facing text; `src/error-visibility.ts` holds the converted-site table, `src/diagnostics.ts` owns the record/visibility decision, and `Settings.show_technical_errors` persists with the other preferences. Node regressions cover classification, default/off persistence, recording versus rendering for both classes, and the 50-entry bound; `cargo test --lib --features localswarm-profiling` passed **576 tests, 6 ignored** and `npm run build` passed. Runtime proof on macOS used a real archive and a truncated/corrupt archive: a missing entry showed the main page plus notice with the setting off and a band with it on, while the corrupt archive still showed its blocking band with the setting off.
- Let a second launch activate the instance that owns the profile instead of aborting. `tauri-plugin-single-instance` keys on the bundle identifier, so a dev build and a bundled build sharing `~/Library/Application Support/org.kiwix.KiwixWorkbench` both started; the loser then failed `store.lock` inside Tauri's setup hook, which panics (`Failed to setup app: ... Resource temporarily unavailable`) and SIGABRTs after the config window exists. Desktop startup now claims the profile lock before the builder (`store::claim_profile_lock`, a typed `ProfileLockError` that distinguishes a live owner from a real open failure) and the store adopts that descriptor instead of locking twice. On Unix the owner also binds `<profile>/instance/rendezvous.sock` (directory 0700, socket 0600, peer uid checked with `getpeereid`/`SO_PEERCRED`): a second process sends one newline-delimited JSON `activate` request with its argv/cwd, the owner comes forward and applies the payload through the existing `parse_options`/`validate_secondary_launch`/`open_paths` path, and the second process exits 0. When no owner is reachable (older build, headless owner, failed listener, non-Unix), the second process shows a native dialog naming the profile, says another instance is running and points at `--data-dir`, then exits 0; nothing is written to the profile. The identifier-keyed plugin is kept only on non-Unix, and a store open that loses the profile there returns a typed `StoreOpenError::Busy` that setup turns into the same dialog instead of a panic. Headless startup claims the profile the same way; on Unix a second headless launch also activates the owner or explains the conflict and exits 0. A read-only profile publishes no socket and gains no `instance/` directory, because that startup may not write to the profile at all. The lock keeps flock semantics (SIGKILL releases it) and stale socket files are replaced only by the process that holds the lock. Exercised on this Mac with two debug binaries embedding different bundle identifiers and the default profile: activation exited 0 and brought the owner from Finder to the front, a ZIM path forwarded over the socket landed in the owner's library, an owner predating the socket and a renamed socket both produced the dialog with exit 0, and SIGKILLing the owner let the next build start normally and rebind over the stale socket. `cargo test --lib --features localswarm-profiling` (579 passed) includes the activation→dialog decision, the typed busy answer at the store boundary, and the 0700/0600 endpoint; `npm run build` passes. Remaining boundary: builds older than this change have no profile socket, so they can only be reported, not activated; cross-bundle `zim://` Apple Events are not forwarded (same-bundle links still reach the owner through `RunEvent::Opened`).

- Open the archive a launch actually asked for. An archive opened from the command line, Finder/`RunEvent::Opened`, a secondary instance or the headless preview queued its first tab with `mainPath` encoded twice: `open_paths` read the shell payload's `mainPath` back into a `Book`, so the already-encoded location sat in `metadata.main_path` and was encoded again (`index%2Ehtml` became `index%252Ehtml`). The reader answered `Entry not found: Cannot find entry` over a blank frame plus the global red banner, and because `save_session` persisted that tab the failure returned on every later switch back to it. The payload now has one boundary: `store::ShellBook` is the only reader of a shell entry and exposes `mainPath` as an opaque location, while `Book::shell_value` stays the only raw-to-location encoder (the read-only launch branch now emits the same shape). A navigation target that genuinely does not exist — a stale bookmark, an article the archive dropped, a legacy double-encoded session tab — now opens the archive's main document and returns `missing: true`, which the reader reports as a quiet in-reader notice instead of the banner; the frame always has content, the resolved path is published so the miss is never retried in a loop, and an unreadable archive (including a missing main document) still fails loudly. `tests/fixtures/main_path.zim` carries a main path with a space, a literal `?`, `#`, `%` and non-ASCII text.

- Give macOS native window tab groups and an explicit session restore. A new `native/tabbing.mm` bridge fills the gap Tauri leaves: it enables AppKit automatic tabbing before the first window, sets `NSWindowTabbingModePreferred` and one shared `org.kiwix.workbench.reader` tabbing identifier on the main window and every later shell, and installs a native **Window** menu whose responder-chain actions are Show Previous/Next Tab (`Cmd+Shift+[` / `Cmd+Shift+]`), Merge All Windows, Move Tab to New Window, Show Tab Bar, Minimize, Zoom and Bring All to Front. The model is deliberately two-level: the native tab bar groups windows (one native tab per window) while the in-window strip stays the reader-tab model, so `Cmd+T`/`Ctrl+Tab` keep their reader meaning and `Cmd+Shift+N` opens a window that joins the group; window titles follow the active article so native tab labels are distinguishable. **File → Restore previous session** calls a new `restore_session` command over `last-session.json`, written atomically (0600), kept within one second of the live session and rewritten at a graceful Quit (AppKit's `terminate:` Quit path bypasses Tauri's exit event), and limited to the windows that were open: it reopens those windows, adopts the saved tabs/active tab/reader surface/geometry into an empty invoking window, never replaces live reader tabs, and is refused in private mode, in read-only profiles and when nothing was saved. Startup restore is unchanged (the `restoreSession` setting, on by default) and its Settings text now says windows and tabs. Limits: native tab-group membership is not persisted (restored windows open ungrouped with the tab bar visible and can be merged again), the native Window menu is English, and Windows/Linux behaviour is unchanged.

- Find ZIM files already on the device and read them where they live. Android Library now has an **On this device** panel backed by `mobile_device_scan`: a native `DeviceZimScan` walker queries MediaStore, walks primary shared storage where Android 12L and earlier grant all-files read, and enumerates every persisted SAF tree grant, and Rust `device_scan.rs` validates every name and opaque token before deduplicating by physical identity: volume + canonical relative path + size + modification time in seconds. The same archive found through MediaStore and a granted folder collapses to one row with merged provenance; split sets are grouped only when their `zimaa…` parts are complete. Rows show size, volume/location, split and in-library markers, and an unreadable marker when a provider cannot yield a seekable regular file. **Open in place** mints the same native provider reference the explicit picker uses (content URIs or a validated shared-storage path), so libzim reads through the existing retained-descriptor path with no copy; **Import a copy** is an explicit fallback, never automatic. **Add folder access** persists one SAF tree grant for later scans. Private mode refuses scanning and folder grants; read-only allows both without saving a reference. No network request and no all-files permission were added: `READ_EXTERNAL_STORAGE` (maxSdk 32) is requested only where MediaStore can still grant shared-storage read, and Android 13+ is told to grant folders instead. The existing import/unpack gates, LocalSwarm/torrent/maps/hotspot/sharing and read-aloud/notification lanes are unchanged.

- Let native read-aloud survive Android backgrounding. `NativeSpeech` now starts a `mediaPlayback` foreground service with an active `MediaSession` and MediaStyle notification; play/pause/stop/previous-paragraph actions and lock-screen media buttons re-enter Rust as `speech-action` events and cross the same capability/ownership validation as the UI commands, so the engine is never driven by an unvalidated transport intent. A started (or paused) session is a suspension exemption: the Android background transition keeps the reader origin, privacy generation and archive cache alive, and the first terminal speech state retires them exactly once. Privacy/read-only transitions, article/tab changes, explicit stop, Activity destruction and task removal still stop and release immediately. The reader bridge preserves paragraph boundaries in extracted text, and the reader exposes pause/resume and previous-paragraph transport beside the existing start/stop control. `FOREGROUND_SERVICE_MEDIA_PLAYBACK` is the normal install-time permission Android 14+ requires for that service type; no runtime permission or microphone access was added. APK build and 557 Rust library tests pass; emulator speech/background/notification evidence is recorded separately when exercised.

- Capture crashes locally and let the user decide whether to report them. A Rust panic hook (desktop and mobile) plus native uncaught-exception handlers on Android (`CrashIntake.kt`) and iOS (`CrashIntake.swift`) write one bounded JSON record under the app-private profile (`crash/pending.json`, 0600; directory 0700). The schema carries app version, platform/architecture, phase, operation, thread, a redacted panic/exception message and stack, and — for reader-thread panics — the archive's last validation result. URLs lose query strings and fragments, reader `/.kiwix/<token>/` authority and `zim://` locations are dropped, filesystem paths are reduced to basenames, secret-looking values and long opaque tokens are redacted, and the whole record is capped at 32 KiB. The next launch offers the unreported record in a modal and in Settings → Crash reports with View, Copy, Delete, and Send by email only where the native `mailto:` compose intent exists; sending opens the user's mail app, never a network call from Kiwix. Archive validation (already present in the integrity panel) now feeds the same result into the crash registry so a crash while reading a book says whether that archive last passed. Read-only profiles do not write records. Exercised on macOS: a gated debug panic wrote a redacted record (`/Users/example/...` → `notes.txt`, `?token=` → `token=[redacted]`), a restart returned it through `crash_report`, the browser UI showed it, Copy placed the JSON on the clipboard, Delete removed the file, and the integrity panel reported the good fixture as verified and a one-byte-corrupted copy as failed. Native Android/iOS handler compilation and device execution are not part of this desktop run.

- Serve the library over the Android hotspot the phone is hosting. `sharing_start` now accepts `binding: {"mode":"hotspot"}`: the listener binds the live LocalOnlyHotspot access-point IPv4 address, resolved from the interface table and ranked so cellular, Wi-Fi Direct, tunnel, loopback and public addresses can never become the address a peer is told to type (vendors differ: `ap0`, `softap0`, `swlan0`, `wlan1`; the emulator reuses `wlan0`). The Nearby hotspot card shows the tokenized access link and a QR for it beside the join credentials, Settings preselects the hotspot binding once one is active, and leaving Nearby no longer tears down the link a joined peer is using. The server and reservation now release together on explicit Stop hotspot, Stop sharing or Stop all, and are retired by background retirement, privacy/read-only transitions, platform hotspot loss and shutdown; a loopback share still survives local-network loss, while a LAN-bound server is retired when its link goes away. Verified on emulator-5562 with Wi-Fi disabled: hotspot started from the app, sharing bound to `192.168.168.209:8080`, the index returned HTTP 200 with both fixture archives over that address, and both the server and reservation retired on either explicit stop. A persistent serving notification is not implemented; no new permission was added. Evidence: `.native/hotspot-serving-20261031/`.

- Add Android platform entry points and small reading-flow settings. The manifest now registers `ACTION_PROCESS_TEXT` (text/plain) and `kiwix://library` / `kiwix://search?q=…`; a new native `AppEntryPoints` queue stores PROCESS_TEXT, deep-link and dynamic-shortcut intents until the trusted shell is ready, Rust drains them through the existing lifecycle snapshot/event bridge, and the shell opens the library, prefills and runs article search, opens a tab, or opens the catalog. Dynamic launcher shortcuts for **New tab** and **Get content** are installed on Activity creation. No new Android permissions were added. Reading list gains a confirmed **Clear reading list** action that removes bookmarks, notes and tags through the existing per-bookmark command and privacy generation; Settings gains a persisted **Open links in a background tab** preference honoured by reader link actions; the reader status bar gains a **Back to top** control. A home-screen widget, pinned-article shortcuts and voice search are not implemented in this change (widget/pinned shortcuts need launcher UI work, and voice input would either require `RECORD_AUDIO` for `SpeechRecognizer` or an external recognizer activity).

- Print to a real macOS printer instead of only saving a PDF. A new `print_pdf` command reuses the admitted article-PDF renderer and then presents the validated file in PDFKit's `NSPrintOperation` with the real system panel (job title and page count shown, printer/page-range/copies selectable, and Save as PDF still available inside the panel). The command shares `save_pdf`'s privacy generation, window/installation authority, one-use commit and private temp-directory ownership; the panel is bounded and the temp PDF is removed when it finishes or is cancelled. iOS is unchanged because upstream has no print button there (the share sheet is the only handoff), and Windows/Linux keep the existing webview print path because WebView2/WebKitGTK physical-printer integration is neither wired nor verified.

- Gate Android builds on 16 KB page compatibility (upstream #4406/#5202). Measurement showed the arm64 `libkiwix_app.so` already had `PT_LOAD` alignment `0x4000` (NDK r28 plus the mobile SDK manifest's `-Wl,-z,max-page-size=16384`) and both debug and release APKs already passed `zipalign -c -P 16 -v 4`, so no link flag changed. A new `Verify16KbPageAlignmentTask` in `src-tauri/gen/android/buildSrc` is finalized by every `assemble<Variant>`: it parses each packaged `.so`'s ELF program headers (16 KiB for 64-bit, 4 KiB for 32-bit) and runs the SDK `zipalign` 16 KiB check on the APK, failing the build on either regression. Exercised against deliberately misaligned ELF and zip inputs (both failed as intended) and re-verified on the rebuilt emulator APK, which cold-launched and loaded `libkiwix_app.so` from the APK. No real 16 KB-page device was available; all test devices report 4 KB pages.

- Carry libzim entry paths as opaque, percent-encoded locations end to end, fixing upstream #2671. A path containing a literal `?` or `#` (common in Wikipedia titles) was split as a URL query/fragment, so the reader opened the prefix article or failed. `Book::shell_value` now encodes `mainPath` once for the shell and notification launches do the same, so the library home action, notifications and mobile imports hand the reader an encoded location instead of a raw libzim path; `Reader.tsx` canonicalizes every location reported by the archive frame (`canonicalLocation`, shared with `documentPath`) so bridge, tab, bookmark and server identities match `encode_path`. `server.rs` documents the split-before-decode contract, rejects malformed percent escapes instead of resolving a different entry, and still keeps genuine archive-authored queries/fragments verbatim. Regression coverage: a generated `tests/fixtures/delimiters.zim` with `A/Foo?bar`, `A/Foo#hash`, the `A/Foo` prefix and `A/100% + café` proves search, bookmark, snapshot `mainPath`, reader open and the HTTP route resolve the exact entry while a genuine `?q=1#heading` still works; unit tests cover the encode/split/decode round trip, no double decode and malformed-escape rejection.

- Watch monitored folders live instead of only rescanning them. A new desktop-only `watch.rs` owns a `notify` 8 backend (FSEvents on macOS, inotify on Linux, ReadDirectoryChangesW on Windows; the dependency is not built for iOS/Android), watches the configured monitor directories non-recursively, and settles each touched directory and candidate archive for about one second — with a second stability check on modification time and a bounded pending set whose overflow degrades to a directory re-check. Settled observations go to `service.rs::apply_watch`, which re-checks private/read-only under the privacy read guard and then calls the existing `import_monitored_paths`, so UUID identity, ownership, acquisition-claim exclusion and integrity handling are unchanged; a claimed path is now rejected before the archive is opened. A disappeared file retires its library entry without deleting anything, except while the book is open in a tab, matching `kiwix-desktop`. Unchanged failed imports are not retried until the file version changes, and the 15 s periodic scan stays as the fallback for backends that emit no events. Verified on this Mac with the headless binary: dropping `small.zim` into a monitored folder produced the entry in 1.15 s, deleting it removed the entry in 1.27 s, a 4-chunk slow copy produced no entry while writing and imported 1.16 s after the last byte, and the claimed-download integration test kept the watcher from adopting the downloader's destination before its verified import. `npm run build` and `cargo test --lib --features localswarm-profiling` (527 passed, 6 ignored) are green.

- Add type-ahead title suggestions to the Library article search. A 120 ms debounce calls a new read-only `suggest` command that runs libzim's per-archive suggestion searcher (`search_cancellable` in title mode, snippets disabled) over the archives the user explicitly selected, and pages book-major with an opaque continuation token bound by digest to the prefix, resolved archive scope, archive cursor and local offset; a stale or replayed token is refused instead of appending a mismatched page. The popup shows title + book with `role="listbox"`/`option` and `aria-activedescendant`, keeps focus in the input (`mousedown` is prevented, Enter with an active row never submits the form), supports arrow up/down, Enter to open, Escape to close, click to open, and fetches the next page on scroll or arrow-past-end. Requests reuse the existing search owner so a new prefix or a closed window cancels the previous page, and privacy/read-only transitions retire them with the other searches; nothing is recorded or sent over the network, and no suggestion ever carries article text. With no archive selected the popup stays closed, matching `kiwix-desktop`'s selected-ZIM requirement. Verified: `cargo test --lib --features localswarm-profiling` (new token, cancellation, paging and fixture-backed dispatch tests) and `npm run build`; the popup itself has not been exercised on a physical device.

- Serve the whole library, not only ticked archives. Settings → Local library sharing now has an explicit **Entire library** choice beside **Selected archives**; starting it requires a distinct confirmation and sends `scope: "library"` to `sharing_start`. The library index and an OPDS catalog (`/catalog/v2/root.xml`, `/entries`, `/categories`, `/languages`, plus `application/x-zim` `/download/{uuid}` acquisition links) live behind the same unguessable path token, and each book gets a listener lazily on first use so root-relative assets keep working. Every request re-reads the library record, requires the installation/path recorded at host creation, and refuses a missing, replaced or relinked file with 410 rather than serving stale bytes; ranged downloads reuse the bounded archive cache keyed by the size/mtime stamp, so a Range request does not re-open libzim, and archive responses now send `frame-ancestors 'none'` so another same-site port cannot frame reader content with its cookie. Private mode now rejects starting sharing, and existing privacy/read-only, shutdown and mobile-background retirement still stop it. Verified with two fixture archives over curl and a multi-thread integration test: index, root/entries feeds, activation cookie, `index.html`, ZIM download, 410 after deleting one file, and connection-refused on the index and both book ports after stop. Not covered: Windows/mobile runtime, third-party reader interop, HTTPS or authentication.
- Host an offline Android Wi-Fi hotspot for iPhone transfers. Nearby now offers **Create hotspot**: the Android plugin calls `WifiManager.startLocalOnlyHotspot`, keeps the reservation alive in the plugin, and returns the platform SSID/passphrase; Rust owns the state and renders `WIFI:T:WPA;S:…;P:…;;` plus the plain credentials. The iPhone joins from Camera/Settings — the UI says plainly that iOS does not join automatically — and then uses the existing discovery/QUIC flow. The reservation is released by Stop hotspot, Stop all, leaving Nearby, background retirement, privacy/read-only transitions, network-policy retirement and shutdown. A system permission or OEM hotspot-warning dialog reports `mobile-inactive` rather than a background retirement; the system-UI marker clears when the start resolves (`onStarted`/`onFailed`/`onStopped`) or on resume, and a real `onStop` reports suspension, so a start that shows no dialog cannot leave background retirement disabled. The pending start, its phase-aware status and its Stop stay admitted across the dialog; background retirement cancels a pending start, and a start that resolves after retirement is stopped instead of adopted. Leaving Nearby stops a `Starting` as well as an active hotspot, and the status exposes `phase` so the UI can show and stop a start that lands between polls. When the platform refuses, the plugin reports the real `onFailed` reason (for example the Samsung's `ERROR_GENERIC`) instead of a lifecycle error. An active hotspot is also published as a local network to the native policy, so hosting with client Wi-Fi off does not retire LocalSwarm or the hotspot itself. Permissions are only `NEARBY_WIFI_DEVICES` (API 33+) or `ACCESS_FINE_LOCATION` (older, already declared by the app), plus the `CHANGE_WIFI_STATE` normal permission `startLocalOnlyHotspot` requires. Verified on the API 33 arm64 emulator with Wi-Fi off: the permission prompt appeared, the start survived it and returned `AndroidShare_3585` / `y8fsdtaqvnijv22` with QR `WIFI:T:WPA;S:AndroidShare_3585;P:y8fsdtaqvnijv22;;`, `dumpsys wifi` reported the same SSID, and explicit Stop produced `CMD_AP_STOPPED` with zero SoftAp managers. The Samsung A35 accepted the permission and once hosted (`AndroidShare_1462`), but returns `onFailed reason=2` (`ERROR_GENERIC`) while connected to a Wi-Fi client network; with Wi-Fi off it reaches `onStarted`. An actual iPhone join and transfer remains untested.

- Add an Apple-only, default-off "Direct connection (no router)" Nearby preference. With it on, discovery publishes an AWDL/llw-resolved source when one exists and each Apple dial tries the peer-to-peer adapter first for a bounded 4 s, then falls back to infrastructure; with it off the infrastructure-first order and source choice are unchanged. The transfer job now carries a bounded route receipt (`peer-to-peer`/`infrastructure` plus the native interface/local address when available), shown in Downloads. Measured on the physical iPhone 17 Pro Max seeding OSM Silicon Valley to a Mac headless client: direct mode selected `peer-to-peer`, completed 303,349,513 bytes with a matching staged size in about 20 s (≈14.5 MiB/s payload phase; job sampled 20.4 MB/s). The matching non-direct run was not completed because the phone was needed by its user; Android has no AWDL and hides the preference.

- Stop archives from reaching the webview's own geolocation provider. When native location was unavailable the reader answered `browser: true`, and both the iframe permission and the archive response allowed `geolocation` for the archive origin, so MapLibre raised WebKit/WebView2's own prompt — repeatedly, per context — instead of this app's archive-scoped consent. The iframe now requests `geolocation 'none'`, archive responses send `Permissions-Policy: geolocation=()`, and a request with no native provider is reported as unavailable. Verified on the Samsung and the iPhone: the first locate tap shows only the app's "Allow location access?" sheet, Block is honoured, and repeating the tap after backgrounding produces no sheet and no web prompt. That decision is remembered per archive for the running session and cleared on privacy transitions, deliberately, so no persistent location grant is stored.
- Keep every map region visible in the Maps filters. `area-groups.json` had no entry for `bahrain`, `taipei` or `thailand`, so the Regions/Countries/Local-areas filters showed 50 of the 53 catalog regions while All areas showed all 53; the mapping is filled in and an unmapped region the catalog adds now stays visible under every filter instead of silently disappearing.
- Make the download Open action a normal-sized button: 52 px and 17 px type instead of 80 px and 22 px. Measured on the iPhone as 374x53 pt versus 374x81 pt before.

- Stop a stale container path from bricking saved transfers. A shared archive record pinned an absolute `share.path`, so an iOS/macOS app-container move (or a portable-profile move before the desktop-only rebase ran) failed the whole `torrents/jobs.json` read with `Invalid shared archive record; database was not reset` — a blocking banner on every launch, including on a physical iPhone. Saved history is now re-anchored before validation: the profile inventory rewrites `share.path` on every platform (mobile relocation and desktop portability), and loading also re-anchors a share to the library entry named by its durable `bookId`/`installationId` plus the id-derived metadata/staging paths, while device/inode/size/mtime evidence is never refreshed. A record that still cannot be validated is no longer a startup failure: the database is renamed byte-for-byte to `torrents/jobs.unreadable-<timestamp>.json`, the profile opens with empty transfer state, the reason goes to the bounded diagnostics log, and Downloads shows a quiet one-time notice with the preserved path (`transfer_history_notice`). Regressions cover a stale absolute share path loading with its transfer intact, a genuinely invalid record (duplicate id) being quarantined intact while the profile opens, and the profile-inventory move of `share.path`. Runtime proof: a headless macOS profile with `share.path` set to `/old/ios/container/Documents/small.zim` listed the share at its real path with no notice; after duplicating the record, `downloads` returned empty, the app snapshot still opened, and `jobs.unreadable-1793480043706.json` compared byte-identical to the corrupted input.

- Share an imported library archive as a BitTorrent seed. `share_torrent` pins the current library file's device/inode/size/mtime, creates deterministic 2 MiB-piece metadata with librqbit, writes the `.torrent` under the profile, starts a read-only seeding session and records an ordinary torrent job with `shared: true`, a magnet URI and the metadata path. The archive is never copied, moved, truncated or written; a changed identity, relink or removal refuses both metadata creation and the seed, and pause, policy retirement, mobile background retirement, limits and shutdown reuse the existing torrent lifecycle. Library exposes the action as a secondary detail button; Downloads shows the magnet with copy/save and an explicit stop. Fixture tests assert a stable info hash/magnet and an unchanged archive, and a second local profile downloads the magnet to a SHA-256-identical archive that imports successfully. No external tracker or third-party client interoperability is claimed.

- Record what did **not** help, so the next attempt does not repeat it. Two parallel connections per source raised the same-host receive path 99 → 128 MB/s (+27 %) but regressed the real Samsung from 10.58 to **4.65 MiB/s** on the same archive, seeder and Wi-Fi lock, so it was reverted rather than kept behind a flag; the phone's limit is shared airtime, not CPU. Switching Quinn's congestion controller (Cubic/BBR/NewReno, host 61.3 vs 62.1 MB/s) was also dropped: the Mac seeds through Network.framework, so a Quinn controller setting cannot affect Mac→phone transfers at all. Final measured rates on the physical phones, sender-timed (first nonzero byte to last partial while a peer is attached): **iPhone 29.7 MiB/s**, **Samsung 10.58 MiB/s with the Wi-Fi lock** (was 0.56 MiB/s before these fixes), against a 46 MiB/s TCP ceiling on the same Wi-Fi. iPhone↔iPhone AWDL remains untestable here (one usable iPhone), and Android has no AWDL transport.

- Hold an Android Wi-Fi performance lock for exactly the LocalSwarm transfer window. The receive path and the outgoing seeder already shared macOS's counter-backed `power_activity::Guard`; Android now backs the same guard with one `WifiManager` Wi-Fi lock (`WIFI_MODE_FULL_HIGH_PERF`, tag `org.kiwix.workbench:kiwix-localswarm`) set through the existing trusted mobile plugin bridge, so the radio stops napping between QUIC bursts while TCP is unaffected. API 34+ automatically replaces the deprecated high-performance mode with `WIFI_MODE_FULL_LOW_LATENCY`, which is what `dumpsys wifi` reports (`type=4`). The lock is released with the last holder on completion, error, cancellation, privacy/read-only retirement and shutdown, and it needed no new permission. Same-archive Samsung A/B (1,073,802,614 B `large-eng-base`, sender-side 1 s counter, first-nonzero→last-partial): 8.17 MiB/s without the lock (no-lock control, 124.4 s) versus 10.58 MiB/s with it (96.3 s), +29.5%; `dumpsys wifi` showed the lock held across the transfer (`low_latency_active_time_ms` 107,390) and empty after release. One run per arm; still below the 201 MB city-eng-base 13.31 MiB/s baseline, so size/ramp/variance are unresolved. The first release candidate silently no-opped because the AppHandle install had been placed in the desktop-only `run_application`; Android boots through `mobile::run()`, so the install moved there.

- Read LocalSwarm payloads whole instead of 16 KiB at a time. With an unlimited rate budget the receiver still reserved and read each chunk in 16 KiB pieces, paying an async mutex, a timer and a wakeup 64 times per megabyte, and the Apple bridge capped every native read at 16 KiB — one serial dispatch round trip per piece. At roughly 1.8 ms per iteration that matches the 5–9 MB/s measured on the phones while the same code carries 58–108 MB/s on a Mac. Unlimited budgets now read the whole payload in one call (a configured limit still reserves per 16 KiB), and the bridge reads up to 256 KiB per completion. Verification note: UI-screenshot timing is not a transfer measurement. The iPhone's own job journal records the 304,161,086-byte German archive as created 04:10:35.562Z and complete 04:10:45.328Z — 9.77 s, 29.7 MiB/s, 291/291 chunks verified and imported 1.2 s later — whereas the automated UI reported 83 s because the Details tap was blocked. Treat job-level timestamps as authoritative and screenshot intervals as upper bounds.
- Retry the first snapshot when the app becomes active. A cold launch could be refused as "The mobile application is suspended; return to the foreground" before iOS marked the scene active, leaving an empty Library and only a "Try again" button; resume now retries the snapshot, clears the error, and the resume listeners are removed with their component. Verified on the Samsung and on the physical iPhone: cold launch reaches a populated Library with no suspension, connection, expiry, menu-command or schema error.

- Find and fix three independent causes of LocalSwarm crawling at ~0.56 MiB/s on a Wi-Fi mesh where raw TCP carries 46 MiB/s. The Mac advertised **every** non-loopback address, so phones dialed its VPN tunnels and VM bridges (`utun0` 172.16.0.3, `utun1` 172.16.128.1, `bridge100/101` 192.168.64/65); only physical interfaces are advertised now, and the Apple transport tries infrastructure Wi-Fi before peer-to-peer. The seeder was throttled by macOS App Nap whenever its window was not frontmost — WebKit logged the page as nappable during exactly the slow stretch, and activating the app tripled the rate — so a single `NSProcessInfo` activity assertion (`UserInitiated | LatencyCritical | IdleSystemSleepDisabled`) is now held by a counter while a transfer is served or received and released when the last one ends. Measured on the phones: Mac→Samsung 304,161,086 B in 54.9 s (5.29 MiB/s end-to-end, 7.7–8.3 MB/s displayed), and a 307 MB archive dropped from 332.9 s to about 34 s with the assertion held; phone→phone measured 6.36 MiB/s. Same-host code remains 92–108 MB/s, so the remaining gap is transport behaviour, not the receive loop, seeder, address advertisement or power management.
- Open search results and Random article with only tab-location fields. Both spread a richer object into the native session schema, so `score`, `snippet`, `url` and `origin` reached the strict validator: search results raised "unknown field `score`, expected one of `id`, `bookId`, `path`, `title`, `scroll`, `back`, `forward`", the article frame stayed blank after backgrounding, and the resulting recovery dialog could not be dismissed with Back or Cancel. Verified on the Samsung: search → Praha renders, background 15 s → return restores the same article and scroll, and an in-page "Česka" tap navigates to "Česko" with no expiry banner, no unknown-field error and no reload.
- Restore reader authority automatically after a privacy change. Background retirement advances reader IPC authority without changing the policy epoch, so a mounted article kept a stale generation: navigation was refused with "Browsing session expired after a privacy transition" and only the manual "Reload article" recovered it. The reader now re-establishes authority under the live generation, preserving tab, article and scroll, and in-page navigation retries once instead of failing silently; an unrecoverable refusal is surfaced rather than swallowed.
- Keep mobile detection independent of how the bundle was built. A frontend built outside the Tauri CLI leaves `TAURI_ENV_PLATFORM` unset, so an iPhone identified as desktop and invoked desktop-only commands, producing "Command update_native_menu not found" banners; the platform now falls back to the webview user agent. A reconcile that reads a background request which native retires first now resyncs quietly instead of painting "Unknown or retired mobile background request" over working content.

- Stop the LocalSwarm receiver from serializing chunk verification with the socket. Each connection now reads one response ahead and hashes it on the blocking pool, so a chunk's SHA-256 overlaps the next response instead of sitting between the socket and the staged write; the chunk's prefetch permit stays charged until the verified handoff, so the 64 MiB per-job prefetch cap, the 32-request window and the 16 MiB per-peer seeder cap are unchanged, and a mismatched hash still retires the source with `LocalSwarm chunk checksum mismatch` before any bytes reach the writer. Same-host release measurements over the Mac's Wi-Fi IPv4 address with 1 MiB chunks: production receive path 92–95 MB/s with the Network.framework seeder (97–100 MB/s before the final durability flush) and 58–60 MB/s with the Quinn seeder, Apple→Quinn pipelined serve rate 108 MB/s, seeder serve cadence 14.9 MB/s at a full 16-chunk window. The debug profile recorded from phone builds is SHA-256-bound: the receive path went from 7.7 to 32–34 MB/s. A new `KIWIX_LOCALSWARM_BENCH_SEED_APPLE` switch makes the receive benchmark serve through the Network.framework bridge. 1 MiB versus 256 KiB send submissions and BBR versus Cubic congestion control were measured and changed nothing on the same-host path, so neither was adopted. Both profiles exceed the 46 MiB/s the Wi-Fi link carries; the remaining physical Mac-to-Android gap is not this Mac's send or receive code.

- Make the LocalSwarm receiver keep its sockets busy instead of committing per chunk. Each connection now runs a persistent worker fed by index batches (`PIPELINE_BUFFERED` 2 → 8) so its request window no longer stops at every 32-chunk scheduling round, with prefetched payload byte-capped at 64 MiB per receive job (charged before the read and released after the write; at most three jobs receive at once). The receive loop no longer re-hashes a chunk the transport already verified against the manifest, and the per-chunk staged-path `unchanged()` checks moved to the durability commit, leaving one seek, one write and one handle fstat per chunk. The durability window changed from 16 chunks or one second (a slow fsync re-armed the interval after every commit) to 128 chunks or 256 MiB, with per-chunk in-memory progress and the fsync, staged-identity check and job-state write running on the blocking pool. The contract is unchanged: the committed checkpoint records exactly the verified prefix whose data was fsynced and whose staged paths still name the written handles, and completion, pause, cancellation and shutdown flush before they return. Release measurements over the production receive path: 64 MiB loopback 81.6 → 87.2 MB/s download (76.9 MB/s including the final flush and teardown); 160 MiB loopback 95.1 MB/s at the old 16-chunk cadence → 108.0 MB/s; with a simulated 300 ms fsync, 160 MiB 24.8 → 62.7 MB/s download (19.9 → 45.4 MB/s flushed); with a simulated 1.8 s fsync, per-chunk commits 0.29 MB/s vs batched 2.2 MB/s. The physical phone's 0.53 MB/s (≈1.9 s per 1 MiB chunk) matches a per-chunk commit cadence; the new cadence is unverified on the phone until the next native transfer.
- Pipeline the LocalSwarm seeder the way the reference engine does. A peer may now have 16 chunk reads/responses outstanding (`SERVE_WINDOW`), capped at 16 MiB of buffered chunk payload (`SERVE_BUFFER_BYTES`); requests beyond that wait in the bounded reader channel and backpressure the QUIC stream, so nothing is dropped and responses stay in request order. The Apple bridge hands every piece of a chunk to Network.framework before waiting, so up to sixteen chunk responses are in flight instead of one send-completion round trip per 256 KiB piece; upload bytes are counted in the native completion callback, and wire framing, chunk SHA checks, upload budgets, cancellation, admission and policy retirement are unchanged. With a 50 ms source-read fixture a pipelined client went from 17.4 to 150 chunks/s (1.14 to 9.83 MB/s) and peak concurrent reads from 1 to 16; a hostile client asking for all 64 chunks and reading nothing peaked at the 16 MiB window instead of buffering the share. Same-host Apple→Quinn pipelined throughput moved from 17.5 to 20.0 MB/s (that path is transport-capped near 20 MB/s here); the physical Mac→phone rate is not yet measured with this build.

- Replace blocking `window.confirm` in every frontend confirmation (mobile folder import, unsaved-session window close, catalog online preview and legacy-Service-Worker download, release-page link, torrent seeding, map cache clearing, LAN sharing start, reading-list removal and history clearing) with one promise-based in-app dialog using the existing modal markup. The dialog resolves false on Cancel, Escape or backdrop dismissal, restores focus, queues overlapping questions and renders outside the app tree so window-close handlers can await it; wording, guards and privacy-generation checks are unchanged. TypeScript and the Vite build pass. The physical phones still run the pre-fix build, so native iOS/Android dialog behavior is unverified.

- Merge the per-platform native-look work with its independent review. Fixes from that review: the iOS search field keeps a visible inset focus ring; the macOS skip link clears the overlay traffic lights; every platform hover rule (including the destructive swipe action) now requires a hover-capable pointer, so a tap no longer leaves a highlight stuck; the 760px RTL reader keeps a draggable region; and the user's System Settings accent tints selections only, so filled primary buttons and their white labels stay readable with yellow, orange or graphite accents. The Android shell-scale policy is preserved alongside the new platform bootstrap. Frontend build and the merged host Rust suite pass (477 tests). Production builds are installed on both phones, and macOS and Linux ARM64 ran native acceptance on the same source; the per-platform limits are recorded in the feature coverage table.

- Add explicit, accessible swipe actions to Library, reading lists, Recently read, Saved Maps and terminal Downloads. Download cleanup distinguishes keeping files from deleting them, confirms installed-archive removal, binds library-archive removal to the current installation identity and every payload unlink to native file-ownership proofs. Hidden retained downloads remain available for later disk cleanup; unknown legacy ownership fails closed.
- Preflight installed map versions before payload transfer and offer Open, Replace or Cancel. Verify the replacement before switching library identity. Preserve map prefixes and verified-block state through Pause, failure, restart and mobile background retirement; display ready/downloaded/reused block counts separately from current-block bytes. Ordinary reader-session retirement no longer invalidates accepted map consent; actual privacy-policy changes still do.
- Scope iOS idle prevention to meaningful map/LocalSwarm work and use the supported finite background task for already admitted transfers. Retain and join retirement owners through expiration; do not claim indefinite background serving. Increase unlimited LocalSwarm send batches to 256 KiB without changing integrity checks or capped budgets. Same-host transport measurements are not physical phone throughput results.
- The integrated frontend build and host Rust suite pass (468 passed, three ignored). Native Android, iOS, macOS and Linux acceptance of this batch is in progress; the batch is not yet installed on either physical phone. The current localization inventory is 1,017 sources across 74 locales: 46 string-complete and 28 explicitly partial, with unreviewed machine translations. The localization audit still fails missing-coverage requirements; no full internationalization-completion claim.
- Diagnose private GitHub Actions run 41088865723: macOS and Linux jobs never started because an Actions budget prevented further use; no compile failure or job logs exist for those jobs. Windows remains gated on its trusted native SDK. Restoring the budget or approving a self-hosted runner is an owner action, not a source-code workaround.
- Prevent accidental Android shell zoom with a native-Android-only root viewport policy applied before React mounts. Browser/iOS zoom, reader text controls and map gestures are unchanged. Real native pinch, double-tap and quick-scale stayed at shell scale 1 on the Samsung; both existing failed-transfer records and verified partial progress remained byte-for-byte unchanged. Emulator acceptance also covered keyboard focus, article size controls, MapLibre pinch, rotation and Activity recreation. The initial WebSettings-only attempt failed real gestures and was removed.
- Keep the native title bar in the application's theme: choosing Light or Dark in Settings now also sets the window theme (app-wide on macOS and Linux, per window on Windows) instead of following the OS. On macOS, windows use an overlay title bar with a hidden title; the traffic lights sit over an empty, draggable strip at the top of the sidebar, the workspace bar is a drag region (buttons and fields still click), double-click zooms, and the strip is dropped in full screen. Right-to-left layouts and the 760px minimum width leave room for the lights inside the workspace bar instead. New windows from `src-tauri/src/windows.rs` match the main window configuration; the shell gained the `core:window:allow-set-theme` and `core:window:allow-start-dragging` permissions, with no capability change for archive origins. Only a simulated browser layout at 1180px, 760px and RTL was checked; no native macOS, Windows or Linux build has run these changes.
- Add per-platform interface skins so the shell follows iOS, Android (Material 3), macOS, Windows 11 (Fluent) and GNOME/libadwaita conventions: system fonts and sizes, palettes in light and dark, button/field/card shapes, navigation selection styles, phone tab bars, bottom-sheet dialogs and switch-style settings on mobile, the desktop arrow cursor, unselectable chrome, thin Windows/Linux scrollbars, and no webview context menu in desktop release builds. `src/platform.ts` sets `data-platform` from the Tauri target (user agent in the headless preview); `src/platform.css` overrides shared shape tokens added to `src/styles.css`. Hover styles now apply only to hover-capable pointers, so tapped controls no longer stay highlighted. Archive content is not restyled. Only Chromium screenshots of Library, Settings and an archive dialog per skin, at 390px and 1180px against a mocked service, were checked; no native webview, device, real platform font or macOS `AccentColor` behaviour has been exercised.
- Correct cold mobile startup while backgrounded before the first session snapshot. A never-hydrated checkpoint now fences pending refresh callbacks and drains preferences plus the existing session queue without writing initial empty defaults; foreground refresh performs the real restoration. Hydrated checkpoints and explicit Quit keep their original flush/privacy checks. Independent applied review and TypeScript checking passed. Actual Android old-APK tracing observed the pre-hydration throw; corrected cold-asleep startup acknowledged background without a session write, then restored all five tabs. The corrected APK was installed on the physical Samsung and Maps/Library/Nearby/Downloads/Home-resume passed without the generic connection banner; settings, the failed Taipei job and staged bytes were preserved, with no automatic Retry.
- iOS separately exercised the cold-bootstrap boundary with an unchanged genuine initial snapshot response held through actual Settings/background and released after native acknowledgment. Two native UI tests passed: no pre-hydration session write, late callback fenced, all four tabs restored by the fresh foreground snapshot, and no generic connection toast. This is simulator evidence, not physical iPhone deployment of that new delta.
- Prepare private source publication: ignore generated caches, mobile `.tauri` API copies, local environment/signing material and `.native/` device evidence while retaining required source, licenses and shared LocalSwarm protocol identity. Document the initial `dist/` build prerequisite for desktop/mobile compilation and the external SDK/toolchain/release gates.
- Start map downloads directly from the map card’s Get map action using its selected language; optional metadata remains available from the title. One actual Bahrain/en click focused Downloads with no dialog, completed/imported 49,236,471 bytes and opened the real offline map. Nearby open and protected shares completed/imported/read with exact source hashes and large Open actions; wrong PIN, Cancel and Stop were exercised against real native responses. Static-asset delivery in that integration harness is explicitly qualified, not counted as native package acceptance. The final Mac package separately proved matching actual WK/headless assets and consent behavior.
- Correct Apple LocalSwarm send-buffer ownership: Network.framework’s send completion means enqueued, not acknowledged. Copy synchronously into dispatch-owned data rather than letting QUIC retain a released Rust buffer; relocate the existing single payload copy instead of adding one. Independent source/SDK review passed. The original bridge corrupts all four delayed-consumer regression cases; the fixed bridge passes all four. Forty-four LocalSwarm tests passed, plus a separately exercised Apple sender/four concurrent Quinn receivers verified 33,554,980 bytes with PIN and chunk SHA checks enabled. The physical Android’s failed 125 MB Taipei manifest/job remain untouched; the exact physical sender is not attributed.
- Retain original reading-history/session authority and gate ordinary autosave/history dispatch and rejection callbacks on actual mobile interactivity and reader-storage admission. Real Android scroll → immediate Home → resume reproduced the stale write before correction and passed afterward. Native-provider permission queries no longer inherit the deliberately denied browser geolocation policy; real Mac MapLibre prompt/Block/reopen/Ask again passed. iOS final native testing passed three UI scenarios, including 16 actual simulated CoreLocation responses, active-watch → Settings/background → resume with retained grant, explicit revoke/deny/reset and confirmed private/read-only roundtrips. Lifecycle-rejected sensor admission and retired native completions return structured cancellation, not false permission denial. The new admission regression also compiled and executed on actual iOS-simulator and Android targets, one pass each.
- Installed the final reviewed Android mobile-admission APK (`9f5df362…`) in place on Samsung SM-A356U; all existing profile-file hashes, including the failed Taipei job/staging, remained exact after install/launch. Installed the development-signed iPhone archive (`69b84010…`) in place on the iPhone 17 Pro Max test device; complete state remained exact with four books/12 tabs and all nine selected payload/capability/job-file hashes unchanged. Both phones are locked: Android process launch is observed, while iOS explicitly denied launch as Locked. No unlock bypass, reset, automatic transfer retry or physical sensor certification.
- Switched the user’s Mac app to the corrected `595b8da4…` package through its guarded normal Quit workflow, not a force kill. The new normal-profile app finished launching as PID15051 with an on-screen window; complete persisted state remained exact. No runtime observer or test-profile override was attached to the user session.
- Make the Library content-first with adaptive book cards and secondary import/filter actions. Discover keeps search and one collapsed filter/sort control above results; map language and Get map share a row, while cache maintenance moves below the map list. Downloads removes the redundant discovery CTA and completed progress bars, retaining an 80px named Open action. Recently read moves into Library actions and deduplicates article identities while preserving the newest full location. Reading/search/settings tools use secondary disclosures without hiding active work, cancellation or errors.
- Add a first-class Nearby tab with explicit Receive/Send, Find nearby, selected-book sharing consent and visible activity/Stop controls. Move endpoints, ports and archive IDs into manual details. Clear receive credentials on endpoint/mode changes; retain original privacy context through protected Resume. Shared frontend Stop fencing plus native inspection/job admission generations reject deferred download/resume after Stop completes, including another root window. Independent adversarial review closed ten findings; the native stale-admission regression passed.
- Actual redesigned surfaces were exercised at 320/430px and desktop width, including Arabic RTL, small/large/filtered Libraries, real discovery/offer/Stop, and a real HTTP download → verified import → 80px Open → article. Native iOS simulator exercised the new screens, real progress/completion, PDF cancellation and a saved 16,434-byte three-page multilingual PDF. The signed consumer build installed in place on the iPhone 17 Pro Max test device with the complete pre-launch state, four books/eight original tabs, capability identity and all three managed payload hashes unchanged. Subsequent user launch and actual new-UI/map capture are observed; newly reported map-session expiry and crossed-out location state remain under correction.
- Correct native PDF path/MIME handling and macOS printing: actual macOS multilingual/image three-page output, 513-page rejection, private explicit export, read-only/overwrite preservation, cooperative policy cancellation and normal Quit passed. Linux ARM64 saved an actual 1,783,099-byte A4 PDF. Android's unsolicited favicon request is handled as bounded producer-owned browser metadata only when not an article dependency; explicit missing resources still fail. Actual Android saved a readable 25,420-byte PDF and cancellation restored its reader. Windows runtime remains unavailable; cooperative limits are not hard native preemption.
- Allow torrent configuration while reversibly suspended without reopening transfer admission. The actual Android post-print lifecycle now saved and restored metered-download consent without the former suspended-manager error, preserving six archives and original tabs/preferences.
- The Android consumer cutover passed actual 320px native screen checks with the populated catalog and saved maps. All six payload hashes, five original tabs, preferences and URI grants were preserved. A historical Unavailable map journal was retained, not erased or counted as a new successful download. APK and source identities are in `.native/android-consumer-ui-20261030/17-final-consumer-acceptance-summary.json`; the subsequent accepted cache-corrected build was installed on the reconnected Samsung as recorded below.
- Actual final macOS consumer screens passed on an isolated, dependency-bundled/ad-hoc-signed debug application using the frozen desktop frontend. Seventeen WK snapshots covered empty/populated Library, loaded catalog/maps, Downloads, Nearby, Settings, Reader, Recently read and saved articles; theme/restart persistence and normal Quit passed. This is not live peer-transfer, notification-delivery or release-package certification.
- Native Linux review exposed failed catalog caching from applying permissions to a cloned `O_PATH` directory descriptor. The reviewed Linux/Android-only correction opens the same retained directory read-only, verifies its identity and applies permissions without ambient path reopening; macOS/iOS keep their existing branch. Actual Android page/facet persistence, conditional revalidation and process-relaunch checks passed with 0700/0600 permissions and all original data preserved. Linux passed 15 existing cache tests and actual native same-profile/unchanged-HTTPS-URL offline reopening in an owned loopback-only network namespace, retaining visible stale-cache warnings and unchanged cache bytes.
- Final Linux ARM64 consumer surfaces passed actual GTK checks at 1280/760px, including Library, populated Discover/Maps, Downloads, Nearby, Recently read, Settings, Reader and native file-picker cancellation. An instantaneous test-input sequence that appeared to ignore Nearby delivered no DOM pointer events; reviewed observation distinguished that harness gap from paced trusted native clicks, which changed the pane correctly. No application workaround or forced navigation was added. Owned sessions quit normally and temporary observers/network namespaces were retired.
- Installed the accepted consumer/cache Android APK in place on reconnected Samsung SM-A356U. Actual Library, cold launch and native Back/warm reopen passed with the same process; the complete prior profile and all preexisting download/torrent/capability file hashes were preserved. The newly introduced completion-notifications preference defaulted off. The user-requested tested macOS app was also launched with its normal default profile and an observed on-screen window, without a runtime observer or test-profile override.

- Separate complete map size from incremental language data. California English is 3,095,000,004 bytes on device, not the former 106,214,997-byte reuse estimate. Selected-map metadata estimates actual ordered chunk transfers without downloading payload; expansion is bounded to 100,000 reads with constant-time cache membership. Primary On device/Download estimate and Get map remain visible; base/reuse information is collapsed. Current 390px LTR/RTL browser and native iOS simulator surfaces passed; no California payload was downloaded for these checks.
- Save Settings automatically with serialized partial writes, bounded text debounce and navigation/background flushing. Keep invalid drafts and failed writes recoverable; root-owned privacy/directory consent and optional migration survive navigation. Actual UI/native-backend checks exercised failed-valid → invalid-draft → Retry, delayed Settings/Reader ordering, confirmation cancellation, retained migration and restart. Native iOS exercised cellular consent → immediate Downloads navigation → process relaunch → retained checkbox, then restored the original off setting; eight archives and 29 tabs survived the in-place simulator update.
- Installed and launched the accepted signed priority location/sharing/sizing/autosave archive on the reconnected iPhone 17 Pro Max test device without reset/uninstall. Actual foreground Maps capture and later same-PID survival passed. Four book records, eight tabs, active tab, existing preferences, reader storage and capability identity were preserved across normal container rebasing; all three managed payload hashes matched. The external provider original was not copied. Evidence `.native/iphone-priority-reconnect-20261030`; this is the pre-PDF priority archive, not acceptance of physical GPS/camera/routerless transfer.
- Permit Apple nearby-sharing attempts without a Wi-Fi access point. Native Network.framework discovery/QUIC includes AWDL, prohibits cellular, validates numeric on-link addresses and binds the selected interface before sending QUIC traffic. Fixed callback capture lifetime, precise ARC receive ownership, DNS subscription retirement, listener-failure propagation and multihomed preflight fallback. Actual native harness observed awdl0 and infrastructure handshakes, rejected an off-link advertisement and joined DNS cancellation; the app received, verified and imported 2,159,190 bytes from the remote M5 Go listener through a truthful owned proxy advertisement. The reference Go hostname's malformed discovery result is not counted as fixed; two-device routerless and physical iPhone 5G acceptance remain separate.
- The final integrated host suite passed **402 Rust tests**, one ignored, after the native PDF, Nearby admission and suspended-torrent configuration corrections. Ten mobile-network policy tests additionally passed as an actual iOS-simulator executable. Actual iOS notification grant/denial, verified completion banner and correct archive tap routing passed; macOS notification delivery remains separately unverified.
- Watch the headless preview's actual `dist/` assets in the native build. Tauri's platform-specific frontendDist does not replace that separate RustEmbed root; stale preview builds are excluded from current UI evidence.
- Keep Apple location acquisition running until a fresh valid fix or the original timeout, instead of failing on CoreLocation's initial stale cached position. Reject malformed/future fixes and keep transient unknown-location updates pending; success, cancellation, permission revocation and terminal errors stop acquisition exactly once. The native regression reproduces stale-first failure before the fix and passes afterward, including real timer expiry. Actual iOS simulator app consent, Apple's Allow While Using App dialog and fresh injected CoreLocation delivery to the article passed without a Reload article error; normal UI cleanup restored the original eight archive IDs, 29 tab IDs and active tab. Physical iPhone fresh-GPS acceptance remains unverified.
- Distinguish unavailable network status, offline Internet, disabled cellular/metered consent and pending connection retirement instead of reporting every denial as a generic network failure. Admission waits for the serialized network preference write; default-off cellular consent remains explicit. Ten native iOS-simulator policy checks and the actual checkbox/relaunch flow passed; physical cellular transfer remains unverified.
- The corrected Android lifetime candidate passed Activity recreation, finish/reopen in the same process, and transfer with no Activity alive for 5.084 seconds. The actual notification Pause action stopped the owned HTTP job; explicit UI Resume fetched the remaining range, verified/imported its 1,109,250-byte fixture and opened it. Four settled native Bahrain renders passed. Evidence `.native/evaluation/android-bounded-map-5560/38-final-lifecycle-acceptance-summary.json`; the candidate excludes later priority UI/map-metadata changes and is not installed on the disconnected Samsung.
- Positive-output Android map cancellation and retry passed on the normal lifetime candidate: an owned partial was observed before cancellation, no result was published, and retry produced the verified 49,691,390-byte Spanish Bahrain map with actual rendered reading. A separate isolated JNI evaluation passed all six 201MB cold/warm/negative modes with observed eviction and an 8MiB final-payload cache bound. Original six archive payloads, settings and five tab identities were preserved; latest priority-APK acceptance and physical Samsung deployment remain separate.
- Physical Moto e20 testing now demonstrates the production assembly/checksum workload on a low-RAM 2GB Android Go / 32-bit ARMv7 phone: 26 operations passed, no failures/skips; 201MB multilingual assembly took 30 seconds and the real 49.5MB Bahrain map 9.3 seconds. Exact integrity, sampled worker memory and workload limits are in [the Android performance report](docs/ANDROID_PERFORMANCE.md). This SDK2 cohort is separate from earlier ARM64 runs. TECNO POP 7 hardware inventory passed, but its workload was rejected by the free quota before execution; no performance numbers or paid retries are claimed.
- Explain producer-declared legacy Service Worker incompatibility in the reader itself, including read-first Library activation and restored tabs, rather than only Catalog/Info. Actual compiled-browser smoke imported tagged and untagged ZIM fixtures: direct/restored tagged reading displayed the notice; service-worker-free metadata read without it. Both 390px surfaces retained 390px document width. Evidence `.native/historical-reader-warning-20261030`; native packaged acceptance remains separate.
- Fall back from missing or file-replaced remembered desktop picker directories to existing platform defaults/cwd without rewriting preferences. The regression failed before the correction, then all four dialog-preference tests passed. Actual production API checks exercised both import/save defaults, remembered directories, missing directories and regular-file replacements with unchanged preference bytes; evidence `.native/directory-fallback-20261030/actual-api-evidence.json`. Native picker visual/cancellation/restart acceptance remains separate.
- An earlier integrated core Rust suite passed **374 tests**, with one ignored and eight warnings, including Android window-incarnation/drain regressions and the missing-directory fallback. That earlier run excluded the then-unregistered native PDF/notification providers. Its focused Android process-lifetime candidate completed one Back/Activity finish without FORTIFY and reopened the map in the same process; the later lifecycle/transfer acceptance is recorded above.
- Show each download once, with simple Progress, collapsed technical details and a full-width named Open action with 22px text and an 80px minimum height after verified import. Keep manual URL entry secondary; Discover and Nearby have their own navigation tabs. Omit filesystem paths from iOS metadata/settings/download details and present path-free errors without modifying backend data or diagnostic exports.
- Replace map/download cancellation and low-storage JavaScript confirmations with app-owned HTML dialogs. Await explicit consent, revalidate the current job/privacy/configuration and reject late confirmations after mobile suspension, noninteractive lifecycle or Quit.
- Bound map source caching to 8 MiB payload and 4,096 entries, evicting before the next cache temporary. Larger valid chunks remain RAM-only. Keep one growing output, source rehashing, streamed MD5, verified import and no-replace rename; retries may redownload evicted chunks.
- Join import, reader, sharing and dropped-request archive work before final native provider retirement. Propagate accumulated teardown/checkpoint failures into exit authorization. Android cleanup uses an application-only endpoint after Activity destruction and drains document completion-generated cleanup before executor closure.
- Prevent monitored folders from auto-registering app-published files before acquisition verification. Fresh live/durable publication claims exclude pending, failed and recovery outputs without weakening UUID-collision protection or explicit imports. Retain previous completed map destinations when a new job replaces the single current map record.
- Label Android integrity timings as standalone measurements rather than mandatory additional app stages. Preserve the historical physical preseeded-cache workload separately from the newer raw-range/bounded-cache evaluator, which times cache misses and writes. Reject mixed or unsupported report policies; no new physical performance numbers are implied.
- Preserve every accepted HTTP, torrent, map and LocalSwarm job beyond the initiating component's lifetime. Carry the original privacy context into the root handoff; stale views may register a watch but cannot steal navigation or cross a policy boundary. Actual desktop HTTP admissions delayed past navigation to Settings remained visible, and opening Hindi then Arabic cleared only the corresponding completion notice.
- Add installation-bound integrity badges, explicit confirmation at the inclusive 100 MiB boundary, queued/current progress and a reviewable redacted diagnostic export. Actual desktop production UI/API acceptance passed 25 checks, including damaged versus unavailable files, cancellation, replacement rejection, read-only/private persistence boundaries and exact reviewed export bytes. Native save pickers remain separate acceptance.
- Add trusted reader link actions and middle-click background app tabs without creating browser popups. Actual desktop UI verified active article/scroll/selection preservation, real clipboard contents, keyboard context actions and stale/synthetic rejection. Later native macOS WK/NSPasteboard and Linux ARM64 GTK/Xvfb also passed actual context/middle-button background-tab and clipboard readback scenarios.
- Reconcile foreground authority before saving mobile session state or admitting native voice discovery. Require an activated reader frame and the current privacy generation, rechecking asynchronous listener/results without weakening native authority checks. Actual Android and iOS simulator Home/system-Settings foreground tracing recovered installed voices without expired/retiring errors and allowed fresh trusted native playback, with no automatic resume.
- Keep the mobile facet open when iOS focuses its containing main region during checkbox pointer-down, while retaining actual outside-pointer, sibling-focus and Escape dismissal. At extremely short Android keyboard viewports, let the facet heading scroll rather than cover the search field. Native acceptance is recorded separately; source review alone does not certify these interactions.
- Separate the snippets checkbox label from its helper with explicit JSX whitespace in Library and Reader. Actual compiled desktop/narrow Reader surfaces displayed separate words without horizontal overflow; the previously frozen native builds do not contain this cosmetic correction.
- Replace unsupported non-macOS predefined Quit with an enabled native command and Ctrl+Q, preserving macOS's predefined action. Native Quit enters the existing prevented-exit reader-retirement/session-flush gate, not direct process termination. Independent source review and actual Linux pointer/keyboard exits passed; both exited 0 and restored the exact retained reader session.
- Rewrite the Android performance report around the practical older-phone question, with readable assembly and integrity-check timings derived from the original physical-device CSVs. Expose real-map structural-validation cost separately from assembly and hashing; distinguish sampled worker memory from full-app memory, storage skips from compute failures, recipe assembly from arbitrary ZIM merging, and checksums from unmeasured digital signatures. Independent numerical/clarity review passed; no benchmark results or immutable bundles were changed.
- Correct the separate Android hardware-inventory target after actual instrumentation rejected its `hasCode=false` manifest. Compile an isolated platform-only Application, keeping the inventory package separate from reader data and excluding Rust, native libraries, fixture assets and reader activities. Its startup requires the dedicated inventory package and a debuggable build. Actual emulator instrumentation/standalone CLI then measured RAM/ABI/storage with `nativeLibraryLoaded=false`, `workloadExecuted=false` and no fixture preparation; this is not physical-device or native-workload acceptance.
- Add a simple four-phone table of release dates, approximate launch prices and current US-dollar prices to the Android performance report. Link retail/launch sources and exchange rates; identify the used A03s quote and A16 RAM difference without burying the prices in sampling terminology. Performance measurements and immutable evidence bundles are unchanged.
- Correct the Android evaluation's coverage claim: the four measured configurations are approximately 3/4/4/6GB, not physical 1–2GB Android Go acceptance. Build a separate ARMv7 SDK/APK and fix oversized raw-cluster reader advancement without allocating the whole blob. Four actual ARM32/Bionic archive regressions passed, including sparse reads beyond 4 GiB; this is functional emulation evidence, not physical-device performance or full APK/JNI acceptance.
- Make Library activation read the archive directly; keep metadata under Info. Move Read/Remove above metadata and replace unreliable native JavaScript confirmation with an explicit, focused HTML confirmation. Bind removal to the archive installation identity so stale confirmation cannot remove a replacement; preserve the payload when removing only its library registration. Actual browser, Android and iOS flows exercised cancellation, visible confirmation and removal.
- Focus newly accepted downloads and show prominent progress, verification/import state and a large named Open action only after the imported archive is available. Preserve reading/navigation while displaying a non-modal completion prompt. Actual Android and iOS HTTP transfers exercised progress, completion and reading the downloaded archive.
- Add conditional, bounded OPDS page/facet caching with locale/query-separated identities, actual HTTP 304 reuse and explicit stale/offline warnings. Actual desktop runtime exercised 200 → 304 → network failure, private nonpersistence/no normal-cache fallback, and read-only profile preservation. Remove the obsolete uncached facet entrypoint after migrating its production caller.
- Keep mobile form controls at 16px without disabling pinch zoom; combine Android keyboard/system/cutout insets and preserve language-picker state through keyboard viewport changes. Android portrait inset behavior and iOS no-zoom focus were observed natively; the final cross-platform picker keyboard changes remain under native acceptance.
- Correct physical iOS directory-capability bootstrap: retain Foundation's native application-container descriptor rather than opening `/` and traversing forbidden ancestors. Keep descriptor-relative no-follow traversal and container identity checks across profile relocation, library, maps, torrent publication and LocalSwarm staging. The installed physical build's matching crash reports and archived Rust stderr establish a setup-hook `EPERM`; exact syscall attribution remains inferred until corrected physical launch acceptance.
- Require matched schema-2 native SDK provenance and actual linked-library hashes, with pinned libzim 9.8.2 split-range/retained-descriptor corrections and POSIX Xapian 1.4.23 explicit-offset support. Add one native macOS/Linux SDK recipe instead of accepting stock Xapian, validate Android stage-output identities, verify temporary downloads before cache promotion and isolate pkg-config sysroots. Independent source reviews passed after corrections; compilation also exposed and corrected two explicit-constructor return expressions missed by source review.
- Collapse narrow-screen reader chrome to one 52px bar, with tabs and labeled secondary tools opened on demand. Preserve archive-frame identity across both controls, visible privacy/read-only state, sensor revocation and desktop toolbars. Adversarial review corrected fullscreen-to-tabs visibility and focus left on collapsed tabs. Actual 360px Arabic execution exposed a full-width dialog Close button; scope action styling away from the header. Final compiled CSS changed dialog scroll/client widths from 376/326px to 326/326px, with a 44px Close target. Native simulator article/map single-bar proof is retained in `.native/ios-compact-chrome-20261029`; final native controls acceptance remains separate. Physical installation succeeded but launch was denied while the iPhone was locked.

- Preserve source-read ownership during LocalSwarm policy retirement: cancel serving and await its blocking callback/reader joins instead of dropping the serving future. Retain a Session-lifetime watcher so idle public-peer QUIC traffic stops on Internet denial, independently of chunk polling; keep LAN and Internet denial frontiers separate.
- Read the ZIM header across bounded, contiguous split parts rather than requiring an 80-byte primary part. The real seven-byte/73-byte/remainder source-to-receiver regression failed before this correction and passes afterward, including truncation rejection.
- Display archive verification status labels with localized numeric counts instead of grammatically incorrect flat plural messages. The actual application verified one archive and displayed “1 of 1 checked”, “Verified: 1”, “Damaged: 0”, and “Unavailable: 0”.
- Let accepted external-archive picker work finish under its existing privacy lease instead of requiring the foreground transition that lease blocks. Defer automatic opening and mobile library-change refresh through native lifecycle reconciliation, preserving the original policy intent and current installation. SDK2 actual Android direct-provider admission, automatic reader opening, non-prefix title/full-text search, integrity verification and persistent grant rehydration passed without a managed copy. Correct managed-copy foreground handoff with single-owner picker/registration admission; actual API 36 picker cancellation, disabled concurrent import controls, verified managed copy and article reading passed with unchanged originals and existing archives. iOS native Open-In managed import/read passed separately; this does not certify its toolbar picker.

- Add default-off mobile cellular/metered acquisition admission from native Network.framework/ConnectivityManager capabilities, manager-level request/stream/session retirement fences, durable HTTP/torrent interruption, and owned-journal map cancellation. Keep verified LAN-only LocalSwarm sockets bound to physical Wi-Fi/Ethernet rather than falling back to a cellular default. Add default-off iOS archive backup eligibility using the current OS backing of `NSURLIsExcludedFromBackupKey`, applied only through caller-retained descriptors before payload writes and after verified publication. Reconcile Store/durable-publication evidence across all owned profile locations, preserving moved/hidden archives and excluding incomplete imports; replay policy before HTTP/torrent/map recovery completes. Validate historical receipts against the actual retained inode/content, never pathname membership; unmarked history lacking its original identity/hash remains excluded until verified current library admission or reimport. Never include a whole profile, preferences, secrets or temporary/cache payloads. Android uses static manifest/cloud/device-transfer exclusions with an honest non-toggle explanation. These are source changes with authored, unexecuted regressions; native network-switch and backup/restore acceptance are not yet exercised.
- Make concurrent startup backup reconciliation use conditional descriptor-bound exclusion under the same native metadata transaction as publication. A stale “not admitted” observation can no longer undo a later publisher's inclusion, including after the scanned temporary name is renamed. Four registered native iOS simulator tests passed. The first run's one failure was a cached Foundation URL readback in the test; a fresh URL preserved the expected inclusion assertion and passed. This does not certify actual iCloud backup/restore.
- Normalize both Foundation and POSIX spellings of owned backup paths only after rejecting raw traversal components. Preserve no-follow descendant traversal, retained inode identity and descriptor-only metadata changes. Six native backup-policy test cases passed across the existing four-test run and two additional alias/symlink regressions; a separate macOS run of the verbatim production helpers verified POSIX `/private/tmp` versus Foundation `/tmp` exclusion/inclusion without inode replacement. This is not a physical iCloud backup/restore claim.
- Require explicit confirmation when a known download would leave less than decimal 1 GB free on its actual chosen filesystem. HTTP, BitTorrent, LocalSwarm and map admission retain their hard insufficient-space checks; acknowledgements are scoped to the request/job, directory identity and remaining bytes, not a permanent preference. Unknown-size metadata is resolved before payload admission where available, and asynchronous warnings remain stopped until an explicit user action. Source changes are not proof of physical disk-full behavior on every phone.
- Recheck map output/cache capacity and request-scoped low-space consent before an unverified local-base hint falls back to network chunks. Preserve each exact initial/fallback volume approval together so explicit composition retries cannot alternate warnings. Compare actual open-directory volume identities on Windows as well as Unix, so a separate cache drive receives its own warning. The integrated macOS Rust suite passed; native mobile and Windows storage-boundary acceptance remains separate.
- Add marker-selected desktop portable profiles for upstream desktop #1179/#1209 and #1166/#461: a checked regular `.portable` file selects adjacent non-symlink `data/`, fixes download/monitor roots before manager admission and on settings updates, disables path controls with a localizable explanation, and scans direct `.zim` files. Store data-owned archive paths as explicit native components and map exact saved job path fields/consumption receipts across stopped-folder moves without refreshing recovery identity evidence. Consume but ignore the typed runtime portability flag when reading settings, preserving unknown-field rejection. Preserve explicitly authorized external migration sources while confining owned destinations. Configure local Windows/Linux WebView/application profile roots before creation; retain ephemeral macOS WKWebView without claiming OS-cache isolation. Source tests are added for profile reopen/move and native filenames; tests, final catalog generation and packaged/removable-drive/native-cache acceptance remain pending.
- Stop remaining search-result/snippet parsing when its reader/library consumer cancels, supersedes a query or retires, and signal accepted searches before privacy/window/shutdown retirement waits. Scope requests to server-issued root-document/consumer capabilities so old cancellations cannot stop another consumer or resurrect after a root reload. The current libzim call and current HTML fragment remain nonpreemptible; source tests cover row boundaries and ownership/policy transitions, but integrated heavy-archive/native lifecycle execution remains pending.
- Add a saved, default-on “Show search snippets” toggle to library/reader search. Disabling skips full-text native snippet extraction, avoids libzim suggestion-iterator materialization that would generate title snippets, and bypasses Rust HTML-fragment parsing while retaining hit identity/ranking. Preference changes cancel obsolete work and clear results; the visible explanation requires a new explicit search. The integrated Rust suite passed; this is not a measured memory/performance claim.
- Replace decorative workbench chrome with a compact archive utility: platform fonts, neutral light/dark surfaces, list-first library/catalog, restrained separators and toolbars, and bottom navigation on narrow screens. Keep archive-authored article typography separate. Actual 1500×1100 and 360×800 English/Arabic surfaces rendered without horizontal document overflow; loaded catalog/map rows, grid/list switching, saved appearance/language, title search and protected LocalSwarm inspection were exercised through the real Rust service in isolated Chrome.
- Independently adversarially review all redesign components and shared styles. Reproduce and correct dark reader-button contrast: white on the light-blue dark-theme accent becomes the theme surface foreground. Actual computed colors changed from white to `rgb(32,35,39)` on `rgb(138,180,248)`; this is not a screen-reader or mobile-native conformance claim.
- Prove desktop BitTorrent restart behavior against an independent peer: graceful exit, policy-off restart, manual pause, retained-piece tampering and SIGKILL. Automatic restart reused 10,808,390 verified bytes and fetched exactly the remaining 60,555,264 bytes of a 71,363,654-byte archive, then matched SHA-256 and imported. The acceptance binary predates the new mobile suspension APIs; those APIs still require native execution.
- Build pinned, split-range-patched static libzim/Xapian/ICU dependency closures for arm64 iOS device, iOS simulator and Android. SDK workspace fingerprints reject stale builds after recipe/toolchain changes; both stale-cache rejection and fresh reviewed builds were exercised. This does not prove application linking or native mobile launch.
- Reproduce and correct Android startup failure from syncing cloned `O_PATH` directory capabilities. Open a readable descriptor relative to the retained directory, verify type/device/inode, and perform real synchronization; errors are not ignored. Migrate mobile relocation, map/torrent publication, archive migration and LocalSwarm directory-sync callers. An actual Android syscall probe reproduced `EBADF` before the correction and succeeded with the same directory identity afterward; the rebuilt application rendered its native Android library surface. Provider import and other mobile acceptance remain separate gates.
- Keep Android-managed archive storage under the native `Context.filesDir` capability instead of broadening document-adapter access to the package data root. The real system picker first reproduced `PATH_OUTSIDE_APP_STORAGE`; the corrected application imported and registered a ZIM under `files/kiwix/archives/mobile-imports-v1` and rendered its isolated reader.
- Apply Android system-bar/display-cutout insets to the native content view while preserving keyboard inset propagation. The API 36 emulator's actual WebView moved below the status bar and its reader rendered in the remaining viewport; older Android keyboard/cutout combinations remain unexercised.
- Keep native archive index reopening bound to descriptors owned for the entire archive lifetime. `Archive::open_files` previously retained clones but passed caller-owned descriptor numbers to libzim; after those callers closed, suggestion-index opening failed and silently became prefix-only search. Construct native inputs from the retained clones. On Android, the unchanged non-prefix English search assertion failed before and passed after this correction against the same SDK; all 34 local evaluation operations passed, with the 1 GiB workload explicitly storage-skipped.
- Implement explicit LocalSwarm v1 QUIC discovery/manual inspection, verified acquisition and selected-library-archive offering with optional PIN authentication, common Downloads controls, no automatic re-offering/restoration, and private/read-only lifecycle retirement. Real Swift/Go and remote Mac/Linux peer scenarios are recorded in [LOCALSWARM](docs/LOCALSWARM.md); final integration acceptance remains in progress.
- Correct adversarial LocalSwarm findings in bounded QUIC parsing/queues, throttled receive heartbeats, pipelined request handling, cancellation-safe fair bandwidth credit, certificate pinning, source identity retention/revocation and joined shutdown. Retire writer authority before final retained-byte MD5 plus stock libzim structural verification.
- Vendor and harden `mdns-sd` 0.17.2 cache/daemon/parser bounds, pressure-removal notification, nonblocking subscribers, resolver/timer/probe lifetime, DNS compression and TTL arithmetic. This is a scoped fork, not blanket dependency certification.
- Add a reproducible pinned libzim split-range patch on all SDK paths: strict start ordering prevents dropping an overlapping final part; empty pointer tables/items are handled before mmap/iterator increment. Actual valid seven-byte-final-part and empty split probes pass after correction. Windows SDK provenance must declare the applied patch.
- Isolate library-registration errors as visible, explicitly retryable acquisition failures without acknowledging the failed receipt or preventing unrelated registrations. An actual LocalSwarm duplicate-UUID download reproduced the global Downloads failure; corrected Downloads remained usable and explicit retry imported after the original library entry was deliberately removed without deleting its file. UUID collision protection is unchanged.
- Extend generated interface/native-purpose localization to LocalSwarm, the utilitarian redesign and mobile controls. The current inventory has 933 sources, 54 string-complete and 20 explicitly partial catalogs. New Progress/nearby-sharing copy was acquired for 53 non-English catalogs; gsw/ia rejected the new-copy service request and retain explicit fallback. Existing partial catalogs remain incomplete; machine output is unreviewed.

- Add opt-in spelling suggestions for zero-result article searches using explicit cancellable, bounded archive-title indexing and verified UUID/version SQLite caches. Cold cache validation is single-flight background work; private/read-only modes disable spelling operations and join indexing writers before policy transitions. This does not change upstream libzim search/index behavior.
- Add delayed/rate-limited hover and keyboard-focus internal-link previews. The trusted shell requests bounded text-only title/excerpts from libzim rather than injecting archive HTML, rejects stale session results and offers article/new-tab actions.
- Add unified search across open tabs, reading list and history; alphabetical/reverse-alphabetical and oldest/newest saved-item sorting; library metadata sorting in both directions, including availability, language, publication date, creator/publisher, content and counts.
- Add shared searchable multi-language/category/content facets, visible edition/flavor/publication metadata, catalog current-page sorting, and explicit confirmed HTTPS online previews. Current-page sorting is not whole-feed sorting.
- Add native tab selection/reordering shortcuts, tab context actions and back/forward history menus. Retain per-tab root-owned reader workspace state, including unfinished note drafts, across reader remounts; privacy epochs reject stale updates. Correct RTL drag/drop geometry and narrow Escape dismissal in source.
- Add explicit move-existing-archives consent when the download directory changes. Native migration handles verified exact split sets using exclusive same-volume rename or verified cross-volume copy, durable journals, reference updates and recoverable staging. Review corrections address staged source cleanup and same-filesystem rollback; real crash/filesystem acceptance remains pending.
- Add reveal-in-file-manager, verified split-part deletion with explicit trash/permanent-delete confirmation, known FAT/FAT32 single-file-limit preflight (not exFAT), and native no-clobber publication without a hard-link-only requirement.
- Replace unreliable universal inline PDF handling with trusted-shell Save and confirmed OS Open for an allowlisted attachment MIME set. Open extracts an untrusted attachment to an owned temporary copy; the default OS application may access the network. A real PDF was opened by macOS Preview and matched the archive bytes. Other platform viewers remain unverified.
- Extend selected-archive LAN sharing with loopback/IPv4/IPv6/dual-stack/selected-local-interface binding, configurable port, bracketed IPv6 links, copy/QR controls and exact host/origin checks. This is source-backed HTTP sharing, not mDNS, Wi-Fi creation or LocalSwarm.
- Add root-owned, acknowledged reader cookie/localStorage queues and a native-owned authenticated same-origin retirement document. Real pagehide and nested-microtask writes finish before iframe removal. A challenge-independent proof and Fetch Metadata restrictions prevent archive fetch/script requests from spoofing retirement; outstanding producer credit is reclaimed without dropping accepted writes.
- Separate app-owned archive installation identities from untrusted ZIM UUIDs for storage and sensor consent. A checksum-valid different ZIM with a copied UUID reproduced inheritance of saved site data in the older baseline. Reject conflicting path imports, rotate identity on explicit relink, preserve controlled moves, and migrate v1 profiles to installation-keyed v2 state. Conflicting import and explicit-relink isolation passed actual service scenarios.
- Harden publication around retained verified file handles and identity-bound no-clobber commit/recovery across HTTP, torrent and maps; reject replaced source/staging/output identities rather than deleting uncertain files. Recovery/checkpoint/cancellation boundaries remain under final integration acceptance.
- Reserve installed library paths at HTTP/torrent/map publication, including missing archives and case/Unicode aliases, under the profile mutation lock. Reject fresh acquisitions reusing an installation UUID at the same path; registration rejection alone cannot protect a missing path from replacement. Avoid canonicalizing every installed/NAS path while holding the profile transaction mutex.
- Add version/platform identification to StreetZim native HTTP requests and correct the title-pagination total estimate beyond the first 1,000 results. Actual pagination now returns entries 1,000–1,049, total at least 1,050 and `hasMore`.
- Correct a sampled native deadlock between accepted archive-storage writes and visit commits: all affected transactions acquire the closed-window guard before the Store lock. Fifty storage writes and fifty visit writes subsequently committed concurrently.
- Separate sensor permission/provider request lifetime from geolocation acquisition timeout. Browser watches own browser acquisition timing; native CoreLocation starts its acquisition deadline only after OS authorization. Actual delayed app-consent denial no longer consumes the two-second acquisition timeout; hardware/OS-grant acceptance remains separate.
- Canonicalize history article paths before deduplication, including existing encoded aliases, without collapsing query/fragment routes or literal encoded delimiters. Actual equivalent-path visits now collapse to the newest title/scroll while a different fragment remains distinct.
- Correct the stopped-sharing TypeScript contract and Settings initialization: native status legitimately has `binding: null` when stopped. Opening Settings reproduced a null-`mode` render failure; optional initialization and active-binding narrowing fix it, and actual Settings/privacy transitions now complete.
- Add persistent anchored passage highlights/comments for desktop #876, trusted editing/jump/delete and lossless application JSON/XML interchange. Validate bounded UTF-16 quote/context anchors, resolve only valid unique matches, and use non-destructive native-highlight/calibrated-overlay rendering.
- Correct annotation adversarial findings: arbitrary legacy bookmark IDs use opaque render tokens; identical highlight IDs in different bookmarks remain independent; fallback rectangles compensate reader zoom; excluded controls do not manufacture false ambiguity. Private comments remain shell-side through an explicit anchor-only wire projection.
- Replace cached annotation-array saves with atomic add/comment/remove deltas against latest Store state. Actual snapshot saves reproduced overwriting concurrent notes/title/tags and deleting another window's newly added passage. Delta saves preserve those edits; a late comment cannot resurrect a deleted highlight.
- Add the saved, opt-in “Automatically expand article sections” choice for #690. Each activated/loaded reader document expands through the existing find/TOC helper; no background iframe or sensor authority is retained. Disabling leaves existing user DOM alone and applies to subsequent documents.

- Resume interrupted transfers when the mobile network policy re-admits Internet (upstream #4748). Network-policy reconciliation now arms exactly one resume per admitted connectivity revision (`resume_revision` changes on a native generation change or a metered-allowance edit; duplicate publications of the same generation do not re-arm it), and `mobile_resume_after_network` consumes that revision once and applies a shared `transfer_policy::reconnect_resume` decision before entering the platform's foreground-service/native admission path. The decision requires the existing automatic-resume opt-in, a non-private and non-read-only profile, an active application, and an admitted unmetered path (or the user's metered/cellular allowance); every skip logs its reason. HTTP and torrent managers expose `resume_interrupted_after_reconnect`, which reuses the existing `interrupted`-only restoration, so manual pauses, cancels and error rows are never restarted and the metered/cellular admission tests are unchanged. Desktop has no network-policy retirement, so its resume remains explicit. Unit tests cover admitted HTTP resume, manual-pause exclusion, metered/offline denial, read-only/private/opt-out/app-retired gates and the one-attempt-per-revision bound; real Wi-Fi↔cellular↔Wi-Fi and hotspot transitions still need device execution.


### Standalone application

- Introduce the standalone Rust/Tauri 2 service and React 19 UI with local library, discovery, maps, downloads, reading list, history, settings and a dedicated reader workspace. Builds do not require the sibling Qt/Apple checkout.
- Add a small exception-safe C++17/C ABI bridge to pinned, patched libzim 9.8.2 with matched Xapian/search support, for metadata, range reads, redirect resolution, title/full-text search and integrity checks. Qt, libkiwix and a separate aria2 process are not runtime dependencies.
- Add local/split-first-part imports, drag/drop and recursive folder import, periodic configured-directory monitoring, missing-file retention/relinking, explicit unlink/trash handling, command-line article URLs and warm-instance file routing.
- Add explicit tab back/forward/scroll/session models, multiple native windows/portable geometry, closed-tab reopening, and searchable visits. These are portable app data, not opaque WebKit/Qt session imports.

### Reading and acquisition

- Add cross-archive selected/all title/full-text queries with snippets, rank-fused bounded pagination, identified sources, per-archive errors and stale-response protection.
- Add persistent article identity, heading-based TOC, find, original/light/paper/dark reader appearance, fonts/sizing/zoom, bookmarks with editable notes/tags, richer JSON and legacy XML interchange, and system-webview speech/printing paths.
- Add exact-byte original HTML/resource saves and bounded original-article MHTML with static dependencies. MHTML is not a live dynamic-DOM capture, and native print/PDF availability remains platform-dependent.
- Add bounded HTTPS OPDS/Atom discovery with custom sources, server-side queries and pagination, plus manual edition discovery. Application-release lookup clearly labels upstream Qt releases as a different, noninstallable channel.
- Add independently persisted HTTP/Metalink jobs, strict resume validation, output ownership/capacity checks, no-clobber publication and visible import errors separate from transport completion.
- Add in-process librqbit acquisition, durable job/ownership records, policy-controlled restoration and piece recheck, limits, explicit completed seeding, DHT/PEX and best-effort BEP 14 local discovery. No ZeroConf/mDNS or optimized fast-resume claim; cancellation retains staging.
- Add **shared Windows/macOS/Linux/iOS/Android StreetZim** catalog/area/language UI and native v1/v2 recipe composition, verified local/chunk reuse, exact bounded ranges, masks/hash/checksum validation, cancellation, no-clobber output and post-publication import. One Rust map engine applies across targets; per-platform runtime acceptance is still required.
- Add selected-archive read-only LAN HTTP sharing/search, separate from privileged application APIs, with IPv4/IPv6/dual-stack/interface and port selection plus collection links/QR controls. Cross-device acceptance remains pending; this is not Wi-Fi network creation or encrypted authenticated account hosting.

### Security, privacy, devices and language

- Isolate archive pages from native commands using archive/window-specific fresh UUID-and-nonce localhost **root** origins, exact-origin/frame/session bridge checks, restrictive response policy, and plain-text error responses. Archive-controlled service workers are rejected.
- Use incognito native views in normal and private modes; persist deliberate compatibility localStorage and expiry-bearing cookies by app-owned installation identity rather than public archive UUID or ephemeral webview origin. Native IndexedDB/profile persistence is not promised.
- Rotate archive listeners at privacy/read-only boundaries, reject stale epoch writes, preserve pre-private normal sessions, and separate private browsing semantics from strict read-only write prohibition.
- Keep native application accelerators in trusted native menus instead of archive-originated shortcut messages; isolate the authenticated headless shell on its own fresh `kiwix-shell-<nonce>.localhost` origin.
- Add active-archive camera-only/location consent, per-window session binding, cancellation and destructive iframe/origin remount on revocation. Add macOS CoreLocation bridge and platform permission metadata. Actual grants, OS prompts and hardware remain unexercised.
- Add 74 offline-loadable locale entries, locale/script matching, RTL direction, number/date/size formatting, cardinal plurals and explicit partial coverage. The current inventory has 54 string-complete and 20 partial catalogs; unreviewed machine text is not a native-quality claim.
- Add opt-in bounded categorized local diagnostics without arbitrary command arguments, URLs, tokens or article-history logs; no silent network telemetry.

### Documentation and distribution implementation

- Preserve full Qt/Apple feature audits and complete closed-issue/PR censuses for both upstream repositories, with source-backed dispositions and explicit acceptance gaps. See the linked historical ledgers in [issue dispositions](docs/ISSUES.md); a closed or merged upstream record is not automatically an implemented or verified fix here.
- Document current module ownership, privilege boundaries, root build commands, exact libzim SDK requirements, import limitations and platform/release gates.
- Add platform packaging configuration/workflow and macOS runtime-dependency closure/signing tooling. Windows requires an externally provisioned trusted MSVC libzim SDK; AppImage/native macOS paths need actual clean-machine acceptance and authorized signing/publication.
- Bound GitHub Actions cost: macOS packaging is manual-only (arm64 by default, Intel opt-in), docs-only pushes and draft PRs start no runs, dependency audits moved to a weekly/lockfile-triggered workflow, Android/Windows type-checks reuse the frontend `dist` artifact, native builds skip UI-only pushes, and artifact retention is 7 days (14 for release staging).

### Verification and known limits

- Physical iPhone acceptance passed on 2026-11-06 against source `12fa8b5`
  (iPhone 17 Pro Max, iOS 27.0.1, run
  `device-acceptance-3582be14763c405dbc7604ebd367c838`): Library → Wikipedia 100,
  search **Animal**, real article body, reading-pan probe
  `hidden=true`/`restored=true`, map home/zoom/pan, 120 s, phone released with
  archives/tabs/settings preserved.
- Physical Android acceptance passed on 2026-11-06 against the same source
  (SM-A356U, Android 16, debug APK installed over the profile): preserved
  Animal article, Wikipedia 100, search **Animal** (about 1,040 results),
  article body, real TOC navigation to **Reproduction**, reading-pan hide with
  the Reader navigation pill and restore on downward scroll; no FATAL/ANR.
  TalkBack, DocumentsUI move, 16 KB-page hardware and background speech remain
  separate gates.
- The LocalSwarm receive-path verification change passed the integrated host Rust suite with `--features localswarm-profiling`: **491 passed, 6 ignored**, including the new pipelined-handoff integrity guard, the 16-chunk seeder window and the crash-safe verified-prefix regressions. The ignored throughput benchmarks were run explicitly in both profiles over the Mac's Wi-Fi IPv4 address (`KIWIX_LOCALSWARM_TEST_HOST=<mac-lan-ip>`, 1 MiB chunks, unlimited budgets): release receive path 92–95 MB/s flushed with the Network.framework seeder and 58–60 MB/s with Quinn, Apple→Quinn pipelined serve rate 108 MB/s, seeder serve cadence 14.9 MB/s at a full window; debug receive path 32–34 MB/s. Physical phone transfer with this build is not measured, so the phone's own b…

- The integrated desktop Rust suite passed **369 tests**, with one ignored and five warnings, against the matched schema-2 macOS SDK. An earlier 366-test run exposed a map-space fixture that no longer exercised disk caching with a 16 MiB chunk; its corrected cacheable-chunk regression and all 12 bounded-cache tests passed. The subsequent retained-map-history correction has its own targeted verification. TypeScript and separate desktop/iOS/Android Vite builds passed; existing bundle-size and static/dynamic import warnings remain.
- Consumer UI acceptance observed one canonical job card and large, verified Open actions on native macOS, Linux ARM64, Android emulator and iOS simulator. The iOS consumer simulator build displayed no filesystem paths in expanded Downloads details or Settings and cancelled actual composition through its HTML confirmation, removing the 50,331,648-byte partial. It then composed/read Washington D.C./en: 123,165,911 output bytes, recipe-matching MD5, sampled cache peak 8,123,658 payload bytes and combined output/cache allocation 144,441,344 bytes. Evidence `.native/ios-consumer-phones-20261030`; final physical V2 deployment is separate below.
- Real Bahrain composition completed with one output on macOS and Linux. The Linux app fitted a 49,235,277-byte output in an owned 64 MiB profile/output filesystem; sampled peak allocation was 51,474,432 bytes. Final V2 macOS Hawaii/en exercised actual eviction across 11,510,612 cacheable bytes: sampled cache payload plus temporary peak 8,334,915 bytes, output 150,605,015 bytes, combined allocated peak 161,443,840 bytes, one output inode through publication, matching recipe MD5 and managed receipt import/read. Bahrain/ar cancel/retry and native Quit/restart preserving six archives/seven tabs passed. Binary `0fbc13ca04d4d561820f423b0f392f4cbf5859ea5a5475f2ceefbbfb40a6caee`; evidence `.native/mac-bounded-map-20261030`. These sampled workload observations exclude filesystem metadata and are not universal allocation bounds, accelerated-map-pixel proof or low-end Android timings.
- Android's V2 consumer build installed/launched on the connected Samsung SM-A356U, but Back still emitted FORTIFY destroyed-mutex errors there and in the emulator despite exit-info reporting EXIT_SELF/status 0. Actual emulator debugger stacks identify HWUI CommonPool worker mutex access concurrent with `__cxa_finalize` → libc `exit` → Rust `process::exit` → Tao's Android event loop. This is a diagnosed, unresolved exit-path defect, not a successful shutdown; Home/resume success does not resolve it. APK `deedd590026076320b0bdb914d9d4f3338bd61d7e23617f264b5c006f9903e5b`, exact ELF, phone logs and debugger transcript retained in `.native/evaluation/android-bounded-map-5560`.
- After the Samsung Back failure, one normal relaunch produced a surviving PID but the captured surface was blank. That capture was approximately 0.8 seconds after launch; timing versus a persistent relaunch defect is unestablished. The phone then disconnected before DOM diagnosis. Initial Library rendering is verified; healthy post-Back physical relaunch and a corrected Android deployment remain pending.
- Final V2 native Linux ARM64 GTK/WebKit composed Hawaii/en with recipe-matching MD5 and passed native integrity verification. On an owned 164 MiB profile/output tmpfs, 22,491 samples observed peak total allocation 156,295,168 bytes, cache payload peak 6,124,221 bytes and one concurrent output. Actual cache eviction, pruning an unrelated recognized 9 MiB legacy entry while preserving an unknown sentinel, numeric low-space decline/continue and HTML cancellation after 16 MiB followed by successful retry/import passed. Native Open measured 1024×80; pointer Quit/Ctrl+Q exited 0, and the restored map rendered tiles/Honolulu search in a loopback-only network namespace. Binary `984b45d21a270378a15b1ec79c8f3388ecdb91dbd9283a0603f68cbcf16dae91`; evidence `.native/linux-bounded-map-retirement-20261030`. No transient cache temporary was sampled; no measured network-byte total, Linux x86 or release-package acceptance is claimed.
- A native Ubuntu ARM64 GTK/WebKit build exercised the real application window, ZIM import, article/title search, navigation, synthetic saved-map rendering and explicitly saved dark theme. Evidence is retained in `.native/linux-arm64`; this is not Linux x86 or release-package acceptance, and that snapshot predates the final LocalSwarm lifetime correction.
- A fresh schema-2 Ubuntu ARM64 SDK and native GTK/WebKit application passed real fixture import/reading/title search/navigation, synthetic saved-map rendering and saved dark theme. Binary SHA-256 `ced3b9013cef7ebefb197af65cf4448e5b3a415037c47d7d3dc5c71b124730ff`, exact SDK/recipe inventory and screenshots are retained in `.native/linux-arm64-sdk2`. That frontend predates the new compact mobile reader controls; Linux x86 and release-package acceptance remain separate.
- The later Linux ARM64 native GTK/WebKit snapshot passed body-read versus Info, identity-bound Remove/Cancel and confirmed unlink without payload deletion, actual integrity badges/diagnostic preview, synthetic Saved Maps, context/middle-button background tabs, actual isolated Gtk clipboard readback and compact controls. Binary `bd999b495b983bf3dbed42c5d4e072e5afb646fd51e41de02d67765140e5d9c7`; evidence `.native/linux-final-native-20261029-161513`. Acceptance exposed disabled native Quit and a concatenated snippets helper; neither is accepted in this frozen build. The Quit correction and separate whitespace build require their own acceptance.
- Final iOS simulator facet/voice-readiness acceptance on `9e1771a3717330afcc595972399efc21997e6d2d66f7cda28ab1b1c62a0a17cd` passed actual English-checkbox selection, Close/outside/Escape retention, native keyboard Done and 16px/scale-1 focus. Genuine system-Settings background/foreground recovered the installed Albert voice and a fresh trusted speech request returned offsets 0/10/18/21 without visible authority errors. All four archive hashes and 23 baseline tab IDs were preserved; settings matched exactly. One cleanup-helper state-precondition failure was retained, diagnosed and corrected; no production failure was hidden. Evidence `.native/ios-final-facet-voice-20261029`. This is simulator callback evidence, not new physical-phone or audible-output proof.
- The final macOS arm64 WKWebView package passed read-first Library, Info, identity-bound Remove/Cancel/reference-only removal, genuine integrity verification/badge and diagnostic preview, native right/middle-button background tabs, actual asynchronous Copy link with exact NSPasteboard readback, compact controls and native Quit/restart retaining both tabs and the active article. Executable `8e5b5e4db3df206599f73e823c5c153c29328bf05c6488338dc969502129831f`; evidence `.native/mac-final-native-20261029`. Eight bundled arm64 Mach-O images passed strict/deep ad-hoc signature checks; this is not notarized, universal or clean-machine release acceptance. The later whitespace correction is excluded; no native save-picker or accelerated-map-pixel proof is claimed.
- Linux Quit regression acceptance passed on `0cadd7a8e72bf8b8c22bc08a60ae39bb6cf7b30c5f1b43ed2285f4ecb9c56502`, with the corrected snippets whitespace. Native pointer Quit exited 0; relaunch without a fixture argument restored two tabs and the active detail article. Back then Ctrl+Q exited 0, retaining map location, forward history and geometry. Evidence `.native/linux-quit-fixed-20261029-162548`. The first build exhausted the guest disk; only its new failed target was removed, and the same source succeeded with dev debug symbols disabled. Original accepted artifacts and failure evidence were preserved; this is not a release-optimized or x86 build.
- The final consumer V2 physical iPhone build was development-signed, strictly/deeply verified, installed in place and launched once successfully on the iPhone 17 Pro Max test device. Executable `ee63e118ef4a8893b66e1f8757763f31652219a7f14e038ee5a0a3b19ebe8d33`, frontend `index-z9C7yfjn.js`/`index-CcRrjaye.css`; evidence `.native/iphone-consumer-bounded-map-20261030`. The screenshot shows downloads first, large named Open actions, collapsed Details and manual URL last, without filesystem paths. Three books/five tabs, managed payload hashes and capability identities were preserved across normal iOS container rebasing; pending grant journal paths advanced with that rebase. External provider bytes were not independently hashed. Installation/startup does not certify physical sensors, speech or transfers.
- An earlier physical iPhone build was development-signed, strictly/deeply verified, installed in place and launched successfully once; matching process evidence and screenshot show the running app. Executable `2d13fc7730175cecb32439eb93ba4f91649eaec0d034f1d39a3a7731c1d7baa2`, UUID `C7B5EF9F-70CD-36A0-9E33-BB9F0CC246D6`, frontend `index-bxX92ScP.js`/`index-BFSI2ZG2.css`; evidence `.native/iphone-current-20261029`. Both books and the existing tab survived; managed archive bytes matched, and profile differences were only normal iOS container-path rebasing. External reference metadata was preserved but its bytes were not independently hashed. No physical interaction, permission, sensor, speech or transfer certification follows from installation/startup.
- Current-SDK iOS simulator torrent replay passed actual independent-peer magnet acquisition, manual pause/restart retaining 14,706,698 verified bytes, genuine SpringBoard background checkpoint in 1.505 seconds, and interrupted non-auto-resume retaining 25,716,746 bytes. Explicit Resume fetched exactly 211,812,352 missing payload bytes; the completed 237,529,098-byte East Polynesia archive matched source SHA-256, imported and rendered offline, including native zoom changing scale from 50 mi to 10 mi. All four baseline archives/hashes, 24 tab IDs, settings and speech preferences were preserved. Evidence `.native/ios-sdk2-torrent-runtime-20261029`; this is simulator/private-peer coverage, not indefinite iOS background execution or physical-device acceptance.
- Stop permission-sensitive macOS dialog acceptance without changing OS settings. Real production Import opened NSOpenPanel and native cancellation worked; remote-panel accessibility inspection returned API-disabled error -25211. The user's observed settings prompt was not captured, so causation remains unconfirmed. Successful native selection, save dialogs, remembered-directory restart behavior and failed-write behavior remain unverified. The owned test app exited normally and runnable harness was removed; evidence `.native/mac-native-dialogs-20261029`.
- The full unsigned iOS debug application built, installed and launched on an owned iOS 27.1 simulator. Actual library/settings, native Open-In confirmation, managed ZIM import and reader/detail navigation passed. Native compilation exposed and corrected Swift event-value typing, lazy shortcut-property inference, the static Swift product name expected by Tauri, and iOS DNS-SD linkage through libSystem. Physical sensors, external providers, backup/restore and background torrents remain separate gates.
- The user's instant physical iPhone startup crash was confirmed by exact-UUID crash reports and archived Rust stderr: setup failed with `EPERM` before profile publication. Starting at a retained native app-container descriptor passed that boundary but exposed a Foundation/POSIX path-spelling mismatch. Using POSIX `realpath` consistently with Rust canonicalization fixed startup without weakening no-follow traversal or resetting data. Final development-signed binary UUID `AE7F889D-397C-34D3-88C8-CF00D53BD09A` was strictly signature-verified, installed in place, survived startup, initialized its root marker/state, and visibly rendered the Taipei offline map on the user's iPhone 17 Pro Max. Independent source reviews and actual physical acceptance are recorded separately in `.native/iphone-deployment-20261029`; physical sensor, backup, torrent and LocalSwarm gates remain separate.
- The platform-specific Android ARM64 application built and launched on API 36; actual mobile settings expose default-off metered acquisition and correctly omit the unsupported Android archive-backup toggle. An independent aria2 peer then exercised actual background torrent continuation, native foreground-service notification Pause, two force-stop/relaunch cycles preserving manual pause, and reuse of 31,490,034 verified bytes. Explicit resume fetched exactly 15,728,640 remaining bytes, completed a 47,218,674-byte ZIM with matching SHA-256, automatically imported it, and rendered its Bahrain map. This is emulator/local-peer acceptance of the pre-SDK2 binary, not physical-device or final SDK acceptance.
- The schema-2 Android emulator independently completed a distinct East Polynesia torrent: notification/UI pause persisted through restart, interruption with automatic resume disabled required explicit Resume, and final resume reused 9,988,106 matching bytes while the peer supplied exactly 227,540,992 missing bytes. The 237,529,098-byte archive matched SHA-256 `e9c1881e0e5281a511f16e063b95951535b36436fe1f7d1c3ec5c0051883cef9`, imported and rendered; Papeete search/popup and the offline Papeete Cathedral article were exercised. Evidence: `.native/evaluation/android-sdk2-torrent-5560`. This remains local emulator/private-peer coverage, not physical Android or every interruption-policy branch.
- The pre-SDK2 iOS simulator independently completed and imported a 47,218,674-byte BitTorrent ZIM with matching SHA-256, preserving 6,324,210 source-matching bytes through manual pause/process restart. A second scenario established genuine SpringBoard foreground ownership and native background delivery; its finite checkpoint ended after approximately 1.376 seconds, retired the active job to interrupted state, preserved 7,897,074 bytes with automatic resume disabled, then explicitly resumed to a matching staging hash. Second publication correctly refused an existing unowned filename; no second import is claimed. The original failed Home gesture remains explicitly unverified, not a product defect.
- The corrected, test-only read-only shell fixture delivery passed all 50 smoke-tier workload operations on each of API 33 and API 36, with zero failures/skips, unchanged source hashes, and no storage-reclamation credit. A symlink manifest was rejected without workload fallback. The first physical Galaxy A06 submission failed during fixture access before any workload; it supplies no device performance numbers.
- The separate corrected physical Galaxy A06 (SM-A065M, Android 15, measured 3.556 GiB RAM) passed all 72 observations across ten workloads, with zero failures/skips/incomplete runs. Both greater-than-4-GB outputs ran cold and warm: real Austria/Czechia/Slovakia composition took 103.239/98.304 seconds for 4,426,552,705 bytes; independent SHA-256 took 44.425/44.292 seconds. Each large mode has one sample. The shareable report, raw records, provenance and SHA-256 index are in `.native/evaluation-combined/physical-a06-report-528596c4-7482-4636-b02f-692734d9ac13`. App-cache cold/warm does not control OS caches; sampled memory is a lower bound, staging is separate, and the frozen JNI benchmark does not certify newer SDKs or interactive WebView memory.
- Shareable physical reports now cover Galaxy A06, Galaxy A03s, Galaxy A16 and Infinix X6525, with actual measured RAM variants and raw integrity/storage/phase evidence. A06, A16 and Infinix executed both greater-than-4-GB workloads; A03s explicitly skipped them under the retained-fixture storage contract. See [Android performance](docs/ANDROID_PERFORMANCE.md) for per-device numbers, portable ZIP digests, sample counts and limitations. Five physical submissions consumed today's free-only allowance, including the first fixture-access failure; the fifth distinct planned Motorola model remains unmeasured until fresh free quota.
- An earlier frontend build and **206-test baseline** passed atomic annotation mutations, UTF-16/UUID/path bounds, XML split-text/trailing-payload rejection, history aliases, guarded publication, Win32 reservation aliases and retirement-document authorization. The macOS debug package was rebuilt with eight bundled Mach-O images and an ad-hoc signature; signing is not Apple authentication or notarization.
- The macOS debug `.app` was bundled with eight Mach-O runtime images, ad-hoc signed, and launched into an observed native window. A packaged WKWebView initially failed with ATS `-1022`; a `localhost`-only HTTP exception fixed archive loading without global arbitrary loads. Actual native reading, same-frame navigation, retirement to Library, and the native Quit validation/flush/acknowledgment path completed. Quit exited with code 0 and retained all fixture writes.
- The current schema-2 macOS debug package was rebuilt with final compact-reader styles, closed over eight arm64 Mach-O images and passed strict/deep ad-hoc signature verification. Actual native WKWebView reading/internal navigation, the 52px compact bar and on-demand tools/tabs at 760px, the original desktop toolbar at 1280px, reader retirement, retained localStorage and native Quit/restart persistence passed in an isolated portable profile. Executable SHA-256 `52005c1788052bfcef6a0eef77fb835ebad3ca8101c405ed328b501237ce2d50`; evidence `.native/mac-sdk2-package-20261029`. Actions used an in-process observation harness, not physical clicks; application-owned snapshots prove static SVG fixture rendering, not GPU map pixels or clean-machine release support.
- Global screen-recording permission is denied. An app-owned WKWebView snapshot captured chrome, but its GPU map canvas was white: WebKit's software snapshot limitation is documented in [bug 221662](https://bugs.webkit.org/show_bug.cgi?id=221662). Native map title, controls and resources loaded without reported errors; native GPU raster appearance remains visually unverified. Chrome rendered the actual map.
- A real BitTorrent job received 2,159,190 bytes from an independent aria2 seeder, published/imported, and matched SHA-256 `4d586bf8214ec73339fd719712643e2ab73894ad6fa1cace5c1e0efb43c546b1`. This is single-host peer interoperability, not DHT/PEX/air-gapped or cross-device acceptance.
- Live StreetZim production composition built/imported English Hawaii, 150,605,015 bytes. Stored and independently recomputed ZIM checksum matched recipe MD5 `1595dadd1f60f8d157a0f6cfbeb2f776`. The real offline vector map, labels and controls rendered in Rust-service/Chrome. Windows/Linux execution remains unverified.
- Dual-stack LAN requests returned only selected Hawaii. Per-book Continue activated an HttpOnly cookie; authorized archive access succeeded, missing credentials/foreign Origin returned 403, and Stop rejected subsequent connections. Cross-device acceptance remains pending.
- A copied-UUID archive was rejected without changing the original installation. Explicit relink rotated installation/origin, removed old site data and retired the old listener. HTTP acquisition targeting a temporarily missing installed archive was rejected before publication; the retained original hash was unchanged.
- Genuine lifecycle testing reproduced that removing an iframe emits no pagehide in the Chromium control, whereas same-frame navigation does. With authenticated retirement, twelve rapid Reader → Library → same-Reader cycles retained all 24 synchronous/nested writes plus the four capture/bubble fixture values. Session cookies survived in memory and were absent from Profile.
- Actual canceled freeze restored the active Reader, query and fragment. Holding the real retirement response beyond its deadline produced an explicit failure; rollback retained the same iframe, restored activity/location and committed genuine pagehide writes. These observations are not a blanket BFCache or crash-durability certification.
- The sampled lock-order deadlock no longer occurred in the actual concurrent scenario: 50 storage writes plus 50 visits committed in 1,274 ms.
- Title pagination, real dictionary correction `Paginatoin` → `Pagination`, and bounded plain-text literal-delimiter previews passed. Mixed-case/parameterized HTML and XHTML MIME fixtures rendered and activated their reader bridges. A confirmed 593-byte PDF Open launched Preview with the actual extracted file; bytes matched SHA-256 `158b55faac12daa492bb43a43d6ce67b846b2ef323f6938599507845df80822f`.
- Actual trusted Unicode bookmark editing, unified note search and section filtering/open passed. Two saved titles reversed correctly under all four reading-list sort modes; a science tag kept only the matching item. On the storage boundary archive, an oversized Unicode cookie threw before advancing sequence; a subsequent ordinary write committed. A burst hit synchronous credit backpressure after three large replacements, and a small recovery write plus all accepted state persisted through retirement without an alert.
- Actual Unicode highlights/comments persisted across application restart and genuine Reader remount. Repeated text resolved to the second paragraph using context; hidden passage Jump expanded its Details ancestor; ambiguous duplicated context stayed unpainted and absent dynamic content reported unavailable. New highlights on a URL-valued query retained their exact document identity instead of merging with the base page.
- Failing-before/passing-after archive scenarios covered a long legacy bookmark ID rejecting an entire batch, excluded-control text creating false ambiguity, and fallback paint displaced at 200% zoom. After correction, all applicable ranges resolved and the fallback visibly covered the intended Unicode passage.
- Concurrent native metadata/annotation changes survived actual trusted comment saves using atomic deltas. Delete sent only its targeted operation; a subsequent stale comment failed without resurrection. Ordinary reading-list note editing retained a highlight added while that editor was open. Reused highlight UUIDs in different bookmarks retained separate comments.
- An actual archive message listener reproduced exposure of personal comment text in annotation messages. After the explicit projection fix, every captured annotation contained only rendering-anchor fields and an opaque bookmark token; comments stayed in the shell while painting continued.
- Real JSON/XML exports imported into a separate native profile with identical annotation data and the long legacy ID intact; reimporting JSON added no duplicate entries.
- Actual empty-tab searches used all available archives without changing selection: title hits came from two sources, and full text retained its indexed result alongside three identified missing-index errors. Reader result totals now use the same approximate-count label as Library instead of presenting native estimates as exact.
- The opt-in section-expansion preference persisted to Profile; a real collapsed Details opened on the next activated reader load and returned to archive-authored collapse after disabling/reloading. The single-frame/window model remained intact.
- Actual Arabic chrome and annotation controls loaded in RTL with `lang=ar`; obvious annotation-context translation errors were corrected, still marked unreviewed. The current inventory has 54 complete and 20 explicitly partial locales after new consumer copy; linguistic review remains outstanding.
- Post-handoff normal/private/normal transitions used fresh origins, hid normal values in private mode, kept private values off disk and restored the normal cookie/data. Strict read-only disabled annotation mutations, rejected direct bookmark writes and kept profile bytes unchanged while archive storage remained transient. An actual `--read-only` restart refused disabling the flag/imports and left profile bytes identical after exit.
- Portable failing-before/passing-after regressions covered significant trailing spaces/dots and Windows device-prefix normalization; thirteen acquisition/reservation tests passed. This is not a Windows runtime test.
- Earlier **63-test baseline** scenarios covered Fetch/WASM/Worker (42), WebGL availability, blocked service workers/external fetch, multilingual UTF-8, collapsed Details find, exact HTML/MHTML/image export, Unicode note drafts/tags/full-location identity, verified Metalink and HTTP pause/resume, live OPDS, and normal/private/normal storage. These older observations do not silently certify subsequent privacy/lifecycle changes.
- That baseline also played a real three-second H.264/AAC ZIM video at 160×90, sought to 1.5 seconds and exported exact bytes, SHA-256 `76b7bb807ef0b0b9477a042237aebe19560539d0e5c409ebcafe12960be9f798`. Chrome needed active-page emulation. This is not universal codec or packaged-WebView acceptance.
- Archive-named camera Block and immediate remembered denial passed. A location request with acquisition timeout 2,000 ms was still pending after 2,834 ms at app consent; Block returned denial code 1 at 2,890 ms. Headless status correctly advertised no native provider and direct native location failed with code 2 rather than hanging. Physical capture, a location fix and delayed OS-grant acceptance remain unexercised. Arabic RTL chrome was observed; linguistic review and all-locale visual acceptance remain outstanding.
- Windows execution, clean-machine packages, native print/speech, screen readers/virtual keyboard, real external-drive/FAT/crash behavior and representative archive compatibility remain unverified. Linux ARM64 debug fixture acceptance is recorded above, not universal Linux packaging. Service-worker-dependent content, universal inline PDF/EPUB and full Qt/Apple settings/session/job migration are not supported.
- No signed independent updater, published release/store listing, Flatpak/MSIX/PortableApps channel, physically single-file Windows distribution or universal macOS binary is claimed. ARMv7 SDK/APK compilation and isolated ARM32 native execution do not establish full app or physical-device acceptance. Runtime SDK architecture/ABI/deployment-target closure remains mandatory.
- The launch-path correction is covered by two regressions: `launch_tests::archive_launch_locations_encode_the_main_path_exactly_once` (fails before the change with `main%2520page…` where the archive expects `main%20page…`) and `service::tests::main_path_with_delimiters_percent_and_unicode_round_trips_once` over the new `tests/fixtures/main_path.zim`, whose main path is `main page 100% ?#é.html`. `cargo test --lib --features localswarm-profiling` against the schema-2 macOS SDK passed **572 tests, 6 ignored**; `npm run build` passed. Runtime proof came from the headless authenticated loopback build started with the delimiter and StreetZim fixtures: before the fix `open_article` posted `path: "index%252Ehtml"` and the loopback API answered `{"error":"Entry not found: Cannot find entry"}`, leaving a blank reader frame under the global banner; after the fix the same launch posts `index%2Ehtml` and renders the main document, and a restored session tab deliberately set to the legacy `index%252Ehtml` opened the main page with the quiet in-reader notice, no banner, no failing request and no retry, saving the healed `index%2Ehtml` path back. One earlier full-suite run hit an environmental failure in `maps::tests::unverified_local_base_stops_before_fallback_network_and_confirms_real_cache_growth` (it passes in isolation in 1.8 s); no maps code was touched.
