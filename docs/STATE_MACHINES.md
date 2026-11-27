# State machines

This file lists the application's explicit and implicit state machines. For each one it gives the states, the invariants the code relies on, and a transition table. It also records the invalid states and transitions that the code could actually reach, with the evidence, the fix and the regression test. Paths are relative to the repository root. Symbols are cited instead of line numbers because line numbers drift. **Keep this file updated whenever a transition, guard or invariant changes.**

"Explicit" means a status enum or string field. "Implicit" means state carried by flags, `Option` slots, refs, generations or locks.

Initial audit snapshot: 2026-11-07, branch `claude/app-state-machines-doc-3zj50h`. The scope is the native service and job managers (`src-tauri/src`) and the React shell (`src`). Platform-native Swift/Kotlin code was read only where a Rust transition depends on it. The transition descriptions below incorporate the [November 17 review](reviews/2026-11-17/PROJECT-REVIEW.md), the [November 23 remediation](reviews/2026-11-23/PROJECT-REVIEW-REMEDIATION.md), and the [November 25 remediation](reviews/2026-11-25/PROJECT-REVIEW-REMEDIATION.md).

---

## 1. Application and service lifecycle

### 1.1 Service retirement (`src-tauri/src/service.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/quit.rs`)

States: **Running** → **Stopping** (`Service.stopping = true`, which is never reset) → **Retired** (`Service::shutdown` returned `Ok`) → process exit.

The Quit attempt (`QuitManager.attempt`) moves **Idle** → **Preparing** (flush requested from every shell) → **Prepared** (all flushed; the attempt is *held* through `Service::shutdown`) → exit, or back to **Idle**.

| From | Event | To | Where |
|---|---|---|---|
| Running, Idle | `RunEvent::ExitRequested` (not yet authorized) | Preparing; `exiting=true` | `lib.rs` run callback, `quit::prepare` → `begin_attempt` |
| Preparing | flush error, timeout or cancellation | Idle (`cancel` emits `kiwix-quit-cancelled`), `exiting=false` | `quit::prepare` |
| Preparing | every shell acknowledged | Prepared | `quit::prepare` |
| Prepared | `Service::shutdown` → Ok | Retired, `authorized=true`, then `app.exit`, or `app.request_restart()` when an installed update armed a relaunch | `lib.rs`, `update::take_restart_after_quit` (fix F-2) |
| Prepared | `Service::shutdown` → Err | Stay Prepared and show the retirement error with a native Retry button. The next Quit reuses the flushed session and retries shutdown. An installed update retains its relaunch request. | `lib.rs`, `quit::prepare`, `quit::needs_flush` |

Invariants:
- Every command passes `ensure_running` at dispatch entry and again after the privacy lease is taken.
- `is_preparing` refuses new windows while an attempt exists.
- `Service::shutdown` is repeatable: after the cause is repaired it re-joins whatever is left (`shutdown_surfaces_failed_download_checkpoint_and_retains_retry_ownership`).
- Final shutdown takes the privacy **write** lease. Therefore nothing may block while holding a privacy read lease and waiting for the exit to finish.
- Tauri ignores `prevent_exit` for `RESTART_EXIT_CODE`. So `restart()`/`request_restart()` must only be called once Quit has authorized the exit; anything earlier skips the flush and retirement (see F-2).

### 1.2 Privacy / read-only mode and its epochs (`service.rs::operation`, `service/dispatch.rs` `save_settings`)

Modes: Normal, Private, Read-only. Mode changes are made only by `save_settings` while holding `privacy_gate.write()`.

Epochs:
- `privacy_generation` is the browsing epoch.
- `privacy_policy_generation` is the policy epoch.

`launch_generation()` is not a third epoch: it returns `privacy_generation`, and the launch queue compares against that.

Both are advanced only while the write lease is held. A privacy or read-only change in `save_settings` advances both; mobile reader retirement in the background advances only the browsing epoch.

Lock order in `Service::operation`: `update_admission` → `mobile.transfer_gate` (mobile) → `settings_writes` (`save_settings`) → `privacy_gate`. `Service::shutdown`, `mobile_resume_policy` and the iOS acquisition bootstrap also take `transfer_gate` before the privacy gate. A task that holds a privacy read lease must not acquire a second one: Tokio's `RwLock` is fair, so a writer queued between the two acquisitions blocks the second read and every command behind it.

`save_settings` order:
1. Admission checks: `Service::validate_settings_patch`. These run **before any retirement** (fix F-6).
2. Retire cancellable work: LocalSwarm/Nearby, searches, private sync, integrity checks, PDF jobs.
3. Take the privacy writer.
4. Re-run the admission checks under the writer.
5. Replace the import manager.
6. When entering a restricted mode: stop LocalSwarm, call `downloads.shutdown()`, cancel maps, take and shut down torrents, stop sharing.
7. Bump the epochs and retire edition authorizations and readers.
8. Persist settings and apply the new limits.
9. Before releasing the writer, notify shells whenever the epoch advanced, including on failure. Clear queued launches if the effective policy changed or cannot be read.

