#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Survey video container formats in the live Kiwix catalog.

The survey has two tiers:

1. MIME-list scan (cheap, broad).  A ZIM's MIME list lives at the start of the
   file (header field ``mimeListPos``), so a 8 KiB HTTP range request against a
   Kiwix mirror is enough to learn whether an archive contains ``video/webm``,
   ``video/mp4``, ``video/ogg``, ... even for 90 GiB archives.  No archive is
   downloaded for this tier.
2. Download sample (expensive, deep).  A small, size-bounded, reproducible
   sample is downloaded and inspected with ``zimdump``: exact entry counts per
   MIME type plus a scan of article HTML for ``<video>``/``<source>`` markup.

All evidence is written under ``.native/video-survey/``.  The catalog snapshot
is cached so the run is reproducible offline; the MIME scan is cached per
archive so interrupted runs resume.

Usage:
    python3 scripts/video-format-survey.py catalog
    python3 scripts/video-format-survey.py mime-scan [--all-archives]
    python3 scripts/video-format-survey.py download [--budget-mb 400]
    python3 scripts/video-format-survey.py report
    python3 scripts/video-format-survey.py all
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import random
import re
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from pathlib import Path

CATALOG_URL = "https://library.kiwix.org/catalog/v2/entries?count=5000"
MIRROR = "https://mirror.download.kiwix.org"
FALLBACK = "https://lb.download.kiwix.org"
USER_AGENT = "kiwix-tauri-video-format-survey/1.0 (+https://github.com/kiwix/kiwix-tauri)"
SEED = 20261101
SAMPLE_BUDGET_MB = 400
SMALLEST_CAP_MB = 150       # largest archive eligible as the "smallest" pick
RANDOM_CAP_MB = 100         # cap for the seeded-random pick keeps the budget bounded
DOWNLOAD_CAP_MB = 150       # no single download above this
HTML_SCAN_LIMIT = 10        # article pages inspected per downloaded archive
# Targeted probes for MIME-list edge cases: phet's ``index.ts`` TypeScript file
# is labelled video/mp2t (MPEG-TS); nhs.uk embeds real Brightcove HLS segments.
PROBES = ("phet_oc_all", "nhs.uk_en_medicines", "lost-stats.github.io_en_all")
RANGE_BYTES = 8192

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / ".native" / "video-survey"
ARCHIVES = OUT / "archives"
CATALOG_XML = OUT / "catalog-entries-2026-11-01.xml"
CATALOG_JSON = OUT / "catalog-entries.json"
MIME_CACHE = OUT / "mime-scan.jsonl"
SAMPLE_JSON = OUT / "sample.json"
ANALYSIS_JSON = OUT / "archive-analysis.jsonl"
SUMMARY_JSON = OUT / "survey-summary.json"
TABLE_MD = OUT / "table.md"

NS = {"a": "http://www.w3.org/2005/Atom", "dc": "http://purl.org/dc/terms/"}
VIDEO_MIMES = ("video/webm", "video/mp4", "video/ogg", "video/x-matroska")


# --------------------------------------------------------------------------
# catalog
# --------------------------------------------------------------------------

