## Summary

Retrieved the complete public inventory: **100 open issues, excluding PRs**, with bodies, labels, timestamps, comment counts, and traceable URLs. GitHub search returned `incomplete_results: false`; its second page was empty. Independently paginated the repository issues endpoint through its empty third page.

Also retrieved **all 15 issues closed since 2026-07-01**, plus older Fetch/WebGL and data-loss incidents relevant to the rewrite. Read material comment threads, including accessibility, search scope, interrupted downloads, filesystem permissions, SMB integration, and closed-as-duplicate cases.

**Persistence blocker:** this investigation agent is explicitly read-only and has no write/shell tool. Therefore **`local://desktop-issues.json` and `local://desktop-issue-audit.md` were not created**. The complete raw inventory is retained in **`artifact://9`**, and the substantial Markdown deliverable follows below for the main agent to persist. Do not report the named artifacts as written.

### Files / retrieved evidence

- `README.md:7-14`: existing product scope and repository identity.
- **`artifact://9`**: definitive **100-open-issue JSON search response**, complete bodies/labels; JSON begins after the tool’s URL/content-type/method header.
- `artifact://3`: first 100 open issues-endpoint records, including PRs.
- `artifact://6`: remaining open issues-endpoint records, including PRs.
- **`artifact://14`**: complete **15 recent closed issues**, excluding PRs.
- `artifact://4`: first 100 recently updated closed endpoint records, including PRs; useful supplemental older regression evidence.
- `artifact://10`: #1346 comments—permissions error misreported as disk-space error.
- `artifact://11`: #1491 comments—final diagnosis narrows SMB-picker problem to KDE.
- `artifact://12`: #1025 comments—GNOME virtual keyboard also fails to appear.
- `artifact://13`: #1486 comments—CLI/file-association first-tab initialization regression.

### Retrieval provenance

Audit date: **2026-10-28**, using the task’s supplied date. No wall-clock tool was available; upstream responses contain some later timestamps, so this is **not a historical “as of midnight” snapshot**.

Definitive requests:

