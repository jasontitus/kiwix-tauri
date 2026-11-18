#!/usr/bin/env python3
"""Reconcile a fresh Cargo release with per-unit compiler receipts."""

import argparse
from collections import Counter, defaultdict
from functools import lru_cache
import hashlib
import json
from pathlib import Path
from urllib.parse import parse_qs, urlsplit


def require(condition, message):
    if not condition:
        raise ValueError(message)


def records(path):
    with path.open() as stream:
        for line in stream:
            yield json.loads(line)


def sha256(path):
    with path.open("rb") as reader:
        digest = hashlib.sha256()
        for chunk in iter(lambda: reader.read(1024 * 1024), b""):
            digest.update(chunk)
        return digest.hexdigest()


def primary_spans(message):
    spans = tuple((s["file_name"], s["line_start"], s["column_start"], s["line_end"],
                   s["column_end"]) for s in message["spans"] if s["is_primary"])
    require(spans, "unsafe-code diagnostic has no primary source location")
    return message["message"], spans


def origin(package, manifest, root):
    if package.startswith("registry+https://github.com/rust-lang/crates.io-index#"):
        return "crates.io"
    if package.startswith("git+"):
        url = urlsplit(package[4:])
        revision, = parse_qs(url.query)["rev"]
        require(len(revision) == 40, "expected a pinned Git dependency")
        return f"{url.hostname}{url.path}@{revision}"
    require(package.startswith("path+file:"), "unknown Cargo package source")
    return "workspace/" + str(Path(manifest).parent.relative_to(root))


@lru_cache(maxsize=None)
def source_file(name, manifest, generated):
    path = Path(name)
    if not path.is_absolute():
        require(".." not in path.parts, "unexpected relative source")
        return str(path)
    if generated and path.is_relative_to(generated):
        return "<generated>/" + str(path.relative_to(generated)) + "@" + sha256(path)
    return str(path.relative_to(manifest))


