# Feature coverage and evidence boundaries

**Implementation evidence, not a verified feature superset.** “Source-backed” means a reachable implementation was inspected; it does not mean a fixture, build, platform, hardware, accessibility, or release check passed. The predecessor audits are [Qt desktop](audit/desktop-source-audit.md) and [Apple](audit/apple-source-audit.md); [ISSUES](ISSUES.md) links the complete historical audits of both repositories and maps the original open-issue snapshot to code and remaining acceptance. Those reports describe their respective source/evidence snapshots, not blanket certification of this implementation.

The [upstream comparison](PLATFORM_COMPARISON.md) records optimized-release sizes
against existing Apple/Android/Qt apps, scoped matched Android runtime data and
source-backed feature differences. It is not universal native acceptance.

## Latest incoming-file correction — 2026-11-04

- Mobile incoming files name the actual file and offer **Add to library** and
  **Cancel**, with explicit copy/in-place consequences. Secondary storage
  choices reflect current native authority; temporary handoffs can instead
  reselect originals in the system picker. Private/read-only admission remains
  session-only. The UI hint cannot grant persistent access.
- Real-component browser interactions and 17 native iOS document tests passed;
  strict Android/iOS production Clippy passed. The reviewed in-place iPhone
  update retained captured existing book records, reader tabs, bookmarks and
  preferences. Automated warm handoff and Add admitted the real Wikipedia
  archive and created its declared-size managed file. No archive-byte
  readback hash or matched iPhone memory/startup/search result is claimed.
- The first cold developer handoff did not expose the dialog. A warm handoff
  did; the subsequent automation failed because it expected **Wikipedia**
  rather than the actual archive title **Wikipedia 100**. That selector
  failure does not invalidate the observed successful admission.

## Latest efficiency and deployment evidence — 2026-11-04

- The final ARM64 APK is 66.98 MB, below the pinned official Android ARM64 APK,
  without removing reader features or locales. Both optimized phone updates
  were installed in place with scoped preservation and normal-launch evidence.
- HTML response construction now shares its existing article buffer and avoids
  an allocated lowercase head scan. Real Wikipedia/StreetZim allocation and
  construction measurements improved; actual warm debug HTTP delivery was
  faster. These are not lower whole-app RAM, shipping startup/search or complete
  physical-provider/accessibility acceptance claims.
- Current checks passed 946 Rust tests/16 ignored and strict host/Android/iOS
  production Clippy. The frontend is unchanged from the separately verified
  211-test/platform-build batch. See the [current comparison](PLATFORM_COMPARISON.md)
  and [response-path receipt](benchmarks/2026-11-04-response-path.json).

## Earlier release comparison and replay correction — 2026-11-04

- ARM64 release symbol stripping reduces our APK to 83.81 MB versus upstream
  Android's 70.37 MB; Mac remains 97.93 MB versus Apple's 19.35 MB expanded app.
  These remain improvement areas, not permission to remove functionality.
- Wombat's original-URL getter no longer sends a real archived link to external
  confirmation. Actual Chrome/native Android offline destination and Back passed;
  genuine external links still require consent. Other native acceptance is not
  inferred from these scenarios.
- Updated checks passed 211 JavaScript tests and TypeScript checking; optimized
  Mac/Android and unsigned iOS device archives built. Android exact-APK reading,
  search screenshots and repeated saved-reader launches were inspected.

## Latest acceptance — 2026-11-03

- Optional mobile speech discovery no longer reports a readable article as
  failed. A fresh Android native first-open kept the article free of a Reload
  band; Read aloud showed its unavailable reason with Start disabled, and normal
  article return/restart passed. No playback or voice installation was requested.
  The shared source repair has this Android-emulator proof, not new physical
  iPhone/audio acceptance.
- Offline article previews now include bounded same-archive raster artwork.
  Real libzim redirect/base/encoded-path regressions and actual browser image
  decoding/text fallback passed, including a 390px viewport. Native mobile
  thumbnail interaction remains separate from source/build/install evidence.
- The iOS bottom-gap correction paints through the home-indicator region.
  Native simulator portrait/dark/light and trusted Settings keyboard checks
  passed. The signed iPhone update preserved the three books, two tabs, settings,
  transfer files and archive metadata before launch. After owner unlock, normal
  launch and an inspected physical portrait-reader screenshot confirmed no light
  bottom strip. Post-launch title/migration-source differences prevent an exact
  State-preservation claim. Library and landscape-notch acceptance remain separate.
- Actual owned Mac/headless resource context saves retained video/audio/image
  names and exact source bytes; separate native API publication also preserved PDF
  bytes. Persisted preview opt-out/restart/re-enable and First→Tab→Second→Escape
  passed before the artwork extension; native-picker/mobile acceptance is separate.
  Export post-publication durability errors now retain completed files and avoid
  misleading success. See [RELIABILITY](RELIABILITY.md).
- Actual Samsung→ARM64 Linux and Mac→ARM64 Linux Nearby flows passed genuine
  LAN discovery, selected single-ZIM QUIC acquisition, source SHA-256, library
  import and main-page HTML serving on an owner-approved bridged guest.
  Phone books/tabs/reading-data/settings comparisons and source-byte preservation
  passed; publishers stopped and the temporary VM was deleted. Retained Linux
  build scope, the original Mac sender's SIGTERM exit, and later separate
  lifecycle check are recorded in [RELIABILITY](RELIABILITY.md). Reverse/bulk,
  rendered Linux UI and Raspberry Pi acceptance do not follow.
- A fresh authorized Samsung→iPhone transfer received all 150,605,015 Hawaii
  bytes with the exact source SHA-256 in 5.186723 seconds, approximately
  **29.04 decimal MB/s effective admission-to-completion throughput**. Owned
  cleanup preserved the named recorded library/reading-data/settings fields.
  This is not an active-payload rate, historical-cause diagnosis or routerless proof.
- Actual host HTTPS catalog/full-archive verification, explicit replacement,
  new-article reading and verified rollback passed. Original/replacement files
  were retained; fresh installation principals, bookmark/live-tab/zoom migration
  and an unsaved Unicode note/title/tags survived both cutovers.
- Private reading-data sync passed real two-process security/causal lifecycle
  acceptance and two-browser mutual enrollment, manual/automatic note transfer,
  concurrent versions, cancelled resolution and explicit resolution convergence.
  Public archive sharing remains separate; missing archives acquired no authority.
- Rebuilt owned virtual devices exercised Android TalkBack map/detail navigation,
  native invitation creation and HTTPS catalog traversal, plus iOS internal links
  and mutually confirmed private-note delivery from a desktop peer. No missing
  archive was installed by note sync. The current iPhone build is installed:
  prelaunch preservation passed; normal launch and the physical portrait-reader
  bottom edge subsequently passed. This is not broad post-launch acceptance;
  the physical Android package remains older.
- The matched-SDK host suite now passed **943 Rust tests, 16 ignored**; production
  frontend and **211 JavaScript tests** passed. Strict all-target Clippy with
  warnings-as-errors remains failing on the existing diagnostic backlog. Owned headless-browser runtime
  covers live-document SPA/TOC history and a synthetic watchdog fallback; a real
  directory-sync fault probe confirms already-latched permanent-delete refusal.
  Broader mobile/assistive-technology, physical provider-intake and competent
  language review remain separate gates. See [RELIABILITY](RELIABILITY.md).

## Earlier acceptance — 2026-11-02

Use [RELIABILITY](RELIABILITY.md) for the newest exercised results; the dated
repair sections and platform snapshot table below retain older evidence rather
than certifying every subsequent change.

- The reviewed iOS build now uses `com.tiltastech.zimreader`, with a real
  device-authorized profile and signed Hotspot Configuration/Access Wi-Fi
  Information entitlements. Main/widget signatures passed independent review;
  physical installation, launch and one native launch smoke passed. This was
  a fresh installation after the user removed the old app, not migration.
- Nearby now uses one public encrypted-QUIC library workflow: **Share all** first,
  then choose the sender and download all or selected archives. Mandatory
  invitation/PIN ceremony is removed. Browser/OPDS HTTP is separately enabled
  in Settings; selected-file QUIC remains a compatibility option.
- Default mobile Add files/Add folder references originals with retained
  provider access. Managed copying is explicit; temporary incoming handoffs
  require original reselection or an explicitly requested copy.
- Source reviews closed the reported intake and transport findings. Final host
  concurrent selected downloads, exact hashes, selected-only imports, reader
  Main/internal detail and Download-all skip/add passed after the native fix;
  earlier bulk/native failures are retained as pre-fix evidence in RELIABILITY.
  Final checks: 835 Rust passes/16 ignored, 149 JavaScript passes and 3 native
  C++ passes; both mobile builds and strict iOS signatures passed.
