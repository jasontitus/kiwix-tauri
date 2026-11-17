# LocalSwarm integration

Nearby has one default workflow: the sender chooses **Share all**; the receiver
opens **Nearby**, chooses the sender, then **Download all** or an individual book.
The library uses encrypted QUIC, without invitation codes, PINs, transport
selection, or sender approval. Sharing starts only on explicit request, never
on application restart. Reading/searching local archives requires none of this.

## Share all and Receive

**Share all** is the first Nearby action. It snapshots available installed
archives, advertises one library, and prepares archive sources lazily when
requested. It does not hash or copy the whole library before advertising.
Newly received archives are not automatically added to an active offer.

Opening Nearby explicitly starts receive-only discovery. Choose a device to
browse its inventory, then download one archive or **Download all**. Ordinary
receiving has no code, PIN, address, or port field. Bluetooth discovery and
system Wi-Fi joining, where supported, assist that same flow; only actual
system permission/join prompts require additional interaction.

Discovery labels and publisher codes distinguish devices; they do not prove
personal identity. Archives transfer over QUIC, not Bluetooth. The reader and
installation checks continue to treat received archives as untrusted data.
The server exposes only admitted archive reads, not arbitrary files, writes,
application IPC, or device-control operations.

**Stop finding** stops discovery without cancelling an accepted connection or
download. **Stop sharing** retires the offered library and its active streams.
**Stop all** additionally retires pending Nearby intentions and pauses nearby
transfers. Web sharing has an independent start/stop lifetime.

Desktop peers use LAN/Apple peer-to-peer discovery; mobile Bluetooth can assist
connection and temporary-network setup. Source support is not physical
interoperability evidence. The latest actual checks, fixes and remaining
phone-pair/routerless gates are recorded in [RELIABILITY](RELIABILITY.md).
No maximum-throughput claim follows from this workflow.

The current isolated Linux–Mac run reached genuine native discovery and
publisher selection, but the NAT VM's inventory request was rejected by the
on-link QUIC-route check. No transfer or Raspberry Pi acceptance follows.
The Linux numeric QUIC path requires a directly on-link peer; a private IP
alone does not grant route authority. A Pi directly on the same subnet is an
unverified test target, not a certified appliance. The privileged `--headless` administration
URL remains loopback-only and must not be published as a sharing endpoint.
See [the scoped route-boundary evidence](RELIABILITY.md).

Final host acceptance completed two concurrent selected downloads with exact
SHA-256, selected-only imports and reader Main/internal detail navigation;
Download all then skipped the two installed archives and added the remaining
archive. Earlier native bulk timeouts are retained as pre-fix evidence.
Both reviewed final mobile packages were installed in place with state preserved.

The Samsung dual-stack **Share all** failure was fixed: public metadata now
supports numeric ULA IPv6 hints and up to 16 supported hints; global/remote,
mapped, scoped and multicast hints remain rejected. Numeric library admission
and retained receipts require actual on-link prefix proof, not merely private
address classification. The in-place update passed actual Samsung publication,
advertising, eleven-archive inventory browsing and verified Samsung-to-Mac
archive transfers. Two active-payload measurements were 16.86 and 18.61 MB/s;
these are not phone-pair or end-to-end import rates.

The corrected iPhone package is installed with prelaunch state preserved. An
earlier launch/transfer run was blocked by lock/authentication cancellation; a
later Samsung→iPhone Hawaii transfer completed (150,605,015 bytes, SHA-256
matched, 5.186723 s, ~29.04 MB/s) as recorded in [RELIABILITY](RELIABILITY.md).
Reverse, routerless and throughput gates remain open.
Phone-pair throughput, original-file intake and OS permission/join acceptance
remain open.

### Other ways to share

The collapsed **Other ways to share → Share to a browser** opens Settings.
**Web sharing** is separately and explicitly enabled there for browsers/OPDS
readers. It uses unencrypted HTTP and visibly warns to use a trusted network.
It is not the default app-to-app path and does not advertise a competing
Nearby library.

**Share a book instead** retains selected-archive LocalSwarm v1 interoperability.
Its optional PIN is scoped to that compatibility offer, not ordinary library
receiving. BitTorrent sharing remains a separate explicit action.

The user-requested [LocalSwarm integration feedback](../../localswarm/docs/KIWIX_INTEGRATION_FEEDBACK.md)
records protocol/API/platform boundaries and upstream capability proposals.
That link requires the sibling checkout.

### Download all

