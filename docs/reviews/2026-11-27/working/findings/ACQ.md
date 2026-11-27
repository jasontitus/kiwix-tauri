Area ACQ: 11 findings, ranked. Base `d98cac2`, read-only; no repository file was touched. Scratch (regex repro, one sample public `.torrent`) is under `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/ACQ/`. No cargo was run. Server-behaviour premises were checked with read-only `curl` against the public Kiwix catalog and mirrors (no user data sent).

### ACQ-1 — Web-seed stage truncates a valid partial torrent payload to zero on any restart after the engine has sized the file
Severity: High        Category: reliability
Platforms: all (worst on iOS/Android, where every background/foreground cycle restarts the worker)
Evidence class: source-traced (premise that Kiwix `.torrent` files carry `url-list` reproduced: a real `.zim.torrent` lists 3 HTTPS mirrors)
Location: /home/user/kiwix-tauri/src-tauri/src/torrent/web_seed.rs:399-426 (`download`), :428-440 (`reset_files`), :478-483 (`download_once`)  [+ /home/user/kiwix-tauri/src-tauri/src/torrent/manager.rs:1958-1999; /home/user/kiwix-tauri/src-tauri/src/torrent/storage.rs:446-466 (`ensure_file_length`); /home/user/kiwix-tauri/src-tauri/vendor/librqbit-9.1.0/src/torrent_state/initializing.rs:275]
Trigger: Start a torrent from a `.torrent` URL with `url-list` (the Downloads "URL or magnet link" box, or an OPDS feed with an `application/x-bittorrent` link). Let the web seed fetch X GB, then lose connectivity briefly so it gives up (see ACQ-4) and the engine takes over. Then Pause→Resume, relaunch, or background/foreground on mobile. The catalog's own "BitTorrent" button is not affected: it resolves a magnet, whose metadata has no `url-list`.
Causal path:
1. After a web-seed failure, `manager.rs:1993-1998` only logs "continuing with peers" and adds the torrent.
2. After its initial check, librqbit calls `ensure_file_length(idx, fi.len)`, and `storage.rs:463` does `f.set_len(length)`. The staged file is now full length and sparse.
3. Every later `run()` re-enters the web-seed stage, because the only guard is `web_seed.filter(|_| !already_verified)` and `verified` is set only after publication.
4. `download_once` takes `current = handle.metadata().len()`, which now equals `file.length`, so `if current < file.length` is false and nothing is fetched.
5. `verify_pieces` hashes the whole file and fails on the first sparse piece, returning `SeedError::Verify`.
6. `download` matches `Err(SeedError::Verify(_)) if attempt < 2 && !force_redownload` and runs `reset_files(factory, seed)?`, i.e. `handle.set_len(0)` on every file. All web-seed bytes and peer pieces are gone; progress drops to 0 and the HTTP download restarts from byte 0.
Smallest correct fix: In `download_once`, track whether this pass wrote any web-seed bytes. If none were written (every file was already full length) and `verify_pieces` fails, return a distinct non-destructive error so `download` skips `reset_files` and the manager hands the files to the engine, whose check keeps the good pieces. Keep the truncate-and-retry only for a pass that itself just downloaded the bytes. Alternative: persist a per-record "engine has initialised storage" flag and skip the web-seed stage when set. No wire or journal contract changes with the first option.
Regression test: `torrent_tests.rs` or the `web_seed.rs` test module. Stage a file whose first pieces are valid and whose length was extended to full with zeros; call `web_seed::download` against a seed that returns 503. Assert the file length and leading valid bytes are unchanged and the call returns `Err`.

