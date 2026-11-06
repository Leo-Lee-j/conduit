"""Attribute command stages and actual test-root renders; preserve failed traces."""
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from desktop_breakdown import distribution
from desktop_performance import built_executable
from desktop_stress import ACTIVE, PACKAGES, dataset_metadata, prepare_packages
from verify import OUT, REPO, ROOT, run

PROBE = "app::plugin_ui::shell::tests::diagnostics::desktop_diagnostics_probe"
DRIVERS = ("explicit-draw", "effects-only")
COUNTERS = ("script_renders", "materializations", "frame_script_calls")
TIMES = ("script_render_ms", "host_module_ms", "materialize_ms")


def read_trace(path):
    """A killed process may leave its final line incomplete. Keep its valid prefix."""
    records = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            records.append(json.loads(line))
        except ValueError:
            return records, "malformed or interrupted trace line"
    return records, None


def failure_context(records):
    """Diagnostic evidence only, never a successful sample summary."""
    for record in records:
        state = record.get("state", {})
        errors = [state.get("error"), state.get("view_error")]
        errors += [r.get("error") for r in state.get("runtimes", [])]
        if any(error is not None for error in errors):
            return {"first_fault": record, "last_record": records[-1]}
    return {"first_fault": None, "last_record": records[-1] if records else None}


def _number(value, *, integer=False):
    if (type(value) not in ((int,) if integer else (int, float))
            or not math.isfinite(value) or value < 0):
        raise ValueError("invalid diagnostic counter or duration")


def _state(state):
    if not isinstance(state, dict):
        raise ValueError("missing state")
    for name in ("root_renders", "runtime_count", "instance_count", "tasks"):
        _number(state.get(name), integer=True)
    for name in ("error", "view_error", "result"):
        if name not in state:
            raise ValueError("missing result/error evidence")
    if state["error"] is not None or state["view_error"] is not None:
        raise ValueError("faulted diagnostic stage")
    runtimes = state.get("runtimes")
    if not isinstance(runtimes, list) or len(runtimes) > ACTIVE:
        raise ValueError("invalid retained runtime metrics")
    for index, runtime in enumerate(runtimes):
        if runtime.get("owner") != f"stress.plugin-{index:02}" or "error" not in runtime or runtime["error"] is not None:
            raise ValueError("missing or faulted runtime")
        for name in COUNTERS:
            _number(runtime.get(name), integer=True)
        for name in TIMES:
            _number(runtime.get(name))


def validate(records, driver, samples, warmup, profile):
    if driver not in DRIVERS or profile not in ("dev", "release"):
        raise ValueError("unknown diagnostic driver/profile")
    header = {"event": "header", "schema_version": 1, "suite": "diagnostics",
              "platform": "gpui-test-no-gpu", "driver": driver, "samples": samples,
              "warmup": warmup, "debug_assertions": profile == "dev",
              "dataset": dataset_metadata(), "registered_package_count": PACKAGES,
              "pace_ms": 16, "window_px": [1600, 800]}
    if not records or any(type(records[0].get(k)) is not type(v) or records[0].get(k) != v
                          for k, v in header.items()):
        raise ValueError("diagnostic metadata differs from requested run")
    _number(records[0].get("pid"), integer=True)
    if not records[0]["pid"]:
        raise ValueError("missing process identity")
    cursor = 1
    sequence = 0
    previous = None

    def take(event, **expected):
        nonlocal cursor, previous
        if cursor >= len(records):
            raise ValueError(f"incomplete trace: expected {event} {expected}")
        record = records[cursor]
        cursor += 1
        if record.get("event") != event or any(type(record.get(k)) is not type(v) or record.get(k) != v
                                              for k, v in expected.items()):
            raise ValueError(f"out-of-order trace: expected {event} {expected}")
        state = record.get("state")
        _state(state)
        if previous is not None:
            if state["root_renders"] < previous["root_renders"] or len(state["runtimes"]) < len(previous["runtimes"]):
                raise ValueError("diagnostic counters went backwards")
            for old, new in zip(previous["runtimes"], state["runtimes"]):
                if any(new[name] < old[name] for name in (*COUNTERS, *TIMES)):
                    raise ValueError("runtime counters went backwards")
        previous = state
        return record

    ready = take("ready")["state"]
    if ready["runtime_count"] or ready["instance_count"] or ready["runtimes"]:
        raise ValueError("registration must not activate a VM")

    def stage(action, index, phase):
        nonlocal sequence
        expected = dict(sequence=sequence, action=action, index=index, phase=phase)
        begin = take("begin", **expected)
        end = take("end", **expected)
        sequence += 1
        _number(end.get("elapsed_ms"))
        if phase == "dispatch-effects":
            detail = end.get("detail", {})
            _number(detail.get("dispatch_ms"))
            if detail.get("accepted") is not True or detail["dispatch_ms"] > end["elapsed_ms"]:
                raise ValueError("command dispatch was not accepted or timing is inconsistent")
        if action in ("local", "metadata", "cached-draw"):
            for record in (begin, end):
                state = record["state"]
                if state["runtime_count"] != ACTIVE or state["instance_count"] != ACTIVE or len(state["runtimes"]) != ACTIVE:
                    raise ValueError("wrong active workload")
        return end

    total = warmup + samples
    for action in ("host-rows", "host-draw"):
        for index in range(total):
            stage(action, index, "update")
    for index in range(ACTIVE):
        end = stage("open", index, "update")["state"]
        if end["runtime_count"] != index + 1 or end["instance_count"] != index + 1:
            raise ValueError("first activation count differs")
    for index in range(total):
        stage("cached-draw", index, "update")
    count = 0
    for action in ("local", "metadata"):
        for index in range(total):
            stage(action, index, "dispatch-effects")
            stage(action, index, "settle")
            if driver == "explicit-draw":
                stage(action, index, "draw")
            count += 1
            verified = take("verified", action=action, index=index, expected_count=count)["state"]
            if verified["tasks"] != 0 or not isinstance(verified["result"], str):
                raise ValueError("command did not settle")
    complete = take("complete", commands=count, stages=sequence)["state"]
    if complete["runtime_count"] != ACTIVE or complete["instance_count"] != ACTIVE or complete["tasks"]:
        raise ValueError("wrong final workload")
    if cursor != len(records):
        raise ValueError("unexpected records after completion")


