import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
from datetime import datetime, timezone


root = Path(__file__).resolve().parents[2]
source = root / ".factory/v2/p07-r1"
target = root / "docs/v2/evidence/p07-r1"
target.mkdir(parents=True, exist_ok=True)
home = str(Path.home())


def read_json(path):
    data = path.read_bytes()
    return json.loads(data.decode("utf16") if data[:2] in [b"\xff\xfe", b"\xfe\xff"] else data.decode("utf-8-sig"))


def redact(value):
    if isinstance(value, dict):
        return {key: redact(item) for key, item in value.items()}
    if isinstance(value, list):
        return [redact(item) for item in value]
    if isinstance(value, str):
        return value.replace(home, "<USERPROFILE>").replace(home.replace("\\", "/"), "<USERPROFILE>")
    return value


def write_json(name, value):
    path = target / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(redact(value), indent=2, ensure_ascii=False) + "\n", encoding="utf8")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def native_trace(text):
    lines = text.splitlines()
    header = [line for line in lines[:24] if line.strip().startswith(("OpenAI Codex", "workdir:", "model:", "approval:", "sandbox:"))]
    start = next((index for index, line in enumerate(lines) if line == "exec"), len(lines))
    trace = []
    for line in lines[start:]:
        if "rmcp::" in line or "<html>" in line or "mcp startup:" in line:
            continue
        trace.append(line[:4096] + (" [line limited]" if len(line) > 4096 else ""))
    return redact("\n".join(header + ["", "Native tool trace (startup transport payloads omitted):"] + trace))


acceptance = read_json(source / "acceptance-results.json")
assert all(any(test["status"] == "passed" and test["name"].startswith(f"Real {name.lower()} coding mission:") for test in acceptance["tests"]) for name in ["Chat", "Form", "Visual"])
assert len({item["project"] for item in acceptance["pathways"].values()}) == 3
summary = {key: acceptance.get(key) for key in ["schema", "sourceCommit", "binarySha256", "scriptSha256", "cli", "cliVersion", "browser", "tests", "finished", "repairedHarnessMetadata", "guarantees"]}
summary["pathways"] = {}
for name, item in acceptance["pathways"].items():
    mission = item["mission"]
    assert mission["status"] == "completed" and item["independentTests"]["exit"] == 0 and item["independentProperties"]["exit"] == 0
    assert item["manualWorktreeWrite"]["code"] == "worktree_read_only"
    approved = [attempt for attempt in item["detail"]["attempts"] if attempt["status"] == "approved"]
    assert approved and approved[-1]["evidence"]["diffPatch"]
    reviewer = next(session for session in item["detail"]["sessions"] if session["role"] == "reviewer" and session["attemptId"] == approved[-1]["id"])
    assert approved[-1]["review"]["decision"] == "approve"
    assert approved[-1]["evidence"]["diffPatch"].strip()[:100] in reviewer["stderr"]
    fields = ["project", "conversation", "command", "root", "submittedObjective", "initialSource", "initialTests", "initialHead", "baseline", "integrationBranch", "verificationRoot", "verifiedSource", "independentTests", "independentProperties", "manualWorktreeWrite", "messages"]
    row = {key: item.get(key) for key in fields}
    row["run_id"] = mission["run_id"]
    row["integration_sha"] = mission["integration_sha"]
    row["tasks"] = item["detail"]["tasks"]
    row["attempts"] = item["detail"]["attempts"]
    row["sessions"] = []
    for session in item["detail"]["sessions"]:
        session_row = {key: session.get(key) for key in ["id", "runId", "taskId", "attemptId", "role", "agent", "command", "status", "exitCode", "durationMs", "startedAt", "finishedAt", "policyAudit", "outputMeta", "stdout"]}
        session_row["native_header"] = [line for line in session["stderr"].splitlines()[:24] if line.startswith(("OpenAI Codex", "workdir:", "model:", "approval:", "sandbox:"))]
        session_row["stderr_sha256"] = hashlib.sha256(session["stderr"].encode()).hexdigest()
        row["sessions"].append(session_row)
        if session["role"] in ["worker", "reviewer"]:
            trace_path = target / "native" / f"{name.lower()}-{session['role']}-{session['id']}.txt"
            trace_path.parent.mkdir(parents=True, exist_ok=True)
            trace_path.write_text(native_trace(session["stderr"]) + "\n", encoding="utf8")
    row["approved_reviewer_session_id"] = reviewer["id"]
    (target / f"{name.lower()}-integrated.patch").write_text(item["patch"] + "\n", encoding="utf8")
    summary["pathways"][name] = row