### ACQ-2 — Segmented HTTP retry budget never resets on progress, so a long download fails after 8 cumulative transient errors per range
Severity: Medium        Category: reliability
Platforms: all (the default path for every catalog download ≥ 4 MiB from a range-capable mirror)
Evidence class: source-traced
Location: /home/user/kiwix-tauri/src-tauri/src/acquisition/segments.rs:213 (`segment_worker`, `let mut retries = 0usize;`), incremented at :256, :297, :311, :439  [+ /home/user/kiwix-tauri/src-tauri/src/acquisition/transfer.rs:543 `body_retries = 0;`]
Trigger: A multi-GB download split into 4 ranges (each many GB) on a flaky link, or a laptop that sleeps and wakes a few times. Each dropped connection, each failed reconnect while Wi-Fi re-associates, and each 429/5xx adds one to that range's `retries`.
Causal path: `retries` only increases. Once it exceeds `SEGMENT_RETRIES` (8) the worker returns `SegmentError::fatal("Receiving download segment: …")`; `run_segments` returns `Err`; the row becomes `failed` and needs a manual Retry. Nothing auto-resumes `failed` rows; `resume_interrupted` covers only `paused && interrupted`. The 2026-11-25 remediation fixed exactly this for the single-stream path ("reset request-phase retry count" at `transfer.rs:543`), so the two paths have diverged. Durable bytes are kept, so this is lost work, not lost data.
Smallest correct fix: In `segment_worker`, set `retries = 0` once a response delivers payload (right after `done += count as u64`, or after the body loop when `received > 0`). It stays bounded because every reset requires forward progress.
Regression test: `acquisition_tests.rs`, extending `test_range_server`/`handle_test_connection` to cut each range's connection after N bytes more than `SEGMENT_RETRIES` times. Assert the job reaches `complete` with correct bytes. A companion case with zero-progress failures must still fail after 8.

### ACQ-3 — iOS names every official-catalog download `<name>.zim`, so flavours and editions of one archive collide
Severity: Medium        Category: usability
Platforms: iOS
Evidence class: source-traced (premise reproduced: the live official feed returns `nopic` and `maxi` entries that both have `<name>wikipedia_ab_all</name>`)
Location: /home/user/kiwix-tauri/src-tauri/src/service/dispatch.rs:2332-2336 (`"start_download"`, `#[cfg(target_os = "ios")]`)  [+ /home/user/kiwix-tauri/src-tauri/src/acquisition/manager.rs:816-835]
Trigger: On iOS, download "Wikipedia (ab) nopic" from the official catalog, then try the maxi flavour or next month's edition of the same archive.
Causal path: `let stem = entry.name.strip_suffix(".zim").unwrap_or(&entry.name); let name = … Some(format!("{stem}.zim"))` passes `wikipedia_ab_all.zim` to `start_official_catalog`. OPDS `<name>` carries neither flavour nor date. Other platforms pass `file_name: None` and get `wikipedia_ab_all_nopic_2026-10.zim` from `filename(&parsed)`. The second start is refused in `start_transport` by one of three checks: "A download already owns this URL or destination…", "Acquisition destination … is reserved by an installed library book…; choose another destination", or "Destination already exists…". iOS offers no destination choice and `errorText` replaces the path with "[File]", so the message is not actionable. The only way out is deleting the installed archive first.
Smallest correct fix: Pass `None` as the name on the iOS official branch so `start_transport` derives it from `entry.url` (`filename()` already strips `.meta4`). `start_official_catalog` has no other caller; already-installed files keep their names.
Regression test: A host-compilable helper for the official-catalog file name with a unit test: two entries sharing `name` with different URLs yield different file names, each equal to `filename(url)`.

### ACQ-4 — Web seed gives up after three back-to-back attempts with no delay, restarts each mirror from a stale offset, then silently falls to peers
Severity: Medium        Category: reliability
Platforms: all
Evidence class: source-traced
Location: /home/user/kiwix-tauri/src-tauri/src/torrent/web_seed.rs:406-424 (`download`), :485-519 (`download_once` base loop; `Err(error) => last_error = Some(error)` at :517)  [+ /home/user/kiwix-tauri/src-tauri/src/torrent/manager.rs:1993-1998]
Trigger: Same job type as ACQ-1, with a network blip of a few seconds, a sleep/wake, or a Wi-Fi↔cellular switch during the web-seed phase.
Causal path:
- The `for attempt in 0..3` loop has no sleep. With the network down, `request.send()` fails immediately for every base URL, so all three attempts burn within moments.
- Inside one attempt, a mid-stream error leaves `current` unrefreshed (only the `Ok` arm re-reads `handle.metadata().len()`). The next mirror is asked for `bytes={stale current}-` and re-downloads everything the failed mirror already wrote. The `progress` callback restarts from the stale offset, so the displayed bytes jump backwards.
- After the third failure the manager only `eprintln!`s and adds the torrent. With an empty swarm the row sits at "Downloading", 0 B/s, with no error. The natural reaction (Pause→Resume) then hits ACQ-1.
Smallest correct fix: Refresh `current` from the handle in the `Err` arm. Add a bounded, token-cancellable backoff (e.g. 1/2/4…≤30 s) between transport attempts, and do not count an attempt that advanced the file length against the limit.
Regression test: `web_seed.rs` tests with a loopback seed that drops the first connection mid-body. Assert the second request's `Range` starts at the bytes already written, and that reported progress never decreases.

