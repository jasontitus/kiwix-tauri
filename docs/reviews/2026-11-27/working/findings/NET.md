## NET review — peer-to-peer and local-network features (base d98cac2, read-only)

Ten findings, most severe first. No repository file was modified; the one scratch script is at `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/NET/errtext.mjs`.

### NET-1 — One-click Web sharing binds a virtual, VPN or cellular adapter, so the link and QR are dead
Severity: High        Category: usability
Platforms: Linux, Windows (source-traced); macOS with VMware `vmnet*`; Android with a private-IPv4 carrier (inference)
Evidence class: source-traced
Location: `src-tauri/src/sharing.rs:194-205` (`default_binding`, `auto_lan_binding`) + `src-tauri/src/localswarm/network.rs:1135-1155` (`reachable_addresses`, `physical_interface`) + `src-tauri/src/transfer_policy.rs:159-185` (`physical_interface`, `virtual_interface`) + `src-tauri/src/hotspot.rs:148-164` (`hotspot_candidate`)
Trigger: A Linux desktop with Docker (`docker0` 172.17.0.1), or a Windows PC with WSL2/Hyper-V (`vEthernet (…)` 172.x.x.1), or a 10.x VPN on `tun0`/`wg0`, on a 192.168.x.x LAN. The user presses "Enable web sharing" without ticking Advanced.
Causal path:
- `SharingControls.tsx:227-231` sends `binding` only when `advanced` is ticked, so `dispatch.rs:2846-2854` calls `default_binding()`.
- `reachable_addresses()` filters names with `VIRTUAL = ["utun","bridge","awdl","llw","lo","gif","stf","anpi","pdp_ip"]`, which are macOS/iOS names only. It then runs `addresses.sort()`.
- `auto_lan_binding` returns the first private IPv4 in that list. 10.x sorts before 172.16–31.x, which sorts before 192.168.x, so the Docker/WSL/VPN address wins.
- `binding_addresses` sets `local = vec![address]`, so `urls` has one entry and the QR (`sharing.urls[0]`) encodes `http://172.17.0.1:PORT/...`.
- The bind succeeds and the UI shows "Sharing is on", but no other device can reach it.
- The interface filter exists in three diverged copies. `transfer_policy::virtual_interface` already lists `docker`, `br-`, `virbr`, `veth`, `wg`, `tun`, `vethernet` and checks `is_p2p()`. `hotspot::hotspot_candidate` lists `rmnet`, `ccmni`, `tun`. `network::physical_interface` has neither.
- Secondary effect: the same unfiltered list is advertised for Nearby. `library_transport.rs:196-206` dials advertised addresses sequentially, with an 8 s timeout each (`network.rs:1003`), so every library request to such a sender first waits on the dead address.
Smallest correct fix:
- Make `network::reachable_addresses` use one shared classifier: `transfer_policy`'s name list plus `!is_p2p() && !is_loopback()`, plus the Android cellular prefixes from `hotspot.rs`. Delete the local copy.
- In `default_binding`, prefer the kernel's default-route source address when it is private and in the candidate list. A UDP `connect()` to a documentation address followed by `local_addr()` gives it without sending a packet; `connect_addresses` already uses this technique.
- Contract touched: the address list advertised over mDNS/BLE shrinks to real LAN interfaces.
Regression test: A unit test beside `session_defaults_and_pin_validation…` in `sharing_interop_tests.rs:535`. Feed named candidates `("docker0",172.17.0.1),("wlan0",192.168.1.50)`, `("vEthernet (WSL)",172.29.0.1),("Wi-Fi",192.168.1.50)` and `("rmnet_data0",10.12.3.4),("wlan0",192.168.1.7)`, and assert the Wi-Fi address is chosen each time.

