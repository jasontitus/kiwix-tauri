# Nearby phone-pair acceptance

## Evidence boundary

### Current recovery evidence, 2026-11-08

Run `device-nearby-27fc9a19f7c348fa8174e6f2ff2f6454` used a clean
reinstallation and ordinary initial launch. Its Android host receipt records
`adb instrumentation failed: 255` and cleanup failed because
`device '<samsung-serial>' not found`. The iPhone diagnostic recorded the Android
peer, but this run did not capture catalog, completed transfer or readable
content. It does not establish an application publisher-lifecycle failure.
The original failed receipt remains unchanged.

After ADB reconnected, the Android target process was absent and all three
current `mCurrentSoftApInfoMap` values were empty; the iPhone process inventory
contained no `KiwixPhoneWindowUI-Runner`. This recovers current automation
cleanup, not the missing transfer or original preservation evidence.

Nearby preflight now initializes an absent installation or uninitialized state
through ordinary startup, gated on successful inventories, then copies the
actual persisted state. It does not synthesize an empty preservation baseline.
The UI scenario never cancels or resumes arbitrary preexisting downloads.

A separate same-host Mac release benchmark passed
`apple_sender_serial_quinn_throughput_measurement`. Its Apple-to-Quinn socket
trials do not certify physical Mac-to-iPhone connectivity or throughput.

The subsequent reviewed repair passed 17 host harness tests, seven Android
intake/lifecycle unit tests, four idle-owner tests in a foreground UIKit
simulator host, two mixed-address advertisement regressions, and the real
Share-all retirement regression through private, read-only and explicit-stop
transitions. The iPhone build at `fc81e1f` prepared successfully off-device.
These checks do not replace physical sharing, full-file hash, readable-content,
preservation or cleanup evidence for a new device run.

The reviewed `68c0669` iOS and Android builds completed. A Mac UI smoke
published the Hawaii library and withdrew it with Stop all; it did not connect
a phone. Run `device-nearby-7ea9982eedae4bffb239580a44cb54db` then stopped
before Android's instrumentation-ready handshake: a retained startup crash
report obscured Nearby. The Android crash buffer records an earlier
`UiAutomation.disconnect` / `DeadObjectException` in the harness's
`Instrumentation.finish`, at the prior ADB-disconnected run's time. This is
not evidence of a new application sharing crash. Both devices' owned cleanup
was confirmed; no transfer was measured in this run.

Read-only capture `.native/android-crash-capture-68c0669` retained the actual
659-byte native report and confirmed that earlier harness stack. Its receipt
records no UI launch, unchanged report bytes and completed cleanup. Nine
receipt-identity and report-retention regressions passed.

The subsequent Android-only smoke
`.native/android-reviewed-crash-publication-68c0669` failed before publication:
the intended View report tap opened Gmail's composer despite DOM inspection
identifying View report. [INFERENCE] Stale geometry during the startup-sheet
animation is the suspected cause. No Send action was
issued. The exact test-created draft was identified by its Kiwix launch origin,
subject and crash body, then discarded through Gmail's ordinary Discard control;
the native UI returned to Kiwix. The smoke's final report-preservation check
expired, so report preservation is not certified by that failed smoke.

The corrected Android-only run `.native/android-hitfix-publication-proof`
passed ordinary View report → exact displayed JSON → Close with retained native
accessibility hit bounds. It preserved the same report and existing library,
settings and tabs, published the app-owned temporary network, then explicitly
stopped publication and confirmed no current AP and completed cleanup. Five
motion-admission fixtures passed. This is not an iPhone transfer result.

The real Mac helper → Python consumer smoke
`.native/mac-peer-adapter-smoke-correct-title` passed publication readiness,
nonce-bound stop, preservation and graceful app/browser reap. Seven Mac helper
regressions and 24 handset transport tests passed. An earlier smoke supplied an
incorrect frozen title and was rejected with clean owned cleanup; the actual Hawaii
metadata title is `Hawaii`, not its prior customized reader-tab caption.
The subsequent `.native/mac-native-pair-host-smoke` exercised two real Mac
helpers through the Python controller: the receiver admitted the exact
150,605,015-byte managed Hawaii archive with matching full-file SHA-256 and
active UUID, and both peers preserved prior state and completed cleanup.
The captured reader shows the archive's text and map controls, but not rendered
map geography; it does not certify map-tile/GPU rendering. This same-host run
does not certify physical Mac↔iPhone connectivity, RF behavior or throughput.

