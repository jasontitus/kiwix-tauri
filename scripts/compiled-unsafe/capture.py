#!/usr/bin/env python3
"""Build the locked desktop binary in an empty target and retain raw evidence."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys


def output(*args):
    return subprocess.check_output(args, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt", type=Path, help="new directory for local evidence")
    args = parser.parse_args()
    directory = args.receipt.resolve()
    root = Path(__file__).resolve().parents[2]
    os.chdir(root)
    rustc = output("rustc", "-vV")
    target = next(line.split()[1] for line in rustc.splitlines() if line.startswith("host:"))
    if target != "aarch64-apple-darwin":
        parser.error("this capture describes the Apple silicon desktop build")
    dirty = output("git", "status", "--porcelain", "--untracked-files=all", "--", "src-tauri")
    if dirty:
        parser.error("commit application, vendor and lockfile changes before measuring")
    speech = os.environ.get("SHERPA_ONNX_LIB_DIR")
    if not speech or not Path(speech).is_dir():
        parser.error("set SHERPA_ONNX_LIB_DIR to the prepared static speech library directory")
    command = ["cargo", "build", "--release", "--locked", "--offline", "--features",
               "custom-protocol", "--manifest-path", "src-tauri/Cargo.toml", "--bin",
               "kiwix-workbench", "--target", target, "--target-dir", str(directory / "target"),
               "--message-format=json"]
    env = os.environ.copy()
    env.pop("RUSTC_WORKSPACE_WRAPPER", None)
    for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "CARGO_BUILD_RUSTC",
                "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"]:
        if env.get(key):
            parser.error(f"clear {key} for this capture")
    directory.mkdir()
    units = directory / "units"
    units.mkdir()
    env.update(RUSTC_WRAPPER=str(Path(__file__).with_name("record-rustc.py").resolve()),
               CARGO_INCREMENTAL="0", KIWIX_AUDIT_UNITS=str(units))
    metadata = {"revision": output("git", "rev-parse", "HEAD"),
                "lock_sha256": hashlib.sha256(Path("src-tauri/Cargo.lock").read_bytes()).hexdigest(),
                "rustc": rustc, "cargo": output("cargo", "-V"), "target": target,
                "os": platform.mac_ver()[0], "command": command, "root": str(root)}
    (directory / "capture.json").write_text(json.dumps(metadata, indent=2) + "\n")
    with (directory / "cargo.jsonl").open("w") as messages, (directory / "build.log").open("w") as log:
        status = subprocess.run(command, env=env, stdout=messages, stderr=log).returncode
    (directory / "exit-code").write_text(f"{status}\n")
    return status


if __name__ == "__main__":
    sys.exit(main())
