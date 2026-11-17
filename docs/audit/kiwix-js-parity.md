# kiwix-js parity audit

Date: 2026-11-09
Subject: **kiwix-js 4.4.2** (`package.json`) at shallow-clone HEAD `5881608` ("Add browser extension e2e coverage #1528").
Method: read `www/js`, `service-worker.js`, `replayWorker.js`, manifests, i18n, tests and docs; grep-audited the application tree for features that are absent. No builds, installs or runtime execution. kiwix-js claims are source-level; our-side claims come from `docs/FEATURES.md`/`PARITY.md` with their own acceptance boundaries.

This is a source audit, not a runtime certificate. A feature listed as implemented has code evidence; it has not been re-run on a browser in this pass.

## Executive summary

kiwix-js is a **browser/PWA reader**, not a desktop library manager. Our app is a superset in library ownership, acquisition, sharing, annotations, printing/PDF, TTS, privacy modes and OS integration. The genuine kiwix-js-only capabilities are all tied to its web delivery model:

1. **Zimit/dynamic-archive replay** through bundled ReplayWeb.page (`replayWorker.js`, service-worker resolver). We detect `_sw:yes` and show an incompatibility notice; we do not replay it. This is the largest functional gap against kiwix-js.
2. **Service-worker offline app shell / Cache API** and PWA/extension distribution with store-driven updates.
3. **Restricted mode** (inspect untrusted content with active content disabled) plus the per-archive “trust source” prompt.
4. **WebP polyfill** for browsers without native WebP.
5. Browser-extension packaging (MV2/MV3), legacy targets, and FSA-based “reopen last selected archive”.

None of these requires abandoning the native architecture except Zimit replay, which would need a bundled replay engine and service-worker-equivalent routing. PWA/extension packaging is not a desktop-app feature and should be treated as out of scope unless explicitly requested.

## Feature matrix