The reviewed Android→iPhone run `device-nearby-00836a5700814f0c873386b459cb3946`
found Android but stopped at the system's **Unable to join the network**
dialog. No transfer was measured. Both sides' owned cleanup completed; Android's
exact reviewed report and preexisting books, settings and tabs were preserved.

The first Apple-LAN window `device-nearby-f694954e38e642e4b77a353051060808`
discovered the Mac library behind that retained Android error, then stopped
before selection. The reviewed runner now captures and acknowledges a
preexisting failure once at startup; any new association failure remains fatal.
It never retries a stale Join request.

Mac→iPhone run `device-nearby-c2b13c95c662490ebc6bfc63f7701895` subsequently
passed transfer of the 150,605,015-byte Hawaii archive, managed admission,
full-file SHA-256, active archive/readable-content checks, preservation and
both sides' owned cleanup. It exercised the startup-only alert acknowledgement.
The instrumented Download→Completed observation was 7.260 seconds, not an
isolated radio-throughput measurement. Mac publication used the real headless
Chromium-backed application. The exported phone reader screenshot
`attachments/49A4EE65-8C57-4EE2-A4D8-32FC4BA3AA0C.png` visibly renders Hawaii's
islands, roads and Honolulu label. Infrastructure-vs-AWDL routing and native
macOS WKWebView behavior were not measured.

Reverse run `device-nearby-7d37470a57cc48e08abc705c58ac170a` passed
iPhone→Mac reception of the same 150,605,015 bytes with matching full-file
SHA-256, managed Mac admission and active reader/content checks. It reused the
existing managed iPhone installation without resetting or deleting data.
The exact iPhone peer ID/generation bound discovery and the observed session;
explicit iPhone Stop sharing, preservation and both sides' cleanup passed.
No iOS Wi-Fi Join was requested. Mac reader evidence covers archive text and
controls, not geographic rendering in headless Chromium.

The first shared-LAN Mac→Android run
`.native/mac-android-forward-run-3bbc2a4` failed before transfer: Android
completed the QUIC connection, but catalog browsing reached its 30-second
`Shared library did not respond` timeout. Both bound peer cleanup receipts
confirmed cleanup and host-process reap. The controller's former generic
cleanup/preservation error obscured that child failure; it now preserves the
actual failed-peer error separately from cleanup validation. This run does not
establish completed Mac↔Android sharing or identify the transport cause by
itself.

### Native interoperability recovery, 2026-11-09

The corrected native library listener retires each request's owning QUIC
group after its handler drains. Quinn recognizes the exact received no-error
close, without accepting local errors or cancellation. The real same-host
Quinn→Apple regression passed eighteen completed production catalog browses,
admission-slot reuse, isolation of an independent client awaiting ACK, invalid-
ACK rejection and bounded cleanup. A typed boundary regression separately
passed exact remote-close and cancellation checks. These are transport proofs,
not a completed physical Mac↔Android transfer.

The real publisher→observer smoke
`.native/publisher-observer-smoke-f255c172def147fd98ec28c4a63da515` passed exact
source/session/offer observation without local intake, download or publication,
then withdrew the observer before stopping its publisher; both peers cleaned
up. Eleven observer boundary tests passed. This does not certify a two-iPhone
transfer or its device cleanup.
The two-iPhone attempt
`device-nearby-ab71f66e262b4dbe89a9fce55856bb3a` stopped at personal-iPhone
lock preflight with CoreDevice 4016, before installation or UI work. Primary
automation stopped; the secondary never started. Its original failed result
is unchanged; `cleanup-recovery.json` records the independently reviewed
ownership classification. No transfer was measured. Two release-boundary
tests passed; partial launches still require exact-device cleanup receipts.

