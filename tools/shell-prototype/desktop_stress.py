"""Fixed, versioned G5 stress workloads and strict report validation."""
import hashlib
import json
import statistics

from desktop_performance import distribution, summarize
from verify import ROOT

STRESS_SCENARIOS = ("baseline", "registered", "active", "mixed", "lifecycle", "idle")
PACKAGES = 20
ACTIVE = 3
STRESS_PROBE = "app::plugin_ui::shell::tests::stress::desktop_stress_probe"
PARAGRAPH = ("固定工作负载：中文与 emoji 😀。 Review the plugin lifecycle and preserve host input.\n\n"
             "- deterministic item\n- bounded data\n\n```rust\nlet count = 42;\n```\n\n")
MEMORY_KEYS = ("working_set_bytes", "private_bytes", "peak_working_set_bytes")


def dataset_metadata():
    digest = hashlib.sha256()
    total = longest = 0
    for _ in range(48):
        for message in range(120):
            content = (PARAGRAPH * (64 if message % 10 == 9 else 4)).encode("utf-8")
            total += len(content)
            longest = max(longest, len(content))
            digest.update(content)
    return {"id": "long-chat-v1", "workspaces": 2, "sessions_per_workspace": 24,
            "messages_per_session": 120, "total_messages": 5760, "text_bytes": total,
            "longest_message_bytes": longest, "text_sha256": digest.hexdigest()}


def prepare_packages(destination, *, breakdown=False):
    source = ROOT / "fixtures/development-overview"
    digest = hashlib.sha256()
    for index in range(20):
        owner = f"stress.plugin-{index:02}"
        folder = destination / owner
        folder.mkdir(parents=True, exist_ok=False)
        for name in ("conduit-plugin.json", "gpui-shell.json", "navigation.js", "summary.js"):
            text = (source / name).read_text(encoding="utf-8").replace("example.workspace-overview", owner)
            if breakdown and name == "conduit-plugin.json":
                manifest = json.loads(text)
                manifest["contributes"]["commands"].extend([
                    {"id": f"{owner}.local", "title": "Local increment", "scope": "workspace",
                     "binding": {"view": f"{owner}.summary", "command": "local"}},
                    {"id": f"{owner}.metadata", "title": "Read metadata", "scope": "workspace",
                     "requiredPermissions": ["workspace.metadata.read", "session.summary.read"],
                     "binding": {"view": f"{owner}.summary", "command": "metadata"}},
                ])
                text = json.dumps(manifest, separators=(",", ":")) + "\n"
            if breakdown and name == "summary.js":
                imports = 'import { getContext, getWorkspaceMetadata } from "conduit";'
                if text.count(imports) != 1:
                    raise ValueError("HostModule fixture import changed")
                text = text.replace(imports, 'import { getContext, getWorkspaceMetadata, getSessionSummary } from "conduit";')
                marker = '    if (name === "increment") {'
                if text.count(marker) != 1:
                    raise ValueError("increment fixture injection point changed")
                text = text.replace(
                    marker,
                    '    if (name === "local") {\n'
                    '      this.count += 1;\n'
                    '      cx.notify();\n'
                    '      return { count: this.count };\n'
                    '    }\n'
                    '    if (name === "metadata") {\n'
                    '      const workspace = getWorkspaceMetadata();\n'
                    '      const session = getSessionSummary();\n'
                    '      if (!workspace?.id || session?.workspaceId !== workspace.id || session.count !== 24) {\n'
                    '        throw new Error("unexpected HostModule workload");\n'
                    '      }\n'
                    '      this.count += 1;\n'
                    '      cx.notify();\n'
                    '      return { count: this.count };\n'
                    '    }\n'
                    + marker,
                    1,
                )
            if index == 19 and name == "summary.js":
                marker = '    if (name === "increment") {\n      this.count += 1;'
                if text.count(marker) != 1:
                    raise ValueError("fault fixture injection point changed")
                text = text.replace(marker, '    if (name === "increment") {\n      while (true) {}', 1)
            data = text.encode("utf-8")
            (folder / name).write_bytes(data)
            digest.update(f"{owner}/{name}\0".encode("utf-8"))
            digest.update(data)
    return digest.hexdigest()


def memory_sample(sample):
    if not isinstance(sample, dict) or set(sample) != set(MEMORY_KEYS):
        raise ValueError("missing process memory sample")
    if any(type(sample[key]) is not int or sample[key] <= 0 for key in MEMORY_KEYS):
        raise ValueError("invalid process memory sample")


