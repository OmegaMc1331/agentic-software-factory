"""Read only validated case identifiers and bounded totals from unittest output."""

import ast
import os
import re
import subprocess

from ci_capture import child_environment
from ci_privacy import JSON_LIMIT, PrivacyError, bounded_read, text_finding


SELECTORS = {("scripts", "test_v2_baseline.py"), ("scripts/v2_p14", "test_*.py")}
IDENTIFIER = rb"[a-zA-Z_][a-zA-Z0-9_.]{0,239}"
COMPLETED = re.compile(rb"(?m)^test_[a-zA-Z0-9_]+ \((" + IDENTIFIER
                       + rb")\) \.\.\. (?:ok|FAIL|ERROR|skipped[^\r\n]{0,256})\r?$")
FAILED = re.compile(rb"(?m)^(FAIL|ERROR): test_[a-zA-Z0-9_]+ \((" + IDENTIFIER + rb")\)\r?$")
SUMMARY = re.compile(rb"(?m)^Ran ([0-9]{1,5}) tests? in [0-9]{1,5}(?:\.[0-9]{1,3})?s"
                     rb"\r?\n(?:\r?\n)?(OK|FAILED)(?: \(([^)\r\n]{1,160})\))?\r?$")
COUNTS = re.compile(rb"(failures|errors|skipped|expected failures|unexpected successes)=([0-9]{1,5})")
MAX_FAILURES = 16


def contract_selector(command):
    if len(command) == 9 and command[:6] == ["python", "-B", "-m", "unittest", "discover", "-s"] and command[7] == "-p":
        selector = (command[6], command[8])
        return selector if selector in SELECTORS else None
    return None


def checkout_contracts(root, selector):
    directory, pattern = selector
    root = root.resolve()
    folder = (root / directory).resolve()
    if not folder.is_relative_to(root):
        raise PrivacyError("Unsafe Python contract directory")
    names = subprocess.run(["git", "ls-files", "-z", "--", f"{directory}/{pattern}"], cwd=root,
                           env=child_environment(os.environ), check=True, capture_output=True, timeout=30).stdout
    if len(names) > JSON_LIMIT:
        raise PrivacyError("Python contract inventory exceeds its bound")
    cases = set()
    for raw in names.split(b"\0"):
        if not raw:
            continue
        path = root / os.fsdecode(raw)
        if path.resolve().parent != folder or path.suffix != ".py":
            raise PrivacyError("Unexpected Python contract inventory path")
        for cls in ast.parse(bounded_read(path, JSON_LIMIT)).body:
            if not isinstance(cls, ast.ClassDef):
                continue
            for case in cls.body:
                if isinstance(case, (ast.FunctionDef, ast.AsyncFunctionDef)) and case.name.startswith("test_"):
                    key = f"{path.stem}.{cls.name}.{case.name}"
                    if key.isascii() and re.fullmatch(IDENTIFIER, key.encode("ascii")) and not text_finding(key):
                        cases.add(key)
    if not cases:
        raise PrivacyError("Empty Python contract inventory")
    return cases


def contract_details(raw, approved, status=None):
    """Project a partial stream or a completed unittest run."""
    if status is None:
        raw = raw[:raw.rfind(b"\n") + 1]
    completed = {match[1].decode("ascii") for match in COMPLETED.finditer(raw)}
    details = {"phase": "python-contracts", "completedTestCount": len(completed & approved)}
    if status is None:
        return details, None

    summaries = list(SUMMARY.finditer(raw))
    if len(summaries) != 1:
        return details, "PROCESS_FAILURE" if status else "INCOMPLETE_EVIDENCE"
    summary = summaries[0]
    count = int(summary[1])
    raw_counts = summary[3] or b""
    pairs = COUNTS.findall(raw_counts)
    totals = {key: int(value) for key, value in pairs}
    if (raw_counts != b", ".join(key + b"=" + value for key, value in pairs)
            or len(totals) != len(pairs) or any(value > 100_000 for value in totals.values())):
        return details, "INCOMPLETE_EVIDENCE"
    detail_total = sum(totals.get(key, 0) for key in (b"failures", b"errors", b"unexpected successes"))
    details.update(tests=count, assertionFailures=totals.get(b"failures", 0),
                   contractErrors=totals.get(b"errors", 0), ignored=totals.get(b"skipped", 0))
    if not status:
        if (summary[2] != b"OK" or not count or detail_total or len(completed) != count
                or not completed <= approved):
            return details, "INCOMPLETE_EVIDENCE"
        return details, None
    if summary[2] != b"FAILED" or not detail_total:
        return details, "PROCESS_FAILURE"
    failures = []
    for match in FAILED.finditer(raw):
        case = match[2].decode("ascii")
        failure = {"testId": case if case in approved else "unknown",
                   "classification": "ASSERTION_FAILURE" if match[1] == b"FAIL" else "CONTRACT_ERROR"}
        if failure not in failures and len(failures) < MAX_FAILURES:
            failures.append(failure)
    details["failedContracts"] = failures or [{"testId": "unknown", "classification": "CONTRACT_ERROR"}]
    if detail_total > len(details["failedContracts"]):
        details["failuresOmitted"] = detail_total - len(details["failedContracts"])
    return details, "TEST_FAILURE"
