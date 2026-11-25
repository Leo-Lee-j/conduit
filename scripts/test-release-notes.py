"""Exercise release-note preparation without publishing or accessing credentials."""

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ARTIFACTS = REPO / "target" / "release-notes-tests"
TAG = "v1.2.3"
NOTES = f"# Conduit {TAG}\n\n让更新更省心。\n\n## 更新亮点\n\n- 更新重启后保留草稿。\n"


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self):
        ARTIFACTS.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=ARTIFACTS)
        self.root = Path(temporary.name).resolve()
        assert self.root.is_relative_to(ARTIFACTS.resolve())
        self.addCleanup(temporary.cleanup)
        self.script = self.root / "scripts" / "prepare-release-notes.ps1"
        self.script.parent.mkdir()
        shutil.copyfile(REPO / "scripts" / self.script.name, self.script)
        self.notes = self.root / "release-notes" / f"{TAG}.md"
        self.notes.parent.mkdir()
        self.notes.write_text(NOTES, encoding="utf-8")
        self.output = self.root / "dist" / "release-notes.md"

    def prepare(self, tag=TAG, copy=True):
        command = ["pwsh", "-NoProfile", "-File", str(self.script), "-Tag", tag]
        if copy:
            command.extend(["-OutputPath", str(self.output)])
        return subprocess.run(command, cwd=ARTIFACTS, capture_output=True)

    def assert_rejected(self, tag=TAG):
        result = self.prepare(tag)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.output.exists())

    def test_copies_chinese_notes_from_other_working_directory(self):
        # CRLF and UTF-8 BOM are accepted; the signing input retains the text.
        text = NOTES.replace("\n", "\r\n")
        self.notes.write_bytes(b"\xef\xbb\xbf" + text.encode("utf-8"))
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.output.read_bytes(), text.encode("utf-8"))

    def test_validation_does_not_create_a_signing_input(self):
        result = self.prepare(copy=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.output.exists())

    def test_missing_or_empty_notes_cannot_be_published(self):
        self.notes.unlink()
        self.assert_rejected()
        self.notes.write_text("\n ", encoding="utf-8")
        self.assert_rejected()

    def test_wrong_version_cannot_be_published(self):
        self.notes.write_text(NOTES.replace(TAG, "v1.2.2"), encoding="utf-8")
        self.assert_rejected()

    def test_invalid_tags_cannot_select_other_files(self):
        for tag in ["v1.2.3-beta.1", "../TEMPLATE", "v01.2.3", "V1.2.3"]:
            with self.subTest(tag=tag):
                self.assert_rejected(tag)

    def test_unfinished_templates_cannot_be_published(self):
        for suffix in ["\n{{SUMMARY}}\n", "\n<!-- fill this in -->\n"]:
            with self.subTest(suffix=suffix):
                self.notes.write_text(NOTES + suffix, encoding="utf-8")
                self.assert_rejected()

    def test_highlights_are_present_and_fit_the_update_summary(self):
        for text in [NOTES.split("## 更新亮点")[0], NOTES.split("- 更新")[0],
                     NOTES + "- 其他变化。\n" * 5]:
            with self.subTest(text=text):
                self.notes.write_text(text, encoding="utf-8")
                self.assert_rejected()

    def test_invalid_encoding_and_oversize_notes_are_rejected(self):
        for content in [NOTES.encode("utf-8") + b"\xff", (NOTES + "中" * 90000).encode("utf-8")]:
            with self.subTest(size=len(content)):
                self.notes.write_bytes(content)
                self.assert_rejected()


if __name__ == "__main__":
    unittest.main()
