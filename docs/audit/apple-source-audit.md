## Priority handoff: StreetZim implementation slice

**Current Apple code implements native StreetZim composition, not merely map-ZIM reading.** It is desktop-applicable and is not gated by the old debug-only `FeatureFlags.map` flag.

Use these files immediately:

| Source | Contract |
|---|---|
| `../kiwix-apple/Model/Maps/StreetZimCatalog.swift:6-27,103-207,247-288` | Catalog schema, language selection, endpoint resolution |
| `../kiwix-apple/Model/Maps/StreetZimRecipe.swift:9-204` | Recipe schema, overflow-safe validation, mask indexing |
| `../kiwix-apple/Model/Maps/StreetZimAssembler.swift:43-210` | Verified streaming assembly, local/archive/cache reuse, publication |
| `../kiwix-apple/Model/Maps/StreetZimAssembler.swift:212-276` | Capacity preflight, unioned chunk references |
| `../kiwix-apple/Model/Maps/StreetZimAssembler.swift:278-409` | Bounded HTTP JSON/range reader |
| `../kiwix-apple/ViewModel/StreetZimViewModel.swift:16-263` | Job lifecycle, catalog/recipe consistency, cancellation, import |
| `../kiwix-apple/Views/Library/Categories/StreetZimMaps.swift:16-160` | Area/language UI, saved maps, size, cache clearing |
| `../kiwix-apple/docs/streetzim-maps.md` | Detailed behavioral constraints; source corroborated below |

### Exact wire formats

**Catalog: `streetzim-build-index/1`**
```text
{
  format,
  regions: [{
    id, title,
    base: { file, size, download, url? },
    menu: [{
      lang, label, name, recipe, size, download_bytes, md5
    }]
  }]
}
```

**Recipe: `streetzim-recipe/2`**
```text
{
  format, uuid, name, file_name?, size, md5, file_md5?,
  base: Source,
  packs: { [packKey]: Source },
  segments: [{
    src, pack?, off?, len?, data?, sha256?
  }]
}

Source = {
  file, size, uuid, chunk, sha256: [hexDigest],
  mask?: [[offset, length]], url?
}
```

`src` is `base`, `pack`, `inline`, or terminal `md5`. Inline bytes are base64 plus length/SHA-256. Masks describe source bytes zeroed **before chunk hashing**; a source-copy segment must never intersect masked bytes.

Important compatibility/security rules:

- Default index: `https://streetzim.web.app/build/recipes/index.json`.
- Default proxy: `https://streetzim-build-proxy.tiltastech.workers.dev`.
- Relative recipe paths resolve against **`/build/`**, not `/build/recipes/`.
- Replace `{proxy}` in source addresses. Resolved recipe/source URLs require HTTPS, a host, and no embedded credentials. This is **not** a same-host allowlist.
- JSON requires HTTP 200, at most 32 MiB; time limit 60 seconds.
- Ranges require HTTP 206 and exact `Content-Range: bytes start-end/sourceSize`, `Accept-Encoding: identity`, exact returned length; limit 16 MiB/request, 90 seconds. Unknown Content-Length is permitted, but received-byte caps still apply.
- Source chunks are bounded to 16 MiB. Validate hash count as `ceil(size/chunk)`, lowercase hex hashes, UUIDs, bounds, total output size, terminal checksum, and filename safety.
- Normalize duplicate/overlapping/adjacent masks into sorted unions; use binary-search intersections, cancellation during validation, and checked arithmetic.
- Cache key is **`<masked-chunk-sha256>-<length>`** under `StreetZimChunks`; verify cached bytes again before reuse.
- Try installed base UUID, then a language version from the selected region. Reuse only bytes matching the **new** recipe’s masked chunk hashes. Open source archives read-only.
- Before assembly, compare recipe/catalog `md5`, `size`, base `file`, and base `size`.
- Validate base UUID from first source chunk’s bytes `[8..<24]`.
- Stream segments while hashing; require recipe `md5` for bytes preceding the terminal 16-byte ZIM checksum; append that checksum; optionally validate `file_md5` including checksum.
- Write a unique hidden `.streetzim-<UUID>.part`, synchronize, then publish without overwriting any existing file. Failed normal jobs remove partial output.
- Retry each source request up to three total attempts, retaining verified cached chunks. Restart reconstructs output rather than resuming partial output.
- Import only verified published output. `LibraryOperations.open` now saves in an owned background context; failed import must not roll back unrelated UI edits.
- Existing readable map + failed database import means report failure, not redownload. Unreadable installed map permits rebuilding under a different filename.
- Once publication succeeds, finish import even if cancellation arrives at that boundary.
- Current implementation allows one foreground composition at a time. It survives navigation, not process termination.

