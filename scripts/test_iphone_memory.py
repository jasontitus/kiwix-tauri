# SPDX-License-Identifier: GPL-3.0-or-later
"""Ownership/generation regressions for saved DVT data; never contacts devices."""
import copy
import unittest

from iphone_memory import checkpoint_footprint, snapshot_footprint

OURS = "/private/var/containers/Bundle/Application/ours/Kiwix.app/Kiwix"
PEER = "/private/var/containers/Bundle/Application/peer/Kiwix.app/Kiwix"


def process(pid, name, footprint, coalition=7, executable=None, compressed=0):
    return {"pid": pid, "startAbsTime": pid * 100, "coalitionID": coalition,
            "responsiblePID": 1, "responsibleUniqueID": 0, "name": name,
            "execName": executable or "/System/" + name,
            "physFootprint": footprint, "memCompressed": compressed}


def fixture():
    return [process(10, "Kiwix", 100, executable=OURS, compressed=3),
            process(20, "com.apple.WebKit.WebContent", 20, compressed=2),
            process(21, "com.apple.WebKit.GPU", 5, compressed=1),
            process(22, "com.apple.WebKit.Networking", 2, coalition=433),
            process(30, "owned-helper", 7),
            process(40, "com.apple.WebKit.WebContent", 800, coalition=9)]


class CoalitionFootprintTests(unittest.TestCase):
    def test_counts_kernel_coalition_not_ambiguous_webkit_names_or_responsible_pid(self):
        result = snapshot_footprint(fixture(), OURS)
        self.assertEqual(result["status"], "complete_observed_coalition")
        self.assertEqual(result["bytes"]["observed_coalition"], 132)
        self.assertEqual(result["bytes"]["web_content"], 20)
        self.assertEqual(result["bytes"]["gpu"], 5)
        self.assertEqual(result["bytes"]["networking"], 0)
        self.assertEqual(result["bytes"]["other"], 7)
        self.assertEqual({row["pid"] for row in result["processes"]}, {10, 20, 21, 30})

    def test_compressed_ledger_is_not_added_to_physical_footprint(self):
        result = snapshot_footprint(fixture(), OURS)
        self.assertEqual(result["bytes"]["compressed"], 6)
        self.assertEqual(result["bytes"]["observed_coalition"], 132)

    def test_missing_member_metric_does_not_become_a_lower_total(self):
        rows = fixture()
        del rows[1]["physFootprint"]
        result = snapshot_footprint(rows, OURS)
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["bytes"]["observed_coalition"])
        self.assertEqual(result["bytes"]["main"], 100)

    def test_missing_compression_is_unavailable_not_zero(self):
        rows = fixture()
        del rows[1]["memCompressed"]
        result = snapshot_footprint(rows, OURS)
        self.assertEqual(result["status"], "complete_observed_coalition")
        self.assertIsNone(result["bytes"]["compressed"])

    def test_no_renderer_or_missing_coalition_cannot_claim_reader_cohort(self):
        for rows in ([fixture()[0]], [dict(fixture()[0], coalitionID=0)]):
            with self.subTest(rows=rows):
                result = snapshot_footprint(rows, OURS)
                self.assertEqual(result["status"], "unavailable")
                self.assertIsNone(result["bytes"]["observed_coalition"])

    def test_competing_app_in_same_coalition_is_not_exclusive_ownership(self):
        rows = fixture() + [process(50, "Kiwix", 200, executable=PEER)]
        result = snapshot_footprint(rows, OURS, [PEER])
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["bytes"]["observed_coalition"])

    def test_duplicate_pid_cannot_be_counted_twice(self):
        rows = fixture()
        rows.append(copy.deepcopy(rows[1]))
        result = snapshot_footprint(rows, OURS)
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["bytes"]["observed_coalition"])

    def test_missing_or_ambiguous_exact_main_is_not_zero_memory(self):
        rows = fixture()
        for sample in (rows[1:], rows + [copy.deepcopy(rows[0])]):
            result = snapshot_footprint(sample, OURS)
            self.assertEqual(result["status"], "unavailable")
            self.assertIsNone(result["bytes"]["main"])

    def test_renderer_restart_cannot_masquerade_as_checkpoint_saving(self):
        first = snapshot_footprint(fixture(), OURS)
        changed = fixture()
        changed[1]["startAbsTime"] += 1
        result = checkpoint_footprint([first, snapshot_footprint(changed, OURS), first])
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["median_bytes"]["observed_coalition"])
        self.assertEqual(result["median_bytes"]["main"], 100)

    def test_main_pid_reuse_rejects_all_aggregation(self):
        first = snapshot_footprint(fixture(), OURS)
        changed = fixture()
        changed[0]["startAbsTime"] += 1
        result = checkpoint_footprint([first, snapshot_footprint(changed, OURS)])
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["median_bytes"]["main"])

    def test_membership_loss_cannot_become_a_lower_stable_checkpoint(self):
        first = snapshot_footprint(fixture(), OURS)
        changed = [row for row in fixture() if row["pid"] != 21]
        result = checkpoint_footprint([first, snapshot_footprint(changed, OURS)])
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["median_bytes"]["observed_coalition"])

    def test_checkpoint_median_is_of_actual_ledger_sums_not_replications(self):
        samples = []
        for footprint in (100, 140, 110):
            rows = fixture()
            rows[0]["physFootprint"] = footprint
            samples.append(snapshot_footprint(rows, OURS))
        result = checkpoint_footprint(samples)
        self.assertEqual(result["status"], "complete_observed_coalition")
        self.assertEqual(result["median_bytes"]["observed_coalition"], 142)
        self.assertEqual(result["sample_count"], 3)
        self.assertEqual(result["independent_sessions"], 1)


if __name__ == "__main__":
    unittest.main()
