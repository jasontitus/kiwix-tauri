# Dependencies and supply chain

Snapshot taken 2026-11-05, before and after the reductions under "Done", from `src-tauri/Cargo.lock` and `package-lock.json`. Re-run the commands below to refresh it. Vendored forks and native patches are covered in [FORKS.md](FORKS.md).

## 2026-11-17 review

The application now uses its already-locked `time` crate directly for timestamps. Chrono remains a transitive dependency of the torrent stack. The workspace policy treats its maintenance advisory as blocking for direct dependencies. The November 17 run passed the native policy for seven application and four connector targets. The npm build-tool audit still reports an open picomatch advisory. [Review and verification](reviews/2026-11-17/PROJECT-REVIEW.md)

## 2026-11-12 refresh

- **npm:** Vite 7 → **8.3.8**, `@vitejs/plugin-react` 5 → **6.3.0**, `lucide-react` 0.468 → **1.72.0**, `puppeteer-core` 25.16 → **25.17.0**. TypeScript remains **5.9.3**: TypeScript 7 was tested but its compiler API no longer exposes `ScriptTarget.Latest`/`ModuleKind.CommonJS`, which the JS test harness uses.
- **ASCII bundle output:** Vite 8 ignores the old `esbuild.charset` option. `vite.config.ts` now uses Rolldown’s `build.rollupOptions.output.charset: 'ascii'`; a fresh build emitted zero non-ASCII JS code points, preserving the documented one-byte V8 source backing.
- **Rust:** `cargo update` moved 61 compatible packages, including librqbit 9.1.0, zip 9.5.1 and toml 1.2.1. `notify-types` is held at **2.2.0** because 2.3.0 requires Rust 1.91 while the workspace documents Rust 1.90.
- **Retest:** `npm ci`, production build, **340 JS tests**, and **1116 Rust tests (19 ignored)** passed. `npm audit --omit=dev` found zero vulnerabilities. Android API 37 APK rebuilt/installed/launched and rendered the real Washington ZIM; the dependency-updated iPhone build installed and passed the launch/preservation smoke.


## Size

| Graph | Before (2026-11-05) | Now |
|---|---|---|
| Rust lockfile, all targets | 726 | 686 |
| Rust, Linux x86_64 build (normal + build deps) | ~564 | ~493 |
| Rust, macOS arm64 | ~488 | ~456 |
| Rust, Windows x86_64 | ~491 | ~460 |
| Rust, Android arm64 | ~472 | ~441 |
| Rust, iOS arm64 | ~473 | ~442 |
| npm runtime packages | 7 | 7 |
| npm dev/build packages | 129 | 128 |

The frontend is lean; nearly all of the weight is in the Rust graph. Most of each platform's ~440–490 packages come from Tauri and its WebView/windowing stack, and that part is not optional. The reduction opportunities are in what the app adds on top.

Per-platform counts: `cargo tree --locked --all-features --target <triple> -e normal,build --prefix none | awk '{print $1, $2}' | sort -u | wc -l` (approximate; it counts unique `name version` pairs).

## Enforcement

- `src-tauri/deny.toml` is checked by `cargo deny --all-features --target <triple> check` by `.github/workflows/dependencies.yml`, once for each of the seven shipping targets (Linux x86_64, macOS x86_64 and aarch64, Windows x86_64, Android aarch64 and armv7, iOS aarch64). Linux aarch64 and the iOS simulator add no crate these do not already cover. Without `--target`, cargo-deny evaluates each dependency edge's platform condition on its own, so it combines edges that no real build has together (app → tauri-plugin-single-instance on Windows, plugin → zbus on Linux) and reports crates that never ship. The check enforces RustSec advisories (vulnerabilities, yanked crates, unsound notices on any crate, and unmaintained direct dependencies), a GPL-3.0-compatible license allowlist, crates.io as the only source, and no wildcard version requirements. Duplicate versions are only reported.
- `unsound = "all"` is set explicitly. cargo-deny's default reports unsound notices for direct dependencies only, so until this change the glib exception below never matched and a new unsound notice on a transitive crate would have passed silently. A platform-specific exception warns `advisory-not-detected` on targets that do not build the crate (glib on non-Linux targets).
- `npm audit --audit-level=moderate` runs in the same workflow.
- Unmaintained notices block only for direct dependencies; transitive ones are not reported by the CI check. The November 5 scan with `unmaintained = "all"` recorded proc-macro-error (RUSTSEC-2024-0370) on the Linux targets, via Tauri's GTK3 stack (`glib-macros` 0.18). The zbus_names, zvariant and zvariant_derive notices (RUSTSEC-2026-0370/0371/0372) appear only in a run without `--target`; no shipping target compiles them (see the tauri-plugin-single-instance entry under Done). RustSec has no notice for every abandoned crate: `fs2` (removed) and `unicode-casefold` (replaced) were found by release date, not by the advisory check.
- Adding a license to the allowlist requires checking GPL-3.0 compatibility. Adding an advisory exception requires the ID, the reason it does not apply, and the condition for removing it.