### Verified Mac↔Android LAN transfers, 2026-11-09

Both physical directions completed on Samsung `SM-A356U` (API 36) over the
ordinary `Up in the air` LAN with the app-owned access point unused. Production
APK `b801eac4…779752` was signed with the same reviewed certificate; the Mac
peer was the rebuilt debug binary `6748ab46…66b10`.

- Mac→Android `.native/forward-fixed2-run-20261109`: `complete`,
  `transfer_complete`, `cleanup_complete`, `device_released`. The 37,162-byte
  `large_article.zim` (`2001a92a…1ba1f60d`) was received, imported and opened
  with the **Large article** content. Controller wall was 48.45 s; the receiver
  ledger was 19.57 s: 5.81 s ADB command wall, 7.66 s observed UI waits,
  14.80 s device automation, 1.08 s cleanup (inclusive categories).
- Android→Mac `.native/reverse-fixed-run-20261109`: `complete` with the
  150,605,015-byte Hawaii archive, independently matching file SHA-256,
  managed admission, readable reader, and a completed job carrying the
  verified `publication` identity `{size, sha256}`. Controller wall was
  48.85 s; the publisher ledger was 46.37 s: 6.11 s ADB command wall,
  0.28 s observed UI waits, 41.49 s device automation, 1.03 s cleanup.

Defects these runs exposed and closed:

- Android `Share all` silently started an access point when native state
  reported no local address, which broke LAN publication and discovery. It now
  fails with `NEARBY_LAN_REQUIRED` and the UI points at **Use temporary Wi-Fi**.
- A completed download job exposed no verified publication identity to the Mac
  receiver. The public snapshot now carries `{size, sha256}` from the durable
  publication intent; the journal keeps a single `publication` key and the
  receipt survives manager reopen.
- Test instrumentation crashed the target app through
  `Instrumentation.finish()` when the automation binder was already dead. That
  call is guarded now. The stale retained report from the earlier crash was
  removed through the app's own **Delete report** action after its contents
  remained in prior receipts; its automation artifact must not be treated as a
  product crash.
- The Mac peer asserted a discovery UI card in the same instant the status
  updated; it now waits a bounded time for the card to render.

These runs prove these two fixture transfers on this pair, not general
throughput. Mac evidence comes from the real headless Chromium-backed
application, not native WKWebView. Exact model-generation time is unavailable
and is not derived from residual wall time.


### Verified Android→iPhone app-hotspot transfer, 2026-11-09

Run `device-nearby-6525d3ea3a704f0298349647ce53ed6b` passed end to end:
Samsung published its app-owned temporary Wi-Fi with the managed
`42f3a714…` fixture, the dedicated iPhone (`00008110…`) joined through the
single OS Join prompt, loaded the catalog, completed the transfer and opened
the readable receiver surface; the phone was released. Earlier
`AndroidShare_…` join failures are not reproduced by this run. The same
iPhone→Android direction passed as `device-nearby-de2e96dc37404299b9f2670a63aee9f0`.

### Verified Android↔Android shared-Wi-Fi transfer, 2026-11-09

Samsung `SM-A356U` (API 36) published its library with ordinary **Share all**
on a shared Wi-Fi network (Mac Internet Sharing, WPA2), no app hotspot. The
Fire HD 10 (API 28, Fire OS 7.3.3.1) joined the same network, discovered the
live sender (11 archives), connected, and downloaded the 60,202-byte
`0816372c…` fixture; Downloads showed **Completed** and its reader rendered
**“Routerless field test”**. The receiver previously aborted with a generic
“Java exception was thrown”; removing hidden-`IpPrefix` use from
`MobilePolicy` fixed the LAN socket binding on Android 9.

Scope: shared-Wi-Fi LAN only. Joining an app-created temporary hotspot still
requires API 29 (`WifiNetworkSpecifier`), and this Fire rejects WPA3/PMF
(`AKMP_NOT_VALID`), so both app-hotspot and WPA3 networks remain unavailable
on that tablet.


