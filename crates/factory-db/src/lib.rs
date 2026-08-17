pub mod error;

pub use error::DbError;

use chrono::Utc;
use factory_types::{
    AgentSession, AgentSessionMode, AttemptStatus, Plan, ReviewResult, RoleArtifact, Run,
    RunStatus, Task, TaskAttempt, TaskEvidence, TaskOperation, TaskState,
};
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
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&mut conn)?;
        Ok(FactoryDb { conn })
    }

    pub fn create_run(&self, objective: &str, planner_agent: Option<&str>) -> Result<Run> {
        self.create_run_with_status(objective, planner_agent, RunStatus::Planned)
    }

    pub fn create_run_with_status(
        &self,
        objective: &str,
        planner_agent: Option<&str>,
        status: RunStatus,
    ) -> Result<Run> {
        let ts = now();
        self.conn.execute(
            "INSERT INTO runs (objective, status, planner_agent, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![objective, status.as_str(), planner_agent, ts, ts],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_run(id)?.ok_or(DbError::NotFound("run"))
    }

    pub fn get_run(&self, id: i64) -> Result<Option<Run>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, objective, status, planner_agent, created_at, updated_at, team
                 FROM runs WHERE id = ?1",
                params![id],
                build_run,
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_runs(&self) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, objective, status, planner_agent, created_at, updated_at, team
             FROM runs ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([], build_run)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn set_run_status(&self, id: i64, status: RunStatus) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.as_str(), now(), id],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("run"));
        }
        Ok(())
    }

    pub fn set_run_team(&self, id: i64, team: &factory_types::WorkflowTeam) -> Result<()> {
        let team = serde_json::to_string(team)?;
        let changed = self.conn.execute(
            "UPDATE runs SET team = ?1, updated_at = ?2 WHERE id = ?3",
            params![team, now(), id],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("run"));
        }
        Ok(())
    }

    pub fn set_run_planner_agent(&self, id: i64, planner_agent: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE runs SET planner_agent = ?1, updated_at = ?2 WHERE id = ?3",
            params![planner_agent, now(), id],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("run"));
        }
        Ok(())
    }

    pub fn persist_plan(&self, run_id: i64, plan: &Plan) -> Result<Vec<Task>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = std::collections::HashMap::new();
        let ts = now();
        for (position, task) in plan.tasks.iter().enumerate() {
            let criteria = serde_json::to_string(&task.acceptance_criteria)?;
            tx.execute(
                "INSERT INTO tasks (run_id, title, objective, acceptance_criteria, state, position, role, operation, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?7, ?8, ?9)",
                params![
                    run_id,
                    task.title,
                    task.objective,
                    criteria,
                    position as i32,
                    task.role,
                    task.operation.map(TaskOperation::as_str),
                    ts,
                    ts
                ],
            )?;
            ids.insert(task.id.clone(), tx.last_insert_rowid());
        }
        for task in &plan.tasks {
            let task_id = ids[&task.id];
            for dependency in &task.dependencies {
                tx.execute(
                    "INSERT INTO task_dependencies (task_id, depends_on) VALUES (?1, ?2)",
                    params![task_id, ids[dependency]],
                )?;
            }
            let state = if task.dependencies.is_empty() {
                TaskState::Ready
            } else {
                TaskState::Pending
            };
            tx.execute(
                "UPDATE tasks SET state = ?1, updated_at = ?2 WHERE id = ?3",
                params![state.as_str(), ts, task_id],
            )?;
        }
        tx.execute(
            "UPDATE runs SET objective = ?1, status = 'planned', updated_at = ?2 WHERE id = ?3",
            params![plan.objective, ts, run_id],
        )?;
        tx.commit()?;
        self.list_tasks(run_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_task(
        &self,
        run_id: i64,
        title: &str,
        objective: &str,
        acceptance_criteria: &[String],
        state: TaskState,
        position: i32,
        role: Option<&str>,
        operation: Option<TaskOperation>,
    ) -> Result<i64> {
        let ts = now();
        let criteria = serde_json::to_string(acceptance_criteria)?;
        self.conn.execute(
            "INSERT INTO tasks (run_id, title, objective, acceptance_criteria, state, position, role, operation, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                run_id,
                title,
                objective,
                criteria,
                state.as_str(),
                position,
                role,
                operation.map(TaskOperation::as_str),
                ts,
                ts
            ],
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
                "SELECT t.id, t.run_id, t.title, t.objective, t.acceptance_criteria, t.state, t.position, t.worktree_path, t.created_at, t.updated_at, t.role, t.operation
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
            "SELECT t.id, t.run_id, t.title, t.objective, t.acceptance_criteria, t.state, t.position, t.worktree_path, t.created_at, t.updated_at, t.role, t.operation
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
        if matches!(current, Some(RunStatus::Planning | RunStatus::Cancelled)) {
            return Ok(());
        }
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
            "INSERT INTO agent_sessions (run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                session.run_id,
                session.task_id,
                session.attempt_id,
                session.role,
                session.operation.map(TaskOperation::as_str),
                session.agent,
                session.mode.as_str(),
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
            "SELECT id, run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr
             FROM agent_sessions
             WHERE (?1 IS NULL OR run_id = ?1)
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![run_id], build_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_agent_session(&self, id: i64) -> Result<Option<AgentSession>> {
        self.conn
            .query_row(
                "SELECT id, run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr
                 FROM agent_sessions WHERE id = ?1",
                params![id],
                build_session,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_agent_sessions_for_agent(
        &self,
        agent: &str,
        limit: usize,
    ) -> Result<Vec<AgentSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr
             FROM agent_sessions
             WHERE agent = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![agent, limit as i64], build_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn append_agent_session_output(
        &self,
        id: i64,
        stdout: Option<&str>,
        stderr: Option<&str>,
    ) -> Result<()> {
        const MAX_LOG_CHARS: i64 = 1_000_000;
        self.conn.execute(
            "UPDATE agent_sessions
             SET stdout = substr(COALESCE(stdout, '') || COALESCE(?1, ''), -?2),
                 stderr = substr(COALESCE(stderr, '') || COALESCE(?3, ''), -?2)
             WHERE id = ?4",
            params![stdout, MAX_LOG_CHARS, stderr, id],
        )?;
        Ok(())
    }

    pub fn set_agent_session_output(
        &self,
        id: i64,
        stdout: Option<&str>,
        stderr: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_sessions SET stdout = ?1, stderr = ?2 WHERE id = ?3",
            params![stdout, stderr, id],
        )?;
        Ok(())
    }

    pub fn finish_agent_session(
        &self,
        id: i64,
        status: &str,
        exit_code: Option<i32>,
        duration_ms: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_sessions
             SET status = ?1, finished_at = ?2, exit_code = ?3, duration_ms = ?4
             WHERE id = ?5",
            params![status, now(), exit_code, duration_ms as i64, id],
        )?;
        Ok(())
    }

    pub fn set_agent_session_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_sessions SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    pub fn create_task_attempt(
        &self,
        task_id: i64,
        role: &str,
        operation: Option<TaskOperation>,
        agent: &str,
        worktree_path: &str,
    ) -> Result<TaskAttempt> {
        let attempt_number: u32 = self.conn.query_row(
            "SELECT COALESCE(MAX(attempt_number), 0) + 1 FROM task_attempts WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO task_attempts
             (task_id, attempt_number, agent, role, operation, status, started_at, worktree_path)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7)",
            params![
                task_id,
                attempt_number,
                agent,
                role,
                operation.map(TaskOperation::as_str),
                now(),
                worktree_path
            ],
        )?;
        self.get_task_attempt(self.conn.last_insert_rowid())?
            .ok_or(DbError::NotFound("task attempt"))
    }

    pub fn count_task_attempts(&self, run_id: i64) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM task_attempts a JOIN tasks t ON t.id = a.task_id
             WHERE t.run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn get_task_attempt(&self, id: i64) -> Result<Option<TaskAttempt>> {
        self.conn
            .query_row(
                "SELECT id, task_id, attempt_number, agent, status, started_at, finished_at,
                        worktree_path, commit_sha, exit_code, error, evidence, review, role, operation
                 FROM task_attempts WHERE id = ?1",
                params![id],
                build_attempt,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_task_attempts(&self, run_id: i64) -> Result<Vec<TaskAttempt>> {
        let mut statement = self.conn.prepare(
            "SELECT a.id, a.task_id, a.attempt_number, a.agent, a.status, a.started_at,
                    a.finished_at, a.worktree_path, a.commit_sha, a.exit_code, a.error,
                    a.evidence, a.review, a.role, a.operation
             FROM task_attempts a
             JOIN tasks t ON t.id = a.task_id
             WHERE t.run_id = ?1
             ORDER BY a.id",
        )?;
        let attempts = statement
            .query_map(params![run_id], build_attempt)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(attempts)
    }

    pub fn latest_task_attempt(&self, task_id: i64) -> Result<Option<TaskAttempt>> {
        self.conn
            .query_row(
                "SELECT id, task_id, attempt_number, agent, status, started_at, finished_at,
                        worktree_path, commit_sha, exit_code, error, evidence, review, role, operation
                 FROM task_attempts WHERE task_id = ?1 ORDER BY attempt_number DESC LIMIT 1",
                params![task_id],
                build_attempt,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_task_attempt_status(&self, id: i64, status: AttemptStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE task_attempts SET status = ?1 WHERE id = ?2",
            params![status.as_str(), id],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_task_attempt(
        &self,
        id: i64,
        status: AttemptStatus,
        exit_code: Option<i32>,
        commit_sha: Option<&str>,
        error: Option<&str>,
        evidence: Option<&TaskEvidence>,
        review: Option<&ReviewResult>,
    ) -> Result<()> {
        let evidence = evidence.map(serde_json::to_string).transpose()?;
        let review = review.map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "UPDATE task_attempts
             SET status = ?1, finished_at = ?2, exit_code = ?3, commit_sha = ?4,
                 error = ?5, evidence = ?6, review = ?7
             WHERE id = ?8",
            params![
                status.as_str(),
                now(),
                exit_code,
                commit_sha,
                error,
                evidence,
                review,
                id
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_role_artifact(
        &self,
        run_id: i64,
        task_id: Option<i64>,
        attempt_id: Option<i64>,
        role: &str,
        operation: Option<TaskOperation>,
        kind: &str,
        content: &str,
    ) -> Result<RoleArtifact> {
        self.conn.execute(
            "INSERT INTO role_artifacts (run_id, task_id, attempt_id, role, operation, kind, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                run_id,
                task_id,
                attempt_id,
                role,
                operation.map(TaskOperation::as_str),
                kind,
                content,
                now()
            ],
        )?;
        self.get_role_artifact(self.conn.last_insert_rowid())?
            .ok_or(DbError::NotFound("role artifact"))
    }

    pub fn get_role_artifact(&self, id: i64) -> Result<Option<RoleArtifact>> {
        self.conn
            .query_row(
                "SELECT id, run_id, task_id, attempt_id, role, operation, kind, content, created_at
                 FROM role_artifacts WHERE id = ?1",
                params![id],
                build_artifact,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_role_artifacts(&self, run_id: i64) -> Result<Vec<RoleArtifact>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, task_id, attempt_id, role, operation, kind, content, created_at
             FROM role_artifacts WHERE run_id = ?1 ORDER BY id",
        )?;
        let artifacts = stmt
            .query_map(params![run_id], build_artifact)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(artifacts)
    }

    pub fn list_artifacts_for_task(&self, task_id: i64) -> Result<Vec<RoleArtifact>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, task_id, attempt_id, role, operation, kind, content, created_at
             FROM role_artifacts WHERE task_id = ?1 ORDER BY id",
        )?;
        let artifacts = stmt
            .query_map(params![task_id], build_artifact)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(artifacts)
    }

    /// Artifacts produced by the given task ids in a run, in task order. The
    /// Factory uses this for dependency-aware context propagation: only
    /// artifacts from the caller's dependency ancestry reach a mission.
    pub fn list_artifacts_for_tasks(&self, task_ids: &[i64]) -> Result<Vec<RoleArtifact>> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; task_ids.len()].join(",");
        let sql = format!(
            "SELECT id, run_id, task_id, attempt_id, role, operation, kind, content, created_at
             FROM role_artifacts WHERE task_id IN ({placeholders}) ORDER BY id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params = rusqlite::params_from_iter(task_ids.iter().copied());
        let artifacts = stmt
            .query_map(params, build_artifact)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(artifacts)
    }

    pub fn reconcile_interrupted(&self) -> Result<Reconciliation> {
        let timestamp = now();
        let sessions = self.conn.execute(
            "UPDATE agent_sessions
             SET status = 'interrupted', finished_at = ?1
             WHERE status IN ('running', 'active')",
            params![timestamp],
        )?;
        let attempts = self.conn.execute(
            "UPDATE task_attempts
             SET status = 'interrupted', finished_at = ?1,
                 error = COALESCE(error, 'Factory stopped while this attempt was running.')
             WHERE status IN ('running', 'reviewing')",
            params![timestamp],
        )?;
        let tasks = self.conn.execute(
            "UPDATE tasks SET state = 'failed', updated_at = ?1 WHERE state = 'running'",
            params![timestamp],
        )?;
        let runs = self.conn.execute(
            "UPDATE runs SET status = 'failed', updated_at = ?1
             WHERE status IN ('planning', 'active')",
            params![timestamp],
        )?;
        Ok(Reconciliation {
            sessions,
            attempts,
            tasks,
            runs,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reconciliation {
    pub sessions: usize,
    pub attempts: usize,
    pub tasks: usize,
    pub runs: usize,
}

fn build_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let team: Option<String> = r.get(6)?;
    Ok(Run {
        id: r.get(0)?,
        objective: r.get(1)?,
        status: run_status(r.get::<_, String>(2)?),
        planner_agent: r.get(3)?,
        team: team.and_then(|value| serde_json::from_str(&value).ok()),
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
        role: r.get(10)?,
        operation: r
            .get::<_, Option<String>>(11)?
            .and_then(|value| value.parse().ok()),
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

fn build_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
    Ok(AgentSession {
        id: r.get(0)?,
        run_id: r.get(1)?,
        task_id: r.get(2)?,
        attempt_id: r.get(3)?,
        role: r.get(4)?,
        operation: r
            .get::<_, Option<String>>(5)?
            .and_then(|value| value.parse().ok()),
        agent: r.get(6)?,
        mode: r
            .get::<_, String>(7)?
            .parse()
            .unwrap_or(AgentSessionMode::Automated),
        command: r.get(8)?,
        status: r.get(9)?,
        started_at: r.get(10)?,
        finished_at: r.get(11)?,
        exit_code: r.get(12)?,
        duration_ms: r.get(13).map(|v: Option<i64>| v.map(|v| v as u64))?,
        stdout: r.get(14)?,
        stderr: r.get(15)?,
    })
}

fn build_attempt(r: &rusqlite::Row<'_>) -> rusqlite::Result<TaskAttempt> {
    let evidence: Option<String> = r.get(11)?;
    let review: Option<String> = r.get(12)?;
    Ok(TaskAttempt {
        id: r.get(0)?,
        task_id: r.get(1)?,
        attempt_number: r.get(2)?,
        agent: r.get(3)?,
        role: r.get(13)?,
        operation: r
            .get::<_, Option<String>>(14)?
            .and_then(|value| value.parse().ok()),
        status: r
            .get::<_, String>(4)?
            .parse()
            .unwrap_or(AttemptStatus::Interrupted),
        started_at: r.get(5)?,
        finished_at: r.get(6)?,
        worktree_path: r.get(7)?,
        commit_sha: r.get(8)?,
        exit_code: r.get(9)?,
        error: r.get(10)?,
        evidence: evidence.and_then(|value| serde_json::from_str(&value).ok()),
        review: review.and_then(|value| serde_json::from_str(&value).ok()),
    })
}

fn build_artifact(r: &rusqlite::Row<'_>) -> rusqlite::Result<RoleArtifact> {
    Ok(RoleArtifact {
        id: r.get(0)?,
        run_id: r.get(1)?,
        task_id: r.get(2)?,
        attempt_id: r.get(3)?,
        role: r.get(4)?,
        operation: r
            .get::<_, Option<String>>(5)?
            .and_then(|value| value.parse().ok()),
        kind: r.get(6)?,
        content: r.get(7)?,
        created_at: r.get(8)?,
    })
}

fn task_state(s: String) -> TaskState {
    s.parse().unwrap_or(TaskState::Pending)
}

fn run_status(s: String) -> RunStatus {
    s.parse().unwrap_or(RunStatus::Planned)
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

const V3_SCHEMA: &str = "
CREATE TABLE task_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    agent TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    worktree_path TEXT NOT NULL,
    commit_sha TEXT,
    exit_code INTEGER,
    error TEXT,
    evidence TEXT,
    review TEXT,
    UNIQUE(task_id, attempt_number)
);
ALTER TABLE agent_sessions ADD COLUMN attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL;
CREATE INDEX idx_attempts_task ON task_attempts(task_id, attempt_number);
CREATE INDEX idx_sessions_attempt ON agent_sessions(attempt_id);
CREATE INDEX idx_sessions_status ON agent_sessions(status);
";

const V4_SCHEMA: &str = "
ALTER TABLE agent_sessions ADD COLUMN mode TEXT NOT NULL DEFAULT 'automated';
CREATE INDEX idx_sessions_mode_status ON agent_sessions(mode, status);
";

const V5_SCHEMA: &str = "
ALTER TABLE tasks ADD COLUMN role TEXT;
ALTER TABLE task_attempts ADD COLUMN role TEXT;
ALTER TABLE runs ADD COLUMN team TEXT;
";

/// Role-aware workflow semantics: tasks carry a semantic operation, sessions
/// and attempts persist the operation they performed, and advisory/verification/
/// review outputs are stored as `role_artifacts` so downstream tasks can
/// consume them.
///
/// Rows persisted before this migration have no operation. Known core roles
/// are backfilled with a compatible default; unknown roles default to
/// `implement`. Factory Core still derives defaults at runtime for custom
/// roles, so no data must be deleted when upgrading.
const V6_SCHEMA: &str = "
ALTER TABLE tasks ADD COLUMN operation TEXT;
ALTER TABLE task_attempts ADD COLUMN operation TEXT;
ALTER TABLE agent_sessions ADD COLUMN operation TEXT;
CREATE TABLE role_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL,
    role TEXT NOT NULL,
    operation TEXT,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_artifacts_run_task ON role_artifacts(run_id, task_id);
CREATE INDEX idx_artifacts_task ON role_artifacts(task_id);
UPDATE tasks SET operation = CASE
    WHEN role IS NULL THEN 'implement'
    WHEN role = 'worker' THEN 'implement'
    WHEN role = 'reviewer' THEN 'review'
    WHEN role = 'test_engineer' THEN 'verify'
    WHEN role IN ('architect', 'researcher') THEN 'advisory'
    WHEN role = 'security_auditor' THEN 'review'
    WHEN role = 'documentation_writer' THEN 'post_process'
    ELSE 'implement'
END WHERE operation IS NULL;
UPDATE task_attempts SET operation = CASE
    WHEN role IS NULL THEN 'implement'
    WHEN role = 'worker' THEN 'implement'
    WHEN role = 'reviewer' THEN 'review'
    WHEN role = 'test_engineer' THEN 'verify'
    WHEN role IN ('architect', 'researcher') THEN 'advisory'
    WHEN role = 'security_auditor' THEN 'review'
    WHEN role = 'documentation_writer' THEN 'post_process'
    ELSE 'implement'
END WHERE operation IS NULL;
UPDATE agent_sessions SET operation = CASE
    WHEN role = 'planner' THEN 'planning'
    WHEN role = 'worker' THEN 'implement'
    WHEN role = 'reviewer' THEN 'review'
    WHEN role = 'test_engineer' THEN 'verify'
    WHEN role IN ('architect', 'researcher') THEN 'advisory'
    WHEN role = 'security_auditor' THEN 'review'
    WHEN role = 'documentation_writer' THEN 'post_process'
    ELSE 'implement'
END WHERE operation IS NULL;
";

const MIGRATIONS: &[&str] = &[
    V1_SCHEMA, V2_SCHEMA, V3_SCHEMA, V4_SCHEMA, V5_SCHEMA, V6_SCHEMA,
];

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
    use factory_types::{
        AgentSession, AgentSessionMode, AttemptStatus, Plan, PlannedTask, ReviewDecision,
        ReviewResult, RunStatus, TaskEvidence, TaskOperation, TaskState,
    };
    use rusqlite::Connection;
    use tempfile::TempDir;

    use crate::{FactoryDb, V1_SCHEMA};

    #[test]
    fn applies_all_migrations_exactly_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let db = FactoryDb::open(&path).unwrap();
        let versions = schema_versions(&path);
        assert_eq!(versions, vec![1, 2, 3, 4, 5, 6]);
        db.create_run("objective", Some("codex")).unwrap();
        drop(db);

        let db = FactoryDb::open(&path).unwrap();
        assert_eq!(schema_versions(&path), vec![1, 2, 3, 4, 5, 6]);
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
        assert_eq!(schema_versions(&path), vec![1, 2, 3, 4, 5, 6]);
        let run = db.get_run(1).unwrap().unwrap();
        assert_eq!(run.objective, "legacy");
        let tasks = db.list_tasks(1).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "old task");
        // backfilled operation for a role-less legacy task
        assert_eq!(tasks[0].operation, Some(TaskOperation::Implement));
    }

    #[test]
    fn v6_backfills_operations_and_creates_role_artifacts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("old.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute_batch(crate::V2_SCHEMA).unwrap();
        conn.execute_batch(crate::V3_SCHEMA).unwrap();
        conn.execute_batch(crate::V4_SCHEMA).unwrap();
        conn.execute_batch(crate::V5_SCHEMA).unwrap();
        // Record the applied versions so the real migration only adds V6.
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             INSERT INTO schema_migrations (version, applied_at) VALUES
               (1, 'x'), (2, 'x'), (3, 'x'), (4, 'x'), (5, 'x');",
        )
        .unwrap();
        for (name, role, state) in [
            ("plain", None::<String>, "completed"),
            (
                "db work",
                Some("database_engineer".to_string()),
                "completed",
            ),
            ("tests", Some("test_engineer".to_string()), "completed"),
            ("design", Some("architect".to_string()), "completed"),
            ("audit", Some("security_auditor".to_string()), "completed"),
        ] {
            conn.execute(
                "INSERT INTO runs (objective, status, planner_agent, created_at, updated_at)
                 VALUES ('legacy', 'completed', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (run_id, title, objective, acceptance_criteria, state, position, role, created_at, updated_at)
                 VALUES (1, ?1, 'old', '[]', ?3, 0, ?2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                rusqlite::params![name, role, state],
            )
            .unwrap();
        }
        drop(conn);

        let db = FactoryDb::open(&path).unwrap();
        // creation records version 6 exactly once
        assert_eq!(schema_versions(&path), vec![1, 2, 3, 4, 5, 6]);
        let tasks = db.list_tasks(1).unwrap();
        let operation_of = |title: &str| {
            tasks
                .iter()
                .find(|task| task.title == title)
                .unwrap()
                .operation
                .expect("operation backfilled")
        };
        assert_eq!(operation_of("plain"), TaskOperation::Implement);
        assert_eq!(
            operation_of("db work"),
            TaskOperation::Implement,
            "unknown/execution custom roles default to implement"
        );
        assert_eq!(operation_of("tests"), TaskOperation::Verify);
        assert_eq!(operation_of("design"), TaskOperation::Advisory);
        assert_eq!(operation_of("audit"), TaskOperation::Review);
        // the new table exists and is queryable
        assert!(db.list_role_artifacts(1).unwrap().is_empty());
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
            .create_task(run.id, "A", "a", &[], TaskState::Ready, 0, None, None)
            .unwrap();
        let b = db
            .create_task(run.id, "B", "b", &[], TaskState::Ready, 1, None, None)
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
                None,
                None,
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
                Some("database_engineer"),
                Some(TaskOperation::Implement),
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
            .create_task(
                run.id,
                "T",
                "objective",
                &[],
                TaskState::Ready,
                0,
                None,
                None,
            )
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
            attempt_id: None,
            role: "planner".to_string(),
            operation: Some(TaskOperation::Planning),
            agent: "codex".to_string(),
            mode: AgentSessionMode::Automated,
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
        assert_eq!(db.get_agent_session(saved.id).unwrap(), Some(saved.clone()));
        assert_eq!(
            db.list_agent_sessions_for_agent("codex", 12).unwrap(),
            vec![saved]
        );
        assert!(db
            .list_agent_sessions_for_agent("unknown", 12)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn persists_a_plan_atomically_with_real_dependency_ids() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db
            .create_run_with_status("draft", Some("planner"), RunStatus::Planning)
            .unwrap();
        let tasks = db
            .persist_plan(
                run.id,
                &Plan {
                    objective: "normalized objective".into(),
                    tasks: vec![
                        PlannedTask {
                            id: "A".into(),
                            title: "First".into(),
                            objective: "first".into(),
                            dependencies: Vec::new(),
                            acceptance_criteria: vec!["done".into()],
                            role: None,
                            operation: Some(TaskOperation::Implement),
                        },
                        PlannedTask {
                            id: "B".into(),
                            title: "Second".into(),
                            objective: "second".into(),
                            dependencies: vec!["A".into()],
                            acceptance_criteria: vec!["reviewed".into()],
                            role: Some("database_engineer".into()),
                            operation: Some(TaskOperation::Implement),
                        },
                    ],
                },
            )
            .unwrap();
        assert_eq!(tasks[0].state, TaskState::Ready);
        assert_eq!(tasks[1].state, TaskState::Pending);
        assert_eq!(tasks[1].dependencies, vec![tasks[0].id]);
        assert_eq!(tasks[0].role, None);
        assert_eq!(tasks[1].role.as_deref(), Some("database_engineer"));
        let loaded = db.get_run(run.id).unwrap().unwrap();
        assert_eq!(loaded.status, RunStatus::Planned);
        assert_eq!(loaded.objective, "normalized objective");
    }

    #[test]
    fn task_attempts_round_trip_evidence_and_review() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("planner")).unwrap();
        let task = db
            .create_task(
                run.id,
                "Task",
                "objective",
                &[],
                TaskState::Ready,
                0,
                None,
                None,
            )
            .unwrap();
        let attempt = db
            .create_task_attempt(task, "worker", None, "opencode", "worktree")
            .unwrap();
        let evidence = TaskEvidence {
            changed_files: vec!["src/lib.rs".into()],
            diff_summary: "1 file changed".into(),
            commit_sha: Some("abc123".into()),
            commands: vec!["cargo test".into()],
            acceptance_criteria: vec!["tests pass".into()],
            worker_exit_code: Some(0),
            artifacts: vec![],
            diff_patch: Some("diff --git a/src/lib.rs".into()),
        };
        let review = ReviewResult {
            decision: ReviewDecision::Approve,
            reason: "verified".into(),
            feedback: Vec::new(),
        };
        db.finish_task_attempt(
            attempt.id,
            AttemptStatus::Approved,
            Some(0),
            Some("abc123"),
            None,
            Some(&evidence),
            Some(&review),
        )
        .unwrap();

        let loaded = db.latest_task_attempt(task).unwrap().unwrap();
        assert_eq!(loaded.status, AttemptStatus::Approved);
        assert_eq!(loaded.agent, "opencode");
        assert_eq!(loaded.role.as_deref(), Some("worker"));
        assert_eq!(loaded.evidence, Some(evidence));
        assert_eq!(loaded.review, Some(review));
        assert!(loaded.finished_at.is_some());
    }

    #[test]
    fn persists_run_team_snapshot() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        assert!(db.get_run(run.id).unwrap().unwrap().team.is_none());

        let team = factory_types::WorkflowTeam {
            planner: "codex".into(),
            workers: vec!["opencode".into(), "qwen".into()],
            reviewers: vec!["claude".into()],
            additional: std::collections::BTreeMap::from([(
                "database_engineer".to_string(),
                vec!["opencode".to_string()],
            )]),
        };
        db.set_run_team(run.id, &team).unwrap();
        assert_eq!(db.get_run(run.id).unwrap().unwrap().team, Some(team));
        assert!(db
            .list_runs()
            .unwrap()
            .iter()
            .any(|listed| listed.team.is_some()));
    }

    #[test]
    fn counts_attempts_per_run_for_routing() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        let other = db.create_run("other", Some("codex")).unwrap();
        let task = db
            .create_task(
                run.id,
                "Task",
                "objective",
                &[],
                TaskState::Ready,
                0,
                None,
                None,
            )
            .unwrap();
        let other_task = db
            .create_task(
                other.id,
                "Other",
                "objective",
                &[],
                TaskState::Ready,
                0,
                None,
                None,
            )
            .unwrap();
        assert_eq!(db.count_task_attempts(run.id).unwrap(), 0);
        db.create_task_attempt(task, "worker", None, "opencode", "worktree")
            .unwrap();
        db.create_task_attempt(task, "worker", None, "qwen", "worktree")
            .unwrap();
        db.create_task_attempt(other_task, "worker", None, "claude", "worktree")
            .unwrap();
        assert_eq!(db.count_task_attempts(run.id).unwrap(), 2);
        assert_eq!(db.count_task_attempts(other.id).unwrap(), 1);
    }

    #[test]
    fn role_artifacts_round_trip_and_filter_by_task() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("planner")).unwrap();
        let research = db
            .create_task(
                run.id,
                "Research",
                "find",
                &[],
                TaskState::Completed,
                0,
                Some("researcher"),
                Some(TaskOperation::Advisory),
            )
            .unwrap();
        let worker = db
            .create_task(
                run.id,
                "Worker",
                "build",
                &[],
                TaskState::Completed,
                1,
                None,
                None,
            )
            .unwrap();
        let attempt = db
            .create_task_attempt(research, "researcher", None, "search-agent", "worktree")
            .unwrap();
        let artifact = db
            .insert_role_artifact(
                run.id,
                Some(research),
                Some(attempt.id),
                "researcher",
                Some(TaskOperation::Advisory),
                "research",
                r#"{"summary":"found","findings":[]}"#,
            )
            .unwrap();
        assert!(artifact.id > 0);

        let all = db.list_role_artifacts(run.id).unwrap();
        assert_eq!(all, vec![artifact.clone()]);
        let per_task = db.list_artifacts_for_task(research).unwrap();
        assert_eq!(per_task, vec![artifact.clone()]);
        let per_worker = db.list_artifacts_for_tasks(&[worker]).unwrap();
        assert!(per_worker.is_empty(), "worker has no artifacts");
        let selected = db.list_artifacts_for_tasks(&[research]).unwrap();
        assert_eq!(selected, vec![artifact.clone()]);
        assert_eq!(
            db.get_role_artifact(artifact.id).unwrap().unwrap().kind,
            "research"
        );
        let loaded = db.list_role_artifacts(run.id).unwrap().remove(0);
        assert_eq!(
            loaded.operation.as_deref(),
            Some(TaskOperation::Advisory.as_str())
        );
    }

    #[test]
    fn restart_reconciliation_interrupts_running_state() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db
            .create_run_with_status("objective", Some("planner"), RunStatus::Active)
            .unwrap();
        let task = db
            .create_task(
                run.id,
                "Task",
                "objective",
                &[],
                TaskState::Running,
                0,
                None,
                None,
            )
            .unwrap();
        let attempt = db
            .create_task_attempt(task, "worker", None, "opencode", "worktree")
            .unwrap();
        let session = db
            .insert_agent_session(&AgentSession {
                id: 0,
                run_id: Some(run.id),
                task_id: Some(task),
                attempt_id: Some(attempt.id),
                role: "worker".into(),
                operation: None,
                agent: "worker".into(),
                mode: AgentSessionMode::Automated,
                command: "worker --task".into(),
                status: "running".into(),
                started_at: "2026-01-01T00:00:00Z".into(),
                finished_at: None,
                exit_code: None,
                duration_ms: None,
                stdout: Some("partial".into()),
                stderr: Some(String::new()),
            })
            .unwrap();

        let reconciled = db.reconcile_interrupted().unwrap();
        assert_eq!(reconciled.sessions, 1);
        assert_eq!(reconciled.attempts, 1);
        assert_eq!(reconciled.tasks, 1);
        assert_eq!(reconciled.runs, 1);
        assert_eq!(
            db.get_agent_session(session.id).unwrap().unwrap().status,
            "interrupted"
        );
        assert_eq!(
            db.latest_task_attempt(task).unwrap().unwrap().status,
            AttemptStatus::Interrupted
        );
        assert_eq!(db.get_task(task).unwrap().unwrap().state, TaskState::Failed);
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Failed
        );
    }
}
