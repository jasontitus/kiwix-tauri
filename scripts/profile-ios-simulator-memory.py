#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Read-only host memory diagnostics for an already-running simulator app.

This tool never launches, activates, terminates, attaches to, or changes an app.
Caller owns activation/settling. A simulator resource coalition is not assumed
app-exclusive. No cohort total or WebKit attribution is produced by this tool.
Kernel contracts: SDK sys/resource.h rusage_info_v0; Apple XNU
bsd/sys/proc_info_private.h PROC_PIDCOALITIONINFO (20, five uint64_t fields),
osfmk/mach/coalition.h resource coalition index 0, jetsam index 1.
"""
import argparse
import ctypes as C
import json
from pathlib import Path
import statistics
import time


class Usage(C.Structure):
    _fields_ = [("uuid", C.c_ubyte * 16)] + [
        (name, C.c_uint64) for name in (
            "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups",
            "pageins", "wired_size", "resident_size", "phys_footprint",
            "proc_start_abstime", "proc_exit_abstime",
        )
    ]


lib = C.CDLL("/usr/lib/libproc.dylib", use_errno=True)
lib.proc_pidinfo.argtypes = [C.c_int, C.c_int, C.c_uint64, C.c_void_p, C.c_int]
lib.proc_pidinfo.restype = C.c_int
lib.proc_listpids.argtypes = [C.c_uint32, C.c_uint32, C.c_void_p, C.c_int]
lib.proc_listpids.restype = C.c_int
lib.proc_pidpath.argtypes = [C.c_int, C.c_void_p, C.c_uint32]
lib.proc_pidpath.restype = C.c_int
lib.proc_pid_rusage.argtypes = [C.c_int, C.c_int, C.c_void_p]
lib.proc_pid_rusage.restype = C.c_int


def executable(pid):
    buf = C.create_string_buffer(4096)
    return (buf.value.decode(errors="replace")
            if lib.proc_pidpath(pid, buf, len(buf)) > 0 else None)


def coalitions(pid):
    buf = (C.c_uint64 * 5)()
    size = lib.proc_pidinfo(pid, 20, 0, C.byref(buf), C.sizeof(buf))
    return list(buf)[:2] if size == C.sizeof(buf) else None


def usage(pid):
    buf = Usage()
    if lib.proc_pid_rusage(pid, 0, C.byref(buf)):
        raise OSError(C.get_errno(), f"proc_pid_rusage failed for pid {pid}")
    return {"startAbsTime": buf.proc_start_abstime,
            "physical_footprint_bytes": buf.phys_footprint,
            "resident_size_bytes": buf.resident_size,
            "wired_size_bytes": buf.wired_size}


def process_ids():
    size = lib.proc_listpids(1, 0, None, 0)
    if size <= 0:
        raise OSError(C.get_errno(), "proc_listpids size query failed")
    buf = (C.c_int * (size // C.sizeof(C.c_int) + 4096))()
    size = lib.proc_listpids(1, 0, buf, C.sizeof(buf))
    if size <= 0 or size >= C.sizeof(buf):
        raise RuntimeError("Process census failed or filled allocated buffer")
    return sorted(set(pid for pid in list(buf)[:size // C.sizeof(C.c_int)] if pid > 0))


def diagnostic_role(path, main_path):
    if path == main_path:
        return "main"
    name = Path(path).name if path else ""
    for suffix, role in (("WebContent", "web_content"), ("GPU", "gpu"),
                         ("Networking", "networking")):
        if name.startswith("com.apple.WebKit." + suffix):
            return role
    return "other"


def snapshot(pid, main_path, competing_path):
    started = time.time_ns()
    before = usage(pid)
    main_coalitions = coalitions(pid)
    if executable(pid) != main_path or not before["startAbsTime"]:
        raise RuntimeError("Main executable path or generation is not proven")
    census = []
    for candidate in process_ids():
        try:
            generation_before = usage(candidate)["startAbsTime"]
        except OSError:
            generation_before = None
        path = executable(candidate)
        coalition = coalitions(candidate)
        try:
            generation_after = usage(candidate)["startAbsTime"]
        except OSError:
            generation_after = None
        validated = bool(generation_before and generation_before == generation_after
                         and executable(candidate) == path
                         and coalitions(candidate) == coalition)
        if path is not None:
            census.append({"pid": candidate, "path": path, "coalition_ids": coalition,
                           "startAbsTime": generation_after if validated else None,
                           "identity_validated": validated,
                           "diagnostic_role": diagnostic_role(path, main_path)})
    after = usage(pid)
    if (executable(pid) != main_path or coalitions(pid) != main_coalitions
            or after["startAbsTime"] != before["startAbsTime"]):
        raise RuntimeError("Main process identity changed during capture")
    main_rows = [row for row in census if row["path"] == main_path]
    if len(main_rows) != 1 or main_rows[0]["pid"] != pid:
        raise RuntimeError("Main executable was not unique in process census")
    resource = main_coalitions[0] if main_coalitions else None
    members = [row for row in census if row["identity_validated"]
               and resource and row["coalition_ids"]
               and row["coalition_ids"][0] == resource]
    competing = [row for row in members if row["path"] == competing_path]
    reason = ("competing-main-in-resource-coalition" if competing else
              "simulator-service-ownership-not-proven" if resource else
              "main-resource-coalition-unavailable")
    return {"wall_time_start_ns": started, "wall_time_end_ns": time.time_ns(),
            "monotonic_ns": time.monotonic_ns(),
            "main": {"pid": pid, "path": main_path, "coalition_ids": main_coalitions,
                     **after},
            "cohort_status": "unavailable", "cohort_reason": reason,
            "cohort_footprint_bytes": None, "competing_main_rows": competing,
            "resource_coalition_members": members, "process_identity_census": census}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--main-executable", required=True)
    parser.add_argument("--competing-executable", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.pid <= 1 or not all(Path(p).is_absolute() for p in
                              (args.main_executable, args.competing_executable)):
        parser.error("Require PID > 1 and exact absolute executable paths")
    samples = []
    for index in range(3):
        samples.append(snapshot(args.pid, args.main_executable, args.competing_executable))
        if index != 2:
            time.sleep(2)
    identities = [(row["main"]["pid"], row["main"]["path"],
                   row["main"]["startAbsTime"], row["main"]["coalition_ids"])
                  for row in samples]
    stable = all(value == identities[0] for value in identities)
    result = {"scope": "Host-side iOS Simulator main-process diagnostic only; not physical iPhone memory, PSS, deduplicated RAM, or whole-reader memory",
              "metric": "libproc proc_pid_rusage RUSAGE_INFO_V0 ri_phys_footprint",
              "settling": "Caller-managed foreground activation and 30-second settle; no lifecycle action in this collector",
              "sample_count": 3, "sample_interval_seconds": 2,
              "independent_sessions": 1, "stable_main_generation": stable,
              "median_main_physical_footprint_bytes": statistics.median(
                  row["main"]["physical_footprint_bytes"] for row in samples) if stable else None,
              "median_main_resident_size_bytes": statistics.median(
                  row["main"]["resident_size_bytes"] for row in samples) if stable else None,
              "cohort_footprint_bytes": None, "samples": samples}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "samples"}, indent=2))


if __name__ == "__main__":
    main()
