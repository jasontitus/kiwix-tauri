#!/usr/bin/env python3
"""Measure the same app spelling queries against freshly built, disposable indexes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
BASE = "96e7afce109a1b753213caf601a4b8586d948313"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def prepare(directory):
    source = ROOT / "src-tauri/src"
    target = directory / "src"
    target.mkdir(parents=True)
    shutil.copy(source / "casefold.rs", target)
    shutil.copytree(source / "casefold", target / "casefold")
    shutil.copy(source / "spelling/index.rs", target)
    live = (source / "spelling.rs").read_text()
    reference = subprocess.check_output(["git", "show", f"{BASE}:src-tauri/src/spelling.rs"], cwd=ROOT, text=True)

    def section(text, start, end):
        return text[text.index(start):text.index(end, text.index(start))]

    # Compile the unchanged functions owning the query, normalization, ranking
    # and both database implementations. Stubs only describe the public rows.
    combined = """
mod casefold;
mod index;
use index::Lexicon;
use std::sync::atomic::Ordering;
use unicode_normalization::UnicodeNormalization;
struct SpellingCorrection { original_query:String, corrected_query:String }
"""
    combined += section(live, "const MAX_TITLES", "// An open dictionary")
    combined += section(live, "fn err(", "fn open_directory(")
    combined += section(live, "fn distance_one(", "#[cfg(test)]\nfn query_corrections")
    combined += section(live, "fn corrections_while(", "#[cfg(test)]\nmod tests")
    combined += """
mod legacy {
use super::*;
use std::collections::BTreeSet;
use rusqlite::{params,params_from_iter,types::Value as SqlValue,Connection};
const MAX_DELETION_KEYS:usize=MAX_WORD_CHARS+1;
"""
    combined += section(reference, "fn deletion_keys(", "struct Temporary")
    combined += section(reference, "fn candidates(", "fn match_case(")
    combined += section(reference, "fn query_corrections(", "#[cfg(test)]\nmod tests")
    combined += """
pub(super) use self::{create_schema as schema, add_titles as add, query_corrections as query};
}
"""
    for symbol in ["create_schema", "add_titles", "query_corrections"]:
        combined = combined.replace(f"fn {symbol}(", f"pub(super) fn {symbol}(")
    (target / "main.rs").write_text(combined + (HERE / "driver.rs").read_text())
    manifest = (ROOT / "src-tauri/Cargo.toml").read_text().splitlines()
    items = ["chksum-hash-sha2-256", "serde_json", "unicode-normalization", "zimru"]
    deps = "\n".join(next(line for line in manifest if line.startswith(f"{name} =")) for name in items)
    (directory / "Cargo.toml").write_text(f"""[package]
name="kiwix-spelling-compare"
version="0.1.0"
edition="2021"
[dependencies]
rusqlite={{version="=0.41.1",features=["bundled","fallible_uint"]}}
tempfile="=3.27.0"
{deps}
""")
    subprocess.run(["cargo", "build", "--release", "--offline"], cwd=directory, check=True)
    return directory / "target/release/kiwix-spelling-compare"


def measure(binary, mode, cache, titles):
    # The collector owns every child and index file. wait4 attributes RSS/CPU
    # to the measured process; SQLite journal samples are a lower bound.
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        begin = time.monotonic()
        process = subprocess.Popen([binary, mode, cache, titles], stdout=output, stderr=errors)
        observed = 0
        try:
            while True:
                total = 0
                for p in cache.iterdir():
                    try:
                        total += p.stat().st_size
                    except FileNotFoundError:
                        pass
                observed = max(observed, total)
                pid, code, usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    process.returncode = os.waitstatus_to_exitcode(code)
                    break
                if time.monotonic() - begin > 900:
                    raise TimeoutError(mode)
                time.sleep(0.002)
        finally:
            if process.returncode is None:
                process.kill()
                process.wait()
        elapsed = time.monotonic() - begin
        if process.returncode:
            errors.seek(0)
            raise RuntimeError(f"{mode}: {errors.read(4096)!r}")
        output.seek(0)
        result = json.loads(output.read(1024 * 1024))
    result.update(wall=elapsed, cpu=usage.ru_utime + usage.ru_stime,
                  rssBytes=usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
                  observedWorkingBytes=observed, fileBytes=(cache / "index").stat().st_size)
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--rounds", type=int, default=4)
    ap.add_argument("--synthetic", action="store_true", help="also compare 25,000 words with common prefixes and suffixes")
    ap.add_argument("archives", type=Path, nargs="+")
    args = ap.parse_args()
    if args.rounds < 2 or args.rounds % 2:
        ap.error("--rounds must be positive and even")
    if args.output.exists():
        ap.error("--output must be a new file")
    result = {"base": BASE, "host": platform.platform(), "machine": platform.machine(),
              "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(), "archives": []}
    with tempfile.TemporaryDirectory(prefix="kiwix-spell-bench-") as work:
        root = Path(work)
        binary = prepare(root / "build")
        result["binarySha256"]=digest(binary)
        result["sourceSha256"]={name:digest(ROOT / name) for name in [
            "src-tauri/src/spelling.rs","src-tauri/src/spelling/index.rs",
            "scripts/bench-spelling-index/driver.rs","scripts/bench-spelling-index/compare.py"]}
        for number, archive in enumerate(args.archives + ([None] if args.synthetic else [])):
            titles = root / f"titles-{number}"
            if archive:
                label=archive.name
                subprocess.run([binary, "extract", titles, archive.resolve()], check=True, timeout=900)
                entry={"archive":label,"bytes":archive.stat().st_size,"sha256":digest(archive)}
            else:
                label="synthetic-common-affixes"
                with titles.open("xb") as stream:
                    for i in range(25000):
                        suffix="".join(chr(97+(i//(26**n))%26) for n in range(3,-1,-1))
                        title=("commonprefix"+suffix+"ending").encode()
                        stream.write(len(title).to_bytes(4,'little')+title)
                entry={"synthetic":label}
            entry.update(titleStreamSha256=digest(titles),samples=[],medians={})
            result["archives"].append(entry)
            expected = None
            for n in range(args.rounds):
                for backend in (["sqlite", "checked"] if n % 2 == 0 else ["checked", "sqlite"]):
                    print(f"{label}: round {n+1}, {backend}", file=sys.stderr, flush=True)
                    with tempfile.TemporaryDirectory(dir=root) as owned:
                        cache = Path(owned)
                        build = measure(binary, backend + "-build", cache, titles)
                        query = measure(binary, backend + "-query", cache, titles)
                        answer = [build["titles"], build["words"], query.pop("results")]
                        if expected is None:
                            expected = answer
                            entry["expected"] = answer
                        if answer != expected:
                            raise ValueError("Ordered query, title or word count mismatch")
                        entry["samples"].append({"backend": backend, "round": n+1, "build": build, "query": query})
            for backend in ["sqlite", "checked"]:
                rows = [row for row in entry["samples"] if row["backend"] == backend]
                entry["medians"][backend] = {phase: {k: statistics.median(row[phase][k] for row in rows)
                    for k, val in rows[0][phase].items() if isinstance(val, (float,int))}
                    for phase in ["build", "query"]}
    # Never replace an archive or old evidence, including links to those paths.
    with args.output.open("x", encoding="utf8") as out:
        json.dump(result, out, indent=2, ensure_ascii=False)
        out.write("\n")


if __name__ == "__main__":
    main()
