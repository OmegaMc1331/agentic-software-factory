import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time

from ci_capture import capture

ROOT = Path(__file__).resolve().parents[2]
MODES = {
    "security-fixes": [["cargo", "test", "-p", "factory-mcp", "--test", "http_mcp", "--locked", "-j", "1", "--", "--test-threads=1"], ["cargo", "test", "-p", "factory-db", "--lib", "linked_sqlite_contains_the_wal_memory_disclosure_fix", "--locked", "-j", "1", "--", "--test-threads=1"]],
    "rust": [["cargo", "fmt", "--all", "--", "--check"], ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "-j", "1", "--", "-D", "warnings"], ["cargo", "test", "--workspace", "--locked", "-j", "1", "--", "--test-threads=1"]],
    "normal": [["cargo", "test", "--workspace", "--locked", "-j", "1"]],
    "native-tools": [["cargo", "test", "-p", "factory-ide", "--test", "services", "--locked", "-j", "1", "--", "--ignored", "--test-threads=1", "--nocapture"]],
    "frontend": [["npm", "run", name] for name in ["format:check", "lint", "typecheck", "build"]],
    "frontend-tests": [["npm", "test"]],
    "build": [["cargo", "build", "--release", "--locked", "--features", "embedded-dashboard", "-p", "factory-cli", "-j", "1"]],
    "embedded-tests": [["cargo", "test", "--release", "--locked", "-p", "factory-api", "--features", "embedded-dashboard", "-j", "1", "--", "--test-threads=1"]],
    "contracts": [["node", "--test", "--test-reporter=tap", "scripts/release-checksums.test.mjs"], ["python", "-B", "scripts/v2_baseline.py", "inventory", "--check"], ["python", "-B", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_v2_baseline.py"], ["python", "-B", "-m", "unittest", "discover", "-s", "scripts/v2_p14", "-p", "test_*.py"]],
}


def tracked_fingerprint():
    result = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "crates", "apps/dashboard", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".node-version", ".github/workflows", "scripts", "install.ps1", "install.sh", "docs/v2/release/INSTALLATION.md", "docs/v2/release/UPGRADE.md", "docs/v2/release/SECURITY.md", "docs/v2/release/KNOWN-ISSUES.md", "docs/v2/release/USER-GUIDE.md"], cwd=ROOT, capture_output=True, check=True)
    digest = hashlib.sha256()
    for name in sorted(set(result.stdout.split(b"\0")) - {b""}):
        file = ROOT / os.fsdecode(name)
        if "/dist/" in file.as_posix() or "/node_modules/" in file.as_posix() or "__pycache__" in file.as_posix():
            continue
        digest.update(name)
        digest.update(file.read_bytes())
    return digest.hexdigest()


