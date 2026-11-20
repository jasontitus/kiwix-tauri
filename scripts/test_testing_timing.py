# SPDX-License-Identifier: GPL-3.0-or-later
"""Host-only behavioral tests; no build tools or devices are contacted."""
from datetime import datetime, timezone
import io
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import contextmanager, redirect_stderr
from unittest import mock

import testing_timing as timing

SCRIPT = Path(timing.__file__).resolve()
# The timing ledger opens its files through a no-follow directory descriptor and
# cancels commands through process groups. Neither exists on Windows, so the
# tests that write a ledger or run a command are skipped there.
posix_host = unittest.skipUnless(os.name == "posix", "requires a POSIX host (O_NOFOLLOW dir_fd opens, process groups)")


@contextmanager
def measured_clock(now):
    with mock.patch.object(timing, "_monotonic_ns", side_effect=lambda: now[0]), \
            mock.patch.object(timing, "datetime", wraps=datetime) as clock:
        clock.now.side_effect = lambda *_args: datetime.fromtimestamp(
            1_700_000_000 + now[0] / 1e9, timezone.utc)
        yield


@posix_host
class TimingLogTests(unittest.TestCase):
    def test_overlapping_spans_use_union_not_sum_and_repeated_summary_is_stable(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            now = [1_000_000_000]
            with mock.patch.object(timing, "_monotonic_ns", side_effect=lambda: now[0]):
                log = timing.TimingLog(Path(temporary), "iteration-1", "tests")
                now[0] = 2_000_000_000
                first = log.span("host_prepare", "prepare")
                first.__enter__()
                now[0] = 4_000_000_000
                second = log.span("adb_command", "device-probe")
                second.__enter__()
                now[0] = 6_000_000_000
                first.__exit__(None, None, None)
                now[0] = 8_000_000_000
                second.__exit__(None, None, None)
                now[0] = 10_000_000_000
                log.close()
                expected = log.summary()
                now[0] = 50_000_000_000
                self.assertEqual(log.summary(), expected)
            self.assertEqual(expected["total_wall_seconds"], 9)
            self.assertEqual(expected["instrumented_union_seconds"], 6)
            self.assertEqual(expected["uninstrumented_seconds"], 3)
            self.assertEqual(expected["category_wall_seconds_inclusive"]["host_prepare"], 4)
            self.assertEqual(expected["category_wall_seconds_inclusive"]["adb_command"], 4)
            self.assertIsNone(expected["inference_seconds"])
            self.assertTrue(expected["complete"])
            self.assertEqual(json.loads((Path(temporary) / "timing-summary.json").read_text()), expected)
            saved = timing.report([temporary])["ledgers"][0]["runs"][0]
            self.assertEqual(saved["instrumented_union_seconds"], 6)
            for name in ("timing.jsonl", "timing-summary.json"):
                self.assertEqual(stat.S_IMODE((Path(temporary) / name).stat().st_mode), 0o600)

    def test_returned_failure_can_be_marked_without_changing_exit_or_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            with timing.TimingLog(Path(temporary), "returned-failure", "tests") as log:
                try:
                    with log.span("host_prepare", "failed-command") as operation:
                        result = subprocess.run([sys.executable, "-c", "raise SystemExit(23)"], timeout=10)
                        if result.returncode:
                            operation.fail()
                finally:
                    with log.span("cleanup", "cleanup"):
                        cleaned = True
            self.assertEqual(result.returncode, 23)
            self.assertTrue(cleaned)
            self.assertEqual(log.summary()["failed_operations"], 1)
            self.assertEqual(timing.report([temporary])["ledgers"][0]["runs"][0]["failed_operations"], 1)

    def test_original_failure_survives_logging_failure_and_cleanup_still_runs(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            log = timing.TimingLog(Path(temporary), "failed-iteration", "tests")
            original = RuntimeError("sensitive-error-payload")
            cleaned = []
            with mock.patch.object(timing.os, "write", side_effect=OSError("disk unavailable")):
                try:
                    with log.span("device_automation", "failed-operation"):
                        raise original
                except RuntimeError as caught:
                    self.assertIs(caught, original)
                finally:
                    with log.span("cleanup", "owned-cleanup"):
                        cleaned.append(True)
                    log.close()
            self.assertEqual(cleaned, [True])
            self.assertEqual(log.summary()["failed_operations"], 1)
            self.assertGreater(log.summary()["logging_errors"], 0)
            self.assertTrue(log.summary()["partial_measurement"])
            self.assertNotIn("sensitive-error-payload", log.path.read_text())

    def test_failed_action_retained_without_argv_path_or_exception_payload(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            with timing.TimingLog(Path(temporary), "https://private.test/token", "/private/role") as log:
                with self.assertRaisesRegex(ValueError, "secret-payload"):
                    with log.span("adb_command", "https://private.test/secret"):
                        raise ValueError("secret-payload")
            rows = [json.loads(line) for line in log.path.read_text().splitlines()]
            self.assertTrue(all(row["iteration_id"] == "redacted" for row in rows))
            self.assertTrue(all(row["role"] == "redacted" for row in rows))
            self.assertTrue(all(row.get("label", "redacted") == "redacted" for row in rows))
            self.assertNotIn("private.test", log.path.read_text())
            self.assertNotIn("secret-payload", log.path.read_text())
            self.assertEqual(timing.report([temporary])["ledgers"][0]["runs"][0]["failed_operations"], 1)

    def test_symlink_ledger_is_not_followed_or_modified(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            output = Path(temporary) / "output"
            output.mkdir()
            unrelated = Path(temporary) / "unrelated"
            unrelated.write_text("preserve-me")
            (output / "timing.jsonl").symlink_to(unrelated)
            with timing.TimingLog(output, "iteration", "tests") as log:
                with log.span("cleanup", "cleanup"):
                    pass
            self.assertEqual(unrelated.read_text(), "preserve-me")
            self.assertTrue((output / "timing.jsonl").is_symlink())
            self.assertGreater(log.summary()["logging_errors"], 0)

    def test_summary_temp_collision_preserves_unowned_file(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            log = timing.TimingLog(Path(temporary), "iteration", "tests")
            collision = Path(temporary) / (".timing-summary-" + log.run_id + ".tmp")
            collision.write_text("unrelated")
            log.close()
            self.assertEqual(collision.read_text(), "unrelated")
            self.assertGreater(log.summary()["logging_errors"], 0)

    def test_missing_output_does_not_create_directories_or_interrupt_action(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            output = Path(temporary) / "missing"
            with timing.TimingLog(output, "iteration", "tests") as log:
                with log.span("host_prepare", "prepare"):
                    completed = True
            self.assertTrue(completed)
            self.assertFalse(output.exists())
            self.assertGreater(log.summary()["logging_errors"], 0)

    def test_partial_run_uses_only_last_observed_event_and_retains_open_spans(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            now = [1_000_000_000]
            with mock.patch.object(timing, "_monotonic_ns", side_effect=lambda: now[0]):
                log = timing.TimingLog(Path(temporary), "iteration", "tests")
                now[0] = 2_000_000_000
                span = log.span("ui_wait", "ui-ready")
                span.__enter__()
                now[0] = 20_000_000_000
                saved = timing.report([temporary])["ledgers"][0]["runs"][0]
                self.assertFalse(saved["complete"])
                self.assertTrue(saved["partial_measurement"])
                self.assertEqual(saved["total_wall_seconds"], 1)
                self.assertEqual(saved["incomplete_operations"], 1)
                self.assertEqual(log.summary()["category_wall_seconds_inclusive"]["ui_wait"], 18)
                span.__exit__(None, None, None)
                log.close()

    def test_truncated_record_is_disclosed_without_losing_completed_failure(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            with timing.TimingLog(Path(temporary), "iteration", "tests") as log:
                with self.assertRaises(ValueError):
                    with log.span("build", "build"):
                        raise ValueError("failed")
            with log.path.open("ab") as stream:
                stream.write(b'{"version":1,"event":')
            ledger = timing.report([temporary])["ledgers"][0]
            self.assertEqual(ledger["rejected_events"], 1)
            self.assertEqual(ledger["runs"][0]["failed_operations"], 1)
            self.assertTrue(ledger["runs"][0]["partial_measurement"])

    def test_event_limit_discloses_partial_measurement_and_still_allows_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            with mock.patch.object(timing, "MAX_EVENTS", 2):
                with timing.TimingLog(Path(temporary), "iteration", "tests") as log:
                    with log.span("host_prepare", "prepare"):
                        pass
                    with log.span("cleanup", "cleanup"):
                        cleaned = True
            self.assertTrue(cleaned)
            self.assertTrue(log.summary()["partial_measurement"])
            self.assertGreater(log.summary()["logging_errors"], 0)


@posix_host
class CommandTimingTests(unittest.TestCase):
    def command(self, output, code):
        return [sys.executable, str(SCRIPT), "command", "--output", str(output),
                "--iteration", "command-iteration", "--category", "host_prepare",
                "--label", "host-command", "--", sys.executable, "-c", code]

    def test_failed_command_streams_are_preserved_and_exit_code_is_exact(self):
        with tempfile.TemporaryDirectory() as temporary:
            code = "import sys; print('original-out'); print('original-error', file=sys.stderr); sys.exit(37)"
            result = subprocess.run(self.command(temporary, code), capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 37)
            self.assertEqual(result.stdout, "original-out\n")
            self.assertIn("original-error\n", result.stderr)
            ledger = Path(temporary) / "timing.jsonl"
            self.assertNotIn("original-error", ledger.read_text())
            self.assertNotIn(code, ledger.read_text())
            summary = timing.report([ledger])["ledgers"][0]["runs"][0]
            self.assertEqual(summary["failed_operations"], 1)
            self.assertTrue(summary["complete"])
            self.assertGreater(summary["instrumented_union_seconds"], 0)

    def test_successful_command_and_report_cli_preserve_actual_file_effect(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifact = Path(temporary) / "command-artifact"
            code = ("import os; from pathlib import Path; Path(" + repr(str(artifact)) +
                    ").write_text(os.environ['KIWIX_TIMING_ITERATION'])")
            result = subprocess.run(self.command(temporary, code), capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(artifact.read_text(), "command-iteration")
            result = subprocess.run([sys.executable, str(SCRIPT), "report", temporary],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0)
            summary = json.loads(result.stdout)["ledgers"][0]["runs"][0]
            self.assertEqual(summary["failed_operations"], 0)
            self.assertTrue(summary["complete"])
            self.assertNotIn(str(artifact), (Path(temporary) / "timing.jsonl").read_text())

    def test_child_signal_is_propagated_as_signal_not_success_or_generic_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = subprocess.run(self.command(temporary, "import os, signal; os.kill(os.getpid(), signal.SIGTERM)"),
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, -signal.SIGTERM)
            self.assertEqual(timing.report([temporary])["ledgers"][0]["runs"][0]["failed_operations"], 1)

    def test_command_failure_survives_unwritable_timing_location(self):
        with tempfile.TemporaryDirectory() as temporary:
            missing = Path(temporary) / "missing"
            result = subprocess.run(self.command(missing, "raise SystemExit(43)"),
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 43)
            self.assertIn("logging", result.stderr)
            self.assertFalse(missing.exists())

    def test_cancellation_escalates_ignored_signal_and_reaps_owned_child(self):
        with tempfile.TemporaryDirectory() as temporary:
            ready = Path(temporary) / "child-ready"
            code = ("import os, signal, time; from pathlib import Path; "
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                    "Path(" + repr(str(ready)) + ").write_text(str(os.getpid())); time.sleep(60)")
            wrapper = subprocess.Popen(self.command(temporary, code), stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
            try:
                deadline = time.monotonic() + 5
                child_pid = None
                while wrapper.poll() is None and time.monotonic() < deadline:
                    value = ready.read_text() if ready.exists() else ""
                    if value.isdigit():
                        child_pid = int(value)
                        break
                    time.sleep(0.01)
                self.assertIsNotNone(child_pid, "child failed to become ready")
                started = time.monotonic()
                wrapper.send_signal(signal.SIGTERM)
                wrapper.communicate(timeout=10)
                self.assertEqual(wrapper.returncode, -signal.SIGTERM)
                self.assertLess(time.monotonic() - started, 8)
                with self.assertRaises(ProcessLookupError):
                    os.kill(child_pid, 0)
                summary = timing.report([temporary])["ledgers"][0]["runs"][0]
                self.assertEqual(summary["failed_operations"], 1)
                self.assertEqual(summary["incomplete_operations"], 0)
                self.assertGreater(summary["category_wall_seconds_inclusive"]["cleanup"], 0)
                self.assertTrue(summary["complete"])
            finally:
                if wrapper.poll() is None:
                    wrapper.kill()
                    wrapper.communicate(timeout=5)
                value = ready.read_text() if ready.exists() else ""
                if value.isdigit():
                    try:
                        os.kill(int(value), signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_cancellation_during_spawn_keeps_child_handle_for_reap(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            log = timing.TimingLog(Path(temporary), "spawn-cancel", "tests")
            original_spawn = subprocess.Popen
            children = []

            def spawn_and_cancel(*args, **kwargs):
                process = original_spawn(*args, **kwargs)
                children.append(process)
                signal.raise_signal(signal.SIGTERM)
                return process

            try:
                with mock.patch.object(timing.subprocess, "Popen", side_effect=spawn_and_cancel):
                    code = timing.run_command(log, "host_prepare", "spawn", [sys.executable, "-c", "import time; time.sleep(60)"])
                self.assertEqual(code, -signal.SIGTERM)
                self.assertEqual(len(children), 1)
                self.assertIsNotNone(children[0].returncode)
                with self.assertRaises(ProcessLookupError):
                    os.kill(children[0].pid, 0)
            finally:
                for child in children:
                    if child.poll() is None:
                        child.kill()
                        child.wait(timeout=5)
                log.close()


@posix_host
class WholeIterationTests(unittest.TestCase):
    def marker(self, mode, output, iteration="whole-pass"):
        return timing.main([mode, "--output", str(output), "--iteration", iteration])

    def test_real_processes_share_one_iteration_clock(self):
        with tempfile.TemporaryDirectory() as temporary:
            common = ["--output", temporary, "--iteration", "cross-process"]
            started = time.monotonic()
            for operation in (
                ["begin", *common],
                ["command", *common, "--category", "host_prepare", "--label", "clock-check",
                 "--", sys.executable, "-c", "import time; time.sleep(0.04)"],
                ["finish", *common],
            ):
                subprocess.run([sys.executable, str(SCRIPT), *operation],
                               check=True, capture_output=True, timeout=10)
            elapsed = time.monotonic() - started
            whole = timing.report([temporary])["iterations"][0]
            self.assertTrue(whole["clock_consistent"])
            self.assertTrue(whole["complete"])
            self.assertFalse(whole["partial_measurement"])
            self.assertGreaterEqual(whole["category_wall_seconds_inclusive"]["host_prepare"], 0.04)
            self.assertGreaterEqual(whole["total_wall_seconds"], whole["instrumented_union_seconds"])
            self.assertLessEqual(whole["total_wall_seconds"], elapsed + 0.1)

    def test_markers_merge_separate_runs_and_ledgers_keep_gaps_and_separate_unrelated_ids(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            first = Path(temporary) / "first"
            second = Path(temporary) / "second"
            first.mkdir()
            second.mkdir()
            now = [0]
            with measured_clock(now):
                self.assertEqual(self.marker("begin", first), 0)
                now[0] = 2_000_000_000
                with timing.TimingLog(first, "whole-pass", "build") as log:
                    with log.span("build", "release-build"):
                        now[0] = 5_000_000_000
                now[0] = 8_000_000_000
                with timing.TimingLog(second, "whole-pass", "device") as log:
                    with log.span("ui_wait", "ui-ready"):
                        now[0] = 10_000_000_000
                with timing.TimingLog(second, "unrelated-pass", "other") as log:
                    with log.span("host_prepare", "prepare"):
                        now[0] = 11_000_000_000
                now[0] = 20_000_000_000
                self.assertEqual(self.marker("finish", first), 0)
                result = timing.report([first, second, first])
            iterations = {row["iteration_id"]: row for row in result["iterations"]}
            self.assertEqual(set(iterations), {"whole-pass", "unrelated-pass"})
            whole = iterations["whole-pass"]
            self.assertTrue(whole["complete"])
            self.assertEqual(whole["scope"], "explicit_begin_finish")
            self.assertEqual(whole["total_wall_seconds"], 20)
            self.assertEqual(whole["instrumented_union_seconds"], 5)
            self.assertEqual(whole["uninstrumented_seconds"], 15)
            self.assertEqual(whole["category_wall_seconds_inclusive"]["build"], 3)
            self.assertEqual(whole["category_wall_seconds_inclusive"]["ui_wait"], 2)
            self.assertEqual(len(whole["run_ids"]), 4)
            self.assertIsNone(whole["inference_seconds"])
            self.assertEqual(iterations["unrelated-pass"]["total_wall_seconds"], 1)
            self.assertFalse(iterations["unrelated-pass"]["complete"])

    def test_marker_only_envelope_is_all_residual_not_measured_activity(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            now = [2_000_000_000]
            with measured_clock(now):
                self.assertEqual(self.marker("begin", temporary), 0)
                now[0] = 12_000_000_000
                self.assertEqual(self.marker("finish", temporary), 0)
                whole = timing.report([temporary])["iterations"][0]
            self.assertEqual(whole["total_wall_seconds"], 10)
            self.assertEqual(whole["instrumented_union_seconds"], 0)
            self.assertEqual(whole["uninstrumented_seconds"], 10)
            self.assertTrue(all(value == 0 for value in whole["category_wall_seconds_inclusive"].values()))

    def test_unfinished_iteration_is_lower_bound_and_duplicate_markers_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            now = [0]
            with measured_clock(now):
                with self.assertRaises(SystemExit) as caught:
                    self.marker("finish", temporary)
                self.assertEqual(caught.exception.code, 2)
                self.assertFalse((Path(temporary) / "timing.jsonl").exists())
                self.assertEqual(self.marker("begin", temporary), 0)
                before = (Path(temporary) / "timing.jsonl").read_bytes()
                with self.assertRaises(SystemExit) as caught:
                    self.marker("begin", temporary)
                self.assertEqual(caught.exception.code, 2)
                self.assertEqual((Path(temporary) / "timing.jsonl").read_bytes(), before)
                now[0] = 2_000_000_000
                with timing.TimingLog(Path(temporary), "whole-pass", "prepare") as log:
                    with log.span("host_prepare", "prepare"):
                        now[0] = 5_000_000_000
                now[0] = 50_000_000_000
                whole = timing.report([temporary])["iterations"][0]
            self.assertFalse(whole["complete"])
            self.assertTrue(whole["partial_measurement"])
            self.assertEqual(whole["total_wall_seconds"], 5)
            self.assertEqual(whole["instrumented_union_seconds"], 3)
            self.assertEqual(whole["uninstrumented_seconds"], 2)

    def test_inconsistent_clock_anchors_are_not_reported_as_merged_measurements(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()):
            first = Path(temporary) / "first"
            second = Path(temporary) / "second"
            first.mkdir()
            second.mkdir()
            now = [0]
            with measured_clock(now):
                with timing.TimingLog(first, "whole-pass", "first"):
                    pass
            # Simulate another host/boot with a different monotonic epoch.
            now[0] = 100_000_000_000
            with mock.patch.object(timing.time, "monotonic_ns", side_effect=lambda: now[0]), \
                    mock.patch.object(timing, "datetime", wraps=datetime) as clock:
                clock.now.return_value = datetime.fromtimestamp(1_700_000_000, timezone.utc)
                with timing.TimingLog(second, "whole-pass", "second"):
                    pass
                whole = timing.report([first, second])["iterations"][0]
            self.assertFalse(whole["clock_consistent"])
            self.assertIsNone(whole["total_wall_seconds"])
            self.assertIsNone(whole["instrumented_union_seconds"])
            self.assertIsNone(whole["uninstrumented_seconds"])
            self.assertTrue(whole["partial_measurement"])


if __name__ == "__main__":
    unittest.main()
