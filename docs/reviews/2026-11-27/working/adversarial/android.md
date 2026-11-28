## Verdict for lane android: only non-blocking findings

I found three should-fix problems in the new code and one should-fix race left over from ANDROID-7. Nothing would crash, corrupt data or weaken an authority check. Nothing ran on Android itself, so every lifecycle and platform statement below comes from reading the source.

### ANDROID-R1: the extra hop to `worker` lets the 4 s start timeout fire falsely, which pauses every download
Severity: Should-fix
Fix under review: ANDROID-7 (plugin half)
Evidence class: source-traced (how long provider calls take is inference)
Location: `KiwixMobilePlugin.kt:1786-1800` (`startTransferService`)
Trigger:
1. A download starts or resumes while `worker` is busy with a slow provider call. Examples: `acquireExternalArchive`, `prepareExternalArchiveMove` (`ApplicationResources.kt:214`, which runs on the same `worker`), or `deviceScan.open` / `probe` (`:1161`). A cloud, USB or SD provider can block for seconds.
2. `transferState(active=true)` arrives in that window.

Causal path:
1. `main.postDelayed(timeout, 4000)` is armed first, and only then `worker.execute { TransferEvents.peek(...) }` is queued, behind the busy task.
2. After 4 s the timeout calls `KiwixTransferService.stopAfterFailure(..., "TRANSFER_FOREGROUND_START_TIMEOUT")`. That persists a `transfers-error` event.
3. Rust's `mobile_reconcile_transfer_events` (`mobile_service.rs:2003`) then runs `mobile_pause_transfers()`, pausing all transfers, and emits an error.
4. Before this change the 4 s only covered the service's own confirmation on main.

The "disk-backed, so read off main" rationale does not hold. `KiwixTransferService.onStartCommand` already makes the same `TransferEvents.peek` call on the main thread.

Smallest correct fix: call `TransferEvents.peek` synchronously on main before reserving `foregroundInvoke` and arming the timeout, as the service already does. Alternatively, arm the timeout inside the `main.post` after the peek, or use a dedicated executor.

### ANDROID-R2: an export picker settled as `ACTIVITY_RECREATED` leaks the prepared export file
Severity: Should-fix
Fix under review: ANDROID-5
Evidence class: source-traced
Location: `KiwixMobilePlugin.kt:545-552` (`retireWindowUi` / `retirementOutcome`); `mobile_service.rs:427-435` (`mobile_export_authorized`)
Trigger:
1. Start "Save export", or `mobile_save_resource` / `mobile_save_mhtml`.
2. While the picker is open, Kiwix's Activity is replaced. This happens with a font-scale or language change applied on return, density, "Don't keep activities", or a system reclaim.

Causal path:
1. Before the change, a pending (non-copying) export op ended in `complete(op, cancelledResult("export"))`.
2. Rust then saw `cancelled` and ran `prepared.cleanup()`.
3. Now `replaced` gives `FAIL_RECREATED` and `fail(...)`.
4. Rust's `.map_err(|error| … "Prepared export retained at …")?` returns early, so `cleanup()` never runs.
5. The file in `mobile-exports-v1/<uuid>/` is never reclaimed. I found no sweeper for `mobile-exports-v1`.

Smallest correct fix, either of:
- In Kotlin, keep `CANCELLED` for `kind == "export"` while `op.exportUri == null` (nothing written yet).
- In Rust, call `prepared.cleanup()` on an `Err` that carries no `destinationUri`.

The copy-import kinds are fine: their new Rust `Err` arms remove the empty root.

### ANDROID-R3: an operation the person already cancelled can later be relabelled "recreated"
Severity: Nit (becomes Should-fix if H2 lands)
Fix under review: ANDROID-5
Evidence class: source-traced
Location: `KiwixMobilePlugin.kt:541` (`if (replaced) op.replaced = true`), `:412` (`bindActivity` → `retireWindowUi(replaced = true)`)
Trigger:
1. A multi-GB copy is running.
2. Back finishes the Activity (API ≤ 30), `finishActivity` is called, or Rust calls `retireWindow`. Each of these retires with `replaced=false` and cancels the op. The worker is still unwinding (`AWAIT_WORKER`).
3. The person relaunches. `bindActivity(B)` retires again with `replaced=true` and sets `op.replaced`.
4. `finishCancelledCleanup` then rejects with `ACTIVITY_RECREATED` instead of cancelled.