### NET-2 — Bluetooth off or denied makes Nearby advertising and scanning fail with nothing shown
Severity: High        Category: usability
Platforms: iOS, Android
Evidence class: source-traced
Location: `src-tauri/src/nearby_service.rs:356-374, 378-385, 459-462` (`nearby_publish_library`, `nearby_status`, `nearby_command`) + `src/components/LocalSwarmControls.tsx:514-515, 550-556, 580` + `src/types.ts:521-532`
Trigger: Bluetooth is off, or the Bluetooth / Nearby devices permission is denied. The user taps "Share all" (sender) or opens Nearby (receiver), and the two devices are not on the same Wi-Fi.
Causal path:
- Native start rejects: `NearbyBluetooth.kt:155-157` gives `NEARBY_PERMISSION_DENIED` / `NEARBY_BLUETOOTH_UNAVAILABLE`; `NearbyBluetooth.swift:310-312` gives `BLE_PERMISSION_DENIED` / `BLE_POWERED_OFF`.
- Both Rust callers run in a spawned task after the command has already returned Ok. They only do `service.nearby_status_update(&task_work,"failed",Some(error))`.
- `nearby_status()` emits `{"supported","publisher","nativeReady","consumer","discovery","peers"}`. It has no top-level `error`.
- The UI reads `status.nearby?.error`, which is typed but never produced. It never reads `publisher`. It reads `consumer.error` only inside the modal when a Bluetooth peer is already selected.
- Result on the sender: "You're sharing your library" while nothing is advertised over Bluetooth. Result on the receiver: "Looking for nearby devices… Keep both apps open." indefinitely.
- The denial is only retried by toggling "Find nearby", and the UI gives no hint to do so.
Smallest correct fix:
- In `nearby_status()`, add `"error"` taken from `publisher.status.error` or, failing that, `consumer.status.error` when the phase is `failed`.
- Add `publisher` to `PublicNearbyStatus`.
- Render the publisher error inside the share card, e.g. "Bluetooth is off or not allowed; devices on other Wi-Fi networks cannot find this share."
- Normalise the code first (see NET-3).
Regression test: A `#[cfg(test)]` unit in `nearby_service.rs` that inserts a publisher owner, calls `nearby_status_update(.., "failed", Some("BLE_POWERED_OFF"))` and asserts `nearby_status()["error"]` is set. Add a Node case in `scripts/test-frontend-surfaces.mjs` asserting the alert renders when `status.nearby.error` is present and no peer is selected.

### NET-3 — Native error-code contract has drifted: Android codes arrive wrapped, about 90 codes are unmapped, and raw identifiers reach the user
Severity: Medium        Category: maintainability
Platforms: Android (wrapping); Android and iOS (unmapped codes)
Evidence class: reproduced for the UI half (ran the real `errorText` extracted from `UI.tsx`); source-traced for the wire format
Location: `src/components/UI.tsx:54-136` (`errorText`, `switch (message.trim())`) + `plugins/tauri-plugin-kiwix-mobile/android/.../NearbyNative.kt:18` + `plugins/tauri-plugin-kiwix-mobile/src/lib.rs:26` + `src-tauri/src/mobile_service.rs:2138` + `src-tauri/src/mobile_policy.rs:340`
Trigger: Any of these — an Android receiver's Wi-Fi join is declined or times out; "Use temporary Wi-Fi" is refused by Android; Wi-Fi drops on a phone during a shared-library download; iOS Bluetooth is off during a connect.
Causal path:
- `NearbyReply.reject(code)` is `invoke.reject(code, code)`.
- `nearbyBluetoothStart` and `nearbyJoin` are not in `ServiceEndpoint::owns` (`android.rs:60-77`), so they go through `run_mobile_plugin_async(..).map_err(|e| e.to_string())`.
- Tauri's `ErrorResponse` Display prints `[code] - message` (`tauri-2.12.5/src/plugin/mobile.rs:147-160`). `location_android.rs:163-166` documents and parses exactly this form.
- Rust therefore forwards `"[NEARBY_JOIN_TIMEOUT] - NEARBY_JOIN_TIMEOUT"`. The exact-match switch misses it. Reproduced: `errorText` returns that string unchanged, whereas the bare code maps correctly.
- `mobile_hotspot_start` prefixes `"Creating the Wi-Fi hotspot failed: "`, and `nearby_service.rs:940` prefixes `"The sender could not join the temporary Wi-Fi network: "`. Both defeat every mapped `HOTSPOT_*` / `JOIN_*` case.
- Comparing the codes produced in Kotlin, Swift and Rust against the switch leaves about 90 unmapped. Reproduced as shown verbatim: `BLE_POWERED_OFF`, `BLE_PERMISSION_DENIED`, `NEARBY_BLUETOOTH_UNAVAILABLE`, `MOBILE_LOCAL_NETWORK_BLOCKED`. Also unmapped by the comparison: `HOTSPOT_TETHERING_DISALLOWED`, `NEARBY_JOIN_LOST`, `JOIN_CANCELLED`.
- `UI.tsx:141` already tolerates `[code]` for one provider code, so the hazard is known but not applied to this switch.
- No test ties the lists together.
Smallest correct fix:
- In `errorText`, extract the code before the switch: accept `^\[(CODE)\]`, a trailing `- CODE`, or a `prefix: CODE` tail, using a `[A-Z][A-Z0-9_]{3,}` token.
- Add cases for Bluetooth off / denied / unsupported, local network lost, tethering disallowed, and join lost / cancelled.
- Add a default for any remaining `^(NEARBY|BLE|JOIN|HOTSPOT)_` token: a generic "Nearby could not connect. Try again." with the code kept for diagnostics.
- No Rust or native contract change is needed.
Regression test: `scripts/test-error-visibility.mjs` (it already extracts `errorText`). Assert the wrapped and prefixed forms map, and grep the Kotlin/Swift/Rust sources for `"(NEARBY|BLE|JOIN|HOTSPOT)_[A-Z_]+"` asserting none comes back unchanged.

