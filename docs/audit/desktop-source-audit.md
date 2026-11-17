## Summary

The Qt app implements substantial desktop-reader functionality: local/online libraries, persisted filters, resumable acquisition, ZIM browsing, title/full-text search, tabs, bookmarks with XML import/export, generated table of contents, optional text-to-speech, PDF export/printing, and an embedded LAN server.

**Important nonfeatures:** “MultiZim” is actually a **single-book radio selector**; reopen-closed-tab and recent-files actions are hidden; browsing history is per-tab and not restored; service-worker archives are explicitly described as unsupported; robust download-error recovery is incomplete.

**Artifact blocker:** I could not create `local://desktop-feature-audit.md`: this worker has only read/search tools and an explicit read-only constraint. The complete artifact-ready report follows. No application files were modified; no builds, tests, formatters, or runtime checks were run.

# Desktop feature audit

### Evidence/status conventions

- **Implemented** means a concrete source implementation exists—not runtime-tested.
- **Conditional** means dependent on Qt version, optional module, or operating system.
- **Partial** means a real feature has significant implementation gaps.
- **Absent** means explicitly hidden/unimplemented, or no implementation was found in the audited source/UI after alternative searches.
- **Major/minor** describes parity importance, not implementation effort.
- Potential defects derived from code are labeled **[INFERENCE]**.
- Current source takes precedence over historical ChangeLog claims.

## Architecture and principal files

| Files | Responsibility |
|---|---|
| `src/main.cpp:36-111` | Custom URL-scheme registration, command-line opening, single-instance forwarding, startup ordering. |
| `src/kiwixapp.cpp:36-204,213-305,320-643` | Application composition, actions, session persistence, printing, random articles, portable/legacy migration. |
| `src/library.cpp:12-252` | Wrapper over libkiwix library, ZIM access, XML persistence, bookmarks, temporary download paths. |
| `src/contentmanager.cpp:57-140,142-850,856-1075` | Local/remote library controller, book states, download integration, deletion, filtering, directory watching. |
| `src/opdsrequestmanager.cpp:8-113` | Catalog endpoint configuration, OPDS HTTP requests, response forwarding. |
| `src/contentmanagerside.cpp:11-133`; `src/kiwixchoicebox.cpp:16-282` | Sidebar filters and searchable multiselect controls. |
| `src/contentmanagermodel.cpp:90-277`; `src/contentmanagerdelegate.cpp:127-286`; `src/contentmanagerview.cpp:8-86` | Library rows/descriptions, sorting, progress and acquisition controls. |
| `src/downloadmanagement.cpp:56-547` | Download state machine, serialized worker requests, storage checks, aria2/libkiwix and optional libtorrent integration. |
| `src/urlschemehandler.cpp:21-295` | Archive entry resolution, MIME responses, redirects, full-text result HTML, missing/corrupt archive pages. |
| `src/kprofile.cpp:16-172`; `src/webpage.cpp:9-28` | Shared browser profile, injected scripts, external-request blocking, external navigation, attachment saving/opening. |
| `src/webview.cpp:61-420`; `src/zimview.cpp:11-175` | Reader behavior, back/forward menus, export, zoom, context menus, speech integration. |
| `src/tabbar.cpp:26-399,575-613` | Tabs, book home, settings tab, tab restoration serialization, navigation state and fullscreen content. |
| `src/searchbar.cpp:16-443`; `src/suggestionlistworker.cpp:5-65`; `src/multizimbutton.cpp:12-129` | Bookmark button, asynchronous title suggestions, full-text entry, single-book search chooser. |
| `src/readinglistbar.cpp:13-149` | Bookmark sidebar and XML import/export. |
| `resources/js/headerAnchor.js:1-59`; `src/tableofcontentbar.cpp:8-108` | HTML heading extraction, synthetic anchors, TOC tree and navigation. |
| `src/texttospeechbar.cpp:5-294` | Optional platform speech engine, voice/language/speed controls. |
| `src/settingsmanager.cpp:10-236`; `src/settingsview.cpp:32-281`; `ui/settings.ui:109-129` | INI settings schema and preferences controls. |
| `src/localkiwixserver.cpp:9-146` | Embedded server start/stop, IP/interface and port selection, browser access. |
| `src/mainwindow.cpp:13-235`; `src/mainmenu.cpp:9-80`; `src/topwidget.cpp:14-157` | Window, sidebars, menus, toolbar, fullscreen behavior. |
| `resources/org.kiwix.desktop`; `resources/org.kiwix.desktop-mime.xml` | Linux desktop/file associations. |
| `src/translation.cpp:12-58`; `resources/i18n/`; `resources/texts/about.html` | Localization with English fallback, privacy/about/source links. |
| `ChangeLog:1-129,190-388` | Historical feature introductions and regressions; corroboration, not proof of current behavior. |