With H2 in place this would tell someone who closed Kiwix themselves "Kiwix restarted its window before this finished."

Smallest correct fix: `if (replaced && !op.cancellation.isCanceled) op.replaced = true`.

### ANDROID-R4: a Pause tap can still stop the service while a foreground start is queued
Severity: Should-fix (left over from ANDROID-7, not introduced here)
Fix under review: ANDROID-7
Evidence class: inference (platform behaviour from memory of `ActiveServices.bringDownServiceLocked` posting the foreground-start crash when `fgRequired` is still set)
Location: `KiwixTransferService.kt:180-183` (ACTION_PAUSE → `finish()` → `stopSelf()`)
Trigger:
1. The plugin's peek returns no events.
2. `ContextCompat.startForegroundService(UPDATE)` is issued.
3. A Pause tapped a few ms earlier is delivered to `onStartCommand` first.

Causal path:
1. `finish()` calls `stopSelf()` (startId −1). That stops the service regardless of the still-queued UPDATE start, which has `fgRequired` set, so the process dies.
2. The new plugin-side check narrows the window but cannot close it. The implementer listed the same class of problem for `stopOwned`.

Smallest correct fix: on the onStartCommand-driven paths, stop with `stopSelf(startId)` / `stopSelfResult(startId)`. The queued UPDATE is then still delivered, lands in the new STALE branch, and enters the foreground before stopping.

### ANDROID-R5: unknown codes in the Rust fallback now skip the frontend's localized message
Severity: Should-fix (needs an owner decision)
Fix under review: ANDROID-4
Evidence class: source-traced
Location: `user_error.rs:230-255` (`android_plain`) against `src/components/UI.tsx:205-221` (`nativeCodeText`) as it stands now
Trigger: a Kotlin rejection of the form `[ACTIVITY_DESTROYED] - ACTIVITY_DESTROYED`. Examples: `scannedOpens`, `deviceScanInvoke`, `speechInvoke` on retirement. The same applies to any unmapped non-Nearby code.

Causal path:
1. With the other lane's `UI.tsx`, the `[C] - C` form used to reach `t("That action did not finish. Try again. ({code})")`, which is localized.
2. Rust now returns an English-only sentence, a mapped one or `TRY_AGAIN`, with no code. Rust's sentences are not localized anywhere.
3. A non-English user who saw localized text now sees English.
4. The JSON form (`[C] - {json}`) did improve: it used to reach the screen raw.

Smallest correct fix, either of:
- Return the bare code for unmapped codes, as the implementer's H6 offers, so the shell's localized fallback runs.
- Move the new sentence table into `t()` on the frontend.

### ANDROID-R6: the read-aloud channel can be renamed to the English fallback
Severity: Nit
Fix under review: ANDROID-12 (channel half)
Evidence class: source-traced
Location: `SpeechPlayback.kt:358-368` (`ensureChannel`), called from `foreground(service)` at `:208`
Trigger: a stale transport intent reaches `KiwixSpeechService` in a fresh process. `labels` then still holds `SpeechLabels.from(null)`, which is English "Reading aloud". `ensureChannel` now always calls `createNotificationChannel`, so it renames the existing channel to English. The transfer service explicitly guards against exactly this ("Never rename an existing channel to the fallback label").
Smallest correct fix: rename only from `start()` (real labels). In `foreground()`, create the channel only if it is missing.

### ANDROID-R7: several new tests check a copied predicate, not the call site
Severity: Nit
Fix under review: ANDROID-1, 2, 5, 7, 9, 10, LIB-2
Evidence class: source-traced
These would still pass if the call site stopped using the predicate:
- `launchIntentConsumable`. The wiring is covered only by a source regex in `test-android-manifest.mjs`.
- `retirementOutcome`
- `stagingFailureRemovesCopies`
- `speechAudioPolicy`
- `transferStartDecision`
- `barAppearance`
- `hiddenScanName`. `DeviceZimScan.kt:150` and `:225` still inline `startsWith('.')` instead of calling it.