| Area | kiwix-js implementation / evidence | ours | Status / gap |
|---|---|---|---|
| Runtime and packaging | MV3 `manifest.json`, MV2 `manifest.v2.json`, PWA `manifest.webmanifest`, plus Electron/NWJS/UWP/Ubuntu Touch targets (`README.md:73-90`) | Tauri desktop Windows/macOS/Linux and mobile iOS/Android (`docs/FEATURES.md:272-292`) | **Arch**. Different delivery model; no extension/PWA target for us. |
| Content serving and sandbox | Same-origin sandboxed iframe (`www/index.html:824`), SW fetch interception (`service-worker.js:300-350`), Restricted DOM-injection fallback (`www/js/app.js:2426-2530`, `3126-3633`), CSP sandbox headers (`service-worker.js:620-640`) | Per-archive loopback HTTP origin; service workers explicitly blocked (`docs/FEATURES.md:291`, `362`, `415`) | **Arch**. Keystone difference: browser fetch interception vs native HTTP origin. |
| ZIM parsing | Custom JS reader for header/MIME/pointer lists and cluster types 0/XZ/Zstd (`www/js/lib/zimfile.js:254-303`, `323-400`, `484-501`); optional libzim WASM/ASM worker (`www/js/lib/zimArchive.js:140-180`) | Native C++ libzim behind a C ABI (`docs/ARCHITECTURE.md:118`) | **Both**. Native libzim on our side; JS parser is kiwix-js's primary path. |
| Split/multipart ZIM | `.zimaa`… grouping in `www/js/lib/util.js:122-128`, called from `www/js/lib/abstractFilesystemAccess.js:208,260,287,327`; directory scan at `abstractFilesystemAccess.js:240-330`; full text disabled for split archives (`zimArchive.js:149-175`) | Opening split first parts and complete split sets (`docs/FEATURES.md:311`) | **Both**. Ours validates complete cohorts natively. |
| Storage | Settings in localStorage/cookies (`www/js/lib/settingsStore.js:27-64`); FSA handles in IndexedDB (`abstractFilesystemAccess.js:167-260`); Cache API/IndexedDB asset cache (`www/js/lib/cache.js:56-105`). OPFS helpers exist but are **unreferenced** (`cache.js:707-882`, `980-992`) | Rust SQLite `Store` plus native reader storage; localStorage compatibility bridge, no Web Storage source of truth (`docs/ARCHITECTURE.md:68-70`) | **Both**. OPFS is dormant code, not a shipped feature. |
| Offline app shell | Precached shell, versioned cache deletion, fetch from cache then ZIM/network (`service-worker.js:36-73`, `130-205`, `300-350`) | Installed binary; local archive bytes; no Cache API/app shell | **KJ-only / Arch**. No direct native analogue. |
| Zimit / dynamic replay | Bundled ReplayWeb.page at `replayWorker.js`; SW collection registration and resolver (`service-worker.js:111`, `241-520`); Zimit Classic/v2 support (`README.md:13-17`) | `_sw:yes` notice; SW blocked; no replay (`docs/FEATURES.md:300`, `362`) | **KJ-only**. Largest functional gap. |
| Title search | Binary search over v0/v1 title pointer lists, many case variants, 500 ms debounce (`www/js/lib/zimArchive.js:300-460`; `app.js:2267-2307`) | Native libzim title search, cancellable paged `suggest`, 120 ms debounce, multi-archive scope, spelling (`docs/FEATURES.md:366`) | **Both**. Ours has richer paging/cancellation. |
| Full-text search | libzim worker `search`/snippets (`zimArchive.js:484-530`); requires a full-text index, non-split archive, `Atomics`, and non-Android (`zimArchive.js:149-175`) | Native libzim full text, snippets, multi-archive rank fusion | **Both**. kiwix-js is conditional and single-archive; ours works across selected archives. |
| Multi-archive search | One `selectedArchive` (`www/js/app.js:65`); no all-library search | Selected or all archives (`docs/FEATURES.md:366`) | **Ours-only**. |
| Library/OPDS | “Browse ZIM Library” loads an external site in an iframe (`www/js/lib/kiwixLibrary.js:321-370`, with `iframe.src` at 337 and mirror fallback at 363); no OPDS parser or catalog state | Native OPDS client with offline cache, facets, pagination, thumbnails, previews, reconciliation, edition cutover (`docs/FEATURES.md:317-318`) | **Ours-only**. |
| Acquisition | No download manager or torrent client; only a small unreferenced download helper in `cache.js:729-783`; library downloads happen in the external iframe | Durable HTTP/Metalink jobs with pause/resume/retry, BitTorrent with seeding, iOS Live Activity (`docs/FEATURES.md:321-322`) | **Ours-only**. |
| LAN sharing / server / QR | No HTTP server, `navigator.share` or QR code in `www/js` | Selected-archive and whole-library Kiwix/OPDS server, token-scoped, local QR (`docs/FEATURES.md:419-420`) | **Ours-only**. |
| LocalSwarm/Nearby | No discovery or peer-transfer code | QUIC DNS-SD discovery, verified receive, offers; ordinary library sharing has no PIN (`docs/LOCALSWARM.md:5`, `18`), while optional PINs are scoped to compatibility/browser offers and protected transfers (`docs/LOCALSWARM.md:71-80`, `237-241`) | **Ours-only**; broad producer/device acceptance remains open. |
| File open/import | Picker, drag-and-drop, `webkitDirectory`, Firefox OS DeviceStorage, FSA persistent handles (`www/index.html:491-506`; `abstractFilesystemAccess.js:167-330`) | Native open/import, monitored folders, Android MediaStore/SAF scan, retained descriptors (`docs/FEATURES.md:311-313,315`) | **Both**. kiwix-js is browser-picker limited. |
| Tabs/windows/sessions | No reader-tab model; browser back/forward (`app.js:413-419`, `1573-1596`); no tab session restore | Reader tabs, background tabs, closed stack, per-window sessions/geometry, macOS window groups, restore session (`docs/FEATURES.md:367`) | **Ours-only**. |
| History/reading position | In-session `pushState` only; history lost on exit; last-page path gated by an undefined option (`www/js/lib/cache.js:327-331`) | Persistent history/recently read, locations, unified search, per-window restore (`docs/FEATURES.md:367`, `372`) | **Ours-only** for persistence. |
| Bookmarks/reading list/notes/highlights | No bookmark/annotation/note code or settings keys in `www/js` (grep of `app.js`, `init.js`, `settingsStore.js`) | Reading list with notes/tags, highlights/comments, JSON/XML interchange, private sync (`docs/FEATURES.md:369-371`) | **Ours-only**. |
| TOC/find | Heading TOC (`www/js/lib/uiUtil.js:175-260`); no app find-in-page | Generated TOC, fragment navigation, find previous/next (`docs/FEATURES.md:365`) | **Both** for TOC; **Ours-only** for find. |
| Link previews | Wikimedia/Wikivoyage popovers with lede/image (`www/js/lib/popovers.js:1-130`; `app.js:2683-2808`) | Inert same-archive previews, bounded raster artwork, persisted opt-out (`docs/FEATURES.md:379`) | **Both**. Different scope/security model. |
| Theme/zoom | Light/dark/auto and native/inversion themes; wide-article toggle; no font size or per-book zoom in `www/js/init.js:110-172`, `index.html:578-594` | App themes, reader themes, font/size, 25–500 % per-book zoom (`docs/FEATURES.md:373`) | **Both**; **Ours-only** zoom/font. |
| PDF/print/export/share | Opens/downloads existing attachments; no article-to-PDF, `window.print`, or OS share API in `www/js` | Native article PDF save/share/print, MHTML/HTML/resource export, OS share sheet, attachment Open/Save (`docs/FEATURES.md:375-378`) | **Ours-only**. |
| TTS | No `speechSynthesis`/TTS code | Article/selection read-aloud with installed voices/rate/stop and native mobile engines | **Ours-only**; physical audibility unverified. |
| Maps | No map module; map ZIMs read as ordinary content and need JS/SW | StreetZim composition/acquisition/resume/cleanup and saved-map library (`docs/FEATURES.md:381-409`) | **Ours-only** for composition. Ordinary map-ZIM reading in kiwix-js is unverified. |
| Media/codecs/WebP | Passes bytes to browser; SW builds 206 ranges (`service-worker.js:570-700`); blob media sources (`app.js:3601-3622`); WebP polyfill (`init.js:404-421`); no transcoding | Native range serving; unsupported codec notice and metadata-only, non-autoplay buffering (`docs/FEATURES.md:362`); codec matrix in `docs/VIDEO-FORMATS.md`; no WebP polyfill documented | **Both / Arch**. Codec support is WebView-dependent; runtime unverified on both sides. |
| Accessibility | ARIA labels, live regions, keyboard TOC/navbar behavior; narrow a11y e2e (`tests/e2e/a11y-check.js`); no conformance claim | Native shortcuts, ARIA, focus/RTL; no screen-reader certification | **Both, partial**. |
| Localization | `i18n/en|es|fr.jsonp.js` only | 74 locale entries, 73 non-English catalogs partial | **Ours-only** breadth; not translation-quality-complete. |
| Updates | Store/PWA auto-update; app-cache version alert (`www/js/lib/uiUtil.js:845-871`) | Release staging can inject a signed updater endpoint/key (`scripts/stage-update-release.mjs:46`), but checked-in `src-tauri/tauri.conf.json:14-16` has empty `endpoints`/`pubkey`; no shipped updater channel is active | **KJ-only in the default build / Unverified**; see documentation inconsistency below. |
| OS integration | Extension action opens its own page; no `file_handlers`/`protocol_handlers` in manifests | File associations, `zim://`, Android shortcuts/widget/PROCESS_TEXT | **Ours-only**; Windows packaged execution unverified. |
| Privacy/permissions | No private/read-only mode; session-only history; no camera/location policy | Private and strict read-only modes, per-archive camera/location consent and revocation | **Ours-only**. |
| Security/integrity | CSP/sandboxed content and per-archive “trust source” prompt (`app.js:782-860`); no JS-level checksum gate | Installation-scoped origins, no SW, checksum/UUID/integrity verification before import and deletion | **Both**, different models. |