Invariants:
- `operation` takes no read lease for `catalog`, `catalog_facets`, `catalog_torrent`, `find_updates`, `map_prepare`, `map_recipe_metadata`, `shared_library_browse`, `sensor_location` and `update_install`, so a network or native wait cannot hold up a privacy writer. Each admits itself. The catalog commands, `find_updates`, `map_prepare` and `shared_library_browse` check the caller's generation, wait without a lease, and re-check under a short read lease before the result is used. `map_recipe_metadata` checks the caller's generation once, before its fetch. A `sensor_location` wait owns no writable resource. `update_install` takes its own leases (§1.5).
- A listener refuses requests from an older epoch.

### 1.3 Reader listeners (`server.rs`, `Service.readers`)

Per `(window, archive)`: **absent** → **started** → **retired** (joined).

| From | Event | To |
|---|---|---|
| absent | `Service::reader` under a read lease, after `ensure_window_active` | started; captures the epoch |
| started | `close_window`, a privacy switch, a missing watched book, shutdown | retired. `ReaderServer::shutdown` consumes the listener and joins its workers. |

Invariant: a request carrying an older epoch is refused, so a retired origin can never serve the new mode.

### 1.4 Store (`store.rs`)

Every change is made on a clone and persisted atomically, then published in memory, with one exception. **Coalesced browsing telemetry** (`Store::update_browsing`, used by `record_visit` when it refreshes the reading position of an already recorded visit): a change confined to `history`/`recent_searches` that arrives within `BROWSING_COALESCE_INTERVAL` (5 s) of the last write is published in memory with `browsing_pending` set and is not written. It becomes durable with the next transaction of any kind, with the first refresh after the interval, in `Store::flush_browsing` (called by `Service::shutdown` and by the mobile background transition), and before a cutover to private mode clears live browsing. A crash or kill can lose only the offsets recorded inside that window. Private and read-only profiles never write it. **Durability-latched**: if the commit succeeded but the durability step failed, the in-memory state is replaced and `durability_error` refuses later writes until restart. `runtime_read_only` refuses all updates. While `settings.read_only` is set, the only allowed change is leaving read-only. Schema versions 1 and 2 are accepted; anything else is refused.

### 1.5 Self-update install (`update.rs`, `update_admission.rs`, `service.rs`)

The `AdmissionGate` (an RwLock) ensures transfer and migration work holds read leases while install holds the write lease. Busy state is the `OPERATION` flag; `PENDING` holds a found update. `ActiveDownloadCancel` registers a monotonic download token before `"downloading"` progress is emitted.

Flow: blocker check → download (under privacy read lease only; cancellable via desktop `"update_cancel"`, which `Service::operation` handles before `privacy_gate` so a pending privacy writer cannot deadlock cancellation) → drop privacy read lease → acquire `update_admission.install()` → recheck privacy generation/mode/blocker → verify → install → **arm `RESTART_AFTER_QUIT`, call `app.exit(0)`, return** (Windows: the installer exits the process).

The command returning releases the dispatch privacy read lease and the admission write lease. The preventable `ExitRequested` then runs the ordinary Quit path (§1.1). Only after `Service::shutdown` succeeds does it call `request_restart()`. Cancelling the shell flush clears the relaunch. A retryable failure after the shells have flushed preserves it. User cancellation aborts an active download (`USER_DOWNLOAD_CANCELLED`) while `ActiveDownloadCancel` is registered—including a cancel racing with download completion before `"verifying"`—and once `ActiveDownloadCancel` is dropped on entering `"verifying"`, `"update_cancel"` returns `false` and cannot cancel verification or install.

### 1.6 Instance lock (`lib.rs`, `instance.rs`, `store.rs`)

**Unlocked** → `flock(store.lock)` → **Owner**: publishes `instance/rendezvous.sock` and holds the lock for the lifetime of the `Store`.

A secondary launch moves **Busy** → forward its argv to the owner, or show a dialog and exit 0. No reachable invalid state was found.

### 1.7 Mobile foreground gate and maintenance cadence (`mobile.rs`, `mobile_service.rs`, `src/api.ts`, `src/suspended-retry.ts`)

Native lifecycle flags: `active`, `desired_active`, `inactive` (Android: an obscuring Activity), `background`.

A command is admitted only when `active && desired_active && !inactive`. Otherwise it fails with `KIWIX_SUSPENDED`.

On iOS, the foreground `maintain()` loop polls at 1 s while reconciliation, local transfers, or Live Activity are active, and backs off to 5 s when completely idle. `transfer_signal_generation` and `transfer_reconciled_generation` record new transfer work and native signals so reconciliation wakes promptly when work starts without a background spin loop.

Frontend `ForegroundQueue` waiter: **waiting** → woken by
- **native**: `kiwix-mobile-active`, which is authoritative; or
- **fallback**: a visibility or focus timer, which is a guess.

`retrySuspended` then:
- runs the command once more after a *native* wake;
- after a *fallback* wake whose retry is suspended again, keeps waiting for the next wake, up to `MAX_FALLBACK_RETRIES` fallback attempts; then it drops the command as before, because the native event may never arrive (fix F-11).

A request whose context moved on (`stillRelevant`) is dropped quietly.

---

## 2. HTTP acquisition (`src-tauri/src/acquisition*.rs`)

### 2.1 Job status

