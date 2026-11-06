"""Reproducible prototype checks; writes real command outcomes, never inferred passes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from prepare_upstream import prepare

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
TARGET = REPO / "target" / "shell-prototype"
OUT = ROOT / "artifacts"


def run(name, args, timeout=1800, env=None):
    started = time.monotonic()
    print(f"[{name}] {' '.join(str(arg) for arg in args)}", flush=True)
    log = OUT / f"{name}.log"
    with log.open("w", encoding="utf-8") as stream:
        try:
            process = subprocess.Popen(args, cwd=REPO, env=env, stdout=stream, stderr=subprocess.STDOUT,
                                       creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                if os.name == "nt":
                    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                   stdout=stream, stderr=subprocess.STDOUT, check=False,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
                else:
                    process.kill()
                process.wait()
                code = "timeout"
        except OSError as error:
            stream.write(str(error))
            code = "launch-failed"
    elapsed = round(time.monotonic() - started, 3)
    print(f"[{name}] exit={code}, seconds={elapsed}, log={log}", flush=True)
    if code != 0:
        print(log.read_text(encoding="utf-8", errors="replace")[-8000:], flush=True)
    return {"name": name, "command": [str(arg) for arg in args], "exit": code,
            "elapsed_seconds": elapsed, "log": str(log.relative_to(ROOT))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=["check", "test", "fast-check", "release-check", "watchdog", "resources", "commands", "development-fault", "build"], default="test")
    options = parser.parse_args()
    OUT.mkdir(exist_ok=True)
    upstream = prepare()
    common = ["--locked", "--manifest-path", ROOT / "Cargo.toml", "--target-dir", TARGET, "--jobs", "4"]
    if options.phase == "check":
        commands = [
            ("fmt", ["cargo", "fmt", "--manifest-path", ROOT / "Cargo.toml", "--package", "conduit-shell-prototype", "--check"]),
            ("check-default", ["cargo", "check", *common]),
            ("clippy", ["cargo", "clippy", *common, "--all-targets", "--features", "ui-tests", "--", "-D", "warnings"]),
        ]
    elif options.phase == "test":
        commands = [("tests", ["cargo", "test", *common, "--features", "ui-tests", "--", "--test-threads=1", "--nocapture"])]
    elif options.phase == "fast-check":
        commands = [("check-fast", ["cargo", "check", *common, "--features", "gpui-fast,ui-tests", "--all-targets"])]
    elif options.phase == "release-check":
        commands = [("check-release", ["cargo", "check", *common, "--release"])]
    elif options.phase == "watchdog":
        commands = [("fault-compile", ["cargo", "test", *common, "--features", "ui-tests", "--no-run"]),
                    ("watchdog", ["cargo", "test", *common, "--features", "ui-tests", "render_watchdog_interrupts_an_infinite_loop", "--", "--ignored", "--test-threads=1", "--nocapture"])]
    elif options.phase == "commands":
        commands = [("fault-compile", ["cargo", "test", *common, "--features", "ui-tests", "--no-run"]),
                    ("command-watchdog", ["cargo", "test", *common, "--features", "ui-tests", "ui_tests::commands::command_watchdogs_stop_sync_and_async_loops", "--", "--ignored", "--exact", "--test-threads=1", "--nocapture"])]
    elif options.phase == "resources":
        commands = [("fault-compile", ["cargo", "test", *common, "--features", "ui-tests", "--no-run"])]
        commands += [(name, ["cargo", "test", *common, "--features", "ui-tests", name, "--", "--ignored", "--exact", "--test-threads=1", "--nocapture"])
                    for name in ["ui_tests::controls::resource_faults_are_bounded_terminal_and_recoverable",
                                 "ui_tests::controls::short_calls_share_a_cumulative_budget",
                                 "ui_tests::controls::native_work_is_charged_after_return_and_cannot_revive_host_calls"]]
    elif options.phase == "development-fault":
        commands = [("fault-compile", ["cargo", "test", *common, "--features", "ui-tests", "--no-run"]),
                    ("development-fault", ["cargo", "test", *common, "--features", "ui-tests", "ui_tests::development::adapter_reaps_async_resource_faults_without_stopping_other_owners", "--", "--ignored", "--exact", "--test-threads=1", "--nocapture"])]
    else:
        commands = [("build", ["cargo", "build", *common])]
    results = []
    try:
        for name, args in commands:
            log_name = name.replace("::", "-")
            result = run(log_name, args, timeout=120 if options.phase in ["watchdog", "resources", "commands", "development-fault"] and name != "fault-compile" else 1800)
            results.append(result)
            if result["exit"] != 0:
                break
    finally:
        report = {"phase": options.phase,
                  "upstream": upstream,
                  "lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
                  "results": results}
        (OUT / f"{options.phase}-results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if results and all(result["exit"] == 0 for result in results) else 1


if __name__ == "__main__":
    sys.exit(main())