**Infrastructure constraint:** these endpoints are StreetZim-operated external services, not Kiwix infrastructure. Production ownership/availability needs agreement; the feature uses stock CoreKiwix, not a patched StreetZim reader.

---

# Apple feature audit

**Artifact handoff:** I could not create `local://apple-feature-audit.md`: this agent has only read/search tools and an explicit read-only constraint; no write/artifact-publishing tool is available. The following is the substantive artifact content for the parent to persist. No application files were modified and no builds, tests, formatters, or runtime checks were executed.

Paths below are relative to the desktop repository unless otherwise stated.

## Summary

The Apple source currently contains a mature reader/library application plus substantial recent local StreetZim and camera work. The strongest desktop additions to preserve are:

1. StreetZim area/language discovery and verified map composition.
2. Rich macOS multiwindow/tab-group session restoration.
3. Archive-isolated persisted cookie compatibility.
4. User-consented camera support for ZIM content and a browser geolocation bridge.
5. Searchable bookmarks and cross-archive title/full-text search with optional matching snippets.
6. Missing-file recovery, explicit integrity checks, native PDF/content sharing, and download diagnostics/accessibility.

Do **not** misclassify these as implemented Apple features: notes/highlights, an independent browsing-history manager, general bookmark interchange, configurable arbitrary OPDS catalogs, cloud synchronization, universally enabled spell correction, or fully tested physical camera capture.

Likewise, TOC, tabs, bookmarks, PDF export, printing, themes, server sharing, and multi-ZIM search are **not uniquely Apple**: the current Qt code already has corresponding capabilities.

## Architecture

- `App/`: separate macOS and iOS entrypoints and platform navigation.
- `Views/`: SwiftUI reader, library, settings, maps, hotspot, search, sharing.
- `ViewModel/`: browser lifecycle, search coordination, navigation, catalog, integrity, maps.
- `SwiftUI/Model/Database.swift`: Core Data/SQLite persistence; `Entities.swift` and `DataModel.xcdatamodel` define stored data.
- `Model/ZimFileService/`: Swift actor and Objective-C++ bridge to libzim/libkiwix.
- `Model/WebViewConfig/` and `Model/Utilities/WebKitHandler.swift`: custom `zim://` WebKit content serving, blocking, cookies, permissions.
- `Model/Maps/`: independent StreetZim catalog/recipe/range-download/assembly pipeline; completed maps reenter ordinary library import.
- `Model/Hotspot/`: libkiwix HTTP server independent of the embedded-reader custom scheme.
- `Kiwix/` currently supplies the splash storyboard; primary application implementation lives in the above directories.

## Evidence-backed implementation checklist

“Implemented” below means source-backed, **not** runtime-verified during this audit.

