import argparse
import ctypes
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import shutil
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

from package import sha, unpack, verify_machine


def checked(program, *args, cwd=None, env=None):
    return subprocess.check_output([str(program), *args], cwd=cwd, env=env, stderr=subprocess.STDOUT, text=True, timeout=30).strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--package-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--without-node", action="store_true")
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit("Preserve prior smoke evidence")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    artifact = json.loads((args.package_dir / "artifact.json").read_text(encoding="utf-8"))
    archive = args.package_dir / artifact["artifact"]
    if sha(archive) != artifact["sha256"]:
        raise ValueError("Archive differs from artifact record")
    fixture = Path(tempfile.mkdtemp(prefix="ASF P14 λ qualification "))
    installed = fixture / "Portable application λ"
    manifest = unpack(archive, installed)
    executable = installed / ("factory.exe" if os.name == "nt" else "factory")
    verify_machine(executable.read_bytes(), manifest["target"])
    elevated = bool(ctypes.windll.shell32.IsUserAnAdmin()) if os.name == "nt" else os.geteuid() == 0
    result = {"schemaVersion": 1, "testId": "PACKAGED-APP", "sourceCommit": manifest["sourceCommit"], "timestamp": datetime.datetime.now(datetime.UTC).isoformat(), "fixture": str(fixture), "artifactSha256": sha(archive), "binarySha256": sha(executable), "driverSha256": sha(Path(__file__)), "environment": {"platform": platform.platform(), "architecture": platform.machine(), "python": platform.python_version()}, "elevated": elevated, "commands": [], "checks": [], "limitations": ["Isolated application data under the current OS account, not a newly provisioned OS account.", "No model invocation, browser launch, signing, notarization or remote write is performed by this smoke test."]}
    if elevated:
        result["limitations"].append("This privileged run does not qualify INST-07 non-administrator launch.")
    child = None
    allocated_console = False
    environment = os.environ.copy()
    environment["LOCALAPPDATA"] = str(fixture / "Local application data")
    environment["XDG_DATA_HOME"] = str(fixture / "Application data")
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    if args.without_node:
        git_path = shutil.which("git")
        if not git_path:
            raise RuntimeError("Git is required for the no-Node package check")
        paths = [str(Path(git_path).parent)]
        paths.extend([str(Path(os.environ["SystemRoot"]) / "System32")] if os.name == "nt" else ["/usr/bin", "/bin"])
        environment["PATH"] = os.pathsep.join(paths)
        if shutil.which("node", path=environment["PATH"]) or shutil.which("rustc", path=environment["PATH"]):
            raise RuntimeError("No-Node fixture still exposes Node or Rust")
        result["runtimePath"] = paths
        result["checks"].append("BUILD-05: application child PATH exposes Git and system tools, neither Node nor Rust")
    project = fixture / "User project λ"
    project.mkdir()
    (project / ".gitignore").write_text(".factory/\n", encoding="utf-8")
    (project / "source.py").write_text("VALUE = 1\n", encoding="utf-8")
    for arguments in [("init", "-q", "-b", "main"), ("config", "user.name", "ASF qualification"), ("config", "user.email", "qualification@example.invalid"), ("config", "core.autocrlf", "false"), ("config", "commit.gpgsign", "false"), ("add", "."), ("commit", "-qm", "baseline")]:
        checked("git", *arguments, cwd=project)
    checked(executable, "init", cwd=project, env=environment)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    cookie, csrf = "", ""

    def request(path, method="GET", body=None, expected=200, auth=True, token=True, origin=None):
        headers = {"Origin": origin or base, "Content-Type": "application/json"}
        if auth:
            headers["Cookie"] = cookie
        if token:
            headers["X-Factory-CSRF"] = csrf
        request = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
        try:
            response = urllib.request.urlopen(request, timeout=25)
        except urllib.error.HTTPError as error:
            response = error
        data = response.read(4_000_001)
        if response.code != expected or len(data) > 4_000_000:
            raise AssertionError(f"HTTP assertion expected {expected}, got {response.code}")
        return response, data

    def api(path, method="GET", body=None, expected=200, **options):
        return json.loads(request(path, method, body, expected, **options)[1])

    def start():
        nonlocal child, cookie, csrf, allocated_console
        flags = 0
        if os.name == "nt":
            if ctypes.windll.kernel32.GetConsoleWindow() == 0:
                allocated_console = bool(ctypes.windll.kernel32.AllocConsole())
                if allocated_console:
                    ctypes.windll.user32.ShowWindow(ctypes.windll.kernel32.GetConsoleWindow(), 0)
            flags = subprocess.CREATE_NEW_PROCESS_GROUP
        log = (fixture / f"server-{len(result['checks'])}.log").open("wb")
        command = [str(executable), "start", "--workspace", "--no-default-project", "--registry", str(fixture / "application/registry.sqlite3"), "--allow-project-root", str(fixture), "--port", str(port), "--no-browser"]
        result["commands"].append({"argv": command, "cwd": str(fixture)})
        child = subprocess.Popen(command, cwd=fixture, env=environment, creationflags=flags, stdout=log, stderr=subprocess.STDOUT)
        log.close()
        began = time.monotonic()
        for _ in range(300):
            if child.poll() is not None:
                raise RuntimeError("Packaged application exited before readiness")
            try:
                request("/api/health", auth=False, token=False)
                break
            except (OSError, urllib.error.URLError):
                time.sleep(0.05)
        else:
            raise TimeoutError("Packaged application readiness timeout")
        response, data = request("/api/auth/session", "POST", {}, auth=False, token=False)
        cookie = "; ".join(value.split(";", 1)[0] for value in response.headers.get_all("Set-Cookie", []))
        csrf = json.loads(data)["csrfToken"]
        return (time.monotonic() - began) * 1000

    def stop():
        if os.name == "nt":
            child.send_signal(signal.CTRL_BREAK_EVENT)
        else:
            child.send_signal(signal.SIGTERM)
        child.wait(timeout=55)
        if child.returncode != 0:
            raise AssertionError(f"Normal owned shutdown failed: {child.returncode}")

    try:
        result["startupMs"] = start()
        result["checks"].append("INST-05/06: actual native package starts outside checkout in spaces/Unicode path")
        if not elevated:
            result["checks"].append("INST-07: actual process token is non-administrator/non-root")
        for asset in manifest["embeddedAssets"]:
            data = request("/" + asset["path"], auth=False, token=False)[1]
            if hashlib.sha256(data).hexdigest() != asset["sha256"]:
                raise AssertionError("Embedded asset differs from build inventory: " + asset["path"])
        result["verifiedEmbeddedAssets"] = len(manifest["embeddedAssets"])
        result["checks"].append("BUILD-04: every embedded asset matches its build hash; V1 and V2 entries served")
        api("/api/projects", expected=401, auth=False, token=False)
        api("/api/projects/register", "POST", {}, expected=403, token=False)
        api("/api/projects", expected=403, origin="https://attacker.invalid")
        result["checks"].append("SEC-01/02/03: authentication, Origin and CSRF enforced by packaged server")
        entry = api("/api/projects/register", "POST", {"path": str(project), "trust_project": True}, 201)
        project_id = entry["project_id"]
        prefix = "/api/projects/" + project_id
        api(prefix + "/open", "POST", {})
        roots = api(prefix + "/files/roots")["roots"]
        root_id = next(root["id"] for root in roots if root["kind"] == "repository")
        file = {"project_id": project_id, "root_id": root_id, "path": "source.py"}
        content = api(prefix + "/files/content?" + urllib.parse.urlencode({"root_id": root_id, "path": "source.py"}))
        edit = {"file": file, "expected_revision": content["revision"], "content": "VALUE = 2\n"}
        api(prefix + "/files/content", "PUT", edit)
        api(prefix + "/files/content", "PUT", edit, 409)
        if (project / "source.py").read_text() != "VALUE = 2\n":
            raise AssertionError("Packaged file save not persisted")
        result["checks"].append("RUN-02/SEC-17: real revision-aware save; stale write denied")
        status = api(prefix + "/source-control/status?" + urllib.parse.urlencode({"rootId": root_id}))["status"]
        api(prefix + "/source-control/index", "POST", {"rootId": root_id, "key": str(uuid.uuid4()), "request": {"expectedGeneration": status["generation"], "paths": ["source.py"], "unstage": False, "hunk": None, "diffVersion": None, "trustPrograms": False}})
        status = api(prefix + "/source-control/status?" + urllib.parse.urlencode({"rootId": root_id}))["status"]
        commit = api(prefix + "/source-control/commit", "POST", {"rootId": root_id, "key": str(uuid.uuid4()), "expectedGeneration": status["generation"], "message": "Packaged native qualification", "trustPrograms": False})
        result["commit"] = checked("git", "rev-parse", "HEAD", cwd=project)
        if checked("git", "show", "HEAD:source.py", cwd=project) != "VALUE = 2" or checked("git", "diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD", cwd=project) != "source.py":
            raise AssertionError("Packaged commit did not contain exactly intended file")
        result["checks"].append("RUN-17/18: actual local staging/commit independently inspected")
        preferences = api("/api/application/preferences")
        preferences["preferences"]["theme"] = "light"
        saved = api("/api/application/preferences", "PUT", preferences)
        api("/api/application/preferences", "PUT", preferences, 409)
        collision = subprocess.run([str(executable), "start", "--port", str(port), "--no-browser", "--no-default-project", "--allow-project-root", str(fixture)], cwd=fixture, env=environment, capture_output=True, timeout=10)
        if collision.returncode == 0 or b"cannot bind" not in collision.stderr:
            raise AssertionError("Port collision did not fail clearly")
        result["checks"].append("INST-08: occupied port rejected before registry mutation")
        stop()
        result["checks"].append("RUN-20: graceful native signal shutdown returns success")
        result["restartMs"] = start()
        if len(api("/api/projects")) != 1 or api("/api/application/preferences") != saved:
            raise AssertionError("Application state did not survive restart")
        stop()
        for database in project.glob(".factory/*.sqlite3"):
            with sqlite3.connect(database) as connection:
                if connection.execute("PRAGMA quick_check").fetchone()[0] != "ok":
                    raise AssertionError("Database integrity failure")
        result["checks"].append("DB-01/05: initialized SQLite stores survive restart and pass quick_check")
        if installed.resolve().parent != fixture.resolve() or installed.is_symlink():
            raise ValueError("Portable removal target escaped the owned fixture")
        for file in sorted(installed.rglob("*"), reverse=True):
            if file.is_file():
                file.unlink()
            else:
                file.rmdir()
        installed.rmdir()
        if not (project / ".factory/db.sqlite3").is_file() or checked("git", "rev-parse", "HEAD", cwd=project) != result["commit"]:
            raise AssertionError("Removing portable application affected user project")
        result["checks"].append("INST-10: removing only portable application preserves repository and project state")
        result["result"] = "PASS"
    except Exception as error:
        result["result"] = "FAIL"
        result["error"] = str(error)
        result["failureCode"] = "SMOKE_ASSERTION" if isinstance(error, AssertionError) else "SMOKE_EXECUTION"
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait(timeout=15)
            result["forcedCleanup"] = True
        if allocated_console:
            ctypes.windll.kernel32.FreeConsole()
        args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"result": result["result"], "checks": len(result["checks"]), "failureCode": result.get("failureCode")}))
    raise SystemExit(0 if result["result"] == "PASS" else 1)


if __name__ == "__main__":
    main()
