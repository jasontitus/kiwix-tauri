Area SERVE: 7 findings, all source-traced. Nothing was reproduced by execution, because `cargo` was off-limits.

One scope correction first: the app no longer links libzim. `src-tauri/Cargo.toml` pins the Rust readers `zimru` and `zimfile-xapian`, and `build.rs` references no SDK. The `.patch` files and `native/*_sdk.rs` are not part of any app build, so I did not review the patches for armv7/Windows correctness; see SERVE-6.

### SERVE-1 — A cached archive handle that has gone bad is never dropped, so the book stays unreadable until restart
Severity: High        Category: reliability
Platforms: macOS, Windows, Linux (mobile self-heals at the next background transition, which clears the cache: `mobile_service.rs:398`, `:1871`)
Evidence class: source-traced
Location: `src-tauri/src/service.rs:1598-1681` (`archive_for_book`), `:1689` (`invalidate`); `src-tauri/src/service/dispatch.rs:1842-1877` (`library_maintenance`); `src-tauri/src/archive_search.rs:54-62` (`Parts::unchanged`); `src-tauri/src/archive.rs:1042-1056`
Trigger:
- (a) A library book sits on a USB or network volume that is disconnected and reconnected while Kiwix runs.
- (b) The file's mtime or length changes in place (`touch`, a sync tool, an in-place overwrite).
- (c) The user deletes a large ZIM in the file manager to free space.

Causal path:
- The cache is keyed `(id, installation_id, path)` and stores `Arc<OnceLock<Result<Archive,String>>>`. A hit returns the stored `Archive`, with its retained descriptors, for good.
- The only eviction on failure is for a failed first open: `if result.is_err() { cache.retain(|item| !Arc::ptr_eq(&item.archive, &slot)) }`.
- Every operation runs `self.parts.unchanged()?`, which returns `"Archive contents changed. Reopen the archive."` on an mtime/len change, or the raw `fstat`/`pread` OS error on a dead descriptor.
- Nothing reacts to that error. `invalidate()` is called only from remove, relink, register and relocate paths.
- `library_maintenance` recomputes `book.available` from `Path::is_file()` and writes it to the store, but calls neither `invalidate` nor `retire_readers`.
- Result for (a) and (b): the book shows as available, every open and every search hit the same dead handle, and the message tells the user to "Reopen the archive", which cannot work. It clears only on restart or after 16 other books evict the slot.
- Result for (c): the unlinked file's descriptors stay in the cache, and in any open reader, so the disk space is not released while Kiwix runs.
- `sharing.rs` already keys its own archive cache by size/mtime stamp and reopens after a UUID match; the two caches have diverged.

Smallest correct fix:
- In `archive_for_book`, on a cache hit with `Ok(archive)`, run a lock-free liveness probe (a new `pub(crate) fn Archive::retained_unchanged()` wrapping `Parts::unchanged`, which is fstat only).
- On `Err`, remove that slot by `Arc::ptr_eq` and fall through to the existing `get_or_init` open. That path already re-runs `archive_source::retain` and the UUID check, so a different file still fails with "Archive file identity changed".
- In `library_maintenance`, for each book whose availability flipped, call `self.invalidate(id)` and `retire_readers(.., Some(&installation_id))`. This also releases deleted files.

Regression test: `service_tests.rs`. Register a temp copy of `small.zim`, call `service.archive(id)`, change the file's mtime (`File::set_modified`), assert the old clone's `entry()` fails with "contents changed", then assert `service.archive(id)?.main_entry()` succeeds. Second test: delete the file, dispatch `library_maintenance`, assert the cache no longer holds the id.

### SERVE-2 — Pressing Enter within 120 ms of the last keystroke lets the pending suggestion timer cancel the submitted search
Severity: Medium        Category: usability
Platforms: all
Evidence class: source-traced
Location: `src/components/Library.tsx:396-411` (debounce effect), `:729-761` (`performSearch`), `:793-829` (`loadSuggestions`); `src/api.ts:486-523` (`createSearchRequest.send`/`cancel`); `src-tauri/src/service.rs:1242-1259` (`search_token`)
Trigger: Library → "Search inside your archives" with at least one archive selected. Type or paste a query and press Enter less than 120 ms after the last character, on a search that takes longer than the remainder of that window (full-text on a large archive).
Causal path:
- The effect schedules `setTimeout(() => loadSuggestions(prefix, [...selected]), 120)`. The timer is cleared only by effect cleanup, when `[search, selected, searchOpen, searchFocused]` change.
- Enter submits the form. `performSearch` calls `closeSuggestions()`, which bumps `suggestionId` but does not clear the timer, and none of the four deps change. The input keeps focus and is not disabled while `busy`.
- The timer fires and calls `request.suggest(...)`. Search and suggest share one request object, so `send()` runs `const cancellation = cancel()`, which posts `cancel_search` for the in-flight search.
- Server-side, `search_token` for the suggest also does `if let Some(cancel) = owner.cancel.take() { cancel.cancel(); }`.
- The search rejects with "Search cancelled". `performSearch`'s `isCurrent()` is still true (`searchId` unchanged), so `onError(e)` runs with no site and shows it as a blocking error. `results` stays null.
- If the search finishes first, the stale timer instead opens the suggestion list on top of the fresh results.
- The same cancellation happens when the user refocuses the input while "Show more results" is loading.

