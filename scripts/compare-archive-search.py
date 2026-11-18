#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare two local Kiwix builds using isolated profiles and alternating runs (POSIX, Python 3.11+)."""
import argparse
from contextlib import contextmanager
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import tempfile
import time
from urllib.parse import urlsplit


def file_sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def ready_endpoint(stream):
    line = bytearray()
    deadline = time.monotonic() + 180
    while b"\n" not in line:
        if len(line) == 512:
            raise RuntimeError("Oversized readiness announcement")
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([stream], [], [], remaining)[0]:
            raise RuntimeError("Headless startup timed out")
        part = os.read(stream.fileno(), 512 - len(line))
        if not part:
            raise RuntimeError("Missing or oversized readiness announcement")
        line.extend(part)
    match = re.fullmatch(rb"KIWIX_READY (http://kiwix-shell-[0-9a-f]{32}\.localhost:[0-9]+/#token=[0-9a-f]{32})",
                         line.split(b"\n", 1)[0].rstrip(b"\r"))
    if not match:
        raise RuntimeError("Unexpected headless address")
    url = urlsplit(match.group(1).decode("ascii"))
    if not url.port:
        raise RuntimeError("Expected a local listening port")
    return url


@contextmanager
def service(binary, archive):
    with tempfile.TemporaryDirectory(prefix="kiwix-search-bench-") as profile:
        with subprocess.Popen(
            [binary, "--headless", "--data-dir", profile, "--port", "0", archive],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        ) as child:
            try:
                url = ready_endpoint(child.stdout)
                headers = {
                    "Content-Type": "application/json", "Host": url.netloc,
                    "Origin": f"http://{url.netloc}",
                    "Authorization": "Bearer " + url.fragment.removeprefix("token="),
                }

                def call(command, args=None):
                    conn = http.client.HTTPConnection("127.0.0.1", url.port, timeout=90)
                    try:
                        conn.request("POST", "/api", json.dumps({"command": command, "args": args or {}}), headers)
                        response = conn.getresponse()
                        limit = 4 * 1024 * 1024
                        data = response.read(limit + 1)
                        if len(data) > limit:
                            raise RuntimeError(f"{command} exceeded the benchmark response limit")
                        if response.status != 200:
                            raise RuntimeError(f"{command} returned HTTP {response.status}")
                        return json.loads(data)["result"]
                    finally:
                        conn.close()

                yield child, call
            finally:
                if child.poll() is None:
                    child.send_signal(signal.SIGINT)
                    try:
                        child.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()


def trial(binary, archive, query, prefix, warm):
    rows = []
    with service(binary, archive) as (child, call):
        snap = call("snapshot")
        if len(snap["books"]) != 1:
            raise RuntimeError("Expected the one supplied archive in the isolated profile")
        book = snap["books"][0]["id"]
        sequence = 0
        for mode, text, snippets in [("fulltext", query, True), ("fulltext", query, False), ("suggest", prefix, False)]:
            call("save_settings", {"settings": {"showSearchSnippets": snippets, "spellingEnabled": False}})
            session = call("search_session")["sessionId"]
            owner = call("search_owner", {"sessionId": session})["ownerId"]
            args = {"bookIds": [book], "privacyGeneration": call("snapshot")["privacyGeneration"],
                    "sessionId": session, "ownerId": owner, "limit": 50}
            if mode == "suggest":
                args["prefix"] = text
            else:
                args.update(mode=mode, query=text, offset=0)
            durations = []
            digest = None
            for _ in range(warm + 1):
                sequence += 1
                start = time.perf_counter_ns()
                page = call("suggest" if mode == "suggest" else "search", dict(args, requestId=sequence))
                durations.append(round((time.perf_counter_ns() - start) / 1e6, 3))
                hits = page["suggestions" if mode == "suggest" else "hits"]
                if page["errors"] or not hits:
                    raise RuntimeError("Query failed or returned an empty page")
                rank = [[r["title"], r["path"]] for r in hits]
                encoded = json.dumps([rank, page["hasMore"]], ensure_ascii=False).encode()
                current = hashlib.sha256(encoded).hexdigest()
                if digest is not None and current != digest:
                    raise RuntimeError("Rank or continuation changed across repeated queries")
                digest = current
            rows.append({"mode": mode, "query": text, "snippets": snippets, "count": len(hits),
                         "ordered_sha256": digest, "first_ms": durations[0], "warm_ms": durations[1:]})
            call("search_release", {"sessionId": session, "ownerId": owner})
        rss = int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(child.pid)]))
    return {"queries": rows, "rss_after_kib": rss}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native", "integrated", "archive"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--query", required=True)
    parser.add_argument("--prefix", required=True)
    parser.add_argument("--rounds", type=int, default=4)
    parser.add_argument("--warm", type=int, default=5)
    args = parser.parse_args()
    if args.rounds < 2 or args.rounds % 2 or args.warm < 1:
        parser.error("Use an even number of rounds and at least one warm request")
    archive = args.archive.resolve(strict=True)
    builds = {"libzim": args.native.resolve(strict=True), "integrated": args.integrated.resolve(strict=True)}
    identities = {name: file_sha256(path) for name, path in builds.items()}
    if identities["libzim"] == identities["integrated"]:
        parser.error("The selected executables are identical")
    results = []
    expected = None
    for i in range(args.rounds):
        for name in (builds if i % 2 == 0 else reversed(builds)):
            result = trial(builds[name], archive, args.query, args.prefix, args.warm)
            ranks = [row["ordered_sha256"] for row in result["queries"]]
            if expected is not None and ranks != expected:
                raise RuntimeError("Builds returned different ranked pages")
            expected = ranks
            results.append({"round": i + 1, "build": name, **result})
    print(json.dumps({"archive": archive.name, "archive_sha256": file_sha256(archive),
                      "executables_sha256": identities, "rounds": results}, indent=2))


if __name__ == "__main__":
    main()
