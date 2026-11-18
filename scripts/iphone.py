#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Prepare off-phone, then install and smoke-test within one ten-minute window."""
import argparse
import hashlib
import json
import importlib.util
from pathlib import Path
import subprocess
import sys
import uuid
import time
import os
import signal
import shlex
import re
from contextlib import nullcontext

from iphone_peer import receiver_cleanup_confirmed

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parent
TOOL_FILES = [SCRIPTS / name for name in ("iphone.py", "iphone-build.py", "iphone-window.py", "iphone_peer.py", "iphone_memory.py", "testing_timing.py")]
TOOL_FILES += [SCRIPTS / "iphone-ui" / name for name in ("PhoneWindowUI.swift", "project.yml")]
NEARBY_DIRECTIONS = ("android-to-iphone", "iphone-to-android", "mac-to-iphone", "iphone-to-mac", "iphone-to-iphone")
APPLE_DIRECTIONS = ("mac-to-iphone", "iphone-to-mac", "iphone-to-iphone")
IPHONE_UDID = re.compile(r"(?:[A-Fa-f0-9]{40}|[A-Fa-f0-9]{8}-[A-Fa-f0-9]{16})")
TIMING = None


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load(path):
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"Expected a JSON object: {path}")
    return value


def local_path(value):
    path = Path(value).expanduser()
    return (REPO / path).resolve() if not path.is_absolute() else path.resolve()


def run_json(command, timeout):
    category = "build" if any(Path(str(part)).name == "iphone-build.py" for part in command) or "prepare-only" in command else "host_prepare"
    with TIMING.span(category, "preparation-helper") if TIMING else nullcontext():
        return run_json_command(command, timeout)


def run_json_command(command, timeout):
    started = time.monotonic()
    result = subprocess.run(command, cwd=REPO, stdout=subprocess.PIPE, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"Preparation stopped (exit {result.returncode}); the phone window has not started.")
    try:
        output = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("Helper did not return its JSON result; inspect its preparation logs.") from error
    print(f"Finished in {time.monotonic() - started:.1f}s", flush=True)
    return output


def receiver_configuration(nearby, device, cache):
    """Resolve the ignored secondary build inputs; none become runtime authority."""
    value = nearby.get("iphone")
    if not isinstance(value, dict):
        raise ValueError("iPhone pair requires nearby.iphone.config and nearby.iphone.cache")
    for key in ("config", "cache"):
        if not isinstance(value.get(key), str) or not value[key].strip():
            raise ValueError("iPhone pair requires explicit nearby.iphone." + key)
    config_path = local_path(value["config"]).resolve(strict=True)
    if subprocess.run(["git", "check-ignore", "--quiet", "--", str(config_path)], cwd=REPO,
                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10).returncode != 0:
        raise ValueError("The secondary iPhone config must be an ignored local file")
    receiver = load(config_path)
    target = receiver.get("device")
    if (not isinstance(target, str) or not IPHONE_UDID.fullmatch(target)
            or not IPHONE_UDID.fullmatch(device) or target.casefold() == device.casefold()
            or receiver.get("platform", "device") != "device"):
        raise ValueError("iPhone pair requires two distinct explicit physical iPhone UDIDs")
    for key in ("signing_source", "signing_identity"):
        if not isinstance(receiver.get(key), str) or not receiver[key].strip():
            raise ValueError("Secondary iPhone config requires " + key)
    receiver_cache = local_path(value["cache"])
    if receiver_cache == cache.resolve():
        raise ValueError("Secondary iPhone artifact cache must differ from the source cache")
    return receiver, receiver_cache