**Download all** queues the current inventory serially through the normal
per-archive admission path; the download manager retains its existing transfer
concurrency. Installed archives and already-active downloads from this exact
session are skipped. Missing-file library entries are reported separately:
use Locate in Library or remove that entry before downloading it again.
Matching names alone never authorize replacement or overwrite.

Keep the inventory open while archives are being added. **Stop adding**, closing
the inventory, navigation, or retirement of its peer/privacy/lifecycle authority
stops further admissions. Already accepted jobs remain tracked in **Downloads**;
**View downloads** opens them explicitly. Existing mobile background/network
policy still applies, and **Stop all** retains its stronger pause/revocation
behavior. An admission error or declined space warning stops the remaining
batch and shows the partial result. Retry Download all, or use individual
downloads, after resolving the problem; stale sessions require a fresh Browse.

This mirrors a snapshot by adding missing archives. It does not delete unrelated
local archives, synchronize future changes, or automatically share received
files. New batch copy is cataloged in English and falls back to English where
translations are missing.

### Publisher names

Nearby shows a privacy-preserving platform label by default: Mac, Windows,
Linux, Android, or iPhone / iPad. **Nearby device name → Save device name**
sets an optional nickname for newly started shares. Leaving it blank restores
the platform default. Account names and OS computer/device nicknames are not
automatically used for this publisher-name TXT hint. This does not anonymize
DNS-SD: Apple selected-archive QUIC still uses system-host records, which can
expose the existing OS hostname. Unsubmitted typing is not saved.

The `device` DNS-SD TXT hint is independent of archive titles, stable publisher
IDs, rotating library sessions and withdrawal handles. Protected archive offers
keep their title hidden. Receiver rows show the complete publisher/discovery
identifier as well as the name. Choose's accessible label includes that code,
and the selected inventory retains it visibly. Older peers without the name
hint retain their existing labels. Neither the nickname nor the code proves
personal identity or authenticates the shared-certificate QUIC transport.

Names are bounded to 160 UTF-8 bytes; controls and direction overrides are
rejected before entering the settings queue. Accepted writes retain the normal
shell-owned retry path. A subsequent publication waits for the accepted name
save, and Stop all cancels a start still waiting on that save. Active shares are
not silently renamed. The six new source strings are cataloged in English;
other locales currently fall back to English for this copy, not reviewed
translations.


### Scope, authority and lifecycle

The public library is open to nearby downloaders after explicit Share all.
Encryption does not authenticate the human sender. Only the selected current
publisher/session and a retained exact local network route authorize a browse
receipt. Receipts and live download grants are memory-only; persisted jobs keep
their durable download identity and owned staging, not reusable network
authority. Re-browse the same selected book to authorize a paused job again.

Membership is bound to installation and retained file/provider principals.
Removal, replacement, revocation, source changes, Stop, privacy/read-only
transitions, window retirement and loss of the selected route close the
affected authority. Changes to an unrelated interface do not revoke a healthy
selected route. Native association alone does not establish a working QUIC
connection.

iOS sharing remains subject to foreground and finite background-task policy;
do not assume an indefinite background server. Android foreground-service
accounting includes the library listener and preserves actual discovery state.
Notification-display permission is not a prerequisite for that service.
Physical automatic joining, cancellation and route-loss scenarios require
their own phone-pair acceptance.

Web sharing independently supports the entire library or selected archives,
an optional memory-only PIN, local interface/port choices, browser links and
OPDS. Its PIN limits access but does not encrypt HTTP. Both enable buttons
honor the configured selection; Stop remains available independently of that
selection. A hotspot is released only when no live Nearby/Web owner uses it.



## Existing archives and cleanup

Whole-library inventories carry archive UUIDs. An already installed archive shows
**In library → Open**, not Download. If its installed file is unavailable, the
action returns to Library to locate or remove that entry first.

Selected-file QUIC manifests may carry only an advisory archive UUID (see
below), which Kiwix does not trust. Kiwix inspects verified
chunks covering the ZIM identity prefix, including split-part boundaries, before
creating a transfer or staging files. This is bounded network metadata work, not
a promise of zero received bytes; an individual protocol chunk can be 16 MiB.
The probe has a 30-second deadline and honors admission/cancellation. With an
empty library there is no duplicate to check, so no identity chunk is requested.
Native start commands recheck independently of the displayed inventory. Final
import collision checks still reject races and misleading UUID claims; matching
a UUID never authorizes replacement of an installed reading copy.

