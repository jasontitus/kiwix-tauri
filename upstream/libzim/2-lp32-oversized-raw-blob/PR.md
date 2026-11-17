# Don't skip oversized uncompressed blobs in Cluster::getReader()

## Problem

This affects platforms where `size_t` is 32 bits (32-bit x86 and ARM, including Android `armeabi-v7a`). Consider an uncompressed (extended) cluster that contains a blob larger than `SIZE_MAX`, i.e. at least 4 GiB, such as a large media file stored without compression:

- **Following blobs are corrupted, silently.** Every blob after the oversized one in the same cluster returns the wrong content: the first bytes of the oversized blob instead of its own.
- **Bounded reads fail.** Partial reads of the oversized blob throw `std::runtime_error` ("Assertion failed at src/buffer.cpp:52 offset.v+size.v[4] <= m_size.v[0]"), even though the requested range fits in memory. These are `Cluster::getBlob(n, offset, size)`, reached from `Item::getData(offset, size)`.

64-bit platforms are not affected.

## Root cause

`Cluster::getReader()` (`src/cluster.cpp`) creates the blob readers sequentially from the cluster's stream reader. For a blob larger than `SIZE_MAX`, it pushes an empty `BufferReader` instead of calling `m_reader->sub_reader(blobSize)`.

**Compressed clusters.** The substitution is necessary. `IStreamReader::sub_reader()` would have to decompress the blob into a buffer of `blobSize` bytes, which is impossible.

**Uncompressed clusters.** `m_reader` is a `RawStreamReader`. Its `sub_reader()` only creates a reader over a file range and advances `m_readerPos`. Skipping it has two effects:

- `m_readerPos` stays at the start of the oversized blob, so the reader of the next blob starts there. This causes the wrong content.
- The oversized blob's reader is an empty buffer, so any bounded `get_buffer()` on it fails the range check in `Buffer::sub_buffer()`. This causes the exception.

## Fix

Substitute the empty reader only for compressed clusters. For uncompressed clusters, always create the file-range sub-reader. That reader holds a 64-bit range and does not allocate anything.

`Cluster::getBlob(n)` (the whole blob) still returns an empty `Blob` for a blob larger than `SIZE_MAX`, through its own existing check. `read_extended_cluster` keeps asserting this.

## Tests

`test/cluster.cpp`, `ClusterTest.read_extended_cluster_oversized_blob_not_last`.

The test builds an uncompressed extended cluster in a sparse file, using the same technique as `read_extended_cluster`. The cluster holds three blobs:

| Blob | Size | Content |
|---|---|---|
| 0 | 30 bytes | 30-byte string |
| 1 | 4 GiB + 1024 bytes | `"head"` at offset 0, `"high"` at offset 4 GiB + 16, `"tail"` at the end; a hole everywhere else |
| 2 | 26 bytes | 26-byte string |

It checks:

- blobs 0 and 2;
- three bounded reads of blob 1;
- on 32-bit only, that `getBlob(1)` (the whole blob) is still empty.

The existing `read_extended_cluster` does not catch the bug because its oversized blob is the last one in the cluster.

On 64-bit the new test passes with and without the fix, because the bug is specific to 32-bit. It is a regression test for 32-bit test runs. The CI currently builds `android-arm` but does not run its tests.

The test is not guarded by `SKIP_BIG_MEMORY_TEST`, because it needs neither memory nor disk space on a filesystem that supports sparse files. On a filesystem without sparse files (NTFS, unless the file is flagged sparse), it writes 4 GiB, as `read_extended_cluster` already does on the Windows CI. I am happy to guard it if you prefer.

How to run (32-bit, e.g. on Ubuntu 24.04 x86_64 with multilib):

```bash
sudo dpkg --add-architecture i386 && sudo apt-get update
sudo apt-get install gcc-multilib g++-multilib libzstd-dev:i386 liblzma-dev:i386 libgtest-dev:i386
# libxapian-dev:i386 cannot be installed alongside libxapian-dev:amd64, hence with_xapian=false.
# A native (not cross) setup is used because tests are not built in cross builds.
CC="gcc -m32" CXX="g++ -m32" PKG_CONFIG_LIBDIR=/usr/lib/i386-linux-gnu/pkgconfig:/usr/share/pkgconfig \
  meson setup build32 -Dwith_xapian=false -Dtest_data_dir=<zim-testing-suite dir>
ninja -C build32
build32/test/cluster --gtest_filter='ClusterTest.*extended*'
meson test -C build32
```

## Evidence

Environment: Ubuntu 24.04 x86_64. GCC 13.3 with `-m32` for the 32-bit build (`ELF 32-bit LSB shared object, Intel 80386`), using the i386 zstd 1.5.5, liblzma 5.4.5 and GTest 1.14 packages. Base: `ad888b4`.

32-bit build, new test without the fix (only `src/cluster.cpp` reverted):

```
[ RUN      ] ClusterTest.read_extended_cluster_oversized_blob_not_last
../../libzim-prs/test/cluster.cpp:393: Failure
Expected equality of these values:
  blob2
    Which is: "abcdefghijklmnopqrstuvwxyz"
  std::string(cluster->getBlob(zim::blob_index_t(2)))
    Which is: "head\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"
[  FAILED  ] ClusterTest.read_extended_cluster_oversized_blob_not_last (0 ms)
```

I also ran an ad hoc variant of the test, using `EXPECT` instead of `ASSERT` so that it continues after the first failure. On the same build, the first bounded read of the oversized blob then throws:

```
Assertion failed at ../../libzim-prs/src/buffer.cpp:52
 offset.v+size.v[4] <= m_size.v[0]
... zim::Buffer::sub_buffer(...)
... zim::BufferReader::get_buffer(...)
... zim::Cluster::getBlob(zim::blob_index_t, zim::offset_t, zim::zsize_t)
```

A 32-bit `-Dbuildtype=release` build behaves the same, because `b_ndebug` defaults to `false` and `ASSERT` stays active.

With the fix, on the 32-bit build:

```
[       OK ] ClusterTest.read_write_extended_cluster (0 ms)
[       OK ] ClusterTest.read_extended_cluster (0 ms)
[       OK ] ClusterTest.read_extended_cluster_oversized_blob_not_last (0 ms)
[  PASSED  ] 3 tests.
```

Full suites:

| Build | Without the fix | With the fix |
|---|---|---|
| x86_64 | 31/31 OK | 31/31 OK |
| i386 (`-Dwith_xapian=false`, Xapian-dependent tests not built) | 27/27 OK | 27/27 OK |

## Risk / compatibility

- Behaviour changes only for blobs larger than `SIZE_MAX` in uncompressed clusters, so only on 32-bit platforms. Compressed clusters are unchanged.
- No API or ABI change.

## Not verified

- **32-bit ARM.** Tested on 32-bit x86 (i386) only, not on ARM (armhf or Android `armeabi-v7a`) and not on 32-bit Windows.
- **Builds with `-Db_ndebug=true`.** The range check in `Buffer::sub_buffer()` is compiled out there. Without this fix, a bounded read of the oversized blob would then build a pointer from an empty buffer instead of throwing. Not tested.
- **Compressed clusters.** A compressed cluster containing a blob larger than `SIZE_MAX` still does not advance its decompression stream past that blob, so on 32-bit the following blobs of such a cluster are still read incorrectly. Fixing that would mean decompressing and discarding more than 4 GiB. This PR does not attempt it; such a cluster would decompress to more than 4 GiB.
