use super::*;
use factory_types::{CheckpointWorktree, MissionCheckpoint};
use serde_json::json;
use sha2::{Digest, Sha256};

impl Factory {
    pub(crate) fn require_lifecycle_active(&self, run: i64) -> Result<(), FactoryError> {
        if self.db.lifecycle_fenced(run)? {
            Err(FactoryError::Suspended)
        } else {
            Ok(())
        }
    }

    pub fn checkpoint_mission(
        &self,
        run: i64,
        generation: i64,
    ) -> Result<MissionCheckpoint, FactoryError> {
        self.execution.validate_root()?;
        let life = self
            .db
            .mission_lifecycle(run)?
            .ok_or(FactoryError::RunNotFound(run))?;
        if life.project_id != self.execution.project_id() || life.generation != generation {
            return Err(FactoryError::PolicyBlocked(
                "Checkpoint project or generation mismatch".into(),
            ));
        }
        for (_, status, metadata) in self.db.lifecycle_processes(run)? {
            if let Some(meta) = metadata.filter(|m| m.process_id.is_some()) {
                if !meta.process_tree_supervised || !meta.process_tree_cleanup_confirmed {
                    return Err(FactoryError::PolicyBlocked(
                        "Owned process-tree cleanup is not confirmed; recovery required".into(),
                    ));
                }
            } else if status == "interrupted" {
                return Err(FactoryError::PolicyBlocked(
                    "Interrupted process ownership requires inspection".into(),
                ));
            }
        }
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        let mut worktrees = Vec::new();
        for task in self.db.list_tasks(run)? {
            if matches!(task.state, TaskState::Completed | TaskState::Superseded) {
                continue;
            }
            if let Some(path) = &task.worktree_path {
                let path = Path::new(path);
                let base = self
                    .db
                    .latest_task_attempt(task.id)?
                    .and_then(|a| a.source_base);
                worktrees.push(CheckpointWorktree {
                    task_id: task.id,
                    path: path.to_string_lossy().into_owned(),
                    head_commit: repo.head_sha(path)?,
                    base_commit: base,
                    status_sha256: format!(
                        "{:x}",
                        Sha256::digest(repo.checkpoint_material(path)?.as_bytes())
                    ),
                });
            }
        }
        let integration_head = if repo.branch_exists(&self.run_branch(run))? {
            repo.resolve_ref(&self.run_branch(run))?
        } else {
            repo.head_sha(repo.root())?
        };
        Ok(self
            .db
            .save_mission_checkpoint(run, generation, worktrees, Some(integration_head))?)
    }

    pub fn validate_mission_resume(&self, run: i64) -> Result<(), FactoryError> {
        self.execution.validate_root()?;
        let life = self
            .db
            .mission_lifecycle(run)?
            .ok_or(FactoryError::RunNotFound(run))?;
        let checkpoint = self
            .db
            .mission_checkpoints(run, None)?
            .into_iter()
            .find(|c| Some(c.id) == life.checkpoint_id)
            .ok_or_else(|| {
                FactoryError::PolicyBlocked(
                    "No valid durable checkpoint; inspect interrupted operations before recovery"
                        .into(),
                )
            })?;
        if checkpoint.schema_version != 1
            || checkpoint.project_id != self.execution.project_id()
            || checkpoint.run_id != run
            || checkpoint.workspace_session_id != life.workspace_session_id
            || checkpoint.plan_revision != self.db.get_plan_revision(run)?
        {
            return Err(FactoryError::PolicyBlocked(
                "Checkpoint identity, schema or plan revision mismatch".into(),
            ));
        }
        if self.db.scheduler_running(run)? || self.db.active_dispatch_count(run)? > 0 {
            return Err(FactoryError::CapacityBusy(
                "Previous execution ownership is not drained".into(),
            ));
        }
        if self.db.get_delivery(run)?.is_some_and(|d| {
            d.recovery_required
                || matches!(
                    d.state,
                    DeliveryState::Pushing | DeliveryState::CreatingPr | DeliveryState::Interrupted
                )
        }) {
            return Err(FactoryError::PolicyBlocked(
                "External delivery outcome requires reconciliation before resume".into(),
            ));
        }
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        if let Some(sha) = &checkpoint.integration_commit {
            let current = if repo.branch_exists(&self.run_branch(run))? {
                repo.resolve_ref(&self.run_branch(run))?
            } else {
                repo.head_sha(repo.root())?
            };
            if current != *sha {
                return Err(FactoryError::PolicyBlocked(
                    "Integration branch changed after checkpoint".into(),
                ));
            }
        }
        for worktree in &checkpoint.worktrees {
            let valid_commit = |sha: &str| {
                matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            };
            if !valid_commit(&worktree.head_commit)
                || worktree
                    .base_commit
                    .as_deref()
                    .is_some_and(|base| !valid_commit(base))
            {
                return Err(FactoryError::PolicyBlocked(
                    "Checkpoint commit identifier is invalid".into(),
                ));
            }
            let task = self
                .db
                .get_task(worktree.task_id)?
                .ok_or(FactoryError::TaskNotFound(worktree.task_id))?;
            if task.run_id != run
                || task.worktree_path.as_deref() != Some(&worktree.path)
                || repo.head_sha(Path::new(&worktree.path))? != worktree.head_commit
                || format!(
                    "{:x}",
                    Sha256::digest(
                        repo.checkpoint_material(Path::new(&worktree.path))?
                            .as_bytes()
                    )
                ) != worktree.status_sha256
            {
                return Err(FactoryError::PolicyBlocked(
                    "Retained task worktree changed or is unavailable; inspect before resume"
                        .into(),
                ));
            }
            if let Some(base) = &worktree.base_commit {
                if !repo.is_ancestor(base, &worktree.head_commit)? {
                    return Err(FactoryError::PolicyBlocked(
                        "Recorded source base is not an ancestor of retained work".into(),
                    ));
                }
            }
        }
        if self.db.integration_records(run)?.iter().any(|r| {
            matches!(
                r.status.as_str(),
                "preparing" | "publishing" | "interrupted"
            )
        }) {
            return Err(FactoryError::PolicyBlocked(
                "Integration journal requires reconciliation; no replay authorized".into(),
            ));
        }
        let run_record = self
            .db
            .get_run(run)?
            .ok_or(FactoryError::RunNotFound(run))?;
        let team = self.resolve_team(&run_record)?;
        self.agents
            .config()
            .validate_team(&team)
            .map_err(FactoryError::InvalidTeam)?;
        let tasks = self.db.list_tasks(run)?;
        let remaining = tasks
            .into_iter()
            .filter(|t| !matches!(t.state, TaskState::Completed | TaskState::Superseded))
            .collect::<Vec<_>>();
        self.validate_run_policies_run(
            run,
            &remaining,
            &team,
            &self.agents.config().catalog(),
            run_record.recipe.as_ref(),
        )?;
        for task in &remaining {
            let role = task.role.as_deref().unwrap_or(roles::WORKER);
            for agent in team
                .agents_for_role(role)
                .iter()
                .filter(|a| task.agent_override.as_ref().is_none_or(|pin| pin == *a))
            {
                self.agents.command_agent_for(role, agent)?;
            }
            if let Some(h) = self
                .db
                .active_handoff(task.id)?
                .filter(|h| h.state == "waiting")
            {
                let current = self.db.agent_profile(&h.target.id)?;
                if current.revision != h.target.revision
                    || current.archived
                    || !current.configuration.enabled
                {
                    return Err(FactoryError::PolicyBlocked(
                        "Pending handoff target changed or became unavailable".into(),
                    ));
                }
                self.agents.command_agent_for(role, &current.id)?;
                let policy = self.policy_with_recipe(run_record.recipe.as_ref(), role, &current.id);
                let operation =
                    roles::resolve_task_operation(task, &self.agents.config().catalog());
                factory_policy::validate_executable(&policy, operation.as_str())
                    .map_err(FactoryError::PolicyBlocked)?;
            }
        }
        Ok(())
    }