def summarize(records, warmup):
    metrics = {}
    for before, after in zip(records, records[1:]):
        if before["event"] != "begin" or after["event"] != "end" or after["index"] < warmup or after["action"] == "open":
            continue
        name = f'{after["action"]}/{after["phase"]}'
        item = metrics.setdefault(name, {key: [] for key in ("elapsed_ms", "root_renders", *COUNTERS, *TIMES)})
        item["elapsed_ms"].append(after["elapsed_ms"])
        item["root_renders"].append(after["state"]["root_renders"] - before["state"]["root_renders"])
        for key in (*COUNTERS, *TIMES):
            item[key].append(sum(b[key] - a[key] for a, b in zip(before["state"]["runtimes"], after["state"]["runtimes"])))
    return {name: {key: (distribution(values) if key.endswith("_ms") else
                         {"count": len(values), "min": min(values), "max": max(values), "total": sum(values)})
                   for key, values in item.items()} for name, item in metrics.items()}


def measure(common, samples=200, warmup=20, repeats=3, profile="dev"):
    parent = OUT / "desktop-diagnostics"
    parent.mkdir(exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-"), dir=parent))
    prefix = folder.relative_to(OUT).as_posix()
    args = ["cargo", "test", *common, "--features", "ui-tests", "--no-run", "--message-format=json"]
    if profile == "release":
        args.append("--release")
    results = [run(f"{prefix}/build", args)]
    measurement = {"platform": "gpui-test-no-gpu", "profile": profile, "production_gate": "not_evaluated",
                   "samples": samples, "warmup": warmup, "repeats": repeats, "raw_traces": [],
                   "artifact_directory": str(folder.relative_to(ROOT)), "runs": [], "summary": []}
    if results[-1]["exit"] != 0:
        return results, measurement
    try:
        built = built_executable((folder / "build.log").read_text(encoding="utf-8"))
        executable = folder / built.name
        shutil.copy2(built, executable)
        measurement["executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
        measurement["rustc"] = subprocess.check_output(["rustc", "-Vv"], text=True).strip()
        package_root = folder / "packages"
        measurement["packages_sha256"] = prepare_packages(package_root, breakdown=True)
        summaries = []
        for repeat in range(repeats):
            order = DRIVERS if repeat % 2 == 0 else DRIVERS[::-1]
            for driver in order:
                name = f"{repeat + 1}-{driver}"
                output = folder / f"{name}.jsonl"
                relative = str(output.relative_to(ROOT))
                measurement["raw_traces"].append(relative)
                env = os.environ.copy()
                env.update(CONDUIT_DIAGNOSTIC_DRIVER=driver, CONDUIT_PERF_SAMPLES=str(samples),
                           CONDUIT_PERF_WARMUP=str(warmup), CONDUIT_PERF_OUTPUT=str(output),
                           CONDUIT_STRESS_PACKAGES=str(package_root),
                           CONDUIT_DATA_DIR=str(REPO / "target" / f"shell-diagnostics-{profile}-data" / folder.name / name))
                results.append(run(f"{prefix}/{name}", [executable, PROBE, "--exact", "--ignored", "--test-threads=1", "--nocapture"],
                                   timeout=max(180, (samples + warmup) * .6 + 90), env=env))
                records, parse_error = read_trace(output) if output.exists() else ([], "trace missing")
                evidence = {"driver": driver, "repeat": repeat + 1, "trace": relative,
                            "records": len(records), "parse_error": parse_error, "exit": results[-1]["exit"]}
                measurement["runs"].append(evidence)
                if results[-1]["exit"] != 0 or parse_error:
                    evidence.update(failure_context(records))
                    if results[-1]["exit"] == 0:
                        results.append({"name": "diagnostic-trace-validation", "exit": 1, "error": parse_error})
                    return results, measurement
                try:
                    validate(records, driver, samples, warmup, profile)
                except (ValueError, KeyError, TypeError):
                    evidence.update(failure_context(records))
                    raise
                summaries.append({"driver": driver, "repeat": repeat + 1, "metrics": summarize(records, warmup)})
        measurement["summary"] = summaries
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        results.append({"name": "desktop-diagnostics-validation", "exit": 1, "error": str(error)})
        print(f"[desktop-diagnostics-validation] {error}", flush=True)
    return results, measurement
