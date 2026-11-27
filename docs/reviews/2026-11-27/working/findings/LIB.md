## LIB review: library intake, move, delete, edition replace, integrity, export

Ten findings, ranked. Nothing was reproduced by running the app (cargo is off-limits); every finding is traced through the current code at `d98cac2`. Where a finding rests on OS behaviour I could not exercise here, it says so.

### LIB-1 — Exclusive rename has no fallback: downloads, deletes and library moves fail on NFS and FUSE volumes
Severity: High        Category: reliability
Platforms: Linux (NFS, ntfs-3g, exfat-fuse, sshfs, eCryptfs, 9p); probably Android removable SD (inference)
Evidence class: source-traced in the app; kernel behaviour is inference (not run here: no NFS/FUSE mount in the sandbox; ext4/tmpfs accept the flag)
Location: `src-tauri/src/publication.rs:563-588` (`rename_noreplace_at`, `platform_publish_in`), versus `publication.rs:669-695` (`platform_publish`, which does fall back). Consumers: `acquisition/finalize.rs:199`, `library_files.rs:2392-2406` (`move_exclusive`), `library_root.rs:974-992` (`publish_exclusive`) and `:1540`, `:1569`.
Trigger: the download folder or library is on an NFS-mounted NAS, or an NTFS/exFAT drive mounted through FUSE. Download a book, delete a book, or move the library.
Causal path:
- `rename_noreplace_at` calls `renameat_with(..., RenameFlags::NOREPLACE)` and its doc says "Return the original OS error if the filesystem lacks support". Linux NFS (`nfs_rename`: `if (flags) return -EINVAL`) and any FUSE server without RENAME2 (`fuse_rename2`) return EINVAL.
- HTTP finalize: `publish_verified_noreplace_bound` → `platform_publish_in` → EINVAL. The whole file has been downloaded and hashed, then fails with "Publishing verified archive without overwrite: Invalid argument (os error 22)". A retry re-hashes and fails the same way.
- Delete: `delete_book_impl` line 820 `move_exclusive(&source, name, staging, name)` → EINVAL, so neither trash nor permanent delete can work on such a volume.
- Move: `finish_move_with_rename` lines 1238 and 1361 fail after the full copy; the journal then replays at every launch (see LIB-5).
- Android: `publish_exclusive` and `remove_recorded_part` use the same flag on the external app directory. If the SD card's FUSE layer rejects it, the switch fails after copying everything, `rollback` fails too (it needs the same rename), and `has_pending_operation` then blocks every later storage change.
- Only the path-based `platform_publish` (used by exports) has the `ENOSYS | EINVAL | EOPNOTSUPP` → `hard_link` + unlink fallback. The two copies of this logic have diverged.
Smallest correct fix: put one fallback inside `publication::rename_noreplace_at` for Linux/Android, taken only on `EINVAL`/`ENOSYS`/`EOPNOTSUPP`:
- `linkat(source_dir, source, destination_dir, destination)`, which is exclusive.
- Compare (dev, ino) of both names, then `unlinkat` the source only if it is still that inode. This answers the concern in the comment at `publication.rs:352`.
- If hard links are also unsupported, create an `O_EXCL` placeholder at the destination, verify it is still ours, then plain `renameat` over it.
- Route `library_root::publish_exclusive` through the same function, and make `platform_publish` call it so there is one implementation.
Regression test: `publication.rs` tests. Inject the rename primitive (as `finish_move_with_rename` already does) to return `EINVAL`; assert the file is published, an existing destination is preserved with `AlreadyExists`, and a source swapped after the link is not unlinked.

