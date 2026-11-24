import hashlib
import json
import pathlib
import shutil
import subprocess
import time


ROOT = pathlib.Path(__file__).resolve().parents[3]
STATE = ROOT / ".factory/v2/p01"
DESTINATION = ROOT / "docs/v2/evidence/p01"
NAMES = ["registry-metadata.json", "theia-service-spike.json", "openvscode-spike.json", "project-sizes.json", "prototype-results.json", "prototype-selected-results.json", "prototype-codemirror-first.json", "security-results.json", "benchmarks.json", "benchmark-summary.json", "service-benchmarks.json", "bundle-sizes.json", "host-windows.json", "host-cpu.json", "npm-audit.json", "npm-audit-patched.json", "license-inventory.json", "packaging-build.json", "packaging-test.json", "validation.json", "codemirror-workspace.png", "monaco-workspace.png", "packaged-codemirror.png"]


def main():
    DESTINATION.mkdir(parents=True, exist_ok=True)
    missing = []
    artifacts = []
    for name in NAMES:
        source = STATE / name
        if not source.exists():
            missing.append(name)
            continue
        shutil.copy2(source, DESTINATION / name)
        artifacts.append({"path": name, "bytes": source.stat().st_size, "sha256": hashlib.sha256(source.read_bytes()).hexdigest()})
    code = []
    experiment = ROOT / "experiments/v2-ide-evaluation"
    tracked = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "--", str(experiment.relative_to(ROOT))], cwd=ROOT, text=True).splitlines()
    for name in sorted(set(tracked)):
        source = ROOT / name
        if source.is_file():
            code.append({"path": name, "sha256": hashlib.sha256(source.read_bytes()).hexdigest()})
    manifest = {"schemaVersion": 1, "capturedAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "baseline": subprocess.check_output(["git", "merge-base", "HEAD", "origin/main"], cwd=ROOT, text=True).strip(), "implementationCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "artifacts": artifacts, "sourceFiles": code, "missing": missing, "remoteCI": {"P00": "PENDING — NO REMOTE RUN", "P01": "PENDING — NO REMOTE RUN", "exactP00HeadRuns": 0}}
    (DESTINATION / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(json.dumps({"copied": len(artifacts), "missing": missing}))
    if missing:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
