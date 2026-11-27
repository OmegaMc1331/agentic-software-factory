"""Contracts for the public projection of Rust qualification diagnostics."""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import ci
import ci_resources
import run as qualifier
from ci_privacy import PrivacyError, digest
from ci_python import contract_details, contract_selector
from ci_results import keyboard_projection, KEYBOARD_CASE, mode_details
from ci_rust import CargoDiagnostics, PTY_TEST, SETTINGS_TEST, Target, absolute, declared_tests, target_key, unit_tests
from run import MODES
from test_ci_privacy import API_KEY, PRIVATE_UNIX, PRIVATE_WIN, SOURCE


COMMAND = MODES["normal"][0]
CHECKOUT = ci.ROOT
EXE = "target/debug/deps/a-fixture-8d1"
SAFE_TEST = "tests::static_failure"
SUITE = Target("public-fixture", "fixture", "test", frozenset({SAFE_TEST}))
TARGET = {"name": SUITE.name, "kind": ["test"], "src_path": "crates/public-fixture/tests/fixture.rs"}
KEY = target_key("fixture-package-id", TARGET, CHECKOUT)


def event(reason, **fields):
    return json.dumps({"reason": reason, **fields}).encode() + b"\n"


def compiled(*, success=True):
    return (
        event("compiler-artifact", package_id=KEY[0], target=TARGET, profile={"test": True},
              executable=absolute(CHECKOUT, EXE), diagnostic=PRIVATE_WIN, fresh=False)
        + event("build-finished", success=success)
        + b"    Finished release profile target(s) in 2m 10.50s\n"
    )


def start(count):
    return f"     Running tests/fixture.rs ({EXE})\n\nrunning {count} tests\n".encode()


def summary(passed, failed, ignored):
    status = "FAILED" if failed else "ok"
    return (f"test result: {status}. {passed} passed; {failed} failed; {ignored} ignored; "
            "0 measured; 0 filtered out; finished in 2.75s\n").encode()