def catalog_fetch(url: str = CATALOG_URL, force: bool = False) -> list[dict]:
    OUT.mkdir(parents=True, exist_ok=True)
    if force or not CATALOG_XML.exists():
        req = urllib.request.Request(
            url, headers={"Accept": "application/atom+xml", "User-Agent": USER_AGENT}
        )
        with urllib.request.urlopen(req, timeout=180) as resp:
            body = resp.read()
        CATALOG_XML.write_bytes(body)
        print(f"catalog: fetched {len(body)} bytes -> {CATALOG_XML}", file=sys.stderr)

    root = ET.parse(CATALOG_XML).getroot()
    total = root.findtext("a:totalResults", namespaces=NS)
    entries = []
    for node in root.findall("a:entry", NS):
        def text(path: str) -> str:
            found = node.find(path, NS)
            return (found.text or "").strip() if found is not None else ""

        link = None
        size = 0
        for candidate in node.findall("a:link", NS):
            if candidate.get("type") == "application/x-zim":
                link = candidate.get("href")
                size = int(candidate.get("length") or 0)
        if not link:
            continue
        entries.append(
            {
                "name": text("a:name"),
                "title": text("a:title"),
                "summary": text("a:summary"),
                "language": text("a:language"),
                "tags": [tag for tag in text("a:tags").split(";") if tag],
                "category": text("a:category"),
                "flavour": text("a:flavour"),
                "articles": int(text("a:articleCount") or 0),
                "media": int(text("a:mediaCount") or 0),
                "size": size,
                "url": link,
                "issued": text("dc:issued"),
            }
        )
    entries.sort(key=lambda e: e["name"])
    payload = {
        "catalog_url": url,
        "fetched_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "total_results": int(total) if total else len(entries),
        "entries": entries,
    }
    CATALOG_JSON.write_text(json.dumps(payload, indent=1))
    print(
        f"catalog: {len(entries)} entries (totalResults={payload['total_results']})",
        file=sys.stderr,
    )
    return entries


def load_catalog() -> dict:
    if not CATALOG_JSON.exists():
        catalog_fetch()
    return json.loads(CATALOG_JSON.read_text())


def family(entry: dict) -> str:
    match = re.search(r"/zim/([^/]+)/", entry["url"])
    return match.group(1) if match else "?"


def is_video_tagged(entry: dict) -> bool:
    return "_videos:yes" in entry["tags"]


def zim_url(entry: dict) -> str:
    """The catalog links a .meta4; mirrors serve the archive at the same path."""
    return entry["url"][: -len(".meta4")] if entry["url"].endswith(".meta4") else entry["url"]


# --------------------------------------------------------------------------
# MIME-list range scan
# --------------------------------------------------------------------------

def _range_get(url: str, start: int, end: int) -> bytes:
    headers = {
        "Range": f"bytes={start}-{end}",
        "User-Agent": USER_AGENT,
        "Accept-Encoding": "identity",
    }
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=60) as resp:
        limit = end - start + 2
        if resp.status == 206:
            return resp.read(limit)
        if resp.status != 200 or start != 0:
            raise RuntimeError(f"HTTP {resp.status}")
        # A 200 means the server ignored Range and is streaming the whole file.
        blob = resp.read(limit)
        if len(blob) >= limit:
            raise RuntimeError("HTTP 200 ignored Range request")
        return blob


def parse_zim_header(blob: bytes) -> dict:
    if len(blob) < 80 or blob[:4] != b"ZIM\x04":
        raise ValueError("not a ZIM (bad magic)")
    major, minor = struct.unpack("<HH", blob[4:8])
    article_count, cluster_count = struct.unpack("<II", blob[24:32])
    url_ptr, title_ptr, cluster_ptr, mime_list = struct.unpack("<QQQQ", blob[32:64])
    return {
        "major": major,
        "minor": minor,
        "entry_count": article_count,
        "cluster_count": cluster_count,
        "url_ptr_pos": url_ptr,
        "title_ptr_pos": title_ptr,
        "cluster_ptr_pos": cluster_ptr,
        "mime_list_pos": mime_list,
    }


def parse_mime_list(blob: bytes, offset: int) -> list[str]:
    if offset >= len(blob):
        raise ValueError("MIME list outside fetched range")
    mimes: list[str] = []
    index = offset
    while index < len(blob):
        end = blob.find(b"\x00", index)
        if end < 0:
            raise ValueError("unterminated MIME list")
        if end == index:
            return mimes
        mimes.append(blob[index:end].decode("utf-8", "replace"))
        index = end + 1
    raise ValueError("unterminated MIME list")


