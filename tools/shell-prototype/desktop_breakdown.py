"""Measure host and plugin components on the fixed long-chat workload."""
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

from desktop_performance import built_executable
from desktop_stress import ACTIVE, PACKAGES, dataset_metadata, prepare_packages
from verify import OUT, REPO, ROOT, run

SCENARIOS = ("host", "registered", "plugin")
PROBE = "app::plugin_ui::shell::tests::breakdown::desktop_breakdown_probe"
COMMON_TIMINGS = ("host_chat_rows_sync_ms", "host_draw_ms")
PLUGIN_TIMINGS = (
    "plugin_cached_view_draw_ms",
    "plugin_local_command_dispatch_ms",
    "plugin_local_command_effects_ms",
    "plugin_local_command_settle_ms",
    "plugin_local_command_draw_ms",
    "plugin_host_module_dispatch_ms",
    "plugin_host_module_effects_ms",
    "plugin_host_module_settle_ms",
    "plugin_host_module_draw_ms",
)
MEMORY_KEYS = ("working_set_bytes", "private_bytes", "peak_working_set_bytes")


def distribution(values):
    """Return nearest-rank quantiles and the observed sample range."""
    if not values or any(type(x) not in (int, float) or not math.isfinite(x) or x < 0 for x in values):
        raise ValueError("expected nonempty, finite, nonnegative samples")
    ordered = sorted(values)
    return {
        "count": len(ordered),
        "min_ms": ordered[0],
        "p50_ms": ordered[math.ceil(len(ordered) * .50) - 1],
        "p95_ms": ordered[math.ceil(len(ordered) * .95) - 1],
        "max_ms": ordered[-1],
    }


def _memory_sample(value):
    if not isinstance(value, dict) or set(value) != set(MEMORY_KEYS):
        raise ValueError("missing breakdown memory sample")
    if any(type(value[key]) is not int or value[key] <= 0 for key in MEMORY_KEYS):
        raise ValueError("invalid breakdown memory sample")


def validate(report, scenario, samples, warmup, profile):
    if scenario not in SCENARIOS or profile not in ("dev", "release"):
        raise ValueError("unknown breakdown scenario or profile")
    expected = {
        "schema_version": 2,
        "suite": "breakdown",
        "scenario": scenario,
        "samples": samples,
        "warmup": warmup,
        "platform": "gpui-test-no-gpu",
        "debug_assertions": profile == "dev",
        "pace_ms": 16,
        "window_px": [1600, 800],
        "dataset": dataset_metadata(),
        "registered_package_count": 0 if scenario == "host" else PACKAGES,
        "registered_runtime_count": 0,
        "registered_instance_count": 0,
        "runtime_count": ACTIVE if scenario == "plugin" else 0,
        "instance_count": ACTIVE if scenario == "plugin" else 0,
        "first_view_count": ACTIVE if scenario == "plugin" else 0,
    }
    if any(type(report.get(key)) is not type(value) or report.get(key) != value
           for key, value in expected.items()):
        raise ValueError("breakdown metadata differs from requested run")
    if type(report.get("pid")) is not int or report["pid"] <= 0:
        raise ValueError("missing process identity")
    timings = report.get("timings")
    if not isinstance(timings, dict) or set(timings) != set(COMMON_TIMINGS):
        raise ValueError("missing or unexpected host timing metric")
    for name in COMMON_TIMINGS:
        values = timings[name]
        if not isinstance(values, list) or len(values) != samples:
            raise ValueError(f"incomplete timing samples: {name}")
        distribution(values)
    memory = report.get("memory")
    if not isinstance(memory, dict) or set(memory) != {"before_fixture", "registered", "after"}:
        raise ValueError("missing or unexpected breakdown memory phase")
    for value in memory.values():
        _memory_sample(value)
    for name in ("plugin_first_view_materialize_ms", *PLUGIN_TIMINGS):
        values = report.get(name)
        required = scenario == "plugin"
        if not isinstance(values, list):
            raise ValueError(f"missing timing array: {name}")
        if name == "plugin_first_view_materialize_ms":
            if required and len(values) != ACTIVE:
                raise ValueError("unexpected first view materialization count")
        elif required and len(values) != samples:
            raise ValueError(f"incomplete timing samples: {name}")
        if not required and values:
            raise ValueError(f"unexpected plugin timing samples: {name}")
        if values:
            distribution(values)