| Target requirement | Actual Apple implementation and meaningful constraints | Evidence |
|---|---|---|
| Read local ZIM HTML, assets, linked pages, redirects | Implemented through `zim://<archive UUID>/<content path>`, WKWebView, libzim/libkiwix. Missing main documents trigger an alert; redirects resolve inside the archive. Preserve decoded content paths and trailing slashes. | `Model/Utilities/URL.swift:18-78`; `ViewModel/BrowserViewModel.swift:333-397,469-550`; `Model/Utilities/WebKitHandler.swift:65-115` |
| Home/random article navigation | Implemented, selecting current or explicit archive. Random controls may be branding-hidden. | `BrowserViewModel.swift:366-397`; `Views/Buttons/ArticleShortcutButtons.swift`; `Model/Brand.swift:47-51` |
| Audio/video and fullscreen | Implemented using WebKit; inline mobile playback, macOS element fullscreen, pause-on-leave except PiP, platform-specific video fixes. Content served in 2 MiB stream chunks; media can use direct archive-file access. | `WebViewConfiguration.swift:19-30`; `BrowserViewModel.swift:419-447,560-579`; `Views/BuildingBlocks/WebView.swift:24-108`; `WebKitHandler.swift:136-175`; `Support/injection.js:79-167` |
| Tabs and multiple windows | macOS uses native window tabs; command-click and target-blank internal links create tabs. iOS uses persisted application tabs, supports individual/all-tab close, maintains exactly one empty tab, and chooses a neighboring/recent tab after deletion. | `App/App_macOS.swift:82-95`; `BrowserViewModel.swift:452-485,713-756`; `NavigationViewModel.swift:57-111,185-262` |
| Tab/session persistence | Tab contains created/last-opened/title/archive plus opaque WebKit `interactionState`. Save on inactive/disappear; macOS also saves after page load. macOS explicit “restore previous session” restores geometry, tab groups, selected tabs/key window and selected navigation item using `session.json`. Not a portable history format. | `Model/Entities/Entities.swift:87-173`; `BrowserViewModel.swift:312-331,399-415,560-579`; `Views/BrowserTab.swift:57-102`; `Model/SessionRestore/SessionRestore.swift:24-120`; `WindowState.swift:20-67`; `App/RootView_macOS.swift:307-359` |
| Full-text/title search across chosen archives | Implemented. Per-archive `includedInSearch` selection; full-text only where archive has an index, plus title suggestions; cancellation/debounce and in-progress UI. Retrieves up to 25 indexed + 25 title results per archive before dedup/ranking, not unlimited pagination. | `SearchViewModel.swift:84-126,160-207`; `SwiftUI/Model/SearchOperation/SearchOperation.mm:99-188`; `SearchOperation.swift:29-103`; `Views/Search/SearchResults.swift:208-236` |
| Search snippets and ranking | Optional matching HTML snippets converted to attributed text; combines title edit distance and full-text probability. Deduplicates URLs, including trailing-slash normalization in native operation. | `SearchOperation.mm:126-188`; `SearchOperation.swift:44-93`; `DefaultKeys.swift:27-30` |
| Spelling-corrected suggestions | **Implemented but configuration-gated, default false.** Creates per-archive `.spellingsdb.v0.1` caches only when enabled; suggestions run only if query length >2 and ordinary results are empty. Do not claim universally active because changelog says “spellchecked search.” | `Model/Utilities/FeatureFlags.swift:18-31`; `SearchViewModel.swift:164-187`; `SearchOperation.swift:106-121`; `SearchOperation.mm:194-222` |
| Recent search queries | Implemented in preferences; selecting a result moves query to front. UI shows six recent queries and supports removal. This is search history, not browsing history. | `DefaultKeys.swift:32-33`; `SearchResults.swift:181-217` |
| Find in page | Implemented separately from archive search: macOS WKFind with wrap/previous/next; mobile uses native find interaction. Branding may hide controls. | `ContentSearchViewModel.swift:20-59`; `Views/BuildingBlocks/ContentSearchBar.swift:32-133`; `Views/Buttons/ContentSearchButton.swift`; `Views/BrowserTab.swift:161-176,251-256` |
| Native table of contents | Implemented by injecting IDs into H1–H6 headings and posting extracted headings. macOS indented menu; iOS collapsible tree/popover. Tree drops sole top-level title matching article title. TOC depends on headings present when injection snapshots DOM; do not assume continuous dynamic recomputation. | `Support/injection.js:1-10,57-77`; `TableOfContents.swift:18-109`; `OutlineButton.swift:18-115`; `BrowserViewModel.swift:821-864`; `Tests/TableOfContentsTests.swift:20-71` |
| Appearance/zoom | System/light/dark native appearance; 50–200% zoom controls. macOS applies WebKit page zoom; mobile adjusts `webkitTextSizeAdjust`. No evidenced custom font family, sepia/reader-mode stylesheet, or forced dark restyling for arbitrary archive pages. | `SwiftUI/Model/UserColorScheme.swift:18-99`; `Views/Settings/Settings.swift:29-78,204-235`; `Views/Commands.swift:107-125`; `WebView.swift:84-87,211-218` |
| Immersive reading | Implemented mobile navigation/toolbar hiding from scroll direction; branding can disable. Desktop uses ordinary native layout/fullscreen rather than this UIKit behavior. | `WebView.swift:167-207`; `Support/injection.js:34-53`; `Brand.swift:66-70` |
| Bookmarks | Implemented add/remove, searchable title grid, latest-created-first, current/new-tab opening and context delete; URL-unique. Orphaned/missing-archive bookmarks are hidden. No folders/tags/notes/editor or import/export UI evidenced. | `Model/Entities/Entities.swift:20-38`; `Views/Bookmarks.swift:20-90`; `BookmarkContextMenu.swift:22-46`; `BrowserViewModel.swift:790-819`; data model `Bookmark` uniqueness constraint |
| Notes/highlights | **Not found.** Broad searches for notes/history followed by alternate annotation/highlighter/note-field/reading-history symbols yielded no corresponding model/UI; schema contains only Bookmark, DownloadTask, Tab, ZimFile. Do not turn desired new-app annotations into claimed Apple parity. | `SwiftUI/Model/DataModel.xcdatamodeld/DataModel.xcdatamodel/contents:3-86`; `Entities.swift`; `Views/Bookmarks.swift` |
| Browsing history | Back/forward and persisted WebKit per-tab state exist. No separate searchable chronological global visit-history model or screen found. | `BrowserViewModel.swift:75-76,170-183,312-331`; `Entities.swift:87-173`; complete schema above |
| OPDS catalog | Implemented fixed Kiwix v2 entries endpoint, parsed via `kiwix::Manager::readOpds`. ETag conditional refresh, manual refresh, auto-refresh when ≥1 hour stale, category/language selection. Refresh replaces nondownloaded records and preserves downloaded ones. No user-facing arbitrary OPDS subscription editor found. | `LibraryViewModel.swift:67-139,225-309`; `Model/OPDSParser/OPDSParser.mm:43-63`; `Views/Settings/Settings.swift:83-120` |
| Catalog/library organization | Category grids, New, Opened, Downloads, Maps; multilingual category filters; language selection searchable/sortable; opened collection sorts name/size and filters all/available/missing. Downloaded books are excluded from catalog-only listings. | `Views/Library/Categories/LibraryTab.swift`; `Views/Library/Categories/ZimFilesCategories.swift`; `Views/Settings/Languages/LanguageSelector.swift`; `ZIMsSortBy.swift:19-75`; `ZIMsShowBy.swift:19-78`; `CHANGELOG.md:42-89` with implementations above |
| Ordinary book downloads | URLSession background HTTP download, byte progress, pause/resume/cancel, stored resume state, error states, network/cellular handling, completion notifications. `.meta4` acquisition URLs are stripped to direct URLs, **not** a Metalink downloader; no torrent path evidenced. | `Model/Downloads/DownloadService.swift:33-52,80-203`; `DownloadState.swift`; `DownloadSessionDelegate.swift:133-257`; `NetworkState.swift`; `Views/Library/ZimFileDetail/DownloadTaskDetail.swift` |
| Download destination | Documents on iOS; macOS default Downloads and security-scoped saved-directory infrastructure. **macOS settings tab for changing destination is commented out**, despite implementation files/changelog. Do not present that picker as currently exposed UI. | `Model/Downloads/DownloadDestination.swift`; `DownloadLocationMac.swift:21-86`; `App/App_macOS.swift:152-156`; `Views/Settings/DownloadsMacOsSettings.swift` |
| Download completion reliability | Ordinary downloader calls import but does not check its returned success before sending completion/removing task. StreetZim path explicitly checks committed import success. Port the stronger map semantics rather than assuming all current download paths guarantee persistence. | `DownloadSessionDelegate.swift:211-225`; `StreetZimViewModel.swift:193-202`; `LibraryOperations.swift:32-68` |
| ZIM import/open | Native single-select `.zim` picker, OS file-open/deeplink handling, library registration, open main page in selected/new tab. Files are opened in place through file bookmarks, not universally copied into app-owned storage. Handler can process URL arrays although picker is single-select. | `Views/ViewModifiers/FileImport.swift:19-130`; `LibraryOperations.swift:27-68`; `App/App_iOS.swift:93-105`; `App/RootView_macOS.swift:143+`; `Support/Info.plist:11-61` |
| Missing archives and removal | Startup/activation revalidation marks inaccessible archives missing; locate-file UI, Finder reveal, unlink versus delete. Unlink explicitly deletes bookmarks, tabs and cookie state; macOS detail UI exposes unlink rather than deleting arbitrary external files. | `LibraryOperations.swift:82-137,176-242`; `ZimFileDetail.swift:180-219,297-330`; `Views/Library/ZimFileDetail/FileLocator.swift` |
| Integrity checking | Explicit check/recheck action, confirmation for archives ≥100 MiB, tri-state result and cancellation UI. Underlying operation checks cancellation around archive calls; not evidence that libzim checksum work itself interrupts immediately. | `ZimIntegrityModel.swift:19-97`; `ZimFileDetail.swift:223-294`; `Model/ZimFileService/ZimFileService.swift` |
| Export/share/print | Text MIME content exported as WebKit PDF; nontext uses archived bytes plus MIME/path extension. macOS native sharing picker and print command; mobile activity-sheet export. Copy current archive URL. This is page/content export, not bookmark/library/session export. | `BrowserViewModel.swift:225-248`; `Entities.swift:175-196`; `Views/Buttons/ExportButton.swift:18-89`; `Views/Buttons/PrintButton.swift`; `Views/BrowserTab.swift:218-249` |
| HTTP server/hotspot | Implemented libkiwix server for selected archives, start/stop, configurable port 1–65535/default 80, persisted selection, URL/QR/copy/share, occupied-port error. It serves over existing connectivity; does not create a Wi-Fi network. | `Model/Hotspot/Hotspot.swift:21-133`; `KiwixHotspot.mm:44-105`; `Views/Hotspot/HotspotDetails.swift:18-137`; `DefaultKeys.swift:56-57` |
| StreetZim maps | Implemented independent catalog, grouped/searchable areas, language variants, local-language priority, size, verified composition/cache reuse, saved-map labels, progress/cancel in Maps and Downloads, offline saved-map reopening. See priority handoff. | Maps source set above; `Views/Library/ZimFilesDownloads.swift:19-104`; `StreetZimAreaPicker.swift`; `Support/StreetZimAreaGroups.plist`; `Support/StreetZimRegionLanguages.plist` |
| Diagnostics/support | Implemented user-facing diagnostic report and integrity/download-related state; donation UI, About/version, support links. These are not evidence of silent telemetry. Donations include Apple Pay/Stripe and an external-link replacement for non-Apple-Pay distribution. | `Views/Settings/Diagnostics/DiagnosticsView.swift`; `DiagnosticsModel.swift`; `GlobalDiagnosticsModel.swift`; `Model/Payment.swift`; `Model/Brand.swift:53-64` |

