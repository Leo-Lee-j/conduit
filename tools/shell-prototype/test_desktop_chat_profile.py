"""A partial, empty or faulted chat profile must never publish success."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import desktop_chat_profile as probe


def trace(scenario="active", profile="dev"):
    active = 3 if scenario == "active" else 0
    records = [dict(event="header", schema_version=2, suite="chat-profile", scenario=scenario,
        samples=20, warmup=1, pid=123, debug_assertions=profile == "dev", platform="gpui-test-no-gpu",
        dataset=probe.dataset_metadata(), registered_package_count=0 if scenario == "baseline" else 20,
        active=active, window_px=[1600, 800], pace_ms=16, input=probe.INPUT)]
    records.append(dict(event="render-proof", counters={name: dict(count=1, elapsed_ms=.1) for name in probe.COUNTERS}))
    for index in range(21):
        for action in probe.ACTIONS:
            fields = dict(action=action, index=index)
            records.append(dict(event="begin", **fields))
            records.append(dict(event="end", **fields, elapsed_ms=100. if index == 0 else 2., root_renders=1,
                counters={name: dict(count=1, elapsed_ms=.1) for name in probe.COUNTERS}))
        records.append(dict(event="verified", index=index, runtime_count=active, instance_count=active,
            input=probe.INPUT if index % 2 == 0 else probe.INPUT[:-1] + "B", transcript_visible=True))
    records.append(dict(event="complete", iterations=21, runtime_count=active, instance_count=active))
    return records


class ChatProfileTests(unittest.TestCase):
    def test_all_profiles_and_scenarios_exclude_warmup(self):
        for profile in ("dev", "release"):
            for scenario in probe.SCENARIOS:
                records = trace(scenario, profile)
                probe.validate(records, scenario, 20, 1, profile)
                summary = probe.summarize(records, 1)
                self.assertEqual(summary["key-00"]["elapsed_ms"]["p95_ms"], 2.)
                self.assertEqual(set(summary), set(probe.ACTIONS))
                self.assertEqual(summary["key-00"]["root_renders"], dict(count=20, min=1, max=1, total=20))

    def test_incomplete_reordered_bad_counters_and_lost_runtime_are_rejected(self):
        records = trace()
        for data in ([], records[:1], records[:-1], records + [records[-1]], records[:2] + records[3:]):
            with self.assertRaises(ValueError):
                probe.validate(data, "active", 20, 1, "dev")
        mutations = [lambda r: r[0].update(samples=True),
            lambda r: r[0]["dataset"].update(text_sha256="wrong"),
            lambda r: r[3].update(elapsed_ms=float("nan")),
            lambda r: r[3].update(root_renders=True),
            lambda r: r[3]["counters"].pop("row_build"),
            lambda r: r[3]["counters"]["row_build"].update(count=-1),
            lambda r: r[-2].update(runtime_count=2),
            lambda r: r[-2].update(input="wrong"),
            lambda r: r[-2].update(transcript_visible=False),
            lambda r: r[1]["counters"]["markdown_paint"].update(count=0),
            lambda r: next(x for x in r if x.get("action") == "notify-draw" and x["event"] == "end").update(root_renders=0)]
        for mutate in mutations:
            data = copy.deepcopy(records)
            mutate(data)
            with self.assertRaises(ValueError):
                probe.validate(data, "active", 20, 1, "dev")

    def test_cached_frames_are_valid_only_with_render_proof_and_visibility(self):
        records = trace()
        for record in records:
            if record["event"] == "end":
                for counter in record["counters"].values():
                    counter.update(count=0, elapsed_ms=0.)
        probe.validate(records, "active", 20, 1, "dev")
        with self.assertRaises(ValueError):
            probe.validate(records[:1] + records[2:], "active", 20, 1, "dev")

    def test_failed_process_keeps_trace_and_does_not_publish_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            out = root / "artifacts"
            out.mkdir()
            historical = out / "desktop-stress-results.json"
            historical.write_bytes(b"historical")
            executable = root / "probe.exe"
            executable.write_bytes(b"fixture")
            def run(name, args, timeout=1800, env=None):
                (out / f"{name}.log").write_text("fixture", encoding="utf-8")
                if env:
                    self.assertIn(probe.PROBE, args)
                    self.assertIn("--exact", args)
                    Path(env["CONDUIT_PERF_OUTPUT"]).write_text("\n".join(json.dumps(r) for r in trace("baseline")[:3]), encoding="utf-8")
                return dict(name=name, exit=101 if env else 0)
            with patch.multiple(probe, ROOT=root, REPO=root, OUT=out), patch.object(probe, "run", side_effect=run), \
                 patch.object(probe, "built_executable", return_value=executable), \
                 patch.object(probe, "prepare_packages", return_value="fixture"), \
                 patch.object(probe.subprocess, "check_output", return_value="rustc fixture"):
                results, measurement = probe.measure([], 20, 1, 3)
            self.assertEqual(results[-1]["exit"], 101)
            self.assertEqual(measurement["summary"], [])
            self.assertEqual(measurement["failure"]["last_record"]["event"], "begin")
            self.assertEqual(historical.read_bytes(), b"historical")


if __name__ == "__main__":
    unittest.main()