def prepare_receiver(config, cache, revision, fixture, mobile):
    """Use the ordinary app/runner preparation for the frozen secondary target."""
    cache.mkdir(parents=True, exist_ok=True)
    device = config["device"]
    signing = ["--signing-source", str(local_path(config["signing_source"])),
               "--signing-identity", config["signing_identity"]]
    app = run_json([sys.executable, str(SCRIPTS / "iphone-build.py"), "--platform", "device", "--build",
                    "--repo", str(REPO), "--revision", revision, "--output", str(cache / "build"),
                    "--device-udid", device, *signing], 7200)
    runner = run_json([sys.executable, str(SCRIPTS / "iphone-window.py"), "prepare-only",
                       "--platform", "device", "--output", str(cache / "runner"), "--device", device,
                       "--mode", "nearby", "--map-source", str(fixture), *signing, *mobile], 1800)
    frozen = {"device": device}
    for key, result in (("app", app), ("runner", runner)):
        manifest = Path(result["manifest_path"]).resolve(strict=True)
        frozen[key] = {"path": str(manifest), "sha256": sha256(manifest)}
    return frozen


def validate_prepared(path, device, expected_sha256=None):
    content = path.read_bytes()
    if expected_sha256 is not None and hashlib.sha256(content).hexdigest() != expected_sha256:
        raise ValueError("Prepared receipt changed after selection; prepare and authorize a new window.")
    prepared = json.loads(content)
    if not isinstance(prepared, dict):
        raise ValueError("Prepared receipt must be a JSON object.")
    if prepared.get("schema") != 3 or prepared.get("device") != device:
        raise ValueError("Prepared receipt belongs to a different target or schema; prepare again.")
    if prepared.get("platform") not in ("device", "simulator"):
        raise ValueError("Prepared receipt must explicitly select device or simulator.")
    expected_tools = {str(p.relative_to(REPO)): sha256(p) for p in TOOL_FILES}
    if prepared.get("tool_sha256") != expected_tools:
        raise ValueError("iPhone tools changed after preparation; run --prepare-only again.")
    if prepared.get("mode") not in ("acceptance", "comparison", "nearby", "launch") or prepared.get("order") not in ("AB", "BA"):
        raise ValueError("Prepared receipt has an invalid mode or order; prepare again.")
    artifacts = ("app", "runner", "baseline") if prepared["mode"] == "comparison" else ("app", "runner")
    for key in artifacts:
        receipt = prepared[key]
        manifest = Path(receipt["path"])
        if sha256(manifest) != receipt["sha256"]:
            raise ValueError(f"Prepared {key} manifest changed; prepare again.")
        data = load(manifest)
        if data.get("schema") != 1 or data.get("status") != "ready":
            raise ValueError(f"The {key} artifact is not ready.")
        if data.get("platform") != prepared["platform"]:
            raise ValueError(f"Prepared {key} artifact platform does not match the selected target.")
    fixtures = () if prepared["mode"] == "launch" else ("map_source", "wiki_source") if prepared["mode"] == "comparison" else ("map_source",)
    for key in fixtures:
        fixture = prepared[key]
        if sha256(Path(fixture["path"])) != fixture["sha256"]:
            raise ValueError(f"Prepared {key} changed; prepare again.")
    if prepared["mode"] == "nearby":
        source = prepared["nearby"]
        if sha256(Path(source["source"])) != source["sha256"] or Path(source["source"]).stat().st_size != source["size"]:
            raise ValueError("Prepared Nearby source changed; prepare again.")
        if source.get("direction") in APPLE_DIRECTIONS:
            mac = source["mac"]
            if sha256(Path(mac["tool"])) != mac["tool_sha256"] or sha256(Path(mac["prepared"])) != mac["prepared_sha256"]:
                raise ValueError("Prepared Mac helper or receipt changed; prepare again.")
        if source.get("direction") == "iphone-to-iphone":
            receiver = source.get("iphone")
            if (prepared["platform"] != "device" or source.get("topology") != "apple-lan"
                    or source.get("operation", "transfer") != "transfer"):
                raise ValueError("iPhone pair requires physical apple-lan transfer topology")
            if (not isinstance(receiver, dict) or set(receiver) != {"device", "app", "runner"}
                    or not isinstance(receiver.get("device"), str)
                    or not IPHONE_UDID.fullmatch(receiver["device"]) or not IPHONE_UDID.fullmatch(device)
                    or receiver["device"].casefold() == device.casefold()):
                raise ValueError("Prepared iPhone pair must freeze two distinct exact device artifacts")
            primary_app = load(Path(prepared["app"]["path"]))
            for key in ("app", "runner"):
                receipt = receiver[key]
                manifest = Path(receipt["path"])
                if (set(receipt) != {"path", "sha256"} or not manifest.is_absolute()
                        or str(manifest.resolve(strict=True)) != str(manifest)
                        or sha256(manifest) != receipt["sha256"]):
                    raise ValueError("Prepared secondary " + key + " manifest changed; prepare again.")
                data = load(manifest)
                if data.get("schema") != 1 or data.get("status") != "ready" or data.get("platform") != "device":
                    raise ValueError("Prepared secondary " + key + " is not a ready physical-device artifact")
                if key == "app" and (not primary_app.get("source_commit")
                        or data.get("source_commit") != primary_app["source_commit"]):
                    raise ValueError("Prepared iPhone pair app source revisions differ")
    return prepared