**Connections:** `KiwixApp` owns a shared libkiwix-backed `Library`, `ContentManager`, browser profile, server and settings. `ContentManager` subclasses `DownloadManager`; OPDS populates a separate in-memory remote library. Tabs contain `ZimView`/`WebView`, resolving `zim://` URLs through the shared scheme handler into libzim archives. A shared Qt action registry connects menus, toolbar, keyboard commands and active-tab behavior. Bookmarks and local metadata share the libkiwix persistence layer.

## 1. Library, catalog and filters

| Priority | Status | Parity behavior and evidence |
|---|---|---|
| Major | Implemented | Separate **Local files** and **Online files** modes. Startup selects local if the library contains any IDs, otherwise online. `KiwixApp::init`, `src/kiwixapp.cpp:82-90`; `ContentManager::setLocal`; `src/contentmanagerside.cpp:19-34`. |
| Major | Implemented | Local and remote book listings support query text, language, category, and content-tag filters. Local listing requests `local(true)` and `valid(true)`; online requests `remote(true)`. `ContentManager::getBookIds`, `src/contentmanager.cpp:808-838`. |
| Major | Implemented | Multiselect languages/categories; searchable choices, removable selection chips, clear controls and persisted values. Local choices derive from local books; remote choices come from catalog feeds. Categories are translated through `kiwix::translateBookCategory`. `src/contentmanagerside.cpp:35-133`; `ContentManager::setCategories/setLanguages`; `src/kiwixchoicebox.cpp:16-282`. |
| Major | Implemented | Content options: pictures/no pictures, videos/no videos, full article/introduction only, mapped to `_pictures`, `_videos`, `_details` yes/no tags. These are selectable tag choices, not a full-text-index filter. `src/contentmanagerside.cpp:54-63`; `ContentManager::getBookIds`. |
| Minor | Implemented | Language codes of length two are converted to ISO-639-3 when possible before library/catalog filtering. `ContentManager::setCurrentLanguage`, `src/contentmanager.cpp:717-741`. |
| Major | Implemented | Metadata rows show illustration, title, publication date, human-readable size and content attributes. Clicking a non-action cell toggles expanded description; double-click opens healthy local books. `ContentManagerModel::createNode`; `ContentManagerView::onClicked`; `ContentManager::openBookWithIndex`. |
| Minor | Implemented | Sort ascending/descending by title, date or size. Illustration/content/action columns do not sort. `src/contentmanagermodel.cpp:219-237`. |
| Minor | Implemented | Local illustrations read from archive metadata; online illustrations downloaded asynchronously and cached in memory. Missing/error illustration paths fall back appropriately. `ContentManager::getBookInfos` helpers, `src/contentmanager.cpp:302-372`; `src/contentmanagermodel.cpp:173-187,240-250`; `Library::getBookIcon`. |
| Major | Implemented | Online preview opens the catalog’s `/viewer#<zim-name>` URL in the default external browser. Name is derived from acquisition URL filename. `ContentManager::openBookPreview`, `src/contentmanager.cpp:452-470`. |
| Major | Implemented | Context menus depend on state: open/delete/open folder for local books; download/preview for online; pause/resume/cancel/preview for downloads. `src/contentmanager.cpp:142-234`. |
| Minor | Implemented | Opening containing folder checks existence/readability and displays an error on failure. `openFileLocation`, `src/contentmanager.cpp:40-53`. |

### Catalog protocol and limits

`OpdsRequestManager` uses:

- Default host `opds.library.kiwix.org`, port `443`.
- Environment overrides `KIWIX_CATALOG_HOST`, `KIWIX_CATALOG_PORT`.
- HTTPS only when configured port is exactly 443; otherwise HTTP.
- `/catalog/v2/entries`, with `lang`, `category`, `count=-1`.
- `/catalog/v2/languages` and `/catalog/v2/categories`.
- Redirect policy `NoLessSafeRedirectPolicy`.
- No paginated fetch: request all entries for the selected server-side filters.
- Query text and content-tag filtering occur against the fetched library locally.

Evidence: `src/opdsrequestmanager.cpp:8-87`; `ContentManager::updateLibrary/updateRemoteLibrary/getBookIds`.

**Failure modes/limitations:**

- **Partial:** network replies have no distinct UI error pathway. `replyContent` discards only canceled responses; other HTTP/network failures are forwarded as content. `ContentManager::updateLibrary` catches `runtime_error` without feedback. `src/opdsrequestmanager.cpp:90-113`; `src/contentmanager.cpp:761-785`.
- **[INFERENCE]** Overlapping requests can replace newer remote results with older results: responses have no generation token or cancellation mechanism, unlike reader suggestions.
- **Partial:** `doUpdate` comments that service-worker archives are unsupported and constructs `excludeTags("_sw:yes")`, but never adds that value to the query. Do not claim current source reliably filters these archives. `src/opdsrequestmanager.cpp:28-45`.
- **[INFERENCE]** Missing archives may be hidden from the local list by `valid(true)` even though context-menu state handling includes missing-file errors.
- Search text/sort/mode persistence is not present in `SettingsManager`; language/category/content choices are persisted.

