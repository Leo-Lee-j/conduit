"""Regression checks for protecting existing profiles and demo directories."""

import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("screenshot_demo", Path(__file__).with_name("screenshot-demo.py"))
demo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo)


class DemoIsolationTests(unittest.TestCase):
    def test_child_environment_does_not_inherit_private_config_or_credentials(self):
        private = {"CONDUIT_DATA_DIR": "real-state", "USERPROFILE": "real-user",
                   "CODEX_HOME": "private-codex", "HOME": "private-home",
                   "CLAUDE_CONFIG_DIR": "private-claude", "OPENCODE_CONFIG": "private-config",
                   "OPENAI_API_KEY": "private-key", "ANTHROPIC_API_KEY": "private-key",
                   "SSH_AUTH_SOCK": "private-agent", "GIT_CONFIG_COUNT": "1",
                   "CONDUIT_CLAUDE_PATH": "real-cli", "PATH": "private-bin"}
        with patch.dict(os.environ, private):
            before = dict(os.environ)
            env = demo.isolated_environment(Path("demo-root").resolve(), str(Path("git.exe").resolve()))
            self.assertEqual(dict(os.environ), before)
        for key in private:
            self.assertNotEqual(env.get(key), private[key], key)
        self.assertEqual(env["GIT_AUTHOR_EMAIL"], "demo@example.invalid")
        self.assertEqual(env["GIT_CONFIG_NOSYSTEM"], "1")

    def test_existing_directory_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            existing = root / "state.json"
            existing.write_text("keep this real state", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "Refusing to overwrite"):
                demo.prepare(root, "unused-git")
            self.assertEqual(existing.read_text(encoding="utf-8"), "keep this real state")

    def test_rerun_keeps_edited_demo_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            demo.write(root / demo.MARKER, json.dumps({"kind": "conduit-screenshot-demo"}))
            demo.write(root / "data/state.json", "edited demo state")
            self.assertFalse(demo.prepare(root, "unused-git"))
            self.assertEqual((root / "data/state.json").read_text(encoding="utf-8"), "edited demo state")

    def test_incomplete_demo_requires_fresh_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            demo.write(root / demo.MARKER, json.dumps({"kind": "conduit-screenshot-demo"}))
            with self.assertRaisesRegex(ValueError, "state is missing"):
                demo.prepare(root, "unused-git")


if __name__ == "__main__":
    unittest.main()
