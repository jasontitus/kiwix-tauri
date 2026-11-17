#!/usr/bin/env python3
"""Private one-run receiver controller for iphone-window; never a device framework."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import time
import uuid

from urllib.parse import quote

# Both ends use this contract. Child work expires before parent escalation,
# leaving time to reap a timed-out command and confirm the remote runner gone.
RECEIVER_PARENT_GRACE_SECONDS = 70
RECEIVER_CLEANUP_SECONDS = RECEIVER_PARENT_GRACE_SECONDS - 15
RECEIVER_FINAL_RUNNER_SECONDS = 30
RECEIVER_OWNER_RESERVE_SECONDS = 75
RECEIVER_TOTAL_RESERVE_SECONDS = RECEIVER_PARENT_GRACE_SECONDS + RECEIVER_OWNER_RESERVE_SECONDS


def receiver_cleanup_confirmed(result, device):
    receiver = result.get("iphone_receiver")
    if result.get("receiver_launch_attempted") is False:
        return receiver == {"device": device, "ownership": "not_started"}
    return (result.get("receiver_launch_attempted") is True
            and isinstance(receiver, dict) and receiver.get("device") == device
            and all(receiver.get(key) is True for key in (
                "cleanup_complete", "remote_runner_stopped", "host_processes_stopped")))

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    with Path(path).open("x") as stream:
        json.dump(value, stream, separators=(",", ":"))
        stream.write("\n")


def canonical_uuid(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value
    except ValueError:
        return False


def validate_observation(value, source, expected=None):
    """Discovery/session plus browse/opaque-offer authority, never an archive echo."""
    publication = value.get("publication", {})
    peer, inventory, offer = (value.get(key, {}) for key in ("selected_peer", "inventory", "offer"))
    if (not canonical_uuid(publication.get("publisher_id"))
            or not re.fullmatch(r"[0-9a-f]{32}", publication.get("session_id", ""))
            or type(publication.get("library_generation")) is not int
            or not 0 < publication["library_generation"] <= 2**53 - 1
            or (expected is not None and any(publication.get(key) != item for key, item in expected.items()))
            or peer.get("transport") != "quic-library"
            or peer.get("peerId") != publication["publisher_id"]
            or peer.get("libraryGeneration") != publication["library_generation"]
            or peer.get("librarySessionId") != publication["session_id"]
            or not peer.get("id")
            or inventory.get("sessionId") != publication["session_id"]
            or not canonical_uuid(inventory.get("receiptId"))
            or not isinstance(inventory.get("books"), list)):
        raise RuntimeError("Independent source discovery/session/browse authority is missing or rebound")
    candidates = [book for book in inventory["books"] if isinstance(book, dict) and (
        book.get("archiveId") == source["archive_id"] or book.get("fileName") == source["file_name"])]
    if (len(candidates) != 1 or candidates[0] != offer or not isinstance(offer.get("id"), str) or not offer["id"]
            or any(offer.get(key) != source[field] for key, field in (
                ("archiveId", "archive_id"), ("fileName", "file_name"), ("title", "title"), ("size", "size")))
            or offer.get("downloadIdentity") != "kiwix-library://public/" + publication["session_id"]
                + "/" + publication["publisher_id"] + "/" + quote(offer["id"], safe="")):
        raise RuntimeError("Independent source opaque offer does not match frozen archive/publication")
    return {key: value[key] for key in ("publication", "selected_peer", "inventory", "offer")}


def validate_request(value, now=None):
    keys = {"schema", "nonce", "deadline", "owner_deadline", "sender_device", "sender_app_manifest_sha256",
            "device", "app", "runner", "source", "pmd", "host_sha256", "peer_sha256"}
    now = time.time() if now is None else now
    if (not isinstance(value, dict) or set(value) != keys or value["schema"] != 1
            or not re.fullmatch(r"[0-9a-f]{32}", value.get("nonce", ""))
            or not isinstance(value.get("sender_device"), str) or not value["sender_device"]
            or value.get("device") == value["sender_device"]
            or not isinstance(value.get("device"), str) or not value["device"]
            or type(value["deadline"]) not in (int, float) or type(value["owner_deadline"]) not in (int, float)
            or not now < value["deadline"] < value["owner_deadline"] <= now + 600
            or value["owner_deadline"] - value["deadline"] < 55
            or not isinstance(value["source"], dict)
            or value["source"].get("direction") != "iphone-to-iphone"
            or value["source"].get("topology") != "apple-lan"
            or value["source"].get("operation", "transfer") != "transfer"):
        raise RuntimeError("Invalid private receiver request/deadline/device binding")
    for key in ("sender_app_manifest_sha256", "host_sha256", "peer_sha256"):
        if not re.fullmatch(r"[0-9a-f]{64}", value.get(key, "")):
            raise RuntimeError("Invalid private receiver source manifest binding")
    for key in ("app", "runner"):
        artifact = value[key]
        if (not isinstance(artifact, dict) or set(artifact) != {"path", "sha256"}
                or not isinstance(artifact["path"], str) or not Path(artifact["path"]).is_absolute()
                or not re.fullmatch(r"[0-9a-f]{64}", artifact.get("sha256", ""))
                or digest(artifact["path"]) != artifact["sha256"]):
            raise RuntimeError("Frozen receiver manifest bytes changed")
    if value["source"].get("iphone") != {key: value[key] for key in ("device", "app", "runner")}:
        raise RuntimeError("Receiver request changed prepared device/artifact identity")
    return value


def control_binding(request, request_sha256):
    return {"schema": 1, "nonce": request["nonce"], "deadline": request["deadline"],
            "device": request["device"], "request_sha256": request_sha256,
            "app_manifest_sha256": request["app"]["sha256"],
            "runner_manifest_sha256": request["runner"]["sha256"]}


class ReceiverControl:
    def __init__(self, request, request_sha256, stream=None):
        self.request = request
        self.control_sha256 = None
        self.cleanup_deadline = None
        self.binding = dict(request, request_sha256=request_sha256)
        self.control_binding = control_binding(request, request_sha256)
        self.stream = sys.stdin.buffer if stream is None else stream
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.stream, selectors.EVENT_READ)
        self.buffer = b""
        self.observation = None

    def begin_cleanup(self):
        deadline = min(self.request["owner_deadline"] - RECEIVER_OWNER_RESERVE_SECONDS,
                       time.time() + RECEIVER_CLEANUP_SECONDS)
        self.cleanup_deadline = deadline if self.cleanup_deadline is None else min(self.cleanup_deadline, deadline)
        return self.cleanup_deadline

    def poll(self):
        if time.time() >= self.request["deadline"]:
            raise TimeoutError("Shared two-phone action deadline expired")
        if not self.selector.select(0):
            return
        chunk = os.read(self.stream.fileno(), 65536)
        if not chunk:
            raise RuntimeError("Receiver controller disconnected; bounded cleanup required")
        self.buffer += chunk
        if len(self.buffer) > 1024 * 1024:
            raise RuntimeError("Receiver control exceeds finite private boundary")
        while b"\n" in self.buffer:
            line, self.buffer = self.buffer.split(b"\n", 1)
            value = json.loads(line)
            if (not isinstance(value, dict) or set(value) != set(self.control_binding) | {"command", "observation"}
                    or any(type(value.get(key)) is not type(item) or value.get(key) != item
                           for key, item in self.control_binding.items())
                    or value.get("command") != "receive" or self.observation is not None):
                raise RuntimeError("Stale, duplicate or unauthorized receiver control")
            self.observation = validate_observation(value["observation"], self.request["source"])
            self.control_sha256 = hashlib.sha256(line + b"\n").hexdigest()

    def ready_and_wait(self, window, status):
        save(window.output / "ready.json", dict(self.binding, status="ready",
            installed_identity=status["installed_identity"], sampler_readiness=status["sampler_readiness"],
            verified_container_transition=status["verified_container_transition"], receiver_selection_started=False))
        while self.observation is None:
            window.remaining()
            self.poll()
            time.sleep(.1)
        # Reject coalesced duplicate/partial trailing messages before any receiver UI.
        if self.buffer:
            raise RuntimeError("Trailing private receiver control bytes")
        return self.observation


class IPhonePeer:
    """Native secondary Window plus independent discovery-only Mac observer."""
    def __init__(self, window, source, nonce, sender, deadline, manifest_sha256, mac_peer_type, pmd):
        self.window, self.source = window, source
        self.output = window.output / "iphone-receive"
        self.observer = None
        self.process = None
        self.log = None
        self.observation = None
        self.observer_result = None
        receiver = source["iphone"]
        request = dict(schema=1, nonce=nonce, sender_device=sender, sender_app_manifest_sha256=manifest_sha256,
            deadline=deadline, owner_deadline=window.owner_deadline, **receiver, source=source, pmd=pmd,
            host_sha256=digest(Path(__file__).with_name("iphone-window.py")), peer_sha256=digest(__file__))
        validate_request(request)
        path = window.output / "iphone-receive-request.json"
        save(path, request)
        self.binding = dict(request, request_sha256=digest(path))
        self.control_binding = control_binding(request, self.binding["request_sha256"])
        self.log = (window.output / "iphone-receive.log").open("xb")
        try:
            self.process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                "--request", str(path), "--request-sha256", self.binding["request_sha256"],
                "--output", str(self.output)], stdin=subprocess.PIPE, stdout=self.log,
                stderr=subprocess.STDOUT, start_new_session=True)
            self.observer = mac_peer_type(window, source, nonce, sender, deadline, manifest_sha256, mode="observe")
        except BaseException:
            # The caller must retain this object to settle owned child cleanup.
            self.start_error = sys.exc_info()[1]
        else:
            self.start_error = None

    def receipt(self, name):
        path = self.output / name
        if not path.is_file():
            return None
        value = json.loads(path.read_bytes())
        if any(value.get(key) != item for key, item in self.binding.items()):
            raise RuntimeError("iPhone receipt differs from frozen nonce/device/artifacts/source/request")
        return value

    def check_alive(self):
        if self.start_error:
            raise RuntimeError("Paired iPhone/observer launch failed") from self.start_error
        if self.observer is not None and self.observer_result is None:
            self.observer.check_alive()
        if self.process is None:
            raise RuntimeError("Receiver host did not start")
        if self.process.poll() is not None:
            result = self.receipt("result.json")
            if (not result or result.get("status") != "passed"
                    or result.get("cleanup_complete") is not True or self.observation is None):
                raise RuntimeError("Receiver host stopped before successful source-bound proof; inspect its cleanup receipt")

    def wait_ready(self, observe=None):
        self.check_alive()
        ending = time.monotonic() + min(120, self.window.remaining() - 15)
        self.observer.wait_ready(observe)
        while True:
            self.check_alive()
            ready = self.receipt("ready.json")
            if ready is not None:
                if (ready.get("status") != "ready" or ready.get("receiver_selection_started") is not False
                        or ready.get("sampler_readiness", {}).get("status") != "ready"
                        or not ready.get("installed_identity") or not ready.get("verified_container_transition")):
                    raise RuntimeError("Secondary iPhone lacks preflight/install readiness before UI")
                self.readiness = dict(ready, observer_ready=self.observer.readiness)
                return
            if observe is not None:
                observe()
            self.window.remaining()
            if time.monotonic() >= ending:
                raise TimeoutError("Secondary iPhone did not become ready within shared window")
            time.sleep(.1)

    def receive(self, publication, observe=None):
        if self.observation is not None:
            raise RuntimeError("Receiver source selection is one-shot")
        self.observer.send(dict(command="observe", **publication))
        ending = time.monotonic() + min(45, self.window.remaining() - 15)
        while True:
            self.check_alive()
            receipt = self.observer.receipt("source-observed.json")
            if receipt is not None:
                if receipt.get("status") != "observed":
                    raise RuntimeError("Mac observer did not independently observe source")
                self.observation = validate_observation(receipt, self.source, publication)
                self.observer.publication = self.observation["publication"]
                break
            self.window.remaining()
            if observe is not None:
                observe()
            if time.monotonic() >= ending:
                raise TimeoutError("Source publication was not independently observed")
            time.sleep(.1)
        control = self.window.output / "iphone-receive-control.json"
        save(control, dict(self.control_binding, command="receive", observation=self.observation))
        self.control_sha256 = digest(control)
        self.process.stdin.write(control.read_bytes())
        self.process.stdin.flush()

    def result(self):
        self.check_alive()
        if self.process.poll() is None:
            return None
        result = self.receipt("result.json")
        proof = result.get("nearby_received", {}) if result else {}
        if (self.process.returncode or not result or result.get("status") != "passed"
                or not self.observation or result.get("source_observation") != self.observation
                or result.get("receiver_control_sha256") != self.control_sha256
                or result.get("cleanup_complete") is not True or result.get("phone_released") is not True
                or result.get("host_processes_stopped") is not True or result.get("remote_runner_stopped") is not True
                or result.get("receiver_work_retired") is not True
                or result.get("preserved_before_books_settings_tabs") is not True
                or proof.get("archive_id") != self.source["archive_id"] or proof.get("sha256") != self.source["sha256"]
                or proof.get("size") != self.source["size"] or proof.get("managed") is not True
                or proof.get("readable_ui") is not True or not result.get("iphone_received_job")
                or not (self.output / "reader.png").is_file()):
            raise RuntimeError("Secondary iPhone lacks real managed-byte/reader/job/preservation/cleanup proof")
        return result

    def retire_observer(self):
        """Settle live observation while the source still holds sender-ready."""
        if self.observer_result is None:
            observed = self.observer.stop(complete=True)
            if not observed or validate_observation(observed, self.source) != self.observation:
                raise RuntimeError("Observer final publication/offer changed")
            self.observer_result = observed
        return self.observer_result

    def stop(self, complete=False):
        errors = []
        try:
            if self.process is not None:
                if self.process.poll() is None:
                    self.process.send_signal(signal.SIGTERM)
                    try:
                        # Remote Window cleanup owns this interval; never kill its
                        # host group before it can stop the exact remote runner.
                        self.process.wait(timeout=min(RECEIVER_PARENT_GRACE_SECONDS, self.window.remaining(cleanup=True)))
                    except subprocess.TimeoutExpired:
                        errors.append("Secondary iPhone bounded remote cleanup timed out")
                if self.process.poll() is None:
                    os.killpg(self.process.pid, signal.SIGKILL)
                    self.process.wait(timeout=3)
                receipt = self.receipt("result.json")
                if (not receipt or receipt.get("cleanup_complete") is not True
                        or receipt.get("remote_runner_stopped") is not True
                        or receipt.get("host_processes_stopped") is not True
                        or receipt.get("receiver_work_retired") is not True):
                    errors.append("Secondary iPhone matching cleanup receipt is missing")
            else:
                errors.append("Secondary iPhone host launch was not confirmed")
        except BaseException as error:
            errors.append(str(error))
        finally:
            if self.process is not None:
                if self.process.poll() is None:
                    os.killpg(self.process.pid, signal.SIGKILL)
                    self.process.wait(timeout=3)
                # Reap descendants in the exact owned host session as well.
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.process.stdin.close()
            if self.log is not None:
                self.log.close()
            if self.observer is not None and self.observer_result is None:
                try:
                    if complete:
                        self.retire_observer()
                    else:
                        self.observer_result = self.observer.stop(complete=False)
                except BaseException as error:
                    errors.append(str(error))
        if errors:
            raise RuntimeError("; ".join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--request", type=Path, required=True)
    parser.add_argument("--request-sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raw = args.request.read_bytes()
    if hashlib.sha256(raw).hexdigest() != args.request_sha256:
        raise RuntimeError("Private receiver request bytes changed before child admission")
    request = validate_request(json.loads(raw))
    host = Path(__file__).with_name("iphone-window.py")
    if digest(host) != request["host_sha256"] or digest(__file__) != request["peer_sha256"]:
        raise RuntimeError("Private receiver implementation changed at child boundary")
    spec = importlib.util.spec_from_file_location("iphone_window_receiver", host)
    driver = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(driver)
    controller = ReceiverControl(request, args.request_sha256)
    run_args = argparse.Namespace(device=request["device"], platform="device", test_mode="nearby",
        app_manifest=Path(request["app"]["path"]), runner_manifest=Path(request["runner"]["path"]),
        nearby_json=request["source"], pmd=Path(request["pmd"]), output=args.output,
        window_seconds=request["owner_deadline"] - time.time(), order="AB", map_source=None,
        baseline_manifest=None, wiki_source=None, _iphone_receiver=controller)
    def interrupted(signum, frame):
        controller.begin_cleanup()
        raise RuntimeError("Paired receiver cancellation; stop owned remote runner")
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, interrupted)
    driver.run(run_args)


if __name__ == "__main__":
    main()
