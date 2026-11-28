## Verdict for lane android (round 2): only non-blocking findings

All seven of my round-1 findings (R1 to R7) and SHELL-R6 are closed. Two items remain: one should-fix that predates this lane (R8) and one nit on the test claims (R9). I traced the new code, ran the builds and tests, and pushed 34 real-shaped error strings through the current `plain` and the current `errorText`. Nothing ran on an Android device or emulator.

### ANDROID-R8 — `stopOwned` still calls `stopService` while a foreground start is queued
Severity: Should-fix. It was already in the code at `d98cac2` and nothing in this lane introduced it; I agree it can wait if it is tracked.
Fix under review: ANDROID-7 / R4 residual
Evidence class: inference. The platform rule comes from my memory of AMS `bringDownServiceLocked` ("Bringing down service while still waiting for start foreground") posting the foreground-start crash; it was not run here.
Location: `KiwixTransferService.kt` `stopOwned` / `stop` → `context.stopService`, reached from:
- `transferState(active=false)` → `stopOwned`
- the 4 s timeout and `retireWindowUi` → `stopAfterFailure`

Trigger: any of those runs after `startForegroundService(UPDATE)` was issued and before the service has called `startForeground`.
Causal path: `stopService` stops the service whatever start is still queued, so the process can be killed. The 4 s timeout path is the worst case: a start that is merely slow (over 4 s, still inside the platform's own window) becomes a certain crash instead of a late success. The other two paths need the shell or the Activity to change state within milliseconds of the start.
Smallest correct fix: as the implementer outlined, never call `stopService` while `foregroundInvoke` is pending. Retire the generation instead, and let the queued start arrive as STALE; it enters the foreground once and then stops itself with `stopSelf(startId)`.

### ANDROID-R9 — the new Kotlin assertions were never seen failing
Severity: Nit
Fix under review: R2
Evidence class: source-traced
`replacedActivityStillCancelsASaveThatWroteNothing` and the per-kind loop do check `retirementOutcome` / `replacementFails` as the code uses them. But the call sites in `retireWindowUi:547` and `finishCancelledCleanup:1360` are still covered only by reading the source, and the implementer says none of the new assertions was observed failing without the fix. Not a blocker.

### Per-fix assessment
- **R1: closed.** `startTransferService` now calls `TransferEvents.peek` on main, after `requireVisible()` and before `foregroundInvoke` is reserved or the timeout is armed. The `worker` hop is gone.
  - ANDR risk: no more than the code already had. `peek` reads three small JSON strings from the `kiwix-mobile-events` prefs; there is one record per event name, so the file stays tiny.
  - It writes to disk only when an earlier persist failed or an old-format record is migrated, and then it is one commit of that tiny file.
  - The only other holders of the `TransferEvents` lock are `peek` / `acknowledge` on the endpoint thread, which hold it for at most a few small commits.
  - `KiwixTransferService.onStartCommand` already makes the identical call on main.
  - The refusal leaves through `transferState`'s existing catch with `foregroundInvoke == null`, so the screen assertion is released.
- **R2: closed.** I checked every kind:
  - `export` is the only kind whose Rust caller cleans up only on `cancelled`. That is `mobile_export_authorized` with `open == false`, used by `mobile_save_text`, `mobile_save_bytes`, `mobile_save_resource` and `mobile_save_mhtml`.
  - Calls with `open == true` keep nothing on the Rust side:
    - `share_book` (`share.rs:267`)
    - attachment open (`dispatch.rs:1847`, cleaned up by an age sweep)
    - the Android PDF share (`native_pdf/android.rs:64`, outcome-independent)
  - `external` / `externalBatch`, `text` and `scanFolder` hold no state before the call.
  - The copy-import kinds remove their root on `Err`.

  For `export`, both the pending-picker path and the worker path (`finishCancelledCleanup`) now end in `cancelled` unless something was already written (`exportUri` → `EXPORT_CANCELLED_PARTIAL`), so Rust's `prepared.cleanup()` runs again.
- **R3: closed.** `if (replaced && !op.cancellation.isCanceled) op.replaced = true` runs before `cancel()`, so an operation already cancelled by the person, by Back or by `retireWindow` keeps its cancelled outcome.
- **R4: closed for every path that goes through `finish()`.**
  - `deliveredStartId` is recorded at the top of `onStartCommand`, before the PAUSE branch, and only when `startId > 0`. `finish()` calls `stopSelf(deliveredStartId)`.
  - On the direct `updateRunning` path (startId 0), CONTROL_PENDING and the catch stop at once, by the newest id Android delivered. STALE cannot happen there.
  - A PAUSE that races a queued UPDATE now leaves the service alive. The UPDATE then arrives with a retired admission (STALE), enters the foreground once and stops with `stopSelf(startId)`.
  - A legitimate start prepared after `finish()` is no longer killed, which is better than before.
  - `onTimeout` already removes the foreground state with `stopForeground` before the bounded stop.
  - The `stopService` residual is R8.
- **R5: closed.**
  - `is_android_code` now matches the shell's `NATIVE_CODE` grammar (`[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+`), plus a 96-character cap.
  - Unmapped codes come back bare, and the shell turns them into its localized "That action did not finish. Try again. ({code})". End to end: `NATIVE_DOCUMENT_ERROR` (EIO), `CANCELLATION_CLEANUP_FAILED`, `DISCOVERY_UI_ADMISSION_REQUIRED`, `NATIVE_PLUGIN_UNAVAILABLE`, `TRANSFER_EVENT_DURABILITY_ERROR`, `PROVIDER_LEASE_CONFLICT` and `SCAN_TOKEN_RETIRED` all reach that line.
  - Mapped codes still become their sentences. That includes ENOSPC and `TRANSFER_CONTROL_PENDING` in the new JSON-envelope form.
  - No path or JSON reached the screen in any sample.
  - Small inconsistency: a bare non-family code such as `STALE_ACTIVITY_GENERATION` still becomes `TRY_AGAIN` through the internal-words check, while its bracketed or Java-wrapped forms map to a sentence. Nothing in Rust emits it bare, so this is not a finding.
- **SHELL-R6: closed.**
  - Bare `BLE_OWNER_RETIRED` gives "Nearby sharing is no longer active. Start it again." and `NEARBY_ACTIVITY_RETIRED` gives "Keep Kiwix open on this device while connecting.".
  - Bracketed and swapped Hotspot forms and `LOCAL_NETWORK_PERMISSION_DENIED` still map in the shell.
  - A Rust sentence followed by `": CODE"` is still not exempt, as scoped.
- **R6: closed.** `ensureChannel(rename)`: `start()` renames the channel using the shell's labels; `foreground()` only creates a missing channel.
- **R7: closed** for the part that was cheap to do. All three scan walks now call `hiddenScanName`. The other predicate-only tests are unchanged (nit only).

### Implementer's round-2 doubts
1. `stopOwned` / `stopService` residual: agree it can wait, tracked as R8. The timeout path is the strongest reason to do it next.
2. A replaced save is silent again: acceptable. That is how it behaved before this lane, and Rust's cleanup depends on it.
3. `stopSelf(id)` does nothing while a newer start is queued: my recollection of the platform agrees; not runnable here.
4. New Kotlin assertions not seen failing: noted as R9.

### What I ran
- Kotlin compile of the plugin and app module plus plugin unit tests (Gradle, offline, `--max-workers=2`, `--no-daemon`, `--rerun`): `BUILD SUCCESSFUL in 7s`, exit 0. 14 result files: `tests=65 skipped=0 failures=0 errors=0`.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib user_error`: `test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 1433 filtered out`.
- `android-clippy.sh`: `Finished dev profile … in 14.63s`, exit 0.
- End-to-end check:
  - I built a copy of the current `user_error.rs` as a scratch binary (`rustc` against the existing serde rlibs).
  - I ran 34 strings in the real Kotlin, endpoint and Rust shapes through it.
  - Each result went through a type-stripped copy of the current `UI.tsx` lines 39–253 (`nativeCodeText`, `nearbyCodeText`, `errorText`) under `node --experimental-strip-types`.
  - Inputs and outputs are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-android/e2e/`; every result is listed above.

### What I could not verify
- Any real service start or stop, the AMS rules behind R4 and R8, ANR timing on real storage, and an Activity replaced during a save picker.