Smallest correct fix: keep the timer id in a ref and `clearTimeout` it inside `closeSuggestions()`. Add a `busyRef` checked at the top of the timer callback and of `loadSuggestions`, so a suggestion never supersedes an explicit search. No backend contract changes.
Regression test: extract the scheduling into a small helper and cover it in `scripts/test-*.mjs` with `node:test` mock timers: schedule → submit → advance 120 ms → assert `suggest` was not called and the `run` promise is still pending.

### SERVE-3 — The spelling cache directory is never garbage-collected
Severity: Medium        Category: reliability
Platforms: all
Evidence class: source-traced
Location: `src-tauri/src/spelling.rs:90-117` (`Identity::generation`/`digest`/`key`), `:911-928` (staging `.tmp`), `:1027-1043` and `:1060-1083` (`publish_index`, `.previous` backup); `src-tauri/src/integrity.rs:44-73` (`ContentIdentity::from_handles`); `src-tauri/src/service.rs:2221`
Trigger:
- Remove or relink a book that has a spelling dictionary.
- Rename, chmod or touch the archive (ctime or mtime changes), or remount a volume whose `st_dev` differs.
- Rebuild a dictionary whose cache file failed validation.
- Kill the app during a build.

Causal path:
- The cache file is `<data>/spelling/{key}.words` (capped at `MAX_DB_BYTES` = 1 GiB each). `key` hashes version, UUID, size, article count, installation id and a `generation` derived from each part's `(device, inode, size, modified_ns, ctime)`.
- Any of those changing yields a new key. `status()` then reports `missing` and the user must rebuild.
- The only `remove_file` calls in `spelling.rs` are the scratch unlink (`:818`) and `Staging::drop` (`:841`). Book removal calls only `spelling.retire_installation()`, which stops a build and deletes nothing.
- On a successful rebuild, `publish_index` renames the old file to `.{key}.{uuid}.previous` and returns `Ok(())` without removing it. Nothing ever reads `.previous` files.
- After process death the `.{key}.{uuid}.tmp` staging file remains (on Unix, `Staging::drop` never ran).
- No sweep exists anywhere, and `storage_summary` does not count this directory, so the space is invisible to the user.

Smallest correct fix:
- Add `SpellingManager::sweep(live_keys)`, run from the blocking part of `library_maintenance` only when no build job is active, so a staging file belonging to a running build is never touched.
- Compute live keys for available books from stored `book.metadata` plus `library_files::archive_content_identity(book)` (no ZIM parse).
- Delete, through the retained no-follow `Dir`, every `*.words`, `.*.previous` and `.*.tmp` not in the live set. Skip the sweep entirely if any book is currently unavailable, so a temporarily missing volume keeps its dictionary.
- In `publish_index`, remove `backup` after `confirm_and_flush()` succeeds.

Regression test: `spelling.rs` tests. Build for identity A, call `sweep(&[])`, assert the directory is empty; build, sweep with A's key, assert only `{A}.words` remains; in the existing rebuild-over-existing test, assert no `.previous` file survives success.

### SERVE-4 — Document navigations that fail for any reason other than "not found" get an uninstrumented text/plain page
Severity: Low        Category: usability
Platforms: all
Evidence class: source-traced
Location: `src-tauri/src/server.rs:790-796` (405 and 400), `:885-894`, `:976-983` (`archive_failure`), `:1955-1963` (`error`)
Trigger: an in-frame link to an entry whose cluster is corrupt, exceeds the 64 MiB decode limit or hits an I/O error; a link with a malformed `%` escape; or an archive `<form method="post">`.
Causal path:
- The handler states its own contract at `:886-889`: "A document navigation must still reach the reader bridge: an uninstrumented 404 leaves the frame inert until the shell's watchdog fires."
- Only the not-found branch honours it (`missing_document`). Every other error returns `archive_failure(...)` → `error(status, body)`: `text/plain` with CSP `default-src 'none'; sandbox`, even when `document_request` is true.
- The 400 and 405 early returns do the same.
- The frame then shows bare text with no bridge. I did not trace `Reader.tsx` beyond the 20 s initial-load watchdog, so the exact shell symptom is unconfirmed; the server comment is the basis.
- `reader_error_document()` already exists and is used for the equivalent `ResourceUnreadable` case with the same wording.