### NET-4 — "Share all" stops itself silently when any shared archive changes; on Android the temporary hotspot is left up
Severity: Medium        Category: reliability
Platforms: all (silent stop); Android (hotspot left active)
Evidence class: source-traced
Location: `src-tauri/src/localswarm/library.rs:114-132, 220-229` (`Sources::principals`, monitor) + `src-tauri/src/localswarm/mod.rs:797-808, 843-864, 1482-1485` (`supervise_library`, `stop_library_locked`, `library_status`, `quiesce_sources`) + `src-tauri/src/service.rs:3466-3482`
Trigger: While sharing a library, one shared archive is removed, relinked, replaced by an edition update (`edition_service.rs:206,246`), flagged by the folder watcher (`service.rs:3118`), or becomes unavailable such as an unplugged drive. The same happens if the listener or route dies.
Causal path:
- The 500 ms monitor runs `principals()`, which requires `same_owner` (`sharing.rs:3140-3144`: `current.available` and unchanged path/installation) for every snapshot book. One failure cancels `sources.cancel`. Explicit mutations call `quiesce_sources`, which runs `stop_library_locked()` if the snapshot contains the book.
- `supervise_library` then runs `stop_library_locked()` and sets `library = None`.
- `library_status()` returns `Default` (`running:false, error:None`). No reason is written to `inner.error`.
- "Nearby sharing stopped; share your library again" exists only in `LibraryServer::status()` (`library.rs:201`), so it is visible only for the instant before supervision completes.
- The sender's card reverts to "Share your library" with no message, and receivers' downloads fail.
- On Android nothing calls `nearby_release_hotspot_if_unused()` on this path. It is only reached from explicit stop commands and `nearby_stop_owner`. The LocalOnlyHotspot and its "Temporary Wi-Fi" card stay active until "Stop all".
- A later plain "Share all" then silently reuses the hotspot socket (`nearby_library_socket` returns it whenever a hotspot is hosting), not the LAN.
Smallest correct fix:
- In `supervise_library` (the `same` branch) and in `quiesce_sources` / `revoke_invalid_sources`, when a running library offer is retired, set `inner.error` to an actionable sentence such as "Nearby sharing stopped because a shared archive changed or the connection ended. Choose Share all to start again."
- Clear it in `share_library_inner` on success. The UI already renders `status.error`.
- In the `localswarm_share_all` success path, spawn a waiter on `descriptor.cancel.cancelled()` that calls `nearby_release_hotspot_if_unused()`.
Regression test: `localswarm/library_tests.rs` or `localswarm/tests.rs`. Start `share_library`, mark one shared book unavailable in the Store, wait, and assert `status()["librarySharing"]["running"] == false` and `status()["error"]` is non-null.