- [All open issues, issue-only, page 1](https://api.github.com/search/issues?q=repo%3Akiwix%2Fkiwix-desktop+is%3Aissue+is%3Aopen&sort=created&order=desc&per_page=100&page=1): `total_count=100`, `incomplete_results=false`.
- [Open issue-only page 2](https://api.github.com/search/issues?q=repo%3Akiwix%2Fkiwix-desktop+is%3Aissue+is%3Aopen&sort=created&order=desc&per_page=100&page=2): `items=[]`.
- [All issues closed since July 1](https://api.github.com/search/issues?q=repo%3Akiwix%2Fkiwix-desktop+is%3Aissue+is%3Aclosed+closed%3A%3E%3D2026-07-01&sort=updated&order=desc&per_page=100): `total_count=15`, `incomplete_results=false`.

REST open pagination additionally retrieved pages 1–2 and an empty page 3. PRs observed incidentally are **not counted or mapped as open issues**.

## Architecture implications

The issue evidence argues for separate boundaries for:

1. **Untrusted ZIM rendering versus privileged desktop commands.**
2. **Library metadata versus persistent download-job state.**
3. **Local/offline operation versus optional remote catalog services.**
4. **Search scope and identity versus currently rendered menu widgets.**
5. **Reader navigation state versus app-wide catalog refreshes.**
6. **OS file-picker permissions/mounts versus actual disk capacity.**

These are acceptance constraints, **not evidence that Rust/Tauri automatically fixes anything**. WebKit/WebView2/WebKitGTK behavior must be exercised separately on actual packaged platforms.

# Report: desktop issue acceptance matrix

Priorities below are **audit recommendations**, not GitHub priorities:

- **P0**: security, data loss, unusable offline startup, unrecoverable download state.
- **P1**: essential runtime parity, compatibility, responsiveness, accessibility.
- **P2**: additional capability, usability, distribution, or maintainability.
- **N/A implementation**: obsolete Qt-specific mechanism; its user-facing consequence may still apply.

Every row is a requirement or explicit disposition, **not a verified fix**. Enhancement requests remain aspirations unless separately identified as existing behavior.

## A. Security and privacy — 3 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1503 Code review: unvalidated ZIM-triggered navigation reaches OS shell, cross-ZIM XSS in error pages, exceptions as control flow on UI thread](https://github.com/kiwix/kiwix-desktop/issues/1503) | P0 | Treat this as an AI-assisted review containing allegations requiring independent confirmation, not established exploit verification. Nevertheless require a strict content/privilege boundary; safe external navigation and exported files; escaped app error pages; bounded resources; safe lifetime/concurrency; malformed-input handling. Expanded scenarios below cover its multiple findings. |
| [#1386 `<iframe>` inside the ZIM is automatically loading the URL in a browser](https://github.com/kiwix/kiwix-desktop/issues/1386) | P0 | Loading an article containing remote frames, redirects, or other automatic navigation must not launch the system browser. Explicit external-link activation follows a documented allowlist/confirmation policy. No page-controlled native opener. |
| [#1203 Implement a private/read-only mode](https://github.com/kiwix/kiwix-desktop/issues/1203) | P1 | Aspirational privacy capability: private mode must prevent persistent browsing/profile/history/session writes. Strict read-only mode must additionally prevent downloads and other writes. The issue explicitly leaves their relationship and `.private` sentinel undecided; document those semantics rather than silently conflating them. No comments resolve that decision. |

### #1503 expanded acceptance coverage

The issue identifies numerous independent concerns. Translate them into platform-neutral checks rather than recreating Qt fixes:

- **External navigation:** ordinary click, Ctrl-click, middle-click, context-menu opener, iframe, meta-refresh, script redirect, and auto-submitted form all obey one external-scheme policy. ZIM content cannot invoke `file:`, SMB, executable-associated schemes, or arbitrary registered protocols.
- **Origin isolation/error rendering:** archive A cannot inject HTML or script into an app-generated error response attributed to archive B; escape entry paths and metadata.
- **Export/open:** show actual filename and type; do not execute archive-provided `.exe`, `.hta`, `.desktop`, or equivalent privileged payloads through a generic “Open” action. Temporary exports have explicit lifetime/cleanup.
- **Responsiveness:** no full-library archive-open/image-decode work on every title change, keystroke, repaint, or tab switch; slow decompression/search does not block the UI; stale suggestion work cannot overwrite current results.
- **Memory/resource bounds:** avoid payload-size duplicate buffers; bound illustration/result caches and in-flight image requests; idle/hidden library views should not repaint continuously.
- **Lifetime:** repeated tab creation/closure, context menus, cancelled/repeated print operations, fullscreen exit after tab closure, and shutdown with worker activity remain safe; resource use must not grow indefinitely.
- **Tab/book identity:** deleting a book closes every related tab, including adjacent tabs; bookmark clicks resolve the exact displayed item even when other archives are missing.
- **Input robustness:** empty/malformed tags, empty language/category names, malformed saved settings, missing books, and malformed catalog filenames must not crash or traverse outside the intended directory.
- **Concurrency:** catalog replacement, disappearance monitoring, cancellation, and download queues cannot expose dangling state or access UI objects from workers.
- **Interaction:** Ctrl+Tab/Ctrl+Shift+Tab work; suggestion pagination preserves selection/scroll; library rows do not swallow unrelated keyboard/mouse events; exported filename suffixes are correct.
- **Qt-only prescriptions** such as changing a signal connection or `beginInsertRows` are not rewrite requirements. The observable behavior and ownership constraints are.

## B. Reader, rendering, media, article export — 14 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1552 Which character encoding should kiwix-desktop use when encoding isn't specified?](https://github.com/kiwix/kiwix-desktop/issues/1552) | P1 | Render UTF-8 ZIM HTML correctly even without an HTML charset declaration. Maintainer explicitly states ZIM HTML is UTF-8 by specification; do **not** turn this into an unrequested encoding-detection project. |
| [#1548 Left column should display Wikipedia ToC](https://github.com/kiwix/kiwix-desktop/issues/1548) | P2 | Aspirational desktop layout: accessible article ToC in otherwise unused side space, without removing readable content on narrow windows. |
| [#1547 Display Wikipedia Article title as separate frame](https://github.com/kiwix/kiwix-desktop/issues/1547) | P2 | Aspirational persistent article-title presentation while scrolling; title stays synchronized with current article/tab. |
| [#1460 WebAssembly support](https://github.com/kiwix/kiwix-desktop/issues/1460) | P1 | Real WASM-containing ZIM fixture executes offline under each supported platform webview and resource-serving scheme. API-name availability alone is insufficient. |
| [#1433 Reorganise "Save as..." to save article](https://github.com/kiwix/kiwix-desktop/issues/1433) | P1 | Separate whole-article export from resource export. Article menu offers actual MHTML export, alongside existing export capabilities retained by parity. Resource context menus must not save the article instead. MHTML is a requested capability, not presumed Tauri support. |
| [#1408 Problems viewing or saving PDFs and other files](https://github.com/kiwix/kiwix-desktop/issues/1408) | P1 | Open and save embedded PDFs/EPUBs through a working viewer or safe OS handoff; save dialog exposes permitted locations and successful save creates an accessible file. No blank, inert tabs. |
| [#1354 Kiwix won't save cookies for the zim website I downloaded using zimit](https://github.com/kiwix/kiwix-desktop/issues/1354) | P1 | Revisit/reload representative Zimit pages without repeatedly losing consent/preferences where supported. Maintainer confirms bug and links upstream `openzim/zimit#276`; do not accept a commenter’s claim that immutable ZIM bytes make separate browser storage impossible. Archive-generation defects remain upstream work, not automatically repaired by a new shell. |
| [#1276 Zimit Videos are not playable on kiwix-desktop Windows](https://github.com/kiwix/kiwix-desktop/issues/1276) | P1 | Exercise the issue’s real Zimit video fixture on Windows; offline playback, navigation, resource URLs, and codecs work in the delivered package, not only online preview. |
| [#1274 Saving a video fails (and try to save a PDF)](https://github.com/kiwix/kiwix-desktop/issues/1274) | P1 | “Save video” exports the media bytes/type/name, never a PDF of the page. Also distinguish image and audio export. |
| [#1249 TOC navigation should automatically uncollapse DOM elements](https://github.com/kiwix/kiwix-desktop/issues/1249) | P1 | ToC activation reveals collapsed ancestors and scrolls to the intended heading. |
| [#1248 TOC navigations should be recorded in history](https://github.com/kiwix/kiwix-desktop/issues/1248) | P1 | Back/forward traverses ToC movements without reloading the document or resetting expanded/collapsed DOM state. |
| [#1195 Issues for ZIMs using fragment in the URLs](https://github.com/kiwix/kiwix-desktop/issues/1195) | P1 | Bookmark/save/reopen URLs round-trip path, query, section fragment, and SPA hash route; distinguish separate routes on the same HTML entry and update bookmark state on fragment-only navigation. |
| [#690 Open background (and new) tabs in expanded view](https://github.com/kiwix/kiwix-desktop/issues/690) | P1 | Middle-click/background and context-menu new tabs expose searchable article sections consistently; find-in-page can locate content formerly hidden inside collapsed sections. |
| [#611 Equivalent wiki-like Page Preview function](https://github.com/kiwix/kiwix-desktop/issues/611) | P2 | Aspirational offline internal-link preview, preferably first paragraph/image; capability-aware and user-toggleable for unsupported/non-Wikipedia content. |

## C. Tabs, bookmarks, notes, personal navigation — 6 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1403 Sorting Favorites](https://github.com/kiwix/kiwix-desktop/issues/1403) | P2 | Allow alphabetical sorting of saved favorites, without manual bookmark-file editing. |
| [#1194 New implement search in tabs/history/bookmarks](https://github.com/kiwix/kiwix-desktop/issues/1194) | P2 | Search open tabs, browsing history, and bookmarks with clear source/context and correct activation. |
| [#1171 `manager.readBookmarkFile(filename)` should be able to avoid duplicates.](https://github.com/kiwix/kiwix-desktop/issues/1171) | P1 | Reimporting the same bookmark file is idempotent by archive+full URL identity; distinct fragments remain distinct. |
| [#1154 Feature "Undo Close tab"](https://github.com/kiwix/kiwix-desktop/issues/1154) | P1 | Keep a closed-tab history; reopen through tab-bar context menu and Ctrl+Shift+T, repeatedly, restoring correct archive/article. |
| [#876 Add a system to make "notes"](https://github.com/kiwix/kiwix-desktop/issues/876) | P2 | Aspirational persistent highlights/comments anchored to article identity, independent of immutable archive bytes. |
| [#140 Open a new tab display the reading list and top articles](https://github.com/kiwix/kiwix-desktop/issues/140) | P2 | Aspirational useful new-tab page with reading list and top-article discovery. “Top” data source is not specified by the image-only body; do not invent a remote ranking service. |

## D. Search and input — 8 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1535 Change the default ZIM file used for searches](https://github.com/kiwix/kiwix-desktop/issues/1535) | P1 | Make startup/no-article search scope explicit instead of silently selecting the alphabetically first book. Preserve discoverable session restoration. A favorite/last-book preference is the reporter’s proposal, **not maintainer consensus**; maintainer prefers solving multi-ZIM suggestions. |
| [#1500 Kiwix search doesn't function as expected](https://github.com/kiwix/kiwix-desktop/issues/1500) | P1 | Distinguish catalog search from article search in UI and behavior. Online catalog must populate/search; selected local archives must produce suggestions before their selector menu has ever rendered. Comments describe two different symptoms; no single confirmed root cause. |
| [#1360 the searched content is not correct](https://github.com/kiwix/kiwix-desktop/issues/1360) | P1 | Preserve a fixture for `November 24, 1946` and expected article coverage; validate query parsing/ranking rather than claiming every textual occurrence must rank first. App versus index/search-engine responsibility remains unconfirmed. |
| [#1257 Suggest similarly spelled suggestions results](https://github.com/kiwix/kiwix-desktop/issues/1257) | P2 | Aspirational spelling corrections for misspelled queries; issue cites `openzim/libzim#731` as blocker. Surface engine-supported corrections; do not silently treat rewrite as upstream completion. |
| [#1200 Clicking outside of Search Suggestion List Should Fall through](https://github.com/kiwix/kiwix-desktop/issues/1200) | P1 | One click outside suggestions both dismisses them and activates the intended control, specifically suggestion settings; no mandatory second click. |
| [#1025 Can‘t input search pattern beyond the 1st letter on a screen keyboard (linux)](https://github.com/kiwix/kiwix-desktop/issues/1025) | P1 | GNOME on-screen keyboard appears for the search field and remains usable while suggestions update; typing a complete query does not lose focus. Comments confirm the “keyboard never appears” variant even after Qt6. |
| [#499 Search is broken on empty tab page](https://github.com/kiwix/kiwix-desktop/issues/499) | P1 | New/empty tabs can search available archives and display suggestions using documented scope. |
| [#315 Implement multizim search/suggestion properly](https://github.com/kiwix/kiwix-desktop/issues/315) | P1 | Select multiple/all archives, obtain cross-archive suggestions/results, identify source archive, and define meaningful combined relevance. Maintainer cites `openzim/libzim#932`; dependency status must be checked during implementation. |

## E. Library, catalog, performance, content updates — 12 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1541 MultiZimButton::updateDisplay() can be too slow](https://github.com/kiwix/kiwix-desktop/issues/1541) | P1 | Opening search/archive selection stays responsive with many/slow books. Do not reopen every unchanged archive on ordinary navigation; invalidate any availability cache on path/book/download changes. |
| [#1539 Switch the local library file from `library.xml` to `library.opds`](https://github.com/kiwix/kiwix-desktop/issues/1539) | P1 | Support the chosen libkiwix generation and preserve existing library/bookmark access. Upstream proposal rebuilds from monitored directories, leaves old XML untouched, and warns that external-location entries/per-book data may be lost. A Rust database need not use OPDS internally, but migration/data-loss policy must be explicit; bookmark XML is separate. |
| [#1511 Make all library columns sortable, including status](https://github.com/kiwix/kiwix-desktop/issues/1511) | P1 | Every displayed library column, including content type and local/downloading/remote status, is sortable consistently. |
| [#1510 Add a Language column to the library view](https://github.com/kiwix/kiwix-desktop/issues/1510) | P1 | Visible language and flavor distinguish otherwise identical editions without applying a filter. Test three languages × maxi/nopic. Exact column versus equally visible equivalent is a UI decision; do not omit distinguishing metadata. |
| [#1506 Surfing has become really slow with the dev version](https://github.com/kiwix/kiwix-desktop/issues/1506) | P1 | Local link navigation does not add the reported ~2-second shell delay, unrelated archive/index work, or transient “Library” window title. Measure against a fixture baseline; “immediate” is not a measured SLA here. |
| [#1334 Intense network pulls that freezes the entire program for minutes, each for every action I perform.](https://github.com/kiwix/kiwix-desktop/issues/1334) | P0 | Startup, local library actions, settings, directory picker, and typing must not synchronously trigger repeated full-catalog transfers or freeze the UI. Catalog traffic stays asynchronous and bounded. |
| [#1306 If one ZIM file is "slow" then it slows down the whole Kiwix **massively**](https://github.com/kiwix/kiwix-desktop/issues/1306) | P1 | Adding a slow SMB/NAS book must not turn unrelated local filtering/article loads into slow remote-file operations. Test a mixed ~20-book library. |
| [#1181 We should request/use remotely (from library.kiwix.org) translated categories](https://github.com/kiwix/kiwix-desktop/issues/1181) | P2 | Use translated catalog categories when supported; track cited `kiwix/libkiwix#844`. Offline labels remain usable; do not confuse remote category translations with application UI language. |
| [#1167 Propose to migrate files if "Download directory" is changed](https://github.com/kiwix/kiwix-desktop/issues/1167) | P2 | Offer—not silently perform—moving existing archives on download-directory change; preserve library references and originals on failure. Current behavior already permits setting the directory while retaining old books. |
| [#957 Online library filter/search API calls are not optimal](https://github.com/kiwix/kiwix-desktop/issues/957) | P1 | Server-side text/filter queries, paginated results with total count, reused book details/illustrations, and duplicate-request avoidance. Do not download the entire catalog for each filter/keystroke. |
| [#877 Allow to customize catalog source URL](https://github.com/kiwix/kiwix-desktop/issues/877) | P2 | Aspirational add/remove configurable OPDS sources, persisted and independently identifiable. |
| [#324 Content update notification](https://github.com/kiwix/kiwix-desktop/issues/324) | P2 | Identify a newer corresponding online edition and offer a direct update download. This is content-update discovery, separate from application updates. |

## F. Download lifecycle, integrity, P2P — 15 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1538 Keep track of in-progress downloads without relying on `Book::downloadId` in the library file](https://github.com/kiwix/kiwix-desktop/issues/1538) | P0 | Persist job identity/state independently of library serialization; restore active/paused jobs after restart; partial files never masquerade as complete archives. Completion/cancellation updates both domains consistently. |
| [#1471 Download status of currently downloading files in one place](https://github.com/kiwix/kiwix-desktop/issues/1471) | P1 | Central downloads view shows every active/paused/failed job and progress regardless of catalog filtering. |
| [#1463 Introduce libtorrent-based downloads](https://github.com/kiwix/kiwix-desktop/issues/1463) | P1 | Preserve functional torrent downloading. The umbrella remains open although single-session support and cross-session restoration have closed child issues; do not classify the entire capability as absent. Specific engine implementation need not be identical if behavior is preserved. |
| [#1413 after i paused a download and reopened it later it became blank](https://github.com/kiwix/kiwix-desktop/issues/1413) | P0 | Pause a large download, exit, reboot, reopen: job and library UI remain visible and actionable with correct state. |
| [#1398 Add a feature to verify ZIM checksum](https://github.com/kiwix/kiwix-desktop/issues/1398) | P1 | Aspirational explicit integrity check reports valid/corrupt/error without freezing reader UI. Checksum verification is not authenticity verification. |
| [#1393 Limit download bandwidth (upload/download via BitTorrent)](https://github.com/kiwix/kiwix-desktop/issues/1393) | P2 | User-configurable upload/download limits apply to actual torrent traffic and persist as documented. |
| [#1383 My PC suffered a power outage mid-download, now Kiwix freezes forever](https://github.com/kiwix/kiwix-desktop/issues/1383) | P0 | Crash/power-loss recovery with partial archives on external NTFS must not hang startup. Retain valid partial data and expose recover/resume/cancel without manual metadata editing. |
| [#1352 Support DHT, ZeroConf and PeerExchange in BitTorrent](https://github.com/kiwix/kiwix-desktop/issues/1352) | P2 | Aspirational tracker-independent peer discovery and LAN exchange using supported DHT/local-discovery/PEX mechanisms. DHT itself is not proof of internet-free LAN discovery. |
| [#1347 Can´t kill aria2c.exe after restarting download](https://github.com/kiwix/kiwix-desktop/issues/1347) | P1 | Pause/resume very large HDD downloads without wedging UI/status, orphaning downloader ownership, or redownloading valid 80-GB partial data unnecessarily. Aria2 executable is Qt-era implementation detail; lifecycle outcome remains required. |
| [#1328 Library should indicate download time remaining](https://github.com/kiwix/kiwix-desktop/issues/1328) | P2 | Show approximate ETA alongside meaningful progress/speed; handle unknown/stalled states honestly. |
| [#1147 Restarting of downloads paused due to termination of kiwix-desktop](https://github.com/kiwix/kiwix-desktop/issues/1147) | P1 | Restore jobs that were running at exit distinctly from deliberately paused jobs; support intended automatic continuation. Also recover jobs enqueued but not yet submitted. |
| [#1091 Download errors are not properly handled](https://github.com/kiwix/kiwix-desktop/issues/1091) | P0 | Start failures and interruptions transition to explicit actionable error state; resume/restart/cancel works without deleting `<book>` entries by hand. Comments document inert progress bars after filesystem/power failures. |
| [#818 Follow-up of #816](https://github.com/kiwix/kiwix-desktop/issues/818) | P0 | Cancellation/file deletion is confined to validated files owned by the selected job/book. Invalid/empty filenames must never delete neighboring user files. Linked historical #809 reports deletion of all files in the directory. |
| [#532 Let users autoseed bittorrent downloads](https://github.com/kiwix/kiwix-desktop/issues/532) | P2 | Aspirational explicit seeding controls/status for completed downloads; respect configured bandwidth and user opt-in/policy. |
| [#68 How to display partially downloaded zim files ?](https://github.com/kiwix/kiwix-desktop/issues/68) | P1 | Partial/downloading archives remain discoverable across local/online views and restart, but are visually distinct from openable completed books. This concerns **displaying job entries**, not a promise that incomplete ZIM bytes are readable. |

## G. Platform runtime, accessibility, localization, shell integration — 21 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1545 Unable to run v.2.0.4](https://github.com/kiwix/kiwix-desktop/issues/1545) | P1 | Delivered Windows package launches on a clean supported machine with required runtimes installed/bundled or clearly diagnosed. Body is attachment-only; maintainer suggests VC runtime, but success/root cause is not confirmed. |
| [#1522 Make a "dark theme" version of the UI](https://github.com/kiwix/kiwix-desktop/issues/1522) | P1 | Dark mode covers application chrome/dialogs/popovers as well as content; light/dark/system behavior stays coherent. Existing content-dark support is explicitly acknowledged in the issue. |
| [#1495 Kiwix-desktop crashes when opening a new tab on wayland](https://github.com/kiwix/kiwix-desktop/issues/1495) | P1 | Open new tabs/files under Wayland without crashes, including empty library. X11 fallback is not acceptance. |
| [#1493 [bug] kiwix-desktop won't start without internet](https://github.com/kiwix/kiwix-desktop/issues/1493) | P0 | Offline/local startup does not wait indefinitely for favicon/catalog/downloader initialization. Handle watch/file-descriptor exhaustion. Comments identify Debian’s old 2.3.0 and RPC errors; title alone does not prove lack of internet is sole cause. |
| [#1491 Open ZIM files from shared folders (Network Access Storage)](https://github.com/kiwix/kiwix-desktop/issues/1491) | P1 | Open mounted network archives and make picker limitations actionable. **Final reporter comment narrows problem to KDE/Dolphin SMB shares without a local mount path**, notes NFS works, and retracts earlier Windows-wide characterization. Do not promise native `smb://` streaming just from using a native dialog. |
| [#1457 "Cannot create downloader" Pop-Up Issue](https://github.com/kiwix/kiwix-desktop/issues/1457) | P0 | Offline CLI-opened ZIM initializes a selectable/closable first tab regardless of error-dialog dismissal timing. Reader startup must not be gated on downloader creation. |
| [#1454 Application window disappears and reappears when opening a local ZIM file](https://github.com/kiwix/kiwix-desktop/issues/1454) | P1 | Opening the first local archive does not visibly destroy/recreate the window, lose focus, or reset geometry. Qt/OpenGL warm-up is not a Tauri acceptance mechanism. |
| [#1453 UI is broken on Linux](https://github.com/kiwix/kiwix-desktop/issues/1453) | P1 | KDE/Wayland launch and restored window state render visible interactive controls; no forced blank fullscreen. |
| [#1451 Kiwix reads external drive as having no available storage #15](https://github.com/kiwix/kiwix-desktop/issues/1451) | P1 | External-drive destination capacity and access are assessed independently; Flatpak permission denial must not be reported as zero space. |
| [#1399 Window buttons not visible on window managers that do not draw window decorations](https://github.com/kiwix/kiwix-desktop/issues/1399) | P1 | Essential app actions remain reachable with Awesome/undecorated windows through app chrome and keyboard, not only OS titlebar controls. |
| [#1389 Doesn't work well on mobile Linux](https://github.com/kiwix/kiwix-desktop/issues/1389) | P2 | Desktop touch/tablet acceptance: usable scrolling, readable media list, collapsible/responsive sidebar. Linux-phone support is a platform expansion, not automatically implied. |
| [#1370 kiwix triggers speech on virtual-terminal](https://github.com/kiwix/kiwix-desktop/issues/1370) | P1 | Do not trigger unsolicited narration when switching Linux virtual terminals. Preserve intentional assistive-technology support; globally disabling accessibility is not a fix. Qt subprocess details are implementation-specific. |
| [#1346 Kiwix should warn properly if the user tries to download/write a ZIM on a readonly fs](https://github.com/kiwix/kiwix-desktop/issues/1346) | P1 | Distinct messages/actions for permission denied, read-only filesystem, missing destination, and insufficient capacity. Comments explicitly establish Flatpak permissions as the incident’s cause. |
| [#1309 Kiwix Windows does not come back on foreground when ZIM file loaded from file navigator](https://github.com/kiwix/kiwix-desktop/issues/1309) | P1 | Second file-open event routes to running instance, opens intended tab, and requests foreground activation under OS rules. Despite title, body’s reproduction uses Flathub; cover both platforms. |
| [#1260 Ugly oversized GUI under Windows](https://github.com/kiwix/kiwix-desktop/issues/1260) | P1 | Appropriate control density, scaling, legibility, and clipping across Windows DPI/display configurations; do not simply hardcode smaller pixels. |
| [#1242 Are .zim files associated with kiwix-desktop on Windows?](https://github.com/kiwix/kiwix-desktop/issues/1242) | P1 | Installed Windows app registers usable ZIM association/icon and handles double-click cold/warm starts. Portable builds explain any deliberate non-registration. |
| [#1161 Opening a specific page via command line](https://github.com/kiwix/kiwix-desktop/issues/1161) | P1 | Documented CLI can open a specified article in a specified archive, preserving full URL identity; works when app already runs. Exact old `-articleByUrl` syntax is legacy, not mandatory if migrated/documented. |
| [#1083 Suggestions for making Kiwix-desktop accessible for blind people](https://github.com/kiwix/kiwix-desktop/issues/1083) | P1 | Orca can reach/read article content and operate chrome through correct roles/names/focus order without excessive technical chatter. Narration/reader-mode support is separately requested inspiration, not existing behavior. Maintainer says Qt6 did not fix this. |
| [#739 Kiwix-desktop seems not really compatible with screen readers](https://github.com/kiwix/kiwix-desktop/issues/739) | P1 | Screen-reader workflow acceptance for library, search, tabs, dialogs, and content; test actual platform AT, not only DOM/ARIA inspection. |
| [#875 Code a solution to localize access keys](https://github.com/kiwix/kiwix-desktop/issues/875) | P2 | Localizable access keys/mnemonics remain discoverable and collision-managed across supported locales. |
| [#478 Allow to change the UI language / Application default language](https://github.com/kiwix/kiwix-desktop/issues/478) | P2 | Explicit UI-language override independent of catalog/content language, persisted with documented restart/runtime-switch behavior. |

## H. Distribution, updates, packaging — 16 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1544 Limit number of visible files in directory](https://github.com/kiwix/kiwix-desktop/issues/1544) | P2 | Windows portable distribution presents an obvious launcher and hides support files in an internal directory where feasible. This is packaging discoverability, **not library pagination**. |
| [#1517 Release 2.6.0](https://github.com/kiwix/kiwix-desktop/issues/1517) | P2 | Old version-specific release task is not a new runtime feature. Carry forward release discipline: translations, dependency/version checks, platform binaries, metadata, changelog/screenshots, release publication, downstream notifications. Do not reuse its old version number. |
| [#1497 build error on my laptop (armv7l) in running linux (slackware). And building kiwix-desktop-2.5.1 with error src/webview.h:5:10: fatal error: QWebEngineView: No such file or directory](https://github.com/kiwix/kiwix-desktop/issues/1497) | N/A implementation | Missing Qt module/qmake build is obsolete for Rust/Tauri. Explicitly publish supported architecture/OS prerequisites; ARMv7 support remains a platform-support decision, not presumed fixed. |
| [#1494 Provide artifacts in github releases](https://github.com/kiwix/kiwix-desktop/issues/1494) | P2 | Publish versioned binaries as GitHub release assets, not only a slow external download host; expose checksums/identifiable architecture. |
| [#1450 Flathub publishing has failed for 2.5.1](https://github.com/kiwix/kiwix-desktop/issues/1450) | P2 | Old release incident is not a runtime bug. New Flatpak channel, if supported, must publish current functioning artifacts and catch manifest failures. |
| [#1437 Prepare Windows package installer (.exe) for Windows7/10](https://github.com/kiwix/kiwix-desktop/issues/1437) | P1 | Supported Windows installer handles per-user/all-user scope, elevation, associations, shortcuts, uninstall, dependencies, automated nightly/release packaging. Historical Windows7 request conflicts with newer runtime baselines; declare actual supported OSes rather than claiming Windows7 support. |
| [#1362 Flatpak should be build in this repository](https://github.com/kiwix/kiwix-desktop/issues/1362) | P2 | Packaging CI must exercise the actual Flatpak manifest/artifact, rather than hide failures in a separate repo. Repository location is process scope, not runtime parity. |
| [#1338 Add "check for updates" option](https://github.com/kiwix/kiwix-desktop/issues/1338) | P2 | Manual application-version check with useful upgrade path; optional automatic install only where safely supported. Work offline without startup blockage and respect store/package-manager ownership. |
| [#1270 Stop relying on vs_redist.64.exe](https://github.com/kiwix/kiwix-desktop/issues/1270) | P1 | Windows portable experience does not unexpectedly require a separate manual VC-runtime installer; account for native library and WebView dependencies too. Rust alone does not remove them. |
| [#1243 Rename to Kiwix](https://github.com/kiwix/kiwix-desktop/issues/1243) | P2 | End-user branding is “Kiwix” in title/app/binary/distribution surfaces; repository identifier may remain `kiwix-desktop`. |
| [#1165 Replace unmaintained linuxdeployqt](https://github.com/kiwix/kiwix-desktop/issues/1165) | N/A implementation | Qt deploy-tool replacement does not apply. Use maintained Linux packaging/dependency tooling and exercise packaged output; no reason to port old scripts. |
| [#757 Deb Package with content](https://github.com/kiwix/kiwix-desktop/issues/757) | P2 | Aspirational distributor workflow packages/configures one or more preselected archives with the app. Issue proposes install-time download; offline bundle behavior must be explicit rather than assumed. |
| [#402 portable apps integrations](https://github.com/kiwix/kiwix-desktop/issues/402) | P2 | Aspirational PortableApps.com platform integration/update path. Windows-specific distribution channel, not required on other platforms. |
| [#314 Prepare Windows package (*.msix) for Windows11](https://github.com/kiwix/kiwix-desktop/issues/314) | P2 | Aspirational MSIX/store-compatible packaging; associations, appropriate installation scope, shortcuts/uninstall, repeatable headless release/nightly builds. Distinct from EXE installer. |
| [#312 Publish Kiwix Desktop in Microsoft App Store](https://github.com/kiwix/kiwix-desktop/issues/312) | P2 | Aspirational Windows Store publication; requires actual compliant package/listing/submission, not merely an MSIX file. |
| [#136 Can we package Kiwix-desktop as a single .exe and/or with a preloaded zim that opens upon launch?](https://github.com/kiwix/kiwix-desktop/issues/136) | P2 | Aspirational simple portable launcher and preloaded-content launch for offline distribution. Single-file packaging and bundled-content startup are separate criteria; do not promise physical single-file capability without implementing it. |

## I. Engineering, observability, service identification — 5 open issues

| Issue / exact title | Priority | Acceptance requirement / disposition |
|---|---|---|
| [#1528 Add Kiwix Desktop architecture and contributor guide](https://github.com/kiwix/kiwix-desktop/issues/1528) | P2 | Document actual Rust/Tauri startup, IPC, archive opening, resource-serving, library/search/download flows, ownership, and dependency roles. Do not port Qt architecture diagrams as current truth. |
| [#1521 Identify your requests to Kiwix infra](https://github.com/kiwix/kiwix-desktop/issues/1521) | P1 | Catalog and illustration/network clients identify application version/platform with a proper User-Agent. Covers requests from both native clients and rendered catalog views where applicable. |
| [#1321 Introduce a logging system](https://github.com/kiwix/kiwix-desktop/issues/1321) | P1 | Actionable startup/downloader/catalog/storage diagnostics available to users/support; avoid leaking secrets/private article history by default. Observability is explicitly requested here, not incidental rewrite scope. |
| [#540 Codebase: emiting a signal forcedly from outside is a bad practice](https://github.com/kiwix/kiwix-desktop/issues/540) | N/A implementation | Qt signal ownership prescription does not apply. State owners should publish their own changes; do not reproduce cross-component synthetic notifications as app architecture. No independent user-facing feature is specified. |
| [#470 Allow to build with packaged version of qtsingleapplication](https://github.com/kiwix/kiwix-desktop/issues/470) | N/A implementation | QtSingleApplication dependency disappears with cutover. Preserve warm-instance file-open routing separately; Linux distribution dependency/vendor policies still need a documented build strategy. |

**Matrix reconciliation: 3 + 14 + 6 + 8 + 12 + 15 + 21 + 16 + 5 = 100 open issues.**

# Recent closed issue audit — all 15 since 2026-07-01

Closure is evidence about upstream handling, **not proof of shipped behavior or a rewrite fix**.

| Closed issue | Rewrite regression acceptance / interpretation |
|---|---|
| [#1536 Margins "broken" around "+" tab](https://github.com/kiwix/kiwix-desktop/issues/1536) | Regression-labelled. New/close-tab controls have correct spacing and hit areas across DPI/themes. Qt stylesheet fix itself is irrelevant. |
| [#1465 Persistence of libtorrent session state](https://github.com/kiwix/kiwix-desktop/issues/1465) | Functional restart recovery was accepted; comments explicitly defer “real” resume-data optimization because rebuilding chunk state can be slow. Do not claim optimized fast-resume already exists. |
| [#1531 Unable to view openstreetmap zim files.](https://github.com/kiwix/kiwix-desktop/issues/1531) | Closed duplicate of #1476; maintainer says fixed in nightly, planned for 2.6.0. Test real World map ZIM with Fetch API on packaged platforms. |
| [#318 Implement Dark Mode](https://github.com/kiwix/kiwix-desktop/issues/318) | Content dark mode exists upstream; retain it. Open #1522 shows app chrome is separate unfinished work. |
| [#1513 "Settings" tab header is somehow visually broken when selected](https://github.com/kiwix/kiwix-desktop/issues/1513) | Selected settings presentation has no erroneous 1-pixel bar/frame artifact. |
| [#1486 Glitch with first tab when launching Kiwix](https://github.com/kiwix/kiwix-desktop/issues/1486) | Confirmed regression. CLI/file association—including multiple startup archives—must create correctly positioned/selectable/closable first tab without misplaced +/X controls. |
| [#1523 Add support for searching across multiple Zim files](https://github.com/kiwix/kiwix-desktop/issues/1523) | **Closed as duplicate of still-open #315, not implemented.** Combined relevance and selection remain rewrite requirements/dependency risks. |
| [#1445 Unable to deal properly with `opds.library.kiwix.org`](https://github.com/kiwix/kiwix-desktop/issues/1445) | OPDS endpoint/path changes previously broke illustrations. Resolve relative/feed URLs correctly; test redirects, metadata, and illustrations against actual endpoint. |
| [#1443 Starting the flatpak version, so pre-2.5.0 version, displayed a warning about the `.desktop` file](https://github.com/kiwix/kiwix-desktop/issues/1443) | Qt desktop-name suffix warning is obsolete implementation detail; installed desktop identity/app metadata must remain correct. |
| [#1472 CJK Search ...](https://github.com/kiwix/kiwix-desktop/issues/1472) | Maintainer narrows failure to mixed-script indexing: Korean `오연서` can work in Korean Wikipedia but fail inside Spanish Wikipedia. Tracked upstream in `openzim/libzim#734`; closure did not mean fixed. Preserve mixed-script acceptance fixture. |
| [#1499 Browse by Bookshelf in Guttenburg ZIM not working](https://github.com/kiwix/kiwix-desktop/issues/1499) | Maintainer reports latest app+ZIM works and expects 2.6.0. Test bookshelf selection actually changes results, rather than redisplaying homepage; record fixture/app versions separately. |
| [#1442 Wayland plugin seems important to run Kiwix on Ubuntu](https://github.com/kiwix/kiwix-desktop/issues/1442) | Qt plugin packaging no longer applies, but installed Linux package must launch and render on default Wayland desktop without fallback instructions. |
| [#1502 Set new privacy policy URL to https://privacy.kiwix.org](https://github.com/kiwix/kiwix-desktop/issues/1502) | Correct policy link in app, branded variants, and store metadata. |
| [#1482 Dark theme or Inverse Color for Kiwix Reader in Windows](https://github.com/kiwix/kiwix-desktop/issues/1482) | Retain Windows content-dark capability; do not confuse duplicate/request closure with app-wide dark chrome. |
| [#1504 Replace deprecated writeToFile with writeAsXML](https://github.com/kiwix/kiwix-desktop/issues/1504) | Build-only API change, now superseded again by open #1539’s OPDS transition. Choose supported libkiwix API/version; do not copy obsolete XML save advice into rewrite design. |

### Additional older closed incidents inspected

- [#1476 Fetch API does not work at all on kiwix-desktop due to custom zim scheme](https://github.com/kiwix/kiwix-desktop/issues/1476): custom-scheme Fetch compatibility varies by Qt version; enabling a flag in newer code does not make old released packages work. **Equivalent Tauri custom-protocol behavior remains unverified until tested.**
- [#1477 appimage build (at least) does not support WebGL](https://github.com/kiwix/kiwix-desktop/issues/1477): require working offline map/WebGL content in actual packaged output, not just development mode.
- [#809 Stack Overflow downloading error, all files in folder deleted](https://github.com/kiwix/kiwix-desktop/issues/809): historical catastrophic deletion incident informs **P0** cancellation ownership and empty-path checks for #818. Older than the recent-closed window; deliberately included because of severity.

# Prioritized end-to-end acceptance scenarios

These are **proposed checks for implementation/main-agent verification; none were run**.

1. **P0 — malicious archive boundary:** load a ZIM with iframe/meta-refresh/script/form external navigation, dangerous protocols, cross-archive error-path HTML, and executable-named resources. No unsolicited browser/native execution, cross-origin script escalation, or privileged IPC.
2. **P0 — cancellation/data ownership:** place unrelated sentinel files beside a download; fail startup, cancel queued/active/failed jobs, and delete selected books. Only explicitly owned files may be removed.
3. **P0 — true offline startup:** disconnect networking, make catalog/favicon endpoint unreachable, disable downloader availability, and open a local ZIM via CLI/association. Reader remains usable with valid first-tab state.
4. **P0 — crash recovery:** interrupt queued/downloading/paused 100-GB-class jobs on internal and external storage; reopen. No permanent freeze/inert bar, no false completed-book state, no required XML editing, and existing valid partial data preserved.
5. **P1 — serving compatibility:** Wikipedia, SPA/hash-route video, Zimit media, Gutenberg shelves/PDF/EPUB, WASM, and World map fixtures run offline on **each shipped webview and package**; test Fetch/WebGL and media seeking, not only static HTML.
6. **P1 — URL and personal state:** bookmark query+fragment/hash routes; export/import twice; close/reopen tab and app; ToC/back/forward retains route and DOM state.
7. **P1 — search scope and internationalization:** empty/new tab, selector never opened, multi-archive results, catalog versus article search, mixed Korean-in-Spanish content, punctuation/date query, keyboard/on-screen keyboard, and stale suggestions.
8. **P1 — responsiveness with slow storage:** mixed local/NAS library; navigate local content, open selector, type/filter catalog, resize window, and open settings. No unrelated archive reads or blocking remote requests.
9. **P1 — accessibility and input:** Orca article/chrome workflow, keyboard-only download/library/search/tab flow, focus order, localized names, Windows/macOS AT as applicable, and GNOME virtual keyboard.
10. **P1 — OS integration:** cold/warm file associations, foreground activation, multi-file startup, Wayland/KDE/Awesome, DPI, permission-denied versus full disk, and mounted network archives.
11. **P1 — migration:** seed old library XML, bookmark XML, OPDS library, out-of-monitor-dir books, and active jobs. Verify documented preservation/rebuild/loss policy, old files left intact, and no job/library state confusion.
12. **P2 — distribution and feature expansion:** clean-machine installer/portable launch, useful new tab, notes/previews, update detection, content updates, configurable OPDS feeds, seeding/discovery/limits, and declared store channels.

# Comment evidence that materially changes interpretation

- UTF-8, not heuristic encoding detection: [#1552 maintainer clarification](https://github.com/kiwix/kiwix-desktop/issues/1552#issuecomment-6460348366).
- Restoring previous tabs **already exists** and satisfies reporter’s immediate need; favorite default is not agreed implementation: [#1535 discussion](https://github.com/kiwix/kiwix-desktop/issues/1535#issuecomment-6188592523).
- Multi-ZIM is still blocked upstream, despite duplicate closure: [#315 blocker](https://github.com/kiwix/kiwix-desktop/issues/315#issuecomment-2566544230), [#1523 duplicate closure](https://github.com/kiwix/kiwix-desktop/issues/1523#issuecomment-5906625357).
- Search report includes empty online catalog, not solely missing local suggestions: [#1500 clarification](https://github.com/kiwix/kiwix-desktop/issues/1500#issuecomment-5653105601).
- Permission denial was misreported as lack of storage: [#1346 diagnosis](https://github.com/kiwix/kiwix-desktop/issues/1346#issuecomment-2722137945), [maintainer requirement in the issue thread](https://github.com/kiwix/kiwix-desktop/issues/1346).
  - Archival correction: the source report included an invalid shortened comment fragment; this snapshot uses the stable issue URL instead. The maintainer wording was: “If this is a permission problem, kiwix shoukd tell this, not mislead the user to believe something else.”
- SMB issue ultimately narrowed to KDE path/mount integration: [#1491 final reporter diagnosis](https://github.com/kiwix/kiwix-desktop/issues/1491#issuecomment-5979985646).
- Accessibility was **not** fixed by switching to Qt6: [#1083 maintainer confirmation](https://github.com/kiwix/kiwix-desktop/issues/1083#issuecomment-2566948897).
- Virtual keyboard may never appear, not only lose focus: [#1025 Qt6 observation thread](https://github.com/kiwix/kiwix-desktop/issues/1025). Archival correction: the source report's unconfirmed shortened comment fragment was replaced with the stable issue URL.
- Interrupted downloads currently require manual library metadata edits: [#1091 user evidence](https://github.com/kiwix/kiwix-desktop/issues/1091#issuecomment-3622411180).
- Torrent restart restoration is accepted but full optimized resume-data persistence was deferred: [#1465 discussion](https://github.com/kiwix/kiwix-desktop/issues/1465#issuecomment-4789873587).
- Mixed-script indexing remains upstream: [#1472 diagnosis](https://github.com/kiwix/kiwix-desktop/issues/1472#issuecomment-5752451240).
- Repeated cookie consent involves Zimit upstream work: [#1354 upstream reference](https://github.com/kiwix/kiwix-desktop/issues/1354#issuecomment-3601412037).

## Access and evidence limitations

- Public GitHub API access succeeded; no observed authentication/rate-limit blocker.
- Retrieved bodies/labels for every open issue. Comments were selectively fetched in full up to `per_page=100` for **24 threads**: open #1083, #1203, #315, #1091, #1541, #1535, #1552, #1493, #1451, #1346, #1491, #1460, #1354, #1500, #1025, #818, #1386, #1545; closed #1531, #1523, #1486, #1476, #1477, #1472, #1499, #1465 were also fetched. **This enumerated list contains 26 threads; use the enumeration, not the preceding 24 typo, when persisting provenance.**
- No comment thread fetched had an observed >100-comment count; no comment pagination truncation was identified for those selected threads.
- Attachments/videos and remote ZIM fixtures were not downloaded or executed. Image-only reports such as #1545/#140 have limited textual reproduction detail.
- Did not claim to verify #1503’s AI-assisted code-review allegations or any proposed rewrite fix.
- No application files changed; no builds, tests, formatters, or smoke checks run.
- Named local-artifact persistence remains the **only tool-enforced delivery blocker**; raw evidence and complete matrix are available above for the main agent.