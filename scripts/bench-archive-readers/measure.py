"""Compare independent readers over a shared deterministic, byte-verified manifest."""

import argparse
import hashlib
import json
import os
import platform
import re
import signal
import statistics
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def write_new_json(path, value):
    """Publish only a complete new file, refusing existing names and input aliases."""
    name = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=path.parent, delete=False
        ) as pending:
            name = Path(pending.name)
            json.dump(value, pending, ensure_ascii=False, indent=1)
            pending.write("\n")
        os.link(name, path)
    finally:
        if name is not None:
            name.unlink()


def file_hash(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(4 << 20):
            h.update(chunk)
    return h.hexdigest()


def requests(native, path, policy):
    listing = subprocess.check_output(
        [native, "choose", path, *policy], text=True, timeout=90
    )
    items = []
    for line in listing.splitlines():
        kind, size, key, title = line.split("\t")
        items.append({"kind": kind, "size": int(size), "path": key, "title": title})
    articles = [x for x in items if x["kind"] == "html"]
    media = [x for x in items if x["kind"] == "media"]
    if not articles:
        raise ValueError(f"No article in {path.name}")
    reads = [(0, min(65536, x["size"]), x["path"]) for x in articles]
    prefixes = list(dict.fromkeys(x["title"][:3] for x in articles))[:8]
    phases = {
        "open": ("open", []),
        "first_article": ("read", reads[:1]),
        "repeat_article_32": ("repeat", reads[:1] * 33),
        "random_articles": ("read", reads),
        "title_prefixes": ("title", [(0, 0, prefix) for prefix in prefixes]),
    }
    if media:
        phases["media_ranges"] = (
            "read",
            [
                (offset, 4096, x["path"])
                for x in media
                for offset in [x["size"] // 2, x["size"] - 4096]
            ],
        )
    return items, phases


def execute(program, mode, query, archive, policy):
    """The child emits framed response bytes separately from its API timer."""
    with (
        tempfile.TemporaryFile() as stdin,
        tempfile.TemporaryFile() as out,
        tempfile.TemporaryFile() as err,
    ):
        stdin.write(query)
        stdin.seek(0)
        start = time.perf_counter_ns()
        proc = subprocess.Popen(
            [program, mode, archive, *policy], stdin=stdin, stdout=out, stderr=err
        )
        try:
            try:
                signal.alarm(90)
                _, status, usage = os.wait4(proc.pid, 0)
                wall = time.perf_counter_ns() - start
                proc.returncode = os.waitstatus_to_exitcode(status)
            finally:
                signal.alarm(0)
        finally:
            if proc.returncode is None:
                proc.kill()
                proc.wait()
        err.seek(0)
        diagnostics = err.read(64 << 10).decode(errors="replace")
        if proc.returncode:
            raise RuntimeError(f"{program.name} {mode} {archive.name}: {diagnostics}")
        ns = re.findall(r"^BENCH_NS ([0-9]+)$", diagnostics, re.MULTILINE)
        if len(ns) != 1 or int(ns[0]) <= 0:
            raise ValueError(diagnostics)
        out.seek(0)
        answer = out.read(8 << 20)
        if out.read(1):
            raise ValueError("Benchmark response exceeded 8 MiB")
    opened = int(re.search(r"^OPEN_NS ([0-9]+)$", diagnostics, re.MULTILINE)[1])
    budget, retained = map(
        int, re.search(r"^CACHE ([0-9]+) ([0-9]+)$", diagnostics, re.MULTILINE).groups()
    )
    return answer, {
        "open_ns": opened,
        "api_open_plus_work_ns": int(ns[0]) + (0 if mode == "open" else opened),
        "cache_budget_bytes": budget,
        "cache_retained_bytes": retained,
        "api_ns": int(ns[0]),
        "process_ns": wall,
        "cpu_ns": round(1e9 * (usage.ru_utime + usage.ru_stime)),
        "peak_rss_bytes": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
    }


def verify_read(answer, rows):
    at = 0
    for _, size, _ in rows:
        (n,) = struct.unpack_from("<Q", answer, at)
        if n != size or at + 8 + n > len(answer):
            raise ValueError("Response length differs from the manifest")
        at += 8 + n
    if at != len(answer):
        raise ValueError("Trailing response data")


def verify_titles(answer, rows):
    at = 0
    for _, _, prefix in rows:
        (count,) = struct.unpack_from("<Q", answer, at)
        at += 8
        if count == 0:
            raise ValueError(
                f"No matching title for selected article prefix {prefix!r}"
            )
        for _ in range(min(count, 4)):
            for kind in ("path", "title"):
                (length,) = struct.unpack_from("<Q", answer, at)
                at += 8
                if not length or length > len(answer) - at:
                    raise ValueError("Invalid title result")
                value = answer[at : at + length].decode()
                if kind == "title" and not value.startswith(prefix):
                    raise ValueError(f"Unexpected title {value!r}")
                at += length
    if at != len(answer):
        raise ValueError("Unexpected title rows")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("native", type=Path)
    parser.add_argument("rust", type=Path)
    parser.add_argument("archives", type=Path, nargs="+")
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--cache-bytes", default="default")
    parser.add_argument("--eager-native", action="store_true")
    parser.add_argument(
        "--phases",
        nargs="+",
        choices=[
            "open",
            "first_article",
            "repeat_article_32",
            "random_articles",
            "title_prefixes",
            "media_ranges",
        ],
        help="Limit to named phases",
    )
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    def expired(signum, frame):
        raise TimeoutError("Benchmark process timed out")

    if args.rounds < 2 or args.rounds % 2 or sys.platform not in {"darwin", "linux"}:
        parser.error("Supported on macOS and Linux with a positive even round count")
    signal.signal(signal.SIGALRM, expired)
    programs = {"libzim": args.native.resolve(), "zimru": args.rust.resolve()}
    policy = [args.cache_bytes, "eager" if args.eager_native else "lazy"]
    result = {
        "cache_bytes": args.cache_bytes,
        "native_xapian_preload": args.eager_native,
        "host": platform.platform(),
        "rounds": args.rounds,
        "driver_sha256": {k: file_hash(v) for k, v in programs.items()},
        "archives": [],
    }
    for path in args.archives:
        items, phases = requests(programs["libzim"], path, policy)
        archive = {
            "file": path.name,
            "size": path.stat().st_size,
            "sha256": file_hash(path),
            "selected": items,
            "phases": {},
        }
        result["archives"].append(archive)
        for phase, (mode, rows) in phases.items():
            if args.phases and phase not in args.phases:
                continue
            query = "".join(f"{at}\t{n}\t{key}\n" for at, n, key in rows).encode()
            data = {"requested": rows, "samples": {key: [] for key in programs}}
            archive["phases"][phase] = data
            reference = None
            for rnd in range(args.rounds + 1):
                for key in list(programs)[:: 1 if rnd % 2 == 0 else -1]:
                    answer, sample = execute(programs[key], mode, query, path, policy)
                    measured = rows[1:] if mode == "repeat" else rows
                    if mode in ("read", "repeat"):
                        verify_read(answer, measured)
                    if mode == "title":
                        verify_titles(answer, measured)
                    if mode == "open" and (
                        len(answer) != 8 or struct.unpack("<Q", answer)[0] == 0
                    ):
                        raise ValueError("Invalid archive entry count")
                    if reference is None:
                        reference = answer
                    if reference != answer:
                        raise ValueError(
                            f"Reader answers differ for {path.name}, {phase}, {key}"
                        )
                    if rnd:
                        data["samples"][key].append(sample)
            data["response_sha256"] = hashlib.sha256(reference).hexdigest()
            data["response_bytes"] = len(reference)
            medians = [
                round(statistics.median(s["api_ns"] for s in samples) / 1e6, 3)
                for samples in data["samples"].values()
            ]
            print(f"{path.name} {phase} ms {medians}", file=sys.stderr, flush=True)
    write_new_json(args.output, result)


if __name__ == "__main__":
    main()