### NET-5 — Sharing addresses are a start-time snapshot: desktop Web sharing keeps showing "on" with a dead link after the network changes
Severity: Medium        Category: reliability
Platforms: macOS, Windows, Linux (Web sharing); Windows, Linux, Android (LocalSwarm mDNS A records, inference)
Evidence class: source-traced
Location: `src-tauri/src/sharing.rs:1705-1708, 1791-1794, 1827-1845` (`urls`, `status`) and `:1921-1936` (`SessionHosts::new`) + `src-tauri/src/mobile_policy.rs:479-487` + `src-tauri/src/localswarm/library_transport.rs:335` + `src-tauri/src/localswarm/discovery.rs:257, 290`
Trigger: A laptop is web-sharing with the default (single-interface) binding. It then switches Wi-Fi, gets a new DHCP lease, or sleeps and wakes with a different address.
Causal path:
- The listener is bound to the old literal address. `status()` returns the `urls`, `catalog_urls` and `bindings` captured at start.
- Only mobile retires a LAN-bound share on network loss (`mobile_network_reconcile_locked`). No desktop code observes interface changes; a search for path-monitor or address-change hooks under `src-tauri/src` and `src-tauri/native` returned nothing.
- The UI therefore shows "Sharing is on" and the old link and QR indefinitely.
- For wildcard bindings, `SessionHosts::current()` refreshes the accepted Host set every 2 s so the server answers on the new address, but the displayed `urls` still never change.
- LocalSwarm on non-Apple platforms likewise registers mDNS records with the literal IPs taken at listen time.
Smallest correct fix:
- In `Service::sharing_status()`, recompute the hosts via `binding_addresses(None, &binding)`. For wildcard bindings, rebuild `urls` and `catalogUrls`. For an Interface binding whose address is gone, add `"addressLost": true`.
- Have `SharingControls` re-poll `sharing_status` every few seconds while running and visible, and show "This device's network address changed. Stop web sharing and enable it again."
- Additive JSON field; `SharingStatus` in `types.ts` needs the optional member.
Regression test: `sharing_tests.rs`. Construct a `SharingServer` status with `SharingBinding::Interface{address}` for an address absent from `interfaces()` (for example 192.0.2.9 via a test-only constructor) and assert `addressLost`. For a wildcard binding built with `SessionHosts::new_expired`, assert the URLs follow the live host list.

### NET-6 — After a discovery failure, "Find nearby" can never restart discovery
Severity: Medium        Category: usability
Platforms: macOS (Local Network permission denied; inference that the denial arrives as browse error -65570), Windows/Linux (browse task ended)
Evidence class: source-traced
Location: `src-tauri/src/localswarm/ios_discovery.rs:449, 463, 518, 603` (`Command::Browse`, `failed`) + `src-tauri/src/localswarm/discovery.rs:175-177` (`start_browse`) + `src/components/LocalSwarmControls.tsx:584-588`
Trigger: macOS Local Network access is denied on first use, the user then allows it in System Settings and presses "Find nearby". Alternatively, any browse-level error or traffic-limit stop on any desktop.
Causal path:
- A browse error runs `Kind::Browse => { self.stop_browse(); self.failed = true; }`, and the error text is shown.
- `is_browsing()` is now false, so the button renders "Find nearby" and sends `{enabled: !status.discovery}`, which is `true`.
- The backend answers `Err("Stop the failed discovery session before restarting")`.
- Only `Command::StopBrowse` clears `failed`, and the UI cannot send `enabled:false` while `status.discovery` is false.
- The mdns-sd implementation has the same latch: `if self.browse.is_some() { return Err("Stop the failed…") }` when the task has finished.
- The only exit is the footer "Stop all", which also stops sharing and pauses transfers. No message points there.
- iOS probably escapes via its background retirement, and on phones a live Bluetooth scan makes the toggle read "Stop finding"; I did not verify either on a device.
Smallest correct fix: In `LocalSwarmManager::discovery(true)` (`mod.rs:741-747`), if `!discovery.is_browsing()`, call `discovery.stop_browse().await` before `start_browse()`. That reaps the finished task and clears `failed`. An explicit user request is the intended reset, so no automatic retry loop is introduced.
Regression test: Non-Apple, in `discovery.rs` tests: install an already-finished `browse` handle, call `start_browse` through the manager path, and assert it does not return the "Stop the failed…" error. Apple, in `ios_discovery.rs` tests: after `Create::Failed{code:-65570}`, a second Browse command must be attempted, not rejected by the latch.