class RustDiagnosticsContracts(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="asf-rust-diagnostics-")
        self.addCleanup(temporary.cleanup)
        self.raw = Path(temporary.name)

    def assert_private(self, output):
        for marker in (API_KEY, PRIVATE_WIN, PRIVATE_UNIX, "::warning::"):
            self.assertNotIn(marker, output)
        self.assertFalse((self.raw / "public").exists())

    def project(self, raw, status):
        log = self.raw / "normal-1.log"
        log.write_bytes(raw)
        receipt = self.raw / "normal-results.json"
        receipt.write_text(json.dumps({
            "schemaVersion": 1, "sourceCommit": SOURCE.commit, "testId": "normal",
            "result": "PASS" if not status else "FAIL", "sourceUnchanged": True,
            "commands": [{"argv": COMMAND, "exit": status, "sha256": digest(raw),
                          "durationSeconds": 14.375, "captureProblem": None}],
        }), encoding="utf-8")
        details, problem = mode_details(self.raw, "normal", SOURCE.commit, status, "TEST_FAILURE", {KEY: SUITE})
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            ci.publish_result("normal", SOURCE.commit, {"exit": status, "classification": problem or "PASS", **details})
        self.assert_private(output.getvalue())
        return json.loads(output.getvalue())

    def test_declarations_are_exact_checkout_owned_symbols(self):
        source = """mod tests {
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn static_failure() {}
    fn helper() {}
}
#[test]
fn plain() {}
"""
        self.assertEqual(declared_tests(source), {SAFE_TEST, "plain"})
        self.assertEqual(declared_tests(source, ("memory",)), {"memory::" + SAFE_TEST, "memory::plain"})

    def test_checkout_identifiers_cover_raw_strings_and_explicit_module_paths(self):
        self.check_explicit_module_paths(self.raw)

    def check_explicit_module_paths(self, directory):
        directory = directory.resolve(strict=True)
        source = directory / "lib.rs"
        service = directory / "service.rs"
        child = directory / "cases.rs"
        contents = {
            source: "mod service;\n",
            service: """#[cfg(test)]
#[path = "cases.rs"]
mod tests;
""",
            child: """const EXAMPLE: &str = r#"
#[test]
fn fake_declaration() {}
"#;
#[test]
fn real_declaration() {}
""",
        }
        self.assertEqual(unit_tests(source, contents), {"service::tests::real_declaration"})

    def test_explicit_module_paths_under_nested_aliases_with_spaces_and_unicode(self):
        canonical = self.raw / "module fixture café"
        canonical.mkdir()
        alias = self.raw / "module alias"
        second = self.raw / "nested module alias"
        for link, destination in ((alias, canonical), (second, alias)):
            if os.name == "nt":
                subprocess.run(
                    ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                     "$ErrorActionPreference='Stop'; New-Item -ItemType Junction "
                     "-Path $env:ASF_CONTRACT_LINK -Target $env:ASF_CONTRACT_DEST | Out-Null"],
                    env={**os.environ, "ASF_CONTRACT_LINK": str(link), "ASF_CONTRACT_DEST": str(destination)},
                    capture_output=True, check=True, timeout=20,
                )
            else:
                link.symlink_to(destination, target_is_directory=True)
        for directory in (canonical, alias, second):
            with self.subTest(alias=directory != canonical):
                self.check_explicit_module_paths(directory)

    def test_failure_preserves_scoped_static_id_counts_duration_and_actual_exit(self):
        raw = (
            compiled() + start(2) + f"test {SAFE_TEST} ... FAILED\n".encode()
            + b"test passing ... ok\nfailures:\n"
            + f"::warning::{API_KEY}\n{PRIVATE_UNIX}\n{PRIVATE_WIN}\n".encode()
            + summary(2, 0, 0)
            + b"test unexpected_panic_payload ... FAILED\n"
            + summary(1, 1, 0)
        )
        result = self.project(raw, 101)
        self.assertEqual(result["classification"], "TEST_FAILURE")
        self.assertEqual(result["exit"], 101)
        case = result["commands"][0]
        self.assertEqual((case["passed"], case["failed"], case["tests"]), (1, 1, 2))
        self.assertEqual((case["phase"], case["cargoBuildSeconds"], case["completedTestSeconds"]),
                         ("test-execution", 130.5, 2.75))
        self.assertEqual(case["failedTests"], [{"suite": SUITE.identifier(), "testId": SAFE_TEST}])

    def test_dynamic_id_with_known_suffix_and_private_path_fail_closed(self):
        for symbol in ("dynamic_module::static_failure", PRIVATE_WIN):
            with self.subTest(symbol_type="generated"):
                result = self.project(compiled() + start(1) + f"test {symbol} ... FAILED\n".encode() + summary(0, 1, 0), 101)
                self.assertEqual(result["commands"][0]["failedTests"][0]["testId"], "unknown")
                self.assertNotIn(symbol, json.dumps(result))

    def test_unknown_binary_cannot_claim_a_known_test(self):
        raw = compiled() + start(1).replace(EXE.encode(), b"target/debug/deps/other") + f"test {SAFE_TEST} ... FAILED\n".encode()
        case = self.project(raw + summary(0, 1, 0), 101)["commands"][0]
        failure = case["failedTests"][0]
        self.assertEqual(failure, {"suite": {"package": "unknown", "target": "unknown", "kind": "unknown"}, "testId": "unknown"})

    def test_rust_checkpoint_requires_reviewed_test_and_target(self):
        test = "concurrent_registration_and_open_share_one_runtime"
        target = Target("factory-projects", "projects", "test", frozenset({test}))
        metadata = {**TARGET, "name": target.name}
        key = target_key(KEY[0], metadata, CHECKOUT)
        for actual_suite, token, expected in (
            (target, "ownership.concurrent.second_owner", True),
            (target, "private_source_fragment", False),
            (SUITE, "ownership.concurrent.second_owner", False),
        ):
            with self.subTest(approved=expected):
                examples = (API_KEY, PRIVATE_WIN, PRIVATE_UNIX, "https://git.example.invalid/private-demo",
                            "fictional conversation record", "private_source_fragment")
                raw = (compiled().replace(b'"name": "fixture"', b'"name": "projects"') + start(1)
                       + f"test {test} ... FAILED\nfailures:\n---- {test} stdout ----\n".encode()
                       + f"ASF_QA_CHECKPOINT:{token}: {examples!r}\n".encode()
                       + summary(0, 1, 0))
                diagnostic = CargoDiagnostics(CHECKOUT, {key: actual_suite}, COMMAND).accept(raw)
                failed = diagnostic.fields()["failedTests"][0]
                self.assertEqual("checkpoint" in failed, expected)
                if expected:
                    self.assertEqual(failed["checkpoint"], token)
                    self.assertEqual(failed["classification"], "ASSERTION_FAILURE")
                view = json.dumps(diagnostic.fields())
                for private in examples:
                    self.assertNotIn(private, view)
                partial = CargoDiagnostics(CHECKOUT, {key: actual_suite}, COMMAND).accept(raw, partial=True)
                heartbeat = json.dumps(partial.fields())
                for private in examples:
                    self.assertNotIn(private, heartbeat)

    def test_compilation_and_link_failures_have_no_test_count_or_message(self):
        for message, phase in (("could not compile " + PRIVATE_WIN, "build"), ("linking with compiler failed " + API_KEY, "linking")):
            with self.subTest(phase=phase):
                raw = event("compiler-message", package_id=KEY[0], target=TARGET,
                            message={"level": "error", "message": message})
                result = self.project(raw + event("build-finished", success=False), 101)
                command = result["commands"][0]
                self.assertEqual((result["classification"], command["phase"]), ("BUILD_FAILURE", phase))
                self.assertEqual(command["completedTestCount"], 0)
                self.assertNotIn("failedTests", command)

    def test_r8_observations_disallow_unknown_values_and_cross_test_fields(self):
        for scope, fields in (
            (SETTINGS_TEST, {"memory.source.status": 403, "memory.source.phase": "open"}),
            (PTY_TEST, {"pty.burst_bytes": 2293760, "pty.healthy_overflow": 0}),
        ):
            target = Target(scope[0], scope[1], "test", frozenset({scope[2]}))
            metadata = {**TARGET, "name": target.name}
            key = target_key(KEY[0], metadata, CHECKOUT)
            allowed = "".join(f"ASF_QA_DETAIL:{k}={v}\n" for k, v in fields.items())
            bad = (f"ASF_QA_DETAIL:memory.source.category={PRIVATE_WIN}\n"
                   f"ASF_QA_DETAIL:pty.bytes={API_KEY}\n"
                   "ASF_QA_DETAIL:memory.source.fixture_git=21\n"
                   "ASF_QA_DETAIL:pty.slow_overflow=500\n"
                   "ASF_QA_DETAIL:other=secret\n")
            raw = (compiled().replace(b'"name": "fixture"', f'"name": "{target.name}"'.encode())
                   + start(1) + f"test {scope[2]} ... FAILED\nfailures:\n---- {scope[2]} stdout ----\n".encode()
                   + (allowed + bad).encode() + summary(0, 1, 0))
            projection = CargoDiagnostics(CHECKOUT, {key: target}, COMMAND).accept(raw).fields()
            self.assertEqual(projection["failedTests"][0]["observations"], fields)
            self.assert_private(json.dumps(projection))

    def test_keyboard_diagnostics_require_the_frozen_case_and_allowlisted_values(self):
        fields = {"checkpoint": "editor_focus", "role": "editor", "action": "quick_open",
                  "focusReached": False, "tabCount": 180}
        case = {"name": KEYBOARD_CASE, "error": {"keyboard": fields}}
        self.assertEqual(keyboard_projection(case), fields)
        self.assertIsNone(keyboard_projection({**case, "name": "unexpected test"}))
        for changes in ({"checkpoint": PRIVATE_UNIX}, {"role": API_KEY}, {"tabCount": 181}, {"selector": PRIVATE_WIN}):
            case["error"]["keyboard"] = fields | changes
            self.assertIsNone(keyboard_projection(case))

    def test_progress_counts_completed_suites_and_ignores_partial_lines(self):
        raw = compiled() + start(2) + summary(1, 0, 1)
        partial = raw + start(1) + f"test {SAFE_TEST} ... FAILED".encode()
        dia = CargoDiagnostics(CHECKOUT, {KEY: SUITE}, COMMAND).accept(partial, partial=True).fields()
        self.assertEqual((dia["completedTestCount"], dia["ignoredTestCount"], dia["newCargoArtifacts"]), (1, 1, 1))
        self.assertEqual(dia["phase"], "test-execution")
        self.assertNotIn("failedTests", dia)
        unresolved = CargoDiagnostics(CHECKOUT, {KEY: SUITE}, COMMAND).accept(
            partial + b"\nfailures:\n" + summary(0, 1, 0), partial=True
        ).fields()
        self.assertEqual(unresolved["completedTestCount"], 1)

    def test_actual_capture_projects_progress_and_preserves_timeout(self):
        raw = compiled() + start(2) + summary(1, 0, 1) + start(2) + f"::warning::{API_KEY}\n".encode()
        log = self.raw / "normal-1.log"
        code = (
            "from pathlib import Path; import time, os;"
            "assert 'GITHUB_OUTPUT' not in os.environ;"
            f"Path({str(log)!r}).write_bytes({raw!r});"
            f"print({PRIVATE_UNIX!r}, flush=True); time.sleep(15)"
        )
        recipe = ci.Run([sys.executable, "-B", "-c", code], "TEST_FAILURE", 1.5)
        output = io.StringIO()
        with (
            patch.object(ci, "diagnostic_targets", return_value={KEY: SUITE}),
            patch("ci_capture.PROGRESS_INTERVAL", 0.1),
            contextlib.redirect_stdout(output),
        ):
            result = ci.execute("normal", recipe, self.raw, SOURCE.commit, {**os.environ, "GITHUB_OUTPUT": str(self.raw / "output")})
            ci.publish_result("normal", SOURCE.commit, result)
        self.assert_private(output.getvalue())
        published = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertTrue(any(row.get("event") == "progress" and row.get("phase") == "test-execution"
                            and row.get("completedTestCount") == 1 for row in published))
        self.assertEqual(published[-1]["classification"], "TIMEOUT")
        self.assertEqual(published[-1]["partial"]["completedTestCount"], 1)
        self.assertNotEqual(ci.result_exit(result), 0)
        self.assertFalse((self.raw / "output").exists())

    def test_interrupted_capture_preserves_closed_suite_counts(self):
        (self.raw / "normal-1.log").write_bytes(compiled() + start(1) + summary(1, 0, 0) + start(1))
        recipe = ci.Run([sys.executable, "-B", "-c", "import time; time.sleep(20)"], "TEST_FAILURE", 10)
        output = io.StringIO()
        with (
            patch.object(ci, "diagnostic_targets", return_value={KEY: SUITE}),
            patch("ci_capture.sleep", side_effect=KeyboardInterrupt),
            contextlib.redirect_stdout(output),
        ):
            result = ci.execute("normal", recipe, self.raw, SOURCE.commit, os.environ.copy())
        self.assertEqual(result["classification"], "INTERRUPTED")
        self.assertEqual(result["partial"]["completedTestCount"], 1)
        self.assert_private(output.getvalue())

    def test_malformed_or_missing_receipt_and_incomplete_success_are_rejected(self):
        for receipt in (None, "{", '{"sourceCommit":"mismatch","commands":[]}'):
            path = self.raw / "normal-results.json"
            if receipt is not None:
                path.write_text(receipt, encoding="utf-8")
            with self.assertRaises((OSError, ValueError)):
                mode_details(self.raw, "normal", SOURCE.commit, 0, "TEST_FAILURE", {KEY: SUITE})
        with self.assertRaises(PrivacyError):
            self.project(compiled() + start(1), 0)

    def test_intel_embedded_budget_is_bounded_and_target_local(self):
        intel = ci.mode_recipes(self.raw, ci.INTEL_MAC)["embedded-tests"]
        linux = ci.mode_recipes(self.raw, ci.LINUX)["embedded-tests"]
        self.assertEqual((intel.argv[-2:], intel.timeout), (["--capture-timeout", "9000"], 9060))
        self.assertEqual((linux.argv[-2:], linux.timeout), (["--capture-timeout", "6600"], 6660))


