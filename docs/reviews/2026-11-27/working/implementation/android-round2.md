# Lane `android` — second report (changes since the first report only)

All seven review items and the SHELL-R6 addition are fixed. Kotlin compiles, 65/65 plugin unit tests pass, `cargo test --lib user_error` passes 18/18 on the host, and `android-clippy.sh` is clean. Nothing ran on Android itself, as before. I did not mutate the shared tree this round; the fail-without-fix check for the Rust changes was done on a scratch copy.

### ANDROID-R1 — false 4 s start timeout from the hop to `worker`
Disposition: fixed
Verification of the finding: Confirmed. I armed the timeout before queuing the peek behind whatever `worker` was doing, and the service already makes the same call on main.
Change: `KiwixMobilePlugin.kt: startTransferService` is back to its base shape with one addition. After `requireVisible()` and before `foregroundInvoke` is reserved or the timeout armed, it calls `TransferEvents.peek(applicationContext)` synchronously on main and throws `NativeDocumentError("TRANSFER_CONTROL_PENDING")` if anything is pending. No `worker` hop; the extra `source`/`generation` parameters are gone. Against `d98cac2` the function differs only by that check and its comment.
- The refusal now leaves through `transferState`'s existing catch, which also releases the transfer screen. The code is the same (`TRANSFER_CONTROL_PENDING`); the message is the plugin's JSON envelope rather than the bare code. Rust does not match on it and `user_error` maps both shapes.
Tests: none new; the decision is one line. `TransferStartDecisionTest` is unchanged and passes.
Not exercised: a real start while a signal is pending.

### ANDROID-R2 — export picker settled as `ACTIVITY_RECREATED` leaks the prepared file
Disposition: fixed in Kotlin
Verification of the finding: Confirmed in `mobile_export_authorized`: with `open == false` it pins a prepared export, runs `prepared.cleanup()` only on `Ok`, and on `Err` returns "Prepared export retained at …".
Change: new pure `replacementFails(kind)` (false only for `"export"`); `retirementOutcome` now takes `kind`.
- Picker still open: an export settles as CANCELLED on replacement.
- Worker running: `finishCancelledCleanup` applies the same rule, so an export cancelled before its destination was opened also settles as CANCELLED. An export that already wrote still fails as `EXPORT_CANCELLED_PARTIAL` first, as before.

Survey of the other Rust callers for "prepared state is cleaned up only on cancelled":

| Caller | Kind | Prepared state | Result |
|---|---|---|---|
| `mobile_save_text`, `mobile_save_bytes`, `mobile_save_resource`, `mobile_save_mhtml` | `export` | pinned export | The R2 case; fixed by the rule above |
| `mobile_pick_archive_copies`, `mobile_pick_directory_copy`, `mobile_copy_incoming` | `archives`/`directory` | per-attempt root | My `Err` arms remove the empty root; an incoming request stays queued either way |
| `mobile_open_external`, `mobile_add_external`, `mobile_open_incoming` | `external`/`externalBatch` | none before the call | Kotlin forgets provider references in `finishCancelledCleanup` before it settles, whatever the outcome |
| `mobile_read_text`, `mobile_device_grant_folder` | `text`/`scanFolder` | none | Nothing to leak |
| `share_book` | `open` | none in Rust | Scratch copy is Kotlin-owned |
| attachment open (`dispatch.rs`) | `open` | prepared attachment | Swept by age (30 days), not by outcome |
| Android PDF share (`native_pdf/android.rs`) | `open` | rendered PDF | Owned by `NativeJob`, removed on drop for any outcome |

Only the export kind had the pattern.
Tests: `replacedActivityStillCancelsASaveThatWroteNothing` (new); the existing retirement test now runs over every kind. Not observed failing without the fix (no mutation in the shared tree); inferred from the function.
Not exercised: a real replacement during a save picker.

