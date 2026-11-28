## Verdict for lane net: only non-blocking findings

I found no blocking defects. All ten claimed fixes do what they say. I found four should-fix items: one regression on Windows, one regression for receipts from a sender that has disappeared, and two gaps where a test or a case does not match what the report claims. There are also some nits. Everything I ran passed: lane tests, both clippy runs, `tsc` and three browser runs.

Reviewed snapshot: `git diff d98cac2` over the lane's paths, plus the stated hunks of `dispatch.rs` and `types.ts`. I saved the diff at review start to `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-net/lane.diff`. At the end it was byte-identical, although HEAD moved from c6f6cd6 to d569b89 through checkpoint commits. `scripts/test-nearby-sharing-surfaces.mjs` is now tracked through a checkpoint commit. The other hunks in `dispatch.rs` (`edition_discard_original`, the move warning, the official catalog name) and in `types.ts` (MapJob, Edition) belong to other lanes and are not reviewed here.

### NET-R1 — Windows Hyper-V external switch: one-click Web sharing now refuses a working LAN
Severity: Should-fix
Fix under review: NET-1
Evidence class: inference (no Windows host). The classifier part is reproduced in a scratch copy at `adv-net/classify.rs`.
Location: `src-tauri/src/transfer_policy.rs` (`virtual_interface`, `VIRTUAL_WINDOWS` has "vethernet"); `src-tauri/src/localswarm/network.rs` (`lan_addresses`); `src-tauri/src/sharing.rs` (`auto_lan_binding`)
Trigger: A Windows PC with a Hyper-V *external* virtual switch. The host's only LAN address (for example 192.168.1.20) sits on "vEthernet (External Switch)". The person presses Enable web sharing.
Causal path:
- `reachable_addresses` now drops every name starting with "vethernet". I confirmed "vEthernet (External Switch)" gives lan=false.
- `auto_lan_binding` then finds no candidate. `routed.filter(|a| eligible(a) && available.contains(a))` cannot rescue it, because the routed address is not in `available`.
- The result is "Connect to a trusted local Wi-Fi or Ethernet network before sharing…".
- At base, `reachable_addresses` used only the macOS name list, so this address was chosen and the link worked.
- The failure is actionable (Advanced binding still lists it), but it is a regression for a working setup.
- The desktop transfer-policy fence already excluded vEthernet at base, so Nearby/LocalSwarm on such a host was not working before either.
Smallest correct fix: In `default_binding`, when `available` has no eligible address, accept `routed_local_address()` if it is private and present in `interfaces()`. The NAT "Default Switch" and WSL adapters never own the default route, and flagged tunnels are already excluded. Add a case to `one_click_binding_chooses_the_lan_another_device_can_reach`.

### NET-R2 — An old receipt for a sender that has left discovery is now admitted indefinitely
Severity: Should-fix
Fix under review: NET-9
Evidence class: source-traced
Location: `src-tauri/src/shared_library.rs` (`download`, `receipt_grant`, `ScopedDownloadGrant::check`)
Trigger:
1. Browse a LAN sender.
2. The sender presses Stop sharing, so mDNS removes the record and `peers.remove`.
3. More than 5 minutes later the receiver presses Download.
Causal path:
- `check()` only compares when the record exists: `if let Some(current) = discovery.selected_library_peer(id).await? { if !matches_peer … }`. An absent record returns `Ok`.
- At base, `RECEIPT_TTL` bounded that window to 300 s. After that, `download()` returned the clear REVOKED message ("Shared library changed or disconnected; browse it again").
- Now the receipt has no age limit, so `download()` admits a job.
- The job then fails on connect: an 8 s timeout per advertised address, and a failed job in the list instead of the actionable message.
- Authority is not widened. The peer identity, session and TLS authority are still pinned, so nobody else can serve the download. The loss is the actionable message.
- The other cases the brief asked about are covered by `require()`: privacy/read-only (`require_store`), local-policy revision (`require_local_since`), descriptor cancellation, and a changed record (`matches_peer`).
Smallest correct fix: In `download()` only (not in `check()`, which also guards running requests and would cut a download on a transient mDNS blip), return `REVOKED` when `grant.inner.discovery` is `Some` and `selected_library_peer(id)` returns `None`. Add that case to `an_open_list_outlives_its_age_but_never_its_grant`.

