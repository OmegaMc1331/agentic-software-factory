import argparse
import hashlib
import http.cookiejar
import json
import math
import os
import platform
import re
import socket
import sqlite3
import statistics
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REFERENCE = "580eeaa29117edc7c2a6255341483dc798890f11"
FIXTURE = ROOT / "crates/factory-db/tests/fixtures/asf-v1-1.0.5.sql"
INVENTORY = ROOT / "scripts/fixtures/v1-inventory.json"
HTTP_METHODS = r"get|post|put|patch|delete|head|options|trace"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fixture_digest():
    return hashlib.sha256(FIXTURE.read_text(encoding="utf-8").encode("utf-8")).hexdigest()


def route_inventory(source):
    routes = []
    for match in re.finditer(r'\.route\(\s*"([^"\\]+)"\s*,', source):
        depth = 1
        quoted = False
        escaped = False
        end = match.end()
        while end < len(source) and depth:
            char = source[end]
            if quoted:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    quoted = False
            elif char == '"':
                quoted = True
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            end += 1
        if depth:
            raise ValueError(f"unterminated route: {match.group(1)}")
        methods = re.findall(rf"\b({HTTP_METHODS})\s*\(", source[match.end():end - 1])
        if not methods:
            raise ValueError(f"unrecognized routing expression: {match.group(1)}")
        routes.append({"path": match.group(1), "methods": sorted({m.upper() for m in methods})})
    if not routes:
        raise ValueError("no routes found")
    return sorted(routes, key=lambda route: route["path"])


def migration_inventory(source):
    migrations = []
    for match in re.finditer(r'const (V(\d+)_SCHEMA): &str = "(.*?)";', source, re.S):
        migrations.append({
            "version": int(match.group(2)),
            "name": match.group(1),
            "source_sha256": hashlib.sha256(match.group(3).encode()).hexdigest(),
        })
    if [m["version"] for m in migrations] != list(range(1, len(migrations) + 1)) or not migrations:
        raise ValueError("migration constants must be contiguous and nonempty")
    return migrations


