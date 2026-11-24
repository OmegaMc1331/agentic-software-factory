import hashlib
import json
import pathlib
import subprocess
import time
import argparse


ROOT = pathlib.Path(__file__).resolve().parents[3]
STATE = ROOT / ".factory/v2/p01"
EXPERIMENT = ROOT / "experiments/v2-ide-evaluation"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--only", nargs="+")
    options = parser.parse_args()
    commands = [
        (ROOT, ["cargo", "fmt", "--all", "--", "--check"], "rust-format"),
        (ROOT, ["cargo", "clippy", "--workspace", "--all-targets", "-j", "1", "--", "-D", "warnings"], "rust-clippy"),
        (ROOT, ["cargo", "test", "--workspace", "-j", "1", "--", "--test-threads=1"], "rust-tests"),
        (ROOT, ["python", "-B", "scripts/v2_baseline.py", "inventory", "--check"], "v1-inventory"),
        (ROOT, ["python", "-B", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_v2_baseline.py"], "tooling-tests"),
        (ROOT, ["node", "--test", "scripts/release-checksums.test.mjs"], "checksums-tests"),
        (ROOT / "apps/dashboard", ["npm.cmd", "run", "format:check"], "dashboard-format"),
        (ROOT / "apps/dashboard", ["npm.cmd", "run", "lint"], "dashboard-lint"),
        (ROOT / "apps/dashboard", ["npm.cmd", "run", "typecheck"], "dashboard-typecheck"),
        (ROOT / "apps/dashboard", ["npm.cmd", "test", "--", "--maxWorkers=1"], "dashboard-tests"),
        (ROOT / "apps/dashboard", ["npm.cmd", "run", "build"], "dashboard-build"),
        (ROOT, ["cargo", "fmt", "--manifest-path", str(EXPERIMENT / "backend/Cargo.toml"), "--", "--check"], "prototype-format"),
        (ROOT, ["cargo", "clippy", "--manifest-path", str(EXPERIMENT / "backend/Cargo.toml"), "--offline", "--target-dir", "target", "--all-targets", "-j", "1", "--", "-D", "warnings"], "prototype-clippy"),
        (ROOT, ["cargo", "clippy", "--manifest-path", str(EXPERIMENT / "backend/Cargo.toml"), "--offline", "--target-dir", "target", "--all-features", "--all-targets", "-j", "1", "--", "-D", "warnings"], "prototype-embedded-clippy"),
        (ROOT, ["cargo", "test", "--manifest-path", str(EXPERIMENT / "backend/Cargo.toml"), "--offline", "--target-dir", "target", "-j", "1", "--", "--test-threads=1"], "prototype-unit-tests"),
        (EXPERIMENT, ["npm.cmd", "run", "typecheck"], "prototype-typecheck"),
    ]
    results = json.loads((STATE / "validation.json").read_text())["results"] if options.only and (STATE / "validation.json").exists() else []
    for directory, command, name in commands:
        if options.only and name not in options.only:
            continue
        started = time.monotonic()
        log = STATE / f"validation-{name}.log"
        with log.open("w", encoding="utf-8") as stream:
            result = subprocess.run(command, cwd=directory, stdout=stream, stderr=subprocess.STDOUT)
        results = [previous for previous in results if previous["name"] != name]
        results.append({"name": name, "command": command, "cwd": str(directory.relative_to(ROOT)) or ".", "exitCode": result.returncode, "elapsedSeconds": time.monotonic() - started, "log": str(log.relative_to(ROOT)), "sha256": hashlib.sha256(log.read_bytes()).hexdigest()})
        (STATE / "validation.json").write_text(json.dumps({"schemaVersion": 1, "measuredAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "results": results}, indent=2), encoding="utf-8")
        print(f"{name}: {result.returncode}, {results[-1]['elapsedSeconds']:.1f}s", flush=True)
    if any(result["exitCode"] for result in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