Current advisory exceptions:

| Advisory | Crate | Why it is ignored | Remove when |
|---|---|---|---|
| RUSTSEC-2026-0293 | ringbuf 0.4 (via librqbit-utp) | The double drop needs an element whose `Drop` panics; librqbit-utp stores `u8` | librqbit-utp moves to ringbuf ≥ 0.5.2 |
| RUSTSEC-2024-0429 | glib 0.18 (Tauri's Linux GTK3 stack) | Pinned by Tauri; `VariantStrIter` is not used | Tauri moves to gtk-rs ≥ 0.20 |

Fixed when cargo-deny was introduced: `rustls-pemfile` (unmaintained) was replaced with `rustls-pki-types` PEM parsing, and the yanked `async-executor` 1.14.0 was updated.

## Where the weight comes from

"Unique" counts the packages that would leave the lockfile (all targets) if only that one direct dependency were removed. tauri-plugin-single-instance is left out: by this measure it still has 31 unique packages: the plugin itself, built only on Windows, plus 30 for its Linux zbus backend, which no shipping target compiles now that the plugin is a Windows-only dependency (see Done). "Total" counts its whole subtree, most of which is shared with Tauri.

| Direct dependency | Unique | Total | Used for |
|---|---|---|---|
| librqbit 9.0.1 | 78 | 346 | BitTorrent downloads and seeding |
| snow 0.10 | 13 | 79 | Noise protocol for private reading-data sync |
| tauri-plugin-updater | 10 | 474 | Desktop self-update |
| rust-embed 8 | 9 | 25 | Embedded frontend for headless mode, plus locale catalogs |
| rusqlite (bundled SQLite) | 8 | 44 | Offline spelling dictionary built from archive titles |
| notify 8 | 7 | 55 | Watched library folders |
| mdns-sd (vendored) | 5 | 37 | Nearby discovery on non-Apple platforms |

Regenerate with `cargo metadata --format-version 1 --locked --all-features`, computing reachability with and without each direct dependency.

## Done

- **Unused direct dependencies removed:** `hyper`, `hyper-util` and `thiserror` were declared but never referenced. They stay in the graph through axum and Tauri, but the app no longer pins them. The npm devDependency `prettier` had no configuration, script or CI step and was removed.
- **`local-ip-address` replaced with `if-addrs`**, which was already a direct dependency (`src/sharing.rs`, `src/hotspot.rs`, `src/localswarm/network.rs`). Removed 10 packages: local-ip-address, neli and neli-proc-macros (netlink), derive_builder ×3, darling 0.20 ×3 and getset. On Apple and Android both crates read `getifaddrs`. On Linux, if-addrs reads glibc `getifaddrs` and local-ip-address read netlink directly; both give the same per-address labels. On Windows both report the adapter FriendlyName, but if-addrs leaves out addresses that are not in the DAD `Preferred` state: deprecated IPv6 temporary addresses, and addresses still running duplicate-address detection just after a connect. The sharing interface list therefore no longer offers them, and a saved binding to one reports that the interface is unavailable (until DAD finishes; a deprecated address does not return).
- **`fs2` replaced with `src/fs_ext.rs`.** fs2 has had no release since January 2018. The replacement makes the same system calls (`flock` with `LOCK_NB`, whole-file `LockFileEx`, `statvfs`), so lock semantics and `raw_os_error()` values are unchanged. The trait methods are named `try_flock_exclusive`/`try_flock_shared` so method syntax cannot fall through to std's inherent `File::try_lock_shared`. On Windows the volume root still comes from `GetVolumePathNameW`, but space is read with `GetDiskFreeSpaceExW` (64-bit byte counts) instead of `GetDiskFreeSpaceW` cluster counts. `std::fs::File::try_lock` is not a substitute yet: through at least Rust 1.97 it returns `Unsupported` on Android.
- **`snow` trimmed to the one Noise suite in use** (`Noise_XX_25519_ChaChaPoly_BLAKE2s`). Its default resolver also compiled AES-GCM and SHA-2 suites. Removed aes, aes-gcm, ctr, ghash and polyval. The wire protocol is unchanged, so already-paired devices keep working (`private_sync` tests pass). Replacing Noise with the rustls/quinn stack would remove the remaining X25519/ChaCha20-Poly1305/BLAKE2s crates, but it would break pairing with existing installations; that is a protocol migration, not a dependency trim.
- **`tauri-plugin-single-instance` is now a Windows-only dependency.** lib.rs registers it only under `cfg(not(unix))`; Linux and macOS already use `instance.rs`'s own socket rendezvous. Because it was declared for every desktop target, Linux builds compiled its zbus/D-Bus backend (31 packages, three RustSec unmaintained notices) without using it. Linux x86_64 dropped 31 packages; macOS dropped 1 and Windows is unchanged. `cargo tree --target <triple>` shows no zbus on any of the nine shipping targets (Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64, Android aarch64/armv7, iOS device/simulator). Cargo.lock and cargo-deny's graph still contain it, because both cover every platform path; setting cargo-deny's `[graph] targets` was tried and does not remove it. A full in-tree replacement for Windows (a per-user named pipe, about 1,050 lines) was written and reviewed, but not adopted. It had never run on Windows and had open hardening items (integrity levels, client identification, burst handling), and the plugin on Windows is small and maintained.

- **Unicode folding of archive titles and peer paths.** The app now folds with checked Unicode 18 tables. Its test and table generator use the locked ICU4X 2.4.0 reference. The normal app dependency graph drops `icu_casemap` on macOS ARM64. Spelling, sharing and file ownership comparisons keep their existing normalization and use the same folded keys. The parity test compares every Unicode scalar individually and in adjacent strings. The generator prints a Rust source file with the Unicode-3.0 notice. The packaged Unicode notice matches the one in `icu_casemap_data`.
  ```sh
  cargo run --locked --manifest-path src-tauri/Cargo.toml --example generate_casefold > /tmp/kiwix-casefold-data.rs
  diff -u src-tauri/src/casefold/data.rs /tmp/kiwix-casefold-data.rs
  cargo test --locked --manifest-path src-tauri/Cargo.toml --lib casefold::tests
  ```

- **Earlier migration from `unicode-casefold` to ICU4X `icu_casemap` 2.4** (`src/casefold.rs`: one compile-time `const` borrowing the baked data, used by `spelling::normalize`, `localswarm::protocol` path keys and `store::acquisition_components_equal`). unicode-casefold 0.2.0 had no release since October 2016 and Unicode 9.0 tables; `icu_casemap_data` 2.4.0 is Unicode 18.0 and comes from the same ICU4X release as the `icu_*` crates already locked. Each call site keeps its NFKC/NFD wrapping. Lockfile: added `icu_casemap` and `icu_casemap_data` 2.4.0, removed `unicode-casefold`, net +1 package; no existing package changed version. Both new crates are Unicode-3.0, which the allowlist already contains. (The lock also gains a `serde` edge under `icu_collections`, from its weak `serde?/alloc` feature; `cargo tree -e features` shows only `icu_collections/alloc` enabled.)
  - *Differential*, run over all 1,112,064 scalar values with a throwaway crate: full, non-Turkic folding differs for exactly 205 code points. Each crate matched its own `CaseFolding.txt` (9.0.0 and 18.0.0, status C + F) for every scalar. All 205 are characters assigned after Unicode 9.0 that the old tables left unchanged: 204 capital letters (Lu) and the small ligature U+1DF95 (Ll). No existing character's mapping changed. Unicode 11: Georgian Mtavruli U+1C90–U+1CBA and U+1CBD–U+1CBF (46, to Mkhedruli), Medefaidrin U+16E40–U+16E5F (32), Latin U+A7B8. Unicode 12–13: Latin U+A7BA–U+A7C6, U+A7C7, U+A7C9, U+A7F5 (10). Unicode 14: Glagolitic U+2C2F, Latin U+A7C0, U+A7D0, U+A7D6, U+A7D8, Vithkuqi U+10570–U+10595 (35). Unicode 16: Cyrillic U+1C89, Latin U+A7CB, U+A7CC, U+A7DA, U+A7DC, Garay U+10D50–U+10D65 (22). Unicode 17: Latin U+A7CE, U+A7D2, U+A7D4, Beria Erfe U+16EA0–U+16EB8 (25). Unicode 18: Latin U+A7DD, U+A7E2, U+AB6C, U+AB6D, 16 capitals in U+1DF40–U+1DF7E, and U+1DF95 (folds to `ss`). Through the NFKC wrapper 204 differ (NFKC already reduces U+1DF95 to the same result); through the NFD wrapper all 205 differ. The edge cases ß, ẞ, İ, ı, final and capital sigma, ŉ, ǰ, ΐ, ΰ, the iota-subscript and prosgegrammeni forms (ᾳ ᾼ ᾀ ᾈ ᾷ ῳ ῼ ᾴ, U+0345), ligatures, Kelvin and Ångström signs fold identically. Because the old crate maps all 205 to themselves and never produces them, the new key is a function of the old key (checked on 200,000 random strings for both wrappers): two strings equal under the old folding stay equal. The change only merges keys and never splits them. Only U+1DF95 produces ASCII: it folds to `ss`. Through the NFKC wrapper both versions already gave `ss` (no change), but under the raw fold and the store's NFD comparison it now aliases `ss`, `SS`, `ß` and `ẞ`. No differing character produces `/`, `\`, `:`, `.`, space or a control character under the raw fold or either wrapper. (The property check covers every differing character through all three pipelines, with no exclusions.)
  - *Compatibility.* No folded value is sent to peers. Only the spelling dictionary stores one. **LocalSwarm**: the key is computed locally for collision and reserved-name checks in `Manifest::validate`. Manifests and `content_address` carry the original paths. Persisted jobs (`validate_stored`) and received manifests are validated again, but `supported()` admits only one `.zim`, or one split set that differs in ASCII `aa`…`zz` suffixes. No pair of those paths can collide under either folding. LocalSwarm uses the NFKC wrapper, under which U+1DF95 was already `ss` and no other differing character yields ASCII, so the separator, dot, control-character and reserved-name checks see the same input as before, so old and new apps return the same result for every manifest either one publishes, stores or downloads. A hand-made manifest with a Mtavruli/Mkhedruli pair is now refused as colliding, not as unsupported. **Store**: paths are stored raw and folded only while comparing, and no load-time validation compares them. Destination reservations, monitored-folder claims, `has_book_at_path` and cleanup ownership now also treat these case pairs as aliases, and U+1DF95 as an alias of `ss`/`SS`/`ß`. Both-ASCII components now compare with `eq_ignore_ascii_case` (exact, since ASCII is NFD-stable and folds to lowercase). Other pairs allocate per component and are about 10× slower than the old lazy comparison (about 1 µs, around 1 ms per check at 1,000 non-ASCII paths), which is acceptable for these user-triggered checks. That is the existing conservative alias model, and ownership matches are still confirmed by file identity before anything is removed. **Spelling**: an existing dictionary (built atomically by one version, `VERSION` 1) keeps old-folded words. *Upgrade* (old dictionary, new app): a query containing one of the 204 changed capitals (spelling uses the NFKC wrapper, which leaves U+1DF95 unchanged) now folds differently. A Mtavruli query finds Mkhedruli dictionary words it used to miss. A word that the archive's titles contain only in Mtavruli is no longer an exact match until the dictionary is rebuilt. *Downgrade* (dictionary built by the new app, read by an older one; the `spelling` directory travels with portable profiles, see `portable.rs`): the dictionary holds new-folded words, so the older app's Mtavruli query (left unfolded) misses the Mkhedruli entries it would have matched in a dictionary it built itself, and Mtavruli-only title words are stored in Mkhedruli. The edge case is the same size in both directions, limited to these 204 characters. Suggestions appear only when a search has no results, so the effect is at most a different or missing suggestion for these scripts. `VERSION` was not bumped: there are no automatic rebuilds, so a bump would discard every user's dictionary for this edge case.
- **`scraper` replaced with `dom_query` 0.28**, which was already in the lockfile through `tauri-utils` (the app now also names `html5ever` 0.39, likewise already present, for the parser entry points and tree sink). Removed 25 packages and added none: cssparser 0.35, cssparser-macros 0.6, ego-tree, futf, fxhash, getopts, html5ever 0.35, mac, markup5ever 0.35, match_token, phf 0.11, phf_codegen 0.11, phf_generator 0.11, phf_macros 0.11, phf_shared 0.11, rand 0.8, rand_core 0.6, scraper, selectors 0.31, string_cache 0.8, string_cache_codegen 0.5, tendril 0.4, unicode-width, utf-8 and web_atoms 0.1. On desktop and iOS, dom_query is now a runtime dependency instead of only a build one. The parsing layer (`exports::html`) keeps scraper's observable rules: html5ever's default options (scripting enabled, so `<noscript>` is raw text), elements in parser creation order rather than tree order, `<template>` contents included in selection and text, attribute lookup only for un-namespaced names, and attributes written in qualified-name order. Three differences come from html5ever 0.39 following the current HTML standard: `<select>` keeps content such as `<div>`, `<img>` and `<button>` (so an uncaptured image there now fails a PDF export instead of being dropped), active formatting elements are reconstructed before `<svg>` and `<math>`, and `<annotation-xml encoding="text/html">` is an HTML integration point. One scraper defect is gone: after misnested formatting elements, ego-tree's stale parent links could cut `text()` short, which truncated search snippets and preview excerpts. Each difference has a regression test. Review of the port also hardened the PDF sanitizer, fixing pre-existing issues. SVG/MathML `<style>` text is no longer written raw, which could turn escaped text into live markup on reparse. In HTML `<style>`, only a `<` that could open a tag is rewritten (`</**/` outside CSS tokens, `\3c ` inside), so the CSS token stream is unchanged. Each `<style>` is resource-checked as the single sheet the browser builds from its joined text. CSS reference discovery (`exports::css_references`) now uses cssparser's tokenizer (cssparser 0.37 was already in the graph through dom_query, and is now also a direct dependency). The PDF check refuses malformed URLs, while archive exports skip them as browsers do.

## To do

- [ ] **File the librqbit PR** (candidate 1). The patch and PR text are in [`upstream/librqbit-sha1-rustcrypto.patch`](upstream/librqbit-sha1-rustcrypto.patch) and [`upstream/librqbit-sha1-rustcrypto.md`](upstream/librqbit-sha1-rustcrypto.md); they were prepared against rqbit `main` at f23bacdd. Fork `ikatson/rqbit`, `git am` the patch onto a branch, re-run `cargo fmt --all -- --check`, `cargo test -p librqbit-sha1-wrapper --no-default-features --features sha1-rustcrypto` and `cargo clippy -p librqbit --no-default-features --features rust-tls-no-provider --all-targets`, then open the PR with that description.
- [ ] **After it is released**, switch the app to librqbit's `rust-tls-no-provider` and install the `ring` process provider before the torrent session starts (candidate 1), then remove aws-lc from this page.

## Reduction candidates

Roughly ordered by payoff over effort.

1. **Two crypto stacks (upstream change prepared).** The app's rustls and quinn use `ring`, but **aws-lc-rs** (a large C library with a CMake build) also gets in, through two paths in librqbit's `rust-tls` feature: (a) it enables `reqwest/rustls` on librqbit's **reqwest 0.13**, whose default rustls provider is aws-lc-rs; feature unification then also compiles aws-lc into the app's shared `rustls`; (b) it enables `librqbit-sha1-wrapper/sha1-ring`, which despite the name has used aws-lc-rs since upstream commit e25612b (August 2024). No librqbit feature set avoids both, and librqbit 9.1.0 is unchanged.
   - **Do not point SHA-1 back at `ring`.** SHA-1 is librqbit's hot path. On x86_64 with SHA-NI (256 MiB, release build), aws-lc-rs measured 1,746 MB/s, RustCrypto `sha1` 1,653 MB/s and `ring` 0.17 only 440 MB/s, because ring's SHA-1 has no hardware path. RustCrypto `sha1` 0.11 also selects the ARMv8 SHA extensions at runtime.
   - **Upstream PR prepared** (not yet filed): a pure-Rust `sha1-rustcrypto` backend in librqbit-sha1-wrapper, and a librqbit `rust-tls-no-provider` feature (reqwest `rustls-no-provider` plus that backend). It is additive, adds no packages to rqbit's lockfile, and comes with FIPS 180-1 vectors and CI.
   - **Once it is released**, the app switches librqbit to `features = ["rust-tls-no-provider"]` and installs `rustls::crypto::ring::default_provider()` as the process default before starting a torrent session. That removes aws-lc-rs, aws-lc-sys, cmake, fs_extra, jobserver and untrusted 0.7, and adds RustCrypto `sha1`. No vendoring or fork is involved; patching librqbit locally was tried and rejected for that reason.
   - `torrent::tests::created_torrent_info_hash_matches_independent_sha1` pins an info hash computed with Python's hashlib, so a backend switch cannot silently change hashing.
   - Moving the app's own reqwest from 0.12 to 0.13 would leave one HTTP client. `rustls-platform-verifier` is already in the Android graph (through librqbit's reqwest 0.13 and Tauri), but it needs JNI initialisation on Android before it can verify, and no such initialisation was found in the app. The app's catalog and download client would then depend on it, so this is a separate change, which should first check librqbit's HTTPS trackers on Android.
2. **Small, stable but idle crates:** `hex` 0.4.3 (2021) and `mime_guess` 2.0.5 (2024; used once, for the headless server's embedded frontend assets) are feature-complete and small (`mime_guess` alone brings `unicase`), so they are low priority. `mime_guess` could become a small extension table over the known `dist/` file types.
3. **Many duplicate versions** (base64 ×3, getrandom ×3, hashbrown ×5, rand ×2, syn ×3, windows-sys ×5, and others). Most come from Tauri and librqbit; `cargo deny check bans` lists them. Upgrades upstream will collapse some of them.
4. **Optional features.** BitTorrent (librqbit, the largest subtree), StreetZim maps and private sync could become Cargo features, so a minimal reader build can leave them out.

## Transitive maintenance scan

On 2026-11-05 every registry crate in the lockfile was checked against crates.io for its newest non-yanked release. The scan ran after the snow trim but before the scraper, casefold and single-instance changes (710 lockfile packages, 621 crate names). 52 of those packages had no release since 2023-11-05 (three years). The scan scripts are not kept. To repeat it, query `https://crates.io/api/v1/crates/<name>` for each `[[package]]` with a registry source.

**Removed this round:** `unicode-casefold` (2016), and `mac` (2017), `fxhash` (2017), `utf-8` and `futf`, all from `scraper`. `ordered-stream` (zbus) is no longer compiled for any shipping target, now that `tauri-plugin-single-instance` is Windows-only, but it remains in Cargo.lock.

**Done, not dead:** these are small, complete crates with no open advisories, and a crate in this group needs no action:
- fnv
- byteorder
- scopeguard
- same-file
- hex
- httpdate
- atomic-waker
- ident_case
- option-ext
- serde_urlencoded
- urlencoding
- untrusted 0.9
- fallible-iterator and fallible-streaming-iterator (rusqlite)
- maybe-owned and ambient-authority (cap-std)
- iana-time-zone-haiku
- vcpkg (a libsqlite3-sys build dependency on every target; only consulted for MSVC builds)

**Upstream only:**
- *Tauri and the GTK3 stack:* cesu8 (2016, via jni), vswhom, foreign-types, embed_plist, ndk-context, field-offset, x11 and x11-dl, winapi 0.3 with its two target shims, and proc-macro-error (RUSTSEC-2024-0370, through `glib-macros` 0.18). These leave when Tauri moves to gtk-rs ≥ 0.20 and drops winapi.
- *librqbit:* size_format (2018), core_detect, nonzero_ext, intervaltree, assert_cfg, and tap, funty and wyz (through bitvec). fs_extra and untrusted 0.7 come from aws-lc and leave with candidate 1.
- *snow:* curve25519-dalek-derive.
- *selectors (Tauri, and the app before and after the scraper port):* precomputed-hash (2017).
- *Lockfile only, not compiled for any shipping target:* libappindicator and libappindicator-sys (Tauri's optional tray backend).

**Kept deliberately:** Tauri's default `dbus` feature pulls `dbus` and `libdbus-sys`, which link the system libdbus. tao uses it only to read and follow the desktop portal's `color-scheme` (`tao` `src/platform_impl/linux/portal.rs`). Without it, Linux windows and WebKitGTK's `prefers-color-scheme` would stop following GNOME/KDE dark mode. Turning off Tauri's default features to drop it would therefore be a visible regression.

## Native (non-Rust) dependencies

Platform WebViews (WebKitGTK, WebView2, WKWebView and Android System WebView) remain native runtime dependencies. The native libzim and Xapian SDK build scripts reproduce older packages and supply comparison tools. The app opens and searches ZIM files with the locked Rust libraries below.

## Rust archive search

The GPL Workbench app links two independent Rust libraries. Each Cargo dependency pins a separate history in the same private Git repository. Development of the MIT reader stays in the MIT checkout. Search, ranking and index building stay in the GPL checkout. The app only consumes the engine subcrate and the reader library. The MIT reader has its default writer feature disabled. Its optional C ABI and legacy file mapping are also disabled. The dependency lockfile records both full commit IDs. Bundle resources include the MIT reader license, its vendored Zstandard license texts, the Apache-2.0 license for the LZMA reader, and the GPL engine license, the Unicode data notice and the stopword license texts.

| Library | Source history | License and build |
|---|---|---|
| Zimru | [Positioned archive reader](https://github.com/jasontitus/zimru-xapian/tree/a6d1b4ccd968c29420135fa3b70ced578509a1e8) | MIT. Split input uses bounded reads from retained files. |
| zimfile-xapian | [Glass engine](https://github.com/jasontitus/zimru-xapian/tree/380d84d7bb4ba9f30144462fd6bb18e932493fc0/engine) | GPL-3.0-or-later. The app reads indexes. The companion workspace separately provides its index builder. |
| Zstandard | Zimru's own `vendor/zstd-rs`, based on Avarok 0.1.0 at `bac4c37d86e5307537c145823a106f7ed8f386d7` | MIT OR Apache-2.0. The fork supplies bounded streaming decode and a single-frame writer. Use the vendored revision for comparisons. |

The application bundle includes the MIT reader notice, both Avarok license texts, the Glass GPL license, and both Stopwords ISO notices. The MIT crate's normal dependency tree has no Glass edge. The historical Linux comparison used libzstd 1.5.7 and rustc 1.100.0. All ten tested codecs decoded the same 65 frames to 128,005,261 bytes. The zero-unsafe-site observation covered the selected builds of zstd-rs, yolowzstd and ms-compress-ruzstd and their codec runtime dependencies. It does not extend to the full reader, app, platform runtime or optional C ABI. [Codec provenance, measurements and reproduction](https://github.com/jasontitus/zimru-xapian/blob/ca5e62e965e0667120319906af2aee9791dc94fb/docs/rust-codecs.md)

A clean build needs Git read access to the private dependencies and the platform build tools. Developers using Git credential helpers can fetch the exact revisions and verify the boundary with these commands.

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo fetch --locked --manifest-path src-tauri/Cargo.toml
cargo tree --offline --locked --manifest-path src-tauri/Cargo.toml -p zimru --edges normal
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib archive_search::
```

Trusted CI builds require the `ZIMRU_READ_TOKEN` repository secret with contents-read access to the private dependency repository only. The checkout token for Kiwix alone cannot read those dependencies. Forks without access can build the independent connector and frontend. A trusted checkout is required to run the private Rust integration. Private source access must be arranged before publishing a downstream build.

The native CJK repeat on the private GPL commit matched the previous checkpoint. Xapian 1.4 matched all 63 ordinary queries and 6 of 12 rare-Han queries. Xapian 2.0 matched 61 of 63 ordinary queries and 6 of 12 rare-Han queries. The two additional differences were fullwidth queries. The writer's separate experiment rebuilt three whole archives at two levels and two worker counts. Its level-19 output stayed within 0.44% of native archive size on these three inputs. Both the production writer and the app pin exclude that experiment. [Writer measurements, peak memory, extraction and reproduction](https://github.com/jasontitus/zimru-xapian/blob/0a44dcf3740cbe4466903d02e98bb87fd0c189db/docs/writer-benchmark-20261118.md)

A separate MIT decoder experiment used about 6–7% less CPU than production Rust across three Apple Silicon archives. Native decoding was faster on all three. The experimental patch stayed outside the app pin. A level-3 single-worker recheck on this host compressed the first 3 MiB of native-decoded Climate text twice to 719,349 bytes in Rust and 716,882 natively. Nollywood produced 775,446 and 774,167 bytes respectively. All six native, production Rust and experimental writer outputs decoded to their complete source bytes. The earlier Linux run produced much smaller native controls on its repeated input. Reproducing those control settings and bytes remains necessary to explain the reported twofold size gap. [Decoder measurements and reproduction](https://github.com/jasontitus/zimru-xapian/blob/d296c58e5f6bd06f1de683cf725a4f5cac14304c/docs/decoder-benchmark-20261118.md)

Independent reviewers followed search from the app and sharing endpoints through retained file ownership, logical split offsets, index limits, result ranking, title resolution, excerpt generation, cancellation and cleanup. Review fixes included a positional title-table lookup, a limit on the cluster-offset allocation during search, cancellable cache waiters, regular-file checks on discovered split parts and shared snippet escaping. Regression tests exercise truncation after cache warmup, replacement after a named open, cross-part index and checksum reads, redirected index entries, offsets and retained file lifetimes.

The Apple Silicon host run passed 1,183 app library tests with the desktop custom protocol enabled. Twenty tests were marked ignored. Clippy passed with warnings denied. Cargo-deny passed its advisory, source, license and ban policies for all seven configured shipping targets. The desktop build staged six license files byte-for-byte from the reviewed sources. Hosted credential provisioning, installed Windows/mobile app behavior, and physical-device performance remain unverified.

The earlier search-only macOS release-build comparison found the same ordered paths, titles and next-page flags as native search on all 31 query cases across four archives. Both 50-result pages also matched in full. For the Simple Wikipedia “solar system” page with excerpts, first-request latency went from 104.1 to 95.6 ms and warm median latency from 79.4 to 8.7 ms. For Wikipedia 100 “water”, the corresponding values went from 252.4 to 117.5 ms and from 221.4 to 14.0 ms. These are host measurements with a warm filesystem. The first request refers to application caches. A browser test opened “Ray Charles in Person” from search results and verified the rendered article text. Native estimates and Rust hit counts can differ, particularly for title suggestions. [Host comparison and limitations](benchmarks/2026-11-18-rust-archive-search.json)

The serial Rust archive runtime passed 1,190 macOS app tests with the desktop custom protocol. Twenty marked tests were ignored. The host compiler checks passed with warnings denied for the app. The release package had two Mach-O executables, with no dynamic libzim or native Xapian imports, and all seven archive-related license files matched their sources. The archive bridge is absent from the application build. [Runtime evidence, host timings and limitations](benchmarks/2026-11-18-rust-archive-runtime.json)

The paired search runs, HTTP content timings and direct Zimru versus libzim reader measurements use the same Apple Silicon host. The report includes first-use and repeated timings, peak memory for archive reads, and results at equal cache budgets. [Performance comparison and reproduction](benchmarks/2026-11-18-libzim-comparison.md)

The Glass release engine now normalizes search text and classifies Unicode characters with checked tables. ICU4X remains the generator and test oracle. The engine matched the oracle for all 1,112,064 Unicode scalars and 114,000 additional multi-scalar inputs in independent testing. The native Xapian 1.4 and 2.0 comparisons reproduced the earlier results, including the rare-Han differences. All 120 ordered English result lists matched. The app bundles the Unicode-3.0 notice with the GPL engine. Other Kiwix features still depend on ICU crates. [Generator, differential tests and benchmarks](https://github.com/jasontitus/zimru-xapian/tree/380d84d7bb4ba9f30144462fd6bb18e932493fc0/engine#unicode-search-data)
