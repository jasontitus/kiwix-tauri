# Account for zstd decoder memory in the cluster cache, and lock the reader in Cluster::getMemorySize()

Refs #1127 (item 8)

This PR has two commits. They are meant to be merged together, in this order.

1. **Lock the reader in Cluster::getMemorySize()** fixes an existing data race.
2. **Fix the memory size of zstd streams** fixes the swapped branches in `ZSTD_INFO::state_size()`.

Applying 2 without 1 would make the race worse: `getMemorySize()` would then also read the live zstd decoder while another thread decodes with it.

## Problem

### 1. zstd decoder memory is never counted

Since 9.4.0 the cluster cache is limited by memory, using `Cluster::getMemorySize()` as the cost of each cluster. That cost includes the size of the decompression stream, `DecoderStreamReader::getMemorySize()`. For zstd, the size comes from `ZSTD_INFO::state_size()`, which always returns 0, so the cache under-counts zstd clusters, the compression format of all current ZIM files.

I measured this on `wikipedia_en_climate_change_mini_2024-06.zim` from zim-testing-suite. The program read every item with an unlimited cluster cache.

| | Before | After |
|---|---|---|
| `getClusterCacheCurrentSize()` for the 19 clusters touched (15 zstd, 4 uncompressed) | 14,943,458 bytes | 44,720,015 bytes |
| Actual growth of anonymous memory (`RssAnon`), same in both builds | 60,768,256 bytes | 60,768,256 bytes |

`wikibooks_be_all_nopic_2017-02.zim` uses LZMA and its cost is unchanged: 1,076,001 bytes in both builds.

### 2. Data race between `getMemorySize()` and decompression

`ConcurrentCache::materializeValue()` deliberately makes the value available to waiting threads *before* it computes its cost (`test/concurrentcache.cpp` checks this order). So `Cluster::getMemorySize()` can run while another thread is in `Cluster::getReader()` decompressing the same cluster.

`getReader()` advances the decompression stream while holding `m_readerAccessMutex`. `getMemorySize()` reads the state of that stream without the mutex:

- `DecoderStreamReader::getMemorySize()` reads `m_encodedDataChunk`, which `readNextChunk()` reassigns.
- It also reads the decoder state through `state_size()`. That is `lzma_memusage()` for LZMA, and `ZSTD_sizeof_DStream()` for zstd once fix 1 is applied.

ThreadSanitizer reports this race on current `main` with the new test (see Evidence).

## Root cause

- **zstd stream size.** In `ZSTD_INFO::state_size()` (`src/compression.cpp`) the two branches are swapped. When the stream is a decoder, it returns `ZSTD_sizeof_CStream(stream.encoder_stream)`, and the other way round. The other pointer is null, and both zstd functions return 0 for a null pointer.
- **Data race.** `Cluster::getMemorySize()` (`src/cluster.cpp`) accesses `m_reader` without `m_readerAccessMutex`. Its comment says no protection is needed because the method is called only once per cluster. That is true, but it is not the relevant concurrency: the call can overlap with decompression on another thread.

## Fix

### Commit 1: lock in `getMemorySize()`

`Cluster::getMemorySize()` now takes `m_readerAccessMutex` (as `getReader()` does), and its comment explains why.

Why this is the right mutex:

- `m_readerAccessMutex` is the mutex `getReader()` holds while it uses `m_reader` and `m_blobReaders`.
- After construction, `m_reader` is used only in `getReader()` and `getMemorySize()`. During construction, `read_header()` runs before the cluster is published.
- `m_blobOffsets`, the only other state `getMemorySize()` reads, is immutable after construction.

Why no caller already holds it, and why there is no lock-order problem:

