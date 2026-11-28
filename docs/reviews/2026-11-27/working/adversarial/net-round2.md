## Verdict for lane net, round 2: only non-blocking findings

Round 1 was NET-R1 to NET-R6. Five of the six are now closed. NET-R4 stays open: the routed preference can still pick a Windows VPN, and I give a safe rule below. I found two new nits. I accept both deviations from the owner's instruction (NET-R1 and NET-R2) as safer than the literal wording. The lane tests, host clippy, `tsc` and three browser runs all pass. Android and Windows clippy could not finish on the current tree: they failed only in other lanes' files.

Snapshot reviewed: `git diff d98cac2` over the lane's paths, saved as `adv-net/lane2.diff`, at HEAD 91f14f9. During the review only `src-tauri/src/localswarm/cleanup.rs` changed, and that is another lane's file. No net-lane file changed. The net hunks in `dispatch.rs` (the `nearby_discover_leased` call and the hosted-hotspot waiter) are the same as round 1.

### Round-1 findings
- **NET-R1 (Hyper-V external switch): closed. I accept the narrower fix.**
  - The new fallback is `routed.filter(eligible && switches.contains)`. It only applies when no real LAN adapter has an eligible address, and only to non-point-to-point `vEthernet*` adapters.
  - The owner's literal rule was "routed, private, present in `interfaces()`". That would also accept a full-tunnel VPN's private address on a machine with no private LAN address, because `interfaces()` does not exclude tunnels. The implementer's reason for narrowing it is correct.
  - The NAT "Default Switch" and WSL adapters never carry the host's default route, so they cannot be picked this way.
  - Residual: an external switch plus a second LAN adapter on a different network gets that other adapter (sort order), not the switch. This is ambiguous, not a defect.
- **NET-R2 (old receipt, sender no longer listed): closed. I accept the age condition.**
  - `admit` keeps the base behaviour for an old list whose sender is unlisted (refused with "browse it again"). It also keeps base behaviour for a recent list whose sender is unlisted, including after Stop finding (admitted).
  - It keeps the NET-9 fix: an old list whose sender is still listed is admitted.
  - Refusing at any age would make every mDNS gap a false "changed or disconnected". So this is the right reading of the instruction.
  - `check()` is unchanged, so running downloads are not affected.
  - `discovered_for_test` is behind `#[cfg(test)]`, and the `mod.rs` copy is also excluded on Apple.
- **NET-R3 (CORE-3 test could not fail): closed.**
  - The lease decision now comes before the `!cfg!(mobile)` and `native_ready` returns. The new test reaches the real `nearby_discover_leased` and `nearby_command("nearby_discover")` on host.
  - The test fails if the read is made unconditional (the leased call would wait behind the queued writer). It also fails if the public path's lease is removed (that call would finish inside 300 ms).
  - The desktop lease is harmless. It is a single, un-nested read taken from the lease-free entry. A queued writer only delays it. The desktop frontend never sends that command.
  - Still uncatchable on host: the `#[cfg(mobile)]` dispatch arm being switched back to `nearby_command`.
- **NET-R4: names closed; the Windows routed-VPN gap stays open (Should-fix, see NET-R7).**
  - `vboxnet`, `zerotier`, `nordlynx`, `virtualbox host-only` and `wwp` were added.
  - These names are also in the desktop peer-locality fence. That narrowing is in the fence's intended direction. Recording it as a known decision is correct.
- **NET-R5 (`pendingOnly` race): closed.** `nearby_stop_owner_if` decides and takes the owner out of its slot under one state lock. `pendingOnly` with no owner returns before stopping anything, so it can never sweep the publisher.
- **NET-R6 (status and UI nits): closed.**
  - `addressLost` now needs two misses at least 2 s apart, and one sighting resets it.
  - The poll has a single chain: the `asking` guard prevents a second one, a `return` inside `try` skips the reschedule after unmount, and a hide during a request does not reschedule.
  - `localOnly` is set for wildcard shares when every accepted host is loopback.
  - `nearby_discover {enabled:false}` now dismisses a finder that had already ended. It takes the nearby gate under the caller's lease, or with no lease on the lease-free path, so lock order is unchanged.
- **Windows clippy item (`sharing_interop_tests.rs:541`):** the rewritten helper (`|&(name, address, point_to_point)|`) reads clean. I could not confirm it with a full Windows run (see below).

