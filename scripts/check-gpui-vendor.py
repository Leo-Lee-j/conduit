"""Verify the pinned GPUI snapshot without a Cargo cache or network."""

import hashlib
import json
from pathlib import Path
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor" / "gpui-kit"
PATCHED = {
    "crates/base/src/accordion.rs",
    "crates/base/src/text/style.rs",
    "crates/base/src/text/text_view.rs",
    "crates/base/src/text/node.rs",
    "crates/base/src/text/state.rs",
    "crates/base/src/text/inline_flow.rs",
    "crates/component/src/text/compat.rs",
    "crates/component/src/accordion.rs",
    "crates/component/src/setting/group.rs",
    "crates/component/src/setting/item.rs",
    "crates/component/src/setting/settings.rs",
}


def digest(path):
    # Git may check out text using CRLF on Windows.
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def main():
    snapshot = json.loads((VENDOR / "upstream-files.json").read_text(encoding="utf-8"))
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    workspace = tomllib.loads((VENDOR / "Cargo.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    errors = []
    revision = snapshot["revision"]
    if manifest["dependencies"]["gpui-kit"]["rev"] != revision:
        errors.append("Application GPUI revision differs from the vendor snapshot")
    source = "https://github.com/longbridge/gpui-kit.git"
    for package, crate in [("gpui-base", "base"), ("gpui-component", "component")]:
        expected = f"vendor/gpui-kit/crates/{crate}"
        if manifest.get("patch", {}).get(source, {}).get(package, {}).get("path") != expected:
            errors.append(f"Missing repository-relative patch for {package}")
        entries = [p for p in lock["package"] if p["name"] == package]
        if len(entries) != 1 or "source" in entries[0]:
            errors.append(f"Lockfile does not resolve {package} solely to the local patch")
    expected_source = f"git+{source}?rev={revision}#{revision}"
    for package in ["gpui-kit", "gpui-kit-assets", "gpui-component-macros"]:
        entries = [p for p in lock["package"] if p["name"] == package]
        if len(entries) != 1 or entries[0].get("source") != expected_source:
            errors.append(f"Lockfile revision differs for {package}")
    for name, dependency in workspace["workspace"]["dependencies"].items():
        if isinstance(dependency, dict) and dependency.get("git") == source:
            if dependency.get("rev") != revision:
                errors.append(f"Vendor workspace revision differs for {name}")
    actual = {p.relative_to(VENDOR).as_posix() for p in (VENDOR / "crates").rglob("*") if p.is_file()}
    expected = set(snapshot["files"])
    for name in sorted(actual ^ expected):
        errors.append(f"Unexpected/missing vendor file: {name}")
    for name in sorted((actual & expected) - PATCHED):
        if digest(VENDOR / name) != snapshot["files"][name]:
            errors.append(f"Undeclared upstream change: {name}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"GPUI vendor OK: {revision}, {len(actual)} files, {len(PATCHED)} declared patches")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