def summarize(reports, profile):
    result = {"profile": profile, "scenarios": {}}
    for scenario in SCENARIOS:
        group = [report for report in reports if report["scenario"] == scenario]
        if not group:
            raise ValueError(f"missing scenario: {scenario}")
        names = list(COMMON_TIMINGS)
        if scenario == "plugin":
            names.extend(("plugin_first_view_materialize_ms", *PLUGIN_TIMINGS))
        metrics = {}
        for name in names:
            runs = [distribution(report["timings"][name] if name in COMMON_TIMINGS
                                  else report[name]) for report in group]
            p95 = [item["p95_ms"] for item in runs]
            metrics[name] = {
                "runs": runs,
                "median_p95_ms": statistics.median(p95),
                "min_p95_ms": min(p95),
                "max_p95_ms": max(p95),
                "min_observed_ms": min(item["min_ms"] for item in runs),
                "max_observed_ms": max(item["max_ms"] for item in runs),
            }
        result["scenarios"][scenario] = {"process_runs": len(group), "metrics": metrics}
    return result


def compare_profiles(dev, release):
    """Compare already validated summaries without treating it as a gate."""
    if dev.get("profile") != "dev" or release.get("profile") != "release":
        raise ValueError("expected dev and release summaries")
    comparison = {}
    for scenario in SCENARIOS:
        dev_metrics = dev["scenarios"][scenario]["metrics"]
        release_metrics = release["scenarios"][scenario]["metrics"]
        if set(dev_metrics) != set(release_metrics):
            raise ValueError("profile timing metrics differ")
        comparison[scenario] = {}
        for name in dev_metrics:
            comparison[scenario][name] = {
                "dev_median_p95_ms": dev_metrics[name]["median_p95_ms"],
                "release_median_p95_ms": release_metrics[name]["median_p95_ms"],
                "release_minus_dev_ms": (release_metrics[name]["median_p95_ms"]
                                          - dev_metrics[name]["median_p95_ms"]),
            }
    return comparison


def measure(common, samples=200, warmup=20, repeats=3, profile="dev"):
    parent = OUT / "desktop-breakdown"
    parent.mkdir(exist_ok=True)
    folder = Path(tempfile.mkdtemp(
        prefix=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-"), dir=parent
    ))
    prefix = folder.relative_to(OUT).as_posix()
    args = ["cargo", "test", *common, "--features", "ui-tests", "--no-run", "--message-format=json"]
    if profile == "release":
        args.append("--release")
    results = [run(f"{prefix}/build", args)]
    measurement = {
        "platform": "gpui-test-no-gpu",
        "profile": profile,
        "production_gate": "not_evaluated",
        "samples": samples,
        "warmup": warmup,
        "repeats": repeats,
        "raw_reports": [],
        "os": platform.platform(),
        "logical_cpus": os.cpu_count(),
        "summary": {},
    }
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
        reports = []
        for repeat in range(repeats):
            order = SCENARIOS[repeat % len(SCENARIOS):] + SCENARIOS[:repeat % len(SCENARIOS)]
            for scenario in order:
                name = f"{repeat + 1}-{scenario}"
                output = folder / f"{name}.json"
                env = os.environ.copy()
                env.update(
                    CONDUIT_PERF_SCENARIO=scenario,
                    CONDUIT_PERF_SAMPLES=str(samples),
                    CONDUIT_PERF_WARMUP=str(warmup),
                    CONDUIT_PERF_OUTPUT=str(output),
                    CONDUIT_STRESS_PACKAGES=str(package_root),
                    CONDUIT_DATA_DIR=str(REPO / "target" / f"shell-breakdown-{profile}-data" / folder.name / name),
                )
                timeout = max(180, (samples + warmup) * .3 + 90)
                results.append(run(
                    f"{prefix}/{name}",
                    [executable, PROBE, "--exact", "--ignored", "--test-threads=1", "--nocapture"],
                    timeout=timeout,
                    env=env,
                ))
                if results[-1]["exit"] != 0:
                    return results, measurement
                report = json.loads(output.read_text(encoding="utf-8"))
                validate(report, scenario, samples, warmup, profile)
                reports.append(report)
                measurement["raw_reports"].append(str(output.relative_to(ROOT)))
        measurement["summary"] = summarize(reports, profile)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        results.append({"name": "desktop-breakdown-validation", "exit": 1, "error": str(error)})
        print(f"[desktop-breakdown-validation] {error}", flush=True)
    return results, measurement