### NET-7 — Private device pairing invitation is cancelled silently when a phone leaves the foreground, although the UI tells the user to send it
Severity: Medium        Category: usability
Platforms: Android, iOS
Evidence class: source-traced
Location: `src-tauri/src/mobile_service.rs:309-314, 1777` (`mobile_note_inactive`, background retirement) + `src-tauri/src/private_sync_service.rs:58-67, 126-131` + `src/components/PrivateReadingSync.tsx:218, 277-290`
Trigger: On a phone, "Create private invitation" then "Copy invitation" (notice: "Invitation copied. Send it only to your other device."). The user switches to another app to send it, then returns.
Causal path:
- Leaving the foreground calls `retire_private_sync()`, then `manager.cancel()`, which takes the session and calls `session.retire()` (token and invitation dropped).
- On return, the status poll rebinds a fresh runtime and returns `session: null`.
- The UI shows "Create private invitation" again with no notice. `ownedSession` is never compared with the status.
- On the other device the pasted invitation fails; the accept side returns "No active private connection invitation" and closes the channel.
- The foreground-only policy is deliberate. The silent loss, and copy that invites the user to leave, are the defect.
Smallest correct fix:
- In `PrivateReadingSync`, when a refresh returns no session while `ownedSession.current` is set and the user did not cancel, clear it and show a notice: "The invitation was cancelled because this app left the foreground. Create a new one and keep this screen open; scan the QR code from your other device."
- On mobile, change the copy notice to say the invitation ends if this app is closed or hidden.
- No backend change.
Regression test: A `scripts/test-frontend-surfaces.mjs` case. Fake `private_sync_pair_start` returning a session and then `private_sync_status` returning `session:null`, and assert the notice renders.

### NET-8 — Closing the connect dialog does not cancel a pending Bluetooth / Wi-Fi join
Severity: Low        Category: usability
Platforms: iOS, Android
Evidence class: source-traced
Location: `src/components/LocalSwarmControls.tsx:243-252, 641` (`closeReceive`) + `src-tauri/src/service/dispatch.rs:2121-2142` (`shared_library_browse`) + `src-tauri/src/nearby_service.rs:806-818`
Trigger: The user chooses a Bluetooth-discovered sender, sees "Joining the sender's network…", and presses Close.
Causal path:
- `closeReceive()` only bumps local revisions and clears state. The frontend never issues `nearby_stop`; `api.ts` only lists it.
- The in-flight `shared_library_browse` keeps driving `nearby_connect`, then `nearbyJoin`.
- The system join prompt can still appear after the dialog is gone. On success the phone stays on the sender's temporary network, and the consumer keeps `descriptor` and `join_lease`, until "Stop all", backgrounding, or another connect.
- The code comment about retaining accepted connections covers accepted work, not a connect the user abandoned.
Smallest correct fix: In `closeReceive()`, when `pendingPeerRef.current` is a Bluetooth library peer and no download was accepted from it, call `api("nearby_stop", { owner: status.nearby.consumer.owner })`. `nearby_command` already supports that owner-scoped stop. Then let discovery restart on the next poll or intent.
Regression test: A Node surface test asserting `nearby_stop` is invoked with the consumer owner when the modal closes during a pending Bluetooth browse, and not when an inventory was already loaded with an accepted download.

### NET-9 — A shared-library list left open for five minutes reports "changed or disconnected" on Download
Severity: Low        Category: usability
Platforms: all
Evidence class: source-traced
Location: `src-tauri/src/shared_library.rs:24, 31, 160-163` (`RECEIPT_TTL`, `REVOKED`, `download`)
Trigger: The receiver browses a sender's library, waits more than 300 s (reading the list or watching a first download), then presses Download or Download all.
Causal path: `if receipt.created.elapsed() >= RECEIPT_TTL { return Err(REVOKED) }` returns "Shared library changed or disconnected; browse it again" although nothing changed. The grant itself is still valid (`grant.check()` is what enforces authority). Retrying "Download all" fails identically until the user finds "Refresh" at the bottom of the modal.
Smallest correct fix: Do not expire a receipt whose grant still passes `check()`. Keep the TTL only for pruning in `browse_peer`'s `retain` (skip receipts whose grant is still held by a job), or refresh `created` on each successful `download()`. If expiry is kept, return a distinct "This list is out of date. Refresh it and try again."
Regression test: `shared_library.rs` tests using `test_download_grant`. Insert a receipt with `created` older than the TTL and a live grant, and assert `download()` does not return `REVOKED`.

