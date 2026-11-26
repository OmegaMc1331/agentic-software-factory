"""Project Cargo and libtest diagnostics onto checkout-owned identifiers."""

from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import subprocess

from ci_capture import child_environment
from ci_privacy import JSON_LIMIT, PrivacyError, bounded_read, parse, text_finding


NAME = re.compile(r"[a-zA-Z_][a-zA-Z0-9_-]{0,127}")
MODULE = re.compile(r"(?:pub(?:\([^)]*\))? )?mod ([a-zA-Z_][a-zA-Z0-9_]*) \{")
MODULE_FILE = re.compile(r'(?m)^#\[path = "([A-Za-z0-9_/.-]+\.rs)"\]\nmod ([A-Za-z_][A-Za-z0-9_]*);$')
RAW_STRING = re.compile(r'(?s)\b(?:br|r)(#+)".*?"\1')
TEST_ATTR = re.compile(r"#\[(?:tokio::)?test(?:\([^]\r\n]*\))?\]")
FUNCTION = re.compile(r"(?:async )?fn ([a-zA-Z_][a-zA-Z0-9_]*)\(")
ANSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
RESULT = re.compile(
    rb"test result: (ok|FAILED)\. ([0-9]{1,7}) passed; ([0-9]{1,7}) failed; ([0-9]{1,7}) ignored;"
    rb" [0-9]{1,7} measured; [0-9]{1,7} filtered out; finished in ([0-9]{1,6}(?:\.[0-9]{1,3})?)s"
)
BUILD_DURATION = re.compile(rb"Finished .* target\(s\) in (?:(\d{1,4})m )?(\d{1,6}(?:\.\d{1,3})?)s")
SUITE_START = re.compile(rb"running ([0-9]{1,7}) tests?")
FAILURE = re.compile(rb"test (.{1,256}) \.\.\. FAILED")
MAX_FAILURES = 16


def portable_name(value):
    return isinstance(value, str) and NAME.fullmatch(value) and not text_finding(value)


def declared_tests(source, prefix=()):
    """Recognize conventionally formatted Rust test declarations."""
    modules = []
    found = set()
    attribute_indent = None
    source = RAW_STRING.sub(lambda match: "\n" * match[0].count("\n"), source)
    for line in source.splitlines():
        body = line.lstrip(" ")
        indent = len(line) - len(body)
        if not body or body.startswith(("//", "#[")):
            if TEST_ATTR.fullmatch(body):
                attribute_indent = indent
            continue
        if modules and indent == modules[-1][0] and body == "}":
            modules.pop()
        if module := MODULE.fullmatch(body):
            modules.append((indent, module[1]))
        if attribute_indent == indent and (function := FUNCTION.match(body)):
            parts = (*prefix, *(name for _, name in modules), function[1])
            name = "::".join(parts)
            if len(name) <= 256 and all(portable_name(part) for part in parts):
                found.add(name)
        attribute_indent = None
    return found


def absolute(root, path):
    return os.path.normcase(os.path.abspath(root / path))


@dataclass(frozen=True)
class Target:
    package: str
    name: str
    kind: str
    tests: frozenset

    def identifier(self):
        return {"package": self.package, "target": self.name, "kind": self.kind}


UNKNOWN_TARGET = Target("unknown", "unknown", "unknown", frozenset())


def target_key(package_id, target, root):
    kinds = target["kind"]
    if len(kinds) == 1 and kinds[0] in {"test", "bin", "lib"}:
        return package_id, kinds[0], target["name"], absolute(root, target["src_path"])
    return None