### ACQ-5 — `errorText` rewrites local verification and interruption messages into "The sender's archive failed verification. Stop sharing and start again…"
Severity: Medium        Category: usability
Platforms: all
Evidence class: reproduced (regex run against the literal backend strings: `scratchpad/ACQ/regex.mjs`)
Location: /home/user/kiwix-tauri/src/components/UI.tsx:137-138 (`errorText`)  [+ /home/user/kiwix-tauri/src-tauri/src/acquisition/persistence.rs:250-251; /home/user/kiwix-tauri/src-tauri/src/acquisition/finalize.rs:29; /home/user/kiwix-tauri/src/components/Downloads.tsx:1388-1394]
Trigger: The app is killed during "Verifying…" of a non-segmented HTTP download, or during the iOS native adoption copy (status is `verifying` before `native.total` is durable). On relaunch the row is `paused`+`interrupted` with error "Verification was interrupted by application shutdown; resume when ready".
Causal path: `/checksum|corrupt|source changed|integrity|verification/i.test(message)` matches. The card shows status "Paused — interrupted; resume when ready" beside a red alert saying the sender's archive failed verification and to download it again. The correct action is Resume; the text invites discarding a complete multi-GB file. The same rewrite hits:
- "Reading download checksum: <I/O error>" (a local disk error).
- "Download ownership changed during verification; retry the import".
- "Downloaded archive failed its advertised SHA-256 checksum; cancel and start a new download", where the useful instruction ("cancel") is lost. A fresh catalog start is then refused with "A download already owns this URL or destination" because the failed row still owns it.
Smallest correct fix: Restrict that branch to Nearby/LocalSwarm errors, e.g. an opt-in flag that only callers rendering LocalSwarm/shared-library errors pass, or apply it in `Downloads.tsx` only when `job.transport === "localswarm"`. Other messages pass through unchanged. Touches only the frontend mapping; locale keys are unchanged.
Regression test: A new `scripts/test-error-text.mjs`. Assert the persistence.rs interruption message and the SHA-256 message are returned unchanged for an HTTP job, and a LocalSwarm verification error still maps to the sender text.

### ACQ-6 — A resolved Metalink mirror is pinned for the life of the job, so Retry on a dead or blocked first mirror can never succeed
Severity: Medium        Category: reliability
Platforms: all
Evidence class: source-traced (the live `.meta4` lists 6 HTTPS mirrors, is marked `<origin dynamic="true">`, and ranks a small volunteer host first)
Location: /home/user/kiwix-tauri/src-tauri/src/acquisition/transfer.rs:69-83 (`execute_transfer`, `if snapshot.transfer_url.is_none()`), :240  [+ /home/user/kiwix-tauri/src-tauri/src/acquisition/http_meta.rs:236-241 (`parse_metalink`); /home/user/kiwix-tauri/src-tauri/src/acquisition.rs:2414-2426 (iOS native)]
Trigger: The priority-1 mirror is down, slow, or blocked on the user's network, at start or mid-download.
Causal path: `parse_metalink` keeps only `mirrors.into_iter().next()`. The worker persists it as `transfer_url` before the first payload request. Every later run skips resolution and requests the same host. `queue_resume` and the segment-restart paths never clear `transfer_url`. The row fails with raw reqwest text ("Requesting download: error sending request for url (…)"), and every Retry repeats it, even at 0 bytes. The only recovery is Cancel plus a new start.
Smallest correct fix: In `queue_resume`, when the job URL is a Metalink and `retained == 0`, clear `transfer_url`, `etag` and `last_modified` so the worker re-resolves. Better, as a follow-up: keep the ordered eligible mirror list and advance to the next after a request-level failure with no durable bytes. With durable bytes, If-Range on the new mirror already forces a safe restart or continuation.
Regression test: `acquisition_tests.rs`. A Metalink fixture whose first mirror refuses connections and whose second serves the file: after one failure, `resume` completes. A second case with a partial file asserts the partial is never appended to under a different validator.

