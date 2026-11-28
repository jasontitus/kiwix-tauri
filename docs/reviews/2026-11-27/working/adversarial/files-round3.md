## Verdict for lane files (round 3): no findings, lane closed

FILES-R8 and FILES-R9 are both fixed and the requested tests pass. I checked HEAD 937ebe2. Since round 2 (681a9b8), only two lane files changed: the one hint string in `library_files.rs` and `folder_held_original` with its test in `edition_updates.rs`. Nothing changed in the working tree beyond that commit.

**FILES-R8: closed.** The hint (`library_files.rs:85`) now says the hidden name may be in "the archive's folder, or in the folder it was last moved from". That matches where the rename actually leaves it.

**FILES-R9: closed.**
- **Logic.** `folder_held_original` (`edition_updates.rs:582`) returns true only if the parent folder opens and either:
  - its device equals the device recorded for the original, or
  - it contains at least one entry.

  An empty leftover mount point on another device, a missing folder, and an unknown device all give "not available"; none of them close the record.
- **Windows.** `Dir::dir_metadata()` builds its metadata from the folder's open handle, so the volume serial is always filled in and `dev()` cannot hit its `expect`. That is the same source `ContentIdentity` uses for each part on Windows, so the two values are comparable. The volume serial stays the same across replugging. An absent drive letter fails at the open. An NTFS folder mount point whose volume is absent is empty and carries C:'s serial, so it is refused.
- **Reading the recorded device by serializing the identity.** `ContentIdentity` serializes transparently as an array of `{"device":…}` objects, and `as_u64` covers the full u64 range. An empty list, or any future change of format, gives `None`. `None` only removes the device test; the folder must then be non-empty. So it fails toward refusing, as you asked. Profiles written by older builds already have to match this format to load at all.

**What I ran:** `cargo test --locked --lib -- library_files:: publication:: edition` on 937ebe2 gave `ok. 101 passed; 0 failed` (52 + 24 + 25). I did not re-run clippy or the Android check this round.

**Residuals of the lane (none blocking):**
- **Handoff to the maps owner:** `maps/partials.rs` reclaim can leave a second name after an interrupted capture. A published partial is then refused for good, and an unpublished one leaks its bytes under the twin name. The exact fix is in my round-2 report.
- **Handoff to the LocalSwarm owner:** `localswarm/cleanup.rs` `erase` and the ownership check near line 392 fail for good when a crash leaves an empty cleanup directory. Fix: remove it if it is empty and owned by us, then capture again.
- **Handoff to the dispatch owner:**
  - After R5, a same-volume move that Windows startup recovery cannot finish because an archive opened at launch is cancelled rather than retried. The person sees "not moved and still in …" and has to move again.
  - The comment at `dispatch.rs:1701` is stale.
- **Handoff to the frontend:** the button for `edition_discard_original` is still missing. It must hide rollback when `originalDiscarded` is set, pass `privacyGeneration`, and show the "not available. Reconnect it" error.
- **Not handled by design:**
  - An interruption between the capture and the unlink still needs the person to delete the hidden `.kiwix-renamed-…` file, although the message now points at it.
  - Discard can still report `missing` if a leftover mount point is not empty.
  - A device number that changes between mounts, for an original that was alone in its folder, keeps the record open.
  - The R2 orphan rule is a stand-in: a library whose only book was removed mid-move keeps showing its warning.
- **Other lanes:** LIB-9's cross-device copy into the home Trash is still not prevented. LIB-1 on Android gives a clear refusal rather than a working move. The Kotlin dot-name skip in the device scan is a handoff to the Android lane.
- **Never exercised:** real NFS or FUSE, macOS and iOS (not compiled), the Windows runtime, the real Trash, and Android devices.