def scan_mime(entry: dict) -> dict:
    """Read only the ZIM header + MIME list over HTTP range requests."""
    path = zim_url(entry)
    candidates = [path.replace("https://lb.download.kiwix.org", MIRROR), path]
    last_error = None
    for url in candidates:
        try:
            blob = _range_get(url, 0, RANGE_BYTES - 1)
            header = parse_zim_header(blob)
            try:
                mimes = parse_mime_list(blob, header["mime_list_pos"])
            except ValueError:
                # MIME list longer than the first range: refetch generously.
                blob = _range_get(url, 0, 65535)
                mimes = parse_mime_list(blob, header["mime_list_pos"])
            return {
                "name": entry["name"],
                "zim": Path(zim_url(entry)).name,
                "family": family(entry),
                "size": entry["size"],
                "tags": entry["tags"],
                "status": "ok",
                "mimes": mimes,
                **header,
            }
        except Exception as exc:  # noqa: BLE001 - recorded as evidence
            last_error = f"{type(exc).__name__}: {exc}"
    return {
        "name": entry["name"],
        "zim": Path(zim_url(entry)).name,
        "family": family(entry),
        "size": entry["size"],
        "tags": entry["tags"],
        "status": "error",
        "error": last_error,
    }


def load_mime_cache() -> dict[str, dict]:
    cache: dict[str, dict] = {}
    if MIME_CACHE.exists():
        for line in MIME_CACHE.read_text().splitlines():
            if line.strip():
                record = json.loads(line)
                cache[record.get("zim") or record["name"]] = record
    return cache


def mime_scan(entries: list[dict], video_only: bool, workers: int = 8) -> None:
    cache = load_mime_cache()
    if video_only:
        targets = [e for e in entries if is_video_tagged(e)]
    else:
        targets = list(entries)
    def key(entry: dict) -> str:
        return Path(zim_url(entry)).name

    todo = [e for e in targets if key(e) not in cache or cache[key(e)]["status"] != "ok"]
    print(
        f"mime-scan: {len(targets)} targets, {len(targets) - len(todo)} cached, {len(todo)} to fetch",
        file=sys.stderr,
    )
    if not todo:
        return

    OUT.mkdir(parents=True, exist_ok=True)
    with MIME_CACHE.open("a") as sink, concurrent.futures.ThreadPoolExecutor(workers) as pool:
        futures = {pool.submit(scan_mime, entry): entry for entry in todo}
        done = 0
        for future in concurrent.futures.as_completed(futures):
            record = future.result()
            cache[record.get("zim") or record["name"]] = record
            sink.write(json.dumps(record) + "\n")
            sink.flush()
            done += 1
            if done % 50 == 0:
                print(f"mime-scan: {done}/{len(todo)}", file=sys.stderr)


# --------------------------------------------------------------------------
# download sample
# --------------------------------------------------------------------------

