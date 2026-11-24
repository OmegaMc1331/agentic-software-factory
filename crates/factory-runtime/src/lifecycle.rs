use super::*;
use factory_types::{MissionLifecycle, MissionLifecycleState, RunStatus};

fn database(runtime: &Runtime) -> Result<FactoryDb, RuntimeError> {
    FactoryDb::open(&runtime.root.join(".factory/db.sqlite3"))
        .map_err(FactoryError::from)
        .map_err(Into::into)
}

pub(super) fn tracing_lifecycle_failure(error: &RuntimeError) {
    eprintln!("Mission lifecycle reconciliation requires inspection: {error}");
}

impl Runtime {
    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        self.budget.close();
        let mut runs: Vec<_> = self
            .active_operations()
            .iter()
            .map(|operation| operation.run_id)
            .collect();
        let mut failure = None;
        match database(self).and_then(|db| {
            db.shutdown_mission_candidates(false)
                .map_err(|error| FactoryError::from(error).into())
        }) {
            Ok(pending) => runs.extend(pending),
            Err(error) => failure = Some(error),
        }
        runs.sort_unstable();
        runs.dedup();
        for run in runs {
            if let Err(error) = self.suspend_for_shutdown(run) {
                failure = Some(error);
            }
        }
        let sessions: Vec<_> = self
            .terminals
            .lock()
            .expect("terminal mutex poisoned")
            .keys()
            .copied()
            .collect();
        for session in sessions {
            if let Err(error) = self.stop_interactive_session(session) {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub(super) fn suspend_for_shutdown(&self, run: i64) -> Result<(), RuntimeError> {
        let result = (|| {
            if let Some(life) = database(self)?
                .mission_lifecycle(run)
                .map_err(FactoryError::from)?
            {
                self.suspend_mission(
                    run,
                    life.generation,
                    &format!("shutdown-{}", life.generation),
                    "application_shutdown",
                )?;
            } else {
                match self.factory()?.request_cancellation(run) {
                    Ok(()) => {}
                    Err(FactoryError::InvalidRunState(id, status))
                        if id == run && status == RunStatus::Cancelled.as_str() => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        })();
        if let Some(active) = self
            .active
            .lock()
            .expect("runtime mutex poisoned")
            .get(&run)
        {
            active.cancel.store(true, Ordering::Release);
        }
        result
    }

    pub fn finish_shutdown_checkpoints(&self) -> Result<(), RuntimeError> {
        if !self.is_idle() || self.budget.checkpoints_deferred() {
            return Err(
                FactoryError::PolicyBlocked("Shutdown operations have not drained".into()).into(),
            );
        }
        let db = database(self)?;
        let mut runs = db
            .shutdown_mission_candidates(false)
            .map_err(FactoryError::from)?;
        runs.extend(
            db.shutdown_mission_candidates(true)
                .map_err(FactoryError::from)?,
        );
        runs.sort_unstable();
        runs.dedup();
        for run in runs {
            self.finish_lifecycle_operation(run)?;
            let life = db
                .mission_lifecycle(run)
                .map_err(FactoryError::from)?
                .ok_or(FactoryError::RunNotFound(run))?;
            if !matches!(
                life.state,
                MissionLifecycleState::Suspended | MissionLifecycleState::Settled
            ) {
                return Err(FactoryError::PolicyBlocked(
                    "Shutdown checkpoint requires recovery; inspect retained evidence".into(),
                )
                .into());
            }
        }
        Ok(())
    }

    pub fn suspend_mission(
        &self,
        run: i64,
        generation: i64,
        key: &str,
        reason: &str,
    ) -> Result<MissionLifecycle, RuntimeError> {
        let db = database(self)?;
        let life = match db.request_mission_suspend(run, generation, key, reason) {
            Ok(life) => life,
            Err(error) => {
                if matches!(
                    error,
                    factory_db::DbError::Sqlite(_) | factory_db::DbError::Io(_)
                ) {
                    if let Some(active) = self
                        .active
                        .lock()
                        .expect("runtime mutex poisoned")
                        .get(&run)
                    {
                        active.cancel.store(true, Ordering::Release);
                    }
                }
                return Err(FactoryError::from(error).into());
            }
        };
        if !matches!(
            life.state,
            MissionLifecycleState::SuspendRequested | MissionLifecycleState::Quiescing
        ) {
            return Ok(life);
        }
        let active = {
            let active = self.active.lock().expect("runtime mutex poisoned");
            let current = db
                .mission_lifecycle(run)
                .map_err(FactoryError::from)?
                .ok_or(FactoryError::RunNotFound(run))?;
            if current.generation != life.generation
                || !matches!(
                    current.state,
                    MissionLifecycleState::SuspendRequested | MissionLifecycleState::Quiescing
                )
            {
                return Ok(current);
            }
            let cancel = active.get(&run).map(|a| a.cancel.clone());
            if let Some(cancel) = &cancel {
                cancel.store(true, Ordering::Release);
            }
            cancel
        };
        if life.state == MissionLifecycleState::SuspendRequested {
            db.lifecycle_transition(
                run,
                life.generation,
                life.state,
                MissionLifecycleState::Quiescing,
                None,
            )
            .map_err(FactoryError::from)?;
        }
        if active.is_none() {
            self.finish_lifecycle_operation(run)?;
        }
        database(self)?
            .mission_lifecycle(run)
            .map_err(FactoryError::from)?
            .ok_or_else(|| FactoryError::RunNotFound(run).into())
    }

    pub(super) fn finish_lifecycle_operation(&self, run: i64) -> Result<(), RuntimeError> {
        if self.budget.checkpoints_deferred() {
            return Ok(());
        }
        let db = database(self)?;
        let Some(life) = db.mission_lifecycle(run).map_err(FactoryError::from)? else {
            return Ok(());
        };
        if life.state == MissionLifecycleState::Active {
            db.settle_mission_lifecycle(run)
                .map_err(FactoryError::from)?;
            return Ok(());
        }
        if !matches!(
            life.state,
            MissionLifecycleState::Quiescing | MissionLifecycleState::SuspendRequested
        ) {
            return Ok(());
        }
        if self
            .active
            .lock()
            .expect("runtime mutex poisoned")
            .contains_key(&run)
        {
            return Ok(());
        }
        if self
            .delivery_operations
            .lock()
            .expect("delivery mutex poisoned")
            .contains_key(&run)
        {
            return Err(FactoryError::PolicyBlocked(
                "Delivery operation still active; suspension requires reconciliation".into(),
            )
            .into());
        }
        if life.state == MissionLifecycleState::SuspendRequested {
            db.lifecycle_transition(
                run,
                life.generation,
                life.state,
                MissionLifecycleState::Quiescing,
                None,
            )
            .map_err(FactoryError::from)?;
        }
        if life.reason.as_deref() == Some("user_cancelled") {
            if db
                .lifecycle_processes(run)
                .map_err(FactoryError::from)?
                .iter()
                .any(|(_, _, meta)| {
                    meta.as_ref()
                        .is_some_and(|m| !m.process_tree_cleanup_confirmed)
                })
            {
                db.lifecycle_transition(
                    run,
                    life.generation,
                    MissionLifecycleState::Quiescing,
                    MissionLifecycleState::RecoveryRequired,
                    Some("Cancellation cleanup is uncertain"),
                )
                .map_err(FactoryError::from)?;
                return Ok(());
            }
            db.set_run_status(run, factory_types::RunStatus::Cancelled)
                .map_err(FactoryError::from)?;
            db.settle_cancelled_lifecycle(run)
                .map_err(FactoryError::from)?;
            return Ok(());
        }
        let status = db
            .get_run(run)
            .map_err(FactoryError::from)?
            .ok_or(FactoryError::RunNotFound(run))?
            .status;
        let tasks = db.list_tasks(run).map_err(FactoryError::from)?;
        if status == factory_types::RunStatus::Completed
            || !tasks.is_empty()
                && tasks.iter().all(|t| {
                    matches!(
                        t.state,
                        factory_types::TaskState::Completed | factory_types::TaskState::Superseded
                    )
                })
        {
            db.set_run_status(run, factory_types::RunStatus::Completed)
                .map_err(FactoryError::from)?;
            db.complete_lifecycle_race(run, life.generation)
                .map_err(FactoryError::from)?;
            return Ok(());
        }
        match self.factory()?.checkpoint_mission(run, life.generation) {
            Ok(_) => db
                .lifecycle_transition(
                    run,
                    life.generation,
                    MissionLifecycleState::Quiescing,
                    MissionLifecycleState::Suspended,
                    None,
                )
                .map_err(FactoryError::from)?,
            Err(error) => {
                db.lifecycle_transition(
                    run,
                    life.generation,
                    MissionLifecycleState::Quiescing,
                    MissionLifecycleState::RecoveryRequired,
                    Some(&error.to_string()),
                )
                .map_err(FactoryError::from)?;
            }
        }
        Ok(())
    }

    pub fn resume_mission(
        &self,
        run: i64,
        expected: i64,
        key: &str,
    ) -> Result<MissionLifecycle, RuntimeError> {
        let db = database(self)?;
        let factory = self.factory()?;
        let life = db
            .mission_lifecycle(run)
            .map_err(FactoryError::from)?
            .ok_or(FactoryError::RunNotFound(run))?;
        if life.state == MissionLifecycleState::RecoveryRequired
            && life.reason.as_deref() == Some("runtime_interruption")
            && life.generation == expected
        {
            factory.reconcile_crashed_mission(run)?;
            db.lifecycle_transition(
                run,
                expected,
                MissionLifecycleState::RecoveryRequired,
                MissionLifecycleState::Quiescing,
                None,
            )
            .map_err(FactoryError::from)?;
            self.finish_lifecycle_operation(run)?;
        }
        if !db
            .begin_mission_resume(run, expected, key)
            .map_err(FactoryError::from)?
        {
            return db
                .mission_lifecycle(run)
                .map_err(FactoryError::from)?
                .ok_or_else(|| FactoryError::RunNotFound(run).into());
        }
        let generation = expected + 1;
        let outcome = (|| {
            factory.validate_mission_resume(run)?;
            factory.prepare_resume_handoffs(run)?;
            let permit = self.budget.operation()?;
            let mut active = self.active.lock().expect("runtime mutex poisoned");
            if self.budget.is_closed() {
                return Err(RuntimeError::ResourceLimit("application shutting down"));
            }
            if active.contains_key(&run) {
                return Err(RuntimeError::AlreadyActive(run));
            }
            db.commit_mission_resume(run, generation)
                .map_err(FactoryError::from)?;
            let cancel = Arc::new(AtomicBool::new(false));
            active.insert(
                run,
                Active {
                    _permit: permit,
                    kind: OperationKind::Executing,
                    cancel: cancel.clone(),
                },
            );
            drop(active);
            let runtime = self.clone();
            tokio::task::spawn_blocking(move || {
                let _ = runtime.guard_execution(run, || {
                    let factory = runtime.factory()?;
                    if factory
                        .get_run(run)?
                        .is_some_and(|r| r.status == factory_types::RunStatus::Planning)
                    {
                        if let Some(plan) = database(&runtime)?
                            .resume_authored_plan(run)
                            .map_err(FactoryError::from)?
                        {
                            factory.persist_authored_plan(run, &plan)?;
                        } else {
                            factory.plan_run(run, &cancel)?;
                        }
                        factory.prepare_start(run)?;
                    }
                    factory.execute_active_run(run, &cancel)?;
                    Ok(())
                });
                runtime.release(run);
            });
            Ok::<_, RuntimeError>(())
        })();
        if let Err(error) = outcome {
            db.lifecycle_transition(
                run,
                generation,
                MissionLifecycleState::Reconciling,
                MissionLifecycleState::RecoveryRequired,
                Some(&error.to_string()),
            )
            .map_err(FactoryError::from)?;
            return Err(error);
        }
        db.mission_lifecycle(run)
            .map_err(FactoryError::from)?
            .ok_or_else(|| FactoryError::RunNotFound(run).into())
    }
}