def unit_tests(source, sources):
    """Resolve the crate's conventional module files and explicit path attributes."""
    parent = source.parent
    prefixes = {source: ()}
    for file in sources:
        if file == source or not file.is_relative_to(parent):
            continue
        parts = file.relative_to(parent).with_suffix("").parts
        if parts not in {("lib",), ("main",)}:
            prefixes[file] = parts[:-1] if parts[-1] == "mod" else parts
    for file, prefix in list(prefixes.items()):
        for relative, name in MODULE_FILE.findall(sources[file]):
            member = (file.parent / relative).resolve()
            if member in prefixes and member != source:
                prefixes[member] = (*prefix, name)
    return {
        name for file, prefix in prefixes.items()
        for name in declared_tests(sources[file], prefix)
    }


def checkout_inventory(root, report):
    root = root.resolve()
    metadata = parse(report)
    members = set(metadata["workspace_members"])
    rows = subprocess.run(["git", "ls-files", "-z", "--cached", "--", "crates"], cwd=root,
                          env=child_environment(os.environ), check=True, capture_output=True, timeout=30).stdout
    if len(rows) > JSON_LIMIT:
        raise PrivacyError("Tracked source inventory exceeds its bound")
    sources = {}
    for raw in rows.split(b"\0"):
        path = Path(os.fsdecode(raw))
        if path.suffix != ".rs":
            continue
        file = root / path
        if path.is_absolute() or not file.resolve().is_relative_to(root) or file.is_symlink():
            raise PrivacyError("Unsafe tracked Rust source")
        sources[file.resolve()] = bounded_read(file, JSON_LIMIT).decode("utf-8")

    targets = {}
    for package in metadata["packages"]:
        name = package["name"]
        if package["id"] not in members or not portable_name(name):
            continue
        folder = Path(package["manifest_path"]).parent.resolve()
        if folder.parent != root / "crates":
            continue
        for target in package["targets"]:
            key = target_key(package["id"], target, root)
            source = Path(target["src_path"]).resolve()
            if not key or not portable_name(target["name"]) or source not in sources or not source.is_relative_to(folder):
                continue
            if key[1] in {"lib", "bin"} and source.parent == folder / "src":
                tests = unit_tests(source, sources)
            else:
                tests = declared_tests(sources[source])
            targets[key] = Target(name, target["name"], key[1], frozenset(tests))
    return targets


