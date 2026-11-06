"""Guard against misleading or incomplete performance reports."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from desktop_performance import SCENARIOS, built_executable, distribution, summarize, validate


def report(scenario="baseline"):
    memory = {"working_set_bytes": 100, "private_bytes": 200, "peak_working_set_bytes": 300}
    timings = {"host_edit_draw_ms": list(range(1, 21)), "host_notify_draw_ms": [1.] * 20}
    if scenario == "dual":
        timings["plugin_command_draw_ms"] = [2.] * 20
    counts = {"navigation": (1, 1), "dual": (1, 2), "hidden": (1, 2)}.get(scenario, (0, 0))
    return {"schema_version": 1, "scenario": scenario, "samples": 20, "warmup": 1,
            "platform": "gpui-test-no-gpu", "debug_assertions": True, "pace_ms": 16,
            "runtime_count": counts[0], "instance_count": counts[1], "timings": timings,
            "memory": {phase: dict(memory) for phase in
                       ("before_fixture", "registered", "active", "ready", "after_samples")}}


class MeasurementTests(unittest.TestCase):
    def test_nearest_rank_and_invalid_samples(self):
        self.assertEqual(distribution(list(range(1, 21))),
                         {"count": 20, "p50_ms": 10, "p95_ms": 19, "max_ms": 20})
        for values in ([], [float("nan")], [float("inf")], [-1], ["1"], [True]):
            with self.subTest(values=values), self.assertRaises(ValueError):
                distribution(values)

    def test_incomplete_wrong_profile_and_failed_activation_are_rejected(self):
        valid = report("dual")
        validate(valid, "dual", 20, 1, "dev")
        for key, value in (("samples", 19), ("scenario", "baseline"),
                           ("debug_assertions", False), ("runtime_count", 0)):
            bad = dict(valid, **{key: value})
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate(bad, "dual", 20, 1, "dev")
        bad = copy.deepcopy(valid)
        bad["timings"]["plugin_command_draw_ms"].pop()
        with self.assertRaises(ValueError):
            validate(bad, "dual", 20, 1, "dev")
        bad = copy.deepcopy(valid)
        bad["memory"]["ready"]["private_bytes"] = None
        with self.assertRaises(ValueError):
            validate(bad, "dual", 20, 1, "dev")

    def test_per_process_quantiles_signed_memory_and_reference_budget(self):
        reports = [report(scenario) for scenario in SCENARIOS]
        second = report("dual")
        second["timings"]["host_edit_draw_ms"] = [100.] * 20
        reports[3]["memory"]["ready"]["private_bytes"] = 100
        second["memory"]["ready"]["private_bytes"] = 100
        summary = summarize([*reports, second])
        dual = summary["dual"]
        self.assertEqual(dual["metrics"]["host_edit_draw_ms"]["median_p95_ms"], 59.5)
        self.assertEqual(dual["memory"]["median_ready_minus_registered_private_bytes"], -100)
        comparison = dual["reference_only_comparison"]["host_edit_draw_ms"]
        self.assertEqual(comparison["reference_budget_ms"], 2.)
        self.assertFalse(comparison["within_reference_budget"])
        with self.assertRaises(ValueError):
            summarize(reports[:-1])

    def test_build_artifact_must_be_unique_and_exist(self):
        with tempfile.TemporaryDirectory() as directory:
            exe = Path(directory) / "probe.exe"
            exe.touch()
            artifact = {"reason": "compiler-artifact", "target": {"name": "conduit-shell-desktop"},
                        "profile": {"test": True}, "executable": str(exe)}
            self.assertEqual(built_executable("warning\n" + json.dumps(artifact)), exe)
            with self.assertRaises(ValueError):
                built_executable(json.dumps(dict(artifact, profile={"test": False})))
            other = dict(artifact, executable=str(exe.with_name("other.exe")))
            with self.assertRaises(ValueError):
                built_executable(json.dumps(artifact) + "\n" + json.dumps(other))
            exe.unlink()
            with self.assertRaises(ValueError):
                built_executable(json.dumps(artifact))


if __name__ == "__main__":
    unittest.main()