## Desktop camera, location, cookies, user agent and content security

### Camera — applicable to desktop

- `ZIMCameraPermission.decision` allows only a **prompt**, never silent grant, for camera-only requests.
- Current page must be a valid `zim://` archive URL. Request origin scheme must be ZIM, UUID must match the current archive, and port must match.
- Denies other archives, opaque/invalid origins, HTTP/file/about/old `kiwix` origins, microphone, and combined camera+microphone.
- Main browser and mobile link-preview WebViews install the permission delegate.
- `project.yml:26-30,66-75` includes camera usage description and sandbox entitlement; `Support/Kiwix.entitlements:11-14` contains camera access.
- Evidence: `Model/WebViewConfig/ZIMCameraPermission.swift:8-42`; `BrowserViewModel.swift:696-712,767-775`; `Tests/ZIMCameraPermissionTests.swift:16-81`.

**Boundary:** permission policy and project plumbing are implemented; this audit did not exercise camera hardware. Local reviewer documentation explicitly says live phone permission/preview remained unverified. HTML image-file-input/“Take Photo” is a distinct OS picker path, not proof of getUserMedia capture working.

### Geolocation — applicable to desktop, not mobile-only

- Injected JavaScript implements browser current-position/watch/clear-watch through a native CoreLocation service.
- Native manager requests when-in-use permission, caches locations for five minutes, publishes position/permission/timeout errors, stops on navigation and browser destruction.
- Includes macOS location entitlement and usage strings.
- `highAccuracy` is parsed; do not assume it is fully implemented merely from request typing.
- Do not claim camera-grade archive-origin validation: the browser’s geolocation message-handler branch constructs a request from payload without equivalent explicit origin/UUID checks.
- Evidence: `Model/Geolocation/GeolocationService.swift:91-226`; `Support/geolocation.js`; `BrowserViewModel.swift:599-676`; `Support/Kiwix.entitlements:23-24`; `project.yml:27-28`.
- `geo:` external-link handling normally hands coordinates to Apple Maps in Release. `FeatureFlags.map` is an older debug-only map route and must not be confused with the native StreetZim catalog/composer.

