"""Isolated, opt-in desktop CPU/layout measurements; never a production gate."""
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile

from verify import OUT, REPO, ROOT, run

SCENARIOS = ("baseline", "registered", "navigation", "dual", "hidden", "stopped")
PROBE = "app::plugin_ui::shell::tests::performance::desktop_performance_probe"


def distribution(values):
    """Nearest-rank quantiles, preserving each process run as a separate sample."""
    if not values or any(type(x) not in (int, float) or not math.isfinite(x) or x < 0 for x in values):
        raise ValueError("expected nonempty, finite, nonnegative samples")
    ordered = sorted(values)
    return {"count": len(ordered), "p50_ms": ordered[math.ceil(len(ordered) * .50) - 1],
            "p95_ms": ordered[math.ceil(len(ordered) * .95) - 1], "max_ms": ordered[-1]}


def validate(report, scenario, samples, warmup, profile):
    expected = {"schema_version": 1, "scenario": scenario, "samples": samples,
                "warmup": warmup, "platform": "gpui-test-no-gpu",
                "debug_assertions": profile == "dev", "pace_ms": 16}
    if any(report.get(key) != value for key, value in expected.items()):
        raise ValueError("probe metadata differs from requested run")
    counts = {"navigation": (1, 1), "dual": (1, 2), "hidden": (1, 2)}.get(scenario, (0, 0))
    if (report.get("runtime_count"), report.get("instance_count")) != counts:
        raise ValueError("unexpected runtime/instance count")
    metrics = {"host_edit_draw_ms", "host_notify_draw_ms"}
    if scenario == "dual":
        metrics.add("plugin_command_draw_ms")
    if set(report["timings"]) != metrics:
        raise ValueError("missing or unexpected timing metric")
    for values in report["timings"].values():
        if len(values) != samples:
            raise ValueError("incomplete timing samples")
        distribution(values)
    for phase in ("before_fixture", "registered", "active", "ready", "after_samples"):
        for key in ("working_set_bytes", "private_bytes", "peak_working_set_bytes"):
            value = report["memory"][phase][key]
            if type(value) is not int or value <= 0:
                raise ValueError("missing process memory sample")


def summarize(reports, scenarios=SCENARIOS):
    summary = {}
    for scenario in scenarios:
        group = [report for report in reports if report["scenario"] == scenario]
        if not group:
            raise ValueError(f"missing scenario: {scenario}")
        metrics = {}
        for name in group[0]["timings"]:
            runs = [distribution(report["timings"][name]) for report in group]
            p95 = [item["p95_ms"] for item in runs]
            metrics[name] = {"runs": runs, "median_p95_ms": statistics.median(p95),
                             "min_p95_ms": min(p95), "max_p95_ms": max(p95)}
        memory = {}
        for name in ("working_set_bytes", "private_bytes"):
            memory[f"median_ready_{name}"] = statistics.median(
                report["memory"]["ready"][name] for report in group)
            memory[f"median_after_samples_{name}"] = statistics.median(
                report["memory"]["after_samples"][name] for report in group)
            memory[f"median_ready_minus_registered_{name}"] = statistics.median(
                report["memory"]["ready"][name] - report["memory"]["registered"][name]
                for report in group)
        summary[scenario] = {"process_runs": len(group), "metrics": metrics, "memory": memory}
    baseline = summary["baseline"]["metrics"]
    for scenario, entry in summary.items():
        if scenario == "baseline":
            continue
        comparison = {}
        for name, reference in baseline.items():
            base = reference["median_p95_ms"]
            delta = entry["metrics"][name]["median_p95_ms"] - base
            budget = max(2., base * .10)
            comparison[name] = {"delta_ms": delta, "reference_budget_ms": budget,
                                "within_reference_budget": delta <= budget}
        entry["reference_only_comparison"] = comparison
    return summary


def built_executable(log):
    candidates = set()
    for line in log.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (message.get("reason") == "compiler-artifact"
                and message.get("target", {}).get("name") == "conduit-shell-desktop"
                and message.get("profile", {}).get("test") is True
                and message.get("executable")):
            candidates.add(message["executable"])
    if len(candidates) != 1:
        raise ValueError("build did not identify exactly one desktop test executable")
    path = Path(candidates.pop())
    if not path.is_file():
        raise ValueError("built test executable is absent")
    return path


