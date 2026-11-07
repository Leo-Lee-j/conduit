"""Real Git patch application on Windows-style checkouts."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import prepare_upstream as upstream


class PrepareTests(unittest.TestCase):
    def test_lf_and_crlf_patches_produce_identical_source_without_rewriting_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            upstream.git("init", source)
            (source / "Cargo.toml").write_text('[workspace.package]\nedition = "2024"\n[workspace.dependencies]\n', encoding="utf-8")
            for name in ("shell", "base", "component", "kit"):
                crate = source / "crates" / name
                crate.mkdir(parents=True)
                (crate / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition.workspace = true\n', encoding="utf-8")
            target = source / "crates/shell/lib.rs"
            target.write_bytes(b"before\n")
            upstream.git("add", ".", cwd=source)
            upstream.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "fixture", cwd=source)
            revision = upstream.git("rev-parse", "HEAD", cwd=source).decode().strip()
            patch_file = root / "fixture.patch"
            data = b"--- a/crates/shell/lib.rs\n+++ b/crates/shell/lib.rs\n@@ -1 +1 @@\n-before\n+after\n"
            outputs = []
            with patch.object(upstream, "REV", revision), patch.object(upstream, "PATCHES", (patch_file,)):
                for name, content in (("lf", data), ("crlf", data.replace(b"\n", b"\r\n"))):
                    patch_file.write_bytes(content)
                    destination = root / name
                    result = upstream.prepare(source, destination)
                    outputs.append(result["source_sha256"])
                    self.assertEqual((destination / "crates/shell/lib.rs").read_bytes(), b"after\n")
                    self.assertEqual(patch_file.read_bytes(), content)
                    self.assertEqual(result["patch_sha256"], hashlib.sha256(b"fixture.patch\0" + content).hexdigest())
                    self.assertEqual(upstream.prepare(source, destination), result)
                self.assertEqual(*outputs)
                (destination / "crates/shell/lib.rs").write_bytes(b"local edit\n")
                with self.assertRaises(RuntimeError):
                    upstream.prepare(source, destination)
                self.assertEqual((destination / "crates/shell/lib.rs").read_bytes(), b"local edit\n")


if __name__ == "__main__":
    unittest.main()