- Those reviewed packages were installed in place and launched, retaining Samsung's
  eleven books/three tabs/settings and iPhone's three books/one tab/bookmarks/history.
  Unlocked iPhone Nearby surface XCTest passed (1 test, 0 failures), not phone
  connection/transfer. The later ULA metadata/on-link correction passed 837 Rust
  tests/16 ignored, both mobile builds and strict iOS signatures, then actual
  Samsung Share all/advertising, inventory browsing and verified Samsung-to-Mac
  transfer. Both corrected packages are installed; new iPhone launch/transfer was
  lock/authentication-canceled. Phone-pair throughput, original-file intake and
  OS prompts remain open.
- The later transport-status correction passed 839 Rust tests/16 ignored and
  154 JavaScript tests. Real QUIC admission, UI Pause/Resume, verified completion,
  import and persisted provenance passed; both final packages were installed
  with preservation. Historical synthetic library rows report unknown rather
  than invented transport. This does not close the phone-pair gates above.
- Preceding Download-all batch: 147 JavaScript tests passed; production/iOS/Android
  frontends, desktop executable and arm64 mobile packages built. Both phone
  packages were installed in place. Samsung cold launch and actual three-archive
  bulk transfer/import/hash/cleanup passed; the iPhone install preserved its
  complete persisted state, but first launch was refused because it was locked.
  The preceding 822-Rust-test run and real DNS-SD regression remain separate
  evidence; no Rust or native Swift/Kotlin logic changed in this batch.
- Earlier reviewed builds exercised physical iPhone official background
  completion after process loss/OS relaunch, exact hash and reading; inline-video
  paused-position recovery and native PiP; Android download/process recovery and
  real SD whole/split archive and removal/recovery paths. The earlier native
  suites passed 26 iOS and 27 Android tests, not rerun for the naming-only batch.
- Nearby publisher naming is implemented and host-runtime exercised, including
  equal-name distinction, accessible action labels, future-share-only changes,
  settings failure/retry, navigation and concurrent typing. Names/codes are not
  authentication; Apple QUIC system-host DNS-SD can still expose an OS hostname.
- Nearby **Browse → Download all** now queues missing archives with truthful
  installed/active/missing-file counts, partial failures and Stop adding.
  Host and physical Samsung acceptance are recorded in RELIABILITY.
- Physical Bluetooth/automatic-join/selected-route acceptance,
  assistive-technology acceptance, locale completion, cross-device personal-data
  sync and incremental catalog/edition workflow remain open.
  Complete predecessor migration is deferred backlog by user priority.
  Lock-screen audibility/headset checks are user-deferred.
- The Linux ARM64 native evidence below is from an earlier snapshot; latest
  changes still need target-runtime acceptance. Windows still lacks native
  runtime acceptance and requires the trusted SDK/runner. No blanket parity or
  release certification follows from source coverage or build success.


## Integrated repair wave — 2026-11-01

The [48-hour review](audit/REVIEW-2026-11-01-48H.md) now has integrated repairs
for all 26 findings. Current verification: 795 host Rust tests passed, nine
ignored; 107 JavaScript tests passed; frontend build passed; 22 native iOS
simulator XCTest regressions passed; arm64 iOS IPA and Android debug APK built.
Actual corrected iOS simulator acceptance exercised launch, picker import,
read/Back, background/return, cold restart and native speech controls after the
non-retained `UIApplication.delegate` reassignment crash was repaired with a
class-level pre-`UIApplicationMain` handoff. Evidence:
`local://AcceptIOS-acceptance.md` and `/tmp/kiwix-accept-ios-evidence/`.
This proves exercised behavior, not infallible startup or hardware audio.

Finite checkpoints bind exact ID/generation and a 1,800 ms deadline. Terminal
`KIWIX_SUSPENDED` (`retry:false`) is cancellation without enqueue/replay or
stale banners, not suppression of genuine persistence failures. Reader
navigation waits for cancellable actual storage admission. Live-page PDF
identity allows equivalent percent-dot spelling, retains descriptor ownership,
and Android presents a visible cancellable preview.
Android actual picker/admission/read/lifecycle, PDF recipient and typed preview
cancellation checks passed; these are exercised scenarios, not blanket certification.
Latest iOS simulator install/launch and maximum-Dynamic-Type navigation passed
on artifact `22088`: native taps/horizontal swipes reached all seven destinations
without error bands (evidence in RELIABILITY). This does not certify physical
hardware or assistive technology. These simulator checks preceded the physical
deployments recorded below. Earlier platform rows remain historical evidence,
not acceptance of later changes.
Actual iOS simple-fixture PDF sharing rendered Main page/H1 Main in Apple Preview
(`/tmp/kiwix-final-ios-pdf-recipient.png`), not complex-page/cancellation proof.
After the final Android root-producer repair, focused `live_pdf_` tests passed
2/2, then the full host suite passed 771, nine ignored (artifact `22172`).
Latest iOS arm64 device build/IPA export passed (artifact `22173`), without
physical installation. Android recipient `22209` rendered Main without native
scrollbars; final APK `22227` typed cancellation prevented write/chooser/handoff
without error bands and left Main readable. Evidence: RELIABILITY and
`/tmp/kiwix-android-finalqa-5586/`. Final desktop asset build passed (`22230`).
Complex-page fidelity, physical hardware and assistive technology remain limits.

Current implementations supersede earlier gaps: official-catalog iOS
background URLSession, identity-checked stale Apple bookmarks, combined
title/full-text search, Dynamic Type shell scaling, native macOS group restore,
live-page PDF, Android selected-file copy/move and external entrypoints,
forward/timed speech, per-app/widget locale, Android PDF sharing, LAN search
and logical split/provider acquisition, and native Windows/Linux printing.
Details and fidelity limits: [PARITY](PARITY.md). These implementations do not
establish physical device/provider/printer or accessibility certification.

## BitTorrent usability and library sharing — 2026-11-01

Catalog cards expose a separate BitTorrent action while normal Download remains
HTTP. Official entries resolve the publisher's magnet lazily through a bounded,
provenance-checked endpoint; arbitrary OPDS entries require explicit torrent
metadata. Downloads exposes URL/magnet entry at the top, and torrent upload
statistics and opt-in seeding controls outside Details. Actual official
one-click acquisition completed, imported and opened a 317,149-byte ZIM.
Controlled real-engine magnet/`.torrent`, pause/resume and seeding checks passed.
Catalog grid/list bounds passed at 320–1365 px; screenshots were unavailable.
Post-change Android APK/iOS IPA builds passed (`22344`); both were subsequently
installed in place. Android process launch passed; iOS launch was blocked by
the physical phone's normal Locked policy. That installation preceded Share all.

Nearby now exposes **Share all**: one explicit click starts an open whole-library
session with automatic LAN address/port selection (or the Android app-owned AP
address for temporary Wi-Fi) and discovery. Optional selected files, PIN and
advanced bindings remain separate; explicit PIN/network choices are honored.
Recipients browse a discovered library and download through scoped native
acquisition, not a global LAN HTTP exception. Browser indexes have Download
links; Stop revokes the advertised session and active responses.
Actual two-instance open/protected/selected inventory, download/import/read,
wrong PIN, browser links and Stop/withdrawal passed. Existing QUIC/AWDL transfers
remain separate; no local BitTorrent cutover or measured speed claim is made.
The normal per-archive path is now **Details → Share** (QUIC), then the recipient
opens **Nearby → Download → Open**. Opening Nearby starts receive-only
discovery; sharing still requires **Share all**. URL/IP/QR and Apple route-order
controls are collapsed. Finding does not offer archives or start downloads.
Actual two-instance acquisition/read/hash/restart and native Android
radio-off/re-entry/Stop/privacy/Share checks passed. Reference BLE bootstrap is
source-reviewed but not integrated; no automatic fastest-radio claim is made.
Active offers and incoming archives now precede setup; native 320 px acceptance
showed the selected archive and Stop sharing on the first screen without overflow.
The final simplified Android APK installed in place and cold-launched in 1,037 ms.
Earlier Share all packages installed/launched on both phones. The newer simplified
iOS package initially hit CoreDevice4016, then installed in place and launched
after reconnection (`bg_66`). These checks are not physical phone-pair acceptance.
Physical pairing, AP-only routes, iOS foreground limits and release acceptance
remain explicit boundaries. Details: [LOCALSWARM](LOCALSWARM.md),
[PARITY](PARITY.md) and [RELIABILITY](RELIABILITY.md).

## Platform status and remaining work

Development snapshot; completed source work is not release certification. Detailed local receipts referenced below remain excluded from the private source repository.

