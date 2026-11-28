# Lane `transfers` — update after adversarial review (changes since the first report only)

All six review items are fixed and exercised. `torrent:: acquisition::` is 217 passed, 0 failed, 3 ignored; host clippy exits 0; Windows-gnu clippy exits 0 (see the note under Commands on that run).

Diff against base: `git diff d98cac2 -- <files below>`. No state-changing git was run.

---

### TRANSFERS-R1 — a truncated archive could be published as complete (BLOCKING)
Disposition: **fixed**

Verification of the finding: confirmed by reading and reproduced. With the fix removed, a close-delimited 200 cut at half length ends as `complete` (observed).

How the rule was chosen: `archive::validate_layout` rejects `mime_list_pos != 72 && checksum_pos.checked_add(16) != Some(size)`. zimru's `Header::parse` reads `mime_list_pos` at bytes 56..64 and `checksum_pos` at 72..80. The new check uses those offsets and that exact rule; `localswarm/mod.rs`, `sharing_download.rs` and `library_files.rs` already read the same offsets the same way. I did not call `Header::parse` itself, because it also rejects unknown major versions, which would be a new and unrelated refusal.

Change:
- `acquisition/finalize.rs`: new `zim_length(header, length)` returning `Whole`, `EndsEarly` or `Mismatch`, and two message constants.
- `acquisition/transfer.rs` (single-stream and segmented completion): the 80-byte header is read where the 4-byte signature was. The check runs before the receipt shortcut and before hashing, so no path reaches publication without it.
- `acquisition/finalize.rs::finalize_durable_download` (startup and sweep finalize, native adoption): same check, before the receipt shortcut.

Messages (new, shown on the Downloads row; neither contains a word the frontend `errorText` rewrite matches):
- Ended early: "The download ended before the whole archive arrived. Retry to fetch the rest; if it stops early again, cancel it and start a new download"
- Any other mismatch, and every mismatch on a fully journaled row: "The downloaded file is not a complete ZIM archive: its size does not match the size its own header gives. Cancel it and start a new download"

Making Retry work: when a non-segmented download ends early, `job.total` is cleared. That total was only the byte count of the cut response; left in place, Retry failed with "Resumed object total changed" (with a validator) or "exceeds its declared length" (without one).

What guarantees the published file: every publication path now requires the ZIM signature and, when the header has a checksum field, a file length of exactly `checksum_pos + 16`, in addition to the existing size and SHA-256 checks.

This also closes my earlier ACQ-10 doubt about a size coincidence: an old prefix would need a header that declares exactly its own length.

Tests:
- `a_download_that_ends_before_its_own_header_says_is_refused_and_retry_fetches_the_rest` — with and without a validator: refused, nothing published, part kept; Retry completes (by `Range` + `If-Range`, or by a fresh request). Observed failing without the fix.
- `a_whole_archive_delimited_only_by_the_connection_closing_is_published`.
- `a_durable_row_whose_bytes_disagree_with_their_zim_header_is_not_published_on_load` (finalize path, no hash pass spent).
- `a_zim_header_measures_the_file_it_begins` (unit, including the early-header case and an overflowing position).
- The `zim_bytes` fixture now writes `mime_list_pos` and `checksum_pos`, so every existing fixture is a coherent header. No existing test needed another change.

Not exercised: a real middlebox-truncated response; iOS native adoption (the finalize check is covered only by the host test above).

Side effect: the first part of a split archive (`.zimaa`) fetched by direct URL is now refused, where it used to be published. The later parts already failed the signature check, so split sets never arrived this way.

---

### TRANSFERS-R2 — Retry after a non-ZIM response never contacted the server
Disposition: **fixed**

Change: `acquisition/transfer.rs` — a part counts as complete only when it is at least 80 bytes, starts with the ZIM signature and passes the R1 check. Anything else takes the normal request path (If-Range with a validator, full request without one).

Tests: `retry_asks_the_server_again_after_a_mirror_served_a_page_instead_of_the_archive` — a 200-byte page with a validator, the same without one, and a 20-byte page ("too short"). Each fails first, then Retry completes with exactly two requests. Observed failing without the fix. `a_complete_single_stream_part_is_verified_and_published_without_a_request` still passes.

---

### TRANSFERS-R3 — tiny progress could reset the web-seed stall counter forever
Disposition: **fixed**

Change: `torrent/web_seed.rs`
- A pass counts as advancing only if it added at least `min(piece_length, bytes remaining)` front to back, or made at least one piece verify in repair.
- `SEED_MAX_PASSES = 32`: the stage ends after 32 passes whatever they add. This is the absolute bound.
- `SEED_STALL_LIMIT = 120 s` (beyond what was asked): the stage also ends once consecutive stalled passes have spent that long waiting on mirrors. Without it, mirrors that connect and never answer would cost five full read timeouts each. Time spent in the full hash pass is not counted, so a restart on a large file still gets its retries.
- `connect_timeout` is 15 s; `read_timeout` stays 90 s.

