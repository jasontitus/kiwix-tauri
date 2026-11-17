#!/usr/bin/env python3
"""Stage one unchanged public production recipe and its real archives, offline.

Run after generate-android-evaluation.py:
  python3 scripts/stage-streetzim-evaluation.py --output .native/evaluation
Downloads only public StreetZim sources; no hosting/cloud operations.
The original catalog, recipe and all checksums are retained as evidence.
Before large device-bound preparation, add --device SERIAL --target-abi
armeabi-v7a --target-apk PATH --max-ram-mib 2048. The live info-only Android
probe must admit the actual installed APK/RAM/ABI before any download.
--probe-only stops after that admission; no --device is explicitly unmeasured
host-only preparation. Neither path submits a cloud job or proves physicality.
--device SERIAL --inventory-only records Java/platform inventory without native
loading, downloads, generation or granting permission to stage fixtures.
For the real >4 GiB workload, use a fresh output directory:
  python3 scripts/stage-streetzim-evaluation.py --output .native/evaluation-real-xlarge --region austria-czech --edition de --tier xlarge
Non-prepare runs require android-assets/evaluation/manifest.json in that root.
Preflight requires all source bytes + retained output bytes + 512 MiB free,
in addition to previously retained files. Source chunks and inline segments
are limited to 64 MiB; article selection checks at most 512 paths per language.
"""

import argparse
import base64
from contextlib import ExitStack
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import shutil
import subprocess
import time
import urllib.parse
import urllib.request

from android_evaluation_hardware import add_hardware_arguments, hardware_preflight

from libzim.reader import Archive
from libzim.search import Query, Searcher
from libzim.suggestion import SuggestionSearcher

CATALOG_URL = "https://streetzim.web.app/build/recipes/index.json"
PUBLIC_ARCHIVE_ROOT = "https://archive.org/download"
CHUNK = 1024 * 1024
MAX_SAFE_SIZE = (1 << 53) - 1
MAX_RECIPE_BLOCK = 64 * CHUNK
DISK_RESERVE = 512 * CHUNK
MAX_ARTICLE_CANDIDATES = 512
MAX_LISTING_LINE = 64 * 1024


def digest(path):
    sha, md5 = hashlib.sha256(), hashlib.md5()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(CHUNK), b""):
            sha.update(block)
            md5.update(block)
    return {"size": path.stat().st_size, "sha256": sha.hexdigest(), "file_md5": md5.hexdigest()}


def save_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


