# NET lane report (NET-1, 2, 4–10 and CORE-3)

Ten of the eleven findings are fixed and verified here; NET-10 is left as a documented residual. Everything was confirmed against the current tree before changing code. No device, Bluetooth, second machine or Apple target was available, so the mobile and Apple paths are type-checked or untested as stated per finding.

Final state: targeted host tests 210 passed / 0 failed / 5 ignored; host clippy clean; Android cfg clippy clean; `tsc` clean; real-browser surface tests 4/4. The full node suite has 2 failures, both stale generated artifacts (see Handoffs).

---

### NET-1 — One-click Web sharing binds a virtual/VPN/cellular adapter
Disposition: fixed

Verification of the finding: confirmed. `default_binding()` took the first private IPv4 from `reachable_addresses()`, whose name list was macOS-only, then sorted, so 10.x/172.x adapters beat 192.168.x.

What each caller of the three filters needs:
- **`network::reachable_addresses`** — callers are `sharing::default_binding`, `network::seed`, `network::seed_apple` (display hints), and `library_transport::listen` (Apple and non-Apple). All ask "which of my addresses can another LAN device dial?".
- **`transfer_policy::physical_interface` / `virtual_interface`** (desktop only) — one caller, `LocalBinding::is_on_link_peer`. It asks "is this peer on-link through a real LAN adapter?". The adapter half is the same question as above.
- **`hotspot::hotspot_candidate`** — one caller, `hotspot_address`. It asks "which interface is my own access point?" and ranks AP names. This is a different question and stays separate. It also excludes Wi-Fi Direct (`p2p`), which the LAN list must not: `p2p1` is a real Ethernet name under biosdevname.

Change:
- `transfer_policy.rs`: new `lan_interface(name, point_to_point, loopback)` is the single classifier; `physical_interface` now calls it. `virtual_interface` (list unchanged) is compiled on every target. New `cellular_interface` (`rmnet`, `ccmni`, `seth`, `ppp`, `v4-`, `clat`, `wwan`).
- Transfer-policy decisions are unchanged. The cellular list is deliberately not part of the locality fence; a test asserts this.
- `localswarm/network.rs`: new pure `lan_addresses(candidates)` over named `InterfaceCandidate`s; `reachable_addresses` feeds it from `if_addrs`. New `routed_local_address()` uses the UDP-connect/`local_addr` probe to 203.0.113.1:9; no packet is sent.
- `sharing.rs`: `auto_lan_binding(available, routed)` prefers the routed address only when it is a private candidate already in `available`. A routed address grants nothing by itself.
- `hotspot.rs`: uses the shared `cellular_interface` plus its own extras. The effective exclusion set is the same, plus `wwan`.
- Contract touched: addresses advertised over mDNS/BLE and in seed rows shrink to real LAN interfaces. On Apple the filter now also applies the point-to-point flag and the Linux/Windows name list.

Tests:
- `sharing::interop_tests::one_click_binding_chooses_the_lan_another_device_can_reach` — the reviewer's three cases plus tunnels by flag, `br-`/`virbr`/`vmnet`/`wwan`, two real LANs with and without a routed address, a VPN owning the default route, only-virtual adapters refused, and hotspot AP names kept.
- `transfer_policy::tests::one_classifier_decides_what_a_lan_interface_is`; `session_defaults_and_pin_validation…` extended.
- Observed failing without the fix, twice: classifier reverted gave `172.17.0.1` instead of `192.168.1.50`; route preference reverted gave `10.0.0.5`.

Not exercised: `routed_local_address()` against a real multi-homed machine, Windows friendly names at runtime, Android and Apple interface tables. A real test needs a Linux box with Docker, a Windows PC with WSL, and a phone with mobile data on: press Enable web sharing and open the link from a second device.

---

### NET-2 — Bluetooth off/denied fails with nothing shown
Disposition: fixed, structured differently from the reviewer's suggestion

Verification of the finding: confirmed. Both tasks record the failure only on the owner status; `nearby_status()` had no top-level `error` and the UI never read `publisher`.

