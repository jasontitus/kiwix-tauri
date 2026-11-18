# Packaged Rust reader on four platforms

The app checkpoint is `ab440ad49e43d444f035a045a99316d102832f6f`. It pins the MIT Zimru archive reader at `a6d1b4ccd968c29420135fa3b70ced578509a1e8` and the separate GPL Glass search library at `380d84d7bb4ba9f30144462fd6bb18e932493fc0`. Tests used isolated libraries. The checks below cover the packages and workflows identified for each platform. The [whole-app inventory](../MEMORY-SAFETY-INVENTORY.md) tracks the remaining memory-safety work.

## macOS ARM64

The release build from `npm run bundle:macos` produced an ad hoc signed Kiwix.app. Strict, deep signature verification passed. Both Mach-O images were inspected. All 43 configured license and source files matched their repository inputs byte for byte.

The signed executable opened the Ray Charles Wikipedia archive in a temporary profile inside its application container. The query “Ray Charles” returned ten results, led by “Ray Charles in Person”. The article returned HTTP 200. Removing the inserted reader bridge left the same 13,031 bytes as native `zimdump`, SHA-256 `e07c0f229a341a7f6054dd69d7e780bb3afd66d1281021668bbea070f5ed2f8d`. A browser using the signed executable's local server independently searched, opened the result and read “Live album by Ray Charles” inside the article frame. The native Mac window launched and the app saved the correct article visit in its test profile. The window's accessibility tree did not expose enough text to verify the native WebView's article body.

The app's Mac Rust library suite passed 1,200 tests with 21 ignored. The JavaScript suite passed 378 with one skipped on Node 24, and the Python suite passed all 182. The Mac distribution ZIP has SHA-256 `111b92ce5abdc1c4491b0ca989a34cd8e5690192d3a1a8a36f261d997dd16c4a`. Its main executable has SHA-256 `e37308227a126a089fa76b0addcc5a882a578d49c8a34c11114828f759be77dc`. This build used local signing. Store distribution and notarization were outside the test.

## iOS ARM64 simulator

An iPhone 17 simulator running iOS 26.5 launched the release app. A UI test delivered the Climate change Wikipedia archive through its incoming file URL, selected **Save a copy in Kiwix**, opened it from the library, searched for “Greenhouse effect”, opened that result and verified “from losing heat to space” in the article WebView. The source and managed copies were both 8,978,471 bytes with SHA-256 `3485a2c475a2356331f33f137ce26ed7116e62c7062d73a7773362d6dff005f4`.

All 43 native plugin tests passed in a foreground application host. Four idle-timer tests had failed in a hostless run, with ten failed assertions across them. The app passed strict, deep signature verification and all 43 configured source and license files matched. None of its three Mach-O images imported native libzim, libkiwix or Xapian. The simulator was removed after the test. Physical-device memory, voice playback, signing for distribution and installation on a phone were outside this run.

The ad hoc simulator ZIP has SHA-256 `de5f7bb4edb459f33680c85b5a6ff163ff635f4b45f8bbee83ef430750c769dc` and its main executable has SHA-256 `f93b8ecccbbb8c0fffb20b28fd0b0e9720749dc53bc7714ce97c7435adef565a`.

## Linux x86-64

The release executable and relay were compiled natively on an x86-64 Ubuntu 24.04 build host. The build used the locked Rust dependency graphs, the production frontend built from the same source snapshot, and Tauri's Debian and AppImage bundlers. The Debian package contained the exact compiled application and relay, and all 43 configured license and source files matched their inputs.

The standalone executable, the extracted Debian executable and the extracted AppImage's `AppRun` ran the same archive checks. Each returned five results apiece for title search, full-text search and suggestions. The result order, article text and integrity result matched after reopening the library and when reading the same archive split across two files. Another archive supplied a 12,414-byte WebP with SHA-256 `c11e2ccd984681167b7ea8e333882ca3c801cdb9fa302d402b9a46c266723811`. All three returned HTTP 206 and the exact bytes 137 through 1111 inclusive. FUSE mounting was unavailable inside the test container.

| Linux artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Debian package | 44,379,156 | `d4dd1289217ba72ecbf1ee6805c66587f5067910f715a23dc073ff514ea1fa2d` |
| AppImage | 119,278,072 | `6489c4016b03a730d14b5d5aea0ea3acafcf693bf8d950fabbf74a692d47b8d6` |

An unprivileged desktop session launched the extracted Debian build under Xvfb. Through GTK and WebKitGTK accessibility, the test entered “climate”, selected “Climate 200” and read the article's paragraph on Australian political funding. A WebDriver attempt launched the window but could not obtain a browser automation context. The accessibility run exercised the existing application window.

## Android

The original ARM64 package from the shared checkpoint crashed during startup. Android's loader could not resolve `__cxa_pure_virtual` in the application library. The bundled speech libraries, signature and 16 KiB alignment had all passed inspection. An independent symbol check found 25 strong imports without a provider in the app's dependency closure. Commit `c9a654aacddb3a7a97ebf0e7f96d6fa81dec637b` links the Android app's shared library with the NDK static C++ runtime and rejects unresolved strong symbols at link time.

