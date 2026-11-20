# SPDX-License-Identifier: GPL-3.0-or-later
"""Host-only admission/authority boundaries; no device or transfer certification."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
import uuid
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("mac_android", Path(__file__).with_name("mac-android.py"))
controller = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(controller)

ARCHIVE = "11111111-2222-4333-8444-555555555555"
PUBLISHER = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
INSTALLATION = "55555555-6666-4777-8888-999999999999"
JOB = "01234567-89ab-4cde-8fab-0123456789ab"
# The controller runs on a macOS host. No-follow opens, unprivileged symlinks,
# process groups and deleting an open file do not exist on Windows, so the
# tests that reach them are skipped there.
posix_host = unittest.skipUnless(os.name == "posix", "requires a POSIX host (O_NOFOLLOW, symlinks, process groups)")


class FrozenHandoffTests(unittest.TestCase):
    def test_changed_receipt_bytes_fail_before_json_or_helper_import(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "prepare.json"
            original = b'{"schema":1}\n'
            expected = hashlib.sha256(original).hexdigest()
            for changed in (b"{", original + b"\n"):
                path.write_bytes(changed)
                with patch.object(controller, "load_helper", side_effect=AssertionError("helper imported before byte admission")):
                    with self.assertRaisesRegex(RuntimeError, "prepare bytes changed"):
                        controller.load_frozen(path, expected, "R5TESTSER01")

    @posix_host
    def test_changed_frozen_dependency_is_rejected_before_helper_or_device_start(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            dependency = root / "helper.py"
            dependency.write_bytes(b"reviewed source\n")
            frozen = controller.pin(dependency)
            receipt = {"schema": 1, "status": "ready", "topology": "lan", "direction": "mac-to-android",
                       "device": "R5TESTSER01", "window_seconds": 600, "inputs": [frozen],
                       "python": {"executable": str(Path(sys.executable).resolve()), "version": sys.version}}
            path = root / "prepare.json"
            path.write_text(json.dumps(receipt))
            expected = controller.pin(path)["sha256"]
            dependency.write_bytes(b"unreviewed source\n")
            with patch.object(controller, "load_helper", side_effect=AssertionError("helper imported before input admission")), \
                    patch.object(controller.subprocess, "Popen", side_effect=AssertionError("device started")):
                with self.assertRaisesRegex(RuntimeError, "Frozen input changed"):
                    controller.load_frozen(path, expected, "R5TESTSER01")

    @posix_host
    def test_missing_timing_source_fails_before_helper_import(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "prepare.json"
            receipt = {"schema": 1, "status": "ready", "topology": "lan", "direction": "mac-to-android",
                       "device": "R5TESTSER01", "window_seconds": 600, "inputs": [],
                       "python": {"executable": str(Path(sys.executable).resolve()), "version": sys.version}}
            path.write_text(json.dumps(receipt))
            with patch.object(controller, "load_helper", side_effect=AssertionError("unadmitted helper import")):
                with self.assertRaisesRegex(RuntimeError, "timing dependency missing"):
                    controller.load_frozen(path, controller.pin(path)["sha256"], "R5TESTSER01")

    @posix_host
    def test_explicit_serial_mismatch_wins_before_reading_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "prepare.json"
            path.write_text(json.dumps({"schema": 1, "status": "ready", "topology": "lan", "direction": "android-to-mac",
                                        "device": "R5TESTSER01", "inputs": [{"path": "/not/an/input"}]}))
            with self.assertRaisesRegex(RuntimeError, "serial differs"):
                controller.load_frozen(path, controller.pin(path)["sha256"], "other-phone")

    @posix_host
    def test_symlinked_source_and_directories_cannot_be_hashed_as_authority(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            actual = root / "actual.zim"
            actual.write_bytes(b"owned fixture bytes")
            link = root / "alias.zim"
            link.symlink_to(actual)
            with self.assertRaisesRegex(RuntimeError, "not canonical"):
                controller.pin(link)
            with self.assertRaisesRegex(RuntimeError, "not a regular file"):
                controller.pin(root)

    @posix_host
    def test_matching_hash_cannot_substitute_for_real_zim_header_uuid(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "source.zim"
            path.write_bytes(b"ZIM\x04" + b"\x00" * 4 + uuid.UUID(ARCHIVE).bytes + b"content")
            source = {"source": str(path), "size": path.stat().st_size,
                      "sha256": controller.pin(path)["sha256"], "archive_id": ARCHIVE}
            controller.validate_fixture(source)
            with self.assertRaisesRegex(RuntimeError, "real ZIM header"):
                controller.validate_fixture(dict(source, archive_id=PUBLISHER))
            path.write_bytes(path.read_bytes() + b"changed")
            with self.assertRaisesRegex(RuntimeError, "fixture bytes changed"):
                controller.validate_fixture(source)


@posix_host
class TimingIterationTests(unittest.TestCase):
    def test_generated_iteration_reaches_real_child_and_nonzero_stays_failed(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ):
            os.environ.pop("KIWIX_TIMING_ITERATION", None)
            for exit_code in (0, 7):
                with self.subTest(exit_code=exit_code):
                    args = SimpleNamespace(output=Path(directory) / str(exit_code))
                    observed = []

                    def operation(arguments):
                        child = subprocess.run(
                            [sys.executable, "-c",
                             "import os,sys;print(os.environ['KIWIX_TIMING_ITERATION']);sys.exit(" + str(exit_code) + ")"],
                            capture_output=True, text=True, timeout=5)
                        observed.append(child.stdout.strip())
                        return child.returncode

                    self.assertEqual(controller.timed_operation(args, operation, "host_prepare"), exit_code)
                    ledger = [json.loads(row) for row in (args.output / "timing.jsonl").read_text().splitlines()]
                    self.assertEqual({row["iteration_id"] for row in ledger}, set(observed))
                    summary = json.loads((args.output / "timing-summary.json").read_text())
                    self.assertEqual(summary["failed_operations"], int(exit_code != 0))
                    self.assertNotIn("KIWIX_TIMING_ITERATION", os.environ)


@posix_host
class ReviewedCrashPreparationTests(unittest.TestCase):
    def receipt(self, root, reviewed):
        # Byte-artifact admission only; these files are not runnable APKs or
        # simulated successful device receipts. The actual helper supplies policy.
        android = controller.load_helper("android-nearby")
        receipt = {"schema": 1, "target_package": android.TARGET, "instrumentation": android.RUNNER,
                   "host_source_sha256": android.sha(Path(android.__file__)), "source_sha256": android.sources(),
                   "certificate_sha256": "c" * 64, "reviewed_crash_sha256": reviewed}
        receipt["timing_tool_sha256"] = controller.pin(controller.SCRIPTS / "testing_timing.py")["sha256"]
        for name in ("target_apk", "tool_apk"):
            path = root / (name + ".apk")
            path.write_bytes((name + " admission bytes").encode())
            receipt[name] = str(path)
            receipt[name + "_sha256"] = controller.pin(path)["sha256"]
        path = root / "prepare.json"
        path.write_text(json.dumps(receipt))
        source = {"android": {"prepared": str(path), "prepared_sha256": controller.pin(path)["sha256"],
                              "tool_sha256": receipt["host_source_sha256"]}}
        return android, receipt, source, path

    def test_clean_phone_empty_identity_and_exact_reviewed_hash_are_valid_frozen_states(self):
        for reviewed in ("", "a" * 64):
            with self.subTest(reviewed=reviewed), tempfile.TemporaryDirectory() as directory:
                android, receipt, source, _ = self.receipt(Path(directory).resolve(), reviewed)
                self.assertEqual(controller.validate_android(source, android)["reviewed_crash_sha256"], reviewed)

    def test_missing_null_or_malformed_crash_identity_is_not_a_clean_phone(self):
        for reviewed in (None, "not-a-hash", "A" * 64, "a" * 63):
            with self.subTest(reviewed=reviewed), tempfile.TemporaryDirectory() as directory:
                android, receipt, source, path = self.receipt(Path(directory).resolve(), reviewed)
                with self.assertRaisesRegex(ValueError, "valid frozen reviewed crash identity"):
                    controller.validate_android(source, android)
                del receipt["reviewed_crash_sha256"]
                path.write_text(json.dumps(receipt))
                source["android"]["prepared_sha256"] = controller.pin(path)["sha256"]
                with self.assertRaisesRegex(ValueError, "valid frozen reviewed crash identity"):
                    controller.validate_android(source, android)

    def test_another_valid_reviewed_hash_cannot_replace_the_frozen_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            android, receipt, source, path = self.receipt(Path(directory).resolve(), "a" * 64)
            receipt["reviewed_crash_sha256"] = "b" * 64
            path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(RuntimeError, "Frozen Android prepare receipt changed"):
                controller.validate_android(source, android)

    def test_stale_or_missing_timing_authority_cannot_admit_android_helper(self):
        for timing_hash in (None, "0" * 64):
            with self.subTest(timing_hash=timing_hash), tempfile.TemporaryDirectory() as directory:
                android, receipt, source, path = self.receipt(Path(directory).resolve(), "")
                receipt["timing_tool_sha256"] = timing_hash
                path.write_text(json.dumps(receipt))
                source["android"]["prepared_sha256"] = controller.pin(path)["sha256"]
                with self.assertRaisesRegex(RuntimeError, "Android preparation differs"):
                    controller.validate_android(source, android)


class ReceivedPublicationAuthorityTests(unittest.TestCase):
    def fixture(self):
        source = {"archive_id": ARCHIVE, "file_name": "fixture.zim", "size": 4096, "sha256": "d" * 64}
        publication = {"publisher_id": PUBLISHER, "library_generation": 7, "session_id": "a" * 32}
        job = {"id": JOB, "journal_key": JOB, "url": f"kiwix-library://public/{'a' * 32}/{PUBLISHER}/{ARCHIVE}",
               "transport": "quic-library", "fileName": "fixture.zim", "status": "complete", "error": None,
               "importError": None, "destination": "/owned/fixture.zim", "downloaded": 4096, "total": 4096,
               "publication": {"size": 4096, "sha256": "d" * 64}, "matches_independent_received_bytes": True,
               "consumed_acquisition": True, "installation_id": INSTALLATION}
        result = {"managed_path": "/owned/fixture.zim", "installation_id": INSTALLATION,
                  "received_journal": {"status": "matched", "match_count": 1, "matched_job": job, "new_rows": [job]}}
        return source, publication, result

    def test_same_uuid_and_bytes_from_another_session_never_prove_this_mac_publisher(self):
        source, publication, result = self.fixture()
        controller.validate_android_job(result, source, publication)
        for changed in ({"session_id": "b" * 32}, {"publisher_id": INSTALLATION}):
            with self.subTest(changed=changed), self.assertRaisesRegex(RuntimeError, "actual Mac publication"):
                controller.validate_android_job(result, source, dict(publication, **changed))

    def test_completed_job_must_be_new_consumed_receipt_for_exact_managed_installation(self):
        source, publication, result = self.fixture()
        for mutation in ("old", "unconsumed", "other_installation", "other_destination", "other_transport", "ambiguous"):
            value = copy.deepcopy(result)
            journal = value["received_journal"]
            job = journal["matched_job"]
            if mutation == "old":
                journal["new_rows"] = []
            elif mutation == "unconsumed":
                job["consumed_acquisition"] = False
            elif mutation == "other_installation":
                job["installation_id"] = PUBLISHER
            elif mutation == "other_destination":
                job["destination"] = "/unrelated/fixture.zim"
            elif mutation == "other_transport":
                job["transport"] = "https"
            else:
                journal["match_count"] = 2
            with self.subTest(mutation=mutation), self.assertRaisesRegex(RuntimeError, "consumed managed installation"):
                controller.validate_android_job(value, source, publication)


class CleanupAuthorityTests(unittest.TestCase):
    def test_failed_browse_with_successful_cleanup_reports_the_actual_peer_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            peer = object.__new__(controller.AndroidPeer)
            peer.output, peer.mode = root, "receive"
            peer.binding = {"nonce": "a" * 32}
            (root / "result.json").write_text(json.dumps({
                **peer.binding, "status": "failed", "cleanup_complete": True,
                "preserved_before_books_settings_tabs": True, "crash_report_preserved": True,
                "error": "Current page/dialog errors: Shared library did not respond",
            }))
            peer.process = subprocess.Popen([sys.executable, "-c", "raise SystemExit(1)"],
                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            peer.process.wait(timeout=5)
            with self.assertRaisesRegex(RuntimeError, "Android receive failed: .*Shared library did not respond"):
                peer.result()

    @posix_host
    def test_exited_host_without_bound_native_cleanup_receipt_cannot_release_device(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            peer = object.__new__(controller.AndroidPeer)
            peer.window = controller.Window(root, 120)
            peer.output = root
            peer.binding = {"nonce": "a" * 32}
            peer.log = (root / "host.log").open("wb")
            peer.process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                            stdout=peer.log, stderr=subprocess.STDOUT, start_new_session=True)
            with self.assertRaisesRegex(RuntimeError, "cleanup is unconfirmed"):
                peer.stop()
            self.assertIsNotNone(peer.process.poll(), "Owned host must still be reaped on missing native proof")
            self.assertTrue(peer.log.closed)


if __name__ == "__main__":
    unittest.main()