def inventory(directory):
    metadata = json.loads((directory / "capture.json").read_text())
    root = Path(metadata["root"])
    target = metadata["target"]
    product = directory / "target" / target / "release" / "kiwix-workbench"
    require((directory / "exit-code").read_text().strip() == "0", "Cargo failed")
    artifacts = {}
    cargo_warnings = Counter()
    linked = defaultdict(set)
    finished = []
    for event in records(directory / "cargo.jsonl"):
        reason = event["reason"]
        if reason == "compiler-artifact":
            require(event["fresh"] is False, "cached artifact in fresh audit")
            files = [Path(name) for name in event["filenames"]]
            require(files, "artifact has no reported outputs")
            target_root = directory / "target" / target
            roles = {path.is_relative_to(target_root) for path in files}
            require(len(roles) == 1, "artifact mixes host and target outputs")
            key = (event["manifest_path"], event["target"]["name"].replace("-", "_"),
                   roles.pop(), tuple(sorted(event["features"])))
            require(key not in artifacts, "ambiguous compiler-artifact identity")
            artifacts[key] = event
        elif reason == "compiler-message":
            message = event["message"]
            if (message.get("code") or {}).get("code") == "unsafe_code":
                cargo_warnings[event["package_id"], primary_spans(message)] += 1
        elif reason == "build-script-executed":
            linked[event["package_id"]].update(event["linked_libs"])
        elif reason == "build-finished":
            finished.append(event["success"])
    require(finished == [True], "missing unique successful completion")
    require(any(a["executable"] == str(product) for a in artifacts.values()),
            "missing requested desktop product")
    artifact_count = len(artifacts)
    wrapper_warnings = Counter()
    units = Counter()
    diagnostics = Counter()
    sites = defaultdict(set)
    source_by_package = {}
    role_kinds = defaultdict(Counter)
    probes = Counter()
    for log in sorted((directory / "units").glob("unit-*.jsonl")):
        rows = records(log)
        unit = next(rows)["unit"]
        manifest = Path(unit["CARGO_MANIFEST_DIR"])
        feature_flags = tuple(sorted(s[9:-1] for s in unit["cfg"] if s.startswith('feature="')))
        key = (str(manifest / "Cargo.toml"), unit["crate"], bool(unit["target"]), feature_flags)
        if not unit["CARGO_CRATE_NAME"]:
            require(key not in artifacts and unit["crate"].startswith("autocfg_"),
                    "unknown compiler invocation outside the Cargo artifact graph")
            tail = list(rows)
            require(len(tail) == 1 and "status" in tail[0], "incomplete or lint-producing probe")
            probes["succeeded" if tail[0]["status"] == 0 else "failed"] += 1
            continue
        require(unit["target"] in (None, target), "unexpected compiler target")
        require(key in artifacts, f"unmatched compiler unit {unit['crate']}")
        artifact = artifacts.pop(key)
        package = artifact["package_id"]
        package_key = (unit["CARGO_PKG_NAME"], unit["CARGO_PKG_VERSION"],
                       origin(package, artifact["manifest_path"], root))
        source_by_package[package] = package_key
        role = "target" if unit["target"] else "host"
        units[package_key, role] += 1
        kind = ",".join(artifact["target"]["kind"])
        role_kinds[role][kind] += 1
        status = None
        for row in rows:
            require(status is None, "trailing data after compiler exit")
            if "status" in row:
                status = row["status"]
                continue
            warning = primary_spans(row["message"])
            wrapper_warnings[package, warning] += 1
            description, spans = warning
            normalized = tuple((source_file(file, manifest, unit["OUT_DIR"]), *pos)
                               for file, *pos in spans)
            sites[package_key, role].add((description, normalized))
            diagnostics[package_key, role] += 1
        require(status == 0, "missing successful compiler exit")
    require(not artifacts, "missing wrapper receipt for a Cargo artifact")
    require(cargo_warnings == wrapper_warnings, "Cargo and rustc warning multisets differ")

    packages = sorted(set(source_by_package.values()))
    require(len(packages) == len(source_by_package), "ambiguous normalized package source")
    package_rows = []
    first_party = defaultdict(list)
    distinct = 0
    for pkg in packages:
        row = list(pkg)
        for role in ("target", "host"):
            row.extend([units[pkg, role], diagnostics[pkg, role], len(sites[pkg, role])])
        combined = sites[pkg, "target"] | sites[pkg, "host"]
        row.append(len(combined))
        distinct += len(combined)
        package_rows.append(row)
        if pkg[2] == "workspace/src-tauri":
            for description, spans in sorted(combined):
                require(len(spans) == 1, "first-party diagnostic has multiple spans")
                (file, *pos), = spans
                require((root / "src-tauri" / file).is_file(), f"unknown app source path {file}")
                roles = [r for r in ("target", "host") if (description, spans) in sites[pkg, r]]
                first_party[file].append([*pos, description, "+".join(roles)])

    summary = {}
    for role in ("target", "host"):
        active = [p for p in packages if units[p, role]]
        summary[role] = {"units": sum(units[p, role] for p in active),
                         "units_by_cargo_kind": dict(sorted(role_kinds[role].items())),
                         "packages": len(active),
                         "packages_with_warnings": sum(bool(sites[p, role]) for p in active),
                         "diagnostics": sum(diagnostics[p, role] for p in active),
                         "distinct_sites": sum(len(sites[p, role]) for p in active)}
    return {
        "app_revision": metadata["revision"], "cargo_lock_sha256": metadata["lock_sha256"],
        "os": "macOS " + metadata["os"], "target": target, "rustc": metadata["rustc"],
        "cargo": metadata["cargo"], "profile": "release", "features": ["custom-protocol"],
        "product_sha256": sha256(product), "fresh_artifacts": artifact_count,
        "excluded_build_script_autocfg_probes": dict(sorted(probes.items())),
        "packages": len(packages), "packages_with_warnings": sum(bool(r[-1]) for r in package_rows),
        "raw_diagnostics": sum(wrapper_warnings.values()), "distinct_sites": distinct,
        "by_role": summary,
        "build_script_link_requests": {label: sorted(libs) for pkg, libs in sorted(linked.items())
                                       if libs for label in [" ".join(source_by_package[pkg][:2])]},
        "first_party_site_columns": ["line_start", "column_start", "line_end", "column_end",
                                     "compiler_message", "compiled_role"],
        "first_party_sites_relative_to_src_tauri": {f: sorted(v) for f, v in sorted(first_party.items())},
        "package_columns": ["name", "version", "source", "target_units", "target_diagnostics",
                            "target_sites", "host_units", "host_diagnostics", "host_sites", "union_sites"],
        "package_rows": package_rows,
    }


def format_inventory(data):
    # Keep every package on one line for reviewable version-to-version diffs.
    rows = data["package_rows"]
    fields = {k: v for k, v in data.items() if k != "package_rows"}
    result = json.dumps(fields, indent=2).removesuffix("\n}")
    return result + ',\n  "package_rows": [\n    ' + ',\n    '.join(map(json.dumps, rows)) + '\n  ]\n}\n'


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.write_text(format_inventory(inventory(args.receipt.resolve())))
