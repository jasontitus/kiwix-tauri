# Project review remediation — 2026-11-23

**Base:** `4459fb9fd91796891513cba2e6749d835882bd0f` (clean tree at review start)
**Input:** user-supplied `project-review.md` (untracked; not committed)
**Scope:** source remediation of the review's highest-impact UX, reliability, and performance findings, followed by independent adversarial review of each cohesive slice and a second pass closing review findings.

## First pass

- **Reader authority and serving:** exact live-origin navigation index independent of the async reader lease; malformed archive MIME headers fall back to `application/octet-stream` instead of panicking; archive metadata wording no longer trips the internal-term filter.
- **Settings and error boundaries:** presentation-only settings saves take a reader lease instead of the write-preferring privacy writer and merge only submitted keys into current settings; `LOW_DISK_SPACE` envelopes are shape-validated and preserved; crash URL redaction rejects nested URIs; stack budgets count frames.
- **Transport:** StreetZim read-idle deadlines, updater artifact read/connect bounds, authenticated segment/single-stream retry with `If-Range`, Private Sync listener survival and 4 KiB pre-enrollment framing, LAN PIN re-render without credentials, and bounded sharing sockets.
- **Acquisition and caches:** LocalSwarm resume rehashes only durable verified bits; Web Seed resume reports retained offset and incoming rate; edition readiness reuses the worker proof against live ownership; publication rejects a post-checksum identity change; prefix/map caches use real LRU promotion; map cache is 64 MiB with 16 MiB entries; catalog traversal publishes once; suggestions open only visited archives; fallback excerpts are read only for visible rows and excerpt failures keep their hits.
- **Mobile and native:** Android PDF descriptor ownership; retained iOS foreground speech restoration; Kokoro pause/seek/rewind and transport acknowledgements; session-bound widget/Live Activity controls; Android external speech stops on window retirement; backward Live Activity seeks; idle model release and background-work separation.
- **Frontend:** StrictMode dialog guard and focus cleanup; single error ownership; tag draft parsing; raw identifier/label cleanup; private-mode share/seed gating; map polling restart and dismissal; catalog page-link containment; watched-download pruning and hidden-pause; per-pane error boundary; dismissible locale failure with retry.

## Second pass (residual review findings)

- **Acquisition receipts/publication:** Unix ctime is recorded and degenerate/zero change times fail closed; Windows receipt reuse is disabled because the current dependency set exposes no change time. Acquisition finalization uses the bound pre-rename publication helper after the journal window, restores the advertised SHA-256 and ownership on a rejected publication, and rechecks edition originals at reserve and commit. Torrent output is re-hashed after copying and compared with the source digest; LocalSwarm seal-time change stamps are rechecked before publication.
- **Catalog cache:** oversized encoded records are skipped with a warning while the remaining accepted pages publish; UUID scratch names use `create_new`; legacy fixed scratch is reused only when private, owned, bounded and empty/prefix-compatible; foreign pending files are never truncated; stale UUID scratch is prefix-checked before unlink.
- **Sharing/updater:** explicit HTTP/1 request framing (`Head`/`Body`/`Framed`/`Unaccounted`) with absolute head/body budgets and a separate response idle bound; handler time is not charged to the first response; 16 KiB storage cap with rolling terminator scan; final updater gate rechecks retirement after the blocker and before install; artifact download is dropped on service retirement.
- **Archive serving:** one retained read-lock acquisition resolves the entry, size and first bytes; compressed clusters decompress once per response; streamed chunks are pinned to the resolved URL index, not re-resolved by public path; queued reads observe cancellation; text-resource escaping is single-pass with worst-case pre-sizing.
- **Store/startup:** coalesced browsing telemetry is limited to history/recent-search-only deltas, flushes before private cutover, on mobile background, on quit/shutdown, and on the next durable transaction; startup hint bytes are bound to the opened descriptor identity and reused only if the file is unchanged after the profile lock; runtime-private cutover persists pending pre-private browsing.
- **Android:** TTS binder calls run on one serial engine lane with generation/token/utterance gating; stop failure fails the live successor; intent parsing and SharedPreferences/AtomicFile/durable intake work run off main; malformed `EXTRA_STREAM` values are filtered; queued duplicates do not mark handled; transient marker commit failures stay retryable; transient batches and grant flags are bounded/keyed.
- **Reader bridge/storage:** theme batches sort the entire pending queue ancestor-first; reads and writes are phased; static frames spend no shared credit; admitted-but-uncommitted bytes are bounded and retried in order; retirement discards blocked batches; BFCache pageshow keeps credit for synchronous writes and reconciles the snapshot; post failure rolls back and surfaces; idle streams are released on disconnect.