### NET-10 — `directConnection` is a dead setting: fully plumbed in the backend, no control anywhere
Severity: Low        Category: maintainability
Platforms: macOS, iOS (the only consumers)
Evidence class: source-traced
Location: `src-tauri/src/store.rs:512, 583` + `src-tauri/src/service/dispatch.rs:1547` + `src-tauri/src/localswarm/mod.rs:744` + `src-tauri/src/service.rs:1499` + `src/api.ts:1045-1046` + `src/types.ts:186`
Trigger: None. No component writes `directConnection` (a search of `src/components` finds no reference), so it is always `false`.
Causal path: `set_direct`, `ConnectionTarget::with_direct`, `StoredJob.direct` and the `api.ts` guard on a failed `directConnection` write all exist for a preference the user cannot change. The Apple peer-to-peer preference therefore cannot be enabled, and the code misleads maintainers into thinking it is user-reachable.
Smallest correct fix: Either add the Apple-only toggle to the Nearby "Device name / other ways" section, gated on macOS/iOS, or remove the setting, the `api.ts` guard and the `with_direct` plumbing. Pick one; do not leave both halves.
Regression test: If kept, a surface test that toggling the control issues `save_settings {directConnection}`. If removed, `tsc --noEmit` and the existing store migration tests cover it.

---

**Covered (read end to end)**
- Frontend: `SharingControls.tsx`, `LocalSwarmControls.tsx`, `PrivateReadingSync.tsx`, `UI.tsx` `errorText`, `shared-library-batch.ts`.
- Rust: `nearby_service.rs`, `private_sync_service.rs`, `private_sync.rs`, `shared_library.rs`, `android_tls.rs`, `nearby_public_link.rs`, `localswarm/library.rs`, `library_transport.rs`, `library_route.rs`, `discovery.rs`.
- Dispatch arms for `sharing_*`, `localswarm_*`, `shared_library_*` and the `private_sync_*` gate.
- Plugin Rust: `src/lib.rs`, `src/android.rs`.

**Covered in part**
- `sharing.rs`: bindings, listener and cancellation, `SharingServer` start/shutdown/status, `SessionHosts`, `LibraryHosts`, `PinAttempts`, `same_owner`, `open_download`. Not the HTTP framing (`SharingIo`), request handlers or OPDS.
- `localswarm/mod.rs`: types, status, library-share lifecycle, start/spawn/pause/resume, first half of the receive loop, seed/supervise/stop. Not publication, cleanup or import.
- `localswarm/network.rs`: timeout and limit table, `reachable_addresses`, `connect_addresses`, seed entry. Not the wire or pipeline code.
- `localswarm/ios_discovery.rs`: browse command, failed latch, error mapping only.
- `localswarm/sources.rs`: lines 1-140.
- `hotspot.rs`: lines 1-260.
- `mobile_service.rs` hotspot section and `mobile_policy.rs` network reconcile.
- Kotlin: `NearbyBluetooth.kt` begin/fail/retire, `KiwixMobilePlugin.kt` Bluetooth start, join and hotspot rejects, `ApplicationResources.kt` `serviceCall`, `NearbyNative.kt`.
- Swift: `NearbyBluetooth.swift` state and retire only.
- Private Sync wire framing, bounds and deadlines, single-use token: nothing met the bar. `android_tls.rs`: nothing met the bar.

**Not covered**
- `localswarm/protocol.rs`, `storage.rs`, `cleanup.rs`, `import.rs`, `profile.rs`, `library_wire.rs`, `apple_transport.rs`, `apple_transport/read_request.rs`.
- `src-tauri/native/localswarm.mm`.
- `nearby_hotspot.rs`, `nearby_route.rs`, `nearby_log.rs`, `nearby_public_link/framing.rs`, `private_sync_identity.rs`, `sharing_download.rs`.
- Bodies of `NearbyJoin.kt`, `NearbyRoutes.kt`, `LocalHotspot.kt`, `NearbyJoin.swift`, `RouteIPAddress.swift`.
- Test files were only spot-checked: wiring, CI commands, and one assertion in `sharing_interop_tests.rs`.
- `docs/LOCALSWARM.md` and `docs/NEARBY-ACCEPTANCE.md` were searched, not read through.
- No device, Windows or Linux runtime was exercised. The Android interface-name claims and the macOS Local Network callback path are inferences, as marked.