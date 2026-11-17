# SPDX-License-Identifier: GPL-3.0-or-later
"""ABI provenance and live, fixture-free Android instrumentation admission."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import zipfile

ABIS = ("armeabi-v7a", "arm64-v8a", "x86", "x86_64")
RUNNER = "org.kiwix.workbench.test/androidx.test.runner.AndroidJUnitRunner"
INVENTORY_RUNNER = "org.kiwix.workbench.inventory.test/androidx.test.runner.AndroidJUnitRunner"
TEST = "org.kiwix.workbench.AndroidEvaluationTest#productionComposerAndChecksumsOnDevice"


def add_hardware_arguments(parser):
    parser.add_argument("--target-abi", choices=ABIS, help="Record intended APK ABI; fixture bytes remain ABI-independent")
    parser.add_argument("--target-apk", type=Path, help="Inspect the APK and bind the receipt to its hash; required for native admission")
    parser.add_argument("--device", help="Explicit adb serial: probe the installed APK before generating/downloading fixtures")
    parser.add_argument("--min-ram-mib", type=int, help="Reject actual Android totalMem below this bound")
    parser.add_argument("--max-ram-mib", type=int, help="Reject actual Android totalMem above this bound (use 2048 for a <=2 GiB request)")
    parser.add_argument("--require-low-ram-device", action="store_true", help="Require actual ActivityManager.isLowRamDevice")
    parser.add_argument("--probe-only", action="store_true", help="Only inspect APK/probe hardware; do not generate or download fixtures")
    parser.add_argument("--inventory-only", action="store_true", help="Java/platform inventory only: never load native code or prepare fixtures; requires --device")


def apk_identity(path, abi, inventory=False):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(128 * 1024), b""):
            digest.update(block)
    native = {}
    expected_elf = {"armeabi-v7a": (1, 40), "arm64-v8a": (2, 183), "x86": (1, 3), "x86_64": (2, 62)}
    with zipfile.ZipFile(path) as apk:
        if inventory and any(name.endswith(".so") for name in apk.namelist()):
            raise ValueError("Inventory APK must not package native libraries")
        for name in apk.namelist():
            parts = name.split("/")
            if len(parts) != 3 or parts[0] != "lib" or parts[2] != "libkiwix_app.so":
                continue
            with apk.open(name) as library:
                header = library.read(64)
            arch = parts[1]
            if (arch not in expected_elf or len(header) < 20 or header[:4] != b"\x7fELF"
                    or header[5] != 1 or (header[4], int.from_bytes(header[18:20], "little")) != expected_elf[arch]
                    or arch in native):
                raise ValueError("APK native library has an invalid, duplicate or mislabeled ELF ABI: " + name)
            native[arch] = {"elfClass": header[4], "elfMachine": int.from_bytes(header[18:20], "little")}
    abis = sorted(native)
    if not inventory and (not abis or (abi is not None and abi not in abis)):
        raise ValueError("APK lacks the requested real native library ABI: " + str(abis))
    return {"path": str(path.resolve()), "sha256": digest.hexdigest(), "nativeAbis": abis, "nativeElf": native}


def hardware_preflight(args):
    inventory = args.inventory_only
    if inventory and not args.device:
        raise ValueError("--inventory-only requires an explicit --device")
    if any(value is not None and not 1 <= value <= 1048576 for value in (args.min_ram_mib, args.max_ram_mib)):
        raise ValueError("RAM bounds must be positive MiB")
    if args.min_ram_mib is not None and args.max_ram_mib is not None and args.min_ram_mib > args.max_ram_mib:
        raise ValueError("RAM bounds are inverted")
    if not args.device and (args.min_ram_mib is not None or args.max_ram_mib is not None or args.require_low_ram_device):
        raise ValueError("Hardware constraints require --device; a catalog name is not a measurement")
    apk = apk_identity(args.target_apk, args.target_abi, inventory=inventory) if args.target_apk else None
    result = {"requestedAbi": args.target_abi, "apk": apk, "hardwareMeasured": False,
              "physicalDeviceStatus": "unverified", "workloadExecuted": False,
              "fixtureBytesAbiIndependent": True,
              "hardwareGateSourceSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    if not args.device:
        result["status"] = "host-only: no device admission or physical execution claimed"
        return result
    if not inventory and (args.target_abi is None or apk is None):
        raise ValueError("Live native admission requires --target-abi and --target-apk matching the installed debug APK")
    options = {"class": TEST, "evaluationInventoryOnly" if inventory else "evaluationProbeOnly": "true",
               "evaluationRequireLowRamDevice": str(args.require_low_ram_device).lower()}
    if args.target_abi:
        options["evaluationRequiredAbi"] = args.target_abi
    if apk:
        options["evaluationTargetApkSha256"] = apk["sha256"]
    for key, value in (("evaluationMinRamMiB", args.min_ram_mib), ("evaluationMaxRamMiB", args.max_ram_mib)):
        if value is not None:
            options[key] = str(value)
    command = ["adb", "-s", args.device, "shell", "am", "instrument", "-w", "-r"]
    for key, value in options.items():
        command.extend(("-e", key, value))
    command.append(INVENTORY_RUNNER if inventory else RUNNER)
    completed = subprocess.run(command, capture_output=True, text=True, timeout=180, check=False)
    if completed.returncode:
        raise RuntimeError(f"Hardware instrumentation exited {completed.returncode}:\n{completed.stdout}{completed.stderr}")
    prefix = "ANDROID_EVALUATION hardware-inventory " if inventory else "ANDROID_EVALUATION hardware-admission "
    receipts = [json.loads(line.split(prefix, 1)[1]) for line in completed.stdout.splitlines() if prefix in line]
    if len(receipts) != 1:
        raise ValueError("Live instrumentation did not produce exactly one hardware admission receipt: " + completed.stdout + completed.stderr)
    receipt = receipts[0]
    if (receipt.get("status") != ("measured" if inventory else "admitted")
            or receipt.get("mode") != ("inventory-only" if inventory else "info-only")
            or (apk is not None and receipt.get("targetApkSha256") != apk["sha256"])
            or (args.target_abi is not None and receipt.get("requiredAbi") != args.target_abi)
            or receipt.get("nativeLibraryLoaded") is not (not inventory) or receipt.get("workloadExecuted") is not False):
        raise ValueError("Actual installed hardware/APK rejected before fixture generation: " + json.dumps(receipt))
    device = receipt["device"]
    if ((args.target_abi is not None and args.target_abi not in device["supportedAbis"])
            or (not inventory and device["process64Bit"] != (args.target_abi in ("arm64-v8a", "x86_64")))
            or type(device["totalPhysicalRamBytes"]) is not int or device["totalPhysicalRamBytes"] <= 0):
        raise ValueError("Inconsistent live Android hardware measurement")
    actual_ram = device["totalPhysicalRamBytes"]
    if ((args.min_ram_mib is not None and actual_ram < args.min_ram_mib * 1024 * 1024)
            or (args.max_ram_mib is not None and actual_ram > args.max_ram_mib * 1024 * 1024)
            or (args.require_low_ram_device and device["lowRamDevice"] is not True)):
        raise ValueError("Actual Android RAM measurement does not satisfy the requested hardware range")
    if "OK (1 test)" not in completed.stdout:
        raise ValueError("Hardware admission instrumentation did not finish successfully: " + completed.stdout + completed.stderr)
    result.update({"status": "inventory only: no native admission or fixture preparation" if inventory else "admitted for fixture preparation only",
                   "nativeAdmission": not inventory, "hardwareMeasured": True,
                   "adbSerial": args.device, "command": command, "receipt": receipt,
                   "instrumentationStdout": completed.stdout, "instrumentationStderr": completed.stderr})
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    add_hardware_arguments(parser)
    args = parser.parse_args()
    if not args.device or args.probe_only == args.inventory_only:
        parser.error("Specify --device and exactly one of --inventory-only or --probe-only")
    print(json.dumps({"androidPreparation": hardware_preflight(args)}, indent=2))
