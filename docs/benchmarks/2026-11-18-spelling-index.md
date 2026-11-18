# Spelling dictionaries from archive titles

A user can build an offline dictionary after a search returns no articles. The
application builds the index from public titles in the selected archive. It
preserves case and separators in the proposed query, ranks single-edit matches
by word frequency and breaks ties by normalized spelling. An adjacent
transposition counts as one edit.

## Compatibility and storage

The format stores frequencies in two sorted sets of 8 KiB pages, one in
forward word order and one in reversed word order. On open, the reader checks
the file length, row and page counts, ordering, valid Unicode words, matching
frequencies and the digest. It retains sparse page anchors and digests. On
query it checks each newly read page and holds up to 64 decoded pages from each
set. The spelling manager retains up to eight warm dictionaries and admits up
to two concurrent read-only cold validations. Counts remain available while a
query holds the dictionary's page cache.

The builder enumerates up to 20 million titles, at most 256 at a time. Each
title is limited to 16 KiB. Up to two million distinct words of two to 64
Unicode scalars can enter the dictionary. The external sort spills batches
at an estimated 2 MiB threshold. A completed index may hold up to 1 GiB.
Simultaneous logical bytes for its active sort files and new output are
limited to 2 GiB. The user’s prior dictionary stays in place during a build.
An explicit repair preserves a superseded invalid generation as a backup.
Abrupt process death can leave a named temporary output. These files can
accumulate across interrupted or repaired builds.

Existing SQLite dictionaries and earlier unbound word indexes are identified
by filename alone. A writable session offers a localized rebuild action.
Opening a search leaves these files intact; only the explicit action derives
a new dictionary. An unavailable or replaced installation invalidates its
actions and suggestions. The opaque key includes the installation ID and the
ordered file identities from the archive's retained handles. Filesystem
identity includes device, inode or file ID, size and timestamps. A provider
that changes those identities across opens can require a new dictionary even
when the archive bytes are identical.

The cache uses no-follow directory and file handles. Unix requires current
ownership and excludes group and other write bits. On Apple systems even a
mode-0700 directory can carry an independent extended access list. The app
asks the OS for a fixed-size security attribute header and requires an empty
extended attribute. This also rejects read-only and deny-only ACLs, and
volumes that cannot report security attributes. Use a local, account-owned
spelling folder and files with ordinary permissions and no extended ACL to
recover. The Windows path reuses the app's existing ACL check. It accepts
ordinary allow entries for the owner, LocalSystem and built-in administrators
and rejects other principals or unsupported entry types. Existing folders or
files have their permissions preserved.

The Rust spelling format forbids unsafe code. Archive bytes and query strings
stay in checked Rust while parsing this format. File ownership, handles,
Windows ACL calls and the Apple security attribute call remain operating
system boundaries. The Apple check passes a retained descriptor, a fixed
request and a 12-byte output buffer. A checked SHA-256 digest detects changed
pages. Access controls and the retained source identity authorize the cache.

## Measurements

The standalone release comparison uses identical length-prefixed title streams.
It compiles the SQLite implementation from application commit
`96e7afce109a1b753213caf601a4b8586d948313` and the current Rust reader,
builder and ranking code. Each of four rounds creates fresh indexes and uses
a fresh query process for each index. The collector alternates backend order.
For each of three inputs it compares title and distinct word counts, and all
ordered results for 33 queries. The warm batch repeats those 33 queries 31
times. Archive extraction, user-interface and archive verification time are
outside the measured dictionary operations. Results reflect a shared ARM64
Mac with warm OS file caches.

| Input | Backend | Build time | Build peak RSS | Index | Validate on open | First 33 queries | Next 1,023 queries |
|---|---|---:|---:|---:|---:|---:|---:|
| Climate mini | SQLite | 0.107 s | 5.57 MiB | 0.88 MiB | 5.20 ms | 1.19 ms | 27.81 ms |
| Climate mini | Rust | 0.039 s | 5.48 MiB | 0.16 MiB | 1.73 ms | 1.00 ms | 3.34 ms |
| Simple Wikipedia | SQLite | 69.957 s | 7.04 MiB | 39.95 MiB | 435.51 ms | 4.33 ms | 92.84 ms |
| Simple Wikipedia | Rust | 6.710 s | 9.97 MiB | 6.36 MiB | 173.43 ms | 12.14 ms | 23.52 ms |
| Common affixes | SQLite | 2.225 s | 6.69 MiB | 30.75 MiB | 80.65 ms | 1.53 ms | 33.28 ms |
| Common affixes | Rust | 0.182 s | 7.12 MiB | 1.58 MiB | 20.49 ms | 0.52 ms | 2.96 ms |

The Simple Wikipedia stream contained 394,563 titles and 179,182 distinct words. The checked format reduced the build time, file size and warm query time. Its initial batch of 33 queries took longer and building used more peak process memory. The Rust builder recorded 9.54 MiB of peak logical storage. The collector observed at least 42.65 MiB of visible SQLite files at its sample points.

Build time includes validation. Peak RSS includes the process and allocator.
For the Rust implementation the collector also records the peak live logical
bytes of all runs and the new index. For SQLite the collector samples visible
files every two milliseconds. The latter is a lower bound. The two storage
measurements use different methods. See all per-process measurements, inputs,
checksums and expected answers in the [raw comparison](2026-11-18-spelling-index.json).

Run from the repository root with the pinned reader sources and comparison
dependencies already present in the local Cargo cache. Supply a fresh output
path and local archives.

```sh
python3 scripts/bench-spelling-index/compare.py --rounds 4 --synthetic \
  --output spelling-comparison.json \
  /path/to/wikipedia_en_climate_change_mini_2024-06.zim \
  /path/to/wikipedia_en_simple_all_nopic_2026-05.zim
```

The locked normal dependency trees for macOS ARM64 and Intel, Linux x86-64 and
ARM64, Windows x86-64 and ARM64, Android ARM64 and ARMv7 and the ARM64 iOS
device and simulator all included the checked SHA-256 crate and excluded
`rusqlite` and `libsqlite3-sys`. These checks inspect Cargo dependency
selection.

The application regression on the real Climate archive built a dictionary,
corrected “climtae” to “climate”, found ordinary search results and repeated
the correction after a cold reopen. The 25 focused tests ran under Unix umasks 002 and 022. Additional tests covered a same-registration
replacement with the same archive bytes, permission failures preserving
unrelated files, stale source keys, concurrent validation, and entering private
mode or cancelling while another reader held the archive. The Apple ACL test
covered both a regular file and a directory. Windows ACL behavior in this
batch was checked in source. Mobile peak memory and latency remain unmeasured.