write_json("coding-acceptance.json", summary)

original = root / ".factory/v2/p07/real-renewal-20261030-1540/browser-projects-AQJ9ly/Project A/.factory/db.sqlite3"
connection = sqlite3.connect(original.as_uri() + "?mode=ro", uri=True)
connection.row_factory = sqlite3.Row
history = {"source": str(original), "read_only_export": True, "runs": [dict(row) for row in connection.execute("select id,objective,status,integration_sha from runs")], "sessions": [], "attempts": [dict(row) for row in connection.execute("select id,task_id,attempt_number,status,exit_code,error,review from task_attempts")]}
for session in connection.execute("select id,role,status,exit_code,command,policy_audit,stdout,stderr from agent_sessions"):
    record = {key: session[key] for key in ["id", "role", "status", "exit_code", "command", "policy_audit", "stdout"]}
    record["native_header"] = [line for line in session["stderr"].splitlines()[:24] if line.startswith(("OpenAI Codex", "workdir:", "model:", "approval:", "sandbox:"))]
    record["native_denials"] = [line for line in session["stderr"].splitlines() if " ERROR codex_core::tools::router: error=exec_command failed:" in line and "rejected: blocked by policy" in line]
    history["sessions"].append(record)
connection.close()
write_json("historical-p07-failure.json", history)
for name in ["startup-comparison.json", "preflight-environment.json", "startup-requirements.json", "mxc-preflight.json", "mxc-no-login-preflight.json", "ci-status.json"]:
    write_json(name, read_json(source / name))
write_json("reproduced-preflight.json", read_json(source / "reproduced-preflight/preflight.json"))
write_json("permission-regressions.json", read_json(source / "permission-regressions.json"))
write_json("final-replay.json", read_json(source / "final-replay.json"))
auxiliary = []
for pattern in ["permission-regressions-attempt-*.json", "final-replay-attempt-*.json"]:
    for path in sorted(source.glob(pattern)):
        data = read_json(path)
        auxiliary.append({"source": path.name, "sha256": digest(path), "tests": data.get("tests"), "failure": data.get("failure")})
write_json("auxiliary-attempt-history.json", auxiliary)
for prefix in ["mxc-preflight", "mxc-no-login-preflight", "write-preflight"]:
    text = (source / (prefix + ".stderr")).read_text(encoding="utf8")
    (target / "native" / (prefix + ".txt")).write_text(native_trace(text) + "\n", encoding="utf8")
doctor = read_json(source / "doctor-workspace.json")
write_json("doctor-permissions.json", {"codexVersion": doctor["codexVersion"], "checks": {key: doctor["checks"][key] for key in ["sandbox.helpers", "sandbox.filesystem_paths"]}})
elevated = Path(read_json(source / "preflight-environment.json")["project"]) / ".factory/worktrees/permission-preflight/permission-probe.txt"
probe = {"observed_at": datetime.now(timezone.utc).isoformat(), "operation": "Independent host read of the leaf created by the earlier elevated-backend model probe", "path": str(elevated), "no_acl_changes": True}
try:
    probe["content"] = elevated.read_bytes().decode("ascii")
    probe["result"] = "readable"
except OSError as error:
    probe.update(result="OS_ACCESS_DENIED" if isinstance(error, PermissionError) else "OS_ERROR", errno=error.errno, type=type(error).__name__)
write_json("elevated-probe-independent-read.json", probe)
attempts = []
for path in sorted((source / "attempts").rglob("acceptance-results.json")):
    data = read_json(path)
    attempts.append({"source": str(path.relative_to(source)), "sha256": digest(path), "started": data.get("started"), "failure": data.get("failure"), "tests": data.get("tests")})
write_json("acceptance-history.json", attempts)
for name in acceptance["screenshots"]:
    if name == "acceptance-failure.png":
        continue
    destination = target / "screenshots" / name
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source / "screenshots" / name, destination)
for name in read_json(source / "final-replay.json")["screenshots"]:
    shutil.copyfile(source / "screenshots" / name, target / "screenshots" / name)
print(json.dumps({"exported": str(target), "files": len(list(target.rglob("*")))}))
