"""Run checkout-owned qualification steps and authorize screened uploads."""

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import platform
import re
import signal
import stat
import subprocess
import sys
from tempfile import TemporaryDirectory

import ci_artifacts as artifacts
from ci_capture import MAX_LOG, capture
from ci_privacy import JSON_LIMIT, PrivacyError, bounded_read, is_link, parse
from ci_python import checkout_contracts, contract_details, contract_selector
from ci_results import ANSI, browser_details, classify, count, mode_details, smoke_details
from ci_rust import CargoDiagnostics, checkout_inventory
import package
from run import MODES as COMMANDS


ROOT = Path(__file__).resolve().parents[2]
DASHBOARD = ROOT / "apps/dashboard"
BROWSERS = ("chromium", "firefox", "webkit")
LINUX = "x86_64-unknown-linux-gnu"
INTEL_MAC = "x86_64-apple-darwin"
RUST_MODES = {"rust", "normal", "build", "embedded-tests"}
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


def mode_recipes(private, target):
    result = {}
    for name, (kind, seconds) in MODES.items():
        if name == "embedded-tests" and target == INTEL_MAC:
            seconds = 9000
        argv = python("run", name, "--output", private, "--capture-timeout", seconds)
        result[name] = Run(argv, kind, seconds + 60 if name in RUST_MODES else seconds)
    return result


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
        **mode_recipes(private, target),
    }


def project(step, raw, private, source, status, default, targets=None):
    if step in MODES:
        return mode_details(private, step, source, status, default, targets)
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


def diagnostic_targets(step, private, environment):
    log = private / (step + "-cargo-metadata.log")
    result = capture(["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
                     log, cwd=ROOT, environment=environment, timeout=30, limit=JSON_LIMIT)
    if result.exit or result.problem:
        return {}
    try:
        return checkout_inventory(ROOT, bounded_read(log, JSON_LIMIT))
    except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.SubprocessError):
        return {}


def mode_progress(step, private, targets):
    commands = COMMANDS[step]
    for index, command in reversed(list(enumerate(commands, 1))):
        log = private / f"{step}-{index}.log"
        try:
            info = log.lstat()
            if is_link(info) or not stat.S_ISREG(info.st_mode) or info.st_size > MAX_LOG:
                break
            with log.open("rb") as child:
                raw = child.read(MAX_LOG + 1)
            if len(raw) > MAX_LOG:
                break
            if step == "contracts":
                selector = contract_selector(command)
                fields = {"phase": "contracts"}
                if selector:
                    fields, _ = contract_details(ANSI.sub(b"", raw), checkout_contracts(ROOT, selector))
            else:
                fields = CargoDiagnostics(ROOT, targets, command).accept(raw, partial=True).fields()
            return {"commandIndex": index, **fields}
        except FileNotFoundError:
            continue
        except (OSError, ValueError, subprocess.SubprocessError):
            break
    return {"phase": "preparing"}


def execute(step, recipe, private, source, environment, limit=MAX_LOG):
    log = private / (step + "-outer.log")
    targets = diagnostic_targets(step, private, environment) if step in RUST_MODES else None
    show_progress = targets is not None or step == "contracts"

    def report_progress(elapsed):
        publish_result(step, source, {"event": "progress", "elapsedSeconds": round(elapsed, 1),
                                     **mode_progress(step, private, targets)})

    captured = capture(recipe.argv, log, cwd=recipe.cwd, environment=environment, timeout=recipe.timeout,
                       limit=limit, progress=report_progress if show_progress else None)
    problem = captured.problem
    details = {}
    if not problem:
        try:
            raw = bounded_read(log, MAX_LOG)
            details, report_problem = project(step, raw, private, source, captured.exit, recipe.category, targets)
            problem = report_problem or (classify(raw, recipe.category) if captured.exit else None)
        except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.SubprocessError):
            # Preserve the command exit even if its evidence is missing or
            # malformed. A successful command without required evidence fails.
            problem = "SCREENING_ERROR"
            if captured.exit and log.is_file():
                try:
                    classified = classify(bounded_read(log, MAX_LOG), problem)
                    problem = classified
                except (OSError, ValueError):
                    pass
    elif show_progress and problem in {"TIMEOUT", "INTERRUPTED", "OUTPUT_LIMIT"}:
        details = {"partial": mode_progress(step, private, targets)}
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


def result_exit(result):
    status = result["exit"]
    return 0 if result["classification"] == "PASS" and status == 0 else (status if 0 < status < 256 else 1)


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
        files = artifacts.read_screened(downloaded, source.commit, LINUX)
        name = package.TARGETS[LINUX][2]
        if files[name + ".sha256"] != artifacts.checksum(name, files[name]):
            raise PrivacyError("Browser archive checksum mismatch")
        with TemporaryDirectory(prefix="browser-archive-", dir=private) as scratch:
            verified = Path(scratch) / name
            verified.write_bytes(files[name])
            manifest = package.unpack(verified, private / "browser-package")
        if (manifest["sourceCommit"], manifest["sourceTree"], manifest["version"], manifest["target"]) != (
            source.commit, source.tree, source.version, LINUX
        ):
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
    return result_exit(result)


if __name__ == "__main__":
    raise SystemExit(main())