class ReleaseBuildContracts(unittest.TestCase):
    def test_d8_source_restricts_event_parent_and_workflow_only_diff(self):
        source, base = "a" * 40, "b" * 40
        environment = {
            "ASF_D8_PARENT": base,
            "GITHUB_EVENT_NAME": "push",
            "GITHUB_REF": "refs/heads/codex/v2-p14-d8-diagnostics",
            "GITHUB_REPOSITORY_ID": "1331414491",
            "GITHUB_SHA": source,
        }
        parent = f"{source} {base}\n".encode()
        workflow = b".github/workflows/p14-d8.yml\0"
        with patch.dict(os.environ, environment, clear=True):
            with patch.object(ci.subprocess, "check_output", side_effect=(parent, workflow)):
                ci.require_diagnostic_source(source)
            with patch.object(ci.subprocess, "check_output", side_effect=(parent, workflow + b"Cargo.lock\0")):
                with self.assertRaises(PrivacyError):
                    ci.require_diagnostic_source(source)
            with patch.object(ci.subprocess, "check_output", side_effect=(source.encode(), workflow)):
                with self.assertRaises(PrivacyError):
                    ci.require_diagnostic_source(source)
        with patch.dict(os.environ, {**environment, "GITHUB_EVENT_NAME": "workflow_dispatch"}, clear=True):
            with self.assertRaises(PrivacyError):
                ci.require_diagnostic_source(source)

    def test_production_and_release_test_compilation_share_one_environment(self):
        original = {"RUSTFLAGS": "external input", "CARGO_TARGET_DIR": "local-target"}
        with patch.object(qualifier, "release_rustflags", return_value="controlled-flags"):
            environment = qualifier.release_environment(original, "42")
        self.assertEqual(qualifier.RELEASE_MODES, {"build", "embedded-compile", "embedded-tests"})
        self.assertEqual(environment, {
            "SOURCE_DATE_EPOCH": "42",
            "CARGO_ENCODED_RUSTFLAGS": "controlled-flags",
            "CARGO_PROFILE_RELEASE_LTO": "true",
            "CARGO_TARGET_DIR": "local-target",
        })
        self.assertIn("RUSTFLAGS", original)
        self.assertIn("--no-run", MODES["embedded-compile"][0])
        self.assertNotIn("--no-run", MODES["embedded-tests"][0])
        for mode in qualifier.RELEASE_MODES:
            self.assertIn("--release", MODES[mode][0])
            self.assertIn("--locked", MODES[mode][0])
            self.assertIn("embedded-dashboard", MODES[mode][0])

    def test_unavailable_resource_counters_are_absent(self):
        with patch.object(ci_resources, "memory_sample", side_effect=OSError(PRIVATE_WIN)):
            with patch.object(ci_resources.shutil, "disk_usage", side_effect=OSError(PRIVATE_UNIX)):
                values = ci_resources.sample(CHECKOUT, {})
        self.assertEqual(set(values) - {"cpuConcurrency"}, set())
        self.assertNotIn(PRIVATE_WIN, json.dumps(values))
        self.assertNotIn(PRIVATE_UNIX, json.dumps(values))


