import json
import pathlib
import sqlite3

root = pathlib.Path(__file__).resolve().parents[2]
output = root / ".factory/v2/p07"
metrics = json.loads((output / "real-session-metrics.json").read_text(encoding="utf8"))
fixture = pathlib.Path(metrics["source_fixture"])
database = fixture / "Project A/.factory/db.sqlite3"
connection = sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)
connection.row_factory = sqlite3.Row

def rows(sql):
    return [dict(row) for row in connection.execute(sql)]

sessions = rows("SELECT id,run_id,task_id,role,agent,command,status,exit_code,started_at,finished_at,duration_ms,stdout FROM agent_sessions ORDER BY id")
assert len(sessions) >= 5 and all(session["status"] == "success" for session in sessions)
assert all("codex.exe" in session["command"].lower() for session in sessions)
runs = rows("SELECT id,objective,status,created_at,updated_at FROM runs ORDER BY id")
artifacts = rows("SELECT id,run_id,task_id,role,operation,kind,content,created_at FROM role_artifacts ORDER BY id")
assert runs and runs[0]["status"] == "completed" and artifacts
messages = rows("SELECT id,conversation_id,role,content,status,run_id,parent_message_id,provenance,created_at FROM conversation_messages ORDER BY id")
for message in messages:
    message["provenance"] = json.loads(message["provenance"])
report = {
    "schema": 1,
    "qualification": "Genuine configured Codex questions and non-mutating research mission; not model-generated code-write qualification.",
    "source": "Read-only extraction of retained native project SQLite records from the earlier successful browser run; not a reconstructed browser pass.",
    "source_fixture": str(fixture),
    "sessions": sessions,
    "conversations": rows("SELECT * FROM conversations ORDER BY created_at,id"),
    "messages": messages,
    "commands": rows("SELECT id,conversation_id,message_id,response_id,kind,state,run_id,error FROM conversation_commands ORDER BY id"),
    "runs": runs,
    "tasks": rows("SELECT id,run_id,title,state,role,operation FROM tasks ORDER BY id"),
    "artifacts": artifacts,
    "limits": ["Raw stderr is omitted; no provider tokens or environment values are exported.", "This extraction does not turn the final provider-limited browser refresh into a pass.", "Code-writing acceptance remains unqualified."],
}
connection.close()
(output / "real-agent-evidence.json").write_text(json.dumps(report, indent=2), encoding="utf8")
print(json.dumps({"sessions": len(sessions), "messages": len(messages), "completed_missions": len(runs), "artifacts": len(artifacts)}))