These are real tests:
- `StagingCleanupTest` exercises `StagedEntries.remove`, the actual code under `cancelCleanup`.
- `NetworkPrefixTest` exercises the real `inPrefix` its callers use.

None of these block.

### Per-fix assessment
- **ANDROID-1:** fixes it.
  - Kotlin removes only paths recorded after `Os.open(O_CREAT|O_EXCL|O_NOFOLLOW)` succeeded, or after a `mkdir()` that returned true.
  - `File.delete()` cannot remove a non-empty directory.
  - The cleanup runs only in the `launchWorker` catch, before `main.post { fail }`, so never after a result was delivered.
  - Rust's `root` is always the fresh per-call UUID directory from `mobile_directory`. `remove_dir` removes only an empty directory and does not follow symlinks.
  - I found no `fail()` path that can run while a copy worker for the same op is still writing.
- **ANDROID-2:** fixes the stale replay. All three old `activity.intent` uses go through `launchIntent`. I walked these cases:
  - Cold "Open with": offered once.
  - Configuration change before the plugin loads: still consumable via `launchUnoffered`.
  - Process death and restore: consumed.
  - Recents with `FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`: consumed.
  - "Don't keep activities": consumed, and a new intent still arrives through `onNewIntent` because the plugin is alive and `bindActivity` has cleared `destroyed`.
  - Notification tap or pinned shortcut on a fresh task: consumable.
  - Multi-window resize: no recreation.

  The change adds no new silent drop of its own. Three small leftovers:
  - (a) A recreation in the same process after a failed durable intake no longer retries it; the person has already seen the failure notice.
  - (b) If the process dies before the first offer, the file is lost.
  - (c) The existing loss when `onNewIntent` arrives before the plugin loads (doubt 2).
- **ANDROID-4:** fixes it for whole-message shapes. All 16 `user_error` tests pass on the host.
  - I found no Rust or desktop message that the new shape detection would catch. D-Bus names such as `org.freedesktop.DBus.Error.X` fail `is_java_throwable`, and `[File] …` fails `is_android_code`.
  - The ENOSPC check reads only `data.detail`, so it cannot plausibly misfire.
  - Every code in the implementer's table exists in Kotlin. Small wording inaccuracies:
    - `NATIVE_OPERATION_BUSY` is also thrown for a pending notification prompt, not only a file action.
    - The ENOSPC sentence says "on this device" even when the export destination is a provider elsewhere.
    - `SCANNED_ARCHIVE_UNAVAILABLE` advises importing a copy even when the file is gone.
  - See R5 for the combined frontend behaviour.
- **ANDROID-5:** fixes it but has two problems (R2, R3).
  - `!isFinishing` correctly reads as not-replaced for Back (API ≤ 30), `finishActivity` and a task swipe.
  - A running copy still cleans up before it settles (`AWAIT_WORKER` → `finishCancelledCleanup`).
- **ANDROID-7:** fixes the reported trigger but has two problems (R1, R4).
  - The service-type argument passed (`FOREGROUND_SERVICE_TYPE_DATA_SYNC` on API ≥ 29) matches the manifest's `dataSync`.
  - The channel is created before posting, and the refusal is caught.
  - Every API is fine against minSdk 24.
  - In the STALE branch, a foreground started there is removed when the service stops.
- **ANDROID-8:** the steps under review are done (`App.tsx` H1 has landed through another lane).
  - iOS specificity is unchanged: `:root:is([a],[b])[c]` and `:root[a][c]` both score (0,3,0), and source order is the same.
  - All 16 large-text selectors were converted, and none remain elsewhere.
  - The height-token claim holds: the only consumers are `min-height` at `styles.css:101`, `:188` and `platform.css:1134`.
  - How the reflow looks on Android was not rendered. The phone-width nav labels stay `display:none` on Android.
