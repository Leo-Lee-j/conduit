"""Validate the component timing breakdown runner."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import desktop_breakdown
import desktop_performance
from desktop_breakdown import PROBE, SCENARIOS, compare_profiles, distribution, summarize, validate
from desktop_stress import ACTIVE, PACKAGES, dataset_metadata, prepare_packages


def report(scenario="host"):
    values = [1.] * 20
    plugin_values = [2.] * 20
    return {
        "schema_version": 2,
        "suite": "breakdown",
        "scenario": scenario,
        "pid": 123,
        "platform": "gpui-test-no-gpu",
        "debug_assertions": True,
        "window_px": [1600, 800],
        "samples": 20,
        "warmup": 1,
        "pace_ms": 16,
        "dataset": dataset_metadata(),
        "registered_package_count": 0 if scenario == "host" else PACKAGES,
        "registered_runtime_count": 0,
        "registered_instance_count": 0,
        "runtime_count": ACTIVE if scenario == "plugin" else 0,
        "instance_count": ACTIVE if scenario == "plugin" else 0,
        "first_view_count": ACTIVE if scenario == "plugin" else 0,
        "memory": {
            phase: {"working_set_bytes": 100, "private_bytes": 200, "peak_working_set_bytes": 300}
            for phase in ("before_fixture", "registered", "after")
        },
        "timings": {
            "host_chat_rows_sync_ms": values[:],
            "host_draw_ms": values[:],
        },
        "plugin_first_view_materialize_ms": [3., 4., 5.] if scenario == "plugin" else [],
        "plugin_cached_view_draw_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_local_command_dispatch_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_local_command_effects_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_local_command_settle_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_local_command_draw_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_host_module_dispatch_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_host_module_effects_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_host_module_settle_ms": plugin_values[:] if scenario == "plugin" else [],
        "plugin_host_module_draw_ms": plugin_values[:] if scenario == "plugin" else [],
    }


class BreakdownTests(unittest.TestCase):
    def test_workload_metadata_memory_and_numbers_are_strict(self):
        original = report("plugin")
        for key in ("suite", "dataset", "registered_package_count", "registered_runtime_count",
                    "registered_instance_count", "first_view_count", "pid"):
            invalid = copy.deepcopy(original)
            del invalid[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate(invalid, "plugin", 20, 1, "dev")
        for mutation in (
            lambda r: r["dataset"].update(text_sha256="0" * 64),
            lambda r: r["memory"].pop("after"),
            lambda r: r["memory"]["registered"].update(private_bytes=True),
            lambda r: r["timings"]["host_draw_ms"].pop(),
            lambda r: r.update(registered_runtime_count=False),
            lambda r: r.update(first_view_count=2),
            lambda r: r.update(plugin_local_command_settle_ms=None),
            lambda r: r.pop("plugin_host_module_dispatch_ms"),
        ):
            invalid = copy.deepcopy(original)
            mutation(invalid)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate(invalid, "plugin", 20, 1, "dev")
        with self.assertRaises(ValueError):
            validate(original, "plugin", 20, 1, "release")
        for value in (float("nan"), float("inf"), -1, True, "1"):
            invalid = copy.deepcopy(original)
            invalid["plugin_host_module_settle_ms"][0] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate(invalid, "plugin", 20, 1, "dev")

    def test_breakdown_packages_keep_stress_identity_and_host_api_bindings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stress = prepare_packages(root / "stress")
            digest = prepare_packages(root / "breakdown", breakdown=True)
            self.assertEqual(stress, prepare_packages(root / "stress-again"))
            self.assertEqual(digest, prepare_packages(root / "breakdown-again", breakdown=True))
            self.assertNotEqual(stress, digest)
            ids = set()
            for folder in (root / "breakdown").iterdir():
                manifest = json.loads((folder / "conduit-plugin.json").read_text())
                for entries in manifest["contributes"].values():
                    for item in entries:
                        self.assertNotIn(item["id"], ids)
                        ids.add(item["id"])
                metadata = next(c for c in manifest["contributes"]["commands"] if c["id"].endswith(".metadata"))
                self.assertEqual(metadata["requiredPermissions"], ["workspace.metadata.read", "session.summary.read"])
                script = (folder / "summary.js").read_text(encoding="utf-8")
                self.assertIn('getSessionSummary } from "conduit"', script)
                self.assertEqual(script.count("while (true) {}"), int(folder.name.endswith("-19")))
            self.assertEqual(len(ids), 180)

    def test_distribution_and_all_scenarios(self):
        self.assertEqual(distribution([1., 2., 3., 4.]), {
            "count": 4, "min_ms": 1., "p50_ms": 2., "p95_ms": 4., "max_ms": 4.
        })
        for scenario in SCENARIOS:
            validate(report(scenario), scenario, 20, 1, "dev")

    def test_missing_metrics_samples_materialization_and_counts_are_rejected(self):
        mutations = []
        missing = copy.deepcopy(report("plugin"))
        del missing["timings"]["host_draw_ms"]
        mutations.append(("host metric", missing))
        short = copy.deepcopy(report("plugin"))
        short["plugin_host_module_dispatch_ms"].pop()
        mutations.append(("host module samples", short))
        first = copy.deepcopy(report("plugin"))
        first["plugin_first_view_materialize_ms"].pop()
        mutations.append(("first view count", first))
        counts = copy.deepcopy(report("plugin"))
        counts["runtime_count"] = 2
        mutations.append(("runtime count", counts))
        instances = copy.deepcopy(report("plugin"))
        instances["instance_count"] = 2
        mutations.append(("instance count", instances))
        for name, invalid in mutations:
            with self.subTest(name=name), self.assertRaises(ValueError):
                validate(invalid, "plugin", 20, 1, "dev")

    def test_host_module_metric_is_required_only_for_plugin(self):
        invalid = copy.deepcopy(report("plugin"))
        invalid["plugin_host_module_draw_ms"] = []
        with self.assertRaises(ValueError):
            validate(invalid, "plugin", 20, 1, "dev")
        validate(report("registered"), "registered", 20, 1, "dev")

    def test_summary_has_ranges_and_profile_comparison(self):
        dev = summarize([report(scenario) for scenario in SCENARIOS], "dev")
        release = copy.deepcopy(dev)
        release["profile"] = "release"
        release["scenarios"]["plugin"]["metrics"]["plugin_host_module_draw_ms"]["median_p95_ms"] = 9.
        comparison = compare_profiles(dev, release)
        self.assertEqual(comparison["plugin"]["plugin_host_module_draw_ms"]["release_minus_dev_ms"], 7.)
        metric = dev["scenarios"]["plugin"]["metrics"]["plugin_first_view_materialize_ms"]
        self.assertEqual(metric["min_observed_ms"], 3.)
        self.assertEqual(metric["max_observed_ms"], 5.)

    def test_runner_uses_separate_breakdown_artifacts_and_exact_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = root / "artifacts"
            out.mkdir()
            historical = out / "desktop-performance-results.json"
            historical.write_bytes(b"existing performance evidence")
            stress_history = out / "desktop-stress-results.json"
            stress_history.write_bytes(b"existing stress evidence")
            executable = root / "probe.exe"
            executable.write_bytes(b"test fixture")
            calls = []

            def fake_run(name, args, timeout=1800, env=None):
                calls.append(args)
                log = out / f"{name}.log"
                log.parent.mkdir(parents=True, exist_ok=True)
                if env is None:
                    artifact = {"reason": "compiler-artifact", "target": {"name": "conduit-shell-desktop"},
                                "profile": {"test": True}, "executable": str(executable)}
                    log.write_text(json.dumps(artifact), encoding="utf-8")
                else:
                    self.assertIn(PROBE, args)
                    self.assertIn("--exact", args)
                    self.assertIn("--ignored", args)
                    self.assertTrue(Path(env["CONDUIT_STRESS_PACKAGES"]).is_dir())
                    value = report(env["CONDUIT_PERF_SCENARIO"])
                    value["debug_assertions"] = False
                    Path(env["CONDUIT_PERF_OUTPUT"]).write_text(json.dumps(value), encoding="utf-8")
                return {"exit": 0}

            with patch.multiple(desktop_breakdown, OUT=out, ROOT=root, REPO=root), \
                    patch.object(desktop_breakdown, "run", side_effect=fake_run), \
                    patch.object(desktop_breakdown, "built_executable", return_value=executable), \
                    patch.object(desktop_breakdown.subprocess, "check_output", return_value="rustc test"):
                results, measurement = desktop_performance.measure([], 20, 1, 1, "release", breakdown=True)
            self.assertTrue(all(result["exit"] == 0 for result in results))
            self.assertEqual(len(results), 4)
            self.assertEqual(calls[0].count("--release"), 1)
            self.assertEqual(len(measurement["raw_reports"]), 3)
            self.assertTrue(all("desktop-breakdown/" in path.replace("\\", "/") for path in measurement["raw_reports"]))
            self.assertEqual(historical.read_bytes(), b"existing performance evidence")
            self.assertEqual(stress_history.read_bytes(), b"existing stress evidence")
            self.assertEqual(measurement["production_gate"], "not_evaluated")

    def test_cli_publishes_only_breakdown_reports(self):
        spec = importlib.util.spec_from_file_location("verify_desktop", Path(__file__).with_name("verify-desktop.py"))
        cli = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cli)
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            history = [out / f"desktop-{phase}-results.json" for phase in ("performance", "stress")]
            for path in history:
                path.write_bytes(b"historical")
            with patch.multiple(cli, OUT=out, prepare=lambda: {}, source_digest=lambda: "stable"), \
                    patch.dict(cli.os.environ), \
                    patch.object(cli.sys, "argv", ["verify-desktop.py", "--phase", "breakdown", "--profile", "release"]), \
                    patch.object(desktop_performance, "measure", return_value=([{"exit": 0}], {})) as measure:
                self.assertEqual(cli.main(), 0)
            self.assertTrue(measure.call_args.kwargs["breakdown"])
            self.assertNotIn("--release", measure.call_args.args[0])
            for path in history:
                self.assertEqual(path.read_bytes(), b"historical")
            result = json.loads((out / "desktop-breakdown-release-results.json").read_text())
            self.assertEqual(result["phase"], "breakdown")
            self.assertTrue(result["measurement"]["valid_for_source_snapshot"])


if __name__ == "__main__":
    unittest.main()
