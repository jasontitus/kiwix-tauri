# Pi sweep review — kiwix-tauri-8d2619ee

Exhaustive per-file pass: 5 code files across 2 batches — model deepinfra/deepseek-ai/DeepSeek-V4.1-Flash-1017:high — 2026-11-12.
Tree: branch=main head=3d289b1013ae worktree=clean uncommitted=none

## Findings

- [medium] scripts/build-android-sdk.py:242 — Patch application is neither verified nor idempotent: the marker at 243 is the only record of success, and `patch --batch --fuzz=0` silently reverse-applies a patch that already looks applied (verified on this host with patch 2.0-12u11). — If the process dies, the disk fills, or the source tree is restored without dotfiles between a successful `patch` and `applied.write_text`, the next run has no marker, runs patch against already-patched files, un-applies the patch, exits 0, and then writes the marker claiming success, so the SDK is built from unpatched sources while `kiwix-sdk.json` lists the patches. The same in-place application also leaves a partially patched tree with no marker when a later hunk fails, so the documented "Rerun to resume this step" does not hold (the rerun re-invokes patch on mutated files).
- [low] scripts/build-android-sdk.py:190 — Every state file is written with a single non-atomic `write_text` and parsed back with an unguarded `json.loads`: the stage stamp (213), the work identity (148 → 146) and the previous manifest (151). — A crash, SIGKILL, or disk-full during `stamp.write_text` leaves truncated or empty JSON; the next run reaches `stamp.exists()` and raises JSONDecodeError before any discard-and-rebuild logic, so the documented resume path stays broken until the file is deleted by hand (same failure at 146 for `.kiwix-build-inputs.json` and at 151 for `kiwix-sdk.json`).
- [low] scripts/build-android-sdk.py:209 — The post-build output snapshot is taken with `output_root.rglob("*")` over the whole shared install prefix rather than the files this stage installed, so the `if not outputs` guard at 211 can only ever fire for the first stage, and every stage's reuse digests include all earlier stages' files. — A later stage whose install step produces nothing (disabled install rule, mis-set `CMAKE_INSTALL_PREFIX`, no-op `make install`) is stamped complete and only surfaces much later as a misleading "SDK is incomplete: lib<name>.a is missing"; separately, rebuilding any earlier stage changes digests recorded in every later stamp, forcing a full reinstall chain.
- [low] scripts/build-android-sdk.py:95 — `(ndk / "source.properties").read_text()` is unguarded, and the `--ndk` default at 79 uses `os.environ.get("ANDROID_NDK_HOME", …)`, which treats an empty value as a real path (`Path("")` → the current directory). — `ANDROID_NDK_HOME=""` (common in CI exports) or a wrong/missing `--ndk` aborts with a FileNotFoundError traceback instead of the script's own `parser.error("Use the pinned NDK r28c (28.2.13676358)")` diagnostic that the surrounding NDK checks use.
- [low] scripts/build-android-sdk.py:133 — Compiler flags are quoted with `shlex.join`, but the resolved prefix is interpolated raw into `CPPFLAGS` (`-I{prefix/'include'}`) and `LDFLAGS` (`-L{prefix/'lib'}`), which configure and make word-split. — A checkout under a path containing a space (or `--prefix '/tmp/my sdk'`) turns the include/library search paths into `-I/tmp/my` plus a stray `sdk/include` argument, so the build fails or drops the prefix from the search path.
- [low] scripts/build-android-sdk.py:172 — A checksum mismatch raises without deleting the cached archive, and the download branch at 167 is skipped whenever the file exists, so the bad file is never refreshed. — A corrupted or stale archive in a reused `--downloads-dir` fails every subsequent run at the same check with no hint that deleting the file is the fix.
- [low] scripts/build-android-sdk.py:169 — curl runs with `--retry` but no `--connect-timeout`/`--max-time`, and `subprocess.run` in `run()` passes no timeout. — A host that accepts the connection and then stops sending (or any hung build child) blocks the whole build indefinitely with no diagnostic instead of failing.
- [low] src-tauri/src/acquisition/transfer.rs:684 — The checksum-loop stop branch calls `worker_stop_action(...)` only for its boolean result and then passes the earlier `*signal.borrow()` value to `stop_file`, unlike the three other call sites that pass the returned action. — A second signal transition between the two borrows (a Pause arriving just after a foreground/background `Interrupt`) records the outcome for the stale value, so a user pause is stored as `interrupted=true` and the download can be auto-resumed on relaunch. One-line fix: use the returned action.
- [low] src-tauri/src/android_evaluation.rs:50 — `OperationDirectory::cleanup` sets `self.cleaned = true` before `fs::remove_dir_all`, so a failed removal returns `Err` and `Drop` then skips its own removal attempt. — A permission or busy-directory failure at cleanup leaves the per-run operation directory (up to the multi-gigabyte evaluation output) in the private files dir, and each subsequent run leaks another one.
- [low] src-tauri/src/exports.rs:2048 — No test asserts the completed-but-unconfirmed warning text: this test only checks `user_error::plain(&error) == error`, and the shared helper `assert_completed_but_unconfirmed` only requires `Err`, although the module documents that post-publication sync failures report unconfirmed durability. — Replacing the literal at exports.rs:335 with a generic error (or one that `plain()` rewrites) keeps every test green while the user is no longer told the file was kept.
- [low] src-tauri/src/exports.rs:2456 — The corpus harness `unwrap()`s `fs::read_to_string(&path)` for every directory entry and `parse_chunked(...)` for every page. — Pointing `KIWIX_HTML_CORPUS` at a directory containing a subdirectory or a non-UTF-8 page panics on the read, and a page that exceeds `ARTICLE_BOUNDS` panics on the parse instead of printing the over-budget numbers the harness exists to report (the panic message also omits the path).

