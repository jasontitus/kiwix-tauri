# SPDX-License-Identifier: GPL-3.0-or-later
"""Host-only process cancellation regression; never contacts an Apple device."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

from unittest.mock import patch
import iphone
from testing_timing import TimingLog
from iphone import REPO, TOOL_FILES, cleanup_confirmed, run_window, sha256, validate_prepared

# The phone window runs on a macOS host. Process groups, signal masks and
# POSIX signals do not exist on Windows, so these tests are skipped there.
posix_host = unittest.skipUnless(os.name == "posix", "requires a POSIX host (process groups, signal masks)")


@posix_host
class PhoneWindowDeadlineTests(unittest.TestCase):
    def test_deadline_kills_descendant_that_ignores_term_after_leader_exits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_file = root / "descendant.pid"
            survived = root / "survived"
            # The descendant exits on its own even when exercising a broken
            # implementation; cleanup never signals a potentially reused PID.
            descendant = (
                "import os,signal,time;from pathlib import Path;"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                f"Path({str(pid_file)!r}).write_text(str(os.getpid()));"
                "time.sleep(6);"
                f"Path({str(survived)!r}).write_text('continued after deadline')"
            )
            leader = (
                "import subprocess,sys,time;"
                f"subprocess.Popen([sys.executable,'-c',{descendant!r}]);"
                "time.sleep(30)"
            )
            started = time.monotonic()
            with self.assertRaises(RuntimeError):
                run_window([sys.executable, "-c", leader], started + 5, cleanup_seconds=1)
            self.assertLess(time.monotonic() - started, 5.5)
            self.assertTrue(pid_file.exists(), "The stubborn descendant must actually have started")
            time.sleep(max(0, started + 6.5 - time.monotonic()))
            self.assertFalse(survived.exists(), "No descendant may keep running the phone workflow after cancellation")

    def test_sigterm_and_repeated_interrupts_complete_group_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready = root / "ready"
            survived = root / "survived"
            descendant = (
                "import signal,time;from pathlib import Path;"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                f"Path({str(ready)!r}).write_text('ready');"
                "time.sleep(6);"
                f"Path({str(survived)!r}).write_text('continued after cancellation')"
            )
            leader = (
                "import signal,subprocess,sys,time;"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                f"subprocess.Popen([sys.executable,'-c',{descendant!r}]);"
                "time.sleep(8)"
            )
            controller = (
                "import sys,time;"
                f"sys.path.insert(0,{str(Path(__file__).resolve().parent)!r});"
                "from iphone import run_window\n"
                "try:\n"
                f"    run_window([sys.executable,'-c',{leader!r}],time.monotonic()+30,cleanup_seconds=1)\n"
                "except RuntimeError:\n"
                "    sys.exit(1)\n"
            )
            started = time.monotonic()
            with subprocess.Popen([sys.executable, "-c", controller]) as worker:
                while not ready.exists() and time.monotonic() - started < 4:
                    time.sleep(.02)
                self.assertTrue(ready.exists(), "Wait for the real child before cancelling")
                worker.send_signal(signal.SIGTERM)
                time.sleep(.2)
                worker.send_signal(signal.SIGINT)
                time.sleep(.5)
                worker.send_signal(signal.SIGINT)
                self.assertEqual(worker.wait(timeout=4), 1, "Cancellation must use cleanup, not default signal death")
            self.assertLess(time.monotonic() - started, 5.5)
            time.sleep(max(0, started + 6.5 - time.monotonic()))
            self.assertFalse(survived.exists(), "Repeated interrupts must not leave a descendant running")

    def test_cooperative_cleanup_can_finish_after_one_second(self):
        with tempfile.TemporaryDirectory() as directory:
            cleaned = Path(directory) / "cleanup-complete"
            helper = (
                "import signal,sys,time;from pathlib import Path\n"
                "def cleanup(signum, frame):\n"
                "    time.sleep(1.3)\n"
                f"    Path({str(cleaned)!r}).write_text('complete')\n"
                "    sys.exit(0)\n"
                "signal.signal(signal.SIGTERM,cleanup)\n"
                "time.sleep(30)\n"
            )
            started = time.monotonic()
            with self.assertRaises(RuntimeError):
                run_window([sys.executable, "-c", helper], started + 5, cleanup_seconds=3)
            self.assertTrue(cleaned.exists(), "Give the helper time to confirm cleanup before forced killing")
            self.assertLess(time.monotonic() - started, 5.5)

    def test_nonzero_window_process_retains_failure_in_timing_ledger(self):
        with tempfile.TemporaryDirectory() as directory:
            timing = TimingLog(Path(directory), "nonzero-window", "iphone-test")
            with patch.object(iphone, "TIMING", timing):
                try:
                    code = run_window([sys.executable, "-c", "import sys;sys.exit(19)"],
                                      time.monotonic() + 10, cleanup_seconds=1)
                finally:
                    timing.close()
            self.assertEqual(code, 19)
            self.assertEqual(timing.summary()["failed_operations"], 1)


class PreparedAuthorizationTests(unittest.TestCase):
    def test_valid_replacement_receipt_cannot_change_authorized_workload(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "artifact.json"
            artifact.write_text(json.dumps({"schema": 1, "status": "ready", "platform": "device"}))
            fixture = root / "fixture.zim"
            fixture.write_bytes(b"fixture bytes")
            manifest = {"path": str(artifact), "sha256": sha256(artifact)}
            source = {"path": str(fixture), "sha256": sha256(fixture)}
            receipt = {"schema": 3, "platform": "device", "device": "test-device", "mode": "comparison", "order": "AB",
                       "tool_sha256": {str(path.relative_to(REPO)): sha256(path) for path in TOOL_FILES},
                       "app": manifest, "baseline": manifest, "runner": manifest,
                       "wiki_source": source, "map_source": source}
            path = root / "prepared.json"
            path.write_text(json.dumps(receipt))
            original = path.read_bytes()
            authorized = sha256(path)
            self.assertEqual(validate_prepared(path, "test-device", authorized)["order"], "AB")
            receipt["order"] = "BA"
            path.write_text(json.dumps(receipt))
            self.assertEqual(validate_prepared(path, "test-device", sha256(path))["order"], "BA",
                             "The replacement is valid on its own, not merely corrupt JSON")
            with self.assertRaisesRegex(ValueError, "changed after selection"):
                validate_prepared(path, "test-device", authorized)
            path.write_bytes(original)
            self.assertEqual(validate_prepared(path, "test-device", authorized)["order"], "AB")
            receipt["platform"] = "simulator"
            path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError, "artifact platform"):
                validate_prepared(path, "test-device", sha256(path))

    def test_pair_rejects_receiver_rebinding_and_changed_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            artifacts = {}
            for name in ("app", "runner", "receiver-app", "receiver-runner"):
                manifest = root / (name + ".json")
                manifest.write_text(json.dumps({"schema": 1, "status": "ready", "platform": "device",
                                                "source_commit": "a" * 40}))
                artifacts[name] = {"path": str(manifest), "sha256": sha256(manifest)}
            fixture = root / "fixture.zim"
            fixture.write_bytes(b"identity validation unit fixture")
            helper = root / "mac-helper"
            helper.write_bytes(b"frozen helper identity")
            source_device = "00008150-000B0B000000001C"
            receiver_device = "00008110-000A11CE0000001E"
            receipt = {
                "schema": 3, "platform": "device", "device": source_device, "mode": "nearby", "order": "AB",
                "tool_sha256": {str(path.relative_to(REPO)): sha256(path) for path in TOOL_FILES},
                "app": artifacts["app"], "runner": artifacts["runner"],
                "map_source": {"path": str(fixture), "sha256": sha256(fixture)},
                "nearby": {
                    "direction": "iphone-to-iphone", "topology": "apple-lan", "operation": "transfer",
                    "source": str(fixture), "sha256": sha256(fixture), "size": fixture.stat().st_size,
                    "mac": {"tool": str(helper), "tool_sha256": sha256(helper),
                            "prepared": str(helper), "prepared_sha256": sha256(helper)},
                    "iphone": {"device": receiver_device, "app": artifacts["receiver-app"],
                               "runner": artifacts["receiver-runner"]},
                },
            }
            path = root / "prepared.json"

            def validate():
                path.write_text(json.dumps(receipt))
                return validate_prepared(path, source_device, sha256(path))

            self.assertEqual(validate()["nearby"]["iphone"]["device"], receiver_device)
            receipt["nearby"]["iphone"]["device"] = source_device.lower()
            with self.assertRaisesRegex(ValueError, "two distinct"):
                validate()
            receipt["nearby"]["iphone"]["device"] = receiver_device
            receipt["nearby"]["iphone"]["config"] = "/mutable/receiver.json"
            with self.assertRaisesRegex(ValueError, "exact device artifacts"):
                validate()
            del receipt["nearby"]["iphone"]["config"]
            receiver_app = Path(artifacts["receiver-app"]["path"])
            receiver_app.write_text(receiver_app.read_text().replace("a" * 40, "b" * 40))
            with self.assertRaisesRegex(ValueError, "secondary app manifest changed"):
                validate()
            artifacts["receiver-app"]["sha256"] = sha256(receiver_app)
            with self.assertRaisesRegex(ValueError, "source revisions differ"):
                validate()
            receiver_app.write_text(receiver_app.read_text().replace("b" * 40, "a" * 40))
            artifacts["receiver-app"]["sha256"] = sha256(receiver_app)
            Path(artifacts["receiver-runner"]["path"]).write_text("{}")
            with self.assertRaisesRegex(ValueError, "secondary runner manifest changed"):
                validate()

    def test_pair_release_requires_both_exact_devices_and_host_cleanup(self):
        source = "00008150-000B0B000000001C"
        receiver = "00008110-000A11CE0000001E"
        prepared = {"platform": "device", "device": source, "mode": "nearby",
                    "nearby": {"direction": "iphone-to-iphone", "iphone": {"device": receiver}}}
        result = {"platform": "device", "device": source, "cleanup_complete": True, "remote_runner_stopped": True}
        result["receiver_launch_attempted"] = True
        self.assertFalse(cleanup_confirmed(result, prepared), "Source-only cleanup cannot release both phones")
        child = {"device": receiver, "cleanup_complete": True, "remote_runner_stopped": True,
                 "host_processes_stopped": True}
        result["iphone_receiver"] = child
        self.assertTrue(cleanup_confirmed(result, prepared))
        for key in ("cleanup_complete", "remote_runner_stopped", "host_processes_stopped"):
            child[key] = False
            self.assertFalse(cleanup_confirmed(result, prepared), key)
            child[key] = True
        child["device"] = source
        self.assertFalse(cleanup_confirmed(result, prepared), "A stale or wrong-device child receipt cannot release the receiver")
        child["device"] = receiver
        result["cleanup_complete"] = False
        self.assertFalse(cleanup_confirmed(result, prepared), "Receiver cleanup cannot mask uncertain source/observer cleanup")

    def test_unstarted_receiver_can_release_only_without_any_launch_attempt(self):
        source = "00008150-000B0B000000001C"
        receiver = "00008110-000A11CE0000001E"
        prepared = {"platform": "device", "device": source, "mode": "nearby",
                    "nearby": {"direction": "iphone-to-iphone", "iphone": {"device": receiver}}}
        result = {"platform": "device", "device": source, "status": "failed", "error": "CoreDevice 4016",
                  "cleanup_complete": True, "remote_runner_stopped": True, "receiver_launch_attempted": False,
                  "iphone_receiver": {"device": receiver, "ownership": "not_started"}}
        self.assertTrue(cleanup_confirmed(result, prepared))
        for attempted in (True, None):
            result["receiver_launch_attempted"] = attempted
            self.assertFalse(cleanup_confirmed(result, prepared), "Partial construction or missing ownership fails closed")
        result["receiver_launch_attempted"] = False
        result["iphone_receiver"]["device"] = source
        self.assertFalse(cleanup_confirmed(result, prepared))
        result["iphone_receiver"]["device"] = receiver
        result["remote_runner_stopped"] = False
        self.assertFalse(cleanup_confirmed(result, prepared), "Unstarted child cannot mask uncertain primary cleanup")


if __name__ == "__main__":
    unittest.main()