### Cookies — applicable to desktop

- `document.cookie` and Cookie Store API are shimmed for custom-scheme ZIM pages.
- Store keyed by archive UUID, persisted to **Documents/`zimCookies.json`**, Codable `[UUID: String]`; each string contains JSON cookie entries.
- Only expiry-bearing cookies persist; session-only cookies stay in JS memory. Implements expiry/max-age and basic get/set/getAll/delete, not evidence of a complete browser cookie security/domain/path implementation.
- Debounced persistence is 1.5 seconds. Unlink deletes archive cookie state.
- Evidence: `Model/WebViewConfig/Cookies/ZimCookieStore.swift:20-66`; `ZimCookiePersistence.swift:25-93`; `Support/zimCookies.js:1-283`; `BrowserViewModel.swift:348-365,614-625`; `LibraryOperations.swift:181-186`.
- Preserve archive-origin isolation when replacing custom scheme/WebView architecture; cookie persistence is not general login sync.

### User agent

- Native catalog and ordinary-download requests use **`kiwix/<shortVersion> (apple-macos)`**, with separate `apple-ios`/`apple-ipados` identifiers.
- Evidence: `Model/Utilities/UserAgent.swift:18-44`; `LibraryViewModel.swift:227-228`; `DownloadService.swift:41-45`.
- No custom WebView user agent assignment found. StreetZim JSON/range requests set Accept/Range/Accept-Encoding but do **not** explicitly set this application User-Agent.
- New desktop should deliberately identify its own platform rather than pretending to be Apple; do not conflate native-request identification with browser responsive-rendering behavior.

