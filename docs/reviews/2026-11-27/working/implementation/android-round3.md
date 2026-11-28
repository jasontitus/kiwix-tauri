# Lane `android` — third report (ANDROID-R8 and the CORE-8 crash-redaction handoff only)

Both items are done. Kotlin compiles, 70/70 plugin unit tests pass (65 before, 5 new), and no Rust was touched, so `android-clippy.sh` was not rerun. Nothing ran on Android itself. The fail-without-fix check ran in an isolated copy of the Gradle project in my scratch directory; the shared tree was not mutated.

### ANDROID-R8 — `stopService` under a queued foreground start
Disposition: fixed
Verification of the finding: Confirmed in `KiwixTransferService.kt`. `stop()` called `context.stopService` unconditionally, and my R4 `finish()` still fell back to an unbounded `stopSelf()` when no start had been delivered yet. That second case is exactly an instance whose creating start is still queued.
Change (`KiwixTransferService.kt`, one call site in `KiwixMobilePlugin.kt`):
- New `requestForegroundStart(context, intent)` in the companion is now the only place `startForegroundService` is called for transfers. It counts each request in `queuedForegroundStarts`; `onStartCommand` decrements the count when Android delivers an `ACTION_UPDATE` (`startId > 0`). The plugin's `startTransferService` calls it in place of `ContextCompat.startForegroundService`.
- `stop()` always retires the generation, then asks the new pure `transferStopDecision(queuedForegroundStarts)`. With a start queued it returns without calling `stopService`; otherwise it stops the service as before.
- `finish()` ends with `transferSelfStopId(deliveredStartId)?.let { stopSelf(it) }`. With nothing delivered yet it does not stop at all: the queued start that created the instance does it.
- The queued start then arrives with a retired admission (STALE), enters the foreground once through the existing `enterForegroundBeforeStop`, and stops itself with `stopSelf(startId)`.
- **Latch:** `stopOwned` still returns the `destroyed` latch of the instance that existed at the call. It opens when that instance is destroyed, which a queued start can now delay until that start has been delivered. `null` means no instance existed; a service that a still-queued start creates afterwards stops itself and has no latch. This is documented on the function. The only caller (`transferState`) discards the return value.

What each caller now does:

| Caller | With a foreground start still queued | With none queued |
|---|---|---|
| `transferState(active=false)` → `stopOwned` | Any existing instance releases its lock and leaves the foreground; no `stopService`. The plugin then rejects the pending invoke with `TRANSFER_START_CANCELLED`, as before. The queued start arrives stale and stops itself; its late confirmation is ignored because that invoke is already settled. | Unchanged: bounded self-stop, then `stopService`. |
| 4 s timeout → `stopAfterFailure` | The durable `transfers-error` (`TRANSFER_FOREGROUND_START_TIMEOUT`) is still written and Rust still pauses transfers. No `stopService`. The slow start arrives stale, enters the foreground once and stops itself: a late clean refusal, not a process kill. | The start was delivered and only its confirmation is late: the service is already in the foreground, and `stopService` runs as before. |
| `retireWindowUi` → `stopAfterFailure("ACTIVITY_DESTROYED")` | Same as the timeout row. | Same as the timeout row. |

A second start requested while the first is still queued is handled: the first arrives stale and its `stopSelf(id)` is a no-op under the newer start, which is then admitted normally.
Tests: `queuedForegroundStartIsNeverStoppedFromOutside` and `serviceStopsItselfOnlyUpToAStartAndroidDelivered` in `TransferStartDecisionTest`. Both observed failing with the two decisions reverted in the isolated copy.
Not exercised: any real start or stop, and the platform rule itself. The tests cover the two pure decisions, not the counter wiring.