`DownloadJob.status`: `queued | downloading | verifying | paused{interrupted} | complete | failed | cancelled`, plus the flags `history_dismissed`, `files_deleted`, `publication` (intent), `published_ownership`, `verification` (receipt), `segmented`/`segments`, `native`.

Worker types:
- `Signal {Run, Pause, Interrupt, Cancel}`, one watch channel per worker.
- `Outcome {Complete, Paused, Interrupted, Cancelled}`.
- `WorkerMode {Transfer, DurableFinalize, NativeAttach}`.

| From | Trigger | To | Where |
|---|---|---|---|
| ∅ | `start_transport` | queued | `manager.rs` |
| queued | worker admitted (guarded on `queued`) | downloading | `transfer.rs` |
| downloading | last byte synced, **under the state lock with no Pause/Cancel command** | verifying | `transfer.rs` (fix F-8) |
| downloading | last byte synced, Pause/Cancel already persisted | paused / cancelled via `stop_file` | `transfer.rs` (fix F-8) |
| queued / downloading / verifying / paused+interrupted with a durable plan and no worker | recovery sweep claim, **only when finalizing is allowed** | verifying | `finalize.rs::sweep_durable_finalize` (fix F-7) |
| verifying | `publish_verified_download` | complete | `finalize.rs` |
| verifying | finalize forbidden (private/read-only) | paused+interrupted | `finalize_durable_download` |
| any non-complete | `stop()`: intent persisted before the worker acknowledges | paused / cancelled | `manager.rs::stop` |
| running | worker settlement, including a worker panic | paused / paused+interrupted / cancelled / failed | `acquisition.rs` |
| paused / failed | `queue_resume` | queued, or complete through recovery | `manager.rs` |
| queued / downloading on load | load repair | paused+interrupted | `persistence.rs` |
| terminal | remove / forget / delete files | dismissed / removed / files_deleted | `manager.rs` |

Invariants:
- **I1:** one worker per job (`state.running` changes only under the state lock).
- **I2:** `complete` implies the archive has been published (the intent is saved before the rename).
- **I3:** a persisted manual Pause/Cancel survives a crash and is never overwritten by a worker checkpoint.
- **I4:** a manual pause is never auto-resumed.
- **I5:** a preallocated part file is never treated as complete without a durable plan.
- **I7:** `Complete` is never downgraded.
- **I8:** nothing is finalized, published or swept while in private or read-only mode.

Start, resume, pause and cancel require `normal_writable` at dispatch entry (`dispatch.rs`), so they are refused in private mode.

### 2.2 Segments, retries and frontend state

Each segment range is unstarted, partial or full. `durable` is copied into the journal on each `SYNC_INTERVAL` and on stop. Single-stream transfers retry transient connect errors (excluding deterministic redirect rejections) and HTTP `429`/`5xx` responses before checking `CONTENT_ENCODING` (so gzipped error responses still retry), and reset the body retry counter once payload bytes stream. SHA-256 verification runs on `spawn_blocking`.

A published HTTP archive can carry a separate `historyWarning` when saving the completion record or synchronizing its publication directory fails. Downloads displays the warning separately. Opening an available archive still requires its matching imported receipt. Recovery can use the previously saved publication intent.

The frontend `downloadStatusDecision` (`src/download-state.ts`) uses this priority: confirmation > cancelled > paused(interrupted) > verifying > failed > seeding/completed > unavailable > preparing.

---

## 3. BitTorrent (`src-tauri/src/torrent*.rs`)

### 3.1 Job record

The status string is one of `queued, resolving, publishing, checking, downloading, paused, completed, seeding, creating, error, cancelled`. Status is combined with `paused`, `interrupted`, `verified`, `job.seeding` (seed *intent*), `share`, `history_dismissed`, `files_deleted`.

| From | Event | To |
|---|---|---|
| — | `start_inner` / `share` | queued / creating (share: verified=true, seeding=true) |
| queued | `run` | resolving → publishing (recovery) or checking/downloading |
| checking/downloading | 250 ms tick | checking / downloading / publishing / seeding / completed (never overwrites `cancelled`) |
| any non-cancelled | `pause` | paused, or completed if verified (a share also gets seeding=false) |
| any | `cancel` | cancelled, seeding=false |
| non-cancelled | `resume` / `seeding(true)` (**refused while another live share of the same file exists**) | queued, or seeding if verified (fix F-13) |
| recovering a published archive | verified publication, no engine | completed with seeding off |
| running | worker error, not cancelled (`apply_worker_error`) | error. **Seed intent is kept** so Retry restarts the seed. |
| terminal | `finish_terminal` / `forget_terminal` | dismissed / files deleted / removed. An errored row is removable even if its seed intent is still set. The intent is not mutated, so a refused removal leaves Retry working (fix F-3). |

Invariants:
- `removable_terminal(r) = (!seeding || status ∈ {error, failed}) && status ∈ {completed, complete, failed, error, cancelled}`.
- A share always has verified=true.
- There is at most one engine handle per info hash (librqbit `AlreadyManaged`).

### 3.2 Runtime and worker

