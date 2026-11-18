#!/usr/bin/env python3
"""Record rustc's unsafe-code diagnostics for both Cargo host and target units."""

import json
import os
import subprocess
import sys
import tempfile


def main():
    compiler, *args = sys.argv[1:]
    if "--crate-name" not in args or "--out-dir" not in args:
        os.execvp(compiler, [compiler, *args])

    def values(option):
        return [value for flag, value in zip(args, args[1:]) if flag == option]

    def value(option):
        return next(iter(values(option)), None)

    keys = ["CARGO_PKG_NAME", "CARGO_PKG_VERSION", "CARGO_MANIFEST_DIR",
            "CARGO_CRATE_NAME", "OUT_DIR"]
    unit = {key: os.environ.get(key) for key in keys}
    unit.update(crate=value("--crate-name"), target=value("--target"),
                cfg=values("--cfg"), out=value("--out-dir"))
    # Cargo's JSON stream alone cannot distinguish identical sites emitted
    # for host and target builds of the same library.
    with tempfile.NamedTemporaryFile(mode="w", dir=os.environ["KIWIX_AUDIT_UNITS"],
                                     prefix="unit-", suffix=".jsonl", delete=False) as record:
        print(json.dumps({"unit": unit}), file=record)
        command = [compiler, *args, "--force-warn", "unsafe-code"]
        with subprocess.Popen(command, stderr=subprocess.PIPE) as process:
            for line in process.stderr:
                sys.stderr.buffer.write(line)
                if b"unsafe_code" in line:
                    try:
                        message = json.loads(line)
                    except ValueError:
                        continue  # rustc may also print plain text
                    if (message.get("code") or {}).get("code") == "unsafe_code":
                        print(json.dumps({"message": message}), file=record)
            status = process.wait()
        print(json.dumps({"status": status}), file=record)
    return status


if __name__ == "__main__":
    sys.exit(main())