class CargoDiagnostics:
    def __init__(self, root, targets, command):
        self.root = root
        self.targets = targets
        self.testing = command[1] == "test"
        self.phase = {"fmt": "formatting", "clippy": "analysis"}.get(command[1], "build")
        self.build_complete = False
        self.build_seconds = None
        self.artifact_counts = {"newCargoArtifacts": 0, "reusedCargoArtifacts": 0}
        self.executables = {}
        self.suite = None
        self.launch_observed = False
        self.expected = None
        self.in_failure_output = False
        self.failed_symbols = []
        self.pending_failure = None
        self.failures = []
        self.omitted = 0
        self.completed_suites = 0
        self.totals = {"passed": 0, "failed": 0, "ignored": 0}
        self.elapsed_tests = 0.0

    def build_message(self, line):
        try:
            event = json.loads(line)
            kind = event.get("reason")
            if kind == "build-finished" and type(event.get("success")) is bool:
                self.build_complete = event["success"]
                if self.build_complete and self.testing:
                    self.phase = "test-discovery"
            elif kind in {"compiler-artifact", "compiler-message"}:
                if kind == "compiler-artifact" and type(event.get("fresh")) is bool:
                    field = "reusedCargoArtifacts" if event["fresh"] else "newCargoArtifacts"
                    self.artifact_counts[field] += 1
                target = self.targets.get(target_key(event["package_id"], event["target"], self.root))
                if kind == "compiler-artifact" and target and event["profile"]["test"] is True and event.get("executable"):
                    self.executables[absolute(self.root, event["executable"])] = target
                elif kind == "compiler-message" and event["message"]["level"] == "error":
                    self.suite = target
                    if re.match(r"linking with .+ failed", event["message"].get("message", "")):
                        self.phase = "linking"
        except (ValueError, TypeError, KeyError, AttributeError):
            return

    def accept(self, raw, *, partial=False):
        if partial:
            raw = raw[:raw.rfind(b"\n") + 1]
        for line in raw.split(b"\n"):
            self.line(ANSI.sub(b"", line).strip())
        if not partial and self.pending_failure:
            self.complete_suite(self.pending_failure)
        return self

    def line(self, line):
        if not self.build_complete:
            if line.startswith(b"{"):
                self.build_message(line)
            return
        if self.build_seconds is None and self.phase in {"build", "test-discovery"}:
            if build := BUILD_DURATION.fullmatch(line):
                self.build_seconds = 60 * int(build[1] or b"0") + float(build[2])
        if not self.testing:
            return
        if self.expected is not None:
            self.test_line(line)
        elif line.startswith(b"Running ") and line.endswith(b")"):
            _, marker, executable = line[:-1].rpartition(b" (")
            name = absolute(self.root, executable.decode("utf-8", errors="replace")) if marker and b"\0" not in executable else None
            self.suite = self.executables.get(name)
            self.launch_observed = True
            self.phase = "test-discovery"
        elif line.startswith(b"Doc-tests "):
            self.suite = None
            self.launch_observed = True
            self.phase = "test-discovery"
        elif self.launch_observed and (started := SUITE_START.fullmatch(line)):
            self.expected = int(started[1])
            self.phase = "test-execution"

    def test_line(self, line):
        if line == b"failures:":
            self.in_failure_output = True
        if result := RESULT.fullmatch(line):
            passed, failed, ignored = map(int, result.groups()[1:4])
            if passed + failed + ignored != self.expected or (result[1] == b"ok") == bool(failed):
                return
            if failed:
                # The modes use Cargo's default fail-fast behavior. Count the
                # last manifest-shaped summary of a failed suite only once,
                # after the protected log has closed.
                self.pending_failure = result
            elif not self.in_failure_output:
                self.complete_suite(result)
        elif not self.in_failure_output and (failed := FAILURE.fullmatch(line)) and len(self.failed_symbols) < MAX_FAILURES + 1:
            candidate = failed[1].decode("ascii", errors="replace")
            self.failed_symbols.append(candidate if self.suite and candidate in self.suite.tests else "unknown")

    def complete_suite(self, result):
        passed, failed, ignored = map(int, result.groups()[1:4])
        if sum(self.totals.values()) + passed + failed + ignored > 10_000_000:
            raise PrivacyError("Test counts exceed their bound")
        for key, count in zip(self.totals, (passed, failed, ignored)):
            self.totals[key] += count
        self.completed_suites += 1
        self.elapsed_tests += float(result[5])
        if failed:
            symbols = self.failed_symbols if len(self.failed_symbols) == failed else ["unknown"] * min(failed, MAX_FAILURES)
            count = min(len(symbols), MAX_FAILURES)
            suite = (self.suite or UNKNOWN_TARGET).identifier()
            self.failures.extend({"suite": suite, "testId": symbol} for symbol in symbols[:count])
            self.omitted = failed - count
        self.expected = None
        self.launch_observed = False
        self.failed_symbols = []
        self.phase = "test-execution" if failed else "between-test-suites"

    def fields(self):
        output = {"phase": self.phase}
        if any(self.artifact_counts.values()):
            output.update(self.artifact_counts)
        if self.testing:
            output["completedTestCount"] = self.totals["passed"] + self.totals["failed"]
            output["ignoredTestCount"] = self.totals["ignored"]
        if self.suite:
            output["suite"] = self.suite.identifier()
        if self.failures:
            output["failedTests"] = self.failures
        if self.omitted:
            output["failuresOmitted"] = self.omitted
        if self.build_seconds is not None:
            output["cargoBuildSeconds"] = self.build_seconds
        if self.completed_suites:
            output["completedTestSeconds"] = round(self.elapsed_tests, 3)
        return output