Smallest correct fix: in `archive_request_inner`, when `is_document_request(&headers)`, return `reader_error_document(&state, method, <status>, "This page could not be opened.", "The archive could not read this entry.")` for the non-not-found `Err(message)` branch (still calling `diagnostics::record`) and for the 400/405 early returns. Non-document requests keep the plain responses.
Regression test: `server_tests.rs`. GET `/%zz` with `sec-fetch-dest: iframe` must return a body containing `/.kiwix/<token>/bridge.js`. Truncate the fixture after `ReaderServer::start` (as `warm_index_rejects_a_truncated_file` does) and assert a document GET returns an instrumented 500 while the same GET without `sec-fetch-dest` stays text/plain.

### SERVE-5 — Library search computes spelling corrections and then never shows them
Severity: Low        Category: usability
Platforms: all
Evidence class: source-traced
Location: `src/components/Library.tsx:1454-1489` (results block); `src-tauri/src/service/dispatch.rs:818-862` (`"search"`); `src/types.ts:265-266`; contrast `src/components/Reader.tsx:5049-5075`
Trigger: enable "Offer spelling suggestions for searches", then run a misspelled query from the Library page.
Causal path:
- For a zero-hit search, the `"search"` command loads each book's dictionary, runs `spelling.corrections(...)` and returns `corrections` and `spellingStatus`.
- `Library.tsx` contains no reference to either field. The block renders only `errors`, `hits` and "No matching articles found."
- The Reader panel renders "Search instead for “{query}”" and build/cancel controls from the same response shape.
- So the opt-in feature silently does nothing from the Library, while still paying the dictionary cost. `docs/FEATURES.md:366` lists `Library.tsx` as implementing spelling suggestions.

Smallest correct fix: in the `!results.hits.length` branch, render `results.corrections` as buttons using the existing localized string, calling `setSearch(c.correctedQuery); void performSearch(c.correctedQuery)`. Optionally add a one-line pointer when `spellingStatus` has a `missing` or `rebuild` entry.
Regression test: extend `scripts/test-frontend-surfaces.mjs` (or a component-level node test) with a stubbed `search` reply carrying `corrections:[{originalQuery:"Paginatoin",correctedQuery:"Pagination"}]`; assert the button renders and clicking it issues a search for "Pagination".

### SERVE-6 — Dead libzim SDK build glue and docs that still say the app links libzim
Severity: Low        Category: maintainability
Platforms: all (build and docs)
Evidence class: source-traced
Location: `src-tauri/native/mobile_sdk.rs` (whole file); `src-tauri/native/desktop_sdk.rs` with `src-tauri/src/lib.rs:21-26`; `src-tauri/Cargo.toml:26-27`; `docs/FORKS.md:30`
Trigger: anyone, human or reviewer, reading the tree to learn what parses untrusted ZIM bytes.
Causal path:
- Nothing references `mobile_sdk.rs`; `rg mobile_sdk` matches only the file itself and historical docs.
- `desktop_sdk.rs` is compiled only as `#[cfg(test)] #[allow(dead_code)] mod desktop_sdk` in `lib.rs`. The comment there says "The build script owns the SDK validation", but `build.rs` no longer includes it.
- The dev-dependency `cc` is kept "to test SDK root resolution" for a build path that no longer exists.
- `docs/FORKS.md:30` states "The app links a pinned libzim 9.8.2 built with five patches". `docs/PACKAGING.md:3` says the opposite: "The app reads ZIM content with Zimru … The native archive SDK recipes below reproduce earlier releases and provide independent comparison tools".
- This review's own scope text was written from the stale picture.

Smallest correct fix: delete `native/mobile_sdk.rs`. Delete `native/desktop_sdk.rs`, its `mod` in `lib.rs` and the test-only `cc` dev-dependency if no other test needs it; or move it under `scripts/` beside the comparison SDK recipes. Correct `docs/FORKS.md` to say the patches apply only to the comparison/previous-release SDK scripts and that the shipping reader is zimru plus zimfile-xapian.
Regression test: a small `scripts/test-*.mjs` doc check asserting no Markdown outside `docs/reviews/` and CHANGELOG contains "The app links a pinned libzim", and that every file in `src-tauri/native/*.rs` is referenced by `build.rs` or `src/lib.rs`.