Deleting a disposable duplicate does not remove the installed original at a
different owned path. Cleanup retains publication ownership and byte verification;
empty library-removal preparation avoids an extra full scan, and the dialog
shows working feedback. The exact cause of the physical Hawaii delay was not
captured. Disposable-host and native-Android evidence is in [RELIABILITY](RELIABILITY.md).

### Reusing partial downloads across transports

A LocalSwarm receive and a BitTorrent acquisition of the same archive reuse
each other's unfinished bytes; the other job's staged file is only read.

- **BitTorrent → LocalSwarm.** Each receive (start and every resume) first
  reads unfinished, unshared torrent jobs from saved history and scans their
  staged files of the same size as a manifest file. A chunk is adopted only if
  its SHA-256 equals the manifest hash, exactly as for a network chunk; it is
  then written, flushed and checkpointed before any network request. All-zero
  regions compare against a cached zero-chunk hash, so unwritten sparse ranges
  cost no hashing. A donor whose first 64 fully written candidates (no
  all-zero 16 KiB block) all fail is abandoned as different content. Donor
  I/O errors skip the donor. The space check and its acknowledgement run
  before any adopted chunk is written. Every resume rescans the donor for
  chunks that are still missing.
- **LocalSwarm → BitTorrent.** On a torrent's first run only, before the engine
  opens storage, each payload file whose size matches an unfinished LocalSwarm
  receive is created through the confined factory (so ownership is journaled
  as for an engine-created file) and seeded with that receive's durably
  verified chunk ranges. With fast resume disabled, the engine's initial
  check hashes every piece and counts only pieces matching the torrent
  metadata; that librqbit behavior is relied on, not exercised by these tests.
  Even a small prefill makes that first check read the whole file. On Windows,
  a LocalSwarm receive that is running holds exclusive locks on its staged
  files, so its bytes are reused only while it is paused or stopped.

Filenames, sizes, saved job metadata and archive UUIDs only select
candidates; acceptance always comes from the receiving transport's own hashes.
Donors are opened read-only without following links and must be singly linked
regular files of the exact size. Both jobs keep their own staged copy, so the
space for both copies is still required while both are retained. Reuse is a
best-effort optimization: errors reading another job's file never fail the
transfer, while errors writing the receiver's own staging fail it as they
would for downloaded bytes.
### Advisory manifest identity

The selected-file manifest accepts an optional advisory `zim.uuid`
(`../localswarm/PROTOCOL.md` §5.1), validated as a canonical lowercase UUID
and excluded from the content address, and ignores unknown top-level keys so
reference senders that emit it interoperate. Kiwix neither emits nor persists
it (saved job manifests stay readable by older builds), and
duplicate detection still reads the identity from verified chunks; the claim
is never authority. Builds before this change reject any manifest carrying an
unknown key, so senders must not emit it to them.

## Protocol and transport

The implementation follows the actual `../localswarm` Swift and Go engines, not only `PROTOCOL.md`. Protected shares require authentication before manifest/chunk access. This build's clients use message types **9–11**: the client sends a fresh nonce (type 9), the seeder answers with its own nonce and an HMAC-SHA-256 proof of the PIN over both nonces (type 10) *before* the client discloses anything PIN-derived, and the client then proves the PIN the same way (type 11). Nothing sent is the deterministic credential, so a captured frame cannot be replayed on another connection.

The legacy message type **8** token — the lowercase hexadecimal SHA-256 of `swarmID + ":" + PIN` — is still accepted by this build's seeder for older clients and the reference engines, which send only that frame; it is never sent by this build's client because any peer that receives it can brute-force a short PIN offline. Consequently a PIN-protected transfer requires this build on the receiving side: an older seeder or a reference-engine seeder rejects types 9–11. A low-entropy PIN is still guessable offline by an active on-link peer that obtains one proof (any PIN-derived MAC has that property); closing it fully needs a PAKE and a negotiated protocol version. The documented message types 1–7 alone are insufficient.

The LocalSwarm chunk protocol uses **QUIC**: TLS 1.3, ALPN `localswarm-v1`, and exact reference shared-leaf DER pinning. Quinn uses a private Rustls provider; Apple Bonjour peers use Network.framework and the same pinned identity. macOS creates the identity in memory through public Security APIs rather than importing a key into the user's keychain. The shared identity is application gating, not individual-client authentication.

Library QUIC uses a bounded extension in `localswarm/library_wire.rs`: JSON
control/reply frames followed by length-framed payload segments over a real
QUIC stream. Controls are limited to 8 KiB, inventories to 4 MiB/10,000 books,
and payload segments to 256 KiB. Session/publisher identity, advertised length,
range bounds and exact payload completion are checked before admission.
The existing sequential/segmented acquisition readers share cancellation,
bandwidth, durable checkpoint, final archive verification and no-overwrite
publication rules through `transfer_response.rs`; this is not HTTP tunnelling.