**Absent** → `spawn` → **Running** (generation, token, `retired` receiver) → token cancelled (pause, cancel, suspend, shutdown, finish) → **Retiring**. Retiring removes the runtime, calls `engine.delete`, `apply_worker_error`, then signals `retired`, and ends **Absent** or, if delete failed, **Fenced** (`retirement_failures`). Fenced blocks spawn, finish and forget until `suspend` or a restart.

### 3.3 Manager

**Running** ⇄ **Suspended** (mobile). Either → `shutdown` (terminal; the dispatch slot is `take()`n and lazily recreated).

### 3.4 Journals and storage concurrency

- **Metadata:** none → `metadata_temporary` → +cleanup → final.
- **Archive publication:** temporaries → intents → no-clobber rename → `completed_paths`. Windows publication uses positional `seek_read` so concurrent seeding does not race on a shared file cursor.
- **Storage locking:** `ConfinedStorage` uses an `RwLock` gate (`read()` running positional `pread_exact`/`pwrite_all` on a cloned `Arc<File>` outside the `files` mutex; `write()` for initialization, length changes, and fencing).
- **Staging:** every file or directory the engine creates is journaled right after it is created. **If the journal write fails, the new empty entry is removed**, so later cleanup cannot meet an unjournaled entry of its own (fix F-14). Cleanup: capture journaled → payload unlinked.

---

## 4. Peer features

### 4.1 Web sharing server (`sharing.rs`, `Service.sharing`)

None → Running(Selected | Library) → Stopping (the server stays in the slot until shutdown returns) → None.

Lock order: `hotspot_use` → `sharing`. No archive read starts after `SharingCancellation::cancel`; `SharingCancellation::drain_native` runs on `spawn_blocking` during shutdown so async worker threads are not blocked. `SessionHosts` caches interface/host bindings with a 2 s TTL, and `LibraryHosts` caps per-book HTTP listeners via LRU eviction (while `LibraryArchives` bounds cached library archive handles).

### 4.2 Hotspot (`hotspot.rs`)

Idle → Starting → Active (`accept`, which checks the generation). Starting|Active → Stopping → Idle, or back to Active on `retry`. Any state → Idle on `lost`.

### 4.3 Nearby BLE (`nearby_service.rs`)

There is one publisher and one consumer `Owner`; each carries a `WorkOwner{id, generation, cancel}`.
- **Publisher:** starting → advertising → joining (iOS reverse join) → advertising → stopped / failed.
- **Consumer:** starting → discovering → connecting → joining → connected / failed.
- **Peer list:** `nearby_peer` (at most 16) and `nearby_lost`.

Invariant: the accepted `descriptor` (route plus cancel token) is the authority for every receipt and grant created from it.

| From | Event | To |
|---|---|---|
| connected to P | browse / refresh / re-select of P while its descriptor is live | **stays connected**. A new receipt is minted on the same descriptor; running downloads keep their grant (fix F-4). |
| connected to P | browse of P fails on the live route, or another peer is selected | `nearby_connect_selected`: cancel the descriptor → connecting |
| connecting to P | `lost` event for **another** peer | stays connecting; that row is dropped (fix F-17) |
| connecting to P | `lost` event for P | failed. On iOS a fresh advertisement for the same sender can make it selectable again. |

### 4.4 LocalSwarm (`localswarm/*.rs`)

- **Library offer:** None → Offered → rebind (new generation and cancel token) → None (stop, supervisor, quiesce, iOS admission off).
- **Jobs:** queued → downloading → verifying → publishing → complete; also paused / cancelled / failed.
- **Seeds & setup fencing:** inserted → supervised → `finish_offer`. Provider `SourceSet::prepare` and `start()` network dials run outside the `operation` mutex under read leases of `seed_setup_gate` / `receive_setup_gate` and `sharing_lifetime`, so `stop()` / `stop_sharing()` still fences in-flight setup without blocking unrelated commands. `SourceSet::retained` uses `RwLock` with positional `read_exact_at`. Cleanup caches file digests keyed by `(swarm_id, index, dev, ino, len, mtime_ns, ctime_ns)` with racy-timestamp separation.

### 4.5 Private sync (`private_sync*.rs`)

The runtime is serialized and revision-fenced; `cancel()` bumps the revision. On mobile, if the pinned `LocalBinding` route disappears (`endpoints()` fails on an automatic-sync tick or before reuse), the listener exits and the next activation retires any prior runtime/session/activity before rebinding.

Pairing: waiting → verifying → local confirm → Confirm exchange → capacity preflight → Ready exchange → policy and capacity recheck → durable local enrollment → confirmed. A pre-commit error retires the session and preserves the previous enrollment. A confirmed session stays confirmed. Invitations are single-use.

---

## 5. Maps, imports and the library

### 5.1 Map job (`maps/manager.rs`, `map-job.json`)

Status: `queued, downloading, validating, composing, paused, failed, cancelled, complete`. Orthogonal fields: `resumable`/`resume`, `owned_partial`, `recovery_records`, `published`, `retained_publications`, `decision.session` (cleared means revoked). In memory: `active: Option<Arc<Control>>`, with `Control.retirement` 0 = run, 1 = pause, 2 = cancel.