## 2. Acquisition and download lifecycle

| Priority | Status | Parity behavior and evidence |
|---|---|---|
| Major | Implemented | Acquire selected catalog books through libkiwix `Downloader`, with optional libtorrent path. `DownloadManager::startDownload`, `src/downloadmanagement.cpp:391-431`. |
| Major | Implemented | Preflight validates download backend, directory existence, writability, free bytes and filesystem maximum file size. FAT32/vfat limit is 4 GiB−1. Unknown free-space result is treated as a storage error. `src/downloadmanagement.cpp:324-389`. |
| Major | Implemented | Repeats preflight in the worker because the download directory may change while queued behind slow storage/backend operations. `ContentManager::downloadBook/startDownload`, `src/contentmanager.cpp:535-588`. |
| Major | Implemented | Asynchronous serialized start/pause/resume/cancel/update requests. Polls once a second when queue is empty. Requested/transitional states suppress stale status responses. `src/downloadmanagement.cpp:67-122,171-207,434-447`. |
| Major | Implemented | Progress percentage, completed size and transfer rate. Rate becomes `"---"` when no update for over two seconds. `DownloadState::update/getDownloadSpeed`, `src/downloadmanagement.cpp:76-100`; delegate progress drawing. |
| Major | Implemented | Pause/resume/cancel; cancel requires confirmation and permanently removes partial data, independent of trash preference. Active downloads offer cancellation through context menu; inline cancel is available when paused. `src/contentmanager.cpp:683-715`; `src/contentmanagerdelegate.cpp:250-280`. |
| Major | Implemented | Tracks unfinished books in local XML using backend download ID and a pseudo-path ending `.beingdownloadedbykiwix`. Actual output path replaces initial URL-derived filename as discovered. `src/library.cpp:124-200`. |
| Major | Implemented | Restores in-progress records from library entries with nonempty download IDs. Optional libtorrent restoration forces recheck and pauses. `src/downloadmanagement.cpp:209-237`. |
| Major | Implemented | On completion: mark path valid, clear download ID, replace pseudo-path with real native path, remove direct acquisition URL, persist library, refresh book/bookmark UI. `ContentManager::downloadCompleted`, `src/contentmanager.cpp:497-512`. |
| Major | Implemented | Race protection avoids deleting already-completed files when delayed pause/cancel confirmation or backend requests arrive after completion. `src/downloadmanagement.cpp:449-538`. |
| Major | Conditional | libtorrent compilation path attempts torrent first, falls back to libkiwix on exception; IDs prefixed `lt:`. `src/downloadmanagement.cpp:209-226,391-416`; `kiwix-desktop.pro:240-250`. |
| Major | Partial | Backend launch failure shows a critical dialog and leaves downloader unavailable; attempted downloads then show “download unavailable.” `createDownloader`, `throwDownloadUnavailableError`, `checkThatBookCanBeDownloaded`. |

### Acquisition failure/recovery gaps

- `DownloadState` includes waiting/error/unknown and transitional states, but `ContentManager::getBookState` maps every tracked state except exact `PAUSED` to `DOWNLOADING`; source explicitly notes missing error handling. `src/contentmanager.cpp:396-406`.
- **[INFERENCE]** An errored/unknown download can become nonactionable: context menu and inline controls require exact active/paused states. No explicit retry action is implemented.
- Worker start failure emits an error but does not explicitly undo the provisional book/download state. `src/contentmanager.cpp:557-588`.
- If fetching backend state throws, it emits `downloadDisappeared`; handler clears download ID and saves, rather than a dedicated recovery dialogue. `src/downloadmanagement.cpp:239-254`; `src/contentmanager.cpp:474-494`.
- Initial output filename is guessed from acquisition URL, explicitly unsuitable for torrent/metalink URLs until backend supplies the real path. `src/library.cpp:153-168`.
- No user-facing scheduler, bandwidth setting, retry policy, checksum/repair workflow or dedicated download-history view was found.

## 3. Import, monitoring and deletion