### ACQ-7 — Catalog "installed" detection compares raw date strings and ignores flavour
Severity: Low        Category: usability
Platforms: all
Evidence class: reproduced (live feed gives `<dc:issued>2026-10-16T00:00:00Z</dc:issued>`; `"2026-10-16" === "2026-10-16T00:00:00Z"` is false)
Location: /home/user/kiwix-tauri/src/components/Catalog.tsx:83-97 (`installedEdition`, `book.date === entry.date`)  [+ /home/user/kiwix-tauri/src-tauri/src/catalog.rs:901 (`date: field(entry, &["issued", …])`); /home/user/kiwix-tauri/src-tauri/src/archive_metadata.rs:131]
Trigger: Open Discover with any official-catalog archive installed.
Causal path: `entry.date` is the raw RFC 3339 `issued`; `book.date` is the ZIM `Date` (`YYYY-MM-DD`). `exact` is therefore never true for the official catalog, so the "Installed" badge, the "Open" button and "This edition is in your library." are unreachable. Every installed archive shows "Installed edition {date}" plus an active Download, which then fails with the reserved-destination error. Matching is by `name` only, so the maxi card is badged as installed when only nopic is. Rust already normalises this (`catalog_index::edition_date`); the frontend copy has drifted.
Smallest correct fix: Compare the first 10 characters of both dates, and require the entry's `_flavour:` tag to equal `book.flavour` for both the badge and `exact`. The date fix alone would hide Download for a different flavour of the same date.
Regression test: `scripts/test-frontend-surfaces.mjs` (or a unit test of an exported `installedEdition`): an RFC 3339 entry date with a matching `YYYY-MM-DD` book is `exact`; same name and date with a different flavour is not a match.

### ACQ-8 — Interrupted and failed rows hide all progress, so the user cannot see how much a Resume/Retry would keep
Severity: Low        Category: usability
Platforms: all
Evidence class: source-traced
Location: /home/user/kiwix-tauri/src/components/Downloads.tsx:1187-1189 (`live`), :1245 (progress block)  [+ /home/user/kiwix-tauri/src-tauri/src/acquisition/persistence.rs:234-239]
Trigger: Quit or get killed mid-download (every relaunch sets "Download interrupted by application shutdown; resume when ready"), or any failed transfer.
Causal path: Both the progress bar and the bytes line require `!job.error`, and `live` also requires `!finished` (which includes `failed`). The card shows only the status, the red text and Resume/Retry; a row holding 32 of 40 GB looks the same as one holding nothing. "Retry" does not say it continues from the saved part. The details twisty has no byte counts either.
Smallest correct fix: For `paused` and `failed`/`error` rows with `job.downloaded > 0`, render the "{downloaded} of {total}" line (and a static bar) regardless of `job.error`. Keep rate/ETA suppressed.
Regression test: `scripts/test-frontend-surfaces.mjs`: a paused+interrupted job with an error and `downloaded/total` set renders the bytes text.