def select_sample(entries: list[dict], budget_mb: int = SAMPLE_BUDGET_MB) -> list[dict]:
    """Reproducible stratified, budget-bounded download sample.

    Stratum = catalog download family (``/zim/<family>/``), a proxy for the
    scraper recipe (ted2zim, youtube2zim, zimit, openedx, ...).  Candidate
    pools are video-tagged entries under the size caps, sorted by name, so the
    seeded random draw is stable across catalog snapshots that only add new
    archives.  Two candidates per stratum (smallest, seeded random) are added
    round-robin; a candidate that would exceed the budget is skipped and
    recorded.
    """
    rng = random.Random(SEED)
    strata: dict[str, list[dict]] = defaultdict(list)
    for entry in entries:
        if is_video_tagged(entry):
            strata[family(entry)].append(entry)

    per_stratum: dict[str, list[dict]] = {}
    for name, pool in strata.items():
        pool = sorted(pool, key=lambda e: e["name"])
        smallest_pool = [e for e in pool if e["size"] <= SMALLEST_CAP_MB * 1_000_000]
        random_pool = [e for e in pool if e["size"] <= RANDOM_CAP_MB * 1_000_000]
        picks = []
        if smallest_pool:
            picks.append({"entry": min(smallest_pool, key=lambda e: e["size"]), "reason": "smallest"})
        if random_pool:
            picks.append({"entry": rng.choice(random_pool), "reason": "seeded-random"})
        per_stratum[name] = picks

    order = sorted(per_stratum, key=lambda name: (-len(strata[name]), name))
    selected: list[dict] = []
    seen: set[str] = set()
    budget = budget_mb * 1_000_000
    used = 0
    skipped: list[dict] = []
    for round_index in (0, 1):
        for name in order:
            picks = per_stratum[name]
            if round_index >= len(picks):
                continue
            pick = picks[round_index]
            entry = pick["entry"]
            if entry["name"] in seen:
                continue
            if entry["size"] > DOWNLOAD_CAP_MB * 1_000_000:
                skipped.append({"name": entry["name"], "reason": "over download cap"})
                continue
            if used + entry["size"] > budget:
                skipped.append(
                    {
                        "name": entry["name"],
                        "reason": f"budget ({used / 1e6:.0f} MiB used, need {entry['size'] / 1e6:.0f} MiB)",
                    }
                )
                continue
            seen.add(entry["name"])
            used += entry["size"]
            selected.append({**entry, "family": name, "reason": pick["reason"]})

    # Control: the smallest entry from a non-video-tagged family, to measure how
    # specific the catalog's _videos tag is.
    controls = [
        e
        for e in entries
        if not is_video_tagged(e) and e["size"] <= 10_000_000 and family(e) in {"wikipedia", "other", "zimit"}
    ]
    for control in sorted(controls, key=lambda e: (family(e), e["size"]))[:2]:
        if control["name"] in seen:
            continue
        selected.append({**control, "family": family(control), "reason": "control (not video-tagged)"})
        used += control["size"]

    # Targeted probes for MIME-list edge cases (not part of the random sample).
    probes = []
    for probe_name in PROBES:
        probe = next((e for e in entries if e["name"] == probe_name), None)
        if probe is None or probe["name"] in seen:
            continue
        selected.append({**probe, "family": family(probe), "reason": "targeted probe"})
        probes.append(probe["name"])
        used += probe["size"]

    sample = {
        "seed": SEED,
        "budget_mb": budget_mb,
        "catalog_snapshot": json.loads(CATALOG_JSON.read_text())["fetched_at"] if CATALOG_JSON.exists() else None,
        "universe": len(entries),
        "video_tagged": sum(1 for e in entries if is_video_tagged(e)),
        "rules": {
            "stratum": "download family (/zim/<family>/)",
            "smallest_cap_mb": SMALLEST_CAP_MB,
            "random_cap_mb": RANDOM_CAP_MB,
            "download_cap_mb": DOWNLOAD_CAP_MB,
            "picks": "smallest + seeded-random per stratum, round-robin, budget-bounded",
        },
        "selected": selected,
        "probes": probes,
        "skipped": skipped,
        "total_mb": round(used / 1e6, 1),
    }
    SAMPLE_JSON.write_text(json.dumps(sample, indent=1))
    print(
        f"sample: {len(selected)} archives, {sample['total_mb']} MiB, {len(skipped)} skipped",
        file=sys.stderr,
    )
    return selected


def download(entry: dict) -> Path:
    target = ARCHIVES / Path(zim_url(entry)).name
    if target.exists() and target.stat().st_size > 0:
        # Catalog lengths can be rounded; trust an existing complete-looking file.
        if abs(target.stat().st_size - entry["size"]) <= max(1_000_000, entry["size"] * 0.02):
            return target
    ARCHIVES.mkdir(parents=True, exist_ok=True)
    partial = target.with_suffix(target.suffix + ".part")
    # lb.download.kiwix.org redirects to a fast MirrorBrain mirror; the plain
    # mirror host throttles sustained downloads.  Keep both as fallbacks.
    urls = [zim_url(entry), zim_url(entry).replace("https://lb.download.kiwix.org", MIRROR)]
    last_error = None
    for url in urls:
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=600) as resp, partial.open("wb") as sink:
                while chunk := resp.read(1 << 20):
                    sink.write(chunk)
            partial.replace(target)
            return target
        except Exception as exc:  # noqa: BLE001
            last_error = exc
            partial.unlink(missing_ok=True)
    raise RuntimeError(f"download failed for {entry['name']}: {last_error}")


