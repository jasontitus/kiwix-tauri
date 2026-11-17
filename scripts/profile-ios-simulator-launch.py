#!/usr/bin/env python3
"""Prepare or run the reviewed, simulator-only standard XCTest launch metric."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import signal
import subprocess
import time

DEVICE = "7C08AECE-54BD-45E4-B250-026BCEB7A12D"
NAME = "KiwixProfilingValidation-20261105-9b64"
TARGET = "KiwixSimulatorLaunchMetrics"
RUNNER = "com.tiltastech.kiwix.simulatorlaunchmetrics.xctrunner"
ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / ".native/iphone-tool/simulator/metrics"
SOURCES = ROOT / "scripts/iphone-launch-ui"
PREPARED = EVIDENCE / "launch-prepared.json"
INTERRUPTION_SEEN = False


def command(argv, timeout):
    process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, start_new_session=True)
    try:
        text, _ = process.communicate(timeout=timeout)
    except BaseException as error:
        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM, signal.SIGHUP})
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            text, _ = process.communicate(timeout=1)
        except subprocess.TimeoutExpired as drain_error:
            text = drain_error.output or getattr(error, "output", "") or ""
            if isinstance(text, bytes):
                text = text.decode(errors="replace")
            process.stdout.close()
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                pass
        error.command_output = text
        raise
    return {"argv": argv, "returncode": process.returncode, "output": text}


def interrupted(signum, frame):
    global INTERRUPTION_SEEN
    if INTERRUPTION_SEEN:
        return
    INTERRUPTION_SEEN = True
    raise InterruptedError("Launch metric run interrupted by signal " + str(signum))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes(directory):
    return {name: digest(directory / name)
            for name in ("SimulatorLaunchMetrics.swift", "project.yml")}


def prepare(prepared_path):
    output = EVIDENCE / ("launch-preparation-" + str(time.time_ns()))
    project = output / "project"
    project.mkdir(parents=True)
    for name in ("SimulatorLaunchMetrics.swift", "project.yml"):
        shutil.copy2(SOURCES / name, project / name)
    hashes = source_hashes(project)
    stages = [
        ("xcodegen", ["xcodegen", "generate", "--spec", str(project / "project.yml"),
                      "--project", str(project)], 30),
        ("build", ["xcodebuild", "build-for-testing", "-project",
                   str(project / (TARGET + ".xcodeproj")), "-scheme", TARGET,
                   "-configuration", "Debug", "-sdk", "iphonesimulator",
                   "-destination", "generic/platform=iOS Simulator",
                   "-derivedDataPath", str(output / "DerivedData"),
                   "-parallel-testing-enabled", "NO", "ARCHS=arm64",
                   "ONLY_ACTIVE_ARCH=NO", "CODE_SIGNING_ALLOWED=NO"], 180),
    ]
    for name, argv, timeout in stages:
        try:
            result = command(argv, timeout)
        except BaseException as error:
            (output / (name + ".log")).write_text(getattr(error, "command_output", ""))
            raise
        (output / (name + ".log")).write_text(result["output"])
        if result["returncode"]:
            raise RuntimeError(name + " failed; see " + str(output / (name + ".log")))
    products = output / "DerivedData/Build/Products"
    plans = list(products.glob(TARGET + "_*.xctestrun"))
    if len(plans) != 1:
        raise RuntimeError("Expected exactly one prepared simulator test plan")
    binary = products / ("Debug-iphonesimulator/" + TARGET + "-Runner.app/PlugIns/" + TARGET + ".xctest/" + TARGET)
    if source_hashes(SOURCES) != hashes or source_hashes(project) != hashes:
        raise RuntimeError("Launch metric source changed during preparation")
    manifest = {"schema": 1, "platform": "iOS Simulator", "target": TARGET,
                "runner_bundle_id": RUNNER, "source_sha256": hashes,
                "plan": str(plans[0]), "plan_sha256": digest(plans[0]),
                "test_binary": str(binary), "test_binary_sha256": digest(binary),
                "build_output": str(output)}
    prepared_path.parent.mkdir(parents=True, exist_ok=True)
    prepared_path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"status": "prepared", "manifest": str(prepared_path), **manifest}))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare-only", action="store_true",
                        help="Generate and build only the dedicated unsigned generic simulator runner; no device access")
    parser.add_argument("--prepared", type=Path, default=PREPARED,
                        help="Dedicated launch preparation manifest")
    parser.add_argument("--reviewed-plan-sha256")
    parser.add_argument("--reviewed-test-binary-sha256")
    parser.add_argument("--host-idle-confirmed", action="store_true")
    args = parser.parse_args()
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, interrupted)
    if args.prepare_only:
        return prepare(args.prepared.resolve())
    if not args.host_idle_confirmed or not args.reviewed_plan_sha256 or not args.reviewed_test_binary_sha256:
        parser.error("Running requires --host-idle-confirmed and both reviewed SHA-256 arguments")
    prepared = json.loads(args.prepared.read_text())
    if (prepared.get("schema") != 1 or prepared.get("platform") != "iOS Simulator"
            or prepared.get("target") != TARGET or prepared.get("runner_bundle_id") != RUNNER
            or prepared.get("source_sha256") != source_hashes(SOURCES)):
        raise RuntimeError("Unexpected or stale launch preparation manifest")
    plan_path = Path(prepared["plan"])
    raw = plan_path.read_bytes()
    if (digest(plan_path) != args.reviewed_plan_sha256
            or prepared["plan_sha256"] != args.reviewed_plan_sha256):
        raise RuntimeError("Reviewed test plan changed")
    plan = plistlib.loads(raw)
    if set(plan) != {TARGET, "__xctestrun_metadata__"}:
        raise RuntimeError("Unexpected test targets")
    target = plan[TARGET]
    if target.get("TestHostBundleIdentifier") != RUNNER or not target.get("UseUITargetAppProvidedByTests"):
        raise RuntimeError("Unexpected test runner binding")
    binary = plan_path.parent / ("Debug-iphonesimulator/" + TARGET + "-Runner.app/PlugIns/" + TARGET + ".xctest/" + TARGET)
    if (binary.resolve() != Path(prepared["test_binary"]).resolve()
            or digest(binary) != args.reviewed_test_binary_sha256
            or prepared["test_binary_sha256"] != args.reviewed_test_binary_sha256):
        raise RuntimeError("Reviewed test binary changed")
    identity = command(["xcrun", "simctl", "list", "devices", "booted", "--json"], 10)
    if identity["returncode"]:
        raise RuntimeError(identity["output"])
    devices = [d for group in json.loads(identity["output"])["devices"].values() for d in group]
    owned = [d for d in devices if d.get("udid") == DEVICE]
    if len(owned) != 1 or owned[0].get("name") != NAME or not owned[0].get("isAvailable") or owned[0].get("state") != "Booted":
        raise RuntimeError("Owned booted simulator identity not established")
    output = EVIDENCE / ("launch-run-" + str(time.time_ns()))
    output.mkdir()
    (output / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    # Leave room within 180 seconds for preflight, bounded drain/reap and runner cleanup.
    deadline = time.time() + 140
    target.setdefault("EnvironmentVariables", {})["KIWIX_LAUNCH_ACTION_DEADLINE"] = str(deadline)
    target["TestTimeoutsEnabled"] = True
    target["DefaultTestExecutionTimeAllowance"] = 60
    target["MaximumTestExecutionTimeAllowance"] = 60
    authorized = plan_path.parent / (output.name + ".xctestrun")
    with authorized.open("xb") as stream:
        plistlib.dump(plan, stream)
    report = {"status": "running", "device": DEVICE, "action_deadline_epoch": deadline,
              "source_plan_sha256": args.reviewed_plan_sha256, "authorized_plan": str(authorized),
              "test_binary_sha256": args.reviewed_test_binary_sha256,
              "source_sha256": prepared["source_sha256"], "prepared_manifest": str(args.prepared),
              "result_bundle": str(output / "LaunchMetrics.xcresult")}
    try:
        argv = ["xcodebuild", "test-without-building", "-xctestrun", str(authorized),
                "-destination", "platform=iOS Simulator,id=" + DEVICE,
                "-destination-timeout", "15", "-parallel-testing-enabled", "NO",
                "-maximum-concurrent-test-device-destinations", "1",
                "-test-timeouts-enabled", "YES", "-default-test-execution-time-allowance", "60",
                "-maximum-test-execution-time-allowance", "60",
                "-only-testing:" + TARGET + "/SimulatorLaunchMetrics",
                "-resultBundlePath", str(output / "LaunchMetrics.xcresult")]
        report["execution"] = command(argv, max(1, deadline - time.time()))
        (output / "xcodebuild.log").write_text(report["execution"]["output"])
        report["status"] = "completed" if report["execution"]["returncode"] == 0 else "failed"
    except BaseException as error:
        report["status"] = "interrupted_or_timed_out"
        report["error"] = repr(error)
        if getattr(error, "command_output", None):
            (output / "xcodebuild-interrupted.log").write_text(error.command_output)
    finally:
        # The simulator runner is not a child of xcodebuild; stop only its exact bundle.
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, signal.SIG_IGN)
        try:
            report["runner_terminate"] = command(["xcrun", "simctl", "terminate", DEVICE, RUNNER], 10)
            census = command(["xcrun", "simctl", "spawn", DEVICE, "launchctl", "list"], 10)
            report["runner_census"] = census
            live = [line for line in census["output"].splitlines()
                    if RUNNER in line and line.split() and line.split()[0].isdigit()]
            report["runner_absence_confirmed"] = census["returncode"] == 0 and not live
            if not report["runner_absence_confirmed"]:
                report["status"] = "cleanup_unconfirmed"
        except BaseException as error:
            report["status"] = "cleanup_unconfirmed"
            report["cleanup_error"] = repr(error)
        (output / "execution.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"status": report["status"], "output": str(output),
                          "runner_absence_confirmed": report.get("runner_absence_confirmed", False)}))
    return 0 if report["status"] == "completed" and report.get("runner_absence_confirmed") else 1


if __name__ == "__main__":
    raise SystemExit(main())
