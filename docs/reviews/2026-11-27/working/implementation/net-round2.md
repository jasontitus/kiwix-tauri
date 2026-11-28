# NET lane — round 2 report (changes since the first report only)

All seven review items are addressed. NET-R1 and NET-R2 are fixed more narrowly than instructed, for reasons given under each; the Windows interface-type part of NET-R4 is skipped because `if-addrs 0.15` does not expose the type. Final state: lane tests 214 passed / 0 failed / 5 ignored; host, Windows-gnu and Android clippy all exit 0; `tsc` clean; browser tests 4/4.

**First, the clippy break you flagged:** `src-tauri/src/localswarm/mod.rs:24` `unused import: reachable_addresses` was mine. I introduced it this round when `default_binding` moved to `local_candidates`; a `cargo test` build only warns, so I missed it until the clippy pass. The re-export is removed and all three clippy targets are clean on the fixed tree.

Every fail-without-fix mutation this round ran in a private mount namespace (`unshare -m` with overlay mounts over `src-tauri/target`, `src-tauri/src` and `src/components`). Nothing mutated ever existed in the shared tree.

---

### NET-R1 — Hyper-V external switch regression
Disposition: fixed differently (narrower than instructed)

Verification of the finding: confirmed. `vEthernet (External Switch)` is dropped by the classifier, and the routed address could not rescue it because it was not in `available`.

Change:
- `transfer_policy.rs`: new `host_switch_interface(name)` (`vethernet*`).
- `localswarm/network.rs`: new pure `host_switch_addresses(candidates)` (not point-to-point, not loopback, on a host switch) and `local_candidates()`, which returns the LAN list and the switch list from one enumeration.
- `sharing.rs`: `auto_lan_binding(available, routed, switches)` gains a last fallback: when no LAN candidate is eligible, accept the routed address if it is private and on a host switch.

Why narrower: "routed, private and present in `interfaces()`" would also accept a full-tunnel VPN's private address when no LAN adapter has a private address (for example a machine on a public-IP network with `tun0`, or Windows with NordLynx/WireGuard-NT). `interfaces()` does not exclude tunnels, so that would replace today's actionable error with a dead link. Limiting the fallback to the one adapter kind that really holds a host's LAN fixes exactly the regression. The NAT "Default Switch" and WSL adapters share the name but never own the default route.

Tests: `one_click_binding_chooses_the_lan_another_device_can_reach` now covers:
- the external switch with the route chosen;
- no route, or a public route, refused;
- a real LAN adapter always winning over a switch;
- VPN, bridge and container names that own the route with no LAN present refused;
- a point-to-point "vEthernet" refused.

Observed failing in the isolated copy with the fallback disabled (`Err("Connect to a trusted…")` instead of `192.168.1.20`).

Not exercised: a real Hyper-V host. Needs a Windows PC with an external virtual switch: press Enable web sharing and open the link from a phone.

---