class PythonDiagnosticsContracts(unittest.TestCase):
    def test_noisy_contracts_keep_case_records_and_failure_payloads_private(self):
        case = "test_contract.Demo.test_ready"
        fixture = (
            "import sys\nimport unittest\n\n"
            "class Demo(unittest.TestCase):\n"
            "    def test_ready(self):\n"
            f"        print({PRIVATE_UNIX!r}, file=sys.stderr)\n"
            "        self.assertTrue(PASSES)\n"
        )
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "test_contract.py"
            for passes in (True, False):
                with self.subTest(passes=passes):
                    path.write_text(fixture.replace("PASSES", str(passes)), encoding="utf-8")
                    attempt = subprocess.run(
                        [sys.executable, "-B", "-m", "unittest", "discover", "-s", folder,
                         "-p", "test_contract.py", "-v", "-b"],
                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=20,
                    )
                    self.assertEqual(attempt.returncode, 0 if passes else 1)
                    details, problem = contract_details(attempt.stdout, {case}, attempt.returncode)
                    self.assertEqual(problem, None if passes else "TEST_FAILURE")
                    self.assertEqual(details["completedTestCount"], 1)
                    self.assertEqual(details["tests"], 1)
                    if not passes:
                        self.assertIn(PRIVATE_UNIX.encode(), attempt.stdout)
                        self.assertEqual(details["failedContracts"][0]["testId"], case)
                    self.assertNotIn(PRIVATE_UNIX, json.dumps(details))

    def test_python_summary_and_progress_validate_known_cases_and_keep_payloads_private(self):
        case = "test_contract.Demo.test_ready"
        unknown = "test_contract.Demo.test_unknown_source"
        samples = (API_KEY, PRIVATE_WIN, PRIVATE_UNIX, "https://git.example.invalid/private-demo",
                   "fictional conversation record", "unrecognized assertion message")
        raw = (f"test_ready ({case}) ... FAIL\n"
               f"test_unknown_source ({unknown}) ... ERROR\n"
               f"FAIL: test_ready ({case})\nERROR: test_unknown_source ({unknown})\n"
               f"{samples!r}\n"
               "Ran 2 tests in 0.041s\n\nFAILED (failures=1, errors=1)\n").encode()
        approved = {case}
        partial, problem = contract_details(raw, approved)
        self.assertIsNone(problem)
        self.assertEqual(partial, {"phase": "python-contracts", "completedTestCount": 1})
        complete, problem = contract_details(raw, approved, 1)
        self.assertEqual(problem, "TEST_FAILURE")
        self.assertEqual(complete["tests"], 2)
        self.assertEqual(complete["failedContracts"], [
            {"testId": case, "classification": "ASSERTION_FAILURE"},
            {"testId": "unknown", "classification": "CONTRACT_ERROR"},
        ])
        projected = json.dumps([partial, complete])
        for private in (*samples, unknown):
            self.assertNotIn(private, projected)

    def test_python_process_exit_and_incomplete_evidence_are_distinct(self):
        case = "test_contract.Demo.test_ready"
        success = f"test_ready ({case}) ... ok\nRan 1 test in 0.010s\n\nOK\n".encode()
        self.assertIsNone(contract_details(success, {case}, 0)[1])
        self.assertEqual(contract_details(success, set(), 0)[1], "INCOMPLETE_EVIDENCE")
        self.assertEqual(contract_details(b"cannot import fixtures\n", {case}, 2)[1], "PROCESS_FAILURE")
        self.assertEqual(contract_details(b"", {case}, 0)[1], "INCOMPLETE_EVIDENCE")
        self.assertEqual(contract_details(success * 2, {case}, 0)[1], "INCOMPLETE_EVIDENCE")
        self.assertIsNotNone(contract_selector(MODES["contracts"][2]))
        self.assertIsNotNone(contract_selector(MODES["contracts"][3]))
        self.assertIsNone(contract_selector(MODES["contracts"][2][:-1] + ["*.py"]))


if __name__ == "__main__":
    unittest.main()