## Features present only in kiwix-js

- Browser-extension packaging (MV2/MV3) and installable PWA with store/PWA update.
- Service-worker offline app shell, fetch interception, Cache API asset cache, version alert.
- Zimit Classic/v2 replay through ReplayWeb.page/Wombat.
- Restricted mode for inspecting untrusted ZIMs with active content disabled, plus the “trust source” prompt.
- WebP decoding polyfill.
- Firefox OS `DeviceStorage` and legacy browser targets.
- FSA “reopen last selected archive”.
- External iframe-based library with mirror fallback.

## Features present only in ours

Native OPDS catalog; durable HTTP/Metalink/BitTorrent acquisition; LocalSwarm and Kiwix/OPDS server with QR; reader tabs/windows/session restore; persistent history and unified search; bookmarks/reading list/notes/tags/highlights and private sync; find-in-page and per-book zoom/font; native article PDF/print/MHTML/resource export and share sheet; native TTS; StreetZim composition; private/read-only modes and per-archive sensor policy; 74-locale catalog (partial); crash capture; OS associations/`zim://`/Android entry points; multi-archive search and spelling; Android device-wide discovery; monitored folders; storage migration/capacity; archive integrity gates.

## Architectural gaps (not simple omissions)

- Service workers/Cache API as the content and offline delivery mechanism.
- Bundled ReplayWeb.page/Wombat Zimit runtime.
- Extension/PWA packaging and store-driven updates.
- Browser-managed tabs/windows/history as the navigation model.
- File System Access API/OPFS/browser pickers and cookie/localStorage config.
- Same-origin sandboxed iframe isolation vs per-archive loopback origins.
- Legacy targets: Firefox OS, UWP, Electron/NWJS, old IE/Edge.
- Hosted external library in an iframe instead of an in-app OPDS client.

