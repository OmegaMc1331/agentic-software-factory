"""Project private command diagnostics onto a small public schema."""

import re

from ci_capture import MAX_LOG
from ci_privacy import JSON_LIMIT, PrivacyError, bounded_read, digest, parse
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
            if argv[:2] == ["cargo", "test"]:
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
        if mode == "contracts" and not status:
            # TAP and unittest summaries can be projected without test names.
            found = re.search(rb"(?m)^(?:# tests |Ran )(\d+)(?: tests?\b|$)", ANSI.sub(b"", raw))
            item["tests"] = count(int(found[1])) if found else 0
        if row.get("captureProblem") in PROBLEMS or status or parsed.get("failed"):
            problem = row.get("captureProblem") if row.get("captureProblem") in PROBLEMS else (
                "TEST_FAILURE" if parsed.get("failed") else classify(raw, kind)
            )
            item["failureCategory"] = problem
        if not exit_code and problem:
            raise PrivacyError("Inconsistent successful driver result")
        details.append(item)
    return {"commands": details}, problem


def count_duration(value):
    if type(value) not in {int, float} or not 0 <= value <= 10_000:
        raise PrivacyError("Invalid command duration")
    return round(value, 3)


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
