pub mod error;

pub use error::DbError;

use chrono::Utc;
use factory_types::{AgentSession, Run, RunStatus, Task, TaskState};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub type Result<T> = std::result::Result<T, DbError>;

pub struct FactoryDb {
    conn: Connection,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

impl FactoryDb {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        Ok(FactoryDb { conn })
    }

    pub fn create_run(&self, objective: &str, planner_agent: Option<&str>) -> Result<Run> {
        let ts = now();
        self.conn.execute(
            "INSERT INTO runs (objective, status, planner_agent, created_at, updated_at)
             VALUES (?1, 'planned', ?2, ?3, ?4)",
            params![objective, planner_agent, ts, ts],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_run(id)?.ok_or(DbError::NotFound("run"))
    }

    pub fn get_run(&self, id: i64) -> Result<Option<Run>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, objective, status, planner_agent, created_at, updated_at
                 FROM runs WHERE id = ?1",
                params![id],
                build_run,
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_runs(&self) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, objective, status, planner_agent, created_at, updated_at
             FROM runs ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([], build_run)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn set_run_status(&self, id: i64, status: RunStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.as_str(), now(), id],
        )?;
        Ok(())
    }

    pub fn create_task(
        &self,
        run_id: i64,
        title: &str,
        objective: &str,
        acceptance_criteria: &[String],
        state: TaskState,
        position: i32,
    ) -> Result<i64> {
        let ts = now();
        let criteria = serde_json::to_string(acceptance_criteria)?;
        self.conn.execute(
            "INSERT INTO tasks (run_id, title, objective, acceptance_criteria, state, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![run_id, title, objective, criteria, state.as_str(), position, ts, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn add_dependency(&self, task_id: i64, depends_on: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on) VALUES (?1, ?2)",
            params![task_id, depends_on],
        )?;
        Ok(())
    }

    pub fn get_task(&self, id: i64) -> Result<Option<Task>> {
        let row = self
            .conn
            .query_row(
                "SELECT t.id, t.run_id, t.title, t.objective, t.acceptance_criteria, t.state, t.position, t.worktree_path, t.created_at, t.updated_at
                 FROM tasks t WHERE t.id = ?1",
                params![id],
                build_task,
            )
            .optional()?;
        if let Some(mut task) = row {
            task.dependencies = self.dependencies_of(task.id)?;
            return Ok(Some(task));
        }
        Ok(None)
    }

    pub fn list_tasks(&self, run_id: i64) -> Result<Vec<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.run_id, t.title, t.objective, t.acceptance_criteria, t.state, t.position, t.worktree_path, t.created_at, t.updated_at
             FROM tasks t WHERE t.run_id = ?1 ORDER BY t.position",
        )?;
        let mut tasks = Vec::new();
        let rows = stmt.query_map(params![run_id], build_task)?;
        for row in rows {
            let mut task = row?;
            task.dependencies = self.dependencies_of(task.id)?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    pub fn dependencies_of(&self, task_id: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT depends_on FROM task_dependencies WHERE task_id = ?1 ORDER BY depends_on",
        )?;
        let ids = stmt
            .query_map(params![task_id], |r| r.get(0))?
            .collect::<std::result::Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    pub fn set_task_state(&self, id: i64, state: TaskState) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![state.as_str(), now(), id],
        )?;
        let run_id: Option<i64> = self
            .conn
            .query_row("SELECT run_id FROM tasks WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(run_id) = run_id {
            self.reconcile_run(run_id)?;
        }
        Ok(())
    }

    pub fn reconcile_run(&self, run_id: i64) -> Result<()> {
        let tasks = self.list_tasks(run_id)?;
        let status = RunStatus::from_tasks(&tasks);
        let current = self.get_run(run_id)?.map(|run| run.status);
        if current != Some(status) {
            self.set_run_status(run_id, status)?;
        }
        Ok(())
    }

    pub fn set_worktree_path(&self, id: i64, path: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET worktree_path = ?1, updated_at = ?2 WHERE id = ?3",
            params![path, now(), id],
        )?;
        Ok(())
    }

    pub fn insert_agent_session(&self, session: &AgentSession) -> Result<AgentSession> {
        let mut session = session.clone();
        self.conn.execute(
            "INSERT INTO agent_sessions (run_id, task_id, role, agent, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                session.run_id,
                session.task_id,
                session.role,
                session.agent,
                session.command,
                session.status,
                session.started_at,
                session.finished_at,
                session.exit_code,
                session.duration_ms.map(|d| d as i64),
                session.stdout,
                session.stderr
            ],
        )?;
        session.id = self.conn.last_insert_rowid();
        Ok(session)
    }

    pub fn list_agent_sessions(&self, run_id: Option<i64>) -> Result<Vec<AgentSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, task_id, role, agent, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr
             FROM agent_sessions
             WHERE (?1 IS NULL OR run_id = ?1)
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![run_id], build_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn build_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    Ok(Run {
        id: r.get(0)?,
        objective: r.get(1)?,
        status: run_status(r.get::<_, String>(2)?),
        planner_agent: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

fn build_task(r: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let criteria_json: String = r.get(4)?;
    let criteria = serde_json::from_str(&criteria_json).unwrap_or_default();
    Ok(Task {
        id: r.get(0)?,
        run_id: r.get(1)?,
        title: r.get(2)?,
        objective: r.get(3)?,
        acceptance_criteria: criteria,
        state: task_state(r.get::<_, String>(5)?),
        position: r.get(6)?,
        dependencies: Vec::new(),
        worktree_path: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

fn build_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
    Ok(AgentSession {
        id: r.get(0)?,
        run_id: r.get(1)?,
        task_id: r.get(2)?,
        role: r.get(3)?,
        agent: r.get(4)?,
        command: r.get(5)?,
        status: r.get(6)?,
        started_at: r.get(7)?,
        finished_at: r.get(8)?,
        exit_code: r.get(9)?,
        duration_ms: r.get(10).map(|v: Option<i64>| v.map(|v| v as u64))?,
        stdout: r.get(11)?,
        stderr: r.get(12)?,
    })
}

fn task_state(s: String) -> TaskState {
    s.parse().unwrap_or(TaskState::Pending)
}

fn run_status(s: String) -> RunStatus {
    match s.as_str() {
        "active" => RunStatus::Active,
        "completed" => RunStatus::Completed,
        "failed" => RunStatus::Failed,
        _ => RunStatus::Planned,
    }
}

const V1_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    objective TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'planned',
    planner_agent TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    objective TEXT NOT NULL,
    acceptance_criteria TEXT NOT NULL DEFAULT '[]',
    state TEXT NOT NULL DEFAULT 'pending',
    position INTEGER NOT NULL,
    worktree_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS task_dependencies (
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, depends_on)
);
CREATE TABLE IF NOT EXISTS agent_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    agent TEXT NOT NULL,
    command TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    exit_code INTEGER,
    duration_ms INTEGER,
    stdout TEXT,
    stderr TEXT
);
CREATE INDEX IF NOT EXISTS idx_tasks_run ON tasks(run_id);
CREATE INDEX IF NOT EXISTS idx_task_deps_dep ON task_dependencies(depends_on);
CREATE INDEX IF NOT EXISTS idx_sessions_run ON agent_sessions(run_id);
";

const V2_SCHEMA: &str = "CREATE INDEX IF NOT EXISTS idx_tasks_run_state ON tasks(run_id, state);";

const MIGRATIONS: &[&str] = &[V1_SCHEMA, V2_SCHEMA];

fn migrate(conn: &mut Connection) -> Result<()> {
    migrate_schemas(conn, MIGRATIONS)
}

fn migrate_schemas(conn: &mut Connection, schemas: &[&str]) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );",
    )?;
    let applied: Vec<i64> = conn
        .prepare("SELECT version FROM schema_migrations ORDER BY version")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (index, schema) in schemas.iter().enumerate() {
        let version = index as i64 + 1;
        if applied.contains(&version) {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(schema)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            params![version, now()],
        )?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use factory_types::{AgentSession, RunStatus, TaskState};
    use rusqlite::Connection;
    use tempfile::TempDir;

    use crate::{FactoryDb, V1_SCHEMA};

    #[test]
    fn applies_all_migrations_exactly_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let db = FactoryDb::open(&path).unwrap();
        let versions = schema_versions(&path);
        assert_eq!(versions, vec![1, 2]);
        db.create_run("objective", Some("codex")).unwrap();
        drop(db);

        let db = FactoryDb::open(&path).unwrap();
        assert_eq!(schema_versions(&path), vec![1, 2]);
        db.list_runs().unwrap();
    }

    #[test]
    fn older_schema_migrates_forward() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("old.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO runs (objective, status, planner_agent, created_at, updated_at)
             VALUES ('legacy', 'planned', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (run_id, title, objective, acceptance_criteria, state, position, created_at, updated_at)
             VALUES (1, 'old task', 'old', '[]', 'completed', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        let db = FactoryDb::open(&path).unwrap();
        assert_eq!(schema_versions(&path), vec![1, 2]);
        let run = db.get_run(1).unwrap().unwrap();
        assert_eq!(run.objective, "legacy");
        let tasks = db.list_tasks(1).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "old task");
    }

    #[test]
    fn failed_migration_rolls_back_without_recording() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::migrate_schemas(&mut conn, &[crate::V1_SCHEMA]).unwrap();
        let result = crate::migrate_schemas(
            &mut conn,
            &[crate::V1_SCHEMA, "INSERT INTO missing_table VALUES (1)"],
        );
        assert!(result.is_err());
        let versions = schema_versions_conn(&conn);
        assert_eq!(versions, vec![1]);
    }

    #[test]
    fn run_status_follows_task_state() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        let a = db
            .create_task(run.id, "A", "a", &[], TaskState::Ready, 0)
            .unwrap();
        let b = db
            .create_task(run.id, "B", "b", &[], TaskState::Ready, 1)
            .unwrap();
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Planned
        );

        db.set_task_state(a, TaskState::Running).unwrap();
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Active
        );

        db.set_task_state(a, TaskState::Failed).unwrap();
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Failed
        );

        db.set_task_state(a, TaskState::Ready).unwrap();
        db.set_task_state(a, TaskState::Running).unwrap();
        db.set_task_state(a, TaskState::Completed).unwrap();
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Active
        );

        db.set_task_state(b, TaskState::Running).unwrap();
        db.set_task_state(b, TaskState::Completed).unwrap();
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Completed
        );
    }

    fn schema_versions(path: &std::path::Path) -> Vec<i64> {
        let conn = Connection::open(path).unwrap();
        schema_versions_conn(&conn)
    }

    fn schema_versions_conn(conn: &Connection) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT version FROM schema_migrations ORDER BY version")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn persists_run_tasks_and_dependencies() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();

        let run = db.create_run("build a thing", Some("codex")).unwrap();
        assert_eq!(run.id, 1);
        assert_eq!(run.status.as_str(), "planned");
        assert_eq!(run.planner_agent.as_deref(), Some("codex"));

        let a = db
            .create_task(
                run.id,
                "Task A",
                "do A",
                &["a works".into()],
                TaskState::Ready,
                0,
            )
            .unwrap();
        let b = db
            .create_task(
                run.id,
                "Task B",
                "do B",
                &["b works".into()],
                TaskState::Pending,
                1,
            )
            .unwrap();
        db.add_dependency(b, a).unwrap();

        let tasks = db.list_tasks(run.id).unwrap();
        assert_eq!(tasks.len(), 2);
        let b_loaded = tasks.iter().find(|t| t.id == b).unwrap();
        assert_eq!(b_loaded.dependencies, vec![a]);
        assert_eq!(b_loaded.state, TaskState::Pending);
        assert!(b_loaded.created_at.starts_with("202"));

        let a_loaded = db.get_task(a).unwrap().unwrap();
        assert_eq!(a_loaded.acceptance_criteria, vec!["a works".to_string()]);
    }

    #[test]
    fn updates_task_state_and_worktree_path() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        let task = db
            .create_task(run.id, "T", "objective", &[], TaskState::Ready, 0)
            .unwrap();

        db.set_task_state(task, TaskState::Running).unwrap();
        let loaded = db.get_task(task).unwrap().unwrap();
        assert_eq!(loaded.state, TaskState::Running);

        db.set_worktree_path(task, Some("C:\\worktrees\\t1"))
            .unwrap();
        let loaded = db.get_task(task).unwrap().unwrap();
        assert_eq!(loaded.worktree_path.as_deref(), Some("C:\\worktrees\\t1"));

        db.set_worktree_path(task, None).unwrap();
        let loaded = db.get_task(task).unwrap().unwrap();
        assert!(loaded.worktree_path.is_none());
    }

    #[test]
    fn persists_and_reads_agent_sessions() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();

        let session = AgentSession {
            id: 0,
            run_id: Some(run.id),
            task_id: None,
            role: "planner".to_string(),
            agent: "codex".to_string(),
            command: "codex exec".to_string(),
            status: "success".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            finished_at: Some("2026-01-01T00:00:01Z".to_string()),
            exit_code: Some(0),
            duration_ms: Some(1200),
            stdout: Some("{\"objective\":\"x\"}".to_string()),
            stderr: Some(String::new()),
        };
        let saved = db.insert_agent_session(&session).unwrap();
        assert!(saved.id > 0);

        let sessions = db.list_agent_sessions(Some(run.id)).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, "codex");
        assert_eq!(sessions[0].role, "planner");
        assert_eq!(sessions[0].exit_code, Some(0));
        assert!(sessions[0].stdout.as_deref().unwrap().contains("objective"));

        assert!(!db.list_agent_sessions(None).unwrap().is_empty());
    }
}