### ACQ-9 — Android drops the foreground transfer service while a torrent share is "creating", and never re-arms it
Severity: Low        Category: reliability
Platforms: Android
Evidence class: source-traced
Location: /home/user/kiwix-tauri/src-tauri/src/mobile_service.rs:2080 (`mobile_transfer_update_locked` torrent status list), :2075, :2094  [+ /home/user/kiwix-tauri/src-tauri/src/service.rs:2881; /home/user/kiwix-tauri/src-tauri/src/torrent/manager.rs:455, :509-516; /home/user/kiwix-tauri/src/download-state.ts:50-53]
Trigger: Share a multi-GB library archive over BitTorrent on Android, then leave the app.
Causal path: `share()` waits at most 2 s and returns with status `creating` while the archive is hashed. `service.rs:2881` then runs `mobile_transfer_update_locked`. The list (`"queued"|"resolving"|"initializing"|"metadata"|"checking"|"downloading"|"publishing"|"seeding"|"live"`) has no `creating`, so `active` is false, `transferState(active=false)` is sent and `foreground_transfers` is cleared. Line 2075 then returns early on every later tick, so the service is not restored when the row reaches `seeding`. On background, `!foreground_transfers` leads to `torrents.suspend()`, and `restore_record` clears the share's `seeding`. A small archive, hashed within 2 s, keeps sharing in the background; a large one stops. The list also contains `initializing`, `metadata` and `live`, which `TorrentManager` never emits, so the string contract has drifted.
Smallest correct fix: Add `"creating"` to the torrent active list in `mobile_transfer_update_locked`, and to `NOTIFICATION_ACTIVE` plus the Android label map for consistency. Ideally expose one `TorrentJob::is_active()` predicate in Rust and use it here.
Regression test: A unit test in `torrent.rs`/`mobile_policy.rs` over a shared predicate: every status `TorrentManager` can write (`queued, resolving, publishing, checking, downloading, creating, seeding`) is active; `paused, completed, error, cancelled` are not.

### ACQ-10 — A fully downloaded single-stream part cannot be finalised: real mirrors answer the end-of-file range with a 416 that carries no validator
Severity: Low        Category: reliability
Platforms: all
Evidence class: reproduced for the server behaviour (all six Kiwix mirrors returned `416` with `Content-Range: bytes */N` and no `ETag`/`Last-Modified` to `Range: bytes=N-` plus a matching `If-Range`); source-traced for the client
Location: /home/user/kiwix-tauri/src-tauri/src/acquisition/transfer.rs:287-300 (`!validators_match(&snapshot, response.headers())`)  [+ /home/user/kiwix-tauri/src-tauri/src/acquisition/persistence.rs:37-47 (`durable_total` returns `None` when `!segmented`), :241-253; test fixture at /home/user/kiwix-tauri/src-tauri/src/acquisition_tests.rs:1788]
Trigger: A non-segmented transfer (file < 4 MiB, or after `ranges_rejected`) with every byte on disk, then pause or kill during verification. Also Retry after a checksum failure.
Causal path: Resume sets `downloaded = part_length = total` and sends `Range: bytes={total}-`. The 416 has no ETag, `validators_match` is false, and the result is `Err("416 response does not prove that the partial file is complete")`. The row is `failed` and every Retry repeats. A server that ignores the range instead returns 200 and the whole file is fetched again. The existing test passes only because its fixture adds `ETag` to the 416. Exposure is small because range-capable servers normally take the segmented path, which finalises locally.
Smallest correct fix: In `execute_transfer`, before the request loop: if `!snapshot.segmented && downloaded > 0 && known_total == Some(downloaded)`, `break 'transfer` straight to the local magic/SHA-256/size verification. That is no weaker than the existing trust in file length for mid-file resume, and `verifying` is only persisted after `sync_all`.
Regression test: `acquisition_tests.rs`: `seed_partial` with a complete part and a server that answers 416 without validators (or refuses connections). Assert `complete` with no payload request.

