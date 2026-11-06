"""Stress evidence validation, workload identity, and runner isolation tests."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import desktop_performance
from desktop_stress import (STRESS_PROBE, STRESS_SCENARIOS, dataset_metadata, growth,
                            prepare_packages, summarize_stress, validate_stress)


def report(scenario="baseline", cycles=3):
    memory = {"working_set_bytes": 100, "private_bytes": 200, "peak_working_set_bytes": 300}
    active = 3 if scenario in ("active", "mixed") else 0
    timings = {"host_edit_draw_ms": [1.] * 20, "host_notify_draw_ms": [2.] * 20}
    if active:
        timings["three_plugin_commands_draw_ms"] = [3.] * 20
    curve = []
    if scenario in ("lifecycle", "idle"):
        for index in range(cycles):
            count = 3 if scenario == "lifecycle" else 0
            curve.append({"cycle": index + 1, "active_runtime_count": count,
                          "active_instance_count": count, "stopped_runtime_count": 0,
                          "stopped_instance_count": 0, "released_runtime_count": count,
                          "activate_draw_ms": 1., "stop_draw_ms": 1.,
                          **{phase: dict(memory) for phase in ("ready", "after_stop", "settled")}})
    fault = None
    if scenario == "mixed":
        fault = {"owner": "stress.plugin-19", "kind": "sync-command-deadline",
                 "error": "execution deadline exceeded", "elapsed_ms": 100., "runtime_released": True,
                 "healthy_runtimes_preserved": 3, "healthy_commands_after_fault": 3,
                 "remaining_runtime_count": 3, "remaining_instance_count": 3}
    return {"schema_version": 1, "suite": "stress", "scenario": scenario,
            "platform": "gpui-test-no-gpu", "debug_assertions": True,
            "window_px": [1600, 800], "samples": 20, "warmup": 1, "pace_ms": 16,
            "cycles": cycles, "dataset": dataset_metadata(),
            "registered_package_count": 0 if scenario == "baseline" else 20,
            "runtime_count": active, "instance_count": active,
            "registered_runtime_count": 0, "registered_instance_count": 0,
            "visible_plugin_view_count": int(active > 0),
            "timings": timings, "fault": fault, "lifecycle": curve,
            "memory": {phase: dict(memory) for phase in ("before_fixture", "registered", "ready", "after_samples")}}


class StressTests(unittest.TestCase):
    def test_all_scenarios_and_missing_metadata(self):
        for scenario in STRESS_SCENARIOS:
            valid = report(scenario)
            validate_stress(valid, scenario, 20, 1, "dev", 3)
            for key in ("dataset", "suite", "registered_package_count", "runtime_count", "instance_count",
                        "registered_runtime_count", "registered_instance_count", "visible_plugin_view_count"):
                invalid = copy.deepcopy(valid)
                del invalid[key]
                with self.subTest(scenario=scenario, key=key), self.assertRaises(ValueError):
                    validate_stress(invalid, scenario, 20, 1, "dev", 3)
        for key in ("registered_package_count", "runtime_count", "instance_count", "registered_runtime_count"):
            invalid = report("active")
            invalid[key] += 1
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_stress(invalid, "active", 20, 1, "dev", 3)
        invalid = report()
        invalid["dataset"]["text_sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            validate_stress(invalid, "baseline", 20, 1, "dev", 3)
        with self.assertRaises(ValueError):
            validate_stress(report(), "baseline", 20, 1, "release", 3)

    def test_fault_requires_isolation_and_recovery_evidence(self):
        for key, value in (("runtime_released", False), ("healthy_runtimes_preserved", 2),
                           ("healthy_commands_after_fault", 0), ("remaining_runtime_count", 4),
                           ("error", "other error"), ("elapsed_ms", float("nan"))):
            invalid = report("mixed")
            invalid["fault"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_stress(invalid, "mixed", 20, 1, "dev", 3)
        invalid = report("mixed")
        invalid["fault"] = None
        with self.assertRaises(ValueError):
            validate_stress(invalid, "mixed", 20, 1, "dev", 3)

    def test_incomplete_curves_and_invalid_measurements(self):
        mutations = [lambda r: r["lifecycle"].pop(),
                     lambda r: r["lifecycle"][0].pop("settled"),
                     lambda r: r["lifecycle"][1].update(cycle=1),
                     lambda r: r["lifecycle"][0].update(released_runtime_count=0),
                     lambda r: r["lifecycle"][0].update(stopped_runtime_count=1),
                     lambda r: r["lifecycle"][0]["after_stop"].update(private_bytes=None),
                     lambda r: r["timings"]["host_edit_draw_ms"].pop(),
                     lambda r: r["memory"].pop("ready")]
        for mutate in mutations:
            invalid = report("lifecycle")
            mutate(invalid)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                validate_stress(invalid, "lifecycle", 20, 1, "dev", 3)

    def test_signed_memory_growth_and_separate_idle_control(self):
        self.assertEqual(growth([30, 20, 10]), {"first_bytes": 30, "last_bytes": 10,
                         "last_minus_first_bytes": -20, "max_above_first_bytes": 0,
                         "slope_bytes_per_cycle": -10.})
        self.assertEqual(growth([10, 40, 20])["max_above_first_bytes"], 30)
        with self.assertRaises(ValueError):
            growth([10, 20])
        reports = [report(scenario) for scenario in STRESS_SCENARIOS]
        for point in reports[4]["lifecycle"]:
            point["settled"]["private_bytes"] += point["cycle"] * 10
        summary = summarize_stress(reports)
        self.assertEqual(summary["lifecycle"]["settled_growth"]["private_bytes"]["median_slope_bytes_per_cycle"], 10.)
        self.assertEqual(summary["idle"]["settled_growth"]["private_bytes"]["median_slope_bytes_per_cycle"], 0.)
        self.assertEqual(summary["lifecycle"]["leak_verdict"], "not_evaluated")
        with self.assertRaises(ValueError):
            summarize_stress(reports[:-1])

    def test_twenty_unique_packages_and_exactly_one_fault(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = prepare_packages(root / "first")
            self.assertEqual(first, prepare_packages(root / "second"))
            owners, ids = set(), set()
            for folder in (root / "first").iterdir():
                manifest = json.loads((folder / "gpui-shell.json").read_text())
                owners.add(manifest["id"])
                config = json.loads((folder / "conduit-plugin.json").read_text())
                for entries in config["contributes"].values():
                    for item in entries:
                        self.assertNotIn(item["id"], ids)
                        ids.add(item["id"])
                        self.assertTrue(item["id"].startswith(manifest["id"] + "."))
                script = (folder / "summary.js").read_text(encoding="utf-8")
                self.assertEqual("while (true) {}" in script, manifest["id"] == "stress.plugin-19")
            self.assertEqual(len(owners), 20)
            self.assertEqual(len(ids), 140)

    def test_stress_runner_keeps_performance_artifacts_and_uses_exact_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = root / "artifacts"
            out.mkdir()
            historical = out / "desktop-performance-results.json"
            historical.write_bytes(b"existing evidence")
            executable = root / "probe.exe"
            executable.write_bytes(b"test fixture")
            calls = []

            def run(name, args, timeout=1800, env=None):
                calls.append(args)
                if env is None:
                    artifact = {"reason": "compiler-artifact", "target": {"name": "conduit-shell-desktop"},
                                "profile": {"test": True}, "executable": str(executable)}
                    (out / f"{name}.log").write_text(json.dumps(artifact), encoding="utf-8")
                else:
                    self.assertIn(STRESS_PROBE, args)
                    self.assertIn("--exact", args)
                    self.assertIn("--ignored", args)
                    self.assertEqual(env["CONDUIT_STRESS_CYCLES"], "3")
                    value = report(env["CONDUIT_PERF_SCENARIO"])
                    value["debug_assertions"] = False
                    Path(env["CONDUIT_PERF_OUTPUT"]).write_text(json.dumps(value), encoding="utf-8")
                return {"exit": 0}

            with patch.multiple(desktop_performance, OUT=out, ROOT=root, REPO=root), \
                    patch.object(desktop_performance, "run", side_effect=run), \
                    patch.object(desktop_performance.subprocess, "check_output", return_value="rustc test"):
                results, measurement = desktop_performance.measure([], 20, 1, 1, "release", stress_cycles=3)
            self.assertTrue(all(result["exit"] == 0 for result in results))
            self.assertEqual(len(results), 7)
            self.assertEqual(calls[0].count("--release"), 1)
            self.assertEqual(len(measurement["raw_reports"]), 6)
            self.assertTrue(all("desktop-stress/" in path.replace("\\", "/") for path in measurement["raw_reports"]))
            self.assertEqual(historical.read_bytes(), b"existing evidence")
            self.assertEqual(measurement["production_gate"], "not_evaluated")


if __name__ == "__main__":
    unittest.main()
