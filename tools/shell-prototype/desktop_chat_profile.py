"""Input scalars, actual test draws and nested Markdown element costs."""
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from datetime import datetime, timezone

from desktop_diagnostics import read_trace
from desktop_performance import built_executable, distribution
from desktop_stress import ACTIVE, PACKAGES, dataset_metadata, prepare_packages
from verify import OUT, REPO, ROOT, run

PROBE = "app::plugin_ui::shell::tests::chat_profile::desktop_chat_profile_probe"
SCENARIOS = ("baseline", "registered", "active")
INPUT = "G5 中文😀 input A"
ACTIONS = ("rows-sync", "select-all", *(f"key-{i:02}" for i in range(len(INPUT))), "input-draw", "notify-draw")
COUNTERS = ("row_build", "markdown_request_layout", "markdown_prepaint", "markdown_paint")


def number(value, integer=False):
    if type(value) not in ((int,) if integer else (int, float)) or not math.isfinite(value) or value < 0:
        raise ValueError("invalid duration or count")


def validate(records, scenario, samples, warmup, profile):
    if scenario not in SCENARIOS or profile not in ("dev", "release"):
        raise ValueError("unknown scenario/profile")
    active = ACTIVE if scenario == "active" else 0
    expected = dict(event="header", schema_version=2, suite="chat-profile", scenario=scenario,
                    samples=samples, warmup=warmup, debug_assertions=profile == "dev",
                    platform="gpui-test-no-gpu", dataset=dataset_metadata(),
                    registered_package_count=0 if scenario == "baseline" else PACKAGES,
                    active=active, window_px=[1600, 800], pace_ms=16, input=INPUT)
    if not records or any(type(records[0].get(k)) is not type(v) or records[0].get(k) != v for k, v in expected.items()):
        raise ValueError("chat workload differs from requested run")
    number(records[0].get("pid"), True)
    if not records[0]["pid"]:
        raise ValueError("missing process identity")
    cursor = 1

    def take(**fields):
        nonlocal cursor
        if cursor >= len(records):
            raise ValueError("incomplete chat trace")
        record = records[cursor]
        cursor += 1
        if any(type(record.get(k)) is not type(v) or record.get(k) != v for k, v in fields.items()):
            raise ValueError("out-of-order chat trace")
        return record

    proof = take(event="render-proof").get("counters")
    if not isinstance(proof, dict) or set(proof) != set(COUNTERS):
        raise ValueError("missing render proof")
    for counter in proof.values():
        number(counter.get("count"), True)
        number(counter.get("elapsed_ms"))
        if not counter["count"]:
            raise ValueError("Markdown was never materialized")
    for index in range(samples + warmup):
        for action in ACTIONS:
            take(event="begin", action=action, index=index)
            item = take(event="end", action=action, index=index)
            number(item.get("elapsed_ms"))
            number(item.get("root_renders"), True)
            counters = item.get("counters")
            if not isinstance(counters, dict) or set(counters) != set(COUNTERS):
                raise ValueError("missing chat counters")
            for counter in counters.values():
                number(counter.get("count"), True)
                number(counter.get("elapsed_ms"))
            if action == "notify-draw" and not item["root_renders"]:
                raise ValueError("chat was not rendered")
        take(event="verified", index=index, runtime_count=active, instance_count=active,
             input=INPUT if index % 2 == 0 else INPUT[:-1] + "B", transcript_visible=True)
    take(event="complete", iterations=samples + warmup, runtime_count=active, instance_count=active)
    if cursor != len(records):
        raise ValueError("unexpected trailing trace data")


def summarize(records, warmup):
    metrics = {}
    for item in records:
        if item["event"] != "end" or item["index"] < warmup:
            continue
        metric = metrics.setdefault(item["action"], {})
        values = {"elapsed_ms": item["elapsed_ms"], "root_renders": item["root_renders"]}
        for key, counter in item["counters"].items():
            values[key + "_ms"] = counter["elapsed_ms"]
            values[key + "_count"] = counter["count"]
        for key, value in values.items():
            metric.setdefault(key, []).append(value)
    return {action: {key: (distribution(values) if key.endswith("_ms") else
                           dict(count=len(values), min=min(values), max=max(values), total=sum(values)))
                     for key, values in metric.items()} for action, metric in metrics.items()}


def measure(common, samples=60, warmup=10, repeats=3, profile="dev"):
    parent = OUT / "desktop-chat-profile"
    parent.mkdir(exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-"), dir=parent))
    prefix = folder.relative_to(OUT).as_posix()
    args = ["cargo", "test", *common, "--features", "ui-tests", "--no-run", "--message-format=json"]
    if profile == "release":
        args.append("--release")
    results = [run(f"{prefix}/build", args)]
    measurement = dict(platform="gpui-test-no-gpu", profile=profile, production_gate="not_evaluated",
                       samples=samples, warmup=warmup, repeats=repeats, artifact_directory=str(folder.relative_to(ROOT)),
                       raw_traces=[], summary=[])
    if results[-1]["exit"] != 0:
        return results, measurement
    try:
        built = built_executable((folder / "build.log").read_text(encoding="utf-8"))
        executable = folder / built.name
        shutil.copy2(built, executable)
        measurement["executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
        measurement["rustc"] = subprocess.check_output(["rustc", "-Vv"], text=True).strip()
        package_root = folder / "packages"
        measurement["packages_sha256"] = prepare_packages(package_root)
        summaries = []
        for repeat in range(repeats):
            order = SCENARIOS[repeat % len(SCENARIOS):] + SCENARIOS[:repeat % len(SCENARIOS)]
            for scenario in order:
                name = f"{repeat + 1}-{scenario}"
                output = folder / f"{name}.jsonl"
                relative = str(output.relative_to(ROOT))
                measurement["raw_traces"].append(relative)
                env = os.environ.copy()
                env.update(CONDUIT_PERF_SCENARIO=scenario, CONDUIT_PERF_SAMPLES=str(samples),
                           CONDUIT_PERF_WARMUP=str(warmup), CONDUIT_PERF_OUTPUT=str(output),
                           CONDUIT_STRESS_PACKAGES=str(package_root),
                           CONDUIT_DATA_DIR=str(REPO / "target" / "shell-chat-profile-data" / folder.name / name))
                results.append(run(f"{prefix}/{name}", [executable, PROBE, "--exact", "--ignored", "--test-threads=1", "--nocapture"],
                                   timeout=max(180, (samples + warmup) * 1.5 + 90), env=env))
                records, error = read_trace(output) if output.exists() else ([], "trace missing")
                if results[-1]["exit"] != 0 or error:
                    measurement["failure"] = dict(trace=relative, error=error, last_record=records[-1] if records else None)
                    if results[-1]["exit"] == 0:
                        results.append(dict(name="chat-trace-validation", exit=1, error=error))
                    return results, measurement
                validate(records, scenario, samples, warmup, profile)
                summaries.append(dict(scenario=scenario, repeat=repeat + 1, metrics=summarize(records, warmup)))
        measurement["summary"] = summaries
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        results.append(dict(name="chat-profile-validation", exit=1, error=str(error)))
    return results, measurement