def cleanup_confirmed(result, prepared):
    if (result.get("platform") != prepared["platform"] or result.get("device") != prepared["device"]
            or result.get("cleanup_complete") is not True or result.get("remote_runner_stopped") is not True):
        return False
    if prepared["mode"] == "nearby" and prepared["nearby"].get("direction") == "iphone-to-iphone":
        return receiver_cleanup_confirmed(result, prepared["nearby"]["iphone"]["device"])
    return True


def workload_arguments(prepared):
    arguments = ["--platform", prepared["platform"], "--mode", prepared["mode"], "--order", prepared["order"]]
    if prepared["mode"] != "launch":
        arguments += ["--map-source", prepared["map_source"]["path"]]
    if prepared["mode"] == "nearby":
        arguments += ["--nearby-json", json.dumps(prepared["nearby"])]
    if prepared["mode"] == "comparison":
        arguments += ["--baseline-manifest", prepared["baseline"]["path"],
                      "--wiki-source", prepared["wiki_source"]["path"]]
    return arguments


def run_window(command, deadline, cleanup_seconds=60):
    with (TIMING.span("device_automation", "device-window") if TIMING else nullcontext()) as measured:
        code = run_window_command(command, deadline, cleanup_seconds)
        if code and measured is not None:
            measured.fail()
        return code


def run_window_command(command, deadline, cleanup_seconds=60):
    cancelled = False
    process = None

    def request_stop(signum, frame):
        nonlocal cancelled
        # Defer cancellation through spawning and cleanup. Repeated interrupts
        # must never interrupt the TERM -> KILL -> reap sequence.
        cancelled = True

    previous = {sig: signal.signal(sig, request_stop) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        if not cancelled:
            process = subprocess.Popen(command, cwd=REPO, start_new_session=True)
        while not cancelled and process is not None:
            remaining = deadline - time.monotonic() - cleanup_seconds
            if remaining <= 0:
                break
            try:
                code = process.wait(timeout=min(.5, remaining))
                if not cancelled:
                    return code
            except subprocess.TimeoutExpired:
                continue
        raise RuntimeError("Phone window stopped; inspect the saved cleanup result.")
    finally:
        try:
            if process is not None:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=max(.01, min(cleanup_seconds, deadline - time.monotonic() - 1)))
                except subprocess.TimeoutExpired:
                    pass
                # The leader may exit while an owned descendant ignores TERM.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait(timeout=max(.01, deadline - time.monotonic()))
        finally:
            mask = signal.pthread_sigmask(signal.SIG_BLOCK, set(previous))
            try:
                for sig, handler in previous.items():
                    signal.signal(sig, handler)
            finally:
                signal.pthread_sigmask(signal.SIG_SETMASK, mask)


def main():
    previous_iteration = os.environ.get("KIWIX_TIMING_ITERATION")
    try:
        return main_impl()
    finally:
        if TIMING:
            TIMING.close()
        if previous_iteration is None:
            os.environ.pop("KIWIX_TIMING_ITERATION", None)
        else:
            os.environ["KIWIX_TIMING_ITERATION"] = previous_iteration


