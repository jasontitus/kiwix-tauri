# Memory-safety checkpoint

The Kiwix app still has app-owned unsafe code and dependencies that handle untrusted input in native code. The acceptance target trusts the operating system and standard runtime. The [memory-safety inventory](../MEMORY-SAFETY-INVENTORY.md) tracks the remaining boundaries.

## Private revisions

| Work | Saved revision | State at this checkpoint |
|---|---|---|
| Kiwix app | [8ea272b](https://github.com/jasontitus/zimru-xapian/commit/8ea272b560df6b543aa90ea7806a439cb6260281) | Includes the reviewed iOS archive-storage change and the performance to-do. The app uses MIT reader `ff8b0ab` and GPL search `380d84d`. |
| Voice installation and cache | [aa2a7e8](https://github.com/jasontitus/zimru-xapian/commit/aa2a7e804f39c5921feb00af8460bf6d4acc0fcf) | Reviewed work-in-progress branch `codex/checked-voice-installation`. Uses a safe Rust bzip2 decoder, checks the distributor's archive and individual model files, and journals installation, repair and removal. Native speech inference remains in scope. |
| Android bundled voice and settings | [19d81ac](https://github.com/jasontitus/zimru-xapian/commit/19d81acfb41d7d16b82620f371eee126a1471000) | Reviewed work-in-progress branch `codex/checked-bundled-voice`. The Android producer and settings UI have host coverage. Verify them together with the final installer before merging either topic. |
| Optional MIT reader optimization | [1c0f5e3](https://github.com/jasontitus/zimru-xapian/commit/1c0f5e3a8d748bf7304588e7db73022555323561) | The reader retains decoded cluster storage when a caller opts in. Platform enablement remains on the app worklist. The GPL search revision is independent. |

## Resume here

At `aa2a7e8`, the installer keeps both selection records after an inconclusive read. It checks each required file by its recorded name and rechecks the selected directory and record before reclaiming the fallback. The focused host run passed 35 tests. An independent run repeated the three new cases for denied access, a missing required member and replacement of the selected directory or record.

1. Verify the optimized host test suite and both real voice-pack inputs on the final source. Combine the reviewed Android producer with that installer, verify the packaged asset, and exercise install, reopen, remove and reinstall. Record the tested commit and package digest.
2. Continue through the [app-owned and dependency worklist](../MEMORY-SAFETY-INVENTORY.md), including Nearby callbacks, platform file and bookmark handoffs, rendering and offline speech. The Mac account lookup still uses the existing narrow native call. Review found that the proposed typed replacement dereferences a potentially absent account field.
3. Revisit the optional [reader performance switch](../MEMORY-SAFETY-INVENTORY.md#performance-follow-up) separately. Native libzim was faster on the measured random-article workloads. The mobile comparisons used virtual devices. [Four-platform results](../benchmarks/2026-11-18-retained-clusters.md)
