# SPDX-License-Identifier: GPL-3.0-or-later
"""Host-only subprocess diagnostics and simulator ownership boundaries."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
import iphone_peer

SPEC = importlib.util.spec_from_file_location("iphone_window", Path(__file__).with_name("iphone-window.py"))
window_module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(window_module)


class TimeoutDiagnosticsTests(unittest.TestCase):
    def test_timeout_retains_both_partial_streams_and_reaps_child(self):
        child = (
            "import sys,time;"
            "print('RSD connected',flush=True);"
            "print('waiting for snapshot',file=sys.stderr,flush=True);"
            "time.sleep(30)"
        )
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "sampler.log"
            from testing_timing import TimingLog
            timing = TimingLog(Path(directory), "timeout-regression", "iphone-window-test")
            with patch.object(window_module, "TIMING", timing), patch.object(
                    window_module, "TIMING_CATEGORY", "device_automation"):
                try:
                    with self.assertRaises(subprocess.TimeoutExpired):
                        window_module.command([sys.executable, "-c", child], timeout=2, output=log)
                finally:
                    timing.close()
            summary = timing.summary()
            self.assertGreater(summary["category_wall_seconds_inclusive"]["device_automation"], 0)
            self.assertEqual(summary["failed_operations"], 1)
            self.assertIsNone(summary["inference_seconds"])
            self.assertTrue((Path(directory) / "timing.jsonl").is_file())
            data = log.read_bytes()
            self.assertIn(b"RSD connected", data)
            self.assertIn(b"waiting for snapshot", data)
            diagnostic = json.loads(data.splitlines()[0])
            self.assertTrue(diagnostic["error"].startswith("TimeoutExpired:"))
            self.assertLess(diagnostic["returncode"], 0, "The timed-out subprocess must be terminated and reaped")

    def test_admitted_timing_bytes_cannot_be_replaced_before_import(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "testing_timing.py"
            source.write_text("VALUE = 'admitted'\n")
            admitted = window_module.digest(source)
            source.write_text("raise AssertionError('replacement must never execute')\n")
            with patch.object(window_module, "ROOT", root):
                with self.assertRaisesRegex(RuntimeError, "Frozen timing dependency changed"):
                    window_module.start_timing(root, expected_sha256=admitted)


class SimulatorOwnershipTests(unittest.TestCase):
    def test_booted_available_but_unowned_targets_are_refused(self):
        device = "00000000-0000-0000-0000-000000000001"
        with tempfile.TemporaryDirectory() as directory:
            for name in ("iPhone 17 Pro Max", "KiwixProfilingValidation", None, 7):
                with self.subTest(name=name):
                    row = {"udid": device, "state": "Booted", "isAvailable": True, "name": name}
                    listing = json.dumps({"devices": {"runtime": [row]}}).encode()
                    window = window_module.SimulatorWindow(Path(directory), device, 600)
                    # Only the external simctl listing is supplied; the actual
                    # identity admission logic must reject this otherwise valid target.
                    with patch.object(window, "sim", return_value=listing):
                        with self.assertRaisesRegex(RuntimeError, "non-owned simulator"):
                            window.ctl("identity", ["info", "details"])


class SimulatorMetadataTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == "darwin", "Requires Apple's OpenStep plist decoder")
    def test_openstep_inventory_reports_absent_app_without_parser_failure(self):
        inventory = b'{ "com.apple.example" = { CFBundleIdentifier = "com.apple.example"; CFBundleDisplayName = "System example"; }; }'
        with tempfile.TemporaryDirectory() as directory:
            window = window_module.SimulatorWindow(Path(directory), "test-simulator", 600)
            with patch.object(window, "sim", return_value=inventory):
                result = window.ctl("before-app", ["info", "apps", "--bundle-id", "com.example.not-installed"])
            self.assertEqual(result, {"apps": []})


class NativeSizeContractTests(unittest.TestCase):
    def test_native_archive_metadata_is_whole_kib_not_exact_payload_bytes(self):
        self.assertTrue(window_module.native_size_matches(333429760, 333429922))
        self.assertTrue(window_module.native_size_matches(1024, 1536))
        self.assertFalse(window_module.native_size_matches(2048, 1536))
        self.assertFalse(window_module.native_size_matches(333430784, 333429922))
        self.assertFalse(window_module.native_size_matches(333429922, 333429922))


class PreservationTests(unittest.TestCase):
    def state(self, tab, settings=None):
        return {"books": [], "settings": settings or {}, "bookmarks": [], "tabs": [tab]}

    def test_missing_default_restricted_policy_is_schema_normalization(self):
        tab = {"id": "tab-1", "bookId": "book-1", "path": "A", "title": "A", "scroll": 0.0}
        before = self.state(tab)
        after = self.state({**tab, "restricted": False})
        window_module.preservation(before, after, "/old", "/new", "com.example.app")

    def test_restricted_policy_change_is_still_rejected(self):
        tab = {"id": "tab-1", "bookId": "book-1", "path": "A", "title": "A", "scroll": 0.0}
        for before_tab, after_tab in ((tab, {**tab, "restricted": True}),
                                      ({**tab, "restricted": True}, {**tab, "restricted": False})):
            with self.subTest(before=before_tab, after=after_tab):
                with self.assertRaisesRegex(RuntimeError, "Unrelated preexisting reader tab changed"):
                    window_module.preservation(self.state(before_tab), self.state(after_tab),
                                               "/old", "/new", "com.example.app")

    def test_missing_default_empty_pin_list_is_schema_normalization(self):
        tab = {"id": "tab-1", "bookId": "book-1", "path": "A", "title": "A", "scroll": 0.0}
        window_module.preservation(self.state(tab), self.state(tab, {"pinnedBookIds": []}),
                                   "/old", "/new", "com.example.app")

    def test_pin_list_change_is_still_rejected(self):
        tab = {"id": "tab-1", "bookId": "book-1", "path": "A", "title": "A", "scroll": 0.0}
        with self.assertRaisesRegex(RuntimeError, "Settings or bookmarks changed"):
            window_module.preservation(self.state(tab), self.state(tab, {"pinnedBookIds": ["book-1"]}),
                                       "/old", "/new", "com.example.app")


class FilteredSamplerOwnershipTests(unittest.TestCase):
    def test_filtered_capture_rejects_foreign_or_malformed_coalition_records(self):
        valid = {"pid": 42, "coalitionID": 17}
        for rows in ([], [valid, {"pid": 43, "coalitionID": 18}],
                     [valid, {"pid": 43, "coalitionID": True}], [valid, None]):
            with self.subTest(rows=rows), tempfile.TemporaryDirectory() as directory:
                window = window_module.Window(Path(directory), "host-only-fixture", 600)

                def external_snapshot(argv, *args, **kwargs):
                    Path(argv[-1]).write_text(json.dumps(rows))

                # Only the external response is supplied. A foreign/malformed
                # member must fail admission, not be dropped to produce a total.
                with patch.object(window_module, "command", side_effect=external_snapshot):
                    with self.assertRaises(RuntimeError):
                        window.filtered_processes("sample", "unused-host-fixture", "coalitionID", 17)

    def test_root_probe_rejects_pid_reused_by_another_executable(self):
        executable = "/private/var/containers/Bundle/Application/owned/Kiwix.app/Kiwix"
        processes = {"runningProcesses": [{"executable": "file://" + executable, "processIdentifier": 42}]}
        row = {"pid": 42, "execName": "/usr/libexec/unrelated", "startAbsTime": 123,
               "coalitionID": 17, "physFootprint": 4096}
        with tempfile.TemporaryDirectory() as directory:
            window = window_module.Window(Path(directory), "host-only-fixture", 600)

            def external_snapshot(argv, *args, **kwargs):
                Path(argv[-1]).write_text(json.dumps([row]))

            with patch.object(window, "ctl", return_value=processes), \
                    patch.object(window_module, "command", side_effect=external_snapshot):
                with self.assertRaises(RuntimeError):
                    window.memory_root("root", "unused-host-fixture", executable)


class CoreDeviceLaunchParserTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == "darwin", "Requires Apple's devicectl argument parser")
    def test_launch_options_are_not_consumed_as_application_arguments(self):
        class CapturedCommand(Exception):
            pass

        with tempfile.TemporaryDirectory() as directory:
            # Never supply a real device. Stop before the transport and then
            # invoke only the vendor's eager help/parser path.
            window = window_module.Window(Path(directory), "HOST-PARSER-" + Path(directory).name, 600)
            captured = []

            def capture(argv, *args, **kwargs):
                captured.extend(str(value) for value in argv)
                raise CapturedCommand()

            with patch.object(window_module, "command", side_effect=capture):
                with self.assertRaises(CapturedCommand):
                    window.ctl("open", ["process", "launch", "--payload-url",
                                       "file:///fixture/Wikipedia.zim", "com.example.kiwix"])
            device_value_end = captured.index("--device") + 2
            captured.insert(device_value_end, "--help")
            parsed = subprocess.run(captured, capture_output=True, text=True, timeout=5)
            # With the broken ordering, --device and --help both became app
            # arguments and the actual parser rejected its missing device.
            self.assertEqual(parsed.returncode, 0, parsed.stdout + parsed.stderr)


class FreshNearbyStateTests(unittest.TestCase):
    bundle = "com.example.kiwix"
    present = {"files": [{"relativePath": "Application Support/" + bundle + "/state.json"}]}

    def test_existing_state_copy_failure_does_not_launch_or_fall_back(self):
        with tempfile.TemporaryDirectory() as directory:
            window = window_module.Window(Path(directory), "unused-host-fixture", 120)
            with patch.object(window, "ctl", side_effect=[self.present, RuntimeError("copy denied")]):
                with self.assertRaisesRegex(RuntimeError, "copy denied"):
                    window.initialized_state(self.bundle)

    def test_inventory_failures_never_initialize(self):
        for failure in (RuntimeError("transport disconnected"), RuntimeError("authorization denied")):
            with self.subTest(failure=str(failure)):
                window = window_module.Window(Path("unused"), "unused-host-fixture", 120)
                with patch.object(window, "ctl", side_effect=failure):
                    with self.assertRaisesRegex(RuntimeError, str(failure)):
                        window.initialized_state(self.bundle)

    def test_malformed_inventory_is_not_empty_state(self):
        for result in ({}, {"files": None}, {"files": {}}, {"files": [{}]}, {"files": [None]}):
            with self.subTest(result=result):
                window = window_module.Window(Path("unused"), "unused-host-fixture", 120)
                with patch.object(window, "ctl", return_value=result):
                    with self.assertRaisesRegex(RuntimeError, "Unexpected application file inventory"):
                        window.initialized_state(self.bundle)

    def test_initialization_launch_failure_propagates(self):
        window = window_module.Window(Path("unused"), "unused-host-fixture", 120)
        with patch.object(window, "ctl", side_effect=[{"files": []}, RuntimeError("launch denied")]):
            with self.assertRaisesRegex(RuntimeError, "launch denied"):
                window.initialized_state(self.bundle)

    def test_initialization_inventory_failure_propagates_without_copy(self):
        window = window_module.Window(Path("unused"), "unused-host-fixture", 120)
        with patch.object(window, "ctl", side_effect=[
                {"files": []}, {}, RuntimeError("transport disconnected")]):
            with self.assertRaisesRegex(RuntimeError, "transport disconnected"):
                window.initialized_state(self.bundle)

    def test_initialization_without_persisted_state_is_bounded_failure(self):
        window = window_module.Window(Path("unused"), "unused-host-fixture", 120)
        with patch.object(window, "ctl", side_effect=[{"files": []}, {}, {"files": []}]), \
                patch.object(window, "remaining", return_value=30), \
                patch.object(window_module.time, "monotonic", side_effect=[0, 16]):
            with self.assertRaisesRegex(RuntimeError, "App did not persist its ordinary initial state"):
                window.initialized_state(self.bundle)


class MacPublicationAuthorityTests(unittest.TestCase):
    archive = "dd8a2bd2-c05b-ac6c-8074-6c30e14803fb"
    publisher = "11111111-1111-4111-8111-111111111111"
    job_id = "22222222-2222-4222-8222-222222222222"

    def setUp(self):
        self.source = {"archive_id": self.archive, "file_name": "fixture.zim", "size": 1234, "sha256": "a" * 64}
        self.publication = {"publisher_id": self.publisher, "session_id": "b" * 32, "library_generation": 3}
        self.job = {"id": self.job_id, "fileName": "fixture.zim", "status": "complete",
                    "url": f"kiwix-library://public/{'b' * 32}/{self.publisher}/{self.archive}",
                    "transport": "quic-library", "downloaded": 1234, "total": 1234,
                    "publication": {"size": 1234, "sha256": "a" * 64}}
        self.job["destination"] = "/owned/archives/fixture.zim"
        self.state = {"books": [{"id": self.archive, "installationId": self.publisher, "managed": True,
                                 "path": self.job["destination"]}], "consumedAcquisitions": [self.job_id]}

    def test_accepts_only_new_complete_exact_publisher_session_job(self):
        self.assertEqual(window_module.mac_received_job({self.job_id: self.job}, {}, self.source,
                                                       self.publication, self.state), self.job)
        with self.assertRaises(RuntimeError):
            window_module.mac_received_job({self.job_id: self.job}, {self.job_id: self.job},
                                           self.source, self.publication, self.state)

    def test_matching_bytes_do_not_authorize_wrong_publication_or_stale_job(self):
        mutations = [
            {"url": self.job["url"].replace("b" * 32, "c" * 32)},
            {"url": self.job["url"].replace(self.publisher, self.job_id)},
            {"publication": {"size": 1234, "sha256": "c" * 64}},
            {"status": "paused"}, {"transport": "http"}, {"downloaded": 1233},
            {"id": self.publisher}, {"fileName": "other.zim"},
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(RuntimeError):
                window_module.mac_received_job({self.job_id: dict(self.job, **mutation)}, {},
                                               self.source, self.publication, self.state)

    def test_duplicate_filename_is_not_disambiguated_by_matching_hash(self):
        duplicate = dict(self.job, id=self.publisher)
        with self.assertRaises(RuntimeError):
            window_module.mac_received_job({self.job_id: self.job, self.publisher: duplicate}, {},
                                           self.source, self.publication, self.state)

    def test_job_must_be_consumed_for_exact_managed_installation(self):
        states = [
            dict(self.state, consumedAcquisitions=[]),
            dict(self.state, books=[dict(self.state["books"][0], path="/different/fixture.zim")]),
            dict(self.state, books=[dict(self.state["books"][0], managed=False)]),
            dict(self.state, books=[dict(self.state["books"][0], sourceReference={"provider": "unrelated"})]),
            dict(self.state, books=[dict(self.state["books"][0], installationId="")]),
        ]
        for state in states:
            with self.subTest(state=state), self.assertRaises((RuntimeError, ValueError)):
                window_module.mac_received_job({self.job_id: self.job}, {}, self.source, self.publication, state)

    def test_pair_job_uses_independently_observed_opaque_offer_not_archive_uuid(self):
        publication = dict(self.publication, offer_id="opaque/installation offer")
        exact = dict(self.job, url=self.job["url"].rsplit("/", 1)[0] + "/opaque%2Finstallation%20offer")
        self.assertEqual(window_module.mac_received_job({self.job_id: exact}, {}, self.source,
                                                       publication, self.state), exact)
        with self.assertRaises(RuntimeError):
            window_module.mac_received_job({self.job_id: self.job}, {}, self.source, publication, self.state)

    def capture(self, root, peer=None, generation=b"3", changed_app=False, symlink=False):
        bundle = "com.example.kiwix"
        apps = [{"bundleIdentifier": bundle,
                 "dataContainerPath": "/private/var/mobile/Containers/Data/Application/" + self.job_id}]
        observed = [dict(apps[0], dataContainerPath=apps[0]["dataContainerPath"].replace(self.job_id, self.publisher))] if changed_app else apps
        values = {"peer-id": (peer or self.publisher.encode()), "library-generation": generation}
        files = [{"relativePath": f"Application Support/{bundle}/localswarm/{name}",
                  "metadata": {"size": len(value)}, "resources": {"isSymbolicLink": symlink}}
                 for name, value in values.items()]
        window = window_module.Window(root, "host-only", 120)

        def ctl(label, args, timeout=15):
            if args[:2] == ["info", "apps"]:
                return {"apps": observed}
            if args[:2] == ["info", "files"]:
                return {"files": files}
            name = args[args.index("--source") + 1].rsplit("/", 1)[-1]
            Path(args[args.index("--destination") + 1]).write_bytes(values[name])
            return {}

        with patch.object(window, "ctl", side_effect=ctl):
            return window_module.iphone_publication(window, "source", bundle, {"device_receipt": apps})

    def test_source_capture_is_persisted_generation_not_invented_session(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(self.capture(Path(directory)), {"publisher_id": self.publisher, "library_generation": 3})

    def test_replaced_container_noncanonical_generation_and_symlinks_fail(self):
        for options in ({"changed_app": True}, {"generation": b"03"}, {"generation": b"0"},
                        {"generation": b"9007199254740992"}, {"symlink": True},
                        {"peer": b"AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"}):
            with self.subTest(options=options), tempfile.TemporaryDirectory() as directory:
                with self.assertRaises((RuntimeError, ValueError)):
                    self.capture(Path(directory), **options)


class MacFrozenReceiptTests(unittest.TestCase):
    def test_ready_receipt_rejects_rebound_request_or_preparation(self):
        with tempfile.TemporaryDirectory() as directory:
            peer = object.__new__(window_module.MacPeer)
            peer.output = Path(directory)
            peer.binding = {"schema": 2, "mobile_platform": "ios", "mobile_device": "00008110-000A11CE0000001E",
                            "mobile_artifact_sha256": "d" * 64, "request_sha256": "a" * 64,
                            "prepared_sha256": "b" * 64, "nonce": "c" * 32}
            for key in peer.binding:
                changed = dict(peer.binding, **{key: "wrong"})
                (peer.output / "ready.json").write_text(json.dumps(changed))
                with self.subTest(key=key), self.assertRaisesRegex(RuntimeError, "not bound"):
                    peer.receipt("ready.json")

    def test_changed_prepared_artifact_is_rejected_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = {}
            for field in ("helper", "lockfile", "executable", "browser"):
                path = root / field
                path.write_text(field)
                artifacts[field] = {"path": str(path), "sha256": window_module.digest(path)}
            receipt = root / "prepare.json"
            runtime = {"puppeteer_core": "25.16.0", "node_major": 24, "platform": "darwin", "arch": "arm64",
                       "node": {"path": "/owned/node", "sha256": "a" * 64}}
            receipt.write_text(json.dumps(dict(schema=1, runtime=runtime, **artifacts)))
            mac = {"tool": artifacts["helper"]["path"], "tool_sha256": artifacts["helper"]["sha256"],
                   "prepared": str(receipt), "prepared_sha256": window_module.digest(receipt)}
            with patch.object(window_module, "mac_runtime", return_value=runtime):
                window_module.validate_mac_prepared(mac)
                (root / "executable").write_text("replaced executable")
                with self.assertRaisesRegex(RuntimeError, "artifact changed: executable"):
                    window_module.validate_mac_prepared(mac)
            changes = [
                {"node_major": 25}, {"puppeteer_core": "25.17.0"},
                {"node": {"path": "/owned/node", "sha256": "b" * 64}},
                {"node": {"path": "/replaced/node", "sha256": "a" * 64}},
            ]
            for change in changes:
                with self.subTest(change=change), patch.object(window_module, "mac_runtime", return_value=dict(runtime, **change)):
                    with self.assertRaisesRegex(RuntimeError, "runtime/platform/architecture changed"):
                        window_module.validate_mac_prepared(mac)

    def test_publisher_must_remain_alive_until_owned_stop(self):
        peer = object.__new__(window_module.MacPeer)
        peer.mode = "publish"
        peer.process = Mock()
        peer.process.poll.return_value = 0
        peer.log = Mock()
        with self.assertRaisesRegex(RuntimeError, "before its owned stop"):
            peer.stop(complete=True)
        peer.process.stdin.close.assert_called_once()
        peer.log.close.assert_called_once()


class AndroidRadioAdmissionIsolationTests(unittest.TestCase):
    def test_apple_publication_never_substitutes_for_android_radio_proof(self):
        with self.assertRaisesRegex(RuntimeError, "disconnected STA/no-AP"):
            window_module.AndroidReceiver.validate_publication({
                "publisher_ready": True,
                "publication": {"publisher_id": "11111111-1111-4111-8111-111111111111",
                                "session_id": "a" * 32, "library_generation": 1},
                "topology": "apple-lan",
            })

    def test_every_existing_android_radio_gate_remains_required(self):
        proof = dict(publisher_ready=True, sta_disabled=True, no_ap_at_instrumentation_ready=True,
                     product_owned_ap_at_publish=True,
                     route_radio={"current_ap_verified": True, "ap_frequency_mhz": 5180})
        window_module.AndroidReceiver.validate_publication(proof)
        for key in ("publisher_ready", "sta_disabled", "no_ap_at_instrumentation_ready",
                    "product_owned_ap_at_publish", "route_radio"):
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                window_module.AndroidReceiver.validate_publication({name: value for name, value in proof.items() if name != key})


class ReceiverWorkRetirementTests(unittest.TestCase):
    def setUp(self):
        fixture = MacPublicationAuthorityTests()
        fixture.setUp()
        self.source, self.state = fixture.source, fixture.state
        self.job = dict(fixture.job, url=fixture.job["url"].rsplit("/", 1)[0] + "/opaque-offer")
        self.observation = {"publication": fixture.publication,
                            "offer": {"id": "opaque-offer", "downloadIdentity": self.job["url"]}}
        self.unrelated = {"id": "44444444-4444-4444-8444-444444444444", "fileName": "owners-download.zim",
                          "url": "https://example.test/owners-download.zim", "destination": "/owners/data.zim",
                          "transport": "http", "status": "downloading", "downloaded": 100}
        self.baseline = {self.unrelated["id"]: self.unrelated}
        self.nonce = "c" * 32
        self.retired = ("KIWIX_RECEIVER_RETIRED " + self.nonce + "\n").encode()
        self.cancelled = ("KIWIX_RECEIVER_CANCELLED " + self.nonce + "\n").encode()

    def proof(self, job, output=None, admitted=True, possible_admission=True, unrelated=None):
        jobs = {self.unrelated["id"]: dict(self.unrelated, downloaded=200) if unrelated is None else unrelated}
        if job is not None:
            jobs[job["id"]] = job
        return window_module.receiver_retirement_proof(self.baseline, self.job if admitted else None,
            jobs, self.source, self.observation, self.state, self.nonce,
            self.retired if output is None else output, possible_admission)

    def test_browse_cancellation_with_no_admission_preserves_unrelated_progress(self):
        self.assertIs(self.proof(None, admitted=False, possible_admission=False)["receiver_work_retired"], True)

    def test_admission_race_does_not_treat_missing_journal_as_retirement(self):
        with self.assertRaisesRegex(RuntimeError, "in flight"):
            self.proof(None, admitted=False)
        with self.assertRaisesRegex(RuntimeError, "in flight"):
            self.proof(self.job, admitted=False)

    def test_download_and_verification_need_actual_scoped_cancellation_drain(self):
        for state in ("downloading", "verifying", "paused", "failed"):
            with self.subTest(state=state), self.assertRaisesRegex(RuntimeError, "retirement"):
                self.proof(dict(self.job, status=state))
        with self.assertRaisesRegex(RuntimeError, "status alone"):
            self.proof(dict(self.job, status="cancelled"))
        result = self.proof(dict(self.job, status="cancelled"), self.cancelled + self.retired)
        self.assertIs(result["receiver_owned_job_retired"], True)

    def test_completed_job_requires_consumed_managed_installation_not_just_terminal_label(self):
        self.assertIs(self.proof(self.job)["receiver_work_retired"], True)
        self.state["consumedAcquisitions"] = []
        with self.assertRaises(RuntimeError):
            self.proof(self.job)

    def test_wrong_publisher_or_unrelated_job_change_is_never_cancel_authority(self):
        with self.assertRaisesRegex(RuntimeError, "observed-offer"):
            self.proof(dict(self.job, url=self.job["url"].replace("b" * 32, "d" * 32)))
        with self.assertRaisesRegex(RuntimeError, "Unrelated download authority"):
            self.proof(self.job, unrelated=dict(self.unrelated, destination="/replaced/data.zim"))
        with self.assertRaisesRegex(RuntimeError, "one exact UI receipt"):
            self.proof(self.job, output=self.retired.replace(self.nonce.encode(), b"d" * 32))


class ReceiverCleanupDeadlineTests(unittest.TestCase):
    def test_early_cancellation_reaps_distinct_command_session_before_remote_confirmation_and_parent_escalation(self):
        elapsed = [0.0]
        processes = []
        events = []
        original_popen = subprocess.Popen
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(window_module.time, "time", side_effect=lambda: 1000 + elapsed[0]), \
                patch.object(window_module.time, "monotonic", side_effect=lambda: elapsed[0]):
            root = Path(directory)
            window = window_module.Window(root, "host-only-no-device", 600)
            controller = object.__new__(iphone_peer.ReceiverControl)
            controller.cleanup_deadline = None
            controller.request = {"owner_deadline": 1600, "nonce": "c" * 32}
            controller.observation = {"offer": {"downloadIdentity": "kiwix-library://public/session/peer/offer"}}
            window.cleanup_deadline = controller.begin_cleanup()
            plan = root / "runner.xctestrun"
            plan.write_bytes(window_module.plistlib.dumps({"Tests": {"BlueprintName": "KiwixPhoneWindowUI"}}))

            def journal(*args, **kwargs):
                # Delayed original-runner shutdown and journal stages consumed
                # most of the work interval, without borrowing the reap reserve.
                elapsed[0] += iphone_peer.RECEIVER_CLEANUP_SECONDS - iphone_peer.RECEIVER_FINAL_RUNNER_SECONDS - .05
                return {}

            def launch(argv, **kwargs):
                self.assertEqual(argv[0], "xcodebuild")
                self.assertIs(kwargs["start_new_session"], True)
                process = original_popen([sys.executable, "-c", "import time; time.sleep(2)"], **kwargs)
                processes.append(process)
                return process

            def remote_stop():
                self.assertEqual(len(processes), 1)
                self.assertIsNotNone(processes[0].poll(), "The separate command session must be reaped first")
                self.assertLess(processes[0].returncode, 0)
                events.append("command-reaped")
                elapsed[0] += 24  # all three bounded remote runner RPCs
                self.assertGreater(window.remaining(cleanup=True), 0)
                events.append("remote-runner-confirmed")
                return dict(cleanup_complete=True, host_processes_stopped=True, remote_runner_stopped=True)

            try:
                with patch.object(window_module, "nearby_jobs", side_effect=journal), \
                        patch.object(window_module.subprocess, "Popen", side_effect=launch), \
                        patch.object(window, "stop", side_effect=remote_stop), \
                        patch.object(window, "ctl", side_effect=AssertionError("No hardware in boundary test")):
                    with self.assertRaises(subprocess.TimeoutExpired):
                        window_module.retire_iphone_receiver(window, {"bundle_id": "owned.app"},
                            {"xctestrun": str(plan)}, {"file_name": "fixture.zim", "peer": "Source", "title": "Fixture"},
                            controller, {}, {}, "/before", "/after", {"device_receipt": []})
                self.assertEqual(events, ["command-reaped", "remote-runner-confirmed"])
                self.assertLess(elapsed[0], iphone_peer.RECEIVER_PARENT_GRACE_SECONDS)
                self.assertIs(window.retirement_cleanup_receipt["remote_runner_stopped"], True)
            finally:
                for process in processes:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=3)


class SharedPhoneDeadlineTests(unittest.TestCase):
    def test_action_deadline_never_borrows_other_phone_cleanup_interval(self):
        with tempfile.TemporaryDirectory() as directory:
            window = window_module.Window(Path(directory), "host-only", 600)
            window.action_deadline = window_module.time.time() - 1
            with self.assertRaises(TimeoutError):
                window.remaining()
            self.assertGreater(window.remaining(cleanup=True), 500)


class MacObserverResultTests(unittest.TestCase):
    def test_observer_requires_stable_readonly_proof_but_no_receiver_installation(self):
        peer = object.__new__(window_module.MacPeer)
        peer.mode = "observe"
        peer.process = Mock(returncode=0)
        peer.process.poll.return_value = 0
        peer.publication = {"publisher_id": MacPublicationAuthorityTests.publisher,
                            "session_id": "b" * 32, "library_generation": 3}
        result = dict(status="complete", cleanup_complete=True, preserved_before_books_settings_tabs=True,
                      publication=peer.publication, stopped=True, observed_source_stable=True,
                      paired_complete=True, selected_peer={"id": "actual-source"},
                      inventory={"receiptId": "actual-browse"}, offer={"id": "actual-offer"})
        peer.receipt = Mock(return_value=result)
        self.assertIs(peer.result(), result)
        for field in ("observed_source_stable", "paired_complete", "stopped", "cleanup_complete"):
            peer.receipt.return_value = dict(result, **{field: False})
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                peer.result()
        peer.receipt.return_value = dict(result, receivedhash="a" * 64)
        with self.assertRaises(RuntimeError):
            peer.result()


if __name__ == "__main__":
    unittest.main()