Each library request owns its QUIC transport. After the response payload and
application ACK are consumed and source work drains, the Apple listener retires
that exact epoch-fenced connection group, not just its stream. Generic selected-
file swarm listeners retain multiplexed behavior. Quinn completion accepts only
an uncancelled remote application close with code zero or a received transport
`CONNECTION_CLOSE(NO_ERROR)`; local transport errors, resets, timeouts, nonzero
codes and bare EOF do not establish completion. One classifier governs the
receive-error and final-close checks.

Acquisition status records transport separately from synthetic library URLs:
new admissions retain `quic-library` through completion/restart. Older library
history without provenance reports `unknown`, not guessed HTTP or QUIC; ordinary
legacy HTTP(S) remains HTTP. Status snapshots capture job/provenance atomically.
Labels grant no network authority and do not change selected-book PIN handling.

Discovery distinguishes `tp=quic-library` from selected-file `tp=quic`.
Apple publishes its supported `awdl=1` capability; non-Apple parsers accept it
while retaining numeric endpoint and property validation. Apple uses scoped
mDNSResponder/Network.framework services; other targets use bounded `mdns-sd`
and Quinn. Effective socket addresses and native interface receipts, not
Quinn's server-only `Connection::local_ip()`, retain client route authority.
Native interface names are copied while their enumeration callback owns them.

The checked-out reference app already supports multi-file manifests. Kiwix's
selected-file receiver deliberately admits one ZIM/split cohort per durable
job; the library extension adds an inventory and independently validated
per-archive jobs rather than pretending the reference can only share one file.
Mobile BLE connection assistance is Kiwix integration code, not a claim of
wire compatibility with the reference's credential-only bootstrap.
Neither source coverage nor infrastructure interoperability proves routerless
radio behavior. Camera/location consent remains separate.



### Throughput measurement boundary

Chunk requests reuse the authenticated QUIC connection and its single bidirectional stream; there is no intentional per-chunk TLS handshake. The receiver keeps up to `PIPELINE_WINDOW` (32) requests outstanding per connection, reads responses in request order into an 8-deep handoff bounded by a 64 MiB prefetch cap per job, and runs each chunk's SHA-256 on the blocking pool one response ahead of the handoff, so verification overlaps the socket instead of sitting between the socket and the staged write. The seeder answers up to `SERVE_WINDOW` (16) reads and responses per peer under a 16 MiB per-peer buffer cap. Upload source reads and checksums precede each response. These are separate possible contributors to transfer time, not evidence of a Wi-Fi link ceiling.

Unlimited uploads submit at most **256 KiB** per send rather than awaiting a native completion after every 16 KiB. The Apple bridge remains sequential: one send request is outstanding per peer, and native dispatch data synchronously owns a copy independently of Rust cancellation and QUIC acknowledgement. This reduces the required payload send callbacks from 64 to 4 per MiB without changing framing, authentication or checksums. Capped upload budgets and all payload receive reservations remain at most 16 KiB. A newly lowered cap can take effect after the already submitted unlimited batch; cancellation and network-policy revocation still close the underlying connection. These are application submission bounds, not a claim about Network.framework's internal retained heap.

An ignored native regression, `localswarm::network::tests::apple_sender_serial_quinn_throughput_measurement`, measures three 128 MiB payload trials over one production Apple-sender/Quinn-receiver connection. It prints verified bytes and elapsed nanoseconds after connection setup. Run it with `KIWIX_LOCALSWARM_TEST_HOST` set to an actual local Wi-Fi/Ethernet IPv4 address; loopback does not satisfy Apple's production local-link gate. `localswarm::tests::receive_path_throughput_measurement` measures the whole receive path, and `KIWIX_LOCALSWARM_BENCH_SEED_APPLE=1` makes it serve through the Network.framework bridge instead of Quinn, which is the shape a Mac actually offers. Same-host results isolate transport/callback work but do not establish physical iPhone-to-Android throughput, radio behavior, staging/checkpoint cost, or background execution. Compare unchanged payloads and build profiles before and after any transport change; retain raw timings rather than interpreting an idle Wi-Fi receive link rate as transfer capacity.

