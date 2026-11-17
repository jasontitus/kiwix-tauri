#!/usr/bin/env python3
"""Offline XCTest preparation and one bounded device or owned-simulator UI window.

Source/artifact independent review is a contributor workflow gate (AGENTS.md),
not a fabricated runtime approval protocol. Never run this helper before review.
"""
import argparse
import datetime as dt
import fnmatch
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from urllib.parse import quote, unquote, urlparse
import uuid
from contextlib import nullcontext
from iphone_memory import checkpoint_footprint, natural, snapshot_footprint
from iphone_peer import (IPhonePeer, RECEIVER_FINAL_RUNNER_SECONDS, RECEIVER_OWNER_RESERVE_SECONDS,
                         RECEIVER_TOTAL_RESERVE_SECONDS, receiver_cleanup_confirmed)

ROOT = Path(__file__).resolve().parent
UI = ROOT / "iphone-ui"
RUNNER_ID = "com.tiltastech.kiwix.phonewindow.xctrunner"
WIKI_ID = "dd8a2bd2-c05b-ac6c-8074-6c30e14803fb"
MAP_ID = "c495a7aa-88f2-5dcc-a7bc-908af8a61cf2"
UPSTREAM_COMMIT = "09a5ff1c9c3ffeda258ad593bbe16da612cfb149"
STATE = "Library/Application Support/{bundle}/state.json"
CLEANING_UP = False
TIMING = None
TIMING_CATEGORY = "host_prepare"


def start_timing(output, category="host_prepare", expected_sha256=None):
    global TIMING, TIMING_CATEGORY
    path = ROOT / "testing_timing.py"
    content = path.read_bytes()
    if hashlib.sha256(content).hexdigest() != (expected_sha256 or digest(path)):
        raise RuntimeError("Frozen timing dependency changed before import")
    spec = importlib.util.spec_from_file_location("testing_timing", path)
    module = importlib.util.module_from_spec(spec)
    exec(compile(content, str(path), "exec"), module.__dict__)
    TIMING = module.TimingLog(output, os.environ.get("KIWIX_TIMING_ITERATION") or uuid.uuid4().hex, "iphone-window")
    TIMING_CATEGORY = category


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def tree(path):
    """Unambiguous owned artifact inventory, including symlink targets."""
    root = Path(path).resolve()
    entries = []
    for directory, directories, files in os.walk(root, followlinks=False):
        links = [name for name in directories if (Path(directory) / name).is_symlink()]
        for name in sorted(files + links):
            item = Path(directory) / name
            relative = item.relative_to(root).as_posix()
            if item.is_symlink():
                item.resolve(strict=True).relative_to(root)
                entries.append([relative, "link", os.readlink(item)])
            elif item.is_file():
                entries.append([relative, "file", digest(item), item.stat().st_mode & 0o111])
            else:
                raise RuntimeError("Non-regular artifact entry")
    return sorted(entries)


