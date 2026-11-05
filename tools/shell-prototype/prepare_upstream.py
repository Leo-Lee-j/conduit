"""Materialize the pinned Shell source plus our reviewable API patch, outside Cargo's cache."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parent
REV = "11345ec002d9f182a266b5d5dd3a217090700a87"
URL = "https://github.com/longbridge/gpui-kit.git"
PATCH = ROOT / "patches" / "shell-host-controls.patch"
PATCHES = (PATCH, ROOT / "patches" / "base-fast-compat.patch",
           ROOT / "patches" / "component-fast-compat.patch",
           ROOT / "patches" / "kit-fast-compat.patch",
           ROOT / "patches" / "shell-fast-compat.patch",
           ROOT / "patches" / "shell-targeted-commands.patch",
           ROOT / "patches" / "shell-owned-overlays.patch")
DEST = ROOT / ".upstream"


def git(*args, cwd=None):
    return subprocess.check_output(["git", "-c", "core.autocrlf=false", *map(str, args)], cwd=cwd,
                                   creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)


def literal(value):
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{json.dumps(k)} = {literal(v)}" for k, v in value.items()) + " }"
    if isinstance(value, list):
        return "[" + ", ".join(literal(v) for v in value) + "]"
    return json.dumps(value, ensure_ascii=False)


def standalone_manifest(source, workspace):
    """Expand workspace inheritance; all sibling crates keep their pinned Git identity."""
    def dependency(name, extra):
        base = workspace["dependencies"][name]
        base = {"version": base} if isinstance(base, str) else dict(base)
        extra = dict(extra)
        extra.pop("workspace", None)
        if "features" in extra:
            extra["features"] = list(dict.fromkeys(base.get("features", []) + extra["features"]))
        base.update(extra)
        if "path" in base:
            base.pop("path")
            base.update(git=URL, rev=REV)
        return f"{name} = {literal(base)}"

    source = source.replace("edition.workspace = true", f'edition = {literal(workspace["package"]["edition"])}')
    source = re.sub(r"(?m)^([\w-]+)\.workspace = true$", lambda m: dependency(m[1], {}), source)
    source = re.sub(r"(?m)^([\w-]+) = (\{ workspace = true[\s\S]*?\})$",
                    lambda m: dependency(m[1], tomllib.loads("v = " + m[2])["v"]), source)
    source = source.replace('path = "../kit",', f'git = "{URL}", rev = "{REV}",')
    source = source.replace("[lints]\nworkspace = true", "[lints]\n" + "\n".join(
        f"{kind} = {literal(values)}" for kind, values in workspace.get("lints", {}).items()))
    return source + "\n# This generated crate does not inherit a second workspace.\n[workspace]\n"


def fingerprint(directory):
    digest = hashlib.sha256()
    for path in sorted(directory.rglob("*")):
        if path.is_file() and path.name != ".prepared.json":
            digest.update(path.relative_to(directory).as_posix().encode())
            digest.update(b"\0")
            digest.update(path.read_bytes())
    return digest.hexdigest()


def record(directory=DEST):
    value = {"revision": REV, "patch_sha256": patch_fingerprint(),
             "source_sha256": fingerprint(directory)}
    (directory / ".prepared.json").write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    return value


def patch_fingerprint():
    digest = hashlib.sha256()
    for patch in PATCHES:
        digest.update(patch.name.encode())
        digest.update(b"\0")
        digest.update(patch.read_bytes())
    return digest.hexdigest()


def prepare(source=None, destination=DEST):
    if destination.exists():
        marker = destination / ".prepared.json"
        if marker.is_file():
            value = json.loads(marker.read_text(encoding="utf-8"))
            if (value["revision"] == REV
                    and value["patch_sha256"] == patch_fingerprint()
                    and value["source_sha256"] == fingerprint(destination)):
                return value
        raise RuntimeError(f"{destination} differs from the recorded source/patch; preserve local edits and use a fresh --destination")
    if source is None:
        cargo = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
        candidates = sorted((cargo / "git" / "checkouts").glob(f"gpui-kit-*/{REV[:7]}"))
        source = candidates[0] if candidates else None
    if source is None:
        source = ROOT / "artifacts" / "upstream-source"
        source.mkdir(parents=True, exist_ok=True)
        if not (source / ".git").exists():
            git("init", source)
        git("fetch", "--depth=1", URL, REV, cwd=source)
    archive = git("archive", REV, "Cargo.toml", "crates/shell", "crates/base", "crates/component", "crates/kit", cwd=source)
    destination.mkdir(parents=True)
    with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
        bundle.extractall(destination, filter="data")
    manifest = destination / "Cargo.toml"
    workspace = tomllib.loads(manifest.read_text(encoding="utf-8"))["workspace"]
    for name in ("shell", "base", "component", "kit"):
        crate_manifest = destination / f"crates/{name}/Cargo.toml"
        crate_manifest.write_text(standalone_manifest(crate_manifest.read_text(encoding="utf-8"), workspace), encoding="utf-8", newline="\n")
    for patch in PATCHES:
        if patch.stat().st_size:
            git("apply", "--check", patch, cwd=destination)
            git("apply", patch, cwd=destination)
    return record(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, help="optional existing Git checkout containing the pinned commit")
    parser.add_argument("--destination", type=Path, default=DEST, help="fresh output directory for reproducibility checks")
    args = parser.parse_args()
    print(json.dumps(prepare(args.source, args.destination), indent=2))


if __name__ == "__main__":
    main()