| Priority | Status | Parity behavior and evidence |
|---|---|---|
| Major | Implemented | File dialog opens `.zim` or first split part `.zimaa`; imports metadata into library without copying the selected archive. Existing registered path reuses its ID. `KiwixApp::openZimFile`, `src/kiwixapp.cpp:213-245`; `Library::openBookFromPath`, `src/library.cpp:46-60`. |
| Major | Implemented | Multiple positional file arguments at launch; second instance forwards each to the existing instance, which opens it. `src/main.cpp:76-110`; `src/kiwixapp.cpp:113-118`. |
| Major | Implemented | Automatically monitors configured download directory plus one optional monitor directory. Deduplicates those paths. `KiwixApp::setupDirectoryMonitoring`, `src/kiwixapp.cpp:124-130`. |
| Major | Implemented | Initial scan and filesystem-watch updates; detects new and removed files, persists changed library. Skips files recognized as own active downloads. `src/contentmanager.cpp:856-1075`. |
| Minor | Implemented | Tracks bad archive modification times; avoids unchanged bad-file rechecks; waits/debounces about one second for updated unstable files before retrying import. `src/contentmanager.cpp:954-1066`. |
| Major | Implemented | Missing watched books removed unless still open in tabs. Startup restores tabs before directory monitoring to preserve knowledge of open books. `ContentManager::handleDisappearedBook`; `src/main.cpp:101-104`. |
| Major | Implemented | Delete asks confirmation; closes matching reader tabs; deletes archive files; removes library entry and per-book settings; persists and refreshes bookmarks. Trash versus permanent deletion is configurable. `src/contentmanager.cpp:614-680`. |

**Unusual archive/file behavior:**

- Watched scans enumerate **only direct `*.zim` children**, not recursion or `.zimaa`, unlike manual import. `ContentManager::updateLibraryFromDir`, `src/contentmanager.cpp:1023-1045`.
- Deletion enumerates `<archive filename>*`, accommodating split/sidecar files but also matching unrelated files with the same prefix. It guards the dangerous empty filename → `"*"` case. `eraseBookFilesFromComputer`, `src/contentmanager.cpp:614-642`.
- **[INFERENCE]** `closeTabsByZimId` iterates forward while deleting tabs; adjacent tabs for one book can be skipped. `src/tabbar.cpp:278-290`.
- Direct open failure warns with path. Library open failure additionally removes the library record and returns to library. `src/kiwixapp.cpp:232-243`; `ContentManager::openBook`, `src/contentmanager.cpp:435-450`.
- No explicit recursive/bulk import wizard, drag-and-drop handlers, or export-library/archive-copy feature was found.

## 4. Reader, archive rendering and navigation

| Priority | Status | Parity behavior and evidence |
|---|---|---|
| Major | Implemented | Chromium/Qt WebEngine renders archive HTML/resources through custom `zim://` scheme and actual item MIME type, stripping MIME parameters. `UrlSchemeHandler::handleContentRequest`; `src/kprofile.cpp:55-71`. |
| Major | Implemented | Article URLs use `zim://<book-id>.zim/<entry-path>`. Empty/root path resolves archive main entry; archive redirects produce browser redirects to resolved item path. `src/urlschemehandler.cpp:21-71`. |
| Major | Implemented | Back/forward, history dropdowns listing titles and direct history-item navigation, home, random article. Mouse back/forward buttons supported. `src/webview.cpp:139-176,254-269,385-408`; `TabBar::openHomePage`; `KiwixApp::openRandomUrl`. |
| Major | Implemented | Local links open in reader/new tab; external navigations hand off to default system handler and are rejected inside reader. External-link context menu supports copy link and open browser. `src/webpage.cpp:20-28`; `src/webview.cpp:345-379`. |
| Major | Implemented | Blocks all embedded requests not beginning `zim://`, preventing archive pages from silently loading external resources. `ExternalReqInterceptor::interceptRequest`, `src/kprofile.cpp:164-172`. |
| Major | Conditional | Qt ≥6.6 scheme enables secure/local access, CORS and Fetch/XHR. Important for JavaScript-driven archives. `src/main.cpp:56-69`. |
| Major | Implemented | Generated TOC from HTML heading elements. Injects `kiwix-toc-N` anchors; displays numbered hierarchy; selects by click/keyboard; scrolls matching current document without navigation. `resources/js/headerAnchor.js`; `src/tableofcontentbar.cpp:34-108`. |
| Major | Implemented | Global default zoom and persisted per-book overrides; changes in 0.1 increments, clamped 25–500%; reset removes per-book override. Ctrl-wheel controls zoom. `src/zimview.cpp:36-85`; `WebView::wheelEvent`; `SettingsManager::getZoomFactorByZimId`. |
| Major | Implemented | Window fullscreen hides tabs/toolbar, reveals at top-edge pointer movement; video/page fullscreen uses separate fullscreen window and Escape exit. `src/mainwindow.cpp:99-163`; `TabBar::fullScreenRequested`; `src/fullscreenwindow.cpp:15-18`. |
| Major | Implemented | Missing archive page includes book ID/name/path; corrupt archive page also identifies entry path. Missing entry gets URL-not-found failure. `src/urlschemehandler.cpp:214-277,41-75`. |
| Minor | Implemented | Hovered link tooltip, archive icon and article title in tabs/window; title search field reflects active article when unfocused. `src/zimview.cpp:105-130`; `TabBar::setTitleOf`; `MainWindow` title connection. |