def tree_hash(entries):
    return hashlib.sha256(json.dumps(entries, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def save(path, value):
    with Path(path).open("x") as stream:
        json.dump(value, stream, indent=2, default=str)
        stream.write("\n")


def command(argv, timeout=30, output=None, input_data=None):
    category = "cleanup" if CLEANING_UP else TIMING_CATEGORY
    if Path(str(argv[0])).name == "xcodebuild" and "build-for-testing" in argv:
        category = "build"
    with TIMING.span(category, "host-command") if TIMING else nullcontext():
        return command_process(argv, timeout, output, input_data)


def command_process(argv, timeout=30, output=None, input_data=None):
    """Bound a whole process group and retain diagnostics even on interruption."""
    started = time.monotonic()
    process = subprocess.Popen([str(x) for x in argv], stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE,
                               stdin=subprocess.PIPE if input_data is not None else None,
                               start_new_session=True)
    stdout, stderr = b"", b""
    failure = None
    try:
        stdout, stderr = process.communicate(input=input_data, timeout=timeout)
    except BaseException as error:
        failure = error
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            stdout, stderr = process.communicate(timeout=2)
        except subprocess.TimeoutExpired as reap_error:
            stdout = reap_error.output or getattr(error, "output", None) or b""
            stderr = reap_error.stderr or getattr(error, "stderr", None) or b""
            process.wait(timeout=2)
        raise
    finally:
        diagnostic = {"stage": Path(output).stem if output else " ".join(map(str, argv[:4])),
                      "argv": list(map(str, argv)), "timeout_seconds": timeout,
                      "duration_seconds": time.monotonic() - started,
                      "returncode": process.returncode,
                      "error": type(failure).__name__ + ": " + str(failure) if failure else None}
        if output:
            Path(output).write_bytes(json.dumps(diagnostic).encode() + b"\nSTDOUT\n" +
                                     stdout + b"\nSTDERR\n" + stderr)
        elif failure:
            print(json.dumps(diagnostic), file=sys.stderr)
            print(stdout.decode(errors="replace"), file=sys.stderr)
            print(stderr.decode(errors="replace"), file=sys.stderr)
    if process.returncode:
        raise RuntimeError(f"Command failed ({process.returncode}): {argv[0:4]}; {stderr.decode(errors='replace')[-1500:]}")
    return stdout


def profile(path):
    return plistlib.loads(command(["security", "cms", "-D", "-i", path], 15))


def signing_check(app, device, team, signer):
    app = Path(app)
    command(["codesign", "--verify", "--strict", "--deep", app], 20)
    bundles = [app] + sorted(app.rglob("*.appex"))
    for bundle in bundles:
        info = plistlib.loads((bundle / "Info.plist").read_bytes())
        embedded = profile(bundle / "embedded.mobileprovision")
        expiry = embedded["ExpirationDate"].replace(tzinfo=dt.timezone.utc)
        if expiry <= dt.datetime.now(dt.timezone.utc):
            raise RuntimeError("Expired embedded provisioning profile")
        entitlements = embedded["Entitlements"]
        bundle_id = info["CFBundleIdentifier"]
        if device not in embedded.get("ProvisionedDevices", []):
            raise RuntimeError("Device absent from embedded profile")
        if embedded.get("TeamIdentifier") != [team]:
            raise RuntimeError("Unexpected signing team")
        if not fnmatch.fnmatchcase(team + "." + bundle_id, entitlements.get("application-identifier", "")):
            raise RuntimeError("Profile does not authorize bundle identifier")
        certificates = embedded.get("DeveloperCertificates", [])
        if signer.upper() not in [hashlib.sha1(cert).hexdigest().upper() for cert in certificates]:
            raise RuntimeError("Requested signing certificate absent from profile")
        # Extract the actual signing leaf; profile membership alone is not signature identity.
        import tempfile
        with tempfile.TemporaryDirectory(prefix="kiwix-signature-") as temporary:
            prefix = str(Path(temporary) / "cert")
            command(["codesign", "-d", "--extract-certificates=" + prefix, bundle], 10)
            if hashlib.sha1(Path(prefix + "0").read_bytes()).hexdigest().upper() != signer.upper():
                raise RuntimeError("Actual signing leaf differs from requested identity")
        actual = plistlib.loads(command(["codesign", "-d", "--entitlements", ":-", bundle], 10))
        if actual.get("application-identifier") != team + "." + bundle_id:
            raise RuntimeError("Signed application identifier mismatch")
        if actual.get("com.apple.developer.team-identifier") != team:
            raise RuntimeError("Signed entitlement team mismatch")
    return plistlib.loads((app / "Info.plist").read_bytes())


def sources():
    return {str(path.relative_to(ROOT)): digest(path) for path in
            [Path(__file__).resolve(), ROOT / "iphone_peer.py", ROOT / "iphone_memory.py",
             ROOT / "testing_timing.py", UI / "PhoneWindowUI.swift", UI / "project.yml"]}


def prepare(args):
    if args.platform == "simulator":
        prepare_simulator(args)
        return
    # No CoreDevice, connected destination, device listing or PMD transport here.
    mobile_tool(args.pmd)
    if args.test_mode != "launch":
        map_fixture(args.map_source)
    app = args.signing_source.resolve()
    embedded = profile(app / "embedded.mobileprovision")
    team = embedded["TeamIdentifier"][0]
    signing_check(app, args.device, team, args.signing_identity)
    candidates = []
    for directory in [Path.home() / "Library/Developer/Xcode/UserData/Provisioning Profiles",
                      Path.home() / "Library/MobileDevice/Provisioning Profiles"]:
        for item in sorted(directory.glob("*.mobileprovision")):
            data = profile(item)
            if (data.get("TeamIdentifier") == [team]
                    and args.device in data.get("ProvisionedDevices", [])
                    and data["ExpirationDate"].replace(tzinfo=dt.timezone.utc) > dt.datetime.now(dt.timezone.utc)
                    and fnmatch.fnmatchcase(team + "." + RUNNER_ID,
                        data.get("Entitlements", {}).get("application-identifier", ""))
                    and args.signing_identity.upper() in [hashlib.sha1(c).hexdigest().upper()
                        for c in data.get("DeveloperCertificates", [])]):
                # Offline app preparation must not silently produce an online-
                # verified runner. Both run while the test phone has no internet.
                if not embedded.get("PPQCheck", False) and data.get("PPQCheck", False):
                    continue
                candidates.append((data["UUID"], item, digest(item)))
    if not candidates:
        raise RuntimeError("No existing local profile authorizes runner/team/certificate/device; no portal changes attempted")
    source_hashes = sources()
    toolchain = command(["xcodebuild", "-version"], 15).decode()
    fingerprint = hashlib.sha256(json.dumps([source_hashes, toolchain, team,
        args.signing_identity, args.device, candidates[0][0], candidates[0][2]], sort_keys=True).encode()).hexdigest()
    output = args.output.resolve() / fingerprint
    manifest_path = output / "manifest.json"
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text())
        validate_runner(manifest, args.device, args.platform)
        print(json.dumps({"manifest_path": str(manifest_path), "status": "ready", "cached": True}))
        return
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    shutil.copy2(UI / "PhoneWindowUI.swift", output / "PhoneWindowUI.swift")
    shutil.copy2(UI / "project.yml", output / "project.yml")
    command(["xcodegen", "generate", "--spec", output / "project.yml",
             "--project", output], 30, output / "xcodegen.log")
    derived = output / "DerivedData"
    # Xcode rejects its managed profiles under Manual signing. Permit its local
    # automatic selection only for that profile kind; the exact selected bytes
    # and signer are still mandatory below. Never allow portal/profile updates.
    xcode_managed = profile(candidates[0][1]).get("IsXcodeManaged") is True
    signing_arguments = (["CODE_SIGN_STYLE=Automatic", "CODE_SIGN_IDENTITY=Apple Development"]
        if xcode_managed else ["CODE_SIGN_STYLE=Manual", f"CODE_SIGN_IDENTITY={args.signing_identity}",
                              f"PROVISIONING_PROFILE_SPECIFIER={candidates[0][0]}"])
    command(["xcodebuild", "build-for-testing", "-project", output / "KiwixPhoneWindowUI.xcodeproj",
        "-scheme", "KiwixPhoneWindowUI", "-configuration", "Debug", "-sdk", "iphoneos",
        "-destination", "generic/platform=iOS", "-derivedDataPath", derived,
        "-parallel-testing-enabled", "NO", f"DEVELOPMENT_TEAM={team}", *signing_arguments],
        1800, output / "build.log")
    products = derived / "Build/Products"
    runner = products / "Debug-iphoneos/KiwixPhoneWindowUI-Runner.app"
    plans = list(products.glob("KiwixPhoneWindowUI_*.xctestrun"))
    if len(plans) != 1:
        raise RuntimeError("Expected one built XCTest plan")
    info = signing_check(runner, args.device, team, args.signing_identity)
    if info["CFBundleIdentifier"] != RUNNER_ID:
        raise RuntimeError("Unexpected built runner identity")
    actual_profile_uuid = profile(runner / "embedded.mobileprovision")["UUID"]
    if actual_profile_uuid != candidates[0][0] or digest(runner / "embedded.mobileprovision") != candidates[0][2]:
        raise RuntimeError("Built runner did not retain the selected exact provisioning profile")
    inventory = tree(runner)
    manifest = {"schema": 1, "platform": "device", "status": "ready", "runner_app": str(runner),
        "runner_bundle_id": RUNNER_ID, "xctestrun": str(plans[0]), "xctestrun_sha256": digest(plans[0]),
        "source_sha256": source_hashes, "artifact_sha256": tree_hash(inventory), "artifact_files": inventory,
        "team": team, "signer_sha1": args.signing_identity.upper(), "toolchain": toolchain,
        "fingerprint": fingerprint, "profile_uuid": actual_profile_uuid,
        "review_provenance": None, "runtime_status": "not_exercised"}
    save(manifest_path, manifest)
    print(json.dumps({"manifest_path": str(manifest_path), "status": "ready"}))


def validate_runner(manifest, device, platform="device"):
    if manifest.get("platform") != platform:
        raise RuntimeError("Runner platform differs from requested platform")
    if manifest.get("status") != "ready" or manifest.get("schema") != 1:
        raise RuntimeError("Runner not ready")
    if manifest["source_sha256"] != sources():
        raise RuntimeError("Runner source changed; rebuild/review offline before a window")
    if digest(manifest["xctestrun"]) != manifest["xctestrun_sha256"]:
        raise RuntimeError("XCTest plan bytes changed")
    if tree_hash(tree(manifest["runner_app"])) != manifest["artifact_sha256"]:
        raise RuntimeError("Runner artifact bytes changed")
    info = (simulator_artifact(manifest, "runner_app") if platform == "simulator" else
            signing_check(manifest["runner_app"], device, manifest["team"], manifest["signer_sha1"]))
    if info["CFBundleIdentifier"] != RUNNER_ID:
        raise RuntimeError("Runner bundle identifier mismatch")


def validate(args):
    app = json.loads(args.app_manifest.read_text())
    runner = json.loads(args.runner_manifest.read_text())
    if app.get("schema") != 1 or app.get("status") != "ready" or not app.get("source_commit"):
        raise RuntimeError("A ready exact-source application manifest is required")
    if tree_hash(tree(app["app_path"])) != app["artifact_sha256"]:
        raise RuntimeError("Application bytes differ from exact reviewed build manifest")
    if app.get("platform") != args.platform:
        raise RuntimeError("Application platform differs from requested platform")
    validate_runner(runner, args.device, args.platform)
    info = (simulator_artifact(app, "app_path") if args.platform == "simulator" else
            signing_check(app["app_path"], args.device, app["team"], app["signer_sha1"]))
    if info["CFBundleIdentifier"] != app["bundle_id"] or (
            args.platform == "device" and runner["team"] != app["team"]):
        raise RuntimeError("Application/runner identity mismatch")
    if args.test_mode == "nearby":
        source = args.nearby_json
        if args.platform != "device" or not isinstance(source, dict):
            raise RuntimeError("Nearby requires an explicit physical-phone source specification")
        for key in ("archive_id", "file_name", "title", "expected_text", "peer", "source", "sha256"):
            if not isinstance(source.get(key), str) or not source[key].strip():
                raise RuntimeError("Missing Nearby source field: " + key)
        uuid.UUID(source["archive_id"])
        if Path(source["file_name"]).name != source["file_name"] or "/" in source["file_name"] or "\\" in source["file_name"]:
            raise RuntimeError("Nearby source file name must be a basename")
        path = Path(source["source"]).resolve(strict=True)
        if not path.is_file() or path.stat().st_size != source.get("size") or digest(path) != source["sha256"]:
            raise RuntimeError("Nearby source bytes changed after preparation")
        direction = source.get("direction", "android-to-iphone")
        operation = source.get("operation", "transfer")
        apple = direction in ("mac-to-iphone", "iphone-to-mac", "iphone-to-iphone")
        if direction not in ("android-to-iphone", "iphone-to-android", "mac-to-iphone", "iphone-to-mac", "iphone-to-iphone") or operation not in ("transfer", "verify-received"):
            raise RuntimeError("Invalid Nearby direction or operation")
        if apple and (source.get("topology") != "apple-lan" or operation != "transfer"):
            raise RuntimeError("Apple Nearby requires explicit apple-lan transfer topology")
        if apple:
            validate_mac_prepared(source["mac"])
            if direction == "iphone-to-iphone" and not getattr(args, "_iphone_receiver", None):
                validate_iphone_prepared(source, args, app)
            if getattr(args, "window_seconds", 600) > 600:
                raise RuntimeError("Paired Nearby windows cannot exceed ten minutes")
        with path.open("rb") as stream:
            header = stream.read(24)
        if header[:4] != b"ZIM\x04" or str(uuid.UUID(bytes=header[8:24])) != source["archive_id"]:
            raise RuntimeError("Nearby frozen source UUID does not match its actual ZIM header")
        if operation == "verify-received":
            if direction != "android-to-iphone":
                raise RuntimeError("Existing-received verification is only valid for the iPhone receiver")
            if not isinstance(source.get("job_id"), str):
                raise RuntimeError("Existing-received verification requires the exact frozen job_id")
            uuid.UUID(source["job_id"])
        if operation == "transfer" and not apple:
            android = source["android"]
            for key, expected in (("prepared", "prepared_sha256"), ("tool", "tool_sha256")):
                if digest(Path(android[key])) != android[expected]:
                    raise RuntimeError("Reviewed Android automation changed after preparation")
            receipt = json.loads(Path(android["prepared"]).read_text())
            if receipt.get("timing_tool_sha256") != digest(ROOT / "testing_timing.py"):
                raise RuntimeError("Prepared Android timing dependency missing or changed")
            spec = importlib.util.spec_from_file_location("android_nearby", android["tool"])
            peer_tool = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(peer_tool)
            if (receipt.get("schema") != 1 or receipt.get("target_package") != peer_tool.TARGET
                    or receipt.get("instrumentation") != peer_tool.RUNNER
                    or receipt.get("host_source_sha256") != digest(android["tool"])
                    or receipt.get("source_sha256") != peer_tool.sources()
                    or not re.fullmatch(r"[0-9a-f]{64}", receipt.get("target_apk_sha256", ""))
                    or digest(receipt["tool_apk"]) != receipt["tool_apk_sha256"]
                    or not receipt.get("certificate_sha256") or not android.get("device")):
                raise RuntimeError("Paired Nearby requires current prepared Android sources, target and independent test APK")
            if getattr(args, "window_seconds", 600) > 600:
                raise RuntimeError("Paired Nearby windows cannot exceed ten minutes")
    pmd = mobile_tool(args.pmd) if args.platform == "device" else None
    map_source = None if args.test_mode in ("nearby", "launch") else map_fixture(args.map_source)
    if args.platform == "simulator" and (
            args.test_mode != "comparison" or app["source_commit"] != "3cce5af5677350996301b6cc335b30a5a00d4938"):
        raise RuntimeError("Simulator requires the pinned actual paired comparison build")
    baseline = None
    wiki_source = None
    if args.test_mode == "comparison":
        if args.baseline_manifest is None or args.wiki_source is None or map_source is None:
            raise RuntimeError("Comparison requires ready native baseline, real Wikipedia source and Hawaii source before START")
        baseline = json.loads(args.baseline_manifest.read_text())
        if (baseline.get("schema") != 1 or baseline.get("status") != "ready"
                or baseline.get("source_commit") != UPSTREAM_COMMIT
                or not baseline.get("bundle_id", "").endswith(".comparison.upstream")
                or baseline["bundle_id"] == app["bundle_id"]):
            raise RuntimeError("Baseline must be the pinned isolated source-built upstream release, never Maps Beta/App Store data")
        if tree_hash(tree(baseline["app_path"])) != baseline["artifact_sha256"]:
            raise RuntimeError("Native baseline artifact bytes changed")
        if baseline.get("platform") != args.platform:
            raise RuntimeError("Native baseline platform mismatch")
        native_info = (simulator_artifact(baseline, "app_path") if args.platform == "simulator" else
                       signing_check(baseline["app_path"], args.device, baseline["team"], baseline["signer_sha1"]))
        if native_info["CFBundleIdentifier"] != baseline["bundle_id"] or (
                args.platform == "device" and baseline["team"] != app["team"]):
            raise RuntimeError("Native baseline identity/team mismatch")
        wiki_source = wiki_fixture(args.wiki_source)
    return app, runner, pmd, map_source, baseline, wiki_source


def validate_iphone_prepared(source, args, sender_app):
    """Recursively validate the exact receiver manifests without device contact."""
    receiver = source.get("iphone")
    if (not isinstance(receiver, dict) or set(receiver) != {"device", "app", "runner"}
            or not isinstance(receiver["device"], str) or not receiver["device"]
            or receiver["device"] == args.device):
        raise RuntimeError("Paired iPhones require exact distinct prepared device identities")
    for field in ("app", "runner"):
        artifact = receiver[field]
        if (not isinstance(artifact, dict) or set(artifact) != {"path", "sha256"}
                or not Path(artifact["path"]).is_absolute() or digest(Path(artifact["path"])) != artifact["sha256"]):
            raise RuntimeError("Prepared secondary iPhone manifest changed: " + field)
    child = argparse.Namespace(**vars(args))
    child.device = receiver["device"]
    child.app_manifest = Path(receiver["app"]["path"])
    child.runner_manifest = Path(receiver["runner"]["path"])
    child._iphone_receiver = True
    receiver_app, _, _, _, _, _ = validate(child)
    if receiver_app["source_commit"] != sender_app["source_commit"]:
        raise RuntimeError("Both iPhone artifacts must use the same reviewed source revision")


def simulator_artifact(manifest, field):
    """Bind inventory, architecture and Mach-O platform, without provisioning claims."""
    app = Path(manifest[field]).resolve(strict=True)
    inventory = tree(app)
    if (manifest.get("platform") != "simulator" or manifest.get("architectures") != ["arm64"]
            or inventory != manifest.get("artifact_files")
            or tree_hash(inventory) != manifest.get("artifact_sha256")):
        raise RuntimeError("Simulator artifact inventory/architecture binding mismatch")
    info = plistlib.loads((app / "Info.plist").read_bytes())
    if (info.get("CFBundleSupportedPlatforms") != ["iPhoneSimulator"]
            or info.get("DTPlatformName") != "iphonesimulator"):
        raise RuntimeError("Expected genuine iPhoneSimulator bundle")
    executable = app / info["CFBundleExecutable"]
    # Check every shipped Mach-O, not only the outer runner/application binary.
    binaries = []
    for entry in inventory:
        if entry[1] != "file":
            continue
        candidate = app / entry[0]
        with candidate.open("rb") as stream:
            magic = stream.read(4)
        if magic not in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf",
                         b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"):
            continue
        binaries.append(candidate)
        if command(["xcrun", "lipo", "-archs", candidate], 10).decode().strip() != "arm64":
            raise RuntimeError("Simulator Mach-O must contain exactly arm64: " + str(candidate))
        load = command(["xcrun", "vtool", "-show-build", candidate], 10).decode()
        if "platform IOSSIMULATOR" not in load:
            raise RuntimeError("Mach-O is not built for the iOS simulator: " + str(candidate))
    if executable not in binaries:
        raise RuntimeError("Missing actual simulator main Mach-O executable")
    return info


def prepare_simulator(args):
    map_fixture(args.map_source)
    source_hashes = sources()
    toolchain = command(["xcodebuild", "-version"], 15).decode()
    fingerprint = hashlib.sha256(json.dumps(
        [source_hashes, toolchain, "simulator", "arm64"], sort_keys=True).encode()).hexdigest()
    output = args.output.resolve() / fingerprint
    manifest_path = output / "manifest.json"
    if manifest_path.exists():
        validate_runner(json.loads(manifest_path.read_text()), args.device, "simulator")
        print(json.dumps({"manifest_path": str(manifest_path), "status": "ready", "cached": True}))
        return
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    shutil.copy2(UI / "PhoneWindowUI.swift", output / "PhoneWindowUI.swift")
    shutil.copy2(UI / "project.yml", output / "project.yml")
    command(["xcodegen", "generate", "--spec", output / "project.yml",
             "--project", output], 30, output / "xcodegen.log")
    derived = output / "DerivedData"
    command(["xcodebuild", "build-for-testing", "-project", output / "KiwixPhoneWindowUI.xcodeproj",
        "-scheme", "KiwixPhoneWindowUI", "-configuration", "Debug", "-sdk", "iphonesimulator",
        "-destination", "generic/platform=iOS Simulator", "-derivedDataPath", derived,
        "-parallel-testing-enabled", "NO", "ARCHS=arm64", "ONLY_ACTIVE_ARCH=NO",
        "CODE_SIGNING_ALLOWED=NO", "CODE_SIGNING_REQUIRED=NO", "CODE_SIGN_IDENTITY="],
        1800, output / "build.log")
    products = derived / "Build/Products"
    runner = products / "Debug-iphonesimulator/KiwixPhoneWindowUI-Runner.app"
    plans = list(products.glob("KiwixPhoneWindowUI_*.xctestrun"))
    if len(plans) != 1:
        raise RuntimeError("Expected one simulator XCTest plan")
    inventory = tree(runner)
    manifest = {"schema": 1, "platform": "simulator", "architectures": ["arm64"],
        "status": "ready", "runner_app": str(runner), "runner_bundle_id": RUNNER_ID,
        "xctestrun": str(plans[0]), "xctestrun_sha256": digest(plans[0]),
        "source_sha256": source_hashes, "artifact_sha256": tree_hash(inventory),
        "artifact_files": inventory, "toolchain": toolchain, "fingerprint": fingerprint,
        "review_provenance": None, "runtime_status": "not_exercised",
        "physical_memory": "not-applicable", "provisioning_applicable": False, "signing": None}
    if simulator_artifact(manifest, "runner_app")["CFBundleIdentifier"] != RUNNER_ID:
        raise RuntimeError("Unexpected simulator runner identity")
    save(manifest_path, manifest)
    print(json.dumps({"manifest_path": str(manifest_path), "status": "ready", "platform": "simulator"}))


def mobile_tool(path=None):
    executable = str(path.expanduser().resolve(strict=True)) if path else shutil.which("pymobiledevice3")
    if not executable:
        raise RuntimeError("Modern pymobiledevice3 must already be installed offline")
    version = command([executable, "version"], 5).decode().strip()
    if not version.startswith("9.36."):
        raise RuntimeError("Reviewed PMD 9.36.x required; resolve tools before START")
    return executable


def map_fixture(source):
    if source is None:
        return None
    source = source.resolve(strict=True)
    with source.open("rb") as stream:
        header = stream.read(32)
    expected = "d2fe53fc134651433c2b174ff8a683aeacf9529eea45111d4e599b0b9f95b18d"
    if (source.stat().st_size != 150605015 or header[:4] != b"ZIM\x04"
            or str(uuid.UUID(bytes=header[8:24])) != MAP_ID or digest(source) != expected):
        raise RuntimeError("Expected byte-exact real Hawaii StreetZIM; prepare fixture offline")
    return source


def wiki_fixture(source):
    source = source.resolve(strict=True)
    with source.open("rb") as stream:
        header = stream.read(32)
    if (source.stat().st_size != 333429922 or header[:4] != b"ZIM\x04"
            or str(uuid.UUID(bytes=header[8:24])) != WIKI_ID
            or digest(source) != "2eb7fd115bfb8644a08a9482f01f52b507428b1b8a30129fead92f7b6715440b"):
        raise RuntimeError("Expected byte-exact real Wikipedia 100 fixture")
    return source


def native_size_matches(metadata_bytes, actual_bytes):
    # libkiwix 14.2.1 Book::update stores getArchiveFileSize (whole KiB) << 10.
    # This validates that metadata contract, not an exact payload-byte readback.
    return metadata_bytes == (actual_bytes // 1024) * 1024


def read_native_database(database, preferences):
    """Read only a private copied store; never open or modify a device database."""
    def blob_hash(value):
        return hashlib.sha256(value).hexdigest() if value is not None else None

    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as connection:
        connection.row_factory = sqlite3.Row
        ids = {row["Z_PK"]: str(uuid.UUID(bytes=row["ZFILEID"]))
               for row in connection.execute("SELECT Z_PK,ZFILEID FROM ZZIMFILE")}
        books = [{"id": ids[row["Z_PK"]], "pk": row["Z_PK"], "title": row["ZNAME"],
                  "size": row["ZSIZE"], "available": row["ZISMISSING"] == 0,
                  "bookmark_sha256": blob_hash(row["ZFILEURLBOOKMARK"])}
                 for row in connection.execute(
                     "SELECT Z_PK,ZNAME,ZSIZE,ZISMISSING,ZFILEURLBOOKMARK FROM ZZIMFILE WHERE ZFILEURLBOOKMARK IS NOT NULL")]
        tabs = [{"id": str(row["Z_PK"]), "bookId": ids.get(row["ZZIMFILE"]), "title": row["ZTITLE"] or "",
                 "created": row["ZCREATED"], "lastOpened": row["ZLASTOPENED"],
                 "interaction_sha256": blob_hash(row["ZINTERACTIONSTATE"])}
                for row in connection.execute("SELECT * FROM ZTAB")]
        bookmarks = [{key: (blob_hash(row[key]) if isinstance(row[key], bytes) else row[key])
                      for key in row.keys()} for row in connection.execute("SELECT * FROM ZBOOKMARK ORDER BY Z_PK")]
    if len({book["id"] for book in books}) != len(books):
        raise RuntimeError("Native opened-archive UUIDs are not unique")
    return {"initialized": True, "books": books, "tabs": tabs, "bookmarks": bookmarks, "preferences": preferences}


def native_preservation(before, after, old_container, new_container, mutable_tabs):
    if before is None:
        return
    new_books = {book["id"]: book for book in after["books"]}
    for book in before["books"]:
        current = new_books.get(book["id"])
        if current is None or any(current[key] != book[key] for key in ("id", "pk", "title", "size", "available")):
            raise RuntimeError("Native preexisting archive metadata disappeared or changed")
        if old_container == new_container and current["bookmark_sha256"] != book["bookmark_sha256"]:
            raise RuntimeError("Native archive bookmark authority changed without container relocation")
    current_tabs = {tab["id"]: tab for tab in after["tabs"]}
    for tab in before["tabs"]:
        current = current_tabs.get(tab["id"])
        if current is None:
            raise RuntimeError("Native preexisting tab disappeared")
        keys = ("id", "bookId", "created") if tab["id"] in mutable_tabs else (
            "id", "bookId", "created", "title", "interaction_sha256")
        if any(current[key] != tab[key] for key in keys):
            raise RuntimeError("Native unrelated tab state or fixture tab identity changed")
    if before["bookmarks"] != after["bookmarks"]:
        raise RuntimeError("Native bookmarks changed")
    # Pinned native startup/catalog/reading metadata is not a settings mutation.
    runtime_keys = {"recentSearchTexts", "savedMenuNavigation", "libraryETag", "libraryLastRefresh",
                    "categoriesToLanguages", "isFirstLaunch", "hasSeenCategories", "lastDonationTime"}
    preferences = lambda state: {key: value for key, value in state["preferences"].items() if key not in runtime_keys}
    if preferences(before) != preferences(after):
        raise RuntimeError("Native unrelated preferences changed")


def foreground_observer(window, log, phase, nonce, bundle):
    """Consume runner heartbeats; never infer foreground from a process name."""
    offset = 0
    pending = ""
    latest = None

    def observe():
        nonlocal offset, pending, latest
        window.remaining()
        with log.open("r", errors="replace") as stream:
            stream.seek(offset)
            pending += stream.read()
            offset = stream.tell()
        lines = pending.split("\n")
        pending = lines.pop()
        for line in lines:
            if line.startswith("KIWIX_FAILURE "):
                raise RuntimeError(line.split("KIWIX_FAILURE ", 1)[1])
            if " error:" in line:
                raise RuntimeError(line.split(" error:", 1)[1])
            if line.startswith("KIWIX_FOREGROUND "):
                fields = line.split("KIWIX_FOREGROUND ", 1)[1].split()
                if len(fields) == 4 and fields[:3] == [phase, nonce, bundle]:
                    latest = float(fields[3])
        return latest is not None and -5 <= time.time() - latest <= 5

    initial = time.monotonic() + min(3, window.remaining())
    while not observe():
        if time.monotonic() >= initial:
            raise RuntimeError("No fresh authenticated XCTest foreground heartbeat at " + phase)
        time.sleep(0.1)

    def require_foreground():
        if not observe():
            raise RuntimeError("XCTest foreground heartbeat expired at " + phase)
    return require_foreground


def acknowledge(window, phase, nonce, bundle, deadline, runner_bundle, payload_url=None, nearby_binding=None, failure=None):
    slug = phase.replace("/", "-")
    path = window.output / (slug + "-ack-payload.json")
    payload = {"schema": 1, "phase": phase, "nonce": nonce, "app_id": bundle, "deadline": deadline}
    if payload_url is not None:
        payload["payload_url"] = payload_url
    if nearby_binding is not None:
        payload["nearby_binding"] = nearby_binding
    if failure is not None:
        payload["failure"] = str(failure)[:1000]
    payload["complete"] = "KIWIX_ACK_END_" + nonce
    with path.open("xb") as stream:
        stream.write(json.dumps(payload, separators=(",", ":")).encode() + b"\n")
    window.ctl(slug + "-ack", ["copy", "to", "--domain-type", "appDataContainer",
        "--domain-identifier", runner_bundle, "--source", str(path),
        "--destination", "Documents/checkpoint-" + nonce + "-" + slug + ".json"], 5)


def nearby_job(window, label, bundle, source):
    """Read the real saved job; verification never schedules another download."""
    path = window.output / (label + "-downloads.json")
    window.ctl(label + "-journal", ["copy", "from", "--domain-type", "appDataContainer",
        "--domain-identifier", bundle, "--source",
        "Library/Application Support/" + bundle + "/downloads/downloads.json",
        "--destination", str(path)], 10)
    jobs = json.loads(path.read_text())
    job = jobs.get(source["job_id"])
    if (not isinstance(job, dict) or job.get("id") != source["job_id"]
            or job.get("fileName") != source["file_name"] or job.get("status") not in ("complete", "completed")
            or job.get("total") != source["size"] or job.get("downloaded") != source["size"]
            or job.get("error") or job.get("transport") != "quic-library"
            or urlparse(job.get("url", "")).path.rsplit("/", 1)[-1] != source["archive_id"]
            or job.get("publication") != {"size": source["size"], "sha256": source["sha256"]}):
        raise RuntimeError("Expected previously received Nearby job is not complete with the frozen source size")
    return job


def mac_runtime():
    current = json.loads(command(["node", "-p",
        "JSON.stringify({node_major:Number(process.versions.node.split('.')[0]),platform:process.platform,arch:process.arch,node:require('node:fs').realpathSync(process.execPath)})"], 10))
    current["node"] = {"path": current["node"], "sha256": digest(Path(current["node"]))}
    metadata = json.loads((ROOT.parent / "node_modules/puppeteer-core/package.json").read_bytes())
    current["puppeteer_core"] = metadata.get("version")
    return current


def validate_mac_prepared(mac):
    """Check frozen helper and artifacts before any Mac process or phone action."""
    if digest(Path(mac["tool"])) != mac["tool_sha256"]:
        raise RuntimeError("Prepared Mac helper changed")
    content = Path(mac["prepared"]).read_bytes()
    if hashlib.sha256(content).hexdigest() != mac["prepared_sha256"]:
        raise RuntimeError("Prepared Mac receipt changed")
    receipt = json.loads(content)
    runtime = receipt.get("runtime", {})
    if receipt.get("schema") != 1 or runtime.get("puppeteer_core") != "25.16.0":
        raise RuntimeError("Mac preparation schema/runtime is incompatible")
    current = mac_runtime()
    if runtime != current or current.get("platform") != "darwin" or current.get("node_major", 0) < 22:
        raise RuntimeError("Prepared Mac Node/Puppeteer runtime/platform/architecture changed")
    for field in ("helper", "lockfile", "executable", "browser"):
        artifact = receipt[field]
        path = Path(artifact["path"])
        if not path.is_absolute() or not path.is_file() or digest(path) != artifact["sha256"]:
            raise RuntimeError("Prepared Mac artifact changed: " + field)
    if receipt["helper"] != {"path": mac["tool"], "sha256": mac["tool_sha256"]}:
        raise RuntimeError("Mac prepared helper identity differs from reviewed helper")
    return receipt


def nearby_jobs(window, label, bundle, cleanup=False):
    path = window.output / (label + "-downloads.json")
    listing = window.ctl(label + "-files", ["info", "files", "--domain-type", "appDataContainer",
        "--domain-identifier", bundle, "--subdirectory", "Library"], 10, cleanup=cleanup)
    relative = "Application Support/" + bundle + "/downloads/downloads.json"
    files = listing.get("files")
    if not isinstance(files, list):
        raise RuntimeError("Missing download journal inventory")
    rows = [row for row in files if row.get("relativePath") == relative]
    if not rows:
        return {}
    if len(rows) != 1 or rows[0].get("resources", {}).get("isSymbolicLink"):
        raise RuntimeError("Ambiguous or symlink download journal")
    window.ctl(label + "-journal", ["copy", "from", "--domain-type", "appDataContainer",
        "--domain-identifier", bundle, "--source", "Library/" + relative, "--destination", str(path)], 10, cleanup=cleanup)
    jobs = json.loads(path.read_bytes())
    if not isinstance(jobs, dict):
        raise RuntimeError("Unexpected download journal schema")
    return jobs


def mac_received_job(jobs, before, source, publication, state):
    expected_url = ("kiwix-library://public/" + publication["session_id"] + "/"
                    + publication["publisher_id"] + "/" + quote(publication.get("offer_id", source["archive_id"]), safe=""))
    candidates = [job for job in jobs.values() if isinstance(job, dict) and (
        job.get("fileName") == source["file_name"] or job.get("url") == expected_url)]
    if len(candidates) != 1:
        raise RuntimeError("Mac publication must identify exactly one actual received job")
    job = candidates[0]
    if (job.get("id") in before or jobs.get(job.get("id")) != job
            or job.get("url") != expected_url or job.get("transport") != "quic-library"
            or job.get("status") not in ("complete", "completed") or job.get("error")
            or job.get("fileName") != source["file_name"]
            or job.get("downloaded") != source["size"] or job.get("total") != source["size"]
            or job.get("publication") != {"size": source["size"], "sha256": source["sha256"]}):
        raise RuntimeError("Received job lacks new run-owned Mac publication and frozen-byte authority")
    uuid.UUID(job["id"])
    books = [book for book in state["books"] if book.get("id") == source["archive_id"]]
    if (len(books) != 1 or books[0].get("managed") is not True or books[0].get("sourceReference") is not None
            or job.get("destination") != books[0].get("path")
            or not isinstance(job.get("destination"), str) or not job["destination"]
            or job["id"] not in state.get("consumedAcquisitions", [])):
        raise RuntimeError("Received Mac job is not the consumed receipt for the exact managed installation")
    installation = books[0].get("installationId", "")
    if str(uuid.UUID(installation)) != installation:
        raise RuntimeError("Received Mac archive lacks canonical installation identity")
    return job


def iphone_publication(window, label, bundle, installed_identity):
    """Copy persisted identity from the exact installed source, never invent a session."""
    apps = window.ctl(label + "-app", ["info", "apps", "--bundle-id", bundle,
        "--include-container-paths"], 8).get("apps")
    if apps != installed_identity["device_receipt"]:
        raise RuntimeError("Installed iPhone source identity changed during publication")
    container_path(apps, bundle)
    listing = window.ctl(label + "-files", ["info", "files", "--domain-type", "appDataContainer",
        "--domain-identifier", bundle, "--subdirectory", "Library"], 8).get("files")
    if not isinstance(listing, list):
        raise RuntimeError("Missing exact installed iPhone publication inventory")
    captured = {}
    for name, maximum in (("peer-id", 36), ("library-generation", 20)):
        relative = "Application Support/" + bundle + "/localswarm/" + name
        rows = [row for row in listing if row.get("relativePath") == relative]
        if (len(rows) != 1 or rows[0].get("resources", {}).get("isSymbolicLink")
                or rows[0].get("resources", {}).get("isDirectory")
                or not 0 < rows[0].get("metadata", {}).get("size", 0) <= maximum):
            raise RuntimeError("Publication identity is not a bounded owned source file: " + name)
        path = window.output / (label + "-" + name)
        window.ctl(label + "-copy-" + name, ["copy", "from", "--domain-type", "appDataContainer",
            "--domain-identifier", bundle, "--source", "Library/" + relative, "--destination", str(path)], 8)
        raw = path.read_bytes()
        if not 0 < len(raw) <= maximum:
            raise RuntimeError("Invalid copied publication identity length")
        captured[name] = raw.decode("ascii")
    publisher = captured["peer-id"]
    generation = captured["library-generation"]
    if str(uuid.UUID(publisher)) != publisher or not re.fullmatch(r"[1-9][0-9]{0,19}", generation) or int(generation) > 2**53 - 1:
        raise RuntimeError("Noncanonical source publisher/generation")
    return {"publisher_id": publisher, "library_generation": int(generation)}


class MacPeer:
    """One fresh Mac peer process, controlled only by frozen run-scoped messages."""
    def __init__(self, window, source, nonce, sender, deadline, manifest_sha256, mode="receive", mobile_platform="ios"):
        validate_mac_prepared(source["mac"])
        self.window, self.source, self.mode = window, source, mode
        self.output = window.output / ("mac-" + mode)
        request = dict(schema=2, mode=mode, nonce=nonce, deadline=deadline,
                       mobile_platform=mobile_platform, mobile_device=sender,
                       mobile_artifact_sha256=manifest_sha256,
                       **{key: source[key] for key in ("archive_id", "file_name", "title", "expected_text",
                                                     "source", "size", "sha256", "peer")})
        path = window.output / ("mac-" + mode + "-request.json")
        save(path, request)
        self.binding = dict(request, request_sha256=digest(path), prepared_sha256=source["mac"]["prepared_sha256"])
        self.log = (window.output / ("mac-" + mode + ".log")).open("xb")
        try:
            self.process = subprocess.Popen(["node", source["mac"]["tool"], "--prepared", source["mac"]["prepared"],
                "--prepared-sha256", source["mac"]["prepared_sha256"], "--request", str(path),
                "--request-sha256", self.binding["request_sha256"], "--output", str(self.output)],
                stdin=subprocess.PIPE, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        except BaseException:
            self.log.close()
            raise
        self.publication = None

    def receipt(self, name):
        path = self.output / name
        if not path.is_file():
            return None
        value = json.loads(path.read_bytes())
        if any(value.get(key) != expected for key, expected in self.binding.items()):
            raise RuntimeError("Mac receipt is not bound to frozen preparation/request/source/mobile identity")
        return value

    @staticmethod
    def validate_publication(value):
        if (not isinstance(value, dict) or str(uuid.UUID(value.get("publisher_id", ""))) != value["publisher_id"]
                or not re.fullmatch(r"[a-f0-9]{32}", value.get("session_id", ""))
                or type(value.get("library_generation")) is not int or value["library_generation"] <= 0):
            raise RuntimeError("Mac publication identity is missing or malformed")
        return value

    def wait_ready(self, observe=None):
        ending = time.monotonic() + min(45, self.window.remaining() - 15)
        while True:
            self.check_alive()
            ready = self.receipt("ready.json")
            if ready is not None:
                if ready.get("status") != "ready":
                    raise RuntimeError("Mac peer failed readiness")
                if self.mode == "publish":
                    self.publication = self.validate_publication(ready.get("publication"))
                self.readiness = ready
                return
            if observe is not None:
                observe()
            self.window.remaining()
            if time.monotonic() >= ending:
                raise TimeoutError("Mac peer did not become ready before paired UI action")
            time.sleep(.2)

    def check_alive(self):
        if self.process.poll() is not None:
            raise RuntimeError("Mac peer stopped before paired proof gates completed")

    def send(self, message):
        self.check_alive()
        self.process.stdin.write((json.dumps(dict(message, nonce=self.binding["nonce"])) + "\n").encode())
        self.process.stdin.flush()

    def receive(self, publication):
        self.publication = publication
        self.send(dict(command="receive", **publication))

    def result(self):
        if self.process.poll() is None:
            return None
        result = self.receipt("result.json")
        if (self.process.returncode or not result or result.get("status") != "complete"
                or result.get("cleanup_complete") is not True
                or result.get("preserved_before_books_settings_tabs") is not True
                or (self.mode != "observe" and (not isinstance(result.get("installed_identity"), dict) or not result["installed_identity"]))):
            raise RuntimeError("Mac result lacks real completion, installation, preservation or cleanup")
        publication = self.validate_publication(result.get("publication"))
        if not self.publication or any(publication.get(key) != value for key, value in self.publication.items()):
            raise RuntimeError("Mac result publication differs from source-bound publication")
        if self.mode == "observe":
            if (result.get("stopped") is not True or result.get("observed_source_stable") is not True
                    or result.get("paired_complete") is not True
                    or result.get("receivedhash") is not None or result.get("job") is not None
                    or not result.get("selected_peer") or not result.get("offer") or not result.get("inventory")):
                raise RuntimeError("Mac observer lacks read-only observation/withdrawal proof")
        elif self.mode == "publish":
            if result.get("publication_stopped") is not True or result.get("owned_sharing_stopped") is not True:
                raise RuntimeError("Mac publisher did not prove owned publication stop")
        elif (result.get("receivedhash") != self.source["sha256"] or result.get("receivedsize") != self.source["size"]
                or result.get("activebookid") != self.source["archive_id"] or result.get("managed") is not True
                or result.get("readable_ui") is not True or not (self.output / "reader.png").is_file()
                or not (self.output / "reader.txt").is_file()):
            raise RuntimeError("Mac result lacks managed full-hash/active-reader/browser proof")
        return result

    def stop(self, complete=False):
        try:
            if complete and self.mode in ("publish", "observe") and self.process.poll() is not None:
                raise RuntimeError("Mac publisher/observer stopped before its owned stop request")
            if self.process.poll() is None:
                self.send({"command": "stop", "complete": complete})
                try:
                    self.process.wait(timeout=min(45, self.window.remaining(cleanup=True)))
                except subprocess.TimeoutExpired:
                    os.killpg(self.process.pid, signal.SIGTERM)
                    self.process.wait(timeout=min(5, self.window.remaining(cleanup=True)))
            receipt = self.receipt("result.json")
            if not receipt or receipt.get("cleanup_complete") is not True:
                raise RuntimeError("Owned Mac peer cleanup is not confirmed")
            if complete:
                return self.result()
        finally:
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=3)
            self.process.stdin.close()
            self.log.close()

class AndroidReceiver:
    """One explicitly prepared receive or publish process, bound to this run."""
    def __init__(self, window, source, nonce, sender, deadline, mode="receive"):
        self.window, self.source = window, source
        if mode not in ("receive", "publish"):
            raise ValueError("Invalid paired Android mode")
        self.mode = mode
        android = source["android"]
        self.binding = dict(nonce=nonce, sender_device=sender, deadline=deadline,
                            device=android["device"], mode=mode, peer=source["peer"],
                            **{key: source[key] for key in ("archive_id", "file_name", "title", "size", "sha256")})
        self.output = window.output / ("android-" + mode)
        self.log = (window.output / ("android-" + mode + ".log")).open("xb")
        argv = [sys.executable, android["tool"], "--prepared", android["prepared"],
                "--prepared-sha256", android["prepared_sha256"], "--output", str(self.output)]
        for key, value in self.binding.items():
            argv += ["--" + key.replace("_", "-"), str(value)]
        self.started = time.monotonic()
        try:
            self.process = subprocess.Popen(argv, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        except BaseException:
            self.log.close()
            raise

    def ready(self):
        path = self.output / "ready.json"
        if self.process.poll() is not None:
            raise RuntimeError("Android peer stopped before its instrumentation-ready handshake")
        if not path.is_file():
            return False
        try:
            ready = json.loads(path.read_text())
        except json.JSONDecodeError:
            return False
        if ready.get("status") != "ready" or any(ready.get(key) != value for key, value in self.binding.items()):
            raise RuntimeError("Android instrumentation-ready receipt does not match this paired run")
        if self.mode == "publish":
            self.validate_publication(ready)
        elif (ready.get("reverse_hotspot_required") is not True or ready.get("reverse_hotspot_verified") is not False
                or ready.get("sta_disabled") is not True or ready.get("no_ap_at_instrumentation_ready") is not True):
            raise RuntimeError("Android readiness does not prove disconnected STA and no preexisting AP for native reverse joining")
        self.readiness = ready
        return True

    def wait_ready(self, observe=None):
        ready_end = time.monotonic() + min(45, self.window.remaining() - 15)
        while not self.ready():
            if observe is not None:
                observe()
            self.window.remaining()
            if time.monotonic() >= ready_end:
                raise TimeoutError("Android peer did not become ready before the paired UI action")
            time.sleep(.2)

    @staticmethod
    def validate_publication(receipt):
        radio = receipt.get("route_radio")
        if (any(receipt.get(key) is not True for key in (
                "publisher_ready", "sta_disabled", "no_ap_at_instrumentation_ready", "product_owned_ap_at_publish"))
                or not isinstance(radio, dict) or radio.get("current_ap_verified") is not True
                or not radio.get("ap_frequency_mhz")):
            raise RuntimeError("Android publisher lacks initial disconnected STA/no-AP and current app-owned AP proof")

    def check_alive(self):
        if self.process.poll() is not None:
            raise RuntimeError("Android publisher stopped before the iPhone bytes/read/preservation gates completed")

    def result(self):
        if self.process.poll() is None:
            return None
        path = self.output / "result.json"
        if self.process.returncode or not path.is_file():
            raise RuntimeError("Android peer stopped without successful completion; inspect its owned log")
        result = json.loads(path.read_text())
        if any(result.get(key) != value for key, value in self.binding.items()):
            raise RuntimeError("Android peer receipt is stale or belongs to a different source/device/run")
        if self.mode == "publish":
            self.validate_publication(result)
            if (result.get("status") != "complete" or result.get("publication_stopped") is not True
                    or result.get("owned_sharing_stopped") is not True
                    or result.get("cleanup_complete") is not True
                    or result.get("preserved_before_books_settings_tabs") is not True
                    or not isinstance(result.get("installed_identity"), dict) or not result["installed_identity"]
                    or not isinstance(result.get("final_radio"), dict)
                    or result["final_radio"].get("no_ap_verified") is not True):
                raise RuntimeError("Android publisher result lacks owned publication stop, preservation or cleanup")
            return result
        source = self.source
        if (result.get("status") != "complete" or result.get("receivedhash") != source["sha256"]
                or result.get("receivedsize") != source["size"] or result.get("activebookid") != source["archive_id"]
                or result.get("managed") is not True or result.get("readable_ui") is not True
                or not isinstance(result.get("installed_identity"), dict) or not result["installed_identity"]
                or not (self.output / "reader.png").is_file()
                or result.get("cleanup_complete") is not True):
            raise RuntimeError("Android result lacks independently verified managed bytes, readable UUID, installed identity or cleanup")
        if any(result.get(key) is not True for key in (
                "reverse_hotspot_required", "reverse_hotspot_verified", "sta_disabled",
                "no_ap_at_instrumentation_ready", "product_owned_ap_at_catalog")):
            raise RuntimeError("Android result does not prove the native automatic reverse hotspot route")
        return result

    def stop(self, complete=False):
        try:
            stopped_before_request = complete and self.process.poll() is not None
            if self.process.poll() is None:
                self.process.send_signal(signal.SIGTERM)
                try:
                    self.process.wait(timeout=min(15, self.window.remaining(cleanup=True)))
                except subprocess.TimeoutExpired:
                    pass
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.process.wait(timeout=min(3, self.window.remaining(cleanup=True)))
            path = self.output / "result.json"
            receipt = json.loads(path.read_text()) if path.is_file() else {}
            if (receipt.get("cleanup_complete") is not True
                    or any(receipt.get(key) != value for key, value in self.binding.items())):
                raise RuntimeError("Owned Android instrumentation cleanup is not confirmed by its bound receipt")
            if stopped_before_request:
                raise RuntimeError("Android publisher stopped before its owned stop request")
            if complete:
                return self.result()
        finally:
            self.log.close()



def measured_checkpoint(window, phase, nonce, bundle, executable, competing, pmd,
                        log, deadline, runner_bundle, semantic_check):
    window.stage = "checkpoint:" + phase
    foreground = foreground_observer(window, log, phase, nonce, bundle)
    slug = phase.replace("/", "-")
    window.ctl(slug + "-before-screen", ["capture", "screenshot", "--destination",
        str(window.output / (slug + "-before.png"))], 5)
    with TIMING.span("ui_wait", "foreground-settle") if TIMING else nullcontext():
        settle_end = time.monotonic() + 30
        while time.monotonic() < settle_end:
            foreground()
            time.sleep(max(0, min(1, settle_end - time.monotonic())))
    semantic_before = semantic_check(slug + "-before", phase)
    foreground()
    root = window.memory_root(slug + "-root", pmd, executable)[0]
    root_identity = [root[key] for key in ("pid", "startAbsTime", "coalitionID")]
    foreground()
    classified = []
    times = []
    for index in range(3):
        window.stage = "memory:" + phase
        foreground()
        start = time.time()
        raw = window.filtered_processes(slug + f"-sample-{index + 1}", pmd,
                                        "coalitionID", root["coalitionID"])
        sample = snapshot_footprint(raw, executable, competing_executables=competing)
        if sample["main_identity"] != root_identity:
            raise RuntimeError("Reader root generation changed after the ownership probe at " + phase)
        classified.append(sample)
        times.append({"started": start, "finished": time.time()})
        save(window.output / (slug + f"-classified-{index + 1}.json"), sample)
        foreground()
        if index < 2:
            time.sleep(min(1, window.remaining()))
    result = checkpoint_footprint(classified)
    result.update(phase=phase, settle_seconds=30, target_spacing_seconds=1, capture_times=times,
                  semantic_before=semantic_before, content_admission="Actual screenshots require visual content admission",
                  archive_byte_scope="Fixture source hashes and admitted UUID/size; existing on-device archives are not reread byte-for-byte")
    result["capture_scope"] = "All observed processes in the probed kernel coalition; not a global process-uniqueness audit"
    result["root_probe_identity"] = root_identity
    window.ctl(slug + "-after-screen", ["capture", "screenshot", "--destination",
        str(window.output / (slug + "-after.png"))], 5)
    result["semantic_after"] = semantic_check(slug + "-after", phase)
    save(window.output / (slug + "-checkpoint.json"), result)
    # Child churn or unsupported cohort accounting is an honest unavailable
    # measurement, not an excuse to lose the other product's real observations.
    if result["main_identity"] is None or result["reason"] == "main-generation-changed-or-unavailable":
        raise RuntimeError("Main process identity changed or is unavailable at " + phase)
    foreground()
    acknowledge(window, phase, nonce, bundle, deadline, runner_bundle)
    return result


class Window:
    def __init__(self, output, device, seconds):
        self.output = output
        self.device = device
        self.start = time.monotonic()
        self.seconds = seconds
        self.child = None
        self.runner_installed = False
        self.commands = []
        self.stage = "preflight"
        self.publisher_peer = None
        self.completed_steps = []
        self.owner_deadline = time.time() + seconds
        self.receiver_control = None
        self.action_deadline = None
        self.cleanup_deadline = None
        self.retirement_cleanup_receipt = None

    def remaining(self, cleanup=False):
        if not cleanup and self.receiver_control is not None:
            self.receiver_control.poll()
        if not cleanup and self.publisher_peer is not None:
            self.publisher_peer.check_alive()
        left = (self.seconds - (5 if cleanup else 60)) - (time.monotonic() - self.start)
        if not cleanup and self.action_deadline is not None:
            left = min(left, self.action_deadline - time.time())
        if cleanup and self.cleanup_deadline is not None:
            left = min(left, self.cleanup_deadline - time.time())
        if left <= 0:
            raise TimeoutError("Phone window deadline; stop and release, never debug/retry")
        return left

    def ctl(self, label, args, timeout=15, cleanup=False):
        self.stage = label
        receipt = self.output / (label + ".json")
        # Launch's positional bundle begins variadic app arguments. Keep device
        # and output options before every leaf command's positional arguments.
        argv = ["xcrun", "devicectl", "device", *args[:2], "--device", self.device,
                "--timeout", str(min(timeout, self.remaining(cleanup))), "--json-output", str(receipt),
                *args[2:]]
        started = time.monotonic()
        command(argv, min(timeout + 1, self.remaining(cleanup)), self.output / (label + ".log"))
        data = json.loads(receipt.read_text())
        if data.get("info", {}).get("outcome") != "success":
            raise RuntimeError("Unexpected CoreDevice outcome: " + label)
        self.commands.append({"label": label, "duration_seconds": time.monotonic() - started})
        if label in {"install-current", "install-runner", "install-baseline"}:
            self.completed_steps.append(label)
        return data["result"]

    def native_state(self, label, bundle):
        listing = self.ctl(label + "-files", ["info", "files", "--domain-type", "appDataContainer",
            "--domain-identifier", bundle, "--subdirectory", "Library"], 10)
        files = {row["relativePath"]: row for row in listing.get("files", [])}
        copied = {}
        for name in ("Application Support/DataModel.sqlite", "Application Support/DataModel.sqlite-wal",
                     "Application Support/DataModel.sqlite-shm", "Preferences/" + bundle + ".plist"):
            if name not in files:
                continue
            metadata = files[name]
            if metadata.get("resources", {}).get("isSymbolicLink") or metadata.get("metadata", {}).get("size", 0) > 64 * 1024 * 1024:
                raise RuntimeError("Native preflight file is not a bounded regular capture")
            target = self.output / (label + "-" + Path(name).name)
            self.ctl(label + "-copy-" + Path(name).name, ["copy", "from", "--domain-type", "appDataContainer",
                "--domain-identifier", bundle, "--source", "Library/" + name, "--destination", str(target)], 15)
            copied[name] = target
        preference_path = copied.get("Preferences/" + bundle + ".plist")
        preferences = plistlib.loads(preference_path.read_bytes()) if preference_path else {}
        database = copied.get("Application Support/DataModel.sqlite")
        state = (read_native_database(database, preferences) if database else
                 {"initialized": False, "books": [], "tabs": [], "bookmarks": [], "preferences": preferences})
        save(self.output / (label + "-inventory.json"), state)
        return state

    def initialized_state(self, bundle):
        # Only a successful inventory can establish absence. Never interpret
        # a failed state copy (transport, authorization or otherwise) as fresh.
        relative = STATE.format(bundle=bundle).removeprefix("Library/")

        def persisted(label):
            listing = self.ctl(label, ["info", "files", "--domain-type", "appDataContainer",
                "--domain-identifier", bundle, "--subdirectory", "Library"], 10)
            files = listing.get("files")
            if not isinstance(files, list) or any(
                    not isinstance(row, dict) or not isinstance(row.get("relativePath"), str)
                    for row in files):
                raise RuntimeError("Unexpected application file inventory")
            return any(row["relativePath"] == relative for row in files)

        if not persisted("before-state-files"):
            # Use ordinary startup to create real state, not a synthetic empty
            # preservation baseline. Existing profiles are captured unlaunched.
            self.ctl("initialize-current", ["process", "launch", bundle], 10)
            deadline = time.monotonic() + min(15, self.remaining())
            attempt = 0
            while not persisted("initialized-state-files-" + str(attempt)):
                self.remaining()
                if time.monotonic() >= deadline:
                    raise RuntimeError("App did not persist its ordinary initial state")
                time.sleep(0.1)
                attempt += 1
        return self.state("before", bundle)

    def state(self, label, bundle, cleanup=False):
        destination = self.output / (label + "-state.json")
        self.ctl(label, ["copy", "from", "--domain-type", "appDataContainer",
            "--domain-identifier", bundle, "--source", STATE.format(bundle=bundle),
            "--destination", str(destination)], cleanup=cleanup)
        value = json.loads(destination.read_text())
        if not isinstance(value.get("books"), list) or not isinstance(value.get("tabs"), list):
            raise RuntimeError("Unexpected application state format")
        return value

    def filtered_processes(self, label, pmd, key, value):
        self.stage = label
        memory = self.output / (label + ".json")
        # PMD applies this filter BEFORE its sequential executable-name RPCs.
        # --mobdev2 does not select Wi-Fi when --userspace takes precedence.
        command([pmd, "developer", "dvt", "sysmon", "process", "single", "--userspace",
                 "--udid", self.device, "--filter", f"{key}={value}", "--output", memory],
                min(12, self.remaining()), self.output / (label + ".log"))
        processes = json.loads(memory.read_text())
        if (not isinstance(processes, list) or not processes
                or any(not isinstance(row, dict) or type(row.get(key)) is not int
                       or row[key] != value for row in processes)):
            raise RuntimeError("DVT snapshot is empty or violates its ownership filter: " + label)
        return processes

    def memory_root(self, label, pmd, executable):
        processes = self.ctl(label + "-processes", ["info", "processes"], 8).get("runningProcesses", [])
        roots = [row for row in processes
                 if unquote(urlparse(row.get("executable", "")).path) == executable]
        if len(roots) != 1 or not natural(roots[0].get("processIdentifier"), True):
            raise RuntimeError("Exact installed executable does not identify one running process")
        rows = self.filtered_processes(label, pmd, "pid", roots[0]["processIdentifier"])
        if (len(rows) != 1 or rows[0].get("execName") != executable
                or any(not natural(rows[0].get(key), True) for key in ("startAbsTime", "coalitionID"))
                or not natural(rows[0].get("physFootprint"))):
            raise RuntimeError("DVT reader-root probe lacks exact identity, generation, coalition or footprint")
        return rows

    def sampler_readiness(self, pmd):
        rows = self.filtered_processes("sampler-readiness", pmd, "pid", 1)
        if (len(rows) != 1 or rows[0].get("execName") != "/sbin/launchd"
                or not natural(rows[0].get("startAbsTime"), True)
                or not natural(rows[0].get("coalitionID"))
                or not natural(rows[0].get("physFootprint"))):
            raise RuntimeError("DVT sampler readiness lacks the expected launchd identity or metric schema")
        result = {"status": "ready", "scope": "Transport/schema probe only; not a reader-memory sample",
                  "transport": "PMD userspace tunnel: USB-first with Bonjour fallback, not forced Wi-Fi"}
        save(self.output / "sampler-readiness-scope.json", result)
        return result

    def snapshot(self, label, pmd, executable):
        self.ctl(label + "-screenshot", ["capture", "screenshot", "--destination",
            str(self.output / (label + ".png"))], 8)
        processes = self.memory_root(label + "-dvt-processes", pmd, executable)
        main = [process for process in processes if process.get("execName") == executable]
        save(self.output / (label + "-memory-scope.json"), {
            "scope": "One DVT process snapshot; exact executable-path matches only",
            "phase": label,
            "phase_scope": "Sample requested at this UI marker; UI is not suspended. Correlate screenshot/process timestamps; not a settled-memory benchmark.",
            "executable": executable,
            "main_process_observed": [{key: process.get(key) for key in
                ["pid", "startAbsTime", "timestamp", "coalitionID", "responsiblePID",
                 "physFootprint", "memCompressed", "memResidentSize", "cpuUsage"]} for process in main],
            "full_webkit_cohort": False, "physical_memory_sum": None,
            "warning": "Empty main-process match is unavailable attribution, never zero memory. Raw observed processes retained."})

    def retire_owned_publisher(self, executable):
        """A killed XCTest cannot run Swift defer; retire only this exact app."""
        def publisher_processes(label):
            data = self.ctl(label, ["info", "processes"], 5, cleanup=True)
            processes = data.get("runningProcesses")
            if not isinstance(processes, list) or any(
                    not isinstance(row, dict) or not isinstance(row.get("executable"), str)
                    or type(row.get("processIdentifier")) is not int for row in processes):
                raise RuntimeError("Cannot confirm owned publisher process absence")
            return [row for row in processes
                    if unquote(urlparse(row["executable"]).path) == executable]

        running = publisher_processes("cleanup-publisher-processes")
        if len(running) > 1:
            raise RuntimeError("Ambiguous exact publisher executable; cleanup not confirmed")
        if running:
            self.ctl("cleanup-publisher-terminate", ["process", "terminate", "--pid",
                str(running[0]["processIdentifier"])], 5, cleanup=True)
            if publisher_processes("cleanup-publisher-confirm"):
                raise RuntimeError("Owned publisher app is still running after exact-PID termination")
        return {"publisher_retired": True, "publisher_retirement": "verified-exact-app-process-absence"}

    def stop(self):
        if self.child is not None:
            try:
                os.killpg(self.child.pid, signal.SIGINT)
            except ProcessLookupError:
                pass
            try:
                self.child.wait(timeout=max(0.1, min(5, self.seconds - 5 - (time.monotonic() - self.start))))
            except subprocess.TimeoutExpired:
                os.killpg(self.child.pid, signal.SIGKILL)
                self.child.wait(timeout=2)
            # The leader can exit before its owned descendants. Stop that exact
            # session group too; never use a global process-name kill.
            try:
                os.killpg(self.child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if self.runner_installed:
            def owned_runners(data):
                processes = data.get("runningProcesses")
                if not isinstance(processes, list) or any(
                        not isinstance(row, dict) or not isinstance(row.get("executable"), str)
                        or not isinstance(row.get("processIdentifier"), int) for row in processes):
                    raise RuntimeError("Cannot establish runner absence from process receipt")
                return [row for row in processes if row["executable"].endswith(
                    "/KiwixPhoneWindowUI-Runner.app/KiwixPhoneWindowUI-Runner")]
            running = owned_runners(self.ctl("cleanup-processes", ["info", "processes"], 8, cleanup=True))
            if len(running) > 1:
                raise RuntimeError("Ambiguous owned-runner processes; cleanup not confirmed")
            for process in running:
                self.ctl("cleanup-runner", ["process", "terminate", "--pid",
                    str(process["processIdentifier"])], 8, cleanup=True)
            if running and owned_runners(self.ctl("cleanup-confirm", ["info", "processes"], 8, cleanup=True)):
                raise RuntimeError("Owned UI runner still present after termination")
        return {"cleanup_complete": True, "host_processes_stopped": True, "remote_runner_stopped": True}


class SimulatorWindow(Window):
    """The explicit owned simulator only; never CoreDevice or PMD."""
    def sim(self, label, arguments, timeout=15, cleanup=False):
        self.stage = label
        started = time.monotonic()
        try:
            return command(["xcrun", "simctl", *arguments],
                           min(timeout, self.remaining(cleanup)), self.output / (label + ".log"))
        finally:
            self.commands.append({"label": label, "duration_seconds": time.monotonic() - started})

    def app_container(self, bundle, kind="data", cleanup=False):
        raw = self.sim("container-" + bundle + "-" + kind,
                       ["get_app_container", self.device, bundle, kind], cleanup=cleanup).decode().strip()
        path = Path(raw)
        boundary = Path.home() / "Library/Developer/CoreSimulator/Devices" / self.device / "data/Containers"
        if (not path.is_absolute() or path.is_symlink() or path.resolve(strict=True) != path
                or not path.is_relative_to(boundary)):
            raise RuntimeError("Simulator container escapes the specified owned simulator")
        return path

    def container_file(self, bundle, relative):
        if Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise RuntimeError("Invalid simulator container-relative path")
        root = self.app_container(bundle)
        target = root / relative
        current = root
        for part in Path(relative).parts:
            current = current / part
            if current.is_symlink():
                raise RuntimeError("Refusing symlink in simulator data path")
        return target

    def ctl(self, label, args, timeout=15, cleanup=False):
        self.stage = label
        args = list(args)
        def option(name):
            return args[args.index(name) + 1]
        if args[:2] == ["info", "details"]:
            listing = json.loads(self.sim(label, ["list", "devices", "--json"], timeout))
            matches = [row for rows in listing["devices"].values() for row in rows
                       if row.get("udid") == self.device]
            if len(matches) != 1 or matches[0].get("state") != "Booted" or not matches[0].get("isAvailable"):
                raise RuntimeError("Specified owned simulator must already be available and booted")
            name = matches[0].get("name")
            if not isinstance(name, str) or not name.startswith("KiwixProfilingValidation-"):
                raise RuntimeError("Refusing non-owned simulator; expected KiwixProfilingValidation- name prefix")
            return {"hardwareProperties": {"udid": self.device, "reality": "simulator"}}
        if args[:2] == ["info", "apps"]:
            bundle = option("--bundle-id")
            raw = self.sim(label, ["listapps", self.device], timeout)
            # simctl emits OpenStep plist, which plistlib cannot decode directly.
            xml = command(["/usr/bin/plutil", "-convert", "xml1", "-o", "-", "--", "-"],
                          min(timeout, self.remaining(cleanup)),
                          self.output / (label + "-plist.log"), input_data=raw)
            apps = plistlib.loads(xml)
            if bundle not in apps:
                return {"apps": []}
            data = self.app_container(bundle)
            app = self.app_container(bundle, "app")
            return {"apps": [{"bundleIdentifier": bundle, "dataContainerPath": str(data),
                              "url": app.as_uri()}]}
        if args[:2] == ["install", "app"]:
            self.sim(label, ["install", self.device, args[2]], timeout)
            self.completed_steps.append(label)
            return {}
        if args[:2] == ["info", "files"]:
            bundle = option("--domain-identifier")
            rows = []
            for name in ("Application Support/DataModel.sqlite", "Application Support/DataModel.sqlite-wal",
                         "Application Support/DataModel.sqlite-shm", "Preferences/" + bundle + ".plist"):
                path = self.container_file(bundle, "Library/" + name)
                if path.exists():
                    rows.append({"relativePath": name, "resources": {"isSymbolicLink": False},
                                 "metadata": {"size": path.stat().st_size}})
            return {"files": rows}
        if args[0] == "copy":
            bundle = option("--domain-identifier")
            source, destination = Path(option("--source")), Path(option("--destination"))
            if args[1] == "from":
                source = self.container_file(bundle, str(source))
            elif args[1] == "to":
                destination = self.container_file(bundle, str(destination))
                destination.parent.mkdir(parents=True, exist_ok=True)
                if destination.exists():
                    raise RuntimeError("Refusing to replace simulator staging/ACK file")
            else:
                raise RuntimeError("Unexpected simulator copy direction")
            if source.is_symlink() or not source.is_file():
                raise RuntimeError("Simulator capture/staging source is not a regular file")
            if source.stat().st_size > 512 * 1024 * 1024:
                raise RuntimeError("Simulator copy exceeds real fixture/capture bound")
            self.remaining(cleanup)
            command(["/bin/cp", "-n", source, destination],
                    min(timeout, self.remaining(cleanup)), self.output / (label + ".log"))
            self.remaining(cleanup)
            save(self.output / (label + ".json"), {"platform": "simulator", "source": str(source),
                 "destination": str(destination), "size": destination.stat().st_size})
            return {}
        if args[:2] == ["capture", "screenshot"]:
            self.sim(label, ["io", self.device, "screenshot", option("--destination")], timeout)
            return {}
        raise RuntimeError("Unsupported simulator operation: " + repr(args))

    def initialized_state(self, bundle):
        path = self.container_file(bundle, STATE.format(bundle=bundle))
        if not path.exists():
            self.sim("initialize-current", ["launch", self.device, bundle], 10)
            deadline = time.monotonic() + min(15, self.remaining())
            while not path.exists():
                self.remaining()
                if time.monotonic() >= deadline:
                    raise RuntimeError("App did not persist its ordinary initial state")
                time.sleep(0.1)
        return self.state("before", bundle)

    def runner_processes(self):
        output = self.sim("cleanup-runner-list-" + uuid.uuid4().hex,
                          ["spawn", self.device, "launchctl", "list"], 8, cleanup=True).decode()
        return [line for line in output.splitlines()
                if len(line.split()) == 3 and line.split()[0].isdigit()
                and line.split()[2].startswith("UIKitApplication:" + RUNNER_ID + "[")]

    def stop(self):
        installed = self.runner_installed
        self.runner_installed = False
        result = super().stop()
        if installed and self.runner_processes():
            self.sim("cleanup-runner", ["terminate", self.device, RUNNER_ID], 8, cleanup=True)
            if self.runner_processes():
                raise RuntimeError("Owned simulator XCTest runner still running")
        result.update(platform="simulator", simulator_runner_stopped=True,
                      physical_device_contact=False)
        return result


def simulator_checkpoint(window, phase, nonce, bundle, log, deadline, runner_bundle, semantic_check):
    window.stage = "ui-checkpoint:" + phase
    foreground = foreground_observer(window, log, phase, nonce, bundle)
    slug = phase.replace("/", "-")
    foreground()
    window.ctl(slug + "-before-screen", ["capture", "screenshot", "--destination",
               str(window.output / (slug + "-before.png"))], 5)
    semantic = semantic_check(slug + "-before", phase)
    foreground()
    window.ctl(slug + "-after-screen", ["capture", "screenshot", "--destination",
               str(window.output / (slug + "-after.png"))], 5)
    result = {"platform": "simulator", "phase": phase, "status": "ui_state_observed",
              "semantic_before": semantic, "semantic_after": semantic_check(slug + "-after", phase),
              "physical_memory": "not-applicable", "physical_memory_tested": False,
              "sampler_transport": "not-tested", "content_admission": "pending human screenshot admission"}
    save(window.output / (slug + "-checkpoint.json"), result)
    foreground()
    acknowledge(window, phase, nonce, bundle, deadline, runner_bundle)
    return result


def container_path(apps, bundle):
    if len(apps) != 1 or apps[0].get("bundleIdentifier") != bundle:
        raise RuntimeError("Application container identity is ambiguous")
    value = apps[0].get("dataContainerPath")
    if not isinstance(value, str) or not value or "\0" in value:
        raise RuntimeError("Missing verified application data container")
    path = Path(value)
    if not path.is_absolute() or str(path) != value or ".." in path.parts:
        raise RuntimeError("Non-normalized application data container")
    if path.parts[-3:-1] != ("Data", "Application"):
        raise RuntimeError("Unexpected iOS application data container boundary")
    uuid.UUID(path.name)
    return value


def fixture_tab(state, book_id):
    candidates = [tab for tab in state["tabs"] if tab.get("bookId") == book_id]
    if not candidates:
        return None
    for tab in candidates:
        title = tab.get("title")
        if (isinstance(title, str) and title and isinstance(tab.get("id"), str)
                and sum(other.get("title") == title for other in state["tabs"]) == 1):
            return {"id": tab["id"], "book_id": book_id, "title": title}
    raise RuntimeError("Fixture tab titles are ambiguous; cannot select an exact existing tab through ordinary UI")


def preservation(before, after, old_container, new_container, bundle, mutable_fixture_tabs=None,
                 expected_sharing_name=None):
    # Mirror profile_paths.rs State's filesystem inventory only. Article paths,
    # provider references, installation identity and unrelated settings stay exact.
    roots = [container + "/Library/Application Support/" + bundle
             for container in (old_container, new_container)]

    def filesystem_path(value):
        if not isinstance(value, str) or not value.startswith("/"):
            return value
        path = Path(value)
        if str(path) != value or ".." in path.parts or "\0" in value:
            return value
        for root in roots:
            if value == root or value.startswith(root + "/"):
                return ("verified-app-profile", value[len(root):])
        return value

    def book_path(book):
        if book.get("managed") is True and book.get("sourceReference") is None:
            return filesystem_path(book.get("path"))
        return book.get("path")

    def settings(value):
        result = dict(value)
        for field in ["downloadDirectory", "lastImportDirectory", "lastSaveDirectory"]:
            if field in result:
                result[field] = filesystem_path(result[field])
        if isinstance(result.get("monitorDirectories"), list):
            result["monitorDirectories"] = [filesystem_path(item) for item in result["monitorDirectories"]]
        # Older profiles omit the default empty pin list; writing it on launch
        # is schema normalization, not a settings change.
        if result.get("pinnedBookIds") == []:
            result.pop("pinnedBookIds", None)
        return result

    old = {book["id"]: book for book in before["books"]}
    new = {book["id"]: book for book in after["books"]}
    for key, book in old.items():
        if key not in new:
            raise RuntimeError("Preexisting archive disappeared")
        for field in ["id", "installationId", "sourceReference", "managed", "size"]:
            if book.get(field) != new[key].get(field):
                raise RuntimeError("Preexisting archive authority changed: " + field)
        if book_path(book) != book_path(new[key]):
            raise RuntimeError("Preexisting archive path changed outside verified profile relocation")
    left_settings, right_settings = settings(before["settings"]), settings(after["settings"])
    if expected_sharing_name is not None:
        # Only the sender that the harness itself renamed may differ, and only
        # to the exact generated value for this run.
        if right_settings.get("sharingName") != expected_sharing_name:
            raise RuntimeError("Generated sharing name is not the retained sender name")
        left_settings.pop("sharingName", None)
        right_settings.pop("sharingName", None)
    if left_settings != right_settings or before.get("bookmarks") != after.get("bookmarks"):
        raise RuntimeError("Settings or bookmarks changed outside verified profile relocation")

    def tab_state(value):
        # Older profiles omit the default session policy; writing it on launch
        # is schema normalization, not a reader-state change.
        result = dict(value)
        if result.get("restricted") is None:
            result["restricted"] = False
        return result

    tabs = {tab["id"]: tab for tab in after["tabs"]}
    for tab in before["tabs"]:
        if mutable_fixture_tabs and tab["id"] in mutable_fixture_tabs:
            current = tabs.get(tab["id"])
            if current is None or current.get("bookId") != mutable_fixture_tabs[tab["id"]]:
                raise RuntimeError("Reused fixture tab identity changed or disappeared")
        elif tabs.get(tab["id"]) is None or tab_state(tabs[tab["id"]]) != tab_state(tab):
            raise RuntimeError("Unrelated preexisting reader tab changed")


def preserve_download_authority(before, after):
    """Progress may advance naturally; cleanup cannot replace unrelated jobs."""
    for job_id, old in before.items():
        current = after.get(job_id)
        if not isinstance(old, dict) or not isinstance(current, dict):
            raise RuntimeError("Preexisting download disappeared during receiver cleanup")
        for field in ("id", "url", "fileName", "destination", "transport"):
            if old.get(field) != current.get(field):
                raise RuntimeError("Unrelated download authority changed during receiver cleanup")
        if old.get("publication") is not None and old["publication"] != current.get("publication"):
            raise RuntimeError("Unrelated download publication changed during receiver cleanup")


def receiver_owned_job(jobs, baseline, source, observation):
    expected_url = observation["offer"]["downloadIdentity"]
    candidates = [job for job in jobs.values() if isinstance(job, dict) and (
        job.get("fileName") == source["file_name"] or job.get("url") == expected_url)]
    if not candidates:
        return None
    if len(candidates) != 1:
        raise RuntimeError("Receiver cleanup cannot select an ambiguous fixture job")
    job = candidates[0]
    if (job.get("id") in baseline or jobs.get(job.get("id")) != job
            or job.get("url") != expected_url or job.get("fileName") != source["file_name"]
            or job.get("transport") != "quic-library"
            or str(uuid.UUID(job.get("id", ""))) != job["id"]):
        raise RuntimeError("Receiver cleanup job lacks new run-owned observed-offer authority")
    return job


def receiver_retirement_proof(baseline, admitted, after_jobs, source, observation, state, nonce, ui_output,
                              possible_admission):
    lines = ui_output.decode(errors="replace").splitlines()
    if lines.count("KIWIX_RECEIVER_RETIRED " + nonce) != 1:
        raise RuntimeError("Receiver discovery retirement lacks one exact UI receipt")
    preserve_download_authority(baseline, after_jobs)
    job = receiver_owned_job(after_jobs, baseline, source, observation)
    if admitted is not None:
        preserve_download_authority({admitted["id"]: admitted}, after_jobs)
    if admitted is None:
        if job is not None or possible_admission:
            raise RuntimeError("Receiver admission was in flight; absent journal data cannot prove retirement")
    elif job is None or job["id"] != admitted["id"]:
        raise RuntimeError("Receiver cleanup lost or replaced the exact authorized job")
    elif job.get("status") == "cancelled":
        if lines.count("KIWIX_RECEIVER_CANCELLED " + nonce) != 1:
            raise RuntimeError("Cancelled status alone does not prove owned worker cancellation/drain")
    elif job.get("status") in ("complete", "completed"):
        authority = dict(observation["publication"], offer_id=observation["offer"]["id"])
        mac_received_job(after_jobs, baseline, source, authority, state)
    else:
        raise RuntimeError("Receiver worker retirement is not independently established")
    return {"receiver_work_retired": True, "receiver_discovery_stopped": True,
            "receiver_owned_job_retired": True, "receiver_retired_job": job,
            "receiver_retirement_scope": "Ordinary Stop finding and exact new observed-offer job only; no Stop all, application kill or archive deletion."}


def retire_iphone_receiver(window, app, runner, source, controller, jobs_before, before, old_container, new_container,
                          installed_identity):
    """Run the prepared ordinary cleanup UI, then independently inspect its effect."""
    observation = controller.observation
    admitted_jobs = nearby_jobs(window, "receiver-retire-before", app["bundle_id"], cleanup=True)
    preserve_download_authority(jobs_before, admitted_jobs)
    admitted = receiver_owned_job(admitted_jobs, jobs_before, source, observation)
    source_plan = Path(runner["xctestrun"])
    plan = plistlib.loads(source_plan.read_bytes())
    targets = ([target for config in plan["TestConfigurations"] for target in config.get("TestTargets", [])]
               if "TestConfigurations" in plan else
               [target for key, target in plan.items() if key != "__xctestrun_metadata__" and isinstance(target, dict)])
    if len(targets) != 1 or targets[0].get("BlueprintName") != "KiwixPhoneWindowUI":
        raise RuntimeError("Unexpected receiver retirement XCTest targets")
    nonce = controller.request["nonce"]
    budget = min(35, window.remaining(cleanup=True) - RECEIVER_FINAL_RUNNER_SECONDS)
    if budget <= 0:
        raise TimeoutError("No bounded receiver retirement interval remains")
    targets[0]["EnvironmentVariables"] = {
        "KIWIX_DEVICE": window.device, "KIWIX_PHONE_WINDOW": "START-" + window.device,
        "KIWIX_APP_ID": app["bundle_id"], "KIWIX_PLATFORM": "device", "KIWIX_MODE": "nearby",
        "KIWIX_DEADLINE": str(int(time.time() + budget)), "KIWIX_CHECKPOINT_NONCE": nonce,
        "KIWIX_NEARBY_RETIRE": "1", "KIWIX_NEARBY_TOPOLOGY": "apple-lan",
        "KIWIX_NEARBY_FILE_NAME": source["file_name"], "KIWIX_NEARBY_PEER": source["peer"],
        "KIWIX_NEARBY_TITLE": source["title"], "KIWIX_NEARBY_RETIRE_JOB_ID": admitted["id"] if admitted else ""}
    authorized = source_plan.parent / ("Receiver-retire-" + uuid.uuid4().hex + ".xctestrun")
    try:
        with authorized.open("xb") as stream:
            plistlib.dump(plan, stream)
        raw = command(["xcodebuild", "test-without-building", "-xctestrun", str(authorized),
            "-destination", "platform=iOS,id=" + window.device, "-destination-timeout", "10",
            "-parallel-testing-enabled", "NO", "-maximum-concurrent-test-device-destinations", "1",
            "-only-testing:KiwixPhoneWindowUI/PhoneWindowUI/testNearbyRetire",
            "-resultBundlePath", str(window.output / "receiver-retire.xcresult")],
            budget, window.output / "receiver-retire.log")
    finally:
        try:
            authorized.unlink(missing_ok=True)
        finally:
            # command() owns a distinct host session. Reap it (including timeout
            # handling), then confirm the exact remote runner before spending
            # the reserved interval on journal/state evidence.
            window.retirement_cleanup_receipt = window.stop()
    installed = window.ctl("receiver-retired-app", ["info", "apps", "--bundle-id", app["bundle_id"],
        "--include-container-paths"], 8, cleanup=True).get("apps")
    if installed != installed_identity["device_receipt"]:
        raise RuntimeError("Receiver application/container identity changed during scoped retirement")
    after_jobs = nearby_jobs(window, "receiver-retire-after", app["bundle_id"], cleanup=True)
    preserve_download_authority(admitted_jobs, after_jobs)
    state = window.state("receiver-retired", app["bundle_id"], cleanup=True)
    preservation(before, state, old_container, new_container, app["bundle_id"])
    original_log = (window.output / "xctest.log").read_text(errors="replace")
    possible_admission = any(line.startswith("KIWIX_WINDOW nearby-catalog ") for line in original_log.splitlines())
    return receiver_retirement_proof(jobs_before, admitted, after_jobs, source, observation, state,
                                     nonce, raw, possible_admission)


def run(args):
    authorized_start = time.monotonic()
    # Admit the new transitive module before importing it, including direct peer runs.
    runner = json.loads(args.runner_manifest.read_text())
    if runner.get("source_sha256") != sources():
        raise RuntimeError("Runner source changed; rebuild/review offline before a window")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    previous_iteration = os.environ.get("KIWIX_TIMING_ITERATION")
    start_timing(output, "device_automation", runner["source_sha256"]["testing_timing.py"])
    os.environ["KIWIX_TIMING_ITERATION"] = TIMING.iteration_id
    try:
        with TIMING.span("device_automation", "iphone-window"):
            return run_instrumented(args, authorized_start)
    finally:
        TIMING.close()
        if previous_iteration is None:
            os.environ.pop("KIWIX_TIMING_ITERATION", None)
        else:
            os.environ["KIWIX_TIMING_ITERATION"] = previous_iteration


def run_instrumented(args, authorized_start):
    global CLEANING_UP
    with TIMING.span("host_prepare", "validate-window"):
        app, runner, pmd, map_source, baseline, wiki_source = validate(args)
    nearby = args.nearby_json if args.test_mode == "nearby" else None
    launch_only = args.test_mode == "launch"
    wiki_workload = not nearby and not launch_only
    receiver_control = getattr(args, "_iphone_receiver", None)
    iphone_pair = bool(nearby and nearby.get("direction") == "iphone-to-iphone")
    apple = bool(nearby and nearby.get("direction") in ("mac-to-iphone", "iphone-to-mac", "iphone-to-iphone"))
    peer_label = "iphone" if iphone_pair else "mac" if apple else "android"
    sending = bool(nearby and not receiver_control and nearby.get("direction") in ("iphone-to-android", "iphone-to-mac", "iphone-to-iphone"))
    verifying = bool(nearby and nearby.get("operation", "transfer") == "verify-received")
    paired_forward = bool(nearby and not sending and not verifying and not receiver_control)
    map_name = "kiwix-window-" + uuid.uuid4().hex + "-hawaii.zim" if map_source else None
    output = args.output.resolve()
    os.umask(0o077)
    simulator = args.platform == "simulator"
    window = (SimulatorWindow if simulator else Window)(output, args.device, args.window_seconds)
    window.start = authorized_start
    window.owner_deadline = time.time() + args.window_seconds - (time.monotonic() - authorized_start)
    if receiver_control:
        window.owner_deadline = receiver_control.request["owner_deadline"]
        window.seconds = window.owner_deadline - time.time() + (time.monotonic() - window.start)
    if iphone_pair:
        # Failed peer cleanup gets its remote interval before source teardown.
        window.action_deadline = receiver_control.request["deadline"] if receiver_control else window.owner_deadline - RECEIVER_TOTAL_RESERVE_SECONDS
        if receiver_control:
            window.cleanup_deadline = window.owner_deadline - RECEIVER_OWNER_RESERVE_SECONDS
    status = {"status": "failed", "platform": args.platform, "device": args.device, "source_commit": app["source_commit"],
              "app_manifest_sha256": digest(args.app_manifest), "runner_manifest_sha256": digest(args.runner_manifest),
              "test_mode": args.test_mode, "order": args.order if baseline else None,
              "completed_steps": [], "memory_checkpoints": {},
              "requested_workload": {"wiki_uuid": WIKI_ID, "map_uuid": MAP_ID if map_source else None},
              "retained_owned_data": "Installed runner, fixture tabs, any admitted map and any unique staging source remain; no archive deletion/reset.",
              "limitations": ["XCTest timings are instrumented end-to-end observation upper bounds, not calibrated cold-launch/search latency.",
                  "DVT raw process physFootprint is not Android PSS; no full WebKit cohort or unique physical-memory sum is claimed.",
                  "Developer file URL plus ordinary Add proves managed intake, not Read originals/File Provider authority.",
                  "Map screenshots and actual archive semantics require human inspection of real rendered geography and gesture changes.",
                  "No official shipping comparison; archives/tabs left intact, histories and recent searches may grow through normal reading."]}
    if receiver_control:
        status.update(receiver_control.binding)
        status["receiver_role"] = "native-iphone-receiver"
        status["receiver_work_started"] = False
        status["receiver_work_retired"] = False
    if launch_only:
        ui_test = os.environ.get("KIWIX_UI_TEST")
        if ui_test == "testUiSmoke":
            status["requested_workload"] = {"ui_smoke": True}
            status["retained_owned_data"] = "Installed application and runner remain; no fixture staging, archive deletion or reset; book details opened and closed."
            status["limitations"] = [
                "Interaction smoke covers Library details, Settings autosave/group heading, Nearby controls and bottom navigation; no archive readability, search, transfer, radio topology or performance certification.",
                "Existing books, tabs and settings are preserved."]
        elif ui_test == "testContentSmoke":
            status["requested_workload"] = {"content_smoke": True}
            status["retained_owned_data"] = "Installed application and runner remain; no fixture staging, archive deletion or reset; a Hawaii reader tab may be opened."
            status["limitations"] = [
                "Content smoke covers an existing Hawaii map, map home/zoom and the Downloads surface; no download throughput, transfer, radio topology or performance certification.",
                "Existing books, tabs and settings are preserved."]
        else:
            status["requested_workload"] = {"launch": True}
            status["retained_owned_data"] = "Installed application and runner remain; no fixture staging, navigation, archive deletion or reset."
            status["limitations"] = ["Install, foreground launch, visible WebView and profile preservation only.",
                "No archive readability, Nearby sharing, transfer, radio topology or performance certification."]
    if nearby:
        status["requested_workload"] = {"nearby": nearby}
        status["limitations"] = ["Completion includes download verification and managed import; UI elapsed time is not isolated radio throughput.",
            "Source hash, admitted archive identity and readable UI are checked; unrelated archives, tabs and settings are preserved."]
        if apple:
            status["limitations"] += [
                "apple-lan uses native Apple LAN/P2P transport on available connectivity; infrastructure versus AWDL route/interface is not measured.",
                "Mac browser-backed headless proof is not native macOS WKWebView proof.",
                "Persisted iPhone peer-id/generation binds discovery, not an independently copied memory-only session or cryptographic human-publisher authentication."]
            if iphone_pair:
                status["limitations"] = [item for item in status["limitations"] if not item.startswith("Mac browser-backed")]
                status["limitations"].append("Data endpoints are the two iPhones; the Mac observes discovery/browse only, never relays archive bytes.")
    if simulator:
        status.update(physical_memory_tested=False, physical_memory="not-applicable",
                      validation_scope="actual-paired-simulator-ui; physical-memory-not-tested",
                      ui_checkpoints={})
        status["limitations"].append("Simulator UI/state evidence is not physical iPhone memory, sampler-transport or performance proof.")
    current_installed = False
    paired_peer = None
    jobs_before = None
    publisher_may_be_owned = False
    publisher_executable = None
    if iphone_pair and not receiver_control:
        status["receiver_launch_attempted"] = False
        status["iphone_receiver"] = {"device": nearby["iphone"]["device"], "ownership": "not_started"}
    try:
        details = window.ctl("identity", ["info", "details"], 30)
        if details.get("hardwareProperties", {}).get("udid") != args.device:
            raise RuntimeError("Requested target identity mismatch")
        if details.get("hardwareProperties", {}).get("reality") != ("simulator" if simulator else "physical"):
            raise RuntimeError("Requested target platform mismatch")
        if not simulator:
            lock = window.ctl("lock", ["info", "lockState"], 30)
            if lock.get("passcodeRequired") is not False or lock.get("unlockedSinceBoot") is not True:
                raise RuntimeError("Device locked or lock state unknown; release immediately")
            status["sampler_readiness"] = window.sampler_readiness(pmd)
        existing_apps = window.ctl("before-app", ["info", "apps", "--bundle-id", app["bundle_id"],
            "--include-container-paths"], 30).get("apps")
        if not isinstance(existing_apps, list):
            raise RuntimeError("Unexpected installed application inventory")
        if (simulator or nearby) and existing_apps == []:
            # A fresh target needs an app before it has a container.
            # Existing apps must be captured BEFORE reinstall can relocate it.
            window.ctl("install-current", ["install", "app", app["app_path"]], 75)
            current_installed = True
            existing_apps = window.ctl("initialized-app", ["info", "apps", "--bundle-id", app["bundle_id"],
                "--include-container-paths"], 10).get("apps", [])
        old_container = container_path(existing_apps, app["bundle_id"])
        before = (window.initialized_state(app["bundle_id"]) if simulator or nearby
                  else window.state("before", app["bundle_id"]))
        existing_nearby = [book for book in before["books"] if nearby and book.get("id") == nearby["archive_id"]]
        if nearby and not verifying and not sending and existing_nearby:
            raise RuntimeError("Nearby source already exists on receiver; choose a missing archive, never delete user data")
        sender_existing = bool(sending and existing_nearby)
        sender_tab = None
        if sender_existing:
            if (len(existing_nearby) != 1 or existing_nearby[0].get("managed") is not True
                    or existing_nearby[0].get("sourceReference") is not None
                    or existing_nearby[0].get("size") != nearby["size"]):
                raise RuntimeError("Existing sender source is not the exact managed UUID and size")
            if sum(tab.get("bookId") == nearby["archive_id"] for tab in before["tabs"]) > 1:
                raise RuntimeError("Existing sender source has ambiguous reader tabs")
            sender_tab = fixture_tab(before, nearby["archive_id"])
        if sending:
            status["sender_source_mode"] = "existing" if sender_existing else "fresh"
        if verifying:
            if len(existing_nearby) != 1 or existing_nearby[0].get("managed") is not True or existing_nearby[0].get("size") != nearby["size"]:
                raise RuntimeError("Existing received archive is not the expected managed UUID and size")
            status["existing_received_job_before"] = nearby_job(window, "before-nearby", app["bundle_id"], nearby)
            status["transfer_performed_this_run"] = False
        wiki_books = [book for book in before["books"] if book.get("id") == WIKI_ID]
        if wiki_workload and (len(wiki_books) > 1 or (wiki_books and wiki_books[0].get("size") != 333429922)):
            raise RuntimeError("Wikipedia inventory is ambiguous or has unexpected size")
        wiki_fresh = wiki_workload and not wiki_books
        if wiki_fresh and not simulator:
            raise RuntimeError("Wikipedia 100 must already be admitted exactly once; do not reimport")
        wiki_tab = fixture_tab(before, WIKI_ID) if wiki_workload else None
        if wiki_fresh and wiki_tab:
            raise RuntimeError("Orphan Wikipedia tab cannot be repurposed")
        wiki_name = "kiwix-window-" + uuid.uuid4().hex + "-wiki.zim" if wiki_fresh else None
        status["wiki_admission_mode"] = "not_requested" if not wiki_workload else "fresh" if wiki_fresh else "existing"
        map_tab = fixture_tab(before, MAP_ID) if map_source else None
        map_books = [book for book in before["books"] if book.get("id") == MAP_ID]
        if map_source and (len(map_books) > 1 or (map_books and map_books[0].get("size") != 150605015)):
            raise RuntimeError("Existing Hawaii archive inventory is ambiguous or has unexpected size")
        map_fresh = bool(map_source and not map_books)
        if map_fresh and map_tab:
            raise RuntimeError("An orphan Hawaii tab exists without its archive; refuse to repurpose it")
        if not map_fresh:
            map_name = None
        reused = [tab for tab in (wiki_tab, map_tab) if tab is not None]
        mutable_tabs = {tab["id"]: tab["book_id"] for tab in reused}
        if sender_tab:
            mutable_tabs[sender_tab["id"]] = sender_tab["book_id"]
        status["reused_fixture_tabs"] = reused
        status["map_admission_mode"] = "fresh" if map_fresh else "existing" if map_source else "not_requested"
        native_before = None
        native_old_container = None
        native_tabs = {}
        native_fresh = {}
        if baseline:
            status["baseline_manifest_sha256"] = digest(args.baseline_manifest)
            status["baseline_source_commit"] = baseline["source_commit"]
            status["baseline_label"] = ("Upstream 3.18.0 source build for simulator; not App Store binary"
                if simulator else "Upstream 3.18.0 source build, developer signed; consented Apple Pay donation merchant omission; not App Store binary")
            native_apps = window.ctl("baseline-before-app", ["info", "apps", "--bundle-id",
                baseline["bundle_id"], "--include-container-paths"], 10).get("apps", [])
            if native_apps:
                native_old_container = container_path(native_apps, baseline["bundle_id"])
                native_before = window.native_state("baseline-before", baseline["bundle_id"])
                if any(book["id"] not in {WIKI_ID, MAP_ID} for book in native_before["books"]):
                    raise RuntimeError("Isolated comparison app contains unrelated opened archives; refuse to alter their grants")
            else:
                if native_apps != []:
                    raise RuntimeError("Unexpected native app inventory")
            inventory = native_before or {"books": [], "tabs": []}
            for key, expected_size in ((WIKI_ID, 333429922), (MAP_ID, 150605015)):
                books = [book for book in inventory["books"] if book["id"] == key]
                if len(books) > 1 or (books and (not native_size_matches(books[0]["size"], expected_size) or not books[0]["available"])):
                    raise RuntimeError("Native fixture is ambiguous, unavailable or has unexpected size")
                native_fresh[key] = not books
                native_tabs[key] = fixture_tab(inventory, key)
                if native_fresh[key] and native_tabs[key]:
                    raise RuntimeError("Native orphan fixture tab would be repurposed by admission")
            status["profile_counts"] = {
                "ours": {"before": {"books": len(before["books"]), "tabs": len(before["tabs"])}, "after": None},
                "baseline": {"before": {"books": len(inventory["books"]), "tabs": len(inventory["tabs"])}, "after": None}}
            status["preservation_scope"] = "Owner's archives/settings retained; preexisting libraries and empty-tab conventions are not identical. This is matched fixture/workload evidence, not fully identical product state or a startup winner."
            save(output / "fixture-sources.json", {"wiki": {"uuid": WIKI_ID, "size": wiki_source.stat().st_size,
                 "sha256": digest(wiki_source)}, "map": {"uuid": MAP_ID, "size": map_source.stat().st_size,
                 "sha256": digest(map_source)}})
        if not current_installed:
            window.ctl("install-current", ["install", "app", app["app_path"]], 75)
        native_container = None
        native_executable = None
        if baseline:
            window.ctl("install-baseline", ["install", "app", baseline["app_path"]], 75)
            native_installed = window.ctl("baseline-installed-app", ["info", "apps", "--bundle-id",
                baseline["bundle_id"], "--include-container-paths"], 10).get("apps", [])
            native_container = container_path(native_installed, baseline["bundle_id"])
            native_info = plistlib.loads((Path(baseline["app_path"]) / "Info.plist").read_bytes())
            native_executable = unquote(urlparse(native_installed[0]["url"]).path).rstrip("/") + "/" + native_info["CFBundleExecutable"]
            status["baseline_container_transition"] = {"before": native_old_container, "after": native_container}
        window.runner_installed = True
        window.ctl("install-runner", ["install", "app", runner["runner_app"]], 30)
        installed = window.ctl("installed-app", ["info", "apps", "--bundle-id", app["bundle_id"],
            "--include-container-paths"], 10).get("apps", [])
        new_container = container_path(installed, app["bundle_id"])
        status["installed_identity"] = {"bundle_id": app["bundle_id"], "source_commit": app["source_commit"],
            "artifact_sha256": app["artifact_sha256"], "app_manifest_sha256": status["app_manifest_sha256"],
            "device_receipt": installed}
        status["verified_container_transition"] = {"before": old_container, "after": new_container,
            "normalization": "Only managed non-provider book paths and designated filesystem settings under these verified app-profile roots"}
        app_info = plistlib.loads((Path(app["app_path"]) / "Info.plist").read_bytes())
        installed_executable = unquote(urlparse(installed[0]["url"]).path).rstrip("/") + "/" + app_info["CFBundleExecutable"]
        after_install = window.state("after-install", app["bundle_id"])
        preservation(before, after_install, old_container, new_container, app["bundle_id"])
        if apple and not sending:
            jobs_before = nearby_jobs(window, "before-mac-receive", app["bundle_id"])
            if any(isinstance(job, dict) and (job.get("fileName") == nearby["file_name"]
                    or urlparse(job.get("url", "")).path.rsplit("/", 1)[-1] == nearby["archive_id"])
                    for job in jobs_before.values()):
                raise RuntimeError("Receiver already has a fixture download job; never resume or replace it")
        if receiver_control:
            observation = receiver_control.ready_and_wait(window, status)
            status["source_observation"] = observation
            status["receiver_control_sha256"] = receiver_control.control_sha256
            window.receiver_control = receiver_control
        data_container = new_container
        if map_fresh and baseline is None:
            window.ctl("stage-owned-map", ["copy", "to", "--domain-type", "appDataContainer",
                "--domain-identifier", app["bundle_id"], "--source", str(map_source),
                "--destination", "Documents/" + map_name], 35)
            save(output / "owned-map.json", {"source_sha256": digest(map_source), "size": map_source.stat().st_size,
                "uuid": MAP_ID, "staged_path": data_container + "/Documents/" + map_name,
                "ownership": "Unique test-owned source left intact; managed admission copies archive. No deletion."})
        source_plan = Path(runner["xctestrun"])
        plan = plistlib.loads(source_plan.read_bytes())
        if "TestConfigurations" in plan:
            targets = [t for c in plan["TestConfigurations"] for t in c.get("TestTargets", [])]
        else:
            targets = [t for k, t in plan.items() if k != "__xctestrun_metadata__" and isinstance(t, dict)]
        if len(targets) != 1 or targets[0].get("BlueprintName") != "KiwixPhoneWindowUI":
            raise RuntimeError("Unexpected XCTest targets")
        variables = targets[0].setdefault("EnvironmentVariables", {})
        variables.update({"KIWIX_DEVICE": args.device, "KIWIX_PHONE_WINDOW": "START-" + args.device,
            "KIWIX_APP_ID": app["bundle_id"], "KIWIX_PLATFORM": args.platform, "KIWIX_MODE": args.test_mode,
            "KIWIX_WIKI_OPEN_LABEL": "Open Wikipedia 100"})
        deadline = receiver_control.request["deadline"] if receiver_control else int(time.time() + window.remaining())
        variables["KIWIX_DEADLINE"] = str(deadline)
        if nearby:
            variables.update(KIWIX_MODE="nearby", KIWIX_NEARBY_FILE_NAME=nearby["file_name"],
                KIWIX_NEARBY_TITLE=nearby["title"], KIWIX_NEARBY_EXPECTED_TEXT=nearby["expected_text"],
                KIWIX_NEARBY_PEER=nearby["peer"])
            variables.update(KIWIX_NEARBY_DIRECTION=nearby.get("direction", "android-to-iphone"),
                             KIWIX_NEARBY_OPERATION=nearby.get("operation", "transfer"),
                             KIWIX_NEARBY_TOPOLOGY="apple-lan" if apple else "android-routerless")
            if not apple:
                # The Android receiver is STA-off, so the infrastructure LAN
                # cannot route this transfer; the observed AP join is the proof.
                # iOS 27 no longer exposes Control Center tiles to accessibility.
                variables["KIWIX_SKIP_NETWORK_PRECONDITION"] = "1"
        if wiki_fresh:
            variables.update(KIWIX_WIKI_MODE="fresh", KIWIX_WIKI_FILENAME=wiki_name)
        if map_name:
            variables["KIWIX_MAP_FILENAME"] = map_name
        if wiki_tab:
            variables["KIWIX_WIKI_TAB_TITLE"] = wiki_tab["title"]
        if map_source:
            variables["KIWIX_MAP_MODE"] = "fresh" if map_fresh else "existing"
            if map_tab:
                variables["KIWIX_MAP_TAB_TITLE"] = map_tab["title"]
        nonce = uuid.uuid4().hex
        sharing_name = None
        requests = {}
        if sending:
            # A unique per-run sharing name removes ambiguity from cached
            # DNS-SD sessions that legitimately share a platform default name.
            # BLE local names are truncated by the platform; keep the unique
            # token inside that budget or the receiver card can never match it.
            sharing_name = "iP" + nonce[:6]
            nearby = dict(nearby, peer=sharing_name)
            variables.update(KIWIX_NEARBY_PEER=sharing_name, KIWIX_NEARBY_SHARING_NAME=sharing_name)
            filename = "kiwix-window-" + nonce + "-nearby.zim"
            binding = json.dumps({"nonce": nonce, "deadline": deadline, "device": args.device,
                "app_manifest_sha256": status["app_manifest_sha256"],
                **({"iphone_device": nearby["iphone"]["device"], "iphone_app_manifest_sha256": nearby["iphone"]["app"]["sha256"],
                    "iphone_runner_manifest_sha256": nearby["iphone"]["runner"]["sha256"],
                    "mac_prepared_sha256": nearby["mac"]["prepared_sha256"]} if iphone_pair else
                   {"mac_prepared_sha256": nearby["mac"]["prepared_sha256"]} if apple else
                   {"android_device": nearby["android"]["device"]}),
                "source": {key: nearby[key] for key in ("archive_id", "file_name", "size", "sha256")}},
                sort_keys=True, separators=(",", ":"))
            variables.update(KIWIX_CHECKPOINT_NONCE=nonce, KIWIX_NEARBY_STAGED_NAME=filename,
                             KIWIX_NEARBY_BINDING=binding,
                             KIWIX_NEARBY_SOURCE_MODE="existing" if sender_existing else "fresh")
            if sender_tab:
                variables["KIWIX_NEARBY_SOURCE_TAB"] = sender_tab["title"]
            if not sender_existing:
                requests["ours/sender-file"] = (app["bundle_id"], Path(nearby["source"]), "Documents/" + filename,
                    Path(data_container + "/Documents/" + filename).as_uri())
            status["nearby_binding"] = json.loads(binding)
        if baseline:
            variables.update(KIWIX_MODE="comparison", KIWIX_ORDER=args.order,
                KIWIX_BASELINE_ID=baseline["bundle_id"], KIWIX_CHECKPOINT_NONCE=nonce)
            wiki_book = wiki_books[0] if wiki_books else None
            selected = next((tab for tab in before["tabs"] if wiki_tab and tab["id"] == wiki_tab["id"]), None)
            variables["KIWIX_OURS_WIKI_AT_HOME"] = "1" if selected is None or unquote(selected.get("path", "")) in ("", wiki_book["mainPath"]) else "0"
            if wiki_fresh:
                requests["ours/wiki-file"] = (app["bundle_id"], wiki_source, "Documents/" + wiki_name,
                    Path(data_container + "/Documents/" + wiki_name).as_uri())
            for book_id, key, name in ((WIKI_ID, "WIKI", "wiki"), (MAP_ID, "MAP", "map")):
                variables["KIWIX_BASELINE_" + key + "_IMPORT"] = "1" if native_fresh[book_id] else "0"
                tab = native_tabs[book_id]
                if tab:
                    variables["KIWIX_BASELINE_" + key + "_TAB_TITLE"] = tab["title"]
                if native_fresh[book_id]:
                    filename = "kiwix-window-" + nonce + "-" + name + ".zim"
                    source = wiki_source if book_id == WIKI_ID else map_source
                    requests["baseline/" + name + "-file"] = (baseline["bundle_id"], source,
                        "Documents/" + filename, Path(native_container + "/Documents/" + filename).as_uri())
                elif tab:
                    main_path = "index" if book_id == WIKI_ID else "index.html"
                    requests["baseline/" + name + "-reset"] = (baseline["bundle_id"], None, None,
                        "zim://" + book_id + "/" + main_path)
            if map_fresh:
                requests["ours/map-file"] = (app["bundle_id"], map_source, "Documents/" + map_name,
                    Path(data_container + "/Documents/" + map_name).as_uri())
        if simulator:
            variables["KIWIX_REQUEST_URLS"] = json.dumps(
                {phase: request[3] for phase, request in requests.items()}, separators=(",", ":"))

        def semantic_check(label, phase):
            role, page = phase.split("/")
            expected = WIKI_ID if page in ("wiki-home", "animal-body") else MAP_ID
            if role == "ours":
                state = window.state(label, app["bundle_id"])
                status["profile_counts"][role]["last_observed"] = {"books": len(state["books"]),
                    "tabs": len(state["tabs"]), "checkpoint": phase}
                active = [tab for tab in state["tabs"] if tab["id"] == state.get("activeTab")]
                if len(active) != 1 or active[0].get("bookId") != expected:
                    raise RuntimeError("Active archive identity mismatch at " + phase)
                if state.get("windows", {}).get("main", {}).get("activeSurface") != "reader":
                    raise RuntimeError("Actual app surface is not reader at " + phase)
                books = [book for book in state["books"] if book.get("id") == expected]
                if len(books) != 1 or books[0].get("size") != (333429922 if expected == WIKI_ID else 150605015):
                    raise RuntimeError("Admitted fixture inventory mismatch at " + phase)
                expected_path = books[0]["mainPath"] if expected == WIKI_ID else "index.html"
                if page != "animal-body" and unquote(active[0].get("path", "")) not in ("", expected_path):
                    raise RuntimeError("Reader is not at the fixture main page at " + phase)
                return {"archive_uuid": expected, "tab_id": active[0]["id"], "path": active[0].get("path"),
                        "content": "pending visual admission of before/after screenshots"}
            state = window.native_state(label, baseline["bundle_id"])
            status["profile_counts"][role]["last_observed"] = {"books": len(state["books"]),
                "tabs": len(state["tabs"]), "checkpoint": phase}
            books = [book for book in state["books"] if book["id"] == expected]
            tabs = [tab for tab in state["tabs"] if tab["bookId"] is not None]
            expected_size = 333429922 if expected == WIKI_ID else 150605015
            if (len(books) != 1 or not books[0]["available"] or not tabs
                    or not native_size_matches(books[0]["size"], expected_size)):
                raise RuntimeError("Native admitted archive identity unavailable at " + phase)
            latest = max(tab["lastOpened"] for tab in tabs)
            active = [tab for tab in tabs if tab["lastOpened"] == latest]
            if len(active) != 1 or active[0]["bookId"] != expected:
                raise RuntimeError("Native most-recent fixture-tab identity mismatch at " + phase)
            return {"archive_uuid": expected, "tab_id": active[0]["id"],
                    "active_identity_scope": "Unique most-recent opened Core Data tab plus authenticated foreground UI checkpoint",
                    "content": "pending visual admission of before/after screenshots"}
        if paired_forward:
            window.stage = peer_label + "-publish-readiness"
            paired_peer = (MacPeer(window, nearby, nonce, args.device, deadline, status["app_manifest_sha256"], mode="publish")
                           if apple else AndroidReceiver(window, nearby, nonce, args.device, deadline, mode="publish"))
            paired_peer.wait_ready()
            status[peer_label + "_instrumentation_ready"] = True
            status[peer_label + "_ready"] = paired_peer.readiness
            window.publisher_peer = paired_peer
        # Must live next to original plan: __TESTROOT__ paths remain valid.
        authorized = source_plan.parent / ("Window-" + uuid.uuid4().hex + ".xctestrun")
        with authorized.open("xb") as stream:
            plistlib.dump(plan, stream)
        log = output / "xctest.log"
        if sending:
            test_name = "testNearbySend"
        elif verifying:
            test_name = "testNearbyVerify"
        elif nearby:
            test_name = "testNearby"
        else:
            test_name = os.environ.get("KIWIX_UI_TEST") or ("testLaunch" if launch_only else "testAcceptanceWindow")
        if launch_only and test_name == "testUiSmoke" and not before.get("books"):
            authorized.unlink(missing_ok=True)
            raise RuntimeError("UI smoke requires a preexisting library archive")
        if (launch_only and test_name == "testContentSmoke"
                and not any(book.get("id") == MAP_ID and book.get("size") == 150605015
                            for book in before.get("books", []))):
            authorized.unlink(missing_ok=True)
            raise RuntimeError("Content smoke requires a preexisting Hawaii archive")
        argv = ["xcodebuild", "test-without-building", "-xctestrun", str(authorized),
            "-destination", ("platform=iOS Simulator,id=" if simulator else "platform=iOS,id=") + args.device, "-destination-timeout", "15",
            "-parallel-testing-enabled", "NO", "-maximum-concurrent-test-device-destinations", "1",
            "-only-testing:KiwixPhoneWindowUI/PhoneWindowUI/" + test_name,
            "-resultBundlePath", str(output / "window.xcresult")]
        started = time.monotonic()
        test_budget = window.remaining() if baseline or sending else min(300, window.remaining())
        last_phase = started
        phases = set()
        rendezvous_seen = set()
        offset = 0
        ui_failure = None
        finalizing_until = None
        pending = ""
        with log.open("xb") as stream:
            if receiver_control:
                # Even a partially successful launch may admit application work.
                status["receiver_work_started"] = True
            window.child = subprocess.Popen(argv, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                while True:
                    window.remaining()
                    now = time.monotonic()
                    finished = window.child.poll() is not None
                    with log.open("r", errors="replace") as reader:
                        reader.seek(offset)
                        chunk = reader.read()
                        offset = reader.tell()
                    pending += chunk
                    lines = pending.split("\n")
                    pending = lines.pop()
                    for line in lines:
                        if line.startswith("KIWIX_STAGE "):
                            stage = line.split("KIWIX_STAGE ", 1)[1]
                            if stage != window.stage:
                                last_phase = time.monotonic()
                            window.stage = stage
                        if line.startswith("KIWIX_FAILURE ") and ui_failure is None:
                            ui_failure = line.split("KIWIX_FAILURE ", 1)[1]
                            status["error_stage"] = window.stage
                            finalizing_until = time.monotonic() + min(15, window.remaining())
                        if line.startswith("KIWIX_TIMING "):
                            label, raw = line.split("KIWIX_TIMING ", 1)[1].split()
                            status.setdefault("instrumented_observation_seconds", {})[label] = float(raw)
                        if line.startswith("KIWIX_NEARBY_JOIN "):
                            event_nonce, raw_count = line.split("KIWIX_NEARBY_JOIN ", 1)[1].split()
                            if (not sending or event_nonce != nonce or raw_count not in ("0", "1")
                                    or int(raw_count) < status.get("nearby_ios_join_count", 0)):
                                raise RuntimeError("Invalid or stale iOS Join-count evidence")
                            if apple and raw_count != "0":
                                raise RuntimeError("Apple LAN flow reported a forbidden temporary-hotspot Join")
                            status["nearby_ios_join_count"] = int(raw_count)
                        if (" error:" in line or ("Test Case" in line and " failed " in line)) and ui_failure is None:
                            ui_failure = line.split(" : failed - ", 1)[-1].strip()
                            status["error_stage"] = window.stage
                            finalizing_until = time.monotonic() + min(15, window.remaining())
                        if (baseline or sending) and ui_failure is None and line.startswith(("KIWIX_REQUEST ", "KIWIX_CHECKPOINT ")):
                            kind = "request" if line.startswith("KIWIX_REQUEST ") else "checkpoint"
                            prefix = "KIWIX_REQUEST " if kind == "request" else "KIWIX_CHECKPOINT "
                            event_phase, event_nonce = line.split(prefix, 1)[1].split()
                            if event_nonce != nonce or event_phase in rendezvous_seen:
                                raise RuntimeError("Stale, duplicate or unauthorized runner rendezvous")
                            rendezvous_seen.add(event_phase)
                            role, page = event_phase.split("/")
                            if role not in (("ours",) if sending else ("ours", "baseline")):
                                raise RuntimeError("Unknown product at runner rendezvous")
                            subject = app if role == "ours" else baseline
                            if kind == "request":
                                if event_phase not in requests:
                                    raise RuntimeError("Unexpected archive-intake/navigation request")
                                foreground = foreground_observer(window, log, event_phase, nonce, subject["bundle_id"])
                                bundle, source, destination, url = requests[event_phase]
                                slug = event_phase.replace("/", "-")
                                if source is not None:
                                    window.ctl(slug + "-stage", ["copy", "to", "--domain-type", "appDataContainer",
                                        "--domain-identifier", bundle, "--source", str(source),
                                        "--destination", destination], 35)
                                    save(output / (slug + "-source.json"), {"archive_source_sha256": digest(source),
                                        "destination": destination, "ordinary_intake": "Developer file URL, no product state injection"})
                                foreground()
                                if simulator:
                                    acknowledge(window, event_phase, nonce, bundle, deadline,
                                                runner["runner_bundle_id"], payload_url=url)
                                else:
                                    window.ctl(slug + "-open", ["process", "launch", "--payload-url", url, bundle], 8)
                                    foreground()
                                    acknowledge(window, event_phase, nonce, bundle, deadline, runner["runner_bundle_id"],
                                                nearby_binding=binding if sending else None)
                            else:
                                if sending:
                                    paired_failure = None
                                    if page == "sender-imported" and paired_peer is None:
                                        imported = window.state("sender-imported", app["bundle_id"])
                                        books = [book for book in imported["books"] if book.get("id") == nearby["archive_id"]]
                                        if len(books) != 1 or books[0].get("managed") is not True or books[0].get("size") != nearby["size"]:
                                            raise RuntimeError("Ordinary iPhone sender intake did not admit the frozen managed archive")
                                        active = [tab for tab in imported["tabs"] if tab["id"] == imported.get("activeTab")]
                                        if (len(active) != 1 or active[0].get("bookId") != nearby["archive_id"]
                                                or imported.get("windows", {}).get("main", {}).get("activeSurface") != "reader"):
                                            raise RuntimeError("Sender readable checkpoint is not the imported source UUID in the active reader")
                                        preservation(before, imported, old_container, new_container, app["bundle_id"], mutable_tabs)
                                        root = Path(new_container) / "Library/Application Support" / app["bundle_id"]
                                        relative = Path(books[0]["path"]).relative_to(root)
                                        if not relative.parts or relative.parts[0] != "archives" or ".." in relative.parts:
                                            raise RuntimeError("Sender import is not in managed archive storage")
                                        copied_source = output / "sender-imported.zim"
                                        window.ctl("sender-imported-bytes", ["copy", "from", "--domain-type", "appDataContainer",
                                            "--domain-identifier", app["bundle_id"], "--source",
                                            str(Path("Library/Application Support") / app["bundle_id"] / relative),
                                            "--destination", str(copied_source)], min(30, window.remaining()))
                                        try:
                                            if copied_source.stat().st_size != nearby["size"] or digest(copied_source) != nearby["sha256"]:
                                                raise RuntimeError("Actual managed iPhone sender bytes differ from the frozen source")
                                        finally:
                                            copied_source.unlink(missing_ok=True)
                                        status["sender_imported"] = {"archive_id": books[0]["id"], "size": books[0]["size"],
                                            "managed": True, "sha256": nearby["sha256"], "reader_tab_id": active[0]["id"],
                                            "readable_ui_checkpoint": event_phase}
                                        if iphone_pair:
                                            status["receiver_launch_attempted"] = True
                                            status["iphone_receiver"] = {"device": nearby["iphone"]["device"],
                                                "ownership": "launch_attempted"}
                                            paired_peer = IPhonePeer(window, nearby, nonce, args.device, deadline,
                                                status["app_manifest_sha256"], MacPeer, pmd)
                                        elif apple:
                                            paired_peer = MacPeer(window, nearby, nonce, args.device, deadline, status["app_manifest_sha256"])
                                        else:
                                            paired_peer = AndroidReceiver(window, nearby, nonce, args.device, deadline)
                                        foreground = foreground_observer(window, log, event_phase, nonce, app["bundle_id"])
                                        try:
                                            paired_peer.wait_ready(foreground)
                                            status[peer_label + "_instrumentation_ready"] = True
                                            status[peer_label + "_ready"] = paired_peer.readiness
                                        except (RuntimeError, TimeoutError) as error:
                                            paired_failure = str(error)
                                            status["paired_failure"] = paired_failure
                                    elif page == "sender-ready" and paired_peer is not None:
                                        foreground = foreground_observer(window, log, event_phase, nonce, app["bundle_id"])
                                        paired_end = time.monotonic() + min(220, window.remaining() - 15)
                                        try:
                                            if apple:
                                                source_publication = iphone_publication(window, "sender-publication-before",
                                                    app["bundle_id"], status["installed_identity"])
                                                foreground()
                                                if iphone_pair:
                                                    paired_peer.receive(source_publication, foreground)
                                                else:
                                                    paired_peer.receive(source_publication)
                                                if iphone_pair:
                                                    status["independent_source_observation"] = paired_peer.observation
                                                status["iphone_source_publication"] = source_publication
                                            while True:
                                                foreground()
                                                peer_result = paired_peer.result()
                                                if peer_result is not None:
                                                    if apple:
                                                        confirmed = iphone_publication(window, "sender-publication-after",
                                                            app["bundle_id"], status["installed_identity"])
                                                        if confirmed != source_publication:
                                                            raise RuntimeError("iPhone source publication changed during Mac reception")
                                                        foreground()
                                                    if iphone_pair:
                                                        status["iphone_observer"] = paired_peer.retire_observer()
                                                        foreground()
                                                    status[peer_label + "_received"] = peer_result
                                                    break
                                                if time.monotonic() >= paired_end:
                                                    raise TimeoutError("Paired peer did not finish transfer within its foreground handshake")
                                                time.sleep(.2)
                                        except (RuntimeError, TimeoutError) as error:
                                            paired_failure = str(error)
                                            status["paired_failure"] = paired_failure
                                    else:
                                        raise RuntimeError("Unexpected or out-of-order Nearby sender checkpoint")
                                    if page == "sender-imported" and paired_failure is None:
                                        # Set before copying the acknowledgement: even a partial
                                        # delivery can release the runner to start Share all.
                                        publisher_may_be_owned = True
                                        publisher_executable = installed_executable
                                    acknowledge(window, event_phase, nonce, app["bundle_id"], deadline,
                                                runner["runner_bundle_id"], nearby_binding=binding, failure=paired_failure)
                                    last_phase = time.monotonic()
                                    continue
                                if page not in ("wiki-home", "animal-body", "map"):
                                    raise RuntimeError("Unexpected memory checkpoint")
                                executable = installed_executable if role == "ours" else native_executable
                                competing = [native_executable if role == "ours" else installed_executable]
                                if simulator:
                                    status["ui_checkpoints"][event_phase] = simulator_checkpoint(
                                        window, event_phase, nonce, subject["bundle_id"], log, deadline,
                                        runner["runner_bundle_id"], semantic_check)
                                else:
                                    status["memory_checkpoints"][event_phase] = measured_checkpoint(
                                        window, event_phase, nonce, subject["bundle_id"], executable, competing, pmd,
                                        log, deadline, runner["runner_bundle_id"], semantic_check)
                            last_phase = time.monotonic()
                        if line.startswith("KIWIX_WINDOW "):
                            phase = line.split("KIWIX_WINDOW ", 1)[1].split()[0]
                            if phase in phases:
                                raise RuntimeError("Repeated UI phase")
                            phases.add(phase)
                            last_phase = time.monotonic()
                            status["phases"] = sorted(phases)
                            if phase.endswith("unexpected-state") and ui_failure is None:
                                ui_failure = "Unexpected UI state captured; no further UI actions"
                                finalizing_until = time.monotonic() + min(15, window.remaining())
                            if not phase.endswith("unexpected-state"):
                                window.completed_steps.append("ui-capture:" + phase)
                            if ui_failure is not None:
                                continue
                            if phase == "wiki-open":
                                window.snapshot("wiki-open", pmd, installed_executable)
                            if phase == "wiki-body":
                                window.snapshot("wiki-body", pmd, installed_executable)
                                if map_name:
                                    url = Path(data_container + "/Documents/" + map_name).as_uri()
                                    window.ctl("warm-owned-map-intake", ["process", "launch", "--payload-url", url,
                                        app["bundle_id"]], 8)
                            if phase == "map-pan":
                                window.snapshot("map-pan", pmd, installed_executable)
                    if finished:
                        break
                    if ui_failure is not None:
                        if now >= finalizing_until:
                            raise RuntimeError(ui_failure + "; bounded result finalization expired")
                    else:
                        phase_limit = 180 if nearby else 120 if os.environ.get("KIWIX_UI_TEST") else 45
                        if now - last_phase > phase_limit:
                            raise TimeoutError(f"XCTest bootstrap/UI phase exceeded {phase_limit} seconds; release now")
                        if now - started > test_budget:
                            raise TimeoutError("Combined XCTest exhausted its remaining action-window budget")
                    time.sleep(0.1)
                if ui_failure is not None:
                    raise RuntimeError(ui_failure)
                if launch_only:
                    required = ({"started", "launch-complete"} if test_name == "testLaunch"
                                else {"started", "complete"})
                elif nearby:
                    required = ({"started", "ours/sender-existing" if sender_existing else "ours/sender-intake", "ours/sender-imported", "ours/sender-ready", "ours/sender-complete"} if sending else
                        {"started", "nearby-existing", "nearby-readable"} if verifying else
                        {"started", "nearby-found", "nearby-catalog", "nearby-completed", "nearby-readable"})
                elif baseline:
                    required = {"started", "complete"}
                    for role in ("ours", "baseline"):
                        required |= {role + "/" + step for step in (
                            "wiki-home", "animal-results", "animal-body", "map-home", "map-zoom", "map-pan", "map")}
                    if map_fresh:
                        required.add("ours/map-intake")
                    if wiki_fresh:
                        required.add("ours/wiki-intake")
                else:
                    required = {"started", "library", "wiki-open", "animal-results", "wiki-body",
                                "reading-pan-restored", "complete"}
                    if map_source:
                        required |= {"map-home", "map-zoom", "map-pan"}
                    if map_fresh:
                        required.add("map-intake")
                if sending:
                    if status.get("nearby_ios_join_count") not in (0, 1):
                        raise RuntimeError("Sender did not report its nonce-bound iOS Join count")
                    if status["nearby_ios_join_count"] == 1:
                        required.add("ours/sender-join")
                if window.child.returncode or not required.issubset(phases):
                    raise RuntimeError("Combined XCTest did not finish all requested actual UI phases")
                status["phases"] = sorted(phases)
            finally:
                if receiver_control and sys.exc_info()[0] is not None:
                    window.cleanup_deadline = receiver_control.begin_cleanup()
                # Host-only temporary plan; keep all evidence and device archive data.
                if window.child.poll() is None:
                    os.killpg(window.child.pid, signal.SIGINT)
                authorized.unlink(missing_ok=True)
        after = window.state("after", app["bundle_id"])
        wiki_after = [book for book in after["books"] if book.get("id") == WIKI_ID]
        if wiki_workload and (len(wiki_after) != 1 or wiki_after[0].get("size") != 333429922
                or (wiki_fresh and wiki_after[0].get("managed") is not True)):
            raise RuntimeError("Real Wikipedia fixture missing or not managed after ordinary intake")
        if nearby:
            received = [book for book in after["books"] if book.get("id") == nearby["archive_id"]]
            if len(received) != 1 or received[0].get("size") != nearby["size"] or received[0].get("managed") is not True:
                raise RuntimeError("Nearby did not admit exactly one managed archive of the expected identity and size")
            active = [tab for tab in after["tabs"] if tab["id"] == after.get("activeTab")]
            if not sending and (len(active) != 1 or active[0].get("bookId") != nearby["archive_id"]
                    or after.get("windows", {}).get("main", {}).get("activeSurface") != "reader"):
                raise RuntimeError("Nearby readable UI is not the selected received archive")
            root = Path(new_container) / "Library/Application Support" / app["bundle_id"]
            relative = Path(received[0]["path"]).relative_to(root)
            if not relative.parts or relative.parts[0] != "archives" or ".." in relative.parts:
                raise RuntimeError("Nearby received archive is outside managed archive storage")
            copied = output / "received-nearby.zim"
            window.ctl("verify-nearby-bytes", ["copy", "from", "--domain-type", "appDataContainer",
                "--domain-identifier", app["bundle_id"], "--source",
                str(Path("Library/Application Support") / app["bundle_id"] / relative),
                "--destination", str(copied)], min(45, window.remaining()))
            try:
                actual_hash = digest(copied)
                if copied.stat().st_size != nearby["size"] or actual_hash != nearby["sha256"]:
                    raise RuntimeError("Nearby received bytes differ from the frozen source archive")
                status["nearby_source" if sending else "nearby_received"] = {
                    "archive_id": received[0]["id"], "size": nearby["size"], "sha256": actual_hash,
                    "managed": True, "readable_ui": True, "reader_tab_id": None if sending else active[0]["id"]}
                if receiver_control:
                    window.ctl("reader", ["capture", "screenshot", "--destination", str(output / "reader.png")], 8)
                if verifying:
                    status["existing_received_job_after"] = nearby_job(window, "after-nearby", app["bundle_id"], nearby)
            finally:
                copied.unlink(missing_ok=True)
        preservation(before, after, old_container, new_container, app["bundle_id"], mutable_tabs,
                     expected_sharing_name=sharing_name if sending else None)
        status["preserved_before_books_settings_tabs"] = True
        if apple and not sending:
            publication = receiver_control.observation["publication"] if receiver_control else paired_peer.publication
            authority = dict(publication)
            if receiver_control:
                authority["offer_id"] = receiver_control.observation["offer"]["id"]
            status["iphone_received_job" if receiver_control else "mac_received_job"] = mac_received_job(
                nearby_jobs(window, "after-mac-receive", app["bundle_id"]), jobs_before, nearby, authority, after)
        if map_source:
            books = [book for book in after["books"] if book.get("id") == MAP_ID]
            if (len(books) != 1 or books[0].get("size") != 150605015
                    or (map_fresh and books[0].get("managed") is not True)):
                raise RuntimeError("Real Hawaii archive is missing, ambiguous or not managed after fresh admission")
        if baseline:
            native_after = window.native_state("baseline-after", baseline["bundle_id"])
            mutable_native = {tab["id"]: tab["book_id"] for tab in native_tabs.values() if tab}
            native_preservation(native_before, native_after, native_old_container, native_container, mutable_native)
            status["profile_counts"]["baseline"]["after"] = {"books": len(native_after["books"]), "tabs": len(native_after["tabs"])}
            status["profile_counts"]["ours"]["after"] = {"books": len(after["books"]), "tabs": len(after["tabs"])}
        window.remaining()
        status["status"] = "passed"
    except BaseException as error:
        if receiver_control:
            window.cleanup_deadline = receiver_control.begin_cleanup()
        window.publisher_peer = None
        status["error"] = str(error)
        status.setdefault("error_stage", window.stage)
        if nearby:
            try:
                window.ctl("first-failure-screen", ["capture", "screenshot", "--destination",
                    str(output / "first-failure.png")], 5)
                status["failure_screenshot"] = "first-failure.png"
            except BaseException as capture_error:
                status["failure_screenshot_error"] = str(capture_error)
        raise
    finally:
        if receiver_control:
            window.cleanup_deadline = receiver_control.begin_cleanup()
        CLEANING_UP = True
        previous_handlers = {sig: signal.signal(sig, signal.SIG_IGN) for sig in (signal.SIGINT, signal.SIGTERM)}
        try:
            window.publisher_peer = None
            window.receiver_control = None
            if paired_peer is not None:
                try:
                    peer_result = paired_peer.stop(complete=(paired_forward or iphone_pair) and status.get("status") == "passed")
                    if paired_forward and peer_result is not None:
                        status[peer_label + "_published"] = peer_result
                    status[peer_label + "_host_stopped"] = True
                    status[peer_label + "_cleanup_complete"] = True
                    if iphone_pair:
                        receiver_receipt = paired_peer.receipt("result.json")
                        status["iphone_receiver"] = {key: receiver_receipt[key] for key in
                            ("device", "cleanup_complete", "remote_runner_stopped", "host_processes_stopped", "phone_released",
                             "receiver_work_retired", "receiver_discovery_stopped", "receiver_owned_job_retired")}
                        status["iphone_observer"] = paired_peer.observer_result
                except BaseException as cleanup_error:
                    status["status"] = "failed"
                    status[peer_label + "_cleanup_error"] = str(cleanup_error)
                    status[peer_label + "_cleanup_complete"] = False
            try:
                status.update(window.stop())
            except BaseException as cleanup_error:
                status["status"] = "failed"
                status["cleanup_error"] = str(cleanup_error)
                status.update(cleanup_complete=False, host_processes_stopped=False, remote_runner_stopped=False)
            if receiver_control:
                try:
                    if status["receiver_work_started"]:
                        status.update(retire_iphone_receiver(window, app, runner, nearby, receiver_control,
                            jobs_before, before, old_container, new_container, status["installed_identity"]))
                    else:
                        status.update(receiver_work_retired=True, receiver_discovery_stopped=True,
                            receiver_owned_job_retired=True, receiver_retirement_scope="Receiver UI was never launched; no discovery or download action was admitted.")
                except BaseException as cleanup_error:
                    status["status"] = "failed"
                    status["receiver_retirement_error"] = str(cleanup_error)
                    status["receiver_work_retired"] = False
                    status["cleanup_complete"] = False
                finally:
                    try:
                        retired_runner = window.retirement_cleanup_receipt or window.stop()
                        status["remote_runner_stopped"] = retired_runner["remote_runner_stopped"]
                        status["host_processes_stopped"] = retired_runner["host_processes_stopped"]
                    except BaseException as cleanup_error:
                        status["status"] = "failed"
                        status["cleanup_error"] = str(cleanup_error)
                        status.update(cleanup_complete=False, host_processes_stopped=False, remote_runner_stopped=False)
            if publisher_may_be_owned:
                try:
                    if status.get("status") == "passed" and "ours/sender-complete" in status.get("phases", []):
                        status.update(publisher_retired=True, publisher_retirement="confirmed-scoped-ui-stop")
                    else:
                        status.update(window.retire_owned_publisher(publisher_executable))
                except BaseException as cleanup_error:
                    status["status"] = "failed"
                    status["publisher_retired"] = False
                    status["publisher_cleanup_error"] = str(cleanup_error)
                    status["cleanup_complete"] = False
            if paired_peer is not None and status.get(peer_label + "_cleanup_complete") is not True:
                status["cleanup_complete"] = False
            status["elapsed_seconds"] = time.monotonic() - window.start
            status["commands"] = window.commands
            status["completed_steps"] = window.completed_steps
            if baseline:
                status["comparisons"] = {}
                for page in ("wiki-home", "animal-body", "map"):
                    measured = {role: status["memory_checkpoints"].get(role + "/" + page)
                                for role in ("ours", "baseline")}
                    complete = all(value and value["status"] == "complete_observed_coalition" for value in measured.values())
                    status["comparisons"][page] = {"status": "pending_visual_admission" if complete else "unavailable",
                        "measurements": measured, "winner": None, "order": args.order, "independent_sessions": 1,
                        "reason": "Content screenshots require admission; no automatic winner, PSS or deduplicated-RAM claim"}
                if simulator:
                    for page in ("wiki-home", "animal-body", "map"):
                        observed = {role: status["ui_checkpoints"].get(role + "/" + page)
                                    for role in ("ours", "baseline")}
                        status["comparisons"][page] = {
                            "status": "pending_visual_admission" if all(observed.values()) else "unavailable",
                            "ui_checkpoints": observed, "physical_memory": "not-applicable",
                            "physical_memory_tested": False, "winner": None, "order": args.order,
                            "reason": "Simulator UI/state validation only; physical-memory-not-tested"}
            status["phone_released"] = status["cleanup_complete"] and status["remote_runner_stopped"]
            if receiver_control:
                status["cleanup_complete"] = status["cleanup_complete"] and status["receiver_work_retired"]
                status["phone_released"] = status["phone_released"] and status["receiver_work_retired"]
            if iphone_pair and not receiver_control:
                status["phone_released"] = status["phone_released"] and receiver_cleanup_confirmed(
                    status, nearby["iphone"]["device"])
            status["timing"] = TIMING.summary()
            status["timing_summary_path"] = str(output / "timing-summary.json")
            save(output / "result.json", status)
            print(json.dumps({"result_path": str(output / "result.json"), "status": status["status"],
                              "platform": args.platform, "phone_released": status["phone_released"]}), flush=True)
        finally:
            for sig, handler in previous_handlers.items():
                signal.signal(sig, handler)
            CLEANING_UP = False
    if status["status"] != "passed":
        raise RuntimeError("Phone run failed; inspect private result, do not retry during this window")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    prepare_parser = sub.add_parser("prepare-only", help="Host-only build; never contacts a phone or simulator")
    prepare_parser.add_argument("--output", type=Path, required=True)
    prepare_parser.add_argument("--device", required=True, help="Offline device-profile member or exact owned simulator UDID")
    prepare_parser.add_argument("--platform", choices=["device", "simulator"], default="device")
    prepare_parser.add_argument("--signing-source", type=Path)
    prepare_parser.add_argument("--signing-identity")
    prepare_parser.add_argument("--map-source", type=Path)
    prepare_parser.add_argument("--mode", dest="test_mode", choices=["acceptance", "comparison", "nearby", "launch"], default="acceptance")
    prepare_parser.add_argument("--baseline-manifest", type=Path)
    prepare_parser.add_argument("--order", choices=["AB", "BA"], default="AB")
    prepare_parser.add_argument("--wiki-source", type=Path)
    prepare_parser.add_argument("--pmd", type=Path, help="Explicit already-installed PMD 9.36.x executable; otherwise honor PATH")
    for name in ["validate-only", "run"]:
        child = sub.add_parser(name)
        child.add_argument("--device", required=True)
        child.add_argument("--platform", choices=["device", "simulator"], default="device")
        child.add_argument("--app-manifest", type=Path, required=True)
        child.add_argument("--runner-manifest", type=Path, required=True)
        child.add_argument("--map-source", type=Path)
        child.add_argument("--mode", dest="test_mode", choices=["acceptance", "comparison", "nearby", "launch"], default="acceptance")
        child.add_argument("--baseline-manifest", type=Path)
        child.add_argument("--order", choices=["AB", "BA"], default="AB")
        child.add_argument("--wiki-source", type=Path)
        child.add_argument("--nearby-json", type=json.loads, help="Frozen source identity and bytes for actual Nearby acceptance")
        child.add_argument("--pmd", type=Path, help="Explicit already-installed PMD 9.36.x executable; otherwise honor PATH")
        if name == "run":
            child.add_argument("--output", type=Path, required=True)
            child.add_argument("--window-seconds", type=int, default=600, choices=range(60, 1801),
                metavar="60..1800", help="Total remaining owner-authorized budget, including cleanup")
            child.add_argument("--start-device-window", action="store_true",
                help="Only after the owner grants one hands-off window of the authorized length")
    args = parser.parse_args()
    if args.mode == "prepare-only" and args.platform == "device" and (
            args.signing_source is None or args.signing_identity is None):
        parser.error("Device preparation requires --signing-source and --signing-identity")
    if args.mode == "run" and args.start_device_window != (args.platform == "device"):
        parser.error("--start-device-window is required only for device runs and forbidden for simulator runs")
    try:
        if args.platform == "simulator":
            uuid.UUID(args.device)
    except ValueError:
        parser.error("Simulator requires an explicit simulator UDID, never a connected-device fallback")
    if args.mode == "run":
        run(args)
        return
    if args.mode == "validate-only":
        runner = json.loads(args.runner_manifest.read_text())
        if runner.get("source_sha256") != sources():
            raise RuntimeError("Runner source changed; rebuild/review offline before validation")
        timing_output = args.runner_manifest.resolve().parent
    else:
        timing_output = args.output.resolve()
        timing_output.mkdir(parents=True, exist_ok=True, mode=0o700)
    previous_iteration = os.environ.get("KIWIX_TIMING_ITERATION")
    start_timing(timing_output, expected_sha256=(
        runner["source_sha256"]["testing_timing.py"] if args.mode == "validate-only" else None))
    os.environ["KIWIX_TIMING_ITERATION"] = TIMING.iteration_id
    try:
        with TIMING.span("host_prepare", "prepare" if args.mode == "prepare-only" else "validate"):
            if args.mode == "prepare-only":
                prepare(args)
            else:
                validate(args)
                print(json.dumps({"status": "ready", "platform": args.platform, "device_contact": False}))
    finally:
        TIMING.close()
        if previous_iteration is None:
            os.environ.pop("KIWIX_TIMING_ITERATION", None)
        else:
            os.environ["KIWIX_TIMING_ITERATION"] = previous_iteration


if __name__ == "__main__":
    def interrupted(signum, frame):
        if not CLEANING_UP:
            raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped; do not retry in the same phone window", file=sys.stderr)
        sys.exit(130)
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
