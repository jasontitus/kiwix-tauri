#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Frozen, off-device preparation and one bounded real Mac/Android LAN window.

Both helper preparations must already exist and have independent source review.
This controller never builds a production APK, installs one, or changes radios.
Mac reader evidence is headless Chromium, not native WKWebView or RF throughput.
"""
import argparse
import hashlib
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / "scripts"
HEX = re.compile(r"[a-f0-9]{64}")
CONFIG_KEYS = {"schema", "direction", "topology", "device", "source", "archive_id", "file_name",
               "title", "expected_text", "peer", "android_prepared", "mac_prepared", "window_seconds"}
FIXTURE_KEYS = ("source", "archive_id", "file_name", "title", "expected_text", "peer", "size", "sha256")

TIMING = None


def timed_operation(args, operation, category, prepared=None):
    global TIMING
    args.output.mkdir(parents=True, exist_ok=False, mode=0o700)
    timing_path = SCRIPTS / "testing_timing.py"
    expected = next(entry["sha256"] for entry in prepared["inputs"] if entry["path"] == str(timing_path)) if prepared else pin(timing_path)["sha256"]
    timing = TIMING = load_helper("testing_timing", expected).TimingLog(
        args.output.resolve(), os.environ.get("KIWIX_TIMING_ITERATION") or uuid.uuid4().hex, "mac-android")
    previous_iteration = os.environ.get("KIWIX_TIMING_ITERATION")
    os.environ["KIWIX_TIMING_ITERATION"] = timing.iteration_id
    try:
        with timing.span(category, "prepare" if prepared is None else "lan-window") as measured:
            code = operation(args) if prepared is None else operation(args, prepared)
            if code:
                measured.fail()
            return code
    finally:
        timing.close()
        TIMING = None
        if previous_iteration is None:
            os.environ.pop("KIWIX_TIMING_ITERATION", None)
        else:
            os.environ["KIWIX_TIMING_ITERATION"] = previous_iteration


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def read_json(path, maximum=1024 * 1024):
    with Path(path).open("rb") as stream:
        data = stream.read(maximum + 1)
    require(len(data) <= maximum, "JSON input exceeds size bound")
    value = json.loads(data)
    require(isinstance(value, dict), "JSON input must be an object")
    return value, data


def pin(path):
    """Hash a canonical regular file without accepting an in-flight replacement."""
    path = Path(path)
    require(path.is_absolute() and path == path.resolve(strict=True), "Input path is not canonical")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    if not stat.S_ISREG(os.fstat(fd).st_mode):
        os.close(fd)
        raise RuntimeError("Input is not a regular file")
    with os.fdopen(fd, "rb") as stream:
        before = os.fstat(stream.fileno())
        h = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
        after = os.fstat(stream.fileno())
    identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns)
    require(identity(before) == identity(after) == identity(path.lstat()),
            "Input changed while hashing")
    return {"path": str(path), "size": before.st_size, "sha256": h.hexdigest()}


def save(path, value):
    with Path(path).open("x") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")


def load_helper(name, expected_sha256=None):
    # Import only on an explicit operation; importing this controller contacts nothing.
    spec = importlib.util.spec_from_file_location("mac_android_" + name.replace("-", "_"), SCRIPTS / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(SCRIPTS))
    try:
        if expected_sha256 is None:
            spec.loader.exec_module(module)
        else:
            content = Path(spec.origin).read_bytes()
            require(hashlib.sha256(content).hexdigest() == expected_sha256,
                    "Frozen timing dependency changed before import")
            exec(compile(content, spec.origin, "exec"), module.__dict__)
    finally:
        sys.path.pop(0)
    return module


def canonical_uuid(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value
    except ValueError:
        return False


def validate_config(config):
    require(set(config) == CONFIG_KEYS and config["schema"] == 1, "Unexpected or missing config fields")
    require(config["direction"] in ("mac-to-android", "android-to-mac") and config["topology"] == "lan",
            "Explicit Mac/Android direction and LAN topology required")
    require(isinstance(config["device"], str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,159}", config["device"]),
            "Explicit safe physical adb serial required")
    require(type(config["window_seconds"]) is int and 120 <= config["window_seconds"] <= 600,
            "Window must be 120..600 seconds including cleanup")
    require(canonical_uuid(config["archive_id"]), "Canonical archive UUID required")
    require(isinstance(config["file_name"], str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._ -]{0,239}\.zim", config["file_name"]),
            "Frozen wire filename must be a single ASCII .zim leaf")
    for key in ("title", "expected_text", "peer"):
        value = config[key]
        require(isinstance(value, str) and value.strip() and len(value) <= (160 if key == "peer" else 4096)
                and not any(ord(c) < 32 or ord(c) == 127 for c in value), "Invalid frozen " + key)
    require(len(config["title"]) <= 500, "Android title exceeds native input bound")
    for key in ("source", "android_prepared", "mac_prepared"):
        require(isinstance(config[key], str) and Path(config[key]).is_absolute(), "Absolute " + key + " required")
        require(str(Path(config[key]).resolve(strict=True)) == config[key], "Canonical " + key + " required")


def validate_fixture(source):
    actual = pin(Path(source["source"]))
    require(actual["size"] == source["size"] and actual["sha256"] == source["sha256"], "Frozen fixture bytes changed")
    with Path(source["source"]).open("rb") as stream:
        header = stream.read(24)
    require(len(header) == 24 and header[:4] == b"ZIM\x04" and str(uuid.UUID(bytes=header[8:24])) == source["archive_id"],
            "Frozen fixture UUID does not match its real ZIM header")


def validate_android(source, android):
    receipt, content = read_json(source["android"]["prepared"])
    require(hashlib.sha256(content).hexdigest() == source["android"]["prepared_sha256"],
            "Frozen Android prepare receipt changed")
    require(receipt.get("schema") == 1 and receipt.get("target_package") == android.TARGET
            and receipt.get("instrumentation") == android.RUNNER
            and receipt.get("host_source_sha256") == source["android"]["tool_sha256"]
            and receipt.get("timing_tool_sha256") == pin(SCRIPTS / "testing_timing.py")["sha256"]
            and receipt.get("source_sha256") == android.sources(), "Android preparation differs from reviewed current helper/target")
    for field in ("target_apk_sha256", "tool_apk_sha256", "certificate_sha256"):
        require(isinstance(receipt.get(field), str) and HEX.fullmatch(receipt[field]), "Invalid Android preparation identity: " + field)
    for field in ("target_apk", "tool_apk"):
        require(pin(Path(receipt[field]))["sha256"] == receipt[field + "_sha256"], "Reviewed Android APK changed: " + field)
    android.reviewed_crash_identity(argparse.Namespace(reviewed_crash_sha256=None), receipt)
    return receipt


def prepare(args):
    return timed_operation(args, prepare_impl, "host_prepare")


def prepare_impl(args):
    require(args.config is not None and args.prepared is None and not args.start_device_window,
            "Preparation requires only an explicit ignored --config")
    config_path = args.config.resolve(strict=True)
    relative = config_path.relative_to(ROOT)
    ignored = subprocess.run(["git", "check-ignore", "--quiet", "--", str(relative)], cwd=ROOT,
                             timeout=10, check=False)
    require(ignored.returncode == 0, "Local config must be explicitly git-ignored")
    config, config_bytes = read_json(config_path)
    validate_config(config)
    window = load_helper("iphone-window")
    android = load_helper("android-nearby")
    source = {key: config[key] for key in FIXTURE_KEYS if key in config}
    fixture = pin(Path(config["source"]))
    source.update(size=fixture["size"], sha256=fixture["sha256"])
    validate_fixture(source)
    for name, tool in (("mac", SCRIPTS / "mac-peer.mjs"), ("android", SCRIPTS / "android-nearby.py")):
        source[name] = {"tool": str(tool), "tool_sha256": pin(tool)["sha256"],
                        "prepared": config[name + "_prepared"],
                        "prepared_sha256": pin(Path(config[name + "_prepared"]))["sha256"]}
    source["android"]["device"] = config["device"]
    mac_receipt = window.validate_mac_prepared(source["mac"])
    android_receipt = validate_android(source, android)
    # Pin host entrypoints, runtime, dependencies and existing lockfiles. No build,
    # SDK setup, adb invocation or device enumeration belongs in preparation.
    paths = {config_path, Path(__file__).resolve(), SCRIPTS / "iphone-window.py", SCRIPTS / "iphone_memory.py",
             SCRIPTS / "iphone_peer.py", SCRIPTS / "testing_timing.py",
             SCRIPTS / "android-nearby.py", SCRIPTS / "mac-peer.mjs", ROOT / "package.json", ROOT / "package-lock.json",
             ROOT / "src-tauri/Cargo.lock", ROOT / "node_modules/puppeteer-core/package.json",
             Path(sys.executable).resolve(), Path(mac_receipt["runtime"]["node"]["path"])}
    for entry in (source["mac"], source["android"]):
        paths.add(Path(entry["prepared"]))
    for field in ("helper", "lockfile", "executable", "browser"):
        paths.add(Path(mac_receipt[field]["path"]))
    for field in ("target_apk", "tool_apk"):
        paths.add(Path(android_receipt[field]))
    for value in android_receipt["tools"].values():
        paths.add(Path(value).resolve(strict=True))
    for name in android.sources():
        paths.add(SCRIPTS / "android-nearby" / name)
    # apksigner is a launcher; its actual signature verifier is a separate jar.
    signer = Path(android_receipt["tools"]["apksigner"])
    paths.add((signer.parent / "lib/apksigner.jar").resolve(strict=True))
    pins = [pin(path.resolve(strict=True)) for path in sorted(paths, key=str)]
    require(pin(config_path)["sha256"] == hashlib.sha256(config_bytes).hexdigest(), "Config changed during preparation")
    receipt = {"schema": 1, "status": "ready", "direction": config["direction"], "topology": "lan",
               "device": config["device"], "window_seconds": config["window_seconds"], "source": source,
               "config": {"path": str(config_path), "sha256": hashlib.sha256(config_bytes).hexdigest()},
               "inputs": pins, "mac_identity": mac_receipt, "android_identity": android_receipt,
               "node_dependencies": {"path": str((ROOT / "node_modules").resolve(strict=True)),
                                     "inventory_sha256": window.tree_hash(window.tree(ROOT / "node_modules"))},
               "python": {"executable": str(Path(sys.executable).resolve()), "version": sys.version}}
    path = args.output.resolve() / "prepare.json"
    save(path, receipt)
    print(json.dumps({"status": "ready", "prepared": str(path), "prepared_sha256": pin(path)["sha256"],
                      "device_contact": False}))
    return 0


def load_frozen(path, expected, device):
    require(isinstance(expected, str) and HEX.fullmatch(expected), "Exact --prepared-sha256 required before device contact")
    # Hash before parsing: even semantically equivalent JSON is a different handoff.
    raw = Path(path).read_bytes()
    require(len(raw) <= 4 * 1024 * 1024 and hashlib.sha256(raw).hexdigest() == expected,
            "Frozen controller prepare bytes changed; refusing device contact")
    receipt = json.loads(raw)
    require(receipt.get("schema") == 1 and receipt.get("status") == "ready" and receipt.get("topology") == "lan"
            and receipt.get("direction") in ("mac-to-android", "android-to-mac"), "Invalid frozen controller preparation")
    require(device == receipt.get("device"), "Explicit adb serial differs from frozen preparation")
    require(type(receipt.get("window_seconds")) is int and 120 <= receipt["window_seconds"] <= 600, "Invalid frozen window duration")
    require(receipt.get("python") == {"executable": str(Path(sys.executable).resolve()), "version": sys.version},
            "Prepared Python runtime changed")
    for expected_pin in receipt["inputs"]:
        require(pin(Path(expected_pin["path"])) == expected_pin, "Frozen input changed: " + expected_pin["path"])
    require(pin(SCRIPTS / "testing_timing.py") in receipt["inputs"],
            "Frozen timing dependency missing or changed; prepare again")
    config, raw_config = read_json(receipt["config"]["path"])
    require(hashlib.sha256(raw_config).hexdigest() == receipt["config"]["sha256"], "Frozen config changed")
    dependencies = receipt["node_dependencies"]
    helper = load_helper("iphone-window")
    require(str((ROOT / "node_modules").resolve(strict=True)) == dependencies["path"]
            and helper.tree_hash(helper.tree(Path(dependencies["path"]))) == dependencies["inventory_sha256"],
            "Frozen installed host dependencies changed")
    validate_config(config)
    source = receipt["source"]
    require(all(config[key] == receipt[key] for key in ("direction", "device", "topology", "window_seconds"))
            and all(config[key] == source[key] for key in FIXTURE_KEYS if key not in ("size", "sha256")),
            "Prepared config/source binding changed")
    validate_fixture(source)
    return receipt


def validate_radio(radio, baseline=None):
    require(isinstance(radio, dict) and radio.get("sta_connected") is True and radio.get("sta_disabled") is False
            and radio.get("no_ap_verified") is True and radio.get("current_ap_verified") is False
            and isinstance(radio.get("station_identity_sha256"), str) and HEX.fullmatch(radio["station_identity_sha256"]),
            "LAN receipt lacks actual connected station/no-AP identity")
    address = ipaddress.IPv4Address(radio.get("station_ipv4", ""))
    require(not address.is_loopback and not address.is_unspecified and not address.is_multicast and not address.is_link_local,
            "LAN receipt lacks usable actual station IPv4")
    if baseline is not None:
        require(all(radio.get(key) == baseline.get(key) for key in ("station_identity_sha256", "station_ipv4")),
                "LAN station changed during owned window")
    return radio


def validate_publication(value, session=False):
    require(isinstance(value, dict) and canonical_uuid(value.get("publisher_id"))
            and type(value.get("library_generation")) is int and 0 < value["library_generation"] < 2**53,
            "Missing actual source publication peer/generation")
    if session:
        require(isinstance(value.get("session_id"), str) and re.fullmatch(r"[a-f0-9]{32}", value["session_id"]),
                "Missing actual source publication session")
    else:
        require(set(value) == {"publisher_id", "library_generation"}, "Mobile source cannot invent a discovery session")
    return value


def validate_android_job(result, source, publication):
    """Bind independently verified managed bytes to this live Mac session, not its label."""
    journal = result.get("received_journal", {})
    job = journal.get("matched_job", {})
    expected_url = "kiwix-library://public/{session_id}/{publisher_id}/".format(**publication) + source["archive_id"]
    require(journal.get("status") == "matched" and journal.get("match_count") == 1
            and isinstance(journal.get("new_rows"), list) and job in journal["new_rows"]
            and canonical_uuid(job.get("id")) and job.get("journal_key") == job["id"]
            and job.get("matches_independent_received_bytes") is True and job.get("url") == expected_url
            and job.get("transport") == "quic-library" and not job.get("error") and not job.get("importError")
            and job.get("fileName") == source["file_name"] and job.get("status") == "complete"
            and job.get("downloaded") == source["size"] and job.get("total") == source["size"]
            and job.get("publication") == {"size": source["size"], "sha256": source["sha256"]}
            and isinstance(result.get("managed_path"), str) and job.get("destination") == result["managed_path"]
            and canonical_uuid(result.get("installation_id")) and job.get("consumed_acquisition") is True
            and job.get("installation_id") == result["installation_id"],
            "Received Android job is not the new consumed managed installation from this actual Mac publication")
    return job


def validate_mac_link(result, source, publication):
    actual = validate_publication(result.get("publication"), session=True)
    require(all(actual.get(key) == value for key, value in publication.items()), "Mac receiver selected a different Android source")
    peer, inventory, offer, job = (result.get(key, {}) for key in ("selected_peer", "inventory", "offer", "job"))
    expected_url = "kiwix-library://public/{session_id}/{publisher_id}/".format(**actual) + str(offer.get("id", ""))
    require(peer.get("peerId") == actual["publisher_id"] and peer.get("libraryGeneration") == actual["library_generation"]
            and peer.get("librarySessionId") == actual["session_id"] and peer.get("transport") == "quic-library"
            and canonical_uuid(inventory.get("receiptId")) and inventory.get("sessionId") == actual["session_id"]
            and offer in inventory.get("books", []) and isinstance(offer.get("id"), str) and offer["id"]
            and offer.get("archiveId") == source["archive_id"] and offer.get("fileName") == source["file_name"]
            and offer.get("title") == source["title"] and offer.get("size") == source["size"]
            and offer.get("downloadIdentity") == expected_url and job.get("url") == expected_url
            and job.get("transport") == "quic-library" and canonical_uuid(job.get("id"))
            and job.get("publication") == {"size": source["size"], "sha256": source["sha256"]},
            "Mac received bytes lack actual Android discovery/session/browse/job linkage")
    return actual


class Window:
    def __init__(self, output, seconds):
        self.output, self.seconds = output, seconds
        self.started = time.monotonic()
        self.publisher = None

    def remaining(self, cleanup=False):
        if not cleanup and self.publisher is not None:
            self.publisher.check_alive()
        left = self.seconds - (3 if cleanup else 60) - (time.monotonic() - self.started)
        require(left > 0, "Bounded LAN window deadline; stop and clean owned work, never retry")
        return left


class AndroidPeer:
    def __init__(self, window, source, identity, nonce, sender, deadline, mode):
        self.window, self.source, self.identity, self.mode = window, source, identity, mode
        self.output = window.output / ("android-" + mode)
        self.binding = {key: source[key] for key in ("archive_id", "file_name", "title", "size", "sha256", "peer")}
        self.binding.update(topology="lan", device=source["android"]["device"], mode=mode, nonce=nonce,
                            deadline=deadline, sender_device=sender, reviewed_crash_sha256=identity.get("reviewed_crash_sha256"))
        argv = [sys.executable, source["android"]["tool"], "--prepared", source["android"]["prepared"],
                "--prepared-sha256", source["android"]["prepared_sha256"], "--output", str(self.output),
                "--adb", identity["tools"]["adb"], "--android-sdk", str(Path(identity["tools"]["android_jar"]).parents[2]),
                "--build-tools", Path(identity["tools"]["apksigner"]).parent.name]
        for key, value in self.binding.items():
            if value is not None and (key != "reviewed_crash_sha256" or value):
                argv += ["--" + key.replace("_", "-"), str(value)]
        self.log = (window.output / ("android-" + mode + ".log")).open("xb")
        try:
            self.process = subprocess.Popen(argv, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        except BaseException:
            self.log.close()
            raise
        self.readiness = None

    def receipt(self, name):
        path = self.output / name
        if not path.is_file():
            return None
        value, _ = read_json(path, 16 * 1024 * 1024)
        require(all(value.get(key) == expected for key, expected in self.binding.items()), "Android receipt changed frozen source/device/run/topology")
        return value

    def check_alive(self):
        require(self.process.poll() is None, "Android publisher stopped before receiver proof/preservation completed")

    def validate_identity(self, value):
        identity = value.get("installed_identity", {})
        require(identity.get("package") == self.identity["target_package"] and identity.get("debuggable") is False
                and identity.get("apk_sha256") == self.identity["target_apk_sha256"]
                and identity.get("certificate_sha256") == self.identity["certificate_sha256"],
                "Android receipt lacks actual installed reviewed target identity")

    def validate_source(self, value):
        require(canonical_uuid(value.get("installation_id")), "Android publisher lacks actual source installation")
        if value.get("source_kind") == "managed":
            require(value.get("source_bytes_verified") is True and value.get("managed") is True
                    and value.get("receivedhash") == self.source["sha256"] and value.get("receivedsize") == self.source["size"]
                    and value.get("source_sha256") == self.source["sha256"], "Android managed source lacks actual full-byte authority")
        else:
            require(value.get("source_kind") == "provider" and value.get("source_bytes_verified") is False
                    and value.get("managed") is False and value.get("source_metadata") == {
                        "archive_id": self.source["archive_id"], "title": self.source["title"], "size": self.source["size"],
                        "installation_id": value["installation_id"]}, "Android provider source lacks actual retained installation authority")

    def wait_ready(self, observe):
        end = time.monotonic() + min(45, self.window.remaining())
        while True:
            self.check_alive()
            observe()
            ready = self.receipt("ready.json")
            if ready is not None:
                require(ready.get("status") == "ready" and ready.get("topology") == "lan", "Android LAN readiness failed")
                self.validate_identity(ready)
                initial = validate_radio(ready.get("initial_radio"))
                validate_radio(ready.get("station_admission"), initial)
                if self.mode == "publish":
                    validate_radio(ready.get("route_radio"), initial)
                    require(ready.get("publisher_ready") is True, "Android source is not publishing")
                    validate_publication(ready.get("publication"))
                    self.validate_source(ready)
                self.readiness = ready
                return
            self.window.remaining()
            require(time.monotonic() < end, "Android LAN peer did not become ready")
            time.sleep(.2)

    def result(self):
        if self.process.poll() is None:
            return None
        value = self.receipt("result.json")
        require(value is not None, "Android peer exited without a bound result receipt")
        require(self.process.returncode == 0 and value.get("status") == "complete",
                "Android " + self.mode + " failed: " + str(value.get("error") or "peer exited without successful completion"))
        require(value.get("cleanup_complete") is True and value.get("preserved_before_books_settings_tabs") is True
                and value.get("crash_report_preserved") is True, "Android result lacks successful cleanup/preservation")
        self.validate_identity(value)
        require(self.readiness is not None, "Android completion preceded source-bound readiness")
        initial = self.readiness["initial_radio"]
        for key in ("station_admission", "initial_radio", "route_radio", "final_radio"):
            validate_radio(value.get(key), initial)
        if self.mode == "publish":
            self.validate_source(value)
            require(value.get("publication") == self.readiness["publication"]
                    and all(value.get(key) is True for key in ("publisher_ready", "publication_stopped", "owned_sharing_stopped"))
                    and value.get("publication_stop_reason") == "STOP_PUBLISH", "Android publisher lacks deliberate owned UI stop")
        else:
            require(value.get("receivedhash") == self.source["sha256"] and value.get("receivedsize") == self.source["size"]
                    and value.get("activebookid") == self.source["archive_id"] and value.get("managed") is True
                    and value.get("readable_ui") is True and (self.output / "reader.png").is_file(),
                    "Android receiver lacks managed full hash and actual active reader proof")
        reviewed = self.identity.get("reviewed_crash_sha256")
        if reviewed:
            closed = value.get("reviewed_crash_closed", {})
            require(closed.get("sha256") == reviewed and closed.get("ordinary_ui_close") is True
                    and all((self.output / name).is_file() for name in ("crash-before-close.png", "crash-after-close.png")),
                    "Exact reviewed crash lacks actual ordinary View/Close evidence")
        return value

    def stop(self, complete=False):
        try:
            require(not complete or self.process.poll() is None, "Android publisher stopped before deliberate owned stop")
            if self.process.poll() is None:
                # SIGTERM alone means STOP_PUBLISH; interruption must never claim paired success.
                self.process.send_signal(signal.SIGTERM if complete else signal.SIGINT)
                self.process.wait(timeout=min(30, self.window.remaining(cleanup=True)))
            value = self.receipt("result.json")
            require(value and value.get("cleanup_complete") is True, "Android owned cleanup is unconfirmed")
            if complete:
                return self.result()
        finally:
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=2)
            self.log.close()


def run(args):
    require(args.start_device_window and args.config is None and args.prepared is not None and args.device,
            "Run requires frozen --prepared/--prepared-sha256, exact --device and --start-device-window")
    prepared = load_frozen(args.prepared, args.prepared_sha256, args.device)
    return timed_operation(args, run_impl, "device_automation", prepared)


def run_impl(args, prepared):
    source = prepared["source"]
    helper = load_helper("iphone-window")
    # Complete all offline receipt/runtime admission before the first adb child.
    mac_identity = helper.validate_mac_prepared(source["mac"])
    android_identity = validate_android(source, load_helper("android-nearby"))
    require(mac_identity == prepared["mac_identity"] and android_identity == prepared["android_identity"], "Frozen helper identities changed")
    window = Window(args.output.resolve(), prepared["window_seconds"])
    nonce = uuid.uuid4().hex
    # Reserve cleanup inside the ten-minute wall bound, including helper teardown.
    deadline = int(time.time() + prepared["window_seconds"] - 25)
    result = {"schema": 1, "direction": prepared["direction"], "topology": "lan", "device": args.device,
              "nonce": nonce, "prepared_sha256": args.prepared_sha256, "status": "failed", "transfer_complete": False,
              "cleanup_complete": False, "device_released": False,
              "proof_surface": "real-android-ui-and-mac-headless-chromium-not-native-wkwebview"}
    mac = android = None
    errors = []
    previous = {}

    def interrupted(signum, frame):
        raise RuntimeError("LAN window interrupted; clean only owned work")

    for signum in (signal.SIGINT, signal.SIGTERM):
        previous[signum] = signal.signal(signum, interrupted)
    try:
        publishing_mac = prepared["direction"] == "mac-to-android"
        mac = helper.MacPeer(window, source, nonce, args.device, deadline, android_identity["target_apk_sha256"],
                             mode="publish" if publishing_mac else "receive", mobile_platform="android")
        mac.wait_ready()
        sender = mac.publication["publisher_id"] if publishing_mac else nonce
        android = AndroidPeer(window, source, android_identity, nonce, sender, deadline,
                              "receive" if publishing_mac else "publish")
        android.wait_ready(mac.check_alive)
        publisher, receiver = (mac, android) if publishing_mac else (android, mac)
        window.publisher = publisher
        if not publishing_mac:
            mac.receive(android.readiness["publication"])
        while True:
            window.remaining()
            received = receiver.result()
            if received is not None:
                publisher.check_alive()
                if publishing_mac:
                    result["received_job"] = validate_android_job(received, source, mac.publication)
                    result["publication"] = mac.publication
                else:
                    result["publication"] = validate_mac_link(received, source, android.readiness["publication"])
                result["receiver_result"] = received
                break
            time.sleep(.2)
        publisher.check_alive()
        window.publisher = None
        result["publisher_result"] = publisher.stop(complete=True)
        result["transfer_complete"] = True
    except BaseException as error:
        result["error"] = type(error).__name__ + ": " + str(error)
    finally:
        window.publisher = None
        # A second interrupt must not skip bounded cleanup or manufacture release.
        for signum in previous:
            signal.signal(signum, signal.SIG_IGN)
        for name, peer in (("android", android), ("mac", mac)):
            if peer is None:
                continue
            try:
                with TIMING.span("cleanup", "peer-stop"):
                    peer.stop()
                receipt = peer.receipt("result.json")
                require(receipt and receipt.get("cleanup_complete") is True, name + " cleanup receipt missing")
                result[name + "_cleanup"] = {"cleanup_complete": True, "process_reaped": peer.process.poll() is not None}
            except BaseException as error:
                errors.append(name + ": " + str(error))
        result["cleanup_errors"] = errors
        result["cleanup_complete"] = not errors
        result["device_released"] = not errors and (android is None or android.process.poll() is not None)
        result["duration_seconds"] = time.monotonic() - window.started
        if result["duration_seconds"] > prepared["window_seconds"]:
            result["cleanup_complete"] = False
            result["device_released"] = False
            errors.append("Owned cleanup exceeded frozen window bound")
        result["status"] = "complete" if result["transfer_complete"] and result["cleanup_complete"] else "failed"
        result["timing"] = TIMING.summary()
        result["timing_summary_path"] = str(window.output / "timing-summary.json")
        save(window.output / "result.json", result)
        for signum, handler in previous.items():
            signal.signal(signum, handler)
    print(json.dumps({"status": result["status"], "transfer_complete": result["transfer_complete"],
                      "cleanup_complete": result["cleanup_complete"], "device_released": result["device_released"],
                      "result": str(window.output / "result.json")}))
    return 0 if result["status"] == "complete" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    operation = parser.add_mutually_exclusive_group(required=True)
    operation.add_argument("--prepare-only", action="store_true")
    operation.add_argument("--start-device-window", action="store_true")
    parser.add_argument("--config", type=Path)
    parser.add_argument("--prepared", type=Path)
    parser.add_argument("--prepared-sha256")
    parser.add_argument("--device")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        return prepare(args) if args.prepare_only else run(args)
    except Exception as error:
        print("mac-android: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
