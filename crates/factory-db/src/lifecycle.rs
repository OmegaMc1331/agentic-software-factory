use crate::{now, DbError, FactoryDb, Result};
use factory_types::{
    CheckpointWorktree, MissionCheckpoint, MissionLifecycle, MissionLifecycleState,
    WorkspaceSessionLifecycle,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

pub(crate) const SCHEMA: &str = "
CREATE TABLE workspace_session_lifecycle (
 id TEXT PRIMARY KEY, project_id TEXT NOT NULL, generation INTEGER NOT NULL DEFAULT 1,
 closed INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL
);
CREATE TABLE mission_lifecycle (
 run_id INTEGER PRIMARY KEY REFERENCES runs(id), project_id TEXT NOT NULL,
 session_id TEXT NOT NULL REFERENCES workspace_session_lifecycle(id),
 generation INTEGER NOT NULL DEFAULT 1, state TEXT NOT NULL DEFAULT 'active',
 reason TEXT, checkpoint_id INTEGER, problem TEXT, updated_at TEXT NOT NULL
);
CREATE INDEX mission_lifecycle_session ON mission_lifecycle(session_id,run_id);
CREATE TABLE mission_checkpoints (
 id INTEGER PRIMARY KEY AUTOINCREMENT, run_id INTEGER NOT NULL REFERENCES runs(id),
 generation INTEGER NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL,
 UNIQUE(run_id,generation)
);
CREATE TABLE lifecycle_commands (
 run_id INTEGER NOT NULL REFERENCES runs(id), command_key TEXT NOT NULL,
 action TEXT NOT NULL, generation INTEGER NOT NULL, PRIMARY KEY(run_id,command_key)
);
CREATE TABLE workspace_session_events (
 id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES workspace_session_lifecycle(id),
 generation INTEGER NOT NULL, event_type TEXT NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL,
 UNIQUE(session_id,generation,event_type)
);
";

fn valid_session(id: &str) -> bool {
    id.len() == 34
        && id.starts_with("s_")
        && id[2..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl FactoryDb {
    pub(crate) fn attach_workspace_mission(
        &self,
        run: i64,
        command: &factory_types::ConversationCommand,
    ) -> Result<()> {
        let Some(session) = command.payload["workspace_session_id"].as_str() else {
            return Ok(());
        };
        let project: Option<String> = self
            .conn
            .query_row(
                "SELECT project_id FROM workspace_session_lifecycle WHERE id=?1 AND closed=0",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(project) = project {
            self.bind_workspace_run(run, &project, session)?;
        } else {
            let closed: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM workspace_session_lifecycle WHERE id=?1)",
                [session],
                |r| r.get(0),
            )?;
            if closed {
                return Err(DbError::Busy("Owning workspace session is closed".into()));
            }
        }
        Ok(())
    }

    pub fn bind_workspace_run(&self, run: i64, project: &str, session: &str) -> Result<()> {
        let session_record = self.workspace_session_lifecycle(project, session)?;
        if session_record.closed {
            return Err(DbError::Busy("Owning workspace session is closed".into()));
        }
        let active:(i64,i64)=self.conn.query_row("SELECT count(*),COALESCE(SUM(CASE WHEN session_id=?1 THEN 1 ELSE 0 END),0) FROM mission_lifecycle WHERE state<>'settled'",[session],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if active.0 >= 1000 || active.1 >= 100 {
            return Err(DbError::Busy(
                "Browser-managed mission ownership limit reached".into(),
            ));
        }
        let status = self.get_run(run)?.ok_or(DbError::NotFound("run"))?.status;
        if !matches!(
            status,
            factory_types::RunStatus::Planning | factory_types::RunStatus::Planned
        ) || self.count_task_attempts(run)? > 0
        {
            return Err(DbError::Busy(
                "Only newly created workflows may acquire browser ownership".into(),
            ));
        }
        if self.mission_lifecycle(run)?.is_some() {
            return Err(DbError::Busy("Mission ownership is immutable".into()));
        }
        self.conn.execute("INSERT INTO mission_lifecycle(run_id,project_id,session_id,updated_at) VALUES(?1,?2,?3,?4)",params![run,project,session,now()])?;
        Ok(())
    }
    pub fn register_workspace_session(&self, project: &str, session: &str) -> Result<()> {
        if !valid_session(session) || project.len() > 256 || project.is_empty() {
            return Err(DbError::Corrupt(
                "Invalid workspace session identity".into(),
            ));
        }
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM workspace_session_lifecycle",
            [],
            |r| r.get(0),
        )?;
        if count >= 10_000 && self.workspace_session_lifecycle(project, session).is_err() {
            return Err(DbError::Busy("Workspace session limit reached".into()));
        }
        self.conn.execute("INSERT OR IGNORE INTO workspace_session_lifecycle(id,project_id,updated_at) VALUES(?1,?2,?3)",params![session,project,now()])?;
        self.workspace_session_lifecycle(project, session)?;
        Ok(())
    }

    pub fn workspace_session_lifecycle(
        &self,
        project: &str,
        session: &str,
    ) -> Result<WorkspaceSessionLifecycle> {
        let (generation, closed) = self.conn.query_row("SELECT generation,closed FROM workspace_session_lifecycle WHERE id=?1 AND project_id=?2",params![session,project],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(DbError::NotFound("workspace session"))?;
        let ids = self.conn.prepare("SELECT run_id FROM mission_lifecycle WHERE session_id=?1 AND project_id=?2 ORDER BY CASE WHEN state='settled' THEN 1 ELSE 0 END,run_id DESC LIMIT 100")?.query_map(params![session,project],|r|r.get::<_,i64>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(WorkspaceSessionLifecycle {
            project_id: project.into(),
            session_id: session.into(),
            generation,
            closed,
            events:self.conn.prepare("SELECT payload FROM workspace_session_events WHERE session_id=?1 ORDER BY id DESC LIMIT 20")?.query_map([session],|r|r.get::<_,String>(0))?.map(|row|Ok(serde_json::from_str(&row?)?)).collect::<Result<Vec<_>>>()?,
            missions: ids
                .into_iter()
                .map(|id| {
                    self.mission_lifecycle(id)?
                        .ok_or(DbError::NotFound("mission lifecycle"))
                })
                .collect::<Result<_>>()?,
        })
    }

    pub fn set_workspace_session_closed(
        &self,
        project: &str,
        session: &str,
        expected: i64,
        closed: bool,
    ) -> Result<WorkspaceSessionLifecycle> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let current = self.workspace_session_lifecycle(project, session)?;
        if current.closed != closed {
            if current.generation != expected {
                return Err(DbError::Conflict {
                    expected,
                    current: current.generation,
                });
            }
            tx.execute("UPDATE workspace_session_lifecycle SET closed=?1,generation=generation+1,updated_at=?2 WHERE id=?3 AND project_id=?4 AND generation=?5",params![closed,now(),session,project,expected])?;
            self.workspace_lifecycle_event(
                project,
                session,
                expected + 1,
                if closed {
                    "session.close_requested"
                } else {
                    "session.reopened"
                },
            )?;
        }
        self.reconcile_workspace_closure(project, session)?;
        tx.commit()?;
        self.workspace_session_lifecycle(project, session)
    }

    pub fn mission_lifecycle(&self, run: i64) -> Result<Option<MissionLifecycle>> {
        let row = self.conn.query_row("SELECT project_id,session_id,generation,state,reason,checkpoint_id,problem,updated_at FROM mission_lifecycle WHERE run_id=?1",[run],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<i64>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?))).optional()?;
        row.map(
            |(
                project_id,
                workspace_session_id,
                generation,
                state,
                reason,
                checkpoint_id,
                problem,
                updated_at,
            )| {
                Ok(MissionLifecycle {
                    schema_version: 1,
                    project_id,
                    workspace_session_id,
                    run_id: run,
                    generation,
                    state: serde_json::from_value(json!(state))?,
                    reason,
                    checkpoint_id,
                    problem,
                    updated_at,
                })
            },
        )
        .transpose()
    }

    pub fn lifecycle_fenced(&self, run: i64) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM mission_lifecycle WHERE run_id=?1 AND state<>'active')",
            [run],
            |r| r.get(0),
        )?)
    }

    pub fn retry_settled_task(&self, task_id: i64, project: &str, generation: i64) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let task = self.get_task(task_id)?.ok_or(DbError::NotFound("task"))?;
        let run = self.get_run(task.run_id)?.ok_or(DbError::NotFound("run"))?;
        let life = self
            .mission_lifecycle(run.id)?
            .ok_or(DbError::NotFound("browser mission"))?;
        if life.project_id != project
            || life.generation != generation
            || life.state != MissionLifecycleState::Settled
            || !matches!(
                run.status,
                factory_types::RunStatus::Failed | factory_types::RunStatus::Blocked
            )
            || !matches!(
                task.state,
                factory_types::TaskState::Failed | factory_types::TaskState::Blocked
            )
            || self
                .workspace_session_lifecycle(project, &life.workspace_session_id)?
                .closed
            || self.scheduler_running(run.id)?
            || self.active_dispatch_count(run.id)? > 0
        {
            return Err(DbError::Busy(
                "Settled retry ownership, generation or session is not eligible".into(),
            ));
        }
        for (_, status, metadata) in self.lifecycle_processes(run.id)? {
            if matches!(status.as_str(), "prepared" | "running")
                || metadata.as_ref().is_some_and(|meta| {
                    meta.process_id.is_some()
                        && (!meta.process_tree_supervised || !meta.process_tree_cleanup_confirmed)
                })
                || metadata.is_none() && status == "interrupted"
            {
                return Err(DbError::Busy(
                    "Process ownership requires reconciliation before an explicit retry".into(),
                ));
            }
        }
        let uncertain: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM integration_records WHERE run_id=?1 AND (status IN ('preparing','publishing') OR (task_id=?2 AND status IN ('published','conflict'))))", params![run.id, task_id], |row|row.get(0))?;
        if uncertain
            || self.get_delivery(run.id)?.is_some_and(|delivery| {
                delivery.recovery_required
                    || delivery.pull_request.is_some()
                    || matches!(
                        delivery.state,
                        factory_types::DeliveryState::Pushing
                            | factory_types::DeliveryState::CreatingPr
                            | factory_types::DeliveryState::Interrupted
                    )
            })
        {
            return Err(DbError::Busy(
                "Integration or external delivery requires reconciliation before retry".into(),
            ));
        }
        let active: (i64,i64) = tx.query_row("SELECT count(*),COALESCE(SUM(CASE WHEN session_id=?1 THEN 1 ELSE 0 END),0) FROM mission_lifecycle WHERE state<>'settled'", [&life.workspace_session_id], |row|Ok((row.get(0)?,row.get(1)?)))?;
        if active.0 >= 1000 || active.1 >= 100 {
            return Err(DbError::Busy(
                "Browser-managed mission limit reached".into(),
            ));
        }
        self.set_task_state(task_id, factory_types::TaskState::Ready)?;
        self.set_run_status(run.id, factory_types::RunStatus::Active)?;
        tx.execute("UPDATE mission_lifecycle SET state='active',generation=generation+1,reason=NULL,problem=NULL,checkpoint_id=NULL,updated_at=?1 WHERE run_id=?2 AND generation=?3 AND state='settled'", params![now(),run.id,generation])?;
        self.lifecycle_event(run.id,"mission.retry_requested", &json!({"task_id":task_id,"generation":generation+1,"reason":"explicit_task_retry","previous_attempts_preserved":true}))?;
        tx.commit()?;
        Ok(())
    }

    pub fn lifecycle_runs(&self) -> Result<Vec<i64>> {
        Ok(self.conn.prepare("SELECT run_id FROM mission_lifecycle WHERE state<>'settled' ORDER BY run_id LIMIT 1000")?.query_map([],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?)
    }

    pub fn shutdown_mission_candidates(&self, pending_only: bool) -> Result<Vec<i64>> {
        let query = if pending_only {
            "SELECT run_id FROM mission_lifecycle WHERE state IN ('suspend_requested','quiescing') OR (state='recovery_required' AND reason='application_shutdown') ORDER BY run_id LIMIT 1001"
        } else {
            "SELECT run_id FROM mission_lifecycle WHERE state IN ('active','reconciling','suspend_requested','quiescing') ORDER BY run_id LIMIT 1001"
        };
        let runs = self
            .conn
            .prepare(query)?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if runs.len() > 1000 {
            return Err(DbError::Busy(
                "Shutdown mission inspection limit exceeded".into(),
            ));
        }
        Ok(runs)
    }

    pub fn task_execution_attempt_count(&self, task: i64) -> Result<usize> {
        Ok(self.conn.query_row("SELECT count(*) FROM task_attempts a JOIN tasks t ON t.id=a.task_id WHERE t.id=?1 AND (a.status<>'interrupted' OR NOT EXISTS(SELECT 1 FROM mission_lifecycle l WHERE l.run_id=t.run_id AND l.checkpoint_id IS NOT NULL))",[task],|r|r.get::<_,i64>(0))? as usize)
    }

    pub fn scheduler_elapsed_millis(&self, run: i64) -> Result<u64> {
        let value:f64=self.conn.query_row("SELECT COALESCE(SUM(MAX(0,(julianday(finished_at)-julianday(started_at))*86400000)),0) FROM scheduler_owners WHERE run_id=?1 AND finished_at IS NOT NULL",[run],|r|r.get(0))?;
        Ok(value.max(0.0).ceil() as u64)
    }

    pub fn lifecycle_processes(
        &self,
        run: i64,
    ) -> Result<Vec<(i64, String, Option<factory_types::AgentOutputMetadata>)>> {
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM agent_sessions WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )?;
        if count > 4096 {
            return Err(DbError::Busy(
                "Lifecycle session history exceeds inspection limit".into(),
            ));
        }
        let rows=self.conn.prepare("SELECT id,status,output_meta FROM agent_sessions WHERE run_id=?1 ORDER BY id LIMIT 4096")?.query_map([run],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|(id, status, meta)| {
                Ok((
                    id,
                    status,
                    meta.map(|s| serde_json::from_str(&s)).transpose()?,
                ))
            })
            .collect()
    }

    pub fn lifecycle_event(&self, run: i64, kind: &str, payload: &Value) -> Result<()> {
        self.scheduler_event(run, None, kind, payload)?;
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT conversation_id FROM conversation_missions WHERE run_id=?1",
                [run],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            self.conn
                .execute_batch("SAVEPOINT lifecycle_presentation")?;
            let content=match kind {
                "mission.suspended"=>Some(format!("Mission #{run} is suspended. Owned execution has stopped and a durable checkpoint preserves remaining tasks and unapproved partial work.")),
                "mission.resumed"=>Some(format!("Mission #{run} resumed from verified recorded state. Completed tasks are preserved; remaining work uses new agent invocations.")),
                  "mission.retry_requested"=>Some(format!("Mission #{run} was explicitly retried after a settled failure. Previous attempts and partial work remain preserved; current policy and review still apply.")),
                "mission.recovery_required"=>Some(format!("Mission #{run} requires recovery inspection. New scheduling is fenced; no uncertain operation will be replayed automatically.")),
                "mission.execution_settled"=>Some(format!("Mission #{run} is {}. {} of {} tasks completed. Inspect review, test evidence and integration commit {}.",payload["status"].as_str().unwrap_or("settled"),payload["completed"],payload["total"],payload["integration_commit"].as_str().unwrap_or("unavailable"))),
                _=>None,
            };
            let delivery = (|| {
                if content.is_some() {
                    let messages: i64 = self.conn.query_row(
                        "SELECT count(*) FROM conversation_messages WHERE conversation_id=?1",
                        [&id],
                        |r| r.get(0),
                    )?;
                    if messages >= 10_000 {
                        return Err(DbError::Busy("Conversation message limit reached".into()));
                    }
                }
                crate::conversations::event(&self.conn, &id, kind, payload)?;
                if let Some(content) = content {
                    self.conn.execute("INSERT INTO conversation_messages(conversation_id,role,content,status,run_id,provenance,created_at) VALUES(?1,'orchestrator',?2,'finished',?3,?4,?5)",params![id,content,run,json!({"source":"runtime_lifecycle","event":kind}).to_string(),now()])?;
                }
                Ok::<_, DbError>(())
            })();
            if delivery.is_err() {
                self.conn
                    .execute_batch("ROLLBACK TO lifecycle_presentation")?;
                self.scheduler_event(run,None,"lifecycle_presentation_blocked",&json!({"reason":"Conversation notification storage is unavailable or full; authoritative lifecycle state remains in the project database"}))?;
            }
            self.conn.execute_batch("RELEASE lifecycle_presentation")?;
        }
        Ok(())
    }

    pub fn request_mission_suspend(
        &self,
        run: i64,
        expected: i64,
        key: &str,
        reason: &str,
    ) -> Result<MissionLifecycle> {
        if ![
            "explicit_session_close",
            "last_browser_client_expired",
            "user_requested_suspend",
            "application_shutdown",
        ]
        .contains(&reason)
            || key.is_empty()
            || key.len() > 128
        {
            return Err(DbError::Corrupt("Invalid lifecycle command".into()));
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut current = self
            .mission_lifecycle(run)?
            .ok_or(DbError::NotFound("browser-managed mission"))?;
        let previous: Option<(String,i64)> = tx.query_row("SELECT action,generation FROM lifecycle_commands WHERE run_id=?1 AND command_key=?2",params![run,key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((action, _)) = previous {
            if action != "suspend" {
                return Err(DbError::Busy("Command identity reused".into()));
            }
            return Ok(current);
        }
        if current.generation != expected {
            return Err(DbError::Conflict {
                expected,
                current: current.generation,
            });
        }
        let status = self.get_run(run)?.ok_or(DbError::NotFound("run"))?.status;
        if matches!(
            status,
            factory_types::RunStatus::Completed | factory_types::RunStatus::Cancelled
        ) || current.state == MissionLifecycleState::Settled
        {
            return Ok(current);
        }
        if matches!(
            current.state,
            MissionLifecycleState::Active | MissionLifecycleState::Reconciling
        ) {
            tx.execute("UPDATE mission_lifecycle SET state='suspend_requested',generation=generation+1,reason=?1,problem=NULL,updated_at=?2 WHERE run_id=?3 AND generation=?4",params![reason,now(),run,expected])?;
            current = self
                .mission_lifecycle(run)?
                .ok_or(DbError::NotFound("mission"))?;
            self.lifecycle_event(
                run,
                "mission.suspend_requested",
                &json!({"run_id":run,"generation":current.generation,"reason":reason}),
            )?;
            tx.execute(
                "UPDATE tool_grants SET revoked_at=?1 WHERE run_id=?2 AND revoked_at IS NULL",
                params![now(), run],
            )?;
        } else {
            return Ok(current);
        }
        tx.execute("INSERT INTO lifecycle_commands(run_id,command_key,action,generation) VALUES(?1,?2,'suspend',?3)",params![run,key,current.generation])?;
        tx.commit()?;
        Ok(current)
    }

    pub fn lifecycle_transition(
        &self,
        run: i64,
        generation: i64,
        from: MissionLifecycleState,
        to: MissionLifecycleState,
        problem: Option<&str>,
    ) -> Result<()> {
        let allowed = matches!(
            (from, to),
            (
                MissionLifecycleState::SuspendRequested,
                MissionLifecycleState::Quiescing
            ) | (
                MissionLifecycleState::Quiescing,
                MissionLifecycleState::Suspended
            ) | (
                MissionLifecycleState::Quiescing,
                MissionLifecycleState::RecoveryRequired
            ) | (
                MissionLifecycleState::Suspended,
                MissionLifecycleState::Reconciling
            ) | (
                MissionLifecycleState::RecoveryRequired,
                MissionLifecycleState::Reconciling
            ) | (
                MissionLifecycleState::RecoveryRequired,
                MissionLifecycleState::Quiescing
            ) | (
                MissionLifecycleState::Reconciling,
                MissionLifecycleState::Active
            ) | (
                MissionLifecycleState::Reconciling,
                MissionLifecycleState::RecoveryRequired
            )
        );
        if !allowed {
            return Err(DbError::Busy("Invalid lifecycle transition".into()));
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let current = self
            .mission_lifecycle(run)?
            .ok_or(DbError::NotFound("mission lifecycle"))?;
        if current.generation == generation
            && (current.state == to
                || from == MissionLifecycleState::SuspendRequested
                    && to == MissionLifecycleState::Quiescing
                    && matches!(
                        current.state,
                        MissionLifecycleState::Suspended
                            | MissionLifecycleState::RecoveryRequired
                            | MissionLifecycleState::Settled
                    ))
        {
            return Ok(());
        }
        let changed=tx.execute("UPDATE mission_lifecycle SET state=?1,problem=?2,updated_at=?3 WHERE run_id=?4 AND generation=?5 AND state=?6",params![to.as_str(),problem,now(),run,generation,from.as_str()])?;
        if changed != 1 {
            return Err(DbError::Busy("Stale lifecycle transition".into()));
        }
        self.lifecycle_event(
            run,
            &format!("mission.{}", to.as_str()),
            &json!({"run_id":run,"generation":generation,"state":to,"problem":problem}),
        )?;
        self.reconcile_workspace_closure(
            &self
                .mission_lifecycle(run)?
                .ok_or(DbError::NotFound("mission"))?
                .project_id,
            &self
                .mission_lifecycle(run)?
                .ok_or(DbError::NotFound("mission"))?
                .workspace_session_id,
        )?;
        tx.commit()?;
        Ok(())
    }

    fn workspace_lifecycle_event(
        &self,
        project: &str,
        session: &str,
        generation: i64,
        kind: &str,
    ) -> Result<()> {
        let count: i64 =
            self.conn
                .query_row("SELECT count(*) FROM workspace_session_events", [], |r| {
                    r.get(0)
                })?;
        if count >= 100_000 {
            return Err(DbError::Busy(
                "Workspace lifecycle event limit reached; history retained".into(),
            ));
        }
        let payload = json!({"schemaVersion":1,"projectId":project,"sessionId":session,"generation":generation,"eventType":kind,"timestamp":now()});
        let changed=self.conn.execute("INSERT OR IGNORE INTO workspace_session_events(session_id,generation,event_type,payload,created_at) VALUES(?1,?2,?3,?4,?5)",params![session,generation,kind,payload.to_string(),now()])?;
        if changed == 1 {
            let runs=self.conn.prepare("SELECT run_id FROM mission_lifecycle WHERE session_id=?1 AND project_id=?2 AND state<>'settled' LIMIT 100")?.query_map(params![session,project],|r|r.get::<_,i64>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
            for run in runs {
                self.lifecycle_event(run, kind, &payload)?;
            }
        }
        Ok(())
    }

    fn reconcile_workspace_closure(&self, project: &str, session: &str) -> Result<()> {
        let state = self.workspace_session_lifecycle(project, session)?;
        if state.closed {
            let event = if state
                .missions
                .iter()
                .any(|m| m.state == MissionLifecycleState::RecoveryRequired)
            {
                Some("session.suspension_blocked")
            } else if state.missions.iter().all(|m| {
                matches!(
                    m.state,
                    MissionLifecycleState::Suspended | MissionLifecycleState::Settled
                )
            }) {
                Some("session.closed")
            } else {
                None
            };
            if let Some(kind) = event {
                self.workspace_lifecycle_event(project, session, state.generation, kind)?;
            }
        }
        Ok(())
    }

    pub fn save_mission_checkpoint(
        &self,
        run: i64,
        generation: i64,
        worktrees: Vec<CheckpointWorktree>,
        integration_head: Option<String>,
    ) -> Result<MissionCheckpoint> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let life = self
            .mission_lifecycle(run)?
            .ok_or(DbError::NotFound("lifecycle"))?;
        if life.generation != generation
            || !matches!(
                life.state,
                MissionLifecycleState::Quiescing | MissionLifecycleState::Suspended
            )
            || self.scheduler_running(run)?
            || self.active_dispatch_count(run)? > 0
        {
            return Err(DbError::Busy(
                "Checkpoint requires drained ownership".into(),
            ));
        }
        let live: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE run_id=?1 AND status IN ('running','active')) OR EXISTS(SELECT 1 FROM integration_records WHERE run_id=?1 AND status IN ('preparing','publishing','interrupted')) OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1 AND status='running')",[run],|r|r.get(0))?;
        if live {
            return Err(DbError::Busy(
                "An operation requires reconciliation before checkpoint".into(),
            ));
        }
        let previous: Option<(i64, String)> = tx
            .query_row(
                "SELECT id,payload FROM mission_checkpoints WHERE run_id=?1 AND generation=?2",
                params![run, generation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, payload)) = previous {
            let checkpoint: MissionCheckpoint = serde_json::from_str(&payload)?;
            if checkpoint.schema_version != 1
                || checkpoint.id != id
                || checkpoint.project_id != life.project_id
                || checkpoint.workspace_session_id != life.workspace_session_id
                || checkpoint.run_id != run
                || checkpoint.generation != generation
                || checkpoint.plan_revision != self.get_plan_revision(run)?
                || checkpoint.worktrees != worktrees
                || checkpoint.integration_commit != integration_head
            {
                return Err(DbError::Busy(
                    "Existing checkpoint does not match reconciled state".into(),
                ));
            }
            return Ok(checkpoint);
        }
        if life.state != MissionLifecycleState::Quiescing {
            return Err(DbError::Busy(
                "Suspended lifecycle has no matching checkpoint".into(),
            ));
        }
        let tasks = self.list_tasks(run)?;
        let attempts=self.conn.prepare("SELECT a.id,a.task_id,a.status FROM task_attempts a JOIN tasks t ON t.id=a.task_id WHERE t.run_id=?1 ORDER BY a.id LIMIT 4097")?.query_map([run],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"taskId":r.get::<_,i64>(1)?,"status":r.get::<_,String>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        if attempts.len() > 4096 {
            return Err(DbError::Busy(
                "Checkpoint attempt history limit reached".into(),
            ));
        }
        let handoffs=self.conn.prepare("SELECT id,state,json_extract(payload,'$.target.id'),json_extract(payload,'$.target.revision') FROM agent_handoffs WHERE run_id=?1 AND state IN ('waiting','ready','executing') ORDER BY id LIMIT 1025")?.query_map([run],|r|Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"target":r.get::<_,Option<String>>(2)?,"revision":r.get::<_,Option<i64>>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let cursor: i64 = tx.query_row(
            "SELECT COALESCE(MAX(id),0) FROM scheduler_events WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )?;
        let records = json!({"tasks":tasks.iter().map(|t|json!({"id":t.id,"state":t.state})).collect::<Vec<_>>(),"attempts":attempts,"handoffs":handoffs,"eventCursor":cursor});
        let mut checkpoint = MissionCheckpoint {
            schema_version: 1,
            id: 0,
            project_id: life.project_id,
            workspace_session_id: life.workspace_session_id,
            run_id: run,
            generation,
            plan_revision: self.get_plan_revision(run)?,
            integration_commit: integration_head,
            worktrees,
            records,
            created_at: now(),
        };
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM mission_checkpoints WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )?;
        if count >= 256 {
            return Err(DbError::Busy(
                "Checkpoint retention limit reached; history retained".into(),
            ));
        }
        tx.execute("INSERT INTO mission_checkpoints(run_id,generation,payload,created_at) VALUES(?1,?2,?3,?4)",params![run,generation,serde_json::to_string(&checkpoint)?,checkpoint.created_at])?;
        let budget: i64 = tx.query_row(
            "SELECT COALESCE(SUM(length(payload)),0) FROM mission_checkpoints",
            [],
            |r| r.get(0),
        )?;
        if budget > 64 * 1024 * 1024 || serde_json::to_vec(&checkpoint)?.len() > 512 * 1024 {
            return Err(DbError::Busy(
                "Checkpoint storage budget reached; history retained".into(),
            ));
        }
        checkpoint.id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE mission_checkpoints SET payload=?1 WHERE id=?2",
            params![serde_json::to_string(&checkpoint)?, checkpoint.id],
        )?;
        tx.execute(
            "UPDATE mission_lifecycle SET checkpoint_id=?1 WHERE run_id=?2 AND generation=?3",
            params![checkpoint.id, run, generation],
        )?;
        self.lifecycle_event(
            run,
            "mission.checkpoint_created",
            &json!({"run_id":run,"generation":generation,"checkpoint_id":checkpoint.id}),
        )?;
        tx.commit()?;
        Ok(checkpoint)
    }

    pub fn mission_checkpoints(
        &self,
        run: i64,
        before: Option<i64>,
    ) -> Result<Vec<MissionCheckpoint>> {
        let rows=self.conn.prepare("SELECT payload FROM mission_checkpoints WHERE run_id=?1 AND (?2 IS NULL OR id<?2) ORDER BY id DESC LIMIT 20")?.query_map(params![run,before],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|v| Ok(serde_json::from_str(&v)?))
            .collect()
    }

    pub fn settle_mission_lifecycle(&self, run: i64) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE mission_lifecycle SET state='settled',updated_at=?1 WHERE run_id=?2 AND EXISTS(SELECT 1 FROM runs WHERE id=?2 AND status IN ('completed','cancelled','failed','blocked')) AND state='active'",params![now(),run])?;
        if changed == 1 {
            self.lifecycle_settlement(run)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn lifecycle_settlement(&self, run: i64) -> Result<()> {
        let life = self
            .mission_lifecycle(run)?
            .ok_or(DbError::NotFound("mission lifecycle"))?;
        let recorded:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM scheduler_events WHERE run_id=?1 AND event_type='mission.execution_settled' AND json_extract(payload,'$.generation')=?2)",params![run,life.generation],|row|row.get(0))?;
        if (life.checkpoint_id.is_some() || life.generation > 1) && !recorded {
            let tasks = self.list_tasks(run)?;
            self.lifecycle_event(run,"mission.execution_settled",&json!({"run_id":run,"generation":life.generation,"status":self.get_run(run)?.map(|r|r.status),"completed":tasks.iter().filter(|t|t.state==factory_types::TaskState::Completed).count(),"total":tasks.iter().filter(|t|t.state!=factory_types::TaskState::Superseded).count(),"integration_commit":self.get_run_integration(run)?}))?;
        }
        self.reconcile_workspace_closure(&life.project_id, &life.workspace_session_id)
    }

    pub fn reconcile_settled_retry_presentation(&self, run: i64) -> Result<()> {
        let pending = || -> Result<bool> {
            Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM mission_lifecycle l JOIN runs r ON r.id=l.run_id WHERE l.run_id=?1 AND l.state='settled' AND l.checkpoint_id IS NULL AND l.generation>1 AND r.status IN ('completed','failed','blocked','cancelled') AND EXISTS(SELECT 1 FROM scheduler_events e WHERE e.run_id=l.run_id AND e.event_type='mission.retry_requested' AND json_extract(e.payload,'$.generation')=l.generation) AND NOT EXISTS(SELECT 1 FROM scheduler_events e WHERE e.run_id=l.run_id AND e.event_type='mission.execution_settled' AND json_extract(e.payload,'$.generation')=l.generation))",[run],|row|row.get(0))?)
        };
        if !pending()? {
            return Ok(());
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if pending()? {
            self.lifecycle_settlement(run)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn request_lifecycle_cancel(&self, run: i64) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if let Some(life) = self.mission_lifecycle(run)?.filter(|l| {
            !matches!(
                l.state,
                MissionLifecycleState::Active | MissionLifecycleState::Settled
            ) && l.reason.as_deref() != Some("user_cancelled")
        }) {
            tx.execute("UPDATE mission_lifecycle SET state='quiescing',reason='user_cancelled',generation=generation+1,updated_at=?1 WHERE run_id=?2 AND generation=?3",params![now(),run,life.generation])?;
            self.lifecycle_event(
                run,
                "mission.cancellation_requested",
                &json!({"run_id":run}),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn settle_cancelled_lifecycle(&self, run: i64) -> Result<()> {
        if self.scheduler_running(run)? || self.active_dispatch_count(run)? > 0 {
            return Err(DbError::Busy(
                "Cancellation process ownership is not drained".into(),
            ));
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE mission_lifecycle SET state='settled',updated_at=?1 WHERE run_id=?2 AND state<>'settled' AND EXISTS(SELECT 1 FROM runs WHERE id=?2 AND status='cancelled')",params![now(),run])?;
        if changed == 1 {
            self.lifecycle_settlement(run)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn complete_lifecycle_race(&self, run: i64, generation: i64) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE mission_lifecycle SET state='settled',updated_at=?1 WHERE run_id=?2 AND generation=?3 AND state<>'settled' AND EXISTS(SELECT 1 FROM runs WHERE id=?2 AND status='completed')",params![now(),run,generation])?;
        if changed == 1 {
            self.lifecycle_settlement(run)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn resume_authored_plan(&self, run: i64) -> Result<Option<factory_types::Plan>> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM conversation_commands WHERE run_id=?1",
                [run],
                |r| r.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let value: Value = serde_json::from_str(&payload)?;
        value["mission"]
            .get("plan")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()).map_err(Into::into))
            .transpose()
    }

    pub fn begin_mission_resume(&self, run: i64, expected: i64, key: &str) -> Result<bool> {
        if key.is_empty() || key.len() > 128 {
            return Err(DbError::Corrupt("Invalid lifecycle command".into()));
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT action FROM lifecycle_commands WHERE run_id=?1 AND command_key=?2",
                params![run, key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(action) = previous {
            if action != "resume" {
                return Err(DbError::Busy("Command identity reused".into()));
            }
            return Ok(false);
        }
        let life = self
            .mission_lifecycle(run)?
            .ok_or(DbError::NotFound("browser-managed mission"))?;
        if self.get_run(run)?.is_some_and(|r| {
            matches!(
                r.status,
                factory_types::RunStatus::Completed | factory_types::RunStatus::Cancelled
            )
        }) {
            return Err(DbError::Busy("Terminal mission cannot resume".into()));
        }
        if life.generation != expected {
            return Err(DbError::Conflict {
                expected,
                current: life.generation,
            });
        }
        if !matches!(
            life.state,
            MissionLifecycleState::Suspended | MissionLifecycleState::RecoveryRequired
        ) {
            return Err(DbError::Busy(
                "Mission has not reached a resumable boundary".into(),
            ));
        }
        let session =
            self.workspace_session_lifecycle(&life.project_id, &life.workspace_session_id)?;
        if session.closed {
            return Err(DbError::Busy(
                "Explicitly reopen the owning workspace session before resume".into(),
            ));
        }
        let commands: i64 = tx.query_row(
            "SELECT count(*) FROM lifecycle_commands WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )?;
        if commands >= 1023 {
            return Err(DbError::Busy("Lifecycle command history limit reached; history retained and safety suspension remains available".into()));
        }
        tx.execute("UPDATE mission_lifecycle SET state='reconciling',generation=generation+1,problem=NULL,updated_at=?1 WHERE run_id=?2 AND generation=?3",params![now(),run,expected])?;
        tx.execute("INSERT INTO lifecycle_commands(run_id,command_key,action,generation) VALUES(?1,?2,'resume',?3)",params![run,key,expected+1])?;
        self.lifecycle_event(
            run,
            "mission.resume_requested",
            &json!({"run_id":run,"generation":expected+1}),
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn commit_mission_resume(&self, run: i64, generation: i64) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let life = self
            .mission_lifecycle(run)?
            .ok_or(DbError::NotFound("mission"))?;
        if life.state != MissionLifecycleState::Reconciling
            || life.generation != generation
            || self.scheduler_running(run)?
        {
            return Err(DbError::Busy("Resume ownership changed".into()));
        }
        if self
            .workspace_session_lifecycle(&life.project_id, &life.workspace_session_id)?
            .closed
        {
            return Err(DbError::Busy(
                "Owning workspace session closed during resume reconciliation".into(),
            ));
        }
        let checkpoint = self
            .mission_checkpoints(run, None)?
            .into_iter()
            .find(|c| Some(c.id) == life.checkpoint_id)
            .ok_or(DbError::NotFound("resume checkpoint"))?;
        if checkpoint.plan_revision != self.get_plan_revision(run)? {
            return Err(DbError::Busy(
                "Plan changed during resume reconciliation".into(),
            ));
        }
        let tasks = self.list_tasks(run)?;
        let task_states = json!(tasks
            .iter()
            .map(|t| json!({"id":t.id,"state":t.state}))
            .collect::<Vec<_>>());
        if task_states != checkpoint.records["tasks"] {
            return Err(DbError::Busy("Task state changed after checkpoint".into()));
        }
        for task in &tasks {
            if matches!(
                task.state,
                factory_types::TaskState::Failed | factory_types::TaskState::Running
            ) {
                let previous = self.latest_task_attempt(task.id)?;
                if previous.as_ref().is_some_and(|a| {
                    a.status == factory_types::AttemptStatus::Approved
                        && a.review
                            .as_ref()
                            .is_some_and(|r| r.decision == factory_types::ReviewDecision::Approve)
                }) {
                    let safe:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM integration_records WHERE run_id=?1 AND task_id=?2 AND attempt_id=?3 AND status='suspended')",params![run,task.id,previous.as_ref().expect("checked attempt").id],|r|r.get(0))?;
                    if safe {
                        tx.execute("UPDATE tasks SET state='awaiting_integration',updated_at=?1 WHERE id=?2 AND state='failed'",params![now(),task.id])?;
                        continue;
                    }
                }
                if previous.as_ref().is_some_and(|a| {
                    !matches!(
                        a.status,
                        factory_types::AttemptStatus::Interrupted
                            | factory_types::AttemptStatus::HandedOff
                    ) || self.task_execution_attempt_count(task.id).unwrap_or(3) >= 3
                }) {
                    return Err(DbError::Busy(
                        "Failed work requires its existing bounded recovery path".into(),
                    ));
                }
                tx.execute("UPDATE tasks SET state='ready',updated_at=?1 WHERE id=?2 AND run_id=?3 AND state IN ('running','failed')",params![now(),task.id,run])?;
            }
        }
        let status = if tasks.is_empty() {
            "planning"
        } else {
            "active"
        };
        tx.execute("UPDATE runs SET status=?1,updated_at=?2 WHERE id=?3 AND status<>'completed' AND status<>'cancelled'",params![status,now(),run])?;
        tx.execute("UPDATE mission_lifecycle SET state='active',updated_at=?1 WHERE run_id=?2 AND generation=?3 AND state='reconciling'",params![now(),run,generation])?;
        self.lifecycle_event(run,"mission.resumed",&json!({"run_id":run,"generation":generation,"notice":"Logical continuation from recorded evidence; external model inference is restarted"}))?;
        tx.commit()?;
        Ok(())
    }

    pub fn mark_lifecycle_runtime_interruption(&self) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        for run in self.lifecycle_runs()? {
            let life = self
                .mission_lifecycle(run)?
                .ok_or(DbError::NotFound("mission"))?;
            if life.state == MissionLifecycleState::Suspended {
                continue;
            }
            let status = self.get_run(run)?.ok_or(DbError::NotFound("run"))?.status;
            if matches!(
                status,
                factory_types::RunStatus::Completed | factory_types::RunStatus::Cancelled
            ) {
                tx.execute(
                    "UPDATE mission_lifecycle SET state='settled' WHERE run_id=?1",
                    [run],
                )?;
                continue;
            }
            tx.execute("UPDATE mission_lifecycle SET state='recovery_required',reason='runtime_interruption',generation=generation+1,problem='Runtime restarted; process and Git state require explicit reconciliation',updated_at=?1 WHERE run_id=?2",params![now(),run])?;
            self.lifecycle_event(
                run,
                "mission.recovery_required",
                &json!({"run_id":run,"reason":"runtime_interruption"}),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod settled_retry_tests {
    use super::*;
    use factory_types::{AttemptStatus, RunStatus, TaskOperation, TaskState};
    const PROJECT: &str = "p_11111111111111111111111111111111";
    const SESSION: &str = "s_11111111111111111111111111111111";

    #[test]
    fn shutdown_selection_preserves_suspended_and_crashed_mission_classifications() {
        let (_dir, db, settled, _, _) = fixture();
        let mut ids = Vec::new();
        for (state, reason) in [
            ("suspended", "user_requested_suspend"),
            ("recovery_required", "runtime_interruption"),
            ("active", ""),
            ("quiescing", "application_shutdown"),
            ("recovery_required", "application_shutdown"),
        ] {
            let run = db
                .create_run("Shutdown selection evidence", None)
                .unwrap()
                .id;
            db.bind_workspace_run(run, PROJECT, SESSION).unwrap();
            db.conn
                .execute(
                    "UPDATE mission_lifecycle SET state=?1,reason=?2 WHERE run_id=?3",
                    params![state, reason, run],
                )
                .unwrap();
            ids.push(run);
        }
        assert_eq!(
            db.shutdown_mission_candidates(false).unwrap(),
            vec![ids[2], ids[3]]
        );
        assert_eq!(
            db.shutdown_mission_candidates(true).unwrap(),
            vec![ids[3], ids[4]]
        );
        assert_eq!(
            db.mission_lifecycle(settled).unwrap().unwrap().state,
            MissionLifecycleState::Settled
        );
        assert_eq!(
            db.mission_lifecycle(ids[1])
                .unwrap()
                .unwrap()
                .reason
                .as_deref(),
            Some("runtime_interruption")
        );
    }

    fn fixture() -> (tempfile::TempDir, FactoryDb, i64, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&dir.path().join("db.sqlite3")).unwrap();
        let run = db.create_run("Controlled explicit retry", None).unwrap().id;
        db.register_workspace_session(PROJECT, SESSION).unwrap();
        db.bind_workspace_run(run, PROJECT, SESSION).unwrap();
        let task = db
            .create_task(
                run,
                "Repair",
                "Preserve unapproved partial evidence",
                &[],
                TaskState::Failed,
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
                "controlled",
                "retained-task-worktree",
                None,
            )
            .unwrap();
        db.finish_task_attempt(
            attempt.id,
            AttemptStatus::Failed,
            Some(0),
            None,
            Some("policy refusal preserved"),
            None,
            None,
        )
        .unwrap();
        db.set_run_status(run, RunStatus::Failed).unwrap();
        db.settle_mission_lifecycle(run).unwrap();
        (dir, db, run, task, attempt.id)
    }
    #[test]
    fn explicit_settled_retry_preserves_history_and_completed_siblings() {
        let (_dir, db, run, task, attempt) = fixture();
        let completed = db
            .create_task(
                run,
                "Already integrated",
                "Do not rerun",
                &[],
                TaskState::Completed,
                1,
                Some("worker"),
                Some(TaskOperation::Implement),
            )
            .unwrap();
        db.retry_settled_task(task, PROJECT, 1).unwrap();
        let life = db.mission_lifecycle(run).unwrap().unwrap();
        assert_eq!(life.state, MissionLifecycleState::Active);
        assert_eq!(life.generation, 2);
        assert_eq!(db.get_run(run).unwrap().unwrap().status, RunStatus::Active);
        assert_eq!(db.get_task(task).unwrap().unwrap().state, TaskState::Ready);
        assert_eq!(
            db.get_task(completed).unwrap().unwrap().state,
            TaskState::Completed
        );
        assert_eq!(
            db.get_task_attempt(attempt)
                .unwrap()
                .unwrap()
                .error
                .as_deref(),
            Some("policy refusal preserved")
        );
        assert!(db.retry_settled_task(task, PROJECT, 1).is_err());
    }
    #[test]
    fn retry_does_not_clear_suspension_closed_sessions_or_foreign_generations() {
        let (_dir, db, run, task, _) = fixture();
        for (project, generation) in [("p_other", 1), (PROJECT, 0)] {
            assert!(db.retry_settled_task(task, project, generation).is_err());
        }
        db.conn
            .execute(
                "UPDATE mission_lifecycle SET state='suspended' WHERE run_id=?1",
                [run],
            )
            .unwrap();
        assert!(db.retry_settled_task(task, PROJECT, 1).is_err());
        db.conn
            .execute(
                "UPDATE mission_lifecycle SET state='settled' WHERE run_id=?1",
                [run],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE workspace_session_lifecycle SET closed=1 WHERE id=?1",
                [SESSION],
            )
            .unwrap();
        assert!(db.retry_settled_task(task, PROJECT, 1).is_err());
        assert_eq!(db.get_task(task).unwrap().unwrap().state, TaskState::Failed);
    }
    #[test]
    fn unsettled_process_and_integration_outcomes_prevent_retry() {
        let (_dir, db, run, task, attempt) = fixture();
        db.conn.execute("INSERT INTO agent_sessions(run_id,task_id,role,agent,command,status,started_at,output_meta) VALUES(?1,?2,'worker','controlled','fixture','success',?3,?4)",params![run,task,now(),json!({"processId":123,"processTreeSupervised":true,"processTreeCleanupConfirmed":false}).to_string()]).unwrap();
        assert!(db.retry_settled_task(task, PROJECT, 1).is_err());
        db.conn.execute("UPDATE agent_sessions SET output_meta=?1 WHERE run_id=?2",params![json!({"processId":123,"processTreeSupervised":true,"processTreeCleanupConfirmed":true}).to_string(),run]).unwrap();
        let integration = db
            .begin_integration(
                run,
                task,
                attempt,
                &"a".repeat(40),
                &"b".repeat(40),
                "preserved-candidate",
            )
            .unwrap();
        for status in ["preparing", "publishing", "conflict", "published"] {
            db.update_integration_record(integration, status, None, &json!({}))
                .unwrap();
            assert!(db.retry_settled_task(task, PROJECT, 1).is_err(), "{status}");
        }
        assert_eq!(
            db.mission_lifecycle(run).unwrap().unwrap().state,
            MissionLifecycleState::Settled
        );
    }
    #[test]
    fn completed_cancelled_or_owned_runs_are_not_reexecuted() {
        let (_dir, db, run, task, _) = fixture();
        for status in [RunStatus::Completed, RunStatus::Cancelled] {
            db.set_run_status(run, status).unwrap();
            assert!(db.retry_settled_task(task, PROJECT, 1).is_err());
        }
        db.set_run_status(run, RunStatus::Failed).unwrap();
        db.conn.execute("INSERT INTO scheduler_owners(id,run_id,status,started_at) VALUES('active-owner',?1,'running',?2)",params![run,now()]).unwrap();
        assert!(db.retry_settled_task(task, PROJECT, 1).is_err());
        assert_eq!(db.get_task(task).unwrap().unwrap().state, TaskState::Failed);
    }

    #[test]
    fn retry_settlement_presentation_is_current_durable_and_generation_deduplicated() {
        let (_dir, db, run, task, _) = fixture();
        db.retry_settled_task(task, PROJECT, 1).unwrap();
        db.set_task_state(task, TaskState::Completed).unwrap();
        db.set_run_status(run, RunStatus::Completed).unwrap();
        db.settle_mission_lifecycle(run).unwrap();
        let events = db.scheduler_events(run, 0, 1000).unwrap();
        let settled: Vec<_> = events
            .iter()
            .filter(|event| event.event_type == "mission.execution_settled")
            .collect();
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].payload["generation"], 2);
        assert_eq!(settled[0].payload["status"], "completed");
        db.conversation_create("c_retry", PROJECT, "Controlled retry history")
            .unwrap();
        let (command, _) = db
            .conversation_accept(
                "c_retry",
                "original",
                "fingerprint",
                "chat",
                "Controlled objective",
                &json!({}),
            )
            .unwrap();
        db.conversation_state(
            command.id,
            "finished",
            "Original failed outcome preserved",
            &json!({"source":"controlled"}),
            None,
        )
        .unwrap();
        db.conn.execute("INSERT INTO conversation_missions(run_id,conversation_id,command_id,snapshot,created_at) VALUES(?1,'c_retry',?2,?3,?4)",params![run,command.id,json!({"status":"completed"}).to_string(),now()]).unwrap();
        assert!(db
            .conversation_observation_ids("c_retry")
            .unwrap()
            .is_empty());
        for _ in 0..2 {
            db.reconcile_settled_retry_presentation(run).unwrap();
        }
        assert_eq!(
            db.scheduler_events(run, 0, 1000)
                .unwrap()
                .iter()
                .filter(|event| event.event_type == "mission.execution_settled")
                .count(),
            1
        );
        db.conn.execute("DELETE FROM scheduler_events WHERE run_id=?1 AND event_type='mission.execution_settled'",[run]).unwrap();
        assert_eq!(
            db.conversation_observation_ids("c_retry").unwrap(),
            vec![run]
        );
        for _ in 0..2 {
            db.reconcile_settled_retry_presentation(run).unwrap();
        }
        assert_eq!(
            db.scheduler_events(run, 0, 1000)
                .unwrap()
                .iter()
                .filter(|event| event.event_type == "mission.execution_settled")
                .count(),
            1
        );
        assert_eq!(
            db.mission_lifecycle(run).unwrap().unwrap().state,
            MissionLifecycleState::Settled
        );
        assert_eq!(db.list_task_attempts(run).unwrap().len(), 1);
        let messages = db.conversation_messages("c_retry", None, 100).unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message
                    .content
                    .starts_with(&format!("Mission #{run} is completed.")))
                .count(),
            1
        );
        assert!(messages
            .iter()
            .any(|message| message.content == "Original failed outcome preserved"));
        assert!(db
            .conversation_observation_ids("c_retry")
            .unwrap()
            .is_empty());
    }
}