### NET-R7 — Routed preference picks an unflagged Windows VPN over the LAN (regression from round 1)
Severity: Should-fix (not blocking: the trigger is narrow, and base already picked such a VPN whenever its address sorted first)
Fix under review: NET-1 / NET-R4
Evidence class: source-traced. No Windows host; `if-addrs 0.15` exposes no IfType (confirmed in `windows.rs`).
Location: `src-tauri/src/sharing.rs` (`auto_lan_binding`, first arm `routed.filter(|a| eligible(a) && available.contains(a))`)
Trigger: Windows, home LAN on "Wi-Fi" at 10.0.0.5 (for example Xfinity's default range). A full-tunnel OpenVPN TAP, WireGuard-NT or GlobalProtect adapter named "Ethernet 3" at 10.8.0.6 owns the default route. The person presses Enable web sharing.
Causal path:
- The VPN adapter is not point-to-point by IfType and matches no name, so it is in `available`.
- `routed` is 10.8.0.6, which is eligible, so the first arm picks it.
- Base sorted the list and picked 10.0.0.5.
- The QR then shows a dead VPN link while the status says "Sharing is on".
- When the VPN address sorts first (the common case: VPN in 10.x, home LAN in 192.168.x), base already picked the VPN. So the regression is limited to cases where the VPN address sorts after the LAN.

Smallest safe rule: on `cfg(windows)`, skip the first (routed-preference) arm and keep base ordering. Keep the new classifier exclusions (Docker, WSL, Hyper-V NAT, VirtualBox and the rest) and keep the vEthernet host-switch fallback. Concretely: `let prefer_routed = !cfg!(windows);`, then `routed.filter(|a| prefer_routed && eligible(a) && available.contains(a))`.

Why this is the right size:
- It removes every case where the change is worse than base on Windows.
- The only cost is the routed tie-break between two real Windows LANs, which base never had.
- A name allowlist cannot help: generic VPN adapters are named "Ethernet N".
- Netmask heuristics are unreliable: OpenVPN subnet topology gives /24.
- Re-enabling the preference on Windows only when the routed adapter's IfType is Ethernet (6) or Wi-Fi (71) needs a `windows-sys` `GetAdaptersAddresses` call. That is new unsafe code, so it is an owner decision.
- Add a test case ("Ethernet 3", 10.8.0.6, routed) next to ("Wi-Fi", 10.0.0.5), expecting 10.0.0.5 on Windows.

### NET-R8 — "localOnly" wording is wrong for an IPv6-only binding on an IPv4 network
Severity: Nit
Evidence class: source-traced
Location: `sharing.rs` (`status`, `loopback_only`); `SharingControls.tsx` (`localOnly` alert)
Trigger: Advanced binding "All IPv6 interfaces" on a network with no global IPv6, which is common.
Causal path: `binding_addresses` keeps only IPv6 addresses, so the host list is just `::1` and `localOnly` is true. The alert then says "This device has no network connection…", although the device is online over IPv4.
Smallest correct fix: Word it by the binding: "No address for this kind of network is available, so only this device can open the links…". Alternatively, set `localOnly` only when the IPv4 view is also loopback-only.

### NET-R9 — Android and Windows clippy are unverified for round 2 because other lanes' files do not compile
Severity: Nit (process; not a lane defect)
Evidence: on this tree, every Android attempt failed at `src/service.rs:1775` (`has_book_at_path`), and once also at `src/localswarm/cleanup.rs:81` (`open_dir_nofollow`). The Windows-gnu attempts failed at `src/maps/manager.rs:756`, `src/store.rs:2456` and `src/crash.rs:1184-1191`. None of these errors is in a net-lane file. The implementer reports exit 0 on an earlier tree.
Smallest correct fix: Re-run `android-clippy.sh` and the Windows-gnu clippy once those lanes compile again, before the lane is closed.

### Implementer's round-2 doubts
- **NET-R1 and NET-R2 deviations:** both accepted, as explained above.
- **NET-R4 Windows VPN gap:** real. Fix it now with the rule in NET-R7. The IfType-based refinement is an owner decision.
- **NET-R3 desktop lease:** acceptable.
- **NET-R6 detection latency:** acceptable. About 8 s at the 4 s poll trades a slower alert for no flapping.

### What I ran
- `cargo test --locked … --lib -- sharing localswarm shared_library transfer_policy hotspot nearby_service` → `test result: ok. 214 passed; 0 failed; 5 ignored`. This is on the current tree, which includes the unused-import removal the implementer had not re-tested.
- Host `cargo clippy --locked … --lib --tests --features custom-protocol -- -D warnings` → exit 0 on the first attempt.
- `android-clippy.sh` → 4 attempts, all failed in other lanes' files (NET-R9).
- Windows-gnu clippy (the brief's command) → 3 attempts, all failed in other lanes' files (NET-R9).
- `npx --no-install tsc --noEmit` → exit 0.
- `KIWIX_TEST_CHROME=… node --test scripts/test-nearby-sharing-surfaces.mjs`, three times at load 1.9 to 4.9 → 3 pass / 0 fail each time.

### What I could not verify
- No Windows, Android or Apple runtime. A real Hyper-V external switch, real Windows adapter names and IfTypes, the Android hotspot waiter and finder dismissal on a phone, and a real sleep/wake address renewal are all unverified.
- I did not repeat the implementer's mutation runs. I checked by reading the code that the new CORE-3 and NET-R2 tests would fail without their fixes.
- The generated locale and `docs/ERRORS.md` handoff is still open. This round added one more frontend string.