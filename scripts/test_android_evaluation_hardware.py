# SPDX-License-Identifier: GPL-3.0-or-later
"""Consumer-boundary regressions; these tests are not physical-device evidence."""

import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from android_evaluation_hardware import apk_identity, hardware_preflight


class HardwareAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.apk = Path(self.directory.name) / "app.apk"
        self.header = bytearray(64)
        self.header[:6] = b"\x7fELF\x01\x01"
        self.header[18:20] = (40).to_bytes(2, "little")
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("lib/armeabi-v7a/libkiwix_app.so", self.header)
        self.args = argparse.Namespace(target_abi="armeabi-v7a", target_apk=self.apk,
                                       device="unit-test-not-a-device", min_ram_mib=512,
                                       max_ram_mib=2048, require_low_ram_device=True, inventory_only=False)
        self.receipt = {"status": "admitted", "mode": "info-only", "requiredAbi": "armeabi-v7a",
                        "targetApkSha256": apk_identity(self.apk, "armeabi-v7a")["sha256"],
                        "nativeLibraryLoaded": True, "workloadExecuted": False,
                        "device": {"supportedAbis": ["armeabi-v7a", "armeabi"], "process64Bit": False,
                                   "totalPhysicalRamBytes": 1800 * 1024 * 1024, "lowRamDevice": True}}

    def probe(self, receipt=None, suffix="OK (1 test)\n"):
        label = "hardware-inventory" if self.args.inventory_only else "hardware-admission"
        stdout = "INSTRUMENTATION_STATUS: stream=ANDROID_EVALUATION " + label + " " + json.dumps(receipt or self.receipt) + "\n" + suffix
        with patch("android_evaluation_hardware.subprocess.run", return_value=subprocess.CompletedProcess([], 0, stdout, "")) as run:
            result = hardware_preflight(self.args)
            self.assertIn("evaluationInventoryOnly" if self.args.inventory_only else "evaluationProbeOnly", run.call_args.args[0])
            expected_runner = "org.kiwix.workbench.inventory.test/" if self.args.inventory_only else "org.kiwix.workbench.test/"
            self.assertTrue(run.call_args.args[0][-1].startswith(expected_runner))
            return result

    def test_apk_must_contain_requested_native_abi(self):
        with self.assertRaisesRegex(ValueError, "lacks"):
            apk_identity(self.apk, "arm64-v8a")

    def test_apk_directory_label_cannot_fake_elf_architecture(self):
        self.header[4] = 2
        self.header[18:20] = (183).to_bytes(2, "little")
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("lib/armeabi-v7a/libkiwix_app.so", self.header)
        with self.assertRaisesRegex(ValueError, "mislabeled ELF"):
            apk_identity(self.apk, "armeabi-v7a")

    def test_platform_low_ram_flag_cannot_be_assumed(self):
        self.receipt["device"]["lowRamDevice"] = False
        with self.assertRaisesRegex(ValueError, "RAM measurement"):
            self.probe()

    def test_higher_ram_cannot_pass_two_gib_request(self):
        self.receipt["device"]["totalPhysicalRamBytes"] = 3 * 1024 ** 3
        with self.assertRaisesRegex(ValueError, "RAM measurement"):
            self.probe()

    def test_actual_process_bitness_must_match(self):
        self.receipt["device"]["process64Bit"] = True
        with self.assertRaisesRegex(ValueError, "Inconsistent"):
            self.probe()

    def test_receipt_for_different_installed_apk_rejected(self):
        self.receipt["targetApkSha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "rejected"):
            self.probe()

    def test_incomplete_instrumentation_is_not_admission(self):
        with self.assertRaisesRegex(ValueError, "finish successfully"):
            self.probe(suffix="INSTRUMENTATION_FAILED: test crashed\n")

    def test_probe_is_not_workload_or_physical_proof(self):
        result = self.probe()
        self.assertTrue(result["hardwareMeasured"])
        self.assertFalse(result["workloadExecuted"])
        self.assertEqual(result["physicalDeviceStatus"], "unverified")

    def test_java_inventory_needs_no_apk_or_native_library(self):
        self.args.inventory_only = True
        self.args.target_apk = None
        self.receipt.update(status="measured", mode="inventory-only", nativeLibraryLoaded=False)
        result = self.probe()
        self.assertTrue(result["hardwareMeasured"])
        self.assertFalse(result["nativeAdmission"])
        self.assertFalse(result["workloadExecuted"])

    def test_inventory_apk_identity_accepts_no_native_payload(self):
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("AndroidManifest.xml", b"unit-test-only manifest entry")
        identity = apk_identity(self.apk, "armeabi-v7a", inventory=True)
        self.assertEqual(identity["nativeAbis"], [])
        self.assertEqual(identity["nativeElf"], {})

    def test_inventory_apk_cannot_hide_any_native_payload(self):
        with self.assertRaisesRegex(ValueError, "must not package native"):
            apk_identity(self.apk, None, inventory=True)

    def test_host_only_cannot_claim_ram_admission(self):
        self.args.device = None
        with self.assertRaisesRegex(ValueError, "catalog name is not a measurement"):
            hardware_preflight(self.args)
        self.args.min_ram_mib = self.args.max_ram_mib = None
        self.args.require_low_ram_device = False
        result = hardware_preflight(self.args)
        self.assertFalse(result["hardwareMeasured"])
        self.assertFalse(result["workloadExecuted"])


if __name__ == "__main__":
    unittest.main()