Change:
- `nearby_service.rs`: `nearby_status()` adds top-level `"error"` from new `discovery_failure(owner)`. It reports the finder's own failure only (phase `failed` and its work token cancelled, which is exactly what the discover task leaves). The lifecycle `KIWIX_SUSPENDED` gate is never surfaced.
- The sharing failure stays on the already-emitted `publisher`. I did not fold it into the top-level error because the share card and the receive view need different sentences, and a failed connect to one sender must not appear as "finding stopped".
- `types.ts`: additive `PublicNearbyStatus.publisher`.
- `LocalSwarmControls.tsx`: share card shows the publisher failure with what still works and how to retry. The receive section shows the finder failure with a "Try Bluetooth again" button, which is an explicit `localswarm_discovery {enabled:true}`. The generic error line no longer reads `status.nearby.error`.
- The raw native code is passed through `errorText`; the mapping lives in the other lane's NET-3 code in `UI.tsx`.

Tests:
- `nearby_service::tests::bluetooth_failures_are_visible_in_status_and_a_failed_connection_stays_with_its_sender`.
- `…::a_finder_that_cannot_start_reports_why_instead_of_looking_forever` — the real discover task; on host the radio is always unavailable.
- Browser test "Nearby says when Bluetooth or sharing stopped…".
- All observed failing with the fix reverted.

Not exercised: real Bluetooth off or permission denied on iOS/Android. Needs two phones on different Wi-Fi: turn Bluetooth off on the sender and tap Share all, then on the receiver and open Nearby; check both messages and the retry.

---

### NET-4 — "Share all" stops silently; Android hotspot left up
Disposition: fixed, using a dedicated slot rather than `inner.error`

Verification of the finding: confirmed. I also found that on phones an explicit Stop sharing cancels the offer first (via `nearby_stop_owner`), so the supervisor would always record a reason before `stop_library()` runs. The fix handles that ordering.

Change:
- `localswarm/mod.rs`: new `Inner.library_stopped`, `note_library_stopped()` and `LIBRARY_STOPPED`: "Nearby sharing stopped because an archive in your library changed or the connection ended. Choose Share all to start again."
- Recorded in `supervise_library`, `quiesce_sources`, `revoke_invalid_sources` and the three `rebind_library_on_socket` failure paths. Not recorded when the manager lifetime is cancelled (Stop all, privacy cutover) or private/read-only mode is on.
- Cleared on a successful start, on `stop_library()` and on `stop()`.
- `library_status()` returns it as `librarySharing.error`, which the share card already rendered. The card now hides it while an operation is in flight, so an explicit stop on a phone does not flash it.
- `dispatch.rs`, `localswarm_share_all` success path, Android only: when this share used the hotspot socket, a waiter on `descriptor.cancel` calls `nearby_release_hotspot_if_unused()`. That release still keeps a hotspot that Web sharing, a receive or a newer share is using. I limited it to shares that actually used the hotspot so an unrelated LAN share ending cannot stop a hotspot the person started for something else.

Tests: `localswarm::tests::a_library_share_that_ends_by_itself_says_so_and_an_explicit_stop_does_not` covers archive unavailable, listener/route cancel followed by explicit stop, `quiesce_sources`, explicit stop, and Stop all. Observed failing with the note removed. Browser test asserts the card text.

Not exercised:
- The clear-on-start line; the test installs the offer without DNS-SD because the ordinary suite must not announce a library.
- The Android hotspot waiter is type-checked only. Needs an Android phone: Use temporary Wi-Fi, then remove a shared archive; the Temporary Wi-Fi card should disappear and a later plain Share all should use the LAN.

---

### NET-5 — Sharing addresses are a start-time snapshot
Disposition: fixed

Verification of the finding: confirmed. `status()` returned stored `urls`/`catalog_urls`; only wildcard `SessionHosts` refreshed, and only for the Host check.

Change:
- `sharing.rs`: `SharingServer` keeps the session's `SessionHosts` and a 2-second observation cache.
- Interface binding: `status()` adds `addressLost`.
- Wildcard bindings: `url`, `urls`, `catalogUrl(s)` and each book's `urls` are rebuilt from `SessionHosts::current()`.
- Loopback and hotspot are unchanged. `service.rs` and the dispatch arms needed no edit.
- `types.ts`: additive `SharingStatus.addressLost`.
- `SharingControls.tsx`: polls `sharing_status` every 4 s while sharing is on and the document is visible, renders from the observed status, shows the lost-address alert, and asks the page to refresh once if sharing ended elsewhere.

Authority after the change: unchanged. `AcceptedAuthorities::check` → `accepted_host` against `SessionHosts` still decides every request. Status only reads the same host list; an interface-bound share still answers only on its original address (asserted in the test).