def validate_stress(report, scenario, samples, warmup, profile, cycles):
    if scenario not in STRESS_SCENARIOS:
        raise ValueError("unknown stress scenario")
    active = 3 if scenario in ("active", "mixed") else 0
    expected = {"schema_version": 1, "suite": "stress", "scenario": scenario,
                "samples": samples, "warmup": warmup, "cycles": cycles,
                "platform": "gpui-test-no-gpu", "debug_assertions": profile == "dev",
                "pace_ms": 16, "window_px": [1600, 800], "dataset": dataset_metadata(),
                "registered_package_count": 0 if scenario == "baseline" else 20,
                "runtime_count": active, "instance_count": active,
                "registered_runtime_count": 0, "registered_instance_count": 0,
                "visible_plugin_view_count": int(active > 0)}
    if any(report.get(key) != value for key, value in expected.items()):
        raise ValueError("stress workload metadata or counts differ from requested run")
    metrics = {"host_edit_draw_ms", "host_notify_draw_ms"}
    if active:
        metrics.add("three_plugin_commands_draw_ms")
    timings = report.get("timings", {})
    if set(timings) != metrics:
        raise ValueError("missing or unexpected stress timing metric")
    for values in timings.values():
        if len(values) != samples:
            raise ValueError("incomplete stress timing samples")
        distribution(values)
    memory = report.get("memory", {})
    if set(memory) != {"before_fixture", "registered", "ready", "after_samples"}:
        raise ValueError("missing stress memory phase")
    for value in memory.values():
        memory_sample(value)
    fault = report.get("fault")
    if scenario == "mixed":
        required = {"owner": "stress.plugin-19", "kind": "sync-command-deadline",
                    "runtime_released": True, "healthy_runtimes_preserved": 3,
                    "healthy_commands_after_fault": 3, "remaining_runtime_count": 3,
                    "remaining_instance_count": 3}
        if not isinstance(fault, dict) or any(fault.get(k) != v for k, v in required.items()):
            raise ValueError("fault isolation was not verified")
        if "execution deadline" not in fault.get("error", ""):
            raise ValueError("expected watchdog fault was not observed")
        distribution([fault.get("elapsed_ms")])
    elif "fault" not in report or fault is not None:
        raise ValueError("unexpected fault evidence")
    curve = report.get("lifecycle")
    expected_cycles = cycles if scenario in ("lifecycle", "idle") else 0
    if not isinstance(curve, list) or len(curve) != expected_cycles:
        raise ValueError("incomplete lifecycle memory curve")
    for index, point in enumerate(curve, 1):
        cycle_active = 3 if scenario == "lifecycle" else 0
        state = {"cycle": index, "active_runtime_count": cycle_active,
                 "active_instance_count": cycle_active, "stopped_runtime_count": 0,
                 "stopped_instance_count": 0, "released_runtime_count": cycle_active}
        if any(point.get(k) != v for k, v in state.items()):
            raise ValueError("lifecycle activation/release evidence differs")
        for phase in ("ready", "after_stop", "settled"):
            memory_sample(point.get(phase))
        distribution([point.get("activate_draw_ms"), point.get("stop_draw_ms")])


def growth(values):
    """Signed end delta and ordinary least-squares trend; neither diagnoses a leak."""
    if len(values) < 3:
        raise ValueError("at least three memory samples required")
    center = (len(values) - 1) / 2
    mean = statistics.mean(values)
    slope = sum((i - center) * (v - mean) for i, v in enumerate(values)) / sum(
        (i - center) ** 2 for i in range(len(values)))
    return {"first_bytes": values[0], "last_bytes": values[-1],
            "last_minus_first_bytes": values[-1] - values[0],
            "max_above_first_bytes": max(values) - values[0], "slope_bytes_per_cycle": slope}


def summarize_stress(reports):
    result = summarize(reports, STRESS_SCENARIOS)
    for scenario in ("lifecycle", "idle"):
        group = [report for report in reports if report["scenario"] == scenario]
        result[scenario]["settled_growth"] = {}
        for key in MEMORY_KEYS[:2]:
            runs = [growth([point["settled"][key] for point in report["lifecycle"]]) for report in group]
            result[scenario]["settled_growth"][key] = {
                "runs": runs, "median_slope_bytes_per_cycle": statistics.median(
                    run["slope_bytes_per_cycle"] for run in runs),
                "median_last_minus_first_bytes": statistics.median(run["last_minus_first_bytes"] for run in runs)}
        result[scenario]["leak_verdict"] = "not_evaluated"
    return result