def main_impl():
    global TIMING
    entered = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=REPO / ".native/iphone-config.json")
    parser.add_argument("--device", help="Override the explicit iPhone or simulator UDID selected by platform")
    parser.add_argument("--platform", choices=("device", "simulator"), help="Target platform (default device); simulator never contacts a phone")
    parser.add_argument("--revision", help="Committed source revision (default HEAD); never includes dirty edits")
    parser.add_argument("--cache", type=Path, help="Artifact cache (default .native/iphone-tool, or its simulator subdirectory)")
    parser.add_argument("--mode", choices=("acceptance", "comparison", "nearby", "launch"), help="Workload (default from local config, otherwise acceptance)")
    parser.add_argument("--order", choices=("AB", "BA"), help="Comparison order: A is ours, B is upstream")
    parser.add_argument("--nearby-direction", choices=NEARBY_DIRECTIONS,
                        help="Nearby direction; Apple directions require explicit nearby.topology=apple-lan")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--prepare-only", action="store_true", help="Build and sign app/runner without contacting any device")
    mode.add_argument("--prepared", type=Path, help="Reuse a previously prepared receipt without building")
    parser.add_argument("--start-device-window", action="store_true", help="Explicitly start now; requires --prepared and never builds")
    parser.add_argument("--window-minutes", type=int, default=10,
                        help="Bounded test window in minutes (2-30; default 10). The owner authorizes any value above 10 explicitly")
    args = parser.parse_args()
    if args.start_device_window and not args.prepared:
        parser.error("--start-device-window requires --prepared; preparation must finish before phone use")
    if args.prepared and args.revision:
        parser.error("--prepared selects its recorded source revision; do not combine it with --revision")
    if not 2 <= args.window_minutes <= 30:
        parser.error("--window-minutes must be between 2 and 30")
    window_deadline = entered + args.window_minutes * 60 if args.start_device_window else None
    config = load(args.config.expanduser().resolve())
    recorded = load(args.prepared.expanduser().resolve()) if args.prepared else None
    platform = args.platform or (recorded.get("platform") if recorded else config.get("platform", "device"))
    if platform not in ("device", "simulator"):
        parser.error("Select an explicit device or simulator platform; prepare old receipts again")
    if args.start_device_window and platform != "device":
        parser.error("--start-device-window is only for a physical iPhone, not a simulator")
    mobile = ["--pmd", str(local_path(config["pmd"]))] if platform == "device" and config.get("pmd") else []
    device = args.device or config.get("simulator_device" if platform == "simulator" else "device")
    if not isinstance(device, str) or not device.strip() or any(c.isspace() for c in device):
        parser.error("Provide the explicit target UDID through --device or the ignored local config")
    cache = args.cache.expanduser().resolve() if args.cache else REPO / ".native/iphone-tool"
    if args.cache is None and platform == "simulator":
        cache /= "simulator"
    cache.mkdir(parents=True, exist_ok=True)
    if args.prepared:
        selected = args.prepared.expanduser().resolve()
        validate_prepared(selected, device, sha256(selected))
    # Admit every transitive timing source before importing it on a frozen run.
    timing_path = SCRIPTS / "testing_timing.py"
    content = timing_path.read_bytes()
    expected = recorded["tool_sha256"]["scripts/testing_timing.py"] if recorded else sha256(timing_path)
    if hashlib.sha256(content).hexdigest() != expected:
        raise RuntimeError("Frozen timing dependency changed before import")
    spec = importlib.util.spec_from_file_location("testing_timing", timing_path)
    module = importlib.util.module_from_spec(spec)
    exec(compile(content, str(timing_path), "exec"), module.__dict__)
    TIMING = module.TimingLog(cache, os.environ.get("KIWIX_TIMING_ITERATION") or uuid.uuid4().hex, "iphone")
    os.environ["KIWIX_TIMING_ITERATION"] = TIMING.iteration_id
    # Cargo already isolates target triples; reuse host dependencies instead of
    # cloning a second multi-gigabyte target cache for simulator preparation.
    build_cache = cache.parent / "build" if platform == "simulator" and args.cache is None else cache / "build"
    if args.prepared:
        prepared_path = args.prepared.expanduser().resolve()
        for key in ("platform", "mode", "order"):
            if getattr(args, key) is not None and getattr(args, key) != recorded.get(key):
                parser.error(f"--prepared selects its recorded {key}; prepare again to change it")
        if args.nearby_direction is not None and args.nearby_direction != recorded.get("nearby", {}).get("direction", "android-to-iphone"):
            parser.error("--prepared selects its recorded Nearby direction; prepare again to change it")
    else:
        workload = args.mode or config.get("mode", "acceptance")
        required = () if platform == "simulator" else ("signing_source", "signing_identity")
        if workload != "launch":
            required += ("map_source",)
        for key in required:
            if not config.get(key):
                parser.error(f"Missing {key} in {args.config}; see docs/PACKAGING.md")
        order = args.order or config.get("order", "AB")
        if workload not in ("acceptance", "comparison", "nearby", "launch") or order not in ("AB", "BA"):
            parser.error("Config mode must be acceptance/comparison/nearby/launch and order must be AB/BA")
        nearby = dict(config.get("nearby", {})) if workload == "nearby" else None
        receiver_config = receiver_cache = None
        if nearby is not None:
            nearby["direction"] = args.nearby_direction or nearby.get("direction", "android-to-iphone")
            if nearby["direction"] not in NEARBY_DIRECTIONS:
                parser.error("Invalid nearby.direction")
            if nearby["direction"] in APPLE_DIRECTIONS and (
                    platform != "device" or nearby.get("topology") != "apple-lan"
                    or nearby.get("operation", "transfer") != "transfer"):
                parser.error("Apple Nearby requires physical device, explicit topology apple-lan and operation transfer")
            if nearby["direction"] == "iphone-to-iphone":
                if args.window_minutes > 10:
                    parser.error("An iPhone pair shares one maximum ten-minute owner window")
                receiver_config, receiver_cache = receiver_configuration(nearby, device, cache)
        if workload == "comparison":
            for key in ("upstream_revision", "wiki_source"):
                if not config.get(key):
                    parser.error(f"Missing {key} for comparison in {args.config}; see docs/PACKAGING.md")
            if platform == "device" and config.get("allow_payment_entitlement_omission") is not True:
                parser.error("The source-built upstream baseline needs explicit allow_payment_entitlement_omission approval")
        signing = (["--signing-source", str(local_path(config["signing_source"])),
                    "--signing-identity", config["signing_identity"]] if platform == "device" else [])
        target = ["--device-udid", device, *signing] if platform == "device" else []
        fixtures = {} if workload == "launch" else {"map_source": local_path(config["map_source"])}
        if workload == "comparison":
            fixtures["wiki_source"] = local_path(config["wiki_source"])
        for key, path in fixtures.items():
            if not path.is_file():
                parser.error(f"{key} fixture is missing: {path}")
        print("Preparing current app off-phone. You can keep using the phone.", flush=True)
        app = run_json([sys.executable, str(SCRIPTS / "iphone-build.py"), "--platform", platform, "--build",
                        "--repo", str(REPO), "--revision", args.revision or "HEAD",
                        "--output", str(build_cache),
                        *target], 7200)
        products = {"app": app}
        if workload == "comparison":
            print(f"Preparing clean upstream Kiwix Apple off-phone ({platform}; source-built, not an App Store binary).", flush=True)
            upstream = [sys.executable, str(SCRIPTS / "iphone-build.py"), "--product", "kiwix-apple", "--platform", platform, "--build",
                        "--repo", str(REPO), "--revision", config["upstream_revision"],
                        "--output", str(cache / "upstream-build"), *target]
            if platform == "device":
                upstream.append("--allow-payment-entitlement-omission")
            for key, flag in (("corekiwix_archive", "--corekiwix-archive"),
                              ("upstream_package_lock", "--upstream-package-lock"),
                              ("upstream_sourcepackages", "--upstream-sourcepackages")):
                if config.get(key):
                    upstream += [flag, str(local_path(config[key]))]
            if config.get("upstream_bundle_id"):
                upstream += ["--upstream-bundle-id", config["upstream_bundle_id"]]
            if config.get("upstream_repo"):
                upstream += ["--upstream-repo", str(local_path(config["upstream_repo"]))]
            products["baseline"] = run_json(upstream, 7200)
        print(f"Preparing {platform} UI runner off-phone.", flush=True)
        runner = run_json([sys.executable, str(SCRIPTS / "iphone-window.py"), "prepare-only",
                           "--platform", platform, "--output", str(cache / "runner"), "--device", device,
                           "--mode", workload, *signing, *mobile,
                           *(["--map-source", str(fixtures["map_source"])] if "map_source" in fixtures else [])], 1800)
        products["runner"] = runner
        prepared = {"schema": 3, "platform": platform, "device": device, "mode": workload, "order": order,
                    "tool_sha256": {str(p.relative_to(REPO)): sha256(p) for p in TOOL_FILES}}
        if workload == "nearby":
            apple = nearby["direction"] in APPLE_DIRECTIONS
            if apple:
                mac = dict(nearby.get("mac", {}))
                for key in ("executable", "browser"):
                    if not isinstance(mac.get(key), str) or not Path(mac[key]).is_absolute():
                        parser.error(f"Apple Nearby requires absolute nearby.mac.{key}")
                    mac[key] = str(Path(mac[key]).resolve(strict=True))
                tool = SCRIPTS / "mac-peer.mjs"
                result = run_json(["node", str(tool), "--prepare-only", "--executable", mac["executable"],
                                   "--browser", mac["browser"], "--output", str(cache / ("mac-" + uuid.uuid4().hex))], 120)
                receipt = Path(result["manifest_path"]).resolve(strict=True)
                mac.update(prepared=str(receipt), prepared_sha256=sha256(receipt),
                           tool=str(tool), tool_sha256=sha256(tool))
                nearby["mac"] = mac
                if receiver_config is not None:
                    primary_manifest = load(Path(app["manifest_path"]))
                    nearby["iphone"] = prepare_receiver(receiver_config, receiver_cache,
                        primary_manifest["source_commit"], fixtures["map_source"], mobile)
            elif nearby.get("operation", "transfer") == "transfer":
                android = dict(nearby.get("android", {}))
                for key in ("device", "prepared"):
                    if not isinstance(android.get(key), str) or not android[key].strip():
                        parser.error(f"Paired Nearby requires nearby.android.{key}")
                receipt = local_path(android["prepared"])
                android.update(prepared=str(receipt), prepared_sha256=sha256(receipt),
                               tool=str(SCRIPTS / "android-nearby.py"),
                               tool_sha256=sha256(SCRIPTS / "android-nearby.py"))
                nearby["android"] = android
            for key in ("archive_id", "file_name", "title", "expected_text", "peer", "source"):
                if not isinstance(nearby.get(key), str) or not nearby[key].strip():
                    parser.error(f"Missing nearby.{key} in {args.config}")
            uuid.UUID(nearby["archive_id"])
            source = local_path(nearby["source"])
            nearby.update(source=str(source), sha256=sha256(source), size=source.stat().st_size)
            prepared["nearby"] = nearby
        for key, path in fixtures.items():
            prepared[key] = {"path": str(path), "sha256": sha256(path)}
        for key, result in products.items():
            manifest = Path(result["manifest_path"]).resolve()
            prepared[key] = {"path": str(manifest), "sha256": sha256(manifest)}
        prepared_path = cache / "prepared.json"
        prepared_path.write_text(json.dumps(prepared, indent=2) + "\n")
    prepared_sha256 = sha256(prepared_path)
    prepared = validate_prepared(prepared_path, device, prepared_sha256)
    pair = prepared["mode"] == "nearby" and prepared["nearby"].get("direction") == "iphone-to-iphone"
    if pair and args.window_minutes > 10:
        parser.error("An iPhone pair shares one maximum ten-minute owner window")
    run_json([sys.executable, str(SCRIPTS / "iphone-window.py"), "validate-only", "--device", device,
              "--app-manifest", prepared["app"]["path"], "--runner-manifest", prepared["runner"]["path"],
              *workload_arguments(prepared), *mobile], 120)
    print(f"READY. {platform} {prepared['mode']} apps and UI runner prepared: {prepared_path}", flush=True)
    if args.prepare_only:
        next_command = ["npm", "run", "iphone", "--", "--config",
                        str(args.config.expanduser().resolve()), "--device", device,
                        "--prepared", str(prepared_path)]
        if platform == "device":
            next_command += ["--start-device-window", "--window-minutes", str(args.window_minutes)]
        print("No phone contacted. Next: " + shlex.join(next_command), flush=True)
        return 0
    if platform == "device" and not args.start_device_window:
        if not sys.stdin.isatty():
            raise RuntimeError("No interactive terminal. Use --prepare-only, then --prepared RECEIPT --start-device-window when the phone is ready.")
        targets = f"both iPhones ({device} → {prepared['nearby']['iphone']['device']})" if pair else "the iPhone"
        print(f"Only now: unlock {targets} and leave unused for at most {args.window_minutes} minutes.", flush=True)
        if input("Type START to install and test, or anything else to leave the phone untouched: ").strip() != "START":
            print("Cancelled. The phone was not contacted.")
            return 0
    if window_deadline is None:
        window_deadline = time.monotonic() + args.window_minutes * 60
    prepared = validate_prepared(prepared_path, device, prepared_sha256)
    window_seconds = int(window_deadline - time.monotonic()) - 5
    if window_seconds < 60:
        raise RuntimeError("Preparation consumed the window budget without starting tests. You can use the phone.")
    runs = cache / "runs"
    runs.mkdir(exist_ok=True)
    output = runs / (platform + "-" + prepared["mode"] + "-" + uuid.uuid4().hex)
    command = [sys.executable, str(SCRIPTS / "iphone-window.py"), "run", "--device", device,
               "--app-manifest", prepared["app"]["path"], "--runner-manifest", prepared["runner"]["path"],
               *workload_arguments(prepared), "--output", str(output),
               "--window-seconds", str(window_seconds), *mobile]
    if platform == "device":
        command.append("--start-device-window")
    print(f"Starting bounded {platform} run. Evidence: {output}", flush=True)
    # Give the helper its reserved remote-cleanup interval before the backstop.
    try:
        code = run_window(command, window_deadline)
    finally:
        result_path = output / "result.json"
        result = load(result_path) if result_path.is_file() else {}
        if cleanup_confirmed(result, prepared):
            if pair:
                receiver = prepared["nearby"]["iphone"]["device"]
                print(f"DEVICE RELEASED — source iPhone {device} and receiver iPhone {receiver}. Both automations stopped. Evidence: {output}", flush=True)
            elif platform == "device":
                print(f"DEVICE RELEASED — you can use your iPhone now. Automation stopped. Evidence: {output}", flush=True)
            else:
                print(f"SIMULATOR FINISHED — owned automation stopped; no physical phone contacted. Evidence: {output}", flush=True)
            print("Outcome: " + str(result.get("status", "unknown")).upper(), flush=True)
            if result.get("error"):
                print("Bug/failure: " + str(result["error"]).splitlines()[0], flush=True)
            if result.get("completed_steps"):
                print("Completed: " + ", ".join(result["completed_steps"]), flush=True)
        else:
            print(f"{platform} automation cleanup is NOT confirmed. Inspect {output}; do not treat this as a successful release.", file=sys.stderr, flush=True)
            code = 1
    return code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"iPhone command stopped: {error}", file=sys.stderr)
        raise SystemExit(1)