| From | Event | To |
|---|---|---|
| none / terminal | `start` (persist the new job). **On a persist failure the previous job keeps its unresolved `owned_partial`.** | queued, active (fix F-15) |
| paused/failed/cancelled + resumable | `resume` | queued, active |
| running | `finish` after a pause request | paused only on cancellation with a resumable checkpoint. Other errors remain visible as failed. |
| any non-complete | `cancel`: retire the worker, then the tail (clear the session, reclaim the partial, paused→cancelled). **The recovery and cleanup steps recheck the admission generation, including when a newer worker has already finished.** | cancelled (fix F-16) |
| restart, running status | `MapManager::new` | paused (resumable) / failed / complete (recovered) |

Invariants:
- At most one active worker.
- Every unresolved partial or publication stays in `owned_partial ∪ recovery_records`, so it keeps a monitored-path claim.
- `MapJob` tracks throttled `speed` and `eta_seconds` (reset on startup, resume, completion, finish, and error backoff). `RECIPE_CACHE` retains at most 1 (mobile) / 2 (desktop) parsed `Arc<Recipe>` entries for 5 minutes and is cleared by `clear_recipe_cache()` on `MapManager::clear_cache()`, `MapManager::cancel()`, `CatalogRegistry::retire()`, `Service::shutdown()`, privacy changes in `save_settings`, and mobile background entry.

### 5.2 Import verification (`imports.rs`, in memory)

Queued → Verifying → Ready / Failed. Ready → `ack` removes the entry. Failed → `retry` → Queued. Verification reports throttled, monotonic `verified_bytes` / `verified_total` across single-file and multi-file batches (reset on start and retry).

`retire_acquisition` cancels the acquisition's entries, adds a tombstone and reports whether this call inserted it. **When the delete or forget it guarded is refused, the caller that inserted the tombstone (and only that caller) releases it, together with its cancelled entries, through `unretire_acquisition`** (fix F-18). Shutdown cancels all entries.

### 5.3 Monitored folders (`watch.rs`)

- **Per file:** unobserved → pending (re-armed on a fingerprint change) → settled after 1 s of quiet → `apply_watch` (under the admission and privacy read leases).
- **Shutdown:** stop flag → `quit` token, which interrupts only the wait for the next message. **An observation being applied is drained for up to `DRAIN_LIMIT` (5 s)**. After that the consumer is aborted, and the still-running blocking import refuses to register because `import_monitored_paths` checks `ensure_running` before `register_books` (fix F-5).

### 5.4 Library move journals (`library_files.rs`, `library_root.rs`)

- **Desktop:** journal → same-filesystem rename or cross-volume staged copy → publish → cleanup → journal removed. Replay on start.
- **Mobile:** Copying → Publishing → Store commit → Committed → cleanup. Recovery checks the configured root, journal phase, registered archive paths and file identities. When the preferred folder has been switched back following a committed move, it verifies the registered destinations and keeps the original files.

### 5.5 Catalog cache (`catalog_cache.rs`)

absent → stored. The conditional request gives 200 (save), 304 (refresh the record only if the response still matches the durable representation) or an offline stale result, and is fenced by the privacy generation.

### 5.6 Kokoro local speech (`kokoro.rs`)

Lock order: `cache_gate` → `state`. Audio publication (`publish_audio`) and cache eviction (`clean_cache`) hold `cache_gate` rather than `state`, so `stop()`, `pause()`, `seek()`, and `status()` do not block on WAV `sync_data()` or cache sweeps. `delete()`, `drain_download()`, privacy cutover, and shutdown fence `cache_gate`.

---

## 6. React shell

### 6.1 Root snapshot and hydration (`src/App.tsx` `refresh`)

Refs: `refreshSerial`, `hydrated`, `observedPrivacy`, `privacyPending`, `snapshotSuspended`, `sessionGeneration`, `privacyRetry`.

| From | Event | To |
|---|---|---|
| cold | first successful snapshot | hydrated |
| hydrated | `kiwix-privacy-changed` | privacyPending (blocks tab mutations and autosave) |
| privacyPending | successful snapshot | hydrated. On a boundary change: `sessionGeneration++`, closed stack cleared, readers remounted. |
| privacyPending | failed (not suspended) snapshot | privacyPending, **plus a backoff retry (1 s … 30 s)**. The error is reported once per series, not on every attempt (fix F-10). |
| privacyPending | suspended snapshot | retried by the API foreground gate (§1.7) |

Only the newest refresh applies (`id === refreshSerial`).

### 6.2 Tabs, live scroll cache and closed-tab stack

`latestSession` is updated synchronously before `setTabs` in every mutation.

- `appendTab` → `insertTab` (`src/tab-stack.ts`) **refuses an ID that is already open**.
- `undoClose` pops exactly the reopened tab (`removeReopened`). If that tab is already open, its stale stack entry is dropped and the tab is selected, so older closed tabs stay reachable (fix F-9).
- `closeTab`: freeze reader storage → re-check → push onto the closed stack (at most 20) → resume.
- Scroll-only updates stay in `liveScrolls` and are merged lazily via `mergeLiveScrolls(tabs)` (cached on `(tabs, liveScrollVersion)` and cleared on tab/session commits), avoiding full-shell React re-renders while preserving reference stability for `flushSession` (`session.tabs === latest.tabs`) and `checkpointTabIndex`.