### Save, print and attachments

- **Major / implemented:** Save page saves HTML articles as **PDF**, not HTML/complete webpage. Suggested filename is slugified archive item title. Non-HTML items go through browser download. `WebView::saveViewContent`, `src/webview.cpp:207-225`.
- **Major / implemented:** Print uses system print dialog and reports unsuccessful print completion. `KiwixApp::printPage`, `src/kiwixapp.cpp:247-278`.
- **Major / implemented:** Archive attachments—including PDFs/EPUBs—offer **Save** or **Open**. Open extracts into a temporary file and invokes external application; this is not a native EPUB reader. `src/kprofile.cpp:101-162`; `ChangeLog` rc3 attachment feature.
- Temporary opened files use `setAutoRemove(false)`; no cleanup mechanism is shown. Download-completion UI is connected to completion without explicit success-state checks. `src/kprofile.cpp:101-143`.
- PDF/attachment export failures are not uniformly surfaced: `saveViewContent` catches everything silently.
- Save dialog remembers directory; initial fallback is Documents, with fallback again if previous directory no longer exists. `askForSaveFilePath`; `KiwixApp::getPrevSaveDir`.
- **Partial/unsupported:** service-worker archives should not be included in baseline “works” claims. Regular JavaScript/CSS/media via archive requests is implemented, but codec availability and web-engine differences were not exercised.

## 5. Reader searches

| Priority | Status | Behavior and evidence |
|---|---|---|
| Major | Implemented | Title suggestions from `zim::SuggestionSearcher`, background worker, 100 ms typing debounce, token rejection of outdated results. `src/searchbar.cpp:126-139,237-250,341-366`; `src/suggestionlistworker.cpp:13-41`. |
| Major | Implemented | Incremental suggestion pagination/endless scrolling; keyboard Down/PageDown can request more. `SearchBarLineEdit::onScroll/fetchMoreSuggestions/eventFilter`. |
| Major | Implemented | Append full-text search entry only if selected archive has full-text index. `src/suggestionlistworker.cpp:43-56`. |
| Major | Implemented | Enter chooses exact case-insensitive title match first, otherwise full-text option if available. It does **not** always open first suggestion. Enter before suggestions finish is handled on initial results. `src/searchbar.cpp:146-154,323-332,368-385`. |
| Major | Implemented | Full-text results use libzim Searcher and libkiwix HTML renderer, with `pattern`, `content`, `start`, `pageLength`; default 25 results/page and estimated match count. `src/urlschemehandler.cpp:126-191`. |
| Major | Partial | “MultiZim” chooser lists available local archives, current book first, but uses exclusive radio buttons and worker uses only selected ID zero. **Not simultaneous cross-book search.** `src/multizimbutton.cpp:12-129`; `src/suggestionlistworker.cpp:20-23`. |
| Major | Implemented | Find in page has next/previous and Enter next; Escape closes, clears highlight and returns focus to document to preserve position. `src/findinpagebar.cpp:8-77`. |
| Minor | Absent | No match count, case-sensitivity control or regex option in find-in-page implementation. |

Full-text failure maps to invalid-URL/request-failed browser errors rather than a dedicated recovery UI. `src/urlschemehandler.cpp:151-169`.

## 6. Tabs, bookmarks, history and session

| Priority | Status | Behavior and evidence |
|---|---|---|
| Major | Implemented | Multiple movable reader tabs, singleton settings tab, fixed nonclosable library tab, plus new-tab button. Protected first/last positions; close by button, shortcut or middle-click. `src/tabbar.cpp:26-199,316-337,420-426,575-603`. |
| Major | Implemented | Next/previous tab cycling; overflow scroll buttons; Alt+digits direct selection. Tab titles fade visually and remain in tooltips. `src/tabbar.cpp:49-59,135-153,203-224,429-515`; `MainWindow::updateTabButtons`. |
| Major | Implemented | Toggle bookmark by book ID + article path, recording title. Immediate save for star additions/removals. `src/searchbar.cpp:16-56`; `KiwixApp::isCurrentArticleBookmarked`. |
| Major | Implemented | Bookmark sidebar (“Reading list”), excludes archives that cannot open. Single click opens current tab after double-click delay; double/middle click and keyboard activation open new tab. `src/readinglistbar.cpp:53-106`. |
| Major | Implemented | Import/export libkiwix bookmark XML; suggested export `Documents/kiwix_readinglist.xml`; error messages when serializer/parser returns false. `src/readinglistbar.cpp:108-137`; `Library::readBookMarksFile`. |
| Major | Implemented | Optional restart restoration of tab URLs, blank tabs and `"SettingsTab"` sentinel; restores current tab index. Disabled by default. URLs persisted during navigation and tab lifecycle. `src/kiwixapp.cpp:183-204,591-623`; `TabBar::getTabUrls`; `WebView::onUrlChanged`. |
| Major | Implemented | Window geometry/state persisted on close/restored at startup; bookmark/TOC sidebar selection retained while switching among reader tabs, hidden in settings and replaced by library filters in library tab. `src/mainwindow.cpp:164-168,183-235`; session save/restore. |
| Major | Partial | Browser history exists per open WebEngine tab, but session serialization stores URLs only—not history stacks, reading scroll positions or find state. `WebView::getHistoryBackMenu`; `TabBar::getTabUrls`; `KiwixApp::restoreTabs`. |
| Minor | Absent | Reopen closed tab and recent files are declared but explicitly hidden, with no implementation hook found. `src/kiwixapp.cpp:425-435`. |
| Major | Absent | Bookmark folders/tags/notes, editing titles, cloud sync and persistent global history are not implemented in audited UI/controller code. |

