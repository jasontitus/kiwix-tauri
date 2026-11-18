"""Combine matched reader runs into a portable JSON result with raw samples and medians."""

import argparse
import hashlib
import json
import statistics
from pathlib import Path

from measure import write_new_json

METRICS = (
    "api_ns",
    "open_ns",
    "api_open_plus_work_ns",
    "process_ns",
    "cpu_ns",
    "peak_rss_bytes",
    "cache_budget_bytes",
    "cache_retained_bytes",
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "default", type=Path, help="Default cache sizes, native preload disabled"
    )
    parser.add_argument("matched", type=Path, help="Both cache targets set to 4 MiB")
    parser.add_argument(
        "eager", type=Path, help="Native preload enabled, open-only control"
    )
    parser.add_argument("--native-library", required=True, type=Path)
    parser.add_argument("--sdk-manifest", type=Path, required=True)
    parser.add_argument("--rust-revision", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    names = ("default", "matched", "eager")
    runs = {name: json.loads(getattr(args, name).read_text()) for name in names}
    default = runs["default"]
    expected_policy = {
        "default": ("default", False),
        "matched": ("4194304", False),
        "eager": ("default", True),
    }
    sdk = json.loads(args.sdk_manifest.read_text())
    root = args.sdk_manifest.resolve().parent
    library = args.native_library.resolve()
    native_hash = hashlib.sha256(library.read_bytes()).hexdigest()
    if sdk["files"].get(library.relative_to(root).as_posix()) != native_hash:
        raise ValueError("Native library differs from its SDK manifest")
    for patch in sdk["sourcePatches"]:
        path = (root / patch["path"]).resolve()
        path.relative_to(root)
        if hashlib.sha256(path.read_bytes()).hexdigest() != patch["sha256"]:
            raise ValueError(
                f"Patched SDK input differs from its manifest: {patch['marker']}"
            )
    result = {
        "host": default["host"],
        "rounds_per_reader_phase": default["rounds"],
        "libzim_revision": sdk["libzim"]["commit"],
        "native_sdk": {
            k: sdk[k]
            for k in ("target", "libzim", "xapian", "sources", "sourcePatches")
        },
        "zimru_revision": args.rust_revision,
        "native_library_sha256": native_hash,
        "driver_sha256": default["driver_sha256"],
        "sample_columns": METRICS,
        "policies": {},
        "archives": {},
    }
    for name, run in runs.items():
        if any(run[key] != default[key] for key in ("host", "driver_sha256", "rounds")):
            raise ValueError(f"Build, host or trial count changed for {name}")
        if (run["cache_bytes"], run["native_xapian_preload"]) != expected_policy[name]:
            raise ValueError(f"Unexpected settings for {name}")
        result["policies"][name] = {
            k: run[k] for k in ("cache_bytes", "native_xapian_preload")
        }
        for archive in run["archives"]:
            identity = {k: archive[k] for k in ("file", "size", "sha256", "selected")}
            out = result["archives"].setdefault(
                archive["file"], {**identity, "phases": {}}
            )
            if any(out[key] != value for key, value in identity.items()):
                raise ValueError("Archive or sample manifest changed between policies")
            required = (
                {"open"} if name == "eager" else {"first_article", "random_articles"}
            )
            if name == "default":
                required.update(["open", "title_prefixes", "repeat_article_32"])
            if name != "eager" and any(
                x["kind"] == "media" for x in archive["selected"]
            ):
                required.add("media_ranges")
            if set(archive["phases"]) != required:
                raise ValueError(
                    f"Unexpected phase coverage for {name}, {archive['file']}"
                )
            for phase, data in archive["phases"].items():
                if set(data["samples"]) != {"libzim", "zimru"}:
                    raise ValueError(f"Missing reader comparison for {phase}")
                shared = {
                    k: data[k]
                    for k in ("requested", "response_sha256", "response_bytes")
                }
                group = out["phases"].setdefault(phase, {**shared, "policies": {}})
                if any(group[key] != value for key, value in shared.items()):
                    raise ValueError(f"Response or requests changed for {phase}")
                summary = {}
                for reader, samples in data["samples"].items():
                    if len(samples) != default["rounds"]:
                        raise ValueError(f"Missing samples for {phase}")
                    summary[reader] = {
                        "medians": {
                            k: statistics.median(s[k] for s in samples) for k in METRICS
                        },
                        "samples": [[s[k] for k in METRICS] for s in samples],
                    }
                group["policies"][name] = summary
    if len({tuple(a["file"] for a in x["archives"]) for x in runs.values()}) != 1:
        raise ValueError("Archive lists differ")
    write_new_json(args.output, result)


if __name__ == "__main__":
    main()