- **Only one caller.** `getMemorySize()` is called only through `ClusterMemorySize::cost()`, from `ConcurrentCache::materializeValue()`. That runs without the cache's `lock_`, which is taken only inside `getCacheSlot()`, `finalizeCacheMiss()`, `drop()`, `dropAll()` and the size accessors, and released before they return.
- **No re-entry.** `getReader()` never calls into the cache, so the non-recursive mutex cannot be taken twice by one thread.
- **No cycle.** Nothing done under `m_readerAccessMutex` takes another libzim lock: decompression only reads from the underlying file or buffer reader.
  - The only libzim lock that can be held when the cache is entered is `FileImpl::m_xapianDbCreationMutex`, because `loadXapianDb()` reads a cluster.
  - That gives the order `m_xapianDbCreationMutex` → `m_readerAccessMutex`, and no path takes them the other way round.

Cost of the lock:

- On a cache miss, `getMemorySize()` may now wait for a decompression already running on another thread for the same cluster.
- Only the thread that materialised the cluster waits, after the value has been delivered to the other waiters, and it does not hold the cache lock while waiting.

I considered computing the cost before `set_value()` instead. That avoids the lock but changes the ordering in `ConcurrentCache`, which is deliberate and tested, and it makes waiters wait for the cost computation.

### Commit 2: zstd stream size

Swap the branches back:

- a decoder stream returns `ZSTD_sizeof_DStream(decoder_stream)`;
- an encoder stream returns `ZSTD_sizeof_CStream(encoder_stream)`.

## Tests

| Test | File | What it checks |
|---|---|---|
| `ClusterTest.getMemorySizeDuringDecompression` | `test/cluster.cpp` | Builds a zstd cluster of 64 blobs of 16 KiB of pseudo-random (poorly compressible) data. One thread `getBlob()`s every blob while the main thread calls `getMemorySize()` in a loop. 10 rounds. |
| `CompressionTest.zstdStreamStateSize` | `test/compression.cpp` | `state_size()` equals `ZSTD_sizeof_CStream()` / `ZSTD_sizeof_DStream()` for an encoder and a decoder stream. |
| `ClusterTest.zstdClusterMemorySizeIncludesDecoderState` | `test/cluster.cpp` | The cost of a freshly read zstd cluster (one 256 KiB blob) is larger than an upper bound on everything except the decoder state: blob offsets, half of the decompressed data, all of the compressed data, and the 1 KiB input chunk. |

`getMemorySizeDuringDecompression` is a smoke test in a normal build. Under `-Db_sanitize=thread` it detects the race.

With highly compressible data, the decoder never refills its input chunk after construction and TSAN sees no race. That is why the test uses pseudo-random data.

How to run:

```bash
meson setup build -Dtest_data_dir=<zim-testing-suite dir>
ninja -C build
build/test/compression --gtest_filter='*zstdStreamStateSize*'
build/test/cluster --gtest_filter='*MemorySize*'
meson test -C build

# ThreadSanitizer
meson setup build-tsan -Db_sanitize=thread -Dtest_data_dir=<zim-testing-suite dir>
ninja -C build-tsan
build-tsan/test/cluster --gtest_filter='*getMemorySizeDuringDecompression*'
```

## Evidence

Environment: Linux x86_64 (Ubuntu 24.04), GCC 13.3 with its ThreadSanitizer, zstd 1.5.5, liblzma 5.4.5, zim-testing-suite 0.10.0. Base: `ad888b4`.

### Race (commit 1)

TSAN build, `getMemorySizeDuringDecompression`, run 3 times without the lock (test file from this PR, `src/cluster.cpp` from `main`):

- Exit code 66 every time, 1 warning per run.
- The test assertions themselves passed.