def database_inventory(sql):
    with closing(sqlite3.connect(":memory:")) as conn:
        conn.executescript(sql)
        if conn.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
            raise ValueError("fixture integrity check failed")
        if conn.execute("PRAGMA foreign_key_check").fetchall():
            raise ValueError("fixture foreign keys are invalid")
        tables = []
        for name, ddl in conn.execute(
            "SELECT name, sql FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        ).fetchall():
            safe = '"' + name.replace('"', '""') + '"'
            columns = [dict(zip(("id", "name", "type", "not_null", "default", "primary_key"), row))
                       for row in conn.execute(f"PRAGMA table_info({safe})")]
            foreign_keys = [dict(zip(("id", "sequence", "table", "from", "to", "on_update", "on_delete", "match"), row))
                            for row in conn.execute(f"PRAGMA foreign_key_list({safe})")]
            tables.append({
                "name": name, "sql": ddl, "columns": columns, "foreign_keys": foreign_keys,
                "fixture_rows": conn.execute(f"SELECT COUNT(*) FROM {safe}").fetchone()[0],
            })
        indexes = [{"name": name, "table": table, "sql": ddl}
                   for name, table, ddl in conn.execute(
                       "SELECT name, tbl_name, sql FROM sqlite_schema WHERE type='index' AND sql IS NOT NULL ORDER BY name"
                   )]
        versions = [row[0] for row in conn.execute("SELECT version FROM schema_migrations ORDER BY version")]
        return {"migrations": versions, "tables": tables, "indexes": indexes}


def current_inventory():
    routes = route_inventory((ROOT / "crates/factory-api/src/app.rs").read_text(encoding="utf-8"))
    migrations = migration_inventory((ROOT / "crates/factory-db/src/lib.rs").read_text(encoding="utf-8"))
    return {
        "schema_version": 1, "reference_commit": REFERENCE,
        "routes": routes,
        "explicit_method_path_count": sum(len(route["methods"]) for route in routes),
        "implicit_http_behavior": "Axum GET also accepts HEAD; /api/*rest is the explicit 404 fallback; dashboard assets use a fallback service.",
        "migration_constants": migrations,
        "fixture_sha256": fixture_digest(),
        "fixture_hash_format": "SHA-256 of UTF-8 SQL text with LF newlines",
        "fixture_database": database_inventory(FIXTURE.read_text(encoding="utf-8")),
    }


def compatibility_errors(reference, current):
    errors = []
    actual_routes = {r["path"]: set(r["methods"]) for r in current["routes"]}
    for route in reference["routes"]:
        missing = set(route["methods"]) - actual_routes.get(route["path"], set())
        if missing:
            errors.append(f"removed V1 route methods: {','.join(sorted(missing))} {route['path']}")
    actual_migrations = {m["version"]: m for m in current["migration_constants"]}
    for migration in reference["migration_constants"]:
        if actual_migrations.get(migration["version"]) != migration:
            errors.append(f"rewritten or missing V1 migration: {migration['version']}")
    if reference["fixture_sha256"] != current["fixture_sha256"]:
        errors.append("frozen V1 fixture changed")
    return errors


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def summarize(samples):
    if not samples or any(not math.isfinite(sample) or sample < 0 for sample in samples):
        raise ValueError("measurements must be nonnegative and finite")
    ordered = sorted(samples)
    return {"samples_ms": samples, "n": len(samples), "median_ms": statistics.median(samples),
            "p95_ms": ordered[math.ceil(0.95 * len(ordered)) - 1], "min_ms": ordered[0], "max_ms": ordered[-1]}


def request(opener, base, path, method="GET", data=None, headers=None, expected=200, timeout=5):
    req = urllib.request.Request(base + path, method=method, data=data, headers=headers or {})
    try:
        with opener.open(req, timeout=timeout) as response:
            status, body = response.status, response.read()
    except urllib.error.HTTPError as error:
        status, body = error.code, error.read()
    if status != expected:
        raise RuntimeError(f"{method} {path}: expected HTTP {expected}, received {status}")
    return body


def unused_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def command_output(args, cwd):
    result = subprocess.run(args, cwd=cwd, capture_output=True, timeout=30, check=True)
    return result.stdout.decode("utf-8", errors="replace").strip()


def timed(call):
    start = time.perf_counter_ns()
    value = call()
    return (time.perf_counter_ns() - start) / 1_000_000, value


def populate_project(binary, root, history):
    (root / ".env").write_text("", encoding="utf-8")
    (root / ".gitignore").write_text(".factory/\n.env\n", encoding="utf-8")
    (root / "README.md").write_text("SYNTHETIC isolated V1 baseline repository\n", encoding="utf-8")
    for args in (
        ["init", "-q", "-b", "main"], ["config", "user.name", "ASF Baseline Fixture"],
        ["config", "user.email", "baseline@example.test"], ["add", "README.md", ".gitignore"],
        ["commit", "-q", "-m", "SYNTHETIC baseline fixture"],
    ):
        command_output(["git", *args], root)
    command_output([str(binary), "init"], root)
    with closing(sqlite3.connect(root / ".factory/db.sqlite3")) as conn, conn:
        tables = conn.execute("SELECT name FROM sqlite_schema WHERE type='table'").fetchall()
        if conn.execute("SELECT COUNT(*) FROM runs").fetchone()[0]:
            raise ValueError("benchmark initialization unexpectedly created workflow history")
        conn.execute("PRAGMA foreign_keys=OFF")
        for (table,) in tables:
            if not table.startswith("sqlite_"):
                conn.execute('DROP TABLE "' + table.replace('"', '""') + '"')
        conn.executescript(FIXTURE.read_text(encoding="utf-8"))
        for index in range(3, history + 1):
            conn.execute(
                "INSERT INTO runs(id,objective,status,planner_agent,created_at,updated_at) VALUES (?,?,'planned',NULL,?,?)",
                (index, f"SYNTHETIC baseline history {index}", "2026-10-26T00:00:00Z", "2026-10-26T00:00:00Z"),
            )
        if conn.execute("PRAGMA foreign_key_check").fetchall():
            raise ValueError("benchmark history violates foreign keys")


def benchmark(binary, output, samples, history, build_profile):
    binary = binary.resolve(strict=True)
    if samples < 3 or history < 2:
        raise ValueError("at least 3 samples and 2 historical workflows are required")
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    metrics = {name: [] for name in (
        "project_init", "process_start_to_health", "auth_bootstrap", "runs_page_25",
        "runs_legacy_all", "run_detail", "graph", "agent_performance",
    )}
    response_sizes = {}
    checks = {}
    with tempfile.TemporaryDirectory(prefix="owned-v1-benchmark-", dir=output.parent) as temporary:
        parent = Path(temporary).resolve()
        if not parent.is_relative_to(output.parent):
            raise ValueError("temporary benchmark directory escaped evidence directory")
        root = parent / "project"
        root.mkdir()
        populate_project(binary, root, history)
        for index in range(samples):
            fresh = parent / f"init-{index}"
            fresh.mkdir()
            (fresh / ".env").write_text("", encoding="utf-8")
            duration, _ = timed(lambda: command_output([str(binary), "init"], fresh))
            metrics["project_init"].append(duration)
            port = unused_port()
            base = f"http://127.0.0.1:{port}"
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
            with (parent / "server.log").open("ab") as log:
                start = time.perf_counter_ns()
                process = subprocess.Popen([str(binary), "dev", "serve", "--port", str(port)], cwd=root,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                                           creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                try:
                    deadline = time.monotonic() + 20
                    while True:
                        if process.poll() is not None:
                            raise RuntimeError("benchmark server exited before readiness")
                        try:
                            body = request(opener, base, "/api/health", timeout=0.05)
                            if json.loads(body) != {"status": "ok"}:
                                raise ValueError("invalid health response")
                            break
                        except (urllib.error.URLError, TimeoutError):
                            if time.monotonic() > deadline:
                                raise TimeoutError("benchmark server readiness exceeded 20 seconds")
                            time.sleep(0.01)
                    metrics["process_start_to_health"].append((time.perf_counter_ns() - start) / 1_000_000)
                    request(opener, base, "/api/runs", expected=401)
                    checks["unauthenticated_runs_401"] = True
                    request(opener, base, "/api/health", headers={"Origin": "https://untrusted.example"}, expected=403)
                    checks["cross_origin_health_403"] = True
                    duration, body = timed(lambda: request(opener, base, "/api/auth/session", method="POST", data=b"{}",
                                                          headers={"Origin": base, "Content-Type": "application/json"}))
                    csrf = json.loads(body)["csrfToken"]
                    if not re.fullmatch("[0-9a-f]{64}", csrf):
                        raise ValueError("invalid CSRF bootstrap response")
                    metrics["auth_bootstrap"].append(duration)
                    request(opener, base, "/api/runs/999999/start", method="POST", data=b"{}",
                            headers={"Origin": base, "Content-Type": "application/json"}, expected=403)
                    checks["mutation_without_csrf_403"] = True
                    detail = json.loads(request(opener, base, "/api/runs/1"))
                    if detail["run"]["objective"] != "SYNTHETIC V1 preservation workflow":
                        raise ValueError("benchmark connected to an unexpected project")
                    for name, path in (
                        ("runs_page_25", "/api/runs?limit=25"), ("runs_legacy_all", "/api/runs"),
                        ("run_detail", "/api/runs/1"), ("graph", "/api/graph"),
                        ("agent_performance", "/api/performance/agents"),
                    ):
                        request(opener, base, path)
                        duration, body = timed(lambda path=path: request(opener, base, path))
                        json.loads(body)
                        metrics[name].append(duration)
                        response_sizes[name] = len(body)
                    if process.poll() is not None:
                        raise RuntimeError("benchmark server exited unexpectedly")
                finally:
                    if process.poll() is None:
                        process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
        result = {
            "schema_version": 1, "reference_commit": REFERENCE,
            "measured_at": datetime.now(timezone.utc).isoformat(), "evidence_kind": "measured_local_backend",
            "host": {"platform": platform.platform(), "machine": platform.machine(), "logical_cpus": os.cpu_count(),
                     "python": platform.python_version()},
            "binary": {"version": command_output([str(binary), "--version"], ROOT), "sha256": digest(binary),
                       "build_profile": build_profile},
            "fixture_sha256": fixture_digest(), "synthetic_workflows": history,
            "fixture_hash_format": "SHA-256 of UTF-8 SQL text with LF newlines",
            "protocol": "Isolated temporary Git repository with synthetic SQLite history; sequential local HTTP, fresh process per sample, readiness HTTP timeout 50ms plus 10ms retry delay, one warmup per read, caches not flushed, no browser, no agent invocation, no remote operation.",
            "metrics": {name: summarize(values) for name, values in metrics.items()},
            "response_bytes": response_sizes, "boundary_checks": checks,
            "not_measured": ["browser_render_or_interaction", "peak_memory", "cold_filesystem_cache",
                             "ide_file_load_search_lsp_dap_extensions", "real_multi_agent_execution", "v2_session_resume"],
        }
    write_json(output, result)
    return result


def main():
    parser = argparse.ArgumentParser(description="Read-only V1 compatibility inventory and isolated local backend measurements")
    sub = parser.add_subparsers(dest="command", required=True)
    inventory_parser = sub.add_parser("inventory")
    inventory_parser.add_argument("--output", type=Path)
    inventory_parser.add_argument("--check", action="store_true")
    bench = sub.add_parser("benchmark")
    bench.add_argument("--binary", required=True, type=Path)
    bench.add_argument("--output", type=Path, default=ROOT / ".factory/v2/p00/backend-benchmark.json")
    bench.add_argument("--samples", type=int, default=9)
    bench.add_argument("--history", type=int, default=1000)
    bench.add_argument("--build-profile", choices=("debug", "release"), required=True)
    args = parser.parse_args()
    if args.command == "inventory":
        current = current_inventory()
        if args.check:
            errors = compatibility_errors(json.loads(INVENTORY.read_text(encoding="utf-8")), current)
            if errors:
                raise SystemExit("\n".join(errors))
            print("PASS: all frozen V1 routes, migrations and fixture content are preserved")
        if args.output:
            if args.output.resolve() == INVENTORY.resolve():
                raise SystemExit("the frozen inventory must not be overwritten; use a new output path")
            write_json(args.output, current)
        elif not args.check:
            print(json.dumps(current, ensure_ascii=False, indent=2))
    else:
        result = benchmark(args.binary, args.output, args.samples, args.history, args.build_profile)
        print(json.dumps({name: {k: v for k, v in metric.items() if k != "samples_ms"}
                          for name, metric in result["metrics"].items()}, indent=2))


if __name__ == "__main__":
    main()
