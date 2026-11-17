#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Secret-free, best-effort host timing. Labels/IDs must be static, not user data.

Category sums include nested/overlapping spans; coverage is their union. ADB
command wall includes device execution, not just transport. Inference is not
observable here. No command arguments or exception messages enter the ledger.
"""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import threading
import time
import uuid

CATEGORIES = ("build", "host_prepare", "adb_command", "device_automation", "ui_wait", "cleanup")
MAX_EVENTS = 20000
MAX_LEDGER_BYTES = 16 * 1024 * 1024
CANCEL_GRACE_SECONDS = 2.0
_SAFE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}\Z")
INFERENCE_REASON = "Model-generation duration is not exposed; residual time is not measured inference."


def _monotonic_ns():
    # Before Python 3.10, macOS monotonic_ns() has a per-process origin.
    if sys.platform == "darwin" and sys.version_info < (3, 10):
        return time.clock_gettime_ns(time.CLOCK_MONOTONIC)
    return time.monotonic_ns()


def _name(value):
    return value if isinstance(value, str) and _SAFE_NAME.fullmatch(value) else "redacted"


def _union(intervals):
    end = None
    total = 0
    for start, stop in sorted(intervals):
        if end is None or start > end:
            total += stop - start
            end = stop
        elif stop > end:
            total += stop - end
            end = stop
    return total


def _summarize(events, end_ns=None):
    first = next((event for event in events if event["event"] == "iteration_start"), None)
    if first is None:
        raise ValueError("Timing run has no iteration start")
    start = first["monotonic_ns"]
    finish = next((event["monotonic_ns"] for event in reversed(events)
                   if event["event"] == "iteration_end"), None)
    complete = finish is not None
    finish = finish if complete else (end_ns if end_ns is not None else max(
        event["monotonic_ns"] for event in events))
    finish = max(start, finish)
    spans = {}
    totals = {category: 0 for category in CATEGORIES}
    intervals = []
    failures = 0
    incomplete = 0
    for event in events:
        if event["event"] == "span_start":
            spans[event["span_id"]] = [event, None]
        elif event["event"] == "span_end" and event["span_id"] in spans:
            spans[event["span_id"]][1] = event
    for opening, closing in spans.values():
        left = max(start, min(finish, opening["monotonic_ns"]))
        right = max(left, min(finish, closing["monotonic_ns"] if closing else finish))
        intervals.append((left, right))
        totals[opening["category"]] += right - left
        failures += int(closing is not None and closing["status"] != "ok")
        incomplete += int(closing is None)
    covered = _union(intervals)
    return {
        "run_id": first["run_id"], "iteration_id": first["iteration_id"], "role": first["role"],
        "started_utc": first["utc"], "complete": complete,
        "total_wall_seconds": (finish - start) / 1e9,
        "instrumented_union_seconds": covered / 1e9,
        "uninstrumented_seconds": max(0, finish - start - covered) / 1e9,
        "category_wall_seconds_inclusive": {key: value / 1e9 for key, value in totals.items()},
        "failed_operations": failures, "incomplete_operations": incomplete,
        "logging_errors": max((event.get("logging_errors", 0) for event in events), default=0),
        "inference_seconds": None, "inference_unavailable_reason": INFERENCE_REASON,
        "adb_measurement": "Command wall includes transport and device execution, not pure ADB latency.",
        "residual_measurement": "Uninstrumented/control-plane wall; not attributable to inference.",
        "partial_measurement": not complete or bool(incomplete),
    }


class SpanOutcome:
    """Record a returned failure without throwing or retaining error payloads."""
    def __init__(self):
        self._failed = False

    def fail(self):
        self._failed = True


class TimingLog:
    """Append bounded private events without changing the outcome of an action.

    Output must already exist. Persistence failures are disclosed once on stderr
    and in summary(), while in-memory measurement continues. Call close() after
    owned cleanup; summary() alone is a live snapshot, not an iteration boundary.
    """
    def __init__(self, output: Path, iteration_id: str, role: str):
        self.output = Path(output)
        self.path = self.output / "timing.jsonl"
        self.iteration_id = _name(iteration_id)
        self.role = _name(role)
        self.run_id = uuid.uuid4().hex
        self._events = []
        self._lock = threading.RLock()
        self._fd = None
        self._logging_errors = 0
        self._closed = False
        self._span_id = 0
        directory = None
        try:
            directory = os.open(self.output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            self._fd = os.open("timing.jsonl", os.O_WRONLY | os.O_APPEND | os.O_CREAT |
                               os.O_NOFOLLOW | os.O_NONBLOCK, 0o600, dir_fd=directory)
            info = os.fstat(self._fd)
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                    or info.st_nlink != 1 or stat.S_IMODE(info.st_mode) & 0o077):
                raise OSError("Timing ledger is not a private owned regular file")
        except Exception:
            self._logging_failure()
            self._close_fd()
        finally:
            if directory is not None:
                try:
                    os.close(directory)
                except OSError:
                    self._logging_failure()
        self._record("iteration_start")

    def _logging_failure(self):
        self._logging_errors += 1
        if self._logging_errors == 1:
            try:
                print("Timing logging unavailable or incomplete; see timing summary.", file=sys.stderr)
            except Exception:
                pass

    def _close_fd(self):
        fd, self._fd = self._fd, None
        if fd is not None:
            try:
                os.close(fd)
            except OSError:
                self._logging_failure()

    def _record(self, event, **fields):
        # The caller holds the reentrant lock for span/close operations.
        try:
            row = {"version": 1, "event": event, "run_id": self.run_id,
                   "iteration_id": self.iteration_id, "role": self.role,
                   "monotonic_ns": _monotonic_ns(),
                   "utc": datetime.now(timezone.utc).isoformat(), **fields}
            if len(self._events) >= MAX_EVENTS:
                self._logging_failure()
                return
            self._events.append(row)
            if self._fd is not None:
                data = (json.dumps(row, separators=(",", ":")) + "\n").encode("utf-8")
                if os.fstat(self._fd).st_size + len(data) > MAX_LEDGER_BYTES:
                    raise OSError("Timing ledger limit reached")
                if os.write(self._fd, data) != len(data):
                    raise OSError("Partial timing write")
        except Exception:
            self._logging_failure()
            self._close_fd()

    @contextmanager
    def span(self, category: str, label: str):
        # Invalid instrumentation is disclosed, but must not break the action.
        valid = category in CATEGORIES
        with self._lock:
            self._span_id += 1
            span_id = self._span_id
            record = valid and not self._closed
            if not record:
                self._logging_failure()
            else:
                self._record("span_start", span_id=span_id, category=category, label=_name(label))
        status = "ok"
        outcome = SpanOutcome()
        try:
            yield outcome
        except BaseException as error:
            status = "cancelled" if isinstance(error, (KeyboardInterrupt, _Cancelled)) else "error"
            raise
        finally:
            if record:
                with self._lock:
                    self._record("span_end", span_id=span_id, category=category,
                                 label=_name(label),
                                 status="error" if status == "ok" and outcome._failed else status)

    def summary(self):
        with self._lock:
            result = _summarize(self._events, _monotonic_ns())
            result["ledger"] = str(self.path)
            result["logging_errors"] = self._logging_errors
            result["partial_measurement"] |= bool(self._logging_errors)
            return result

    def _persist_summary(self):
        directory = None
        fd = None
        owned_temporary = False
        temporary = ".timing-summary-" + self.run_id + ".tmp"
        try:
            directory = os.open(self.output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         0o600, dir_fd=directory)
            owned_temporary = True
            data = (json.dumps(self.summary(), indent=2) + "\n").encode("utf-8")
            if os.write(fd, data) != len(data):
                raise OSError("Partial timing summary write")
            os.fsync(fd)
            os.close(fd)
            fd = None
            os.replace(temporary, "timing-summary.json", src_dir_fd=directory, dst_dir_fd=directory)
            owned_temporary = False
            os.fsync(directory)
        except Exception:
            self._logging_failure()
        finally:
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    self._logging_failure()
            if directory is not None:
                if owned_temporary:
                    try:
                        os.unlink(temporary, dir_fd=directory)
                    except FileNotFoundError:
                        pass
                    except OSError:
                        self._logging_failure()
                try:
                    os.close(directory)
                except OSError:
                    self._logging_failure()

    def close(self):
        with self._lock:
            if not self._closed:
                self._record("iteration_end", logging_errors=self._logging_errors)
                self._closed = True
                if self._fd is not None:
                    try:
                        os.fsync(self._fd)
                    except OSError:
                        self._logging_failure()
                self._close_fd()
                self._persist_summary()
                try:
                    print("Timing ledger: {}; summary: {}{}".format(
                        self.path, self.output / "timing-summary.json",
                        " (logging incomplete)" if self._logging_errors else ""), file=sys.stderr)
                except Exception:
                    pass

    def __enter__(self):
        return self

    def __exit__(self, *_error):
        self.close()
        return False


class _Cancelled(BaseException):
    def __init__(self, signum):
        self.signum = signum


class _CommandFailed(Exception):
    def __init__(self, returncode):
        self.returncode = returncode


def _stop_command(process, signum):
    """Bounded process-group cancellation; always attempt to reap our direct child."""
    try:
        os.killpg(process.pid, signum)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=CANCEL_GRACE_SECONDS)
    except subprocess.TimeoutExpired:
        pass
    # A group member may outlive a direct child that already exited.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=CANCEL_GRACE_SECONDS)


def run_command(log, category, label, command):
    """Inherit the original streams/argv, with no retry or execution timeout."""
    process = None
    previous = {}
    cancellation = None

    def interrupted(signum, _frame):
        nonlocal cancellation
        # Do not raise inside Popen: interruption before it returns can orphan a
        # successfully spawned child whose handle has not yet been assigned.
        cancellation = signum

    try:
        with log.span(category, label):
            for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
                previous[signum] = signal.signal(signum, interrupted)
            if cancellation is not None:
                raise _Cancelled(cancellation)
            environment = dict(os.environ, KIWIX_TIMING_ITERATION=log.iteration_id)
            process = subprocess.Popen(command, start_new_session=True, env=environment)
            while True:
                if cancellation is not None:
                    raise _Cancelled(cancellation)
                try:
                    code = process.wait(timeout=0.1)
                    if cancellation is not None:
                        raise _Cancelled(cancellation)
                    break
                except subprocess.TimeoutExpired:
                    continue
            if code:
                raise _CommandFailed(code)
            return 0
    except _CommandFailed as error:
        return error.returncode
    except OSError as error:
        try:
            print("Timing command could not be launched.", file=sys.stderr)
        except Exception:
            pass
        return 127 if isinstance(error, FileNotFoundError) else 126
    except (_Cancelled, KeyboardInterrupt) as error:
        signum = error.signum if isinstance(error, _Cancelled) else signal.SIGINT
        for number in previous:
            signal.signal(number, signal.SIG_IGN)
        if process is not None:
            try:
                with log.span("cleanup", "command-cancel-reap"):
                    _stop_command(process, signum)
            except (OSError, subprocess.TimeoutExpired):
                # Preserve cancellation while disclosing unconfirmed cleanup.
                try:
                    print("Command cancellation could not confirm process cleanup.", file=sys.stderr)
                except Exception:
                    pass
        return -signum
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def _iteration_summary(iteration_id, runs):
    """Merge same-host/boot runs; marker envelopes are never activity spans."""
    events = [event for rows in runs.values() for event in rows]
    begins = [event for event in events if event["event"] == "iteration_begin"]
    finishes = [event for event in events if event["event"] == "iteration_finish"]
    left = min(event["monotonic_ns"] for event in (begins or events))
    right = max(event["monotonic_ns"] for event in (finishes or events))
    explicit = len(begins) == len(finishes) == 1 and right >= left
    offsets = []
    for event in events:
        stamp = datetime.fromisoformat(event["utc"])
        if stamp.tzinfo is None:
            raise ValueError("Timing UTC anchor is missing its timezone")
        offsets.append(stamp.timestamp() - event["monotonic_ns"] / 1e9)
    consistent = max(offsets) - min(offsets) < 1.0
    intervals = []
    totals = {category: 0 for category in CATEGORIES}
    failures = 0
    incomplete = 0
    logging_errors = 0
    source_partial = False
    for rows in runs.values():
        summary = _summarize(rows)
        failures += summary["failed_operations"]
        incomplete += summary["incomplete_operations"]
        logging_errors += summary["logging_errors"]
        source_partial |= summary["partial_measurement"]
        run_end = max(event["monotonic_ns"] for event in rows)
        starts = {}
        ends = {}
        for event in rows:
            if event["event"] == "span_start":
                starts[event["span_id"]] = event
            elif event["event"] == "span_end":
                ends[event["span_id"]] = event
        for span_id, opening in starts.items():
            closing = ends.get(span_id)
            start = max(left, min(right, opening["monotonic_ns"]))
            stop = max(start, min(right, closing["monotonic_ns"] if closing else run_end))
            intervals.append((start, stop))
            totals[opening["category"]] += stop - start
    covered = _union(intervals)
    return {
        "iteration_id": iteration_id, "run_ids": sorted(runs),
        "scope": "explicit_begin_finish" if explicit else "observed_events_only",
        "begin_markers": len(begins), "finish_markers": len(finishes),
        "complete": explicit and not source_partial,
        "clock_consistent": consistent,
        "clock_basis": "Same-host/boot monotonic clocks; UTC anchors checked for discontinuities.",
        "total_wall_seconds": max(0, right - left) / 1e9 if consistent else None,
        "instrumented_union_seconds": covered / 1e9 if consistent else None,
        "uninstrumented_seconds": max(0, right - left - covered) / 1e9 if consistent else None,
        "category_wall_seconds_inclusive": {
            key: value / 1e9 if consistent else None for key, value in totals.items()},
        "failed_operations": failures, "incomplete_operations": incomplete,
        "logging_errors": logging_errors,
        "partial_measurement": not explicit or source_partial or not consistent,
        "inference_seconds": None, "inference_unavailable_reason": INFERENCE_REASON,
        "residual_measurement": "Includes between-command model/review/tool-roundtrip/idle gaps; not measured inference.",
        "adb_measurement": "Command wall includes transport and device execution, not pure ADB latency.",
    }


def report(paths):
    """Merge matching iteration IDs from same-host/boot ledgers, not unrelated IDs.

    Begin/finish markers retain the whole iteration envelope, including gaps;
    only spans count as measured activity. Without finish, a report is a lower
    bound ending at the last persisted event. Cross-host/reboot inputs must not
    be merged; inconsistent UTC/monotonic anchors disable aggregate durations.
    """
    ledgers = []
    all_runs = {}
    total_rejected = 0
    for value in paths:
        path = Path(value)
        if path.is_dir():
            path = path / "timing.jsonl"
        runs = {}
        rejected = 0
        with path.open("rb") as stream:
            if os.fstat(stream.fileno()).st_size > MAX_LEDGER_BYTES:
                raise ValueError("Timing ledger exceeds size limit")
            for line in stream:
                try:
                    row = json.loads(line)
                    if (row.get("version") != 1 or row.get("event") not in
                            ("iteration_start", "iteration_end", "span_start", "span_end",
                             "iteration_begin", "iteration_finish")
                            or not isinstance(row.get("monotonic_ns"), int)
                            or row["monotonic_ns"] < 0
                            or any(_name(row.get(key)) != row.get(key) for key in
                                   ("run_id", "iteration_id", "role"))
                            or not isinstance(row.get("utc"), str)):
                        raise ValueError("Invalid timing event")
                    if row["event"].startswith("span_"):
                        if (row.get("category") not in CATEGORIES
                                or not isinstance(row.get("span_id"), int)
                                or _name(row.get("label")) != row.get("label")):
                            raise ValueError("Invalid timing span")
                    if row["event"] == "span_end" and row.get("status") not in ("ok", "error", "cancelled"):
                        raise ValueError("Invalid timing status")
                    if not isinstance(row.get("logging_errors", 0), int):
                        raise ValueError("Invalid error count")
                    runs.setdefault(row["run_id"], []).append(row)
                except (ValueError, TypeError, KeyError, AttributeError):
                    rejected += 1
        summaries = []
        for events in runs.values():
            try:
                summary = _summarize(events)
            except (ValueError, KeyError, TypeError):
                rejected += len(events)
                continue
            summary["partial_measurement"] |= bool(rejected)
            summaries.append(summary)
            if len(events) > len(all_runs.get(summary["run_id"], [])):
                all_runs[summary["run_id"]] = events
        ledgers.append({"ledger": str(path), "rejected_events": rejected, "runs": summaries})
        total_rejected += rejected
    grouped = {}
    for run_id, events in all_runs.items():
        grouped.setdefault(events[0]["iteration_id"], {})[run_id] = events
    iterations = [_iteration_summary(key, runs) for key, runs in grouped.items()]
    if total_rejected:
        for iteration in iterations:
            iteration["partial_measurement"] = True
    return {"ledgers": ledgers, "iterations": iterations,
            "aggregation": "Matching iteration IDs merge across same-host/boot ledgers; unrelated IDs remain separate.",
            "inference_seconds": None, "inference_unavailable_reason": INFERENCE_REASON}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="mode", required=True)
    wrapper = commands.add_parser("command", help="Time one actual command without capturing its arguments")
    wrapper.add_argument("--output", type=Path, required=True)
    wrapper.add_argument("--iteration", required=True)
    wrapper.add_argument("--category", choices=("build", "adb_command", "host_prepare"), required=True)
    wrapper.add_argument("--label", required=True, help="Static secret-free operation name, never command data")
    wrapper.add_argument("command", nargs=argparse.REMAINDER)
    reader = commands.add_parser("report", help="Report one or more timing.jsonl files/output directories")
    reader.add_argument("ledgers", nargs="+", type=Path)
    for name in ("begin", "finish"):
        marker = commands.add_parser(name, help=name.capitalize() + " a whole-iteration envelope (not activity)")
        marker.add_argument("--output", type=Path, required=True)
        marker.add_argument("--iteration", required=True)
    args = parser.parse_args(argv)
    if args.mode == "report":
        try:
            print(json.dumps(report(args.ledgers), indent=2))
        except (OSError, ValueError):
            parser.exit(2, "Timing report could not read a valid bounded ledger.\n")
        return 0
    if args.mode in ("begin", "finish"):
        if _name(args.iteration) != args.iteration:
            parser.error("iteration must be a static secret-free identifier of at most 64 characters")
        try:
            existing = report([args.output]) if (args.output / "timing.jsonl").exists() else {"iterations": []}
            prior = next((row for row in existing["iterations"]
                          if row["iteration_id"] == args.iteration), None)
            if args.mode == "begin" and prior and prior["begin_markers"]:
                parser.error("iteration already has a begin marker; use a unique iteration ID")
            if args.mode == "finish" and (not prior or prior["begin_markers"] != 1 or prior["finish_markers"]):
                parser.error("finish requires exactly one unmatched begin marker in this output")
            with TimingLog(args.output, args.iteration, "iteration-marker") as log:
                log._record("iteration_" + args.mode)
            if log.summary()["logging_errors"]:
                parser.exit(2, "Timing marker could not be persisted completely.\n")
        except (OSError, ValueError):
            parser.exit(2, "Timing marker could not read its existing ledger.\n")
        return 0
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("command requires COMMAND after --")
    log = TimingLog(args.output, args.iteration, "command")
    try:
        code = run_command(log, args.category, args.label, command)
    finally:
        log.close()
        try:
            print(json.dumps({"timing": log.summary()}, separators=(",", ":")), file=sys.stderr)
        except Exception:
            pass
    return code


if __name__ == "__main__":
    exit_code = main()
    if exit_code < 0:
        number = -exit_code
        signal.signal(number, signal.SIG_DFL)
        os.kill(os.getpid(), number)
        exit_code = 128 + number
    sys.exit(exit_code)
