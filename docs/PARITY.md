# Feature parity with the existing Kiwix applications

Source-level audit of this tree against the three shipped Kiwix applications: `kiwix-desktop` 2.5.1 (Qt, Windows/Linux), `kiwix-apple` (iOS/macOS) and `kiwix-android`. Statuses below are read from source, not runtime-verified; the per-platform tables live in the audit receipts named at the end.

The [2026-11-01 review](audit/REVIEW-2026-11-01-48H.md) records the original findings
and repair disposition. All 26 findings have integrated fixes and passing host
regressions. Implementation scope is not release acceptance or complete parity.

## Latest status — 2026-11-05

### 2026-11-12 — upstream follow-ups

- Advisory download admission now subtracts the remaining bytes of other active
  HTTP/native/shared-library acquisition jobs on the same volume, matching
  Apple's merged 3.18.0 free-space correction. Torrent records and LocalSwarm
  multi-file owned-set transfers retain separate capacity guards. Per-job hard
  capacity guards remain; a real two-download near-full-volume run is still the
  acceptance gate.
- Archive lookup tries the exact entry first, then one trailing-slash variant
  only after a recoverable `EntryNotFound` (kiwix-js #1231). Distinct
  `path`/`path/` entries and non-entry archive failures are preserved. The real
  Mathematics ZIM’s slash-only `+-*/` entry was reached through the fallback
  from `/%2B-%2A` (307 to `/Arithmetic`).
- Android #5271/#5270 are recorded as packaged TalkBack/Switch Access
  acceptance items; current labelled HTML Settings avoid the upstream Compose
  hidden-row and test-ID mechanisms. Android 17 API 37 and Xapian 1.4.32 are
  implemented for the app/desktop recipes with build and focused-search
  evidence; physical Android 17 permission/transfer acceptance and Android/iOS
  SDK rebuilds remain platform gates.


### 2026-11-06 — cross-platform sharing reliability

Same-LAN Mac↔Android and Android/Mac↔iPhone discovery, shared-catalog browsing
and download are now physically verified. The Mac→Android StreetZim download
completed; the Android→iPhone run reported 6 archives/362.5 MB sent and the
Mac→iPhone StreetZim download completed. Library discovery uses a
session-specific DNS-SD instance with an optional generation hint; older
sessions remain selectable under a disclosure. A `testNearbyCancel` regression
on the dedicated iPhone 13 Pro test device confirms that opening a catalog and cancelling leaves no
duplicate peer and releases the action gate immediately. The remaining sharing
gates are physical BLE/hotspot join/cancel/route-loss, AP-only routerless
transfer, reverse direction and phone-to-phone throughput; the Mac remains
LAN/Bonjour-only. See [the comparison](PLATFORM_COMPARISON.md#same-lan-sharing-acceptance--2026-11-06).

The current Android APK is smaller than official ARM64 Kiwix, but both its
standard and separately readiness-settled 32-sample Animal cohorts still favor
official on total PSS. A smaller application-package size-optimization trial
established no whole-app memory/startup win and was not promoted.
Archive-bootstrap ownership and idle HTTP/TLS-client initialization were
improved without dropping features; the normal optimization-level-3 build is
restored. See [the comparison](PLATFORM_COMPARISON.md),
[lifetime receipt](benchmarks/2026-11-05-android-idle-memory.json) and
[compiler trial](benchmarks/2026-11-05-android-root-size.json).

Current native real-Wikipedia recovery and real HTTPS Hawaii composition/import
passed. Physical production Animal/Hawaii/Hilo rendering also passed.
Separate ARMv7 real-archive and >4 GiB LP32 checks passed within a 64-bit OS with
a 64-bit renderer; an Android compatibility warning remains. Silent AX-provider
observations do not certify physical TalkBack. These close bounded acceptance
gaps, not complete platform parity. Apple/Windows comparisons, broader physical
providers/routerless pairs, localization review and distribution remain open.

## Earlier status — 2026-11-04

The optimized ARM64 Android APK is now below the pinned official Android APK
without removing reader features or locales. The latest optimized Android and
signed iPhone builds are installed in place, with captured profile/native
preference preservation and separate successful launch-command receipts.
These updates do not recertify all earlier physical reader or accessibility
checks. Strict host/Android/iOS production Clippy passed; current host tests
passed 946/16 ignored and JavaScript tests passed 211.

Performance remains an explicit improvement criterion, **not completed parity**.
Real Wikipedia/StreetZim bounded memory cohorts and sampled content-ready
starts did not establish a reliable memory or startup improvement; warm native
search timings were mixed, including regressions. Keep reducing reader/runtime
allocation and measuring end-to-end search/content readiness, long-session
memory and comparable Apple/Windows behavior without removing functionality.
See [the current comparison](PLATFORM_COMPARISON.md) and
[measurement receipt](benchmarks/2026-11-04-reader-efficiency.json).
A later [response-path correction](benchmarks/2026-11-04-response-path.json)
demonstrates substantial reductions in **additional construction allocations**
and constructor elapsed time on real Wikipedia/StreetZim HTML, plus faster warm
debug HTTP delivery. Both rebuilt phone updates were installed with captured
state preservation and ordinary launch evidence. This is concrete incremental
efficiency work, not a reversal of the earlier settled-PSS/readiness findings or
proof of shipping-device performance superiority.
Localization, physical provider/media/routerless pairs, remaining assistive-
technology acceptance and production distribution are still open.

## Earlier status — 2026-11-03

The owner-authorized Samsung→iPhone Hawaii transfer completed with source-matching
bytes and approximately 29.0 MB/s effective admission-to-completion throughput;
guarded cleanup preserved the recorded original data. Historical slowdown cause,
reverse direction and routerless joining remain unproven.

Incremental source-scoped catalog reconciliation and explicit verified full-edition
replacement/rollback are implemented and host-runtime exercised, including live
reading references and unsaved drafts. Separate authenticated private reading-data
sync passed real two-process and two-browser pairing, manual/automatic transfer and
explicit conflict-resolution scenarios. The latest host suite passed 943 Rust
tests/16 ignored and 211 JavaScript tests. Rebuilt owned Android/iOS virtual-device
runs now cover the scoped native flows in [RELIABILITY](RELIABILITY.md), including
Android TalkBack internal navigation and iOS mutually confirmed private-note delivery.
This is not complete mobile or assistive-technology acceptance. The current
reviewed iPhone build is installed with prelaunch preservation passed; subsequent
normal launch and physical portrait-reader bottom-edge inspection passed.
Post-launch title/migration-source differences are recorded, not an exact State
match. Broader physical acceptance remains open; Android's phone package is older.
Competent locale completion remains open.

Actual owned Mac/headless checks now cover original video/audio/image context
saves with matching completed-download hashes, and persisted link-preview opt-out,
restart, re-enable and keyboard dismissal. Export directory-sync failures retain
the completed destination and report uncertain power-loss durability. Bounded
same-archive raster artwork now extends text previews; real-ZIM path/fallback
regressions and actual browser decoding passed. Native-picker/mobile thumbnail
acceptance and strict Clippy cleanup remain open. See [RELIABILITY](RELIABILITY.md).

Actual Mac VoiceOver output now covers the scoped map/detail reader navigation,
dialog-close announcement and native modal/Find focus return. Body-paragraph
speech after dismissal remains unverified even after explicit frame navigation;
this is partial acceptance, not a full VoiceOver pass or diagnosed product defect.

Linux acceptance also covers native printer-dialog cancellation followed by
reader navigation and reopening a second article's dialog; no print job was
submitted. An earlier Linux–Mac Nearby run genuinely discovered the publisher
but could not browse across the VM's NAT topology.

A subsequent owner-approved **real bridged LAN** run passed Samsung→Linux
(150,605,015 bytes) and Mac→Linux (22,783,565 bytes), with source SHA-256,
library import and main-page HTML serving. These use a retained reviewed ARM64
Linux headless build, not proof of every newer source change or native GUI
rendering. Reverse directions, multi-archive batches and Raspberry Pi remain
unverified. See [RELIABILITY](RELIABILITY.md) for exact builds and cleanup limits.

A separate real isolated ARM64 Linux namespace run reproduced and repaired a
multi-address selection bug: a connected off-link candidate no longer masks a
later usable on-link candidate. Inventory and the full fixture's source SHA256
passed afterward; off-link-only access stayed denied. This numeric transport
proof does not certify discovery, Linux–Mac interoperability or Raspberry Pi.

Actual owned headless-browser acceptance now covers live-document SPA query/path
Back/Forward, normalized router history state, repeated URL occurrences, TOC
scroll/Details preservation and genuine cross-document navigation. A synthetic
unanswered traversal also exercised retirement and fresh-origin fallback.
These scoped fixes address current reader issue dispositions, not blanket
archive, restart, native WebView or mobile parity. Already-latched profile
durability failures now block archive removal before file deletion while active
download Pause/Cancel remain available. See [RELIABILITY](RELIABILITY.md).

## Earlier status — 2026-11-02

The dated sections below retain historical source and acceptance snapshots;
newer [RELIABILITY](RELIABILITY.md) receipts supersede older locked-phone and
unexercised-background statements for the specific scenarios they prove.

Nearby source now uses one open-by-default encrypted-QUIC whole-library path
after explicit Share all, without invitation codes/PINs. Receivers can choose
all or several archives; browser/OPDS HTTP is separately enabled in Settings.
Default mobile Add files/Add folder references originals, with copying a
separate explicit action. Final host selected/concurrent QUIC downloads, exact
hashes, import/read and Download-all skip/add passed after the native fix; the
earlier bulk/native failures remain pre-fix evidence in RELIABILITY.
Reviewed final mobile builds were installed in place with both phones' state
preserved. The subsequent Samsung ULA metadata/on-link correction passed actual
Share all, advertising, inventory browsing and verified Samsung-to-Mac transfer.
Both corrected packages are installed. After fresh owner authorization, the
Samsung→iPhone Hawaii transfer completed with source-matching SHA-256 and
`quic-library` provenance; guarded cleanup restored the original iPhone library
and preserved both phones' reading data/settings. The earlier lock/authentication
cancellation is historical, not the current transfer result. Historical speed
variation, reverse-direction/routerless joining and physical original-file
intake acceptance remain open; isolated-simulator ownership/recovery passed.
The reviewed iOS build now uses `com.tiltastech.zimreader`; its actual app/widget
signatures and profiles passed independent inspection, including both required
Wi-Fi entitlements. An earlier physical iPhone installation/launch/native smoke
was a fresh installation after user removal, not migration evidence. The later
in-place update preserved three books, one tab, bookmarks and history. The earlier
Nearby surface XCTest was surface-only; subsequent phone-pair transfer and
cleanup evidence is recorded in RELIABILITY.

The preceding Download-all verification was 147 JavaScript tests passed, production and
target-specific frontend builds, a desktop executable and both arm64 mobile
packages. Both phones received those builds in place. Samsung cold launch
and actual three-archive Download all/import/hash/cleanup passed. The iPhone
install preserved its persisted state; first launch was blocked by its lock.
The preceding 822 Rust tests (11 ignored), real DNS-SD restart/withdrawal check
and earlier physical iPhone background/media acceptance remain distinct
evidence, not rerun or inferred from this frontend-only batch.

Recognizable publisher names and publisher/discovery identifiers are implemented
in receiver rows and Choose's accessible labels. **Nearby → Choose → Download all** adds missing archives from the
current inventory without deleting unrelated local files or automatically
sharing received files. It reports partial results and preserves existing
per-file admission/consent and cancellation boundaries.

Physical Bluetooth connection assistance, automatic joining/selected-route
acceptance, assistive-technology and competent locale review remain distinct gates.
New private-sync and catalog/edition host acceptance is recorded above; mobile
runtime acceptance is not inferred from it.
Complete version-scoped predecessor migration is deferred backlog by user
priority, not a blocker for those tasks. Physical lock-screen sound/headset
checks remain user-deferred. Service-worker-dependent content remains
unsupported; Windows has no native runtime acceptance. No overall parity
percentage is established.


## Current changes — 2026-11-01

- iOS: OS-owned background downloads for backend-proven official Kiwix HTTPS
  acquisitions; custom URLs retain strict foreground transport. The restricted
  native route uses user-approved iOS redirect policy. Offline reconciliation
  is independent of auto-resume; explicit pauses and private/read-only cutovers
  remain authoritative. Physical suspended/relaunch delivery is not certified.
- Apple: valid stale provider bookmarks refresh only after archive UUID and
  every part's identity match. Already-stale legacy references without identity
  evidence require reselection. iOS speech pause/resume/previous and Dynamic Type
  shell scaling are implemented. macOS window groups retain membership, order,
  selection and focus across restore.
- Search: default combined title + indexed full-text results includes archives
  without a full-text index. Title-only/full-text-only modes remain available.
  LAN readers now have scoped search, suggestions and paging.
- Android: selected archive copy/move, external ZIM/zimhost entrypoints,
  forward/previous/timed speech, per-app language and translated widgets, and
  article-PDF sharing are implemented. Timed seek estimates text offsets from
  observed speech rate, not sample-accurate audio positions.
- PDF captures the current authenticated page, live form/details state,
  same-origin nested documents, open shadow DOM, canvas/video stills and frozen
  layout. Inaccessible resources fail explicitly. Android sharing uses public
  PdfDocument/WebView drawing; it can rasterize and does not promise print-engine
  links, tagging or CSS page-break behavior. Windows/Linux have native printer
  selection and printing; target-runtime acceptance remains outstanding.
- Shared split/provider archives expose one retained logical byte stream, with
  ranged acquisition/HEAD support rather than temporary concatenation.
- Native menus/new controls use 74 locales and 1,296 sources: 59 catalogs are
  string-complete. The 15 incomplete locales are `dag`, `gsw`, `ie`, `igl`,
  `io`, `kab`, `nqo`, `sc`, `sdc`, `skr-Arab`, `sms`, `sro`, `zgh`, `zza`,
  and `roa-Tara`; explicit English fallback remains pending competent
  exact-language translation and native review.
- Release workflow and signed staging exist. Production updater keys,
  Developer ID/notarization and Windows signing/runner prerequisites remain
  external gates; the base build has no fabricated key or update URL.
- BitTorrent: visible catalog action, top-level URL/magnet entry, upload metrics
  and opt-in seeding controls. Official publisher magnets resolve lazily with
  backend provenance and bounded HTTPS/privacy/lifecycle admission. Actual
  official one-click download/import/read and controlled real-engine
  magnet/`.torrent`/pause/resume/seeding checks passed. Latest mobile builds
  passed (`22344`), subsequently installed on both phones; Android process
  launch passed, iOS launch was blocked by normal Locked policy.
- Sharing: Nearby **Share all** starts an explicit open whole-library session
  in one click, with automatic LAN IPv4/port (or the Android app-owned AP address
  for temporary Wi-Fi) and one scoped library advertisement. Selected
  installations/PIN/advanced binding remain optional. Recipients browse and
  download through current-peer/session/native-route authority; ordinary
  arbitrary LAN HTTP remains rejected. Same-LAN Mac↔Android and
  Android/Mac↔iPhone discovery, catalog and download are now verified; physical
  BLE/hotspot join/cancel/route-loss, AP-only routerless transfer and reverse
  direction remain outstanding. QUIC/AWDL per-file transfers remain separate.
  No local BitTorrent cutover or measured gain.

Verified: frontend build; 795 host Rust tests passed (nine ignored); 107
JavaScript tests passed; 22 native iOS simulator XCTest regressions passed;
arm64 iOS build/export; Android plugin tests and full debug APK build.
Actual corrected iOS simulator launch/import/read/Back/background/cold restart
and native speech controls were exercised after repairing the non-retained
`UIApplication.delegate` reassignment crash with a class-level
pre-`UIApplicationMain` handoff. Evidence: `local://AcceptIOS-acceptance.md`
and `/tmp/kiwix-accept-ios-evidence/`.

Integrated finite checkpoint terminal cancellation preserves genuine persistence
errors; cancellable reader storage admission waits for actual admission. PDF
identity uses the live URL with percent-dot spelling equivalence, corrected
descriptor ownership and Android visible cancellable preview.
Android actual picker/admission/read/lifecycle, PDF recipient and typed preview
cancellation checks passed; these are exercised scenarios, not blanket certification.
Latest iOS simulator install/launch and maximum-Dynamic-Type navigation passed
on artifact `22088`: native taps/horizontal swipes reached all seven destinations
without error bands (evidence in RELIABILITY). Physical hardware and assistive
technology remain separate. Both phones subsequently received the verified
pre-sharing repair/BitTorrent build (`22344`); physical iOS launch was blocked
by normal Locked policy. Physical iOS URLSession suspended/relaunch
delivery, Windows/Linux printers and signing/distribution credentials remain
external prerequisites; simulator `nsurlsessiond` failure did not certify
official native download completion.
Actual iOS simple-fixture PDF share rendered Main page/H1 Main in Apple Preview
(`/tmp/kiwix-final-ios-pdf-recipient.png`), not complex-page/cancellation proof.
After the final Android root-producer repair, focused `live_pdf_` tests passed
2/2, then the full host suite passed 771, nine ignored (artifact `22172`).
Latest iOS arm64 device build/IPA export passed (artifact `22173`), without
physical installation. Android recipient `22209` rendered Main without native
scrollbars; final APK `22227` typed cancellation prevented write/chooser/handoff
without error bands and left Main readable. Evidence: RELIABILITY and
`/tmp/kiwix-android-finalqa-5586/`. Final desktop asset build passed (`22230`).
Complex-page fidelity, physical hardware and assistive technology remain limits.

## Earlier implementation inventory — historical

The inventory below predates this repair pass. Limitations superseded above
are retained as audit context, not the current implementation contract.

Library import/relink/delete with proven ownership, catalog/OPDS browsing with facets and conditional offline cache, concurrent downloads with pause/resume/retry and speed/ETA, torrents (including creating a share), LocalSwarm sharing, maps, search with snippets and spelling, reading lists with notes/tags/anchors, history, tabs, reader appearance/zoom, TOC, find-in-page, TTS, attachments, integrity verification, private/read-only modes, multi-window, portable profiles, update discovery, folder import, per-installation data isolation. Several upstream features are inert in their own code (reopen-closed-tab, browse-library, help/feedback actions are created then hidden in `kiwix-desktop`), so they are not counted as gaps.

## Real gaps, ranked by user-visible value and effort

| # | Gap | Platforms | Effort | Blocker |
|---|---|---|---|---|
| 1 | ~~**Whole-library Kiwix server + OPDS** — serve every library book with a catalog feed and `/catalog/v2`, choose port and interface~~ **done** — explicit `scope: "library"` start, token-scoped index, `/catalog/v2/root.xml`, `/entries`, `/categories`, `/languages` and `/download/{uuid}` acquisition links; per-book listeners are allocated lazily and re-verify the library identity on every request | Win/Linux, macOS | Medium | Third-party reader interop unverified |
| 2 | ~~**Type-ahead title suggestions** in search (debounced suggestion searcher with paging)~~ **done** — the Library article search debounces 120 ms and calls a cancellable native `suggest` command; the popup lists title + book with arrow/Enter/Escape navigation, click-to-open and continuation paging on scroll or arrow-past-end. Suggestions are title-only (no snippets, no full-text fallback row), scoped to the explicitly selected archives, share the search owner's cancellation scope, and continuation tokens are bound to the prefix and resolved archive scope. With no archive selected there is no suggestion scope, matching `kiwix-desktop`'s selected-ZIM requirement. Native cancellation/paging and fixture paging are covered by Rust tests; the popup has not been exercised on a physical device | All | Medium | Device runtime check pending |
| 3 | ~~**Live monitored-folder watching** with a stability debounce and in-flight-download exclusion~~ **done** — `watch.rs` runs the desktop-only `notify` backend (FSEvents/inotify/ReadDirectoryChangesW) over the configured monitor directories, settles each touched directory and candidate archive for ~1 s, and hands only settled observations to the existing monitored scan (`service.rs::apply_watch` → `import_monitored_paths`), so identity, claim exclusion and integrity rules are unchanged. Disappeared files retire the library entry without deleting anything, except while the book is open in a tab (matching `kiwix-desktop`). Private/read-only modes unwatch everything and re-check under the privacy read guard; unchanged failed imports are not retried; the 15 s periodic scan remains the fallback for backends that emit no events (network shares) and a 30 s directory re-check covers coalesced events. Verified on macOS: drop → entry in 1.15 s, delete → entry gone in 1.27 s, 4-chunk slow copy → no partial entry, imported 1.16 s after completion; claimed downloader output is excluded by the integration test | Desktop | Medium | Network shares may not emit native events; huge directories fall back to the periodic scan |
| 4 | ~~**Android device-wide ZIM discovery / in-place library** (MediaStore scan, read where the file lives)~~ **done** — a new native `DeviceZimScan` walker enumerates MediaStore rows, a bounded primary-shared-storage walk on Android 12L and earlier, and every persisted SAF tree grant; Rust `device_scan.rs` validates names/tokens and deduplicates by volume + canonical relative path + size + mtime in seconds, merging a MediaStore and SAF sighting of one file and grouping split sets only when the parts are complete. Library gains an Android-only **On this device** panel (name, size, volume/location, split/in-library/unreadable markers, Scan, Add folder access, Open in place, Import a copy). Open mints the same opaque provider reference the explicit picker uses, so reading goes through the existing retained-descriptor/libzim path with no copy. A granted tree can be remembered in normal mode; a MediaStore-only authority is session-scoped in read-only and otherwise refused with an explicit import path. Private mode refuses scanning and folder grants; read-only never persists. No network and no all-files permission: Android 13+ exposes no shared-storage read grant, so those releases depend on granted folders and the panel says so. | Android | Medium-large | Android 13+ MediaStore visibility; physical removable storage unverified |
| 5 | ~~**Removable/SD-card managed storage, storage summary, copy/move**~~ **done** — Android Settings lists internal app storage and every `getExternalFilesDirs` app-specific external directory (including a removable card) with free/total space and the active location. Choosing another root runs a new `library_root` journaled migration: each managed archive is verified through the existing libzim integrity gate, copied to a private staging directory on the destination volume, verified again, published without replacing an existing name, and only then are the library paths and `downloadDirectory` committed in one atomic Store update. Old copies are removed only when the user opts in after verification; the default keeps them. An interrupted copy/publish rolls back to the previous root on the next writable start; an interrupted cleanup finishes when the Store already points at the new root. A free-space check refuses an oversized copy, private/read-only modes refuse the switch, and the switch is refused with a plain “Finish or cancel your downloads, then switch storage” message while a queued/downloading/verifying/paused transfer or active map job still targets the old root. Archives are still served only from app-owned paths: the removable choice is `Android/data/<package>/files/archives` on that volume, never a raw SD-card path. Emulator acceptance covers location selection, copy, reopen and the summary; a physical card remains unverified. | Android | Large | Physical removable-card acceptance |
| 6 | ~~**Richer Android download notification** — pause/resume/cancel/retry actions, group summary, completion Open~~ **done** — per-HTTP-job progress and authenticated, state-bound Pause / Resume / Cancel / Retry intents use the ordinary foreground Downloads command/confirmation paths. Native ownership lasts through action execution or cancellation-dialog dismissal; acknowledgement atomically retires the issued identity. Private/read-only transitions revoke queued actions and notices. The existing foreground-service Pause-all action remains; its explicit group-summary flag is set only while multiple HTTP jobs are active (Android may independently auto-group retained notices). Opt-in completion Open comes only from a new verified-admission receipt and revalidates the current installation. Four focused Rust helper tests passed; an optimized production APK on emulator-5560 exercised two real HTTP transfers, the explicit group and per-job progress/actions, Retry, Pause, Resume, Cancel→Close retention, confirmed Cancel, and a completion-notification tap opening the verified archive. Receipts: `.native/notification-parity-20261031/`. | Android | Medium | Physical/OEM notification UI, cold-process tap and permission-denial/grant UX remain unverified |
| 7 | **Read-aloud that survives backgrounding** with transport controls — Android now runs native read-aloud inside a `mediaPlayback` foreground service with an active `MediaSession` and MediaStyle notification (play/pause/stop/previous paragraph). Transport actions and media buttons re-enter Rust (`speech-action`) and cross the same capability/ownership path as the UI; the engine never acts on an unvalidated request. A started session is a suspension exemption: the Android background transition retains the reader origin, privacy generation and archive cache, and the first terminal speech state retires them exactly once. The iOS background transition now preserves a started speech capability instead of blocking it; prepared-only sessions still retire. The optional Kokoro engine uses the same ownership path and its own audio host. Privacy/read-only, article/tab changes, explicit stop, Activity destruction and task removal still stop and release immediately. The reader bridge preserves paragraph boundaries in extracted text. APK build and the 557-test desktop suite pass; emulator end-to-end speech, background continuation and notification-action evidence still require a run on the debug emulator | Android | Medium-large | Runtime emulator evidence pending |
| 8 | ~~**Hotspot-integrated LAN serving**~~ **done for serving** — `sharing_start` accepts `binding: {"mode":"hotspot"}`: the listener binds the live LocalOnlyHotspot access-point address resolved from the interface table (never the client Wi-Fi link or a vendor constant), the Settings sharing controls show the access link and its QR, and leaving Nearby no longer drops the link a joined peer is using. Explicit Stop hotspot/Stop sharing/Stop all, network-policy retirement without a reservation, privacy/read-only transitions, background retirement and shutdown release the server and reservation coherently; a loopback share survives local-network loss. Emulator-proven with Wi-Fi off: index fetched from the AP address, both retired on either explicit stop. A persistent “serving” notification is **not** implemented (it needs a new foreground-service type/notification flow, which this change did not add) | Android | Medium | Serving notification remains |
| 9 | ~~**iOS Live Activity / Dynamic Island** download progress~~ **done** — a new `KiwixDownloadActivity` widget extension (embedded from `src-tauri/gen/apple/project.yml`, bundle id `org.kiwix.workbench.DownloadActivity`, deployment target 16.4, `com.apple.widgetkit-extension`) renders an `ActivityConfiguration` with lock-screen, expanded/compact/minimal Dynamic Island presentations. `src-tauri/src/live_activity.rs` projects `DownloadManager::list()` onto it once per second from the existing mobile maintenance tick: one activity follows the most recently updated `queued`/`downloading`/`verifying`/`paused` row (terminal rows end it after a short confirmation, a row that leaves the list dismisses it), and private/read-only/unavailable-notification states clear the surface instead of publishing content. `plugins/tauri-plugin-kiwix-mobile/ios/Sources/LiveActivity.swift` is the ActivityKit bridge (one activity at a time, reconciled from `Activity.activities`), driven by the same native plugin transport as the other Rust-only commands. ActivityKit needs 16.1+ and the app floor is 16.4, so no runtime fallback is required; the island presentation exists only on island-capable iPhones while iPad and older phones get the lock-screen banner. A simulator run with a real throttled loopback download started the activity and updated it through `downloading` and `paused` with ActivityKit `active` state and a `WidgetRenderer-Activities` session carrying Jindo island metrics; that run stopped before completion, so the terminal/dismiss presentation is covered only by the projection unit tests, and `simctl` screenshots of this simulator do not composite the island overlay — the visual island/lock-screen capture, the completion capture and physical-device behaviour remain unverified | iOS | Medium | Physical-device visual capture |
| 10 | ~~**macOS window tab groups** and an explicit “restore previous session” command~~ **done** — a new `native/tabbing.mm` bridge supplies what Tauri does not expose: it enables `NSWindow.allowsAutomaticWindowTabbing` before the first window, sets `NSWindowTabbingModePreferred` plus the shared `org.kiwix.workbench.reader` tabbing identifier on the config-created main window and every later shell, and installs a native **Window** menu with AppKit responder-chain actions — Show Previous/Next Tab (`Cmd+Shift+[` / `Cmd+Shift+]`), Merge All Windows, Move Tab to New Window, Show Tab Bar, Minimize, Zoom, Bring All to Front. The chosen model is two levels: the native tab bar groups whole windows (one native tab per window) while the existing in-window strip stays the reader-tab model, so `Cmd+T`/`Ctrl+Tab` keep their reader meaning and `Cmd+Shift+N` opens a window that joins the group. Window titles follow the active article (the title bar stays hidden) so native tab labels are distinguishable. File → **Restore previous session** calls a new `restore_session` command over `last-session.json` (0600, atomic replace), which the running app keeps within one second of the live session (and rewrites at a graceful Quit) because AppKit's `terminate:` Quit path bypasses Tauri's exit event; it contains only the windows that were open and their tabs/active tab/reader surface/geometry, and an empty session never erases the last session with tabs. The command reopens those windows, adopts the saved tabs into an empty invoking window, never replaces live reader tabs, and is refused in private mode, in read-only profiles and when no session was saved. Private/read-only profiles never write or read the snapshot, no tokens are stored, and startup restore remains the existing `restoreSession` setting (on by default, now labelled “Restore open windows and tabs when the app starts”). Limits: native tab-group *membership* is not persisted (restored windows open ungrouped with the tab bar visible and can be merged again), the new native Window menu is English, and Windows/Linux are unchanged | macOS | Medium | None |
| 11 | **OS print dialog** — desktop Print uses the admitted native export path. macOS presents the validated PDF through PDFKit; Linux opens GTK's native printer dialog, and Windows has Win32 selection plus WebView2 printing. Current Linux real Ctrl+P, Print to File selection, Cancel, reader navigation and a new article's print-dialog reopening/cancellation passed in the owned Xvfb session. This corrects the older claim that Windows/Linux providers were not wired; it does not establish physical job delivery or new PDF output. See [RELIABILITY](RELIABILITY.md). | macOS, Windows, Linux | Small–medium | Windows native runtime and physical printer delivery |
| 12 | ~~**System share sheet** for an article/finished file~~ **done** — article PDF save/share on iOS, macOS and Android; finished archive/file on all three. Android uses `NativeArticlePdf` (public-SDK WebView capture drawn into `PdfDocument`, shared through the existing `FileProvider`/chooser path); print jobs use the spool adapter. A simple-fixture Android PDF recipient rendered Main without native scrollbars and typed cancellation prevented a write/chooser/handoff. A physical Samsung SM-A356U share to Google Drive passed (2-page PDF preview); StreetZim’s WebGL map canvas was blank in that PDF while UI chrome was present. Complex/WebGL fidelity and universal recipient interop remain open | iOS, macOS, Android | Small–medium | Physical/recipient breadth |
| 13 | ~~**Archive element fullscreen** (video)~~ **done** — delegated to the trusted archive origin only; policy verified in a WK harness, real in-app enter/exit still unverified, and WebKitGTK likely no-ops | All | Small | Linux verification |
| 14 | ~~**Menu/accelerator completeness**~~ **done** — home, settings, random, TOC, reading list, library search and search scope use the upstream Kiwix bindings; verified in a release build through AppKit's key-equivalent path. The audit also exposed a muda/Cocoa mismatch where Shift+letter items shadowed the plain letter (Cmd+R, Cmd+T); a one-shot AppKit pass now rewrites those equivalents to Cocoa's uppercase form | Desktop | Small | None |
| 15 | ~~**`zim://` URL scheme**~~ **done** — iOS and macOS register `zim` and route `zim://<archive-uuid>/<article>` through the existing launch-location queue: the parser accepts only a canonicalizable ZIM UUID host, rejects credentials, ports, remote hosts, malformed escapes, control characters and raw/percent-encoded traversal, and never imports or fetches anything; the link must resolve to an available library archive. Apple's URL layer normalizes literal dot segments before Rust receives the URL; the normalized remainder is still used only as an archive-relative libzim key, so it cannot escape the archive. Open-in-place document providers remain architectural: they conflict with the retained-descriptor/copy design | iOS, macOS | Medium | Open-in-place conflicts with the retained-descriptor/copy design |
| 16 | **Video PiP and background audio** — `.playback` is installed before media starts, while WebKit owns archive audio activation/release. Explicit app activation is reserved for native read-aloud. Media reports retain reader resources during background playback; interruption notifications clear shell bookkeeping without a duplicate document pause. PiP was exercised on the iPad simulator. Physical iPhone foreground unmuted TED WebM playback was confirmed working after removal of app activation/reassertion; **lock-screen/background audio, PiP audio and headset/interruption transitions remain unverified**. Earlier offline-device/signing blockers no longer describe the current installation | iOS | Small–medium | Physical lock-screen/background and route/interruption acceptance |
| 17 | ~~**Android platform entry points**~~ **done** — `PROCESS_TEXT`, dynamic New tab / Get content shortcuts, `kiwix://` links, a home-screen search widget, pinned article shortcuts and voice search. The widget reads only the last shell-published archive summary (the launcher redraw never starts the WebView), clears archive titles in private/read-only mode, and its search/library/get-content buttons reuse the existing native entry queue, so a cold start lands in the library search. Pinning uses `ShortcutManager.requestPinShortcut` with a `zim://<archive-uuid>/<path>` link; Rust validates and canonicalizes it through the same `zim_scheme` parser as external links, and the shell lands on the library with an actionable notice when that archive was removed. Voice search launches an installed `ACTION_RECOGNIZE_SPEECH` activity; there is no `RECORD_AUDIO` permission, no `SpeechRecognizer` and no in-app microphone session, and the action is hidden when package visibility finds no handler | Android | Small | Launcher widget/pin dialogs and recognizer availability are launcher/device-dependent; physical-device acceptance remains separate |
| 18 | **Small settings/UX misses** — ~~clear-all bookmarks/notes~~ **done** (confirmed bulk removal through the existing per-bookmark command and privacy generation), ~~back-to-top control~~ **done** (reader status bar), ~~“open new tab in background” setting~~ **done** (persisted `openNewTabInBackground`, honoured by reader link actions), ~~first-run intro~~ **done** (the empty Library offers Open file vs Browse catalog and disappears once the library has an archive; no modal or dismissal nag), ~~storage used/free summary~~ **done** (Settings reports library bytes/archive count and, when the platform exposes it, free/total volume space; unavailable values are stated, not shown as zero), ~~About dependency inventory~~ **done** (app, Tauri, Rust ZIM and Glass search reader versions, spelling index format and licence links). **Rate prompt** remains (deliberately not added) | All | Small each | None |
| 19 | ~~**Crash capture and report** with archive validation~~ **done** — a Rust panic hook plus native Android/iOS uncaught-exception intake writes one bounded, redacted record in the app-private profile (URL queries/fragments, reader tokens, `zim://` locations, filesystem paths and secret-looking values removed; 32 KiB cap). The next launch offers View / Copy / Delete and Send only through the native `mailto:` compose intent — never network telemetry. Reader-phase records include the archive’s last libzim/retained-byte validation result, and the existing integrity panel remains the validation action. macOS exercised a provoked panic, restart prompt, redacted content, clipboard copy, UI delete and valid/corrupt fixture results; native mobile execution remains unverified | Android, desktop | Small–medium | Native mobile run not exercised here |
| 20 | ~~**Signed update channel**~~ **implemented; distribution credentials and a real restart remain unverified** — desktop builds use `tauri-plugin-updater` with a compiled public key and HTTPS endpoint (`plugins.updater`), `requireSignedVersion`, an explicit confirmation, download progress and restart. A daily background check never downloads; private/read-only, active transfers, archive moves and concurrent operations are refused with plain explanations. `scripts/make-update-manifest.mjs` packages/signs artifacts and writes the static manifest; the loopback proof checks, downloads, installs into a temporary `.app`, rejects a tampered artifact and persists the last-check time. iOS/Android say plainly that they update through the App Store/Play and offer no dead button. A real Developer ID/notarized macOS update, signed Windows installer, production key/endpoint and the final process restart still require the distributor (see `docs/UPDATE.md`) | Desktop | Large | Distribution keys, notarization, hosted feed |
| 21 | ~~**Reader controls during vertical panning**~~ **done** — on a compact phone layout, a genuine one-finger downward reading pan that also scrolls the document hides the reader toolbar, the shell navigation/status bars and the workspace bar; they become inert and the reading area grows (736 → 844 CSS px at 390×844). An upward pan, reaching the top, an explicit 44 px reveal button, any key press, focus change, dialog/panel, navigation, tab change, suspension or viewport resize restores them. Horizontal map drags, pinch zoom, text selection, editing and programmatic scrolling never hide. Pan reports carry the bridge's own one-per-document token, sequence and current URL; forged messages and programmatic scrolls did not hide controls in the actual browser run. Desktop layouts are unchanged. Measured on Chromium at 390×844 with the real Wikipedia and Hawaii archives; **not** exercised on a physical iPhone or in WebKit | Reader UI | Done | WebKit/physical-phone acceptance |

## Explicitly not pursued (architecture, not omission)

- **Wi-Fi Direct (Android P2P)**: our LocalSwarm over LAN/hotspot covers the need; adding a second P2P transport is not planned.
- **Branded/custom app builds**: single-product distribution has no analogue.
- **`zim://` as an internal content origin**: archive pages are still served from a per-archive loopback HTTP origin with installation-scoped storage. The `zim://` scheme is now only an external intake format that resolves to a library archive and enqueues a normal launch location; it never loads remote content.
- **aria2 segmentation / `.aria2` control-file interop**: different engine, equivalent capability.

## Remaining acceptance

Complete the remaining physical iPhone/Android, native macOS window-group,
Windows/Linux printer and real-provider scenarios. Current generated metadata
has 1,446 sources across 74 locales: English alone is string-complete and all
73 non-English catalogs are partial. Competent exact-language translation and
independent review remain required; coverage is not linguistic acceptance.
Production credentials and signed clean-machine update/restart also remain open.

## Evidence

Per-platform matrices with file-level references are retained in the audit receipts: `agent://ParityDesktop`, `agent://ParityApple`, `agent://ParityAndroid`. Cross-checks: `docs/FEATURES.md`, `docs/ISSUES.md`, `docs/PACKAGING.md`.
