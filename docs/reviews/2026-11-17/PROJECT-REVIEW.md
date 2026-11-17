# Project review, November 17, 2026

The review began with a clean working tree at `39102a92081d398d823b18df3d825f1734c1c583`. It followed the data paths from archive and document selection through native storage, reading, sharing, recovery and packaging. The review used local builds, temporary profiles, retained test archives, loopback services and disposable signing inputs.

## Findings and dispositions

| Impact | Trigger and affected contract | Change and evidence |
|---|---|---|
| High | Desktop connector and second-instance activation could confuse paths differing only by case or disturb another entry when restarting a local listener. A relay could wait indefinitely for a stalled peer. | `ipc.rs`, `instance/socket.rs` and `agent_connector.rs` check filesystem identity and peer ownership. The two listeners publish exclusively through a retained private directory. Startup refuses live peers, occupied paths and ambiguous failures. Retirement captures the entry before removal. The relay checks its greeting and applies bounded inactivity waits. A live macOS relay listed, searched and read a fixture archive, rejected calls after disabling, reconnected after repeated toggles and cleaned up its socket. |
| High | An export or library import started in one window or privacy generation could finish after that context changed. A reader Save dialog could outlive the archive installation shown in its frame. | Shell handoffs carry the initiating policy and reader installation. The importer distinguishes returning from the mobile picker from a change in privacy policy. Desktop imports cap reads from the open file. Browser imports check the immutable file size before reading its contents. Browser exercises covered delayed picker responses and export cancellation. Native guards cover the final application-controlled publication. |
| High | Corrupt or oversize transfer journals could cause unbounded reads, be reopened through a changed pathname, or clobber a backup. A selected path could change during export. | HTTP and torrent recovery open a regular file once and cap input. Corrupt history is moved by identity to a unique no-clobber name before replacement. Map, backup and notification reads also apply bounds. New document exports are staged exclusively. Existing-document writes preserve the selected file’s permissions and identity, with ordinary write-error recovery. |
| High | A release publisher could disclose its signing secret in a child command line, reuse stale signed bytes or leave the staging area half-updated after a collision. Windows packaging omitted runtime files used by the app. | Release preparation signs or verifies its own snapshots, streams prehashed payloads and protects input/output names. The manifest is published after payloads and signatures. The Windows job constructs and checks its runtime resource list, including speech DLLs and the relay. Local signing/verification used disposable keys. Windows installer launch still requires a trusted Windows runner. |
| Medium | A long reading history or search could mount thousands of interactive rows. Repeated catalog snapshots could reset settings work. | Collections and unified search mount up to 50 results per section, retain full-dataset search and provide keyboard paging. In Chrome a 10,000-item fixture stayed navigable after filtering and shrinking the last page. Settings comparisons use the catalog-source values. Connector archive listing avoids copying the full profile. |
| Medium | Independent build jobs missed frontend or relay changes. An explicit Cargo target could select a stale relay binary. The app directly used an unmaintained time library. | Build triggers cover the shipped sources. Relay build and dependency checks cover its separate manifest and lockfile. The build locates the explicit target output. App timestamp users moved to the existing maintained time library. A differential check of 5,600 timestamp inputs and a leap-second HTTP validator check covered compatibility. The torrent dependency still pulls in Chrono transitively. |

### Findings carried forward from the state-machine audit