## Adversarial checks on this audit

- **Zimit path correction:** `replayWorker.js` is at the repository root, not under `www/js/lib`. The service worker imports it at `service-worker.js:111`/`241`. The feature is real; the path in earlier notes was wrong.
- **OPFS is not shipping:** the helpers have no call sites in `app.js`/`init.js`; do not count OPFS or the unreferenced download helper as kiwix-js features.
- **No hidden reader features:** grep found zero occurrences of `bookmark`, `readingList`, `speechSynthesis`, `navigator.share` and `window.print` in the app's own `www/js` and `www/*.html` (the bundled `replayWorker.js` contains internal bookmark identifiers and is not app UI). `torrent` occurs only in user-facing help text pointing at qBittorrent; `tabs` occurrences are browser-window comments, not a tab model.
- **Full-text limits are real:** the `Atomics`/Android/split guards are in `zimArchive.js:149-180`, not inferred from UI.
- **No runtime certification:** Zimit replay, the libzim WASM worker, PWA install/offline/update, FSA permission persistence, and media codec behavior were not executed here.

## Documentation inconsistencies found

- `docs/FEATURES.md:319` formerly said “No signed updater channel exists”, while `docs/PARITY.md:308` records a `tauri-plugin-updater` implementation. The checked-in `src-tauri/tauri.conf.json` has empty updater `endpoints` and `pubkey`; release staging can inject values (`scripts/stage-update-release.mjs:46`). The accurate statement, now in FEATURES, is: updater code and packaging exist, but the checked-in build has no active endpoint/key and no production credentials or real restart have been verified.
- The Android article-PDF omission recorded in the older `PARITY.md` row was stale: `share_pdf` is wired on Android through `NativeArticlePdf` (public-SDK WebView capture into `PdfDocument` + `FileProvider` chooser). The row now reflects the implementation and its unverified breadth.

## Top unverified items

1. kiwix-js Zimit replay, including Wombat compatibility and collection routing.
2. kiwix-js full-text search on non-split archives and Android/NWJS exclusions.
3. kiwix-js media/codec behavior on target browsers, and our WebView codec matrix.
4. PWA install/offline/update and FSA permission persistence.
5. Our Windows runtime/PDF/lifecycle; our accessibility/TTS certification; LAN/LocalSwarm third-party and packaged-device breadth.

## Review record

Independent review (`DocsAdversarialReview`, 2026-11-09) checked this audit, `docs/WINDOWS-TEST-PLAN.md` and the Apple media test plan against the kiwix-js checkout, our source, and the fixture. It found no blocking defects; nine evidence/citation and test-plan defects were corrected: the service-worker and media citations, LocalSwarm PIN scope and citations, three FEATURES row references, two kiwix-js source paths, the Windows compile-only command block, the update-check expectation, the Apple plan's missing no-`AVAudioSession` prerequisite, and the monitored-folder citation. The reviewer confirmed each fix. No runtime execution was performed for this review; all runtime claims remain as stated.

## Post-audit implementation (2026-11-09)

Restricted reading mode is now implemented in the native reader (session-scoped per window/archive): the toolbar/banner toggle serves archive HTML with a per-response script nonce, blocks archive scripts, forms, workers, frames, objects and remote network requests, and keeps the trusted storage/reader bridge executable. The feature row and verification boundary are in `docs/FEATURES.md`; the regression test is `service::tests::restricted_reader_serves_only_the_injected_nonce_bootstrap`. This moves Restricted mode from the kiwix-js-only list to both. Zimit replay, the service-worker/PWA shell, extension packaging and the WebP polyfill remain as stated above.