**Compatibility details:**

- `TabBar::currentArticleUrl()` returns `.path()`: bookmark identity excludes query and fragment. `src/tabbar.cpp:244-249`.
- Bookmark XML remains stored when inaccessible archive bookmarks are hidden; sidebar visibility is not equivalent to deletion.
- Import reads into existing libkiwix library; exact duplicate/ID reassociation semantics belong to libkiwix, not implemented here.
- Import emits bookmark updates but does not immediately call `Library::save`; normal library save/destruction subsequently persists it. `src/library.cpp:239-248`.
- Back/forward history is not a “recent articles” feature.

## 7. Sharing/server

| Priority | Status | Behavior and evidence |
|---|---|---|
| Major | Implemented | Embedded libkiwix HTTP server serves same library; no separate kiwix-serve executable needed. `KiwixApp` server construction; `ChangeLog:229-236`. |
| Major | Implemented | Start/stop from dialog; persisted port, IPv4-only/IPv6-only/all-IPs or selected interface address. Interface list excludes IPv4 link-local addresses beginning `169.254`. `src/localkiwixserver.cpp:19-72,87-145`. |
| Major | Implemented | Valid port 1–65535; default 8080. Server-start failure shows critical message. Displays first server access URL and opens browser. `LocalKiwixServer::runOrStopServer/openInBrowser`; `SettingsManager::initSettings`. |
| Major | Implemented | Server naming follows library updates via `UpdatableNameMapper`; stops when application shuts down. `src/kiwixapp.cpp:42-43,134,505-506,565-568`. |
| Major | Absent | No authentication, HTTPS configuration, per-book sharing selection, QR code or remote-device pairing controls found. |
| Minor | Implemented | Closing dialog does not call stop; application owns server lifecycle. `src/localkiwixserver.cpp:23`; `KiwixApp::~KiwixApp`. |

This is LAN/library hosting—not the same as copying a public article URL or OS-native share-sheet integration.

## 8. Preferences, shortcuts and accessibility

### Preferences

- **Major / implemented:** default zoom, download directory/reset, monitor directory/clear, trash preference, restore-tabs preference. Changes apply immediately; directory changes use confirmations. `src/settingsview.cpp:32-218`.
- **Minor / implemented:** copy full directory path, tooltip for elided path. `src/settingsview.cpp:220-250`.
- **Major / conditional:** System/Light/Dark enum values 0/1/2. UI and application color-scheme behavior are enabled only on Qt ≥6.8, with reader reload on theme change. No archive stylesheet rewriting is present. `src/settingsmanager.h:18-23`; `src/settingsview.cpp:100-104`; `src/kiwixapp.cpp:509-524`.
- Preferences zoom control is 30–500%, while reader actions reach 25%; preserve intention, not accidental inconsistency. `ui/settings.ui:119-123`; `src/zimview.cpp:36-60`.
- **Major / conditional:** portable mode disables download/monitor directory controls and fixes both to portable data directory. `src/settingsmanager.cpp:192-198`; `src/settingsview.cpp:88-93`.
- No explicit app-language chooser, proxy settings, custom catalog UI, font controls, privacy/history controls or update preferences were found.

### Keyboard parity

Evidence: `src/kiwixapp.cpp:374-499`; `src/tabbar.cpp:49-59`.