```
WARNING: ThreadSanitizer: data race
  Write of size 8 ... by thread T2 (mutexes: write M0):
    #0 zim::Buffer::operator=(zim::Buffer const&)
    #1 zim::DecoderStreamReader<ZSTD_INFO>::readNextChunk()
    #2 zim::DecoderStreamReader<ZSTD_INFO>::decodeMoreBytes()
    #3 zim::DecoderStreamReader<ZSTD_INFO>::readImpl(char*, zim::zsize_t)
    #4 zim::IStreamReader::sub_reader(zim::zsize_t) src/istreamreader.cpp:35
    #5 zim::Cluster::getReader(zim::blob_index_t) const src/cluster.cpp:167
    #6 zim::Cluster::getBlob(zim::blob_index_t) const src/cluster.cpp:180
  Previous read of size 8 ... by main thread:
    #0 zim::Buffer::size() const src/buffer.h:46
    #1 zim::DecoderStreamReader<ZSTD_INFO>::getMemorySize() const
    #2 zim::Cluster::getMemorySize() const src/cluster.cpp:227
```

With the lock, the same 3 runs exited 0 with no warning.

### zstd stream size (commit 2)

Without the fix:

```
[ RUN      ] CompressionTest.zstdStreamStateSize
test/compression.cpp:134: Failure
  ZSTD_INFO::state_size(encoder)                  Which is: 0
  ZSTD_sizeof_CStream(encoder.encoder_stream)     Which is: 5248
test/compression.cpp:139: Failure
  ZSTD_INFO::state_size(decoder)                  Which is: 0
  ZSTD_sizeof_DStream(decoder.decoder_stream)     Which is: 95992

[ RUN      ] ClusterTest.zstdClusterMemorySizeIncludesDecoderState
test/cluster.cpp:197: Failure
Expected: (cluster2->getMemorySize()) > (sizeWithoutDecoderState), actual: 131160 vs 132151
```

With the fix, both tests pass. The cluster cost is then 393,244 bytes: the stream size is clamped to the decompressed size, as `getMemorySize()` intends.

### Full suites

| Build | Unmodified `main` | With both commits |
|---|---|---|
| Normal | 31/31 OK | 31/31 OK |
| `-Db_sanitize=thread` | 4 failures (`concurrentcache`, `indexing_criteria`, `search`, `search_iterator`); 15 warnings | the same 4 failures and 15 warnings |

The 4 TSAN failures already exist on unmodified `main` and these commits add no TSAN warning. They are a lock-order-inversion inside the `concurrentcache` test harness and "unlock of an unlocked mutex" reports from `MultiMutex` in the search code. I did not investigate them further. The `cluster` test, including the new tests, is clean under TSAN.

## Risk / compatibility

- **Fewer zstd clusters fit in the cache.** zstd clusters now report a larger cost, so the default 16 MiB cache holds fewer of them: in the measurement above, the same 19 clusters went from 14.9 MB to 44.7 MB.
  - The per-cluster increase is bounded by the cluster's decompressed size, through the existing clamp in `Cluster::getMemorySize()`.
  - The new figure is still below the measured anonymous memory growth.
  - Applications that tuned their cache size to the old under-counted values may see more cluster re-decompression. They can raise the limit with `zim::setClusterCacheMaxSize()`.
- **Encoder streams.** They now also report their real size, but `state_size()` is only used by `DecoderStreamReader::getMemorySize()`, so the writer is not affected.
- **Lock contention.** The lock is taken once per cache miss, on the thread that read the cluster.
- **No API or ABI change.**
- **Open PRs.** This applies cleanly on top of open #1036, whose `!m_reader` early return in `getMemorySize()` would then run under the lock, and on top of open #1119.

## Not verified

- **Races inside the compression libraries.** liblzma and libzstd are not instrumented in my build, so TSAN cannot see races inside `lzma_memusage()` or `ZSTD_sizeof_DStream()`. The race on the decoder state itself is established by reading the code; the reported race is on `m_encodedDataChunk`.
- **LZMA.** There is no concurrency test for LZMA clusters, because the writer can only produce zstd clusters.
- **Platforms.** Only tested on Linux x86_64.
- **Real workloads.** I did not measure the effect on cache hit rates in a real application.