### Earlier retained windows

A physical Android→iPhone transfer over the app-owned temporary Wi-Fi, managed import, readable reader and matching full-file SHA-256 are verified across two bounded windows. The initial completion-check automation selected the filename heading rather than its containing download article; its descendant **Open** lookup failed despite a completed transfer. The captured hierarchy names the actual container `wikipedia_en_100.zim, article`. With that selector corrected, run `device-nearby-f99e9246695e4271877d08b5cdd341b9` opened the same received archive, displayed the Wikipedia home page with its **Animal** gallery tile, and verified its active UUID and bytes. That recovery scheduled no new download and did not open the Animal article body. The iPhone's initial off-router state was not enforced in those earlier runs; they establish the temporary-network transfer path, not the complete both-phones-initially-disconnected scenario.

Run `device-nearby-37095deab287446bacfb6d2b804a034f` subsequently completed iPhone→Android with both phones initially off-router, one automated iOS Join and no manual password. Android imported and opened the 299,762-byte reader fixture; the captured screen visibly shows its **Image article** and **External image** links. The actual managed file's full SHA-256 was `64c381c2d63bff5f30ac3223292b613e6f6bbec9ef2b9e2d6818c7d9a4a578d8`, matching the frozen iPhone source, with active UUID `066871ed-6825-bc1e-4321-71db74da1d08`. Existing archives, settings and tabs were preserved. This small fixture is functional evidence, not a throughput benchmark. Earlier XCTest runs that passed after finding **Browse** or **Download all** were not transfer evidence.

The stricter fresh Android→iPhone scenario is still an open acceptance gate on
`66ab5a6`. Run `device-nearby-fdb89261c07f4f08822789c83f733d4a` had an active
Android publisher but never started native iPhone Bluetooth discovery. After
the native-readiness correction, run
`device-nearby-a830206639624580b743bdb224ee439f` found **Android**, read its BLE
information and subscribed, then failed with `BLE_SEND_TIMEOUT` before Wi-Fi
Join. Android's stack received the channel write but reported
`Context not found for ID 174`; its handle map retained an unregistered owner
whose descriptor handle overlapped the current channel. That collision is the
source-traced explanation for the lost write, not proof of the orphan's origin.
The current app must not fake a successful send, extend the timeout to hide it,
or reset the user's Bluetooth settings.

Android teardown now retains its exact server while service addition is
outstanding. The matching late completion may only clean up a retired owner,
never advertise it. Missing completion retains ownership and reports incomplete
cleanup rather than admitting a replacement. This prevents a separate
pending-add teardown race identified in source review; it does not repair an
already orphaned OS entry, and the pending-add cancellation path still needs
physical acceptance.

A one-time full Android Bluetooth reset was performed to clear the contaminated
test-stack state; Bluetooth and the saved scanning preference were restored. The next run,
`device-nearby-dbad6142578d48ebb8ba59d7032ec761`, used both `66ab5a6` builds
but stopped before the iPhone test began: iOS displayed **Enter iPhone Passcode
for “XCTest” — Enable UI Automation**. Enter that passcode locally on the test
phone; do not put it in a script or configuration. Automation cannot approve its
own initial system authorization. The run's Android publisher was withdrawn
and owned automation stopped. It provides no new connection or transfer proof.

The development Mac passed six host library regressions: four local membership, file-identity and reply-metadata tests, plus two real encrypted Quinn integration tests. The integration scenarios exercise a discarded initial response followed by four parallel exact-length ranges, retained response objects without an EOF poll, a subsequent catalog request, and exact socket rebinding. This is host transport evidence, not a phone-pair throughput result or native Apple interoperability certification.

## No phone Apple Account required

Leave the test iPhone signed out of iCloud and the App Store. The existing Mac Apple Developer account signs the app and registers the phone's UDID; an Apple Account on the phone is not a provisioning requirement.

A configuration profile cannot be fabricated to bypass app signature verification. The previous instruction to install a generic developer-trust configuration profile was incorrect.