| ID | Path or symbol | Disposition and verification |
|---|---|---|
| O-1 | `Service::operation`, settings dispatch | A failed settings write can still retire old origins. An epoch advance triggers a shell-refresh notification before the writer is released. Queued launches survive when effective policy stays the same. Tested with an injected persistence failure. |
| O-2, O-3 | `acquisition/manager.rs`, acquisition worker settlement | Automatic resume checks the current interrupted intent and worker. A panic enters ordinary settlement. Settlement and journal-error paths reject stale worker generations. Native regressions exercise interrupted and superseded workers. |
| O-4 | Torrent publication recovery | Recovery with a verified archive and no running engine reports completion with seeding off. Exercised with unavailable staging and peers. |
| O-5 | `private_sync_pair_exchange`, `PairSession::retire` | Ready is exchanged before either side attempts local enrollment. Capacity, expiry and policy are rechecked, with terminal state and cancellation serialized. A real encrypted loopback exchange covered disconnect before Ready and successful confirmation. Both devices still need a fresh authenticated session for reading data. Independent storage failure after both peers send Ready can require re-pairing. |
| O-6 | `NearbyBluetoothSession.failConnection` | Losing the selected sender releases its discovery slot. A new advertisement from the same sender becomes selectable. A Swift callback fixture covered loss, duplicate discovery and stale-manager delivery. |
| O-7 | `library_root::recover`, settings validation | Recovery checks journal phase, registered destinations and file identity before rollback. Changing back to the old preferred folder preserves original and adopted archives. Pending moves block a conflicting preference update. Cleanup preserves unrelated staging contents. Reserving and recording empty files before copying lets recovery identify partially written archives after low space. Older journals lacking that proof preserve unrecognized staging entries. An interruption while publishing a folder marker can leave a small temporary file. Tested both pre- and post-commit journal phases in disposable directories. |
| O-8, O-11 | `MapManager::finish`, `cancel`, `recover_terminal_publication_for` | A pause only becomes resumable on a cancellation with a checkpoint. Genuine errors stay visible. Admission generations protect new work even if it finishes while an older cancel is waiting. Exercised both races in the host manager. |
| O-9 | `CatalogCache::fetch` | A delayed 304 refreshes metadata only for the representation that is still stored. A loopback 304/200 race and offline read verified that newer XML survives. Existing revalidation still enforces private permissions and reports persistence trouble. |
| O-10 | HTTP finalization, `DownloadJob.historyWarning`, Downloads | A completion-history or directory-durability warning stays separate from archive availability. Opening the book still requires its imported receipt. A real publication followed by a forced final-write failure and manager restart recovered the archive and cleared the warning. |
| O-12 | `quit::prepare`, exit handler, updater | Retry retains already-flushed shells and the installed update’s requested restart. A cancellation before retirement releases the attempt. Host tests exercised failed shutdown and retry. Presentation of the actual OS failure dialog remains unverified. |

## Independent review

The impact column describes the consequence traced in the source. The verification statements identify the transitions that were also exercised. Release packaging on operating systems unavailable to the reviewer was checked against its build and consumer contracts.

An integration reviewer and separate native, frontend and shipping reviewers inspected each other’s changes. Findings returned to the author included socket probing through a movable directory, cache permission changes on 304, stale worker settlement, overwriting a terminal pairing state, update-relaunch loss on Retry, leap-second validator compatibility, release rollback ownership and stale reader export authority. Each revised contract was submitted for a fresh review.

## Evidence and limits