The retained macOS debug, same-host comparison transferred three 128 MiB trials per variant. Aggregate rates were 6.693924 versus 6.809449 MiB/s for 16 versus 256 KiB submissions: a small observed difference, not an established causal or statistically significant speedup. Raw per-trial durations and calculation inputs are retained in `.native/swarm-session-peer-20261030/host-transport-timings.json`. This transport-only result does **not** explain or resolve the reported approximately 2 MB/s between the physical iPhone and Android. Recorded phone-build logs identified unoptimized Rust development profiles; that remains a hypothesis for the physical slowdown, not its diagnosis. Controlled phone comparisons must hold source, receiver, archive, network and limits constant, separate setup/import time, and measure hashing, payload send/receive, durable writes and checkpoints before attributing the bottleneck.

Same-host measurements on the development Mac over its own Wi-Fi IPv4 address, 1 MiB chunks, unlimited budgets, `--release --features localswarm-profiling`: the production receive path moved 92–95 MB/s end to end (97–100 MB/s before the final durability flush) with the Network.framework seeder and 58–60 MB/s with the Quinn seeder; the Apple-sender/Quinn-receiver pipelined serve rate was 108 MB/s; the seeder serve cadence with 50 ms source reads was 14.9 MB/s at a full 16-chunk window, where serialized reads would be about 1.3 MB/s. The same benchmarks in the debug profile are SHA-256-bound: 32–34 MB/s receive (7.7 MB/s before chunk verification moved off the socket) and 21 MB/s Apple serve rate. Both profiles therefore exceed the 46 MiB/s the Wi-Fi link carries, so the remaining physical Mac-to-Android gap is not this Mac's send or receive code.

Tested and not adopted because same-host throughput did not change: 1 MiB versus 256 KiB send submissions, and BBR versus Cubic congestion control (no loss on the same-host path, so the controller is not observable there). A single stream remains deliberate: multiplexing chunk responses across streams would change the wire contract, and the request window already keeps the connection busy.

Parallel connections per source were implemented and then removed. The reference engine credits parallel connections for its LAN rate, so the receiver dialed the same validated endpoint twice and handed each connection disjoint chunk ranges, keeping one stream per connection and sharing verification, checkpoints, the prefetch byte cap and the rate budget. On the host it was a clear gain (release, Apple seeder, 256 × 1 MiB, three runs per arm: 99.1 → 125.7 MB/s before the final flush, 94.3 → 108.0 MB/s flushed, `socket_wait` above 100% of wall time from the two concurrent sessions). On the physical Samsung, against the same large-eng-base archive, seeder, Wi-Fi lock and sender-timed 1-second counter, two connections regressed the transfer to 4.65 MiB/s active window (4.62–4.67 MiB/s full-file bounds, 1.87 MiB/s for the first 44 s) against 10.58 MiB/s for one connection, with `effective=2` proven in release logcat and up to four concurrent sessions at the seeder. The phone's shared Wi-Fi link is the bottleneck, so a second in-flight request window only adds airtime contention; the same change that removes CPU serialization on a Mac therefore has to stay off on the phone. It was reverted rather than kept behind a flag.

## User flow and lifecycle

The ordinary flow is **Nearby → Share all** on the sender and
**Nearby → choose sender → Download all** on the receiver. Finding sends local
discovery traffic and can reveal network presence, but does not offer local
contents, start downloads, or automatically re-share received files.
Opening the Nearby surface starts discovery once; it does not start sharing.
Restored windows and process restart do not grant sharing intent; **Share all**
remains explicit.
**Details → Share** explicitly offers one archive through the compatibility
QUIC path; **Share as BitTorrent** remains separate.

The common Downloads list exposes verified progress, errors, pause/resume/cancel and import state. `localswarm:` job IDs are distinct from swarm content addresses and archive UUIDs. Protected transfers require the PIN again after pause/restart; PINs are memory-only. Opening Nearby starts discovery; sharing, receiving, and offering still require explicit actions. Stop LocalSwarm stops discovery/offers and pauses transfers. Private and strict read-only transitions retire actual network/writer tasks before committing the mode change; they do not merely hide UI controls.

### Discovery, sessions and temporary Wi-Fi

Share all advertises a **session-specific** DNS-SD instance
(`Kiwix-Library-<publisher-id>-<session-prefix>`) with an optional monotonic
hint. The receiver keeps the highest hint first and exposes older sessions
under a disclosure; an unauthenticated hint never removes the only selectable
sender. A restarted share is therefore a new record rather than a stale TXT
value, and an already-open catalog rebinds or closes when the same publisher's
session changes.