### ANDROID-R3 — already-cancelled operation relabelled "recreated"
Disposition: fixed
Change: `retireWindowUi` now sets the flag with `if (replaced && !op.cancellation.isCanceled) op.replaced = true`, before it cancels. An operation cancelled by the person, by Back, or by an earlier non-replacing retirement keeps its cancelled outcome when a later `bindActivity` retires again.
Tests: none; the condition reads `CancellationSignal`, which the JVM tests cannot construct.

### ANDROID-R4 — Pause can stop the service under a queued foreground start
Disposition: fixed for every self-stop; `stopOwned` reported, not changed
Change: `KiwixTransferService` records `deliveredStartId` at the top of `onStartCommand` (only when `startId > 0`; `updateRunning` calls it directly with 0). `finish()` now ends with `stopSelf(deliveredStartId)`, falling back to `stopSelf()` only if no start was ever delivered.
- Why the recorded id and not the parameter: on the direct-update path `stopSelf(0)` would never match Android's last start id and would leave a stopped-but-alive service.

Every path that reaches `finish()`:

| Path | Before | Now |
|---|---|---|
| `onStartCommand`, `ACTION_PAUSE` | `stopSelf()` | Bounded by this start's id. A queued UPDATE keeps the service alive, arrives with a retired admission (STALE), enters the foreground once, then stops by its own id |
| `onStartCommand`, other or null action | `stopSelf()` | Same |
| `onStartCommand`, CONTROL_PENDING (Android delivery or direct update) | `stopSelf()` | Same; on the direct path no start can be queued, so it stops at once |
| `onStartCommand`, catch | `stopSelf()` | Same |
| `onTimeout(startId[, type])` | `stopSelf()` | Same; `stopForeground` already meets the timeout obligation |
| `stopOwned()` → `finish()` then `stop(context)` | `stopSelf()` then `stopService` | `finish()` is bounded, but `stopService` follows and is unbounded: **outcome unchanged** |

The STALE branch already used `stopSelf(startId)`.
- **Still open:** `stop(context)` → `context.stopService`, reached from `transferState(active=false)`, the 4 s timeout and `retireWindowUi` while a start is pending, can still bring the service down under a queued foreground start. A fix in the same style would bump the generation and let the instance (or the queued start) stop itself, never calling `stopService` while a start is pending. I did not make it: it changes `stopOwned`'s returned latch and was outside the item.
Tests: none new; it is one bounded call. The platform rule itself cannot be exercised here.

### ANDROID-R5 — unmapped codes skipped the shell's localized fallback
Disposition: fixed as decided
Change (`user_error.rs: android_plain`): an unmapped code is returned bare, from both the bracketed and the Java-wrapped shape. The sentence table, the ENOSPC sentence and the Nearby-family handling are unchanged. Java exception text with no code still becomes `TRY_AGAIN`.
- I also tightened `is_android_code` to the shell's `NATIVE_CODE` grammar (upper-case words joined by single underscores), so every bare code Rust emits is one `nativeCodeText` recognises. `FOO__BAR` and `FOO_` are no longer codes.
- Consequence: `NATIVE_DOCUMENT_ERROR` without a full-disk detail now reaches the shell as that bare code.
Tests: `unmapped_android_codes_reach_the_shell_bare_for_its_localized_fallback` and `android_failures_without_a_code_never_show_java_text_or_paths` replace the old unknown-code test; the full-disk test expects the bare code for the generic case.

### SHELL-R6 — bare Nearby codes containing "RETIRED" became `TRY_AGAIN`
Disposition: fixed
Change (`user_error.rs: plain`): right after the Android-shape check and before the tables and the internal-terms check, a whole-message code of the `NEARBY_`/`BLE_`/`JOIN_`/`HOTSPOT_`/`LOCAL_NETWORK_` families is returned unchanged (trimmed).
Tests: `bare_nearby_codes_reach_the_shell_even_when_they_contain_internal_words` covers `BLE_OWNER_RETIRED`, `BLE_CONNECTION_RETIRED`, `NEARBY_BLUETOOTH_RETIRED`, `NEARBY_ACTIVITY_RETIRED` and three others, and checks that a sentence containing such a code is still not exempt.
- Not changed: a Rust sentence followed by such a code ("…: BLE_OWNER_RETIRED") still hits the internal-terms check. The request was for bare whole-message codes only.

