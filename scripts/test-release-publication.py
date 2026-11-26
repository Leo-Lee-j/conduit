"""Run the workflow's PowerShell publication step against a local fake GitHub CLI."""

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ARTIFACTS = REPO / "target" / "release-publication-tests"
TAG = "v1.2.3"
RELEASE_ID = 123

MOCK_GH = r'''
import json
import os
import sys
from pathlib import Path

path = Path(os.environ["MOCK_STATE"])
state = json.loads(path.read_text(encoding="utf-8"))
args = sys.argv[1:]
state["calls"].append(args)
if args[0] == "release":
    path.write_text(json.dumps(state), encoding="utf-8")
    release = {"tagName": state["tag"], "isDraft": state["draft"]}
    if args[1] not in ("list", "view"):
        print("Unexpected release mutation", file=sys.stderr)
        sys.exit(1)
    print(json.dumps([release] if args[1] == "list" else release))
    sys.exit(0)
is_patch = "PATCH" in args
is_latest = any(arg.endswith("/latest") for arg in args)
kind = "publish" if "draft=false" in args else "latest_patch" if is_patch else "latest" if is_latest else "read"
state["counts"][kind] = state["counts"].get(kind, 0) + 1
failure = next((item for item in state["failures"]
                if item["kind"] == kind and item["at"] == state["counts"][kind]), None)

def respond_failure():
    path.write_text(json.dumps(state), encoding="utf-8")
    print(failure["message"], file=sys.stderr)
    sys.exit(1)

if failure and not failure.get("after"):
    respond_failure()
if kind == "publish":
    state["draft"] = False
    state["latest"] = not state.get("skip_latest", False)
elif kind == "latest_patch" and not state.get("ignore_latest", False):
    state["latest"] = True
if failure:
    respond_failure()
path.write_text(json.dumps(state), encoding="utf-8")
print(json.dumps({"id": 123 if not is_latest or state["latest"] else 122,
                  "tag_name": state["tag"], "prerelease": state["prerelease"], "draft": state["draft"]}))
'''


def workflow_script(step_name="Publish verified draft with retry"):
    workflow = (REPO / ".github/workflows/release.yml").read_text(encoding="utf-8")
    step = workflow.split(f"      - name: {step_name}\n", 1)[1]
    lines = step.split("        run: |\n", 1)[1].splitlines()
    body = []
    for line in lines:
        if line and not line.startswith("          "):
            break
        body.append(line[10:])
    return "\n".join(body)