Invariant: tab IDs are unique within a window (React keys, workspace keys, `closeTab` filter, saved session). `<ReaderHost>` stays continuously mounted across tab switches (its `<PaneBoundary>` sits inside `<div key={epoch}>`) so tab switches never bypass `retireReaderDocuments`.

### 6.3 Reader (`src/components/Reader.tsx`, `src/reader-bridge.js`)

- `openSequence` / `navigationSequence` are re-checked after every await.
- `searchSequence`.
- `bookmarkWrite` single-flight.
- `authorityRefresh {generation, pending}`: **a refresh resolving `false` (or throwing) resets the generation, so a stale frame can ask again** (fix F-10).
- `HistoryDisclosure` closes on `Escape` only when no `<dialog open>` is present and stops propagation.
- `reader-bridge.js` defers `selectionchange` scans during primary-pointer drags (`pointerSelecting`) and resets on `pointerup`, `pointercancel`, `mouseup`, `blur`, `visibilitychange` (hidden), `contextmenu`, `dragstart`, `dragend`, and mouse `buttons === 0`.

### 6.4 Session save, Quit and launch queue

- **Autosave:** debounced 220 ms and serialized through `sessionQueue`. Gated on hydrated, not privacyPending, no Quit request, no explicit flush, and an unchanged generation.
- **Quit:** request → freeze → flush → `ack_quit_flush`; `kiwix-quit-cancelled` resumes.
- **Launch queue:** unseen → consumed and acknowledged, or capacity-blocked and retried. The launch ID is also the tab ID.

### 6.5 Reader storage leases (`src/reader-storage.ts`)

`freezeLeases`, `retiringDocuments`, `suppressFreezeRestore` (sticky), `deferredRestores`, `quarantinedBindings` (terminal; never cleared by `retireReaderStorageGeneration`). `adoptFreeze` skips pre-ready registered iframes (`!binding.ready`) rather than waiting 5 s. Restore waits until there are no leases, no retirement and no quarantine. The double-close race was traced and is handled.

---

## 7. Reachable invalid states and transitions

Severity reflects the user-visible impact. "Repro" means a regression test that fails without the fix (checked by reverting the fix and re-running the test).

Tracking issues:
- #9: lifecycle (F-1, F-2, F-5)
- #10: mode switches (F-6, F-7)
- #11: persisted intent races (F-8, F-15, F-16, F-18)
- #12: torrent (F-3, F-13, F-14)
- #13: Nearby (F-4, F-17, O-5, O-6)
- #14: shell (F-9, F-10, F-11)
- #15: open findings O-1 – O-12

### Fixed