Fail-without-fix for R5 and SHELL-R6: observed on a scratch copy of the file compiled with plain `rustc`. With both changes reverted in the copy, the three relevant tests fail (12 passed, 3 failed); the current file passes 15/15 there and 18/18 under cargo.

### ANDROID-R6 — read-aloud channel renamed to the English fallback
Disposition: fixed
Change: `SpeechPlayback.ensureChannel(context, rename)`. `start()` passes `rename = true` and always re-creates the channel with the shell's labels. `foreground(service)` passes `rename = false` and creates the channel only when it is missing.
Tests: none (`NotificationChannel` is not constructible in the JVM tests).

### ANDROID-R7 — tests check a predicate, not the call site (cheap part)
Disposition: cheap part done
Change: `DeviceZimScan.kt` — the MediaStore and shared-storage walks now call `hiddenScanName(name)`, as the SAF walk does. No inline `startsWith('.')` remains outside the predicate.
The other predicates the review lists are unchanged; their call-site wiring is still covered only by reading and, for the launch intent, a source assertion.

---

## Commands run (final results)

- Kotlin compile, plugin + app: `BUILD SUCCESSFUL in 19s`.
- Plugin unit tests: `BUILD SUCCESSFUL in 17s`; 13 result files, 65 tests, 0 failures, 0 errors.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib user_error`: `test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 1432 filtered out`.
- `android-clippy.sh`: `Finished dev profile … exit=0`. The first run this round failed on `src/acquisition/transfer.rs:748` (`clippy::blocks_in_conditions`), another lane's file mid-edit; I waited until that line changed and the rerun was clean.
- `rustfmt --edition 2021 --check src/user_error.rs`: clean.
- `node --test scripts/test-system-text-scale.mjs scripts/test-android-manifest.mjs`: 11 pass, 0 fail.
- Not rerun: `tsc` and the full node suite (no TypeScript, CSS or script changes this round).

## Files changed this round

- `KiwixMobilePlugin.kt` (R1, R2, R3)
- `KiwixTransferService.kt` (R4)
- `SpeechPlayback.kt` (R6)
- `DeviceZimScan.kt` (R7)
- `MobileIntakePolicyTest.kt` (R2 tests)
- `src-tauri/src/user_error.rs` (R5, SHELL-R6, tests)

No new user-visible strings. No change to `mobile_service.rs`, the manifest, the frontend files or the scripts.

## Contract changes for other lanes

- After `user_error::plain`, an Android failure is now one of: a mapped English sentence; a bare status code (any unmapped code, and Nearby-family codes that arrived Java-wrapped); an unchanged bracketed Nearby-family rejection; or `TRY_AGAIN` when Java exception text carried no code. This replaces H6 in the first report.
- On every platform, a bare whole-message Nearby-family code passes through `plain` unchanged.
- `TRANSFER_CONTROL_PENDING` from the plugin-side check now carries the plugin's JSON envelope as its message instead of the bare code.

## Doubts

1. **`stopOwned`/`stopService` under a pending start** is still open (see R4). It is the same platform rule, reachable when the shell turns transfers off within moments of turning them on.
2. **R2 makes a replaced save silent again.** That is the behaviour before my lane's change and what Rust's cleanup needs; the person gets no message that the save did not happen. Cleaning up in Rust on an `Err` with no `destinationUri` would allow a visible failure instead.
3. **R4 relies on the Android rule that `stopSelf(id)` is a no-op while a newer start is queued.** I recall this from the platform source and could not run it.
4. **New Kotlin assertions this round were not observed failing without the fix**, since I did not mutate the shared tree and have no isolated Gradle copy.