"""Check CPU accounting and the owned-child evidence lifecycle."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

from windows_cpu_trace import intervals, run, summarize


def sample(ms, idle=0, kernel=0, user=0, process=0, observer=0):
    return {"monotonic_ns": int(ms * 1_000_000), "read_span_ns": 100,
            "system": {"idle_100ns": idle * 10_000, "kernel_100ns": kernel * 10_000,
                       "user_100ns": user * 10_000},
            "process": {"kernel_100ns": 0, "user_100ns": process * 10_000},
            "observer": {"kernel_100ns": observer * 10_000, "user_100ns": 0}}


class AccountingTests(unittest.TestCase):
    def test_kernel_includes_idle_and_process_can_use_multiple_cores(self):
        rows = intervals([sample(0), sample(100, idle=250, kernel=300, user=100,
                                           process=150, observer=2)], 4)
        row = rows[0]
        self.assertEqual(row["system_busy_pct"], 37.5)
        self.assertEqual(row["process_core_equivalents"], 1.5)
        self.assertEqual(row["process_capacity_pct"], 37.5)
        self.assertEqual(row["observer_capacity_pct"], .5)

    def test_summary_weights_counters_and_actual_intervals(self):
        rows = intervals([sample(0), sample(100, kernel=400, process=50),
                          sample(400, idle=1200, kernel=1600, process=80)], 4)
        result = summarize(rows, 4, .1)
        self.assertEqual(result["system_busy_pct"], 25)
        self.assertEqual(result["process_capacity_pct"], 5)
        self.assertEqual(result["time_fraction_system_over_50pct"], .25)
        self.assertEqual(result["max_interval_ms"], 300)
        self.assertEqual(result["intervals_over_2x_requested"], 1)

    def test_short_final_interval_preserves_unknown_system_counter(self):
        samples = [sample(0), sample(100, kernel=400), sample(101, kernel=400)]
        rows = intervals(samples, 4)
        self.assertIsNone(rows[-1]["system_busy_pct"])
        result = summarize(rows, 4, .1)
        self.assertEqual(result["system_busy_pct"], 100)
        self.assertAlmostEqual(result["system_counter_wall_coverage"], 100 / 101)
        result = summarize(intervals([sample(0), sample(1)], 4), 4, .1)
        self.assertIsNone(result["system_busy_pct"])

    def test_rejects_regression_bad_timestamps_and_bad_counters(self):
        pairs = [[sample(1), sample(1)], [sample(2), sample(1)],
                 [sample(0, user=5), sample(100, user=4)],
                 [sample(0), sample(100, idle=10, kernel=5)]]
        bad = sample(100);bad["observer"]["user_100ns"] = float("nan")
        pairs.append([sample(0), bad])
        for pair in pairs:
            with self.subTest(pair=pair), self.assertRaises(ValueError):
                intervals(pair, 4)
        for count in [0, -1, True, 1.5]:
            with self.subTest(count=count), self.assertRaises(ValueError):
                intervals([sample(0), sample(100, kernel=100)], count)


@unittest.skipUnless(os.name == "nt", "Windows native counters")
class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def capture(self, code, **kwargs):
        return run([sys.executable, "-c", code], cwd=self.root, env=os.environ.copy(),
                   log_path=self.root / "child.log", trace_path=self.root / "trace.json", **kwargs)

    def test_real_child_counter_trace_and_cpu_work(self):
        result = self.capture("import time; end=time.perf_counter()+.4\nwhile time.perf_counter()<end: pass\nprint('finished')")
        self.assertTrue(result["valid"], result)
        self.assertGreaterEqual(len(result["raw_samples"]), 2)
        self.assertGreater(result["summary"]["process_cpu_ms"], 0)
        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(json.loads((self.root / "trace.json").read_text()), result)
        self.assertIn("finished", (self.root / "child.log").read_text())

    def test_disabled_observation_uses_same_launch_and_keeps_failure(self):
        result = self.capture("raise SystemExit(7)", interval_s=None)
        self.assertEqual(result["exit_code"], 7)
        self.assertFalse(result["valid"])
        self.assertFalse(result["observation_enabled"])
        self.assertEqual(result["raw_samples"], [])

    def test_timeout_terminates_owned_child_and_keeps_trace(self):
        result = self.capture("import time; time.sleep(30)", timeout_s=.25)
        self.assertTrue(result["timed_out"], result)
        self.assertFalse(result["valid"])
        self.assertIsNotNone(result["exit_code"])
        self.assertGreaterEqual(len(result["raw_samples"]), 2)
        self.assertLess(result["elapsed_ms"], 10_000)

    def test_existing_evidence_is_never_overwritten_or_executed(self):
        path = self.root / "trace.json";path.write_text("original")
        with self.assertRaises(FileExistsError):
            self.capture("raise RuntimeError('must not run')")
        self.assertEqual(path.read_text(), "original")
        self.assertFalse((self.root / "child.log").exists())

    def test_launch_failure_is_recorded(self):
        result = run([str(self.root / "absent.exe")], cwd=self.root, env=os.environ.copy(),
                     log_path=self.root / "child.log", trace_path=self.root / "trace.json")
        self.assertFalse(result["valid"])
        self.assertIn("FileNotFoundError", result["error"])
        self.assertIsNone(result["exit_code"])


if __name__ == "__main__":
    unittest.main()
