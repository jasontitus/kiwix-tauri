## Verdict for lane transfers: only non-blocking findings

I found no data-loss, hang or compile defect in the lane. The ACQ-1 in-place repair holds up against the attacks in the brief. Two defects are worth fixing before release: R1 and R2 are interactions between ACQ-10 and ACQ-11 on the direct-HTTP path, and R3 is a stall in the new web-seed retry loop. The owner may want to raise R1 to Blocking, because it publishes a truncated archive as "complete".

### TRANSFERS-R1 — ACQ-11 removes the only truncation guard for direct catalog links whose response has no Content-Length
Severity: Should-fix (borderline Blocking: a "complete" download can be a truncated archive)
Fix under review: ACQ-11 (and the ACQ-10 doubt)
Evidence class: source-traced
Location: `src-tauri/src/acquisition/transfer.rs:111` (first run clears `job.total`), `:383`, `:695`, `:731`; `src-tauri/src/acquisition/manager.rs:880`
Trigger:
1. Start a direct `.zim` link from a third-party OPDS feed that supplies a `length`.
2. The server answers `200` with neither `Content-Length` nor chunked encoding (an HTTP/1.0 or `Connection: close` body), or a middlebox cuts the stream with a clean FIN.
Causal path:
- `expected_size` is now `None`, and the first run sets `stored.job.total = None`.
- At `:383`, `content_length.or(snapshot.expected_size).or(snapshot.job.total)` is therefore `None`. The comment there still says this fallback stops "a short chunked body [being] adopted as a complete archive".
- The body loop ends on EOF, and `:695` sets `stored.job.total = Some(downloaded)`.
- The only local checks are length ≥ 80 bytes and the `ZIM\x04` magic at `:731`. There is no SHA-256 for direct links and no `expected_size`.
- `publish_verified_download` publishes the file and the row shows `complete`.
- Before the change, the catalog estimate made this case fail with "Download ended before the expected object size".
- No ZIM MD5 or layout check runs anywhere on this path. `archive::validate_layout` (`checksum_pos + 16 == size`) only runs when the book is opened later.
Smallest correct fix: before publication, read the 80-byte header. When `mime_list_pos != 72` (a checksum is present), require `checksum_pos + 16 == downloaded`. Do the same in `finalize.rs:~288-319`. This cheap check also closes the ACQ-10 coincidental-size doubt, because an old prefix almost never has a matching header.

### TRANSFERS-R2 — With ACQ-10, Retry after a local "not a ZIM" or "too short" failure never contacts the server again
Severity: Should-fix
Fix under review: ACQ-10
Evidence class: source-traced
Location: `src-tauri/src/acquisition/transfer.rs:164` (`complete_part`), `:725-732`
Trigger:
1. A mirror answers a direct link with a full `200` that is not a ZIM, for example a maintenance or "soft 404" HTML page carrying a Content-Length.
2. The row fails with "Downloaded resource is not a ZIM archive (invalid magic)".
3. The mirror is fixed and the user presses Retry.
Causal path:
- `job.total` holds that response's Content-Length and the part holds that many bytes.
- `complete_part` is true, so the worker hits `break 'transfer` and the magic check fails again, with no request.
- Every Retry repeats this, and the message does not say to cancel.
- Base, with no validator, reset to byte 0 and refetched, so Retry could recover.
Smallest correct fix: decide `complete_part` only when the part's first 4 bytes are the ZIM magic, and with R1's header check only when that check passes. Otherwise fall through to the existing path (no validator → `downloaded = 0`, or If-Range).

### TRANSFERS-R3 — The web-seed stall counter can be reset forever by tiny progress, and peers wait the whole time
Severity: Should-fix
Fix under review: ACQ-4
Evidence class: source-traced
Location: `src-tauri/src/torrent/web_seed.rs:489` (`stalled = if state.advanced { 0 } …`), `:561-565`, `:461`
Trigger:
- A broken or hostile mirror sends a few bytes on each connection, then closes or resets it.
- Or the URL-list hosts are black-holed (SYN dropped), as on a censored network.
Causal path:
- Any pass that raises `state.sequential` by even one byte sets `advanced` and resets `stalled` to 0, then waits 1 s.
- The mark is monotone and bounded by `seed.total`. With 1 byte per pass on a 90 GB torrent, that bound is about 9×10¹⁰ passes, so in practice there is no limit.
- `manager.rs:1990` runs the stage before the engine add, so the peer swarm never starts. Base gave up after 3 attempts.
- For black-holed mirrors, each pass pays the 90 s connect timeout per mirror. With 3 mirrors, five stalled passes cost about 5×270 s + 15 s ≈ 23 min before peers on every start (base: about 13.5 min). The implementer's "about 15 s" excludes this.
- The repair path is fine: `advanced` there needs a whole piece to verify, so it is bounded by piece count.
Smallest correct fix:
- Count a pass as advancing only if it raised the mark by at least `min(piece_length, remaining)`, or put an absolute cap on passes or stage duration.
- Shorten `connect_timeout` (e.g. 15–20 s) while keeping the 90 s `read_timeout`.

