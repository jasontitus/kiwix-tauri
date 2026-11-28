## Verdict for lane files (round 2): only non-blocking findings

R1 to R7 are closed. I found two small new issues (R8 and R9), and confirmed both handoffs (maps and LocalSwarm) as real but rare defects.

I reviewed HEAD 681a9b8, diffed against `d98cac2`. The lane files had these sha1 values when the tests ran:
- `publication.rs` 81990ac…
- `library_files.rs` 317b1e7…
- `library_files_tests.rs` 50e4720…
- `edition_updates.rs` 414b2a5…
- `device_scan.rs` 32c48b7…

The lane's only change in `maps/partials.rs` is the comment at about line 707, as required. The `record_vanished`/`Kept` additions in that file belong to the maps lane. I also read `localswarm/cleanup.rs` (`erase` and the ownership check near line 392), the maps reclaim closure, and the Windows recovery gate in `service/dispatch.rs`.

### Item status
- **R1: closed.**
  - `source_intact` (`library_files.rs:1279`) requires the directory (dev, ino) to match `source_directory_identity`. It also requires every part's `FileStamp` to match: (dev, ino, size, mtime_ns), plus nlink == 1 through `stamp()`.
  - A folder recreated at the same path fails, because the directory is a new inode. The copies inside are new inodes too, which the new test covers. Getting past both checks would need a reused directory inode plus file inodes with matching size and nanosecond mtime, which does not happen in practice.
  - Checking stamps without hashing is the right strength here. The question is whether the source still exists, not whether its content is intact. A hash would bring back the full re-read at every abandon that LIB-5 removed.
  - It is checked first in `clear_unpublished_copy`, so both the abandon path and the orphan path need it. The "still in …" text now appears only after that proof.
  - The remaining race between this check and the deletion is milliseconds wide.
- **R2: closed.**
  - The orphan rule needs all of: no load warnings, at least one persistent book, none with this id, and `source_intact`. A fresh or quarantined profile therefore never orphans a journal, and a split archive is never dropped.
  - Cost of the "at least one book" proxy: someone who removed their only book mid-move gets one named warning at each launch. It is cheap, because `validate_journal` fails before any hashing. It is cosmetic.
  - The backup-restore doubt is cosmetic or recoverable, not a data-loss path, because the drop requires every part whole in the source folder. Worst cases:
    - The book later comes back pointing at the destination; Locate fixes it.
    - Published destination copies are left behind as duplicates.