### Offline/network boundary

- WK content rules block all resources except `zim://`, `blob:`, and `data:`.
- Navigation blocks non-ZIM iframes; top-level external links follow ask/always/never policy and open in OS browser, not reader.
- Native catalog/download/map services remain permitted: the resource blocker applies to page content, not all app networking.
- Service-worker-required archives have an explicit warning. Do not claim full service-worker compatibility.
- Evidence: `WebContentBlocker.swift:21-61`; `BrowserViewModel.swift:489-550`; `Views/ViewModifiers/ExternalLinkHandler.swift:35-91`; `Views/Library/ZimFileDetail/ServiceWorkerWarning.swift`.

## Shortcuts and accessibility

Source-backed desktop commands:

| Shortcut | Behavior | Source |
|---|---|---|
| Cmd+O | Open ZIM | `FileImport.swift:48` |
| Cmd+T | Native new tab | `App_macOS.swift:85-94` |
| Cmd+[ / Cmd+] | Back/forward | `Commands.swift:94-103` |
| Cmd+0 / Cmd++ / Cmd+- | Reset/increase/decrease zoom | `Commands.swift:107-125` |
| Cmd+Shift+C | Copy current ZIM URL | `App_macOS.swift:115-123` |
| Cmd+P | Print | `PrintButton.swift:53-54` |
| Cmd+G / Cmd+Shift+G | Next/previous in-page match | `ContentSearchBar.swift:111-133` |
| Escape | Dismiss find/search focus | `ContentSearchBar.swift:33-37`; `SearchResults.swift:97-102` |
| Cmd+Option+R | Random article | `ArticleShortcutButtons.swift:82-83` |
| Cmd+1 | Bookmarks | `Commands.swift:134-154` |
| Cmd+Ctrl+1…6 | Opened/categories/downloads/new/hotspot/maps | `Commands.swift:137-139` |
| Arrow keys + Return | Native result navigation/activation | `SearchResults.swift:88-107,157-178`; `Views/ViewModifiers/Focusable.swift` |

**Shortcut constraint:** both the app command and reader find component register Cmd+F; reader adds hidden Cmd+Shift+F archive-search trigger. Do not claim a conflict-free shortcut map without verifying focus resolution.

Accessibility evidence includes:

- Search result count announcements using `AccessibilityNotification.Announcement`, with title labels and snippet hints.
- Labeled in-page search and previous/next controls; decorative search icon hidden.
- TOC accessibility label.
- Map-area selected traits and stable identifiers; identifiers themselves are test hooks, not a substitute for accessible labels.
- Evidence: `SearchResults.swift:56-63,89-90,149-154`; `ContentSearchBar.swift:59-64,102-114`; `OutlineButton.swift:45`; `StreetZimAreaPicker.swift:133-134`.

Do not claim comprehensive WCAG conformance or screen-reader behavior from these source annotations alone.

## Migration/interchange formats

| Data | Actual format / migration | Desktop migration implication |
|---|---|---|
| Library/bookmarks/tabs | Core Data SQLite (`DataModel`); UUID archive identity; bookmark URL/title/date; opaque tab state | Not Qt XML-compatible. Need explicit importer and portable internal schema rather than copying opaque Core Data blobs. |
| Archive references | Apple security-scoped bookmark `Data` | macOS-specific access token, not portable filesystem path. Resolve/export on Apple side or reselect files. |
| Article URLs | `zim://<UUID>/<path>`; legacy `kiwix://` converted | Preserve UUID/path/fragment semantics. Qt currently constructs hosts `<bookId>.zim`; normalize deliberately. |
| Window session | Documents `session.json`, `[WindowState]`: frame, identifier, keyWindowId, menuItemId, otherTabIds, selectedTabId | JSON geometry/groups are understandable; menu navigation can contain Core Data object URI references, not portable stable IDs. |
| Tab history/session | WKWebView `interactionState` binary `Data` | WebKit-specific, not cross-engine history interchange. |
| Cookies | Documents `zimCookies.json`, `[UUID:String]`; inner JSON entry arrays | Can migrate explicit cookie values with expiration rules and archive isolation. |
| Download resumes | URLSession opaque resume data/state | Cannot assume reusable by Rust HTTP downloader. |
| Search caches | `<uuid>.spellingsdb.v0.1`, temporary `.tmp` cleanup | Regenerable cache, not user data. |
| StreetZim chunks | hash-and-length files in `StreetZimChunks` | Reusable only after hash verification; safe to evict. |
| Preferences | Defaults/UserDefaults, including `libraryLanguageCodesSorted`, snippets, zoom, selected hotspot IDs | Migration must preserve actual stored names, including typo `slectedHotspotIds`; do not silently “correct” input key names. |

Evidence: `Database.swift:20-81`; complete data model; `DefaultKeys.swift:20-80`; `SessionRestore.swift`; `WindowState.swift`; `Model/Utilities/URL.swift`; cookie persistence above.

Additional migrations:

- `Migrations.all()` runs language codes → URL scheme → tags, with per-migration UserDefaults checkpoints.
- Scheme migration updates persisted bookmarks; restored legacy tab URLs migrate on demand.
- Branded apps with bundled ZIM updates reassign old bookmarks/tabs to newest archive and rewrite host IDs. This is **branded-app-specific**, not evidence of automatic version migration for arbitrary downloaded library archives.
- That old tab-state host rewrite decodes opaque bytes as UTF-8 and regex-replaces legacy URLs; do not reuse as a robust cross-platform session strategy.
- Sources: `Model/Migration/Migrations.swift:18-28`; `MigrationService.swift:25-62`; `MigrateSchemeToZIM.swift:20-40`; `SwiftUI/Model/ZimMigration.swift:23-126`; `Tests/BookmarkMigrationTests.swift`.

## Apple versus current Qt: actual differences

**Clearly additional Apple application code:**

- StreetZim catalog/recipe composition, geographic browser and verified reuse.
- Custom-scheme cookie compatibility/persistence.
- Archive-scoped camera prompt policy and native geolocation bridge.
- macOS session restoration of multiple windows, geometry, groups and active selections.
- Native sharing surfaces and platform background-download integration.
- Optional spelling correction code, **disabled by default**.

For Qt absence conclusions, broad searches were followed by alternate terminology checks. Treat “no corresponding app implementation found” as a source-audit finding, not a statement that Chromium intrinsically cannot provide camera/cookies/location.

**Already in Qt; avoid false differentiation:**

