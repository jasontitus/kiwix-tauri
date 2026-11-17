# Reliability

Status of the "open a downloaded ZIM, read it, restart, still read it" loop.
The newest exercised scope is recorded first below. Dated sections retain their
own build, platform and hardware boundaries; none constitutes release,
blanket hardware or complete-upstream-parity certification.

## Runtime suite and real-archive checks — 2026-11-12

This batch records exercised verification of the integrated parity work at
`828cec4` and the dependency refresh at `e49e64c` (the later `512c9be` and
`a02c458` changes are test-harness-only). It is not release certification and
does not replace the platform-specific acceptance gates below.

- **Full host suites:** `scripts/dev.sh test` passed **1116 tests, 0 failed,
  19 ignored** against the reviewed Xapian 1.4.32 desktop SDK. `node --test
  scripts/test-*.mjs` passed **340 tests, 0 failed**. The full JS run exposed
  one test-harness omission, not a product defect: the extracted history
  functions now call `resetReadAloud`, so the harness needed the same
  paint-only stub it already uses for `stopSpeech`/`resetReadingPan`; fixed in
  `512c9be` and re-run clean.
- **Real trailing-slash ZIM:** `wikipedia_en_mathematics_mini_2026-06.zim`
  contains the slash-only entry `+-*/` and no exact `+-*`. A headless request
  for `/%2B-%2A` returned **307 Temporary Redirect → `/Arithmetic`**, proving
  the exact lookup missed and the one-step trailing-slash fallback resolved the
  real entry. The canonical slash request produced the same redirect. Distinct
  entries and non-entry failures remain covered by the focused Rust tests.
- **iOS simulator:** a fresh `aarch64-apple-ios-sim` SDK was built, the
  simulator app was installed and launched on an owned iOS 27.2 simulator, and
  the actual `KiwixPhoneWindowUI/PhoneWindowUI/testLaunch` XCTest passed
  **1/1**. The normal `iphone.py` simulator harness still refuses a non-pinned
  launch because its runner is bound to the old `3cce5af…` comparison revision;
  this run used a manually authorized XCTest plan against the new app.
- **Android emulator:** the integrated API 37 arm64 debug APK installed and
  launched on the existing API 36 emulator. Through the ordinary **On this
  device** flow, a real SAF folder grant exposed
  `osm-washington-dc.zim`; the app opened it in place and rendered the
  Washington map. Gradle `testDebugUnitTest` passed **38 tests, 0 failures**,
  and the Android Rust typecheck passed. The real
  `AndroidEvaluationTest#productionComposerAndChecksumsOnDevice` instrumentation
  probe also passed on a clean AVD after rebuilding the debug app with the
  `android-evaluation` feature: `status=admitted`,
  `nativeLibraryLoaded=true`, `workloadExecuted=false`, exact APK SHA-256 and
  arm64 ABI bound. The emulator is API 36, so the Android 17
  `ACCESS_LOCAL_NETWORK` runtime prompt/transfer path was not exercised.
- **Android evaluation workload:** the fixture-backed composition/checksum
  workload was not run because `.native/evaluation/android-assets` and its
  manifest/cases are not present. The probe proves the native bridge and
  hardware/APK admission only; it is not a performance or fixture-workload
  result.

- **Dependency refresh and physical iPhone:** after the Vite 8/Rust lockfile
  refresh at `e49e64c`, `npm ci`, production build, **340 JS tests** and
  **1116 Rust tests** passed, with zero non-ASCII JS chunks. A fresh Android
  AVD installed/launched the API 37 APK and rendered the real Washington ZIM.
  The dependency-updated iPhone build installed on the iPhone 17 Pro Max test device and passed the
  launch/preservation smoke (`device-launch-4b97119f…`); the only preservation
  delta found in the prior attempt was the new empty `pinnedBookIds` default,
  now normalized as schema, not a user-state change. Physical-device scope is
  install/foreground launch/profile preservation only.

Evidence: `.native/runtime-smoke-20261112/` (screenshots and summary; private
local artifacts). Simulator/emulator instances and test-owned app data were
removed afterward. Not run: fixture-backed Android evaluation workload, full
iOS comparison/acceptance XCTest scenario, or broader physical-device reader
acceptance for this build.

## Reader controls and document bootstrap — 2026-11-05

Exercised in Chromium at 390×844 through the production headless app, an owned
profile and the real Wikipedia and Hawaii archives:

- A genuine one-finger downward pan that also scrolled the document hid the
  reader toolbar, the shell bottom navigation and the workspace bar, and the
  article box grew from **736 to 844 CSS px**. An upward pan restored them, as
  did touch cancellation, a key press and the explicit reveal button.
  Programmatic scrolling never hid them.
- A first pan that started at the top of the article revealed, then hid once the
  document actually scrolled, instead of discarding the gesture.
- A forged `reading-pan` message posted by an archive script before the bridge
  did not hide the controls. Pan reports carry the bridge's own per-document
  token, a monotonic sequence and the current document URL.
- The bridge now precedes archive code. An actual `<!DOCTYPE html SYSTEM
  "id><script>…` document ran its script before the bridge at the previous
  insertion point and runs after it now. Pre-head scripts, a fake `<head>` inside
  an earlier script, abrupt comments and `PUBLIC`/`SYSTEM` doctypes keep the
  bridge first.
- Authored root styles survive: `<html style="--article-width:42rem">` still
  yields a 756 px article after bootstrap appearance ran. The parsing mode is
  the one the browser gives that doctype; an XHTML Transitional public
  identifier is quirks mode in Chrome itself, not a regression.
- Leading text at the 64 KiB scan limit is never split: injection moves before
  the run, so a multibyte character or an entity at the boundary stays intact.

Limits: Chromium, not WebKit and not a physical iPhone; the pan is a CDP touch
sequence, so real-finger momentum beyond the observed settle frame is unverified.
This is reader behaviour, not a claim that arbitrary archive content is safe.

The physical-window acceptance flow was rehearsed on an owned iPhone 17 Pro
simulator (iOS 26.4) with a seeded managed Wikipedia archive: launch, Library,
offline search, the real Animal article and body assertions, and the map steps
completed. The reading-pan probe recorded `hidden=false`: XCTest drags on a web
view are delivered as native scrolls without DOM touch events, and the feature
deliberately requires a genuine touch, so the probe is diagnostic only. A
Documents-seeded archive also showed the mobile profile requires managed
archives under `archives/mobile-imports-v1/`; that is a seeding constraint, not
a user path.

## Branch integration and original-file opening — 2026-11-05

The integration retains the already-merged dependency reductions and combines
the Android HTTPS-verifier, Windows cfg/CI, bounded-HTML, macOS sandbox, Apple
SDK declaration fix and libzim upstream submission bundles. Those upstream
patch documents do not change the application SDK recipe. Verification below
is the initial six-branch integration snapshot, not a measurement of new APK,
IPA, startup or whole-app memory gains.

Independent review found and corrected four boundaries:

- Capture the original privacy/session generation before macOS folder prompts;
  a later private-mode change must not turn a stale operation into a persistent
  import.
- Normalize trailing separators and dots before the sandbox no-follow directory
  syscall. Actual Darwin regressions reproduced the trailing-slash symlink
  bypass and verify refusal alongside successful real-directory writes.
- Keep failed bookmark writes dirty for retry. The failure/repair/restore
  regression failed before the correction and passed afterward.
- Include deferred HTML tree-building work during EOF and explicit table
  closure. Sink checkpoints enforce cancellation and work/node/attribute/CPU
  budgets inside that work; finalization occurs once and its final partial
  interval is checked and charged to the shared MHTML budget. Only the private
  refusal marker is caught; unrelated panics retain their normal behavior.
  Both the concrete EOF budget-bypass and shared-budget regressions failed
  before the correction.

### Actual macOS sandbox boundary

An isolated ad-hoc-signed bundle, `org.kiwix.workbench.integration20261105`,
initially blocked before showing its window. A native sample placed the main
thread in the new bookmark accessibility probe's directory `open`. The fresh
profile supplied only the real Downloads root. Its underlying kernel wait
was not diagnosed.

The real Downloads directory is already persistently authorized by
`com.apple.security.files.downloads.read-write`; probing/bookmarking it during
profile persistence is unnecessary. The registry now excludes only that exact
normalized entitlement-backed root, before both probing and restoring old
entries. Old entries are pruned on the next permitted save, not during restore.
Custom folders retain the original grant-before-commit and restore rules.

The corrected signed bundle passed fresh native startup, ordinary file and
containing-folder panels for an external real Wikipedia copy, normal Quit,
and relaunch without archive arguments. The bookmark registry retained only
the chosen external folder. The same signed bundle then restored that profile
in headless mode, read the actual Animal preview, and exported its **611,348
bytes** into real Downloads **without a new picker grant**. Host readback
matched the source ZIM exactly; the owned proof export was removed. An
ungranted existing fixture path failed import with a generic native open error;
that message does not retain errno and is not a specific OS-permission diagnosis.

Native UI/state were observed through accessibility without VoiceOver.
Native screenshot capture was unavailable (`CGPreflightScreenCaptureAccess`
was false); Chromium screenshots separately admit actual Animal content and
Hawaiian geography.

The same signed bundle then exercised the download and removal paths that the
earlier pass did not. An HTTP download from loopback published a real 36 KB ZIM
into the entitled Downloads root, imported it as a managed library archive, and
`remove_book` with `deleteFile` moved it to
`~/.Trash/kiwix-deleted-<uuid>/` with the library record gone; a second import of
the same archive was refused as a duplicate. The sandbox sources
(`tauri.conf.json`, entitlements, `library_files.rs`, `downloads.rs`) are
unchanged since the bundle's integration commit. Torrent downloads, removable
volumes, printing, camera/location, and every sandbox transition remain
uncertified. Existing unsandboxed Mac profiles are still not migrated
automatically.

### iPhone report and no-copy intake correction

Read-only collection of the connected iPhone's retained OS logs and profile
corroborated the owner's force-quit at **16:56:52 PST** and relaunch at
**16:57:04–05**. That terminated process was the earlier comparison process
which had shown an archive-document retirement error. **[INFERENCE]** A
retained/inert reader may explain the later noninteraction, but continuity,
the exact retirement branch and the underlying stall are not established.
Local app diagnostics were disabled. The collected export has no matching
branch-level app diagnostic or stall stack. No speculative timeout, retry,
authority relaxation or claimed hang fix follows.

The managed-file inventory did confirm new copies. The incoming dialog had
treated an absent/false `canReadOriginals` hint as permission to make its
ordinary primary action copy. It now routes those cases to the native
original-file picker (`asCopy: false`); true hints use the existing authenticated
incoming-original path. Only the separate explicit storage action copies.
Errors/cancellation do not fall back to copying. No existing copy or original
is migrated or deleted.

Nine trusted browser interaction cases exercised the actual React dialog:
missing/false/true hints in ordinary and session-only mode, plus each explicit
copy action. This proves rendered action selection, not native File Provider
authority. The new iOS change was not installed on the phone; the log pull did
not launch, reinstall, kill or reproduce the app failure.

### Physical iPhone acceptance window — 2026-11-06

Source `12fa8b5`, iPhone 17 Pro Max (iOS 27.0.1), run
`device-acceptance-3582be14763c405dbc7604ebd367c838`, status **passed** in
120 s after installing the signed app and runner over the existing profile
(seven archives preserved). The runner opened Library → Wikipedia 100,
searched **Animal**, rendered the real article body, and exercised the
reading-pan probe: `KIWIX_PAN_PROBE hidden=true shellBefore=true` then
`restored=true` — the first physical-finger confirmation that the shell chrome
hides during an upward reading drag and returns afterward. Map home/zoom/pan and
every other phase passed; the tool released the phone (`phone_released: true`)
and left archives, tabs and settings intact. The first attempt failed because
the Wikipedia card sat below the fold in the seven-archive library; the runner
now scrolls the library before tapping, and the observation timing remains an
existence check rather than a hit test.

### Physical Android acceptance — 2026-11-06

Source `12fa8b5`; device SM-A356U (Galaxy A35, Android 16, arm64). A universal
debug APK (518 MB) signed with the existing debug certificate
(`b2a7c3e8…`) installed over the current profile with `adb install -r`; the
archive library, tabs and the open Animal article were preserved. The build used
the rustup stable toolchain because the Homebrew Rust installation has no
Android `core`/std (`E0463`), and the `mobile-decoded-cluster-v1` schema-2 SDK.

Exercised with real touches and screenshots: launch restored the Animal article;
Library still listed the archives; opened Wikipedia 100; searched **Animal**
(about **1,040** results) and opened the article body; Reader navigation →
**Table of contents** listed the real headings and selecting **Reproduction**
scrolled to that section; an upward reading swipe hid the reader chrome behind
the compact **Reader navigation** pill and a downward swipe restored it. Back at
the article scrolled it to the top; three further Back presses backgrounded the
task (launcher foreground) while the process stayed alive, and relaunching
rendered the article normally. No `FATAL EXCEPTION`, ANR, FORTIFY or
destroyed-mutex message appeared, though the task was backgrounded rather than
exiting, so the earlier process-exit defect was not re-triggered. `uiautomator`
does not expose the WebView text, so TalkBack traversal, the Android 16
DocumentsUI move, 16 KB-page hardware and physical background speech remain
separate acceptance gates; this was a debug build, not a store release.

### Exercised verification scope

Reading width and reader-search accessibility were exercised in Chromium against
the real Wikipedia 100 fixture on the current build. **Comfortable** produced a
936 px measure with 82 px auto margins at an 1100 px reader viewport; **Full
width** removed the archive's `#content` limit (8096 px → `none`) and body
limit; **Original** removed the attribute and stylesheet. Search announced
`1–50 of 500` and then `51–100 of 500` after **Next**, the empty-query
validation was announced once, and Escape closed the panel and restored focus to
**Search books**. The native windowed build launched and created its Kiwix
window under the new navigation policy; the external-navigation refusal is
covered by the policy unit test, while a packaged-webview runtime assertion
remains open.

The initial consolidated Rust suite passed **993 tests, 17 ignored**; after
the later preview and entitlement-root corrections, **12 focused sandbox and
article-preview tests** passed. Rust Clippy with `--all-targets`, custom protocol
and warnings denied passed. Vendored Rust cfg and Objective-C warnings remain.
TypeScript/Vite and **221 JavaScript tests** passed with the final intake UI.
Four explicitly selected native LocalSwarm TLS/application-exchange tests passed.
Workflow structural actionlint passed; its default ShellCheck integration
reported four existing SC2094 informational false positives on checksum
commands which explicitly exclude their output file.

Real Wikipedia and Hawaii rendered through the integrated backend in Chromium,
including genuine Hawaii → Library → Wikipedia/Animal → Library → Hawaii
iframe handoffs. The real Animal preview's formerly empty lead now contains
the introductory paragraph. Its MHTML has **81 decoded parts**, **1,463,112
container bytes**, and a byte-identical original root. Deep-markup refusal
preserved a pre-existing export sentinel. These are functional smoke results,
not physical-iPhone performance or a repair of the reported iPhone hang.

### Later-arriving torrent-runtime integration

The additional runtime-isolation branch (`4763857`) and the Apple compatibility
branch's temporary-CI reverts (`943c858`) were subsequently merged. The permanent
explicit-dispatch native TLS check remains available; temporary probes are gone.
Equivalent local iPhone-profiling commits were also retained in merge ancestry.

The combined host Rust suite then passed **1,001 tests, 17 ignored**, serially,
including the real-session blocked-runtime pause/cancel/suspend, abandoned-add
reaping and re-add regressions, without an IPv6 compatibility shim. Clippy with
`--all-targets`, custom protocol and warnings denied, TypeScript/Vite, and
workflow structural actionlint passed again. The verifier script's three
remaining checks cover pinned dependency compatibility, vendored upstream
integrity and revocation-host policy. Source-text wiring assertions were removed;
they are not substitutes for manager behavior or native packaging acceptance.

The rebuilt production headless app also downloaded the real **150,605,015-byte
Hawaii StreetZIM** from an independent aria2 seeder through a controlled HTTP
tracker. The harness confined outbound networking to localhost and verified
nonlocal egress denial; native wildcard listeners were not claimed to be
loopback-bound. An in-flight pause returned in **18.26 ms** at **1,969,367
reported verified bytes**, with stable reported progress afterward. Resume then
completed, the asynchronous import reached `imported`, and the managed Hawaii
book was available. Source and received SHA-256 both were
`d2fe53fc134651433c2b174ff8a683aeacf9529eea45111d4e599b0b9f95b18d`.
The application and seeder both exited normally with status zero.
This proves that controlled real-payload application path, not network
quiescence during pause, saturated-worker behavior or platform TLS verification.

Prompt pause/cancel return, eventual abandoned-add cleanup, and session-wide
network retirement remain distinct guarantees. These host results do not
establish slow-CRL behavior in the merged Android APK or new mobile performance
measurements.

## Idle HTTP clients and archive bootstrap lifetime — 2026-11-05

Archive storage callbacks now retain only the initial read-only flag and privacy
generation, rather than the complete bootstrap payload. Seeding, local mirrors,
quota/credit checks, mutation sequences and lifecycle generation checks are
unchanged. A bounded isolated Chromium diagnostic seeded 524,288 ASCII characters,
cleared storage and made a fresh write. With explicit GC, the original bootstrap
remained reachable before the fix and became collectible afterward; generation
17 and sequences 1/2 matched. This establishes object reachability, **not a
payload-byte or whole-app memory saving**. Two prior diagnostic attempts timed
out because ordinary tool evaluation used an isolated JavaScript world; explicit
document main realms corrected the observer, not the product.

Acquisition and map managers now construct their independent HTTP/TLS clients
only when an actual manager HTTP operation needs them. Successful initialization
is shared; errors are not cached. Builders, transport policies and cancellation
fences remain separate and unchanged. Manager/journal initialization and local
durable recovery still run offline. A builder failure now becomes a request/job
error while history remains available, rather than preventing manager creation.

The current native debug binary (`a6ac8f2b…`) exercised all six real-Wikipedia
recovery cases again: complete source-matching files, durable publication,
fresh-library import and actual Animal HTTP 200/611,641-byte HTML passed.
All six two-second restart windows made zero runner API calls; positive opt-in
recovery produced requests 5→9, while the other five remained 5→5.
Manual-pause precedence, changed ETag/date generations and zero-checkpoint
rollback remain explicit. This is current native functional acceptance, not
physical mobile recovery or a timing benchmark.

The same current native binary composed and imported actual Hawaii using the
official HTTPS catalog/recipe/range paths: **148,771,331 bytes**, SHA-256
`37aa2a97418352a8d5189d2123653049c910d769726ef08c82ff0d0cf4691b6d`.
Recipe, stored trailer and independently recomputed MD5 all matched
`e0efd5395d673ac92d5bf5d676056484`. This newer output is distinct from the older
150,605,015-byte phone archive. An initial helper incorrectly expected nested
book metadata after successful composition; it remains an incomplete attempt.
The corrected flat-ID check passed in a fresh full run. All owned children stopped.

The reviewed production ARM64 APK (`7d45d21e…`) was installed in place and
physically rendered Animal images, Hawaii, Hilo search and selected detailed
roads/buildings/streams. Its [receipt](benchmarks/2026-11-05-android-idle-memory.json)
separates those observations from native debug transfer checks and the
[still-unfavorable whole-app comparison](PLATFORM_COMPARISON.md).
One English Animal tab and ordinary permission settings were restored; no
speech or iPhone access occurred. Host verification passed 946 Rust tests
(16 ignored), 216 JavaScript tests and the Rust Clippy command with warnings
denied. Vendored Rust cfg and Objective-C compiler warnings still appeared;
this is not a warning-free native build.

