## Verdict for lane net, round 3: no findings. The lane is closed, with the residuals listed below

All three round-3 items are closed: the cleanup placeholder recovery, NET-R7 and NET-R8. Every check passed on this snapshot, including the Android and Windows clippy runs that could not finish in round 2.

Reviewed snapshot: `git diff d98cac2 -- src-tauri/src/localswarm/cleanup.rs src-tauri/src/sharing.rs src-tauri/src/sharing_interop_tests.rs src/components/SharingControls.tsx src/types.ts`, saved as `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-net/r3.diff`. HEAD moved from 0b669c7 to 618b809 during the review; the diff over these paths did not change.

### Cleanup placeholder recovery (`discard_capture_placeholder`): closed
**It can remove nothing but an empty directory.**
- The removal is cap-std `remove_dir`, which is `unlinkat(AT_REMOVEDIR)` on the name, relative to the retained parent descriptor.
- The inspection uses a no-follow open of the same name. The entry must be a directory owned by the effective uid, with `mode & 0o077 == 0`, not the journaled dev/ino, and with no entries.

**TOCTOU between inspection and `remove_dir`.** Only something outside this process can swap the entry, because within the process both callers hold `operation`:
- `terminal_owned_publications` uses `try_lock`.
- `delete_terminal` and `erase` use `lock`.
- The process lock excludes a second instance.

What a swap can lead to:
- A non-empty directory makes rmdir fail with ENOTEMPTY.
- A symlink or a file makes it fail with ENOTDIR, because rmdir does not follow the final component.
- A failure returns `Ok(false)`, and the caller's original "both names exist" error stands.
- The worst case is removing some other empty directory placed at `.kiwix-localswarm-<id>.cleanup`. No bytes can be lost.

**The stricter source check cannot block a real recovery.** An interrupted two-step capture leaves the source untouched. That source is still the journaled dev/ino, and it was made 0700 by `Directory::child`. `parent.child(source)` applies exactly the predicate that the normal capture path (`owned_directory`, `capture_directory`) requires anyway. Any state that fails it would also fail ordinary cleanup.
- The check is also necessary. Take a capture that completed and emptied the owned directory, after which a new job reused the destination name. Without the source check, the placeholder test alone would pass on the new job's directory. With it, the new directory's identity does not match and the case is refused. The identity check on the placeholder adds a second guard.
- On mounts that report directories as non-private or as another uid (FAT/exFAT with a `dmask`, Android FUSE), LocalSwarm already refuses to own directories, so this state cannot arise there. That is the implementer's doubt, and I agree.

**Mutation on the proof path.** `terminal_owned_publications` now performs at most one rmdir of an empty, private, non-journaled directory, followed by `parent.sync()`. It does this under the `operation` try-lock, so it cannot overlap `erase_with_hook`. After it, `captured = false` and the proof is computed from the journaled source as before. Archive bytes and the user's unrelated archive are untouched; the test asserts both. I accept this behaviour change.

**Call sites.**
- `erase_with_hook`: the moved "both exist" check is equivalent to the old `else` branch when the recovery does not apply.
- When the helper returns `Err` (parent changed, metadata failure), that error replaces the generic message. This is acceptable.
- The non-Linux/Android stub returns false, so behaviour there is unchanged.

**The test is meaningful.** It covers a non-empty directory (both sites), a 0755 directory, another uid (root only, as here), a symlink, a plain file, a replaced source, and then the real case at both sites.

### NET-R7: closed
- `prefer_routed = !windows` gates only the first arm. `default_binding` passes `cfg!(windows)`.
- On Windows: the classifier exclusions stay, base ordering is restored, and the vEthernet host-switch fallback still applies.
- The test covers ("Ethernet 3" 10.8.0.6 routed, "Wi-Fi" 10.0.0.5) with windows=true giving 10.0.0.5, and the switch fallback with windows=true.

### NET-R8: closed
- The alert no longer claims the device is offline, and it names the real "Listen on" label (`SharingControls.tsx:451`).
- Keeping `localOnly` true for an IPv6-only share with no IPv6 address is correct: its links really are loopback-only.
- Wording nit, not a finding: "Sharing continues on a new connection without restarting" holds only for a network of the bound family. The sentence already offers the other remedy (choose a different "Listen on" option).

### Residuals of the closed lane
1. **NET-10:** `directConnection` has no control. Owner decision, once it can be tested on two Apple devices.
2. **Windows one-click Web sharing:**
   - A full-tunnel VPN whose private address sorts before the LAN is still chosen, as at base.
   - The routed tie-break between two real LANs is not used on Windows.
   - Closing either needs the adapter IfType (`windows-sys`, new unsafe code). Owner decision.
3. **CORE-3:** the `#[cfg(mobile)]` dispatch arm being switched back to `nearby_command` cannot be caught by a host test. It is type-checked for Android only.
4. **NET-5:** the non-Apple LocalSwarm mDNS A records still carry the addresses taken at listen time. This was out of scope.
5. **NET-4:** the Android backgrounding stop and iOS `set_sharing_admission(false)` still stop sharing silently. This predates the lane.
6. **Owner handoff:** regenerate the locales and `docs/ERRORS.md`. The lane added 10 frontend strings (one of them replaced this round) and 1 Rust string. The two node manifest tests stay red until then, and they gate CI.
7. **Never exercised on real hardware or a real OS:** Windows (Hyper-V switch, VPN adapter names), Android (hotspot waiter, finder dismissal, Bluetooth off or denied), Apple interface tables and Local Network, a real sleep/wake address change, a 5-minute sender-gone receipt, and a crash between `mkdir` and `rename` on a volume without RENAME_NOREPLACE.

### What I ran
- `cargo test --locked … --lib -- sharing localswarm shared_library transfer_policy hotspot nearby_service` → `test result: ok. 215 passed; 0 failed; 5 ignored`. This includes `an_interrupted_capture_leaves_a_placeholder_that_cleanup_discards_and_nothing_else` and `one_click_binding_chooses_the_lan_another_device_can_reach`, both ok.
- Host `cargo clippy --locked … --lib --tests --features custom-protocol -- -D warnings` → exit 0.
- `android-clippy.sh` → exit 0.
- Windows-gnu clippy (`--target x86_64-pc-windows-gnu --lib --tests`) → exit 0. The log shows a real recheck of `kiwix-workbench`.
- `npx --no-install tsc --noEmit` → exit 0.
- I did not re-run the browser surface test this round. The only frontend change is one replaced string, and the implementer reports 3/3. It last passed 3/3 under my runs in round 2.