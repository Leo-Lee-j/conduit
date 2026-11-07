"""Build and test the real Conduit desktop against the isolated Shell candidate."""
import argparse
import hashlib
import json
import os
import re
import sys
import tomllib
from pathlib import Path
from prepare_upstream import prepare
from verify import OUT, ROOT, REPO, TARGET, run


def source_digest():
    """Identify the inputs even when other work changes the shared checkout."""
    files = [REPO / "Cargo.toml", REPO / "Cargo.lock", REPO / "build.rs",
             ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "desktop/Cargo.toml"]
    files.extend(ROOT.glob("*.py"))
    for directory in [REPO / "src", ROOT / "src", ROOT / "desktop", ROOT / "fixtures"]:
        files.extend(path for path in directory.rglob("*")
                     if path.is_file() and path.suffix in {".rs", ".js", ".json"})
    digest = hashlib.sha256()
    for path in sorted(set(files)):
        digest.update(path.relative_to(REPO).as_posix().encode("utf-8") + b"\0")
        digest.update(hashlib.sha256(path.read_bytes()).digest())
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=["check", "test", "fast-check", "release-check", "build", "performance", "stress", "breakdown", "diagnostics", "chat-profile"], default="test")
    parser.add_argument("--samples", type=int, default=200, help="performance samples per metric (20..2000)")
    parser.add_argument("--warmup", type=int, default=20, help="performance warmup actions per metric (1..200)")
    parser.add_argument("--repeats", type=int, default=3, help="fresh processes per performance scenario (1..20)")
    parser.add_argument("--cycles", type=int, default=20, help="stress lifecycle/idle cycles per process (3..200)")
    parser.add_argument("--profile", choices=["dev", "release"], default="dev", help="Cargo build profile for checks, tests, builds and measurements")
    parser.add_argument("--target-dir", type=Path, default=TARGET, help="Cargo artifact directory")
    options = parser.parse_args()
    if not (20 <= options.samples <= 2000 and 1 <= options.warmup <= 200 and 1 <= options.repeats <= 20):
        parser.error("performance samples/warmup/repeats outside documented bounds")
    if not 3 <= options.cycles <= 200:
        parser.error("stress cycles outside documented bounds")
    original = tomllib.loads((REPO / "Cargo.toml").read_text(encoding="utf-8"))
    desktop = tomllib.loads((ROOT / "desktop/Cargo.toml").read_text(encoding="utf-8"))
    assert desktop["package"]["version"] == original["package"]["version"], "sync desktop package version"
    for name, dependency in original["dependencies"].items():
        assert desktop["dependencies"].get(name) == dependency, f"sync desktop dependency: {name}"
    OUT.mkdir(exist_ok=True)
    upstream = prepare()
    profile = "release" if options.phase == "release-check" else options.profile
    os.environ["CONDUIT_DATA_DIR"] = str(REPO / "target" / f"shell-desktop-{options.phase}-{profile}-data")
    common = ["--locked", "--manifest-path", ROOT / "Cargo.toml", "-p", "conduit-shell-desktop", "--target-dir", options.target_dir.resolve(), "--jobs", "4"]
    if profile == "release" and options.phase not in ("performance", "stress", "breakdown", "diagnostics", "chat-profile"):
        common.append("--release")
    if options.phase == "check":
        format_files = [REPO / name for name in [
            "build.rs", "src/main.rs", "src/app/plugin_ui.rs",
            "src/app/plugin_ui/plugin_shell.rs", "src/app/plugin_ui/plugin_shell/tests.rs",
            "src/app/plugin_ui/plugin_shell/tests/interaction.rs",
            "src/app/plugin_ui/plugin_shell/tests/performance.rs",
            "src/app/plugin_ui/plugin_shell/tests/stress.rs",
            "src/app/plugin_ui/plugin_shell/tests/breakdown.rs",
            "src/app/plugin_ui/plugin_shell/tests/diagnostics.rs",
            "src/app/plugin_ui/plugin_shell/tests/chat_profile.rs",
            "src/app/plugin_ui/plugin_shell/tests/host_profile.rs",
            "src/app/sidebar.rs",
            "src/app/plugin_ui/plugin_shell/tests/cache.rs",
            "src/app/plugin_ui/plugin_shell/tests/native_memory.rs", "src/allocation_probe.rs",
            "src/app/chat.rs", "src/app/chat/profiling.rs",
            "src/app/plugin_ui/tests.rs", "src/app/plugin_ui/right/tests.rs",
            "src/app/plugin_ui/example.rs", "src/app/plugin_ui/example/tests.rs",
            "src/app/plugin_ui/status.rs", "src/app/plugin_ui/status/integration_tests.rs",
            "src/app/dock.rs", "src/app/files.rs", "src/app/shell.rs",
            "src/app/shell/cache.rs", "src/app/shell/tests.rs", "src/app/chat/rows.rs",
            "src/app/theme.rs", "src/performance.rs",
            "src/app/notifications.rs", "src/app/tool_details.rs", "src/app/toolbar.rs",
            "src/app/settings/workspace.rs", "src/app/settings.rs",
            "src/app/settings/navigation.rs", "src/app.rs",
            "tools/shell-prototype/desktop/build.rs", "tools/shell-prototype/desktop/args.rs",
            "tools/shell-prototype/src/lib.rs", "tools/shell-prototype/src/adapter.rs",
        ]]
        commands = [("fmt", ["rustfmt", "--edition", "2024", "--config", "skip_children=true", "--check", *format_files]),
                    ("check", ["cargo", "check", *common, "--all-targets", "--features", "ui-tests"]),
                    ("clippy", ["cargo", "clippy", *common, "--all-targets", "--features", "ui-tests", "--", "-D", "warnings"])]
    elif options.phase == "test":
        commands = [("ui-tests", ["cargo", "test", *common, "--features", "ui-tests", "app::plugin_ui", "--", "--test-threads=1"]),
                    ("update-tests", ["cargo", "test", *common, "--features", "ui-tests", "app::update::tests", "--", "--test-threads=1"])]
    elif options.phase == "fast-check":
        commands = [("fast-check", ["cargo", "check", *common, "--features", "ui-tests,gpui-fast", "--all-targets"])]
    elif options.phase == "release-check":
        commands = [("release-check", ["cargo", "check", *common])]
    else:
        commands = [("build", ["cargo", "build", *common])]
    results = []
    measurement = None
    before = source_digest()
    if options.phase == "chat-profile":
        from desktop_chat_profile import measure
        results, measurement = measure(common, options.samples, options.warmup, options.repeats, profile)
    elif options.phase == "diagnostics":
        from desktop_diagnostics import measure
        results, measurement = measure(common, options.samples, options.warmup, options.repeats, profile)
    elif options.phase in ("performance", "stress", "breakdown"):
        from desktop_performance import measure
        results, measurement = measure(common, options.samples, options.warmup, options.repeats, options.profile,
                                       stress_cycles=options.cycles if options.phase == "stress" else None,
                                       breakdown=options.phase == "breakdown")
    else:
        for name, args in commands:
            result = run(f"desktop-{profile}-{name}", args)
            results.append(result)
            if options.phase == "test" and result["exit"] == 0:
                log = (ROOT / result["log"]).read_text(encoding="utf-8")
                counts = re.search(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", log)
                if counts is None or int(counts[1]) == 0 or int(counts[2]) != 0:
                    result.update(exit=1, error="expected a nonempty successful test group")
                else:
                    result["tests"] = dict(zip(("passed", "failed", "ignored"), map(int, counts.groups())))
            if results[-1]["exit"] != 0:
                break
    after = source_digest()
    report = {"phase": options.phase, "profile": profile, "upstream": upstream,
              "lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
              "source_sha256_before": before, "source_sha256_after": after,
              "source_changed_during_checks": before != after,
              "results": results}
    if before != after:
        print("[desktop-source-stability] source changed; results are not accepted", flush=True)
        results.append({"name": "desktop-source-stability", "exit": 1,
                        "error": "source changed during checks; rerun on a stable checkout"})
    if measurement is not None:
        report["measurement"] = measurement
        measurement["valid_for_source_snapshot"] = all(result["exit"] == 0 for result in results)
    (OUT / f"desktop-{options.phase}-results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    (OUT / f"desktop-{options.phase}-{profile}-results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if options.phase in ("diagnostics", "chat-profile"):
        (ROOT / measurement["artifact_directory"] / "run-results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if results and all(result["exit"] == 0 for result in results) else 1


if __name__ == "__main__":
    sys.exit(main())