The separate application-package size trial subsequently passed physical Animal
article-title search and Hawaii/Hilo detail on `55de4378…`, but its memory and
startup measurements did not justify changing the default optimization policy.
The default `7d45d21e…` APK was restored with exact installed hash and actual
English Animal/one-tab rendering observed; ordinary file access was restored
Off. The [compiler-trial receipt](benchmarks/2026-11-05-android-root-size.json)
keeps artifact-size gains, the initial symbol-stripping mismatch, two different
startup-enabled cohorts and restored-default evidence separate. This is not
full-text, routing, search-latency or broader lifecycle certification.

## ARMv7 real-archive and LP32 boundary acceptance — 2026-11-05

A fresh six-correction SDK and frozen production ARMv7 APK were built from
`a044370736519e400ab9d0901bb88714c31a5eb8`, excluding paused working-tree
changes. The APK is **57,122,861 bytes**, SHA-256
`c032fdb82a229ac3b63ee06def8098ee18c706cd4fdb5524573a0588e1fb2bd2`.
Its manifest matches the accepted ARM64 ASCII build; ELF32 ARM, all 27 JNI
exports, 16 KiB ELF/ZIP alignment and local development signing passed.
The [architecture receipt](benchmarks/2026-11-05-android-armv7-compatibility.json)
binds the SDK, source, artifact and private runtime evidence.

On the Samsung SM-A356U, Android 16, actual installation selected
`armeabi-v7a`. The native app ran ARM32, but its WebView renderer ran ARM64.
Actual Wikipedia 100 Animal text/images/classification, submitted title-search
results, Hawaii vector maps, Hilo offline search and selected detailed roads
rendered. This is **not a 32-bit-only OS/WebView or low-memory-device proof**.

First launch displayed: “This app isn't compatible with the latest version
of Android. Check for an update or contact the app's developer.” Ordinary OK
was acknowledged; no warning suppression was added. Deprecated ABI is a
possible explanation, **not an established diagnosis**. Warning-free modern
Android acceptance remains open. One Back press did not retire MainActivity;
no ARMv7 activity-free notification/lifecycle acceptance is claimed.

A separate actual LP32 native differential exercised a sparse raw cluster:
4-byte pointers/`size_t`, 8-byte `off_t`, **4,294,968,349 logical file bytes**
using **16,384 allocated bytes**. Reverting only the LP32 raw-blob correction
failed all five head/boundary/high/tail/next reads (exit 1, four exceptions);
the corrected implementation passed all five (exit 0, no exceptions).
Both retained the safe empty oversized-whole-blob behavior and removed their
fixtures. This synthetic boundary case supplements, not replaces, the
separate real-ZIM application smoke.

The accepted ARM64 ASCII APK (`0f043901…`) was then restored in place;
installed hash/ABI and actual English Animal images with one tab were checked.
Both benchmark apps were stopped and Home restored. No APK memory, startup
or search-speed comparison was performed in this compatibility run; it does
not change the ARM64 comparison results. No accessibility speech or iPhone
access occurred.

## Silent Android accessibility-provider observation — 2026-11-05

A one-shot UIAutomator dump had exposed only native wrappers despite visible
Settings controls. A bounded same-signer diagnostic on the installed ASCII APK
(`0f043901…`, Samsung SM-A356U / WebView 156.0.8078.49) then kept one silent
UiAutomation connection and queried the owned WebView provider. At 856 ms the
provider was absent and the exported sample contained seven native wrappers;
at 1,133 and 1,922 ms the provider existed with a virtual child and the bounded
64-node samples included named navigation buttons and Settings content.
**Those populated observations preceded CDP AX activation.** Provider queries
can themselves initialize accessibility; this is not an unperturbed baseline.

The [sanitized diagnostic receipt](benchmarks/2026-11-05-android-silent-accessibility.json)
also records a later 358-node Chromium AX tree with named theme/language selects,
reader controls and headings. The intended native after-CDP phase did **not**
complete: the bounded host-marker handshake expired. An initial helper attempt
also failed by checking before asynchronous WebView attachment; both incomplete
attempts remain explicit, not successful paired-probe certification.

This supports an activation/observation sensitivity, not a demonstrated globally
hidden application tree. No unconditional accessibility forcing was added.
It is **not physical TalkBack, gesture, focus-order or full accessibility
acceptance**. No VoiceOver/TalkBack enabling, spoken feedback or audio playback
occurred. The helper and owned forwarding were removed, the product hash was
unchanged, accessibility user attributes matched before/after, and the actual
English Animal tab was restored before stopping the benchmark apps.

## Real HTTP download process-death recovery — 2026-11-05

Six distinct bounded cases exercised the **existing desktop headless binary**,
version `Kiwix Rust 0.1.0`, SHA-256
`574741423e3649f2852af7319c6776c3dd353a7c2ebb5b76818de9cf59491e11`.
This is not a rebuilt/current-source-HEAD identity claim, Android/iOS recovery
acceptance, or complete acquisition parity. The
[sanitized recovery receipt](benchmarks/2026-11-05-http-download-recovery.json)
records the exact helper snapshots, observations and retained incomplete attempt.

Each case used a fresh owned profile/download root, authenticated production
commands, a throttled loopback HTTP source and an actual child-process SIGKILL;
no download journal was edited. The source was the real
`wikipedia_en_100_2026-08.zim`, **333,429,922 bytes**, SHA-256
`2eb7fd115bfb8644a08a9482f01f52b507428b1b8a30129fead92f7b6715440b`.
Source access remained read-only and its hash matched before/after each batch.

| Case | Restart observation | Recovery |
| --- | --- | --- |
| Default auto-resume off, deliberate pause | Paused/non-interrupted; **28,311,552** retained bytes; source requests **5 → 5** | Explicit Resume used saved Range/If-Range and 206 |
| Default off, active process death | Paused/interrupted; durable checkpoint rolled back to **0** bytes; requests **5 → 5** | Explicit Resume completed using validator-protected ranges |
| Default off, changed ETag | Paused/non-interrupted; **23,330,816** retained bytes; requests **5 → 5** | Stale ETag Range/If-Range received full 200; replacement completed without old-generation bytes |
| Opt-in on, active process death | Requests **5 → 9 before any restart API call**; first observed API row already complete | Automatic startup recovery; no explicit Resume |
| Opt-in on, deliberate pause | Paused/non-interrupted; **28,311,552** retained bytes; requests **5 → 5 before any restart API call** | Manual pause took precedence over opt-in; explicit Resume completed |
| Default off, changed Last-Modified only | No ETag emitted; paused/non-interrupted; **27,262,976** retained bytes; requests **5 → 5 before any restart API call** | Old-date If-Range received full 200, then fresh-date ranges received 206 |

Every request-count observation window was **two seconds**, not an indefinite
no-resume guarantee. In the original three cases, `downloads`/`snapshot` reads
preceded that window; those observations do not independently prove an API-free
startup trigger. The additional three cases opened no frontend and made **no
privileged API calls** during the restart window. Positive opt-in recovery
therefore cannot be attributed to the runner polling `downloads`.

The inspected source contract spawns desktop headless maintenance independently
of a frontend. Its initial interval tick dispatches `downloads`; `download_jobs`
initializes the lazy manager, whose saved-policy gate resumes eligible
paused/interrupted jobs. Direct `downloads` API reads can initialize it too.
That source tracing explains the intended trigger, while the API-free request
observation establishes behavior of the separately fingerprinted existing
binary; it does not establish that binary's source revision.

Both changed-generation cases proved the owned partial retained byte **70** at
offset **4096**, versus the real source's **185**, before restart. The date-only
case used valid HTTP dates `Thu, 05 Nov 2026 00:00:01 GMT` and
`Thu, 05 Nov 2026 00:00:02 GMT`, with ETag absent. All six final files independently
matched the real archive's complete size/hash and the production durable
publication receipt. All six then appeared in their fresh libraries and
resolved actual `Animal`, `missing:false`; reader HTTP returned **200**,
`text/html`, **611,641 bytes**. This is actual import/article retrieval, not
fixture output or a journal-state assertion.

The first helper attempt stopped after successful final-file hashing because
it incorrectly required public `DownloadJob.sha256` to contain the computed
digest. That field is an optional expected Metalink checksum; ordinary HTTP
completion stores its computed digest in the durable `publication` receipt.
The independently re-reviewed helper checked that receipt plus an independent
file hash; the incomplete attempt remains recorded, not counted as a passed
case or product checksum failure. The active-death/default-off case's
pre-kill API progress was **not** retained; positive automatic recovery's
intermediate restored offset was not observed.

Independent review approved each exact helper before execution and accepted
both successful batches' runtime evidence. All retained child processes and
source servers stopped; existing user profiles/settings were untouched.
Capability-bearing raw logs/HTML stay private. No product fix, build, test,
formatter, linter or device operation was performed in this slice.

## Shared HTML response ownership and delivery — 2026-11-04

After `b7012ee`, validated ordinary HTML keeps the existing archive-read buffer
as shared response slices around the bridge injection. It no longer allocates
and copies the complete document again. The bounded head scan avoids allocating
or lowercasing its prefix; HEAD retains representation validation/length without
constructing a discarded body. Partial body consumers retain owned bytes, not
new archive/provider leases. CSP, UTF-8 failure behavior and socket retirement
are unchanged.