- The final macOS application suite passed 1,175 tests, with 20 ignored. All 374 JavaScript tests, 182 Python tests and 8 standalone connector tests passed. The production frontend, native host and release connector built. Host Clippy passed for the application and connector with warnings denied. Vite still reports that its event and window modules are also statically imported.
- Both Rust lockfiles passed advisory, license, ban and source-policy checks for their CI targets, seven application targets and four relay targets. The runtime-only npm audit reported zero vulnerabilities. These are local results against the checked-out dependency files.
- The Chrome reading check opened an actual archive in the headless service and reached an article without loading the native speech window module. A separate listener test covered unmount during the delayed import.
- macOS host builds used the project’s retained libzim SDK. The real headless connector exercised archive reads through the release-build relay executable. The notification bridge ran against a disposable journal and a test notification-center adapter. No OS notification was delivered.
- The Windows and Linux relay code and tests passed cross-target Rust type checks on the Mac. A network-isolated Alpine Linux VM ran the production socket helper, including 0600 permissions, client reads, live and stale sockets, and preservation of a replacement file. This test caught a Linux-only socket permission failure before final review. Chrome exercised the reader UI and mobile picker lifecycle with a native adapter fixture. Swift callback tests and an iOS simulator SDK type check covered the Bluetooth change. Physical Bluetooth discovery, platform document providers, a signed App Sandbox installation and installed Windows/Linux bundles still require platform verification.
- Release-signing and publication exercises used local files. With four 64 MiB prehashed artifacts, one process run per revision used roughly 296 MiB peak RSS before and 132 MiB after. These are memory observations from one Mac.
- The frontend build-tool chain still includes a high-severity picomatch advisory. The upstream advisory lists no patched release. The build takes glob patterns from project configuration. The npm dependency-policy check remains red. [Upstream advisory](https://github.com/advisories/GHSA-fc5q-39xc-vjpf)
- An existing text or binary document saved in place can be partially written if the process dies during the overwrite. Descriptor-bound writes preserve the approved object and its access rules. Ordinary reported I/O failures attempt restoration. Staged replacement of an existing resource or MHTML export also has a narrow POSIX race between the last destination-identity check and rename when another process edits the same name. Fresh destinations publish without replacement. Published releases, user-device installation, and process-kill testing on a real document provider were outside this run.

## Working-tree snapshot

The 82 paths below were compared to the base commit, including 4 untracked additions. All tracked edits are unstaged. The SHA-256 for the 81 reviewed files other than this report is `c0e08dae2f6ae4c7afcee311b6b5a449e2aae4668797599aa330431b9653fe12`. The input is each sorted repository-relative UTF-8 path, a zero byte, and the raw SHA-256 of that file’s contents.

```text
M .github/workflows/build.yml
M .github/workflows/checks.yml
M .github/workflows/dependencies.yml
M docs/DEPENDENCIES.md
M docs/PACKAGING.md
M docs/STATE_MACHINES.md
A docs/reviews/2026-11-17/PROJECT-REVIEW.md
M kiwix-mcp/Cargo.lock
M kiwix-mcp/Cargo.toml
M kiwix-mcp/src/ipc.rs
M scripts/build-mcp.mjs
M scripts/make-update-manifest.mjs
M scripts/test-collections-async.mjs
M scripts/test-download-state.mjs
M scripts/test-mobile-voice-search.mjs
M scripts/test-provider-recovery.mjs
M scripts/test-update-manifest.mjs
A scripts/windows-bundle-resources.mjs
M src-tauri/Cargo.lock
M src-tauri/Cargo.toml
M src-tauri/deny.toml
M src-tauri/native/notifications.mm
M src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/NearbyBluetooth.swift
M src-tauri/src/acquisition.rs
M src-tauri/src/acquisition/finalize.rs
M src-tauri/src/acquisition/http_meta.rs
M src-tauri/src/acquisition/manager.rs
M src-tauri/src/acquisition/persistence.rs
M src-tauri/src/acquisition/segments.rs
M src-tauri/src/acquisition/transfer.rs
M src-tauri/src/acquisition_tests.rs
M src-tauri/src/agent_connector.rs
M src-tauri/src/catalog_cache.rs
M src-tauri/src/catalog_cache_tests.rs
M src-tauri/src/catalog_index.rs
M src-tauri/src/crash.rs
M src-tauri/src/diagnostics.rs
M src-tauri/src/exports.rs
M src-tauri/src/instance.rs
A src-tauri/src/instance/socket.rs
M src-tauri/src/integrity.rs
M src-tauri/src/lib.rs
M src-tauri/src/library_files.rs
M src-tauri/src/library_root.rs
M src-tauri/src/live_activity.rs
M src-tauri/src/localswarm/mod.rs
M src-tauri/src/maps.rs
M src-tauri/src/maps/manager.rs
M src-tauri/src/maps_tests.rs
M src-tauri/src/migration.rs
M src-tauri/src/mobile_service.rs
M src-tauri/src/photo_capture.rs
M src-tauri/src/private_sync.rs
M src-tauri/src/private_sync_service.rs
M src-tauri/src/private_sync_service_tests.rs
M src-tauri/src/publication.rs
M src-tauri/src/quit.rs
M src-tauri/src/reading_data.rs
M src-tauri/src/service.rs
M src-tauri/src/service/dispatch.rs
M src-tauri/src/service_tests.rs
M src-tauri/src/sharing.rs
M src-tauri/src/store.rs
A src-tauri/src/timestamps.rs
M src-tauri/src/torrent.rs
M src-tauri/src/torrent/manager.rs
M src-tauri/src/torrent_tests.rs
M src-tauri/src/update.rs
M src-tauri/vendor/wry-0.57.1/build.rs
M src/api.ts
M src/components/AdvancedSettings.tsx
M src/components/Collections.tsx
M src/components/Downloads.tsx
M src/components/IntegrityPanel.tsx
M src/components/Library.tsx
M src/components/Reader.tsx
M src/components/Settings.tsx
M src/components/UI.tsx
M src/components/UnifiedSearch.tsx
M src/main.tsx
M src/native-speech.ts
M src/types.ts
```