### NET-R3 — The CORE-3 regression test cannot detect the nesting on the path it names
Severity: Should-fix (test only; the code fix itself is correct)
Fix under review: CORE-3
Evidence class: source-traced
Location: `src-tauri/src/nearby_service.rs` (`starting_the_finder_under_a_held_lease_does_not_wait_behind_a_queued_writer`, `nearby_command_with`); `dispatch.rs` (`localswarm_discovery` arm)
Trigger: Either of these regressions still passes the test:
- reverting `let _mode = if leased { None } else { Some(self.privacy_gate.read().await) };` to an unconditional read; or
- switching the arm back to `nearby_command("nearby_discover", …)`.
Causal path:
- The test calls `nearby_discover_start` directly, which never touches the lease.
- On host, `nearby_command_with` returns at `if !cfg!(mobile)`, and `native_ready` is false, both before the lease line.
- The dispatch arm is `#[cfg(mobile)]`.
- So the test fails only if a read is put inside `nearby_discover_start`. It does not meet the brief's bar ("must fail if a second `privacy_gate.read()` is reintroduced on the leased path").
Smallest correct fix: Move the lease decision into a small helper that the test can drive on host, for example `fn discover_lease(&self, leased: bool) -> impl Future<Output = Option<RwLockReadGuard>>`. Alternatively, put the `!cfg!(mobile)` and `native_ready` early returns after the lease decision behind a `cfg(test)` seam. Then drive `nearby_discover_leased` under a held lease with a queued writer.

### NET-R4 — Windows VPN and VirtualBox adapters are still LAN candidates, and the routed preference now actively picks an unflagged VPN
Severity: Should-fix (partial fix of NET-1 on Windows; the report and test overstate it)
Fix under review: NET-1
Evidence class: classifier reproduced (`adv-net/classify.rs`); Windows flags inferred from `if-addrs-0.15.0/src/windows.rs`, where `is_p2p` is only `IfType` 23 or 131.
Location: `transfer_policy.rs` (`virtual_interface`), `sharing.rs` (`auto_lan_binding`)
Trigger:
- **VPN:** Windows with NordLynx, WireGuard-NT or OpenVPN TAP (friendly name such as "Ethernet 3") owning the default route with a private address. These are not point-to-point by IfType and no name matches, so they stay `lan=true`. `routed` equals the VPN address and is in `available`, so it wins over Wi-Fi.
  - At base, sorting usually picked a 10.x VPN too. Now it is chosen whenever it holds the route, even when the LAN sorts first.
  - The test's "VPN owns the default route" case only covers a tunnel flagged point-to-point, which is a Linux-style adapter.
- **VirtualBox:** "VirtualBox Host-Only Network" and Linux `vboxnet0` (192.168.56.1) are classified as LAN. The brief listed `vboxnet*`. The routed probe hides this only when a default route exists.
- **ZeroTier on Windows:** "ZeroTier One […]" is also classified as LAN.
- `wwp*` (Linux predictable WWAN names) is not in the cellular list.
Smallest correct fix:
- Add "vboxnet", "virtualbox host-only", "zerotier", "nordlynx" and "wwp" to the lists.
- Optionally, on Windows, do not let `routed` override the sort unless the routed adapter is IfType Ethernet or Wi-Fi.
- Narrow the claim in the report.

### NET-R5 — `pendingOnly` is checked and acted on under two separate state-lock acquisitions
Severity: Nit
Fix under review: NET-8
Evidence class: source-traced
Location: `nearby_service.rs` (`nearby_command_with`, `nearby_stop` branch)
Trigger: A connect task sets `current.descriptor = Some(..)` (line ~811, under the state lock but not the nearby gate) on another worker thread. This has to land between the `pending` check guard being dropped and `nearby_stop_owner` taking the lock again.
Causal path: `let state = …lock(); let pending = …descriptor.is_none(); if !pending {return}` is followed by a separate `self.nearby_stop_owner(owner).await`, which locks again. The window is microseconds. It can only end a connection that has just been established and has no accepted downloads, which matches the person's Close. Running downloads cannot be affected.
Smallest correct fix: Take the owner out of the slot under the same guard that checks `descriptor.is_none()`, or pass a predicate into `nearby_stop_owner`.