class HttpsRedirectsOnly(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        require_https(newurl)
        return super().redirect_request(request, fp, code, message, headers, newurl)


def require_https(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("only credential-free HTTPS source URLs are permitted")


def download(url, path, expected_size=None):
    require_https(url)
    if path.exists():
        raise ValueError("refusing to overwrite evidence " + str(path))
    path.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(url, headers={"User-Agent": "Kiwix-public-evaluation/1", "Accept-Encoding": "identity"})
    opener = urllib.request.build_opener(HttpsRedirectsOnly())
    with opener.open(request, timeout=180) as response, path.open("xb") as output:
        total = 0
        while True:
            data = response.read(CHUNK)
            if not data:
                break
            total += len(data)
            if expected_size is not None and total > expected_size:
                raise ValueError("download exceeds declared source size")
            output.write(data)
    if expected_size is not None and total != expected_size:
        raise ValueError("download size mismatch")
    return digest(path)


def verify_source(path, record):
    if path.stat().st_size != record["size"]:
        raise ValueError("source size mismatch")
    with path.open("rb") as source:
        for index, expected in enumerate(record["sha256"]):
            block = bytearray(source.read(record["chunk"]))
            start = index * record["chunk"]
            for offset, length in record.get("mask", []):
                low, high = max(start, offset), min(start + len(block), offset + length)
                if low < high:
                    block[low-start:high-start] = bytes(high-low)
            if hashlib.sha256(block).hexdigest() != expected:
                raise ValueError("production masked chunk mismatch: " + path.name + " chunk " + str(index))
    archive = Archive(path)
    if not archive.check():
        raise ValueError("source ZIM checksum mismatch")
    return {"nativeChecksum": True, "uuid": str(archive.uuid), "chunks": len(record["sha256"]), "chunkBytes": record["chunk"], "masks": record.get("mask", [])}


def bounded_size(value, label, minimum=0, maximum=MAX_SAFE_SIZE):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError(label + " must be an integer in [" + str(minimum) + ", " + str(maximum) + "]")
    return value


def storage_preflight(recipe, source_records, root):
    """Validate byte accounting before downloading any archive."""
    output_bytes = bounded_size(recipe["size"], "output size", minimum=16)
    source_bytes = 0
    names = set()
    for key, record in source_records.items():
        size = bounded_size(record["size"], key + " source size", minimum=1)
        chunk = bounded_size(record["chunk"], key + " chunk size", minimum=1, maximum=MAX_RECIPE_BLOCK)
        if len(record["sha256"]) != (size + chunk - 1) // chunk:
            raise ValueError("source chunk hashes do not cover declared size: " + key)
        for offset, length in record.get("mask", []):
            bounded_size(offset, key + " mask offset", maximum=size)
            bounded_size(length, key + " mask length", maximum=size - offset)
        name = record["file"]
        if not isinstance(name, str) or not name or name in (".", "..") or Path(name).name != name or name in names:
            raise ValueError("source filenames must be distinct basenames")
        names.add(name)
        source_bytes += size
    assembled_bytes = 0
    for index, segment in enumerate(recipe["segments"]):
        kind = segment["src"]
        if kind == "md5":
            if index != len(recipe["segments"]) - 1:
                raise ValueError("production checksum must be the final segment")
            length = 16
        else:
            length = bounded_size(segment["len"], "segment length")
            if kind == "inline":
                bounded_size(length, "inline segment length", maximum=MAX_RECIPE_BLOCK)
                if len(segment["data"]) != 4 * ((length + 2) // 3):
                    raise ValueError("inline encoded size differs from declared length")
            elif kind in ("base", "pack"):
                source = source_records["base" if kind == "base" else segment["pack"]]
                offset = bounded_size(segment["off"], "segment offset", maximum=source["size"])
                if length > source["size"] - offset:
                    raise ValueError("segment exceeds declared source size")
            else:
                raise ValueError("unsupported production segment source")
        assembled_bytes += length
        if assembled_bytes > output_bytes:
            raise ValueError("segments exceed declared output size")
    if not recipe["segments"] or recipe["segments"][-1]["src"] != "md5" or assembled_bytes != output_bytes:
        raise ValueError("segments do not cover declared output and final checksum")
    required = source_bytes + output_bytes + DISK_RESERVE
    # Python integer sums cannot wrap; reject totals beyond interoperable recipe integers.
    bounded_size(required, "required storage", minimum=1)
    free = shutil.disk_usage(root).free
    result = {"sourceBytes": source_bytes, "retainedOutputBytes": output_bytes,
              "operationalReserveBytes": DISK_RESERVE, "requiredFreeBytes": required,
              "availableFreeBytes": free}
    if free < required:
        raise ValueError("insufficient free disk before archive downloads: " + json.dumps(result))
    return result


def assemble(recipe, sources, output_path):
    md5 = hashlib.md5()
    with ExitStack() as stack:
        files = {key: stack.enter_context(path.open("rb")) for key, path in sources.items()}
        output = stack.enter_context(output_path.open("xb"))
        for segment in recipe["segments"]:
            if segment["src"] == "md5":
                if md5.hexdigest() != recipe["md5"]:
                    raise ValueError("production assembly MD5 differs")
                output.write(md5.digest())
                continue
            if segment["src"] == "inline":
                data = base64.b64decode(segment["data"], validate=True)
                if len(data) != segment["len"] or hashlib.sha256(data).hexdigest() != segment["sha256"]:
                    raise ValueError("production inline hash mismatch")
                output.write(data)
                md5.update(data)
                continue
            source = files["base" if segment["src"] == "base" else segment["pack"]]
            source.seek(segment["off"])
            remaining = segment["len"]
            while remaining:
                data = source.read(min(CHUNK, remaining))
                if not data:
                    raise ValueError("truncated production source")
                output.write(data)
                md5.update(data)
                remaining -= len(data)
    actual = digest(output_path)
    if actual["size"] != recipe["size"] or actual["file_md5"] != recipe["file_md5"]:
        raise ValueError("production output identity mismatch")
    return actual


class VisibleText(HTMLParser):
    def __init__(self):
        super().__init__()
        self.hidden = 0
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if tag in ("head", "h1", "nav", "footer", "script", "style"):
            self.hidden += 1

    def handle_endtag(self, tag):
        if tag in ("head", "h1", "nav", "footer", "script", "style"):
            self.hidden = max(0, self.hidden - 1)

    def handle_data(self, data):
        if not self.hidden and len(data.strip()) >= 12:
            self.parts.append(data.strip())


def article_paths(listing_path, prefix):
    """Stream the on-disk listing, without materializing a large archive's paths."""
    with listing_path.open(encoding="utf-8") as listing:
        matched = 0
        while matched < MAX_ARTICLE_CANDIDATES:
            line = listing.readline(MAX_LISTING_LINE + 1)
            if not line:
                return
            if len(line) > MAX_LISTING_LINE:
                raise ValueError("archive listing path exceeds 64 Ki characters")
            path = line.rstrip("\r\n")
            if path.startswith(prefix):
                matched += 1
                yield path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(".native/evaluation"))
    parser.add_argument("--region", choices=("bahrain", "east-polynesia", "austria-czech"), default="bahrain")
    parser.add_argument("--edition", default="de")
    parser.add_argument("--tier", choices=("smoke", "city", "large", "xlarge"), default="smoke")
    parser.add_argument("--prepare-only", action="store_true", help="Stage and verify archives; write listing for assertion selection without changing manifest")
    add_hardware_arguments(parser)
    args = parser.parse_args()
    admission = hardware_preflight(args)
    print(json.dumps({"androidPreparation": admission}, ensure_ascii=False), flush=True)
    if args.probe_only or args.inventory_only:
        return
    root = args.output.resolve()
    assets = root / "android-assets" / "evaluation"
    case_id = "production-" + args.region + "-" + args.edition
    evidence = root / ("production-" + args.region)
    if evidence.exists():
        parser.error("production evidence already exists; choose fresh output")
    evidence.mkdir(parents=True)
    started = time.monotonic()
    catalog_path = evidence / "catalog.json"
    download(CATALOG_URL, catalog_path)
    catalog = json.loads(catalog_path.read_text())
    region = next(region for region in catalog["regions"] if region["id"] == args.region)
    menu = next(menu for menu in region["menu"] if menu["lang"] == args.edition)
    recipe_url = "https://streetzim.web.app/build/" + menu["recipe"]
    recipe_path = assets / "recipes" / (case_id + ".json")
    recipe_identity = download(recipe_url, recipe_path)
    recipe = json.loads(recipe_path.read_text())
    source_records = {"base": recipe["base"], **recipe["packs"]}
    preflight = storage_preflight(recipe, source_records, root)
    save_json(evidence / "storage-preflight.json", preflight)
    print(json.dumps({"storagePreflight": preflight}), flush=True)
    sources, provenance = {}, []
    for key, record in source_records.items():
        name = record["file"]
        if Path(name).name != name:
            raise ValueError("source filename must be a basename")
        path = assets / "candidates" / name
        url = region["base"]["download"] if key == "base" else record["url"].replace("{proxy}", PUBLIC_ARCHIVE_ROOT)
        identity = download(url, path, record["size"])
        checked = verify_source(path, record)
        sources[key] = path
        provenance.append({"key": key, "url": url, "path": str(path.relative_to(root)), **identity, **checked})
        print(json.dumps({"source": key, **identity}), flush=True)
    output_path = evidence / (case_id + ".zim")
    output = assemble(recipe, sources, output_path)
    archive = Archive(output_path)
    if not archive.check():
        raise ValueError("assembled production ZIM checksum failed")
    checked = subprocess.run(["zimcheck", "--checksum", "--integrity", str(output_path)], capture_output=True, text=True, check=True)
    listing_path = evidence / "entries.txt"
    with listing_path.open("x", encoding="utf-8") as listing:
        subprocess.run(["zimdump", "list", str(output_path)], stdout=listing, stderr=subprocess.PIPE, text=True, check=True)
    metadata = {key: archive.get_metadata(key).decode("utf-8") for key in archive.metadata_keys if not key.startswith("Illustration_")}
    result = {"schema": 1, "synthetic": False, "case": case_id, "catalogUrl": CATALOG_URL, "catalog": digest(catalog_path), "recipeUrl": recipe_url, "recipe": recipe_identity, "recipeSegments": len(recipe["segments"]), "sourceRecords": provenance, "output": {**output, "uuid": str(archive.uuid), "zimChecksumMd5": recipe["md5"]}, "metadata": metadata, "hasTitleIndex": archive.has_title_index, "hasFulltextIndex": archive.has_fulltext_index, "reusedBaseBytes": sum(s.get("len", 0) for s in recipe["segments"] if s["src"] == "base"), "packBytes": sum(s.get("len", 0) for s in recipe["segments"] if s["src"] == "pack"), "inlineBytes": sum(s.get("len", 0) for s in recipe["segments"] if s["src"] == "inline"), "zimcheckStdout": checked.stdout, "zimcheckStderr": checked.stderr, "hostElapsedSeconds": round(time.monotonic() - started, 6)}
    result["storagePreflight"] = preflight
    result["androidPreparation"] = admission
    result["articleSelectionLimits"] = {"articlePathsPerLanguage": MAX_ARTICLE_CANDIDATES,
                                       "htmlBytesPerArticle": CHUNK,
                                       "listingLineCharacters": MAX_LISTING_LINE,
                                       "titleSuggestionHits": 32, "fulltextHitsPerProbe": 100,
                                       "fulltextProbesPerArticle": 13}
    save_json(evidence / "provenance.json", result)
    print(json.dumps({"output": output, "metadata": metadata, "hasTitleIndex": archive.has_title_index, "hasFulltextIndex": archive.has_fulltext_index}, ensure_ascii=False), flush=True)
    result["opaqueRecipeGeneratedSha256"] = recipe.get("generated_sha256")
    result["opaqueRecipeFingerprint"] = recipe.get("fingerprint")
    if args.prepare_only:
        return
    # Read genuine per-language HTML. Search coverage differs by actual index:
    # this producer omits base-English titles from title lookup/suggestions.
    articles, searches, coverage = [], [], []
    content_languages = ["en"] + [pack["lang"] for pack in menu["packs"] if not pack.get("in_base")]
    suggestions = SuggestionSearcher(archive)
    searcher = Searcher(archive) if archive.has_fulltext_index else None
    for language in dict.fromkeys(content_languages):
        selected = None
        examined = 0
        prefix = "wiki-article/" if language == "en" else "~w/" + language + "/wiki-article/"
        for path in article_paths(listing_path, prefix):
            examined += 1
            entry = archive.get_entry_by_path(path)
            item = entry.get_item()
            if item.size > CHUNK or "html" not in item.mimetype:
                continue
            body = bytes(item.content).decode("utf-8")
            if '<html lang="' + language + '"' not in body:
                raise ValueError("actual article HTML language mismatch")
            text = VisibleText()
            text.feed(body)
            candidates = [part for part in text.parts if part[:80] in body and part != entry.title and not part.startswith("This page is issued")]
            if not candidates:
                continue
            query = entry.title
            results = list(suggestions.suggest(query).getResults(0, 32))
            fulltext, fulltext_query, probes = [], query, []
            if archive.has_fulltext_index:
                words = [word for word in re.findall(r"[^\W\d_]+", " ".join(candidates[:8]), flags=re.UNICODE) if len(word) >= 4]
                for probe in list(dict.fromkeys([query] + words[:12])):
                    fulltext = list(searcher.search(Query().set_query(probe)).getResults(0, 100))
                    probes.append({"query": probe, "selectedArticleHit": path in fulltext, "returnedHits": len(fulltext)})
                    if path in fulltext:
                        fulltext_query = probe
                        break
            if path not in results and path not in fulltext:
                continue
            try:
                title_lookup = archive.get_entry_by_title(entry.title).path == path
            except KeyError:
                title_lookup = False
            selected = {"path": path, "title": entry.title, "contains": candidates[0][:80]}
            articles.append(selected)
            if path in results:
                searches.append({"query": query, "fulltext": False, "expectedPath": path})
            if path in fulltext:
                searches.append({"query": fulltext_query, "fulltext": True, "expectedPath": path})
            coverage.append({"language": language, "path": path, "scope": "selected article only, not language-wide index coverage", "exactTitleLookup": title_lookup, "titleSuggestion": path in results, "fulltext": path in fulltext, "fulltextProbes": probes, "limitation": None if path in fulltext else "No selected-article hit for the recorded fulltext probes; language-wide support was not inferred"})
            coverage[-1]["articlePathsExamined"] = examined
            break
        if selected is None:
            raise ValueError("no verified native title/search article found for actual language " + language
                             + " after " + str(examined) + " article paths (limit " + str(MAX_ARTICLE_CANDIDATES) + ")")
    manifest_path = assets / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    if any(case["id"] == case_id for case in manifest["cases"]):
        raise ValueError("production case already in manifest")
    case = {"id": case_id, "tier": args.tier, "recipe": "recipes/" + recipe_path.name, "localCandidates": ["candidates/" + record["file"] for record in source_records.values()], "expected": {"size": output["size"], "sha256": output["sha256"], "uuid": str(archive.uuid), "languages": metadata["Language"].split(";"), "articles": articles, "searches": searches}}
    manifest["cases"].append(case)
    save_json(manifest_path, manifest)
    result.update({"manifest": digest(manifest_path), "articleAssertions": articles, "searchAssertions": searches, "contentLanguages": list(dict.fromkeys(content_languages))})
    result["origin"] = "production"
    result["searchCoverage"] = coverage
    result["generatorSha256"] = digest(Path(__file__))["sha256"]
    save_json(evidence / "provenance.json", result)
    print(json.dumps({"case": case, "manifest": digest(manifest_path)}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
