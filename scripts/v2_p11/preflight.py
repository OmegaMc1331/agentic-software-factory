import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


parser = argparse.ArgumentParser()
parser.add_argument("--cli", required=True)
parser.add_argument("--output", required=True)
args = parser.parse_args()
if os.name != "nt":
    raise SystemExit("This qualification profile requires native Windows MXC; other platforms are not qualified by it.")
output = Path(args.output).resolve()
output.mkdir(parents=True, exist_ok=True)
record_path = output / "preflight.json"
if record_path.exists():
    raise SystemExit("Choose a new output directory; existing preflight evidence is preserved.")
cli = str(Path(args.cli).resolve(strict=True))
project = Path(tempfile.mkdtemp(prefix="asf-p11-preflight-"))
root = project / ".factory" / "worktrees" / "permission-preflight"


def checked(command, cwd):
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=True, timeout=60, creationflags=subprocess.CREATE_NO_WINDOW).stdout.strip()


(project / "README.md").write_text("Disposable ASF P11 permission preflight; no remote.\n", encoding="utf8")
for command in [
    ["git", "init", "-q", "-b", "main"],
    ["git", "config", "user.name", "ASF P11 preflight"],
    ["git", "config", "user.email", "r1@example.invalid"],
    ["git", "add", "README.md"],
    ["git", "commit", "-qm", "permission preflight"],
    ["git", "worktree", "add", "--detach", str(root), "HEAD"],
]:
    checked(command, project)
argv = [cli, "exec", "--ignore-user-config", "--ephemeral", "--disable", "apps", "--sandbox", "workspace-write", "-c", "windows.sandbox=mxc", "-c", "allow_login_shell=false", "--color", "never", "--model", "gpt-6.1-sol", "-c", "model_reasoning_effort=low"]
prompt = "Permission preflight, not application coding acceptance. Read README.md and inspect git status --short. Create only permission-probe.txt containing exactly ASCII ASF_P11_WRITE_OK followed by LF. Read it and run python to assert those exact bytes. Use individual commands without shell chains, and report actual commands and outcomes as JSON. Leave the probe for independent verification. Do not modify other files, commit, stage, install, publish, alter policies, request broader permissions or bypass a denial. If any operation is denied, stop and describe the observed failure."
record = {"schema": 1, "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "project": str(project), "cwd": str(root), "argv": argv, "cli_version": checked([cli, "--version"], project), "write": "NOT_QUALIFIED", "qualification": "Real-model write/read/test preflight only; this does not qualify an application patch"}
started = time.monotonic()
process = subprocess.Popen(argv, cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf8", creationflags=subprocess.CREATE_NO_WINDOW)
try:
    stdout, stderr = process.communicate(prompt, timeout=240)
except subprocess.TimeoutExpired:
    subprocess.run(["taskkill.exe", "/PID", str(process.pid), "/T", "/F"], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
    stdout, stderr = process.communicate()
    record["failure"] = "Owned preflight process tree terminated after the 240-second limit"
record["exit"] = process.returncode
record["duration_ms"] = round((time.monotonic() - started) * 1000)
record["stdout_sha256"] = hashlib.sha256(stdout.encode()).hexdigest()
record["stderr_sha256"] = hashlib.sha256(stderr.encode()).hexdigest()
record["provider_usage_limit"] = any(term in (stdout+stderr).lower() for term in ["you have hit your usage limit", "usage limit reached", "usage_limit_reached", "exceeded your current quota"])
record["native_header"] = [line for line in stderr.splitlines()[:24] if line.startswith(("OpenAI Codex", "workdir:", "model:", "approval:", "sandbox:"))]
record["model_response"] = stdout[:12000]
record["diagnostic"] = [line[:400] for line in (stdout+stderr).splitlines() if any(term in line.lower() for term in ["usage limit", "quota", "blocked by policy", "access denied", "permission denied"])][:12]
try:
    probe = root / "permission-probe.txt"
    content = probe.read_bytes()
    record["independent_probe_content"] = content.decode("ascii")
    record["git_status"] = checked(["git", "status", "--short"], root)
    if process.returncode == 0 and content == b"ASF_P11_WRITE_OK\n" and "sandbox: workspace-write" in stderr and "rejected: blocked by policy" not in stderr and "PermissionError" not in stderr:
        record["write"] = "PASS"
        probe.unlink()
        record["cleanup"] = "Removed only the independently verified probe leaf; no recursive cleanup"
except (OSError, UnicodeError) as error:
    record["independent_error"] = {"type": type(error).__name__, "errno": getattr(error, "errno", None)}
if record["provider_usage_limit"]: record["write"] = "BLOCKED_PROVIDER_USAGE_LIMIT"
record_path.write_text(json.dumps(record, indent=2), encoding="utf8")
print(json.dumps({"write": record["write"], "evidence": str(record_path), "project": str(project)}))
if record["write"] != "PASS":
    raise SystemExit(1)