| Action | Existing binding |
|---|---|
| Open / Print / New tab / Save / Quit | Qt platform standard sequences |
| Close tab | Ctrl+W, Ctrl+F4 |
| Search focus | F6, Ctrl+L, Alt+D |
| Find in page | Standard Find, F3 |
| History | Alt+Left / Alt+Right; reversed for RTL |
| Archive home | Alt+Home |
| Random | Ctrl+R |
| Book search chooser | Ctrl+Shift+L |
| Bookmark | Ctrl+D |
| Reading list | Ctrl+B |
| TOC | Ctrl+M |
| Server | Ctrl+I |
| Settings | F12 |
| Zoom | Standard zoom-in/out, Ctrl+=, Ctrl+0 reset |
| Direct tab selection | Alt+1…9, Alt+0 selects tenth |
| TTS article / selection / stop | Alt+Shift+A / S / X |
| TTS language / voice | Alt+Shift+L / V |
| TTS speed | `>` / `<` |
| Donate | Ctrl+Shift+D |
| Fullscreen | Platform FullScreen; F11 fallback |

**[INFERENCE] Shortcut defect:** `createActions` creates `NextTabAction` twice and `PreviousTabAction` twice; the second assignment replaces the first. Effective LTR bindings are Ctrl+PageDown/PageUp, not both those and Ctrl+Tab/Ctrl+Shift+Tab. RTL swaps direction. The ChangeLog claims both families over time, but source does not preserve both. `src/kiwixapp.cpp:389-401`.

### Accessibility/localization

- **Major / implemented:** OS-locale translation, JSON message dictionaries, English fallback for missing/empty strings, separate Qt translations. No manual runtime language setting. `src/translation.cpp:12-58`; `src/kiwixapp.cpp:50-69`.
- **Major / implemented:** RTL-aware icons/history direction, search alignment, mixed-direction titles/chooser text. `KiwixApp::createActions`; `SearchBarLineEdit`; `TabBar::paintEvent`; `ZimItemWidget`.
- **Major / conditional:** TTS via optional Qt TextToSpeech module. Read entire extracted plain text or selection, stop/close, platform-provided locales/voices, remembered voice and speed per language. Speeds 0.25–2.0; changing voice/speed can restart speech from beginning. `src/zimview.cpp:131-175`; `src/texttospeechbar.cpp:5-263`; `kiwix-desktop.pro:12`.
- No pause/resume speech, word-follow highlighting or saved speech position is implemented.
- Keyboard filter navigation supports Up/Down/Enter/Escape; TOC supports activation key; find/speech Escape closes and restores focus.
- `ChangeLog:220` reports screen-reader improvements for Search/Main Menu. Current source supplies tooltips, but no explicit `QAccessible`/accessible-name assignments were found. Custom painted tab titles and painted download controls merit accessibility acceptance testing; do not claim audited screen-reader completeness.
- High-DPI enablement exists for Qt5; Qt6 uses defaults. `src/main.cpp:51-54`.

## 9. Persistence and migration contract

| Data | Existing location/format | Important fields/behavior |
|---|---|---|
| Local library | `<data>/library.xml`, libkiwix XML | Book identity/path/metadata, acquisition URL, download ID. Read through Manager; written by `writeAsXML`. |
| Bookmarks | `<data>/library.bookmarks.xml`, libkiwix bookmark XML | Book ID, article path, title; imports/exports same library serializer format. |
| Preferences | `<config>/Kiwix-desktop.conf`, **QSettings INI** | Exact capitalization matters. |
| Session | `<data>/kiwix-desktop.session`, **QSettings INI** | URLs, active tab, window state, dialogs, speech preferences. |
| Download backend | Data directory passed to libkiwix Downloader | Legacy migration references `kiwix.session`, `.aria2`, `.meta4`; exact backend serialization is delegated to libkiwix/aria2. |
| Portable marker | `.portable` next to executable | Changes data/config/download/monitor roots. |

Evidence: `src/library.cpp:30-40,221-225`; `src/settingsmanager.cpp:10-48`; `src/kiwixapp.cpp:46-49,151-180`; `createDownloader`.

### Exact preferences to import

`src/settingsmanager.cpp:94-236`:

- `download/dir`: data directory default.
- `monitor/dir`: empty default.
- `localKiwixServer/port`: 8080.
- `localKiwixServer/ipAddress`: `"0.0.0.0"` default; UI normalizes to IPv4 mode; can become `"ipv4"`, `"ipv6"`, `"all_ips"` or literal address.
- `view/zoomFactor`: 1.0.
- `<book-id>/zoomFactor`: per-book override.
- `moveToTrash`: true.
- `reopenTab`: false.
- `theme`: 0=System, 1=Light, 2=Dark.
- `language`: QVariant list of `"display-name|code"` strings, initial OS language.
- `category`: string list of category identifiers.
- `contentType`: QVariant list of `"display-name|tag"` strings.

Old categories containing `|` are discarded by existing migration. Pair deserialization assumes both fields exist; the Rust importer should parse defensively rather than inherit malformed-config crashes.

### Exact session keys

`src/kiwixapp.cpp:183-204,591-643`:

