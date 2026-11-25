"""Run checkout-owned qualification steps and authorize screened uploads."""

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import platform
import re
import signal
import sys

import ci_artifacts as artifacts
from ci_capture import MAX_LOG, capture
from ci_privacy import JSON_LIMIT, PrivacyError, bounded_read, parse
from ci_results import browser_details, classify, count, mode_details, smoke_details
import package


ROOT = Path(__file__).resolve().parents[2]
DASHBOARD = ROOT / "apps/dashboard"
BROWSERS = ("chromium", "firefox", "webkit")
LINUX = "x86_64-unknown-linux-gnu"
MODES = {
    "frontend": ("BUILD_FAILURE", 1500), "frontend-tests": ("TEST_FAILURE", 1500),
    "contracts": ("TEST_FAILURE", 900), "rust": ("TEST_FAILURE", 6600),
    "normal": ("TEST_FAILURE", 6600), "build": ("BUILD_FAILURE", 6600),
    "embedded-tests": ("TEST_FAILURE", 6600),
}
ID = re.compile(r"[a-z][a-z0-9-]{0,39}")
STEPS = set(MODES) | {
    "install-npm", "npm-ci", "frontend-build", "npm-audit", "dependencies", "host", "native-package",
    "native-smoke", "native-checksum", "assemble", "verify-inventory", "finalize-inventory", "inventory-checksum",
    "playwright-install", "browser", "stage-native", "approve-native", "stage-inventory", "approve-inventory",
    "browser-unpack",
}


@dataclass(frozen=True)
class Run:
    argv: list
    category: str
    timeout: int = 900
    cwd: Path = ROOT


class Parser(argparse.ArgumentParser):
    def error(self, _):
        raise PrivacyError("Unsupported qualification invocation")


def python(script, *args):
    return [sys.executable, "-B", f"scripts/v2_p14/{script}.py", *map(str, args)]


def node(script, *args):
    return ["node", *map(str, (script, *args))]


def recipes(private, target, source):
    archive = private / "packages" / package.TARGETS[target][2] if target else None
    checksums = "scripts/release-checksums.mjs"
    assembled = private / "assembled"
    return {
        "install-npm": Run(["npm", "install", "--global", "--ignore-scripts", "npm@12.0.2"], "DEPENDENCY_FAILURE"),
        "npm-ci": Run(["npm", "ci"], "DEPENDENCY_FAILURE", cwd=DASHBOARD),
        "frontend-build": Run(["npm", "run", "build"], "BUILD_FAILURE", cwd=DASHBOARD),
        "npm-audit": Run(["npm", "audit", "--audit-level=high", "--json"], "DEPENDENCY_FAILURE", cwd=DASHBOARD),
        "dependencies": Run(python("dependencies", "--output", private / "dependencies"), "DEPENDENCY_FAILURE", 1400),
        "host": Run(python("package", "host", "--target", target), "BUILD_FAILURE"),
        "native-package": Run(python("package", "build", "--target", target, "--build-record", private / "build-results.json",
                                     "--output", private / "packages"), "BUILD_FAILURE"),
        "native-smoke": Run(python("smoke", "--package-dir", private / "packages",
                                  "--output", private / "packaged-smoke.json"), "TEST_FAILURE"),
        "native-checksum": Run(node(checksums, "verify", archive), "SCREENING_ERROR"),
        "assemble": Run(python("assemble", "--input", private / "downloaded",
                               "--output", assembled, "--source", source), "SCREENING_ERROR"),
        "verify-inventory": Run(node(checksums, "verify-release", assembled), "SCREENING_ERROR"),
        "finalize-inventory": Run(node(checksums, "finalize-release", assembled), "SCREENING_ERROR"),
        "inventory-checksum": Run(["sha256sum", "-c", artifacts.CHECKSUMS], "SCREENING_ERROR", cwd=assembled),
        "playwright-install": Run(["npx", "playwright", "install", "--with-deps", *BROWSERS], "DEPENDENCY_FAILURE", 1200, DASHBOARD),
        "browser": Run(node("scripts/v2_p13/browser.mjs"), "TEST_FAILURE", 1500),
        **{name: Run(python("run", name, "--output", private), kind, seconds) for name, (kind, seconds) in MODES.items()},
    }


def project(step, raw, private, source, status, default):
    if step in MODES:
        return mode_details(private, step, source, status, default)
    if step == "browser":
        return browser_details(private / "browser/results.json", status), None
    if step == "native-smoke":
        return smoke_details(private / "packaged-smoke.json", source, status), None
    if step == "npm-audit":
        scores = parse(raw)["metadata"]["vulnerabilities"]
        return {"findings": {level: count(scores[level]) for level in ("info", "low", "moderate", "high", "critical")}}, None
    if step == "dependencies":
        audit = parse(bounded_read(private / "dependencies/cargo-audit.json", JSON_LIMIT))
        return {"vulnerabilities": count(audit["vulnerabilities"]["count"])}, None
    return {}, None