class ReleasePublicationTests(unittest.TestCase):
    def setUp(self):
        ARTIFACTS.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=ARTIFACTS)
        self.root = Path(temporary.name).resolve()
        assert self.root.is_relative_to(ARTIFACTS.resolve())
        self.addCleanup(temporary.cleanup)
        self.state_path = self.root / "state.json"
        self.mock = self.root / "gh.py"
        self.mock.write_text(MOCK_GH, encoding="utf-8")
        self.sleep_path = self.root / "sleeps.txt"
        self.script = self.root / "publish.ps1"
        # All gh calls go to the local fake, even when a real gh is installed.
        # A strict native-error preference verifies that nonzero exits are caught.
        self.harness = """
$PSNativeCommandUseErrorActionPreference = $true
function gh {
    & python $env:MOCK_GH @args
    $global:LASTEXITCODE = $LASTEXITCODE
}
function Start-Sleep([double]$Seconds) {
    Add-Content -LiteralPath $env:MOCK_SLEEPS -Value $Seconds
}
"""
        self.script.write_text(self.harness + workflow_script(), encoding="utf-8")

    def publish(self, failures=(), release_id=str(RELEASE_ID), **overrides):
        state = {"calls": [], "counts": {}, "failures": list(failures),
                 "draft": True, "latest": False, "tag": TAG, "prerelease": False}
        state.update(overrides)
        self.state_path.write_text(json.dumps(state), encoding="utf-8")
        env = os.environ.copy()
        env.update(MOCK_GH=str(self.mock), MOCK_STATE=str(self.state_path),
                   MOCK_SLEEPS=str(self.sleep_path), RELEASE_TAG=TAG,
                   RELEASE_ID=release_id, GITHUB_REPOSITORY="example/conduit")
        result = subprocess.run(["pwsh", "-NoProfile", "-File", str(self.script)],
                                cwd=self.root, env=env, capture_output=True, timeout=30)
        self.state = json.loads(self.state_path.read_text(encoding="utf-8"))
        self.sleeps = ([float(line) for line in self.sleep_path.read_text().splitlines()]
                       if self.sleep_path.exists() else [])
        # The retry step must never re-upload assets or alter release notes.
        for call in self.state["calls"]:
            if call[0] == "release":
                self.assertIn(call[1], ("list", "view"))
                continue
            self.assertEqual(call[0], "api")
            self.assertNotIn("body", call)
            if "PATCH" in call:
                self.assertIn(f"tag_name={TAG}", call)
        return result

    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.state["draft"])
        self.assertTrue(self.state["latest"])

    def test_publishes_and_checks_latest(self):
        self.assert_success(self.publish())
        self.assertEqual(self.state["counts"], {"read": 2, "publish": 1, "latest": 1})
        self.assertEqual(self.sleeps, [])

    def test_500_before_publication_retries_without_uploading(self):
        failures = [dict(kind="publish", at=attempt, message="HTTP 500 (release endpoint)")
                    for attempt in [1, 2]]
        self.assert_success(self.publish(failures))
        self.assertEqual(self.state["counts"]["publish"], 3)
        self.assertEqual(self.sleeps, [5, 10])

    def test_500_after_publication_only_reads_on_retry(self):
        failure = dict(kind="publish", at=1, message="HTTP 500 (release endpoint)", after=True)
        self.assert_success(self.publish([failure]))
        self.assertEqual(self.state["counts"]["publish"], 1)
        self.assertNotIn("latest_patch", self.state["counts"])

    def test_partial_publication_repairs_only_latest(self):
        failure = dict(kind="publish", at=1, message="HTTP 502 (release endpoint)", after=True)
        self.assert_success(self.publish([failure], skip_latest=True))
        self.assertEqual(self.state["counts"]["publish"], 1)
        self.assertEqual(self.state["counts"]["latest_patch"], 1)

    def test_failed_read_after_success_does_not_publish_again(self):
        failure = dict(kind="read", at=2, message="HTTP 503 (release endpoint)")
        self.assert_success(self.publish([failure]))
        self.assertEqual(self.state["counts"]["publish"], 1)

    def test_latest_response_lost_after_server_success(self):
        failure = dict(kind="latest_patch", at=1, message="connection reset by peer", after=True)
        self.assert_success(self.publish([failure], skip_latest=True))
        self.assertEqual(self.state["counts"]["latest_patch"], 1)

    def test_already_published_and_latest_needs_no_write(self):
        self.assert_success(self.publish(draft=False, latest=True))
        self.assertFalse(any("PATCH" in call for call in self.state["calls"]))

    def test_transient_reads_retry(self):
        failures = [dict(kind="read", at=attempt, message=message)
                    for attempt, message in enumerate(
                        ["HTTP 429 (rate limit)", "TLS handshake timeout", "HTTP 408 (timeout)"], 1)]
        self.assert_success(self.publish(failures))
        self.assertEqual(self.sleeps, [5, 10, 20])

    def test_permanent_api_error_fails_without_retry(self):
        result = self.publish([dict(kind="publish", at=1, message="HTTP 403 (forbidden)")])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.sleeps, [])
        self.assertTrue(self.state["draft"])

    def test_persistent_server_failure_is_bounded_and_retains_draft(self):
        failures = [dict(kind="publish", at=attempt, message="HTTP 500 (release endpoint)")
                    for attempt in range(1, 6)]
        result = self.publish(failures)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.state["draft"])
        self.assertEqual(self.state["counts"]["publish"], 5)
        self.assertEqual(self.sleeps, [5, 10, 20, 40])
        self.assertIn(b"inspect the release", result.stderr)

    def test_published_but_latest_never_changes_does_not_report_success(self):
        result = self.publish(skip_latest=True, ignore_latest=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.state["draft"])
        self.assertEqual(self.state["counts"]["publish"], 1)
        self.assertEqual(self.state["counts"]["latest_patch"], 5)

    def test_wrong_tag_is_rejected_without_mutation(self):
        result = self.publish(tag="v1.2.4")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.state["counts"], {"read": 1})

    def test_prerelease_is_rejected_without_mutation(self):
        result = self.publish(prerelease=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.state["counts"], {"read": 1})

    def test_missing_release_id_never_contacts_api(self):
        result = self.publish(release_id="")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.state["calls"], [])

    def test_upload_step_rejects_published_release_before_any_mutation(self):
        self.script.write_text(self.harness + workflow_script("Upload and verify draft assets"),
                               encoding="utf-8")
        result = self.publish(draft=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([call[:2] for call in self.state["calls"]],
                         [["release", "list"], ["release", "view"]])
        self.assertIn(b"Expected a draft release", result.stderr)


if __name__ == "__main__":
    unittest.main()