### NET-R6 — Small status and UI accuracy nits
Severity: Nit
Fix under review: NET-5, NET-2
Evidence class: source-traced
Location: `sharing.rs` (`status`, `interface_address_lost`); `SharingControls.tsx` (poll effect); `LocalSwarmControls.tsx` (`finderError`)
- **Wildcard share with only loopback left** (implementer doubt):
  - `binding_addresses` always appends 127.0.0.1 and ::1, so `hosts.current()` is never empty.
  - `urls`, the QR and the "Sharing is on" badge become loopback-only, with no warning.
  - This matches start-time behaviour.
  - Fix: set `addressLost: true` (or a `localOnly` flag) for a wildcard binding when every current host is loopback.
- **`addressLost` has no debounce:** one 2 s observation during a sleep/wake re-lease shows the alert for at least one 4 s poll. Requiring two consecutive absent observations would stop the flap.
- **Duplicate poll chains:** a visibility change from hidden to visible while a `sharing_status` request is in flight starts a second poll chain. All chains die the next time the page is hidden or unmounted, so nothing leaks after unmount, and the extra calls are cheap.
- **Finder error after turning Find nearby off:** a failed, cancelled finder stays in the consumer slot after the person turns Find nearby off, so "Finding devices over Bluetooth stopped… Try Bluetooth again" keeps showing.

### Per-fix assessment
- **NET-1:** fixes it for Linux, Android and Apple names, and for flagged tunnels.
  - Remaining issues are NET-R1 (Hyper-V external-switch regression) and NET-R4 (unflagged Windows VPN, VirtualBox, ZeroTier gaps).
  - `routed_local_address` sends no packet: a UDP connect to a literal address, synchronous, no DNS. It returns `None` with no IPv4 route, including IPv6-only.
  - The routed address is used only when it is already an eligible private candidate. With Android mobile data only, it is the rmnet address, not a candidate, so the result is an actionable error.
  - The transfer-policy fence is unchanged: `physical_interface` is exactly the old expression, and the old `network::physical_interface` list is a subset of `VIRTUAL`.
  - Hotspot exclusions are the old set plus `wwan`.
  - On Apple, no real LAN is point-to-point (only utun, ipsec, ppp, gif, stf). The only Apple names that change are `vmnet*`, which is now excluded, an improvement, and `ipsec*`, now excluded by its flag. `bridge100` and `pdp_ip` were already excluded.
- **NET-2:** fixes it. `discovery_failure` needs phase "failed" and a cancelled work token. A restart replaces the consumer owner (`nearby_discover_start` stops the old one), so no stale error survives a successful restart. The SUSPENDED code is filtered out.
- **NET-4:** fixes it.
  - Every explicit stop clears or avoids the message: `stop_library` clears it after the supervisor races; when stop wins, the supervisor sees `same == false`; `stop()` cancels the lifetime first and clears; Android `stop_sharing` takes the offer first.
  - The Android waiter cannot release a hotspot that a newer share is using: the new share holds `hotspot_use` until it is installed in the library slot, and the release re-checks `library_descriptor` under that lock. The publisher never sets `hotspot_claim`, so it cannot pin the hotspot.
  - The waiter holds an `Arc<Service>` until the descriptor is cancelled, which is bounded by the manager lifetime.
- **NET-5:** fixes it.
  - The server's `AcceptedAuthorities::check` and `SessionHosts` are untouched.
  - Status reads the same `SessionHosts` clone and the same TTL'd `live` Arc, through the same `access_url`. The links shown therefore equal the accepted host set, and the test asserts this.
  - The interface binding keeps its pinned address and adds `addressLost`.
  - The poll stops when the share stops (effect cleanup on `running`/`sessionId`) and while the page is hidden.
