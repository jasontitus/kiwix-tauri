# NET lane — round 3 report (cleanup placeholder item + NET-R7 + NET-R8)

All three items are fixed and verified. Final tree: lane tests 215 passed / 0 failed / 5 ignored; host, Windows-gnu and Android clippy all exit 0; `tsc` clean; nearby browser test 3/3. Every fail-without-fix mutation ran in an isolated overlay namespace, never in the shared tree.

---

### Cleanup placeholder — interrupted directory capture blocks cleanup for good (handed over from the files lane)
Disposition: fixed (with two conditions stricter than instructed, see below)

Verification of the finding: confirmed by reading the code. `exclusive_rename_fallback` does `mkdirat(0700)` then `renameat` for directories. A stop between the two leaves an empty directory at `.kiwix-localswarm-<id>.cleanup` beside the intact source. Both `erase_with_hook` and `terminal_owned_publications` then return "Both LocalSwarm cleanup and source names exist…" on every attempt.

Change, all in `src-tauri/src/localswarm/cleanup.rs`:
- New `discard_capture_placeholder(parent, name, source, saved)`. It returns true only after removing the placeholder. The placeholder qualifies only when the entry at the cleanup name, opened no-follow, is all of:
  - a directory;
  - owned by the effective uid and private (`mode & 0o077 == 0`);
  - not the journaled identity (`saved.owned.identity`);
  - empty.
- Removal is `parent.dir.remove_dir(name)`, which refuses anything that gained an entry since inspection, followed by `parent.sync()`. Any failure returns false, so the caller's existing error stands.
- Both sites call it when both names exist, then continue as if the capture had not happened. `erase_with_hook` falls into the normal capture path; `terminal_owned_publications` sets `captured = false`.
- The rename primitive is untouched; it does not adopt an empty directory.

Stricter than instructed, in two ways:
- **Source check:** the source must still be the journaled directory before anything is removed. Otherwise the state is not known to be an interrupted capture.
- **Privacy check:** the placeholder must also be private, using the same owner-and-0700 predicate `Directory::child` applies to every owned directory. The fallback creates it 0700.

Platform scope: the recovery is compiled only for Linux and Android, the only targets that have the two-step fallback. Elsewhere the helper is a stub returning false, so behaviour there is unchanged.

Tests: `localswarm::cleanup::tests::an_interrupted_capture_leaves_a_placeholder_that_cleanup_discards_and_nothing_else` (Linux/Android).
- Still refused, with the entry left in place and the archive bytes unchanged: a non-empty directory (at both sites), an empty 0755 directory, another user's directory (only when the test runs as root, as it does here), a symlink to an empty private directory, a plain file, and a correct placeholder beside a source that is no longer the journaled directory.
- Then the real case: the removal proof is granted again and the placeholder is gone; `delete_terminal` completes; the unrelated user archive is untouched.
- Observed failing in the isolated copy two ways: with the recovery disabled ("Both LocalSwarm cleanup and source names exist…"), and with the owner/private/source checks removed ("not private: proof was granted").

Not exercised: a real filesystem without RENAME_NOREPLACE and a real crash between the two syscalls; the test pre-creates the resulting state. Needs a FAT/exFAT or similar volume as the download folder on Linux or Android, with the process killed during capture.

---

### NET-R7 — Routed preference picks an unflagged Windows VPN
Disposition: fixed as specified

Change in `sharing.rs`: `auto_lan_binding(available, routed, switches, windows)` with `let prefer_routed = !windows;` gating the first arm only. `default_binding` passes `cfg!(windows)`. The classifier exclusions and the vEthernet host-switch fallback are unchanged on Windows. No IfType detection and no unsafe code.

Tests: `one_click_binding_chooses_the_lan_another_device_can_reach` gains:
- ("Ethernet 3", 10.8.0.6, routed) with ("Wi-Fi", 10.0.0.5): windows=true picks 10.0.0.5; windows=false keeps the routed preference (10.8.0.6).
- Two real LANs: Windows keeps plain order.
- WSL and NordLynx still excluded on both.
- The Hyper-V external switch still chosen with windows=true, and refused without a route.

