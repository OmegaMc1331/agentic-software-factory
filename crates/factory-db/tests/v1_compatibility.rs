use std::path::Path;

use factory_db::FactoryDb;
use factory_types::{AgentSessionMode, DeliveryState, RunStatus, TaskOperation, TaskState};
use rusqlite::{types::Value, Connection};
use tempfile::TempDir;

const FIXTURE: &str = include_str!("fixtures/asf-v1-1.0.5.sql");

#[derive(Debug, PartialEq)]
struct TableSnapshot {
    name: String,
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn rows(conn: &Connection, table: &str, columns: &[String]) -> Vec<Vec<Value>> {
    let projection = columns
        .iter()
        .map(|c| quote(c))
        .collect::<Vec<_>>()
        .join(",");
    let restriction = if table == "schema_migrations" {
        " WHERE version <= 18"
    } else {
        ""
    };
    conn.prepare(&format!(
        "SELECT {projection} FROM {}{restriction} ORDER BY rowid",
        quote(table)
    ))
    .unwrap()
    .query_map([], |row| {
        (0..columns.len()).map(|index| row.get(index)).collect()
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap()
}

fn snapshot(path: &Path) -> Vec<TableSnapshot> {
    let conn = Connection::open(path).unwrap();
    let names = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let columns = conn
                .prepare(&format!("PRAGMA table_info({})", quote(&name)))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            TableSnapshot {
                rows: rows(&conn, &name, &columns),
                name,
                columns,
            }
        })
        .collect()
}

fn restore(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute_batch(FIXTURE).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(!conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
}

#[test]
fn opening_frozen_v1_preserves_every_original_row_and_column() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("v1.sqlite3");
    restore(&path);
    let original = snapshot(&path);
    assert_eq!(original.len(), 16);
    assert!(original.iter().all(|table| !table.rows.is_empty()));

    for _ in 0..2 {
        let db = FactoryDb::open(&path).unwrap();
        assert_eq!(db.list_runs().unwrap().len(), 2);
        drop(db);
        let conn = Connection::open(&path).unwrap();
        for table in &original {
            assert_eq!(
                rows(&conn, &table.name, &table.columns),
                table.rows,
                "{} changed",
                table.name
            );
        }
        assert!(!conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
    }
}

#[test]
fn frozen_v1_history_is_readable_through_current_typed_contracts() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("v1.sqlite3");
    restore(&path);
    let db = FactoryDb::open(&path).unwrap();
    let run = db.get_run(1).unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.objective, "SYNTHETIC V1 preservation workflow");
    assert_eq!(run.team.unwrap().planner, "fixture-agent");
    assert_eq!(run.recipe.unwrap().id, "fixture-recipe");
    let tasks = db.list_tasks(1).unwrap();
    assert_eq!(tasks[1].dependencies, [1]);
    assert_eq!(tasks[1].operation, Some(TaskOperation::Implement));
    assert_eq!(tasks[1].agent_override.as_deref(), Some("fixture-agent"));
    assert_eq!(db.list_tasks(2).unwrap()[0].state, TaskState::Ready);
    let attempts = db.list_task_attempts(1).unwrap();
    assert_eq!(attempts.len(), 2);
    assert!(attempts[1].evidence.as_ref().unwrap().diff_patch.is_some());
    assert!(attempts[1].review.is_some());
    let sessions = db.list_agent_sessions(None).unwrap();
    assert_eq!(sessions.len(), 5);
    assert!(sessions
        .iter()
        .any(|s| s.mode == AgentSessionMode::Interactive));
    assert!(sessions
        .iter()
        .any(|s| s.policy_audit.is_some() && s.output_meta.is_some()));
    assert_eq!(db.list_role_artifacts(1).unwrap()[0].kind, "architecture");
    assert_eq!(db.list_plan_revisions(1).unwrap()[0].snapshot.tasks, tasks);
    assert_eq!(
        db.list_routing_decisions_for_task(2).unwrap()[0].selected_agent,
        "fixture-agent"
    );
    assert_eq!(db.list_integration_outcomes().unwrap().len(), 1);
    assert_eq!(db.get_run_github_link(1).unwrap().unwrap().issue_number, 42);
    assert_eq!(
        db.list_mcp_tools(None).unwrap()[0].qualified_name,
        "fixture.inspect"
    );
    assert!(db
        .get_tool_grant_by_hash(&"0".repeat(64))
        .unwrap()
        .unwrap()
        .revoked_at
        .is_some());
    assert_eq!(db.list_tool_invocations().unwrap().len(), 1);
}

#[test]
fn reopening_v1_does_not_clear_an_uncertain_delivery_or_its_operation_identity() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("v1.sqlite3");
    restore(&path);
    let first = FactoryDb::open(&path)
        .unwrap()
        .get_delivery(1)
        .unwrap()
        .unwrap();
    assert_eq!(first.state, DeliveryState::Interrupted);
    assert!(first.recovery_required);
    assert_eq!(
        first.operation_id.as_deref(),
        Some("synthetic-v1-operation")
    );
    let reopened = FactoryDb::open(&path)
        .unwrap()
        .get_or_create_delivery(1)
        .unwrap();
    assert_eq!(reopened, first);
}