### TRANSFERS-R4 — `docs/WEB-SEEDS.md` now contradicts the code
Severity: Should-fix (documentation claim; required by AGENTS.md)
Fix under review: ACQ-1, ACQ-4
Evidence class: source-traced
Location: `docs/WEB-SEEDS.md:31-36` (Flow steps 4–5), `:57-59` (Residual limits)
Trigger: anyone reading the docs.
Causal path:
- The doc still says a verification failure "discards the staged bytes and retries one from-scratch acquisition" and that a corrupt file is "re-downloaded from zero".
- The code now repairs failing pieces in place, retries with backoff for five stalled passes, and hashes the full file on every start.
- The implementer's handoff #2 was not applied: `git diff d98cac2 -- docs/` shows no WEB-SEEDS.md change.
Smallest correct fix: apply the implementer's handoff text, plus the connect-timeout cost from R3.

### TRANSFERS-R5 — The IPv4 fallbacks treat any socket error as "no IPv6"
Severity: Nit
Fix under review: OWN-1
Evidence class: source-traced
Location: `src-tauri/vendor/librqbit-dualstack-sockets/src/socket.rs:110` (`Err(error) => { return match addr {…`), `src-tauri/src/torrent/manager.rs:19` (`dual_stack_available`)
Trigger: EMFILE/ENFILE/ENOBUFS at the moment of the IPv6 `Socket::new` on a host that has IPv6.
Causal path:
- The vendored arm falls back on any `Socket::new` error.
- If IPv4 creation then succeeds (an fd was freed in between), the UDP tracker socket is IPv4-only for the session, and IPv6 trackers fail silently.
- The app probe likewise picks `ipv4_only` for the engine's lifetime (until a suspend replaces the cell).
- Under real fd exhaustion the IPv4 bind also fails, so this is not harmful in practice. EACCES-only-for-IPv6 sandboxes do benefit from the fallback.
Smallest correct fix: fall back only on `EAFNOSUPPORT`/`EPROTONOSUPPORT` (`WSAEAFNOSUPPORT` on Windows), in both places.

### TRANSFERS-R6 — The ACQ-4 cancellation assertion cannot fail
Severity: Nit
Fix under review: ACQ-4
Evidence class: source-traced
Location: `src-tauri/src/torrent/web_seed.rs`, test `an_unreachable_web_seed_fails_after_its_bounded_retries_and_stops_when_cancelled` (second half)
Trigger: n/a.
Causal path: with `SEED_BACKOFF` = 10 ms in tests, a non-cancellable sleep would also finish, and `download_once` then returns `Ok(())` because the token is cancelled. The test passes whether or not the wait is cancellable.
Smallest correct fix: give the test a long backoff (for example a parameter or a large `SEED_BACKOFF` override), then assert that `download` returns well before that backoff ends.

**Per-fix assessment**
- **ACQ-1: fixes it.**
  - The stage runs to completion before `add_on_torrent_runtime` in the same worker.
  - Earlier generations' engine storage is fenced: `ConfinedFactory.stop` is the cancelled worker token, and `pwrite_all`/`ensure_file_length`/`pread_exact` bail on it.
  - Pause awaits retirement and `storage_gate.write()` while holding `shutdown_gate`, and resume needs the same gate. So an abandoned add cannot write while a new stage runs.
  - The range arithmetic is correct: last short piece, pieces spanning files, padding counted as zeros and never fetched, zero-length files skipped, and `file_ranges` offsets.
  - A mirror that ignores Range is streamed one chunk at a time, bounded by the file length, and dropped once the last needed byte is written. A `206` is checked only for its start; extra bytes are ignored, and a wrong total or wrong content just fails SHA-1.
  - The hash runs in `spawn_blocking`, stops at the next piece after a pause or cancel, and is awaited rather than detached.
  - Nothing unverified is counted: `Ok` needs every piece to pass SHA-1, the engine re-hashes (`fastresume: false`), and publication still uses `PieceReceipt`.
  - The cost is a full SHA-1 pass by the stage plus the engine's full check on every start of an unverified full-length file, with no progress shown during the stage's pass.
  - The `current > file.length → set_len(0)` truncation is pre-existing.