Android can bind the library listener to its app-owned LocalOnlyHotspot AP and
publish the endpoint and join metadata over BLE; the iOS receiver joins through
the system Wi-Fi prompt. **Use temporary Wi-Fi** is the explicit path for
devices on different networks. The Mac remains LAN/Bonjour-only and cannot
host an app-owned hotspot. A peer on a different subnet or a VPN source is
refused by the Android listener; use the same LAN or the temporary Wi-Fi path.

**Share all never creates an access point.** When native network state reports
no usable local Wi-Fi/Ethernet, it fails with `NEARBY_LAN_REQUIRED` instead of
silently switching topology; the UI directs the sender to **Use temporary
Wi-Fi**. A silent switch broke LAN publication and advertising, so the explicit
route owns access-point creation.


Stop all retires pending intentions across settings/low-space waits and rejects stale admissions across root windows. Successful already-admitted responses still reach root job tracking after navigation; cancelling a dialog does not discard an accepted job. Private/read-only transitions retire actual network/writer tasks, not merely UI controls.

Source preparation retains verified file/directory identities. Removing, relinking or moving an offered archive holds admission closed through offer retirement and the library mutation. File metadata or installation-principal changes revoke the whole offer, including buffered/throttled uploads, and withdraw its advertisement. Late, uncancellable native verification workers retain read-only authority only; they cannot publish files, update Store, or advertise.

iOS screen-awake admission covers meaningful receive work and active outgoing
payload sessions, plus an explicitly open library session under the existing
library lifecycle policy. Idle selected-file advertising/inspection alone
does not keep the screen awake. Assertions release at completion, cancellation,
error, privacy/read-only retirement, shutdown or actual backgrounding.
Transient inactive UI such as Control Center is not background retirement.

Explicit locking or switching apps is not blocked. The genuine background notification reserves one supported, finite `UIApplication` background task. Reader storage flush, sensor revocation and reader/source retirement remain prompt and independent. Only already admitted transfer work receives bounded grace; new outgoing sessions and new receive dials are denied. Cleanup starts before the OS-reported deadline to allow cancellation, verified-chunk checkpointing and worker joins before the task ends. The OS task still ends at its deadline if a native join has not completed: the service retains that retirement owner, and foreground admission must await it rather than abandoning I/O or claiming it finished. Earlier OS expiration remains authoritative and can prevent cleanup from completing; recovery uses the last durable verified prefix. Returning before retirement starts supersedes the grace waiter without stopping an active transfer. Stopped receives retain verified chunks for explicit Resume, including PIN re-entry for protected shares. No audio/location entitlement or indefinite background QUIC behavior is claimed. Physical-device sleep/lock/expiry acceptance is separate from source implementation.

The same iOS assertion and finite task also cover actual map download/composition workers, not map catalog browsing. Background retirement and application shutdown pause map work through its durable verified-prefix checkpoint rather than treating sleep as an explicit destructive Cancel. At background admission, LocalSwarm and map activity are evaluated independently: an already-idle LocalSwarm offer stops even when an active map receives grace. Privacy/read-only transitions and explicit Cancel retain their separate cancellation fences.

Android holds one process-wide `WifiManager` Wi-Fi lock through the same counter-backed RAII guard that macOS uses for its activity assertion. The lock is requested as `WIFI_MODE_FULL_HIGH_PERF`; since API 34 the platform documents that this deprecated mode is automatically replaced with `WIFI_MODE_FULL_LOW_LATENCY`, which is what `dumpsys wifi` shows (`type=4`, counted as "full low latency"). It is held for admitted receive work (after the bounded worker slot, through prefix rehash, transfer and final commit) and from the first authenticated chunk request of an outgoing peer until the last such peer is gone; it is released with the last holder on completion, error, cancellation, privacy/read-only retirement and shutdown. It uses the existing trusted mobile plugin bridge, needs no permission beyond the already-declared `WAKE_LOCK` and `ACCESS_WIFI_STATE`, and does not change transfer semantics. A same-archive A/B on the Samsung (1,073,802,614-byte `large-eng-base`, sender-side first-nonzero→last-partial counter delta) measured 8.17 MiB/s without the lock and 10.58 MiB/s with it (+29.5%, one run each); `dumpsys wifi` showed the lock held for 107.4 s across the transfer and released afterwards. That remains below the 201 MB city-eng-base baseline (13.31 MiB/s), so file size, ramp and run-to-run variance are unresolved confounds.

## Trust model