Observed failing in the isolated copy with the gate forced on (`10.8.0.6` instead of `10.0.0.5`).

Residual, as you recorded: on Windows the routed tie-break between two real LANs is not used, and a VPN whose address sorts before the LAN is still picked, as at base.

Not exercised: a Windows host.

---

### NET-R8 — `localOnly` wording for an IPv6-only binding
Disposition: fixed by rewording (the flag is unchanged)

I kept `localOnly` set for an IPv6-only share with no IPv6 address. Its links really are loopback-only, so suppressing the flag would hide a share nobody else can reach. The alert no longer claims the device is offline.

Changed string (replaces the round-2 one):
- Old: "This device has no network connection, so only this device can open the links below. Connect to a trusted Wi-Fi or Ethernet network; sharing continues there without restarting."
- New: "No network address is available for this share, so only this device can open the links below. Connect to a trusted Wi-Fi or Ethernet network, or stop web sharing and choose a different “Listen on” option. Sharing continues on a new connection without restarting."

Also updated the comments in `sharing.rs` and `types.ts` to match. The browser test still matches on "only this device can open the links below" and passes.

---

## Commands run (final results)
- Lane tests: `cargo test … --lib -- sharing localswarm shared_library transfer_policy hotspot nearby_service` → `test result: ok. 215 passed; 0 failed; 5 ignored`.
- `cargo test … --lib -- localswarm::` → `124 passed; 0 failed; 5 ignored`. This ran after the cleanup change and before R7/R8, which do not touch localswarm.
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 0 on the 7th try. The six earlier tries failed only in other lanes' files: `store.rs:2456`, `crash.rs` (two `is_multiple_of` lints at 530 and 820, plus errors near 1184–1195), `service_tests.rs:6288`.
- Windows-gnu clippy → exit 0. It finished in under a second, so cargo judged the tree already checked; I did not force a recheck.
- Android clippy → six tries failed only in other lanes' files (`service.rs:1775`, `store.rs:2456`, `crash.rs`); a final run after those compiled → exit 0.
- `npx --no-install tsc --noEmit` → exit 0.
- `node --test scripts/test-nearby-sharing-surfaces.mjs` (with `KIWIX_TEST_CHROME`) → 3 pass / 0 fail.
- `check-unsafe-census.mjs` → 75/75 documented; no `unsafe` added.
- Isolated mutation runs: the cleanup test failed under both mutations; the one-click test failed under the R7 mutation. The shared tree was verified unmutated afterwards.

## Files changed this round
- `src-tauri/src/localswarm/cleanup.rs` (helper, two call sites, test)
- `src-tauri/src/sharing.rs`, `src-tauri/src/sharing_interop_tests.rs`
- `src/components/SharingControls.tsx` (one string), `src/types.ts` (comment only)

## Handoffs / contract changes
- **Owner:** regenerate locales for the one replaced string; the earlier `docs/ERRORS.md` and locale regeneration handoff still stands.
- No JSON contract change this round.
- **Files lane:** the cleanup recovery lives entirely in `localswarm/cleanup.rs`; `publication.rs` is untouched.

## Doubts
- **Cleanup, stricter conditions:** if a mount presents directories as non-private or under another uid, the placeholder is not discarded and the old error remains. LocalSwarm already refuses to own directories on such a mount, so I do not think the state can arise there.
- **Cleanup, proof path now mutates:** `terminal_owned_publications` is otherwise a read-only proof path and now performs one `rmdir` plus a sync in this recovery case. It runs under the operation try-lock and can only remove an empty directory; flagging it because it is a behaviour change on that path.
- **NET-R8 wording:** the new sentence names the "Listen on" control. That label is only visible with Advanced network settings open, which is also the only way to reach a wildcard binding.