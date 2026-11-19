# Pinned Zimru and libzim on macOS ARM64

This comparison uses the MIT Zimru revision selected by Kiwix,
`3861631f0e8302d765956f6acac73ed00c1b30b1`, with default features disabled.
The native driver uses libzim 9.8.2 from the prepared SDK, upstream revision
`26ec526f74e8342a40da5ab14e364546988e0e1a`, with the existing retained-file
and split-range patches. The linked libzim SHA-256 is
`b471b66a241f75c9a5a40ac13d1dd5f2147a7346f28f3593e03d61644c55b83b`.
The release Rust driver used the standalone harness lockfile with the pinned
reader source. [Harness and measurement scope](../../scripts/bench-archive-readers/README.md)

On macOS 27.2.1 both drivers requested a 4 MiB cache, and the native driver
left Xapian preloading off. Eight measured pairs followed one warm-up pair
for each case. Drivers alternated order and called the libraries directly.
The collector compared every response byte across archive open, first
article, 32 repeated reads, 24 random article reads, title prefixes and 16
media ranges. Each article read requested up to its first 64 KiB, and each
media request covered 4 KiB. All 18 cases agreed, including the sampled title
order. The [raw results](2026-11-19-mac-pinned-reader.json) include archive hashes,
requests, response hashes, individual timings and process memory.

| Archive | Open | Open and first article | Random article batch | Media range batch |
| --- | ---: | ---: | ---: | ---: |
| Climate mini | 2.028 / 0.055 ms | 3.452 / 1.569 ms | 18.577 / 28.488 ms | 2.218 / 2.102 ms |
| Wikipedia 100 maxi | 2.259 / 0.059 ms | 3.403 / 2.451 ms | 33.640 / 43.649 ms | 0.209 / 0.121 ms |
| Chinese Chemistry mini | 1.909 / 0.061 ms | 3.305 / 1.594 ms | 15.865 / 27.702 ms | 2.407 / 3.350 ms |

The table reports medians with libzim first. The first-article column
includes opening the archive in the same sample. Random article and media
columns measure lookup and copying after the archive opens. In the
random-article batches, Zimru used 1.07–1.22 times the native whole-process CPU
time. This measures startup, reads, framing and cleanup. Median process peak
RSS for those batches was 20.8 / 29.3 MiB for Climate, 20.9 / 13.2 MiB for
Wikipedia 100 and 19.4 / 20.8 MiB for Chinese Chemistry.

Zimru opens all three archives faster and takes longer in all three random
article batches. The slowest Zimru median in the table was 43.649 ms for the
Wikipedia 100 random-read API batch. That case took 50.955 ms including
driver startup and cleanup. These results cover standalone reader operations
with warm filesystem caches. App rendering, full-text search, writing and
the other platforms require separate measurements. [Performance follow-up](../MEMORY-SAFETY-INVENTORY.md#performance-follow-up)
