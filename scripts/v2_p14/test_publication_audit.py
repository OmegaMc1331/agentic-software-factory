import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from publication_audit import markers, path_reasons

TOOL = Path(__file__).resolve().with_name("publication_audit.py")
TEST_EMAIL = "publication-test@example.invalid"


def windows_profile():
    return "C:" + "\\Users\\" + "synthetic-profile\\workspace"


def unix_profile():
    return "/" + "home/" + "synthetic-profile/workspace"


class PublicationAuditTests(unittest.TestCase):
    def test_profile_and_private_session_markers(self):
        windows = windows_profile()
        paths = [
            windows,
            unix_profile(),
            "/mnt/c/" + "Users/" + "synthetic-profile/workspace",
            windows.replace("\\", "\\\\"),
            windows.replace(":", "\\u003a").replace("\\", "\\u005c"),
            "file:" + "/" * 3 + windows.replace("\\", "%2F"),
        ]
        for path in paths:
            with self.subTest(path=path):
                self.assertIn("profile-path", markers(f'"path":"{path}"'))
        self.assertIn("provider-private-directory", markers(".codex/" + "sessions/fixture.jsonl"))
        self.assertIn("provider-private-directory", markers(".claude/" + "projects/fixture.jsonl"))
        self.assertNotIn("profile-path", markers("assets/UsersView.tsx"))

    def test_environment_credentials_and_non_text_review(self):
        for path in (".env", "nested/.env.local", ".factory/state.json", "docs/v2/evidence/receipt.json"):
            self.assertIn("private-file-location", path_reasons(path, "100644"))
        self.assertNotIn("private-file-location", path_reasons(".env.example", "100644"))
        self.assertIn("credential-marker", markers("sk-" + "proj-" + "A" * 28))
        self.assertIn("non-regular-git-path", path_reasons("submodule", "160000"))
        self.assertIn("non-text-artifact-review", path_reasons("screenshot.png", "100644"))
        for markup, content in [
            ("svg", "<text>" + unix_profile() + "</text>"),
            ("html", '<div data-fixture="' + windows_profile() + '"></div>'),
        ]:
            with self.subTest(markup=markup):
                self.assertIn("profile-path", markers(content))

    def test_history_only_file_and_commit_message_block_publication(self):
        with tempfile.TemporaryDirectory(prefix="asf-publication-test-") as directory:
            root = Path(directory)

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, text=True).strip()

            git("init", "-q")
            git("config", "user.name", "Synthetic Publication Fixture")
            git("config", "user.email", TEST_EMAIL)
            git("config", "commit.gpgsign", "false")
            git("commit", "-q", "--allow-empty", "-m", "public base")
            base = git("rev-parse", "HEAD")
            fixture = root / "report.html"
            fixture.write_text("<div>" + unix_profile() + "</div>", encoding="utf-8")
            git("add", "report.html")
            git("commit", "-qm", "receipt at " + windows_profile())
            blob = git("rev-parse", "HEAD:report.html")
            private_commit = git("rev-parse", "HEAD")
            git("rm", "-q", "report.html")
            git("commit", "-qm", "remove old receipt")
            head = git("rev-parse", "HEAD")
            audit_report = root / "result.json"
            result = subprocess.run(
                [sys.executable, "-B", str(TOOL), head, "--public-ref", base,
                 "--public-email", TEST_EMAIL, "--output", str(audit_report)],
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 1, result.stderr)
            report = json.loads(audit_report.read_text(encoding="utf-8"))
            self.assertEqual(report["outgoingCounts"]["commit"], 2)
            self.assertEqual(report["result"], "BLOCKED")
            self.assertTrue({blob, private_commit}.issubset({
                finding["object"] for finding in report["unresolvedFindings"]
                if finding["kind"] == "profile-path"
            }))
            self.assertNotIn(unix_profile(), audit_report.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