### NET-R2 — Old receipt for a sender that left discovery
Disposition: fixed differently (conditioned on the list's age)

Verification of the finding: confirmed. `check()` passes when the record is absent, and I had removed the only bound.

Change in `shared_library.rs`:
- `download()` now calls new `admit(receipt_id, book_id, now)`: `receipt_grant` → `check()` → REVOKED when the list is at least `RECEIPT_TTL` old and `sender_unlisted()` is true.
- `sender_unlisted()` is true only when the grant has a discovery record id and `selected_library_peer(id)` returns `None`.
- `check()` is untouched, so running downloads are not cut by a scan gap.

Why age-conditioned: refusing at any age would revoke an open list immediately on any mDNS gap and after "Stop finding". The base admitted both (the existing test's "Stop finding is not Stop all", and the frontend deliberately keeps a LAN list open when the entry is missing). That would be a new false "changed or disconnected", worse than NET-9. With the condition, base behaviour is restored exactly for "old and sender gone" (REVOKED), and the NET-9 fix holds for "old and sender still listed" (admitted).

Tests:
- New `an_old_list_is_refused_only_when_its_sender_is_no_longer_listed` covers: unlisted and recent admitted; unlisted and old refused with REVOKED; the grant not revoked by that refusal; relisted with the same identity admitted at any age.
- `an_open_list_outlives_its_age_but_never_its_grant` now goes through `admit` with an injected instant, so it no longer depends on machine uptime.
- Test hooks: `Discovery::discovered_for_test`, `LocalSwarmManager::discovered_for_test`.
- Observed failing in the isolated copy with the check disabled.

Not exercised: a real sender pressing Stop sharing followed by a 5-minute wait.

---

### NET-R3 — CORE-3 test could not fail on the leased path
Disposition: fixed (test now drives the real entries; small reordering in production code)

Change in `nearby_service.rs`: the lease decision (`let _mode = if leased { None } else { Some(read) }`) now sits before the `!cfg!(mobile)` and `native_ready` early returns. No `cfg(test)` seam is needed: the "exactly one lease" rule is the same code on every platform.

Behaviour effects:
- On desktop, the lease-free `nearby_discover` entry now takes one read lease briefly before returning status. It is harmless, and the desktop frontend never sends that command.
- On mobile only the order of the lease and the `native_ready` check changed.

Tests: new `the_leased_entry_takes_no_second_lease_and_the_public_entry_takes_exactly_one`. With a read lease held and a writer queued:
- `nearby_discover_leased(args)` must return within 5 s;
- `nearby_command("nearby_discover", args)` must not finish until the lease is released.

Observed failing in the isolated copy when the read is made unconditional. The earlier start-body test is kept.

Not exercised: the dispatch arm is `#[cfg(mobile)]`, so "the arm was switched back to `nearby_command`" still cannot be caught on host. It is type-checked for Android only.

---

### NET-R4 — Missing virtual/cellular names; Windows VPN claim
Disposition: partial (names added; interface-type gating skipped)

Change in `transfer_policy.rs`: `vboxnet`, `zerotier` and `nordlynx` added to the Linux-style list; `virtualbox host-only` to the Windows list; `wwp` to the cellular list.

Contract note: the virtual list is shared with the desktop peer-locality fence. A peer that is on-link only through a VirtualBox host-only, ZeroTier or NordLynx adapter is therefore no longer "local" on desktop. That is the fence's intended direction, but it is a transfer-policy change my first round avoided. You asked for it explicitly; flagging it so it is a known decision.

Windows interface type: skipped. `if-addrs 0.15` exposes only `name`, `addr`, `index`, `oper_status`, `is_p2p` and (Windows) `adapter_name`. `IfType` is read internally to compute `is_p2p` (types 23 and 131 only) and is not exposed. Gating the routed preference on Ethernet/Wi-Fi would need a new dependency or new unsafe code, so it is not done.

Correction to my first report: its statement that the fix covers "a VPN owning the default route" is true only where the tunnel is flagged point-to-point (Linux, macOS, Android) or its name is on the list. On Windows, WireGuard-NT and OpenVPN TAP adapters with a generic friendly name such as "Ethernet 3" are not flagged and match no name. They remain LAN candidates, and because the routed address is now preferred, such a VPN is chosen whenever it holds the default route, even when the real LAN sorts first. NordLynx, ZeroTier and VirtualBox are now excluded by name. This Windows gap remains open.

Tests: `tunnels_and_container_adapters_are_not_physical_lans` and `one_classifier_decides_what_a_lan_interface_is` extended; a "hosted" block added to the one-click test. Observed failing in the isolated copy with the names removed.

Not exercised: real Windows friendly names.

---

### NET-R5 — `pendingOnly` check and stop under two lock acquisitions
Disposition: fixed

Change in `nearby_service.rs`: `nearby_stop_owner` now delegates to `nearby_stop_owner_if(id, pending_only)`, which decides and takes the owner out of its slot under one state lock (`descriptor.is_none()` evaluated at removal). `pendingOnly` with no owner stops nothing.

Tests: the existing `abandoning_a_pending_connection_never_ends_an_established_one` still passes. The race itself is not reproducible in a test; the single-lock property is by construction.

---

### NET-R6 — Status and UI nits (all four)
Disposition: fixed

- **Wildcard with only loopback:** new additive field `localOnly`, emitted for wildcard bindings only. I did not reuse `addressLost` because the remedy differs: nothing needs restarting. `SharingControls` shows an alert. `types.ts`: additive `SharingStatus.localOnly`.
- **Debounce:** `addressLost` needs two consecutive missed observations a refresh interval (2 s) apart; one sighting clears it. With the 4 s UI poll the alert appears on the second poll after the address disappears.
- **Duplicate poll chains:** `SharingControls` has an in-flight guard; a visibility change during a request no longer starts a second chain.
- **Finder error after Find nearby is turned off:** `nearby_discover {enabled:false}` now removes a finder that had already ended (work token cancelled). A native error while reaping it is ignored, since it has nothing running.

Tests:
- Rust: `status_reports_an_interface_binding…` (debounce, clearing); `status_links_of_a_wildcard…` (`localOnly` equals "all accepted hosts are loopback"); `a_share_with_only_loopback_addresses_is_local_only`; `turning_finding_off_dismisses_a_finder_that_already_failed`.
- Browser: the web-sharing test now covers the `localOnly` alert and the single poll chain (six visibility events during a slow request add no requests).
- All observed failing in the isolated copy with each part reverted (the duplicate chain showed 14 requests against 8).

Not exercised: a real sleep/wake lease renewal; the finder dismissal on a phone (type-checked for Android only).

---

### Item 7 — Windows clippy at `sharing_interop_tests.rs:541`
Disposition: fixed

The `explicit_auto_deref` there was fixed late in round 1, and the helper is rewritten this round. Windows-gnu clippy `--lib --tests` now exits 0.

---

## Commands run (final results)
- Lane tests (`cargo test … --lib -- sharing localswarm shared_library transfer_policy hotspot nearby_service`) → `test result: ok. 214 passed; 0 failed; 5 ignored`. This ran before the one-line unused-import removal; it was not re-run after.
- Host `cargo clippy … --lib --tests --features custom-protocol -- -D warnings` → exit 0, on the 8th try. Earlier tries failed only in other lanes' files mid-edit: `store.rs`, `torrent/web_seed.rs`, `library_files_tests.rs`, `service.rs:211`.
- Windows-gnu clippy (the brief's command) → exit 0.
- `android-clippy.sh` → exit 0 on the fixed tree.
- `npx --no-install tsc --noEmit` → exit 0.
- Browser: `test-nearby-sharing-surfaces.mjs` plus `test-frontend-surfaces.mjs` → 4 pass / 0 fail.
- `check-unsafe-census.mjs` → 75/75 documented; no `unsafe` added.
- Isolated mutation run → 8 of 8 mutated Rust tests failed, 2 of 2 frontend mutations failed. The shared tree was verified unmutated afterwards.

## Files changed this round
- `src-tauri/src/transfer_policy.rs`, `sharing.rs`, `sharing_tests.rs`, `sharing_interop_tests.rs`, `shared_library.rs`, `nearby_service.rs`
- `src-tauri/src/localswarm/network.rs`, `mod.rs`, `discovery.rs`
- `src/components/SharingControls.tsx`, `src/types.ts` (additive `localOnly`)
- `scripts/test-nearby-sharing-surfaces.mjs`

No further edits to `dispatch.rs`.

## New user-visible string
- "This device has no network connection, so only this device can open the links below. Connect to a trusted Wi-Fi or Ethernet network; sharing continues there without restarting."

## Handoffs / contract changes
- **Owner:** regenerate locales and `docs/ERRORS.md`; one more frontend string was added this round.
- **New JSON:** `sharing_status.localOnly` (wildcard bindings only).
- **Changed behaviour:** `addressLost` is now delayed by one extra observation.
- **Transfer policy:** VirtualBox host-only, ZeroTier and NordLynx adapters are no longer a physical LAN for desktop peer locality (see NET-R4).
- **`nearby_discover {enabled:false}`** now removes an already-ended finder.

## Doubts
- **NET-R1 and NET-R2 deviate from the literal instruction.** If you prefer the literal versions, each is a one-line change, but I believe each would introduce the failure described above.
- **NET-R4, Windows VPN gap:** the unflagged, generically named Windows VPN case is still open and is now more likely to be picked when it owns the default route. Closing it needs the adapter type, which means a dependency or unsafe decision for the owner.
- **NET-R3, desktop lease:** the public `nearby_discover` entry now takes a brief read lease on desktop. I found no caller, but it is a behaviour change outside mobile.
- **NET-R6, detection latency:** the debounce makes loss detection about 8 s with the 4 s poll. That is slower than before, by design.