### CORE-8 handoff — crash redaction stopped at the first space
Disposition: fixed
Verification of the finding: Confirmed in `CrashIntake.kt`. `pathPattern` and `urlPattern` both ended at `\s`, so `/storage/emulated/0/My Books/…` kept everything after "My".
Change (`CrashIntake.kt`), a port of the Rust rule in `src-tauri/src/crash.rs`:
- `pathEnd(text, start)` mirrors `path_end`. A path runs to a closing `"` (one not escaped by an odd run of backslashes), `<`, `>`, a backtick, a line break, `:` followed by whitespace, or end of text. Trailing whitespace before the delimiter is trimmed. It does not stop at `'`.
- `pathAt` and `redactPaths` replace the regex and mirror `path_at`/`redact_paths`: same start forms (`/x`, `X:\` or `X:/`, `~/`, `\\`) and same left-boundary characters as before.
- `replaceUrls` walks URL matches left to right. A `file:` URL followed by `/` is redacted to `pathEnd`; every other URL still ends at whitespace, as in Rust's `replace_urls`.
- The order of steps in `redact` is unchanged (URLs, bearer, secrets, paths, opaque tokens, whitespace).
- Stack frames are untouched: `Class.method(File.kt:12)` and `File.kt:12` contain no path start, and a test pins that.
Tests (`CrashCapturePolicyTest.kt`):
- `redactionKeepsNoPartOfAPathThatContainsSpaces`: Android storage path, the same path followed by `: open failed…`, a spaced Windows-style path, quoted and escaped-quote forms, an apostrophe in a folder name, a share, a home-relative path, and a path ended by a line break.
- `fileUrlsWithSpacesAreRedactedToTheEndOfThePath`: `file:` with spaces, upper-case `FILE:` followed by `: denied`, a quoted `file:` URL, and `content:`/`https:` URLs that still end at whitespace.
- `stackFramesKeepTheirFileAndLine`.
- The existing `reportTextRedactsReaderLocationsAndNonHttpUrls` expectation changed in one place. Its line `path=/book-uuid/A/Private relative=/.kiwix/token/A/Private` now reduces to `path=[path]`, because the path runs to the end of the line; `relative=…` is asserted on its own. The Rust test made the same adjustment.
- The three redaction tests were observed failing in the isolated copy with the old stop-at-whitespace behaviour restored.
Not exercised: a real crash record written on a device.

### ANDROID-R9 follow-up
`replacedActivityStillCancelsASaveThatWroteNothing` (from the previous round) was included in the same isolated-copy run and observed failing with `replacementFails` reverted. The call sites in `retireWindowUi` and `finishCancelledCleanup` are still covered only by reading.

---

## Commands run

- Kotlin compile, plugin + app: `BUILD SUCCESSFUL in 45s`.
- Plugin unit tests: `BUILD SUCCESSFUL in 22s`; 13 result files, 70 tests, 0 failures, 0 errors.
- Isolated copy with five decisions reverted (`transferStopDecision`, `transferSelfStopId`, `pathEnd` stopping at whitespace, the `file:` rule off, `replacementFails`): `70 tests completed, 6 failed` — the two transfer tests, the three redaction tests and the R2 test. The copy was deleted afterwards.
- Not run: `android-clippy.sh`, cargo, `tsc`, node tests (no Rust, TypeScript or script changes).

## Files changed this round

- `KiwixTransferService.kt`, `KiwixMobilePlugin.kt` (one call site)
- `CrashIntake.kt`
- `TransferStartDecisionTest.kt`, `CrashCapturePolicyTest.kt`

No new user-visible strings, commands or codes.

## Doubts

1. **The queued-start count assumes Android delivers every start it accepted.** If one were ever dropped, the count would stay above zero and outside stops would keep skipping `stopService`. `stopOwned` would still stop an existing instance through its own bounded stop. `stopAfterFailure` is only called while a start is pending and nothing is running, so I see no path that leaves a running service behind, but I cannot run it.
2. **A slow start is still refused after 4 s** and transfers are paused by the timeout's error event. That policy is unchanged; only the process kill is gone.
3. **Redaction now drops more prose.** Everything between a path and its delimiter goes with it (for example "open [path]" loses "failed"). This is the same trade the Rust rule makes.
4. **A new compile warning line appears for `CrashIntake.kt:127`** (`timestampFormat.get().format(…)`, nullable receiver). That line is unchanged from `d98cac2`; the warning shows now only because the file was recompiled.