## Coverage
scripts/build-android-sdk.py — NOT REVIEWED
src-tauri/src/acquisition/transfer.rs — NOT REVIEWED
src-tauri/src/android_evaluation.rs — NOT REVIEWED
src-tauri/src/exports.rs — NOT REVIEWED
src-tauri/src/mobile_rebase.rs — NOT REVIEWED

## Run stats

input 447463 tok (+220672 cached), output 4904745 tok — default requests, discounted — 5 files in 93m (3.2 files/h, 46.9 min/batch)

## Refuted in triage

Every line dropped from the sweep's Findings section, with the reason. "Merged" means the item shared a root cause with a kept finding and is covered by it.

- [medium] scripts/build-android-sdk.py:79 — merged into the :95 finding; the empty `ANDROID_NDK_HOME` path only matters because the properties read is unguarded.
- [low] scripts/build-android-sdk.py:95 — merged into the :95 finding (duplicate of the same unguarded read).
- [low] scripts/build-android-sdk.py:133 — merged into the :133 finding; same raw-prefix interpolation.
- [low] scripts/build-android-sdk.py:146 — merged into the :190 finding; same non-atomic write plus unguarded parse.
- [low] scripts/build-android-sdk.py:151 — merged into the :190 finding; same root cause in the manifest read.
- [low] scripts/build-android-sdk.py:161 — refuted: `stage()` declares `nonlocal log` and opens the log file before calling `build()`, and every `run()` call happens inside a stage's with-block, so the `log = None` initializer is never dereferenced and no AttributeError path exists.
- [medium] scripts/build-android-sdk.py:176 — refuted: `SOURCES` is part of `identity`, so any checksum change makes the `.kiwix-build-inputs.json` check at 146 demand a fresh work dir; the stale `.kiwix-extracted` marker is unreachable.
- [medium] scripts/build-android-sdk.py:176 — refuted for the same reason (duplicate of the marker/checksum claim).
- [low] scripts/build-android-sdk.py:181 — refuted: every pinned archive (zlib, xz, zstd, xapian, icu4c, libzim) has a single top-level wrapper directory and archives are checksum-pinned, so the root-member skip cannot drop a file today.
- [low] scripts/build-android-sdk.py:181 — refuted for the same reason (duplicate).
- [low] scripts/build-android-sdk.py:182 — refuted: no pinned archive carries hardlinks or top-prefixed linknames, and relative symlink targets still resolve after the wrapper directory is stripped; latent only, no shipped trigger.
- [low] scripts/build-android-sdk.py:182 — refuted for the same reason (duplicate).
- [low] scripts/build-android-sdk.py:182 — refuted for the same reason (duplicate).
- [low] scripts/build-android-sdk.py:190 — merged into the :190 finding.
- [low] scripts/build-android-sdk.py:190 — merged into the :190 finding (third copy).
- [low] scripts/build-android-sdk.py:204 — refuted: the rebind is deliberate and every `run()` call is inside the with-block that rebinds `log`, so the closed-file case has no caller.
- [medium] scripts/build-android-sdk.py:209 — merged into the :209 finding.
- [medium] scripts/build-android-sdk.py:209 — merged into the :209 finding (third copy).
- [medium] scripts/build-android-sdk.py:238 — merged into the :242 finding; the marker is never compared against source content, which is the same missing-verification root cause.
- [low] scripts/build-android-sdk.py:242 — merged into the :242 finding (partial-patch rollback is part of the same in-place-application defect).
- [medium] scripts/build-android-sdk.py:242 — merged into the :242 finding (second copy of the non-idempotent re-apply claim).
- [medium] scripts/build-android-sdk.py:268 — refuted: `work` is `.resolve()`d at line 88, so the `--prefix={work/'icu-host'}` argument is absolute and cannot be re-based on the `host_build` cwd.
- [medium] scripts/build-android-sdk.py:269 — refuted: startup validation rejects a non-positive job count (`args.jobs < 1` → `parser.error`).
- [medium] scripts/build-android-sdk.py:269 — refuted: nothing consumes `work/icu-host`; the cross build uses `--with-cross-build=host_build` and `host_build/bin/icupkg`, so the uninstalled prefix is inert, not missing.
- [medium] scripts/build-android-sdk.py:271 — refuted: `host_build` is the stage's `output_root`, not its completion marker; the stamp is written only after `build()` returns, so a failed build leaves no stamp and the stage re-runs.
- [low] scripts/build-android-sdk.py:273 — refuted: ICU 73 is pinned in `SOURCES` and the recipe hash forces a fresh work dir, so `icudt73l.dat` matches the pinned data set.
- [medium] scripts/build-android-sdk.py:340 — refuted: verified locally with meson 1.12.1 that `meson setup <build> <source> --reconfigure` exits 0 and reconfigures; no rejection to work around.
- [low] scripts/build-android-sdk.py:353 — refuted: no pinned source ships a lowercase `copying` file, so the case-sensitivity asymmetry has no trigger; the real license files are found.
- [medium] scripts/build-android-sdk.py:363 — refuted: `prefix/share/provenance` is created in the mkdir loop at line 109, so the copies at 363-366 have their parent.
- [high] src-tauri/src/acquisition/transfer.rs:574 — refuted: a 206 always carries a total (`parse_range` requires `<start>-<end>/<total>` and rejects `end >= total`), so `total` is never None on that path; the `total.is_none()` at 574 only back-fills the durable row for a length-less 200.
- [high] src-tauri/src/acquisition/transfer.rs:580 — refuted: a 206 requires `range.start == downloaded` with `length >= 1`, and a short body trips the declared-length check at 562, so each iteration either advances `downloaded` or errors; there is no non-progressing loop.
- [medium] src-tauri/src/acquisition/transfer.rs:680 — refuted: the watch sender lives in `state.running` and is removed only after the worker's own result is computed, so the channel cannot be closed while hashing runs; `changed()` cannot return Err there.
- [medium] src-tauri/src/acquisition/transfer.rs:689 — refuted: `tokio::fs::File::read` keeps the in-flight blocking op and its buffer in the `File`'s own state, so a dropped read future's bytes are handed to the next read; dropping the branch does not skip bytes or desync the cursor.
- [medium] src-tauri/src/android_evaluation.rs:42 — refuted: this module is gated to `feature = "android-evaluation"` debug Android builds and operates on the app's own private files dir, so a racer needs same-UID or root write access and gains no capability it does not already have.
- [low] src-tauri/src/android_evaluation.rs:50 — merged into the :50 finding (duplicate).
- [low] src-tauri/src/android_evaluation.rs:99 — refuted: `tree_bytes` walks a tree the evaluation itself produced, with symlinks rejected and depth bounded by the path length limit; no attacker-controlled deep tree exists.
- [low] src-tauri/src/android_evaluation.rs:130 — refuted: the JNI entry resets `PHASE` to 0 after every `run()` (line 244), including all error and negative-case returns, so a stale phase is not observable by the poller.
- [low] src-tauri/src/android_evaluation.rs:140 — refuted for the same reason (duplicate of the stale-phase claim).
- [medium] src-tauri/src/android_evaluation.rs:159 — refuted: `Archive::open_files` passes `zim::FdInput{fd, 0, size}` and libzim reads by offset, so the handle's current position after the footer read is irrelevant.
- [low] src-tauri/src/exports.rs:1675 — refuted: unverified inference, and the accounting shape contradicts it — the deferred EOF drain walks the ~500-level ancestor stack per text node (the neighbouring cancellation test notes "tens of millions of stack comparisons"), dwarfing the 500-tag initial parse, so the second parse does reach finalization.
- [low] src-tauri/src/exports.rs:1689 — refuted for the same reason (short source, same 1.5× budget shape, same dominant deferred charge).
- [low] src-tauri/src/exports.rs:2154 — refuted: template-content inclusion is a deliberate, commented policy in this extractor ("template contents are selectable and part of an ancestor's text") with tests on both text and base selection; it is a product decision, not an unintended spec violation.
- [low] src-tauri/src/exports.rs:2379 — refuted: 10 units/byte is a deliberate canary margin below the documented 12-unit ceiling, not a mis-stated bound; a parse creeping toward the ceiling is meant to fail this test first.
- [low] src-tauri/src/exports.rs:2434 — refuted: `article(28_000)` is ~8 MiB (`article(56_000)` is asserted > 15 MiB) and cancellation is checked per 64 KiB chunk, so the 20 ms sleep and 100 ms post-cancel bound both carry large margins.
- [low] src-tauri/src/exports.rs:2434 — refuted for the same reason (duplicate).
- [low] src-tauri/src/exports.rs:2438 — refuted for the same reason (duplicate).
- [low] src-tauri/src/exports.rs:2456 — merged into the :2456 finding (duplicate).
- [medium] src-tauri/src/mobile_rebase.rs:141 — refuted: `overlaps()` (line 477) drops any recorded or intermediate root nested with the current container in either direction before the `Relocation` is constructed, so the old_root/new_root nesting the double-prefix requires cannot occur.
- [medium] src-tauri/src/mobile_rebase.rs:169 — refuted for the same reason: a `new_root` that is an ancestor of `old_root` is exactly the rejected overlap, so no path is silently left behind.
- [low] src-tauri/src/mobile_rebase.rs:934 — refuted: the marker's `profile`/`capability` identities are unconditionally rebuilt from the live directories at the end of `initialize()`, and the neighbouring test asserts those identities; this is a coverage suggestion, not a defect.
- [low] src-tauri/src/mobile_rebase.rs:1019 — refuted: `rebase_json` only does lexical prefix rewriting, so it returns false for `OUTSIDE_PROFILE` regardless of the zeroed inode; the zeroed inode was a negative-evidence probe for `validate_evidence`, which the test calls before the substitution.
- [low] src-tauri/src/mobile_rebase.rs:1025 — refuted for the same reason (duplicate of the mutation-not-restored claim).