The old development profiles contained `PPQCheck=true`. Apple documents that these development/ad-hoc builds must reach `https://ppq.apple.com` during installation or first launch. A routerless Android hotspot supplies a local network, not internet access. The generic Xcode trust error alone does not identify its cause, and the retained device security archive did not cover the earlier rejection timestamps.

For offline testing, use Apple's **Development - Offline** provisioning option. On this workstation, two such profiles were issued on 2026-11-07, restricted to the iPhone 13 test device and valid through 2026-11-14:

- Explicit `com.tiltastech.zimreader`, retaining Hotspot Configuration and Access Wi-Fi Information.
- Wildcard `*`, authorizing the existing XCTest runner and download widget.

Both authorize the existing development signer. Their decoded profiles omit `PPQCheck`; the isolated app/widget signing source passes strict deep signature verification. Both the app and XCTest runner subsequently installed and launched on the signed-out physical iPhone during the prepared Nearby windows. Profiles are kept in ignored local signing storage, not committed. Regenerate and re-sign before the seven-day expiration; do not edit Apple's signed profile payload.

An installed offline profile alone is insufficient when a superseded online profile remains eligible. The 15:15:14 device security log explicitly selected old profile `2658b803-0a05-4801-8e5c-6e3c01814cc2` and rejected launch with **Profile Needs Network Validation**. All four installed development profiles were backed up and decoded before removing only the two superseded PPQ profiles, `2658b803-0a05-4801-8e5c-6e3c01814cc2` and `4cda2663-43d4-4085-b247-cc40b6013504`. Both Apple-issued offline profiles remained installed. The subsequent prepared run `device-nearby-96614436f59542648672f63fc4db147c` cold-launched the app and runner offline successfully, then reached the separate download-card selector failure. No configuration profiles, accounts or archive data were removed.

