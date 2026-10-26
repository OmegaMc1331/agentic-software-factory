pub mod error;

pub use error::DbError;

use chrono::Utc;
use factory_types::{
    resolve_patch, resolve_replan, AgentOutputMetadata, AgentSession, AgentSessionMode,
    AttemptStatus, GitHubDelivery, GitHubIssueLink, IntegrationOutcome, IntegrationOutcomeKind,
    McpToolMetadata, Plan, PlanApplyOutcome, PlanPatch, PlanRevisionRecord, PlanRevisionSource,
    PlanSnapshot, PlanState, ReplanRequest, ResolvedPlan, ReviewResult, RoleArtifact,
    RoutingCandidateScore, RoutingDecision, Run, RunStatus, Task, TaskAttempt, TaskEvidence,
    TaskOperation, TaskState, ToolClassification, ToolGrant, ToolInvocation, ToolMetrics,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
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
                "SELECT id, objective, status, planner_agent, created_at, updated_at, team, plan_revision, recipe
                 FROM runs WHERE id = ?1",
                params![id],
                build_run,
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_runs(&self) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, objective, status, planner_agent, created_at, updated_at, team, plan_revision, recipe
             FROM runs ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([], build_run)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// All run summaries, newest first, with task counts aggregated in one
    /// query. Used by the legacy unparameterized run list.
    pub fn list_run_summaries(&self) -> Result<Vec<RunSummaryRecord>> {
        self.list_run_summaries_query(None, None)
    }

    /// One newest-first page of run summaries. `before` is an exclusive
    /// run-id cursor; task counts are aggregated by SQLite for the selected
    /// page only.
    pub fn list_run_summaries_page(
        &self,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<RunSummaryRecord>> {
        self.list_run_summaries_query(Some(limit), before)
    }

    fn list_run_summaries_query(
        &self,
        limit: Option<usize>,
        before: Option<i64>,
    ) -> Result<Vec<RunSummaryRecord>> {
        // SQLite treats a negative LIMIT as unbounded.
        let limit = limit
            .map(|value| value.min(i64::MAX as usize) as i64)
            .unwrap_or(-1);
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.objective, r.status, r.planner_agent, r.created_at, r.updated_at,
                    r.team, r.plan_revision, r.recipe, t.state, COUNT(t.id)
             FROM (
                 SELECT id, objective, status, planner_agent, created_at, updated_at,
                        team, plan_revision, recipe
                 FROM runs
                 WHERE (?1 IS NULL OR id < ?1)
                 ORDER BY id DESC
                 LIMIT ?2
             ) r
             LEFT JOIN tasks t ON t.run_id = r.id
             GROUP BY r.id, r.objective, r.status, r.planner_agent, r.created_at,
                      r.updated_at, r.team, r.plan_revision, r.recipe, t.state
             ORDER BY r.id DESC, t.state",
        )?;
        let rows = stmt.query_map(params![before, limit], |row| {
            Ok((
                build_run(row)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, i64>(10)?,
            ))
        })?;

        let mut summaries: Vec<RunSummaryRecord> = Vec::new();
        for row in rows {
            let (run, state, count) = row?;
            let count = count.max(0) as usize;
            if let Some(last) = summaries.last_mut() {
                if last.run.id == run.id {
                    if let Some(state) = state {
                        last.counts.record(task_state(state), count);
                    }
                    continue;
                }
            }
            let mut counts = TaskStateCounts::default();
            if let Some(state) = state {
                counts.record(task_state(state), count);
            }
            summaries.push(RunSummaryRecord { run, counts });
        }
        Ok(summaries)
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

    /// Persists the immutable recipe snapshot on a run. Written once at
    /// creation; never updated afterwards.
    pub fn set_run_recipe(&self, id: i64, recipe: &factory_types::RecipeSnapshot) -> Result<()> {
        let recipe = serde_json::to_string(recipe)?;
        let changed = self.conn.execute(
            "UPDATE runs SET recipe = ?1, updated_at = ?2 WHERE id = ?3",
            params![recipe, now(), id],
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

    /// Records the head of the run's integration branch after available work is
    /// integrated. `None` clears it (run was never or is no longer integrated).
    pub fn set_run_integration(&self, id: i64, integration_sha: Option<&str>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE runs SET integration_sha = ?1, updated_at = ?2 WHERE id = ?3",
            params![integration_sha, now(), id],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("run"));
        }
        Ok(())
    }

    /// The latest integrated head of the run's `factory/run-<id>` branch, or
    /// `None` when no implementation work has been integrated yet.
    pub fn get_run_integration(&self, id: i64) -> Result<Option<String>> {
        let sha: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT integration_sha FROM runs WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(sha.flatten())
    }

    /// Records the immutable base commit the run started from (the head the
    /// `factory/run-<id>` branch was created at). `None` clears it.
    pub fn set_run_base(&self, id: i64, base_sha: Option<&str>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE runs SET base_sha = ?1, updated_at = ?2 WHERE id = ?3",
            params![base_sha, now(), id],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("run"));
        }
        Ok(())
    }

    /// The base commit the run started from, or `None` when it has not started.
    pub fn get_run_base(&self, id: i64) -> Result<Option<String>> {
        let sha: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT base_sha FROM runs WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(sha.flatten())
    }

    /// Atomically replaces one server's lightweight discovery cache. External
    /// I/O is completed before this method is called, so the transaction never
    /// waits on an MCP peer.
    pub fn replace_mcp_tools(&self, server_id: &str, tools: &[McpToolMetadata]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM mcp_tools WHERE server_id = ?1",
            params![server_id],
        )?;
        for tool in tools {
            tx.execute(
                "INSERT INTO mcp_tools
                 (server_id, tool_name, description, input_schema, annotations, classification, discovered_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    server_id,
                    tool.name,
                    tool.description,
                    serde_json::to_string(&tool.input_schema)?,
                    tool.annotations
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?,
                    tool.classification.as_str(),
                    tool.discovered_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes ephemeral discovery/concurrency state for a deleted server.
    /// Historical invocation rows remain intact as audit evidence.
    pub fn remove_mcp_server_cache(&self, server_id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM mcp_tools WHERE server_id = ?1",
            params![server_id],
        )?;
        tx.execute(
            "DELETE FROM mcp_server_leases WHERE server_id = ?1",
            params![server_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_mcp_tools(&self, server_id: Option<&str>) -> Result<Vec<McpToolMetadata>> {
        let sql = if server_id.is_some() {
            "SELECT server_id, tool_name, description, input_schema, annotations, classification, discovered_at
             FROM mcp_tools WHERE server_id = ?1 ORDER BY server_id, tool_name"
        } else {
            "SELECT server_id, tool_name, description, input_schema, annotations, classification, discovered_at
             FROM mcp_tools ORDER BY server_id, tool_name"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let build = |row: &rusqlite::Row<'_>| -> rusqlite::Result<McpToolMetadata> {
            let server: String = row.get(0)?;
            let name: String = row.get(1)?;
            let schema: String = row.get(3)?;
            let annotations: Option<String> = row.get(4)?;
            let classification: String = row.get(5)?;
            Ok(McpToolMetadata {
                qualified_name: format!("{server}.{name}"),
                server,
                name,
                description: row.get(2)?,
                input_schema: serde_json::from_str(&schema).unwrap_or(serde_json::Value::Null),
                annotations: annotations.and_then(|value| serde_json::from_str(&value).ok()),
                classification: classification.parse().unwrap_or_default(),
                discovered_at: row.get(6)?,
            })
        };
        let tools = match server_id {
            Some(server_id) => stmt
                .query_map(params![server_id], build)?
                .collect::<std::result::Result<Vec<_>, _>>()?,
            None => stmt
                .query_map([], build)?
                .collect::<std::result::Result<Vec<_>, _>>()?,
        };
        Ok(tools)
    }

    pub fn insert_tool_grant(&self, grant: &ToolGrant) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tool_grants
             (id, token_hash, run_id, task_id, attempt_id, session_id, agent, role, operation,
              allowed_tools, expires_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                grant.id,
                grant.token_hash,
                grant.run_id,
                grant.task_id,
                grant.attempt_id,
                grant.session_id,
                grant.agent,
                grant.role,
                grant.operation,
                serde_json::to_string(&grant.allowed_tools)?,
                grant.expires_at,
                grant.revoked_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_tool_grant_by_hash(&self, token_hash: &str) -> Result<Option<ToolGrant>> {
        self.conn
            .query_row(
                "SELECT id, token_hash, run_id, task_id, attempt_id, session_id, agent, role,
                        operation, allowed_tools, expires_at, revoked_at
                 FROM tool_grants WHERE token_hash = ?1",
                params![token_hash],
                |row| {
                    let allowed: String = row.get(9)?;
                    Ok(ToolGrant {
                        id: row.get(0)?,
                        token_hash: row.get(1)?,
                        run_id: row.get(2)?,
                        task_id: row.get(3)?,
                        attempt_id: row.get(4)?,
                        session_id: row.get(5)?,
                        agent: row.get(6)?,
                        role: row.get(7)?,
                        operation: row.get(8)?,
                        allowed_tools: serde_json::from_str(&allowed).unwrap_or_default(),
                        expires_at: row.get(10)?,
                        revoked_at: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn revoke_tool_grant(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tool_grants SET revoked_at = COALESCE(revoked_at, ?1) WHERE id = ?2",
            params![now(), id],
        )?;
        Ok(())
    }

    pub fn revoke_session_tool_grants(&self, session_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tool_grants SET revoked_at = COALESCE(revoked_at, ?1) WHERE session_id = ?2",
            params![now(), session_id],
        )?;
        Ok(())
    }

    pub fn start_tool_invocation(&self, invocation: &ToolInvocation) -> Result<ToolInvocation> {
        self.conn.execute(
            "INSERT INTO tool_invocations
             (run_id, task_id, attempt_id, session_id, agent, role, server, tool,
              classification, status, started_at, duration_ms, input_summary, output_summary, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'running', ?10, NULL, ?11, NULL, NULL)",
            params![
                invocation.run_id,
                invocation.task_id,
                invocation.attempt_id,
                invocation.session_id,
                invocation.agent,
                invocation.role,
                invocation.server,
                invocation.tool,
                invocation.classification.as_str(),
                invocation.started_at,
                invocation.input_summary,
            ],
        )?;
        let mut stored = invocation.clone();
        stored.id = self.conn.last_insert_rowid();
        stored.status = "running".into();
        Ok(stored)
    }

    pub fn finish_tool_invocation(
        &self,
        id: i64,
        status: &str,
        duration_ms: u64,
        output_summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tool_invocations
             SET status = ?1, duration_ms = ?2, output_summary = ?3, error = ?4
             WHERE id = ?5 AND status = 'running'",
            params![status, duration_ms as i64, output_summary, error, id],
        )?;
        Ok(())
    }

    pub fn update_tool_invocation_classification(
        &self,
        id: i64,
        classification: ToolClassification,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tool_invocations SET classification = ?1 WHERE id = ?2 AND status = 'running'",
            params![classification.as_str(), id],
        )?;
        Ok(())
    }

    /// Marks an in-flight call cancelled only if no terminal result was
    /// recorded. Used by cancellation-safe lease cleanup.
    pub fn cancel_running_tool_invocation(&self, id: i64, duration_ms: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE tool_invocations
             SET status = 'cancelled', duration_ms = COALESCE(duration_ms, ?1),
                 error = COALESCE(error, 'MCP operation cancelled')
             WHERE id = ?2 AND status = 'running'",
            params![duration_ms as i64, id],
        )?;
        Ok(())
    }

    pub fn list_tool_invocations(&self) -> Result<Vec<ToolInvocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, task_id, attempt_id, session_id, agent, role, server, tool,
                    classification, status, started_at, duration_ms, input_summary,
                    output_summary, error
             FROM tool_invocations ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            let classification: String = row.get(9)?;
            Ok(ToolInvocation {
                id: row.get(0)?,
                run_id: row.get(1)?,
                task_id: row.get(2)?,
                attempt_id: row.get(3)?,
                session_id: row.get(4)?,
                agent: row.get(5)?,
                role: row.get(6)?,
                server: row.get(7)?,
                tool: row.get(8)?,
                classification: classification.parse().unwrap_or_default(),
                status: row.get(10)?,
                started_at: row.get(11)?,
                duration_ms: row.get::<_, Option<i64>>(12)?.map(|value| value as u64),
                input_summary: row.get(13)?,
                output_summary: row.get(14)?,
                error: row.get(15)?,
            })
        })?;
        let invocations = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(invocations)
    }

    pub fn tool_metrics(&self) -> Result<ToolMetrics> {
        let invocations = self.list_tool_invocations()?;
        let mut durations: Vec<u64> = invocations
            .iter()
            .filter_map(|invocation| invocation.duration_ms)
            .collect();
        durations.sort_unstable();
        Ok(ToolMetrics {
            calls: invocations.len(),
            successes: invocations
                .iter()
                .filter(|invocation| invocation.status == "success")
                .count(),
            failures: invocations
                .iter()
                .filter(|invocation| !matches!(invocation.status.as_str(), "success" | "running"))
                .count(),
            median_duration_ms: factory_types::median_u64(&durations),
        })
    }

    /// Cross-process, per-server concurrency lease. The update transaction is
    /// short and is committed before any external call begins.
    pub fn try_acquire_mcp_server_slot(
        &self,
        server_id: &str,
        max_concurrency: usize,
        stale_before: &str,
    ) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO mcp_server_leases (server_id, active_calls, updated_at)
             VALUES (?1, 0, ?2)",
            params![server_id, now()],
        )?;
        tx.execute(
            "UPDATE mcp_server_leases SET active_calls = 0
             WHERE server_id = ?1 AND updated_at < ?2",
            params![server_id, stale_before],
        )?;
        let changed = tx.execute(
            "UPDATE mcp_server_leases
             SET active_calls = active_calls + 1, updated_at = ?1
             WHERE server_id = ?2 AND active_calls < ?3",
            params![now(), server_id, max_concurrency as i64],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn release_mcp_server_slot(&self, server_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE mcp_server_leases
             SET active_calls = MAX(active_calls - 1, 0), updated_at = ?1
             WHERE server_id = ?2",
            params![now(), server_id],
        )?;
        Ok(())
    }

    /// Atomically claims a Ready task for dispatch. Returns `true` exactly when
    /// this caller won the claim and the task is now Running.
    pub fn try_claim_task(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE tasks SET state = 'running', updated_at = ?1 WHERE id = ?2 AND state = 'ready'",
            params![now(), id],
        )?;
        Ok(changed == 1)
    }

    /// Atomically claims an AwaitingIntegration task for the serialized
    /// integration lane. Returns `true` exactly when this caller won the claim
    /// and the task is now Integrating.
    pub fn try_claim_integration(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE tasks
             SET state = 'integrating', updated_at = ?1
             WHERE id = ?2 AND state = 'awaiting_integration'",
            params![now(), id],
        )?;
        Ok(changed == 1)
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
        let final_tasks: Vec<Task> = plan
            .tasks
            .iter()
            .enumerate()
            .map(|(position, task)| Task {
                id: ids[&task.id],
                run_id,
                title: task.title.clone(),
                objective: task.objective.clone(),
                acceptance_criteria: task.acceptance_criteria.clone(),
                state: if task.dependencies.is_empty() {
                    TaskState::Ready
                } else {
                    TaskState::Pending
                },
                position: position as i32,
                dependencies: task.dependencies.iter().map(|d| ids[d]).collect(),
                worktree_path: None,
                role: task.role.clone(),
                operation: task.operation,
                agent_override: None,
                created_at: ts.clone(),
                updated_at: ts.clone(),
            })
            .collect();
        tx.execute(
            "UPDATE runs SET objective = ?1, status = 'planned', plan_revision = 1, updated_at = ?2 WHERE id = ?3",
            params![plan.objective, ts, run_id],
        )?;
        let snapshot = serde_json::to_string(&PlanSnapshot {
            objective: plan.objective.clone(),
            tasks: final_tasks.clone(),
        })?;
        tx.execute(
            "INSERT INTO plan_revisions (run_id, revision, source, reason, planner_session_id, snapshot, created_at)
             VALUES (?1, 1, 'planner', NULL, NULL, ?2, ?3)",
            params![run_id, snapshot, ts],
        )?;
        tx.commit()?;
        Ok(final_tasks)
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
                "SELECT t.id, t.run_id, t.title, t.objective, t.acceptance_criteria, t.state, t.position, t.worktree_path, t.created_at, t.updated_at, t.role, t.operation, t.agent_override
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
            "SELECT t.id, t.run_id, t.title, t.objective, t.acceptance_criteria, t.state, t.position, t.worktree_path, t.created_at, t.updated_at, t.role, t.operation, t.agent_override
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

    /// The run's current plan revision. Initial planner output is revision 1.
    pub fn get_plan_revision(&self, run_id: i64) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT plan_revision FROM runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(DbError::NotFound("run"))
    }

    /// Snapshot of the run's plan for validation and resolution: revision,
    /// objective, tasks, and per-task attempt counts.
    pub fn plan_state(&self, run_id: i64) -> Result<PlanState> {
        load_plan_state(&self.conn, run_id)
    }

    /// Applies a batch of plan mutations atomically. The expected revision is
    /// re-checked inside the transaction so a stale editor session is rejected
    /// with [`DbError::Conflict`], while a structurally invalid patch returns
    /// [`PlanApplyOutcome::Invalid`] (nothing is written).
    pub fn apply_plan_patch(
        &self,
        run_id: i64,
        patch: &PlanPatch,
        source: PlanRevisionSource,
        reason: Option<&str>,
        planner_session_id: Option<i64>,
    ) -> Result<PlanApplyOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let state = load_plan_state(&tx, run_id)?;
        if state.revision != patch.expected_revision {
            return Err(DbError::Conflict {
                expected: patch.expected_revision,
                current: state.revision,
            });
        }
        let resolved = match resolve_patch(&state, patch) {
            Ok(resolved) => resolved,
            Err(diagnostics) => return Ok(PlanApplyOutcome::Invalid(diagnostics)),
        };
        let (revision, tasks) = apply_resolved(
            &tx,
            run_id,
            &state,
            &resolved,
            source,
            reason,
            planner_session_id,
        )?;
        tx.commit()?;
        Ok(PlanApplyOutcome::Applied {
            run_id,
            revision,
            tasks,
        })
    }

    /// Applies a partial replan atomically. The scope is computed from the
    /// real dependency graph inside the transaction; the mutable scope is
    /// superseded and the new plan is spliced in. Conflicts and invalid replans
    /// behave like [`apply_plan_patch`](Self::apply_plan_patch).
    pub fn apply_replan(
        &self,
        run_id: i64,
        request: &ReplanRequest,
        source: PlanRevisionSource,
        reason: Option<&str>,
        planner_session_id: Option<i64>,
    ) -> Result<PlanApplyOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let state = load_plan_state(&tx, run_id)?;
        if state.revision != request.expected_revision {
            return Err(DbError::Conflict {
                expected: request.expected_revision,
                current: state.revision,
            });
        }
        let resolved = match resolve_replan(&state, request) {
            Ok(resolved) => resolved,
            Err(diagnostics) => return Ok(PlanApplyOutcome::Invalid(diagnostics)),
        };
        let (revision, tasks) = apply_resolved(
            &tx,
            run_id,
            &state,
            &resolved,
            source,
            reason,
            planner_session_id,
        )?;
        tx.commit()?;
        Ok(PlanApplyOutcome::Applied {
            run_id,
            revision,
            tasks,
        })
    }

    /// Every durable revision of the run's plan, oldest first.
    pub fn list_plan_revisions(&self, run_id: i64) -> Result<Vec<PlanRevisionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, revision, source, reason, planner_session_id, snapshot, created_at
             FROM plan_revisions WHERE run_id = ?1 ORDER BY revision",
        )?;
        let rows = stmt
            .query_map(params![run_id], build_plan_revision)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// A single durable revision record, when it exists.
    pub fn get_plan_revision_record(
        &self,
        run_id: i64,
        revision: i64,
    ) -> Result<Option<PlanRevisionRecord>> {
        self.conn
            .query_row(
                "SELECT id, run_id, revision, source, reason, planner_session_id, snapshot, created_at
                 FROM plan_revisions WHERE run_id = ?1 AND revision = ?2",
                params![run_id, revision],
                build_plan_revision,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_worktree_path(&self, id: i64, path: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET worktree_path = ?1, updated_at = ?2 WHERE id = ?3",
            params![path, now(), id],
        )?;
        Ok(())
    }

    /// Pins (`Some`) or clears (`None`) the manual routing override for a
    /// task. The pin is honored by every routing mode and re-validated at
    /// dispatch time.
    pub fn set_task_agent_override(&self, id: i64, agent: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET agent_override = ?1, updated_at = ?2 WHERE id = ?3",
            params![agent, now(), id],
        )?;
        Ok(())
    }

    /// Persists one routing audit record. `decision.id` is ignored on input
    /// and set to the inserted row id.
    pub fn insert_routing_decision(&self, decision: &RoutingDecision) -> Result<RoutingDecision> {
        let mut decision = decision.clone();
        let scores = serde_json::to_string(&decision.candidate_scores)?;
        self.conn.execute(
            "INSERT INTO routing_decisions (task_id, attempt_id, mode, selected_agent, role, operation, language, candidate_scores, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                decision.task_id,
                decision.attempt_id,
                decision.mode,
                decision.selected_agent,
                decision.role,
                decision.operation.map(TaskOperation::as_str),
                decision.language,
                scores,
                decision.reason,
                decision.created_at,
            ],
        )?;
        decision.id = self.conn.last_insert_rowid();
        Ok(decision)
    }

    /// Every routing decision recorded for a task, oldest first.
    pub fn list_routing_decisions_for_task(&self, task_id: i64) -> Result<Vec<RoutingDecision>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, attempt_id, mode, selected_agent, role, operation, language, candidate_scores, reason, created_at
             FROM routing_decisions WHERE task_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![task_id], build_routing_decision)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_agent_session(&self, session: &AgentSession) -> Result<AgentSession> {
        let mut session = session.clone();
        self.conn.execute(
            "INSERT INTO agent_sessions (run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr, policy_audit, output_meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                session.stderr,
                session
                    .policy_audit
                    .as_ref()
                    .map(|audit| serde_json::to_string(audit).unwrap_or_default()),
                session
                    .output_meta
                    .as_ref()
                    .map(|metadata| serde_json::to_string(metadata).unwrap_or_default())
            ],
        )?;
        session.id = self.conn.last_insert_rowid();
        Ok(session)
    }

    pub fn list_agent_sessions(&self, run_id: Option<i64>) -> Result<Vec<AgentSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr, policy_audit, output_meta
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
                "SELECT id, run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr, policy_audit, output_meta
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
            "SELECT id, run_id, task_id, attempt_id, role, operation, agent, mode, command, status, started_at, finished_at, exit_code, duration_ms, stdout, stderr, policy_audit, output_meta
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

    pub fn set_agent_session_output_metadata(
        &self,
        id: i64,
        metadata: Option<&AgentOutputMetadata>,
    ) -> Result<()> {
        let encoded = metadata.map(|metadata| serde_json::to_string(metadata).unwrap_or_default());
        self.conn.execute(
            "UPDATE agent_sessions SET output_meta = ?1 WHERE id = ?2",
            params![encoded, id],
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
        source_base: Option<&str>,
    ) -> Result<TaskAttempt> {
        let attempt_number: u32 = self.conn.query_row(
            "SELECT COALESCE(MAX(attempt_number), 0) + 1 FROM task_attempts WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO task_attempts
             (task_id, attempt_number, agent, role, operation, status, started_at, worktree_path, source_base)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7, ?8)",
            params![
                task_id,
                attempt_number,
                agent,
                role,
                operation.map(TaskOperation::as_str),
                now(),
                worktree_path,
                source_base
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

    /// All attempts attributed to one agent across every run. A cheap,
    /// deterministic observation count used to order cold-start exploration
    /// (not a quality metric — quality lives in `factory-eval`).
    pub fn count_agent_attempts(&self, agent: &str) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM task_attempts WHERE agent = ?1",
            params![agent],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    pub fn get_task_attempt(&self, id: i64) -> Result<Option<TaskAttempt>> {
        self.conn
            .query_row(
                "SELECT id, task_id, attempt_number, agent, status, started_at, finished_at,
                        worktree_path, commit_sha, exit_code, error, evidence, review, role, operation, source_base
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
                    a.evidence, a.review, a.role, a.operation, a.source_base
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

    /// Every attempt of one task, oldest first. Routing uses this to apply
    /// the retry penalty to agents that already failed or were rejected on
    /// this specific task.
    pub fn list_task_attempts_for_task(&self, task_id: i64) -> Result<Vec<TaskAttempt>> {
        let mut statement = self.conn.prepare(
            "SELECT a.id, a.task_id, a.attempt_number, a.agent, a.status, a.started_at,
                    a.finished_at, a.worktree_path, a.commit_sha, a.exit_code, a.error,
                    a.evidence, a.review, a.role, a.operation, a.source_base
             FROM task_attempts a
             WHERE a.task_id = ?1
             ORDER BY a.attempt_number",
        )?;
        let attempts = statement
            .query_map(params![task_id], build_attempt)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(attempts)
    }

    pub fn latest_task_attempt(&self, task_id: i64) -> Result<Option<TaskAttempt>> {
        self.conn
            .query_row(
                "SELECT id, task_id, attempt_number, agent, status, started_at, finished_at,
                        worktree_path, commit_sha, exit_code, error, evidence, review, role, operation, source_base
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

    /// Records the commit the attempt was merged onto the run branch as. Used
    /// by the integration lane after the freshness review approves; the
    /// attempt stays `Approved` until integrated.
    pub fn set_attempt_commit(&self, id: i64, commit_sha: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE task_attempts SET commit_sha = ?1 WHERE id = ?2",
            params![commit_sha, id],
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

    // --- GitHub linkage and delivery ---------------------------------------

    /// Persists the imported GitHub Issue a run was created from. The content
    /// is stored verbatim (bounded at import) as untrusted context for
    /// traceability; it is never interpreted as instructions.
    pub fn set_run_github_link(&self, run_id: i64, link: &GitHubIssueLink) -> Result<()> {
        self.conn.execute(
            "INSERT INTO github_links
                 (run_id, provider, repository, issue_number, issue_url, issue_title,
                  issue_body, issue_state, issue_author, issue_labels, issue_comments, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(run_id) DO UPDATE SET
                 provider = excluded.provider,
                 repository = excluded.repository,
                 issue_number = excluded.issue_number,
                 issue_url = excluded.issue_url,
                 issue_title = excluded.issue_title,
                 issue_body = excluded.issue_body,
                 issue_state = excluded.issue_state,
                 issue_author = excluded.issue_author,
                 issue_labels = excluded.issue_labels,
                 issue_comments = excluded.issue_comments,
                 imported_at = excluded.imported_at",
            params![
                run_id,
                link.provider,
                link.repository,
                link.issue_number,
                link.issue_url,
                link.issue_title,
                link.issue_body,
                link.issue_state,
                link.issue_author,
                serde_json::to_string(&link.issue_labels)?,
                serde_json::to_string(&link.issue_comments)?,
                link.imported_at
            ],
        )?;
        Ok(())
    }

    /// The GitHub Issue a run was imported from, when it has one.
    pub fn get_run_github_link(&self, run_id: i64) -> Result<Option<GitHubIssueLink>> {
        let row = self
            .conn
            .query_row(
                "SELECT provider, repository, issue_number, issue_url, issue_title,
                        issue_body, issue_state, issue_author, issue_labels, issue_comments, imported_at
                 FROM github_links WHERE run_id = ?1",
                params![run_id],
                |row| {
                    Ok(RawGithubLink {
                        provider: row.get(0)?,
                        repository: row.get(1)?,
                        issue_number: row.get(2)?,
                        issue_url: row.get(3)?,
                        issue_title: row.get(4)?,
                        issue_body: row.get(5)?,
                        issue_state: row.get(6)?,
                        issue_author: row.get(7)?,
                        issue_labels: row.get(8)?,
                        issue_comments: row.get(9)?,
                        imported_at: row.get(10)?,
                    })
                },
            )
            .optional()?;
        row.map(|raw| raw.into_link().map_err(DbError::Json))
            .transpose()
    }

    /// The delivery record for a run, creating a fresh `not_ready` row when
    /// none exists yet. The insert is conflict-safe: two connections racing
    /// to initialize the same run's row cannot fail.
    pub fn get_or_create_delivery(&self, run_id: i64) -> Result<GitHubDelivery> {
        if let Some(existing) = self.get_delivery(run_id)? {
            return Ok(existing);
        }
        let delivery = GitHubDelivery::initial(run_id, &now());
        self.conn.execute(
            "INSERT INTO github_deliveries
                 (run_id, state, repository, remote, base_branch, head_branch,
                  pushed_head, pr_number, pr_url, pr_state, pr_is_draft, error, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(run_id) DO NOTHING",
            params![
                delivery.run_id,
                delivery.state.as_str(),
                delivery.repository,
                delivery.remote,
                delivery.base_branch,
                delivery.head_branch,
                delivery.pushed_head,
                None::<i64>,
                None::<String>,
                None::<String>,
                None::<i64>,
                delivery.error,
                delivery.created_at,
                delivery.updated_at
            ],
        )?;
        self.get_delivery(run_id)?
            .ok_or_else(|| DbError::Corrupt(format!("delivery row for run {run_id} vanished")))
    }

    /// The persisted delivery state of a run, when a record exists.
    pub fn get_delivery(&self, run_id: i64) -> Result<Option<GitHubDelivery>> {
        let row = self
            .conn
            .query_row(
                "SELECT run_id, state, repository, remote, base_branch, head_branch, pushed_head,
                        pr_number, pr_url, pr_state, pr_is_draft, error, created_at, updated_at,
                        operation_id, recovery_required
                 FROM github_deliveries WHERE run_id = ?1",
                params![run_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, i64>(15)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            run_id,
            state,
            repository,
            remote,
            base_branch,
            head_branch,
            pushed_head,
            pr_number,
            pr_url,
            pr_state,
            pr_is_draft,
            error,
            created_at,
            updated_at,
            operation_id,
            recovery_required,
        )) = row
        else {
            return Ok(None);
        };
        let pull_request = match (pr_number, pr_url.clone()) {
            (Some(number), Some(url)) => Some(factory_types::PullRequestInfo {
                number,
                url,
                state: pr_state.unwrap_or_else(|| "OPEN".to_string()),
                is_draft: pr_is_draft.unwrap_or(0) != 0,
            }),
            _ => None,
        };
        Ok(Some(GitHubDelivery {
            run_id,
            state: state.parse().map_err(|message| {
                DbError::Corrupt(format!("delivery state for run {run_id}: {message}"))
            })?,
            repository,
            remote,
            base_branch,
            head_branch,
            pushed_head,
            pull_request,
            error,
            operation_id,
            recovery_required: recovery_required != 0,
            created_at,
            updated_at,
        }))
    }

    /// Persists the full delivery record (state, PR metadata, pushed head).
    pub fn set_delivery(&self, delivery: &GitHubDelivery) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE github_deliveries
             SET state = ?1, repository = ?2, remote = ?3, base_branch = ?4,
                 pushed_head = ?5, pr_number = ?6, pr_url = ?7, pr_state = ?8,
                 pr_is_draft = ?9, error = ?10, updated_at = ?11,
                 operation_id = ?12, recovery_required = ?13
             WHERE run_id = ?14",
            params![
                delivery.state.as_str(),
                delivery.repository,
                delivery.remote,
                delivery.base_branch,
                delivery.pushed_head,
                delivery.pull_request.as_ref().map(|pr| pr.number),
                delivery.pull_request.as_ref().map(|pr| pr.url.clone()),
                delivery.pull_request.as_ref().map(|pr| pr.state.clone()),
                delivery.pull_request.as_ref().map(|pr| pr.is_draft as i64),
                delivery.error,
                now(),
                delivery.operation_id,
                delivery.recovery_required as i64,
                delivery.run_id
            ],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound("github delivery"));
        }
        Ok(())
    }

    /// Persists a delivery transition only while `operation_id` still owns the
    /// row. A stale worker (an attempt superseded after a restart or retry)
    /// cannot overwrite a newer outcome — including `published`.
    pub fn set_delivery_if_operation(
        &self,
        delivery: &GitHubDelivery,
        operation_id: &str,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE github_deliveries
             SET state = ?1, pushed_head = ?2, pr_number = ?3, pr_url = ?4, pr_state = ?5,
                 pr_is_draft = ?6, error = ?7, updated_at = ?8, recovery_required = ?9
             WHERE run_id = ?10 AND operation_id = ?11 AND state IN ('pushing','creating_pr')",
            params![
                delivery.state.as_str(),
                delivery.pushed_head,
                delivery.pull_request.as_ref().map(|pr| pr.number),
                delivery.pull_request.as_ref().map(|pr| pr.url.clone()),
                delivery.pull_request.as_ref().map(|pr| pr.state.clone()),
                delivery.pull_request.as_ref().map(|pr| pr.is_draft as i64),
                delivery.error,
                now(),
                delivery.recovery_required as i64,
                delivery.run_id,
                operation_id
            ],
        )?;
        Ok(changed == 1)
    }

    /// Atomically claims the run's delivery operation: exactly one caller can
    /// move an idle/failed record into `pushing` with the given operation
    /// identity. SQLite serializes the compare-and-set, so concurrent
    /// connections cannot both win. Returns the claimed record, or `None`
    /// when another state (active, interrupted, or published) owns the row.
    pub fn claim_delivery(
        &self,
        run_id: i64,
        operation_id: &str,
        repository: &str,
        remote: &str,
        base_branch: &str,
    ) -> Result<Option<GitHubDelivery>> {
        let changed = self.conn.execute(
            "UPDATE github_deliveries
             SET state = 'pushing', operation_id = ?2, repository = ?3, remote = ?4,
                 base_branch = ?5, error = NULL, recovery_required = 0, pushed_head = NULL,
                 updated_at = ?6
             WHERE run_id = ?1
               AND state IN ('not_ready','ready','failed')
               AND pr_number IS NULL",
            params![run_id, operation_id, repository, remote, base_branch, now()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.get_delivery(run_id)?
            .ok_or_else(|| DbError::Corrupt(format!("delivery row for run {run_id} vanished")))
            .map(Some)
    }

    /// Records a definitive pre-operation failure on an idle delivery record
    /// (no push or PR creation was attempted). Conditional on the record
    /// still being idle so it can never overwrite an active operation or a
    /// newer outcome.
    pub fn mark_delivery_failed_if_idle(&self, run_id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE github_deliveries
             SET state = 'failed', error = ?2, recovery_required = 0, updated_at = ?3
             WHERE run_id = ?1 AND state IN ('not_ready','ready','failed') AND pr_number IS NULL",
            params![run_id, error, now()],
        )?;
        Ok(())
    }

    /// Demotes deliveries left in an active state by a previous process:
    /// single-writer project ownership means no live worker can hold them at
    /// startup, so their outcome is unknown until reconciled. Returns the
    /// affected run ids. Never pushes or creates anything.
    pub fn demote_interrupted_deliveries(&self) -> Result<Vec<i64>> {
        let timestamp = now();
        let mut statement = self.conn.prepare(
            "SELECT run_id FROM github_deliveries WHERE state IN ('pushing','creating_pr')",
        )?;
        let run_ids = statement
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        for run_id in &run_ids {
            self.conn.execute(
                "UPDATE github_deliveries
                 SET state = 'interrupted', recovery_required = 1,
                     error = 'Factory stopped before the delivery outcome was confirmed',
                     updated_at = ?1
                 WHERE run_id = ?2 AND state IN ('pushing','creating_pr')",
                params![timestamp, run_id],
            )?;
        }
        Ok(run_ids)
    }

    // --- Evaluation reads ------------------------------------------------------

    /// Every task attempt across all runs, ordered by task then attempt number.
    /// The evaluation engine derives agent performance from this immutable
    /// history; no separate event store is kept.
    pub fn list_task_attempts_all(&self) -> Result<Vec<TaskAttempt>> {
        let mut statement = self.conn.prepare(
            "SELECT id, task_id, attempt_number, agent, status, started_at,
                    finished_at, worktree_path, commit_sha, exit_code, error,
                    evidence, review, role, operation, source_base
             FROM task_attempts
             ORDER BY task_id, attempt_number",
        )?;
        let attempts = statement
            .query_map([], build_attempt)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(attempts)
    }

    /// Lean attempt projection for the evaluation engine (ASF-N06). The
    /// evaluator consumes identity, classification, timing, error text, and
    /// only `evidence.changed_files` — loading the full evidence JSON (which
    /// carries multi-kilobyte diff patches per attempt) made history reads
    /// grow hundreds of megabytes at tens of thousands of attempts. This
    /// query extracts the changed-file list SQL-side and leaves the other
    /// evidence fields and the review blob at their defaults; it is not a
    /// general-purpose read.
    pub fn list_task_attempts_for_evaluation(&self) -> Result<Vec<TaskAttempt>> {
        let mut statement = self.conn.prepare(
            "SELECT id, task_id, attempt_number, agent, status, started_at,
                    finished_at, exit_code, error, role, operation,
                    json_extract(evidence, '$.changedFiles')
             FROM task_attempts
             ORDER BY task_id, attempt_number",
        )?;
        let attempts = statement
            .query_map([], |row| {
                let changed_files_json: Option<String> = row.get(11)?;
                let changed_files = changed_files_json
                    .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok());
                Ok(TaskAttempt {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    attempt_number: row.get(2)?,
                    agent: row.get(3)?,
                    status: row
                        .get::<_, String>(4)?
                        .parse()
                        .unwrap_or(AttemptStatus::Failed),
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    worktree_path: String::new(),
                    commit_sha: None,
                    exit_code: row.get(7)?,
                    error: row.get(8)?,
                    evidence: changed_files.map(|files| TaskEvidence {
                        changed_files: files,
                        ..TaskEvidence::default()
                    }),
                    review: None,
                    role: row.get(9)?,
                    operation: row
                        .get::<_, Option<String>>(10)?
                        .and_then(|value| value.parse().ok()),
                    source_base: None,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(attempts)
    }

    /// Lean `agent_sessions` projection for the evaluation engine: timing and
    /// identity only, never session output. Only rows attached to a task
    /// attempt are returned (durations are attributed per attempt).
    pub fn list_session_durations(&self) -> Result<Vec<SessionDuration>> {
        let mut statement = self.conn.prepare(
            "SELECT id, attempt_id, agent, role, operation, mode, status, duration_ms
             FROM agent_sessions
             WHERE attempt_id IS NOT NULL
             ORDER BY id",
        )?;
        let rows = statement
            .query_map([], |r| {
                Ok(SessionDuration {
                    id: r.get(0)?,
                    attempt_id: r.get(1)?,
                    agent: r.get(2)?,
                    role: r.get(3)?,
                    operation: r
                        .get::<_, Option<String>>(4)?
                        .and_then(|value| value.parse().ok()),
                    mode: r
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or(AgentSessionMode::Automated),
                    status: r.get(6)?,
                    duration_ms: r.get(7).map(|v: Option<i64>| v.map(|v| v as u64))?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Records how one approved attempt landed on the run's integration
    /// branch. Written for every integration attempt, including conflicts.
    pub fn record_integration_outcome(
        &self,
        run_id: i64,
        task_id: i64,
        attempt_id: i64,
        agent: &str,
        outcome: IntegrationOutcomeKind,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO integration_outcomes (run_id, task_id, attempt_id, agent, outcome, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![run_id, task_id, attempt_id, agent, outcome.as_str(), now()],
        )?;
        Ok(())
    }

    /// All recorded integration outcomes, oldest first.
    pub fn list_integration_outcomes(&self) -> Result<Vec<IntegrationOutcome>> {
        let mut statement = self.conn.prepare(
            "SELECT id, run_id, task_id, attempt_id, agent, outcome, created_at
             FROM integration_outcomes
             ORDER BY id",
        )?;
        let rows = statement
            .query_map([], |r| {
                Ok(IntegrationOutcome {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    task_id: r.get(2)?,
                    attempt_id: r.get(3)?,
                    agent: r.get(4)?,
                    outcome: r
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or(IntegrationOutcomeKind::Conflict),
                    created_at: r.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn reconcile_interrupted(&self) -> Result<Reconciliation> {
        let timestamp = now();
        self.conn.execute(
            "UPDATE tool_grants SET revoked_at = ?1 WHERE revoked_at IS NULL",
            params![timestamp],
        )?;
        self.conn.execute(
            "UPDATE tool_invocations
             SET status = 'cancelled', duration_ms = COALESCE(duration_ms, 0),
                 error = COALESCE(error, 'Factory stopped during MCP operation')
             WHERE status = 'running'",
            [],
        )?;
        self.conn.execute("DELETE FROM mcp_server_leases", [])?;
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
        // A task interrupted mid-integration retries the lane from its
        // awaiting_integration spot on the next start; the run branch head is
        // untouched because integration commits only land after approval.
        let requeued_integrations = self.conn.execute(
            "UPDATE tasks SET state = 'awaiting_integration', updated_at = ?1
             WHERE state = 'integrating'",
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
            requeued_integrations,
            runs,
        })
    }
}

/// A run plus its task-state counts, produced by one bounded summary query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummaryRecord {
    pub run: Run,
    pub counts: TaskStateCounts,
}

/// Per-state task counts for one run. Mirrors the API summary contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TaskStateCounts {
    pub pending: usize,
    pub ready: usize,
    pub running: usize,
    pub awaiting_integration: usize,
    pub integrating: usize,
    pub blocked: usize,
    pub failed: usize,
    pub completed: usize,
    pub superseded: usize,
    pub total: usize,
}

impl TaskStateCounts {
    fn record(&mut self, state: TaskState, count: usize) {
        match state {
            TaskState::Pending => self.pending += count,
            TaskState::Ready => self.ready += count,
            TaskState::Running => self.running += count,
            TaskState::AwaitingIntegration => self.awaiting_integration += count,
            TaskState::Integrating => self.integrating += count,
            TaskState::Blocked => self.blocked += count,
            TaskState::Failed => self.failed += count,
            TaskState::Completed => self.completed += count,
            TaskState::Superseded => self.superseded += count,
        }
        self.total += count;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reconciliation {
    pub sessions: usize,
    pub attempts: usize,
    pub tasks: usize,
    /// Integrating tasks reset to awaiting_integration on restart.
    pub requeued_integrations: usize,
    pub runs: usize,
}

/// Lean `agent_sessions` row used by the evaluation engine to attribute
/// durations to attempts without loading session output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDuration {
    pub id: i64,
    pub attempt_id: i64,
    pub agent: String,
    pub role: String,
    pub operation: Option<TaskOperation>,
    pub mode: AgentSessionMode,
    pub status: String,
    pub duration_ms: Option<u64>,
}

fn build_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let team: Option<String> = r.get(6)?;
    let recipe: Option<String> = r.get(8)?;
    Ok(Run {
        id: r.get(0)?,
        objective: r.get(1)?,
        status: run_status(r.get::<_, String>(2)?),
        planner_agent: r.get(3)?,
        team: team.and_then(|value| serde_json::from_str(&value).ok()),
        plan_revision: r.get(7)?,
        recipe: recipe.and_then(|value| serde_json::from_str(&value).ok()),
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

/// Raw `github_links` row; JSON columns are decoded in [`Self::into_link`]
/// where errors can surface as [`DbError::Json`].
struct RawGithubLink {
    provider: String,
    repository: String,
    issue_number: i64,
    issue_url: String,
    issue_title: String,
    issue_body: String,
    issue_state: String,
    issue_author: String,
    issue_labels: String,
    issue_comments: String,
    imported_at: String,
}

impl RawGithubLink {
    fn into_link(self) -> std::result::Result<GitHubIssueLink, serde_json::Error> {
        Ok(GitHubIssueLink {
            provider: self.provider,
            repository: self.repository,
            issue_number: self.issue_number,
            issue_url: self.issue_url,
            issue_title: self.issue_title,
            issue_body: self.issue_body,
            issue_state: self.issue_state,
            issue_author: self.issue_author,
            issue_labels: serde_json::from_str(&self.issue_labels)?,
            issue_comments: serde_json::from_str(&self.issue_comments)?,
            imported_at: self.imported_at,
        })
    }
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
        agent_override: r.get(12)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

fn build_routing_decision(r: &rusqlite::Row<'_>) -> rusqlite::Result<RoutingDecision> {
    let scores_json: Option<String> = r.get(8)?;
    let candidate_scores = scores_json
        .and_then(|json| serde_json::from_str::<Vec<RoutingCandidateScore>>(&json).ok())
        .unwrap_or_default();
    Ok(RoutingDecision {
        id: r.get(0)?,
        task_id: r.get(1)?,
        attempt_id: r.get(2)?,
        mode: r.get(3)?,
        selected_agent: r.get(4)?,
        role: r.get(5)?,
        operation: r
            .get::<_, Option<String>>(6)?
            .and_then(|value| value.parse().ok()),
        language: r.get(7)?,
        candidate_scores,
        reason: r.get(9)?,
        created_at: r.get(10)?,
    })
}

fn build_plan_revision(r: &rusqlite::Row<'_>) -> rusqlite::Result<PlanRevisionRecord> {
    let snapshot: String = r.get(6)?;
    Ok(PlanRevisionRecord {
        id: r.get(0)?,
        run_id: r.get(1)?,
        revision: r.get(2)?,
        source: r
            .get::<_, String>(3)?
            .parse()
            .unwrap_or(PlanRevisionSource::Planner),
        reason: r.get(4)?,
        planner_session_id: r.get(5)?,
        snapshot: serde_json::from_str(&snapshot).unwrap_or_else(|_| PlanSnapshot {
            objective: String::new(),
            tasks: Vec::new(),
        }),
        created_at: r.get(7)?,
    })
}

/// Loads the run's current plan (revision, objective, full task graph with
/// dependencies and attempt counts) for validation and resolution.
fn load_plan_state(conn: &Connection, run_id: i64) -> Result<PlanState> {
    let objective = conn
        .query_row(
            "SELECT objective FROM runs WHERE id = ?1",
            params![run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(DbError::NotFound("run"))?;
    let revision = conn.query_row(
        "SELECT plan_revision FROM runs WHERE id = ?1",
        params![run_id],
        |row| row.get::<_, i64>(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, run_id, title, objective, acceptance_criteria, state, position, worktree_path, created_at, updated_at, role, operation, agent_override
         FROM tasks WHERE run_id = ?1 ORDER BY position",
    )?;
    let mut tasks: Vec<Task> = stmt
        .query_map(params![run_id], build_task)?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut dep_stmt = conn.prepare(
        "SELECT td.task_id, td.depends_on
         FROM task_dependencies td JOIN tasks t ON t.id = td.task_id
         WHERE t.run_id = ?1 ORDER BY td.task_id",
    )?;
    let mut deps: HashMap<i64, Vec<i64>> = HashMap::new();
    let dep_rows = dep_stmt.query_map(params![run_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in dep_rows {
        let (task_id, depends_on) = row?;
        deps.entry(task_id).or_default().push(depends_on);
    }
    for task in &mut tasks {
        task.dependencies = deps.remove(&task.id).unwrap_or_default();
    }

    let mut attempt_stmt = conn.prepare(
        "SELECT a.task_id, COUNT(*)
         FROM task_attempts a JOIN tasks t ON t.id = a.task_id
         WHERE t.run_id = ?1 GROUP BY a.task_id",
    )?;
    let mut attempts: HashMap<i64, usize> = HashMap::new();
    let attempt_rows = attempt_stmt.query_map(params![run_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? as usize))
    })?;
    for row in attempt_rows {
        let (task_id, count) = row?;
        attempts.insert(task_id, count);
    }

    Ok(PlanState {
        revision,
        objective,
        tasks,
        attempts,
    })
}

/// Persists a fully resolved plan as the run's new revision: mutates tasks,
/// supersedes/removes scope, rewrites the dependency graph, records the
/// snapshot in `plan_revisions`, and reconciles the run status.
#[allow(clippy::too_many_arguments)]
fn apply_resolved(
    tx: &rusqlite::Transaction,
    run_id: i64,
    state: &PlanState,
    resolved: &ResolvedPlan,
    source: PlanRevisionSource,
    reason: Option<&str>,
    planner_session_id: Option<i64>,
) -> Result<(i64, Vec<Task>)> {
    let ts = now();
    let mut id_map: HashMap<i64, i64> = HashMap::new();

    for task in &resolved.tasks {
        if !task.insert {
            continue;
        }
        let criteria = serde_json::to_string(&task.acceptance_criteria)?;
        tx.execute(
            "INSERT INTO tasks (run_id, title, objective, acceptance_criteria, state, position, role, operation, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                run_id,
                task.title,
                task.objective,
                criteria,
                task.state.as_str(),
                task.position,
                task.role,
                task.operation.map(TaskOperation::as_str),
                ts,
                ts
            ],
        )?;
        id_map.insert(task.id, tx.last_insert_rowid());
    }

    for task in &resolved.tasks {
        if task.insert {
            continue;
        }
        let criteria = serde_json::to_string(&task.acceptance_criteria)?;
        tx.execute(
            "UPDATE tasks SET title = ?1, objective = ?2, acceptance_criteria = ?3, state = ?4, position = ?5, role = ?6, operation = ?7, updated_at = ?8
             WHERE id = ?9",
            params![
                task.title,
                task.objective,
                criteria,
                task.state.as_str(),
                task.position,
                task.role,
                task.operation.map(TaskOperation::as_str),
                ts,
                task.id
            ],
        )?;
    }

    for id in &resolved.superseded {
        tx.execute(
            "UPDATE tasks SET state = 'superseded', updated_at = ?1 WHERE id = ?2",
            params![ts, id],
        )?;
    }

    if !resolved.removed.is_empty() {
        let placeholders = vec!["?"; resolved.removed.len()].join(",");
        let sql = format!("DELETE FROM tasks WHERE id IN ({placeholders})");
        tx.execute(
            &sql,
            rusqlite::params_from_iter(resolved.removed.iter().copied()),
        )?;
    }

    for task in &resolved.tasks {
        let real_id = id_map.get(&task.id).copied().unwrap_or(task.id);
        tx.execute(
            "DELETE FROM task_dependencies WHERE task_id = ?1",
            params![real_id],
        )?;
        for dep in &task.dependencies {
            let real_dep = id_map.get(dep).copied().unwrap_or(*dep);
            tx.execute(
                "INSERT INTO task_dependencies (task_id, depends_on) VALUES (?1, ?2)",
                params![real_id, real_dep],
            )?;
        }
    }

    let mut final_tasks: Vec<Task> = Vec::new();
    for original in &state.tasks {
        if resolved.removed.contains(&original.id) {
            continue;
        }
        let mut task = original.clone();
        if resolved.superseded.contains(&original.id) {
            task.state = TaskState::Superseded;
            final_tasks.push(task);
            continue;
        }
        if let Some(rt) = resolved
            .tasks
            .iter()
            .find(|rt| !rt.insert && rt.id == original.id)
        {
            task.title = rt.title.clone();
            task.objective = rt.objective.clone();
            task.acceptance_criteria = rt.acceptance_criteria.clone();
            task.state = rt.state;
            task.position = rt.position;
            task.role = rt.role.clone();
            task.operation = rt.operation;
            task.dependencies = rt
                .dependencies
                .iter()
                .map(|d| id_map.get(d).copied().unwrap_or(*d))
                .collect();
            task.updated_at = ts.clone();
        }
        final_tasks.push(task);
    }
    for rt in resolved.tasks.iter().filter(|rt| rt.insert) {
        let real_id = id_map[&rt.id];
        final_tasks.push(Task {
            id: real_id,
            run_id,
            title: rt.title.clone(),
            objective: rt.objective.clone(),
            acceptance_criteria: rt.acceptance_criteria.clone(),
            state: rt.state,
            position: rt.position,
            dependencies: rt
                .dependencies
                .iter()
                .map(|d| id_map.get(d).copied().unwrap_or(*d))
                .collect(),
            worktree_path: None,
            role: rt.role.clone(),
            operation: rt.operation,
            agent_override: None,
            created_at: ts.clone(),
            updated_at: ts.clone(),
        });
    }

    let status = RunStatus::from_tasks(&final_tasks);
    let current: String = tx.query_row(
        "SELECT status FROM runs WHERE id = ?1",
        params![run_id],
        |row| row.get(0),
    )?;
    if !matches!(current.as_str(), "planning" | "cancelled") {
        tx.execute(
            "UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.as_str(), ts, run_id],
        )?;
    }
    tx.execute(
        "UPDATE runs SET objective = ?1, plan_revision = ?2, updated_at = ?3 WHERE id = ?4",
        params![resolved.objective, resolved.revision, ts, run_id],
    )?;

    let snapshot = serde_json::to_string(&PlanSnapshot {
        objective: resolved.objective.clone(),
        tasks: final_tasks.clone(),
    })?;
    tx.execute(
        "INSERT INTO plan_revisions (run_id, revision, source, reason, planner_session_id, snapshot, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            run_id,
            resolved.revision,
            source.as_str(),
            reason,
            planner_session_id,
            snapshot,
            ts
        ],
    )?;

    Ok((resolved.revision, final_tasks))
}

fn build_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
    let policy_audit: Option<String> = r.get(16)?;
    let output_meta: Option<String> = r.get(17)?;
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
        policy_audit: policy_audit.and_then(|value| serde_json::from_str(&value).ok()),
        output_meta: output_meta.and_then(|value| serde_json::from_str(&value).ok()),
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
        source_base: r.get(15)?,
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

/// Tracks the head of the per-run integration branch (`factory/run-<id>`):
/// the latest commit sha integrated from approved implementation work.
/// `NULL` until the first implementation task is approved.
const V7_SCHEMA: &str = "ALTER TABLE runs ADD COLUMN integration_sha TEXT;";

/// Parallel runtime: the immutable base commit a run started from. The
/// `factory/run-<id>` branch is created at this commit when the run starts.
const V8_SCHEMA: &str = "ALTER TABLE runs ADD COLUMN base_sha TEXT;";

/// Parallel runtime: the commit each task attempt's worktree was based on.
/// The integration lane compares this with the run branch head to detect
/// stale bases and replay/rebase before merging.
const V9_SCHEMA: &str = "ALTER TABLE task_attempts ADD COLUMN source_base TEXT;";

/// Visual plan editor: durable plan revisions with optimistic concurrency.
/// `runs.plan_revision` is bumped on every plan change (planner, manual edit,
/// replan); `plan_revisions` records each revision with a full JSON snapshot
/// of the objective and task list.
const V10_SCHEMA: &str = "
ALTER TABLE runs ADD COLUMN plan_revision INTEGER NOT NULL DEFAULT 1;
CREATE TABLE plan_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    source TEXT NOT NULL,
    reason TEXT,
    planner_session_id INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
    snapshot TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(run_id, revision)
);
CREATE INDEX idx_plan_revisions_run ON plan_revisions(run_id, revision);
";

/// Policy engine: which policy applied to an automated AgentSession. The
/// audit keeps compact *metadata* (source, mode, write scopes) only — never
/// secret values.
const V11_SCHEMA: &str = "ALTER TABLE agent_sessions ADD COLUMN policy_audit TEXT;";

/// GitHub integration: the imported Issue a run was seeded from (untrusted
/// external context, persisted so the workflow survives GitHub outages), and
/// the delivery record that prevents duplicate pull requests across restarts.
const V12_SCHEMA: &str = "
CREATE TABLE github_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    repository TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    issue_url TEXT,
    issue_title TEXT NOT NULL,
    issue_body TEXT NOT NULL,
    issue_state TEXT,
    issue_author TEXT,
    issue_labels TEXT NOT NULL DEFAULT '[]',
    issue_comments TEXT NOT NULL DEFAULT '[]',
    imported_at TEXT NOT NULL
);
CREATE TABLE github_deliveries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'not_ready',
    repository TEXT,
    remote TEXT,
    base_branch TEXT,
    head_branch TEXT NOT NULL,
    pushed_head TEXT,
    pr_number INTEGER,
    pr_url TEXT,
    pr_state TEXT,
    pr_is_draft INTEGER,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_github_links_issue ON github_links(repository, issue_number);
";

/// Evaluation: how each approved attempt landed on the run's integration
/// branch (`clean` fast-forward, `rebased` from a stale base, or `conflict`).
/// Integration outcomes are downstream quality signals kept separate from
/// agent-quality metrics; a conflict is not automatically an agent failure.
const V13_SCHEMA: &str = "
CREATE TABLE integration_outcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER NOT NULL REFERENCES task_attempts(id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    outcome TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_integration_outcomes_task ON integration_outcomes(task_id);
CREATE INDEX idx_integration_outcomes_attempt ON integration_outcomes(attempt_id);
";

/// Routing: per-dispatch audit records explaining why an agent was selected,
/// plus the per-task manual routing override pinned from the Task Inspector.
const V14_SCHEMA: &str = "
CREATE TABLE routing_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL,
    mode TEXT NOT NULL,
    selected_agent TEXT NOT NULL,
    role TEXT,
    operation TEXT,
    language TEXT,
    candidate_scores TEXT,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_routing_decisions_task ON routing_decisions(task_id);
CREATE INDEX idx_routing_decisions_attempt ON routing_decisions(attempt_id);
ALTER TABLE tasks ADD COLUMN agent_override TEXT;
";

/// Recipes: the immutable snapshot of the recipe a run was created from
/// (selected roles, routing/concurrency preferences, policy preset
/// references, Planner guidance). Stored as JSON next to `team`; editing or
/// deleting the source recipe never mutates existing runs.
const V15_SCHEMA: &str = "ALTER TABLE runs ADD COLUMN recipe TEXT;";

/// External Tools / MCP: lightweight discovery metadata, scoped capability
/// grants (hash only), compact invocation audit, and short per-server leases.
const V16_SCHEMA: &str = "
CREATE TABLE mcp_tools (
    server_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    description TEXT NOT NULL,
    input_schema TEXT NOT NULL,
    annotations TEXT,
    classification TEXT NOT NULL,
    discovered_at TEXT NOT NULL,
    PRIMARY KEY (server_id, tool_name)
);
CREATE INDEX idx_mcp_tools_server ON mcp_tools(server_id);
CREATE TABLE tool_grants (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    run_id INTEGER REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE CASCADE,
    session_id INTEGER REFERENCES agent_sessions(id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    role TEXT NOT NULL,
    operation TEXT,
    allowed_tools TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);
CREATE INDEX idx_tool_grants_session ON tool_grants(session_id);
CREATE TABLE tool_invocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    task_id INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL,
    session_id INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
    agent TEXT NOT NULL,
    role TEXT NOT NULL,
    server TEXT NOT NULL,
    tool TEXT NOT NULL,
    classification TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    duration_ms INTEGER,
    input_summary TEXT NOT NULL,
    output_summary TEXT,
    error TEXT
);
CREATE INDEX idx_tool_invocations_run ON tool_invocations(run_id);
CREATE INDEX idx_tool_invocations_task ON tool_invocations(task_id);
CREATE INDEX idx_tool_invocations_agent ON tool_invocations(agent);
CREATE TABLE mcp_server_leases (
    server_id TEXT PRIMARY KEY,
    active_calls INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
";

/// Bounded agent output accounting: retained/emitted byte counts and explicit
/// truncation flags for each stream. Stored separately from process output so
/// legacy rows and interactive sessions remain valid.
const V17_SCHEMA: &str = "ALTER TABLE agent_sessions ADD COLUMN output_meta TEXT;";

/// ASF-18: durable identity and recovery flags for GitHub delivery
/// operations. `operation_id` scopes every state transition to the attempt
/// that wrote it, so a stale worker cannot overwrite a newer outcome;
/// `recovery_required` marks attempts whose remote outcome is unknown.
const V18_SCHEMA: &str = "
ALTER TABLE github_deliveries ADD COLUMN operation_id TEXT;
ALTER TABLE github_deliveries ADD COLUMN recovery_required INTEGER NOT NULL DEFAULT 0;
";

const MIGRATIONS: &[&str] = &[
    V1_SCHEMA, V2_SCHEMA, V3_SCHEMA, V4_SCHEMA, V5_SCHEMA, V6_SCHEMA, V7_SCHEMA, V8_SCHEMA,
    V9_SCHEMA, V10_SCHEMA, V11_SCHEMA, V12_SCHEMA, V13_SCHEMA, V14_SCHEMA, V15_SCHEMA, V16_SCHEMA,
    V17_SCHEMA, V18_SCHEMA,
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
        AgentOutputMetadata, AgentSession, AgentSessionMode, AttemptStatus, IntegrationOutcomeKind,
        Plan, PlanApplyOutcome, PlanMutation, PlanPatch, PlanRevisionSource, PlannedTask,
        ReplanRequest, ReviewDecision, ReviewResult, RunStatus, TaskEvidence, TaskOperation,
        TaskRef, TaskState, ToolClassification, ToolGrant, ToolInvocation,
    };
    use rusqlite::Connection;
    use tempfile::TempDir;

    use crate::{DbError, FactoryDb, TaskStateCounts, V1_SCHEMA};

    #[test]
    fn applies_all_migrations_exactly_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let db = FactoryDb::open(&path).unwrap();
        let versions = schema_versions(&path);
        assert_eq!(
            versions,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]
        );
        db.create_run("objective", Some("codex")).unwrap();
        drop(db);

        let db = FactoryDb::open(&path).unwrap();
        assert_eq!(
            schema_versions(&path),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]
        );
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
        assert_eq!(
            schema_versions(&path),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]
        );
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
        // opening records versions 6 and 7 exactly once
        assert_eq!(
            schema_versions(&path),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]
        );
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
    fn records_and_reads_run_integration() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        assert_eq!(db.get_run_integration(run.id).unwrap(), None);

        db.set_run_integration(run.id, Some("abc123")).unwrap();
        assert_eq!(
            db.get_run_integration(run.id).unwrap(),
            Some("abc123".to_string())
        );

        db.set_run_integration(run.id, None).unwrap();
        assert_eq!(db.get_run_integration(run.id).unwrap(), None);
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
    fn run_summary_page_aggregates_counts_and_handles_empty_runs() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let empty = db.create_run("empty", Some("codex")).unwrap();
        let mixed = db.create_run("mixed", Some("codex")).unwrap();
        let states = [
            TaskState::Pending,
            TaskState::Ready,
            TaskState::Running,
            TaskState::AwaitingIntegration,
            TaskState::Integrating,
            TaskState::Blocked,
            TaskState::Failed,
            TaskState::Completed,
            TaskState::Superseded,
            TaskState::Completed,
        ];
        for (position, state) in states.into_iter().enumerate() {
            db.create_task(
                mixed.id,
                &format!("task {position}"),
                "objective",
                &[],
                state,
                position as i32,
                None,
                None,
            )
            .unwrap();
        }

        let records = db.list_run_summaries_page(10, None).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].run.id, mixed.id);
        assert_eq!(
            records[0].counts,
            TaskStateCounts {
                pending: 1,
                ready: 1,
                running: 1,
                awaiting_integration: 1,
                integrating: 1,
                blocked: 1,
                failed: 1,
                completed: 2,
                superseded: 1,
                total: 10,
            }
        );
        assert_eq!(records[1].run.id, empty.id);
        assert_eq!(records[1].counts, TaskStateCounts::default());
    }

    #[test]
    fn run_summary_page_uses_an_exclusive_newest_first_cursor() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        for index in 0..6 {
            let run = db.create_run(&format!("run {index}"), None).unwrap();
            db.create_task(
                run.id,
                "task",
                "objective",
                &[],
                TaskState::Ready,
                0,
                None,
                None,
            )
            .unwrap();
        }

        let first = db.list_run_summaries_page(2, None).unwrap();
        assert_eq!(
            first.iter().map(|record| record.run.id).collect::<Vec<_>>(),
            vec![6, 5]
        );

        let second = db
            .list_run_summaries_page(2, Some(first[1].run.id))
            .unwrap();
        assert_eq!(
            second
                .iter()
                .map(|record| record.run.id)
                .collect::<Vec<_>>(),
            vec![4, 3]
        );

        let third = db
            .list_run_summaries_page(2, Some(second[1].run.id))
            .unwrap();
        assert_eq!(
            third.iter().map(|record| record.run.id).collect::<Vec<_>>(),
            vec![2, 1]
        );
        assert!(db
            .list_run_summaries_page(2, Some(third[1].run.id))
            .unwrap()
            .is_empty());

        let all = db.list_run_summaries().unwrap();
        assert_eq!(
            all.iter().map(|record| record.run.id).collect::<Vec<_>>(),
            vec![6, 5, 4, 3, 2, 1]
        );
    }

    #[test]
    fn run_summary_pages_do_not_duplicate_or_skip_when_runs_change() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        for index in 0..5 {
            db.create_run(&format!("run {index}"), None).unwrap();
        }

        let first = db.list_run_summaries_page(2, None).unwrap();
        assert_eq!(
            first.iter().map(|record| record.run.id).collect::<Vec<_>>(),
            vec![5, 4]
        );

        let inserted = db.create_run("new run", None).unwrap();
        assert!(inserted.id > first[0].run.id);
        let second = db
            .list_run_summaries_page(2, Some(first[1].run.id))
            .unwrap();
        assert_eq!(
            second
                .iter()
                .map(|record| record.run.id)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(second.iter().all(|record| record.run.id < first[1].run.id));

        let deleted = second[0].run.id;
        db.conn
            .execute("DELETE FROM runs WHERE id = ?1", rusqlite::params![deleted])
            .unwrap();
        let third = db
            .list_run_summaries_page(2, Some(second[1].run.id))
            .unwrap();
        assert_eq!(
            third.iter().map(|record| record.run.id).collect::<Vec<_>>(),
            vec![1]
        );
        assert!(third.iter().all(|record| record.run.id < second[1].run.id));
    }

    #[test]
    fn run_summary_page_stays_bounded_for_large_histories() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        for index in 0..1_000 {
            let run = db
                .create_run(&format!("run {index}"), Some("codex"))
                .unwrap();
            db.create_task(
                run.id,
                "task",
                "objective",
                &[],
                if index % 2 == 0 {
                    TaskState::Completed
                } else {
                    TaskState::Pending
                },
                0,
                None,
                None,
            )
            .unwrap();
        }

        let page = db.list_run_summaries_page(25, None).unwrap();
        assert_eq!(page.len(), 25);
        assert_eq!(page[0].run.id, 1_000);
        assert_eq!(page[24].run.id, 976);
        assert!(page
            .iter()
            .all(|record| record.counts.total == 1 && record.counts.completed <= 1));
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
            policy_audit: None,
            output_meta: Some(AgentOutputMetadata {
                limit_bytes: 1_048_576,
                stdout_bytes: 1_200_000,
                stderr_bytes: 0,
                stdout_dropped_bytes: 151_424,
                stderr_dropped_bytes: 0,
                stdout_truncated: true,
                stderr_truncated: false,
            }),
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
    fn plan_revisions_record_every_change_with_concurrency_guard() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db
            .create_run_with_status("draft", Some("planner"), RunStatus::Planning)
            .unwrap();
        let tasks = db
            .persist_plan(
                run.id,
                &Plan {
                    objective: "objective".into(),
                    tasks: vec![PlannedTask {
                        id: "A".into(),
                        title: "First".into(),
                        objective: "first".into(),
                        dependencies: Vec::new(),
                        acceptance_criteria: vec!["done".into()],
                        role: None,
                        operation: Some(TaskOperation::Implement),
                    }],
                },
            )
            .unwrap();

        assert_eq!(db.get_plan_revision(run.id).unwrap(), 1);
        let state = db.plan_state(run.id).unwrap();
        assert_eq!(state.revision, 1);
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.tasks[0].id, tasks[0].id);

        let outcome = db
            .apply_plan_patch(
                run.id,
                &PlanPatch {
                    expected_revision: 1,
                    mutations: vec![PlanMutation::AddTask {
                        client_id: "draft-B".into(),
                        title: "Second".into(),
                        objective: "second".into(),
                        acceptance_criteria: vec!["reviewed".into()],
                        dependencies: vec![TaskRef::Id(tasks[0].id)],
                        role: Some("database_engineer".into()),
                        operation: Some(TaskOperation::Implement),
                    }],
                },
                PlanRevisionSource::Manual,
                Some("edit session"),
                None,
            )
            .unwrap();
        let PlanApplyOutcome::Applied {
            run_id,
            revision,
            tasks,
        } = outcome
        else {
            panic!("expected applied");
        };
        assert_eq!(run_id, run.id);
        assert_eq!(revision, 2);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1].title, "Second");
        assert_eq!(tasks[1].dependencies, vec![tasks[0].id]);
        assert_eq!(tasks[1].role.as_deref(), Some("database_engineer"));

        assert_eq!(db.get_plan_revision(run.id).unwrap(), 2);
        let state = db.plan_state(run.id).unwrap();
        assert_eq!(state.revision, 2);
        assert_eq!(state.tasks.len(), 2);

        let revisions = db.list_plan_revisions(run.id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].revision, 1);
        assert_eq!(revisions[0].source, PlanRevisionSource::Planner);
        assert_eq!(revisions[0].reason, None);
        assert_eq!(revisions[1].revision, 2);
        assert_eq!(revisions[1].source, PlanRevisionSource::Manual);
        assert_eq!(revisions[1].reason.as_deref(), Some("edit session"));
        assert_eq!(revisions[1].snapshot.tasks.len(), 2);

        let record = db.get_plan_revision_record(run.id, 2).unwrap().unwrap();
        assert_eq!(record.revision, 2);
        assert_eq!(record.snapshot.objective, "objective");
        assert!(!record.snapshot.tasks.is_empty());

        let stale = db
            .apply_plan_patch(
                run.id,
                &PlanPatch {
                    expected_revision: 1,
                    mutations: vec![PlanMutation::RemoveTask {
                        task: TaskRef::Id(tasks[0].id),
                    }],
                },
                PlanRevisionSource::Manual,
                None,
                None,
            )
            .unwrap_err();
        assert!(matches!(
            stale,
            DbError::Conflict {
                expected: 1,
                current: 2
            }
        ));
    }

    #[test]
    fn invalid_plan_patch_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db
            .create_run_with_status("draft", Some("planner"), RunStatus::Planning)
            .unwrap();
        let tasks = db
            .persist_plan(
                run.id,
                &Plan {
                    objective: "objective".into(),
                    tasks: vec![PlannedTask {
                        id: "A".into(),
                        title: "First".into(),
                        objective: "first".into(),
                        dependencies: Vec::new(),
                        acceptance_criteria: vec!["done".into()],
                        role: None,
                        operation: Some(TaskOperation::Implement),
                    }],
                },
            )
            .unwrap();

        db.set_task_state(tasks[0].id, TaskState::Running).unwrap();

        let outcome = db
            .apply_plan_patch(
                run.id,
                &PlanPatch {
                    expected_revision: 1,
                    mutations: vec![PlanMutation::RemoveTask {
                        task: TaskRef::Id(tasks[0].id),
                    }],
                },
                PlanRevisionSource::Manual,
                None,
                None,
            )
            .unwrap();
        assert!(matches!(outcome, PlanApplyOutcome::Invalid(_)));

        assert_eq!(db.get_plan_revision(run.id).unwrap(), 1);
        assert_eq!(db.plan_state(run.id).unwrap().tasks.len(), 1);
    }

    #[test]
    fn replan_supersedes_scope_and_bumps_revision() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db
            .create_run_with_status("draft", Some("planner"), RunStatus::Planning)
            .unwrap();
        let tasks = db
            .persist_plan(
                run.id,
                &Plan {
                    objective: "objective".into(),
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
                            id: "C".into(),
                            title: "Third".into(),
                            objective: "third".into(),
                            dependencies: vec!["A".into()],
                            acceptance_criteria: vec!["done".into()],
                            role: None,
                            operation: Some(TaskOperation::Implement),
                        },
                    ],
                },
            )
            .unwrap();

        db.create_task_attempt(
            tasks[0].id,
            "worker",
            Some(TaskOperation::Implement),
            "opencode",
            "worktree",
            None,
        )
        .unwrap();

        let outcome = db
            .apply_replan(
                run.id,
                &ReplanRequest {
                    expected_revision: 1,
                    seed: TaskRef::Id(tasks[0].id),
                    reason: Some("scope changed".into()),
                    plan: Plan {
                        objective: "new objective".into(),
                        tasks: vec![
                            PlannedTask {
                                id: "A".into(),
                                title: "First (revised)".into(),
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
                                role: None,
                                operation: Some(TaskOperation::Implement),
                            },
                        ],
                    },
                },
                PlanRevisionSource::Replan,
                Some("replanned"),
                None,
            )
            .unwrap();
        let PlanApplyOutcome::Applied { revision, .. } = outcome else {
            panic!("expected applied");
        };
        assert_eq!(revision, 2);
        assert_eq!(db.get_plan_revision(run.id).unwrap(), 2);
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().objective,
            "new objective"
        );

        let state = db.plan_state(run.id).unwrap();
        assert_eq!(state.tasks.len(), 4);
        let old_a = state.tasks.iter().find(|t| t.id == tasks[0].id).unwrap();
        assert_eq!(old_a.state, TaskState::Ready);
        assert_eq!(old_a.title, "First");
        let superseded_c = state.tasks.iter().find(|t| t.title == "Third").unwrap();
        assert_eq!(superseded_c.state, TaskState::Superseded);
        let new_a = state
            .tasks
            .iter()
            .find(|t| t.title == "First (revised)")
            .unwrap();
        assert_eq!(new_a.state, TaskState::Ready);
        assert_ne!(new_a.id, tasks[0].id);
        let b = state.tasks.iter().find(|t| t.title == "Second").unwrap();
        assert_eq!(b.state, TaskState::Pending);
        assert_eq!(b.dependencies, vec![new_a.id]);

        let revisions = db.list_plan_revisions(run.id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[1].source, PlanRevisionSource::Replan);
        assert_eq!(revisions[1].snapshot.tasks.len(), 4);
        assert_eq!(
            revisions[1]
                .snapshot
                .tasks
                .iter()
                .find(|t| t.title == "Third")
                .unwrap()
                .state,
            TaskState::Superseded
        );
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
            .create_task_attempt(
                task,
                "worker",
                None,
                "opencode",
                "worktree",
                Some("base123"),
            )
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
        assert_eq!(loaded.source_base.as_deref(), Some("base123"));
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
    fn persists_run_recipe_snapshot_and_keeps_old_runs_without_one() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        // Runs created before recipes exist deserialize with no snapshot.
        assert!(db.get_run(run.id).unwrap().unwrap().recipe.is_none());

        let snapshot = factory_types::RecipeSnapshot {
            id: "security".into(),
            name: "Security Sensitive".into(),
            roles: vec!["planner".into(), "worker".into(), "security_auditor".into()],
            routing: factory_types::RecipeRouting::RoundRobin,
            max_parallel_tasks: Some(2),
            policy_presets: std::collections::BTreeMap::from([(
                "worker".to_string(),
                "implementation".to_string(),
            )]),
            planner_guidance: "Keep the plan proportional.".into(),
            tools: Vec::new(),
            github: None,
        };
        db.set_run_recipe(run.id, &snapshot).unwrap();
        assert_eq!(db.get_run(run.id).unwrap().unwrap().recipe, Some(snapshot));
        assert!(db
            .list_runs()
            .unwrap()
            .iter()
            .any(|listed| listed.recipe.is_some()));
        // Another run stays untouched.
        let other = db.create_run("other", Some("codex")).unwrap();
        assert!(db.get_run(other.id).unwrap().unwrap().recipe.is_none());
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
        db.create_task_attempt(task, "worker", None, "opencode", "worktree", None)
            .unwrap();
        db.create_task_attempt(task, "worker", None, "qwen", "worktree", None)
            .unwrap();
        db.create_task_attempt(other_task, "worker", None, "claude", "worktree", None)
            .unwrap();
        assert_eq!(db.count_task_attempts(run.id).unwrap(), 2);
        assert_eq!(db.count_task_attempts(other.id).unwrap(), 1);
    }

    #[test]
    fn evaluation_reads_cover_attempts_sessions_and_integration_outcomes() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        let task = db
            .create_task(
                run.id,
                "Task",
                "objective",
                &[],
                TaskState::Completed,
                0,
                Some("worker"),
                Some(TaskOperation::Implement),
            )
            .unwrap();
        let attempt = db
            .create_task_attempt(
                task,
                "worker",
                Some(TaskOperation::Implement),
                "opencode",
                "worktree",
                None,
            )
            .unwrap();
        db.insert_agent_session(&AgentSession {
            id: 0,
            run_id: Some(run.id),
            task_id: Some(task),
            attempt_id: Some(attempt.id),
            role: "worker".into(),
            operation: Some(TaskOperation::Implement),
            agent: "opencode".into(),
            mode: AgentSessionMode::Automated,
            command: "opencode run".into(),
            status: "success".into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: Some("2026-01-01T00:01:00Z".into()),
            exit_code: Some(0),
            duration_ms: Some(60_000),
            stdout: Some("large output".into()),
            stderr: None,
            policy_audit: None,
            output_meta: None,
        })
        .unwrap();
        // Unattached planner sessions must not appear in the lean projection.
        db.insert_agent_session(&AgentSession {
            id: 0,
            run_id: Some(run.id),
            task_id: None,
            attempt_id: None,
            role: "planner".into(),
            operation: Some(TaskOperation::Planning),
            agent: "codex".into(),
            mode: AgentSessionMode::Automated,
            command: "codex exec".into(),
            status: "success".into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: Some("2026-01-01T00:00:30Z".into()),
            exit_code: Some(0),
            duration_ms: Some(30_000),
            stdout: None,
            stderr: None,
            policy_audit: None,
            output_meta: None,
        })
        .unwrap();
        db.record_integration_outcome(
            run.id,
            task,
            attempt.id,
            "opencode",
            IntegrationOutcomeKind::Rebased,
        )
        .unwrap();

        let attempts = db.list_task_attempts_all().unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].agent, "opencode");
        assert_eq!(attempts[0].operation, Some(TaskOperation::Implement));

        let durations = db.list_session_durations().unwrap();
        assert_eq!(durations.len(), 1, "only attempt-attached sessions");
        assert_eq!(durations[0].attempt_id, attempt.id);
        assert_eq!(durations[0].duration_ms, Some(60_000));
        assert_eq!(durations[0].operation, Some(TaskOperation::Implement));

        let outcomes = db.list_integration_outcomes().unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].agent, "opencode");
        assert_eq!(outcomes[0].outcome, IntegrationOutcomeKind::Rebased);
        assert_eq!(outcomes[0].task_id, task);
    }

    #[test]
    fn evaluation_projection_matches_the_full_rows_on_evaluator_fields() {
        // ASF-N06: the lean evaluation projection must agree with the full
        // rows on every field the evaluation engine consumes, while skipping
        // the payloads (diff patches, reviews) the metrics never read.
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        let task = db
            .create_task(
                run.id,
                "Task",
                "objective",
                &[],
                TaskState::Completed,
                0,
                Some("worker"),
                Some(TaskOperation::Implement),
            )
            .unwrap();
        let attempt = db
            .create_task_attempt(
                task,
                "worker",
                Some(TaskOperation::Implement),
                "opencode",
                "worktree",
                None,
            )
            .unwrap();
        db.finish_task_attempt(
            attempt.id,
            AttemptStatus::Approved,
            Some(0),
            Some("abc"),
            Some("boom"),
            Some(&TaskEvidence {
                changed_files: vec!["src/日本語.rs".to_string(), "lib/core.rs".to_string()],
                diff_summary: "summary".to_string(),
                commit_sha: Some("abc".to_string()),
                commands: vec!["cargo test".to_string()],
                acceptance_criteria: vec![],
                worker_exit_code: Some(0),
                artifacts: vec![],
                diff_patch: Some("@@ patch @@".repeat(4_000)),
            }),
            Some(&ReviewResult {
                decision: ReviewDecision::Approve,
                reason: "ok".to_string(),
                feedback: vec![],
            }),
        )
        .unwrap();
        // A second attempt without any evidence at all.
        db.create_task_attempt(task, "worker", None, "qwen", "worktree", None)
            .unwrap();

        let full = db.list_task_attempts_all().unwrap();
        let lean = db.list_task_attempts_for_evaluation().unwrap();
        assert_eq!(full.len(), lean.len());
        for (full, lean) in full.iter().zip(lean.iter()) {
            assert_eq!(lean.id, full.id);
            assert_eq!(lean.task_id, full.task_id);
            assert_eq!(lean.attempt_number, full.attempt_number);
            assert_eq!(lean.agent, full.agent);
            assert_eq!(lean.status, full.status);
            assert_eq!(lean.started_at, full.started_at);
            assert_eq!(lean.finished_at, full.finished_at);
            assert_eq!(lean.exit_code, full.exit_code);
            assert_eq!(lean.error, full.error);
            assert_eq!(lean.role, full.role);
            assert_eq!(lean.operation, full.operation);
            assert_eq!(
                lean.evidence.as_ref().map(|e| e.changed_files.clone()),
                full.evidence.as_ref().map(|e| e.changed_files.clone()),
                "changed-file lists drive language attribution and must match"
            );
            if full.evidence.is_some() {
                let lean_evidence = lean
                    .evidence
                    .as_ref()
                    .expect("lean keeps evidence when the full row has it");
                assert!(
                    lean_evidence.diff_patch.is_none(),
                    "diff patches must not be transferred"
                );
            }
        }
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
            .create_task_attempt(
                research,
                "researcher",
                None,
                "search-agent",
                "worktree",
                None,
            )
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
            .create_task_attempt(task, "worker", None, "opencode", "worktree", None)
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
                policy_audit: None,
                output_meta: None,
            })
            .unwrap();

        let integrating_task = db
            .create_task(
                run.id,
                "Integrating",
                "objective",
                &[],
                TaskState::Integrating,
                1,
                None,
                None,
            )
            .unwrap();

        let reconciled = db.reconcile_interrupted().unwrap();
        assert_eq!(reconciled.sessions, 1);
        assert_eq!(reconciled.attempts, 1);
        assert_eq!(reconciled.tasks, 1);
        assert_eq!(reconciled.requeued_integrations, 1);
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
        // Interrupted integrations retry from the awaiting_integration spot.
        assert_eq!(
            db.get_task(integrating_task).unwrap().unwrap().state,
            TaskState::AwaitingIntegration
        );
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Failed
        );
    }

    #[test]
    fn restart_reconciliation_revokes_mcp_grants_and_cancels_active_calls() {
        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        db.insert_tool_grant(&ToolGrant {
            id: "grant-1".into(),
            token_hash: "hash-1".into(),
            run_id: None,
            task_id: None,
            attempt_id: None,
            session_id: None,
            agent: "codex".into(),
            role: "worker".into(),
            operation: Some("implement".into()),
            allowed_tools: vec!["docs.search".into()],
            expires_at: "2099-01-01T00:00:00Z".into(),
            revoked_at: None,
        })
        .unwrap();
        let invocation = db
            .start_tool_invocation(&ToolInvocation {
                id: 0,
                run_id: None,
                task_id: None,
                attempt_id: None,
                session_id: None,
                agent: "codex".into(),
                role: "worker".into(),
                server: "docs".into(),
                tool: "search".into(),
                classification: ToolClassification::Read,
                status: "running".into(),
                started_at: "2026-01-01T00:00:00Z".into(),
                duration_ms: None,
                input_summary: "input omitted".into(),
                output_summary: None,
                error: None,
            })
            .unwrap();
        assert!(db
            .try_acquire_mcp_server_slot("docs", 1, "2026-01-01T00:00:00Z")
            .unwrap());
        assert!(!db
            .try_acquire_mcp_server_slot("docs", 1, "2026-01-01T00:00:00Z")
            .unwrap());

        db.reconcile_interrupted().unwrap();

        let grant = db.get_tool_grant_by_hash("hash-1").unwrap().unwrap();
        assert!(grant.revoked_at.is_some());
        let audit = db.list_tool_invocations().unwrap();
        assert_eq!(audit[0].id, invocation.id);
        assert_eq!(audit[0].status, "cancelled");
        assert!(db
            .try_acquire_mcp_server_slot("docs", 1, "2026-01-01T00:00:00Z")
            .unwrap());
    }

    fn record_invocation(db: &FactoryDb, tool: &str, status: &str, duration_ms: Option<u64>) {
        let invocation = db
            .start_tool_invocation(&ToolInvocation {
                id: 0,
                run_id: None,
                task_id: None,
                attempt_id: None,
                session_id: None,
                agent: "codex".into(),
                role: "worker".into(),
                server: "docs".into(),
                tool: tool.into(),
                classification: ToolClassification::Read,
                status: "running".into(),
                started_at: "2026-01-01T00:00:00Z".into(),
                duration_ms: None,
                input_summary: "input omitted".into(),
                output_summary: None,
                error: None,
            })
            .unwrap();
        if let Some(duration) = duration_ms {
            db.finish_tool_invocation(invocation.id, status, duration, None, None)
                .unwrap();
        }
    }

    #[test]
    fn tool_metrics_median_matches_the_conventional_median() {
        let dir = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();

        let empty = db.tool_metrics().unwrap();
        assert_eq!(empty.median_duration_ms, None);
        assert_eq!(empty.calls, 0);

        record_invocation(&db, "search", "success", Some(120));
        let single = db.tool_metrics().unwrap();
        assert_eq!(single.median_duration_ms, Some(120));
        assert_eq!(single.calls, 1);
        assert_eq!(single.successes, 1);
        assert_eq!(single.failures, 0);

        // The regression this hotfix exists for: an even-sized sample
        // previously returned the upper-middle element (30) instead of the
        // mean of the two central values.
        let regression = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&regression.path().join("test.db")).unwrap();
        for (tool, duration) in [("fetch", 10), ("fetch", 40), ("fetch", 20), ("fetch", 30)] {
            record_invocation(&db, tool, "success", Some(duration));
        }
        let metrics = db.tool_metrics().unwrap();
        assert_eq!(metrics.median_duration_ms, Some(25));
        assert_eq!(metrics.calls, 4);
        assert_eq!(metrics.successes, 4);
        assert_eq!(metrics.failures, 0);
        assert_eq!(
            db.tool_metrics().unwrap(),
            metrics,
            "repeated calls are stable"
        );
    }

    #[test]
    fn tool_metrics_median_covers_odd_identical_and_rounding_cases() {
        let dir = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();

        record_invocation(&db, "search", "success", Some(30));
        record_invocation(&db, "search", "success", Some(10));
        record_invocation(&db, "search", "success", Some(20));
        assert_eq!(db.tool_metrics().unwrap().median_duration_ms, Some(20));

        let other = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&other.path().join("test.db")).unwrap();
        record_invocation(&db, "search", "success", Some(7));
        record_invocation(&db, "search", "success", Some(7));
        assert_eq!(db.tool_metrics().unwrap().median_duration_ms, Some(7));
        record_invocation(&db, "search", "success", Some(7));
        record_invocation(&db, "search", "success", Some(7));
        assert_eq!(db.tool_metrics().unwrap().median_duration_ms, Some(7));

        let rounding = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&rounding.path().join("test.db")).unwrap();
        record_invocation(&db, "search", "success", Some(0));
        record_invocation(&db, "search", "success", Some(1));
        assert_eq!(db.tool_metrics().unwrap().median_duration_ms, Some(1));
    }

    #[test]
    fn tool_metrics_preserves_eligibility_and_counting_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();

        // Failed invocations with recorded durations keep contributing to
        // the median (pre-existing eligibility), while unfinished invocations
        // without a duration never do and count as neither success nor
        // failure.
        record_invocation(&db, "search", "success", Some(10));
        record_invocation(&db, "search", "tool_failure", Some(30));
        record_invocation(&db, "search", "timeout", Some(50));
        record_invocation(&db, "search", "success", None);
        let metrics = db.tool_metrics().unwrap();
        assert_eq!(metrics.median_duration_ms, Some(30));
        assert_eq!(metrics.calls, 4);
        assert_eq!(metrics.successes, 1);
        assert_eq!(metrics.failures, 2);

        let zero = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&zero.path().join("test.db")).unwrap();
        record_invocation(&db, "search", "success", Some(0));
        record_invocation(&db, "search", "success", Some(0));
        let metrics = db.tool_metrics().unwrap();
        assert_eq!(metrics.median_duration_ms, Some(0));
        assert_eq!(metrics.calls, 2);
    }

    #[test]
    fn github_links_and_deliveries_round_trip() {
        use factory_types::{DeliveryState, GitHubIssueLink, IssueComment, PullRequestInfo};

        let dir = TempDir::new().unwrap();
        let db = FactoryDb::open(&dir.path().join("test.db")).unwrap();
        let run = db.create_run("Resolve GitHub Issue #42", None).unwrap();

        let link = GitHubIssueLink {
            provider: "github".into(),
            repository: "octocat/example".into(),
            issue_number: 42,
            issue_url: "https://github.com/octocat/example/issues/42".into(),
            issue_title: "Fix refresh token race".into(),
            issue_body: "Tokens rotate.".into(),
            issue_state: "open".into(),
            issue_author: "octocat".into(),
            issue_labels: vec!["bug".into()],
            issue_comments: vec![IssueComment {
                author: "reviewer".into(),
                body: "Also mobile.".into(),
            }],
            imported_at: "2026-08-21T10:00:00Z".into(),
        };
        db.set_run_github_link(run.id, &link).unwrap();
        assert_eq!(db.get_run_github_link(run.id).unwrap().unwrap(), link);
        assert!(db.get_run_github_link(999999).unwrap().is_none());

        let delivery = db.get_or_create_delivery(run.id).unwrap();
        assert_eq!(delivery.state, DeliveryState::NotReady);
        assert_eq!(delivery.head_branch, format!("factory/run-{}", run.id));
        let updated = factory_types::GitHubDelivery {
            state: DeliveryState::Published,
            repository: Some("octocat/example".into()),
            remote: Some("origin".into()),
            base_branch: Some("main".into()),
            pull_request: Some(PullRequestInfo {
                number: 58,
                url: "https://github.com/octocat/example/pull/58".into(),
                state: "OPEN".into(),
                is_draft: true,
            }),
            pushed_head: Some("abc123".into()),
            error: None,
            ..delivery.clone()
        };
        db.set_delivery(&updated).unwrap();
        let persisted = db.get_delivery(run.id).unwrap().unwrap();
        assert_eq!(persisted.state, updated.state);
        assert_eq!(persisted.repository, updated.repository);
        assert_eq!(persisted.base_branch, updated.base_branch);
        assert_eq!(persisted.pushed_head, updated.pushed_head);
        assert_eq!(persisted.pull_request, updated.pull_request);
        assert!(
            persisted.updated_at > delivery.updated_at,
            "updated_at bumps"
        );
        assert!(db.get_delivery(999999).unwrap().is_none());
        // get_or_create returns the persisted record, never a reset one.
        assert_eq!(db.get_or_create_delivery(run.id).unwrap(), persisted);
    }
    #[test]
    fn delivery_claim_is_atomic_across_connections() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("claim.db");
        let db = FactoryDb::open(&db_path).unwrap();
        let run = db.create_run("durable delivery", None).unwrap();
        drop(db);

        let a = FactoryDb::open(&db_path).unwrap();
        let b = FactoryDb::open(&db_path).unwrap();
        a.get_or_create_delivery(run.id).unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = [a, b]
            .into_iter()
            .map(|db| {
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    db.claim_delivery(run.id, "op-id", "octocat/example", "origin", "main")
                })
            })
            .collect();
        let mut winners = 0;
        for handle in handles {
            let outcome = handle.join().unwrap().unwrap();
            if outcome.is_some() {
                winners += 1;
            }
        }
        assert_eq!(winners, 1, "exactly one connection may claim the operation");
        let delivery = FactoryDb::open(&db_path)
            .unwrap()
            .get_delivery(run.id)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.state.as_str(), "pushing");
        assert_eq!(delivery.operation_id.as_deref(), Some("op-id"));
    }

    #[test]
    fn delivery_claim_rejects_active_interrupted_and_published_records() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("guards.db");
        let db = FactoryDb::open(&db_path).unwrap();
        let run = db.create_run("claim guards", None).unwrap();
        db.get_or_create_delivery(run.id).unwrap();

        let claim = |db: &FactoryDb| {
            db.claim_delivery(run.id, "op", "octocat/example", "origin", "main")
                .unwrap()
                .is_some()
        };
        assert!(claim(&db), "an idle record is claimable");
        assert!(!claim(&db), "an active record is not claimable twice");

        db.demote_interrupted_deliveries().unwrap();
        assert!(
            !claim(&db),
            "an interrupted outcome must be reconciled first"
        );

        let mut delivery = db.get_delivery(run.id).unwrap().unwrap();
        delivery.state = factory_types::DeliveryState::Failed;
        delivery.error = Some("definitive".into());
        db.set_delivery(&delivery).unwrap();
        assert!(claim(&db), "a failed attempt is retryable");

        let mut delivery = db.get_delivery(run.id).unwrap().unwrap();
        delivery.state = factory_types::DeliveryState::Published;
        delivery.pull_request = Some(factory_types::PullRequestInfo {
            number: 58,
            url: "https://github.com/octocat/example/pull/58".into(),
            state: "OPEN".into(),
            is_draft: false,
        });
        db.set_delivery(&delivery).unwrap();
        assert!(!claim(&db), "a published PR must never be recreated");
    }

    #[test]
    fn stale_delivery_operations_cannot_write() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("stale.db");
        let db = FactoryDb::open(&db_path).unwrap();
        let run = db.create_run("stale writer", None).unwrap();
        db.get_or_create_delivery(run.id).unwrap();
        let claimed = db
            .claim_delivery(run.id, "op-1", "octocat/example", "origin", "main")
            .unwrap()
            .unwrap();
        assert_eq!(claimed.state.as_str(), "pushing");

        // A different operation id never writes.
        let mut from_stale = claimed.clone();
        from_stale.state = factory_types::DeliveryState::Published;
        assert!(!db.set_delivery_if_operation(&from_stale, "op-2").unwrap());

        // The owning operation can, including terminal states.
        assert!(db.set_delivery_if_operation(&from_stale, "op-1").unwrap());
        let persisted = db.get_delivery(run.id).unwrap().unwrap();
        assert_eq!(persisted.state.as_str(), "published");

        // Once terminal, even the same operation id cannot write again.
        from_stale.state = factory_types::DeliveryState::Failed;
        assert!(!db.set_delivery_if_operation(&from_stale, "op-1").unwrap());
        let persisted = db.get_delivery(run.id).unwrap().unwrap();
        assert_eq!(persisted.state.as_str(), "published");
    }

    #[test]
    fn demotion_marks_only_active_deliveries_and_preserves_history() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("restart.db");
        let db = FactoryDb::open(&db_path).unwrap();
        let run = db.create_run("restart recovery", None).unwrap();
        let idle = db.create_run("idle run", None).unwrap();

        db.get_or_create_delivery(run.id).unwrap();
        db.get_or_create_delivery(idle.id).unwrap();
        db.claim_delivery(run.id, "op", "octocat/example", "origin", "main")
            .unwrap()
            .unwrap();

        let demoted = db.demote_interrupted_deliveries().unwrap();
        assert_eq!(demoted, vec![run.id]);
        let interrupted = db.get_delivery(run.id).unwrap().unwrap();
        assert_eq!(interrupted.state.as_str(), "interrupted");
        assert!(interrupted.recovery_required);
        assert!(interrupted.error.unwrap().contains("stopped"));
        assert_eq!(interrupted.repository.as_deref(), Some("octocat/example"));
        assert_eq!(interrupted.base_branch.as_deref(), Some("main"));
        assert_eq!(
            interrupted.head_branch,
            format!("factory/run-{}", run.id),
            "reconciliation metadata is preserved"
        );
        // Idempotent: a second startup pass changes nothing.
        assert!(db.demote_interrupted_deliveries().unwrap().is_empty());
    }

    #[test]
    fn legacy_delivery_rows_gain_operation_columns_on_migration() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("legacy-delivery.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        // The delivery table exactly as V12 created it, before V18 columns.
        conn.execute_batch(
            "CREATE TABLE github_deliveries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
                state TEXT NOT NULL DEFAULT 'not_ready',
                repository TEXT,
                remote TEXT,
                base_branch TEXT,
                head_branch TEXT NOT NULL,
                pushed_head TEXT,
                pr_number INTEGER,
                pr_url TEXT,
                pr_state TEXT,
                pr_is_draft INTEGER,
                error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             INSERT INTO schema_migrations (version, applied_at) VALUES
             (1,'2026-01-01T00:00:00Z'),(2,'2026-01-01T00:00:00Z'),(3,'2026-01-01T00:00:00Z'),
             (4,'2026-01-01T00:00:00Z'),(5,'2026-01-01T00:00:00Z'),(6,'2026-01-01T00:00:00Z'),
             (7,'2026-01-01T00:00:00Z'),(8,'2026-01-01T00:00:00Z'),(9,'2026-01-01T00:00:00Z'),
             (10,'2026-01-01T00:00:00Z'),(11,'2026-01-01T00:00:00Z'),(12,'2026-01-01T00:00:00Z'),
             (13,'2026-01-01T00:00:00Z'),(14,'2026-01-01T00:00:00Z'),(15,'2026-01-01T00:00:00Z'),
             (16,'2026-01-01T00:00:00Z'),(17,'2026-01-01T00:00:00Z');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runs (objective, status, planner_agent, created_at, updated_at)
             VALUES ('legacy delivery', 'completed', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO github_deliveries
                 (run_id, state, head_branch, created_at, updated_at)
             VALUES (1, 'pushing', 'factory/run-1', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        let db = FactoryDb::open(&db_path).unwrap();
        let delivery = db.get_delivery(1).unwrap().unwrap();
        assert_eq!(delivery.state.as_str(), "pushing");
        assert_eq!(
            delivery.operation_id, None,
            "legacy rows have no operation yet"
        );
        assert!(
            !delivery.recovery_required,
            "legacy rows default to no recovery flag"
        );
        // The legacy active row migrates cleanly into the recovery flow.
        assert_eq!(db.demote_interrupted_deliveries().unwrap(), vec![1]);
    }
}
