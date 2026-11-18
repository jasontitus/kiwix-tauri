"""Replay the recorded public reader manifests on one explicitly named Android emulator."""

import argparse
import itertools
import json
import re
import subprocess
import tempfile
from pathlib import Path

from measure import file_hash, write_new_json

MODES = {
    "open": "open",
    "first_article": "read",
    "repeat_article_32": "repeat",
    "random_articles": "read",
    "title_prefixes": "title",
    "media_ranges": "read",
}


def sample(log, mode):
    def numbers(pattern):
        found = re.findall(pattern, log, re.MULTILINE)
        if len(found) != 1:
            raise ValueError(f"Missing or repeated reader measurement: {log}")
        return [int(x) for x in found[0].split()]

    (opened,) = numbers(r"^OPEN_NS (\d+)$")
    (work,) = numbers(r"^BENCH_NS (\d+)$")
    budget, retained = numbers(r"^CACHE (\d+ \d+)$")
    wall, cpu_us, peak_kib = numbers(r"^PROCESS (\d+ \d+ \d+)$")
    if not (opened and work and wall and peak_kib):
        raise ValueError(f"Invalid reader measurement: {log}")
    return {
        "open_ns": opened,
        "api_ns": work,
        "api_open_plus_work_ns": work + (0 if mode == "open" else opened),
        "process_ns": wall,
        "cpu_ns": cpu_us * 1000,
        "peak_rss_bytes": peak_kib * 1024,
        "cache_budget_bytes": budget,
        "cache_retained_bytes": retained,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--adb", default="adb")
    p.add_argument("--serial", required=True)
    p.add_argument("--avd", required=True)
    p.add_argument("--timer", type=Path, required=True)
    p.add_argument("--native", type=Path, required=True)
    p.add_argument("--rust", type=Path, required=True)
    p.add_argument("--candidate", type=Path)
    p.add_argument("--rounds", type=int, default=6)
    p.add_argument("--cache-bytes", type=int, default=4194304)
    p.add_argument("--manifest", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("archives", nargs="+", type=Path)
    args = p.parse_args()
    if not re.fullmatch(r"emulator-[0-9]+", args.serial):
        p.error("Specify a task-owned emulator serial.")
    if not args.cache_bytes > 0 or not 1 <= args.rounds <= 100:
        p.error("Use a positive cache target and at most 100 measured rounds.")
    programs = {"libzim": args.native, "zimru": args.rust}
    if args.candidate:
        programs["candidate"] = args.candidate
    orders = list(itertools.permutations(programs))
    if args.rounds % len(orders):
        p.error(f"Use a multiple of {len(orders)} rounds for balanced reader order.")

    devices = subprocess.check_output([args.adb, "devices", "-l"], timeout=15).decode()
    targets = re.findall(
        rf"^{re.escape(args.serial)}\s+device\s+[^\n]*\btransport_id:(\d+)\s*$",
        devices,
        re.MULTILINE,
    )
    if len(targets) != 1:
        raise RuntimeError("The selected emulator needs one active ADB transport.")
    transport = targets[0]

    def adb(*command):
        return (
            subprocess.check_output(
                [args.adb, "-t", transport, *map(str, command)], timeout=90
            )
            .decode()
            .strip()
        )

    def identity():
        if adb("emu", "avd", "name").splitlines()[0] != args.avd:
            raise RuntimeError("Emulator identity changed.")

    def shell(command):
        return adb("shell", "-T", command)

    identity()
    manifest = json.loads(args.manifest.read_text())["archives"]
    if isinstance(manifest, list):
        manifest = {entry["file"]: entry for entry in manifest}
    result = {
        "android_api": shell("getprop ro.build.version.sdk"),
        "android_fingerprint": shell("getprop ro.build.fingerprint"),
        "architecture": shell("getprop ro.product.cpu.abi"),
        "page_size": int(shell("getconf PAGE_SIZE")),
        "rounds": args.rounds,
        "permutations": orders,
        "cache_bytes": args.cache_bytes,
        "native_xapian_preload": False,
        "manifest_sha256": file_hash(args.manifest),
        "driver_sha256": {name: file_hash(path) for name, path in programs.items()},
        "timer_sha256": file_hash(args.timer),
        "archives": {},
    }
    run_dir = shell("mktemp -d /data/local/tmp/kiwix-readers-XXXXXXXX")
    if not re.fullmatch(r"/data/local/tmp/kiwix-readers-[a-zA-Z0-9]{8}", run_dir):
        raise RuntimeError("Unexpected emulator temporary directory.")

    def push(path, name, digest=None):
        adb("push", path, f"{run_dir}/{name}")
        if digest and shell(f"sha256sum {run_dir}/{name}").split()[0] != digest:
            raise RuntimeError(f"Emulator copy of {name} differs from its source.")

    try:
        for name, path in programs.items():
            push(path, name, result["driver_sha256"][name])
        push(args.timer, "timer", result["timer_sha256"])
        shell(f"cd {run_dir} && chmod 700 timer {' '.join(programs)}")
        with tempfile.TemporaryDirectory() as temp:
            query = Path(temp) / "request"
            for archive in args.archives:
                entry = manifest[archive.name]
                if (
                    archive.name in result["archives"]
                    or file_hash(archive) != entry["sha256"]
                ):
                    raise ValueError(f"Duplicate or mismatched archive: {archive.name}")
                push(archive, "archive.zim", entry["sha256"])
                phases = {}
                result["archives"][archive.name] = {
                    "sha256": entry["sha256"],
                    "phases": phases,
                }
                for phase, reference in entry["phases"].items():
                    mode = MODES[phase]
                    query.write_bytes(
                        "".join(
                            f"{offset}\t{size}\t{key}\n"
                            for offset, size, key in reference["requested"]
                        ).encode()
                    )
                    push(query, "query", file_hash(query))
                    rows = {name: [] for name in programs}
                    phases[phase] = {
                        "response_sha256": reference["response_sha256"],
                        "response_bytes": reference["response_bytes"],
                        "samples": rows,
                    }
                    for rnd in range(args.rounds + 1):
                        order = orders[(rnd - 1) % len(orders)]
                        for name in order:
                            try:
                                report = shell(
                                    f"cd {run_dir} && ./timer ./{name} {mode} archive.zim "
                                    f"{args.cache_bytes} lazy <query >answer 2>metrics && "
                                    "cat metrics && wc -c <answer && sha256sum answer"
                                )
                            except subprocess.CalledProcessError as error:
                                message = shell(f"cd {run_dir} && tail -c 4096 metrics")
                                raise RuntimeError(
                                    f"{name}/{phase}: {message}"
                                ) from error
                            lines = report.splitlines()
                            if (
                                int(lines[-2]) != reference["response_bytes"]
                                or lines[-1].split()[0] != reference["response_sha256"]
                            ):
                                raise ValueError(
                                    f"Response differs for {archive.name}/{phase}/{name}"
                                )
                            measurement = sample(report, mode)
                            if rnd:
                                rows[name].append(measurement)
        write_new_json(args.output, result)
    finally:
        identity()
        shell(f"rm -r {run_dir}")


if __name__ == "__main__":
    main()