Tests (first two observed failing without the fix):
- `drops_that_add_less_than_a_piece_still_count_as_stalls` — one byte per connection ends after five requests; the bytes are kept.
- `the_stage_ends_at_its_pass_limit_however_much_each_pass_adds` — a piece per connection ends at exactly 32 requests.
- `drops_that_each_add_a_whole_piece_do_not_use_up_the_stall_limit` — replaces the earlier three-bytes-per-drop test, which this rule correctly makes a stall.

Not exercised: the 120 s limit and the 15 s connect timeout (no test waits that long); they are covered by reading.

---

### TRANSFERS-R4 — `docs/WEB-SEEDS.md` contradicted the code
Disposition: **fixed**

Change: Flow steps 4–5, the Verification list and Residual limits now describe in-place repair, the retry and stop rules, the timeouts and the full hash on every start. The delay figures are stated as derived from the timeouts, not measured. I also recorded that the multi-file and padding repair paths have no host test.

---

### TRANSFERS-R5 — the IPv4 fallbacks treated any socket error as "no IPv6"
Disposition: **fixed**

Change:
- `src-tauri/vendor/librqbit-dualstack-sockets/src/socket.rs`: the fallback now also requires `address_family_unsupported(&error)`. The crate has no `libc` dependency but already depends on `nix` on Unix, so I used `nix::errno::Errno::{EAFNOSUPPORT, EPROTONOSUPPORT} as i32` rather than raw numbers (the values differ between Linux and Apple). Windows compares `raw_os_error()` with 10047. No dependency added; the crate still forbids unsafe.
- `torrent/manager.rs::dual_stack_available`: same rule, with `libc::EAFNOSUPPORT | libc::EPROTONOSUPPORT` on Unix and 10047 on Windows. The probe now only creates the sockets; a bind failure for any other reason is left for the engine to report.
- `docs/FORKS.md`: the paragraph now names the error condition.

Tests: the five OWN-1 tests still pass here (this kernel returns EAFNOSUPPORT).

Not exercised: a host that has IPv6; the Windows and Apple error values at runtime.

---

### TRANSFERS-R6 — the cancellation assertion could not fail
Disposition: **fixed**

Change: `download` now delegates to `download_with_backoff`. The test passes a 30 s wait, cancels after 100 ms and asserts the call returns in under 10 s.

Tests: `an_unreachable_web_seed_fails_after_its_bounded_retries_and_stops_when_cancelled`. With the wait made non-cancellable it failed with "stopping took 30.001371057s" (observed).

---

## Commands run (final result lines)
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- torrent:: acquisition::` → `test result: ok. 217 passed; 0 failed; 3 ignored; 0 measured; 1231 filtered out; finished in 13.14s`
- `cargo test … --lib -- service:: shared_library:: edition` (other modules that drive downloads, to check the header rule) → `test result: ok. 255 passed; 0 failed; 3 ignored`
- Revert run (R1, R2, R3 and R6 temporarily removed, then restored and re-verified) → `2 passed; 5 failed`; the five are the new tests named above
- Host clippy, `--lib --tests --features custom-protocol -- -D warnings` → `Finished`, EXIT 0
- Windows-gnu clippy (the brief's command) → `Finished … in 0.38s`, EXIT 0. Cargo found the target fresh: another session had compiled it at 00:40, after my last edit at 00:38 (I checked the artifact times against my files). So I did not watch that compile myself; the exit status and the absence of replayed warnings are the evidence.
- `node scripts/check-unsafe-census.mjs` → passes ("every item documented")

## Files changed since the first report
- `src-tauri/src/acquisition/finalize.rs`
- `src-tauri/src/acquisition/transfer.rs`
- `src-tauri/src/acquisition_tests.rs`
- `src-tauri/src/torrent/web_seed.rs`
- `src-tauri/src/torrent/manager.rs`
- `src-tauri/vendor/librqbit-dualstack-sockets/src/socket.rs`
- `docs/WEB-SEEDS.md`
- `docs/FORKS.md`

New user-visible strings: the two Rust messages under R1. `docs/ERRORS.md` will need regenerating if it lists backend messages. No frontend strings.

## Handoffs / contract changes
- None outstanding from the first report: the vendor fallback and the WEB-SEEDS text are now in the tree.
- A download whose length disagrees with its own ZIM header now fails before publication on every path, including shared-library and native rows.

## Doubts
- **Time lost to dead mirrors (corrected figure).** Peers wait for this stage. Derived from the timeouts, not measured:

  | Mirror behaviour | One mirror | Three mirrors | Base, three mirrors |
  |---|---|---|---|
  | Refuses, or device offline | about 15 s | about 15 s | instant |
  | Drops the connection attempt silently | about 1.5 min | about 2.3 min | about 13.5 min |
  | Connects, then never answers | about 3 min | about 4.5 min | about 13.5 min |

  DNS resolution time is not covered by the connect timeout.
- **A mirror that trickles data without ever closing** is still not bounded: the pass and stall limits apply between passes. This is unchanged from base.
- **After 32 passes on a peerless torrent**, the row waits until the next start for the web seed to be tried again; the bytes are kept.
- **R1 trusts the header's own `checksum_pos`.** A file cut short whose header was also altered to match is not caught here; without an advertised digest nothing can catch that.
- **`EPROTONOSUPPORT`** is included as instructed; I could not observe it on any host.