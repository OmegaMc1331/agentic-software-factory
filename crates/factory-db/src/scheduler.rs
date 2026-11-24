use factory_types::{IntegrationRecord, SchedulerEvent, SchedulerPolicy};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::{now, DbError, FactoryDb, Result};

pub(crate) const SCHEMA: &str = "
CREATE TABLE scheduler_policies (
    run_id INTEGER PRIMARY KEY REFERENCES runs(id),
    policy TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE scheduler_owners (
    id TEXT PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES runs(id),
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT
);
CREATE UNIQUE INDEX scheduler_one_owner ON scheduler_owners(run_id) WHERE finished_at IS NULL;
CREATE TABLE task_dispatches (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES scheduler_owners(id),
    run_id INTEGER NOT NULL REFERENCES runs(id),
    task_id INTEGER NOT NULL REFERENCES tasks(id),
    revision INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    outcome TEXT
);
CREATE UNIQUE INDEX task_one_dispatch ON task_dispatches(task_id) WHERE finished_at IS NULL;
CREATE TABLE scheduler_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL DEFAULT 1,
    run_id INTEGER NOT NULL REFERENCES runs(id),
    task_id INTEGER REFERENCES tasks(id),
    revision INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX scheduler_events_run ON scheduler_events(run_id,id);
CREATE TABLE integration_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id),
    task_id INTEGER NOT NULL REFERENCES tasks(id),
    attempt_id INTEGER NOT NULL REFERENCES task_attempts(id),
    before_sha TEXT NOT NULL,
    source_sha TEXT NOT NULL,
    candidate_sha TEXT,
    worktree_path TEXT NOT NULL,
    status TEXT NOT NULL,
    details TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX integration_one_publication ON integration_records(attempt_id) WHERE status = 'published';
CREATE UNIQUE INDEX integration_one_lane ON integration_records(run_id) WHERE status IN ('preparing','publishing');
";