- **ANDROID-9:** fixes it. Every API call in `WindowTheme.kt` is guarded or safe at minSdk 24:

  | API | Minimum level and guard |
  |---|---|
  | `Configuration.uiMode` | API 8 |
  | `setBackgroundDrawable` | API 1 |
  | `WindowInsetsControllerCompat` light status bars | no-op below 23 |
  | `WindowInsetsControllerCompat` light navigation bars | no-op below 26 |
  | `navigationBarColor` | API 21, guarded to 26..28 |

  - The two scrim constants match androidx `EdgeToEdge`'s API 26–28 defaults.
  - Timing is safe: the call runs after `onCreate`, and `onConfigurationChanged` reapplies for the "system" theme.
  - The Rust arm validates `system|light|dark`. `applyTheme` is not in `owns()`, so the call goes through `run_mobile_plugin_async` to the `@Command`.
- **ANDROID-10:** fixes it. Every exit path after registration calls `stop` → `releaseAudio`, which unregisters. A registration failure now fails a system-voice session closed. That registration (`ContextCompat.registerReceiver` with NOT_EXPORTED) is the same one the bundled-voice path already depends on.
- **ANDROID-11:** fixes it. `inPrefix` is correct for /0, /31, /32, IPv6 /10 and /128, mismatched families and out-of-range lengths. The dead commands are still there.
- **ANDROID-12:** partial, as stated. The channel half is fixed (with R6); the device-scan half is handoff H4.
- **LIB-2 (Kotlin half):** fixes it.
- **R8 rule:** the syntax is valid. It is the standard `-keepclasseswithmembernames,includedescriptorclasses … { native <methods>; }` form, scoped to `org.kiwix.mobile.**`.

### Implementer's doubts
1. Checkpoint `08f3094`: acceptable. I built and tested the working tree, not the checkpoint.
2. New intent lost before the plugin loads: the defect exists and predates this change. It needs an owner decision on a companion buffer drained in `load()`. This change does not make it worse.
3. Unanswered "Open with" across process death now shows a notice: acceptable.
4. `layoutDirection`: acceptable. I recall the same thing: `Configuration.diff` adds `CONFIG_LAYOUT_DIRECTION` on any locale-list change. Owner decision.
5. Native errors behind a Rust prefix are not normalized: acceptable as a follow-up.
6. iOS side effects: acceptable. Upper-case iOS codes are either `SPEECH_` (now mapped) or the Nearby families (passed through). The rest falls under R5.
7. `is_java_throwable` breadth: acceptable. I checked D-Bus and Rust `::` names.
8. `replaced = !isFinishing`: acceptable for a system reclaim. R3 is the case where it mislabels.
9. Transfer start timing: real problem to fix now (R1).

### What I ran
- Kotlin compile and plugin tests (Gradle, offline, `--max-workers=2`, `--no-daemon`): `:tauri-plugin-kiwix-mobile:compileDebugKotlin :app:compileUniversalDebugKotlin :tauri-plugin-kiwix-mobile:testDebugUnitTest --rerun`.
  - Result: `BUILD SUCCESSFUL in 41s`; 14 result files, `tests=64 skipped=0 failures=0 errors=0`.
  - The compile tasks were up to date against the current sources.
- `cargo test --locked --lib user_error` on the host: `test result: ok. 16 passed; 0 failed; … 1426 filtered out`.
- `android-clippy.sh`: `Finished dev profile … exit=0`. Warnings come only from the vendored `cesu8`.
- `node --test scripts/test-system-text-scale.mjs scripts/test-android-manifest.mjs`: 11 pass, 0 fail.
- `npx --no-install tsc --noEmit`: exit 0.

### What I could not verify
- Any real Activity lifecycle: process-death restore, Recents, "Don't keep activities", `onNewIntent` timing against plugin registration.
- Foreground-service start/stop behaviour, and the AMS rule R4 relies on.
- Bar colours and icon contrast on API 24–37.
- WebView `fontScale` text zoom, and how the large-text reflow looks on Android.
- Real native error strings from a device.
- R8 output from a minified build.