- **ACQ-2: fixes it.** `short_bodies` is never reset and is still capped at 8, the same bound as before for early-ending 206s. A server sending one byte per connection is bounded only by forward progress, the same as the single-stream path.
- **ACQ-3: fixes it.** I read the iOS branch character by character. `start_official_catalog(entry.url, directory, None, Some(entry.size).filter(|size| *size > 0), acknowledgement)` matches the signature, and `entry.name` is still read elsewhere, so there is no dead-code warning. Not compiled for iOS.
- **ACQ-4: fixes it but** see R3, plus R6 for its test. Every wait is cancellable. The offset is re-read after every mirror attempt.
- **ACQ-6: fixes it.**
  - "Nothing durable" comes from `resume_offset`: the part file's length for single-stream, the journaled plan for segmented. Neither can lag behind durable state in an unsafe direction.
  - A preallocated segmented part with zero verified bytes gives `downloaded == 0`. The request then has no Range, the `200` truncates before writing, and an unsolicited `206` is refused. So no kept byte is ever appended under another mirror's validator.
  - `(used + 1) % len` is only computed when `position` finds the mirror, so there is no division by zero.
- **ACQ-9: fixes it.** The one token in `mobile_transfer_update_locked` keeps `foreground_transfers` set while a share is `creating`. The Android type-check is clean.
- **ACQ-10: fixes it but** see R2. The ACQ-10 doubt is not closed: no MD5 or layout check runs before publication (R1's fix would close it).
- **ACQ-11: fixes it but** see R1.
  - FAT32 and free-space admission still run up front in `start_transport` from `expected_size`, so too-large downloads are refused before the part is reserved.
  - The worker re-checks against the response total.
  - The iOS native path keeps the estimate in `job.total` for `admit_native_transport` and no longer sends it to Swift as `expected_size`.
  - The Downloads total and Android notifications show "unknown" from worker start until the first response.
  - Legacy rows with an estimate in `expected_size` behave as before.
- **OWN-1: fixes it.**
  - The probe runs once per engine creation, a cheap synchronous call on the runtime. A suspend replaces the cell, so the probe runs again.
  - The vendored fallback applies only when `Socket::new` fails and the address is `[::]` with `request_dualstack`. A specific IPv6 address still errors, and the recursion is bounded because the IPv4 address does not match the arm.
  - On hosts where IPv6 works, nothing changes (R5 aside).
  - The UDP tracker parses peer lists by the reply's address family, so an IPv4 socket gets IPv4 compact peers, which is correct.
  - An IPv6-only (NAT64) host can create IPv6 sockets, so neither fallback triggers there.
  - The `docs/FORKS.md` paragraph is accurate.

**Implementer's doubts**
- **ACQ-10 residual:** real but needs an exact size coincidence. ACQ-11 makes the related truncation case reachable (R1). The header check would close both, so it should be fixed now along with R1.
- **ACQ-6 rotation at 0 bytes, including a manual pause:** acceptable. No bytes are at risk, and it is never worse than a dead pinned mirror.
- **Dead mirror delays peers by about 15 s:** understated. Per R3, it is about 7.5 min per black-holed mirror per start. Fix now.
- **Wrong-content mirror refetched for up to five passes:** acceptable. The repair loop's progress needs a verified piece, so it is bounded.
- **`dual_stack_available` treats any error as "no IPv6":** acceptable as a Nit (R5).
- **Clippy on the test code:** not re-run by me for host tests. The Android lib clippy passes `-D warnings`.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- torrent:: acquisition::` → `test result: ok. 210 passed; 0 failed; 3 ignored; 0 measured; 1229 filtered out; finished in 12.54s`.
  - It includes `ok` for all five OWN-1 tests: `cancel_and_suspend_do_not_wait_for_blocked_torrent_threads`, `pause_does_not_wait_for_a_blocked_worker_unpause`, `shared_library_archive_downloads_from_magnet_in_second_profile`, `web_seed_only_torrent_completes_through_the_engine` and `pause_does_not_wait_for_blocked_torrent_threads_and_reaps_the_abandoned_add`.
  - It also includes all 12 `web_seed::` tests and every new acquisition test.
- `android-clippy.sh`:
  - First run: exit 101 on one error only, in `src/maps/manager.rs:803` (another lane mid-edit).
  - Retry: exit 0, `Finished dev profile` with no errors, so `mobile_service.rs` type-checks and lints clean for aarch64-linux-android.
- Read the registry `librqbit-tracker-comms-9.1.0/src/tracker_comms_udp.rs:270-280`, which confirms the hard-coded `[::]` bind.

**What I could not verify**
- iOS compile of the ACQ-3 branch (read only).
- Any host that has IPv6, and Windows or macOS sockets.
- The multi-file, padding-file and zero-length repair paths at runtime: no test covers them, and I checked them by tracing only.
- Multi-GB hash time, and hangs from synchronous `sync_all` on large files on the async runtime (a pre-existing pattern, also present in base).
- A real close-delimited truncated response (R1 is traced, not reproduced).
- Device or UI behaviour of the Downloads total, notifications and Live Activity.

My scratch logs are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-transfers/` (`test1.log`, `android.log`, `android2.log`, `web_seed.diff`).