def execute(step, recipe, private, source, environment, limit=MAX_LOG):
    log = private / (step + "-outer.log")
    captured = capture(recipe.argv, log, cwd=recipe.cwd, environment=environment, timeout=recipe.timeout, limit=limit)
    problem = captured.problem
    details = {}
    if not problem:
        try:
            raw = bounded_read(log, MAX_LOG)
            details, report_problem = project(step, raw, private, source, captured.exit, recipe.category)
            if captured.exit:
                problem = report_problem or classify(raw, recipe.category)
        except (OSError, ValueError, KeyError, TypeError, AttributeError):
            # Preserve the command exit even if its evidence is missing or
            # malformed. A successful command without required evidence fails.
            problem = "SCREENING_ERROR"
            if captured.exit and log.is_file():
                try:
                    classified = classify(bounded_read(log, MAX_LOG), problem)
                    problem = classified
                except (OSError, ValueError):
                    pass
    return {"exit": captured.exit, "seconds": captured.seconds, "classification": problem or "PASS", **details}


def publish_result(step, source, result):
    if not ID.fullmatch(step):
        step = "invalid"
    system = {"nt": "windows"}.get(os.name, "macos" if sys.platform == "darwin" else "linux")
    cpu = platform.machine().lower()
    output = {"step": step, "source": source, "platform": system,
              "architecture": cpu if cpu in {"amd64", "x86_64", "arm64", "aarch64"} else "unknown", **result}
    print(json.dumps(output, separators=(",", ":"), ensure_ascii=True, allow_nan=False), flush=True)


def invalidated(_signal, _frame):
    raise KeyboardInterrupt


def approve(publish, seal, source, scope):
    files = artifacts.read_screened(publish, source, scope, seal)
    for name in files.keys() | {artifacts.RECEIPT}:
        (publish / name).chmod(0o400)
    publish.chmod(0o500)
    artifacts.read_screened(publish, source, scope, seal)
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8", newline="\n") as out:
        out.write("approved=true\n")


def run(args, source):
    base = Path(os.environ["RUNNER_TEMP"]) / "asf-p14"
    private = base / "private"
    private.mkdir(parents=True, exist_ok=True)
    artifacts.directory(base)
    artifacts.directory(private)
    step, target = args.step, args.target
    if step in {"host", "native-package", "native-checksum", "stage-native", "approve-native"} and not target:
        raise PrivacyError("Native target required")
    publish = base / "public"
    if step == "stage-native":
        artifacts.stage_native(private / "packages", private / "packaged-smoke.json",
                               publish, private / "seal.json", target, source, ROOT, private)
    elif step == "approve-native":
        approve(publish, private / "seal.json", source.commit, target)
    elif step == "stage-inventory":
        artifacts.stage_inventory(private / "assembled", artifacts.received_inventory(private / "downloaded", source.commit),
                                  publish, private / "seal.json", source.commit)
    elif step == "approve-inventory":
        approve(publish, private / "seal.json", source.commit, "inventory")
    elif step == "browser-unpack":
        downloaded = private / "downloaded-linux"
        artifacts.read_screened(downloaded, source.commit, LINUX)
        manifest = package.unpack(downloaded / package.TARGETS[LINUX][2], private / "browser-package")
        if manifest["sourceCommit"] != source.commit or manifest["target"] != LINUX:
            raise PrivacyError("Unexpected browser package identity")
    else:
        if step == "assemble":
            artifacts.received_inventory(private / "downloaded", source.commit)
        env = {**os.environ, "ASF_P14_PUBLIC_CI": "1", "PYTHONUTF8": "1", "NO_COLOR": "1"}
        if step == "browser":
            env.update(ASF_P13_BINARY=str(private / "browser-package/factory"),
                       ASF_P13_OUTPUT=str(private / "browser"), ASF_BROWSER_ENGINE=args.engine)
        return execute(step, recipes(private, target, source.commit)[step], private, source.commit, env)
    return {"exit": 0, "classification": "PASS"}


def main():
    step = "invalid"
    source = None
    signal.signal(signal.SIGTERM, invalidated)
    try:
        parser = Parser()
        parser.add_argument("step")
        parser.add_argument("--target", choices=package.TARGETS)
        parser.add_argument("--engine", choices=BROWSERS, default="chromium")
        args = parser.parse_args()
        if args.step not in STEPS:
            raise PrivacyError("Invalid step identifier")
        step = args.step
        identity = artifacts.source_identity(ROOT)
        source = identity.commit
        result = run(args, identity)
    except KeyboardInterrupt:
        result = {"exit": 130, "classification": "INTERRUPTED"}
    except PrivacyError:
        result = {"exit": 1, "classification": "PRIVACY_SCREEN_FAILURE"}
    except Exception:
        result = {"exit": 1, "classification": "SCREENING_ERROR"}
    publish_result(step, source, result)
    exit_code = result["exit"]
    return 0 if result["classification"] == "PASS" and exit_code == 0 else (exit_code if 0 < exit_code < 256 else 1)


if __name__ == "__main__":
    raise SystemExit(main())
