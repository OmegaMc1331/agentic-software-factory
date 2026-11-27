"""Project private command diagnostics onto a small public schema."""

import re

from ci_capture import MAX_LOG
from ci_privacy import JSON_LIMIT, PrivacyError, bounded_read, digest, parse
from ci_python import checkout_contracts, contract_details, contract_selector
from ci_rust import CargoDiagnostics
from run import MODES, ROOT


ANSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
RUST = re.compile(rb"(?m)^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;")
VITEST = re.compile(rb"(?m)^[ \t]*Tests[ \t]+(.+)$")
PROBLEMS = {"TIMEOUT", "INTERRUPTED", "OUTPUT_LIMIT", "SCREENING_ERROR"}
RESOURCE = re.compile(rb"ENOSPC|no space left on device|cannot allocate memory|out of memory|disk full|paging file is too small", re.I)
COMPILE = re.compile(rb"could not compile|linking with .+ failed|error\[E\d{4}\]", re.I)


def count(value):
    if type(value) is not int or not 0 <= value <= 10_000_000:
        raise PrivacyError("Malformed diagnostic count")
    return value


def classify(raw, default):
    if RESOURCE.search(raw):
        return "RESOURCE_EXHAUSTION"
    if COMPILE.search(raw):
        return "BUILD_FAILURE"
    return default


def test_counts(command, raw, exit_code):
    raw = ANSI.sub(b"", raw)
    if command[:2] == ["cargo", "test"]:
        rows = RUST.findall(raw)
        if rows:
            totals = [count(sum(int(n) for n in column)) for column in zip(*rows)]
        elif exit_code:
            return {}
        else:
            raise PrivacyError("Missing cargo test results")
    elif command == ["npm", "test"]:
        lines = VITEST.findall(raw)
        if not lines and exit_code:
            return {}
        if len(lines) != 1:
            raise PrivacyError("Missing or ambiguous frontend test results")
        totals = []
        for outcome in (b"passed", b"failed", b"skipped"):
            values = re.findall(rb"(\d+) " + outcome + rb"\b", lines[0])
            if len(values) > 1:
                raise PrivacyError("Ambiguous frontend test count")
            totals.append(count(int(values[0])) if values else 0)
        if not sum(totals):
            raise PrivacyError("Empty frontend test result")
    else:
        return {}
    return dict(zip(("passed", "failed", "ignored"), totals)) | {"tests": sum(totals)}


def mode_details(folder, mode, source, exit_code, kind, targets=None):
    record = parse(bounded_read(folder / f"{mode}-results.json", JSON_LIMIT))
    rows = record["commands"]
    commands = MODES[mode]
    if (record["sourceCommit"] != source or record["testId"] != mode or type(rows) is not list
            or len(rows) > len(commands)
            or (not exit_code and (record["result"] != "PASS" or record.get("sourceUnchanged") is not True
                                   or len(rows) != len(commands)))):
        raise PrivacyError("Invalid qualification receipt")
    details = []
    problem = None
    for index, row in enumerate(rows):
        raw = bounded_read(folder / f"{mode}-{index + 1}.log", MAX_LOG)
        argv = commands[index]
        status = row["exit"]
        if row["argv"] != argv or row["sha256"] != digest(raw) or type(status) is not int:
            raise PrivacyError("Invalid private command record")
        diagnostic = {}
        if targets is not None:
            cargo = CargoDiagnostics(ROOT, targets, argv).accept(raw, partial=bool(row.get("captureProblem")))
            diagnostic = cargo.fields()
            if "--no-run" in argv and not status and not cargo.build_complete:
                raise PrivacyError("Missing successful Cargo compile receipt")
            if argv[:2] == ["cargo", "test"] and "--no-run" not in argv:
                if not status and (not cargo.completed_suites or cargo.expected is not None
                                   or cargo.totals["failed"] or cargo.launch_observed):
                    raise PrivacyError("Incomplete cargo test execution")
                parsed = {**cargo.totals, "tests": sum(cargo.totals.values())} if cargo.completed_suites else {}
            else:
                parsed = {}
        else:
            parsed = test_counts(argv, raw, status)
        item = {"commandIndex": index + 1, "exit": status,
                "seconds": count_duration(row["durationSeconds"]), **parsed, **diagnostic}
        contract_problem = None
        selector = contract_selector(argv) if mode == "contracts" else None
        if selector:
            observed, contract_problem = contract_details(ANSI.sub(b"", raw), checkout_contracts(ROOT, selector),
                                                          None if row.get("captureProblem") else status)
            item.update(observed)
        elif mode == "contracts" and not status:
            found = re.search(rb"(?m)^# tests (\d+)\b", ANSI.sub(b"", raw))
            item["tests"] = count(int(found[1])) if found else 0
        if row.get("captureProblem") in PROBLEMS or status or parsed.get("failed") or contract_problem:
            problem = row.get("captureProblem") if row.get("captureProblem") in PROBLEMS else (
                contract_problem or ("TEST_FAILURE" if parsed.get("failed") else classify(raw, kind))
            )
            item["failureCategory"] = problem
        if not exit_code and problem and problem != "INCOMPLETE_EVIDENCE":
            raise PrivacyError("Inconsistent successful driver result")
        details.append(item)
    return {"commands": details}, problem


