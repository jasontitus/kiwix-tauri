#!/usr/bin/env python3
"""Generate original, public synthetic StreetZim v2 Android evaluation inputs.

Requires python-libzim 3.1.0 (installed host binding) and zimcheck. No archive
downloads, personal archives, app builds or cloud submissions are used.
Run: python3 scripts/generate-android-evaluation.py --tiers smoke city large
Fixtures are ABI-independent. --target-abi armeabi-v7a --target-apk PATH
checks the actual ELF ABI without changing fixture bytes. Add --device SERIAL
--max-ram-mib 2048 to run a live, fixture-free installed-APK admission first;
--probe-only stops there. No device means host-only preparation, not evidence
of a low-RAM or physical pass. Build the fixture-free test APK first with
Gradle -PkiwixEmbedEvaluationFixtures=false; install it and the target APK.
For SDK-independent Java/platform inventory, build/install the separate
org.kiwix.workbench.inventory target and .inventory.test APKs first:
  ./gradlew :app:assembleDebug :app:assembleDebugAndroidTest -PkiwixEvaluationInventoryOnly=true
Then use --device SERIAL --inventory-only: no native load, fixture admission or
generation; it selects the separate inventory test runner, preserving reader data.
Opt-in >4 GiB case: add --tiers xlarge --xlarge-mib 4096 with a fresh --output.
The xlarge payload accepts 4096–6144 MiB; archive overhead makes even its
minimum output strictly larger than 4 GiB. It is not part of the default tiers.
Generation retains one base and temporarily holds a payload and either the
target or assembled output: budget three payload sizes plus 512 MiB free disk
per tier. The target is removed only after its recipe and hashes are captured.
Generated files stay under ignored .native/evaluation; never commit binaries.
The payload repeats a nonzero SHA-256-derived 16 KiB block with a unique
deterministic marker in EVERY 1 MiB chunk, stored without ZIM compression.
This forces distinct production chunk reads/hashes while compressing transport.
It is not representative of incompressible real map tiles.
"""

import argparse
import base64
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import resource
import shutil
import struct
import subprocess
import time
import uuid
import zlib

from android_evaluation_hardware import add_hardware_arguments, hardware_preflight

from libzim import get_versions
from libzim.reader import Archive
from libzim.search import Query, Searcher
from libzim.suggestion import SuggestionSearcher
from libzim.writer import Compression, Creator, FileProvider, Hint, Item, StringProvider

MIB = 1024 * 1024
CHUNK = MIB
NAMESPACE = uuid.UUID("c07ba030-d5ad-5e18-af36-b95448e8008c")
MARKER = b"KIWIX-SYNTHETIC-EVALUATION-PAYLOAD-V1\x00"
BLOCK = MARKER + b"".join(hashlib.sha256(b"kiwix-public-evaluation-v1:" + struct.pack("<I", n)).digest() for n in range(512))
BLOCK = BLOCK[:16384]
PAGES = {
    "eng": {"path": "en/orchard.html", "title": "Knowledge orchard", "contains": "An orchard grows apples, and a library grows shared knowledge.", "query": "orchard", "token": "orchardevaluation", "lang": "en", "dir": "ltr"},
    "ara": {"path": "ar/orchard.html", "title": "بستان المعرفة", "contains": "تنمو أشجار التفاح في البستان، وتنمو المعرفة عندما نتشارك الكتب.", "query": "بستان", "token": "bustanevaluation", "lang": "ar", "dir": "rtl"},
    "hin": {"path": "hi/orchard.html", "title": "ज्ञान उद्यान", "contains": "बगीचे में सेब के पेड़ उगते हैं और पुस्तकें बाँटने से ज्ञान बढ़ता है।", "query": "ज्ञान", "token": "udyanevaluation", "lang": "hi", "dir": "ltr"},
}


