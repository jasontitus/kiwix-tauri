# SPDX-License-Identifier: GPL-3.0-or-later
"""Host-only private receiver admission, publication authority and cancellation."""
import copy
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import sys
import unittest
from unittest.mock import Mock, patch

import iphone_peer as peer


class ReceiverBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        artifacts = {}
        for field in ("app", "runner"):
            artifact = self.root / (field + ".json")
            artifact.write_text("{}")
            artifacts[field] = {"path": str(artifact), "sha256": peer.digest(artifact)}
        self.source = {"direction": "iphone-to-iphone", "topology": "apple-lan", "operation": "transfer",
            "archive_id": "11111111-1111-4111-8111-111111111111", "file_name": "fixture.zim",
            "title": "Real fixture", "size": 1234, "sha256": "a" * 64,
            "iphone": dict(device="receiver", **artifacts)}
        self.request = dict(schema=1, nonce="b" * 32, deadline=time.time() + 200,
            owner_deadline=time.time() + 310, sender_device="sender", sender_app_manifest_sha256="c" * 64,
            **self.source["iphone"], source=self.source, pmd="/prepared/pmd",
            host_sha256="d" * 64, peer_sha256="e" * 64)
        self.publication = {"publisher_id": "22222222-2222-4222-8222-222222222222",
            "library_generation": 7, "session_id": "f" * 32}
        self.offer = {"id": "opaque/offer not archive UUID", "archiveId": self.source["archive_id"],
            "fileName": "fixture.zim", "title": "Real fixture", "size": 1234,
            "downloadIdentity": "kiwix-library://public/" + "f" * 32
                + "/22222222-2222-4222-8222-222222222222/opaque%2Foffer%20not%20archive%20UUID"}
        self.observation = {"publication": self.publication, "selected_peer": {
            "id": "discovery-route", "peerId": self.publication["publisher_id"], "transport": "quic-library",
            "libraryGeneration": 7, "librarySessionId": "f" * 32},
            "inventory": {"receiptId": "33333333-3333-4333-8333-333333333333", "sessionId": "f" * 32,
                "books": [self.offer]}, "offer": self.offer}

    def controller(self):
        reader, writer = os.pipe()
        stream = os.fdopen(reader, "rb", buffering=0)
        self.addCleanup(stream.close)
        self.addCleanup(os.close, writer)
        controller = peer.ReceiverControl(self.request, "a" * 64, stream)
        self.addCleanup(controller.selector.close)
        return controller, writer

    def control(self, controller):
        return dict(controller.control_binding, command="receive", observation=self.observation)

    def test_changed_manifest_and_same_device_rejected_before_receiver_admission(self):
        peer.validate_request(self.request)
        for mutation in ({"device": "sender"}, {"deadline": time.time() + 400},
                         {"owner_deadline": time.time() + 601}):
            with self.subTest(mutation=mutation), self.assertRaises(RuntimeError):
                peer.validate_request(dict(self.request, **mutation))
        Path(self.request["runner"]["path"]).write_text("changed runner")
        with self.assertRaisesRegex(RuntimeError, "manifest bytes changed"):
            peer.validate_request(self.request)

    def test_one_shot_control_pins_actual_discovery_and_opaque_offer_before_ui(self):
        controller, writer = self.controller()
        raw = (json.dumps(self.control(controller)) + "\n").encode()
        os.write(writer, raw)
        window = Mock(output=self.root)
        status = {"installed_identity": {"device_receipt": ["actual receiver"]},
            "sampler_readiness": {"status": "ready"}, "verified_container_transition": {"before": "a", "after": "b"}}
        observed = controller.ready_and_wait(window, status)
        self.assertEqual(observed, self.observation)
        self.assertEqual(controller.control_sha256, peer.hashlib.sha256(raw).hexdigest())
        self.assertIs(json.loads((self.root / "ready.json").read_bytes())["receiver_selection_started"], False)
        os.write(writer, raw)
        with self.assertRaisesRegex(RuntimeError, "duplicate"):
            controller.poll()

    def test_coalesced_duplicate_is_rejected_before_receiver_ui_release(self):
        controller, writer = self.controller()
        raw = (json.dumps(self.control(controller)) + "\n").encode()
        os.write(writer, raw + raw)
        with self.assertRaisesRegex(RuntimeError, "duplicate"):
            controller.poll()

    def test_rebound_nonce_device_artifact_and_deadline_never_release_receiver(self):
        for field in ("nonce", "device", "app_manifest_sha256", "runner_manifest_sha256", "deadline", "request_sha256"):
            controller, writer = self.controller()
            control = self.control(controller)
            control[field] = "rebound"
            os.write(writer, (json.dumps(control) + "\n").encode())
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, "unauthorized"):
                controller.poll()
            self.assertIsNone(controller.observation)

    def test_matching_archive_and_bytes_do_not_authorize_wrong_session_or_offer(self):
        peer.validate_observation(self.observation, self.source, self.publication)
        for field, key, value in (
            ("selected_peer", "librarySessionId", "0" * 32),
            ("selected_peer", "libraryGeneration", 8),
            ("inventory", "sessionId", "0" * 32),
            ("offer", "downloadIdentity", self.offer["downloadIdentity"].replace("f" * 32, "0" * 32)),
            ("offer", "id", self.source["archive_id"]),
        ):
            observation = copy.deepcopy(self.observation)
            observation[field][key] = value
            with self.subTest(field=field, key=key), self.assertRaises(RuntimeError):
                peer.validate_observation(observation, self.source, self.publication)
        observation = copy.deepcopy(self.observation)
        observation["inventory"]["books"].append(copy.deepcopy(self.offer))
        with self.assertRaisesRegex(RuntimeError, "opaque offer"):
            peer.validate_observation(observation, self.source)

    def test_controller_eof_and_shared_deadline_cancel_instead_of_selecting_receiver(self):
        controller, writer = self.controller()
        with patch.object(peer.time, "time", return_value=self.request["deadline"]):
            with self.assertRaises(TimeoutError):
                controller.poll()
        # A different pipe makes EOF real without a double-close cleanup.
        reader, writer = os.pipe()
        os.close(writer)
        with os.fdopen(reader, "rb", buffering=0) as stream:
            controller = peer.ReceiverControl(self.request, "a" * 64, stream)
            try:
                with self.assertRaisesRegex(RuntimeError, "disconnected"):
                    controller.poll()
                self.assertIsNone(controller.observation)
            finally:
                controller.selector.close()

    def test_early_cancellation_cleanup_bound_is_relative_capped_and_never_extended(self):
        controller, _ = self.controller()
        with patch.object(peer.time, "time", return_value=1000):
            deadline = controller.begin_cleanup()
        self.assertEqual(deadline, 1000 + peer.RECEIVER_CLEANUP_SECONDS)
        self.assertLess(deadline, 1000 + peer.RECEIVER_PARENT_GRACE_SECONDS)
        with patch.object(peer.time, "time", return_value=1020):
            self.assertEqual(controller.begin_cleanup(), deadline)
        controller.cleanup_deadline = None
        controller.request = dict(controller.request, owner_deadline=1100)
        with patch.object(peer.time, "time", return_value=1000):
            self.assertEqual(controller.begin_cleanup(), 1100 - peer.RECEIVER_OWNER_RESERVE_SECONDS)

    def test_observer_settles_while_source_live_then_cached_receipt_survives_withdrawal(self):
        spec = peer.importlib.util.spec_from_file_location("pair_window_boundary", Path(__file__).with_name("iphone-window.py"))
        driver = peer.importlib.util.module_from_spec(spec)
        spec.loader.exec_module(driver)
        source_live = self.root / "source-live"
        source_live.write_text("publishing")
        output = self.root / "observer"
        output.mkdir()
        receipt = dict(self.observation, nonce=self.request["nonce"], status="complete", cleanup_complete=True,
            preserved_before_books_settings_tabs=True, stopped=True, observed_source_stable=True, paired_complete=True)
        # A real one-shot control subprocess refuses a stop after source withdrawal.
        program = ("import json,pathlib,sys; control=json.loads(sys.stdin.readline()); "
                   "assert control==json.loads(sys.argv[1]); assert pathlib.Path(sys.argv[2]).is_file(); "
                   "pathlib.Path(sys.argv[3]).write_text(sys.argv[4])")
        observer = object.__new__(driver.MacPeer)
        observer.mode = "observe"
        observer.binding = {"nonce": self.request["nonce"]}
        observer.source = self.source
        observer.output = output
        observer.publication = self.publication
        observer.window = Mock()
        observer.window.remaining.return_value = 100
        observer.log = (self.root / "observer.log").open("wb")
        observer.process = subprocess.Popen([sys.executable, "-c", program,
            json.dumps(dict(command="stop", complete=True, nonce=self.request["nonce"])),
            str(source_live), str(output / "result.json"), json.dumps(receipt)],
            stdin=subprocess.PIPE, stdout=observer.log, stderr=subprocess.STDOUT, start_new_session=True)
        def cleanup_observer():
            if observer.process.poll() is None:
                observer.process.kill()
            observer.process.wait(timeout=3)
            observer.process.stdin.close()
            observer.log.close()
        self.addCleanup(cleanup_observer)
        host = object.__new__(peer.IPhonePeer)
        host.observer = observer
        host.observer_result = None
        host.source = self.source
        host.observation = self.observation
        self.assertEqual(host.retire_observer(), receipt)
        source_live.unlink()  # ordinary source Stop sharing happens only now
        host.window = observer.window
        host.process = Mock(pid=4321)
        host.process.poll.return_value = 0
        host.receipt = Mock(return_value=dict(cleanup_complete=True, remote_runner_stopped=True,
            host_processes_stopped=True, receiver_work_retired=True))
        host.log = Mock()
        with patch.object(peer.os, "killpg"):
            host.stop(complete=True)
        self.assertEqual(host.observer_result, receipt)