- **R3: closed.** Both hooks are now `#[cfg(test)] thread_local!` cells, and every setter and reader runs on the test thread. 20 consecutive runs passed.
- **R4: closed.**
  - The `EEXIST` healing only applies when the destination is the source's own inode, so no data can be lost. The "consumes a user-made hard-link pair" difference from native `EEXIST` is acceptable:
    - It is reachable only on filesystems without `RENAME_NOREPLACE`.
    - Every caller either writes to a fresh or private name, or refuses before renaming when both names exist (`move_archives` pre-check, the "Both migration names exist" check, and `library_root`'s nlink == 1 check on the captured name).
  - The a→a case: the capture moves the only name aside, the second destination read finds nothing, the file is restored, and the call returns an error. The file always keeps a name; `fallback_never_removes_the_only_name_of_a_file` exercises this.
  - The deterministic capture name gets a `-1` suffix if it collides with either name being worked on. A leftover is unlinked only while it is the published inode. A different file under that name stops the move.
- **R5: acceptable and safe.**
  - The journal is removed only when all of these hold: `intact_source` (every part fully hashed in the source folder), the source path committed back, and every destination part name absent.
  - A partly published move fails `intact_source`, because some part is no longer at the source. A fallback link that left both names fails the "destination absent" check. Both keep the journal.
  - Changing the existing test was justified; it now asserts the new contract and still covers the collision half.
  - One residual: Windows startup recovery runs without closing readers (`dispatch.rs:1701`). A same-volume journal pinned by a launch-time open is now cancelled with "not moved and still in …" instead of retried later. It is consistent but needs the person to move again. The comment there is stale; that is outside this lane, so it is a handoff.
- **R6: mostly closed; see R9.** Requiring the parent directory catches a missing drive letter, a removed `/Volumes/X` or `/media/user/X`, and a mount point where the original sat in a subfolder. It does not catch an original placed directly at the root of a static mount point (fstab `/mnt/data`) while the volume is unmounted.
- **R7: closed.**
  - `archive_basename` now mentions sharing.
  - The no-hard-links refusal no longer blames the drive alone and holds for EPERM.
  - The device-scan line is neutral.
  - The foreign file at an unpublished part name no longer blocks.
  - `settled_delete_failure` is right: it says "left where it is" only when every part is back with its exact stamp. It gives the "N of M" wording only when the staging folder is gone or everything is back; otherwise it falls back to "recoverable at". Its test covers the wording only.

### FILES-R8 — The hidden-name hint points at the wrong folder after a library move
Severity: Nit
Fix under review: FILES-R4
Evidence class: source-traced
Location: `library_files.rs:~84` (`FileStamp::of`, the `HARD_LINKED` hint); `publication.rs:776` (capture renames within `source_dir`)
Trigger: a move from folder A to folder B on a filesystem without `RENAME_NOREPLACE` is interrupted after the capture. The person later deletes or moves the book from B.
Causal path: the leftover `.kiwix-renamed-<dev>-<ino>` stays in A, the source directory of the interrupted rename. The refusal says it "may be left in the archive's folder", which is B. For deletes the hint is right, because the staging folder is a child of the archive's folder.
Smallest correct fix: say "in the archive's folder, or in the folder it was last moved from".

### FILES-R9 — `missing` still closes the record for an original at the root of an unmounted static mount point
Severity: Nit (owner may accept)
Fix under review: FILES-R6
Evidence class: source-traced
Location: `edition_updates.rs:589-593` (`remove_original`)
Trigger: `/mnt/data/original.zim`, where `/mnt/data` is an fstab mount point that stays as an empty directory while the drive is unmounted.
Causal path: `symlink_metadata` returns NotFound, and the parent `/mnt/data` exists and is a directory, so the result is `Missing`. The record is closed and rollback is given up, while the file is still on the drive.
Smallest correct fix: return `Missing` only if the parent exists and either:
- its `st_dev` equals the device recorded for the first part in `transaction.original_identity` (a file's parent is always on the file's own filesystem); or
- the parent has at least one entry, which covers device numbers that change between mounts. An unmounted mount point is normally empty.

Otherwise return `ORIGINAL_UNAVAILABLE`.

### Handoffs (item 4)
- **`maps/partials.rs` reclaim (closure around lines 469-510).**
  - This is a real, reachable defect, but only for map storage on NFS/FUSE without `RENAME_NOREPLACE`, and only after an interruption inside the fallback's sub-millisecond link-to-unlink window. Severity: low, Should-fix.
  - When the staged entry already exists, the capture is skipped and the staged file can have nlink 2. Its twin is either the public name or `.kiwix-renamed-<dev:x>-<ino:x>` in the public directory.
  - A published partial is then refused for good ("Captured map partial changed identity or size"), reported as non-retryable `Kept::Unconfirmed`, so the person cannot delete it.
  - An unpublished partial passes, because nlink is checked only when `published`. The staging name is unlinked and the record removed, leaving the full partial's bytes under the twin name with no record: a silent space leak.
  - Exact fix: after the `if !map_entry_exists(&staging, …)` block, if the staged entry has nlink 2:
    1. If `directory.directory` holds `record.name` with the same (dev, ino), call `capture(&directory.directory, &record.name, &staging, &record.name)` again inside the same `store.with_acquisition_destinations` wrapper. The healed primitive finishes the move.
    2. Otherwise, if `.kiwix-renamed-{dev:x}-{ino:x}` in `directory.directory` is the same (dev, ino), `remove_file` it and sync.
    3. Re-stat, and require `nlink() == 1` whether or not the partial was published, before any unlink.
- **`localswarm/cleanup.rs::erase` (line 279) and the ownership check (lines 390-392).**
  - Also real and reachable, only for a LocalSwarm folder on NFS/FUSE with a crash between the fallback's `mkdirat` and `renameat`. Severity: low, Should-fix.
  - The leftover is an empty 0700 directory at `.kiwix-localswarm-<id>.cleanup` while the source still exists. Both sites then fail for good ("Both LocalSwarm cleanup and source names exist"). Nothing is lost, but cleanup and the removal authorization can never finish.
  - Exact fix, at both sites, when both names exist: open the cleanup-name entry with no-follow. If it is a directory whose identity is not `saved.owned.identity`, owned by the effective uid, and with no entries, run `parent.dir.remove_dir(&name)` (it refuses if anything appeared) and sync. Then continue as if not yet captured. Keep every other combination as the existing error.
  - Making the primitive adopt an empty directory would be wrong, as the implementer says.

**Implementer's round-2 doubts**
- R2 proxy: acceptable; the effect is cosmetic or recoverable.
- R4 consuming a hard-link pair: acceptable.
- R4 crash between capture and unlink: acceptable now that the name is recognisable, apart from the wrong-folder hint (R8).
- R5 transient-failure cancel: acceptable. The Windows startup-pin case and the stale comment are a handoff to the dispatch owner.

**What I ran**
- `cargo test --locked --lib -- library_files:: publication:: edition device_scan::` on the main tree: `ok. 110 passed; 0 failed` (52 + 24 + 24 + 10).
- I copied that test binary to scratch and ran the same filter 20 times: `pass=20`, so the R3 flake is gone.
- I did not re-run clippy, Windows-gnu or Android this round. The round-1 Android type-check of the lane's `cfg(mobile)` code was clean, apart from another lane's unused-import lint.

**Disclosure about round 1:** my scratch-copy build used the shared target directory. Cargo gave it the same artifact name (`deps/kiwix_app-9cc670a89c2ec02d`), so it briefly replaced the main tree's test binary. Cargo has since rebuilt it from the main tree; this round's binary contains the round-2 tests. The repository was never modified. Round-2 scratch files are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-files/r2/`.

**Not verified:** real NFS/FUSE, macOS, iOS, the Windows runtime, the real Trash, and Android hard-link refusal on a device.