### ACQ-11 — The OPDS display length is enforced as an exact size for non-Metalink acquisition links
Severity: Low        Category: reliability
Platforms: all (third-party OPDS sources only; the official catalog always supplies `.meta4`)
Evidence class: source-traced (rounding premise reproduced: the official feed advertises `length="17439744"` for a 17 439 698-byte file, i.e. rounded up to a KiB multiple)
Location: /home/user/kiwix-tauri/src/components/Catalog.tsx:418-422 (`expectedSize: entry.size`)  [+ /home/user/kiwix-tauri/src-tauri/src/acquisition/manager.rs:776-777, :875; /home/user/kiwix-tauri/src-tauri/src/acquisition/transfer.rs:345-351; /home/user/kiwix-tauri/src-tauri/src/acquisition/finalize.rs:31-37]
Trigger: Add an HTTPS OPDS source whose entries link straight to `.zim` with a rounded `length`, and press Download.
Causal path: `start_transport` stores `expected_size = Some(entry.size)` (only the Metalink path replaces it). The first 200 response has the true `Content-Length`, and `expected != actual` gives "Download size differs from expected size: A versus B". The row fails and Retry repeats. `catalog.rs:751-752` states the intent ("OPDS display lengths can be rounded, and must not reject a correctly sized archive response"), which holds only for Metalink links.
Smallest correct fix: Treat a catalog-supplied size as an estimate for Internet starts: use it for `check_file_size`/`check_space`/`job.total` admission but store `expected_size: None` unless it came from a Metalink or a shared-library grant, which stay exact. This touches the `start_download` `expectedSize` contract; the shared-library path is unaffected.
Regression test: `acquisition_tests.rs`: start with `expected_size` a few hundred bytes above the served `Content-Length` on a direct `.zim` URL and assert `complete`. The shared-library exact-size test must still reject a mismatch.

**Covered** (read end to end unless noted)
- `acquisition.rs`; `acquisition/{manager,transfer,segments,finalize,persistence,http_meta}.rs`; `transfer_policy.rs`; `transfer_response.rs`; `partial_donor.rs`.
- `torrent/manager.rs`; `torrent/web_seed.rs` (non-test part); `torrent/storage.rs` lines 185-470; `torrent.rs` lines 980-1240 plus outline; vendored librqbit `initializing.rs` around `ensure_file_length`.
- `ios_background.rs`; `mobile_service.rs` transfer sections (1000-1100, 1570-1640, 1796-1850, 1885-2096) with the related call sites in `service.rs`, `mobile.rs` and `service/dispatch.rs` (`start_download`, pause/resume/cancel, `catalog`, the settings auto-resume block).
- `catalog.rs` (`http_client`, `parse_catalog`, official torrent/magnet resolution); `catalog_cache.rs` fetch/request path (315-546).
- `Downloads.tsx`, `Catalog.tsx`, `download-state.ts`; `errorText`/`jobStatus`/`terminalStatus` in `UI.tsx`.
- State machines traced: HTTP start → pause → resume → cancel → retry; kill and relaunch; segment restart; 416/429/5xx/short body/wrong length; finalize and sweep; torrent pause/resume/cancel/suspend and web-seed→engine handoff.
- `docs/reviews/2026-11-23` and `2026-11-25`, and `docs/STATE_MACHINES.md` §2-3.

**Not covered**
- `torrent/publication.rs`; most of `torrent.rs` (load/validate/cleanup-entry code); `torrent/storage.rs` outside 185-470.
- `catalog_index.rs` (only `edition_date`); `catalog_cache.rs` persistence/scratch half (550-952). I found nothing meeting the bar in the catalog fetch path itself: offline falls back to a stale cached page with a visible notice, and bodies are capped at 16 MiB.
- `android_notifications_service.rs` and `live_activity.rs` (not read); the rest of `mobile_service.rs`; `mobile_policy.rs`; the Swift/Kotlin native side of the iOS background and Android foreground-service contracts.
- Test modules: only test names and the 416 fixture in `acquisition_tests.rs` were read; `torrent_tests.rs`, `catalog_cache_tests.rs` and `catalog_index_tests.rs` were not.
- Nothing was executed on a device or under cargo. ACQ-3 and ACQ-9 are in cfg-gated code that could not be type-checked here.
- Noted but not reported as findings: a catalog magnet with zero peers stays at "Finding torrent peers" with no timeout or hint, although the official magnet carries `xs=`/`ws=` fallbacks the app ignores; cancelling a torrent keeps its staging directory until "Delete download", whereas HTTP cancel removes the part.