Tests: `sharing::tests::status_reports_an_interface_binding_whose_address_left_this_device`, `…status_links_of_a_wildcard_binding_follow_the_addresses_the_server_accepts`, and the browser test "web sharing shows the addresses the device has now…". All observed failing with the fix reverted.

Not exercised:
- A real network change. Needs a laptop sharing with the default binding: switch Wi-Fi and the alert should appear within about 6 s; with "All IPv4 interfaces" the links should change.
- Not done: the non-Apple LocalSwarm mDNS A-records still carry listen-time addresses. The reviewer marked that as inference and it is outside the described fix.

---

### NET-6 — "Find nearby" can never restart after a discovery failure
Disposition: fixed

Verification of the finding: confirmed in both implementations (`browse.is_some()` latch in mdns; `failed` flag on Apple cleared only by `StopBrowse`).

Change: `LocalSwarmManager::discovery(true)` in `localswarm/mod.rs` calls `stop_browse()` first when `!is_browsing()`, then `start_browse()`. There is no retry loop. The reap's own error is ignored and the start reports the real state. `discovery.rs` gains a `#[cfg(test)]` hook `end_browse_for_test`.

Tests: `localswarm::tests::find_nearby_restarts_a_browse_that_ended_by_itself` (non-Apple). Observed failing with "Stop the failed discovery session before restarting" when the line is removed.

Not exercised: the Apple path was not compiled here; I did not edit `ios_discovery.rs`, the fix reaches it only through the shared manager call. Needs a Mac: deny Local Network, allow it in System Settings, press Find nearby.

---

### NET-7 — Pairing invitation cancelled silently on a phone
Disposition: fixed (frontend only; foreground-only policy kept)

Verification of the finding: confirmed. `mobile_note_inactive` → `retire_private_sync()` drops the session; the UI never compared `ownedSession` with status. I also found the owned session was never cleared after the person's own Cancel, so the detection had to distinguish that.

Change in `PrivateReadingSync.tsx`:
- `OwnedSession` now tracks expiry, last phase and whether the app was hidden.
- When a status refresh shows the owned session gone while it was waiting or verifying, an alert is shown: expired / hidden-or-interrupted (mobile only) / generic.
- The person's own actions clear it silently.
- On mobile, the copy notice and a hint under the QR say the invitation ends if the app is closed or hidden.

Tests: browser test "a pairing invitation that ends by itself is explained; cancelling it is not". Observed failing with the alert removed.

Not exercised: the mobile-only wording (the harness is not a native build) and real backgrounding. Needs a phone: create an invitation, switch apps, return.

---

### NET-8 — Closing the connect dialog does not cancel a pending join
Disposition: fixed, with a backend guard added

Verification of the finding: confirmed. `closeReceive()` only reset local state. I also confirmed that an unconditional owner stop would revoke running downloads from an already-connected sender, so the guard is in Rust rather than in stale frontend state.

Change:
- `nearby_service.rs`: `nearby_stop` accepts `pendingOnly: true`. It stops the owner only if it has no established descriptor, checked under the state lock; plain `nearby_stop` is unchanged.
- `LocalSwarmControls.tsx`: `closeReceive()` sends `nearby_stop {owner, pendingOnly:true}` when a Bluetooth library peer's browse is still pending and no list loaded. It then issues `localswarm_discovery {enabled:true}` so Bluetooth finding does not silently stay off.

Tests: `nearby_service::tests::abandoning_a_pending_connection_never_ends_an_established_one`; browser test asserts the call and arguments, the discovery restart, and no call after a loaded list. Both observed failing with the fix reverted.

Not exercised: the native join cancel. Needs two phones: choose a Bluetooth sender, close during "Joining…"; no join prompt should follow and the phone should stay on its own Wi-Fi.

---

### NET-9 — A list open for five minutes reports "changed or disconnected"
Disposition: fixed

Verification of the finding: confirmed at `download()`.

Change in `shared_library.rs`:
- `download()` no longer consults receipt age; new `receipt_grant()` returns the grant.
- New `make_room()` drops receipts whose grant is permanently retired. Only when the table is full (64) does it evict the oldest receipts past `RECEIPT_TTL`, so another window's browse no longer drops an old live list. A full table of recent lists is still refused as before.

Authority after the change: `ScopedDownloadGrant::check()`, called in `download()` before admission and again on every request. It covers private/read-only mode, the owner/descriptor/`revoked` tokens, the local-network policy revision, `descriptor.check()` and the discovery-record match.