impl FactoryDb {
    pub fn apply_scheduler_replan(
        &self,
        run_id: i64,
        owner: &str,
        expected: i64,
        plan: &factory_types::Plan,
        reason: &str,
        session: i64,
    ) -> Result<factory_types::PlanApplyOutcome> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM scheduler_owners WHERE run_id=?1 AND id=?2 AND finished_at IS NULL AND status='running') AND NOT EXISTS(SELECT 1 FROM task_dispatches WHERE run_id=?1 AND finished_at IS NULL)", params![run_id,owner], |r|r.get(0))?;
        if !owned {
            return Err(DbError::Busy(
                "Replanning requires the current owner and a drained dispatch barrier.".into(),
            ));
        }
        if self.lifecycle_fenced(run_id)? {
            return Err(DbError::Busy("Plan revision fenced by lifecycle".into()));
        }
        let state = crate::load_plan_state(&tx, run_id)?;
        if state.revision != expected {
            return Err(DbError::Conflict {
                expected,
                current: state.revision,
            });
        }
        let resolved = match factory_types::plan_edit::resolve_unfinished_replan(&state, plan) {
            Ok(value) => value,
            Err(diagnostics) => return Ok(factory_types::PlanApplyOutcome::Invalid(diagnostics)),
        };
        let (revision, tasks) = crate::apply_resolved(
            &tx,
            run_id,
            &state,
            &resolved,
            factory_types::PlanRevisionSource::Replan,
            Some(reason),
            Some(session),
        )?;
        tx.commit()?;
        Ok(factory_types::PlanApplyOutcome::Applied {
            run_id,
            revision,
            tasks,
        })
    }

    pub fn publish_integration(&self, record_id: i64, sha: &str) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let (run,task): (i64,i64) = tx.query_row("SELECT run_id,task_id FROM integration_records WHERE id=?1 AND status IN ('publishing','published') AND candidate_sha=?2", params![record_id,sha], |r|Ok((r.get(0)?,r.get(1)?)))?;
        tx.execute(
            "UPDATE integration_records SET status='published',updated_at=?1 WHERE id=?2",
            params![now(), record_id],
        )?;
        tx.execute(
            "UPDATE runs SET integration_sha=?1,updated_at=?2 WHERE id=?3",
            params![sha, now(), run],
        )?;
        tx.execute("UPDATE tasks SET state='completed',updated_at=?1 WHERE id=?2 AND state IN ('integrating','awaiting_integration','completed')",params![now(),task])?;
        tx.commit()?;
        Ok(())
    }
    pub fn scheduler_policy(&self, run_id: i64) -> Result<Option<SchedulerPolicy>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT policy FROM scheduler_policies WHERE run_id=?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|text| serde_json::from_str(&text).map_err(Into::into))
            .transpose()
    }

    pub fn configure_scheduler(&self, run_id: i64, policy: &SchedulerPolicy) -> Result<()> {
        policy.validate().map_err(DbError::Corrupt)?;
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM task_attempts a JOIN tasks t ON t.id=a.task_id WHERE t.run_id=?1) OR EXISTS(SELECT 1 FROM scheduler_owners WHERE run_id=?1)", [run_id], |row| row.get(0))?;
        if active {
            return Err(DbError::Busy(
                "Scheduler policy is immutable after execution starts.".into(),
            ));
        }
        tx.execute("INSERT INTO scheduler_policies(run_id,policy,created_at) VALUES(?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET policy=excluded.policy", params![run_id,serde_json::to_string(policy)?,now()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn acquire_scheduler(&self, run_id: i64, owner: &str) -> Result<bool> {
        let changed = self.conn.execute("INSERT OR IGNORE INTO scheduler_owners(id,run_id,status,started_at) SELECT ?1,id,'running',?2 FROM runs WHERE id=?3 AND status='active' AND NOT EXISTS(SELECT 1 FROM mission_lifecycle WHERE run_id=?3 AND state<>'active')", params![owner,now(),run_id])?;
        Ok(changed == 1)
    }

    pub fn finish_scheduler(&self, owner: &str, status: &str) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        tx.execute("UPDATE task_dispatches SET finished_at=?1,outcome=COALESCE(outcome,'owner ended; inspect external effects') WHERE owner_id=?2 AND finished_at IS NULL",params![now(),owner])?;
        tx.execute("UPDATE tool_grants SET revoked_at=?1 WHERE revoked_at IS NULL AND run_id=(SELECT run_id FROM scheduler_owners WHERE id=?2)",params![now(),owner])?;
        tx.execute("UPDATE tool_invocations SET status='cancelled',error=COALESCE(error,'Owned execution stopped; inspect external effects') WHERE status='running' AND run_id=(SELECT run_id FROM scheduler_owners WHERE id=?1)",[owner])?;
        tx.execute("UPDATE agent_sessions SET status='interrupted',finished_at=?1 WHERE status IN ('running','active') AND run_id=(SELECT run_id FROM scheduler_owners WHERE id=?2)",params![now(),owner])?;
        tx.execute("UPDATE task_attempts SET status='interrupted',finished_at=?1,error=COALESCE(error,'Owned process stopped before result persistence') WHERE status IN ('running','reviewing') AND task_id IN (SELECT id FROM tasks WHERE run_id=(SELECT run_id FROM scheduler_owners WHERE id=?2))",params![now(),owner])?;
        tx.execute("UPDATE tasks SET state='failed',updated_at=?1 WHERE state='running' AND run_id=(SELECT run_id FROM scheduler_owners WHERE id=?2)",params![now(),owner])?;
        tx.execute("UPDATE scheduler_owners SET status=?1,finished_at=?2 WHERE id=?3 AND finished_at IS NULL", params![status,now(),owner])?;
        tx.commit()?;
        Ok(())
    }

    pub fn scheduler_stopping(&self, owner: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM scheduler_owners WHERE id=?1 AND status='cancelling' AND finished_at IS NULL)",[owner],|row|row.get(0))?)
    }

    pub fn interrupt_scheduler_run(&self, run_id: i64) -> Result<()> {
        let owners = self
            .conn
            .prepare("SELECT id FROM scheduler_owners WHERE run_id=?1 AND finished_at IS NULL")?
            .query_map([run_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for owner in owners {
            self.finish_scheduler(&owner, "interrupted")?;
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        tx.execute("UPDATE agent_sessions SET status='interrupted',finished_at=?1 WHERE run_id=?2 AND status IN ('running','active')",params![now(),run_id])?;
        tx.execute(
            "UPDATE tool_grants SET revoked_at=?1 WHERE run_id=?2 AND revoked_at IS NULL",
            params![now(), run_id],
        )?;
        tx.execute("UPDATE tool_invocations SET status='cancelled',error=COALESCE(error,'Coordinator interrupted; inspect external effects') WHERE run_id=?1 AND status='running'",[run_id])?;
        tx.execute(
            "UPDATE runs SET status='failed',updated_at=?1 WHERE id=?2",
            params![now(), run_id],
        )?;
        tx.commit()?;
        self.reconcile_handoffs(run_id)?;
        Ok(())
    }

    pub fn request_scheduler_stop(&self, run_id: i64) -> Result<()> {
        self.conn.execute("UPDATE scheduler_owners SET status='cancelling' WHERE run_id=?1 AND finished_at IS NULL",[run_id])?;
        Ok(())
    }

    pub fn scheduler_running(&self, run_id: i64) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM scheduler_owners WHERE run_id=?1 AND finished_at IS NULL)",
            [run_id],
            |row| row.get(0),
        )?)
    }

    pub fn claim_dispatch(
        &self,
        run_id: i64,
        task_id: i64,
        revision: i64,
        owner: &str,
        claim: &str,
    ) -> Result<bool> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if self.lifecycle_fenced(run_id)? {
            return Ok(false);
        }
        let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks t JOIN runs r ON r.id=t.run_id JOIN scheduler_owners o ON o.run_id=r.id WHERE t.id=?1 AND r.id=?2 AND t.state='ready' AND r.status='active' AND r.plan_revision=?3 AND o.id=?4 AND o.status='running' AND o.finished_at IS NULL AND NOT EXISTS(SELECT 1 FROM task_dependencies d JOIN tasks p ON p.id=d.depends_on WHERE d.task_id=t.id AND p.state NOT IN ('completed','superseded')))", params![task_id,run_id,revision,owner], |row|row.get(0))?;
        if !valid {
            return Ok(false);
        }
        let changed = tx.execute("INSERT OR IGNORE INTO task_dispatches(id,owner_id,run_id,task_id,revision,started_at) VALUES(?1,?2,?3,?4,?5,?6)", params![claim,owner,run_id,task_id,revision,now()])?;
        if changed != 1 {
            return Ok(false);
        }
        tx.execute(
            "UPDATE tasks SET state='running',updated_at=?1 WHERE id=?2 AND state='ready'",
            params![now(), task_id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn finish_dispatch(&self, claim: &str, outcome: &str) -> Result<()> {
        self.conn.execute("UPDATE task_dispatches SET outcome=?1,finished_at=?2 WHERE id=?3 AND finished_at IS NULL", params![outcome,now(),claim])?;
        Ok(())
    }

    pub fn task_has_dispatch(&self, task_id: i64) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_dispatches WHERE task_id=?1 AND finished_at IS NULL)",
            [task_id],
            |row| row.get(0),
        )?)
    }

    pub fn active_dispatch_count(&self, run_id: i64) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM task_dispatches WHERE run_id=?1 AND finished_at IS NULL",
            [run_id],
            |row| row.get::<_, i64>(0),
        )? as usize)
    }

    pub fn scheduler_event(
        &self,
        run_id: i64,
        task_id: Option<i64>,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        let text = serde_json::to_string(payload)?;
        if text.len() > 64 * 1024 {
            return Err(DbError::Corrupt("Scheduler event exceeds 64 KiB".into()));
        }
        self.conn.execute("INSERT INTO scheduler_events(run_id,task_id,revision,event_type,payload,created_at) SELECT id,?1,plan_revision,?2,?3,?4 FROM runs WHERE id=?5", params![task_id,event_type,text,now(),run_id])?;
        Ok(())
    }

    pub fn recent_scheduler_events(&self, run_id: i64) -> Result<Vec<SchedulerEvent>> {
        let newest: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id),0) FROM scheduler_events WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )?;
        self.scheduler_events(run_id, newest.saturating_sub(200), 200)
    }

    pub fn scheduler_events(
        &self,
        run_id: i64,
        after: i64,
        limit: usize,
    ) -> Result<Vec<SchedulerEvent>> {
        let mut query=self.conn.prepare("SELECT id,schema_version,run_id,task_id,revision,event_type,payload,created_at FROM scheduler_events WHERE run_id=?1 AND id>?2 ORDER BY id LIMIT ?3")?;
        let rows = query
            .query_map(params![run_id, after, limit.clamp(1, 200) as i64], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get::<_, String>(6)?,
                    row.get(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    id,
                    schema_version,
                    run_id,
                    task_id,
                    revision,
                    event_type,
                    payload,
                    created_at,
                )| {
                    Ok(SchedulerEvent {
                        id,
                        schema_version,
                        run_id,
                        task_id,
                        revision,
                        event_type,
                        payload: serde_json::from_str(&payload)?,
                        created_at,
                    })
                },
            )
            .collect()
    }

    pub fn begin_integration(
        &self,
        run_id: i64,
        task_id: i64,
        attempt_id: i64,
        before: &str,
        source: &str,
        worktree: &str,
    ) -> Result<i64> {
        self.conn.execute("INSERT INTO integration_records(run_id,task_id,attempt_id,before_sha,source_sha,worktree_path,status,details,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'preparing','{}',?7,?7)", params![run_id,task_id,attempt_id,before,source,worktree,now()])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_integration_record(
        &self,
        id: i64,
        status: &str,
        candidate: Option<&str>,
        details: &serde_json::Value,
    ) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if status == "publishing" {
            let attempt: i64 = self.conn.query_row(
                "SELECT attempt_id FROM integration_records WHERE id=?1",
                [id],
                |row| row.get(0),
            )?;
            if self.human_review_blocks(attempt)? {
                return Err(crate::DbError::Busy("Human review requested changes for this attempt; integration remains blocked until explicit resolution".into()));
            }
        }
        self.conn.execute("UPDATE integration_records SET status=?1,candidate_sha=COALESCE(?2,candidate_sha),details=?3,updated_at=?4 WHERE id=?5", params![status,candidate,serde_json::to_string(details)?,now(),id])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn unsettled_integration_runs(&self) -> Result<Vec<i64>> {
        let mut query=self.conn.prepare("SELECT DISTINCT run_id FROM integration_records WHERE status IN ('preparing','publishing') ORDER BY run_id")?;
        let rows = query
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn integration_records(&self, run_id: i64) -> Result<Vec<IntegrationRecord>> {
        let mut query=self.conn.prepare("SELECT id,run_id,task_id,attempt_id,before_sha,source_sha,candidate_sha,worktree_path,status,details,created_at,updated_at FROM integration_records WHERE run_id=?1 ORDER BY id")?;
        let rows = query
            .query_map([run_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get::<_, String>(9)?,
                    row.get(10)?,
                    row.get(11)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    id,
                    run_id,
                    task_id,
                    attempt_id,
                    before_sha,
                    source_sha,
                    candidate_sha,
                    worktree_path,
                    status,
                    details,
                    created_at,
                    updated_at,
                )| {
                    Ok(IntegrationRecord {
                        id,
                        run_id,
                        task_id,
                        attempt_id,
                        before_sha,
                        source_sha,
                        candidate_sha,
                        worktree_path,
                        status,
                        details: serde_json::from_str(&details)?,
                        created_at,
                        updated_at,
                    })
                },
            )
            .collect()
    }

    pub fn reconcile_scheduler_owners(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE task_dispatches SET finished_at=?1,outcome='interrupted; external effects require inspection' WHERE finished_at IS NULL", [now()])?;
        tx.execute("UPDATE scheduler_owners SET finished_at=?1,status='interrupted' WHERE finished_at IS NULL", [now()])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_types::{Plan, PlannedTask, RunStatus};

    fn setup() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        FactoryDb,
        i64,
        Vec<factory_types::Task>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let db = FactoryDb::open(&path).unwrap();
        let run = db.create_run("claim concurrency", None).unwrap();
        let tasks = db
            .persist_plan(
                run.id,
                &Plan {
                    objective: run.objective,
                    tasks: vec![
                        PlannedTask {
                            id: "A".into(),
                            title: "first".into(),
                            objective: "first".into(),
                            dependencies: vec![],
                            acceptance_criteria: vec!["done".into()],
                            role: None,
                            operation: None,
                        },
                        PlannedTask {
                            id: "B".into(),
                            title: "second".into(),
                            objective: "second".into(),
                            dependencies: vec!["A".into()],
                            acceptance_criteria: vec!["done".into()],
                            role: None,
                            operation: None,
                        },
                    ],
                },
            )
            .unwrap();
        db.set_run_status(run.id, RunStatus::Active).unwrap();
        (dir, path, db, run.id, tasks)
    }

    #[test]
    fn durable_owner_and_task_claim_have_one_winner_across_connections() {
        let (_dir, path, db, run, tasks) = setup();
        assert!(db.acquire_scheduler(run, "owner").unwrap());
        assert!(!db.acquire_scheduler(run, "other").unwrap());
        let handles: Vec<_> = (0..12)
            .map(|i| {
                let path = path.clone();
                let task = tasks[0].id;
                std::thread::spawn(move || {
                    FactoryDb::open(&path)
                        .unwrap()
                        .claim_dispatch(run, task, 1, "owner", &format!("claim-{i}"))
                        .unwrap()
                })
            })
            .collect();
        let winners: usize = handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum();
        assert_eq!(winners, 1);
        assert_eq!(db.active_dispatch_count(run).unwrap(), 1);
    }

    #[test]
    fn revision_dependencies_and_owner_gate_dispatch() {
        let (_dir, _path, db, run, tasks) = setup();
        assert!(db.acquire_scheduler(run, "owner").unwrap());
        assert!(!db
            .claim_dispatch(run, tasks[1].id, 1, "owner", "dependency")
            .unwrap());
        assert!(!db
            .claim_dispatch(run, tasks[0].id, 2, "owner", "stale")
            .unwrap());
        assert!(!db
            .claim_dispatch(run, tasks[0].id, 1, "foreign", "wrong-owner")
            .unwrap());
        assert!(db
            .claim_dispatch(run, tasks[0].id, 1, "owner", "first")
            .unwrap());
        assert!(!db
            .claim_dispatch(run, tasks[0].id, 1, "owner", "duplicate")
            .unwrap());
        db.finish_dispatch("first", "done").unwrap();
        assert_eq!(db.active_dispatch_count(run).unwrap(), 0);
    }

    #[test]
    fn restart_releases_claims_without_automatically_replaying_work() {
        let (_dir, _path, db, run, tasks) = setup();
        db.acquire_scheduler(run, "before-crash").unwrap();
        db.claim_dispatch(run, tasks[0].id, 1, "before-crash", "interrupted")
            .unwrap();
        db.reconcile_scheduler_owners().unwrap();
        assert!(!db.scheduler_running(run).unwrap());
        assert_eq!(db.active_dispatch_count(run).unwrap(), 0);
        assert_eq!(
            db.get_task(tasks[0].id).unwrap().unwrap().state,
            factory_types::TaskState::Running
        );
        assert!(db.acquire_scheduler(run, "after-crash").unwrap());
        assert!(!db
            .claim_dispatch(run, tasks[0].id, 1, "after-crash", "no-blind-replay")
            .unwrap());
    }
    #[test]
    fn replan_compare_and_swap_preserves_completed_history_and_attempts() {
        let (_dir, _path, db, run, tasks) = setup();
        db.set_task_state(tasks[0].id, factory_types::TaskState::Completed)
            .unwrap();
        db.set_task_state(tasks[1].id, factory_types::TaskState::Failed)
            .unwrap();
        let before = db.get_task(tasks[0].id).unwrap().unwrap();
        db.create_task_attempt(
            tasks[1].id,
            "worker",
            Some(factory_types::TaskOperation::Implement),
            "worker",
            "retained",
            Some("original-base"),
        )
        .unwrap();
        db.set_run_status(run, RunStatus::Active).unwrap();
        db.acquire_scheduler(run, "owner").unwrap();
        let session = db
            .insert_agent_session(&factory_types::AgentSession {
                id: 0,
                run_id: Some(run),
                task_id: None,
                attempt_id: None,
                role: "planner".into(),
                operation: Some(factory_types::TaskOperation::Planning),
                agent: "planner".into(),
                mode: factory_types::AgentSessionMode::Automated,
                command: "controlled".into(),
                status: "success".into(),
                started_at: now(),
                finished_at: Some(now()),
                exit_code: Some(0),
                duration_ms: Some(1),
                stdout: None,
                stderr: None,
                policy_audit: None,
                output_meta: None,
            })
            .unwrap();
        let plan = factory_types::Plan {
            objective: "original objective is preserved".into(),
            tasks: vec![factory_types::PlannedTask {
                id: "repair".into(),
                title: "Repair".into(),
                objective: "repair remaining work".into(),
                dependencies: vec![tasks[0].id.to_string()],
                acceptance_criteria: vec!["pass".into()],
                role: None,
                operation: None,
            }],
        };
        assert!(matches!(
            db.apply_scheduler_replan(run, "owner", 1, &plan, "real observed obstacle", session.id)
                .unwrap(),
            factory_types::PlanApplyOutcome::Applied { revision: 2, .. }
        ));
        assert_eq!(db.get_task(tasks[0].id).unwrap().unwrap(), before);
        assert_eq!(
            db.get_task(tasks[1].id).unwrap().unwrap().state,
            factory_types::TaskState::Superseded
        );
        assert_eq!(
            db.list_task_attempts_for_task(tasks[1].id).unwrap().len(),
            1
        );
        assert_eq!(db.list_plan_revisions(run).unwrap().len(), 2);
        assert!(matches!(
            db.apply_scheduler_replan(run, "owner", 1, &plan, "stale", session.id),
            Err(DbError::Conflict { .. })
        ));
        assert_eq!(db.list_plan_revisions(run).unwrap().len(), 2);
    }

    #[test]
    fn active_barrier_cycles_and_foreign_owners_cannot_change_the_plan() {
        let (_dir, _path, db, run, tasks) = setup();
        db.acquire_scheduler(run, "owner").unwrap();
        db.claim_dispatch(run, tasks[0].id, 1, "owner", "claim")
            .unwrap();
        let plan = factory_types::Plan {
            objective: "cycle".into(),
            tasks: vec![factory_types::PlannedTask {
                id: "self".into(),
                title: "Cycle".into(),
                objective: "cycle".into(),
                dependencies: vec!["self".into()],
                acceptance_criteria: vec!["bad".into()],
                role: None,
                operation: None,
            }],
        };
        assert!(matches!(
            db.apply_scheduler_replan(run, "owner", 1, &plan, "active", 0),
            Err(DbError::Busy(_))
        ));
        db.finish_dispatch("claim", "failed").unwrap();
        db.set_task_state(tasks[0].id, factory_types::TaskState::Failed)
            .unwrap();
        assert!(matches!(
            db.apply_scheduler_replan(run, "foreign", 1, &plan, "foreign", 0),
            Err(DbError::Busy(_))
        ));
        assert!(matches!(
            db.apply_scheduler_replan(run, "owner", 1, &plan, "invalid DAG", 0)
                .unwrap(),
            factory_types::PlanApplyOutcome::Invalid(_)
        ));
        assert_eq!(db.get_plan_revision(run).unwrap(), 1);
        db.request_scheduler_stop(run).unwrap();
        assert!(!db
            .claim_dispatch(run, tasks[0].id, 1, "owner", "after-cancel")
            .unwrap());
    }
}