### LIB-2 — Android device scan fails entirely when any ZIM sits at a volume root
Severity: High        Category: reliability / usability
Platforms: Android
Evidence class: source-traced on both sides of the contract
Location: `src-tauri/src/device_scan.rs:92-101` (`clean`), `:181`, `:155-158`; `plugins/tauri-plugin-kiwix-mobile/android/.../DeviceZimScan.kt:176-177`, `:237-239`, `:478-489`, `:296-299`
Trigger: a `.zim` at the top level of internal shared storage or of an SD card (the usual place people copy one to), then Library → scan device. Second trigger: a granted SAF folder containing a dot-named `*.zim`, e.g. a `.trashed-…zim`.
Causal path:
- Kotlin emits `location = ""` for root-level files: `RELATIVE_PATH` is `"/"` and gets `.trimEnd('/')`; `parent.relativeTo(root).path` is `""`; `documentId.substringBeforeLast('/', "")` is `""`.
- Rust: `let location = clean(&candidate.location, MAX_LOCATION, "location")?;` and `clean` returns `Err("Invalid location in device scan")` when `value.is_empty()`. The `?` aborts `scan()`, so the user gets that error and no results at all.
- The UI already expects an empty location (`Library.tsx:1091`, `{entry.location && …}`).
- `walk()` for SAF trees checks only `archiveName.matches(name)`, while the media and storage walks skip `name.startsWith('.')`. Rust line 156 turns a dot-name into `Err("Invalid archive name in device scan")`, again failing the whole scan.
Smallest correct fix: in `device_scan::scan`, accept an empty location (trim, bound, reject control characters, allow empty; `identity()` already copes). Turn per-candidate validation failures into "skip this candidate and add one limitation line" instead of `return Err`. In `DeviceZimScan.walk`, skip dot-names like the other two sources.
Regression test: `device_scan.rs` tests. A candidate with `"location": ""` yields one entry; a batch with one dot-named candidate still returns the valid ones plus a limitation.

### LIB-3 — Opening an archive that is already in the library re-runs a full integrity pass with no feedback
Severity: Medium        Category: usability / reliability
Platforms: macOS, Windows, Linux (also mobile "scan folder")
Evidence class: source-traced
Location: `src-tauri/src/service.rs:1694-1721` (`import_paths`, line 1707); `src-tauri/src/lib.rs:616-651` (`open_paths`); `service/dispatch.rs:452-473`
Trigger: double-click a 50–100 GB ZIM that is already in the library, run `kiwix file.zim`, drop it on the window, or use "Scan folder" on a folder whose archives are already imported.
Causal path:
- `open_paths` in writable mode always dispatches `import_archives`.
- `import_paths` calls `load_book(&path, managed, true)` → `Archive::check_integrity()` → `retained_checksum_cancellable` (MD5 over every byte) plus the structure check, before `register_books` discovers the path is already registered.
- There is no cancel token and no progress. For a running instance, `window.show()` and `set_focus()` at `lib.rs:680-682` run only after the import returns, so nothing visibly happens for minutes.
- The command holds the `privacy_gate` read lease throughout; a non-presentation settings save queues a writer and stalls every other command behind it.
- The read-only branch (`lib.rs:617-642`) and the folder watcher (`service.rs:1904`, `has_book_at_path`) already have a path fast path; the writable import does not.
Smallest correct fix: at the top of the per-path closure in `import_paths`, canonicalize the path. If `store.has_book_at_path(&canonical)` and the book is available, return that book without `load_book(..., true)`. Reader open still checks the UUID in `archive_for_book`.
Regression test: `service_tests.rs`. Import a fixture, then call `import_paths` again with the same path and a verification hook or counter; assert it returns the same installation with no integrity pass.

