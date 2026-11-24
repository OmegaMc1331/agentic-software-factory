use crate::{now, DbError, FactoryDb, Result};
use factory_types::{
    AgentExecutionSnapshot, AgentHandoff, AgentProfile, AgentSession, TaskEvidence,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

pub(crate) const SCHEMA: &str = "
CREATE TABLE agent_profiles(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, revision INTEGER NOT NULL, archived INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE TABLE agent_profile_revisions(profile_id TEXT NOT NULL REFERENCES agent_profiles(id), revision INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(profile_id,revision));
CREATE TABLE agent_execution_snapshots(session_id INTEGER PRIMARY KEY REFERENCES agent_sessions(id), profile_id TEXT NOT NULL REFERENCES agent_profiles(id), profile_revision INTEGER NOT NULL, payload TEXT NOT NULL, observation TEXT);
CREATE INDEX agent_snapshots_profile ON agent_execution_snapshots(profile_id,session_id);
CREATE TABLE agent_handoffs(id TEXT PRIMARY KEY, run_id INTEGER NOT NULL REFERENCES runs(id), task_id INTEGER NOT NULL REFERENCES tasks(id), operation_key TEXT NOT NULL, state TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(run_id,operation_key));
CREATE UNIQUE INDEX handoff_one_active ON agent_handoffs(task_id) WHERE state IN ('waiting','ready','executing');
CREATE INDEX handoff_run ON agent_handoffs(run_id,id);
";

impl FactoryDb {
    pub fn agent_profiles(&self, archived: bool) -> Result<Vec<AgentProfile>> {
        let mut query = self.conn.prepare(
            "SELECT payload FROM agent_profiles WHERE (?1 OR archived=0) ORDER BY id LIMIT 1000",
        )?;
        let rows = query.query_map([archived], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn agent_profile(&self, id: &str) -> Result<AgentProfile> {
        let payload: String = self
            .conn
            .query_row(
                "SELECT payload FROM agent_profiles WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(DbError::NotFound("agent profile"))?;
        Ok(serde_json::from_str(&payload)?)
    }

    pub fn save_agent_profile(&self, profile: &AgentProfile, expected: Option<i64>) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let current: Option<i64> = tx
            .query_row(
                "SELECT revision FROM agent_profiles WHERE id=?1",
                [&profile.id],
                |r| r.get(0),
            )
            .optional()?;
        if current != expected {
            return Err(DbError::Conflict {
                expected: expected.unwrap_or(0),
                current: current.unwrap_or(0),
            });
        }
        if profile.revision != expected.unwrap_or(0) + 1 {
            return Err(DbError::Corrupt("Invalid agent profile revision".into()));
        }
        if current.is_none() {
            let count: i64 =
                tx.query_row("SELECT COUNT(*) FROM agent_profiles", [], |r| r.get(0))?;
            if count >= 1000 {
                return Err(DbError::Busy("Project profile limit reached".into()));
            }
        }
        if !profile.archived
            && profile.configuration.enabled
            && profile.configuration.automatic_delegation
            && profile.configuration.preferred
        {
            let mut query=tx.prepare("SELECT payload FROM agent_profiles WHERE id<>?1 AND archived=0 AND json_extract(payload,'$.enabled')=1 AND json_extract(payload,'$.automaticDelegation')=1 AND json_extract(payload,'$.preferred')=1")?;
            let rows = query.query_map([&profile.id], |r| r.get::<_, String>(0))?;
            for row in rows {
                let other: AgentProfile = serde_json::from_str(&row?)?;
                if other
                    .configuration
                    .roles
                    .iter()
                    .any(|r| profile.configuration.roles.contains(r))
                {
                    return Err(DbError::Busy(
                        "Another profile is already preferred for this role".into(),
                    ));
                }
            }
        }
        let payload = serde_json::to_string(profile)?;
        tx.execute("INSERT INTO agent_profiles VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,archived=excluded.archived,payload=excluded.payload", params![profile.id,profile.project_id,profile.revision,profile.archived,payload])?;
        tx.execute(
            "INSERT INTO agent_profile_revisions VALUES(?1,?2,?3)",
            params![profile.id, profile.revision, payload],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn agent_profile_revisions(
        &self,
        id: &str,
        before: Option<i64>,
    ) -> Result<Vec<AgentProfile>> {
        self.agent_profile(id)?;
        let mut query = self.conn.prepare("SELECT payload FROM agent_profile_revisions WHERE profile_id=?1 AND (?2 IS NULL OR revision<?2) ORDER BY revision DESC LIMIT 100")?;
        let rows = query.query_map(params![id, before], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn insert_profile_session(
        &self,
        session: &AgentSession,
        snapshot: &AgentExecutionSnapshot,
    ) -> Result<AgentSession> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let current = self.agent_profile(&snapshot.profile.id)?;
        if current.revision != snapshot.profile.revision {
            return Err(DbError::Conflict {
                expected: snapshot.profile.revision,
                current: current.revision,
            });
        }
        if !current.configuration.enabled || current.archived {
            return Err(DbError::Busy(
                "Agent profile is disabled or archived".into(),
            ));
        }
        let session = self.insert_agent_session(session)?;
        tx.execute("INSERT INTO agent_execution_snapshots(session_id,profile_id,profile_revision,payload) VALUES(?1,?2,?3,?4)", params![session.id,current.id,current.revision,serde_json::to_string(snapshot)?])?;
        if let Some(task) = session.task_id {
            if let Some(mut handoff) = self.active_handoff(task)? {
                if handoff.state == "ready"
                    && handoff.target.id == current.id
                    && handoff.target.revision != current.revision
                {
                    return Err(DbError::Conflict {
                        expected: handoff.target.revision,
                        current: current.revision,
                    });
                }
                if handoff.state == "ready"
                    && handoff.target.id == current.id
                    && handoff.target.revision == current.revision
                {
                    handoff.state = "executing".into();
                    handoff.replacement_session_id = Some(session.id);
                    self.write_handoff(&mut handoff)?;
                }
            }
        }
        tx.commit()?;
        Ok(session)
    }

    pub fn execution_snapshot(&self, session: i64) -> Result<Option<AgentExecutionSnapshot>> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM agent_execution_snapshots WHERE session_id=?1",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(DbError::from))
            .transpose()
    }

    pub fn attempt_execution_snapshot(
        &self,
        attempt: i64,
        agent: &str,
        role: &str,
    ) -> Result<Option<AgentExecutionSnapshot>> {
        let payload:Option<String>=self.conn.query_row("SELECT x.payload FROM agent_execution_snapshots x JOIN agent_sessions s ON x.session_id=s.id WHERE s.attempt_id=?1 AND s.agent=?2 AND s.role=?3 ORDER BY s.id LIMIT 1",params![attempt,agent,role],|r|r.get(0)).optional()?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(DbError::from))
            .transpose()
    }

    pub fn record_model_observation(
        &self,
        session: i64,
        observation: &serde_json::Value,
    ) -> Result<()> {
        self.conn.execute("UPDATE agent_execution_snapshots SET observation=?2 WHERE session_id=?1 AND observation IS NULL",params![session,observation.to_string()])?;
        Ok(())
    }

    pub fn model_observation(&self, session: i64) -> Result<Option<serde_json::Value>> {
        let observation: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT observation FROM agent_execution_snapshots WHERE session_id=?1",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        observation
            .flatten()
            .map(|s| serde_json::from_str(&s).map_err(DbError::from))
            .transpose()
    }

    pub fn active_handoffs_for_run(&self, run: i64) -> Result<Vec<AgentHandoff>> {
        let mut query=self.conn.prepare("SELECT payload FROM agent_handoffs WHERE run_id=?1 AND state IN ('waiting','ready','executing') ORDER BY rowid LIMIT 1000")?;
        let rows = query.query_map([run], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn pin_idle_task(&self, task: i64, agent: Option<&str>) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let record = self.get_task(task)?.ok_or(DbError::NotFound("task"))?;
        let claimed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_dispatches WHERE task_id=?1 AND finished_at IS NULL)",
            [task],
            |r| r.get(0),
        )?;
        if claimed
            || self.active_handoff(task)?.is_some()
            || !matches!(
                record.state,
                factory_types::TaskState::Pending
                    | factory_types::TaskState::Ready
                    | factory_types::TaskState::Blocked
                    | factory_types::TaskState::Failed
            )
        {
            return Err(DbError::Busy(
                "Task is already claimed or has a pending handoff".into(),
            ));
        }
        self.set_task_agent_override(task, agent)?;
        tx.commit()?;
        Ok(())
    }

    pub fn handoffs(&self, run: i64) -> Result<Vec<AgentHandoff>> {
        self.handoffs_before(run, None)
    }

    pub fn handoffs_before(&self, run: i64, before: Option<&str>) -> Result<Vec<AgentHandoff>> {
        if let Some(before) = before {
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_handoffs WHERE run_id=?1 AND id=?2)",
                params![run, before],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(DbError::NotFound("handoff cursor"));
            }
        }
        let mut query = self.conn.prepare(
            "SELECT payload FROM agent_handoffs WHERE run_id=?1 AND (?2 IS NULL OR rowid<(SELECT rowid FROM agent_handoffs WHERE run_id=?1 AND id=?2)) ORDER BY rowid DESC LIMIT 200",
        )?;
        let rows = query.query_map(params![run, before], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn handoff_by_key(&self, run: i64, key: &str) -> Result<Option<AgentHandoff>> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM agent_handoffs WHERE run_id=?1 AND operation_key=?2",
                params![run, key],
                |r| r.get(0),
            )
            .optional()?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(DbError::from))
            .transpose()
    }

    pub fn active_handoff(&self, task: i64) -> Result<Option<AgentHandoff>> {
        let payload: Option<String> = self.conn.query_row("SELECT payload FROM agent_handoffs WHERE task_id=?1 AND state IN ('waiting','ready','executing')", [task], |r| r.get(0)).optional()?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(DbError::from))
            .transpose()
    }

    pub fn request_handoff(&self, handoff: &AgentHandoff) -> Result<AgentHandoff> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if let Some(existing) = self.handoff_by_key(handoff.run_id, &handoff.idempotency_key)? {
            if existing.task_id != handoff.task_id
                || existing.target.id != handoff.target.id
                || existing.target.revision != handoff.target.revision
                || existing.reason != handoff.reason
            {
                return Err(DbError::Busy(
                    "Idempotency key already has a different handoff".into(),
                ));
            }
            return Ok(existing);
        }
        if self.active_handoff(handoff.task_id)?.is_some() {
            return Err(DbError::Busy("Handoff already pending for task".into()));
        }
        let task = self
            .get_task(handoff.task_id)?
            .ok_or(DbError::NotFound("task"))?;
        let run = self
            .get_run(handoff.run_id)?
            .ok_or(DbError::NotFound("run"))?;
        if task.run_id != handoff.run_id
            || !matches!(
                run.status,
                factory_types::RunStatus::Planned
                    | factory_types::RunStatus::Active
                    | factory_types::RunStatus::Blocked
            )
        {
            return Err(DbError::Busy("Task no longer eligible".into()));
        }
        let mut record = handoff.clone();
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agent_handoffs WHERE run_id=?1",
            [handoff.run_id],
            |r| r.get(0),
        )?;
        if count >= 1024 {
            return Err(DbError::Busy("Handoff audit limit reached".into()));
        }
        let target = self.agent_profile(&record.target.id)?;
        if target.revision != record.target.revision
            || !target.configuration.enabled
            || target.archived
        {
            return Err(DbError::Busy(
                "Replacement profile changed since validation".into(),
            ));
        }
        match task.state {
            factory_types::TaskState::Pending | factory_types::TaskState::Ready => {
                let claimed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM task_dispatches WHERE task_id=?1 AND finished_at IS NULL)", [task.id], |r| r.get(0))?;
                if claimed {
                    return Err(DbError::Busy(
                        "Dispatch already claimed; wait for Worker startup".into(),
                    ));
                }
                record.state = "ready".into();
                record.boundary = "before_dispatch".into();
                self.set_task_agent_override(task.id, Some(&record.target.id))?;
            }
            factory_types::TaskState::Running => {
                let attempt = self
                    .latest_task_attempt(task.id)?
                    .ok_or(DbError::Busy("No active attempt".into()))?;
                if attempt.status != factory_types::AttemptStatus::Running
                    || attempt.attempt_number >= 3
                {
                    return Err(DbError::Busy(
                        "Unsafe boundary: review has started or attempt limit reached".into(),
                    ));
                }
                record.state = "waiting".into();
                record.boundary = "worker_exit_and_evidence_capture".into();
                record.from_attempt_id = Some(attempt.id);
                record.from_agent = Some(attempt.agent);
                record.from_session_id=tx.query_row("SELECT id FROM agent_sessions WHERE attempt_id=?1 AND role=?2 ORDER BY id DESC LIMIT 1",params![attempt.id,task.role.as_deref().unwrap_or("worker")],|r|r.get(0)).optional()?;
            }
            _ => {
                return Err(DbError::Busy(
                    "Unsafe handoff boundary: task is reviewing, integrating or terminal".into(),
                ))
            }
        }
        tx.execute(
            "INSERT INTO agent_handoffs VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                record.id,
                record.run_id,
                record.task_id,
                record.idempotency_key,
                record.state,
                serde_json::to_string(&record)?
            ],
        )?;
        self.scheduler_event(
            run.id,
            Some(task.id),
            "agent_handoff_requested",
            &serde_json::json!({"id":record.id,"state":record.state,"target":record.target.id}),
        )?;
        tx.commit()?;
        Ok(record)
    }

    fn write_handoff(&self, handoff: &mut AgentHandoff) -> Result<()> {
        handoff.updated_at = now();
        self.conn.execute(
            "UPDATE agent_handoffs SET state=?2,payload=?3 WHERE id=?1",
            params![handoff.id, handoff.state, serde_json::to_string(handoff)?],
        )?;
        Ok(())
    }

    pub fn record_rejected_handoff(&self, handoff: &AgentHandoff, reason: &str) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if let Some(existing) = self.handoff_by_key(handoff.run_id, &handoff.idempotency_key)? {
            if existing.state == "rejected"
                && existing.task_id == handoff.task_id
                && existing.target.id == handoff.target.id
                && existing.target.revision == handoff.target.revision
                && existing.reason == handoff.reason
            {
                return Ok(());
            }
            return Err(DbError::Busy(
                "Conflicting handoff operation already recorded".into(),
            ));
        }
        if self
            .get_task(handoff.task_id)?
            .is_none_or(|t| t.run_id != handoff.run_id)
        {
            return Err(DbError::NotFound("task"));
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agent_handoffs WHERE run_id=?1",
            [handoff.run_id],
            |r| r.get(0),
        )?;
        if count >= 1024 {
            return Err(DbError::Busy("Handoff audit limit reached".into()));
        }
        let mut record = handoff.clone();
        record.state = "rejected".into();
        if let Some(attempt) = self.latest_task_attempt(record.task_id)? {
            record.from_agent = Some(attempt.agent);
            record.from_attempt_id = Some(attempt.id);
            record.from_session_id=tx.query_row("SELECT id FROM agent_sessions WHERE attempt_id=?1 AND role=?2 ORDER BY id DESC LIMIT 1",params![attempt.id,attempt.role.as_deref().unwrap_or("worker")],|r|r.get(0)).optional()?;
        }
        record.boundary = "authorization_validation".into();
        record.evidence = serde_json::json!({"authorization":"denied","reason":reason.chars().take(2000).collect::<String>(),"truncated":reason.chars().count()>2000});
        tx.execute(
            "INSERT INTO agent_handoffs VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                record.id,
                record.run_id,
                record.task_id,
                record.idempotency_key,
                record.state,
                serde_json::to_string(&record)?
            ],
        )?;
        self.scheduler_event(
            record.run_id,
            Some(record.task_id),
            "agent_handoff_rejected",
            &serde_json::json!({"id":record.id,"target":record.target.id}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn handoff_boundary(
        &self,
        task: i64,
        attempt: i64,
        evidence: &TaskEvidence,
    ) -> Result<bool> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let handoff = self.active_handoff(task)?;
        let Some(mut handoff) =
            handoff.filter(|h| h.state == "waiting" && h.from_attempt_id == Some(attempt))
        else {
            self.set_task_attempt_status(attempt, factory_types::AttemptStatus::Reviewing)?;
            tx.commit()?;
            return Ok(false);
        };
        let live: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE attempt_id=?1 AND status='running')",
            [attempt],
            |r| r.get(0),
        )?;
        if live {
            return Err(DbError::Busy(
                "Worker process has not reached a confirmed exit".into(),
            ));
        }
        self.finish_task_attempt(attempt,factory_types::AttemptStatus::HandedOff,Some(0),evidence.commit_sha.as_deref(),Some("Worker exited; captured changes handed to a replacement for independent continuation and review"),Some(evidence),None)?;
        self.set_task_agent_override(task, Some(&handoff.target.id))?;
        handoff.state = "ready".into();
        handoff.evidence = serde_json::json!({"attemptId":attempt,"evidence":evidence,"notice":"Captured work is unapproved; original source base is retained for final review"});
        self.write_handoff(&mut handoff)?;
        self.scheduler_event(
            handoff.run_id,
            Some(task),
            "agent_handoff_boundary",
            &serde_json::json!({"id":handoff.id,"attemptId":attempt,"commit":evidence.commit_sha}),
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn handoff_resume_boundary(
        &self,
        task: i64,
        attempt: i64,
        evidence: &TaskEvidence,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let Some(mut h) = self
            .active_handoff(task)?
            .filter(|h| h.state == "waiting" && h.from_attempt_id == Some(attempt))
        else {
            return Err(DbError::Busy("Handoff boundary changed".into()));
        };
        let life = self
            .mission_lifecycle(h.run_id)?
            .ok_or(DbError::NotFound("mission lifecycle"))?;
        let old = self
            .get_task_attempt(attempt)?
            .ok_or(DbError::NotFound("interrupted attempt"))?;
        if old.task_id != task || old.status != factory_types::AttemptStatus::Interrupted {
            return Err(DbError::Busy(
                "Only a reconciled interrupted attempt may cross this handoff boundary".into(),
            ));
        }
        if life.state != factory_types::MissionLifecycleState::Reconciling
            || self.active_dispatch_count(h.run_id)? > 0
        {
            return Err(DbError::Busy(
                "Handoff resume requires a fenced and drained reconciliation boundary".into(),
            ));
        }
        let current = self.agent_profile(&h.target.id)?;
        if current.revision != h.target.revision
            || current.archived
            || !current.configuration.enabled
        {
            return Err(DbError::Busy("Handoff target changed".into()));
        }
        h.state = "ready".into();
        h.boundary = "reconciled_suspension".into();
        h.evidence = serde_json::json!({"attemptId":attempt,"evidence":evidence,"lifecycleGeneration":life.generation,"notice":"Interrupted attempt remains historical and unapproved; replacement starts a new invocation"});
        self.set_task_agent_override(task, Some(&h.target.id))?;
        self.write_handoff(&mut h)?;
        self.scheduler_event(
            h.run_id,
            Some(task),
            "agent_handoff_boundary",
            &serde_json::json!({"id":h.id,"attemptId":attempt,"boundary":h.boundary}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn reconcile_handoffs(&self, run: i64) -> Result<()> {
        if self.lifecycle_fenced(run)? {
            return Ok(());
        }
        for mut handoff in self.active_handoffs_for_run(run)? {
            if !matches!(handoff.state.as_str(), "waiting" | "ready" | "executing") {
                continue;
            }
            let task = self
                .get_task(handoff.task_id)?
                .ok_or(DbError::NotFound("task"))?;
            let status = self.get_run(run)?.ok_or(DbError::NotFound("run"))?.status;
            if task.state == factory_types::TaskState::Completed {
                handoff.state = if handoff.replacement_session_id.is_some() {
                    "completed"
                } else {
                    "failed"
                }
                .into();
            } else if status == factory_types::RunStatus::Cancelled {
                handoff.state = "cancelled".into();
            } else if matches!(
                status,
                factory_types::RunStatus::Failed | factory_types::RunStatus::Blocked
            ) || task.state == factory_types::TaskState::Superseded
            {
                handoff.state = "failed".into();
            } else {
                continue;
            }
            self.write_handoff(&mut handoff)?;
            self.scheduler_event(
                run,
                Some(task.id),
                "agent_handoff_finished",
                &serde_json::json!({"id":handoff.id,"state":handoff.state}),
            )?;
        }
        Ok(())
    }
}