| Platform | Latest exercised state | Remaining acceptance |
| --- | --- | --- |
| macOS Apple Silicon | Merged native-look build `f3c5cb14…` exercised: overlay title bar with traffic lights at (16,13), skip link clear of them, 760px RTL drag region, 13px skin, native and reader full-screen round trips, Light/Dark/System window theme, real Bahrain 49,236,471-byte pause/Quit/Resume on the same inode, retained-predecessor cleanup across restart, and `NSFileManager` Trash (17 ms, visible 0700 container, move-out recovery). | Physical window dragging (local synthetic drags cannot move the window), global non-blue system accent, camera/location, notifications, Developer ID/notarization and clean-machine checks. |
| Android | Merged production Release `e61e4866…` (cert `b2a7c3e8…`, not debuggable) installed in place on Samsung SM-A356U; header still 0 books/0 tabs, both failed LocalSwarm jobs and settings unchanged, dark Material 3 skin live. Native 34-event touch swipe revealed the row action and Escape closed it; the 320px Downloads map header now reads on one line. | Final `d53ba1a` emulator runs additionally passed the Downloads-row swipe reveal, its native Escape dismissal, and the cleanup dialog opened then cancelled with no destructive choice; the retained-files list was genuinely empty. Post-install private `.part` hashes are unavailable on the non-debuggable release, and the broader zoom/RTL/rotation matrix was not re-run on this artifact. Phone-to-phone sharing and physical camera/location remain. |
| iOS | Final production build `d53ba1a` (`93563e65…`, team A6G8H8NGAM, deep-strict signature) installed in place on the iPhone 17 Pro Max test device and launched: 5 books/13 tabs, theme and locale preserved, Maps shows the completed Baltics card with 78/78 blocks. An unlocked physical UI run reached all six surfaces, dismissed a sheet by Close and backdrop, revealed a swipe action on tap without opening the article, and opened then cancelled the unlink and download-cleanup dialogs. | Physical hardware-keyboard focus ring, physical sensors/radio, and phone-to-phone LocalSwarm throughput. |
| Linux ARM64 | Final build `928716db…` from `d53ba1a`: real Bahrain pause/Quit/Resume on the same inode, replacement, retained-predecessor deletion, cross-window stale-delete refusal, three-window normal Quit, secondary windows loading the real app (vendored runtime protocol-registration fix), libadwaita skin, inset focus ring, RTL and history non-resurrection. | Decorated title-bar pixels under a real window manager (Xvfb had none), Windows and optimized-release packaging. |
| Windows | Native MSVC build and installed NSIS runtime acceptance on Windows 11 24H2 (build 26100, x64; MSVC 19.44, Windows SDK 10.0.26100, WebView2 157). The Windows agent rebuilt libzim 9.8.2 from the patched source, passed **1009 lib tests** on `x86_64-pc-windows-msvc`, and exercised launch/single-instance, 12-file import, reader/TOC/find, title/full-text search, H.264/VP8/VP9 playback, HTTPS download, BitTorrent URL/catalog, HTTP map composition/read, storage, updater, uninstall/reinstall, delete-while-open, web sharing/OPDS, Android Nearby receive, iPhone download from Windows, and the native Print dialog. Confined publish/move/delete failures (os error 87/32/5/33), verbatim path display, PDF scratch cleanup and print-cancel runtime abort were fixed and runtime-verified. | [Windows report](WINDOWS-TEST-REPORT.md) residuals: PDF Save As dialog automation, map torrent web-seed support, clean-machine dependency closure, FAT/ReFS identity semantics, some F17 path-race notes, and remote-device sharing after firewall policy. No Windows 7 or MSIX claim. |

Release gates beyond those platform rows:

- Accept physical suspended/relaunch behavior for the approved official-only iOS
  background path; custom URLs remain strict foreground downloads.
- Complete the 15 partial locales: `dag`, `gsw`, `ie`, `igl`, `io`, `kab`,
  `nqo`, `sc`, `sdc`, `skr-Arab`, `sms`, `sro`, `zgh`, `zza`, `roa-Tara`.
  Only 59/74 catalogs are string-complete; all still require native-speaker review.
- Finish platform signing/distribution, clean-machine installation and accessibility coverage. Zimit2 archives are served through the shared reader path; real Zimit fixtures on packaged WKWebView/WebView2/WebKitGTK remain a release gate. Zimit1 service-worker replay remains unsupported. Raw `.warc` files are not a library format; `.warc` entries inside a ZIM can be saved with the reader's Save original file action.
- Historical issue/PR censuses are complete, but Windows, macOS notifications and iOS background transfer gates prevent blanket historical-fix certification.

The merged native-look, map-resume/replacement and swipe-cleanup batch passes the integrated frontend build and **477 host Rust tests** (three ignored). Production builds from `d53ba1a` are installed on both phones (iOS `93563e65…`, Android `e61e4866…`), and macOS (`f3c5cb14…`) and Linux ARM64 (`928716db…`) ran native acceptance on the same source. These platform runs do not establish physical phone-to-phone transfer speed, physical hardware-keyboard focus, provable macOS title-bar dragging or Windows behaviour. Private GitHub Actions run 41088865723 did not start its macOS/Linux jobs because of the account Actions budget; no source compilation failure was observed there.

The following results describe the earlier, pre-swipe/map-resume batch:

