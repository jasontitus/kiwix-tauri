#!/usr/bin/env python3
"""Copy the embedded search indexes of a ZIM archive into a fuzz corpus directory.

Usage: extract-index-seeds.py ARCHIVE.zim OUTPUT_DIRECTORY

The `search_index` fuzz target takes one Glass index as its whole input. libzim stores
`X/fulltext/xapian` and `X/title/xapian` in uncompressed clusters, so their bytes are
copied straight out of the archive. Anything unexpected is an error, never an empty seed.
"""
import struct
import sys
from pathlib import Path

GLASS_MAGIC = b"\x0f\x0dXapian Glass\x04\x6e"
INDEXES = {b"fulltext/xapian": "fulltext.glass", b"title/xapian": "title.glass"}


def main(archive: Path, output: Path) -> None:
    data = archive.read_bytes()
    if len(data) < 80 or data[:4] != b"ZIM\x04":
        raise SystemExit(f"{archive}: not a ZIM archive")
    entry_count, cluster_count, url_pointers, _titles, cluster_pointers = struct.unpack_from("<IIQQQ", data, 24)
    found = {}
    for index in range(entry_count):
        (dirent,) = struct.unpack_from("<Q", data, url_pointers + 8 * index)
        (mime,) = struct.unpack_from("<H", data, dirent)
        if mime == 0xFFFF or data[dirent + 3 : dirent + 4] != b"X":
            continue  # a redirect, or not in the index namespace
        cluster, blob = struct.unpack_from("<II", data, dirent + 8)
        path = data[dirent + 16 : data.index(b"\0", dirent + 16)]
        if path in INDEXES:
            found[path] = (cluster, blob)
    if set(found) != set(INDEXES):
        raise SystemExit(f"{archive}: expected both embedded search indexes, found {sorted(found)}")
    output.mkdir(parents=True, exist_ok=True)
    for path, (cluster, blob) in found.items():
        if cluster >= cluster_count:
            raise SystemExit(f"{archive}: {path.decode()} names a missing cluster")
        (start,) = struct.unpack_from("<Q", data, cluster_pointers + 8 * cluster)
        info = data[start]
        if info & 0x0F not in (0, 1):
            raise SystemExit(f"{archive}: {path.decode()} is stored compressed")
        width, code = (8, "<Q") if info & 0x10 else (4, "<I")
        table = start + 1
        (begin,) = struct.unpack_from(code, data, table + width * blob)
        (end,) = struct.unpack_from(code, data, table + width * (blob + 1))
        payload = data[table + begin : table + end]
        if not payload.startswith(GLASS_MAGIC):
            raise SystemExit(f"{archive}: {path.decode()} is not a Glass index")
        (output / INDEXES[path]).write_bytes(payload)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(Path(sys.argv[1]), Path(sys.argv[2]))