- TOC: `src/tableofcontentbar.cpp:35-100`; `src/kiwixapp.cpp:462-463`.
- PDF/content export: `src/webview.cpp:189-208`.
- Printing: `src/kiwixapp.cpp:244-265`.
- Per-tab back/forward history menus: `src/webview.cpp:129-160,241-262`.
- Session reopen: `src/kiwixapp.cpp:183-203`, `kiwix-desktop.session`; less rich than Apple window-group restoration.
- Bookmarks/read list and XML interchange: `src/library.cpp:35-36,198-237`; `src/readinglistbar.cpp:108-136`.
- System/light/dark theme support in newer Qt: `src/kiwixapp.cpp:509-523`; `src/settingsview.cpp:63-66,95-97`.
- Qt also has conditional text-to-speech actions (`src/mainmenu.cpp:39+`, `src/kiwixapp.h:51+`) and download backends including conditional libtorrent (`src/downloadmanagement.cpp`). No Apple reader TTS application implementation found.

**Important asymmetry:** Qt has explicit reading-list XML import/export; Apple has no corresponding bookmark interchange UI found. A superset must preserve Qt interchange even though Apple cannot supply that requirement.

## Platform separation

**Desktop-applicable:** reader, media, TOC, bookmarks/search, library/catalog/downloads, missing-file recovery, integrity, PDF/raw-content export, printing, HTTP serving, map composition, cookie persistence, camera/location permission concepts, themes, keyboard/accessibility, diagnostics.

**Apple-platform implementations to adapt, not literal cross-platform requirements:**

- Security-scoped file bookmarks, Finder reveal, NSWindow tab groups/NSSharingServicePicker, macOS print APIs.
- CoreLocation/WKWebView permission and cookie bridges.
- Apple Pay/Stripe donation/signing configuration.

**Mobile-only integrations:**

- iPhone compact and iPad split-navigation layouts, toolbar hiding on scroll.
- iOS Files/Open-In/document sharing, activity sheet/photo-library write permission.
- Cellular-data preference, mobile notification lifecycle/background URLSession callbacks.
- Memory-warning cache purge, iOS idle-lock suppression during maps/server use.
- Optional iCloud **backup inclusion of ZIM documents**. This is not iCloud library/bookmark synchronization.

## Recent local work and validation evidence boundaries

`CHANGELOG.md:1-29` records 3.18.0 map/camera changes, and the corresponding implementations are present. Current source includes:

- Grouped/searchable area browser and explicit classification snapshots.
- Local-language/script/region-aware selection.
- Saved maps surviving navigation/language changes/relaunch.
- Current-part incoming bytes and stale-progress isolation on cancellation.
- Off-main-actor recipe validation and indexed masks.
- Owned-context import rollback and damaged-map rebuild without overwrite.
- Camera consent policy and required project metadata.

Relevant checked-in tests, **not executed here**:

- `Tests/StreetZimTests.swift`
- `Tests/StreetZimHTTPTests.swift`
- `Tests/StreetZimViewModelTests.swift`
- `Tests/StreetZimAreaTests.swift`
- `Tests/DownloadedMapsTests.swift`
- `Tests/ZIMCameraPermissionTests.swift`
- `Tests/TableOfContentsTests.swift`
- `Tests/ZimCookieStoreTest.swift`
- `Tests/BookmarkMigrationTests.swift`
- `Tests/URLContentPathTests.swift`
- `UITests_common/SavedMapsUITests.swift`
- `UITests_macOS/AreaPickerUITests.swift`
- Real tiny offline fixture: `Tests/Fixtures/streetzim-demo.zim`.

The local beta documentation is useful for recent-work context but is not independent verification. It explicitly distinguishes implementation/unit-policy checks from unverified physical camera capture and records private release-overlay work; private distribution settings are not product capabilities.

### Compressed acceptance requirements for the Rust/Tauri superset

- Preserve ordinary Qt features **and** add native StreetZim composition with exact v1/v2 protocols above.
- Make reader origin/archive identity a first-class boundary for URLs, cookies and device permissions.
- Keep map composition separate from ordinary downloads while exposing unified progress/saved content.
- Commit imported library state before announcing success; preserve unrelated edits and files on failure.
- Use portable explicit session/history/bookmark models; do not depend on WebKit blobs or security-scope tokens.
- Include explicit bookmark interchange from Qt; notes/global history/cloud sync are new scope, not Apple parity.
- Treat default-disabled spell correction, hidden download-location UI, debug map route and physical camera acceptance honestly.
- Carry keyboard/screen-reader behavior as requirements rather than copying visual UI alone.