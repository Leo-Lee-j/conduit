"""Build and test the real Conduit desktop against the isolated Shell candidate."""
import argparse
import hashlib
import json
import os
import sys
import tomllib
from pathlib import Path
from prepare_upstream import prepare
from verify import OUT, ROOT, REPO, TARGET, run


def source_digest():
    """Identify the inputs even when other work changes the shared checkout."""
    files = [REPO / "Cargo.toml", REPO / "Cargo.lock", REPO / "build.rs",
             ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "desktop/Cargo.toml"]
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
    parser.add_argument("--phase", choices=["check", "test", "fast-check", "release-check", "build"], default="test")
    options = parser.parse_args()
    original = tomllib.loads((REPO / "Cargo.toml").read_text(encoding="utf-8"))
    desktop = tomllib.loads((ROOT / "desktop/Cargo.toml").read_text(encoding="utf-8"))
    assert desktop["package"]["version"] == original["package"]["version"], "sync desktop package version"
    for name, dependency in original["dependencies"].items():
        assert desktop["dependencies"].get(name) == dependency, f"sync desktop dependency: {name}"
    OUT.mkdir(exist_ok=True)
    upstream = prepare()
    os.environ["CONDUIT_DATA_DIR"] = str(REPO / "target" / f"shell-desktop-{options.phase}-data")
    common = ["--locked", "--manifest-path", ROOT / "Cargo.toml", "-p", "conduit-shell-desktop", "--target-dir", TARGET, "--jobs", "4"]
    if options.phase == "check":
        format_files = [REPO / name for name in [
            "build.rs", "src/main.rs", "src/app/plugin_ui.rs",
            "src/app/plugin_ui/plugin_shell.rs", "src/app/plugin_ui/plugin_shell/tests.rs",
            "src/app/plugin_ui/plugin_shell/tests/interaction.rs",
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
        commands = [("release-check", ["cargo", "check", *common, "--release"])]
    else:
        commands = [("build", ["cargo", "build", *common])]
    results = []
    before = source_digest()
    for name, args in commands:
        results.append(run(f"desktop-{name}", args))
        if results[-1]["exit"] != 0:
            break
    after = source_digest()
    report = {"phase": options.phase, "upstream": upstream,
              "lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
              "source_sha256_before": before, "source_sha256_after": after,
              "source_changed_during_checks": before != after,
              "results": results}
    (OUT / f"desktop-{options.phase}-results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if results and all(result["exit"] == 0 for result in results) else 1


if __name__ == "__main__":
    sys.exit(main())
