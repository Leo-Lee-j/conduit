"""Reject partial diagnostics and preserve evidence from failed subprocesses."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import desktop_diagnostics as probe
from desktop_stress import dataset_metadata


def trace(driver="explicit-draw"):
    records = [{"event": "header", "schema_version": 1, "suite": "diagnostics",
                "platform": "gpui-test-no-gpu", "driver": driver, "samples": 20,
                "warmup": 1, "pid": 123, "debug_assertions": True, "dataset": dataset_metadata(),
                "registered_package_count": 20, "pace_ms": 16, "window_px": [1600, 800]}]
    state = dict(root_renders=0, runtime_count=0, instance_count=0, tasks=0,
                 result=None, error=None, view_error=None, runtimes=[])
    sequence = 0

    def append(event, **fields):
        records.append(dict(event=event, state=copy.deepcopy(state), **fields))

    def stage(action, index, phase):
        nonlocal sequence
        fields = dict(action=action, index=index, phase=phase, sequence=sequence)
        append("begin", **fields)
        state["root_renders"] += 1
        if action == "open":
            state["runtime_count"] += 1
            state["instance_count"] += 1
        detail = dict(dispatch_ms=.2, accepted=True) if phase == "dispatch-effects" else None
        append("end", **fields, elapsed_ms=1., detail=detail)
        sequence += 1
        if action == "open":
            state["runtimes"].append(dict(owner=f"stress.plugin-{index:02}", error=None,
                                          **{key: 0 for key in (*probe.COUNTERS, *probe.TIMES)}))

    append("ready")
    for action in ("host-rows", "host-draw"):
        for index in range(21):
            stage(action, index, "update")
    for index in range(3):
        stage("open", index, "update")
    for index in range(21):
        stage("cached-draw", index, "update")
    count = 0
    for action in ("local", "metadata"):
        for index in range(21):
            state["result"] = None
            stage(action, index, "dispatch-effects")
            stage(action, index, "settle")
            if driver == "explicit-draw":
                stage(action, index, "draw")
            count += 1
            state["result"] = str(count)
            append("verified", action=action, index=index, expected_count=count)
    append("complete", commands=count, stages=sequence)
    return records


class DiagnosticsTests(unittest.TestCase):
    def test_both_drivers_validate_and_summarize_actual_counts(self):
        for driver in probe.DRIVERS:
            records = trace(driver)
            probe.validate(records, driver, 20, 1, "dev")
            summary = probe.summarize(records, 1)
            self.assertEqual(summary["local/settle"]["root_renders"], dict(count=20, min=1, max=1, total=20))
            self.assertEqual("local/draw" in summary, driver == "explicit-draw")

    def test_truncated_and_reordered_stages_are_never_valid(self):
        records = trace()
        for end in (0, 1, 2, 3, len(records) - 1):
            with self.subTest(end=end), self.assertRaises(ValueError):
                probe.validate(records[:end], "explicit-draw", 20, 1, "dev")
        for invalid in (records + [records[-1]], records[:2] + records[3:], records[:3] + records[2:]):
            with self.assertRaises(ValueError):
                probe.validate(invalid, "explicit-draw", 20, 1, "dev")

    def test_faults_bad_numbers_wrong_workload_and_unsettled_results_fail(self):
        original = trace()
        mutations = [
            lambda r: r[0].update(driver="other"),
            lambda r: r[0].update(samples=True),
            lambda r: r[0]["dataset"].update(text_sha256="wrong"),
            lambda r: r[3].update(elapsed_ms=float("nan")),
            lambda r: r[3]["state"].update(error="execution deadline exceeded"),
            lambda r: r[3]["state"].update(root_renders=True),
            lambda r: r[-1]["state"].update(runtime_count=2),
            lambda r: r[-2]["state"].update(tasks=1),
            lambda r: r[-2]["state"].update(result=None),
            lambda r: r[-2]["state"]["runtimes"][0].update(error="fault"),
            lambda r: r[-2]["state"]["runtimes"][0].update(materialize_ms=-1),
            lambda r: r[-1]["state"].update(root_renders=0),
            lambda r: next(x for x in r if x.get("detail") is not None)["detail"].update(accepted=False),
        ]
        for mutate in mutations:
            invalid = copy.deepcopy(original)
            mutate(invalid)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                probe.validate(invalid, "explicit-draw", 20, 1, "dev")

    def test_partial_last_line_retains_first_fault_and_stage(self):
        records = trace()[:4]
        records[-1]["state"]["error"] = "execution deadline exceeded [entry=render]"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.jsonl"
            path.write_text("\n".join(json.dumps(r) for r in records) + '\n{"event":', encoding="utf-8")
            prefix, error = probe.read_trace(path)
            self.assertIsNotNone(error)
            self.assertEqual(prefix, records)
            failure = probe.failure_context(prefix)
            self.assertEqual(failure["first_fault"]["phase"], "update")

    def test_failed_runner_keeps_trace_and_has_no_success_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = root / "artifacts"
            out.mkdir()
            historical = out / "desktop-breakdown-results.json"
            historical.write_bytes(b"historical evidence")
            executable = root / "probe.exe"
            executable.write_bytes(b"fixture")

            def fake_run(name, args, timeout=1800, env=None):
                log = out / f"{name}.log"
                log.write_text("build fixture", encoding="utf-8")
                if env:
                    self.assertIn(probe.PROBE, args)
                    records = trace(env["CONDUIT_DIAGNOSTIC_DRIVER"])[:5]
                    Path(env["CONDUIT_PERF_OUTPUT"]).write_text("\n".join(json.dumps(r) for r in records) + "\n", encoding="utf-8")
                return dict(name=name, exit=101 if env else 0, log=str(log))

            with patch.multiple(probe, ROOT=root, REPO=root, OUT=out), patch.object(probe, "run", side_effect=fake_run), \
                 patch.object(probe, "built_executable", return_value=executable), \
                 patch.object(probe, "prepare_packages", return_value="fixture"), \
                 patch.object(probe.subprocess, "check_output", return_value="rustc fixture"):
                results, measurement = probe.measure([], 20, 1, 3)
            self.assertEqual(results[-1]["exit"], 101)
            self.assertEqual(measurement["summary"], [])
            self.assertEqual(len(measurement["raw_traces"]), 1)
            self.assertEqual(measurement["runs"][0]["last_record"]["event"], "begin")
            self.assertEqual(historical.read_bytes(), b"historical evidence")


if __name__ == "__main__":
    unittest.main()
