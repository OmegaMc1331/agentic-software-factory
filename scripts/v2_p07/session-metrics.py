import datetime
import json
import pathlib
import sqlite3

root = pathlib.Path(__file__).resolve().parents[2]
output = root / ".factory/v2/p07"
browser = json.loads((output / "browser-results.json").read_text(encoding="utf8"))
fixtures = pathlib.Path(browser["fixture"])
rows = []
for name in ["Project A", "Project B", "Controlled C"]:
    database = fixtures / name / ".factory/db.sqlite3"
    connection = sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    sessions = [dict(row) for row in connection.execute(
        "SELECT id,run_id,role,status,started_at,finished_at,duration_ms FROM agent_sessions ORDER BY id"
    )]
    for session in sessions:
        session["project"] = name
        rows.append(session)
    connection.close()

handoffs = []
for project, run_id in sorted({(row["project"], row["run_id"]) for row in rows if row["run_id"] is not None}):
    sessions = [row for row in rows if row["project"] == project and row["run_id"] == run_id]
    planners = [row for row in sessions if row["role"] == "planner" and row["finished_at"]]
    workers = [row for row in sessions if row["role"] not in ["planner", "reviewer"] and row["started_at"]]
    if planners and workers:
        parsed = lambda value: datetime.datetime.fromisoformat(value)
        handoffs.append({"project": project, "run_id": run_id, "planning_to_agent_start_ms": (
            min(parsed(row["started_at"]) for row in workers) - max(parsed(row["finished_at"]) for row in planners)
        ).total_seconds() * 1000})

report = {"schema": 1, "source_fixture": str(fixtures), "sessions": rows, "handoffs": handoffs,
          "scope": "Persisted native session timestamps/durations; configured CLI inference and process overhead included. No fabricated token or partial-output latency."}
(output / "session-metrics.json").write_text(json.dumps(report, indent=2), encoding="utf8")
print(json.dumps({"sessions": len(rows), "handoffs": handoffs}, indent=2))