    pub fn prepare_resume_handoffs(&self, run: i64) -> Result<(), FactoryError> {
        for h in self
            .db
            .active_handoffs_for_run(run)?
            .into_iter()
            .filter(|h| h.state == "waiting")
        {
            let attempt = self
                .db
                .latest_task_attempt(h.task_id)?
                .ok_or(FactoryError::TaskNotFound(h.task_id))?;
            let evidence = attempt.evidence.as_ref().ok_or_else(|| {
                FactoryError::PolicyBlocked("Pending handoff has no preserved evidence".into())
            })?;
            self.db
                .handoff_resume_boundary(h.task_id, attempt.id, evidence)?;
        }
        Ok(())
    }

    pub(super) fn lifecycle_resume_context(
        &self,
        run: i64,
        task: Option<i64>,
    ) -> Result<Option<String>, FactoryError> {
        let Some(life) = self
            .db
            .mission_lifecycle(run)?
            .filter(|l| l.checkpoint_id.is_some())
        else {
            return Ok(None);
        };
        let previous = task
            .map(|id| self.db.list_task_attempts_for_task(id))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.status != AttemptStatus::Running)
            .max_by_key(|a| a.id);
        Ok(Some(format!("\nASF_LOGICAL_RESUME\n{}\nThe previous invocation has been reconciled after interruption; inspect the recorded lifecycle reason and process evidence. Existing partial changes are unapproved. Inspect the current files and recorded tests; preserve valid work, complete the objective and rerun authorized validation. This is a new invocation, not preserved inference state. Do not repeat publication or other external side effects.\n",json!({"checkpointId":life.checkpoint_id,"generation":life.generation,"previousAttempt":previous.as_ref().map(|a|json!({"id":a.id,"status":a.status,"base":a.source_base,"commit":a.commit_sha}))}))))
    }

    pub fn recover_lifecycle_after_restart(&self) -> Result<(), FactoryError> {
        self.db.mark_lifecycle_runtime_interruption()?;
        Ok(())
    }

    pub fn reconcile_crashed_mission(&self, run: i64) -> Result<(), FactoryError> {
        self.execution.validate_root()?;
        for (session, _, metadata) in self.db.lifecycle_processes(run)? {
            if let Some(mut meta) = metadata {
                if !meta.process_tree_cleanup_confirmed {
                    if !meta.process_tree_supervised
                        || !meta
                            .process_id
                            .is_some_and(factory_proc::owned_process_absent)
                    {
                        return Err(FactoryError::PolicyBlocked("An interrupted owned process may still be alive; no automatic termination or replay".into()));
                    }
                    #[cfg(unix)]
                    if !meta.process_tree_cleanup_confirmed {
                        return Err(FactoryError::PolicyBlocked(
                            "POSIX crash process group requires verified cleanup before recovery"
                                .into(),
                        ));
                    }
                    meta.process_tree_cleanup_confirmed = true;
                    meta.process_cleanup_observed_unix_ms =
                        Some(chrono::Utc::now().timestamp_millis().max(0) as u64);
                    self.db
                        .set_agent_session_output_metadata(session, Some(&meta))?;
                }
            }
        }
        Ok(())
    }
}