class ReceiverCleanupBoundaryTests(unittest.TestCase):
    def host(self, root, receipt):
        host = object.__new__(peer.IPhonePeer)
        host.window = Mock()
        host.window.remaining.return_value = 100
        host.process = Mock(pid=4321)
        host.process.poll.side_effect = [None, 0, 0]
        host.process.wait.return_value = 0
        host.receipt = Mock(return_value=receipt)
        host.log = Mock()
        host.observer = Mock()
        host.observer_result = None
        host.output = root
        return host

    def test_cancellation_gives_remote_cleanup_interval_before_exact_host_group_reap(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = dict(cleanup_complete=True, remote_runner_stopped=True, host_processes_stopped=True,
                           receiver_work_retired=True)
            host = self.host(Path(directory), receipt)
            sequence = []
            host.process.send_signal.side_effect = lambda sig: sequence.append(("signal", sig))
            host.process.wait.side_effect = lambda timeout: sequence.append(("wait", timeout))
            with patch.object(peer.os, "killpg", side_effect=lambda pid, sig: sequence.append(("reap", pid, sig))):
                host.stop()
            self.assertEqual(sequence, [("signal", signal.SIGTERM), ("wait", 70), ("reap", 4321, signal.SIGKILL)])
            host.observer.stop.assert_called_once_with(complete=False)

    def test_host_exit_without_remote_receipt_cannot_release_phone(self):
        with tempfile.TemporaryDirectory() as directory:
            host = self.host(Path(directory), dict(cleanup_complete=True, host_processes_stopped=True))
            with patch.object(peer.os, "killpg"), self.assertRaisesRegex(RuntimeError, "matching cleanup receipt"):
                host.stop()
            host.process.wait.assert_called_once_with(timeout=70)

    def test_failed_receiver_receipt_aborts_readiness_without_waiting_full_deadline(self):
        host = self.host(Path("/host-only"), dict(status="failed", cleanup_complete=True))
        host.start_error = None
        host.observation = None
        host.process.poll.side_effect = None
        host.process.poll.return_value = 1
        with self.assertRaisesRegex(RuntimeError, "stopped before successful"):
            host.wait_ready()
        host.observer.wait_ready.assert_not_called()

    def test_remote_runner_exit_without_application_work_retirement_cannot_release(self):
        host = self.host(Path("/host-only"), dict(cleanup_complete=True, remote_runner_stopped=True,
                                                host_processes_stopped=True, receiver_work_retired=False))
        with patch.object(peer.os, "killpg"), self.assertRaisesRegex(RuntimeError, "matching cleanup receipt"):
            host.stop()


if __name__ == "__main__":
    unittest.main()