The corrected release APK, signed with a test key, launched on an ARM64 Android 15 emulator (API 35). Android Files supplied the Climate change Wikipedia archive. A title search for “solar” returned 35 hits, and “Solar energy” opened with the article text “is radiant light and heat from the Sun”. Full-text search for the same term returned 271 hits and opened “Solar power”. After a forced stop and cold relaunch, the app restored “Solar power” with its article body. The test emulator was shut down after cleanup. The signed ARM64 APK has SHA-256 `b8e33947c34d184fea2fc2bb1a29d953f8c63f2ab1e80634b93b5895c5aab1d1`.

The corrected ARMv7 release APK has SHA-256 `ec8add9d91295146508d9d238c084f138873ab21c86ff482ef34c272a418a102` before signing and `95a6a0d5c03364b2ae17aacdce4b03c42c4f72b16ef3c5cd4f9973b3b0e8e674` after test signing. All five expected libraries were present. Both release builds passed an independent check of strong symbol providers against the API 24 system exports and of application ELF load-segment alignment. The emulator supports ARM64 only. ARMv7 execution and physical Android testing were outside this run.

A separate packaging fix at `7b1f1611d664f124d26fa02ad9c71c6c6c50f6d8` checks for required libraries in each requested ABI of APKs and app bundles. The existing alignment check alone could accept a package missing speech libraries. The positive cases included normal debug and release artifacts and the explicitly native-free inventory build. Six deliberately incomplete package fixtures failed the Gradle check. These checks apply to the final package contents even when native output has been cached.

## Direct reader performance on Linux

The native x86-64 comparison used release drivers for libzim 9.8.2 with the project's retained-file patches and the pinned MIT Zimru reader. Each phase used fresh, alternating processes, the same requested 4 MiB cache budget and eight measured runs per reader per archive. The 192 measured process runs formed 96 matched native/Rust pairs. Every pair returned identical bytes. An earlier benchmark driver emitted raw entry paths on legacy archives. Carrying each legacy namespace into its public path brought all eight tested title prefixes into agreement with libzim and the app's existing path convention.

Median timed API work in milliseconds follows. Every phase except archive open excludes the initial open. The standalone drivers and package tests measure different entry points. [Raw runs, selected content and response hashes](../benchmarks/2026-11-18-linux-archive-readers.json)

| Operation | Climate libzim | Climate Zimru | Wikipedia 100 libzim | Wikipedia 100 Zimru |
| --- | ---: | ---: | ---: | ---: |
| Open | 3.151 | 0.032 | 3.059 | 0.033 |
| First article | 2.591 | 4.225 | 1.907 | 5.559 |
| 32 repeated article reads | 0.031 | 0.062 | 0.080 | 0.097 |
| Random article reads | 29.537 | 57.477 | 52.983 | 78.724 |
| Title prefixes | 0.170 | 0.292 | 0.233 | 0.221 |
| Media ranges | 3.939 | 5.139 | 0.285 | 0.189 |

Including the initial open, median time to the first article was 4.259 ms for Zimru and 5.814 ms for libzim on Climate, and 5.594 ms and 4.995 ms respectively on Wikipedia 100. Random article reads took about 1.95× native time on Climate and 1.49× on Wikipedia 100. The earlier full-app comparison on Simple Wikipedia measured 3.27 s to ready with libzim and 3.40 s with bounded parallel Zimru verification. Median CPU through the subsequent article check and exit was 3.25 s and 7.94 s respectively, with peak memory of 31.1 MiB and 70.3 MiB. Mobile uses serial verification. [Full-app comparison and verification differences](../benchmarks/2026-11-18-bounded-archive-verification.json)

A second Linux pass measured peak resident memory with a small C launcher calling `wait4`. The Python benchmark parent imposed an approximately 26 MiB floor on its child-process readings. The second pass used the same binaries, requests and response hashes, eight measured processes per reader per workload and one warm-up. Zimru's median peak was lower for all 12 archive/operation pairs. Random article reads used 14.98 MiB with Zimru and 20.84 MiB with libzim on Climate. On Wikipedia 100 they used 13.62 MiB and 24.13 MiB respectively. These figures include process startup and the decoder. [Raw Linux memory samples and binary hashes](../benchmarks/2026-11-18-linux-reader-memory.json)

## Subsequent integration checks

The Android producer at `848ad370b8668a814d72ae1344ea69e05db1983e` stages speech libraries from the Cargo output that produced the selected ABI’s application library. The release build recovered all four missing ARM64 speech libraries with an old version marker present. A separate build started with empty native staging and no marker. The five ARM64 ELF files matched the first build byte for byte. The independent review also exercised incomplete-package rejection, both target layouts and the explicit native-free build. The integration commit `43052455b11cabdeff7ef2c186dbfb67cd711f43` contains the reviewed producer and corrected release-mode navigation and updater tests.

On the Mac, the subsequent `40093abf1ce8778a4bb72386f8febdef209f2e9b` build served “Ray Charles in Person” through its signed executable. A browser read “Live album by Ray Charles” in the article frame. The app and relay also passed ten immediate-start and ten private-mode or disable cutovers. The scoped transfer power assertion was released after the last overlapping guard dropped on another thread. The real power assertion and the relay's peer checks had separate independent reviews. These runs used temporary profiles.