- **NET-6:** fixes it. The discovery mutex and `operation` lock are held across `is_browsing()` and `stop_browse()`, so a healthy browse cannot be stopped. On Apple, `browsing` is set synchronously inside the Browse command. The finished task is awaited, so nothing leaks. Side effect: with no advertisements, the mDNS daemon is shut down (5 s bound) and recreated.
- **NET-7:** fixes it.
  - It does not misfire after the person's own Cancel: the `action()` path clears `ownedSession`, and in-flight polls are discarded by `revision` and `pending`.
  - It does not misfire after a successful pairing: the backend keeps the session in phase "confirmed", and `finish_pair_error` keeps "confirmed".
  - On desktop it shows only the generic sentence.
- **NET-8:** fixes it. A race exists but is harmless (NET-R5). The follow-up `localswarm_discovery` goes through `run()`, which refuses in private or read-only mode, and the backend `allowed()` and `normal_writable` refuse it as well. It fires only if Bluetooth finding was on when the dialog closed.
- **NET-9:** fixes it, with NET-R2. `make_room` is bounded (at most 64 entries, O(n²) worst case). A running or paused download keeps its own grant clone, so eviction cannot affect it.
- **NET-10:** left as a residual. I agree it needs an owner decision.
- **CORE-3:** fixes it.
  - Callers: `localswarm_discovery` (one lease, from `operation`, then `nearby_discover_leased`); frontend `nearby_*` (lease-free early return at `service.rs:2548`, then one read in `nearby_command_with`); `shared_library_browse` (lease-free, `nearby_connect`, which returns before the lease).
  - No path takes the nearby gate and then waits on the privacy lease.
  - The test is weak (NET-R3).

### Implementer's doubts
- **NET-1, Apple flags and names:** acceptable. Nothing real on Apple is excluded. Unverified on a device.
- **NET-1, `wwan`:** acceptable. Also add `wwp` (NET-R4).
- **NET-2, publisher not in the top-level error:** acceptable. The split is correct and tested.
- **NET-4, single message sentence:** acceptable. The Android/iOS background silent stop predates this work.
- **NET-5, wildcard with only loopback:** a real but minor problem (NET-R6). Needs an owner decision on wording.
- **NET-5, polling while collapsed:** acceptable.
- **NET-6, real mDNS in a test:** acceptable. I ran it in `unshare -n` with loopback down and up: it passes in a few milliseconds and cannot hang (daemon shutdown is bounded at 5 s). It still fails without the fix, because the latch check comes before daemon creation. On a networked CI runner it does send real mDNS queries.
- **NET-8, race:** acceptable (NET-R5).
- **NET-9, `checked_sub` fallback:** acceptable.
- **Browser-test timing:** not reproduced. Three runs at load 3.2 to 4.2 all passed 3/3.

### Scope
No creep found in the lane files. The removed `network::physical_interface` and the newly un-cfg'd `virtual_interface` are within the NET-1 change.

### What I ran
- `cargo test --locked … --lib -- sharing localswarm shared_library transfer_policy hotspot nearby_service` → `test result: ok. 210 passed; 0 failed; 5 ignored`.
- The NET-6 and NET-5 tests from a copy of the test binary, in `unshare -n` with loopback down and with loopback up → 2 passed each time, in a few milliseconds.
- Host `cargo clippy --locked … --lib --tests --features custom-protocol -- -D warnings`:
  - First run failed only at `src/acquisition_tests.rs:4396` (another lane, mid-edit).
  - Retry → exit 0.
- `android-clippy.sh`:
  - First run failed only at `src/acquisition/transfer.rs:748` (another lane).
  - Retry → exit 0.
- `npx --no-install tsc --noEmit` → exit 0.
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-nearby-sharing-surfaces.mjs`, three times → 3 pass / 0 fail each time.
- A scratch copy of the classifier over the interface names in the brief → `adv-net/classify.rs`. These results are the basis of NET-R1 and NET-R4.

### What I could not verify
- No Windows, macOS, iOS or Android runtime, and no device. The Windows friendly names and IfType behaviour, the Apple interface table, the Android hotspot waiter at runtime, and real Bluetooth off or denied are all unverified.
- I did not re-run the full node suite. The implementer's two failures there (the `docs/ERRORS.md` and localization-manifest generated artifacts, which include this lane's 9 new strings and 1 new Rust string) are still an open handoff to the owner and block a green CI.