The shared certificate provides encrypted QUIC and application gating, **not trusted-peer or publisher authentication**. Public library access needs no PIN. An optional compatibility/Web PIN restricts access, not human identity; Web HTTP remains unencrypted. Chunk/content checks and ZIM structural/MD5 validation detect inconsistency or corruption, not whether the publisher is trustworthy.

Use a trusted network and verify the sender/content separately. This implementation does not claim to resist an authorized sender distributing a malicious but structurally valid archive; the reader's separate untrusted-content boundary still applies.

## Hostile input, staging and publication

The selected-file v1 protocol bounds frames at 64 MiB, manifests at 32 MiB, chunk count at 262,144, and chunk size at 16 MiB. Validation recomputes the content address and checks exact chunk ranges, byte totals, hashes, Unicode-normalized/case-folded path collisions, reserved names and platform-unsafe paths before file creation. Each job admits one root ZIM or one complete contiguous split-ZIM cohort; the library extension queues independent jobs for its inventory, not arbitrary directory writes.

Private capability-bound staging rejects symlinks, hard links and replaced directories/files. Verified availability alone is persisted. Resume rehashes retained data rather than trusting saved bits. Writer handles are irreversibly sealed before final rehash/native verification. Windows uses an identity-guarded read-only handoff; POSIX uses retained descriptors. Verification covers the exact retained bytes, including a split stored MD5, then every native structural check. The matched schema-2 libzim/Xapian SDK, with split-range, retained-descriptor and explicit-index-offset corrections, is mandatory; see [PACKAGING](PACKAGING.md).

Publication is an owned, atomic, no-overwrite directory transition. Installed paths, missing registered paths and Windows filename aliases remain reserved. A partial/corrupt transfer is never registered. Cancel/history removal does not grant authority to delete unrelated destination files. A verified transfer enters the existing acquisition-import pipeline; a UUID collision must remain a visible, retryable import failure rather than replace an installed principal or block unrelated imports.

Resource limits include 64 retained jobs, three concurrent receives, four source/session-dial slots per receive, eight active offers and sixteen peers per offer. Saved history is serialized under a 64 MiB byte/allocation budget rather than allocating an oversized JSON document before checking it. Failed pre-commit initial admission removes the unstarted job; a post-rename durability failure retains its visible job instead of pretending the durable record was rolled back. Error responses identify retained private staging. Immutable manifests are shared rather than deep-cloned across state/setup/verification workers.
 
Upload request parsing remains active while responses are throttled, with bounded pending queues. Useful reference-client requests/Have frames are not rejected merely because an unlimited transfer is fast. Duplicate control traffic and untrusted allocations are bounded. Receive heartbeats retain a truthful all-false upload bitfield, so a throttled receiver does not become a source.

The vendored `mdns-sd` 0.17.2 fork bounds cache entries/owners/payload, reverse indexes, pending removals, resolver/retry/probe state, packet-processing turns and subscriber queues. Pressure eviction emits removal before stale peers can remain authoritative; full subscriber queues retire the subscription instead of blocking the daemon. DNS name expansion/compression and TTL arithmetic are bounded. Upstream license files are retained. These are scoped hardening changes, not certification of every third-party dependency.

## Historical selected-file integration evidence

These observations used the real app Rust service and actual reference engines, not mocked peer responses. The application was built on macOS arm64; the Linux peer was a real Alpine aarch64 QEMU VM, not a Linux build of the Tauri app.

| Scenario | Archive bytes | Observed result |
| --- | ---: | --- |
| Remote M5 Mac Go peer → app, UI Inspect/Download | 2,159,190 | SHA-256 matched; imported; article read through the actual browser reader |
| App protected offer → remote M5 Mac Go peer | 2,159,190 | Completed; remote SHA-256 matched |
| App protected offer → Alpine Linux Go peer | 2,159,190 | Completed; VM SHA-256 matched |
| Actual Swift protected sender → app | 2,159,187 | Completed; SHA-256 matched; library import observed |
| App protected offer → actual Swift engine via Bonjour | 71,363,654 | All 69 chunks completed; SHA-256 matched |
| Linux Go peer → app | 2,159,190 | Earlier transfer matched SHA-256 and exposed a duplicate-UUID import defect; later per-receipt failure/retry acceptance preserved the original installation |
| Corrupt first source with valid discovered alternatives | 2,159,190 | Completed with the correct archive SHA-256 |
| Manifest-valid/chunk-valid archive with invalid internal MD5 | 2,159,190 | Failed native integrity verification; no completed paths/publication |