def zimdump_entries(path: Path) -> list[dict]:
    """Parse ``zimdump list --details`` into path/mime records."""
    output = subprocess.run(
        ["zimdump", "list", "--details", str(path)],
        check=True,
        capture_output=True,
        text=True,
        errors="replace",
    ).stdout
    records = []
    current: dict = {}
    for line in output.splitlines():
        if line.startswith("path: "):
            if current.get("path"):
                records.append(current)
            current = {"path": line[len("path: "):]}
        elif line.startswith("* mime-type:"):
            current["mime"] = line.split(":", 1)[1].strip()
        elif line.startswith("* idx:"):
            current["idx"] = int(line.split(":", 1)[1].strip())
        elif line.startswith("* type:"):
            current["type"] = line.split(":", 1)[1].strip()
    if current.get("path"):
        records.append(current)
    return records


def split_namespace(path: str) -> tuple[str, str]:
    """Old ZIMs list entries as ``A/<url>``; newer ones omit the namespace."""
    match = re.match(r"^([A-Za-z])/(.*)$", path)
    if match:
        return match.group(1), match.group(2)
    return "C", path


def zim_show(path: Path, entry_path: str) -> str | None:
    namespace, url = split_namespace(entry_path)
    try:
        result = subprocess.run(
            ["zimdump", "show", f"--url={url}", f"--ns={namespace}", str(path)],
            check=True,
            capture_output=True,
            text=True,
            errors="replace",
            timeout=120,
        )
    except Exception:  # noqa: BLE001 - recorded by omission
        return None
    if result.stdout.strip() in {"", "Entry not found"}:
        return None
    return result.stdout


def scan_markup(body: str, hits: Counter) -> None:
    lowered = body.lower()
    hits["video_tag"] += lowered.count("<video")
    hits["source_webm"] += len(re.findall(r'type="video/webm', lowered))
    hits["source_mp4"] += len(re.findall(r'type="video/mp4', lowered))
    hits["source_ogg"] += len(re.findall(r'type="video/ogg', lowered))
    hits["webm_path"] += len(re.findall(r"\.webm\b", lowered))
    hits["mp4_path"] += len(re.findall(r"\.mp4\b", lowered))
    hits["ogvjs"] += lowered.count("ogvjs") + lowered.count("ogv.js")
    hits["hls_playlist"] += lowered.count(".m3u8")
    # YouTube/ReplayWeb player JSON (zimit) declares containers as mimeType.
    hits["json_mime_webm"] += len(re.findall(r'"mimetype"\s*:\s*"video/webm', lowered))
    hits["json_mime_mp4"] += len(re.findall(r'"mimetype"\s*:\s*"video/mp4', lowered))


COMMON_TOKENS = {"video", "index", "thumb", "poster", "subs", "assets", "webm", "mp4", "xblock"}


def pick_html_pages(records: list[dict], main_page: str | None, limit: int) -> list[str]:
    """Prefer pages that can reference a media entry over alphabetical noise."""
    html = [r["path"] for r in records if r.get("mime", "").startswith("text/html")]
    videos = [r["path"] for r in records if r.get("mime", "").startswith("video/")]
    ranked: list[str] = []
    if main_page:
        ranked.extend(p for p in html if p.endswith(main_page) or p == main_page)
    # Pages in the same directory tree as a video (Open edX xblock layout).
    prefixes = set()
    for video in videos:
        parts = video.split("/")
        if len(parts) >= 3:
            prefixes.add("/".join(parts[:-2]))
    ranked.extend(p for p in html if any(p.startswith(prefix + "/") for prefix in prefixes))
    # Pages whose name shares a distinctive token with a video filename (wikis).
    tokens = set()
    for video in videos:
        stem = video.rsplit("/", 1)[-1].rsplit(".", 1)[0].lower()
        for token in re.split(r"[^a-z0-9]+", stem):
            if len(token) >= 6 and token not in COMMON_TOKENS:
                tokens.add(token)
    ranked.extend(p for p in html if any(token in p.lower() for token in tokens))
    ranked.extend(html)
    seen: set[str] = set()
    picked = []
    for page in ranked:
        if page in seen:
            continue
        seen.add(page)
        picked.append(page)
        if len(picked) >= limit:
            break
    return picked


