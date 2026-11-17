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
   250 ms. Partial files resume with a Range request; a server that ignores
   Range restarts that file from zero.
5. Every torrent piece is SHA-1 verified. A verification failure discards the
   staged bytes and retries one from-scratch acquisition. Transport failures
   keep partial files and retry with Range.
6. The normal librqbit engine then opens/hashes the same files and publishes
   through the existing no-replace publication path. The adapter is an
   acquisition shortcut, not a replacement verifier.

## Verification

- Host tests cover parsing, URL construction, unsafe metadata, single-file
  download, corrupt full-length staging recovery, multi-file download and an
  end-to-end local HTTP torrent that completes through the engine and
  publication path.
- The real StreetZim Bahrain map torrent (`50,781,499` bytes) was downloaded
  from its real HTTPS `url-list` and SHA-1 verified in 6.7 s in a host test.
- The network test was temporary; the committed tests are deterministic and
  use local HTTP.

## Residual limits

- If a web seed is unavailable, the failure is logged and the normal engine
  peer path continues. A peerless torrent can therefore remain without
  progress instead of immediately reporting that the seed is dead.
- Whole-file acquisition means a full-length but corrupt staging file is
  re-downloaded from zero after verification fails; it is never trusted by
  length alone.
- No physical device/UI acceptance is claimed for this adapter. It is a
  torrent-transport change; existing Downloads/UI state and publication
  contracts are unchanged.
