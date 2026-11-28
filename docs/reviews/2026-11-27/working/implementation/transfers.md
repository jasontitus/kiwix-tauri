# Lane `transfers` — report

All nine findings are confirmed real. Seven are fixed and exercised here, two are edited but not compilable here (ACQ-3, ACQ-9). **OWN-1 is only partly fixed: the app-level change you asked for is in, but the five torrent tests still fail in this container until one change is made in a vendored crate I was told not to touch.** I prototyped that change outside the tree and the five tests pass with it.

Diff against base: `git diff d98cac2 -- <files below>`. No state-changing git was run. No file outside the lane was edited.

---

### OWN-1 — torrent engine cannot start on a host without IPv6
Disposition: **partial + handoff** (app fix done; a second, unconditional `[::]` bind lives outside the app)

Verification of the finding: Baseline `cargo test --lib torrent::` gave `88 passed; 5 failed`, all five with `error starting listeners`. The container kernel has no `AF_INET6` at all (socket creation returns EAFNOSUPPORT for TCP and UDP). In the vendored sources:
- `ListenerOptions.ipv4_only` moves the TCP listener to `0.0.0.0` and turns off dual-stack.
- `SessionOptions.ipv4_only` moves the DHT socket to `0.0.0.0` and makes the connector and live torrent state skip IPv6 peers.

Change: `src-tauri/src/torrent/manager.rs`
- New `dual_stack_available()` creates and binds a `[::]:0` dual-stack TCP socket and a UDP one (the DHT is UDP), the same rule as the LocalSwarm listeners.
- `engine()` sets `ipv4_only` on both `SessionOptions` and `ListenerOptions` when the probe fails. It runs once per engine, not as a process-wide static. On hosts where IPv6 works nothing changes.
- `src-tauri/src/torrent_tests.rs`: `shared_library_archive_downloads_from_magnet_in_second_profile` dialled `::1` unconditionally; it now dials the loopback of the listener's own family.

What still fails: after the app fix the same five tests fail with `error creating UDP tracker client`. `Session::new_with_opts` (vendored `session.rs:768`) always builds `UdpTrackerClient::new`, and `librqbit-tracker-comms 9.1.0` (a registry crate, not vendored) hard-codes a `[::]` dual-stack UDP bind (`tracker_comms_udp.rs:272`) with no `ipv4_only` input. No session option avoids it.