def html_scan(path: Path, records: list[dict], main_page: str | None = None) -> dict:
    hits = Counter()
    scanned = []
    video_paths: list[str] = []
    html_paths = pick_html_pages(records, main_page, HTML_SCAN_LIMIT)
    for html_path in html_paths:
        body = zim_show(path, html_path)
        if body is None:
            continue
        scanned.append(html_path)
        scan_markup(body, hits)
    # SPA ZIMs (youtube2zim, zimit) reference media from JSON or HLS manifests,
    # not from <video> markup.  Scan a bounded number of data entries too.
    data_paths = [
        r["path"]
        for r in records
        if r.get("mime", "").startswith(("application/json", "application/x-mpegURL"))
    ]
    for data_path in data_paths[:20]:
        body = zim_show(path, data_path)
        if body is None:
            continue
        scanned.append(data_path)
        scan_markup(body, hits)
        for match in re.finditer(r'"videoPath"\s*:\s*"([^"]+)"', body):
            video_paths.append(match.group(1))
        for match in re.finditer(r'"(?:url|src)"\s*:\s*"([^"]+\.(?:webm|mp4|m3u8))"', body):
            video_paths.append(match.group(1))
    for video_path in video_paths:
        hits[f"json_{video_path.rsplit('.', 1)[-1]}"] += 1
    return {"scanned": scanned, "hits": dict(hits), "video_paths": video_paths[:20]}


def analyze_archive(entry: dict) -> dict:
    path = download(entry)
    records = zimdump_entries(path)
    counts = Counter(r.get("mime", "?") for r in records)
    info = subprocess.run(
        ["zimdump", "info", str(path)], check=True, capture_output=True, text=True
    ).stdout
    main_page = next(
        (line.split(":", 1)[1].strip() for line in info.splitlines() if line.startswith("main page:")),
        None,
    )
    result = {
        "name": entry["name"],
        "zim": path.name,
        "family": entry.get("family", family(entry)),
        "reason": entry.get("reason"),
        "size": entry["size"],
        "downloaded_bytes": path.stat().st_size,
        "entries": len(records),
        "video_counts": {mime: counts.get(mime, 0) for mime in VIDEO_MIMES},
        "other_media_counts": {
            mime: count
            for mime, count in sorted(counts.items())
            if mime.startswith(("video/", "audio/")) or mime in {"application/octet-stream"}
        },
        "mime_counts": dict(sorted(counts.items())),
        "html": html_scan(path, records, main_page),
    }
    return result


def download_and_analyze(entries: list[dict], budget_mb: int = SAMPLE_BUDGET_MB) -> None:
    sample = select_sample(entries, budget_mb)
    existing = {}
    if ANALYSIS_JSON.exists():
        for line in ANALYSIS_JSON.read_text().splitlines():
            if line.strip():
                record = json.loads(line)
                existing[record.get("zim") or record["name"]] = record
    for entry in sample:
        key = Path(zim_url(entry)).name
        if key in existing:
            continue
        print(f"download: {entry['name']} ({entry['size'] / 1e6:.1f} MiB)", file=sys.stderr)
        result = analyze_archive(entry)
        existing[key] = result
        with ANALYSIS_JSON.open("a") as sink:
            sink.write(json.dumps(result) + "\n")
    # Records outside today's sample are retained evidence, not stale rows.
    ANALYSIS_JSON.write_text("".join(json.dumps(r) + "\n" for r in existing.values()))
    print(f"download: analyzed {len(sample)} archives", file=sys.stderr)


# --------------------------------------------------------------------------
# report
# --------------------------------------------------------------------------

def classify(mimes: list[str]) -> str:
    """Container class from the ZIM MIME list (presence, not entry counts)."""
    present = {mime for mime in mimes if mime.startswith("video/")}
    if not present:
        return "none"
    webm = "video/webm" in present
    mp4 = "video/mp4" in present
    if webm and mp4:
        return "mixed"
    if webm:
        return "webm-only"
    if mp4:
        return "mp4-only"
    return "other-only"