## Independent review and dispositions

- Reader authority/header review: found a low cutover-window origin and preexisting Store-error retirement gaps; the MIME test teardown finding was fixed and re-reviewed.
- Transport review: found host-dependent private-sync test endpoints and an unbounded partial-body retry; both fixed and re-reviewed. Sharing head/body framing and updater final-gate findings were fixed and rechecked; only the low pipelining residual remains.
- Diagnostics review: found a nested-URL leak in all three redactors; fixed in Rust, Swift, and Kotlin; Rust regression added.
- Acquisition review: found stale post-checksum receipt identity, torrent/LocalSwarm publication TOCTOU, and edition original-identity gaps. Unix receipt/identity, finalize binding, LocalSwarm seal, edition recheck, and torrent output digest are fixed and rechecked. Torrent source identity between engine verification and publication and Windows mtime-only identity remain documented residuals.
- Caches review: found an unaccounted post-rename cache leaf, search hits dropped on late excerpt failure, and catalog scratch findings. All are fixed and rechecked; the catalog legacy-foreign-file fail-closed policy is intentional.
- Shell/Store review: found the Reader error-suppression flag dropped, stale watched polling, private-cutover browsing loss, mobile background flush, and startup identity binding. All are fixed and rechecked.
- Mobile review: found Android external Kokoro surviving window retirement, Android retention predicates diverging from Rust, a Live Activity compile/offset bug, and Android main-thread/input findings. All are fixed and rechecked; physical device proof is still required.
- Frontend/storage/bridge review: found private-mode Downloads sharing/seeding, map polling delay, import-error visibility, map status dismissal, catalog pagination containment, root-credit accepted-write loss, BFCache credit, and cross-chunk theme inversion. All are fixed and rechecked.

## Verification exercised

- `npx --no-install tsc --noEmit` — passed.
- `KIWIX_TEST_CHROME=… node --test scripts/test-frontend-surfaces.mjs` — passed; isolated Chromium, local fake IPC, StrictMode dialog/tag/catalog/map surfaces.
- `node --test scripts/test-reader-storage-admission.mjs scripts/test-reader-storage-bridge.mjs` — **15 passed**.
- `node --test scripts/test-reader-video-capability.mjs` (Node 22) — **21 passed**, including batched theme reads and inverted mutation rescan.
- `cargo test --offline --manifest-path src-tauri/Cargo.toml --lib` — **1353 passed, 0 failed, 25 ignored**.
- `npm run build` (TypeScript + Vite production bundle) — passed, with the existing Node/Vite version warning and ineffective-dynamic-import warnings.
- Android plugin host tests: `:tauri-plugin-kiwix-mobile:testDebugUnitTest` — **BUILD SUCCESSFUL** (Gradle, offline, rerun).
- `xcrun swiftc -parse` on the changed iOS sources — passed (syntax only, not type-check or runtime).

## Not measured / residual limits

- No physical iOS/Android device or instrumented native run; Kotlin/Swift changes are host-tested or syntax-checked only.
- No Windows/Linux runtime; Windows receipt/publication identity remains mtime-only and same-user local rewrite cannot be fully excluded without a write-denying handle.
- Torrent publication cannot yet bind the engine's internal piece-verification identity; a same-user rewrite between engine verification and the publication-time snapshot remains a documented local-write residual. Output bytes are now re-hashed and must match the copied source digest.
- Archive inline text still builds the escaped document in memory; worst-case pre-sizing reduces reallocation but a 16 MiB quote-heavy text entry can still peak above 100 MiB.
- Sharing HTTP/1 pipelining retains a narrow low-severity deadline interaction; browsers do not pipeline.
- Updater still buffers the artifact and holds the exclusive admission writer across the download because the plugin API returns a `Vec<u8>` and changing the admission/privacy handoff would require a service-level split. Metadata checks remain bounded but non-cancellable.
- Android intake over-cap deliveries still enqueue one worker task before the bound rejects them; transient/grant collections themselves are bounded.
- No long-running network, multi-GB transfer, memory-pressure, thermal, or battery measurement.
