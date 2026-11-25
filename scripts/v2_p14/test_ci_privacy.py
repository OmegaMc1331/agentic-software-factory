import contextlib
import io
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import ci
import ci_artifacts as artifacts
from ci_capture import command_args
from ci_privacy import PrivacyError, digest, encode, screen
from ci_results import browser_details, test_counts
import package


WINDOWS = "x86_64-pc-windows-msvc"
SOURCE = artifacts.Source("a" * 40, "b" * 40, 1_700_000_000, "2.0.0-rc.11")
PRIVATE_WIN = "/".join(("C:", "Users", "fixture-private-person", "work", "example.rs"))
PRIVATE_UNIX = "/" + "/".join(("home", "fixture-private-person", "work", "example.rs"))
PRIVATE_SESSION = "/".join((".cod" + "ex", "sessions", "synthetic.jsonl"))
API_KEY = "sk" + "-proj-" + "SyntheticTestOnly" * 3


class PrivacyContracts(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="asf-privacy-tests-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.raw = self.root / "private"
        self.raw.mkdir()
        self.publish = self.root / "public"
        self.seal = self.raw / "seal.json"
        self.gh_output = self.root / "step-output"

    def execute(self, code, step="synthetic", category="UNCLASSIFIED_FAILURE", timeout=15, limit=65536, **env):
        recipe = ci.Run([sys.executable, "-B", "-c", code], category, timeout)
        result = ci.execute(step, recipe, self.raw, SOURCE.commit, {**os.environ, **env}, limit=limit)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            ci.publish_result(step, SOURCE.commit, result)
        self.assertFalse(self.publish.exists())
        self.assertNotIn("::warning::", output.getvalue())
        return json.loads(output.getvalue()), output.getvalue()

    def capture_and_hide(self, value, stream):
        output, outward = self.execute(f"import sys; print({value!r}, file=sys.{stream})")
        self.assertEqual(output["classification"], "PASS")
        self.assertNotIn(value, outward)
        self.assertIn(value, (self.raw / "synthetic-outer.log").read_text())

    def no_approval(self, target=WINDOWS):
        with patch.dict(os.environ, {"GITHUB_OUTPUT": str(self.gh_output)}):
            with self.assertRaises((PrivacyError, OSError)):
                ci.approve(self.publish, self.seal, SOURCE.commit, target)
        self.assertFalse(self.gh_output.exists())

    def test_PRIV_01_private_windows_stdout(self):
        self.capture_and_hide(PRIVATE_WIN, "stdout")

    def test_PRIV_02_private_unix_stderr(self):
        self.capture_and_hide(PRIVATE_UNIX, "stderr")

    def test_PRIV_03_private_provider_session(self):
        self.capture_and_hide(PRIVATE_SESSION, "stderr")

    def test_PRIV_04_synthetic_api_token(self):
        self.capture_and_hide(API_KEY, "stdout")

    def test_PRIV_05_authorization_header(self):
        self.capture_and_hide("Authorization: Bearer " + "fixture-value-" * 3, "stderr")

    def test_PRIV_06_session_cookie(self):
        self.capture_and_hide("Set-Cookie: factory_session=" + "fixture-value-" * 3, "stderr")

    def test_PRIV_07_authenticated_http_browser_failure(self):
        result = subprocess.run(
            ["node", "--test", "--test-reporter=tap", str(package.ROOT / "scripts/v2_p13/browser-diagnostics.test.mjs")],
            stdin=subprocess.DEVNULL, capture_output=True, timeout=20,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn(b"# pass 1", result.stdout)
        self.assertNotIn(b"synthetic-private-response-body", result.stdout + result.stderr)
        self.assertFalse(self.publish.exists())

    def fixture(self, target=WINDOWS, *, binary_suffix=b"", notices=b"Public fixture notices\n"):
        fixture = self.root / target
        fixture.mkdir()
        asset = fixture / "apps/dashboard/dist"
        asset.mkdir(parents=True)
        assets = []
        for name in ("index.html", "projects.html", "workspace.html"):
            public = b"<html>Public fixture</html>\n"
            (asset / name).write_bytes(public)
            assets.append({"path": name, "bytes": len(public), "sha256": digest(public)})
        _, _, name, executable = package.TARGETS[target]
        launcher = package.launcher_name(target)
        static = {doc: f"docs/v2/release/{doc}" for doc in package.DOCUMENTS}
        static.update({"LICENSE": "LICENSE", launcher: "scripts/v2_p14/" + launcher})
        files = {}
        for filename, relative in static.items():
            file = fixture / relative
            file.parent.mkdir(parents=True, exist_ok=True)
            data = b"Public fixture\n"
            file.write_bytes(data)
            files[filename] = (data, 0o755 if filename == launcher else 0o644)
        lockfiles = {}
        for path in package.LOCKFILES:
            (fixture / path).write_bytes(b"Synthetic public input\n")
            lockfiles[path] = digest((fixture / path).read_bytes())
        files["sbom.cdx.json"] = (encode({"bomFormat": "CycloneDX", "specVersion": "1.6", "components": [{"name": "fixture"}]}), 0o644)
        files["license-review.json"] = (encode({"componentCount": 1}), 0o644)
        files["THIRD-PARTY-NOTICES.txt"] = (notices, 0o644)
        binary = bytearray(128)
        if target == WINDOWS:
            binary[:2] = b"MZ"
            struct.pack_into("<I", binary, 60, 64)
            binary[64:68] = b"PE\0\0"
            struct.pack_into("<H", binary, 68, 0x8664)
        elif "linux" in target:
            binary[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<H", binary, 18, 62)
        else:
            binary[:4] = b"\xcf\xfa\xed\xfe"
            struct.pack_into("<I", binary, 4, 0x0100000C if target.startswith("aarch64") else 0x01000007)
        files[executable] = (bytes(binary) + binary_suffix, 0o755)
        manifest = {
            "schemaVersion": 1, "version": SOURCE.version, "sourceCommit": SOURCE.commit, "sourceTree": SOURCE.tree,
            "sourceDateEpoch": SOURCE.epoch, "target": target, "buildProfile": "release with LTO",
            "signing": "UNSIGNED", "notarization": "NOT QUALIFIED", "lockfiles": lockfiles,
            "sourceFingerprint": "c" * 64, "embeddedAssets": assets,
            "files": [{"path": file, "sha256": digest(data), "bytes": len(data), "mode": mode}
                      for file, (data, mode) in sorted(files.items())],
        }
        files["PACKAGE-MANIFEST.json"] = (encode(manifest), 0o644)
        packages = fixture / "packages"
        packages.mkdir()
        archive = packages / name
        package.write_archive(archive, files, SOURCE.epoch)
        smoke = self.seal.parent / (target + "-smoke.json")
        smoke.write_bytes(encode({"result": "PASS", "sourceCommit": SOURCE.commit, "archiveSha256": package.sha(archive),
                                  "binarySha256": digest(files[executable][0]), "verifiedEmbeddedAssets": len(assets)}))
        (packages / (name + ".sha256")).write_bytes(artifacts.checksum(name, archive.read_bytes()))
        return fixture, packages, archive, smoke

    def stage_native(self, fixture, packages, _archive, smoke, target=WINDOWS):
        artifacts.stage_native(packages, smoke, self.publish, self.seal, target, SOURCE, fixture, self.raw)

    def rejected(self, fixture, packages, archive, smoke, target=WINDOWS):
        with self.assertRaises(PrivacyError):
            self.stage_native(fixture, packages, archive, smoke, target)
        self.assertFalse(self.publish.exists())
        self.no_approval(target)

    def test_PRIV_08_escaped_json_private_path(self):
        escaped = json.dumps(PRIVATE_WIN.replace("/", "\\")).encode()
        self.rejected(*self.fixture(notices=escaped))

    def test_PRIV_09_utf16_compiler_path(self):
        self.rejected(*self.fixture(binary_suffix=PRIVATE_WIN.encode("utf-16-le")))

    def test_PRIV_10_sensitive_archive_text(self):
        self.rejected(*self.fixture(notices=API_KEY.encode()))

    def test_PRIV_11_unexpected_nested_archive(self):
        self.rejected(*self.fixture(notices=b"PK\x03\x04fixture"))

    def unexpected_file(self, name):
        self.stage_native(*self.fixture())
        (self.publish / name).write_text("Synthetic rejected file")
        self.no_approval()

    def test_PRIV_12_screenshot_in_stage(self):
        self.unexpected_file("failure.png")

    def test_PRIV_13_browser_trace_in_stage(self):
        self.unexpected_file("trace.zip")

    def test_PRIV_14_runtime_db_in_stage(self):
        self.unexpected_file("registry.sqlite3")

    def bad_zip(self, path, mode):
        fixture, packages, archive, smoke = self.fixture()
        with zipfile.ZipFile(archive, "a") as out:
            entry = zipfile.ZipInfo(path)
            entry.create_system = 3
            entry.external_attr = mode << 16
            out.writestr(entry, "synthetic")
        (packages / (archive.name + ".sha256")).write_bytes(artifacts.checksum(archive.name, archive.read_bytes()))
        self.rejected(fixture, packages, archive, smoke)
        self.assertFalse((self.root / "escape").exists())

    def test_PRIV_15_archive_traversal(self):
        self.bad_zip("../escape", stat.S_IFREG | 0o644)

    def test_PRIV_16_unsafe_archive_symlink(self):
        fixture, packages, archive, smoke = self.fixture()
        with zipfile.ZipFile(archive) as source:
            entries = [(entry, source.read(entry)) for entry in source.infolist()]
        with zipfile.ZipFile(archive, "w") as out:
            for entry, data in entries:
                if entry.filename == "LICENSE":
                    entry.external_attr = (stat.S_IFLNK | 0o777) << 16
                out.writestr(entry, data)
        (packages / (archive.name + ".sha256")).write_bytes(artifacts.checksum(archive.name, archive.read_bytes()))
        self.rejected(fixture, packages, archive, smoke)

    def test_PRIV_17_scanner_execution_failure(self):
        fixture = self.fixture()
        with patch.object(artifacts, "screen", side_effect=OSError("Synthetic scanner unavailable")):
            with self.assertRaises(OSError):
                self.stage_native(*fixture)
        self.assertFalse(self.publish.exists())
        self.no_approval()

    def test_PRIV_18_missing_receipt(self):
        self.stage_native(*self.fixture())
        (self.publish / artifacts.RECEIPT).unlink()
        self.no_approval()

    def test_PRIV_19_post_screening_substitution(self):
        self.stage_native(*self.fixture())
        name = package.TARGETS[WINDOWS][2]
        changed = b"Unsanctioned substitution"
        files = {name: changed, name + ".sha256": artifacts.checksum(name, changed)}
        for filename, data in files.items():
            (self.publish / filename).write_bytes(data)
        (self.publish / artifacts.RECEIPT).write_bytes(artifacts.receipt_for(files, SOURCE.commit, WINDOWS))
        self.no_approval()

    def test_PRIV_20_failed_test_stderr(self):
        result, public = self.execute(f"import sys; print({API_KEY!r}, file=sys.stderr); sys.exit(7)", category="TEST_FAILURE")
        self.assertEqual((result["exit"], result["classification"]), (7, "TEST_FAILURE"))
        self.assertNotIn(API_KEY, public)

    def test_PRIV_21_compile_failure_with_paths(self):
        result, public = self.execute(f"import sys; print('error: could not compile ' + {PRIVATE_WIN!r}); sys.exit(101)")
        self.assertEqual((result["exit"], result["classification"]), (101, "BUILD_FAILURE"))
        self.assertNotIn(PRIVATE_WIN, public)

    def test_PRIV_22_timeout_and_interrupted_capture(self):
        outcome, public = self.execute(f"import time; print({API_KEY!r}, flush=True); time.sleep(30)", timeout=0.5)
        self.assertEqual(outcome["classification"], "TIMEOUT")
        self.assertNotIn(API_KEY, public)
        with patch("ci_capture.time.sleep", side_effect=KeyboardInterrupt):
            outcome, public = self.execute("import time; time.sleep(30)", step="interruption")
        self.assertEqual(outcome["classification"], "INTERRUPTED")
        self.assertNotIn(API_KEY, public)

    def test_PRIV_23_workflow_command_output(self):
        command = "::" + "warning::" + API_KEY
        result, public = self.execute(f"import os; assert 'GITHUB_OUTPUT' not in os.environ; print({command!r})",
                                      GITHUB_OUTPUT=str(self.gh_output))
        self.assertEqual(result["classification"], "PASS")
        self.assertNotIn(API_KEY, public)
        self.assertFalse(self.gh_output.exists())

    def test_PRIV_24_job_summary_and_untrusted_fields(self):
        summary = self.root / "job-summary"
        result, public = self.execute(f"import os; assert 'GITHUB_STEP_SUMMARY' not in os.environ; print({PRIVATE_UNIX!r})",
                                      GITHUB_STEP_SUMMARY=str(summary))
        self.assertEqual(result["classification"], "PASS")
        self.assertNotIn(PRIVATE_UNIX, public)
        self.assertFalse(summary.exists())

    def test_PRIV_25_clean_build_summary(self):
        result, public = self.execute("print('Synthetic successful compiler output')", category="BUILD_FAILURE")
        self.assertEqual((result["exit"], result["classification"]), (0, "PASS"))
        self.assertNotIn("compiler output", public)

    def test_PRIV_26_clean_archive_formats_and_screened_path_scope(self):
        for target in package.TARGETS:
            with self.subTest(target=target):
                root, _packages, archive, _smoke = self.fixture(target)
                _binary, count = artifacts.inspect_distribution(archive.read_bytes(), target, SOURCE, root, self.raw)
                self.assertEqual(count, 3)
        screen(("/" + "/".join(("home", "runner", "work", "build.rs"))).encode())
        with self.assertRaises(PrivacyError):
            screen(PRIVATE_UNIX.encode())
        self.assertFalse(self.publish.exists())

    def test_PRIV_27_real_nonzero_exit_and_resource_classification(self):
        seen, public = self.execute("import sys; print('No space left on device'); sys.exit(11)")
        self.assertEqual((seen["exit"], seen["classification"]), (11, "RESOURCE_EXHAUSTION"))
        self.assertNotIn("No space left", public)

    def test_PRIV_28_clean_package_publication_preparation(self):
        self.stage_native(*self.fixture())
        with patch.dict(os.environ, {"GITHUB_OUTPUT": str(self.gh_output)}):
            ci.approve(self.publish, self.seal, SOURCE.commit, WINDOWS)
        self.assertEqual(self.gh_output.read_text(), "approved=true\n")
        self.assertEqual({name.name for name in self.publish.iterdir()}, artifacts.names_for(WINDOWS) | {artifacts.RECEIPT})

    def test_PRIV_29_raw_diagnostics_excluded_from_upload(self):
        fixture, packages, archive, smoke = self.fixture()
        (packages / "raw.log").write_text(API_KEY)
        (self.raw / "error.log").write_text(PRIVATE_WIN)
        self.stage_native(fixture, packages, archive, smoke)
        self.assertEqual(set(artifacts.read_screened(self.publish, SOURCE.commit, WINDOWS, self.seal)), artifacts.names_for(WINDOWS))
        self.assertFalse(any(p.name.endswith(".log") for p in self.publish.iterdir()))

    def test_PRIV_30_missing_and_ambiguous_evidence_fail_closed(self):
        seen, output = self.execute("print('Successful process with missing audit JSON')", step="npm-audit")
        self.assertEqual(seen["classification"], "SCREENING_ERROR")
        self.assertNotIn("missing audit JSON", output)
        self.stage_native(*self.fixture())
        receipt = self.publish / artifacts.RECEIPT
        receipt.write_bytes(b'{"result":"PASS","result":"PASS"}')
        self.no_approval()

    def test_compiled_counts_and_browser_metadata_are_projected(self):
        cargo = b"test result: ok. 1093 passed; 0 failed; 10 ignored; 0 measured"
        self.assertEqual(test_counts(["cargo", "test"], cargo, 0), {"passed": 1093, "failed": 0, "ignored": 10, "tests": 1103})
        npm = b"      Tests  572 passed (572)\n"
        self.assertEqual(test_counts(["npm", "test"], npm, 0)["passed"], 572)
        browser = self.raw / "browser.json"
        browser.write_bytes(encode({"status": "FAILED", "tests": [{"name": API_KEY, "status": "FAILED"}],
                                    "error": {"code": "HTTP_STATUS", "actual": 401, "message": PRIVATE_WIN}}))
        self.assertEqual(browser_details(browser, 1), {"tests": 1, "passed": 0, "failed": 1, "ignored": 0,
                                                       "failedTestIndex": 1, "assertionCode": "HTTP_STATUS", "httpStatus": 401})
        self.assertFalse(self.publish.exists())

    def test_output_limit_and_source_maps_fail_closed(self):
        result, public = self.execute(f"print({API_KEY!r} * 1000)", limit=200)
        self.assertEqual(result["classification"], "OUTPUT_LIMIT")
        self.assertNotIn(API_KEY, public)
        self.assertLessEqual((self.raw / "synthetic-outer.log").stat().st_size, 200)
        data = b"//# sourceMappingURL=data:application/json;base64,fixture"
        with self.assertRaises(PrivacyError):
            screen(data)

    def test_tar_extraction_limits_precede_member_processing(self):
        target = "x86_64-unknown-linux-gnu"
        _root, _packages, archive, _smoke = self.fixture(target)
        with patch.object(artifacts, "TOTAL_LIMIT", 64):
            with self.assertRaises(PrivacyError):
                artifacts.tar_members(archive.read_bytes(), {"factory"})
        self.assertFalse(self.publish.exists())

    def test_assembled_inventory_matches_screened_native_bytes(self):
        screened = {}
        for target in package.TARGETS:
            _root, _packages, archive, _smoke = self.fixture(target)
            files = {archive.name: archive.read_bytes()}
            files[archive.name + ".sha256"] = artifacts.checksum(archive.name, files[archive.name])
            handoff = self.raw / "handoff" / f"candidate-native-{target}-{SOURCE.commit}"
            artifacts.stage(files, handoff, self.raw / (target + "-seal.json"), SOURCE.commit, target)
            screened.update(files)
        self.assertEqual(artifacts.received_inventory(self.raw / "handoff", SOURCE.commit), screened)
        assembled = self.root / "assembled"
        assembled.mkdir()
        summary = b"".join(artifacts.checksum(name, data) for name, data in sorted(screened.items()) if not name.endswith(".sha256"))
        for name, data in (screened | {artifacts.CHECKSUMS: summary}).items():
            (assembled / name).write_bytes(data)
        artifacts.stage_inventory(assembled, screened, self.publish, self.seal, SOURCE.commit)
        self.assertEqual(set(artifacts.read_screened(self.publish, SOURCE.commit, "inventory", self.seal)), artifacts.names_for("inventory"))
        (assembled / package.TARGETS[WINDOWS][2]).write_bytes(b"unexpected")
        with self.assertRaises(PrivacyError):
            artifacts.stage_inventory(assembled, screened, self.root / "unsafe", self.raw / "unsafe-seal", SOURCE.commit)
        self.assertFalse((self.root / "unsafe").exists())


class WorkflowPrivacyContracts(unittest.TestCase):
    def test_package_manager_keeps_colon_script_names(self):
        argv = ["npm", "run", "format:check"]
        self.assertEqual(command_args(argv)[-2:], ["run", "format:check"])

    def test_every_shell_step_is_wrapped_and_graph_is_preserved(self):
        root = package.ROOT
        text = (root / ".github/workflows/ci.yml").read_text()
        runs = [line.split("run: ", 1)[1] for line in text.splitlines() if "run: " in line]
        self.assertTrue(runs)
        self.assertTrue(all(run.startswith("python -B scripts/v2_p14/ci.py ") for run in runs))
        self.assertNotIn("always()", text)
        self.assertNotIn("continue-on-error:", text)
        self.assertNotIn("pull_request_target", text)
        self.assertNotIn("secrets.", text)
        self.assertEqual(text.count("persist-credentials: false"), 5)
        self.assertEqual(text.count("package-manager-cache: false"), 5)
        for runner in ("windows-2025", "ubuntu-24.04", "macos-15", "macos-15-intel"):
            self.assertIn("os: " + runner, text)
        for step in ("frontend", "frontend-tests", "rust", "normal", "build", "embedded-tests", "dependencies",
                     "native-package", "native-smoke", "native-checksum", "inventory-checksum", "browser"):
            self.assertTrue(any(run.split()[3] == step for run in runs), step)
        self.assertIn("engine: [chromium, firefox, webkit]", text)

    def test_only_two_explicit_uploads_follow_verified_receipts(self):
        workflow = (package.ROOT / ".github/workflows/ci.yml").read_text()
        reference = "refs/heads/codex/v2-p14-release-qualification"
        self.assertIn("branches: [main, codex/v2-p14-release-qualification]", workflow)
        self.assertIn(reference, workflow)
        self.assertEqual(workflow.count("uses: actions/upload-artifact@"), 2)
        self.assertEqual(workflow.count("retention-days: 14"), 2)
        for step in ("native_privacy", "inventory_privacy"):
            self.assertIn(f"success() && steps.{step}.outputs.approved == 'true'", workflow)
        self.assertNotIn("upload-artifact@", workflow.split("\n  browsers:", 1)[1])
        paths = [line.strip() for line in workflow.splitlines() if line.strip().startswith("${{ runner.temp }}/asf-p14/public/")]
        self.assertEqual(len(paths), 13)
        self.assertTrue(all(file.rsplit("/", 1)[1] and "*" not in file for file in paths))
        manual = (package.ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("uses: ./.github/workflows/ci.yml", manual)
        self.assertIn("retain_artifacts: true", manual)
        self.assertIn("if: inputs.authorization == 'QUALIFY ONLY'", manual)


if __name__ == "__main__":
    unittest.main()