- `reopenTabList`: QStringList of complete reader URLs, empty URLs and `"SettingsTab"`.
- `currentTabIndex`.
- `geometry`, `windowState`: Qt serialized byte arrays.
- `zim-import-dir`.
- `prevSaveDir`.
- `voice/<locale-name>`.
- `speed/<locale-name>`; default 1.0.

### Root discovery and existing migration

`src/settingsmanager.cpp:10-41`; `src/kiwixapp.cpp:151-180`:

- Normal data root: Qt `AppDataLocation`; create if possible, otherwise current directory.
- Normal config root: Qt `AppConfigLocation`.
- Portable data/config root: `<executable-directory>/data`.
- Portable detection: `<executable-directory>/.portable`.
- Legacy migration checks parent of new data directory for `library.xml`, moves `*.zim` and `library*.xml`, then removes `.aria2`, `.meta4`, `kiwix.session` records because they contain absolute paths. Historical behavior explicitly sacrifices ongoing downloads.

**Migration requirements for Rust/Tauri:**

1. Discover existing **Qt roots**, not just Tauri defaults; include portable installations.
2. Preserve archive files in place; library metadata import must not silently duplicate/move/delete large archives.
3. Preserve book IDs used by URLs, bookmarks, zoom and in-progress records.
4. Import library and bookmark XML through a compatible parser; full wire schema and duplicate/association rules are libkiwix-owned and are not defined in this repo.
5. Recognize pseudo-paths/download IDs so unfinished downloads are not treated as corrupt final archives.
6. Either genuinely resume legacy backend downloads or explicitly surface unsupported recovery; do not silently discard them.
7. Decode QSettings list/byte-array encodings, not merely generic INI scalars.
8. Translate settings-tab sentinel and selected-tab index; Qt geometry/state requires mapping, not blind reuse in a webview framework.
9. Keep split-ZIM manual opening, root/main-entry redirects, Unicode paths, native path separators and archived attachments.
10. Maintain offline network isolation while permitting explicitly requested external navigation and catalog/acquisition network traffic.

## 10. Platform integrations and nonfeatures

- **Linux / implemented:** desktop launcher accepts multiple files via `%F`; MIME registration recognizes `.zim`, `.zimaa` and little-endian magic `72173914`. `resources/org.kiwix.desktop`; `resources/org.kiwix.desktop-mime.xml`.
- **Windows / implemented:** network-drive/UNC and Parallels `C:\Mac\` launch detection disables Chromium sandbox early, then asks consent. This is a Windows workaround, not a generic recommended Tauri behavior. `src/main.cpp:16-47,94-99`.
- **Windows / conditional:** fullscreen native-window border handling. `src/kiwixapp.cpp:100-111`.
- **Qt historical compatibility:** source retains Qt5 branches, but current project requires Qt6 (`kiwix-desktop.pro:7`). Trash’s Qt<5.15 fallback is historical rather than current baseline.
- **App metadata:** About/privacy/source/issue links and donations are implemented; dedicated Help/Feedback/Report Bug/Request Feature menu actions are hidden. `src/kiwixapp.cpp:479-496`; `src/about.cpp`; `resources/texts/about.html`.
- **Absent in audited app:** account system/cloud sync, annotations/highlights, reader font/line-spacing customization, native EPUB reading, global history view, download scheduler, explicit repair/verification UI, app auto-updater, OS share sheet, drag/drop archive import, server auth/TLS controls.
- These absences are **not** parity requirements unless requested for the new superset. Native speech/printing/default-app opening, Linux associations and Windows workarounds must be distinguished from portable core features.

## Compressed implementation requirements

1. **Library:** local/OPDS catalog, searchable persistent language/category/content filters, metadata/description rows, sorting, preview, import, file reveal, safe trash/delete.
2. **Acquisition:** real resumable downloads, durable state, progress/rate, pause/resume/cancel, free-space/filesystem checks, backend failures and completion races; improve current broken error-state recovery rather than copying it.
3. **Reader:** faithful ZIM entry/MIME/redirect handling, HTML/JS/media, CORS/fetch compatibility, external-request blocking, explicit external navigation, home/random/back-forward, context menus and fullscreen.
4. **Search:** indexed title suggestions with stale-result rejection and paging, indexed full-text results/pagination, find in page; simultaneous multi-book search would be a new feature.
5. **Reading workflow:** tabs, settings singleton, bookmark sidebar/import/export, persistent per-book zoom, optional session restoration, TOC, native print/PDF/attachment handling, optional native TTS.
6. **Server:** embedded LAN serving of same live library, IPv4/IPv6/interface/port selection and useful failure messages.
7. **Migration:** Qt INI/session plus libkiwix XML, archive identities/paths, split archives, portable roots, backend in-progress records.
8. **Quality boundaries:** keyboard/RTL/high-DPI/accessibility parity; do not count hidden actions, historical changelog promises or platform-specific integrations as universally working features.