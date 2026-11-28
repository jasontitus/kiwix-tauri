# BitTorrent HTTP web seeds (BEP 19)

`src-tauri/src/torrent/web_seed.rs` fills torrents that advertise HTTP(S)
web seeds through the top-level `url-list` field. This exists because the
in-process librqbit engine does not implement BEP 19 and a web-seed-only
torrent has no peers to make progress.

## Scope

- One URL or a list of URLs, each a direct file or a base ending in `/`.
- Single-file and multi-file torrents. Multi-file URLs append the torrent name
  and the relative file path with URL segment encoding.
- HTTP Range resume for partial staging files.
- BEP 47 padding entries are counted and verified as zero bytes but are never
  requested from the seed.
- HTTPS is required except for literal loopback HTTP used by local tests.
  Credentials and redirects to non-admitted schemes are rejected.
- Not implemented: BEP 17 `httpseeds`, HTTP authentication, mirror ranking,
  or an ActivityKit-style external progress surface.

## Flow

1. `TorrentManager::run` parses `url-list` and the minimal `info` fields needed
   for paths, lengths, piece length and SHA-1 hashes. A malformed optional
   web-seed field is logged and ignored so ordinary peer torrents still run.
2. Already verified jobs skip this adapter; completed seeding payloads are not
   re-downloaded.
3. Files are opened through the same `ConfinedFactory` used by the engine, so
   path confinement, no-follow checks, ownership journaling and backup
   exclusion remain in force.
4. The adapter downloads into those files, updating progress at most every
   250 ms. A file shorter than the torrent says was written front to back by
   this adapter and resumes with a Range request from its length; a server
   that ignores that Range restarts the file from zero. When one mirror drops
   the connection, the next mirror continues from the bytes already on disk.
5. Every torrent piece is SHA-1 verified. Nothing found on disk is discarded
   for failing: a file already at its full length (sized by the engine or by
   partial reuse) can hold valid pieces among unwritten ones, so only the byte
   ranges of failing pieces are fetched again, in place, mirror by mirror, and
   only those pieces are hashed again. Bytes of a piece that verified are never
   rewritten. A mirror without Range support is read once from the start and
   only the missing ranges are kept from its answer.
   Each pass tries every mirror. Connection loss, timeouts, HTTP 408/429/5xx,
   short bodies and a mirror that sends too slowly are transient: the pass is
   repeated after 1, 2, 4 and 8 seconds. The stage ends at once when no
   mirror's failure in a pass was transient. It also ends after five
   consecutive passes that add less than one piece, after two minutes of such
   passes waiting on mirrors, or after 32 passes in all, whichever comes
   first. Connecting to a mirror may take 15 seconds and a connected mirror
   may stay silent for 90. Once a response body has started it must also keep
   a minimum pace: 128 KiB (one piece, or all that is left, when that is
   less) in every 60 seconds, about 2 KiB/s. A response that falls behind is
   dropped. A pause or cancel ends any wait immediately. On every failure the
   staged files are left as they are.
6. The normal librqbit engine then opens/hashes the same files and publishes
   through the existing no-replace publication path. The adapter is an
   acquisition shortcut, not a replacement verifier.

## Verification

- Host tests cover parsing, URL construction, unsafe metadata, single-file
  download, corrupt full-length staging recovery, multi-file download and an
  end-to-end local HTTP torrent that completes through the engine and
  publication path. They also cover, for a single-file torrent: repairing only
  the missing pieces of a full-length file, a failing mirror leaving that file
  untouched, a mirror without Range support, the hand-over between mirrors,
  the stall and pass limits, a trickling mirror on both the download and the
  repair path, a slow mirror that keeps the pace, and a stop during a wait
  between passes or inside a response.
- The real StreetZim Bahrain map torrent (`50,781,499` bytes) was downloaded
  from its real HTTPS `url-list` and SHA-1 verified in 6.7 s in a host test.
- The network test was temporary; the committed tests are deterministic and
  use local HTTP.

## Residual limits

- If a web seed is unavailable, the failure is logged and the normal engine
  peer path continues. A peerless torrent can therefore remain without
  progress instead of immediately reporting that the seed is dead.
- Peers are not contacted until this stage ends. Mirrors that refuse the
  connection, or a device that is offline, cost about 15 seconds of retries.
  Mirrors that silently drop the connection attempt cost about 1.5 minutes for
  one mirror and about 2.3 minutes for three. Mirrors that connect and then
  never answer cost about 3 minutes for one and 4.5 minutes for three. These
  figures follow from the timeouts above and were not measured against real
  hosts.
- A mirror that only trickles data is dropped after 60 seconds each time and
  costs about 2 minutes for one mirror and about 3 minutes for three.
- The minimum pace does not bound a mirror that wants to hold the stage. A
  mirror that sends one whole piece quickly and then trickles makes its pass
  count as progress, so only the 32-pass limit applies, and each mirror in the
  list can hold a pass for about 60 seconds: roughly 32 x (mirrors x 60 s),
  about 33 minutes for one mirror and about 97 minutes for three. A mirror
  that keeps exactly to the minimum pace is never cut off and can hold the
  stage, and the peers behind it, for as long as the file takes at that rate.
  No rate floor can prevent that, and there is no deadline for the whole
  stage.
- The pace is a fixed byte count, not a whole piece, because Kiwix's torrents
  use 4 MiB pieces (three public `.torrent` files were checked), and a piece a
  minute would demand about 68 KiB/s from honest mirrors on slow links.
- A full-length staging file is never trusted by length alone. It is hashed in
  full on every start before any missing range is requested, and again by the
  engine. No progress is shown during that pass.
- The repair path for multi-file torrents and BEP 47 padding entries is
  covered by reading the code, not by a host test.
- No physical device/UI acceptance is claimed for this adapter. It is a
  torrent-transport change; existing Downloads/UI state and publication
  contracts are unchanged.
