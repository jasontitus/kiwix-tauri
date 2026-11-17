# SPDX-License-Identifier: GPL-3.0-or-later
"""Pure accounting for DVT snapshots; no device access or process-name ownership.

Ownership is the exact installed main executable's observed nonzero kernel
coalition. Sums are per-process physical-footprint ledgers, not Android PSS,
unique physical RAM, or an assertion about WebKit services outside that coalition.
"""
import statistics
from collections import Counter

SCOPE = "Observed main-executable kernel coalition; per-process footprint ledger sum, not PSS or deduplicated RAM"
ROLES = ("main", "web_content", "gpu", "networking", "other")
FIELDS = ("pid", "startAbsTime", "coalitionID", "name", "execName", "physFootprint", "memCompressed")


def natural(value, positive=False):
    return type(value) is int and value >= int(positive)


def role(process, main_pid):
    if process["pid"] == main_pid:
        return "main"
    name = process.get("name", "")
    for prefix, result in (("com.apple.WebKit.WebContent", "web_content"),
                           ("com.apple.WebKit.GPU", "gpu"),
                           ("com.apple.WebKit.Networking", "networking")):
        if isinstance(name, str) and name.startswith(prefix):
            return result
    return "other"


def snapshot_footprint(processes, executable, competing_executables=()):
    if not isinstance(processes, list) or any(not isinstance(row, dict) for row in processes):
        raise ValueError("Expected a DVT list of process objects")
    if not isinstance(executable, str) or not executable.startswith("/"):
        raise ValueError("An exact absolute installed executable path is required")
    result = {"status": "unavailable", "scope": SCOPE, "reason": None,
              "main_identity": None, "member_identity": [], "processes": [],
              "bytes": {key: None for key in (*ROLES, "observed_coalition", "compressed")}}
    roots = [row for row in processes if row.get("execName") == executable]
    if len(roots) != 1:
        result["reason"] = "main-executable-not-unique"
        return result
    main = roots[0]
    if not natural(main.get("pid"), True) or not natural(main.get("startAbsTime"), True):
        result["reason"] = "invalid-main-generation"
        return result
    if natural(main.get("physFootprint")):
        result["bytes"]["main"] = main["physFootprint"]
    coalition = main.get("coalitionID")
    result["main_identity"] = [main["pid"], main["startAbsTime"], coalition]
    if not natural(coalition, True):
        result["reason"] = "missing-kernel-coalition"
        return result
    if any(row.get("execName") in competing_executables and row.get("coalitionID") == coalition
           for row in processes):
        result["reason"] = "competing-main-in-coalition"
        return result
    members = [row for row in processes if row.get("coalitionID") == coalition]
    pids = [row.get("pid") for row in members]
    pid_counts = Counter(row.get("pid") for row in processes if natural(row.get("pid"), True))
    if any(not natural(pid, True) or pid_counts[pid] != 1 for pid in pids):
        result["reason"] = "ambiguous-coalition-process-identity"
        return result
    if any(not natural(row.get("startAbsTime"), True) for row in members):
        result["reason"] = "missing-member-generation"
        return result
    result["member_identity"] = sorted([[row["pid"], row["startAbsTime"]] for row in members])
    for row in members:
        evidence = {key: row.get(key) for key in FIELDS}
        evidence["role"] = role(row, main["pid"])
        result["processes"].append(evidence)
    if any(not natural(row.get("physFootprint")) for row in members):
        result["reason"] = "missing-member-footprint"
        return result
    if not any(row["role"] == "web_content" for row in result["processes"]):
        result["reason"] = "reader-webcontent-not-observed"
        return result
    totals = {key: 0 for key in ROLES}
    for row in result["processes"]:
        totals[row["role"]] += row["physFootprint"]
    totals["observed_coalition"] = sum(totals.values())
    totals["compressed"] = (sum(row["memCompressed"] for row in members)
                            if all(natural(row.get("memCompressed")) for row in members) else None)
    result.update(status="complete_observed_coalition", reason=None, bytes=totals)
    return result


def checkpoint_footprint(snapshots):
    if not isinstance(snapshots, list) or not snapshots:
        raise ValueError("At least one classified snapshot is required")
    result = {"status": "unavailable", "scope": SCOPE, "reason": None,
              "sample_count": len(snapshots), "independent_sessions": 1,
              "main_identity": snapshots[0]["main_identity"],
              "member_identity": snapshots[0]["member_identity"],
              "median_bytes": {key: None for key in (*ROLES, "observed_coalition", "compressed")},
              "samples": snapshots}
    main_ids = [row["main_identity"] for row in snapshots]
    if main_ids[0] is None or any(identity != main_ids[0] for identity in main_ids[1:]):
        result["reason"] = "main-generation-changed-or-unavailable"
        return result
    main_values = [row["bytes"]["main"] for row in snapshots]
    if all(natural(value) for value in main_values):
        result["median_bytes"]["main"] = statistics.median(main_values)
    if any(row["status"] != "complete_observed_coalition" for row in snapshots):
        result["reason"] = "incomplete-observed-coalition"
        return result
    identities = [row["member_identity"] for row in snapshots]
    if any(identity != identities[0] for identity in identities[1:]):
        result["reason"] = "coalition-membership-or-generation-changed"
        return result
    for key in result["median_bytes"]:
        values = [row["bytes"][key] for row in snapshots]
        if all(natural(value) for value in values):
            result["median_bytes"][key] = statistics.median(values)
    result.update(status="complete_observed_coalition", reason=None)
    return result