| ID | Sev | Machine | Invalid state or transition and trigger | Fix | Regression test (repro?) |
|---|---|---|---|---|---|
| F-1 | High | §1.1 | **Failed shutdown wedges the app.** A successful `quit::prepare` holds its attempt through shutdown, but the shutdown-error branch only reset `exiting`. Afterwards every Quit failed with "already pending", windows were refused, and `stopping` refused every command. Trigger: any retirement error, e.g. a download checkpoint write failing. | Reset `exiting` first, then `quit::abandon` releases the attempt and notifies the shells; the next Quit retries the repeatable shutdown. | `quit::tests::a_failed_final_retirement_releases_the_quit_attempt_for_a_retry` (exercises the extracted attempt state machine; the `lib.rs` wiring has no harness) |
| F-2 | High | §1.5 | **Self-update restart skipped Quit.** `install` called `app.restart()`. Tauri raises `ExitRequested` with `RESTART_EXIT_CODE`, for which `prevent_exit` is ignored (`tauri-2.12.5/src/app.rs`, `ExitRequestApi::prevent_exit`). The process therefore exited and relaunched while the spawned Quit flush and `Service::shutdown` were still running: unsaved sessions, transfers not checkpointed. (The first pass of this audit misread this as a deadlock and switched to `request_restart()`, which has the same flaw. The independent review caught it.) | `install` arms `RESTART_AFTER_QUIT` and requests an ordinary exit; Quit relaunches only after retirement succeeds, and clears the flag on cancel or failure. | `update::tests::an_installed_update_relaunches_only_through_a_completed_quit` (flag protocol; the Tauri event-loop wiring has no harness) |
| F-3 | High | §3.1 | **Errored seed or share can never be removed.** `apply_worker_error` keeps `seeding=true`, and remove/forget/delete required `!seeding`. Trigger: a shared file changed or was deleted, or a seed hit an engine error. Observed in this environment's baseline run: `status:"error", seeding:true`. | `removable_terminal` treats errored rows as removable without mutating them, so a refused removal still lets Retry restart the seed. | `torrent::tests::an_errored_seed_can_still_be_removed_or_forgotten` (repro; it also checks that a refused removal keeps the intent) |
| F-4 | High | §4.3 | **Refreshing a connected Bluetooth sender destroyed its live session.** Every browse called `nearby_connect`, which cancels the descriptor. That revoked the running downloads' grants, left the sender's Wi-Fi and, on the reverse path, ended the sender's Share all. | Browse reuses the live descriptor when it resolves and still answers; it reconnects only on failure. | None. It needs the native BLE transport; compile-checked only. |
| F-5 | Medium | §5.3 | **Watcher shutdown aborted an in-flight import.** `task.abort()` dropped the privacy lease while the `spawn_blocking` import kept running and could register a book under the shutdown write lease. | `consume` drains the current observation (bounded at 5 s); after that, `import_monitored_paths` refuses to register while stopping. | `watch::tests::shutdown_drains_the_observation_being_applied_instead_of_aborting_it` (pins the new helper; the old code had no seam) |
| F-6 | Medium | §1.2 | **A rejected mode switch still tore down work.** LocalSwarm/Nearby/private-sync/PDF/integrity retirement ran before validation. A stale `privacyGeneration`, the read-only rule, invalid settings or the notification rule then refused the save, with the work already cancelled. | `validate_settings_patch` runs before retirement and again under the writer. | `service::tests::a_rejected_mode_switch_does_not_retire_local_work_first` (repro) |
| F-7 | Medium | §2.1 | **Sweep loop in private/read-only mode.** The 5 s recovery sweep claimed durable rows (save), the finalizer refused (save) and settlement saved again, every tick, flickering the row and breaking I8. Trigger: switching mode while a large file is being hashed. | The sweep skips everything while `finalize_forbidden`. | `acquisition::tests::the_recovery_sweep_leaves_durable_rows_alone_while_finalizing_is_forbidden` (repro) |
| F-8 | Medium | §2.1 | **A persisted Cancel/Pause was overwritten by `verifying`.** The checkpoint after the last byte was unconditional. A crash in that window restarted with a durable `verifying` row, and startup finalize published an archive the user had cancelled (I3). | Decide under the state lock; Pause/Cancel goes to `stop_file`. | `acquisition::tests::a_cancel_persisted_at_the_end_of_transfer_is_never_overwritten_by_verifying` (repro, deterministic gates) |
| F-9 | Medium | §6.2 | **Duplicate tab IDs.** `undoClose` read a stale `closed`, and `appendTab` did not dedupe. Two reopen shortcuts handled before a re-render (key repeat, or the mobile async listener) opened the same tab twice and discarded an unrelated closed tab. | `insertTab` refuses duplicates; `removeReopened` uses a functional update; an already-open top entry is dropped and selected. | `scripts/test-tab-stack.mjs` (pure helpers; the App wiring is TypeScript-checked) |
| F-10 | Medium | §6.1, §6.3 | **Shell stuck after a failed boundary snapshot.** `privacyPending` was cleared only on success. Reader `authorityRefresh` latched its generation before `refresh()` resolved `false`. Tabs and links stayed inert until an unrelated refresh. | Bounded backoff retry; the latch resets on `false` or throw. | TypeScript-checked; no unit seam (React component) |
| F-11 | Medium | §1.7 | **Commands silently dropped on mobile.** The visibility fallback (1.5 s) spent the single retry while the native shell was still `inactive` (Android Activity obscured, WebView visible). The second suspension dropped bookmark, session and snapshot writes. | Wakes carry their reason; a fallback re-suspension keeps waiting, for at most `MAX_FALLBACK_RETRIES` attempts, then drops. | `scripts/test-suspended-retry.mjs` (3 new cases) |
| F-13 | Medium | §3.1 | **Restarting a stopped share beside a live share of the same file turned the old row into a permanent error.** Share metadata is deterministic, so librqbit refuses `AlreadyManaged`. | `refuse_duplicate_share` on `resume` and `seeding(true)`. | `torrent::tests::a_stopped_share_cannot_restart_beside_a_live_share_of_the_same_archive` (repro) |
| F-14 | Medium | §3.4 | **Unjournaled staging entry blocks deletion forever.** A file or directory was created, then the journal write failed (e.g. a full disk). The next attempt adopted it without a ledger entry, and deletion refused it as "unexpected temporary file". | Remove the just-created empty entry when journaling fails. | `torrent::tests::an_unjournaled_new_staging_entry_is_removed_so_cleanup_stays_possible` (repro) |
| F-15 | Medium | §5.1 | **Failed map `start` dropped an unresolved recovery record from memory.** The monitored-path claim and warning vanished, and the next persist made the loss durable. | Restore `owned_partial` on persist failure. | `maps::tests::a_failed_new_job_checkpoint_keeps_the_previous_unresolved_record` (repro) |
| F-16 | Medium | §5.1 | **Map `cancel` tail revoked a newer worker.** The tail ran after awaiting retirement without re-checking `active`. A resume admitted meanwhile lost its authorization, and its live partial could be reclaimed. | Skip the tail if a worker is active or the job was replaced. | `maps::tests::cancel_never_revokes_a_worker_admitted_while_the_old_one_retired` (repro) |
| F-17 | Low | §4.3 | **Any `lost` advertisement aborted an in-progress connect**, including a third device leaving range. | Only the selected sender's loss fails the exchange. | None (native transport); compile-checked |
| F-18 | Low | §5.2 | **A refused delete/forget left a permanent import tombstone.** A job resumed from another window could never be imported before restart. | `unretire_acquisition` on refusal, only by the caller that inserted the tombstone, so a concurrent second delete can't lift the first one's. | `imports::tests::a_refused_deletion_releases_its_acquisition_tombstone` |

### Refuted