class Entry(Item):
    def __init__(self, path, title, content=None, file=None):
        super().__init__()
        self.path, self.title, self.content, self.file = path, title, content, file

    def get_path(self):
        return self.path

    def get_title(self):
        return self.title

    def get_mimetype(self):
        return "application/octet-stream" if self.file else "text/html"

    def get_contentprovider(self):
        # FileProvider streams even an extended (>4 GiB) uncompressed blob.
        # Never replace this with read_bytes()/StringProvider for payloads.
        return FileProvider(self.file) if self.file else StringProvider(self.content)

    def get_hints(self):
        return {Hint.FRONT_ARTICLE: not bool(self.file)}


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def digests(path):
    sha, md5 = hashlib.sha256(), hashlib.md5()
    with path.open("rb") as source:
        for data in iter(lambda: source.read(CHUNK), b""):
            sha.update(data)
            md5.update(data)
    return {"sha256": sha.hexdigest(), "file_md5": md5.hexdigest(), "size": path.stat().st_size}


def hash_prefix(path, size):
    digest = hashlib.md5()
    with path.open("rb") as source:
        while size:
            data = source.read(min(CHUNK, size))
            if not data:
                raise ValueError("truncated checksum input")
            digest.update(data)
            size -= len(data)
    return digest.digest()


def stabilize(path, identifier):
    # ZIM UUID is the 16-byte header field at byte offset 8. Regenerate the
    # format-defined trailing MD5 after deterministic identity normalization.
    with path.open("r+b") as output:
        output.seek(8)
        output.write(uuid.uuid5(NAMESPACE, identifier).bytes)
        # python-libzim 3.1.0 embeds two Xapian Glass 046e indexes. Their
        # random database UUIDs are the only other nondeterministic bytes.
        # Pin/check that observed format; normalize identity, not index data.
        magic = b"\x0f\x0dXapian Glass\x04\x6e"
        positions, tail, position = [], b"", 0
        output.seek(0)
        while True:
            block = output.read(CHUNK)
            if not block:
                break
            data = tail + block
            start = 0
            while True:
                found = data.find(magic, start)
                if found < 0:
                    break
                positions.append(position - len(tail) + found + len(magic))
                start = found + len(magic)
            tail = data[-(len(magic) - 1):]
            position += len(block)
        if len(positions) != 2:
            raise ValueError("expected exactly two pinned Xapian Glass indexes")
        for index, offset in enumerate(positions):
            output.seek(offset)
            output.write(uuid.uuid5(NAMESPACE, identifier + "-xapian-" + str(index)).bytes)
    checksum = hash_prefix(path, path.stat().st_size - 16)
    with path.open("r+b") as output:
        output.seek(-16, 2)
        output.write(checksum)


def create_archive(path, identifier, languages, payload):
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise ValueError("refusing to overwrite archive: " + str(path))
    metadata = {
        "Name": "synthetic_" + identifier,
        "Title": "Public synthetic evaluation " + identifier,
        "Description": "Original synthetic multilingual recipe evaluation; not Wikipedia or a real map",
        "Language": ";".join(languages),
        "Creator": "Kiwix Android evaluation fixture generator",
        "Publisher": "Kiwix Android evaluation fixture generator",
        "Date": "2026-01-01", "License": "CC0-1.0", "Tags": "_category:other;synthetic;evaluation",
    }
    with Creator(path).config_compression(Compression.none).config_nbworkers(1).config_clustersize(CHUNK).config_indexing(True, "eng") as creator:
        for key, value in metadata.items():
            creator.add_metadata(key, value)
        for language in languages:
            page = PAGES[language]
            body = '<!doctype html><html lang="{lang}" dir="{dir}"><meta charset="utf-8"><title>{title}</title><h1>{title}</h1><p>{contains}</p><p>{token}</p></html>'.format(**page)
            creator.add_item(Entry(page["path"], page["title"], body))
        creator.add_item(Entry("data/payload.bin", "Synthetic deterministic payload", file=payload))
        creator.set_mainpath(PAGES[languages[0]]["path"])
    stabilize(path, identifier)