### SERVE-7 — The ZIM fuzz target never decodes a cluster or touches the search index
Severity: Low        Category: maintainability
Platforms: n/a (test tooling; not run by CI)
Evidence class: source-traced
Location: `src-tauri/fuzz/fuzz_targets/zim_archive.rs:23-43`; `docs/SANITIZERS-AND-FUZZING.md:12`; `scripts/fuzz.sh`
Trigger: relying on the recorded "100,000 runs, no crash" as coverage of the parser for hostile archives.
Causal path:
- After `from_positioned`, the target calls only `entry_count`, `title_count`, `main_entry`, `entry_by_url_index(0)`, `entry_by_title_index(0)` and `check()`.
- `check()` is an MD5 pass; `main_entry` and the two index lookups parse dirents only. None calls `get_item`/`get_data`/`cluster*`.
- So the zstd and xz decoders, the blob-offset tables and `blob_direct_access` (the largest untrusted-input surface) are never reached.
- The Glass/Xapian parser (`zimfile_xapian::Index::open_with_limits`, `search_page`, `suggest_page`) has no target at all. The limits also differ from the app's (`zstd_window_log` 27 vs 26 in `archive.rs:392-397`).
- The doc's scope column is literally accurate ("entry/check paths"), but the 1,549-edge figure shows how shallow it is.

Smallest correct fix:
- In the target, loop over the first `min(entry_count, 64)` URL indices: `entry_by_url_index(i)` → `get_item(true)` → `size()` plus `read_at(0, &mut [0; 4096])`.
- Call `title_prefix_range(b'C', "a")` and `validate_cluster_references(i, &[])` for the first few clusters.
- Use the app's `ClusterReadLimits` values.
- Add a second target that feeds bytes as a `zimfile_xapian::Source` to `Index::open_with_limits` and runs one `search_page` and one `suggest_page`, seeded with the index blob extracted from `tests/fixtures/indexed_search.zim`.

Regression test: n/a beyond the targets themselves; update the doc row after a run.

**Covered** (read end to end unless noted)
- `archive.rs` (all non-test code; tests to about line 1460), `archive_search.rs`, `archive_metadata.rs`, `archive_metadata/clusters.rs`
- `server.rs` (all), `search.rs` (all non-test code, most tests), `spelling.rs` lines 1–1230 (all non-test code)
- `casefold.rs`, `ffi.rs`, `zim_scheme.rs`, `publisher.rs`, `apple_requests.rs`, `apple_replies.rs`, `sharing_reader.rs`, `sharing_download.rs`
- `build.rs`, `Cargo.toml`, fuzz target `zim_archive.rs`, `scripts/fuzz.sh`
- `UnifiedSearch.tsx` (all), `Library.tsx` search sections (271–411, 697–919, 1301–1491), `api.ts` `createSearchRequest`
- Consumers traced: `service.rs` archive cache, `search_token`, `reader()`, `resolve_open_entry`; `dispatch.rs` `search`, `suggest`, `library_maintenance`, invalidate sites; `integrity.rs` `ContentIdentity`; `library_files::retain_files`; `archive_source.rs`
- Partially: zimru (title order, cluster cache, `Item::size`/`read_at`, `ArchiveLimits`) and zimfile-xapian (`ReaderLimits`, `suggest_page`)

Checked and found sound, so no finding:
- Range parsing: `bytes=-N`, range past end, zero-length entries, multi-range → 416.
- Redirect chain cap (256).
- Percent-decode and `?`/`#` handling.
- Old vs new namespace lookup.
- Streaming by resolved entry index.
- Lock order `read_lock` → search cache.
- Per-archive lock scope: a slow read does not block other books' reads.
- Integrity checks and imports use their own handle.
- Multi-archive search ordering and cancellation.
- Suggestion token binding.
- Stale-response guards in `Library.tsx`.
- The MRU cluster stays pinned, so there is no re-decompression storm on large compressed items.

**Not covered**
- `server_tests.rs`; test modules of `spelling.rs` and `archive.rs` beyond the first few.
- `spelling/index.rs` beyond its first 140 lines (builder, merge and `Lexicon` internals).
- The six `.patch` files, `native/desktop_sdk.rs` and `native/mobile_sdk.rs` bodies: confirmed unreferenced by the app build, not reviewed for correctness.
- `mcp_protocol` fuzz target, `sharing_reader.js`, the in-reader search UI in `Reader.tsx` (only its corrections block).
- 32-bit and Windows behaviour inside zimru and zimfile-xapian. App-side offset arithmetic in `archive.rs`, `archive_search.rs` and `server.rs` was checked; I found no truncation, because every `as usize` is bounded by a 16 MiB or 1 MiB cap upstream.
- Deliberately not reported: UnifiedSearch matching is case-insensitive but not diacritic-insensitive; the no-index title fallback is case-sensitive (documented in the `archive.rs` header); non-UTF-8 HTML and metadata are rejected (stated policy).