The [comparison](PLATFORM_COMPARISON.md#measured-follow-on-optimization-shared-html-responses)
and [response-path receipt](benchmarks/2026-11-04-response-path.json) record
release-profile constructor measurements and 1,200 actual debug headless HTTP
GETs using real Wikipedia/production StreetZim HTML. Every measured constructed
body was compared completely; actual GETs retained the source bytes and checked
security headers, while HEAD status/empty-body/length matched. HEAD security
headers were not separately compared. These show additional-allocation,
construction and debug-delivery gains, **not lower whole-app RSS/PSS or faster
shipping/mobile startup/search**.

Actual final-app browser internal navigation rendered Animal/images. An owned
explicit SwiftShader browser rendered the map; the default browser's unavailable-
WebGL warning and earlier observation timeouts are retained separately, not
hidden or called map success. A fresh owned Android emulator then rendered the
final release APK's real title results, Animal/images and Hawaii map tiles
offline. Initial UIAutomator output was missing; independent native pixels
supplied the corresponding surface proof.

Final source checks passed **946 Rust tests/16 ignored** and strict host
all-targets, Android production and iOS production Clippy. Native/vendored
warnings remain. No TypeScript changed in this follow-on; the frozen platform
frontends retain their earlier 211-test/build evidence. An initial host build
correctly refused an absent default SDK; selecting the existing verified
schema-2 desktop SDK built the actual before/after applications.

Both final optimized phone updates were installed in place without uninstalling
the app or clearing data. Android's fresh snapshot retained twelve books/four
tabs, 31 profile files, seven native preference files, one persisted URI grant
and all eight runtime-permission records exactly. Installed APK hash matched;
the temporary signature-authorized snapshot helper was removed. Ordinary
Activity Manager launch succeeded; no new physical-reader screenshot is claimed.

iPhone fresh prelaunch captures retained three books/two tabs, all ten exported
profile files and native preferences byte-exact. Archive reported metadata and
absent provider-bookmark inventory matched; archive bytes were not hashed.
The raw strict check remains **false** for the new data-container UUID and five
specific OS-attributed data-protection xattr additions, accepted separately.
After normal launch, only three managed book paths and `downloadDirectory`
rebased to the new root, with matching root/pending bookkeeping; one existing
history entry's title changed without changing identity, timestamp, scroll or
history membership/order. Attribution to ordinary restoration refresh is
[INFERENCE], not backed by a per-event trace. Every other setting, complete tabs/navigation and
remaining state matched. This is narrowly reviewed acceptance, not blanket
path normalization or complete byte-exact State. No fresh physical-reader visual,
archive-byte or OS-grant usability certification follows. VoiceOver remained
off and untouched.

## Reader efficiency and in-place phone updates — 2026-11-04

This later batch supersedes earlier “no physical phone was updated” statements
only for these artifacts and the scoped preservation below. Evidence:
[definitive receipt](benchmarks/2026-11-04-reader-efficiency.json),
[official-app comparison](PLATFORM_COMPARISON.md) and
[packaging policy](PACKAGING.md#reader-efficiency-packaging-acceptance--2026-11-04).
Local evidence is `/private/tmp/kiwix-efficiency.z5otfQ8l`; these are development
artifacts, not a published release or broader feature-parity certification.

### Reader correctness and exercised checks

The archive LRU publishes a pending per-key initialization slot before native
opening. Concurrent misses for the same book, installation identity and path
share one open, without holding the global cache lock during native/provider
work. A failed attempt shares its error with existing waiters, then removes only
that exact slot so a later request can retry. Invalidation removes pending as
well as completed slots: a completing old open may satisfy its existing callers
but must not reinsert itself, and failure cleanup must not remove a replacement.
Deterministic before-fix regressions exposed **eight opens instead of one** for
concurrent misses and a pending open reinserted after invalidation; the corrected
tests cover single-flight, failed-open retry and pending invalidation.

Other bounded changes allocate the reader buffer after acquiring its lock,
use narrow Store getters rather than cloning complete state, cache the logical
archive end and lazily load six non-reader surfaces. Reader/Library stay eager.
No public JSON format contract changed. Full-versus-filtered Android native
probes matched normalization, title/repeated full-text search and expected
article bytes through pathname and retained-FD sources.

Recorded checks: **946 Rust tests passed, 16 ignored; 211 JavaScript tests
passed**. Strict host all-targets Clippy and production Android/iOS Clippy
passed; desktop/Android/iOS frontends built, native probes ran and actual native
Wikipedia/StreetZim rendering and lazy surfaces were exercised. Native and
vendored warnings remain; this is not a blanket warning-free claim.
Bounded Wikipedia/StreetZim memory differences were about 0.1%/0.3% with direction
reversing; coarse readiness shows no robust baseline win. Same-full-SDK warm
native search was mixed, including regressions, and is not whole-app timing.
The package-size result is not evidence of reliable memory or speed improvement
over official apps.

### Physical phone update and preservation boundary

Both phones were updated **in place**, without uninstalling the target app,
clearing its data or substituting emulator state for the phone profile.

- **Samsung:** the accepted release APK is non-debuggable and retains the same
  signing certificate. A temporary same-certificate, read-only instrumentation
  snapshot mechanism was first proven on the owned emulator, used for prelaunch
  preservation evidence and removed afterward. Twelve books, four tabs, 31
  profile files, seven native preference files, the native provider registry,
  one persisted URI grant and eight runtime-permission records were preserved.
  Archive evidence is size/mtime metadata only: archive content was not hashed.
- **iPhone:** three books and two tabs survived the in-place signed installation;
  ten small profile files and native preferences were byte-exact, including
  preserved native provider-registry state (provider bookmarks absent before and
  after). Reported archive metadata matched, but archive bytes were not hashed.
  **Strict identity/metadata equality is false**: the data-container UUID changed
  and five captured JSON files gained only OS `containermanagerd`
  data-protection-attribution xattrs. Preservation was explicitly accepted with
  those differences, not silently reported as fully identical.

Launch commands succeeded, but they do **not** establish a fresh physical reader
surface or postlaunch migration acceptance. The Samsung capture shows the
notification shade, not reader rendering. The Android receipt separately records
the prelaunch preservation boundary and subsequent successful launch command.
iPhone postlaunch capture failed with **CoreDevice 4016**, and later device
inventory was unavailable after disconnect. No phone performance certification
follows from these installs. VoiceOver remained **off and untouched**: no
VoiceOver query or playback was performed.

## Release code generation and Rust diagnostic remediation — 2026-11-04

The isolated one-codegen-unit ARM64 release experiment reduced the signed APK
by **6,766,592 bytes (8.07%)** without changing features, optimization level,
unwinding, native SDK or frontend contents. Actual compiler flags, unchanged
dynamic exports/dependencies, alignment/signature checks and real
Wikipedia/Hawaii map acceptance are recorded in the
[comparison](PLATFORM_COMPARISON.md#measured-follow-on-optimization-one-release-codegen-unit)
and [machine receipt](benchmarks/2026-11-04-codegen-clippy.json).
This is a package-size result, not a claimed phone memory/startup improvement.

Clippy remediation addresses ownership and execution boundaries rather than
adding a crate-wide suppression:

- End synchronous test-lock guards before asynchronous work.
- Preserve complete/cancel/failure precedence and durable receive-state writes.
- Share immutable publication and segmented-transfer context instead of cloning
  its strings/paths for each peer or worker.
- Preserve non-truncating profile-lock opens and native callback/server/archive
  resource lifetimes; retain provider rollback, PDF cleanup retries and errors.
- Remove unreferenced forwarding wrappers and gate genuinely platform/test-only
  APIs at their consumers' target boundaries, including Android evaluation and
  iOS lifecycle/reconnection paths. Public data formats remain unchanged.
- Keep narrow, documented boundary exceptions where native/transaction APIs
  genuinely need their independent arguments or cross-target fields. No MSRV
  increase or blanket dead-code/Clippy allowance was introduced.

An actual rebuilt headless application downloaded the **333,429,922-byte real
Wikipedia** from an owned loopback range server, paused at **30,146,560 bytes**,
exited cleanly, restored that exact paused count in a new process, resumed and
completed with the source SHA-256. Both application processes exited 0.
This proves exercised pause/restart/resume and final byte integrity, not a
throughput win or exact network-byte reuse. Expected client cancellation also
left server-side connection-reset diagnostics; those were not suppressed.
The private profile/server were separate from the owner's normal application.

Final exercised Rust checks used `--locked` and schema-2 native SDKs:

- Host `cargo clippy --all-targets --features custom-protocol,localswarm-profiling -- -D warnings`: passed.
- iOS `cargo clippy --target aarch64-apple-ios --lib --features custom-protocol -- -D warnings`: passed.
- Android equivalent with `aarch64-linux-android`: passed, separately with and
  without `android-evaluation`. The evaluation build retains debug assertions.
- Host library tests with `custom-protocol,localswarm-profiling`: **943 passed,
  16 ignored**. Final headless binary build and real pause/resume smoke passed.

These are specified development-configuration Rust checks, not a blanket
warning-free build. Vendored Wry still reports `macos_12_unavailable`, and
Objective-C notification compilation still reports unused parameters and a
potential strong-capture retain cycle. Windows/Linux native targets and mobile
physical lifecycle/provider/media acceptance were not rerun here.

The integrated optimized Android APK is **77,029,419 bytes**, SHA-256
`911ee1ebbf82388cdc99f0ab8b0319cc5069fa0da08fcc8a2d15c27338ff1e33`.
Its development signatures, 16 KiB alignment, installed hash and unchanged
27-export/five-dependency native boundary passed. Actual offline Wikipedia
title/full-text snippets, Eukaryote/Back/Animal cold restoration and production
Hawaii Honolulu-city map selection/zoom/pan passed separately on this artifact.
Those three-/four-tab acceptance states are not the earlier upstream runtime
cohort. No physical phone was updated. The owned emulator exited 0.
The private AVD, completed download/profile copies and isolated compiler-source
clone were removed after proof. Frozen APKs, receipts/screenshots and reviewed
helper sources as inert `*.source.txt` remain under the owned evidence roots.
Original archives, normal application profiles and VoiceOver were untouched.

## Upstream release comparison and replay navigation — 2026-11-04

The [corrected comparison](PLATFORM_COMPARISON.md) measures our optimized builds
against pinned Apple 3.18.0, Android 3.14.1 ARM64 and Qt 2.5.1 distributions.
The first current release still retained 18,611,674 non-runtime ELF symbol/string
bytes; standard Cargo release symbol stripping reduced its signed APK from
102,420,523 to 83,808,299 bytes. Dynamic JNI exports and actual native reading
remain functional. Release symbolication is an explicit tradeoff.

The same real indexed Go by Example archive exposed a replay defect: a native
Hello World tap requested an external browser, while upstream opened it offline.
Wombat replaces reflected `href` getters with original web URLs. Capturing native
anchor/area accessors before archive scripts preserves actual offline targets
and `<base>` semantics across link actions. Existing shell origin checks,
external confirmation and CSP remain unchanged. Real browser and Android
destination/Back checks passed after correction; Chrome preview and genuine
external confirmation passed. Context-menu runtime proof remains separate.

All 211 JavaScript tests and TypeScript checking passed for the replay correction.
The frozen optimized Mac/Android builds and unsigned iOS device archive built;
no physical phone update followed.

The representative comparison now uses the **333,429,922-byte Wikipedia 100**
and **150,605,015-byte production Hawaii StreetZim**, not the tiny replay archive.
Both apps rendered Animal/Eukaryote, title results and actual map search/pan/zoom.
Our indexed `multicellular` search returned actual full-text snippets.
Wikipedia's five Activity Manager process-cold starts had medians **287/543 ms**;
main-plus-exclusively-attributed-renderer PSS was **10.0% higher ours**.
Post-interaction rendered Hawaii PSS was **18.8% higher ours**. Different restored
map cameras/blank upstream canvases were excluded from startup rankings; upstream
native Home recovered rendering before its accepted map-memory cohort. These are
not first-paint, unrestricted whole-app-memory or physical-device efficiency claims.
Sequential samples, actual spans and workflow-specific storage are explicit.

Representative receipts: `/private/tmp/kiwix-real-archives.MJz5D8fN/`; frozen
artifacts/replay evidence: `/private/tmp/kiwix-upstream-release.DDIvhEIK/`.
The representative owned emulator process exited 0; its private AVD and
executable probe/build scaffolding were removed after acceptance. Inert reviewed
sources, receipts, original archives and frozen artifacts remain.
Historical disposable AVDs/replay profiles were also stopped/removed.
Normal profiles and VoiceOver were untouched. The iPhone portrait bottom-edge
proof below remains separate development-build acceptance.

## Optional speech availability and platform measurements — 2026-11-03

Actual first-open Android screenshots on the preceding APK showed
`SPEECH_OFFLINE_VOICES_UNAVAILABLE` in both the global error surface and an
article **Reload article** band, while the real HTML/SVG article had rendered.
Native discovery already described an unavailable capability; the frontend
incorrectly routed that reason through the reader's blocking failure callback.

Discovery responses and exceptions now reach the speech-capability owner.
The speech panel retains an actionable unavailable status and disabled Start,
with the underlying reason available in technical details and a recoverable
root diagnostic. Successful discovery clears the reason. Genuine playback
errors and existing session/privacy/foreground retirement checks are unchanged.
This does not hide a document failure, enable network voices or install assets.

The corrected ARM64 debug APK SHA-256 is
`a0f182fd3e9faae963de6db0a34acec4cb1e3d74f8c42b85642125f24070ba4b`;
its sole native library is
`7d5fb69502e9ffd3e85ee2861b084c2dce1d1ee14f3458809399271c1ece4586`.
A new owned API 33 emulator used the real app's Open file action and
DocumentsUI Downloads selection, not a fabricated provider grant. The unique
article-body sentence appeared, remained present after 30 seconds settling and
10 memory samples, and no Reload band appeared. Actual Reader navigation →
Read aloud exposed the contextual unavailable status with **Read article
disabled**. Closing that panel returned to the article, and an owned-app
process restart restored the saved article. Screenshots were inspected.
No playback, voice installation, TalkBack/VoiceOver change or physical-phone
mutation was requested by the run.

Independent source/helper/localization review passed. **211 JavaScript tests**
and production TypeScript/Vite passed after this correction; the final Android
package built and its APK signature verified. The earlier **943 Rust / 16
ignored** result belongs to the preview/export batch; it is not described as a
new post-speech-repair test run. Current TypeScript symbol references work; one
diagnostic request timed out, so it supplies no new clean-diagnostics claim.
Offline locale generation now inventories **74 locales / 1,446 sources**.
Strict localization audit **failed** on missing translations: English alone is
complete, and all 73 other catalogs explicitly lack the new English key.
No machine translations or reviewed-language flags were invented.

The prior debug-cohort measurements were superseded for the user's comparison
by the optimized upstream report above. Their warm/restored residency and
controller observations remain historical, not a platform-efficiency ranking.
The installed iPhone bottom-gap/artwork build remains `0cfdd…`; the owner's
subsequent unlock enabled normal launch and the physical portrait bottom-edge
check recorded below. Current optimized comparison builds include the later
speech repair; that is not a new physical phone deployment.

Final Android local receipts:
`/var/folders/jw/h7c1tkps4qn28mv639_hxzx40000gn/T/kiwix-android-native-comparison-htyn4hmy/`.
The preceding-APK failing screenshot, provider-grant/ownership harness failures
and earlier measurements remain separate, not substituted for the corrected
APK's accepted results.

## iOS bottom edge and offline preview artwork — 2026-11-03

The owner's physical-iPhone screenshot showed a light strip below the dark
bottom navigation. The viewport now uses `viewport-fit=cover`; iOS chrome owns
its top/side insets while the existing compact navigation owns its bottom inset
once. Android retains its separate native viewport policy.

Actual application-hosted XCTest on a fresh iPhone 17 Pro Max simulator with
the genuine iOS 27.2 runtime measured a full **440×956** WebView/viewport and
navigation bottom **956**, with native/CSS top and bottom insets **62/34**.
Portrait dark/light snapshots were inspected. A separate XCUI exercise tapped
the real Settings Font size field, displayed the software keyboard, selected its
value through the real edit menu, typed `18`, and verified the field remained
above the keyboard. No VoiceOver setting or user profile was touched.

The earlier landscape-shaped viewport check was insufficient: a stronger
window/controller/WebView receipt showed **956×440** with scene orientation
landscape, but all native inset owners still reported portrait-shaped
`[62,0,34,0]`. The bounded lateral-inset assertion failed. Therefore these runs
do **not** establish landscape-notch safety or identify a product defect;
programmatic scene geometry is not substituted for device-rotation acceptance.
Earlier keyboard attempts using programmatic DOM focus or simulated delete
characters also failed as harness interactions; the final trusted edit-menu
scenario above passed without weakening its visible-field requirement.

The separately built physical-iPhone debug executable is
`0cfddf9a6bb6d69127a680caaf11f54db0d945c35883e77686a8d9049aa2e3a3`.
Strict main/extension signature verification and independent current-profile,
device-membership and Wi-Fi-entitlement review passed. In-place installation
of `com.tiltastech.zimreader` completed in **14.52 seconds**, preserving the
fresh baseline's complete State values, three archive records/metadata, two
tabs, settings, transfer/root files and peer identity under legitimate container
rebasing. Archive bytes were not rehashed. The initial normal launch was denied
while locked. After the owner unlocked the phone, normal launch succeeded in
**1.86 seconds**. A real DVT screenshot (`device/unlocked-surface.png`,
1320×2868) shows the restored Japan/Tokyo reader and dark bottom navigation
extending to the screen edge, without the reported light strip. This captures
the reader, not Library or landscape rotation. No navigation or VoiceOver
interaction was required.

Post-launch comparison retained three books, two tab identities, settings,
archive metadata, downloads/torrents/root files and peer identity. It was **not
an exact State match**: the active tab/window/history title fields and the
pending migration file's `source` field differed. The strict receipt correctly
remains `preserved:false`; these differences are not waived or described as
byte-identical preservation. Receipts: `launch-unlocked.json`,
`after-launched-state.json` and `preservation-launched.json` under `device/`.
The separate `org.kiwix.workbench` installation was not modified.

### Offline article artwork

`article_preview` now returns nullable bounded raster artwork in addition to
plain title/excerpt text. It parses at most 256 KiB of article HTML, resolves
images from the canonical redirected entry and document base, examines at most
eight image elements, and reads only same-archive candidates of at most 256 KiB
each. It never fetches a URL. The existing PNG/JPEG/GIF/WebP signature policy is
shared with catalog thumbnails; SVG and executable archive HTML are excluded.
The shell accepts only the bounded raster data-URL contract and retains text if
an image fails browser decoding.

A real generated ZIM regression passed redirected relative images, a filename
containing a space and literal `%23`, local/external bases, external and
protocol-relative sources, missing/SVG/oversized candidates before valid
artwork, lazy `data-src`, and no-artwork excerpts. Actual owned Chrome against
the rebuilt native Service decoded the **40×24 PNG**, rendered its title and
excerpt, and displayed the external-image article without artwork. At a
**390px viewport**, the card occupied x **22–368** without horizontal overflow;
the observed shell resource list contained no non-loopback resource requests.
This is shared-browser/Service proof, not native mobile thumbnail interaction.

Updated checks passed **943 Rust tests / 16 ignored**, **211 JavaScript tests**,
TypeScript/Vite, Mac compilation and the signed iOS device build. Rust/TypeScript
language-server diagnostics on the changed implementation reported hints, not
errors. The previously recorded strict all-target Clippy backlog remains open;
these results are not a lint-clean or release certification.

Local evidence: `/private/tmp/kiwix-ios-safe-area.PtdKvAYO/` contains geometry
and keyboard `.xcresult` bundles, extracted snapshots/JSON, and `device/`
installation/signing/preservation receipts. Prior failed harness runs are
retained separately; the portrait/keyboard pass is not attributed to them.

## Resource saves, export durability and preview preference — 2026-11-03

The rebuilt Mac executable SHA-256 was
`a96feb432898477b047a3557e9ced1e009272abe64d8340a2d63d62b6ee7ac91`.
Acceptance used an owned actual ZIM, an isolated application profile and an
explicitly headless Chrome profile. No user-phone package, normal library,
permission grant or VoiceOver setting changed.

### Export publication

Before correction, injected parent-directory synchronization failures returned
success for new files, explicit replacement and bare relative filenames.
Post-rename cleanup also deleted an unrelated file that reused the former
temporary pathname. Publication now relinquishes that name immediately, uses
`.` for an empty relative parent, and propagates directory open/sync failures
without rolling back the already-completed destination.

Independent review also found that including an arbitrary destination filename
in the warning could trigger the existing error sanitizer's generic retry copy.
That consumer failure was reproduced before correction. The final filename-
independent warning preserves the distinction: the file was kept, but survival
of sudden power loss is unconfirmed. Five behavioral regressions pass.
Windows directory-durability behavior is unchanged.

### Original resource Save and link previews

Trusted video, audio and image context-menu Save actions completed browser
downloads, retaining the selected resource's name and exact original bytes:

| Resource | Bytes | SHA-256 |
| --- | ---: | --- |
| `sample.mp4` | 2,275 | `4f18740d21ce9e7e5f97655307faa9716f5b946847adf9524b7721034afc62ab` |
| `sample.wav` | 16,044 | `56d4af65701c26df20bd4021eda95b6e830348ce3a746086079fe89285548dc9` |
| `sample.png` | 68 | `b8859a16fbdb1e2483245a867e7d4d53adf8f5888ae482e2bc29a75fe7beb844` |

These match both fixture sources and separately published native Service exports.
The native API also retained the 125-byte PDF's MIME, name and exact bytes
(`034360b8929e7273d543cf892ed752d6f6add7cf407ab271cbdd5a6d290fe5a0`).
This addresses the scoped media-save behavior in desktop
[#1274](https://github.com/kiwix/kiwix-desktop/issues/1274), not PDF rendering or
native-picker/mobile-engine acceptance. Empty reflected image sources cannot
silently select the containing article; external resource hints are rejected.

The persisted “Show article previews for links” choice defaults on for existing
profiles. Actual Settings opt-out survived an application restart; disabled
focus produced no card. Re-enabling restored First preview → trusted Tab →
Second preview, and Escape removed the card. The former bridge cooldown dropped
the second link permanently; its failing-before timing regression now passes.
Root coalescing remains bounded, and disabling invalidates pending/queued replies.
Previews remain bounded plain text without thumbnails or executable archive HTML;
this advances [#611](https://github.com/kiwix/kiwix-desktop/issues/611), not full
wiki-style thumbnail parity.

Verification: **942 Rust tests passed / 16 ignored**, **211 JavaScript tests
passed**, production TypeScript/Vite build, Mac build and iOS-simulator library
check passed. Strict all-target Clippy with `-D warnings` **failed** on the existing
diagnostic backlog (186 library-test errors); independent scoped review found no
reported diagnostic attributable to these export/setting additions. This is not
a lint-clean or release-certified build. Generated resources now inventory 74
locales and 1,445 sources; all 73 non-English catalogs remain partial, including
the three new English strings.

Local receipts: `/private/tmp/kiwix-preview-export.hwEX5oof/`
(`baseline-evidence.json`, `export-evidence.json`, `ui-evidence.json`).
Temporary helpers, application/browser profiles and readiness credentials were
removed after proof. The final bounded supervisor and child exited 0; the earlier
pre-fix baseline's process-group termination was -15, not a graceful exit.
Earlier shared-browser input and download-policy probes were inconclusive, not
product failures. The native `<dialog open>` and a closed recovery-dialog alert
were initially queried incorrectly; final acceptance used actual visible controls
and retained downloaded files rather than those diagnostic predicates.

## On-link Android/Mac → Linux Nearby — 2026-11-03

Owner-authorized tests used a fresh ARM64 Ubuntu guest bridged through `en14`
onto the real LAN. The guest obtained `192.0.2.170/24` on `probe0`;
its actual route to the Samsung at `192.0.2.241` had no gateway.
USB carried phone control only, not Nearby data. No peer record, proxy,
data tunnel or host route override substituted for genuine discovery.

Both publishers used their actual Share all interface. The native Linux
headless application discovered the current publisher session, browsed its
inventory and received one explicitly selected archive through `quic-library`:

| Publisher → receiver | Archive UUID | Received bytes | Actual file SHA-256 |
|---|---|---:|---|
| Samsung SM-A356U → ARM64 Linux | `c495a7aa-88f2-5dcc-a7bc-908af8a61cf2` | 150,605,015 | `d2fe53fc134651433c2b174ff8a683aeacf9529eea45111d4e599b0b9f95b18d` |
| Mac → ARM64 Linux | `28ebe7c3-476c-e916-23b1-44841ff07e48` | 22,783,565 | `315b99b48ca45570900347a68aa759b20f2a2809b461d9898173b4901e8c6e01` |

Each source-matching destination had a complete durable job, appeared as an
available library installation referencing those bytes, opened through
`open_article`, and served its main page as HTTP 200 HTML. Both receiver
processes exited 0 with no cleanup errors. This is acquisition/import and
reader-resource acceptance, not rendered-page or native Linux GUI acceptance.
It does not exercise reverse directions, a multi-archive batch or Raspberry Pi.

Build scope: retained reviewed Linux binary
`03a5d3a412b42e42ff0c027311698c160ea661de061fb92885016048e2d98f26`,
Samsung installed APK
`c671bffbebd0e375af81e218cc1329ad2929b1bb7d3ed0ef6876c14c3a88a310`,
and current Mac binary
`f3d456dceec275dfd8dc4cd781e132316fa7cc46fb90a254751a4fd20dce46b6`.
The Linux test does not certify every newer source change, and no phone package
was installed. Neither helper wall time nor the Android aggregate sent counter
is reported as transfer throughput.

The phone retained all 12 books and 4 tabs, with exact before/after books, tabs,
activeTab, settings, bookmarks, history and recentSearches. Its source file
SHA-256 matched again afterward. Reader storage was not exposed by this phone
snapshot and is not claimed as compared. Actual Stop all withdrew each publisher;
phone debug forwarding and owned browser controls were removed.

The independently reviewed VM helper used fresh standalone copies and public
UTM export/import to disable clipboard sharing without bypassing App Data
protection. Its import input had empty audio/display arrays. A VirtIO `vdb`
seed produced fresh NoCloud provisioning, a real LAN address and an announced
SSH public key that was pinned before connection. Later serial announcement
`echo: I/O error` warnings remain in the log; `cloud-init status: done` with
`errors: []` is not a warning-free-console claim.

The bounded VM session deleted exactly
`8F99F37C-1ACD-464D-B2C7-F1D09CD60378`, exited 0 and preserved the existing
Starshot VM stopped. The original NAT VM was not stopped or reconfigured.
Both immutable backing-image hashes matched before/after. The transfer-session
Mac publisher ended with SIGTERM (-15), with source/executable preservation and
no remaining group/thread errors; that session is not a graceful-exit pass.
A separate later **no-sharing** check, stopping only the supervisor and letting
it signal its child, exited 0. It does not replace the retained -15 receipt.

Receipts: `/private/tmp/kiwix-linux-android.yntQxDoN/cross-platform-result.json`,
`android-transfer-preservation.json`, `android-actual-sharing.png`, and
`mac-publisher/` plus `mac-graceful/publisher-cleanup.json`. The Android durable
journal/state were retained outside the guest at
`<mac-hostname>.local:/tmp/kiwix-linux-android.GCoF0nBq/linux-phone-evidence/`.
The Mac journal was checked inline but not retained: its later export attempt
timed out after automatic VM teardown. Successful captured receiver output,
not a retained Mac journal, supports that scenario. Setup failures remain in
private receipts; none was repinned as a successful transfer.

## Live-document reader history — 2026-11-03

A real owned ZIM reproduced a query/path SPA history defect: toolbar Back
reopened the archive document, replaced its document nonce, discarded an
unsaved input and collapsed its expanded Details. Hash-only TOC Back did not
reproduce that DOM loss; its after-change coverage is a regression check.

The reader now asks the active bridge to traverse only its bounded, positively
known live-document history segment. The complete destination chain and source
session must match; archive `history.state` remains archive-owned. Correlated
completion commits the captured shell stacks once, including repeated URLs and
router `replaceState` during `popstate`. Refusal uses normal reopening.
An unanswered traversal retires the old reader document before fresh-origin
fallback; stale completion cannot revive a retired source.

The actual rebuilt headless application and hidden Chromium passed:
- query/path Back and Forward retained the same nonce, unsaved draft, expanded
  Details and the fixture router's normalized history state;
- TOC A/B Back and Forward restored the recorded scroll positions, and a TOC
  target revealed an initially collapsed Details ancestor;
- the Back history menu selected the earlier occurrence of a repeated URL,
  distinguished by its archive history state;
- a genuine second-document link rendered, and returning created a new document;
- a counted no-op replacement for the owned fixture's `history.go` invoked the
  watchdog, then reopened the exact target on a fresh origin with a new nonce
  and original input; no mounted frame retained the old origin.

That last scenario synthetically suppresses traversal completion. It does not
prove a real browser hang or delayed-pop behavior. Receipts:
`/private/tmp/kiwix-reader-history.7aUKXXO7/spa-before.json` and
`after/result.json`, with `after/cleanup.json` recording bounded owned-process
cleanup (exit -15, no group/thread errors, executable unchanged). The owned
browser tab was closed. No native WebView, physical device, assistive technology,
audio or user profile was exercised.

Final matched-SDK checks passed **936 Rust tests, 16 ignored**, all-target
Clippy with warnings, the desktop build and iOS-simulator library check.
TypeScript/Vite built and **196 JavaScript tests passed**. Independent adversarial
review covered production changes, regression harnesses and smoke helpers.
No device package was deployed.

## Latched profile durability and archive deletion — 2026-11-03

The service's writable admission previously checked only read-only mode.
A profile that had already reported a post-replacement directory-sync failure
would reject subsequent Store mutations, but permanent deletion could first
remove the archive and then return that known error while retaining its library
record. The new owned-profile regression reproduced the missing archive before
the fix and passes after it.

`Store::require_writable` now checks read-only authority and the existing
durability latch under one lock. Archive removal explicitly requires that
admission before destructive work; the general service writable gate is not
broadened. A separate regression latches the profile while a real loopback
download is active and verifies Pause and Cancel still stop and join its worker.
Existing read-only error precedence and private-mode policy are unchanged.

The rebuilt actual headless executable
`f3d456dceec275dfd8dc4cd781e132316fa7cc46fb90a254751a4fd20dce46b6`
passed an independently reviewed, narrowly scoped Darwin ARM64 fault probe.
After normal startup, a one-shot interposer returned EIO only for
`fcntl(F_FULLFSYNC)` on the exact owned profile directory. A real book-zoom
mutation exposed its replacement state and reported the synchronization error;
the following permanent-delete API command returned the latched refusal before
changing the archive's SHA256/inode, neighboring notes, persisted state,
library, tabs or settings. The repository fixture stayed unchanged and the
owned process exited 0 with no remaining group/thread errors.

Local receipts: `/private/tmp/kiwix-durability-final.wJi0WFjW/result.json`,
`fault.events` and `cleanup.json`. The regression uses the existing Store
directory-sync failpoint; the application smoke exercises the actual
`F_FULLFSYNC` failure boundary. Neither is a physical power-loss test.
Admission is not a transaction spanning deletion and a future Store commit:
a new concurrent persistence failure or crash remains a separate boundary.
No foreground UI, audio, assistive technology or user profile was touched.

## Linux Nearby multi-address selection — 2026-11-03

A real two-namespace test on the owned `aarch64-unknown-linux-gnu` VM reproduced
a numeric-candidate selection defect. The server owned `10.123.0.2/24` and
`10.120.0.2/24`; the client owned only `10.123.0.1/24`, with a namespace-local
route to `10.120.0.0/24` through the server. No host route was added.

The production wildcard QUIC library listener advertised both actual addresses.
Before the fix, QUIC to the off-link address succeeded and an on-link-only
inventory request succeeded, but the mixed list with off-link first failed
`Nearby requires a local QUIC route`. The numeric library fallback now closes
and drains a connected off-link candidate, tries the next address, and retains
route authority only for an accepted on-link peer. Legacy LocalSwarm, native
Apple service and retained joined-route branches are unchanged.

After this change, locked all-target macOS Clippy completed with existing
warnings, the full host suite passed **934 tests, 16 ignored**, and the iOS
simulator library cross-check passed. No device package was deployed.

The independently reviewed probe then passed against the repaired production
library/transport path: inventory from the later on-link candidate, all
**36,196 bytes** of the fixture, and source SHA256
`198ef368c47a4dcf6f48819a4baa93af55680b9cb1bdedd5da42b45b20bdd93c`.
Off-link-only inventory still failed closed. Client/server exited successfully,
both owned namespaces and process groups were removed, and the executable
remained unchanged. The after-run host-route records differ only in two IPv6
router-advertisement expiration countdowns; all other route fields match.
The first run's raw before-route record was not retained, so its failed exact
string comparison is not retroactively claimed as unchanged routes.

Before/after test executable SHA256:
`49ee2e8448418c558e5079aacec3e844286d400f0b208973323eb9a3c0732997` /
`8893993b1d8dcbca103e2b2a8daca440290e471bab4c7ebe162f6e6e3eb6428f`.
Local receipts are under
`/private/tmp/kiwix-nearby-candidates.UjMNG0KP/{before,after}/`.
The privileged topology harness and probe lived only in an owned source copy,
not the ordinary test suite. No foreground UI, audio, existing profile or
original VM network configuration was changed.

This proves the isolated Linux numeric-candidate fallback and encrypted
archive-stream integrity, not native discovery, Linux–Mac interoperability,
normal acquisition/import, or Raspberry Pi packaging/hardware acceptance.
It does not remove the earlier VM NAT boundary or explain that cross-platform
failure as this distinct multi-address defect.

## Project language servers and recovery checks — 2026-11-03

Installed Homebrew `rust-analyzer` (2026-11-02 package), linked the actual
`src-tauri/Cargo.toml`, and supplied the verified desktop SDK environment,
locked Cargo resolution, build-script/procedural-macro analysis and Clippy
on save. Actual Rust cross-module references, definitions and standard-library
source navigation worked. The local OMP project configuration is workstation
specific, not a portable repository prerequisite.

The installed TypeScript language server uses this project's TypeScript 5.9.3
server, not the global TypeScript version. Actual references/definitions
worked. An owned temporary file produced TS2322 and then explicit clean
diagnostics after correction; it was removed. Initial clean-file diagnostic
requests timed out, so this is not a blanket workspace-LSP diagnostics claim.

Configured Xcode's Apple clangd 21.0.0 with an owned SDK-scoped compilation
database. Actual `zim::Archive` definition navigation, `preloadXapianDb` API
hover and native bridge diagnostics worked. Installed `cargo-bloat` 0.12.1 and
the matching Rust `llvm-tools` component; actual version commands worked,
including the previously failing `rust-objcopy`. Existing Hyperfine 2.0.0 was
also launch-checked. This does not make the known Clippy backlog clean.

The checks exposed and the reviewed changes repaired:

- A desktop-only `clippy::never_loop` notification-permission compile failure.
  The mobile foreground wait, authority checks and commit guards are retained.
  The final iOS simulator library cross-check passed; native permission
  callbacks and suspension transitions were not exercised in this session.
- A reproduced overflow restoring a segmented journal with range end
  `u64::MAX`. The retained regression fails before and passes after the fix.
  Restoration now rejects the plan while retaining the partial archive.
- Ignored backup synchronization errors in unreadable-journal copy recovery.
  A scoped Darwin interposer forced cross-device rename failure and denied the
  backup's read-only reopen. Before the fix the live journal became `{}`;
  afterward construction fails and the original malformed bytes remain.
  Ordinary recovery still passes. This exercises the backup-open prerequisite,
  not an actual fsync failure or physical power-loss durability.

Final exercised checks: frontend production build and **180 JavaScript tests**;
all-target locked Clippy; **934 Rust tests passed, 16 ignored**; desktop
executable build; and `aarch64-apple-ios-sim` library cross-check. Existing Rust,
native and frontend warnings remain; these are not warning-free builds or
release certification.

The rebuilt Mac executable SHA256
`02d488a840df74a11d7b6a53cfcae6318fb3ffd648a696948346cd50c9cc573c`
then passed a silent authenticated headless application smoke with two isolated
profiles: malformed JSON produced an empty list and a notice referencing its
byte-preserved backup; overflowing segmented history restored a paused job
with the unsafe plan removed and progress reset. The retained source journal,
partial bytes and partial inode stayed unchanged; no destination was published.
Both owned process groups exited. No browser, foreground UI, notification
permission or VoiceOver operation was used.

Local receipts: `/private/tmp/kiwix-toolchain-recovery.bxvuBJ/result.json` and
case cleanup receipts; `/private/tmp/kiwix-quarantine-fault.mGJsBA/` before/after
open-fault results and private console logs. These temporary workstation paths
are not included in a source checkout. Independent review covered the production
fixes, regression and fault/smoke harnesses before execution.

## Actual Mac VoiceOver reader output — 2026-11-03

After the owner's later approval and VoiceOver-ON change, the same frozen native
Mac executable (`b0d411a6c59b654c9ecd7ababc0b2a925bce75a56c22c7573234f5944eb6b339`)
ran with a fresh isolated profile and copied fixtures. Actual VoiceOver
last-phrase output—not an AX-tree substitute—established:

- Map heading/title and its unique body paragraph; Fixture detail link
  navigation and exact native-link Enter activation.
- The newly opened detail article's heading and unique paragraph.
- Native Back to map link activation followed by map heading/paragraph speech.
  Its spoken-label capture did not match, so no spoken return-link pass follows.
- The genuine Keyboard shortcuts dialog and spoken Close dialog button.
  Escape restored native focus to the invoking Shortcuts button.
- Real Command-F, exact native Find-field entry of `fixture`, guarded Enter,
  exact native value verification and Escape restoring focus to Shortcuts.
  No Match found, highlighted-result or spoken-query claim is made.

Post-dismissal reading remains partial. Heading captures from the footer did
not match. Apple's documented previous-frame command subsequently reached the
article and produced its heading, but next/current-paragraph captures still
did not match the body sentence. This is explicit frame-navigation access,
not automatic reading recovery, a diagnosed product defect or full VoiceOver
acceptance. Actual audio was not independently listened to. Library search,
history controls and compact overflow remain unexercised.

The record preserves the old-window-title `-1728` postguard failure after
admitted detail-link activation, successful settled-context recovery, a Find
tree-observer timeout, and an orchestration error that admitted one paragraph
command after a nonmatching heading capture. No failed observation was repinned
as a pass; the earlier modal tree was not reused as current Find evidence.

Owned process 29391 is absent. The stop helper's transient identity-acknowledgment
error is recorded separately from process absence; fresh final state reported
VoiceOver ON and Terminal foreground. No grants/preferences or normal library
were changed. Helpers and stages were independently reviewed.
Evidence: `/private/tmp/kiwix-vo-5xys_j7s/acceptance-result.json`,
`native-controls.jsonl`, `events.jsonl` and `owned-native-tree.json`.
The preceding control-announcement-only run is retained separately under
`/private/tmp/kiwix-vo-9r9jzebl/acceptance-result.json`.

A subsequent owner-authorized listening comparison stopped during setup:
native focus reached the exact map button, but the initial spoken-title
capture did not match. Neither baseline sentence reading nor the Find sequence
ran, so no paired listening result or audible-reading failure follows.
Owned process 40485 is absent; fresh final VoiceOver state remained ON.
Receipt: `/private/tmp/kiwix-vo-wc_lonkj/acceptance-result.json`.

## VoiceOver control, Linux printing and Nearby route boundary — 2026-11-03

- The owner enabled VoiceOver and its AppleScript-control checkbox. A fresh
  macOS process reported VoiceOver ON; a parent last-phrase-content request
  succeeded with empty content. A long-lived NSWorkspace object had reported
  stale state, so fresh-process observations are used. This is control-channel
  evidence, not an announcement or reader pass.
- The current native Mac executable, SHA-256
  `b0d411a6c59b654c9ecd7ababc0b2a925bce75a56c22c7573234f5944eb6b339`,
  launched in fresh owned profiles with copied fixtures. After native foreground
  and owned-window checks, the distinct public `vo cursor` bounds query timed
  out with AppleEvent error `-1712` after 8.54 seconds. Earlier caption and
  keyboard-caret attempts remain separately recorded; the latter's underlying
  response was not retained. No VoiceOver movement, content capture, activation
  or semantic scenario passed. These failures do not prove a missing grant,
  that all scripting is blocked, or an application accessibility defect.
- Mac test processes stopped; Terminal focus was restored where appropriate,
  VoiceOver remained ON, and no preferences/TCC permissions were written.
  Evidence: `/private/tmp/kiwix-vo-_klw5syv/acceptance-result.json` and
  `ownership-probes.jsonl`, with the earlier run under
  `/private/tmp/kiwix-vo-42my5c5w/`.
- In the subsequent owner-authorized Accessibility attempt, the actual
  `osascript` caller reported trusted and System Events reported UI access
  enabled. Native foreground/window/focus checks and a bounded AX traversal
  succeeded; the 74-node tree was depth-truncated and is not semantic acceptance.
  VoiceOver's keyboard-focus commander request still timed out with `-1712`.
  The foreground then changed, invalidating the owned context.
- A public, non-prompting Automation permission check produced no status:
  the initial JXA descriptor bridge failed, and the corrected call reached its
  outer 20-second timeout. Neither result establishes permission denial;
  no further grant request was made. The final native-keyboard alternative
  stopped before sending any keys because System Events reported zero windows
  for the exact owned process at that moment (“Owned app has no native window”,
  `-2700`). An earlier native window was observed; this later report does not
  establish window destruction or its cause. No actual VoiceOver announcement,
  reader navigation or modal/Find recovery passed.
- The owned process stopped. Fresh final state reported VoiceOver ON; later
  foreground activity was preserved without lock/unlock/authentication input.
  Owner grants were preserved, with no TCC or AT preference writes by the worker.
  Evidence: `/private/tmp/kiwix-vo-dhr1qkv6/acceptance-result.json`,
  `native-controls.jsonl` and the final STOP event in `events.jsonl`.
- In a separate isolated sharing run, current native Linux genuinely discovered
  the real Mac whole-library publisher. Exact publisher selection opened its
  dialog, then inventory failed with “Nearby requires a local QUIC route.”
  No proxy or injected peer record was used. Discovery was observed after a
  bounded native Mac DNS diagnostic; no unassisted discovery-latency claim follows.
- Source tracing places that refusal at `localswarm/library_transport.rs:95–104`,
  after connection establishment and before route-authority retention/inventory.
  The Linux VM had `10.0.2.15/24` and gateway `10.0.2.2`; this numeric QUIC path
  requires an on-link peer. That routed/NAT topology did not establish an
  accepted Nearby session. No policy check was weakened, no Retry was activated,
  and no transfer, destination hash, reverse direction or Raspberry Pi run passed.
- Both sharing surfaces used actual Stop all. The owned processes exited
  cleanly; original Linux AT preferences, all seven tracked original profile
  fields and application focus were preserved. The Mac source archive hash
  remained unchanged. Full Linux Nearby acceptance needs an on-link test target.
  Evidence: `/private/tmp/kiwix-linux-mac-nearby.86qCM2ss/`
  `linux-nearby-route-result.json`, `cleanup-result.json` and the viewed
  `nearby-inventory-failure.png`. Native DNS evidence is a first-completed
  address observation, not a complete address census or packet trace.
- The current Linux ARM64 executable,
  `03a5d3a412b42e42ff0c027311698c160ea661de061fb92885016048e2d98f26`,
  ran in a fresh owned profile/HOME in the retained Xvfb/Openbox session.
  Real Ctrl+P opened GTK's native print dialog for StreetZim test map.pdf.
  Keyboard traversal reached the printer list with Print to File selected;
  the native state and screenshot were observed. Only a positively identified
  Cancel button was activated.
- Cancellation dismissed the dialog. Real keyboard traversal and internal-link
  activation opened Fixture detail with its expected native heading/paragraph.
  A subsequent Ctrl+P opened a new dialog for StreetZim fixture detail.pdf,
  proving the preceding cancellation had not left export ownership stuck.
  That dialog was also cancelled. No Print or Preview activation, PDF output,
  physical printer delivery or post-cancel Orca speech is claimed.
- A reverse-tab-order assertion in the second-dialog driver stopped before
  activation: the disabled Print button was skipped, so an extra Shift+Tab
  reached Preview rather than Cancel. Fresh observed focus guided one Tab back
  to Cancel before Return. This was a harness assumption, not a product failure.
- The original system Orca process was not replaced. The guarded supervisor
  stopped the owned app and restored original-application focus. Original AT
  preferences and all seven tracked original profile fields compared exactly;
  cleanup reported no errors.
- Evidence:
  `/private/tmp/kiwix-edition-live-04g1iz06/linux-native-print/runtime-receipt.json`,
  with viewed `native-print-dialog.png` and
  `native-reader-after-print-cancel.png` in the same directory.
- Source reconciliation: desktop Print already uses the admitted native path;
  GTK and WebView2 printer providers are wired. Earlier wording that described
  Windows/Linux printer integration as absent was stale. Windows runtime and
  physical job-delivery acceptance remain open.

## Current Linux accessibility comparison — 2026-11-03

Scope: the current native binary in an owned Ubuntu 24.04 ARM64 Xvfb/Openbox
session, not a GNOME/Wayland or complete assistive-technology certification.
The executable SHA-256 was
`03a5d3a412b42e42ff0c027311698c160ea661de061fb92885016048e2d98f26`.

### Released-stack acceptance

- An isolated, unmodified released Orca 51.1, commit
  `244bdadd98f07fe9bfda9326fab621d686c70d2d`, with genuine AT-SPI 2.62.1
  client libraries passed the current native map/detail reader paths.
  A fresh owned app profile and fresh reader defaults were used; reader
  GSettings used the keyfile backend under its isolated XDG configuration.
  The same current executable bytes and system GTK accessibility bridge
  were retained. No global packages were replaced.
- Without Find, DevTools or synthesized accessibility actions, actual Tab and
  Return activated Fixture detail. Orca spoke the new article's heading and
  “Navigation inside the saved map works offline.” Real Back to map activation
  then spoke its heading and “This map is stored entirely in the local ZIM archive.”
- Actual Shortcuts activation opened the genuine Keyboard shortcuts HTML dialog.
  Orca announced the dialog and Close dialog button. Escape dismissed it and
  restored native focus to Shortcuts; heading/paragraph commands then spoke the
  map again. This is structural-reading evidence after modal dismissal, distinct
  from the older-stack focus-only run below.
- Real Ctrl+F and typing put the exact seven-character query `archive` in the
  native editable Find entry. After Return and Escape, heading/paragraph reading
  still reached the map. No spoken Match found claim is made.
- The actual reader process mapped the owned 2.62.1 library and typelib.
  Application, bridge, accessibility bus and registry identities were checked
  before and after the scenario. A native screenshot after modal/Find dismissal
  was viewed. MathCAT was disabled for this plain-text fixture; MathML, braille,
  GNOME/Wayland, suspend/resume and general release accessibility are not certified.
- Speech evidence is Orca's actual speech-dispatch output, not independently
  listened-to audio. The retained log includes BrlAPI-unavailable warnings and
  an invalid-source/script event; subsequent reading succeeded. This run does
  not establish structural-link K activation, Shift+Tab or complete
  Library/history/overflow acceptance.
- The guarded supervisor stopped the experimental app and reader and restored
  the original system reader environment and original-application focus.
  Original AT preferences and all seven named profile fields compared exactly;
  cleanup reported no errors.
- Evidence: `linux-current-orca51-runtime.json`, `orca51-debug.log`,
  `orca51-console.log` and the viewed `orca51-after-modal-and-find.png` in
  `/private/tmp/kiwix-edition-live-04g1iz06/`.
- This establishes a working tested stack, not a minimum supported version or
  an isolated causal fix. The fresh process/profile and reader defaults differ
  from the older comparisons; the earlier WebKit abort still has no established
  root cause. No production workaround or system-reader upgrade was installed.

### Older-stack counterexamples and inconclusive comparison

- With installed Orca 46.1 and AT-SPI 2.52, real keyboard input reached the
  Find entry but the typed query did not appear. Internal link navigation
  exposed fresh native article nodes, but heading/paragraph commands reported
  no more content.
- An isolated, unmodified Orca 49.8 fixed actual Find input, displayed
  Match found and highlighted the match. It read the article already loaded
  before the reader change, but still reported no more headings/paragraphs after
  actual link navigation.
  This is a counterexample to complete Orca acceptance, not an application fix.
- A further isolated comparison used the same Orca 49.8 with genuine AT-SPI
  2.62.1 client libraries, typelib and upstream Python identity override.
  Actual mapped libraries and the reader's version header were checked.
  The application executable, mapped GTK accessibility bridge, accessibility
  bus and registry identities remained unchanged. Find and the already-loaded
  map were readable; before link-navigation acceptance, WebKit child 38009 of
  application 37956 aborted with SIGABRT. The available Ubuntu crash report
  identifies WebKitGTK 2.52.6 but contains no backtrace/assertion message.
  Root cause is not established; this comparison is inconclusive.
- A read-only observer originally propagated that native disconnect because
  its generic RuntimeError branch also caught GLib.GError. An independently
  reviewed observer-only correction keeps traversal bounds fatal and records
  native errors explicitly. A fresh observation confirmed the missing WebKit
  recipient; partial trees were not treated as a pass, and admitted input was
  not replayed.
- Separately, actual keyboard input opened the reader's genuine HTML dialog,
  focused Close dialog, then Escape dismissed it and restored native focus to
  the invoking Shortcuts button. This establishes native modal focus lifecycle,
  not successful Orca structural reading after dismissal.
- Experimental readers and the current comparison app were stopped through
  their guarded supervisors. The original reader environment and focus were
  restored. Original AT preferences and named profile fields compared exactly:
  books, tabs, active tab, settings, bookmarks, reader storage and recent searches.
  Both cleanup receipts report no errors. No global AT packages were replaced
  and no production application change was made for these comparisons.
- Evidence in `/private/tmp/kiwix-edition-live-04g1iz06/`:
  `linux-current-orca46-runtime.json`, `linux-current-orca49-runtime.json`,
  `linux-current-orca49-atspi262-runtime.json`,
  `linux-current-native-html-modal.json`, the viewed
  `orca49-find-typed.png`, reader logs and `current-webkit.crash`.

## New native mobile builds on owned virtual devices — 2026-11-03

These runs do not install the new features on the user's physical phones.

- The rebuilt Android APK, SHA-256
  `437eb740a033615c33cf8e908f74b9772c15537597e2fdd76f4d1c425e8e212f`,
  replaced the app on the owned emulator without clearing its profile. Actual
  TalkBack gestures reached the detail paragraph and link, activated Back to map,
  then activated Fixture detail and reached the replacement paragraph/link without
  Find or a DevTools accessibility attachment. Closing the reader-tab panel with
  Escape also retained native reader navigation. This is not an HTML-modal,
  media/PiP, suspend/quit or cross-platform accessibility certification.
- Only after that accessibility run, real page input exercised Android bookmark
  consent, private-sync enablement and native invitation creation. Advertised
  emulator-NAT endpoints were not treated as cross-device reachability proof.
  The invitation was cancelled; sync and both data categories ended disabled,
  with no connected peers.
- Android traversed the real HTTPS Hawaii catalog and its linked pages. The
  unrelated installed fixture produced zero edition choices, not a replacement
  claim. The original catalog setting was restored. The extra tab opened during
  the UI run was closed, retaining the original tab and active-tab identity.
  Books, settings, bookmarks, reader storage and recent searches compared exactly
  against the actual on-disk baseline; navigation history intentionally changed.
  The owned raw CDP socket and forwarding rule were closed.
- Android evidence:
  `/private/tmp/kiwix-edition-live-04g1iz06/android-product-acceptance/runtime-receipt.json`
  and `native-features-receipt.json` in the same directory, including a viewed
  native screenshot. Subsequent CDP-assisted feature actions are not TalkBack proof.
- The rebuilt iOS simulator app preserved the owned archive, installation identity,
  tabs, bookmarks, history and reader storage. Managed paths rebased to its actual
  new sandbox; the archive hash remained unchanged. A real native UI test opened
  the fixture and activated its internal detail link successfully. This does not
  establish VoiceOver speech.
- The iOS native Settings UI then enabled bookmark consent, created and copied
  an invitation, and independently matched the desktop peer's public transcript
  code before confirming. With only iOS confirmed, neither profile had peer trust
  and the iOS reading list remained empty. Explicit counterpart confirmation
  enrolled both; a fresh authenticated Sync now delivered the actual note.
  The native Reading list displayed the exact note and Archive unavailable,
  without installing its archive. A native screenshot was viewed.
- Native iOS Disconnect device and its confirmation removed peer trust; global
  sync and bookmark consent were disabled through the actual UI. The desktop
  counterpart was also unpaired and disabled. These are owned simulator/desktop
  flows, not independent physical-phone, background or VoiceOver acceptance.
- Confirmed native removal returned the iOS reading list to its empty baseline.
  The owned extra detail tab was closed, restoring the exact original tab and
  selection. Books, tabs, active tab, settings, bookmarks, reader storage and
  recent searches matched the rebased pre-test profile; the archive hash was
  unchanged. Navigation history and private-sync identity/causal bookkeeping
  intentionally remain different; no whole-profile byte-equality claim is made.
- The exact owned simulator's clipboard was cleared through a UDID-guarded native
  helper, restoring its original empty value. The host pasteboard's complete
  item/type backup compared exactly. `simctl pbcopy` had exited successfully
  without changing the observed simulator clipboard; it is not cleanup proof.
- iOS receipt:
  `/private/tmp/kiwix-edition-live-04g1iz06/ios-native-private-runtime-receipt.json`.
  It names the successful native runs and distinguishes failed harness attempts.
  In particular, tab cleanup closed the owned tab successfully but its subsequent
  assertion incorrectly expected the automatically closed panel to remain open;
  final native disk state, not that failed XCTest, proves exact tab preservation.

## Host edition cutover and private reading sync — 2026-11-03

Scope: the matched-SDK macOS native executable and actual browser interfaces,
not new phone installation, packaged-mobile, assistive-technology or release proof.
The combined host suite passed **933 Rust tests, 16 ignored**; production frontend
and **180 JavaScript tests** passed. Independent native/UI reviewers approved their
scoped fixes; source review alone is not the runtime evidence below.

### Official full-edition replacement and rollback

- A real HTTPS traversal of `https://library.kiwix.org/catalog/v2/entries?lang=haw`
  found the corresponding official Hawaii series. Preparation required explicit
  association with the owned older edition; no title-only automatic lineage was
  granted. The original remained installed while a separate full archive downloaded.
- The actual official 22,783,565-byte archive verified SHA-256
  `315b99b48ca45570900347a68aa759b20f2a2809b461d9898173b4901e8c6e01`
  and reached Ready without replacing the original. A real subsequent catalog
  refresh was followed by retained-intent review/reauthorization in the UI.
- Explicit Replace assigned a fresh installation principal and reference revision 1,
  moved bookmark/live-tab/zoom references and rendered the new Wikipedia article.
  The owned original's bytes remained unchanged. Its website-storage marker was
  absent from the new reader origin.
- Explicit confirmed rollback verified the retained original, assigned another
  fresh principal and revision 2, preserved both archive files and moved reading
  references back. The original article rendered again. An unsaved title, Unicode
  note and tags survived the round trip and saved through the normal reader action;
  the same live tab survived. The former website-storage marker remained absent.
- Receipt: `/private/tmp/kiwix-edition-live-04g1iz06/runtime-receipt.json`.
  Browser screenshot capture timed out; DOM/ARIA values, actual frame content,
  native state and file hashes prove the named behavior, not a visual screenshot pass.

#### Preserved-original readmission after rollback — 2026-11-07

- The current release code reproduced a later lifecycle failure on an owned copy
  of the retained profile: successful rollback → remove library reference with
  `deleteFile:false` → import the same preserved original. Removal succeeded and
  kept the archive, but import returned no books and the historical-edition
  reservation error. This host reproduction used an owned ad-hoc-signed copy of
  the release app without App Sandbox; it does not establish packaged sandbox
  access to external files.
- Registration now distinguishes a completed rollback from an actionable
  transaction. Completed rollback history no longer reserves the original against
  ordinary import after removal. Intent destinations and transactions that can
  still be rolled back remain protected; acquisition overwrite reservations are
  unchanged. Intent, transaction and reading-reference history remain durable.
- The consumer regression extends the existing cutover/rollback transition
  through reference-only removal, ordinary import and restart. It checks a fresh
  installation principal, retained bookmarks/reading data, retired reader
  storage/sensor authority, repeat-rollback denial and actual reader HTTP access.
- Executed after the change: the extended regression passed, and the rebuilt
  release host passed the owned readmission smoke. The smoke re-admitted the
  preserved original as UUID `e6af831e-e0c6-4e4a-bad3-3e39904728e2` with fresh
  installation `c67294dc-bf15-4f6f-afad-80396fc4e5b1`, kept reference revision 2
  and the existing bookmark, served 84 characters of the actual retained article,
  rejected retired storage and sensor authority, denied a repeat rollback, and
  denied pending-intent and actionable-transaction imports. The retained source
  profile and all three archive hashes were unchanged. This is host logic, not
  packaged sandbox access or physical-device acceptance.

### Separately authenticated private reading data

- The reviewed real two-process smoke passed mutual Noise pairing, no trust or
  reading data after one-sided confirmation, encrypted convergence, concurrent
  edits/conflicts, deletion and explicit recreation, replay/restart, public-endpoint
  denial, revocation and private-mode cutover. A fresh-generation request while
  private was rejected separately from stale-generation rejection; normal exit
  used the fresh snapshot rather than weakening native guards.
- Two actual browser interfaces then enabled bookmark-category consent, generated
  and exchanged an invitation without logging its secret, compared the same
  transcript code and confirmed both screens. Sync now delivered a real note into
  the other Reading list. An uninstalled archive remained unavailable; no archive
  bytes or installation authority were created.
- Enabling automatic sync through Settings delivered a later edit from the other
  UI without another Sync now action. With automatic sync disabled, independent
  UI edits produced two visible causal versions. Cancelling the discard dialog
  kept both; explicit confirmed selection converged both peers and cleared conflicts.
- A separately consented saved position and nonempty reading history exercised
  confirmed Clear saved sync positions and reset connections. It removed the
  candidate and peer trust while keeping the bookmark and history. An unrelated
  real settings transaction did not refill candidates from unchanged history.
- Receipt: `/private/tmp/kiwix-edition-live-04g1iz06/private-ui-runtime-receipt.json`.
  These are real desktop network/UI flows, not proof of phone synchronization,
  forensic secret erasure, native screen-reader behavior or every failure boundary.

## Authorized phone-pair transfer and isolated intake — 2026-11-03

- The user explicitly authorized a fresh test, unlocked the iPhone and opened
  Kiwix. The already-installed reviewed builds were used; in-flight catalog,
  private-sync and reader-accessibility changes were not deployed.
- Fresh baselines were **Samsung: 12 books / 4 tabs; iPhone: 3 books / 1 tab**.
  These supersede older preservation counts only for this scenario; user-added
  content was preserved.
- Samsung Share all advertised 12 archives. The iPhone selected the exact
  Samsung publisher and received only Hawaii, UUID
  `c495a7aa-88f2-5dcc-a7bc-908af8a61cf2`. Its durable job recorded
  `transport: "quic-library"` and completed **150,605,015 bytes** from
  `2026-11-03T05:56:01.928941Z` to `05:56:07.115664Z`: **5.186723 seconds**,
  approximately **29.04 decimal MB/s effective admission-to-completion throughput**.
  This is not an active partial-byte rate; the finite sender sampler preceded
  admission and is excluded from throughput evidence.
- A streamed read of the actual iPhone file matched source SHA-256
  `d2fe53fc134651433c2b174ff8a683aeacf9529eea45111d4e599b0b9f95b18d`.
  The product's exact-job Delete download and files flow removed the owned
  received archive. Both phones' original books, tabs, bookmarks, history,
  settings and recent searches compared exactly; the iPhone's reader storage
  also compared exactly and its library returned to 3/1. The Samsung API
  snapshot does not expose reader storage, so it is not an exact-storage receipt.
  Samsung test sharing stopped; the owned debug forward, control tab, read-only
  evidence channels and physical UI runner were removed or closed.
- The first newly authorized UI run did not admit a download: its obscured
  Download tap typed an extra `o` into the keyboard. A corrected reviewed harness
  dismissed the keyboard and required exact `Hawaii` plus a hittable sole
  Download action. Its UI pass alone is not transfer proof; the durable receipt
  and streamed bytes above establish completion. The earlier lock/cancellation
  attempt likewise admitted no transfer.
- Historical 20+ MB/s versus roughly 10 MB/s remains the user's observation.
  This successful current run does not establish its cause, reverse-direction
  behavior, Bluetooth-only discovery or automatic/routerless network joining.
- On owned iOS 27.1 simulator `F8ADC13A-69B1-4F84-9792-74DBCC3A133A`, actual
  Add files selected a separate helper's original through Files. Coherent
  full-container checks proved a persistent native bookmark, `managed: false`,
  and no new app-managed archive/staging bytes. The original rendered after
  process restart; removing the source produced Locate, and explicit restoration
  plus Files reselection recovered both main and internal article reads.
  Removing the library reference kept the exact external original.
- Only Library actions → Import a managed copy created app-owned archive bytes;
  that copy and the untouched external original matched SHA-256. The frozen
  candidate omitted native-accessible internal links after restart and managed
  copying despite visible content. That is retained accessibility failure
  evidence, not full reader acceptance; the independently reviewed readiness
  repair still requires rebuilt native checks. These simulator results do not
  certify physical iPhone provider intake. Owned test history remained in the
  disposable simulator, not in either user phone.
- Evidence: `/private/tmp/kiwix-public-quic-ebjjemol/phone-pair-authorized-runtime-receipt.json`,
  `phone-hawaii-authorized-keyboard-fixed.xcresult`,
  `phone-hawaii-owned-cleanup.xcresult`, and fresh before/after snapshots;
  `/private/tmp/kiwix-reference-intake-pdpjn59m/simulator-evidence/intake-mode-receipt.json`
  links the original/retired/recovered container assertions and actual UI runs.

## Public QUIC and original-file intake verification — 2026-11-02

- The current source removes mandatory invitation/PIN setup from ordinary
  library sharing. Share all is first; browser/OPDS HTTP is separately enabled
  in Settings. Default mobile Add files/Add folder registers originals;
  managed copying is a separate action. Multi-selection/filtering and Download
  selected reuse the existing batch admission and cancellation path.
- Independent source reviews closed intake authority, duplicate handoff,
  native routing/discovery, exact-owner retirement, hotspot lifetime and Web
  subset-scope findings. The selected-download UI also passed bounded source
  review. These are not runtime or hardware certification.
- Final pre-dual-stack-fix host checks passed **835 Rust tests, 16 ignored**;
  the frontend built and passed **149 JavaScript tests**. The final native C++
  per-request privacy-context/group-listener fix passed all **3 native tests**.
  The earlier incoming-authority/duplicate-handoff iOS checks passed **37 tests**.
  The later dual-stack/on-link correction passed **837 Rust tests, 16 ignored**,
  both mobile builds and strict iOS signature verification.
- Actual two-process native Bonjour/QUIC Browse initially failed. The debugger
  showed a live connection but invalid UTF-8 interface text: a borrowed name
  outlived its enumeration callback. Copying the name inside the callback fixed
  actual Browse without weakening route authority. Two downloaded fixtures
  subsequently imported and matched their source SHA-256; the delimiter-path
  archive opened in the real reader.
- **Pre-fix evidence:** two fresh bulk runs completed their first job but timed
  out before the second received bytes; re-browse recovered that job. The isolated
  native regression also exposed a single-accept harness that missed multiplexed
  request streams. Those failures are retained, not current unresolved host results.
  The final native fix uses per-request privacy context and a group listener.
- Actual final host acceptance completed two concurrent selected QUIC archive
  downloads with exact source SHA-256 and imported only the selected archives.
  Download all then skipped those two installed archives and added the remaining
  archive. Downloaded reader Main and internal detail pages opened. Same-name
  publisher codes were visible and accessible at 390px. Native v1 Bonjour inspect
  and legacy Quinn numeric download also passed with exact hash.
- Both reviewed final packages were installed in place and launched. Samsung
  retained all eleven books, three tabs and settings; iPhone retained three books,
  one tab, bookmarks and history, with only app-container paths correctly rebased.
  Nearby XCUITest was initially lock-blocked; after unlock,
  `testCurrentNearbySurface` passed **1 test, 0 failures (5.668s)**, checking
  Share all above Receive and removal of the technical section. It did not tap
  Share all or establish phone connection/transfer. Evidence: receipt `24509`,
  `/private/tmp/kiwix-public-quic-ebjjemol/current-nearby-unlocked.xcresult`.
- **Fixed physical failure:** Samsung Share all initially reported “The sender
  supplied invalid public Nearby connection information”. Its reachable IPv4 plus
  ULA IPv6 hints were rejected by the old IPv4-only public validator. The fix
  accepts numeric ULA hints and filters supported publication hints to 16, still
  rejecting global/remote, mapped, scoped and multicast hints. Independent review
  also caught private-address classification bypassing native prefix checks;
  numeric library admission and retained receipts now require actual on-link proof.
  Corrected packages were installed in place. Samsung cold-launched, retained its
  eleven books/three tabs/settings (consumed-acquisition set order alone changed),
  and actual Share all reached running/advertising. The current Mac receiver
  browsed all eleven archives and completed/imported Hawaii with source-matching
  SHA-256. iPhone's prelaunch state remained exact; iOS blocked new launch while
  locked, and the subsequent test runner reported authentication canceled.
- Two controlled Samsung-to-Mac Hawaii transfers measured **16.86 and 18.61 MB/s**
  over active partial-byte samples; each received 150,605,015 bytes with SHA-256
  `d2fe53fc134651433c2b174ff8a683aeacf9529eea45111d4e599b0b9f95b18d`.
  These are not phone-pair measurements or end-to-end import rates. Radio snapshots
  showed 5 GHz infrastructure Wi-Fi, RSSI −72 then −69 dBm and Tx links 228 then
  258 Mbps; adjacency does not establish a direct peer-to-peer route.
- Same-LAN Mac↔Android and Android/Mac↔iPhone discovery, shared-catalog browsing
  and download were later physically verified (Mac→Android StreetZim 36,196 bytes;
  Mac→iPhone 36.2 kB; Android→iPhone 6 archives/362.5 MB). Phone-to-phone
  throughput, both-direction/routerless/AP-only behavior, original-file intake
  and OS permission/join acceptance remain unresolved. The user's earlier
  20+ MB/s versus later roughly 10 MB/s observation is retained; historical
  active-byte/route evidence is unavailable. An earlier locked/authentication-
  canceled phone test admitted no download; no OS authentication retry or bypass
  follows.
  Evidence: `samsung-mac-hawaii-controlled-runs.json`, `dual-stack-*-before.json`,
  `dual-stack-samsung-after.json`, `dual-stack-iphone-after-install.json`, and
  `phone-hawaii-first.xcresult` under the owned evidence root below.
- A subsequent real QUIC acquisition exposed a hardcoded `transport: "http"`
  status label. The reviewed correction durably records new `quic-library`
  provenance; older synthetic library rows remain `unknown` because that URL
  also identified the former HTTP path. Owned job/provenance snapshots remove
  a review-found concurrent-forget lookup race. Final checks passed **839 Rust
  tests, 16 ignored**, **154 JavaScript tests**, desktop/Android/iOS builds and
  strict iOS signatures. Actual new admission, UI Pause at 43,067,328 bytes,
  UI Resume, completion/import, source SHA-256 and process-restart provenance
  passed. Host screenshots timed out, but DOM/actions passed and the final
  physical Samsung Downloads/Nearby surfaces were visually checked.
- Both provenance-corrected packages were installed in place. Samsung
  cold-launched in 1,130 ms, retained eleven books/three tabs/settings, displayed
  both unproven historical library rows as `unknown`, and exercised Share all
  advertising eleven archives before Stop all withdrew sharing/discovery.
  iPhone's exact prelaunch state retained three books/one tab; no launch or
  authentication retry was attempted after the prior cancellation. Owned host
  archive copies, the temporary UI runner and test publishers were removed or
  stopped. Evidence: `provenance-runtime-receipt.json` and
  `provenance-deployment-receipt.json` under the same owned evidence root.
- Actual Web OPDS root and All books feeds returned HTTP 200 and both expected
  titles. Stopping Web sharing preserved the running QUIC library. A separate
  primary-button exercise preserved configured selected-archive Web scope.
- Evidence: `/private/tmp/kiwix-public-quic-ebjjemol`, host build/check receipt
  `24341`, selected-UI/check receipt `24379`, iOS native `24316`, mobile builds
  `24361`/`24362`, and pre-fix failed native scenarios `24371`/`24378`.
  Later final-runtime and in-place-preservation observations above supersede
  those earlier package and pre-update snapshot boundaries only for their scope.


## New iOS identity and signed physical installation — 2026-11-02

- Built the reviewed iOS arm64 application as `com.tiltastech.zimreader`, with
  widget `com.tiltastech.zimreader.DownloadActivity`; shared desktop/Android
  identity remains unchanged. Final iOS frontend entry
  `assets/index-CwCUVe-g.js` is present in the signed executable.
- Apple issued an explicit development profile authorizing the connected iPhone,
  the actual local development certificate and both Hotspot Configuration and
  Access Wi-Fi Information. CMS signature-math verification passed; no separate
  certificate-chain/revocation audit is claimed. Both Wi-Fi entitlements are
  boolean true in the actual signed app. The widget requests neither privilege
  and uses an authorized wildcard profile.
- Strict main/widget signature verification passed. Independent source,
  generator and signing-artifact reviews found no remaining signing blockers.
  The signed app installed and launched on the physical iPhone; a native XCUI
  launch smoke passed **1 test, 0 failures**, observing Library, Settings and
  Nearby controls and retaining a launch screenshot.
- This was a fresh installation after the user deleted the old app, not an
  in-place migration or a claim that its archive files were backed up.
- Signed package: `src-tauri/gen/apple/build/arm64/ZimReader.ipa`,
  SHA-256 `cde05aba22395711bd1037274aa46c8a8616cbe10a854203e4817a3e360ad66c`.
  Evidence: `/private/tmp/kiwix-latest-phone-wp70__5t`,
  `zimreader-launch-final.xcresult`; build/launch receipts `24176`, `24219`.
  This resolves the signing prerequisite below, not physical BLE, automatic
  joining, route-loss or complete accessibility acceptance.

## Authenticated Nearby integration — 2026-11-02

- Independently reviewed Noise/invitation, native Android/iOS, pinned-TLS,
  orchestration and UI contracts. Repairs cover strict nested decoding, joined
  versus ordinary-route receipts, permission aliases, bounded native cleanup,
  publisher offer deadlines/startup drain, stale BLE candidate events,
  joined-lease PIN retries, and receipt-independent exact-route refresh.
- Final reviewed host suite: **843 passed, 11 ignored**; JavaScript: **148 passed**.
  Production and both mobile frontends built. Native iOS package compiled and
  passed 26 simulator tests; Android native unit task passed. Android arm64 APK
  and unsigned iOS arm64 archive built. Those native suites do not establish BLE
  hardware behavior.
- Actual compiled host, isolated owned profile: desktop pairing correctly
  refused unsupported native admission. At 390px the Nearby surface showed
  separate authenticated/legacy explanations, no horizontal overflow and no
  browser errors. A loopback share served TLS 1.3 with a validated owned leaf,
  returned one genuine inventory book and a secure download identity, streamed
  its exact 35,708-byte ZIM/SHA-256, rejected a TLS catalog route, and closed its
  TLS listener after Share Stop. No BLE or native-join success is inferred.
- Host evidence: `/private/tmp/kiwix-nearby-smoke.Ny8euS`; native iOS result:
  `/private/tmp/kiwix-nearby-native-check.xcresult`. Checks are recorded in session
  artifacts `23732`, `23737`, `23738`, `23750`, `23757`, `23758`.
- Signed iOS build failed before acceptance: Xcode lacks usable account
  credentials and the existing profile omits Hotspot Configuration/Access Wi-Fi
  Information. No new pairing package was installed on either phone. Physical
  phone-pair BLE, automatic join, prompt/cancel/loss and camera-URL provenance
  remain acceptance gates; earlier installed naming/Download-all builds retain
  their own receipts below. No release or overall parity certification.

## Download all and current phone installations — 2026-11-02

- The current JavaScript suite passed 147 tests, including the production batch
  helper's mixed inventory, exact-session URI matching, partial-error,
  cancellation/late-acceptance and immutable-progress cases. Production, iOS and
  Android frontends, the desktop executable and both arm64 packages built.
  Independent UI and helper/authority reviews found no findings; generated
  English copy/provenance was separately reviewed. No Rust or native
  Swift/Kotlin logic changed. The earlier 822 Rust / 26 iOS / 27 Android test
  results remain their own snapshots, not newly rerun suites.
- Actual host processes advertised and browsed a three-ZIM library through
  DNS-SD. Download all skipped one installed archive, downloaded/imported the
  other two, and both SHA-256 values matched the sender. A received delimiter
  article rendered `Question / literal question mark`; browser errors were empty.
  Actual 390px screens had no horizontal overflow.
- A separate owned receiver admitted one archive, then refused an existing
  destination and displayed one accepted / two unchecked, without claiming
  completion. Holding another real admission request, clicking Stop adding,
  then releasing it proved that its late accepted job remained tracked/imported
  while the final archive was never admitted. Neither receiver enabled sharing.
- Samsung SM-A356U received APK SHA-256
  `04c8e9447f22ce9d679c5a693839bcad4e052256367abe9e069b1164d5cac927`
  in place and cold-launched in 1,594 ms. Actual Nearby/Browse/Download all
  admitted, completed and imported all three test archives; all three on-device
  SHA-256 values matched the sender. Native screenshots were inspected.
  Confirmed per-job cleanup removed only those acquisitions and payloads.
  All original ten books, two tabs, bookmarks, reader-storage values and existing
  settings matched the baseline; the new empty `sharingName` default was added.
  Object-key ordering was not treated as a reader-data change. Discovery started
  for this check and its inspection forward/adoption were subsequently stopped.
- The iPhone 17 Pro Max test device received the current iPhone app in place after strict/deep signature
  verification; IPA SHA-256
  `01351e3e388eca4702d0ebe2cbfe20ef47b188dc6dffe1b419719bfd206153d0`.
  Its complete persisted state, including sixteen books, matched the pre-update
  baseline before first launch. iOS refused launch because the device was locked;
  current-build physical iPhone UI/Download-all acceptance awaits unlock.
  Earlier physical iPhone background/media receipts are not substituted for it.
- Evidence: `/private/tmp/kiwix-download-all.uERXBu/`, including
  `host-acceptance.json`, `deployment.json`, physical screenshots, payload hashes
  and `samsung-preservation.json`. New batch strings use English fallback where
  translations are missing. Bluetooth bootstrap, automatic hotspot joining and
  lock-screen audibility/headset acceptance are not certified by this batch.

## Nearby publisher names — 2026-11-02

- Full host suite: 822 Rust tests passed, 11 ignored. The opt-in real DNS-SD
  restart/withdrawal regression separately passed, including a rename and two
  publishers with the same nickname. The final JavaScript suite passed 134
  tests; production/target-specific frontends and arm64 iOS/Android packages
  built. This batch adds no native Swift/Kotlin implementation changes and does
  not constitute fresh physical radio or native-layer-suite acceptance.
- Actual isolated application processes advertised Mac by default. A saved
  Unicode nickname did not change an active announcement; Stop/Start rotated
  its session under the same publisher ID. Two equal-nickname senders remained
  separate, and a protected QUIC offer exposed its publisher hint while keeping
  the archive title hidden.
- Actual browser interactions exercised distinct accessible Browse labels,
  narrow-screen layout, invalid multibyte-name rejection, save completion after
  navigation and preservation of a newer unsubmitted draft during a delayed
  save. Holding the actual save request proved Share all waited; another real
  receiver observed the committed name. Stop all during that wait sent no
  publication-start request and left sharing off.
- Real filesystem fault injection changed only an owned temporary profile to
  non-writable. Before the final repair, successful retry left a stale copied
  inline error. After the repair, the retained failure was shown with Retry
  saving, retry persisted the name, and the obsolete permission error vanished.
  No user profile or phone library was used by these host checks.
- Independent Rust and frontend reviews closed all scoped findings, including
  invalid-input queue poisoning, retry ownership, stale root/draft state,
  accessible duplicate labels and stale failure presentation. New UI copy is
  English fallback outside English; translation completion remains separate.
- Host evidence: `/private/tmp/kiwix-publisher-names.oi3YNX/`. Device deployment
  and physical pairing are not implied by the package builds above. Bluetooth
  bootstrap and automatic hotspot joining remain separate roadmap work; the
  reference credential characteristic is not an authenticated protocol.


## Final inline-video recovery and PiP — 2026-11-02

- The earlier physical inline TED reset, 6 seconds → Home → zero, was a real
  failure despite passing helper checks. The correction preserves only bounded
  ephemeral position data for the current tab. A replacement reader must
  authenticate afresh and match policy, installation/source, article and
  descendant occurrence before a paused seek. It neither preserves a retired
  capability nor automatically starts playback.
- Final IPA SHA-256:
  `183016ae00036068a209dc286f9af4082dba59a161ea868b50c07c7e321b1132`.
  Strict/deep signature verification, in-place installation and launch passed.
  Actual TED flow (`bg_208`, `artifact://23484`): 6 seconds playing → Home for
  8 seconds → return paused at 8 seconds with Play Video visible → explicit
  Play Video → 9 seconds playing. Paused and resumed screenshots were inspected.
- The same IPA separately passed actual native PiP (`bg_211`,
  `artifact://23493`): 6:02 → Home for 8 seconds → return at 6:14. The Home
  floating video and foreground PiP placeholder were visually inspected.
  A first harness attempt encountered auto-hidden Fullscreen controls before
  entering PiP; it is excluded from passing evidence. Revealing the actual
  controls fixed that harness setup without a product edit.
- Both independent reader/native reviewers approved the final data-only
  protocol and closed the exercised inline-reset scenario after inspecting
  receipts. Neither source review nor advancing playheads certifies audibility,
  lock-screen/headset interruptions, arbitrary players or all interleavings.
- Final Android APK SHA-256:
  `dde90786f494d4234cb28c0e1d12562ba87cf57cd41d5422ef64972c8c1a45d3`.
  In-place installation and cold launch passed in 1,390 ms. Opening the existing
  Hawaii archive rendered the actual map with Honolulu, controls and attribution;
  browser errors were empty. The phone was returned to Library and the borrowed
  browser adoption and owned inspection forward were released.
- The rebuilt isolated desktop reader rendered the delimiter fixture's Main
  page with no browser errors. Compositor screenshot attempts timed out; desktop
  screenshot proof is not claimed. Its owned tab and runtime were stopped.
- Physical media artifacts remain under
  `/private/tmp/kiwix-reliability-ios.8np9dW/`:
  `final-inline-data-restoration.xcresult`,
  `final-webkit-pip-visible-controls.xcresult` and their exported screenshots.
  The exercised PiP was closed through SpringBoard's observed Close Picture in
  Picture control (`bg_213`); the final screenshot shows paused inline video,
  not a leftover floating playback session.
- The existing persistent, unmanaged iPhone Taipei provider archive also rendered
  its actual map (`bg_214`, `artifact://23508`); its provider reference and
  installation identity stayed unchanged. This is existing-provider access, not
  cloud-eviction, nonseekable-stream or permission-revocation certification.
- Final preservation receipts found all sixteen iPhone and ten Android original
  book records, bookmarks, settings and existing reader-storage entries preserved.
  Only the exact OS sandbox prefix was rebased on iPhone. Normal reading changed
  navigation/tab state; exact tab-array preservation is not claimed.
  The owned physical UI runner was uninstalled after acceptance.

## Large-file verification performance — 2026-11-01

- Actual native debug application, owned 134,253,370-byte ZIM: a five-second
  process sample put 3,238 samples under SHA-256 compression during cleanup.
  RustCrypto 0.10's aarch64 backend requires its optional `asm` feature; without
  it, this Apple Silicon build used the software compressor.
- Enable that runtime-checked ARM backend (software fallback remains available)
  and optimize only the SHA-2 dependency in debug builds. No validation/removal
  logic or checksum coverage changed.

| Actual native operation | Before | After |
|---|---:|---:|
| Prepare confirmed download cleanup | 5.468 s | 0.069 s |
| Remove installed downloaded archive | 12.568 s | 0.580 s |

- Both scenarios downloaded/imported the real generated ZIM, removed only that
  owned installation/payload, and actually read the unrelated Main archive
  afterward. Its SHA-256 stayed
  `ccfc653cedc73507328a9bcf3360ad15f3cbca6379fc31fbe7da928db8334e1a`.
  Timings are debug-host measurements, not physical-phone speed guarantees
  (`bg_118`, `bg_121`). The first profiling attempt used inconsistent /var versus
  /private/var harness paths and correctly failed ownership admission; it is not
  counted as a successful cleanup.
- Rebuilt host plus full Rust suite: 801 passed, ten ignored (`bg_120`, `bg_122`).
- Actual accelerated-binary HTTP, LAN LocalSwarm and confined real BitTorrent
  recovery also passed (`bg_126`): pause/restart, transfer SIGKILL, finalization
  SIGKILL, exact received SHA-256, reading before/after restart and confirmed
  cleanup preserving the unrelated installed archive.
- First-start torrent caveat remains: one earlier attempt transferred no bytes
  for 180 s without a captured connection/handshake error. Fourteen subsequent
  fresh starts passed, including three peer-wire traces with real payload within
  0.282–0.294 s; the accelerated final run also passed. This does not establish
  the original stall's root cause. No speculative retry/transport change made.

## Physical Android acquisition and SD acceptance — 2026-11-01

- Accelerated arm64 packages built (`bg_123`, `bg_124`) and installed in place on
  both phones (`bg_127`, `bg_128`). Android cold launch passed in 1,315 ms.
  APK: `8bcd307e6259f4e8c03e54d5b3b7ec442657fe000db601f07160179db358336a`;
  IPA: `582d8052476cc987e6e529efb16f9b191b8a148259864ae1a798d47d90c9319b`.
  Subsequent unlocked iPhone background-completion acceptance is recorded below.
- Physical Samsung: actual official catalog search → download → pause/resume →
  Completed → Open rendered Pixel Gun 3D Wiki. The 70,784,282-byte file matched
  its official Metalink SHA-256
  `fe0cb57cede4bb3869d52255d3128c5ff59ba594ae9bf3aaf20c79148f54d4f8`.
  Exact Metalink size superseded the rounded catalog estimate. Confirmed native
  UI deletion completed in 613 ms, removing only this test acquisition.
- A separate confined USB-loopback HTTP run paused at 24,117,248 durable bytes;
  cold restart retained that exact checkpoint. Later app-process SIGKILL at
  37,486,592 durable bytes restored exactly those bytes as interrupted/paused,
  respecting the user's disabled automatic-resume preference. Explicit resume
  used nonzero Range offsets, completed/imported/read and matched the same full
  SHA-256. Confirmed cleanup removed that owned acquisition too. This is physical
  transport/process recovery, not radio-loss or iOS background certification.
- Physical SD volume `3478-CD05`: native DocumentsUI opened the genuine archive
  in place without a managed copy and reopened after cold restart. Revoking only
  its actual persisted URI grant made only that archive unavailable; native
  Locate/reselection/regrant restored reading. A fresh installation deliberately
  cleared that archive's page-local storage rather than transferring cookies or
  storage across an unverified replacement principal.
- Native folder picker also admitted the genuine archive split as
  `PG3D.ZIMAA/PG3D.ZIMAB` on this SD card; actual reading and cold reopen passed.
  Both source-part SHA-256 values remained unchanged before source-preserving
  reference removal and cleanup of the owned directory.
- Final physical library matched the original ten book records, original tabs,
  bookmarks and settings exactly; original reader-storage entries and key set
  matched. No test job/reference/payload remained. Browser errors were empty.
  Owned source server, USB reverse/inspection forwards and adoption stopped.
  No original-file deletion or QA fixture import; only genuine official archive
  copies in explicitly owned paths were exercised.
- With the user's approval, a subsequent owned SD archive was opened, the card
  unmounted, and the app cold-started. A separate remount watchdog and a
  `finally` remount restored the volume. After remount, Reload article restored
  the correct H1 under the same provider reference and installation, without
  Locate/regrant. The source hash and all ten original books, tabs, bookmarks,
  settings and reader-storage entries were unchanged after owned cleanup.
- That first removal scenario exposed a real presentation defect: an opaque
  native exception appeared globally and a stale saved-item warning insisted on
  Locate even after reading recovered. Repaired APK `e50c6e7…` passed the physical
  Samsung repeat: unavailable storage produced plain, reader-local reconnect/
  reload guidance; remount and Reload restored the same reference/installation,
  and the transient notice disappeared automatically. All ten original books,
  tabs, bookmarks, settings and reader-storage entries matched after owned cleanup.
- Physical reboot, nonseekable/cloud-provider fallback, lock-screen/headset
  interruption and cellular-policy cutovers remain open acceptance gates.
  The separate physical iPhone PiP result is recorded in the current audit;
  it does not certify inline-video position retention or audibility.

## Physical iPhone background completion — 2026-11-01

- After the user unlocked the phone, the installed IPA above performed an actual
  Discover search and official Pixel Gun 3D download, then spent 25 seconds on
  Home. A native UI screenshot showed advancing bytes (2.7 MB) but `0 B/s`;
  independent review traced missing native rate/ETA accounting. This does not
  establish transport-stall causality or a throughput comparison.
- The native lease was still running when CoreDevice delivered SIGKILL to app
  PID 68394 (`bg_145`). Before any subsequent manual activation, process
  inspection found the same installed app relaunched as PID 68580; its durable
  job was complete and the native completion lease retired.
- The received 70,784,282-byte archive matched official SHA-256
  `fe0cb57cede4bb3869d52255d3128c5ff59ba594ae9bf3aaf20c79148f54d4f8`
  (`bg_147`). Actual Downloads → Open rendered Pixel Gun 3D Wiki (`bg_146`);
  the screenshot was inspected. All sixteen original book records remained
  unchanged. This original acquisition was subsequently removed through the
  actual confirmation UI (`bg_159`, `artifact://23235`): job
  `71c7b71a-272b-469d-a1b2-73aba5ea068e` and its archive were absent afterward.
  This cleanup is separate from the later reviewed-rate acquisition below.
- Evidence: `/private/tmp/kiwix-reliability-ios.8np9dW/`,
  `iphone-native-before-kill.json`, `iphone-processes-after-loss.json`,
  `iphone-downloads-after-os-relaunch.json`, and
  `official-os-completed-read.xcresult`. This proves one official background
  completion/process-loss recovery, not low-space, cellular, durability-fault,
  force-quit, or arbitrary OS-scheduling certification.

### Reviewed native rate display and owned cleanup — 2026-11-02

- Reviewed IPA `3e393fe…`: the actual download UI showed 49 kB/s and remaining
  time as bytes advanced. This verifies rate/ETA accounting, not a throughput
  improvement. Slow observed iOS background throughput remains unexplained;
  differently configured/timed foreground and background probes do not establish
  the production cause or justify a transport-policy change.
- Owned job `97ac0a2c-0ea8-4b10-ac4e-a8297121468f` completed. Its actual downloaded
  `pg3d_en_all.zim` matched the official SHA-256 above, and Downloads → Open
  rendered Pixel Gun 3D Wiki. Confirmation UI cleanup passed afterward (`bg_197`);
  this later job was absent from the durable ledger.
- `reviewed-native-cleanup-preservation.json` records the original sixteen books,
  installation IDs/provider references, bookmarks, settings and existing reader
  storage preserved. The comparison rebases only the exact old/new OS sandbox
  prefixes in app-owned archive paths and `downloadDirectory`; no other field is
  ignored. Opening articles adds navigation history/tabs, so exact tab-array
  equality is not claimed. Receipts, copied archive hash and actual surface
  screenshots remain under `/private/tmp/kiwix-reliability-ios.8np9dW/`.


## Android external-provider split correction — 2026-11-01

- Actual native folder picker rejected `COHORT.ZIMAA/COHORT.ZIMAB` before the
  correction while the same bytes in lowercase read correctly. Folder admission
  now orders and validates suffixes case-insensitively, preserving exact provider
  names and URIs. Gaps, duplicates and mixed archive bases remain invalid.
- Full native Android plugin unit task passed (`bg_109`), including the cohort
  regression. Repaired APK built (`bg_110`), installed and hash-verified in an
  owned API-36 emulator:
  `5cc195ac54dbda42e56d99482c2327925934e338b4a85022a7b0d78233b381b5`.
- Actual DocumentsUI uppercase and deliberately reversed exact-case mixed
  cohorts both read StreetZim test map and reopened after cold restart. Invalid
  gap/other-base selections left the complete installed-books array unchanged.
- Missing-source and real persisted-URI revocation both made only the affected
  external archive unavailable. Native Locate/reselection/regrant restored
  Question. The managed installation/path/hash, bookmark and actual article
  localStorage persisted; no reference or grant registry was forged.
- Physical SD open/restart/regrant, uppercase splits and approved unmount/remount
  recovery are recorded above, including the repaired transient-error repeat.
  Reboot and nonseekable/cloud-copy fallback remain gates. Owned emulator,
  forward and browser adoption were stopped; no offers were published.


## Physical iPhone export path repair — 2026-11-01

- Archive Share rejected app-owned files with `outside_app_storage`. A disposable
  physical-iPhone runner reproduced the failure using the actual native
  `prepareExport`; the equivalent simulator test passed before the repair.
- Foundation normalized existing `/private/var` URLs to `/var`, but kept the
  physical spelling for the not-yet-created scratch child. Root admission now
  compares lexical components against OS-owned roots and their POSIX `realpath`
  spellings. Only trusted roots are resolved; descendant symlinks remain rejected.
- Physical before/after: the owned export failed before (`bg_95`) and passed
  after (`bg_96`). Both permanent regressions also passed on the phone: exact
  archive-copy bytes with scratch-only cleanup, and rejection of descendant/
  final symlinks and a foreign path while preserving the source. The complete
  simulator native suite passed 24 tests (`bg_98`).
- Rust no longer appends “Prepared export retained” when the operation is sharing
  an original archive and has not created a Rust-owned prepared export.
- Repaired arm64 iPhone package built (`bg_97`), passed strict/deep signature
  verification and installed in place (`bg_99`); IPA SHA-256
  `f8c68dd31b8abf4e061a20e691a4c758736b75e68370be93e11c33daa0343fcb`.
- Actual installed app: Downloads → Share opened the native activity sheet for
  `osm_austria-czech_wiki-sk.zim`, identified as an OpenZIM Content File, 143.1 MB.
  Corrected native UI acceptance passed in 11.456 s (`bg_102`), including dismissing
  the sheet and restoring the enabled Share control without a new error band.
  The first UI assertion searched for a button instead of iOS's Copy cell; that
  timed out despite the sheet being present, and is not counted as a passing run.
- Read-only copies of the original archive before and after sharing/cancellation
  had identical SHA-256
  `bf2f93ca677e884867435d4fc7761a3e56084db8371dc730780ab1144e3f5016`
  (`bg_104`). Post-dismissal tmp enumeration found no `kiwix-export-*` scratch
  directories (`bg_105`). No physical archive import, deletion or library reset.
  Recipient transfer/AirDrop completion is not certified by opening this sheet.

## Repeated library listings — 2026-11-01

- The user's screenshot included the owned Main-path fixture offer and an owned
  test library that had remained live during evidence recording. Both were
  immediately withdrawn. A fresh receive-only scan then found one distinct
  library session. The image alone does not identify every previous row.
- Source review found random DNS-SD library names on every sharing start. A cached
  older record can therefore coexist with its replacement. Library discovery now
  uses a session-specific instance (`Kiwix-Library-<publisher>-<session-prefix>`)
  with an optional generation hint; the Nearby list keeps the newest session
  first and exposes older sessions under a disclosure rather than hiding the
  real sender. Ports/session IDs/capabilities still rotate. Private ownership
  handles rotate per announcement, preventing delayed withdrawal or cancelled
  admission from deleting a newer announcement.
- Full host suite passed 801, ten ignored; the opt-in real DNS-SD regression
  separately passed in 3.20 s (`bg_81`). It exercises a new discovery row across
  session rotation, stale withdrawal, distinct publishers, fresh browsing and final
  withdrawal. The default suite does not run this network-publishing regression.
- Actual native application server/common UI: Share all → one publisher row →
  Stop/Share all → a new row for the new session → Stop → row removed; browser
  errors empty. Both test shares were withdrawn in the run.
  Distinct devices can still have the generic Shared library label; equal labels
  are not proof of duplicate records and are never used to hide other publishers.
- Android/iOS packages built (`bg_82`). Before the subsequent Android update, its
  actual running library share was stopped gracefully; the app still had ten
  books. No archive deletion/reset accompanied this discovery repair.
- Discovery correction installed in place on both phones: Android install/cold
  launch passed in 1,126 ms (`bg_83`); iPhone strict/deep signature verification
  and install passed, but automatic launch was denied by normal Locked policy
  (`bg_84`). No unlock bypass/retry. APK SHA-256:
  `19ee432b9cbb18f2af2046693fe93dd47cdfe2351f8186f944c9490d311d2761`;
  IPA: `c084d09d19fa92b4c2a35b9be566baa924abb67272debbb29bda65e70d39d3fa`.
  Final inspection found no remaining test offers or library publishers; Android
  sharing remains off after the update until the user explicitly shares again.

## Duplicate preflight and download cleanup — 2026-11-01

- Physical Android read-only inspection found a failed received Hawaii duplicate
  alongside an available installed copy. Later snapshot/download queries returned
  promptly: the latest duplicate row was absent and the original installation
  identity/path remained available. No repeat delete, reset or diagnostic restart
  was issued. The earlier delay has no captured stage trace; no deadlock cause
  is claimed.
- Two disposable native hosts reproduced the old behavior: a complete 35,708-byte
  copy, followed by duplicate-UUID import failure. After repair, selected-file
  inspection returned the verified ZIM UUID, the UI offered In library → Open,
  and actual reading rendered H1 Main. Native QUIC and whole-library HTTP start
  calls independently rejected duplicates without adding jobs. The existing
  installation/path stayed unchanged. Final import collision guards are retained.
- Header probing spans split parts and is bounded to 30 seconds; an empty library
  needs no identity payload probe. An initial verification run exposed unnecessary
  first-chunk reads delaying a deliberately throttled empty-library start and
  changing reconnect behavior. Skipping that unnecessary probe repaired those
  regressions; the final full suite passed 801, nine ignored (`bg_75`).
- Cleanup skips a redundant full verification when preparation has no installed
  removals to authorize. Final deletion still checks reservations and ownership.
  HTTP cleanup retains the complete durable SHA-256 check after private capture;
  growing-source reads stop at admitted length plus one byte. LocalSwarm checks
  original admission cancellation between chunks, not after an unbounded scan.
  Cancellation does not bypass the mobile transfer gate, and protected installed
  removals retain their reader/source/sharing guards.
- Actual native-host UI deleted only the disposable failed duplicate; busy
  feedback appeared, the publication disappeared, and the installed original
  retained its identity/path and SHA-256
  `ccfc653cedc73507328a9bcf3360ad15f3cbca6379fc31fbe7da928db8334e1a`.
- Final Android package, in an owned isolated emulator: native DocumentsUI imported
  Main-path fixture; Nearby discovered the real host offer, showed In library
  before creating a job, and Open rendered Main. Downloads stayed empty. Library
  inventory likewise offered Open instead of Download. A distinct delimiter
  archive then downloaded/imported/read (H1 Question); confirmed cleanup removed
  that archive and its download while Main-path fixture remained readable.
  Small-fixture cleanup was observed in 165 ms, not a large-file speed guarantee.
  Image: `/tmp/kiwix-duplicate-preflight-android.png`; native browser errors were empty.
- Frontend and target-bound Android/iOS assets built; final Android APK and arm64
  iOS IPA built (`bg_76`). Focused download-state/error-visibility JavaScript tests
  passed 21 (`bg_70`). Ownership regressions include replaced files, same-inode
  tampering, safe retry, unavailable existing entries, split-header identity and
  installed-original preservation. These are not physical-phone deletion or
  physical iOS background/route acceptance.

- Final repair packages installed in place and launched on both physical phones:
  Samsung `<samsung-serial>` cold launch 1,232 ms (`bg_78`); the iPhone 17 Pro Max test device strict/deep
  signature verification, install and launch passed (`bg_79`). No uninstall,
  reset, permission changes or QA fixture imports into either physical library.
  APK SHA-256: `5ef7fe7bdbfe793f61fb1f5d9dd464cc8c637d9fe677c837edb084d077f5f9a3`.
  IPA SHA-256: `857584d5bc240413349c6aa1080857e6755f6066761c136aa595351abeb79c62`.
  Installation/launch does not certify physical-phone deletion or a phone-pair run.

## Nearby intent and Android log review — 2026-11-01

- Reproduced physical Android Find nearby refusal while Wi-Fi was off. Admission
  failed before multicast/mDNS startup. Native initial `known=false` could persist
  without any network callback; observed no-network startup now initializes as
  offline, without granting a route. Local admission gives Wi-Fi/hotspot guidance,
  not a generic instruction to retry a download.
- The inspected Android capture contained no fatal exception or Rust panic.
  Chromium variations-seed/DNS initialization, codec capability enumeration and
  missing WebView Bluetooth media permission messages are not proof of a sharing
  transport failure. The suspended search retry and automatic-resume-disabled
  notices are expected policy decisions. Download history also contained
  duplicate-import, timeout and checksum failures; they were not established as
  new failures of the currently browsed library.
- A genuine lifecycle warning remains: `RustWebView.evalScript` posted work to an
  already-destroyed WebView during Activity recreation at 15:29. Source review
  also found Wry's `onEval` retaining completed callbacks with `get`, not `remove`.
  These are recorded review findings, not claimed repaired by the Nearby UX work.
- Actual isolated native Android exercised cold radio-off entry with actionable
  text, Wi-Fi restoration and explicit re-entry, Stop finding, fresh entry,
  private-mode refusal, restoration and Stop all. Native DocumentsUI imported
  Main-path fixture. One Share all click kept access links collapsed; the plain
  archive Share action opened Nearby and offered only Main-path fixture over QUIC.
  Actual images: `/tmp/kiwix-nearby-offline-final.png` and
  `/tmp/kiwix-nearby-sharing-simple-final.png`.
- Two real independently profiled native hosts exercised archive Details → Share,
  receiver Nearby (no Find tap) → discovered Main-path fixture → Download →
  Completed/import → Open → actual reader H1 `Main`. No manual details were opened.
  Source/received SHA-256 matched:
  `ccfc653cedc73507328a9bcf3360ad15f3cbca6379fc31fbe7da928db8334e1a`.
  Receiver used an owned download directory, never the user's Downloads folder.
  Stop withdrew the offer. Graceful receiver restart restored Nearby; discovery
  starts when that surface opens, sharing remained off, and the received archive
  remained readable; browser errors were empty.
- Opening the Nearby surface starts discovery once; it does not start sharing.
  Sharing remains an explicit Share all. Stop is not undone by status polls;
  privacy/read-only changes do not create a new intention. Finding sends local
  discovery traffic but neither advertises library inventory nor starts a writer
  or archive upload. Apple discovery already included P2P/AWDL with the old
  switch off; its retained advanced setting changes connection order only.
- Production frontend/native host and arm64 Android/iOS builds passed. Focused
  existing local-admission regression passed once and error-visibility tests
  passed 10 (`22559`); this is not a new full-suite run. Locale generation retains
  59 complete/74 catalogs over 1,296 sources, with unreviewed translation provenance.
  Physical two-phone throughput, BLE credential bootstrap, AP-only routing and
  automatic fastest-route selection remain separate gates; see LOCALSWARM.
- Final native visual acceptance moved active offers and incoming archives ahead
  of setup. At 320 px the selected archive and Stop sharing were on the first
  screen; no horizontal overflow, visible alerts or browser errors. Image:
  `/tmp/kiwix-nearby-priority-final.png`. Final host/frontend/arm64 packages passed
  (`22617`); final APK installed in place on Samsung and cold-launched in 1,037 ms.
  The simplified iOS package initially failed installation with CoreDevice4016.
  After reconnection, strict/deep signature verification, in-place installation
  and native launch all passed (`bg_66`); no reset/uninstall was used.

## Library-session acceptance

- Actual native host/common UI: one Share all click → real DNS-SD library →
  Browse → download → import → Main-path fixture read. A protected selected-file
  session rejected a wrong PIN, exposed only StreetZim Demo with the correct PIN,
  then completed/imported/read that archive. Normal Share all after Stop again
  exposed an open session, not a silently persisted PIN.
- Browser landing/index exposed read and explicit Download links. Stop closed
  the actual listener (`ECONNREFUSED`) and withdrew the receiver's peer card.
  At 320/402 px the Share all button was 44 px high without page overflow.
  Browser screenshots timed out; no new screenshot certification is claimed.
- Adversarial review repaired filename suffix/reserved-name poisoning, terminal
  grant retention, Android hotspot Stop/Pause sharing-mutex re-entry, and missing
  iOS HTTP-receive awake ownership. Durable recovery tests use real persisted
  range/adoption proof, not a merely full visible byte count.
- Full host suite passed 795, nine ignored (`22429`); JavaScript 107 (`22395`);
  iOS XCTest 22 (`22391`), Android plugin unit build/test (`22392`). These do not
  certify physical phone pairs, AP-only routes, radio throughput or routerless
  whole-library HTTP. Library discovery uses the physical LAN on Mac and either
  the LAN or the Android app-owned AP with BLE join metadata on mobile;
  QUIC/AWDL is a separate selected-archive route.
- Both physical phones received the earlier verified repair/BitTorrent packages
  (`22344`) in place. Android cold process launch took 1,024 ms; its display was
  locked during capture. iOS signature verification/install passed, but launch
  was denied by the normal Locked policy. No lock bypass was attempted.
- Final common-UI/native host checks also honored an explicitly entered PIN
  when pressing Share all and retired sharing on private-mode entry. A received
  Main-path archive remained readable after a real receiver process restart.
  The first final-profile attempt correctly refused an existing destination;
  an owned isolated download directory and renewed browse receipt then completed.
  Earlier smoke files accidentally written to the default Downloads folder were
  removed only after matching their recorded device/inode/size/SHA-256 ownership.
- Final native Android emulator check (`22510` packages): Share all → Home →
  actual HTTP `200 OK` library landing while its foreground service remained
  active → return → Stop → connection refused and foreground service gone.
  Notification-display permission was denied throughout. The failed-before run
  exposed `onStop` releasing the multicast lease; the repair retains only an
  already-held lease under the admitted foreground service, never new background
  discovery. Return/Stop left no visible alerts or browser errors. Actual 320 px
  screenshot: `/tmp/kiwix-share-native-android-final.png`.
- A real loopback-bound session exercised discovery failure: the default UI
  showed a neutral access-link notice only; Show technical error details exposed
  the native reason and disabling it hid that reason. Final platform-bound
  Android/iOS packages built; iOS strict/deep signature verification passed.
  The rejected intermediate preview omitted `TAURI_ENV_PLATFORM`; it was not
  deployed to either phone. Mobile asset builds must retain that native target.
  After the iPhone 17 Pro Max test device was reconnected/unlocked, the final signed Share all build
  installed in place and launched successfully (`bg_51`). No library reset.
- Final repaired Share all APK installed in place on physical Samsung
  `<samsung-serial>`; cold launch succeeded in 1,393 ms. No library reset, uninstall
  or physical permission changes. This is install/launch proof, not a physical
  two-phone sharing run.

## BitTorrent acquisition acceptance

- Actual official catalog button → publisher magnet resolution → native torrent
  completion → import → readable Sanctuary Type Classes documentation passed
  with a 317,149-byte ZIM. Continued seeding remained off. A missing privacy
  generation on the new resolver command was caught in this smoke and corrected
  before the successful run.
- Controlled real-engine magnet and `.torrent` transfers matched source SHA-256,
  imported/opened, paused/resumed and exercised seeding confirmation, cancellation
  and stop. URL/magnet entry, upload statistics and seeding controls are visible
  outside Details. Evidence: `local://TorrentFlowQA.md`.
- Catalog grid/list bounds passed at 320, 402 and 1365 px, with no observed
  horizontal overflow and 44 px download buttons. Browser screenshots timed out;
  DOM bounds and actual actions do not substitute for screenshot certification.
- Trusted profile-root alias canonicalization repairs `/tmp` torrent metadata
  publication without admitting descendant symlinks. A real publication/security
  regression passed alongside four official-resolver boundary/payload tests.
- Latest JavaScript suite and host/frontend build passed (`22338`); full host
  suite passed 776, nine ignored (`22326`). Post-change Android APK and iOS IPA
  builds passed (`22344`). These checks do not certify physical-phone torrent
  UI, radio throughput or iOS background torrent delivery; phones were unchanged.

## Android HTTPS tracker acceptance

- librqbit's reqwest 0.13 client verifies HTTPS through the Android platform
  verifier, which the app never initialised. On an API 24 x86_64 emulator, the
  real librqbit 9.0.1 `add_torrent` for a magnet with an `https://` tracker
  panicked with "Expect rustls-platform-verifier to be initialized", and no
  request reached the tracker. UDP trackers, DHT and plain-HTTP trackers do
  not use this verifier, unless an HTTP tracker redirects to HTTPS. A torrent
  added from `.torrent` bytes was not run; by source, its HTTPS announce would
  panic inside librqbit's tracker task instead. The app's own catalog, `.torrent` and HTTP-download client
  is reqwest 0.12 with bundled roots and was not affected. Earlier torrent
  acceptance used controlled peers, not HTTPS trackers.
- `src-tauri/src/android_tls.rs` now initialises the verifier from Tao's
  application Context. This happens when the torrent engine first starts, on a
  Tokio worker. It first loads the Kotlin class and resolves the method and
  fields the verifier uses. If the class is missing, every torrent job fails
  with "BitTorrent is unavailable because this installation is incomplete.
  Reinstall Kiwix.", and logcat (`Kiwix`) gets the technical detail. The probe
  checked this with a package built without the module. The class is built
  from a pinned vendored module.
- With the app's exact network config, an R8 release probe and a debug probe
  passed. Android fetched the leaf's CRL from a listed CA host, and librqbit's
  announce reached a local HTTPS tracker with UA `Kiwix Workbench`. A revoked
  leaf failed as `Revoked`, and an untrusted root failed as `UnknownIssuer`.
  Without upstream's CRL hosts, a CRL-only leaf failed as `Revoked`.
  Evidence and steps: `scripts/android-https-tracker-probe/README.md`.
- Not covered: the full app APK, physical devices, Android versions other than
  7.0, and public trackers and CAs on the Internet (the session's network proxy
  intercepts TLS). Upstream's verifier also rejects a certificate that has
  neither OCSP nor a CRL and chains to a system root. Some short-lived public
  certificates omit both.
- The allowed CRL hosts are upstream's 0.7.1 CCADB snapshot. A CRL-only
  certificate whose CRL lives on another host, such as a CA intermediate added
  later, fails as `Revoked`; librqbit records that only in its debug log.
- Verification is a blocking JNI call on the thread running the handshake,
  and Android may fetch a CRL inside it. On the software-CPU emulator, a
  handshake that fetched a CRL from a LAN server took about 5–8 s. The
  revocation check without a fetch took 1.3–2.5 s, and a rejection before
  revocation took about 25 ms. Slow CA servers, several HTTPS trackers at
  once, or librqbit's retry backoff can repeat this. Device latency against
  real CA servers was not measured.
- librqbit starts its tasks on the runtime that calls it. Its calls that start
  tasks or poll trackers now run on a dedicated two-thread `kiwix-torrent`
  runtime: session creation, `add_torrent` (a list-only magnet add polls
  trackers inside the call), and `unpause`. So blocked verifications occupy
  only that runtime; torrent networking can stall, but Tauri's command
  handlers keep their workers. The same applies to the platform verifier on
  macOS and Windows.
- The manager's worker loops, gates and retirement stay on the caller's
  runtime, and no gate is held while a routed call is awaited. Pause, cancel,
  suspend, shutdown and the Android privacy, read-only and notification
  cutovers therefore never wait for a torrent thread. (Retirement still takes
  librqbit's own locks, which an add holds while opening storage: a disk
  wait, not a verification.) `resume()` no longer unpauses directly; the
  job's worker does it within one 250 ms tick. So turning seeding back on for
  a live completed job takes effect within 250 ms, and an unpause failure now
  appears as the job's error rather than as the resume command's.
- librqbit registers a torrent synchronously after the last await of
  `add_torrent`, so a routed add can finish after its worker gave up. An
  abandoned add is aborted and reaped: a detached task deletes any torrent it
  registered, and the job's next worker waits for that before adding the
  torrent again. Without this, resuming failed as "already managed". The
  job's next worker gives up with a retryable error after 60 s if the reap
  has not finished, for example because storage hangs. Until the reaper
  runs, normally milliseconds after a torrent thread frees up, the abandoned
  torrent is registered and started (usually still checking files), and its
  DHT and local-discovery announces have begun.
  Separately, and unchanged by this work, librqbit's local-discovery
  announce for a torrent keeps running until the session stops, even after
  the torrent is paused or deleted.
- Two earlier attempts were reverted after review: worker loops on the
  two-thread runtime, and routed calls awaited under the manager's gate.
  Both made pause wait for blocked verifications (about 3 s in standalone
  repros); the second could also orphan a torrent in the session.
- Evidence and limits: integration removed source-text wiring assertions;
  the verifier script retains dependency compatibility, upstream integrity
  and revocation-host security-policy checks. Manager-level tests use real
  librqbit sessions and block every torrent thread. They require pause and
  cancel to return within 1 s, and suspend within 2.5 s (librqbit's
  `Session::stop` sleeps 1 s). They also require an abandoned, registered add
  to be reaped and the job to re-add its torrent. librqbit binds `[::]`, so
  these tests need IPv6; on a host without it they were run with a socket
  shim that maps IPv6 onto IPv4. The independent review also ran a stress
  harness that is not in the repository. With natural timing, each of two
  300-pause runs abandoned one in-flight add; in one, the add had already
  registered and the reaper deleted it.
  With a 3 ms delay injected after `add_torrent` returned, 110 of 300 adds
  were abandoned after registering, and all were reaped. No run had
  orphans or "already managed" errors, and pause took at most 8.9 ms.
  Thread blocking was simulated; this was not exercised on Android hardware
  or an emulator, or with the macOS and Windows verifiers. A verification
  already inside JNI still cannot be interrupted; it only delays torrent
  work.

## Current acceptance boundaries

- Actual disposable iOS simulator acceptance caught a fresh-launch crash caused
  by reassignment of non-retained `UIApplication.delegate`. The class-level,
  pre-`UIApplicationMain` handoff repaired that crash. The corrected app was
  actually launched, imported fixtures through the picker, read/navigated Back,
  backgrounded/returned, cold-restarted and exercised native speech controls.
  Evidence: `local://AcceptIOS-acceptance.md` and
  `/tmp/kiwix-accept-ios-evidence/`. Speech dispatch/control state, not hardware
  audio quality, was accepted. This is not a guarantee that every launch succeeds.
- Android actual-emulator picker cancel/success, admission-before-read,
  delimiter-path reading, link/native Back, widget return and background/return
  passed functionally without new stale checkpoint/storage bands. The actual
  PDF failure band remained visible. Evidence:
  `local://AndroidFinalQA-acceptance.md` and `/tmp/kiwix-android-finalqa-5586/`.
  This is not blanket checkpoint/no-storage-error certification. Subsequent
  PDF recipient/cancellation checks passed below. These checks preceded the
  subsequent physical-phone deployments recorded above.
- Implemented finite durability checkpoints bind exact ID, generation and a
  1,800 ms deadline. Terminal `KIWIX_SUSPENDED` with `retry:false` cancels without
  enqueue/replay or stale error banners; genuine persistence failures remain
  visible. Reader navigation awaits cancellable actual storage admission.
  Android `onPause` is inactive, while `onStop` suspends; foreground and visible
  are distinct lifecycle facts.
- PDF capture binds the exact live document URL (allowing equivalent percent-dot
  spelling), with corrected descriptor ownership and localized controls.
  Android uses a visible, cancellable preview; native recipient and typed
  cancellation acceptance subsequently passed as recorded below.
- Signing/distribution credentials and Windows/Linux printers require external
  environments or owner inputs. The earlier simulator official-download attempt
  failed because `nsurlsessiond` was unavailable; subsequent physical iPhone
  background completion/process-loss delivery passed as recorded above.
- Subsequent parent-recorded evidence: latest iOS frontend/manual Xcode simulator
  build passed (artifact `22088`); Android JVM suite passed 18 tests, zero
  failures (artifact `22083`: intake 6, crash capture 2, PDF layout 2,
  speech/provider authority 8). Native iOS incoming-file opening reached real
  **Import incoming archives** consent and copied one Main-path fixture card
  (`/tmp/kiwix-final-ios-import-outcome.png`).
- Parent-recorded latest simulator install/launch passed on artifact `22088`
  (PID 46914). At `accessibility-extra-extra-extra-large`, actual HID taps and
  horizontal bottom-navigation swipes reached all seven destinations: Library,
  Discover, Maps, Nearby, Reading list, Settings and Downloads. Every screen
  and selected control was visibly correct, with no error bands. Text size was
  restored to `large`. Evidence:
  `/tmp/kiwix-final-ios-maxtext-{library,discover,maps,nearby,reading-list,settings,downloads}.png`
  plus navigation-middle/downloads captures. This accepts maximum-scale
  simulator navigation, not physical hardware or assistive-technology behavior.
- Latest parent-reported Android Share run reached native preview-visible,
  tile-drawn, write-complete and retire-complete stages in about 1.1 seconds,
  but export handoff failed with `PATH_OUTSIDE_APP_STORAGE`. Android `/data/data`
  and `/data/user/0` are bind aliases here, not symlinks; canonicalization alone
  cannot unify them. The integrated repair obtains `cachePath` from trusted
  native `storageRoot` metadata, then creates, validates and canonicalizes the
  scratch directory there. Native `prepareViewer` likewise uses its canonical
  cache producer; strict native export ownership validation remains unchanged.
  Subsequent repaired-artifact native acceptance passed as detailed below.
  iOS simple-fixture PDF sharing passed end-to-end: authenticated Main-path
  reader → Appearance → Print and export → Share → native share sheet →
  Apple Preview rendered PDF title Main page and correct H1 Main. Evidence:
  `/tmp/kiwix-final-ios-pdf-recipient.png` and
  `/tmp/kiwix-final-ios-share-{outcome,settled}.png`. Complex long-page fidelity
  and cancellation remain unverified.
  Returning from Preview to Kiwix (PID 46914) also restored normal export
  controls without a lingering Saving state or error:
  `/tmp/kiwix-final-ios-share-return.png`. This accepts foreground post-handoff
  recovery for that run. Physical phones were unchanged; evidence is retained.
- Parent-recorded latest host binary (artifact `22117`) reopened the same real
  smoke profile: Main/H1 Main and two archive tabs restored, Downloads showed
  no downloads, deleted publication did not resurrect, and no browser errors
  were observed. This does not substitute for native PDF handoff QA.
- After the last trusted-native-cache-root producer repair, focused host
  `live_pdf_` regressions passed 2/2 (artifact `22165`), followed by the full
  integrated host suite: **771 passed, nine ignored** (artifact `22172`).
  Latest iOS arm64 device build/export passed after the final scratch-root repair
  (artifact `22173`): `src-tauri/gen/apple/build/arm64/Kiwix.ipa`.
  This IPA was not installed on a physical phone. Android root-fixed `22176`,
  scrollbar-corrected `22209`, and final typed-cancellation APK `22227` compiled
  the integrated Android repairs; native outcomes follow.
- Repaired Android artifact `22176` installed and relaunched without the old
  `PATH_OUTSIDE_APP_STORAGE` band. Main-path Share passed: job `756cba8b`
  completed writing, real chooser offered Main page.pdf, and native Android
  Print recipient (`com.android.printspooler` PrintActivity) rendered correct
  Main heading and two pages. Evidence:
  `/tmp/kiwix-android-finalqa-5586/44-pdf-print-recipient.png`.
  Cancellation passed separately on the final artifact below. The large-article
  fixture hit the existing live-page-too-large guard before any native job;
  that refusal is not renderer failure.
- Subsequent Android artifact `22209` recipient acceptance confirmed removal of
  the baked-in native WebView scrollbar by disabling native vertical/horizontal
  bars and overscroll glow only; archive content/CSS was unchanged. Evidence:
  `/tmp/kiwix-android-finalqa-5586/64-final-main-print-recipient.png`.
  The localized Cancel preview was visible (`65-visible-main-native-preview.png`).
  Actual dialog-focused Back prevented writing/chooser handoff but incorrectly
  displayed blocking English cancellation bands. A typed cancellation repair
  now resolves native UI Cancel/Back as `{cancelled:true}` through Rust
  completion/outcome cancellation, without pretending sharing opened or hiding
  genuine render/policy/cleanup failures.
- Final Android typed cancellation passed on artifact `22227`: job `8185a9a5`
  showed preview at 11:05:12.312; native Back after actual dialog focus produced
  `retire-cancelled` at 11:05:13.389, with no write, chooser or handoff.
  `67-final-typed-cancel-return` screenshot/XML shows no global/reader error
  bands and re-enabled export controls; `68-final-readable-main-after-cancel`
  shows Main clean and readable. Evidence:
  `/tmp/kiwix-android-finalqa-5586/` and `local://AndroidFinalQA-acceptance.md`.
  Artifact `22209` recipient and `22227` cancellation evidence are distinct
  exercised runs, not complex-page fidelity or physical-device certification.
  Final desktop asset restoration build passed (artifact `22230`).
- The exercised core headless process exited 0 after normal Ctrl-C, and its
  managed browser was closed. Cleanup targets only parent-owned disposable
  simulator/companion/OCR resources, not user profiles or physical devices.

## Invariants the app now enforces
- **Migration cleanup requires ownership and a verified replacement.** Journals
  retain exact staged/publication/source identities. Colliding foreign files are
  never rollback targets. Missing or corrupt replacements preserve originals.
  Store errors after commit preserve adopted copies and the journal.
  Provider-dependent replay starts only after the native resolver exists.
- **Download lifecycle has one owner.** Explicit pauses survive restart; local
  verification sweeps are joined before pause/resume. Deletion retires pending
  import authority before removing publication. Torrent metadata publishes
  atomically. Saturated LocalSwarm pipelines hand pending verified bytes to the
  writer before awaiting more budget.
- **Policy changes drain writers.** Native/Rust crash capture observe read-only
  policy. iOS reconnect rechecks policy after locks. Native background HTTP has
  a joined private/read-only barrier even before a download manager exists.
  Updater installation excludes new transfers and migrations.


- **Profile contention has an explicit startup path.** A second instance
  activates the profile owner (any bundle id) or shows a specific dialog and
  exits 0. Headless and read-only launches follow the same path. A lock failure
  that is not contention remains a real startup failure with its OS error.
- **No blank window.** Every window paints a themed placeholder before the
  bundle loads; a watchdog adds a diagnostic panel (reason, reset saved window
  state, quit) when the interface never reports ready. Unknown persisted
  surfaces fall back to the library; an unreadable surface no longer aborts.
- **Saved state is migrated, not dropped.** Unknown fields are ignored, unknown
  surfaces normalize, legacy records upgrade in place, and the normalized state
  is written once so later launches are silent. Quarantine (torrents,
  downloads, mobile state) preserves the original bytes and reports it quietly.
- **Startup cannot be bricked by user data.** An unbound mobile root marker is
  rebound, a moved container re-anchors saved library paths, an auxiliary
  record that fails to parse is left for its manager, and a native
  initialization failure renders a real panel instead of panicking.
- **Reading never dead-ends.** Archives without a main page open a random
  article; an in-frame miss redirects to the archive's main page with the
  bridge attached; malformed stored locations are treated as misses; HTML over
  16 MiB streams; entry failures map to 404 and archive damage to 500.
- **Recoverable failures are quiet.** Severity is decided at the call site.
  Only blocking failures paint a band; the technical-details setting (off by
  default) reveals the rest from a bounded in-memory list. User copy should state
  the failed action and recovery; `docs/ERRORS.md` is an inventory, not a test of
  correctness. Severity/visibility have behavioral regressions.
- **A denied sensor stays denied.** A failed decision write is blocking and
  aborts the reload; a stale granted state can no longer auto-allow a request
  the user blocked.
- **Android lifecycle holds.** `onStop` always reports suspension; transient
  URI grants are never persisted; re-importing an installed archive resolves to
  the existing book; dismissal works in read-only mode.

## Verified core loop

- macOS (headless shell + real service, real profile copy): launch, catalog
  download, open, read, link, Back, restart, history reopen, import from
  `~/Downloads`, remove (file kept), permanent delete (file removed), quit and
  restore. `startupWarnings` empty; second launch byte-identical.
- iOS simulator: install, picker import, open, read, link, Back,
  background/return, cold restart, remove, permanent delete, and a reinstall
  that changed the container UUID (book path rebased, tab painted, no band).
- Copy regression: corrupted torrents/downloads history, unknown surface, and
  a stale container book path all produce quiet plain-language notices.

## Read-aloud Live Activity and highlight — 2026-11-14

- The signed iOS build with the read-aloud Live Activity, header play/pause,
  next-sentence prefetch and per-sentence highlight was installed and launched
  on the iPhone 17 Pro Max test device. The install/launch harness passed and preserved the existing
  profile; audio, the lock-screen card, its control links, and sentence-by-
  sentence paint were not exercised by the automated window.
- Progress paint is now accepted during a retained-storage freeze when the
  privacy generation is current; a terminal event clears it. This is a
  paint-only path and does not admit navigation or writes.
- A physical report says tapping the Live Activity body opens the app and
  playback stops. The body now has an explicit `kiwix://library` widget URL and
  the Dynamic Island default URL, so it cannot synthesize an implicit speech
  action; the card's own previous/pause/play/stop links still route through
  `kiwix://speech` into the ownership-checked `mobile_speech_action` path.
  This does **not** stop the system from opening the app on a body tap, and it
  does not fix or explain the reported playback stop. No physical body-tap,
  lock-screen, Dynamic Island, control, or playback-retention acceptance is
  claimed.
- Android still exposes read-aloud through its `mediaPlayback` foreground
  service and MediaSession notification; this Live Activity work is iOS-only.

## Residual checks that need a physical device or a different environment

- Official-catalog iOS HTTP uses OS-owned background URLSession. Backend-observed
  official HTTPS provenance grants eligibility; custom URLs keep strict
  foreground transport. The user-approved native path uses iOS redirect policy,
  not Rust per-hop checks. One physical background/process-loss completion passed
  above; cellular-policy cutovers and fault-boundary acceptance remain open.
- Live-page PDF, native window-group restoration, accessibility text scaling,
  Android selected-file copy/move, speech transport and native print paths have
  new implementations. Host/native tests are not physical printer, provider or
  assistive-technology certification.


- Physical iPhone: audible read-aloud, lock-screen/headset media buttons,
  Doze/OEM foreground-service restrictions, notification-permission denial,
  real share-sheet grants, and a real iCloud/device-transfer restore.
- Background audio on a physical iPhone. `UIBackgroundModes: audio` and the
  startup `.playback` category are declared. WebKit, not DOM media-state
  callbacks, owns archive audio activation. Explicit native activation is
  reserved for read-aloud; its claim survives a stale speech-inclusive audible
  report until the shell reports quiet. Repeated unmuted TED WebM pauses were
  confirmed on the physical iPhone and the user confirmed playback works after
  removing app activation/reassertion and duplicate interruption pauses.
  That foreground result does not certify lock-screen/background playback,
  PiP audio, headphone unplugging, or interruption/resume behavior.
- macOS pixel-level verification is unavailable to automation in this
  environment (no Screen Recording or Accessibility grant); window presence and
  state were verified instead, with UI rendered through the same bundle in
  headless mode.
- Cross-bundle `zim://` Apple Events are not forwarded to a running instance.

## Known non-defects

- Silent recovery of a self-healed state (for example an unknown window
  surface) is intentional; it is recorded for troubleshooting but not shown.
- A ZIM page whose own JavaScript search reports no results is archive content,
  not the reader.