def write_payload(path, size):
    with path.open("xb") as output:
        remaining = size
        # Bounded 1 MiB write buffer; no whole archive materialization.
        buffer = bytearray(BLOCK * (CHUNK // len(BLOCK)))
        index = 0
        while remaining:
            start = len(MARKER)
            buffer[start:start + 32] = hashlib.sha256(b"kiwix-unique-chunk-v1:" + struct.pack("<Q", index)).digest()
            index += 1
            part = buffer[:min(remaining, len(buffer))]
            output.write(part)
            remaining -= len(part)


def find_payload(path):
    with path.open("rb") as source:
        tail = b""
        position = 0
        while True:
            block = source.read(CHUNK)
            if not block:
                raise ValueError("uncompressed payload marker absent: " + str(path))
            data = tail + block
            at = data.find(MARKER)
            if at >= 0:
                return position - len(tail) + at
            tail = data[-len(MARKER):]
            position += len(block)


def copy_range(source, output, offset, length):
    source.seek(offset)
    while length:
        data = source.read(min(CHUNK, length))
        if not data:
            raise ValueError("truncated source range")
        output.write(data)
        length -= len(data)


def source_record(path, source_uuid):
    hashes = []
    with path.open("rb") as source:
        for data in iter(lambda: source.read(CHUNK), b""):
            hashes.append(hashlib.sha256(data).hexdigest())
    return {"file": path.name, "size": path.stat().st_size, "uuid": str(source_uuid), "chunk": CHUNK, "sha256": hashes, "url": "https://synthetic-fixtures.invalid/" + path.name}


def build_recipe(base, target, pack, identifier, payload_size):
    base_start, target_start = find_payload(base), find_payload(target)
    size = target.stat().st_size
    # Header changes are inline, the true shared payload comes from the English
    # base, and target directory/index/language article bytes come from the pack.
    with target.open("rb") as source, pack.open("xb") as output:
        header = source.read(80)
        copy_range(source, output, 80, target_start - 80)
        copy_range(source, output, target_start + payload_size, size - 16 - target_start - payload_size)
    with base.open("rb") as source:
        source.seek(8)
        base_uuid = uuid.UUID(bytes=source.read(16))
    expected = digests(target)
    recipe = {
        "format": "streetzim-recipe/2", "uuid": str(uuid.UUID(bytes=header[8:24])),
        "name": "synthetic_" + identifier, "file_name": identifier, "size": size,
        "md5": hash_prefix(target, size - 16).hex(), "file_md5": expected["file_md5"],
        "base": source_record(base, base_uuid),
        "packs": {"languages": source_record(pack, uuid.uuid5(NAMESPACE, identifier + "-pack"))},
        "segments": [
            {"src": "inline", "len": len(header), "data": base64.b64encode(header).decode("ascii"), "sha256": hashlib.sha256(header).hexdigest()},
            {"src": "pack", "pack": "languages", "off": 0, "len": target_start - 80},
            {"src": "base", "off": base_start, "len": payload_size},
            {"src": "pack", "pack": "languages", "off": target_start - 80, "len": size - 16 - target_start - payload_size},
            {"src": "md5"},
        ],
    }
    return recipe, expected


def check_archive(path, languages):
    archive = Archive(path)
    if not archive.check():
        raise ValueError("libzim checksum verification failed: " + str(path))
    if archive.get_metadata("Language").decode("utf-8").split(";") != languages:
        raise ValueError("metadata languages differ")
    articles, searches, observations = [], [], []
    for language in languages:
        page = PAGES[language]
        entry = archive.get_entry_by_path(page["path"])
        item = entry.get_item()
        # Materialize only these small HTML articles, never data/payload.bin.
        content = bytes(item.content).decode("utf-8")
        if entry.title != page["title"] or page["contains"] not in content:
            raise ValueError("article assertion failed")
        title_entry = archive.get_entry_by_title(page["title"])
        if title_entry.path != page["path"]:
            raise ValueError("title lookup assertion failed")
        articles.append({key: page[key] for key in ("path", "title", "contains")})
        suggestion = SuggestionSearcher(archive).suggest(page["query"])
        suggestions = list(suggestion.getResults(0, 32))
        suggestion_paths = suggestions
        if page["path"] not in suggestion_paths:
            raise ValueError("native title suggestion absent: " + repr(suggestion_paths))
        searches.append({"query": page["query"], "fulltext": False, "expectedPath": page["path"]})
        # The generated mixed-language index uses libzim's supported English
        # analyzer. Unique ASCII tokens make the fulltext expectation portable;
        # native-script title suggestions above remain independently required.
        result = Searcher(archive).search(Query().set_query(page["token"]))
        paths = list(result.getResults(0, 32))
        if page["path"] not in paths:
            raise ValueError("native fulltext result absent: " + repr(paths))
        searches.append({"query": page["token"], "fulltext": True, "expectedPath": page["path"]})
        observations.append({"language": language, "titleResults": suggestion_paths, "fulltextResults": paths})
    command = ["zimcheck", "--checksum", "--integrity", str(path)]
    checked = subprocess.run(command, capture_output=True, text=True, check=True)
    return articles, searches, {"libzimChecksum": True, "hasTitleIndex": archive.has_title_index, "hasFulltextIndex": archive.has_fulltext_index, "queries": observations, "zimcheckCommand": command, "zimcheckStdout": checked.stdout, "zimcheckStderr": checked.stderr}


def assemble(recipe, base, pack, destination):
    digest = hashlib.md5()
    with base.open("rb") as source_base, pack.open("rb") as source_pack, destination.open("xb") as output:
        for segment in recipe["segments"]:
            if segment["src"] == "md5":
                if digest.hexdigest() != recipe["md5"]:
                    raise ValueError("assembled checksum mismatch")
                output.write(digest.digest())
                continue
            if segment["src"] == "inline":
                data = base64.b64decode(segment["data"])
                digest.update(data)
                output.write(data)
                continue
            source = source_base if segment["src"] == "base" else source_pack
            source.seek(segment["off"])
            remaining = segment["len"]
            while remaining:
                data = source.read(min(CHUNK, remaining))
                if not data:
                    raise ValueError("assembly input truncated")
                digest.update(data)
                output.write(data)
                remaining -= len(data)


def compress_size(path):
    encoder, size = zlib.compressobj(6, zlib.DEFLATED, -15), 0
    with path.open("rb") as source:
        for data in iter(lambda: source.read(CHUNK), b""):
            size += len(encoder.compress(data))
    return size + len(encoder.flush())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(".native/evaluation"))
    parser.add_argument("--tiers", nargs="+", choices=("smoke", "city", "large", "xlarge"), default=["smoke"])
    parser.add_argument("--city-mib", type=int, default=192)
    parser.add_argument("--large-mib", type=int, default=1024)
    parser.add_argument("--xlarge-mib", type=int, default=4096)
    add_hardware_arguments(parser)
    args = parser.parse_args()
    if not 128 <= args.city_mib <= 256 or not 1024 <= args.large_mib <= 1100:
        parser.error("city payload must be 128–256 MiB, large 1024–1100 MiB")
    if not 4096 <= args.xlarge_mib <= 6144:
        parser.error("xlarge payload must be 4096–6144 MiB")
    admission = hardware_preflight(args)
    print(json.dumps({"androidPreparation": admission}, ensure_ascii=False), flush=True)
    if args.probe_only or args.inventory_only:
        return
    if importlib.metadata.version("libzim") != "3.1.0":
        parser.error("reproducible fixture generation requires python-libzim 3.1.0")
    if get_versions().get("libzim") != "8.2.0":
        parser.error("reproducible writer requires the installed native libzim 8.2.0")
    if not shutil.which("zimcheck"):
        parser.error("zimcheck executable is required")
    root = args.output.resolve()
    if root.exists():
        parser.error("output already exists; choose a fresh --output to preserve evidence")
    root.mkdir(parents=True)
    assets = root / "android-assets" / "evaluation"
    candidates = assets / "candidates"
    candidates.mkdir(parents=True)
    work = root / "host-validation"
    work.mkdir()
    started = time.monotonic()
    cases, records = [], []
    for tier in dict.fromkeys(args.tiers):
        payload_size = {"smoke": 2 * MIB, "city": args.city_mib * MIB, "large": args.large_mib * MIB, "xlarge": args.xlarge_mib * MIB}[tier]
        if shutil.disk_usage(root).free < payload_size * 3 + 512 * MIB:
            raise RuntimeError("insufficient host disk for bounded tier " + tier)
        payload = work / (tier + "-payload.bin")
        write_payload(payload, payload_size)
        base = candidates / (tier + "-eng-base.zim")
        create_archive(base, tier + "-eng-base", ["eng"], payload)
        variants = [(tier + "-multilingual", ["eng", "ara", "hin"])]
        if tier == "smoke":
            variants = [("smoke-eng", ["eng"]), ("smoke-ara", ["ara"]), ("smoke-hin", ["hin"])] + variants
        for identifier, languages in variants:
            case_started = time.monotonic()
            target = work / (identifier + "-target.zim")
            create_archive(target, identifier, languages, payload)
            pack = candidates / (identifier + "-languages.pack")
            recipe, expected = build_recipe(base, target, pack, identifier, payload_size)
            # The recipe now owns all required target bytes and expected hashes.
            # Release this full-size copy before creating the assembled output.
            target.unlink()
            if tier == "xlarge" and expected["size"] <= 4096 * MIB:
                raise ValueError("xlarge archive must be strictly larger than 4 GiB")
            recipe_path = assets / "recipes" / (identifier + ".json")
            write_json(recipe_path, recipe)
            assembled = work / (identifier + "-assembled.zim")
            assemble(recipe, base, pack, assembled)
            if digests(assembled) != expected:
                raise ValueError("assembled bytes differ from native writer target")
            articles, searches, validation = check_archive(assembled, languages)
            expected.update({"uuid": recipe["uuid"], "languages": languages, "articles": articles, "searches": searches})
            # file_md5 is recipe/provenance only; harness schema is strict.
            expected.pop("file_md5")
            cases.append({"id": identifier, "tier": tier, "recipe": "recipes/" + recipe_path.name, "localCandidates": ["candidates/" + base.name, "candidates/" + pack.name], "expected": expected})
            record = {"id": identifier, "synthetic": True, "payloadBytes": payload_size, "base": {"path": str(base.relative_to(root)), **digests(base)}, "pack": {"path": str(pack.relative_to(root)), **digests(pack)}, "recipe": {"path": str(recipe_path.relative_to(root)), **digests(recipe_path)}, "output": {**digests(assembled), "uuid": recipe["uuid"], "zimChecksumMd5": recipe["md5"]}, "chunkBytes": CHUNK, "baseChunks": len(recipe["base"]["sha256"]), "packChunks": len(recipe["packs"]["languages"]["sha256"]), "reusedBaseBytes": payload_size, "packSegmentBytes": pack.stat().st_size, "inlineBytes": 80, "finalChecksumBytes": 16, "deflateTransportBaseBytes": compress_size(base), "hostValidation": validation, "hostCaseElapsedSeconds": round(time.monotonic() - case_started, 6)}
            if len(set(recipe["base"]["sha256"])) != len(recipe["base"]["sha256"]):
                raise ValueError("base chunk hash collision would shortcut the intended read/hash workload")
            record["origin"] = "synthetic"
            record["uniqueBaseChunkHashes"] = len(set(recipe["base"]["sha256"]))
            record["payloadChunkVariation"] = "Every 1MiB payload interval replaces 32 bytes after the marker with SHA256(kiwix-unique-chunk-v1: || LE64 interval index)"
            records.append(record)
            print(json.dumps({"id": identifier, "size": expected["size"], "sha256": expected["sha256"], "seconds": record["hostCaseElapsedSeconds"]}), flush=True)
            assembled.unlink()
        payload.unlink()
    manifest = assets / "manifest.json"
    write_json(manifest, {"schema": 1, "cases": cases})
    provenance = {"schema": 1, "synthetic": True, "license": "CC0-1.0", "generator": str(Path(__file__).name), "generatorSha256": digests(Path(__file__))["sha256"], "pythonLibzim": importlib.metadata.version("libzim"), "nativeLibzim": subprocess.run(["zimcheck", "--version"], capture_output=True, text=True, check=True).stdout, "payloadAlgorithm": "16KiB prefix of marker + SHA256(kiwix-public-evaluation-v1: || little-endian uint32 counter 0..511), repeated; Compression.none; bounded 1MiB IO", "compressionCaveat": "Repetitive nonzero payload deliberately compresses transport. It does not model real tile entropy, article counts, index size or arbitrary archive merging.", "manifest": {"path": str(manifest.relative_to(root)), **digests(manifest)}, "hostElapsedSeconds": round(time.monotonic() - started, 6), "hostMaxRssBytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss, "cases": records}
    provenance["payloadAlgorithm"] += "; each 1MiB interval has its own SHA256-derived 32-byte variation after marker"
    provenance["writerNativeVersions"] = get_versions()
    provenance["zimcheckVersions"] = provenance.pop("nativeLibzim")
    provenance["androidPreparation"] = admission
    write_json(root / "provenance.json", provenance)
    print("manifest=" + str(manifest), flush=True)
    print("provenance=" + str(root / "provenance.json"), flush=True)


if __name__ == "__main__":
    main()
