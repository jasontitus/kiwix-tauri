# Project review remediation — 2026-11-23

**Base:** `4459fb9fd91796891513cba2e6749d835882bd0f` (clean tree at review start)
**Input:** user-supplied `project-review.md` (untracked; not committed)
**Scope:** source remediation of the review's highest-impact UX, reliability, and performance findings, followed by independent adversarial review of each cohesive slice.

## Implemented areas

- **Reader authority and serving:** exact live-origin navigation index independent of the async reader lease; malformed archive MIME headers fall back to `application/octet-stream` instead of panicking; archive metadata wording no longer trips the internal-term filter.
- **Settings and error boundaries:** presentation-only settings saves take a reader lease instead of the write-preferring privacy writer and merge only submitted keys into current settings; `LOW_DISK_SPACE` envelopes are shape-validated and preserved; crash URL redaction rejects nested URIs; stack budgets count frames.
- **Transport:** StreetZim read-idle deadlines, updater artifact read/connect bounds, authenticated segment/single-stream retry with `If-Range`, Private Sync listener survival and 4 KiB pre-enrollment framing, LAN PIN re-render without credentials, and bounded sharing sockets.
- **Acquisition and caches:** LocalSwarm resume rehashes only durable verified bits; Web Seed resume reports retained offset and incoming rate; edition readiness reuses the worker proof against live ownership; publication rejects a post-checksum identity change; prefix/map caches use real LRU promotion; map cache is 64 MiB with 16 MiB entries; catalog traversal publishes once; suggestions open only visited archives; fallback excerpts are read only for visible rows and excerpt failures keep their hits.
- **Mobile and native:** Android PDF descriptor ownership; retained iOS foreground speech restoration; Kokoro pause/seek/rewind and transport acknowledgements; session-bound widget/Live Activity controls; Android external speech stops on window retirement; backward Live Activity seeks; idle model release and background-work separation.
- **Frontend:** StrictMode dialog guard and focus cleanup; single error ownership; tag draft parsing; raw identifier/label cleanup; private-mode share/seed gating; map polling restart and dismissal; catalog page-link containment; watched-download pruning and hidden-pause; per-pane error boundary; dismissible locale failure with retry.

## Independent review and dispositions

- Reader authority/header review: found a low cutover-window origin and preexisting Store-error retirement gaps; the MIME test teardown finding was fixed and re-reviewed.
- Transport review: found host-dependent private-sync test endpoints and an unbounded partial-body retry; both fixed and re-reviewed. The 128-connection/90 s sharing idle tradeoff remains open by design.
- Diagnostics review: found a nested-URL leak in all three redactors; fixed in Rust, Swift, and Kotlin; Rust regression added.
- Acquisition review: found a stale post-checksum receipt identity (F1); fixed with an explicit identity gate and re-reviewed. ctime-restored receipt reuse (F2), torrent/LocalSwarm publication TOCTOU (F3), and edition original-identity recheck (F4) remain preexisting residuals.
- Caches review: found an unaccounted post-rename cache leaf and search hits dropped on late excerpt failure; both fixed. Catalog oversized-page abort and fixed pending-name scratch reuse remain low-severity residuals.
- Shell review: found the Reader error-suppression flag dropped and stale watched polling; fixed and re-reviewed.
- Mobile review: found Android external Kokoro surviving window retirement, Android retention predicates diverging from Rust, and a Live Activity monotonic offset; fixed. Physical/instrumented Android and iOS proof is still required.
- Frontend review: found private-mode Downloads sharing/seeding, map polling delay, import-error visibility, map status dismissal, and catalog pagination containment; fixed. Remaining low items are source-traced in the review transcript.

## Verification exercised

- `npx --no-install tsc --noEmit` — passed.
- `KIWIX_TEST_CHROME=… node --test scripts/test-frontend-surfaces.mjs` — passed; isolated Chromium, local fake IPC, StrictMode dialog/tag/catalog/map surfaces.
- `cargo test --offline --manifest-path src-tauri/Cargo.toml --lib` — **1313 passed, 0 failed, 25 ignored**. The first integrated run found two map admission regressions caused by using the new cache budget as a chunk size, fixed by using the 16 MiB protocol chunk limit.
- `npm run build` (TypeScript + Vite production bundle) — passed, with the existing Node/Vite version warning and ineffective-dynamic-import warnings.
- `xcrun swiftc -parse` on the changed iOS sources — passed (syntax only, not type-check or runtime).
- Targeted map, acquisition, reader-authority, MIME, and settings regressions were exercised during remediation.

## Not measured

- No physical iOS/Android device or instrumented native run; Kotlin/Swift changes are source-reviewed only.
- No Windows/Linux runtime; platform-specific filesystem and WebView behavior is not certified.
- No long-running network, multi-GB transfer, memory-pressure, thermal, or battery measurement.
- Structural residuals (Store write-behind, archive double-read serving, reader-bridge theme scan, reader-storage credit redesign, Android main-thread TTS, updater streaming/cancel) remain.