def count_duration(value):
    if type(value) not in {int, float} or not 0 <= value <= 10_000:
        raise PrivacyError("Invalid command duration")
    return round(value, 3)


KEYBOARD_CASE = "A11Y keyboard-only project, editor, settings, conversation and notification journey"
KEYBOARD_STAGES = frozenset((
    "palette_activation", "palette_focus", "palette_query", "palette_selection", "project_switch",
    "quick_open_search", "root_ready", "result_selection", "editor_focus", "editor_input",
    "disk_save", "editor_tab_mode", "settings_search", "settings_category", "settings_category_focus",
    "preference_update", "save_focus_restore", "conversation_create", "composer_focus",
    "project_return", "notification_navigation",
))
KEYBOARD_ROLES = {"search", "editor", "result", "settings", "button", "composer", "none"}
KEYBOARD_ACTIONS = {"switch_project", "quick_open", "settings", "conversation", "return_project", "notifications", "none"}


def keyboard_projection(test):
    if test.get("name") != KEYBOARD_CASE or type(test.get("error")) is not dict:
        return None
    item = test["error"].get("keyboard")
    if (type(item) is dict and set(item) == {"checkpoint", "role", "action", "focusReached", "tabCount"}
            and all(type(item[key]) is str for key in ("checkpoint", "role", "action"))
            and item["checkpoint"] in KEYBOARD_STAGES and item["role"] in KEYBOARD_ROLES
            and item["action"] in KEYBOARD_ACTIONS and type(item["focusReached"]) is bool
            and type(item["tabCount"]) is int and 0 <= item["tabCount"] <= 180):
        return item
    return None


def browser_details(file, exit_code):
    receipt = parse(bounded_read(file, JSON_LIMIT))
    states = [test["status"] for test in receipt["tests"]]
    if any(state not in {"PASS", "FAILED"} for state in states):
        raise PrivacyError("Unknown browser test status")
    if not exit_code and (not states or "FAILED" in states or receipt["status"] != "PASS"
                          or receipt["cleanup"]["ownedServerExited"] is not True or receipt["screenshots"]):
        raise PrivacyError("Incomplete public browser test")
    details = {"tests": count(len(states)), "passed": states.count("PASS"), "failed": states.count("FAILED"), "ignored": 0}
    if "FAILED" in states:
        details["failedTestIndex"] = states.index("FAILED") + 1
        if keyboard := keyboard_projection(receipt["tests"][details["failedTestIndex"] - 1]):
            details["keyboard"] = keyboard
    failure = receipt.get("error")
    if type(failure) is dict and failure.get("code") in {"HTTP_STATUS", "BROWSER_ASSERTION", "BROWSER_EXECUTION"}:
        details["assertionCode"] = failure["code"]
        actual = failure.get("actual")
        if failure["code"] == "HTTP_STATUS" and type(actual) is int and 100 <= actual <= 599:
            details["httpStatus"] = actual
    return details


def smoke_details(file, source, exit_code):
    receipt = parse(bounded_read(file, JSON_LIMIT))
    if receipt["sourceCommit"] != source or (not exit_code and receipt["result"] != "PASS"):
        raise PrivacyError("Invalid smoke receipt")
    return {"checks": count(len(receipt["checks"])), "embeddedAssets": count(receipt.get("verifiedEmbeddedAssets", 0))}