Integrated checking passed **403 Rust tests** (two ignored), eight production sensor-bridge regressions and TypeScript; the mobile-admission regression additionally executed on actual Android and iOS-simulator targets. Native PDF output has macOS, Linux ARM64, Android emulator and iOS-simulator evidence. Real direct map/Nearby downloads completed, imported and opened; Apple transport ownership passed fail-before/pass-after and four concurrent real receivers. The mobile cold-bootstrap checkpoint now fences stale initial snapshot callbacks and never persists initial empty defaults; hydrated checkpoints and explicit Quit retain their original durability checks. See the [changelog](../CHANGELOG.md#verification-and-known-limits) for build-scoped qualifications.

Producer metadata tagged `_sw:yes` receives an incompatibility notice in the reader as well as Catalog/Info; legacy replay is not implemented. Compiled-browser direct-open/restored-tab fixtures displayed that notice, while `_sw:no` content read without it at 390px without horizontal overflow. Incoming/provider paths share the same Reader surface but do not gain separate physical-device acceptance from that smoke.

Current mobile correction evidence: `.native/mobile-priority-ui-20261030/runtime-evidence.json` and `.native/mobile-priority-20261030/ios-priority-ui.xcresult`. California English occupies **3.10 GB** on device; **106.2 MB** is conditional incremental reuse data, not its complete size. The selected dialog calculates bounded, recipe-based transfer estimates without fetching map payload and keeps technical reuse fields collapsed. In-place iOS simulator update preserved all eight archive IDs and 29 tab IDs; cellular consent saved before immediate navigation and survived relaunch, then was restored off. Separately, `ios-priority-location.xcresult` exercised app consent, Apple's normal Allow While Using App dialog and fresh simulated CoreLocation updates delivered to the actual article as `Location 37.4`, without an article-reload error. Normal UI cleanup restored the original eight archive IDs, 29 tab IDs and active tab; this does not establish physical-phone GPS or cellular acceptance.

Consumer UI evidence: `.native/ui-density-20261030` and `.native/ios-consumer-ui-20261030/acceptance-result.json`. Library/Catalog use adaptive cards; imports and filters are secondary with visible active-filter recovery. Nearby has its own Send/Receive tab. Recently read is a Library action, not a primary tab; repeated article visits share one row with the latest complete location. Downloads has no redundant discovery CTA or completed progress bar; its verified Open action is 80px high. Independent source review closed all ten findings in `.native/ui-density-20261030/final-adversarial-review.md`.


## Library, discovery, acquisition

| Major behavior | Current source/reference | Limit / acceptance still needed |
| --- | --- | --- |
| Open local archives and split first parts, multiple CLI/file-open events, import without copying | `src-tauri/src/lib.rs::open_paths`, `service.rs::import_paths`, `archive.rs`; `src/App.tsx` import/drag-drop; `provider_reference.rs`, native `ExternalDocuments` | Desktop cold/warm association and foreground behavior on each packaged OS; Unicode paths, `.zimaa`, mounted NAS. No direct `smb://` streaming promise. Both Android and iOS implement retained seekable provider descriptors plus explicit managed copies; stream-only providers need copying. Native Android provider search/checksum/restart and managed-copy reading passed; physical iOS USB/cloud-provider retention/relaunch/revocation and arbitrary provider compatibility remain unverified. |
| Android device-wide ZIM discovery and in-place opening | `src-tauri/src/device_scan.rs`, native `DeviceZimScan` and `ExternalDocuments`, `mobile_service.rs`, `Library.tsx` | MediaStore rows, a bounded primary-shared-storage walk on Android 12L and earlier, and every persisted SAF tree grant, deduplicated by volume + canonical relative path + size/mtime and grouped split sets; opening reuses the existing retained-descriptor provider path with no copy. Private mode refuses scanning and folder grants; read-only keeps references session-only. Emulator acceptance covers Android 11 MediaStore + SAF dedup and newer SAF-only releases; physical removable storage and Android 13+ MediaStore visibility remain unverified. |
| Removable/app-specific library location with verified migration | `src-tauri/src/library_root.rs`, native `StorageLocations.kt`, `mobile_service.rs`, `Settings.tsx` | Android Settings lists internal app storage and each `getExternalFilesDirs` app-specific external directory with free/total space and the active root. Selecting another root copies every managed archive to private staging on that volume, verifies the staged archive through the existing libzim integrity gate, publishes without replacing names, and commits all book paths plus `downloadDirectory` atomically; old copies are kept by default and removed only on request after verification. Interrupted copy/publish rolls back while the old root is active; interrupted cleanup finishes when the Store already points at the new root. Free-space and private/read-only refusals are explicit, and the switch is refused while a queued/downloading/verifying/paused transfer or active map job still targets the old root. Archives are still served only from app-owned paths: the removable choice is `Android/data/<package>/files/archives`, never a raw SD-card path. Emulator acceptance covered selection, copy, reopen and summary; a physical SD card, card removal/reinsertion and a real full-volume case remain unverified. |
| Library metadata, grid/list, persisted multiselect facets and all offered metadata sorts | `Library.tsx`, shared filters/sorts and `EditionBadges` in `UI.tsx` | Language/category/content selections, availability, ascending/descending title/size/publication date/language/availability/creator/publisher/article count/media count/content/description, plus local added-date sorting. Publication date is distinct from import date; flavor/content badges depend on archive metadata. Mixed editions/languages and slow-NAS responsiveness need acceptance. |
| Folder import and monitored directories | `service.rs::scan`, `service.rs::apply_watch`, `watch.rs`, `lib.rs::maintenance`, Settings | Desktop explicit recursive scan; monitored directories are now watched live (`notify`, ~1 s stability window, direct children, no symlinks) and import through the same scan path. Removing a file retires its library entry without deleting anything, unless the book is open in a tab. Private/read-only modes pause watching. Mobile imports folder copies rather than monitoring provider originals. Verified on macOS: add 1.15 s, delete 1.27 s, slow 4-chunk copy imported only after completion, claimed downloader destination not adopted; network shares that emit no events still rely on the periodic scan, and very large directories fall back to it. |
| Missing books retained/relinked, reveal containing folder, unlink versus trash/permanent deletion | `Library.tsx`, `service.rs`, `library_files.rs` | Relink requires matching UUID. Deletion verifies the exact checksum-bounded `.zim`/split-part set, UUID/integrity and file/directory identities; private staging avoids prefix-glob deletion of unrelated neighbors. Partial failures report retained recovery paths. Real OS Trash, replaced-file and split-set failure acceptance remains needed; integrity is not authenticity or immediate libzim cancellation. |
| Online OPDS discovery, conditional offline cache, server-side query/facets, pagination, thumbnails and online preview | `Catalog.tsx`, `catalog.rs`, `catalog_cache.rs`, `UI.tsx` | Actual desktop HTTPS 200 → 304 → network failure exercised page/facet reuse, distinct locale/query identities, visible offline warnings, read-only nonmutation and private nonpersistence/no normal-history fallback. Producer metadata is not automatically translated. Raster thumbnails use a bounded native proxy/cache; confirmed online preview opens an external viewer. Sorting applies to the fetched page, not the entire remote catalog. Custom feeds and native mobile cache acceptance remain separate. |
| Incremental catalog reconciliation and verified archive-edition cutover | `catalog_index.rs`, `edition_updates.rs`, `edition_service.rs`, `AdvancedSettings.tsx`, `edition-update-queue.ts`, `Downloads.tsx`, `reference-transitions.ts` | On-demand bounded traversal of all configured catalogs preserves source/query identity and opaque Atom IDs, revalidates conditional metadata pages and reports partial/stale results without treating omission as deletion. Unbound lineage requires explicit association. A full separate archive is verified before explicit Replace; native transactions migrate reading references, issue a fresh installation principal and retain originals for verified rollback. **Update all** queues only bound, single-candidate editions with bounded concurrency, reports per-archive failures and cancellation, and still requires an explicit Replace for each verified output; ambiguous lineage is reviewed individually. Web storage, sensor and sharing consent are not migrated. Actual official Hawaii HTTPS acquisition, replacement/read/rollback and unsaved-draft preservation passed on the host; mobile and broader failure-matrix acceptance remain separate. |
| Application update channel | `service.rs` `check_updates`, `AdvancedSettings.tsx`, `tauri-plugin-updater`, `scripts/stage-update-release.mjs` | The checked-in build performs a manual upstream **Qt** release check and has empty updater `endpoints`/`pubkey` in `tauri.conf.json`; release staging can inject a signed endpoint/key. No shipped signed update, production credentials or real restart has been verified. |
| iOS download Live Activity / Dynamic Island | `src-tauri/src/live_activity.rs`, `plugins/tauri-plugin-kiwix-mobile/ios/Sources/LiveActivity.swift`, `DownloadActivityAttributes.swift`, `src-tauri/gen/apple/DownloadActivity/`, `project.yml` | One activity follows the newest active acquisition row (queued/downloading/verifying/paused), updates on status/byte change, ends on complete/failed/cancelled and dismisses when the row leaves the list; private/read-only and unavailable/denied notifications publish nothing. Real throttled loopback download on the iOS 26.4 iPhone 16 Pro simulator produced `active` ActivityKit state and a `WidgetRenderer-Activities` session with Jindo island metrics; seven host tests cover selection, terminal, dismiss, suppression and throttle. Simulator screenshots do not composite the island overlay, so the visual island/lock-screen capture, the completion presentation and physical-device rendering are unverified. |
| Durable HTTP/Metalink jobs, progress/speed/ETA, errors/pause/resume/cancel | `acquisition.rs`, `Downloads.tsx`, `publication.rs` | Job state independent of library. Validators/ranges/no-clobber publication are source-backed, not crash/power-loss acceptance. Partial jobs stay visible but incomplete ZIM reading is not promised. |
| BitTorrent, restart restoration, limits, explicit completed seeding, sharing an imported archive, BEP 19 HTTP web seeds | `torrent.rs`, `torrent/web_seed.rs`, `Downloads.tsx`, `Library.tsx`, `AdvancedSettings.tsx` | Web-seed-only torrents (including StreetZim map torrents whose `url-list` points at a direct HTTPS object) are filled through the confined staging factory, every advertised piece is SHA-1 verified, and the normal engine then hashes and publishes the same files; Range resume, multi-file URL construction, padding, redirect/credential policy and cancellation are bounded. A real 50,781,499-byte StreetZim Bahrain web seed downloaded and verified in 6.7 s in a host test; [details](WEB-SEEDS.md). Actual independent-peer desktop transfer survived graceful exit, manual pause, tampered staging and SIGKILL; resumed bytes matched the exact remainder and final SHA-256 before import. Current-SDK Android exercised foreground-service background progress, persistent pause, interrupted non-auto-resume, retained pieces, final hash/import and map reading. Current-SDK iOS simulator exercised persistent pause/restart, genuine background checkpoint, interrupted non-auto-resume and explicit resume fetching exactly the missing verified payload before final hash/import/map reading. iOS does not promise indefinite background torrent execution. Physical-phone/network, HDD and large-file acceptance remain separate; cancel retains staging and acquisition may upload. Sharing an existing library archive creates deterministic metadata and seeds the original read-only file; a fixture test asserts a stable magnet and untouched bytes, and a second local profile downloads the magnet to a SHA-256-identical, importable archive. External tracker and third-party client interoperability are not claimed. |
| LocalSwarm QUIC discovery/manual inspection, verified receive and selected-archive offers | `src-tauri/src/localswarm/`, `LocalSwarmControls.tsx`, `Downloads.tsx` | Real protected transfers both directions with Swift/Go, remote M5 Mac and Alpine Linux peers; exact output hashes checked. Current native Apple client/server also transferred the exact 2,159,190-byte fixture with the remote M5 Go peer; client discovery used a truthful owned proxy record. No automatic offering; opening Nearby starts discovery. PINs memory-only. Single root ZIM/complete root split set; shared certificate is not publisher trust. Broad producer/multihomed, import-error/retry, restart/rehash and packaged/RTL acceptance remain; [details](LOCALSWARM.md). TCP/BLE are not advertised. Apple QUIC includes AWDL. Pairing coverage and platform limits: see **Sharing pairing matrix** below. |
| Capacity/access, FAT/FAT32 checks and managed-download migration | `storage.rs`, `library_files.rs`, Settings, acquisition/torrent/maps and publication | Known FAT/FAT32 limits reject oversized individual files; exFAT is not classified as FAT32. Changing download directory offers an explicit move of managed archives, leaving imported originals/unavailable records unchanged. Exact-part SHA-256 manifests, no-replace publication, owned recovery journals and same-/cross-volume ordering are source-backed; real FAT/exFAT/NTFS, collisions, full-disk, crash/restart and recovery acceptance remain needed. |

### Sharing pairing matrix (verified 2026-11-09)

Every supported pairing was exercised with real devices and verified fixtures.
Full receipts and scope notes live in
[NEARBY-ACCEPTANCE.md](NEARBY-ACCEPTANCE.md).

| Sender → receiver | Path | Verified result | Constraints |
| --- | --- | --- | --- |
| iPhone → Android | App-owned temporary Wi-Fi: Android hosts the AP, iPhone joins | Passed, `device-nearby-de2e96dc…`; import → publish → join → complete | iPhone must hold the Hotspot Configuration entitlement and accept its OS join |
| Android → iPhone | Same app-owned temporary Wi-Fi | Passed, `device-nearby-6525d3ea…`; catalog → completed → readable | iOS joins through its normal Wi-Fi prompt; Hotspot Configuration entitlement required |
| Android ↔ Android | Shared Wi-Fi LAN, no app-owned access point | Passed both directions: Samsung→Fire `0816372c…`, Fire→Samsung `42f3a714…`; reader content matched | Both devices must be on the same reachable subnet; **no** app-AP join |
| iPhone → iPhone | Apple LAN | Passed iPhone 17 → iPhone 13 (`device-nearby-5805dafb…`); reverse ordering not separately certified | Infrastructure versus AWDL was not measured; both phones must be on a reachable Apple LAN |
| Mac ↔ Android | Apple/Android LAN library | Passed both directions, `forward-fixed2…` and `reverse-routing…`; SHA-256 verified | Mac cannot host an app-owned AP; use the same LAN |
| Mac ↔ iPhone | Apple LAN | Passed both directions, `device-nearby-c2b13c95…` / `7d37470a…` | Same LAN requirement as iPhone↔iPhone |

Platform capability limits (answered explicitly, never by silently changing topology):

- **Android 9 / API 28 receivers** (e.g. Fire OS 7) cannot join an app-created
  temporary hotspot — `WifiNetworkSpecifier` requires API 29; the app returns
  `NEARBY_JOIN_UNSUPPORTED`. They **can** receive over a shared WPA2 LAN.
- **WPA3/SAE**: the tested Fire HD 10 on Fire OS 7.3.3.1 rejected the tested
  WPA3 network and the tested iOS 26 Personal Hotspot (`AKMP_NOT_VALID`); that
  device needs a WPA2 network. Other tablets/hotspot configurations were not
  measured.
- **`Share all` never creates an access point.** With no usable local Wi-Fi it
  returns `NEARBY_LAN_REQUIRED` and points at **Use temporary Wi-Fi**.
- LocalOnlyHotspot creation can fail on OEM builds (`HOTSPOT_UNAVAILABLE`);
  the UI reports the failure. That retained Fire failure began with
  `sta_connected:false`, so station state was not established as the cause;
  automatic preset restoration was not demonstrated (manual restoration was
  required). Consult the run's cleanup/restoration evidence.

## Reading and personal navigation

| Major behavior | Current source/reference | Limit / acceptance still needed |
| --- | --- | --- |
| HTML/JS/resources, MIME/UTF-8, root links, redirects, Fetch, byte-range media | `server.rs::archive_request`, `native/archive.cpp`, `reader-bridge.js` | Real Wikipedia/Zimit/Gutenberg/maps/WASM/WebGL/media-seeking fixtures on packaged WKWebView/WebView2/WebKitGTK are required. Codec/API presence is not compatibility evidence. Service workers are explicitly blocked. Phone WebKit without a VP8/VP9 decoder shows a per-element format notice and gets metadata-only, non-autoplay buffering; decoder DOM churn is excluded from the theme scan. Actual decode speed and thermal behavior on a physical phone remain unverified. |
| Back/forward, home/random, query/fragment/SPA route identity, link actions and background tabs | `Reader.tsx`, `reader-bridge.js`; explicit `TabLocation`/`Tab` state | Actual desktop production UI exercised middle-click background app tabs while preserving the active article, 640px scroll and selection, plus trusted internal/external Copy link, keyboard context actions and stale/synthetic rejection. Current native macOS WK/NSPasteboard and Linux ARM64 GTK/Xvfb also passed context/middle-button background tabs and actual clipboard readback. Portable state is not opaque WebKit interaction-state fidelity. |
| Error severity and quiet recoverable failures | `src/error-visibility.ts`, `src/diagnostics.ts`, `App.tsx` `onError`, `Reader.tsx` `report`, `Settings.tsx` Troubleshooting, `store.rs` `Settings.show_technical_errors`, `archive.rs` `is_entry_not_found` | Severity is a call-site parameter, never a parsed message. Blocking failures (unreadable archive, storage handoff/acknowledgement, privacy/authority refusal, disk full, a user-started download/import, quit/close) always render. Recoverable failures (missing navigation target, visit/search bookkeeping, optional metadata/illustration/suggestion refresh, remembering a dialog directory, sensor/capability activation, a subresource the archive server answers 404 to) render only when the default-off “Show technical error details” setting is on. Recoverable failures are still recorded in a 50-entry in-memory list with a copy action under Settings → Troubleshooting; no file, telemetry or network. A missing entry opens the archive's main document with a quiet inline notice. Runtime acceptance on macOS covered the recoverable miss with the setting off and on, and a corrupted archive (blocking) with the setting off; decorative/refresh failures are recorded without a band, and archive subresource 404s remain server responses rather than shell errors. |
| Title/TOC panels, collapsed ancestor expansion and find | `Reader.tsx`, `reader-bridge.js`, Settings | Generated heading TOC, fragment navigation and find previous/next exist. The saved opt-in “Automatically expand article sections” choice expanded a real collapsed Details on the next reader load; disabling restored archive-authored collapse on a subsequent load. Background tabs apply the preference when activated without retaining background frames. Dynamic archives/history fidelity and native semantic PDF TOC remain separate boundaries. |
| Title/full-text search across selected/all archives and snippets; opt-in spelling suggestions; debounced type-ahead title suggestions | `Library.tsx`, `Reader.tsx`, `search.rs`, `spelling.rs`, archive bridge | Actual title boundary returned 1,000–1,049, total at least 1,050 and `hasMore`; a real title dictionary corrected `Paginatoin` to `Pagination`. Empty selection means all available archives; full text requires an index. Type-ahead suggestions run only over explicitly selected archives, are title-only (never snippet text), and page with a prefix/scope-bound continuation token; fixture and dispatch tests cover paging, cancellation and stale tokens. Explicit cancellable dictionary builds, identity-keyed bounded caches and single-flight background loading; private/read-only disables builds. Not universal linguistic correction. |
| Movable tabs, reopen closed, per-window sessions/geometry, reading positions/history and unified search | `App.tsx`, `UnifiedSearch.tsx`, `store.rs`, `windows.rs`, `native/tabbing.mm` | Drag/drop and explicit tab moves; combined local open-tab/bookmark/history search with section selection. Closed stack is runtime-only (up to 20); persistent tabs up to 200/window, native windows up to 16. macOS also exposes AppKit window tab groups (one native tab per window, preferred tabbing mode, shared identifier) with a native Window menu for Show Previous/Next Tab, Merge All Windows and Show Tab Bar; the in-window strip remains the reader-tab model. File → Restore previous session reopens the snapshot written at the last non-private Quit and is refused in private/read-only profiles. Native tab-group membership is not persisted; Qt geometry import and remote ranking remain out of scope. |
| New/empty tab search, recent queries and saved-article suggestions | `Reader.tsx` empty-reader/search UI; Store recent searches | Recent queries are privacy-gated and keyed by the current book, so removing one in a book does not remove it elsewhere. Library-wide and empty-tab queries remain global and appear on the empty-tab search surface; there is no remotely ranked “top articles” service. Do not equate useful new-tab UI with the entire aspirational upstream request. |
| Reading list, editable titles/notes/tags, anchored highlights/comments, JSON/XML interchange | `Collections.tsx`, `Reader.tsx`, `reader-bridge.js`, `migration.rs`, `store.rs` | Actual Unicode annotations survived restart/remount; repeated/ambiguous/missing passages, collapsed-ancestor Jump, long/duplicate imported IDs and 200% fallback zoom passed targeted scenarios. Atomic deltas preserve concurrent notes/other passages; personal comments never enter archive messages. Maximum 200/bookmark and 1,000 displayed/document, with bounded indexing. JSON/XML preserve this app's extensions; older clients may discard them. |
| Private reading-data device controls | `PrivateReadingSync.tsx`, `private-sync-api.ts`, `Settings.tsx`, `Collections.tsx`, `App.tsx` | **Connect my devices** is separate from public **Share all**. QR/copy invitations require matching-code confirmation on both devices; bookmark/annotation and per-device-position categories need explicit consent. Controls include **Sync now**, foreground-only automatic sync, disable and disconnect. Concurrent note/comment versions offer Copy before confirmed destructive resolution; native commits reject stale presented-version sets. Continue reading resolves an installed archive natively and opens a new tab; missing archives remain unavailable. Reset requires resolved conflicts, disconnects peers, keeps the local reading list/local positions and removes received positions; other devices must reset before reconnecting. Real two-process desktop lifecycle/security scenarios and actual two-browser pairing/manual/automatic transfer/conflict-cancel/explicit-resolution flows passed; mobile, hardware-accessibility and broader security acceptance remain separate. |
| Explicit private sync position recovery | `PrivateReadingSync.tsx`, `private-sync-api.ts`, native `private_sync_clear_positions` | Separate confirmed **Clear saved sync positions and reset connections** forgets all saved sync position candidates and disconnects every device while preserving local reading history, bookmarks and open tabs. Unchanged history/tabs do not automatically restore forgotten candidates; subsequent real reading-position edits may create new ones. Other devices must also reset before reconnecting. Normal reset remains non-destructive to this device’s saved position candidates; neither reset promises to bypass active-data or safety bounds. Actual host UI recovery removed a real candidate/peer trust, kept a nonempty bookmark/history and did not refill after an unrelated settings transaction; mobile and larger active-data acceptance remain separate. |
| Recently read, combined history search, clearing and JSON export | `Collections.tsx::recentHistoryEntries`, `UnifiedSearch.tsx`, service `record_visit`, Store | Library secondary action, not primary navigation. Deduplicate by book/decoded article/query, preserve newest full fragment/scroll and distinct articles; malformed-percent and invalid-UTF8 identities are separated. Eleven actual-helper behavior cases and repeated UI archive openings passed. Raw visits remain intact; privacy/read-only gates remain. Export history writes a versioned, lossless JSON document of every stored visit, newest first with deterministic identity tiebreakers, through the same save picker and cancellation path as reading-list export; there is no matching import and the action never mutates history. Broader multiwindow/restart and native picker acceptance are separate. |
| App system/light/dark chrome; reader original/light/paper/dark/font/size and per-book 25–500% zoom | `styles.css`, `reader.css`, `Reader.tsx`, bridge, `api.ts`, Settings | Book UUID overrides reset to the global default; strict read-only changes remain tab-local. Control+wheel over the active article uses the same 10-percentage-point steps as reader zoom controls, without changing shell scale; ordinary wheels remain archive scrolling/map input. The bridge announces a private per-document token before archive scripts run; the shell pins the first announcement and rejects synthetic wheels, replayed sequences, cross-frame posts, stale sessions, and archive documents that replaced themselves. Archive-reported navigation or fragment changes never reopen that pin; only a new iframe context does. Actual integrated-release Chrome acceptance passed genuine 100→110→100, unchanged shell scale, ordinary scrolling, right-book persistence across relaunch, those forged-proof rejections, and strict read-only tab-local zoom with byte-identical durable state. After a full in-article navigation that reuses the iframe context, Ctrl+wheel stays inactive until the article is reopened, while mobile reading-pan chrome keeps its per-session binding; continuous post-navigation zoom needs a fresh-context handoff. Native WebKit/WebView2 delivery, packaged sandbox grants, trackpad-pinch distinction, physical map input, and the 25%/500%/reset runtime boundaries remain unverified. Reader styling may conflict with arbitrary archive layouts; no universal archive-dark fidelity claim. Multiwindow and privacy-transition persistence remain separate acceptance. |
| Read aloud article/selection, installed voice/rate/pause/resume/stop, read-along highlighting | `Reader.tsx`, `native-speech.ts`, `mobile_speech.rs`, native Android/iOS engines, `reader-bridge.js`, `kokoro.rs` | Desktop uses system webview speech; mobile uses native TTS with root-owned, source-bound capabilities. Optional English Kokoro voices are downloaded in Settings and run through sherpa-onnx + Misaki G2P + rodio; the existing native ownership/capability path remains authoritative. A play control sits in the reader header and starts the current article without opening the speech panel; on mobile it becomes pause/resume, while desktop keeps stop. Kokoro is offered only after a one-time uncached performance probe passes the real-time threshold; slow devices automatically keep system TTS. Kokoro prefetches the next sentence while the current one plays, paints each sentence boundary as its WAV advances (including while retained reader storage is frozen), moves currency after magnitude (`$500 million` → “five hundred million dollars”), filters the silent legacy `Junior` voice so an installed Kokoro voice wins for English, and publishes a Now Playing card with play/pause/previous/stop on iOS 27+. On iOS 16.4+ the same session also publishes a read-aloud Live Activity with the article title, voice, sentence progress and lock-screen/Dynamic Island previous/pause/stop links; private mode clears it, and the links route back through the existing speech-action ownership path. Android emulator exercised a real 181.9 MB model download, engine load, AAudio playback, cached WAV output and a background media session. The extraction rules skip site chrome, duplicate titles and trailing reference sections. Actual Android/iOS simulator article/selection callbacks, Stop, preferences and genuine background retirement were exercised. Latest native foreground checks recovered installed voices and allowed fresh trusted playback without expired/retiring authority errors or automatic resume. Native callbacks are not proof of audible output, physical iPhone acceptance or every installed language. Read-along sentence/word paint is implemented from deterministic text extraction plus engine boundary offsets; CSS Custom Highlight is used where available and an aria-hidden overlay otherwise. Boundary callbacks are engine-advisory, so a voice that emits no ranges may leave the starting sentence painted, and audible alignment on physical devices is not certified. |
| Original HTML, MHTML/static assets and exact resource saves | `api.ts`, `exports.rs`, resource context handling in reader bridge | MHTML exports original article/static dependency graph, not dynamic runtime captures; explicit resource/size bounds fail visibly. A reference to an entry the archive never contained (DevDocs styles name unpacked sprites) is left unresolved, as the reader shows it, instead of refusing the export; the article itself and every other read error still fail. Windows: MHTML of a DevDocs article failed on such a sprite before this change. Trusted attachment opening is a separate confirmed, MIME/signature-allowlisted action, not arbitrary execution or an archive-authorized OS opener. |
| Text/media resource views, attachment Open/Save and system print/PDF | `server.rs::resource_document`, `Reader.tsx`, `library_files.rs`, `exports.rs` | Mixed-case/parameterized HTML and XHTML fixtures activated their reader bridges. A trusted confirmed PDF Open launched macOS Preview with byte-exact extracted data. PDF/EPUB use trusted-shell Save or confirmed OS Open, not unreliable inline plugins. Allowlisted temporary copies are bounded at 512 MiB; external apps may network. No native PDF/EPUB renderer, universal codec/print support or full Apple OS sharing picker. |
| Direct article PDF Save/Share | `native_pdf.rs`, platform providers, `exports.rs`, Reader/API | Actual native PDF output: macOS multilingual/image three-page output, Linux ARM64 multilingual one-page A4 output, Android native Save as PDF/DocumentsUI output, and iOS native picker multilingual three-page save. iOS/macOS/Android also share the rendered PDF through the system share sheet; Android uses the same public-SDK `NativeArticlePdf` capture and returns `SystemShareOpened`. Static original-article snapshot, not dynamic reader DOM. Source caps include 512 pages, 64 MiB final output and cooperative cancellation; macOS 513-page rejection, policy cancellation and read-only/overwrite preservation passed. A physical Samsung SM-A356U share to Google Drive produced a valid two-page PDF; StreetZim’s WebGL map canvas was blank while UI chrome was preserved. WebGL/canvas fidelity, Windows provider runtime and universal recipient interop remain unverified. Original HTML/MHTML/resource saves remain separate. |
| Native desktop printer selection | `api.ts::canPrintArticlePdf`, `pdf_export.rs`, `native_pdf/linux.rs`, `native_pdf/windows.rs`, `native_pdf/windows_print.rs`, `native/print.mm` | Desktop Print is wired to native providers, not only a browser `window.print()` fallback. Current Linux real Ctrl+P opened GTK selection, native Tab selected Print to File, and Cancel returned to reader navigation. A new article's dialog reopened and cancelled successfully. This is dialog/lifecycle proof, not physical output, new PDF generation or Orca speech. Windows native Print dialog open/cancel and crash recovery are now recorded in the [Windows report](WINDOWS-TEST-REPORT.md); filling the PDF Save As dialog was not automated. See RELIABILITY for receipts. |
| Internal link previews | `reader-bridge.js`, `Reader.tsx`, service `article_preview`, Settings/Store `linkPreviews` | Persisted opt-out defaults on for existing profiles. Actual disable/restart/re-enable and keyboard preview/dismissal passed; queued/pending replies cannot revive disabled cards. Same-archive PNG/JPEG/GIF/WebP artwork is bounded to 256 KiB and embedded as an inert data URL, never fetched from the network. Redirect-relative/encoded/base/fallback regressions and actual PNG decoding passed. No executable archive HTML, remote thumbnails, SVG or universal content-type promise. |

## StreetZim on all five platform targets

`src-tauri/src/maps.rs` is the single shared Windows/macOS/Linux/iOS/Android engine; `src/components/Maps.tsx` and `src/map-data/` implement area groups/search, language selection, transfer/output size, saved maps, progress, pause/resume and cleanup. It is **native recipe composition**, not merely opening prebuilt map archives. The same map job appears in central Downloads with separate network/assembly progress and verified-import state. Map resume follows its own durable output-prefix protocol, not HTTP range-resume or BitTorrent seeding semantics.

Source-backed protocol behavior: `streetzim-build-index/1`, `streetzim-recipe/2`, `/build/` relative resolution, app/version/platform User-Agent, HTTPS/proxy source handling, bounded exact 206 range reads, checked masked chunk SHA-256/length cache, revalidated local base/other-language reuse, output UUID/size/MD5/checksum validation, no-clobber publication, and post-publication library import. Pause retains a journaled, synced output prefix; explicit Resume verifies directory/inode/length, streams its SHA-256 and reconstructed MD5 state with a bounded buffer, and requires the exact recipe/source fingerprint before appending. Checkpoints persist at bounded output intervals and on pause/failure; restart preserves a valid checkpoint rather than deleting it. Terminal map status is persisted; a failed import does not erase a verified published file.

Source caching is limited to 64 MiB payload (16 MiB per entry) and 4,096 entries, with eviction before temporary writes and RAM-only handling for larger valid chunks. Composition retains one growing output; checksumming/import/publication do not require a second full output. Allocation rounding, metadata, recipe/job journals and pre-existing local inputs are additional space. Retry can redownload evicted chunks.

Map acquisition first prepares metadata-only native intent bound to the chosen recipe, settings, privacy generation and exact installed library principal. A matching installation offers Open existing, Replace or Cancel before source payload; a new map starts without an extra confirmation. Replace stages a uniquely named, no-clobber output, verifies it, then atomically switches the library and its bookmark/history/tab references under source and reader-retirement gates. It is not an in-place file overwrite: old archive bytes remain, including managed old files; provider originals are never implicitly deleted. Old-file reclamation is outside Replace and outside cleanup of the new download; this change does not claim an old-file cleanup UI. Changed installations/settings/version or a privacy cutover invalidate the original choice rather than retargeting it. Resume after restart can explicitly renew only the same original snapshot. A completed unimported replacement needs a fresh explicit choice after restart; that choice can reuse its still-proven complete output and reverify it without transferring it again. Fresh new-map completions reconcile through native Library startup/maintenance without relying on an ephemeral frontend watcher.

Accepted map work uses the native privacy-policy epoch, not the reader IPC epoch. Ordinary background/screen-lock reader retirement therefore does not revoke an already accepted download or replacement. A genuine private/read-only policy transition still invalidates its original replacement authority, including an on/off policy cycle.

New map jobs report the exact unique required SHA-256/length block count, including the base UUID check, with verified blocks ready and the network-downloaded subset. Reused blocks are ready minus downloaded; repeated references and retry attempts do not inflate progress. The byte counter describes the current block, not total map transfer. Historical journals without block counters remain unknown rather than displaying invented zeros.

Delete download uses only recorded job-owned partial or completed-file authority; completed outputs bind original directory/inode/size and the SHA-256 captured during composition. Cleanup checks current contents and the exact staged file before unlinking, and refuses installed reservations until the protected current-installation removal succeeds. Legacy completed jobs without proven publication authority fail closed before any library file is removed. Shared map cache entries are not globally cleared by deleting one download. List-only dismissal is refused while a job owns a partial or an unimported completed output, so its only Resume/Delete handle cannot disappear. A historical cancelled job reporting written bytes without an owned partial is not recoverable merely because that number was saved; only genuinely retained, rehashed cache/local blocks may reduce a fresh transfer.

Observed native storage, decimal MB; different recipes/filesystems and accounting scopes are **not** a platform performance comparison or universal allocation bound:

| Native target / map | Final output | Sampled peak allocated space | Accounting and acceptance |
| --- | ---: | ---: | --- |
| macOS arm64 — Hawaii/en | 150.6 MB | 161.4 MB | New output + cache; actual eviction, one output inode, recipe MD5, managed import/read, Bahrain/ar cancel/retry and Quit/restart passed. |
| Linux ARM64 — Hawaii/en | 150.6 MB | 156.3 MB | Entire owned 164 MiB profile/output tmpfs, including metadata and unknown sentinel; actual eviction, numeric low-space consent, cancel/retry/import/integrity and offline restored map/search passed. |
| Linux ARM64 — Bahrain/en | 49.2 MB | 51.5 MB | Entire owned 64 MiB profile/output volume; completed with one output. |
| iOS simulator — Washington D.C./en | 123.2 MB | 144.4 MB | New output + cache; recipe MD5, managed import/read and actual HTML cancel/retry passed. Not physical-iPhone transfer acceptance. |
| Android emulator — Bahrain/en | 49.2 MB | 51.5 MB | Output + cache/maps metadata; one growing output, recipe MD5 and managed import. Later native runs also passed positive-output Spanish Bahrain cancellation/cleanup/retry and actual rendered reading, plus Activity recreation/finish/reopen and transfer without an Activity. Separate 201MB JNI cold/warm and four negative modes passed with observed 8MiB cache eviction; these are not physical low-end performance numbers or final latest-APK acceptance. |

Evidence: `.native/mac-bounded-map-20261030`, `.native/linux-bounded-map-retirement-20261030`, `.native/ios-consumer-phones-20261030`, `.native/evaluation/android-bounded-map-5560`. Samples can miss transient peaks; directory/volume metadata is excluded where the row does not include the whole volume. No low-end Android timings, macOS accelerated-canvas pixels, Linux x86 or release-package certification follow from these measurements.

Live discovery returned a format-1 index with 46 regions. Production English Hawaii was composed, imported and rendered as an offline vector map: 150,605,015 bytes, with stored/recomputed ZIM MD5 matching the recipe's `1595dadd1f60f8d157a0f6cfbeb2f776`. This exercised the real catalog/proxy/recipe on macOS Rust-service/Chrome, not a fixture-only substitute. Remaining acceptance includes damaged-local rebuild, cache corruption/reuse, cancellation/publication boundaries, saved maps after relaunch, and offline Fetch/WebGL/location on **each** packaged OS. External StreetZim catalog/proxy availability is not controlled by Kiwix. Windows SDK provisioning and Linux WebKit packaging are prerequisites, not reduced map functionality.

## Permissions, privacy, sharing, internationalization

| Major behavior | Current source/reference | Limit / acceptance still needed |
| --- | --- | --- |
| Restricted archive reading (untrusted active content) | `server.rs` archive CSP/nonce injection, `Reader.tsx`, `service.rs::set_reader_restricted`, `reader-bridge.js` | Per reader window/archive, with the tab's choice carried through saved-session restore: the Library archive details offer a pre-open restricted choice, and a reader toolbar/banner toggle serves archive HTML under `script-src 'nonce-…'` (only the injected bridge/storage scripts), `connect-src 'self'`, `frame-src/worker-src/object-src 'none'` and `form-action 'none'`; archive scripts, forms, workers and remote network requests are blocked while images/media/styles remain. Normal reading mode is unchanged: scripts, inline/eval/WASM, blob Workers, camera permission and the native geolocation bridge remain available for StreetZim-style apps. The same reader server keeps its origin and bridge, so navigation, TOC and find continue where the archive's HTML supports them. HTTP CSP/nonce, pre-open restricted selection (including an already-existing reader), saved-tab preservation and toggle round-trip are covered by service regression tests; the visual toolbar/banner and real archive-script blocking were not exercised in a packaged WebView in this pass. | 
| Archive-isolated content and rooted storage ownership | `server.rs`, `service.rs`, `store.rs`, `storage-bridge.js`, `reader-storage.ts`, `windows.rs` | App-owned installation identities isolate storage/sensor decisions from spoofable ZIM UUIDs; actual copied-UUID rejection/relink isolation passed. Twelve rapid genuine-retirement cycles retained all 24 synchronous/nested writes, with session cookies memory-only. Freeze cancellation and real handoff-timeout rollback restored active location and committed pagehide writes. Native WKWebView retirement/Quit also retained fixture data. Native IndexedDB/service workers remain excluded; crash/BFCache/all-WebView acceptance is not implied. |
| Camera/location prompt, remembered denial, trusted reset and revocation | `permissions.rs`, `native/location.mm`, `Reader.tsx`, bridge, platform metadata | Per-window/archive runtime decisions distinguish not asked, granted and denied. Block/Revoke auto-deny later requests until trusted Archive permissions → Ask again resets that sensor; privacy transitions clear grants and denials. Revoke/reset destroy the iframe/origin and cancel pending work. Camera-only policy; microphone denied. Native Apple CoreLocation now ignores stale/invalid initial fixes until a fresh result or the original timeout. Location comes from an app-owned provider, never the webview: CoreLocation on macOS/iOS and Windows.Devices.Geolocation on Windows (consent prompt, the map control and Windows's own access dialog verified on Windows 11; a live fix awaits the account's location switch). Linux (GeoClue) and Android (LocationManager) providers are written and reviewed but not yet compiled, so they stay off in build.rs. Where no provider is built, a request is answered at once with code 2 (position unavailable), no prompt, and a `watchPosition` ends. Actual iOS simulator app/OS location grants and fresh injected GPS delivery to the article passed. Physical GPS, camera capture and arbitrary provider/entitlement compatibility remain unverified. |
| Photo chooser for archive image inputs (camera or file) | `reader-bridge.js`, `Reader.tsx`, `photo_capture.rs`, `camera.html`/`camera.ts`, vendored wry; [PHOTO-CHOOSER](PHOTO-CHOOSER.md) | Windows only: an image file input in a top archive frame shows the shell's chooser (Use camera / Choose a file / Cancel). The camera runs in the app-owned `camera-capture` window, the only webview granted camera while a user-started capture is open; archives gain no camera access. Requests carry a per-session secret hidden from archive scripts and are bound to frame, session, archive and privacy generation; picked files are read bounded (25 MB) and content-sniffed, and the page never sees a path. Installed Windows 11 build with a Logitech C930e: camera photo and picked file both reached StreetZim's photo search; Cancel delivered `cancel`; no archive grant was created. macOS/Linux keep the native picker; new strings are English-only. |
| Private versus strict read-only | Store/service gates, App privacy-generation lifecycle, diagnostics | Post-handoff actual normal/private/normal isolation/restoration passed. Strict read-only disabled annotation writes, rejected direct writes and kept profile bytes unchanged while archive storage stayed transient. CLI `--read-only` refused in-process disabling and retained identical profile bytes after exit. Explicit private-mode library/bookmark/download actions may still write; neither mode is anonymity or OS-wide no-cache I/O. Multiwindow/crash and platform boundaries remain separate acceptance. |
| Automatic settings persistence | `SettingsAutosave.tsx`, Settings/AdvancedSettings, shared `api.ts` settings queue | Toggles/selects save immediately; valid text/numbers debounce for 600 ms and flush on blur, route departure and background/page hide. Invalid drafts and failed writes retain Retry. Root-owned policy/directory confirmation and optional migration survive navigation; metered downloads remain default-off. Actual compiled UI/native backend exercised failed valid write → invalid replacement → Retry, delayed older Settings/newer Reader ordering, cancellation/migration and restart. Native iOS simulator retained cellular consent across immediate Downloads navigation and process relaunch, then restored it off. Physical cellular admission and injected mobile foreground-failure recovery remain separate acceptance. |
| Selected-archive LAN serving/search, interface selection and offline QR | `sharing.rs`, `SharingControls.tsx`, `localization.rs` | Real dual-stack IPv4/IPv6 requests returned the selected Hawaii collection without unselected library entries. Per-book Continue/cookie activation returned the archive; missing credentials and foreign Origin returned 403, and explicit Stop rejected subsequent connections. Token-scoped separate listener; loopback, all IPv4, IPv6-only, dual-stack or enumerated interface. Scoped/link-local addresses excluded. Local QR generation makes no QR-service request; sharing the URL exposes its token. LAN pages localize using available catalogs. Android can bind the listener to the active LocalOnlyHotspot access-point address (`{"mode":"hotspot"}`), resolved from the live interface table rather than a vendor constant, and the Nearby hotspot card shows that access link plus a QR; leaving Nearby keeps the link a joined peer is using, while explicit Stop hotspot, Stop sharing, Stop all, background retirement, privacy/read-only and shutdown release both. Not HTTPS, account authentication, VPN or native share-sheet parity, and no persistent serving notification is claimed. Cross-device/firewall and physical QR-scanner acceptance remain needed. |
| Whole-library Kiwix server and OPDS catalog | `sharing.rs` (`SharingScope::Library`, `LibraryHosts`, OPDS builders), `service.rs`, `SharingControls.tsx` | An explicit `scope: "library"` start serves every currently available archive behind the same unguessable path token; the UI shows a separate "Entire library" choice and confirmation. The index lazily binds one listener per book (root-relative assets keep working), lists books and links `/catalog/v2/root.xml`, `/entries`, `/categories`, `/languages` and `/download/{uuid}`. Every request re-reads the library record, requires the recorded installation/path and re-opens a changed file only after its ZIM UUID matches; a removed or replaced file returns 410. Ranged downloads share the bounded archive cache keyed by the size/mtime stamp, so a Range request does not re-open libzim, and archive responses add `frame-ancestors 'none'`. Verified with two fixture archives: curl returned the index (2 books), the root and entries feeds, a per-book activation cookie and `index.html`, then connection-refused on every port after stop; the integration test asserts the same plus downloads, language facets and the missing-file refusal. Not covered: Windows/mobile runtime verification, third-party reader interop, HTTPS or account authentication. |
| Native application shortcuts and keyboard UI | `menus.rs`, App shell events, Reader trusted keyboard paths, vendored wry `forward_accelerator_keys` | Native accelerator authority is not archive `postMessage`. Windows: keys typed into WebView2 never reach the message loop where Tauri translates menu accelerators, and a single-webview window reports itself unfocused while its page has focus, so shortcuts were dead with a page focused. The vendored wry now re-posts Ctrl key-downs (not the Ctrl+A/C/V/X/Y/Z editing keys or auto-repeat), the menu's Alt keys (0–9, Home, Left, Right) and function keys except F10 to the top-level window, skipping Ctrl+Alt (AltGr); `menus.rs` falls back to the foreground shell window. Verified on Windows 11 with real keystrokes: Ctrl+T, Ctrl+W, Ctrl+Shift+N, F12, Alt+1, Alt+F4 closing exactly one window, and copy/paste/undo acting once in a text field. AltGr dead keys on non-US layouts are untested. Focus, RTL shortcuts, mnemonic/access-key localization and actual screen readers remain separate acceptance; ARIA alone is not conformance. |
| 74 locale entries, offline catalog loading, RTL, formatting/plurals and localized LAN pages | `i18n.tsx`, `src/locales/`, `scripts/localization.mjs`, `localization.rs` | Current generated metadata: **1,442 sources**, English alone string-complete, all **73 non-English catalogs partial** with English fallback. Coverage does not establish translation quality or competent independent language review. Earlier redesigned Library at 320/430px Arabic RTL and desktop width had no horizontal overflow; that is not acceptance of every new dialog. Producer metadata and raw backend diagnostics are not wholly translated surfaces. |
| Local diagnostics/About/privacy/support | `diagnostics.rs`, Settings, service, UI links | Opt-in bounded categorized local logs, reports and source/privacy links; no silent telemetry. Inspect before sharing. No comprehensive AT/security audit certification is claimed. |
| Look up from other apps and the AI assistant connector | `launch_search.rs`, `windows/installer-hooks.nsh`, `linux/main.desktop`, `Info.plist`, `agent_connector.rs`, `kiwix-mcp/`; [PLATFORM-INTEGRATIONS](PLATFORM-INTEGRATIONS.md), [AGENT-CONNECTOR](AGENT-CONNECTOR.md) | `kiwix-workbench --search "<text>"`, a bare desktop-entry URL, and `kiwix://search?q=<text>` open Library search without recording it as a recent search. The Windows installer, the macOS bundle and the Linux desktop entry register the `kiwix:` handler; Windows uninstall removes only its own key. The opt-in, off-by-default connector exposes three read-only tools (search/list/read) to local MCP clients through the `kiwix-mcp` relay over an owner/SID-checked named pipe on Windows or a same-UID Unix socket on macOS/Linux; private and read-only modes close it and cancel calls. Windows was exercised on installed Windows 11 builds (cold/warm links, rejected links, a 20-link flood, discover/list/search/read/paging, legacy handshake, toggling, squatter refused). The packaged sandboxed macOS app and its bundled relay were exercised end-to-end on 2026-11-16 (socket creation, list/search, private-mode stop, packaged relay discovery). The relay compiles for Linux and has socket-path tests; a Linux desktop app build and runtime run remain required. |

## Compatibility and distribution exclusions

- Explicit legacy library XML/local-library OPDS and bookmark XML imports exist; local acquisition paths resolve against the selected original directory and missing references are retained. Broad producer compatibility remains unverified. No automatic Qt QSettings/portable-root/session/job conversion or Apple Core Data/security-scoped/URLSession migration. Original files are not rewritten in place; local-library import is distinct from OPDS catalog discovery.
- Linux file launcher/MIME metadata, Windows NSIS associations, macOS bundle metadata/runtime relocation and workflow instructions are source-backed in [PACKAGING](PACKAGING.md). Packaged execution, signing/notarization, clean-machine runtime closure, GitHub release publication beyond unsigned Windows test prereleases, stores, MSIX, Flatpak, PortableApps, ARMv7 and a signed updater are not claimed shipped.
- Mobile is now part of the requested scope. Direct external-provider reading, cellular/metered-download policy, archive backup policy, native lifecycle and packaged iOS/Android acceptance are tracked as mobile parity work, not dismissed as desktop-only exclusions. Apple Pay requires an authorized merchant/payment integration; no working payment flow is claimed. No account/cloud synchronization, native EPUB renderer or arbitrary browser-extension/service-worker subsystem is claimed.
- [LocalSwarm P2](LOCALSWARM.md) is implemented with real peer interoperability and remaining integration gates. The complete historical audit now covers [792 desktop issues](audit/desktop-all-closed-issues.md), [590 desktop PRs](audit/desktop-all-closed-prs.md), [801 Apple issues](audit/apple-all-closed-issues.md), and [919 Apple PRs](audit/apple-all-closed-prs.md). Applicable new fixes require their own adversarial and runtime acceptance; historical closure alone is not proof.