Wrong Swift PIN, wrong certificate and parent-path manifests were rejected by real connections. The wrong-PIN reference server closes the stream before a manifest; it does not supply a detailed authentication error. Clipboard denial was visible; granting only the owned browser clipboard permission allowed an actual copy/read round trip. Protected pause/restart/rehash/resume is now exercised below; literal PIN scans of the owned profile were clean, not proof of every possible representation everywhere.

A separate actual protected low-rate native transport scenario passed over the application idle deadline. A seven-byte final split and a valid empty archive split across its stored digest passed corrected SDK probes; the empty fixture passed all seven stock integrity checks. These probes do not replace packaged-app acceptance.

Latest correction evidence is under `.native/mobile-priority-20261030`. A native production-source harness completed shared-identity QUIC on **awdl0** and infrastructure **bridge100**, rejected an owned advertisement resolving to `203.0.113.1`, and observed one terminal callback with zero retained DNS operations. These peers were on the same host; this does not certify two-device radio transfer without an access point. A multihomed M5 scenario failed before interface fallback (bridge100 rejected the real M5 address), then succeeded on en0 after the correction. The actual application received, verified, imported and served **2,159,190 bytes** from the remote Go listener at `192.0.2.52:19682`; SHA-256 was `4d586bf8214ec73339fd719712643e2ab73894ad6fa1cace5c1e0efb43c546b1`.

The M5 run used a truthful, owned proxy advertisement for the genuine remote listener. The reference Go advertisement's hostname `<mac-hostname>.localdomain.local.` instead resolved only to `fe80::1` on the observed interface and did not connect; that discovery failure remains explicitly excluded, not silently repaired in the sibling repository. Numeric manual connection/proper producer records remain alternatives.

Actual iOS-simulator policy execution passed ten network-admission tests, including allowing a routerless native attempt while denying cellular Internet and unbound Quinn. Independent native/source reviews resolved pre-handshake locality, asynchronous shared-pointer capture, ARC mapping lifetime, duplicate DNS-operation ownership and listener-supervisor findings. Physical iPhone 5G, two-device routerless, release-package and post-QUIC-start multi-address fallback acceptance remain separate.

Final consumer/lifecycle evidence is sealed in `.native/localswarm-final-20261030/final-acceptance-summary.json`. A protected receive paused after 1,048,576 verified bytes; graceful exit/restart left it paused with discovery/offers off, and missing-PIN resume failed. Tampering the owned first chunk caused rehash to discard that progress; PIN-supplied resume recovered all 2,159,187 bytes with matching SHA-256. The rehash/progress transition is observed; a specific chunk refetch is inferred because no per-chunk server log was captured. Two-part publication/import produced 2,159,190 matching bytes and an HTTP-200 article. Private/read-only transitions retired established Go consumers with EOF. Explicit Stop paused a downloading job and cleared discovery/offers; source removal retired an actively uploading offer while preserving both owned split files. Real Downloads retry returned queued then failed on UUID collision without replacing the original installation. The final compiled LTR/Arabic UI used genuine discovered peers at 390px without overflow or visible opaque Bonjour tokens, with correct PIN/Inspect focus and cleared manual fallback. Earlier recovery/lifecycle receipts and final-binary UI/new-transfer receipts retain their distinct binary identities.


## Remaining acceptance gates

- Finish broad producer/multihomed discovery acceptance. Native preflight now recovers from a wrong first interface; actual M5 success used a truthful proxy record. The reference Go hostname failure above is not counted as fixed, and preflight fallback is not post-handshake connection racing.
- Packaged macOS/mobile peer flows, physical iPhone 5G and two-device routerless radio still need their own acceptance; browser/headless/native-harness evidence does not silently establish those surfaces.
- Exercise genuine local-network OS permission denial where supported; do not treat an explicit app switch or a network timeout as OS permission evidence.
- Windows still requires its trusted SDK and native runner. Actual Linux ARM64 GTK peer/PDF workflows passed on separately sealed binaries in `.native/linux-priority-final-20261030`; the newest consumer UI cutover has its own acceptance. Shared Rust code and Go peer interoperability are not substitutes for native execution.

The exhaustive closed-issue/closed-PR audits of both predecessor applications have since completed: [792 desktop issues](audit/desktop-all-closed-issues.md), [590 desktop PRs](audit/desktop-all-closed-prs.md), [801 Apple issues](audit/apple-all-closed-issues.md) and [919 Apple PRs](audit/apple-all-closed-prs.md). Applicable corrections have separate adversarial/runtime acceptance; audit completion does not certify every historical regression or close the LocalSwarm gates above.
