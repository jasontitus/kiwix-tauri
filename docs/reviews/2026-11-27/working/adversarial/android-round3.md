## Verdict for lane android (round 3): only non-blocking findings

R8 is closed. The crash-redaction port is not closed: Kotlin still runs the secret and bearer redaction before the path redaction, so the secret rule can stop at a space and leave the rest of a spaced path in the record. That is the one finding below. Nothing ran on an Android device or emulator.

### ANDROID-R10 — Kotlin redacts secrets before paths, so the end of a spaced path can survive
Severity: Should-fix (privacy; needs a secret-style key right before the path, so it is uncommon)
Fix under review: CORE-8 port in `CrashIntake.kt`
Evidence class: source-traced (I did not compile a scratch case)
Location: `CrashIntake.kt` `redact()`, which now runs `replaceUrls` → `bearerPattern` → `secretPattern` → `redactPaths` → `opaquePattern`
Trigger: an exception message such as `password=/storage/emulated/0/My Books/a.zim`, `token: /sdcard/My Books/x`, or `bearer /storage/My Books/x`.
Causal path:
1. `secretPattern`'s unquoted value `[^\s,;&)\]}]+` stops at the first space, giving `password=[redacted] Books/a.zim`.
2. `redactPaths` then finds no path start in `Books/a.zim`: `B` cannot start a path, and the `/` follows `s`, which is not a boundary character. So ` Books/a.zim` is kept.
3. `bearerPattern` (`\S+`) behaves the same way.
4. Rust's `redact` runs `replace_urls` → `redact_paths` → `redact_secret_values`. The path is consumed first, giving `password=[path]`.
5. So the port matches `path_end` / `path_at` but not the order of steps, and the order is what leaks.

Smallest correct fix: call `redactPaths` right after `replaceUrls`, before `bearerPattern` and `secretPattern`, which is Rust's order. Add a test asserting `redact("password=/storage/emulated/0/My Books/a.zim")` contains no `Books`.

### Per-fix assessment
- **R8: closed.** I could not make the counter drift into either failure:
  - **Single thread.** `requestForegroundStart`, `foregroundStartDelivered` and `stop` are `@Synchronized` and only run on main. A delivery is a main-looper message, so it cannot land between `startForegroundService` and the increment. A start that throws is never counted.
  - **Decrement.** It happens only for Android-delivered `ACTION_UPDATE` (`startId > 0`), and that start has exactly one call site. It is floored at 0.
  - **Process restart.** The counter resets to 0; it cannot go negative.
  - **A start that is never delivered.** The count then stays above zero, and outside stops skip `stopService`. Any running instance still stops itself through `finish()` → `stopSelf(deliveredStartId)`, so nothing is left running.
  - **No stop under a newer admitted start.** `finish()` stops only up to the newest delivered id. The STALE branch stops only when `!running`, using its own `startId`. A newer start that is still queued makes both of those a no-op. `stop()` always retires the generation, so a queued old start arrives STALE, enters the foreground once and stops itself.
  - **Latch.** The `stopOwned` latch contract is honest as documented: it is the latch of the instance that existed at the call, and a queued start can only delay it. The only caller (`transferState`) discards it.
  - **Doubt, not caused by this round.** If the process dies between `startForegroundService` and delivery, I recall the system restarts the service to deliver the pending start. In a fresh process the generation is 0 again, so a start whose admission was 0 would be admitted without a Rust owner. The window is milliseconds and the exposure was the same at `d98cac2`.
- **CORE-8 redaction port: not closed, because of R10.** Everything else in the port matches Rust:
  - `pathEnd` stops at the same characters as `path_end`: line breaks, `<`, `>`, backtick, a `"` not escaped by an odd run of backslashes (counted back to `start`, as Rust does), and `:` followed by whitespace. It trims trailing whitespace the same way.
  - `pathAt` has the same boundary characters and the same start forms as `path_at` (`X:\` or `X:/`, `/x` not followed by `/` or space, `~/`, `\\`).
  - The `file:` rule matches: `file:/` in any letter case runs to `pathEnd` from the `/`, and output is `[url]`. Other URLs still end at whitespace, `"`, `<`, `>` or a backtick, and the loop always moves forward.
  - Stack frames are redacted one frame at a time. `Class.method(File.kt:12)` and `File.kt:12` contain no path start, so they are kept, and a test pins this.

### What I ran
- Kotlin compile of the plugin and app module plus plugin unit tests (Gradle, offline, `--max-workers=2`, `--no-daemon`, `--rerun`): `BUILD SUCCESSFUL in 16s`, exit 0. 14 result files: `tests=70 skipped=0 failures=0 errors=0`.
- No Rust changed this round, so I did not rerun clippy or cargo.

### What I could not verify
- Real service start and stop behaviour, and the platform rules for redelivering a queued start.
- A crash record written on a device.