def release_rustflags(environment):
    flags = ["--remap-path-prefix=" + str(ROOT) + "=/asf/source"]
    public_ci = environment.get("ASF_P14_PUBLIC_CI") == "1"
    if public_ci or platform.system() == "Linux":
        sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], cwd=ROOT, env=environment, text=True, timeout=30).strip()
        version = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, env=environment, text=True, timeout=30)
        commit = next(line.removeprefix("commit-hash: ") for line in version.splitlines() if line.startswith("commit-hash: "))
        if len(commit) != 40 or any(c not in "0123456789abcdef" for c in commit):
            raise ValueError("Pinned compiler commit required for standard-library source remapping")
        if public_ci:
            cargo_home = Path(environment.get("CARGO_HOME", Path.home() / ".cargo")).resolve()
            for root, virtual in ((cargo_home, "/asf/cargo"), (Path(sysroot), "/asf/toolchain")):
                for spelling in dict.fromkeys((str(root), root.as_posix())):
                    flags.append(f"--remap-path-prefix={spelling}={virtual}")
            flags.append("--remap-path-prefix=" + ROOT.as_posix() + "=/asf/source")
            if "host: x86_64-pc-windows-msvc" in version.splitlines():
                flags.extend(["-C", "link-arg=/PDBALTPATH:%_PDB%"])
        rust_sources = Path(sysroot) / "lib/rustlib/src/rust"
        flags.append(f"--remap-path-prefix={rust_sources}=/rustc/{commit}")
    return "\x1f".join(flags)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=MODES)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    record = args.output / (args.mode + "-results.json")
    if record.exists():
        raise SystemExit("Preserve prior evidence; choose a new output directory")
    result = {"schemaVersion": 1, "testId": args.mode, "sourceCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "startedAt": datetime.datetime.now(datetime.UTC).isoformat(), "environment": {"platform": platform.platform(), "architecture": platform.machine()}, "sourceFingerprint": tracked_fingerprint(), "commands": []}
    public_ci = os.environ.get("ASF_P14_PUBLIC_CI") == "1"
    final_exit = 1
    try:
        for number, command in enumerate(MODES[args.mode], 1):
            cwd = ROOT / "apps/dashboard" if command[0] == "npm" else ROOT
            actual = command.copy()
            if not public_ci and os.name == "nt" and command[0] == "npm":
                actual = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", "npm.cmd", *command[1:]]
            if command[0] == "python":
                actual[0] = os.sys.executable
            log = args.output / f"{args.mode}-{number}.log"
            began = time.monotonic()
            environment = {**os.environ, "CARGO_INCREMENTAL": "0"}
            if args.mode == "build":
                environment["SOURCE_DATE_EPOCH"] = subprocess.check_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT, text=True).strip()
                environment.pop("RUSTFLAGS", None)
                environment["CARGO_ENCODED_RUSTFLAGS"] = release_rustflags(environment)
                environment["CARGO_PROFILE_RELEASE_LTO"] = "true"
            problem = None
            if public_ci:
                captured = capture(actual, log, cwd=cwd, environment=environment, timeout=6600, supervise=False)
                exit_code, problem = captured.exit, captured.problem
            else:
                with log.open("wb") as stream:
                    child = subprocess.Popen(actual, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT, env=environment)
                    print(json.dumps({"command": command, "pid": child.pid, "log": str(log)}), flush=True)
                    exit_code = child.wait()
            result["commands"].append({"argv": command, "cwd": str(cwd), "exit": exit_code, "captureProblem": problem, "durationSeconds": time.monotonic() - began, "log": log.name, "sha256": hashlib.sha256(log.read_bytes()).hexdigest()})
            record.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            if exit_code or problem:
                final_exit = exit_code if public_ci and 0 < exit_code < 256 else 1
                raise RuntimeError(f"Command failed ({exit_code}); inspect {log}")
        result["result"] = "PASS"
        if args.mode == "build":
            binary = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "release" / ("factory.exe" if os.name == "nt" else "factory")
            result["binary"] = {"path": str(binary.resolve()), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "bytes": binary.stat().st_size, "profile": "release with LTO"}
            result["embeddedAssets"] = [{"path": file.relative_to(ROOT / "apps/dashboard/dist").as_posix(), "sha256": hashlib.sha256(file.read_bytes()).hexdigest(), "bytes": file.stat().st_size} for file in sorted((ROOT / "apps/dashboard/dist").rglob("*")) if file.is_file()]
    except Exception as error:
        result["result"] = "FAIL"
        result["error"] = str(error)
    result["finishedAt"] = datetime.datetime.now(datetime.UTC).isoformat()
    result["finalSourceFingerprint"] = tracked_fingerprint()
    result["sourceUnchanged"] = result["finalSourceFingerprint"] == result["sourceFingerprint"]
    if not result["sourceUnchanged"]:
        result["result"] = "FAIL"
    record.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: result[key] for key in ["result", "sourceUnchanged", "sourceCommit"]}))
    raise SystemExit(0 if result["result"] == "PASS" else final_exit)


if __name__ == "__main__":
    main()