**Handoff (exact change):** in `src-tauri/vendor/librqbit-dualstack-sockets/src/socket.rs`, `MaybeDualstackSocket::bind`: when `Socket::new` fails and the request is a dual-stack `[::]` wildcard, bind `0.0.0.0:<port>` instead. Every bind in the engine goes through this function, so one edit covers the tracker client.
- Ready patch (40 lines): `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/transfers/handoff/dualstack-ipv4-fallback.patch`
- Patched copy of the crate: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/transfers/handoff/librqbit-dualstack-sockets`

Tests (observed):

| Configuration | `torrent::` result |
|---|---|
| Base `d98cac2` | 88 passed, 5 failed (`error starting listeners`) |
| App fix only | same 5 fail (`error creating UDP tracker client`) |
| App fix + prototype patch, via `--config 'patch.crates-io.librqbit-dualstack-sockets.path=…'` (tree, `Cargo.lock`, target dir untouched) | **99 passed, 0 failed** |

All five named tests pass in the last row, with the final web-seed code.

Not exercised: the vendor patch without the app change; any host that has IPv6; Windows/macOS/Android.

---

### ACQ-1 — web-seed stage truncates a valid partial payload on restart
Disposition: **fixed differently**

Verification of the finding: confirmed by reading. The engine's `ensure_file_length` (and `prefill_from_donors`) leave files at full length; the old stage then fetched nothing, failed hashing, and `reset_files` truncated everything.

Why not the reviewer's fix: skipping the reset and handing over to the engine would mean the web seed can never contribute again once a file is full length. For a torrent with no peers (the case web seeds exist for) that turns "progress lost" into "stalled for good".

Change: `src-tauri/src/torrent/web_seed.rs`
- `reset_files` and the truncate-and-retry are removed.
- A full hash pass now returns the list of failing pieces instead of stopping at the first.
- `repair` fetches only those pieces' byte ranges, in place, mirror by mirror, then re-hashes only those pieces.
- `fetch_ranges` writes only requested bytes. A mirror that ignores Range has its whole-file answer read once, and only the missing ranges are kept from it.
- On any failure the files are left as they are.

What guarantees the published file:
- `download` returns `Ok` only when every piece passes SHA-1.
- On `Err` the engine opens the files with `fastresume: false`, and its initial check hashes every piece before counting any.
- Publication re-checks through the piece receipts.
- Bytes of a piece that verified are never rewritten; only ranges of pieces that failed SHA-1 are written.

Tests (all observed failing with the base implementation):
- `full_length_file_keeps_its_valid_pieces_and_fetches_only_the_others` — requests are exactly `5-9` and `15-15`; progress starts at the 10 valid bytes. Base requested the whole file.
- `failed_web_seed_leaves_a_full_length_file_as_it_was` — 503 mirror, file byte-identical afterwards. Base left an empty file.
- `repair_never_overwrites_a_piece_that_already_verified` — a mirror with different content in already-valid pieces, no Range support, one request.
- Existing `downloads_single_file_and_verifies_pieces` still passes.

Not exercised: manager-level pause→resume against a live engine (needs the OWN-1 handoff); multi-GB files; padding-file torrents on the repair path.

---

### ACQ-4 — web seed gives up after three instant attempts and re-requests from a stale offset
Disposition: **fixed**

Verification of the finding: confirmed by reading (no sleep; `current` refreshed only on `Ok`).

Change: `src-tauri/src/torrent/web_seed.rs`
- The offset is re-read from the file after every mirror attempt, so the next mirror continues from what is on disk.
- Errors are now classed: connection, timeout, 408/429/5xx and short bodies retry; other statuses, bad ranges and local I/O end the stage at once.
- Passes are retried after 1, 2, 4, 8 s, each wait cancellable by the job token; a pause or cancel returns immediately.
- A pass counts against the limit only if it raised no high-water mark. Five consecutive such passes end the stage (I chose 5, about 15 s, rather than more, so a dead mirror does not hold back a healthy swarm for long).
- Bounded: every non-counted pass raises a monotone mark that the torrent size limits.

Tests (observed failing on base):
- `the_next_mirror_continues_from_the_bytes_a_dropped_one_wrote` — second mirror is asked `6-`; base sent no Range.
- `drops_that_still_add_bytes_do_not_use_up_the_retry_limit` — base returned `Err`.
- `an_unreachable_web_seed_fails_after_its_bounded_retries_and_stops_when_cancelled`.

Not addressed: when the stage does give up, the manager still only logs and continues with peers, so a peerless row shows "Downloading, 0 B/s" with no error. That is a UI/status decision outside the reviewer's fix.

---

### ACQ-2 — segmented retry budget never resets
Disposition: **fixed**

Verification of the finding: confirmed; `retries` only grows, unlike `transfer.rs:543`.

Change: `src-tauri/src/acquisition/segments.rs::segment_worker`
- `retries = 0` whenever payload bytes arrive.
- A separate `short_bodies` counter, never reset and capped by the existing `SHORT_BODY_CONTINUES`, so a server that keeps ending ranges early without an error still cannot loop.
- Zero-progress failures are counted exactly as before.

Tests: `a_segment_survives_more_drops_than_its_retry_limit_while_each_adds_bytes` (9 mid-body drops on one range, about 9 s). Observed failing without the fix, by hitting the fixture's 60 s wait rather than by the row turning `failed` (that would take about 79 s of backoff). The fixture gained a `truncate_range` knob.

Not exercised: the companion "eight zero-progress failures still fail" case (about 79 s of real backoff); unchanged by construction.

---

### ACQ-6 — resolved Metalink mirror pinned for the life of the job
Disposition: **fixed, beyond the minimal fix, for the nothing-kept case; partial otherwise**

Verification of the finding: confirmed by reading.

Change:
- `acquisition/transfer.rs::execute_transfer`: a Metalink job with nothing durable (`resume_offset == 0`) resolves again at every worker start. It clears `etag`, `last_modified`, `segments` and `verification` (never `segmented`, so a preallocated part is still not read as progress).
- `acquisition/http_meta.rs`: `resolve_metalink` / `parse_metalink` take `after: Option<&str>` and pick the mirror listed after the one already used, wrapping around.
- If re-fetching the Metalink fails, the earlier mirror is kept, so this is never worse than before.

I did this in the worker rather than in `queue_resume` because plain re-resolution would usually return the same first-ranked mirror, so Retry would still fail.

Safety: once any bytes are kept the mirror stays pinned and is continued only with Range + If-Range under its own validator. I did not implement moving kept bytes to another mirror, because that would rely on a validator issued by a different server.

Tests:
- `a_metalink_retry_moves_to_the_next_mirror_only_while_nothing_is_kept` — first mirror 404, Retry completes from the second; with a kept 32 KiB part there is no re-resolution, no request to the other mirror, and the request carries `range: bytes=32768-` and `if-range`. Observed failing without the fix.
- `a_metalink_retry_continues_with_the_mirror_after_the_one_already_used` (unit).

Remaining:
- A mirror that dies after bytes are kept still needs Cancel plus a new start.
- The iOS native path (`execute_native_transfer`) is unchanged, because the native session checks `source_url` against `transfer_url`.

---

### ACQ-10 — complete single-stream part cannot be finalised on a validator-less 416
Disposition: **fixed**

Verification of the finding: confirmed by reading; the existing test passed only because its fixture added an ETag to the 416.

Change: `acquisition/transfer.rs` — when `!segmented && downloaded > 0 && known_total == Some(downloaded)`, the worker skips the request and goes straight to local verification.

What guarantees the published file:
- The same checks as every download: ZIM signature, SHA-256 against the Metalink hash when there is one, exact size against `expected_size`, identity before and after hashing.
- Segmented and preallocated parts are excluded (`segmented` is persisted before any `set_len`).

Tests:
- `a_complete_single_stream_part_is_verified_and_published_without_a_request` — also asserts a complete part with a wrong advertised SHA-256 is refused. Observed failing without the fix (hangs).
- `validated_416_publishes_already_complete_partial_file` now clears the known total so the 416 branch stays covered.

---

### ACQ-11 — OPDS display length enforced as exact
Disposition: **fixed** (Rust side; `expectedSize` contract unchanged)

Verification of the finding: confirmed; I checked every reader of `expected_size`.

Change:
- `acquisition/manager.rs::start_transport` stores `expected_size` only for shared-library grants. Internet starts store `None`; Metalink still sets it on resolution. The supplied size is still used for file-size and space admission and for the initial `job.total`.
- `acquisition/transfer.rs`: at the worker's first run on a non-Metalink URL, `job.total` is cleared when there is no exact size. Otherwise a chunked 200 would fall back to the estimate as a "previously learned total" and enforce it.

Tests: `a_rounded_catalog_size_admits_a_direct_download_without_rejecting_its_real_size` (Content-Length and chunked). Observed failing without the fix: `Download size differs from expected size: 98258 versus 98304`.

Side effects: the total is unknown from worker start until the first response. Rows persisted before this change with an estimate in `expected_size` behave as before.

---

### ACQ-3 — iOS names every official download `<name>.zim`
Disposition: **fixed — not compiled here**

Verification of the finding: confirmed. `start_official_catalog` passes the name to `start_transport`, which derives `filename(url)` when it is `None`, as other platforms do. `ios_background.rs` and `NativeRequest` carry no file name. `edition_service.rs:158` is a second caller (the finding said there was none); it passes its own name and is unaffected.

Change: `src-tauri/src/service/dispatch.rs`, iOS `start_download` branch — pass `None`. Three lines replaced, nothing else.

Tests: `a_download_started_without_a_name_keeps_the_flavour_and_edition_of_its_url` checks the naming rule on the host; it does not exercise the iOS branch.

---

### ACQ-9 — Android drops the foreground service while a share is "creating"
Disposition: **fixed — not compiled here**

Verification of the finding: confirmed by reading `share()`, `service.rs:2881` and `mobile_transfer_update_locked`.

Change: `src-tauri/src/mobile_service.rs` — `"creating"` added to the torrent active list. One token.

Kotlin / TS: I recommend no change. `NOTIFICATION_ACTIVE` (`src/download-state.ts`), `activeStates` (`DownloadNotifications.kt`) and the label map at `mobile_service.rs:1925` drive per-download notifications. They leave out `seeding` on purpose, and `creating` is the same kind of non-download row; adding it would show Pause/Cancel on a share being hashed and need a new label.

---

## Commands run (final result lines)
- Base, `cargo test … --lib torrent::` → `88 passed; 5 failed`
- Final tree, `cargo test … --lib -- torrent:: acquisition::` → `205 passed; 5 failed; 3 ignored` (the five are the OWN-1 tests above)
- Earlier in the same tree state: `web_seed::` → `12 passed; 0 failed`; `acquisition::` → `111 passed; 0 failed; 3 ignored`
- With the prototype vendor patch via `--config`, `torrent::` → `99 passed; 0 failed`
- Revert run (fixes temporarily removed, new tests kept, then restored and re-verified) → `6 passed; 10 failed`; every new behavioural test failed
- `cargo clippy … --lib --tests --features custom-protocol -- -D warnings` → lib target clean; lib-test fails on two lints in `sharing_interop_tests.rs:541` and `store_tests.rs:1385`, neither mine. Nothing reported in my files.
- `node scripts/check-unsafe-census.mjs` → fails: `src-tauri/src/lib.rs: 0 -> 1`, not mine. I added no `unsafe`.
- Not run: Windows-gnu clippy (no `cfg(windows)` code touched, though `dual_stack_available` compiles there too); the full suite.

## Files changed
- `src-tauri/src/torrent/manager.rs`
- `src-tauri/src/torrent/web_seed.rs`
- `src-tauri/src/torrent_tests.rs`
- `src-tauri/src/acquisition.rs` (one call-site argument)
- `src-tauri/src/acquisition/transfer.rs`
- `src-tauri/src/acquisition/segments.rs`
- `src-tauri/src/acquisition/http_meta.rs`
- `src-tauri/src/acquisition/manager.rs`
- `src-tauri/src/acquisition_tests.rs`
- `src-tauri/src/service/dispatch.rs` (iOS branch only)
- `src-tauri/src/mobile_service.rs` (one token)

New user-visible strings: none in the frontend. One new Rust message that only reaches the log: "Web seed file is not the size the torrent describes".

## Handoffs / contract changes
1. **Vendor patch for OWN-1** (above). Without it the five torrent tests keep failing here and BitTorrent stays broken on IPv6-less hosts.
2. **`docs/WEB-SEEDS.md` is now wrong** in Flow steps 4–5 and the second "Residual limits" bullet ("discards the staged bytes and retries one from-scratch acquisition", "re-downloaded from zero"). It should say:
   - failing pieces are re-fetched in place by byte range, and nothing is truncated;
   - a mirror without Range support is read once and only the missing ranges are kept;
   - transient failures retry after 1–8 s, and five passes that add nothing end the stage;
   - a full-length file is hashed in full on every start.
3. `parse_metalink` and `resolve_metalink` have a new last argument; all callers are in my files.
4. No JSON or IPC changes. The download-size mismatch error is no longer produced for direct links.

## Doubts
- **ACQ-10 residual:** if a non-Metalink object is replaced by one whose size equals exactly the bytes already kept, and the replacement's 200 response then fails local admission, the next resume would treat the old prefix as complete. No checksum exists for such jobs. This needs an exact size coincidence; I could not close it without a new persisted field.
- **ACQ-6 rotation** happens on every start with nothing kept, including a manual pause at 0 bytes, so such a job can move off the best-ranked mirror.
- **Web seed with a dead mirror** now delays peers by about 15 s of backoff (plus connect timeouts) on each start for connection-type failures; it was three instant attempts before. Definite refusals such as 404 fall through at once.
- **A mirror serving wrong content** can have its failing ranges fetched again on each of up to five passes when another mirror is failing transiently.
- **`dual_stack_available`** treats any socket error as "no IPv6" (as the LocalSwarm listeners do), so file-descriptor exhaustion at engine start would also select IPv4-only for that engine.
- Clippy over my test code ran only in passes that other lanes' lint errors ended; no finding was in my files, but the integration run should confirm.