Sources: [Apple provisioning updates and offline profiles](https://developer.apple.com/help/account/provisioning-profiles/provisioning-profile-updates/), [distribution to registered devices](https://developer.apple.com/documentation/xcode/distributing-your-app-to-registered-devices).

## Routerless interaction

Wi-Fi radios must be enabled even though neither phone is joined to a router. Bluetooth carries discovery and temporary-network credentials, not archive bytes.

- Android sender: **Nearby → Use temporary Wi-Fi → Share all** creates an app-owned LocalOnlyHotspot. iPhone receiver: **Nearby → choose Android → approve the system Join prompt**. The catalog should load without another Browse/rejoin cycle or manual password entry.
- iPhone sender: **Share all** publishes its library. The Android receiver's reverse-join flow hosts the temporary network; the iPhone joins that network and serves the selected library on the retained route.

The app cannot suppress the system's Wi-Fi consent. Configuration `apply` alone
does not prove a usable route. When Wi-Fi information is available, the iPhone
checks its exact SSID and retained BSSID. When iOS withholds it after the
app-owned apply, the join may retain the last OS-identified Wi-Fi interface only
while its current flags and IPv4 prefixes remain valid. Socket binding must
still prove the selected private endpoint is on that exact interface; it cannot
fall back to the default route. Once bound, interface replacement or loss of a
bound local prefix rejects the route in both association paths. A missing
Wi-Fi-information response never clears a previously observed BSSID.
See [Apple's API contract](https://developer.apple.com/documentation/networkextension/nehotspotnetwork/fetchcurrent(completionhandler:)).

Two SSIDs in a Settings scan do not prove two active app-owned reservations. Repeated start must reuse the live reservation and its route, not close it while the receiver is joining.

Android's exact characteristic-property equality also rejected the iPhone
before credential exchange. Run `device-nearby-869e994678ca488488845bc94eb203e7`
captured GATT discovery success, service present, channel properties **168
(`0xA8`)**, info properties **2**, and CCCD present. The channel is the required
write + indicate pair (`0x28`) with the standard Extended Properties bit (`0x80`).
Validation now ignores only that optional bit; both required operations, the
read-only info characteristic and subscription descriptor remain mandatory.
Other operation modes are still rejected. See [Android's property definition](https://developer.android.com/reference/android/bluetooth/BluetoothGattCharacteristic#PROPERTY_EXTENDED_PROPS).

Run `device-nearby-4ea903dd1a2141e1816bbd71ac21ddb6` passed that GATT boundary
and automated the reverse-network Join, then rejected the route as stopped
sharing. `WorkOwner` had passed its privacy generation to
`localswarm_manager`, whose fence expects a different sharing-admission
generation; those counters initialize to 1 and 0. They are now captured and
named separately. The owner inherits the captured admission cancellation token,
so explicit Stop and privacy retirement still revoke old work. This is not a
relaxation to accept whichever generation is current after a join.

The later `c0055522` association failure is separate: iOS accepted the app's configuration and Join consent, but its directed scan returned zero matching networks. It remained associated with the home router throughout; this is not evidence of a rejected password. Android reported its AP enabled but also logged a vendor `SET_PMK` failure. Neither the router connection nor that vendor message has been established as the cause of the empty scan.

The prepared UI runner now explicitly proves **Not Connected** or **Disconnected** in Control Center, Bluetooth **On**, and Airplane Mode **Off** before starting a connection. It uses Control Center's disconnect action, not the Settings Wi-Fi-off switch, and never forgets saved networks. Run `device-nearby-9b006687f8334f2bacedd863da143258` exercised that state and reopened the prior download; the subsequent `37095dea` run exercised a fresh reverse transfer. This starting-condition correction does not certify that every OS association failure is fixed.

On this iOS version, the screenshot of an existing reader can show its content while its accessibility tree omits the iframe subtree after visiting SpringBoard (`e827428d`). Sender content, UUID and full-file verification therefore occur before leaving the reader; the disconnected-network precondition follows immediately before publishing. No content or byte gate is skipped. On Android, the runner hides the native IME and proves its insets are hidden before obtaining fresh Download geometry; `37095dea` exercised that boundary successfully.

## Bounded automated Android→iPhone acceptance

Use the existing prepared-artifact workflow, not a hand-operated Xcode build/install sequence. The local config selects the exact phone, retained SDK and offline signing source. Its `nearby` object provides `direction`, `archive_id`, `file_name`, `title`, `expected_text`, `peer`, a frozen host `source`, and `android.device` plus `android.prepared`. The Android receipt pins its optimized target APK, signer, instrumentation identity and host/Java/manifest/inspector source hashes; stale recipes fail before phone access. Choose an archive missing from the receiver; never delete a user's archive or cancel unrelated jobs to force a test.

```sh
npm run iphone -- --config .native/iphone-config-device.json \
  --mode nearby --prepare-only
```

Preparation must finish without contacting the phone. After independent review, use the dedicated iPhone's standing authorization recorded in `AGENTS.md`; do not wait for another per-run approval. Other devices still require an authorized hands-off window:

```sh
npm run iphone -- --config .native/iphone-config-device.json \
  --prepared .native/iphone-tool/prepared.json \
  --start-device-window --window-minutes 10
```

The same command starts Android publication for `android-to-iphone`, or Android reception for `iphone-to-android`. Both require initial STA-off/no-AP evidence; publication readiness additionally requires real temporary-network/sharing UI and a current OS AP frequency. The iPhone runner proves disconnected Wi-Fi with Bluetooth on, handles app-scoped permissions and at most one system Join, then requires catalog, download, managed import and the selected reader. The host independently checks UUID, size and actual received file SHA-256 against the frozen source. Provider-backed Android publication explicitly does not claim a pre-transfer source hash; final iPhone byte verification remains mandatory. `verify-received` is a separate Android-free check of an exact old job, not a transfer.

Runtime authorization and deadlines are injected into the frozen XCTest plan. There is no permanent device authorization or infinite deadline in the scheme. Offline app preparation rejects an online-verification-only runner profile; runner signing pins and verifies the selected profile bytes rather than allowing automatic signing to substitute another profile.

The run stops on the first unexpected state and records bounded cleanup in its private `result.json`. Announce **DEVICE RELEASED** only after owned host/remote automation is confirmed stopped. Existing archives, settings, unrelated tabs and downloads remain intact. Received archives and their newly opened tabs are retained.

Publisher shutdown uses the nonce-bound `STOP_PUBLISH` path, the real Stop sharing control, preserved state, ordinary app relaunch, and final OS no-AP proof. A repeated Join, association failure, missing peer or uncertain cleanup is a failed run, not an automatic retry loop.

## Bounded automated Apple LAN pairs

Use the same `npm run iphone` preparation and bounded-window entrypoint. Select
`nearby.direction` as `mac-to-iphone` or `iphone-to-mac`, set
`nearby.topology` to `apple-lan`, and provide `nearby.mac.executable` and
`nearby.mac.browser` as canonical absolute paths. No Android object is required.
Keep the existing exact fixture UUID, wire filename, actual metadata title,
expected article text and frozen source fields. Source basenames may contain
timestamps even when the published wire filename does not.

Preparation pins the real Mac executable, Chrome executable, Node identity,
Puppeteer version, helper and lockfile. Each run owns a fresh Mac app/browser
profile and contained download directory. Do not reuse a normal user profile.
The handset source's installed peer ID and publication generation bind Mac
discovery; the selected session must match the browse receipt. Received jobs
must link to their actual managed installation, full-file hash and readable
reader. Existing receiving archives or conflicting download jobs are rejected.

```sh
npm run iphone -- --config .native/iphone-config-mac-pair.json --mode nearby --nearby-direction mac-to-iphone --cache .native/iphone-mac-receive --prepare-only
```

Use the exact printed next command, including its selected config and device.
Prepare each direction separately. Apple LAN runs retain available connectivity
and reject temporary-hotspot Join prompts; they do not use the routerless
precondition's skip environment variable. Evidence identifies Chromium-backed
real headless Kiwix, not a native macOS WKWebView, measured AWDL routing or radio
throughput. No physical pair passes without its transfer/readability/preservation
and owned cleanup results.

### Two iPhones

Select `nearby.direction: "iphone-to-iphone"` with `topology: "apple-lan"`.
The main config's exact device is the sender. Set `nearby.iphone.config` to an
ignored receiver build config with a distinct exact device, SDK and signing
source, and `nearby.iphone.cache` to its separate canonical artifact cache.
Keep `nearby.mac.executable` and `nearby.mac.browser`: this isolated Mac peer
only observes discovery and the selected source inventory; it never relays,
downloads or publishes archive bytes.

Preparation builds both signed app/runner pairs off-phone and freezes their
manifest hashes, exact devices, common source revision, fixture and transitive
helper artifacts. A single maximum ten-minute window includes both preflights,
installations, UI, transfer and cleanup. Obtain authorization for both devices;
the dedicated iPhone's standing authorization does not cover a personal phone.

The sender remains foreground and sharing while the independent observer
binds its actual peer ID, generation, session and opaque archive offer. Only
then may the receiver select that sender through Nearby. Completion requires
the receiver's new managed installation, full-file hash, active reader and
download journal to match that observed publication. Both phones and the
observer must preserve prior state and finish owned cleanup before release.
An unexpected Join prompt fails this Apple-LAN scenario. This workflow does
not certify a particular infrastructure/AWDL route or RF throughput.

## Reviewed Android startup crash reports

Unexpected startup reports stop the sharing scenario. The Android helper's
`capture-crash` mode reads only the exact installed application's bounded native
crash record without launching an activity or injecting input. Retain and inspect
`crash-report.json` and its identity receipt before any subsequent run.

If the report is an already diagnosed harness failure, prepare a fresh Android
tool receipt with its exact `--reviewed-crash-sha256`; update the ignored paired
config and reprepare the iPhone receipt. The frozen Android preparation checksum
is checked on the same bytes consumed by the child. Only that exact report can
be viewed and closed through ordinary UI, with motion-free bounds and an exact
native accessible target. Different records still fail. Nothing is deleted,
acknowledged or emailed; closing the prompt is session-local, so a normal
relaunch may display the retained report again.

## Transfer and performance checks

Parallel acquisition workers must share the listener's bounded admission rather than open four connections against its two-per-IP cap. Completion must consume the final frame and ACK even when the caller stops reading at Content-Length. Sender source work must drain before admission is released, and slot release must precede the remote close used as the client's handoff.

An alive, retained, uncancelled response cannot currently be distinguished from a deliberately slow consumer. Such waits remain count-bounded, not wall-clock-bounded; cancellation, dropped responses and connection loss retire them. Permanently hung provider I/O retains its admission until the actual source operation exits rather than releasing its resources early.

### Physical baseline, 2026-11-07

Native source: `7cc77499c0935fa4f7d6d1ce9a22a5501cd4d004`. Android used the optimized release with its station connection disabled and its app-owned LocalOnlyHotspot active. iPhone joined using BLE-supplied credentials; no password was entered. Android's current AP diagnostic reported 2462 MHz, Wi-Fi standard 4.

| Direction / archive | Native source | Received bytes | Observed interval | Effective rate |
|---|---|---:|---|---:|
| Android→iPhone, Wikipedia 100 | `7cc7749` | 333,429,922 | 127.384257 s, job admission→publication | 2.618 MB/s |
| iPhone→Android, synthetic city | `26fa9d4` | 201,387,380 | 44.264 s, Download input→Open observed | 4.550 MB/s |

The interval comes from the exact QUIC-library job `7a98ecb4-e5d6-416e-a9b6-4003fa63d2d7`: admission through completed publication, not isolated radio throughput. The actual managed file was copied back independently; its SHA-256 was `2eb7fd115bfb8644a08a9482f01f52b507428b1b8a30129fead92f7b6715440b`, matching the frozen sender source, and `zimcheck --checksum` passed. Library UUID was `dd8a2bd2-c05b-ac6c-8074-6c30e14803fb`. The matching run, `device-nearby-8d81ff85d7fd41f28e2356415eae3b65`, still failed its UI completion gate; no readable-reader or reverse-transfer success is claimed from it.

Run `device-nearby-bc50f28db5634638a643a3ee1d86f1f8` completed the bulk reverse transfer with one automated Join, both phones initially off-router, Android download/upload limits both zero, iPhone upload limit zero, and a current 2462 MHz AP. The actual received SHA-256 was `7976468a60744bafd4fc91b02bea3fba43bfad24de28b3400cefd5fdd20d68d9`; active UUID was `c1dab55f-229f-569d-934d-7145dd451cec`. The inspected Android screen displays **Knowledge orchard** and its article body. Its input/Open timestamps are UI observation bounds including verification/import, not isolated radio speed. These two rows are not a controlled improvement comparison: directions, archives and interval definitions differ.

A historical same-host release benchmark measured 61.2 MB/s before final flush and 56.0 MB/s including flush. It did not measure either phone or the current radio. Both routerless directions use interface-bound Quinn sockets, not the Apple byte-stream bridge. Source inspection also found redundant Unix metadata reads: content identity now uses one fresh descriptor snapshot for device/inode/size/mtime/ctime, retaining both before/after checks. This removes work without establishing a measured phone throughput gain or attributing the radio's bottleneck.

### Android release packaging

The Android frontend must be compiled with `TAURI_ENV_PLATFORM=android`, and that exact output must be supplied as `frontendDist` when a native build skips `beforeBuildCommand`. Reusing a generic desktop `dist` makes the release frontend omit its Android activity/window authority and the boot guard correctly rejects it. Do not weaken that guard to accommodate an incorrectly packaged bundle.

## Diagnostics

```sh
KIWIX_IPHONE_UDID=<explicit-iphone-udid> sh scripts/nearby-debug.sh pull
adb logcat -d -v time -s KiwixNearby RustStdoutStderr KiwixHotspot
# Debuggable Android builds only; optimized releases reject run-as:
adb shell run-as org.kiwix.workbench cat files/kiwix/nearby-debug.log
```

Keep complete build, device, XCTest and transfer outputs. Do not interpret `local=true` in a refusal log as an off-link rejection: it means the peer passed that check; request-capacity evidence must be inspected separately.