MISLABELED_FAMILIES = {"phet"}  # probe: phet's only video/mp2t entry is a TypeScript index.ts


def report() -> None:
    catalog = load_catalog()
    entries = catalog["entries"]
    by_zim = {Path(zim_url(e)).name: e for e in entries}
    cache = load_mime_cache()
    scanned = [r for r in cache.values() if r["status"] == "ok"]

    def is_mislabeled(record: dict) -> bool:
        return record["family"] in MISLABELED_FAMILIES and classify(record["mimes"]) == "other-only"

    def summarize(records: list[dict]) -> dict:
        kinds = Counter(classify(r["mimes"]) for r in records)
        bearing = [r for r in records if any(m.startswith("video/") for m in r["mimes"])]
        corrected = [r for r in bearing if not is_mislabeled(r)]
        sizes = Counter()
        for r in bearing:
            sizes[classify(r["mimes"])] += r["size"]
        corrected_sizes = Counter()
        for r in corrected:
            corrected_sizes[classify(r["mimes"])] += r["size"]
        return {
            "archives_scanned": len(records),
            "with_any_video_mime": len(bearing),
            "mislabeled_mime_archives": len(bearing) - len(corrected),
            "corrected_video_bearing": len(corrected),
            "classes": dict(kinds),
            "corrected_classes": dict(Counter(classify(r["mimes"]) for r in corrected)),
            "webm_present": sum(1 for r in corrected if "video/webm" in r["mimes"]),
            "mp4_present": sum(1 for r in corrected if "video/mp4" in r["mimes"]),
            "ogg_present": sum(1 for r in corrected if "video/ogg" in r["mimes"]),
            "size_gb_by_class": {k: round(v / 1e9, 2) for k, v in sizes.items()},
            "corrected_size_gb_by_class": {k: round(v / 1e9, 2) for k, v in corrected_sizes.items()},
        }

    tagged = [r for r in scanned if is_video_tagged(by_zim[r["zim"]])]
    untagged = [r for r in scanned if not is_video_tagged(by_zim[r["zim"]])]
    bearing = [r for r in scanned if any(m.startswith("video/") for m in r["mimes"])]
    corrected = [r for r in bearing if not is_mislabeled(r)]

    def share(n: int, total: int) -> float:
        return round(100 * n / total, 1) if total else 0.0

    family_stats = {}
    by_family: dict[str, list[dict]] = defaultdict(list)
    for record in scanned:
        by_family[record["family"]].append(record)
    for name, records in sorted(by_family.items(), key=lambda kv: -len(kv[1])):
        classes = Counter(classify(r["mimes"]) for r in records)
        if classes.get("none", 0) == len(records):
            continue
        family_stats[name] = {
            "archives": len(records),
            "classes": dict(classes),
            "mislabeled_mime": sum(1 for r in records if is_mislabeled(r)),
        }

    summary = {
        "catalog": {
            "url": catalog["catalog_url"],
            "fetched_at": catalog["fetched_at"],
            "total_results": catalog["total_results"],
            "entries_parsed": len(entries),
            "total_size_gb": round(sum(e["size"] for e in entries) / 1e9, 1),
            "video_tagged": len(tagged),
            "video_tagged_by_family": dict(Counter(family(e) for e in entries if is_video_tagged(e)).most_common()),
        },
        "mime_scan": {
            "archives_scanned": len(scanned),
            "video_tagged": summarize(tagged),
            "not_video_tagged": summarize(untagged),
            "all_scanned": summarize(scanned),
        },
        "headline": {
            "corrected_video_bearing": len(corrected),
            "share_of_catalog_pct": share(len(corrected), len(scanned)),
            "webm_only": sum(1 for r in corrected if classify(r["mimes"]) == "webm-only"),
            "mp4_only": sum(1 for r in corrected if classify(r["mimes"]) == "mp4-only"),
            "mixed": sum(1 for r in corrected if classify(r["mimes"]) == "mixed"),
            "other_only": sum(1 for r in corrected if classify(r["mimes"]) == "other-only"),
            "webm_only_pct": share(sum(1 for r in corrected if classify(r["mimes"]) == "webm-only"), len(corrected)),
            "mp4_capable_pct": share(sum(1 for r in corrected if "video/mp4" in r["mimes"]), len(corrected)),
            "webm_present_pct": share(sum(1 for r in corrected if "video/webm" in r["mimes"]), len(corrected)),
        },
        "tag_accuracy": {
            "tagged_archives": len(tagged),
            "tagged_with_video_mime": sum(1 for r in tagged if any(m.startswith("video/") for m in r["mimes"])),
            "tag_precision_pct": share(
                sum(1 for r in tagged if any(m.startswith("video/") for m in r["mimes"])), len(tagged)
            ),
            "untagged_archives": len(untagged),
            "untagged_with_video_mime": sum(1 for r in untagged if any(m.startswith("video/") for m in r["mimes"])),
        },
        "by_family": family_stats,
        "download_sample": [],
    }

    if ANALYSIS_JSON.exists():
        analyses = [json.loads(line) for line in ANALYSIS_JSON.read_text().splitlines() if line.strip()]
        summary["download_sample"] = analyses

    SUMMARY_JSON.write_text(json.dumps(summary, indent=1))

    lines = [
        "| archive | family | size | video/webm | video/mp4 | video/ogg | references found |",
        "|---|---|---:|---:|---:|---:|---|",
    ]
    if ANALYSIS_JSON.exists():
        for record in summary["download_sample"]:
            hits = record["html"]["hits"]
            notes = []
            if hits.get("source_webm"):
                notes.append(f"`<source type=video/webm>`×{hits['source_webm']}")
            if hits.get("source_mp4"):
                notes.append(f"`<source type=video/mp4>`×{hits['source_mp4']}")
            if hits.get("json_webm"):
                notes.append(f"JSON videoPath .webm×{hits['json_webm']}")
            if hits.get("json_mp4"):
                notes.append(f"JSON videoPath .mp4×{hits['json_mp4']}")
            if hits.get("json_mime_webm"):
                notes.append(f'JSON mimeType webm×{hits["json_mime_webm"]}')
            if hits.get("json_mime_mp4"):
                notes.append(f'JSON mimeType mp4×{hits["json_mime_mp4"]}')
            if hits.get("hls_playlist"):
                notes.append(f".m3u8×{hits['hls_playlist']}")
            if hits.get("ogvjs"):
                notes.append(f"ogv.js×{hits['ogvjs']}")
            if not notes:
                notes.append("none in sampled pages")
            lines.append(
                "| {name} | {family} | {mb:.1f} MiB | {webm} | {mp4} | {ogg} | {notes} |".format(
                    name=record["name"],
                    family=record["family"],
                    mb=record["size"] / 1e6,
                    webm=record["video_counts"].get("video/webm", 0),
                    mp4=record["video_counts"].get("video/mp4", 0),
                    ogg=record["video_counts"].get("video/ogg", 0),
                    notes=", ".join(notes),
                )
            )
    TABLE_MD.write_text("\n".join(lines) + "\n")
    print(json.dumps({k: v for k, v in summary.items() if k != "download_sample"}, indent=1))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "stage",
        choices=["catalog", "mime-scan", "download", "report", "all"],
        nargs="?",
        default="all",
    )
    parser.add_argument("--all-archives", action="store_true", help="MIME-scan the whole catalog, not just _videos:yes")
    parser.add_argument("--budget-mb", type=int, default=SAMPLE_BUDGET_MB)
    parser.add_argument("--workers", type=int, default=8)
    args = parser.parse_args()

    if args.stage in {"catalog", "all"}:
        entries = catalog_fetch(force=False)
    else:
        entries = load_catalog()["entries"]

    if args.stage in {"mime-scan", "all"}:
        mime_scan(entries, video_only=not args.all_archives, workers=args.workers)

    if args.stage in {"download", "all"}:
        download_and_analyze(entries, budget_mb=args.budget_mb)

    if args.stage in {"report", "all"}:
        report()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