Tests: `shared_library::tests::an_open_list_outlives_its_age_but_never_its_grant`. Observed failing (REVOKED) with the TTL check restored.

Not exercised: the real 5-minute wait end to end.

---

### NET-10 — `directConnection` has no control
Disposition: left as a documented residual; no code changed, field not removed

Why:
- Removing it would break existing profiles (`deny_unknown_fields`), as you noted.
- Adding the toggle would expose, blind, an Apple-only route preference (AWDL-first in `apple_transport::connect_impl`, direct-interface selection in `ios_discovery.rs`) that no user has ever been able to reach and that cannot be compiled or run here.
- The web harness cannot exercise an `isIOS || isMacos` gate either.

The decision for the owner is whether to add the toggle once it can be tested on two Apple devices.

---

### CORE-3 — `localswarm_discovery` takes the privacy read lease twice on mobile
Disposition: fixed

Verification of the finding: confirmed. `operation` gives the arm a read lease (it is not in the lease-free list), and `nearby_command("nearby_discover")` took `privacy_gate.read()` again.

Change:
- `nearby_service.rs`: the body moves to `nearby_command_with(command, args, leased)`.
- `nearby_command` (the lease-free entry from `operation`) still takes the lease itself.
- New `nearby_discover_leased` skips it.
- The start body is split out as `nearby_discover_start`, which never takes the lease.
- `dispatch.rs`: the `localswarm_discovery` arm now calls `nearby_discover_leased`.
- Lock order is unchanged: privacy read, then nearby gate.

Tests: `nearby_service::tests::starting_the_finder_under_a_held_lease_does_not_wait_behind_a_queued_writer` (hold the read lease, queue a writer, start the finder within 5 s). Observed timing out when the lease is put back in the start body.

Not exercised: the full mobile path is not reachable under `cfg(test)` on host, because `nearby_command_with` returns at `if !cfg!(mobile)` before the lease point. So the reviewer's second test (the public path takes exactly one lease) could not be written. The mobile path is type-checked only.

---