### LIB-4 — "Open with", CLI and second-instance failures are written to stderr only
Severity: Medium        Category: usability
Platforms: macOS, Windows, Linux
Evidence class: source-traced
Location: `src-tauri/src/lib.rs:668-690` (`open_paths`); callers at `:396`, `:1207`, `:1358`
Trigger: double-click a ZIM that is corrupt or unreadable, or a second copy of a book already in the library at another path.
Causal path:
- `import_paths` returns `{"books":[],"errors":[…]}`. For the duplicate case `register_books` (`service.rs:2119-2123`) returns "Another copy of this archive is already in your library…".
- `open_paths`: `if locations.is_empty() { … eprintln!("Archive open errors: {}", …); return Ok(()); }`. No window is shown or focused and nothing reaches the shell.
- With partial success the errors are also only `eprintln!`'d (line 688).
- The startup caller records a warning only when `open_paths` returns `Err`, which per-file errors never produce.
Smallest correct fix: in `open_paths`, for each entry in `imported["errors"]` call `service.note_startup_warning(format!("{file name}: {error}"))`. Then emit `kiwix-open` and show/focus the target window even when `locations` is empty; the shell already refreshes on that event and renders `startupWarnings`. For the duplicate case, opening the existing installation (as mobile's `admit_incoming_books` does) would be better still.
Regression test: `lib.rs` tests beside the existing `open_paths` ones. Call it with a corrupt copy; assert the snapshot's `startupWarnings` contains the file's error.

### LIB-5 — Desktop "move previous downloads": no space check, and a move that cannot finish is retried at every launch forever
Severity: Medium        Category: reliability / usability
Platforms: macOS, Windows, Linux
Evidence class: source-traced
Location: `src-tauri/src/library_files.rs:1624-1729` (`move_archives`), `:1296-1353` (cross-volume copy), `:1733-1807` (`recover`); `service/dispatch.rs:1797-1816`; `src/components/SettingsAutosave.tsx:314-363`
Trigger: change the download folder to a drive with less free space than the library and choose "Move previous downloads"; or unplug the target mid-move; or later delete the target folder or remove the book.
Causal path:
- Per book, `move_archives` runs `verified()` (full MD5) and `digest()` (full SHA-256), then `journals.write`, then `finish_move`. It never calls `volume_space`; only `check_file_size` (the FAT limit) is checked.
- `copy_hashed` fails with ENOSPC. The partial file stays in the hidden `.kiwix-move-<id>/` on the target and the journal is kept. The loop continues to the next book, repeating both full reads before failing again.
- At every launch `maintenance()` dispatches `recover_migrations`. For each journal, `finish_move(…, &mut Vec::new())` re-hashes the whole source (`check_part_proven` has no proof), deletes the partial, and re-copies until the disk is full again.
- `journals.remove` is reached only on success. A journal that can never succeed is retried and warned about for ever. Examples: "Journal is not bound to a managed library archive" after the book was removed; "Migration directory identity changed" after the target folder was recreated.
- The warning (`dispatch.rs:1806-1812`) keeps only the `error` string. The title was already set to the journal's `<uuid>.json` name (`library_files.rs:1803`). The user sees "Some saved items need attention. No space left on device (os error 28)" with no archive, no location and no way out.
- The modal shows "Moving previous downloads…" with no progress and no cancel, for hours.
Smallest correct fix:
- In `move_archives`, sum part sizes for books whose source device differs from the target and refuse up front when `storage::volume_space(&destination)` is smaller. `library_root::ensure_space` is the existing pattern.
- When a cross-volume attempt fails before any part is published (store still points at the source, every `part.target` unpublished), remove the owned staging entries by recorded identity, remove the staging directory and the journal, and return the error. Keep journals only once a destination part or the store commit exists.
- In `recover`, drop a journal whose `validate_journal` fails because the book is gone or belongs to neither path.
- Put the archive title and the from/to directories into the recovery warning.
Regression test: `library_files_tests.rs`. Use the existing rename injection plus a failing writer to assert a failed cross-volume first copy leaves no journal and no staging file; and that `recover` removes the journal of a book no longer in the store.

### LIB-6 — After an edition update the superseded archive can no longer be deleted in the app or re-imported
Severity: Medium        Category: usability / reliability (storage)
Platforms: all; worst on Android, where app storage cannot be browsed
Evidence class: source-traced
Location: `src-tauri/src/edition_updates.rs:627-664` (`replace_edition`); `src-tauri/src/service.rs:2105-2114` (`register_books`); `src/components/Downloads.tsx:1489-1509`; `src-tauri/src/store.rs:2573-2578`
Trigger: update one book, or "update all", to newer editions; then try to free the space the old editions occupy.
Causal path:
- `replace_edition` swaps `books[index]` for the new edition and pushes an `EditionTransaction`. The old file stays on disk and is no longer a library row.
- Nothing ever removes a transaction or its file: `edition_transactions` is only pushed to and read.
- The "Retained original editions" panel shows title and dates with one action, "Roll back to original edition". It offers no delete, reveal or path.
- Re-adding the old file to delete it normally is refused by `register_books`: "This file belongs to an archive edition decision…".
- Each update of a 100 GB archive therefore consumes another 100 GB permanently. The only in-app route is the old book's download record, if one still exists. Originals that were imported, moved, or copied in on mobile have none.
- The original's path stays reserved against acquisitions and its provider grant stays retained for ever.
Smallest correct fix: add an `edition_discard_original` command under the same guards as `edition_rollback`. It re-retains `transaction.original`, checks `original_identity`, calls `library_files::delete_book(&transaction.original, trash)`, and marks the transaction closed with a new `#[serde(default)]` field that the three reservation sites treat like a completed rollback. Add a "Delete retained original" button with a confirmation naming the file. This touches the persisted `EditionTransaction` schema, additively.
Regression test: extend the cutover/rollback test in `edition_updates.rs`. After commit and discard, the original file is gone, rollback is refused with a clear message, and the path is importable and publishable again.

### LIB-7 — A monitored folder that is an empty mount point removes every book in it from the library
Severity: Medium        Category: reliability
Platforms: Linux (fstab, NFS or SMB mount points); macOS when a stale `/Volumes/X` directory remains
Evidence class: source-traced
Location: `src-tauri/src/service.rs:1968-2051` (`reconcile_watched_directory`), `:2056-2101`, `:3834-3858`
Trigger: the monitored folder is a mount point such as `/mnt/nas/zim`. The NAS is offline at launch, or the share drops, so the directory exists but is empty.
Causal path:
- The guard covers only a directory that vanished: `directory.canonicalize()` or `read_dir` failing gives `None`. An unmounted mount point lists successfully as an empty set.
- `known` is every non-provider book whose path is a direct child of that folder, including managed downloads.
- For each one not open in a tab, `retire_missing_watched_book` runs `state.books.retain(…)`, `state.reader_storage.remove(…)` and unpins it.
- On remount, `import_monitored_paths` re-admits each file through `load_book(&path, false, true)`: a full integrity pass per archive, a new installation id, `managed` reset to false, and the pins, website storage and edition bindings gone.
Smallest correct fix: in `reconcile_watched_directory`, do not retire when the listing is completely empty while `known` is non-empty, and have `scan_watched_directory` report whether the directory had any entry at all. Leave those books in place; the existing `unavailable` probe already represents a missing file.
Regression test: `service_tests.rs`. A monitored temporary directory with one imported fixture; empty it and reconcile; assert the book is still in `store.books()`. With a second unrelated file left present, a deleted archive is still retired.

### LIB-8 — Mobile "Import a managed copy" of a book whose existing entry is unavailable deletes the fresh copy; the batch is also admitted unverified
Severity: Medium        Category: reliability / usability
Platforms: Android, iOS
Evidence class: source-traced
Location: `src-tauri/src/service.rs:2179-2219` (`admit_incoming_books`); `src-tauri/src/mobile_service.rs:1094-1121` (lines 1101, 1113-1117); `src-tauri/src/library_files.rs:583-655`
Trigger: the library copy of a book is unavailable (SD card removed, provider grant revoked). The user shares or opens the same ZIM into Kiwix to get it back.
Causal path:
- The native side copies the whole file into `mobile-imports-v1/<uuid>/`.
- `admit_incoming_books` matches by UUID only. `Some(existing)` with a different path pushes the new path to `duplicates` and resolves to `existing`, without looking at `existing.available`.
- `mobile_import_batch` then calls `remove_duplicate_intake_copy(&path, uuid)`, which deletes the copy just made. The shell opens the existing entry, which still fails as unavailable. The import "succeeded" and the user has nothing.
- Separately, line 1101 is `load_book(&path, true, false)`. This is the only admission path that skips the integrity check, despite the rule stated at `service.rs:1704-1706`.
Smallest correct fix: in `admit_incoming_books`, treat `!existing.available` (or a failed `archive_source::retain(existing)`) as a relink. Replace the entry's path, source reference and installation id under the same guards the `relink_book` handler uses, and do not list the path as a duplicate. Change line 1101 to verify, ideally through the cancellable `ImportManager` for progress.
Regression test: `service_tests.rs`. Mark an installed fixture unavailable, admit a second copy at a new path; assert the copy still exists on disk and the book's path now points at it.

### LIB-9 — Linux "Move to Trash" hides the trashed folder and can silently copy the whole archive into the home trash
Severity: Low        Category: usability / reliability
Platforms: Linux
Evidence class: source-traced (app and vendored `trash` 5.3.0); the file manager hiding dot-names in Trash is inference
Location: `src-tauri/src/library_files.rs:804`, `:874-882`; `trash-5.3.0/src/freedesktop.rs:58-66`, `:592-623`
Trigger: trash a book with "Move deleted archive files to Trash" on. For the copy: the archive is on a second filesystem whose mount root the user cannot write (a root-owned `/mnt/data` with a user subfolder, NFS/SMB).
Causal path:
- `let prefix = if cfg!(any(target_os = "macos", windows)) && trash { "kiwix-deleted" } else { ".kiwix-delete" };`. The comment on that line explains why the folder must be visible and recognisable in the Trash, but Linux is left out, so the trashed item is the dot-folder `.kiwix-delete-<uuid>`.
- `trash::delete`: when the per-volume trash fails with `PermissionDenied`, it falls back to the home trash. `move_items_no_replace` then sees `CrossesDevices` and runs `copy_dir_all` followed by `remove_dir_all`: tens of GB copied into `~/.local/share/Trash`, slowly, possibly filling the home filesystem.
- On any failure the message always ends "Any retained archive parts are recoverable at <staging path>", even when everything was restored and that directory was removed.
Smallest correct fix: use the visible `kiwix-deleted` prefix on Linux as well when trashing. Before `trash::delete` on Linux, refuse when neither `$topdir/.Trash-$uid` (or `.Trash/$uid`) is creatable nor the home trash is on the source's device, with a message such as "This drive has no Trash; turn off 'Move deleted archive files to Trash' to delete permanently". Append the "recoverable at" sentence only when the staging directory still exists.
Regression test: `library_files_tests.rs`. Unit-test the prefix choice and the new Linux preflight helper; assert the error for a fully restored failure does not mention a recovery path.

### LIB-10 — Archives with a platform-valid but "unsafe" file name cannot be deleted, revealed or moved, and the error does not say why
Severity: Low        Category: usability
Platforms: macOS, Linux
Evidence class: source-traced
Location: `src-tauri/src/library_files.rs:172-188` (`filename`), `:202-209` (`basename`), `:556-563` (`archive_files`), `:746-750` (`reveal_book`), `:787`, `:1678`
Trigger: a ZIM whose name contains `:` (a `/` typed in Finder is stored as `:`) or is longer than 240 UTF-8 bytes (about 80 CJK characters). Add it, then choose Delete, Show in folder, or move the library.
Causal path: import and reading go through `read_filename`, which accepts these names. Every mutation or reveal goes through `basename` → `filename`, which rejects `:`, more than 240 bytes, and trailing dot or space with "Invalid single-component archive filename". `Library.tsx remove()` shows that text. Reveal changes nothing on disk, yet is blocked too.
Smallest correct fix: have `basename` return a message that names the cause and the way out, e.g. "This archive's file name contains a character Kiwix will not modify (':' or control characters) or is longer than 240 bytes. Rename the file, use Locate, or remove it from the library instead". Let `reveal_book` use `read_filename` plus the read cohort, since it is nondestructive.
Regression test: `library_files_tests.rs`. A fixture named `a:b.zim` on Unix: `reveal_book` succeeds and `archive_files` returns the new message.

**Covered** (read end to end, non-test code)
- `library_files.rs` in full.
- `imports.rs`, the manager and worker.
- `integrity.rs` and `IntegrityPanel.tsx`. Nothing met the bar: cancellation and "file changed underneath" are handled, and the "unavailable" wording is deliberate.
- `library_root.rs` lines 1–1700.
- `edition_updates.rs` and `edition_service.rs`.
- `publication.rs` rename and publish paths.
- `device_scan.rs` and `provider_reference.rs`.
- `exports.rs`: `save_document`, `prepare_output`, `publish` and the resource path. No finding.
- Callers: `service.rs` import, watch and register paths; `dispatch.rs` remove, relink, move, cleanup; `lib.rs` `open_paths`; `mobile_service.rs` intake batch.
- Frontend: `Library.tsx` removal flow, `SettingsAutosave.tsx` migration dialog, the edition parts of `Downloads.tsx`, `SelectedStorageActions.tsx`, `edition-update-queue.ts`.
- `DeviceZimScan.kt` (consumer contract), the `trash` crate's Linux and Windows paths, and the two newest review records.

**Not covered**
- `mobile_rebase.rs`, `mobile_backup.rs`, `shared_library.rs`.
- `archive_source.rs` beyond its use sites.
- The MHTML and HTML-parsing internals of `exports.rs`.
- `library_files_tests.rs` and the in-file test modules, which I only skimmed for what is already proven.
- `OwnedDocuments.kt`, `ExternalArchiveMove.kt`, `ProviderMoveIdentity.kt`, `DocumentStorage.swift`.
- `IncomingArchiveDialog.tsx`, `DownloadCleanupDialog.tsx`, and the rest of `Library.tsx`.
- Windows-only branches were read but not exercised.