- **Resume/Retry of HTTP downloads in private mode.** Reported as reachable, but `dispatch_inner` applies `normal_writable()` to `start_download`, `pause_download`, `resume_download` and `cancel_download` before routing (`service/dispatch.rs`). The UI button stays enabled and the click shows "This operation is disabled in private mode".

### Follow-up findings from November 7

These rows preserve the original findings and proposed fixes. The November 17 [review record](reviews/2026-11-17/PROJECT-REVIEW.md) records their dispositions, regression coverage and platform limits.

| ID | Sev | Machine | Finding | Suggested fix |
|---|---|---|---|---|
| O-1 | Low | §1.2 | `save_settings` bumps the epochs, retires readers and clears launches *before* `store.update`. Any later `?` returns with the mode unchanged and no `kiwix-privacy-changed` event. This fails closed, but leaves windows on a stale generation and loses queued launches. | Bump after a successful commit, or emit the event on every path that bumped. |
| O-2 | Low | §2.1 | `resume_interrupted` writes "still stopping" plus `interrupted=false` in a separate lock section from `queue_resume`. If the worker settles in between, an interrupted job becomes a permanent manual pause. | Re-check `state.running` inside the update closure. |
| O-3 | Low | §2.1 | A panicking worker leaves `running[id]` with command Run. Resume is a no-op, stop reports "exited without reporting", and remove is refused. Only the sweep checks `has_changed().is_err()`. | Apply the same "worker gone" test in `queue_resume` and `stop`, or add a drop guard. |
| O-4 | Low | §3.1 | Publication recovery returns `completed` with `seeding=true` and no worker, so the UI shows "Stop seeding" for a seed that isn't running. | Fall through to the engine, or mark it `interrupted`. |
| O-5 | Low–Med | §4.5 | Pairing enrolls the peer durably before the Ready exchange. If the other side fails after receiving Confirm, this side is left enrolled while the UI reports an error, and an earlier `automatic=true` keeps retrying a sync every 15 s. | Remove or restore the entry on any error after the enroll. |
| O-6 | Medium | §4.3 (iOS native) | `NearbyBluetooth.swift` `seenSources` is never cleared on `lost`, so a briefly failed sender stays hidden until Nearby restarts. | Remove the identifier when emitting `lost`. |
| O-7 | Speculative | §5.4 | Mobile root recovery decides rollback only from the configured root and ignores the journal phase. That could roll back a committed move whose books already point at the new root. | Decide like `selection_state`; refuse `downloadDirectory` changes while a journal exists. |
| O-8 | Low | §5.1 | A pause racing a real failure maps the error to `paused` with no error text and `resumable=false`, so the job can neither resume nor be removed. | Map only cancellation to paused, or keep the error when not resumable. |
| O-9 | Low | §5.5 | A stale 304 can re-save an older catalog record over a newer 200. It heals itself on the next refresh. | Skip the re-save when the stored digest differs. |
| O-11 | Low | §5.1 | A resume admitted **and finished** (paused or failed, same job ID) while `cancel` awaited the old worker still has that job cancelled by the tail. The window is narrow (found in review of F-16). | Compare a per-admission generation instead of the job ID. |
| O-12 | Low | §1.1 | After `quit::abandon` the service stays Stopping (watcher, imports and integrity already torn down), so the resumed shells can only Quit again. This is by design for a repeatable shutdown, but the UI does not say so. | Show a "Quit could not finish" state that offers only Retry Quit. |
| O-10 | Low | §2.2 | `complete` + `error` (the final history save failed) is shown as Failed and the book can't be opened until restart. | Rank `complete` above `error` in `downloadStatusDecision`. |

### Independent review

An independent reviewer read the first-pass diff (commit `7c7f6c3`) adversarially, as `AGENTS.md` requires. It confirmed F-4, F-7, F-8, F-13, F-14, F-15 and F-17 as correct. It found four real problems, all fixed in the follow-up commit:
- F-2's diagnosis and fix were wrong (`prevent_exit` is ignored for restarts).
- F-11 polled forever after the guess cap.
- F-18 could lift another deletion's tombstone.
- F-3 mutated the seed intent before steps that could still refuse.

It also raised lower-severity items that were applied: the bounded watcher drain, a single error report per privacy-retry series, `exiting` reset before `abandon`, an already-open undo entry, and checking `ensure_running` before settings validation. Residuals are listed as O-11 and O-12.

### Verification notes

- **Environment.** The native SDK was built locally with `scripts/build-desktop-sdk.py`. The libzim source was fetched by `git clone` because the codeload tarball was blocked; the archive hash matched the pinned SHA-256.
- **Baseline.** `cargo test --lib` (1019 tests) had 4 pre-existing failures. All four are torrent tests that need real listener sockets (`error starting listeners`): `cancel_and_suspend_do_not_wait_for_blocked_torrent_threads`, `pause_does_not_wait_for_a_blocked_worker_unpause`, `pause_does_not_wait_for_blocked_torrent_threads_and_reaps_the_abandoned_add`, `shared_library_archive_downloads_from_magnet_in_second_profile`. They are environmental and unrelated to these changes.
- **Not exercised.** Native BLE/Nearby, iOS/Android lifecycle, the Tauri updater restart and macOS/Windows builds. Per `AGENTS.md`, these changes still need independent review and platform runtime evidence before release.