## Commands run (final results)
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- sharing localswarm shared_library transfer_policy hotspot nearby_service` → `test result: ok. 210 passed; 0 failed; 5 ignored`. An earlier run had one failure, `torrent::tests::shared_library_archive_downloads_from_magnet_in_second_profile`, which is not mine (it matched the filter by name) and passed in the final run.
- Revert experiments: round A 9/9 new tests failed, round B 2/2 failed; files restored from backups and verified by `diff`.
- `cargo clippy … --lib --tests --features custom-protocol -- -D warnings` → `Finished`, clean.
- `android-clippy.sh` → `Finished`, exit 0. I confirmed it really compiles mobile-only code by planting a deliberate type error (reported as E0308), then removing it.
- `npx --no-install tsc --noEmit` → exit 0.
- `KIWIX_TEST_CHROME=… node --test scripts/test-nearby-sharing-surfaces.mjs scripts/test-frontend-surfaces.mjs` → 4 pass / 0 fail.
- `node --test scripts/test-*.mjs` → 497 tests, 484 pass, 2 fail, 11 skip. The two failures are `docs/ERRORS.md matches…` and `every UI string… localization manifest`.
- `node scripts/check-unsafe-census.mjs` → 75/75, every item documented. No `unsafe` added.
- Windows cfg check not run: no `cfg(windows)` code touched.

## Files changed
Owned:
- `src-tauri/src/sharing.rs`, `sharing_tests.rs`, `sharing_interop_tests.rs`
- `src-tauri/src/shared_library.rs`, `hotspot.rs`, `transfer_policy.rs`, `nearby_service.rs`
- `src-tauri/src/localswarm/mod.rs`, `network.rs`, `discovery.rs`, `tests.rs`
- `src/components/SharingControls.tsx`, `LocalSwarmControls.tsx`, `PrivateReadingSync.tsx`, `localswarm-controls.css`
- new `scripts/test-nearby-sharing-surfaces.mjs`

Shared, allowed edits only:
- `src-tauri/src/service/dispatch.rs` — the `localswarm_discovery` arm and the `localswarm_share_all` success path.
- `src/types.ts` — additive `SharingStatus.addressLost` and `PublicNearbyStatus.publisher`.

`service.rs` and `scripts/test-frontend-surfaces.mjs` are untouched.

## New user-visible strings
Frontend, all through `t(...)`:
1. "This device is no longer connected at the address this share uses, so its link and QR code will not open. Reconnect to that network, or stop web sharing and enable it again to share on the current one."
2. "Devices on other Wi-Fi networks cannot find this share over Bluetooth: {reason} Devices on this Wi-Fi can still find it. To try Bluetooth again, stop sharing and choose Share all."
3. "Finding devices over Bluetooth stopped: {reason} Devices on this Wi-Fi can still appear below."
4. "Try Bluetooth again"
5. "The invitation expired before your other device connected. Create a new invitation."
6. "Connecting your devices was cancelled because this app was hidden or interrupted. Create a new invitation and keep this screen open on both devices until they connect."
7. "The invitation is no longer active. Create a new invitation to connect your other device."
8. "Invitation copied. Keep this app open: the invitation is cancelled if this app is closed or hidden. If you cannot paste it on your other device from here, scan the QR code instead."
9. "Keep this screen open until your other device connects. The invitation is cancelled if this app is closed or hidden."

Rust (shown untranslated in the share card): "Nearby sharing stopped because an archive in your library changed or the connection ended. Choose Share all to start again." I avoided the phrase "shared archive", which `user_error::plain` treats as internal vocabulary.

## Handoffs / contract changes
- **Owner:** regenerate `docs/ERRORS.md` and the locales (`node scripts/localization.mjs generate`). The two node failures above are exactly these and include other lanes' strings.
- **Owner:** the new browser test file needs `KIWIX_TEST_CHROME` like the existing ones; add it to CI if surface tests are listed explicitly.
- **UI lane (NET-3):** items 2 and 3 embed `errorText(code)` as `{reason}`. They rely on `nearbyCodeText` mapping the Bluetooth start codes (`BLE_POWERED_OFF`, `BLE_PERMISSION_DENIED`, `BLE_PERMISSION_TIMEOUT`, `BLE_UNSUPPORTED`, `NEARBY_BLUETOOTH_UNAVAILABLE`, `NEARBY_PERMISSION_DENIED`, `NEARBY_BLUETOOTH_UNSUPPORTED`, `NEARBY_LOCATION_SERVICES_REQUIRED`, `NEARBY_BLUETOOTH_START_TIMEOUT`, in bare and `[CODE] - CODE` forms) to full sentences ending in a period.
- **New JSON:** `sharing_status.addressLost` (interface bindings only); `localswarm_status.nearby.error` now produced (finder failure only); `nearby_stop` accepts `pendingOnly`; `librarySharing.error` can be set while `running:false`.
- **Behaviour:** `sharing_status` URLs for wildcard bindings now change over time. Advertised LocalSwarm/Nearby address lists exclude virtual, tunnel and cellular adapters.

## Doubts
- **NET-1, Apple:** `reachable_addresses` now applies the point-to-point flag and the Linux/Windows name list on macOS/iOS as well. It is platform-neutral Rust and compiled for Linux and Android, but not on Apple. I believe no real Apple LAN interface matches; unverified on device.
- **NET-1, `wwan`:** I added it to the cellular list beyond the hotspot list, as the Linux analogue of `rmnet`.
- **NET-2, publisher not in the top-level error:** this deviates from the reviewer's literal suggestion; reasons above.
- **NET-4, message accuracy:** the single sentence is slightly inaccurate when the share ended because the initiating window closed or the Bluetooth publisher failed while the app was not live. It is still true that sharing stopped and what to do. Android backgrounding (`stop_sharing`) and iOS `set_sharing_admission(false)` still stop silently; that predates this work and is outside the finding.
- **NET-5, wildcard with only loopback left:** the links simply become loopback, with no warning.
- **NET-5, polling scope:** the poll runs while Settings is mounted even if the Web sharing section is collapsed (one cheap status call per 4 s).
- **NET-6, real mDNS in a test:** the test starts a real mDNS browse (queries only, nothing advertised) on non-Apple hosts and tolerates machines that cannot browse. If the suite must stay completely off the network, mark it `#[ignore]`.
- **NET-8, race:** if the connection completes between the person's Close and the stop, it is kept (the previous behaviour). I chose never to revoke an established route.
- **NET-9, aged receipt in the test:** it uses `Instant::checked_sub` and falls back to "now" on a machine booted less than 10 minutes ago, where it would exercise less. Uptime here was sufficient and the failure-without-fix was observed.
- **Browser test timing:** one boot wait timed out once while the machine was under heavy load from other lanes' builds (load average about 8); it passed on every other run.