def measure(common, samples=200, warmup=20, repeats=3, profile="dev", stress_cycles=None, *, breakdown=False):
    if breakdown:
        if stress_cycles is not None:
            raise ValueError("breakdown and stress cannot share a run")
        from desktop_breakdown import measure as measure_breakdown
        return measure_breakdown(common, samples, warmup, repeats, profile)
    suite = "stress" if stress_cycles is not None else "performance"
    scenarios, probe = SCENARIOS, PROBE
    if stress_cycles is not None:
        from desktop_stress import STRESS_SCENARIOS, STRESS_PROBE, prepare_packages, validate_stress, summarize_stress
        scenarios, probe = STRESS_SCENARIOS, STRESS_PROBE
    parent = OUT / f"desktop-{suite}"
    parent.mkdir(exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-"), dir=parent))
    prefix = folder.relative_to(OUT).as_posix()
    args = ["cargo", "test", *common, "--features", "ui-tests", "--no-run", "--message-format=json"]
    if profile == "release":
        args.append("--release")
    results = [run(f"{prefix}/build", args)]
    measurement = {"platform": "gpui-test-no-gpu", "profile": profile,
                   "production_gate": "not_evaluated", "samples": samples, "warmup": warmup,
                   "repeats": repeats, "raw_reports": [], "os": platform.platform(),
                   "logical_cpus": os.cpu_count(), "summary": {}}
    if results[-1]["exit"] != 0:
        return results, measurement
    try:
        built = built_executable((folder / "build.log").read_text(encoding="utf-8"))
        # Another task may rebuild the shared Cargo target between child runs.
        # Every repetition must execute the same bytes, so retain a private copy.
        executable = folder / built.name
        shutil.copy2(built, executable)
        measurement["executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
        measurement["rustc"] = subprocess.check_output(["rustc", "-Vv"], text=True).strip()
        reports = []
        if stress_cycles is not None:
            package_root = folder / "packages"
            measurement["packages_sha256"] = prepare_packages(package_root)
            measurement["cycles"] = stress_cycles
        # Rotate order between repetitions to reduce systematic warm-machine bias.
        for repeat in range(repeats):
            order = scenarios[repeat % len(scenarios):] + scenarios[:repeat % len(scenarios)]
            for scenario in order:
                name = f"{repeat + 1}-{scenario}"
                output = folder / f"{name}.json"
                env = os.environ.copy()
                env.update(CONDUIT_PERF_SCENARIO=scenario, CONDUIT_PERF_SAMPLES=str(samples),
                           CONDUIT_PERF_WARMUP=str(warmup), CONDUIT_PERF_OUTPUT=str(output),
                           CONDUIT_DATA_DIR=str(REPO / "target" / f"shell-{suite}-data" / folder.name / name))
                timeout = max(120, (samples + warmup) * .12 + 60)
                if stress_cycles is not None:
                    env.update(CONDUIT_STRESS_PACKAGES=str(package_root), CONDUIT_STRESS_CYCLES=str(stress_cycles))
                    timeout = max(300, (samples + warmup) * .5 + stress_cycles * 3 + 60)
                results.append(run(f"{prefix}/{name}", [executable, probe, "--exact", "--ignored",
                                                       "--test-threads=1", "--nocapture"],
                                   timeout=timeout, env=env))
                if results[-1]["exit"] != 0:
                    return results, measurement
                # A successful empty filter or stale output must never count as a sample.
                report = json.loads(output.read_text(encoding="utf-8"))
                if stress_cycles is None:
                    validate(report, scenario, samples, warmup, profile)
                else:
                    validate_stress(report, scenario, samples, warmup, profile, stress_cycles)
                reports.append(report)
                measurement["raw_reports"].append(str(output.relative_to(ROOT)))
        measurement["summary"] = summarize(reports) if stress_cycles is None else summarize_stress(reports)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        results.append({"name": f"desktop-{suite}-validation", "exit": 1, "error": str(error)})
        print(f"[desktop-{suite}-validation] {error}", flush=True)
    return results, measurement
