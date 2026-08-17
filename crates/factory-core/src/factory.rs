use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use factory_agent::{AgentError, AgentRequest, AgentResult, CommandAgent, OutputStream};
use factory_db::{FactoryDb, Reconciliation};
use factory_git::{Repo, WorktreeInfo};
use factory_types::{
    AgentSession, AgentSessionMode, AttemptStatus, ReviewDecision, ReviewResult, RoleArtifact, Run,
    RunStatus, SpecializedReview, Task, TaskAttempt, TaskEvidence, TaskOperation, TaskState,
    WorkflowTeam,
};
use thiserror::Error;

use crate::config::{AgentResolutionError, Agents, ConfigError};
use crate::mission::{
    build_mission, parse_advisory_report, parse_producer_report, parse_review,
    parse_specialized_review, review_result_from, MissionContext, ReviewInput,
    MAX_REVIEW_DIFF_CHARS,
};
use crate::planner::{
    mission as planner_mission, normalize_plan, normalize_plan_with_operations, parse_plan,
    validate_plan_roles, PlanError,
};
use crate::roles::{self, RoleCatalog, RoleDefinition};

pub const FACTORY_DIR: &str = ".factory";
pub const MAX_TASK_ATTEMPTS: u32 = 3;

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("factory not initialized here; run `factory init` first")]
    NotInitialized,
    #[error("run {0} not found")]
    RunNotFound(i64),
    #[error("task {0} not found")]
    TaskNotFound(i64),
    #[error("invalid state transition: {0} -> {1}")]
    InvalidTransition(TaskState, TaskState),
    #[error("workflow #{0} cannot be started while it is {1}")]
    InvalidRunState(i64, String),
    #[error("workflow #{0} has no planned tasks")]
    EmptyPlan(i64),
    #[error("workflow #{0} has an invalid task dependency graph: {1}")]
    InvalidDag(i64, String),
    #[error("task #{0} requires role '{1}' which has no agents in this workflow's team")]
    TaskRoleUnavailable(i64, String),
    #[error("invalid workflow team: {0}")]
    InvalidTeam(String),
    #[error("task #{0} reached the retry limit of {MAX_TASK_ATTEMPTS} attempts: {1}")]
    RetryLimit(i64, String),
    #[error("planning failed: {0}")]
    Plan(#[from] PlanError),
    #[error("agent process failed: {0}")]
    AgentProcess(#[from] AgentError),
    #[error("agent resolution: {0}")]
    Agent(#[from] AgentResolutionError),
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("database error: {0}")]
    Db(#[from] factory_db::DbError),
    #[error("git error: {0}")]
    Git(#[from] factory_git::GitError),
    #[error("task {0} is not ready to run")]
    NotReady(i64),
    #[error("objective must not be empty")]
    EmptyObjective,
    #[error("workflow operation was cancelled")]
    Cancelled,
    #[error("io error: {0}")]
    Io(std::io::Error),
}

impl FactoryError {
    pub fn is_agent_configuration(&self) -> bool {
        matches!(self, FactoryError::AgentProcess(error) if error.is_configuration())
            || matches!(self, FactoryError::Agent(_))
    }
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub run: Run,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone)]
pub struct MarkOutcome {
    pub task: Task,
    pub from: TaskState,
    pub updated: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowResult {
    Completed,
    Failed,
    Cancelled,
}

struct Invocation {
    session_id: i64,
    result: AgentResult,
}

struct InvocationScope<'a> {
    run_id: Option<i64>,
    task_id: Option<i64>,
    attempt_id: Option<i64>,
    role: &'a str,
    operation: Option<TaskOperation>,
    working_dir: &'a Path,
}

pub struct Factory {
    db: FactoryDb,
    agents: Agents,
    root: std::path::PathBuf,
}

impl Factory {
    pub fn init(root: &Path) -> Result<Factory, FactoryError> {
        let factory_dir = root.join(FACTORY_DIR);
        std::fs::create_dir_all(&factory_dir).map_err(FactoryError::Io)?;
        crate::config::Config::ensure_default(root)?;
        let db = FactoryDb::open(&factory_dir.join("db.sqlite3"))?;
        Ok(Factory {
            db,
            agents: Agents::load(root)?,
            root: root.to_path_buf(),
        })
    }

    pub fn open(root: &Path) -> Result<Factory, FactoryError> {
        let db_path = root.join(FACTORY_DIR).join("db.sqlite3");
        if !db_path.exists() {
            return Err(FactoryError::NotInitialized);
        }
        Ok(Factory {
            db: FactoryDb::open(&db_path)?,
            agents: Agents::load(root)?,
            root: root.to_path_buf(),
        })
    }

    pub fn agents(&self) -> &Agents {
        &self.agents
    }

    pub fn begin_run(
        &self,
        objective: &str,
        team: Option<WorkflowTeam>,
    ) -> Result<Run, FactoryError> {
        let objective = objective.trim();
        if objective.is_empty() {
            return Err(FactoryError::EmptyObjective);
        }
        let team = self.resolve_new_team(team)?;
        let planner = self.agents.command_agent_for("planner", &team.planner)?;
        let run =
            self.db
                .create_run_with_status(objective, Some(planner.name()), RunStatus::Planning)?;
        self.db.set_run_team(run.id, &team)?;
        self.db
            .get_run(run.id)?
            .ok_or(FactoryError::RunNotFound(run.id))
    }

    fn resolve_new_team(&self, team: Option<WorkflowTeam>) -> Result<WorkflowTeam, FactoryError> {
        let (team, complete) = match team {
            Some(team) => (team, true),
            None => (
                self.agents
                    .config()
                    .initial_team()
                    .map_err(FactoryError::InvalidTeam)?,
                false,
            ),
        };
        let validation = if complete {
            self.agents.config().validate_team(&team)
        } else {
            self.agents.config().validate_partial_team(&team)
        };
        validation.map_err(FactoryError::InvalidTeam)?;
        Ok(team)
    }

    /// The team recorded on the run, or the current default team persisted as
    /// a snapshot for runs created before teams existed.
    fn resolve_team(&self, run: &Run) -> Result<WorkflowTeam, FactoryError> {
        if let Some(team) = &run.team {
            return Ok(team.clone());
        }
        let team = self
            .agents
            .config()
            .default_team()
            .map_err(FactoryError::InvalidTeam)?;
        self.db.set_run_team(run.id, &team)?;
        Ok(team)
    }

    pub fn create_run(&self, objective: &str) -> Result<RunOutcome, FactoryError> {
        let run = self.begin_run(objective, None)?;
        self.plan_run(run.id, &AtomicBool::new(false))
    }

    pub fn plan_run(&self, run_id: i64, cancel: &AtomicBool) -> Result<RunOutcome, FactoryError> {
        let result = self.plan_run_inner(run_id, cancel);
        if result.is_err() {
            let status = if cancel.load(Ordering::Relaxed) {
                RunStatus::Cancelled
            } else {
                RunStatus::Failed
            };
            let _ = self.db.set_run_status(run_id, status);
        }
        result
    }

    fn plan_run_inner(&self, run_id: i64, cancel: &AtomicBool) -> Result<RunOutcome, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        if run.status != RunStatus::Planning {
            return Err(FactoryError::InvalidRunState(
                run_id,
                run.status.as_str().to_string(),
            ));
        }
        let team = self.resolve_team(&run)?;
        let planner = self.agents.command_agent_for("planner", &team.planner)?;
        let catalog = self.agents.config().catalog();
        let available_roles = team.task_roles();
        let allowed_roles: std::collections::HashSet<String> =
            available_roles.iter().cloned().collect();
        let planner_roles = crate::planner::planners_catalog(&catalog, &available_roles);
        let mut rejection: Option<String> = None;
        for attempt in 0..crate::planner::MAX_ATTEMPTS {
            let instruction = planner_mission(&run.objective, &planner_roles, rejection.as_deref());
            let invocation = self.invoke_with_agent(
                planner.clone(),
                InvocationScope {
                    run_id: Some(run_id),
                    task_id: None,
                    attempt_id: None,
                    role: "planner",
                    operation: Some(TaskOperation::Planning),
                    working_dir: &self.root,
                },
                &instruction,
                cancel,
            )?;
            if invocation.result.cancelled {
                return Err(FactoryError::Cancelled);
            }
            if invocation.result.exit_code != Some(0) {
                return Err(PlanError::Agent(AgentError::Spawn(
                    invocation.result.exit_code.map_or_else(
                        || "planner".to_string(),
                        |code| format!("planner exited with code {code}"),
                    ),
                    invocation.result.stderr,
                ))
                .into());
            }
            let parsed = parse_plan(&invocation.result.stdout).and_then(|plan| {
                let plan = normalize_plan(plan);
                validate_plan_roles(&plan, &allowed_roles)?;
                // Reject role/operation mismatches and fill derived operations.
                normalize_plan_with_operations(plan, &catalog)
            });
            match parsed {
                Ok(plan) => {
                    let tasks = self.db.persist_plan(run_id, &plan)?;
                    let run = self
                        .db
                        .get_run(run_id)?
                        .ok_or(FactoryError::RunNotFound(run_id))?;
                    return Ok(RunOutcome { run, tasks });
                }
                Err(reason) => {
                    self.db
                        .set_agent_session_status(invocation.session_id, "rejected")?;
                    if attempt + 1 >= crate::planner::MAX_ATTEMPTS {
                        return Err(PlanError::Invalid(reason).into());
                    }
                    rejection = Some(reason);
                }
            }
        }
        unreachable!("planner attempt loop returns")
    }

    pub fn prepare_start(&self, run_id: i64) -> Result<WorkflowTeam, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        if run.status != RunStatus::Planned {
            return Err(FactoryError::InvalidRunState(
                run_id,
                run.status.as_str().to_string(),
            ));
        }
        let tasks = self.db.list_tasks(run_id)?;
        if tasks.is_empty() {
            return Err(FactoryError::EmptyPlan(run_id));
        }
        validate_task_dag(&tasks).map_err(|reason| FactoryError::InvalidDag(run_id, reason))?;
        let mut team = self.resolve_team(&run)?;
        if team.workers.is_empty() || team.reviewers.is_empty() {
            let defaults = self
                .agents
                .config()
                .default_team()
                .map_err(FactoryError::InvalidTeam)?;
            if team.workers.is_empty() {
                team.workers = defaults.workers;
            }
            if team.reviewers.is_empty() {
                team.reviewers = defaults.reviewers;
            }
            self.db.set_run_team(run_id, &team)?;
        }
        self.agents
            .config()
            .validate_team(&team)
            .map_err(FactoryError::InvalidTeam)?;
        for task in &tasks {
            if let Some(role) = &task.role {
                if team.agents_for_role(role).is_empty() {
                    return Err(FactoryError::TaskRoleUnavailable(task.id, role.clone()));
                }
            }
        }
        for role in team.roles() {
            for agent in team.agents_for_role(&role) {
                self.agents.command_agent_for(&role, agent)?;
            }
        }
        Repo::detect_bounded(&self.root, &self.root)?;
        self.db.set_run_status(run_id, RunStatus::Active)?;
        Ok(team)
    }

    /// Replaces the team of a workflow that has not started yet.
    pub fn update_run_team(
        &self,
        run_id: i64,
        team: WorkflowTeam,
    ) -> Result<WorkflowTeam, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        if !matches!(
            run.status,
            RunStatus::Planning | RunStatus::Planned | RunStatus::Blocked
        ) {
            return Err(FactoryError::InvalidRunState(
                run_id,
                run.status.as_str().to_string(),
            ));
        }
        self.agents
            .config()
            .validate_team(&team)
            .map_err(FactoryError::InvalidTeam)?;
        let planner = self.agents.command_agent_for("planner", &team.planner)?;
        self.db.set_run_team(run_id, &team)?;
        if run.status == RunStatus::Planning {
            self.db.set_run_planner_agent(run_id, planner.name())?;
        }
        Ok(team)
    }

    pub fn execute_active_run(
        &self,
        run_id: i64,
        cancel: &AtomicBool,
    ) -> Result<WorkflowResult, FactoryError> {
        let result = self.execute_active_run_inner(run_id, cancel);
        if result.is_err() {
            let status = if cancel.load(Ordering::Relaxed) {
                RunStatus::Cancelled
            } else {
                RunStatus::Failed
            };
            let _ = self.db.set_run_status(run_id, status);
        }
        result
    }

    fn execute_active_run_inner(
        &self,
        run_id: i64,
        cancel: &AtomicBool,
    ) -> Result<WorkflowResult, FactoryError> {
        loop {
            if cancel.load(Ordering::Relaxed) {
                self.db.set_run_status(run_id, RunStatus::Cancelled)?;
                return Ok(WorkflowResult::Cancelled);
            }
            let tasks = self.db.list_tasks(run_id)?;
            if tasks.iter().all(|task| task.state == TaskState::Completed) {
                self.db.set_run_status(run_id, RunStatus::Completed)?;
                return Ok(WorkflowResult::Completed);
            }
            let next = tasks
                .iter()
                .filter(|task| task.state == TaskState::Ready)
                .min_by_key(|task| (task.position, task.id))
                .cloned();
            let Some(task) = next else {
                let status = if tasks.iter().any(|task| task.state == TaskState::Failed) {
                    RunStatus::Failed
                } else {
                    RunStatus::Blocked
                };
                self.db.set_run_status(run_id, status)?;
                return Ok(WorkflowResult::Failed);
            };
            if !self.execute_task(task.id, cancel)? {
                if cancel.load(Ordering::Relaxed) {
                    self.db.set_run_status(run_id, RunStatus::Cancelled)?;
                    return Ok(WorkflowResult::Cancelled);
                }
                self.db.set_run_status(run_id, RunStatus::Failed)?;
                return Ok(WorkflowResult::Failed);
            }
        }
    }

    pub fn prepare_retry(&self, task_id: i64) -> Result<i64, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        if !matches!(task.state, TaskState::Failed | TaskState::Blocked) {
            return Err(FactoryError::InvalidTransition(
                task.state,
                TaskState::Ready,
            ));
        }
        if self
            .db
            .latest_task_attempt(task_id)?
            .is_some_and(|attempt| attempt.attempt_number >= MAX_TASK_ATTEMPTS)
        {
            return Err(FactoryError::RetryLimit(
                task_id,
                "manual retry after the bounded attempt limit".into(),
            ));
        }
        let run = self
            .db
            .get_run(task.run_id)?
            .ok_or(FactoryError::RunNotFound(task.run_id))?;
        let team = self.resolve_team(&run)?;
        self.agents
            .config()
            .validate_team(&team)
            .map_err(FactoryError::InvalidTeam)?;
        self.mark_task(task_id, TaskState::Ready)?;
        self.db.set_run_status(task.run_id, RunStatus::Active)?;
        Ok(task.run_id)
    }

    /// Executes one task by its semantic operation. Returns `false` when the
    /// workflow must stop (cancellation, retry limit, or a rejection without
    /// anything to rework).
    fn execute_task(&self, task_id: i64, cancel: &AtomicBool) -> Result<bool, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let catalog = self.agents.config().catalog();
        let operation = roles::resolve_task_operation(&task, &catalog);
        let role = task
            .role
            .clone()
            .unwrap_or_else(|| roles::WORKER.to_string());
        let role_definition = catalog
            .get(&role)
            .cloned()
            .or_else(|| roles::core_role(roles::WORKER))
            .ok_or_else(|| FactoryError::TaskRoleUnavailable(task.id, role.clone()))?;
        match operation {
            TaskOperation::Planning => Err(FactoryError::RetryLimit(
                task.id,
                "the planner role cannot be scheduled as a task".into(),
            )),
            TaskOperation::Advisory => {
                self.execute_advisory(task_id, &role_definition, &catalog, cancel)
            }
            TaskOperation::Implement | TaskOperation::Verify | TaskOperation::PostProcess => {
                self.execute_implementation(task_id, operation, &role_definition, &catalog, cancel)
            }
            TaskOperation::Review => {
                self.execute_specialized_review(task_id, &role_definition, &catalog, cancel)
            }
        }
    }

    /// Implementation-family operations (implement, verify, post_process): the
    /// agent runs in an isolated worktree, produces evidence, and is accepted
    /// by the built-in final Reviewer. Verification and post-process work may
    /// legitimately change only tests or documentation files.
    fn execute_implementation(
        &self,
        task_id: i64,
        operation: TaskOperation,
        role_definition: &RoleDefinition,
        catalog: &RoleCatalog,
        cancel: &AtomicBool,
    ) -> Result<bool, FactoryError> {
        loop {
            let task = self
                .db
                .get_task(task_id)?
                .ok_or(FactoryError::TaskNotFound(task_id))?;
            let previous_feedback = self
                .db
                .latest_task_attempt(task_id)?
                .and_then(|attempt| attempt.review)
                .filter(|review| review.decision == ReviewDecision::RequestChanges);
            let next_attempt = self
                .db
                .latest_task_attempt(task_id)?
                .map_or(1, |attempt| attempt.attempt_number + 1);
            if next_attempt > MAX_TASK_ATTEMPTS {
                return Ok(false);
            }
            let worktree = if let Some(path) = &task.worktree_path {
                std::path::PathBuf::from(path)
            } else {
                self.create_worktree(task_id)?
            };
            let repo = Repo::detect_bounded(&worktree, &self.root)?;
            let base_sha = self.attempt_base_sha(&task, &repo, &worktree)?;
            let run = self
                .db
                .get_run(task.run_id)?
                .ok_or(FactoryError::RunNotFound(task.run_id))?;
            let team = self.resolve_team(&run)?;
            let role = task
                .role
                .clone()
                .unwrap_or_else(|| roles::WORKER.to_string());
            let worker_pool = team.agents_for_role(&role).to_vec();
            let worker_index = self.db.count_task_attempts(task.run_id)?;
            let worker_name = roles::select_agent(&worker_pool, worker_index)
                .ok_or_else(|| FactoryError::TaskRoleUnavailable(task.id, role.clone()))?
                .clone();
            let worker = self.agents.command_agent_for(&role, &worker_name)?;
            let attempt = self.db.create_task_attempt(
                task_id,
                &role,
                Some(operation),
                worker.name(),
                &worktree.to_string_lossy(),
            )?;
            self.mark_task(task_id, TaskState::Running)?;

            let upstream = self.upstream_artifacts(&task)?;
            let worker_instruction = build_mission(&MissionContext {
                role: role_definition,
                operation,
                task: &task,
                run_objective: &run.objective,
                upstream_artifacts: &upstream,
                previous_feedback: previous_feedback.as_ref(),
                review_input: None,
                final_review: false,
            });
            let worker_run = self.invoke_with_agent(
                worker,
                InvocationScope {
                    run_id: Some(task.run_id),
                    task_id: Some(task.id),
                    attempt_id: Some(attempt.id),
                    role: &role,
                    operation: Some(operation),
                    working_dir: &worktree,
                },
                &worker_instruction,
                cancel,
            );
            let mut evidence = collect_evidence(
                &repo,
                &worktree,
                &base_sha,
                &task,
                worker_run.as_ref().ok().map(|run| &run.result),
            )?;

            let worker_run = match worker_run {
                Ok(run) if run.result.cancelled => {
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Cancelled,
                        run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some("Workflow cancelled while the agent was running."),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Ok(false);
                }
                Ok(run) if run.result.exit_code == Some(0) => {
                    if matches!(
                        operation,
                        TaskOperation::Verify | TaskOperation::PostProcess
                    ) {
                        self.persist_operation_artifact(
                            task.run_id,
                            task.id,
                            attempt.id,
                            &role,
                            operation,
                            &run.result.stdout,
                            &mut evidence,
                        )?;
                    }
                    run
                }
                Ok(run) => {
                    let error = run.result.exit_code.map_or_else(
                        || {
                            format!(
                                "{} process ended without an exit code.",
                                role_definition.name
                            )
                        },
                        |code| format!("{} process exited with code {code}.", role_definition.name),
                    );
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some(&error),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    if attempt.attempt_number >= MAX_TASK_ATTEMPTS {
                        return Ok(false);
                    }
                    self.mark_task(task_id, TaskState::Ready)?;
                    continue;
                }
                Err(error) => {
                    let message = error.to_string();
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        None,
                        evidence.commit_sha.as_deref(),
                        Some(&message),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    if error.is_agent_configuration() {
                        return Err(error);
                    }
                    if attempt.attempt_number >= MAX_TASK_ATTEMPTS {
                        return Ok(false);
                    }
                    self.mark_task(task_id, TaskState::Ready)?;
                    continue;
                }
            };

            self.db
                .set_task_attempt_status(attempt.id, AttemptStatus::Reviewing)?;
            let reviewer_name = roles::select_agent(
                &team.reviewers,
                attempt.attempt_number.saturating_sub(1) as usize,
            )
            .ok_or_else(|| FactoryError::TaskRoleUnavailable(task.id, "reviewer".into()))?
            .clone();
            let reviewer = self.agents.command_agent_for("reviewer", &reviewer_name)?;
            let reviewer_role = catalog
                .get(roles::REVIEWER)
                .cloned()
                .unwrap_or_else(|| roles::core_role(roles::REVIEWER).expect("core reviewer"));
            let review_instruction = build_mission(&MissionContext {
                role: &reviewer_role,
                operation: TaskOperation::Review,
                task: &task,
                run_objective: &run.objective,
                upstream_artifacts: &upstream,
                previous_feedback: None,
                review_input: Some(&ReviewInput {
                    producer_title: task.title.clone(),
                    producer_role: role_definition.name.clone(),
                    evidence: evidence.clone(),
                    producer_output: worker_run.result.stdout.clone(),
                    diff: evidence.diff_patch.clone().unwrap_or_default(),
                }),
                final_review: true,
            });
            let review_run = self.invoke_with_agent(
                reviewer,
                InvocationScope {
                    run_id: Some(task.run_id),
                    task_id: Some(task.id),
                    attempt_id: Some(attempt.id),
                    role: "reviewer",
                    operation: Some(TaskOperation::Review),
                    working_dir: &worktree,
                },
                &review_instruction,
                cancel,
            );
            let review = match review_run {
                Ok(run) if run.result.cancelled => {
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Cancelled,
                        worker_run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some("Workflow cancelled while the Reviewer was running."),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Ok(false);
                }
                Ok(run) if run.result.exit_code == Some(0) => {
                    match parse_review(&run.result.stdout) {
                        Ok(review) => review,
                        Err(reason) => {
                            self.db
                                .set_agent_session_status(run.session_id, "rejected")?;
                            ReviewResult {
                                decision: ReviewDecision::RequestChanges,
                                reason: format!(
                                    "Reviewer returned invalid structured output: {reason}"
                                ),
                                feedback: vec![
                                    "Return a valid approve or request_changes decision.".into(),
                                ],
                            }
                        }
                    }
                }
                Ok(run) => ReviewResult {
                    decision: ReviewDecision::RequestChanges,
                    reason: run.result.exit_code.map_or_else(
                        || "Reviewer ended without an exit code.".to_string(),
                        |code| format!("Reviewer exited with code {code}."),
                    ),
                    feedback: nonempty_lines(&run.result.stderr),
                },
                Err(error) if error.is_agent_configuration() => {
                    let message = error.to_string();
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        worker_run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some(&message),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Err(error);
                }
                Err(error) => ReviewResult {
                    decision: ReviewDecision::RequestChanges,
                    reason: error.to_string(),
                    feedback: Vec::new(),
                },
            };

            if review.decision == ReviewDecision::Approve {
                let integrated_sha =
                    self.integrate_approved_task(&run, &task, &worker_name, &worktree)?;
                let commit_sha = integrated_sha.or_else(|| evidence.commit_sha.clone());
                self.db.finish_task_attempt(
                    attempt.id,
                    AttemptStatus::Approved,
                    worker_run.result.exit_code,
                    commit_sha.as_deref(),
                    None,
                    Some(&evidence),
                    Some(&review),
                )?;
                self.mark_task(task_id, TaskState::Completed)?;
                self.remove_worktree(task_id, false)?;
                return Ok(true);
            }

            self.db.finish_task_attempt(
                attempt.id,
                AttemptStatus::ChangesRequested,
                worker_run.result.exit_code,
                evidence.commit_sha.as_deref(),
                Some(&review.reason),
                Some(&evidence),
                Some(&review),
            )?;
            self.mark_task(task_id, TaskState::Failed)?;
            if attempt.attempt_number >= MAX_TASK_ATTEMPTS {
                return Ok(false);
            }
            self.mark_task(task_id, TaskState::Ready)?;
        }
    }

    /// Advisory operations (Researcher, Architect, custom analysts): the agent
    /// produces context, a persisted artifact is created, and the task
    /// succeeds even with zero repository changes. No implementation reviewer
    /// runs for advisory work.
    fn execute_advisory(
        &self,
        task_id: i64,
        role_definition: &RoleDefinition,
        _catalog: &RoleCatalog,
        cancel: &AtomicBool,
    ) -> Result<bool, FactoryError> {
        loop {
            let task = self
                .db
                .get_task(task_id)?
                .ok_or(FactoryError::TaskNotFound(task_id))?;
            let next_attempt = self
                .db
                .latest_task_attempt(task_id)?
                .map_or(1, |attempt| attempt.attempt_number + 1);
            if next_attempt > MAX_TASK_ATTEMPTS {
                return Ok(false);
            }
            let worktree = if let Some(path) = &task.worktree_path {
                std::path::PathBuf::from(path)
            } else {
                self.create_worktree(task_id)?
            };
            let repo = Repo::detect_bounded(&worktree, &self.root)?;
            let base_sha = self.attempt_base_sha(&task, &repo, &worktree)?;
            let run = self
                .db
                .get_run(task.run_id)?
                .ok_or(FactoryError::RunNotFound(task.run_id))?;
            let team = self.resolve_team(&run)?;
            let role = task
                .role
                .clone()
                .unwrap_or_else(|| roles::WORKER.to_string());
            let pool = team.agents_for_role(&role).to_vec();
            let index = self.db.count_task_attempts(task.run_id)?;
            let agent_name = roles::select_agent(&pool, index)
                .ok_or_else(|| FactoryError::TaskRoleUnavailable(task.id, role.clone()))?
                .clone();
            let agent = self.agents.command_agent_for(&role, &agent_name)?;
            let attempt = self.db.create_task_attempt(
                task_id,
                &role,
                Some(TaskOperation::Advisory),
                agent.name(),
                &worktree.to_string_lossy(),
            )?;
            self.mark_task(task_id, TaskState::Running)?;

            let upstream = self.upstream_artifacts(&task)?;
            let instruction = build_mission(&MissionContext {
                role: role_definition,
                operation: TaskOperation::Advisory,
                task: &task,
                run_objective: &run.objective,
                upstream_artifacts: &upstream,
                previous_feedback: None,
                review_input: None,
                final_review: false,
            });
            let agent_run = self.invoke_with_agent(
                agent,
                InvocationScope {
                    run_id: Some(task.run_id),
                    task_id: Some(task.id),
                    attempt_id: Some(attempt.id),
                    role: &role,
                    operation: Some(TaskOperation::Advisory),
                    working_dir: &worktree,
                },
                &instruction,
                cancel,
            );
            let evidence = collect_evidence(
                &repo,
                &worktree,
                &base_sha,
                &task,
                agent_run.as_ref().ok().map(|run| &run.result),
            )?;

            match agent_run {
                Ok(run) if run.result.cancelled => {
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Cancelled,
                        run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some("Workflow cancelled while the advisory agent was running."),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Ok(false);
                }
                Ok(run) if run.result.exit_code == Some(0) => {
                    let output = run.result.stdout.clone();
                    let mut evidence = evidence;
                    self.persist_operation_artifact(
                        task.run_id,
                        task.id,
                        attempt.id,
                        &role,
                        TaskOperation::Advisory,
                        &output,
                        &mut evidence,
                    )?;
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Approved,
                        run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        None,
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Completed)?;
                    self.prune_inert_worktree(task_id)?;
                    return Ok(true);
                }
                Ok(run) => {
                    let error = run.result.exit_code.map_or_else(
                        || {
                            format!(
                                "{} process ended without an exit code.",
                                role_definition.name
                            )
                        },
                        |code| format!("{} process exited with code {code}.", role_definition.name),
                    );
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some(&error),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    if attempt.attempt_number >= MAX_TASK_ATTEMPTS {
                        return Ok(false);
                    }
                    self.mark_task(task_id, TaskState::Ready)?;
                }
                Err(error) => {
                    let message = error.to_string();
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        None,
                        evidence.commit_sha.as_deref(),
                        Some(&message),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    if error.is_agent_configuration() {
                        return Err(error);
                    }
                    if attempt.attempt_number >= MAX_TASK_ATTEMPTS {
                        return Ok(false);
                    }
                    self.mark_task(task_id, TaskState::Ready)?;
                }
            }
        }
    }

    /// Specialized review operations (Security Auditor, custom review roles,
    /// an explicit final Reviewer): evaluate the evidence, diff, and upstream
    /// artifacts of the implementation they depend on. `request_changes`
    /// routes back to the implementation task (bounded rework) instead of
    /// failing the workflow permanently.
    fn execute_specialized_review(
        &self,
        task_id: i64,
        role_definition: &RoleDefinition,
        catalog: &RoleCatalog,
        cancel: &AtomicBool,
    ) -> Result<bool, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let run = self
            .db
            .get_run(task.run_id)?
            .ok_or(FactoryError::RunNotFound(task.run_id))?;
        let team = self.resolve_team(&run)?;
        let role = task
            .role
            .clone()
            .unwrap_or_else(|| roles::REVIEWER.to_string());
        let pool = team.agents_for_role(&role).to_vec();
        let index = self.db.count_task_attempts(task.run_id)?;
        let reviewer_name = roles::select_agent(&pool, index)
            .ok_or_else(|| FactoryError::TaskRoleUnavailable(task.id, role.clone()))?
            .clone();
        let reviewer = self.agents.command_agent_for(&role, &reviewer_name)?;

        loop {
            let task = self
                .db
                .get_task(task_id)?
                .ok_or(FactoryError::TaskNotFound(task_id))?;
            let next_attempt = self
                .db
                .latest_task_attempt(task_id)?
                .map_or(1, |attempt| attempt.attempt_number + 1);
            if next_attempt > MAX_TASK_ATTEMPTS {
                return Ok(false);
            }
            let worktree = if let Some(path) = &task.worktree_path {
                std::path::PathBuf::from(path)
            } else {
                self.create_worktree(task_id)?
            };
            let repo = Repo::detect_bounded(&worktree, &self.root)?;
            let base_sha = self.attempt_base_sha(&task, &repo, &worktree)?;
            let attempt = self.db.create_task_attempt(
                task_id,
                &role,
                Some(TaskOperation::Review),
                reviewer.name(),
                &worktree.to_string_lossy(),
            )?;
            self.mark_task(task_id, TaskState::Running)?;

            let run_tasks = self.db.list_tasks(task.run_id)?;
            let upstream = self.upstream_artifacts(&task)?;
            let review_input = self.build_review_input(&task, &run_tasks, catalog)?;
            let instruction = build_mission(&MissionContext {
                role: role_definition,
                operation: TaskOperation::Review,
                task: &task,
                run_objective: &run.objective,
                upstream_artifacts: &upstream,
                previous_feedback: None,
                review_input: Some(&review_input),
                final_review: false,
            });
            let review_run = self.invoke_with_agent(
                reviewer.clone(),
                InvocationScope {
                    run_id: Some(task.run_id),
                    task_id: Some(task.id),
                    attempt_id: Some(attempt.id),
                    role: &role,
                    operation: Some(TaskOperation::Review),
                    working_dir: &worktree,
                },
                &instruction,
                cancel,
            );
            let evidence = collect_evidence(
                &repo,
                &worktree,
                &base_sha,
                &task,
                review_run.as_ref().ok().map(|run| &run.result),
            )?;
            let review_exit_code = review_run
                .as_ref()
                .ok()
                .and_then(|run| run.result.exit_code);

            let decision = match review_run {
                Ok(run) if run.result.cancelled => {
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Cancelled,
                        run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some("Workflow cancelled while the review agent was running."),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Ok(false);
                }
                Ok(run) if run.result.exit_code == Some(0) => {
                    match parse_specialized_review(&run.result.stdout) {
                        Ok(decision) => decision,
                        Err(_reason) => {
                            self.db
                                .set_agent_session_status(run.session_id, "rejected")?;
                            SpecializedReview {
                                decision: ReviewDecision::RequestChanges,
                                findings: Vec::new(),
                            }
                        }
                    }
                }
                Ok(run) => {
                    let error = run.result.exit_code.map_or_else(
                        || format!("{} ended without an exit code.", role_definition.name),
                        |code| format!("{} exited with code {code}.", role_definition.name),
                    );
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        run.result.exit_code,
                        evidence.commit_sha.as_deref(),
                        Some(&error),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    if attempt.attempt_number >= MAX_TASK_ATTEMPTS {
                        return Ok(false);
                    }
                    self.mark_task(task_id, TaskState::Ready)?;
                    continue;
                }
                Err(error) if error.is_agent_configuration() => {
                    let message = error.to_string();
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        None,
                        evidence.commit_sha.as_deref(),
                        Some(&message),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Err(error);
                }
                Err(error) => {
                    self.db.finish_task_attempt(
                        attempt.id,
                        AttemptStatus::Failed,
                        None,
                        evidence.commit_sha.as_deref(),
                        Some(&error.to_string()),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    if attempt.attempt_number >= MAX_TASK_ATTEMPTS {
                        return Ok(false);
                    }
                    self.mark_task(task_id, TaskState::Ready)?;
                    continue;
                }
            };

            let content = serde_json::to_string(&decision)
                .unwrap_or_else(|_| r#"{"decision":"request_changes","findings":[]}"#.to_string());
            let artifact = self.db.insert_role_artifact(
                task.run_id,
                Some(task.id),
                Some(attempt.id),
                &role,
                Some(TaskOperation::Review),
                roles::artifact_kind_for(&role, TaskOperation::Review).as_str(),
                &content,
            )?;
            let mut evidence = evidence;
            evidence.artifacts.push(artifact.id);
            let review = review_result_from(&decision.findings);

            if decision.decision == ReviewDecision::Approve {
                self.db.finish_task_attempt(
                    attempt.id,
                    AttemptStatus::Approved,
                    review_exit_code,
                    evidence.commit_sha.as_deref(),
                    None,
                    Some(&evidence),
                    Some(&review),
                )?;
                self.mark_task(task_id, TaskState::Completed)?;
                self.prune_inert_worktree(task_id)?;
                return Ok(true);
            }

            self.db.finish_task_attempt(
                attempt.id,
                AttemptStatus::ChangesRequested,
                review_exit_code,
                evidence.commit_sha.as_deref(),
                Some(&review.reason),
                Some(&evidence),
                Some(&review),
            )?;

            match self.rework_after_review(&task, &run_tasks, catalog)? {
                true => {
                    // The implementation task is Ready again and the review
                    // waits on it; hand control back to the scheduler, which
                    // picks the reworked implementation next.
                    return Ok(true);
                }
                false => {
                    let reason = if find_evaluation_target(&task, &run_tasks, catalog).is_none() {
                        "no implementation dependency is available to rework".to_string()
                    } else {
                        "the implementation reached the bounded retry limit".to_string()
                    };
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Err(FactoryError::RetryLimit(
                        task_id,
                        format!("{} requested changes but {reason}", role_definition.name),
                    ));
                }
            }
        }
    }

    /// Resets the implementation task that a review evaluated to `Ready` so
    /// the workflow reworks it. Completed tasks on the review's dependency
    /// ancestry are unwound to `Pending` so the dependency cascade recomputes
    /// them when the reworked implementation completes again.
    fn rework_after_review(
        &self,
        review_task: &Task,
        run_tasks: &[Task],
        catalog: &RoleCatalog,
    ) -> Result<bool, FactoryError> {
        let Some(seed_id) = find_rework_seed(review_task, run_tasks, catalog) else {
            return Ok(false);
        };
        let seed_attempts = self
            .db
            .latest_task_attempt(seed_id)?
            .map_or(0, |attempt| attempt.attempt_number);
        if seed_attempts >= MAX_TASK_ATTEMPTS {
            return Ok(false);
        }
        for ancestor in transitive_ancestors(review_task, run_tasks)
            .into_iter()
            .filter(|id| *id != seed_id)
        {
            let ancestor_task = run_tasks
                .iter()
                .find(|task| task.id == ancestor)
                .expect("ancestor is in the run tasks");
            if ancestor_task.state == TaskState::Completed {
                self.db.set_task_state(ancestor, TaskState::Pending)?;
            }
        }
        // The review waits for the reworked dependency.
        self.mark_task(review_task.id, TaskState::Blocked)?;
        // Kick off the rework.
        self.mark_task(seed_id, TaskState::Ready)?;
        Ok(true)
    }

    fn build_review_input(
        &self,
        review_task: &Task,
        run_tasks: &[Task],
        catalog: &RoleCatalog,
    ) -> Result<ReviewInput, FactoryError> {
        let target_id = find_evaluation_target(review_task, run_tasks, catalog)
            .or_else(|| review_task.dependencies.first().copied())
            .ok_or_else(|| {
                FactoryError::RetryLimit(
                    review_task.id,
                    "the review task has no dependency providing implementation evidence".into(),
                )
            })?;
        let target = run_tasks
            .iter()
            .find(|task| task.id == target_id)
            .ok_or_else(|| FactoryError::TaskNotFound(target_id))?;
        let attempt = self
            .db
            .latest_task_attempt(target_id)?
            .ok_or_else(|| FactoryError::TaskNotFound(target_id))?;
        let producer_output = self
            .db
            .list_agent_sessions(Some(review_task.run_id))?
            .into_iter()
            .filter(|session| session.attempt_id == Some(attempt.id))
            .filter(|session| session.role != "reviewer")
            .find_map(|session| session.stdout)
            .unwrap_or_default();
        let evidence = attempt.evidence.unwrap_or_default();
        let diff = evidence
            .diff_patch
            .clone()
            .unwrap_or_else(|| evidence.diff_summary.clone());
        Ok(ReviewInput {
            producer_title: target.title.clone(),
            producer_role: target
                .role
                .clone()
                .unwrap_or_else(|| roles::WORKER.to_string()),
            evidence,
            producer_output,
            diff: truncate_chars(&diff, MAX_REVIEW_DIFF_CHARS),
        })
    }

    /// Artifacts produced by a task's direct dependencies. Only artifacts from
    /// the task's own dependency ancestry reach a mission; unrelated branches
    /// never leak context.
    fn upstream_artifacts(&self, task: &Task) -> Result<Vec<RoleArtifact>, FactoryError> {
        if task.dependencies.is_empty() {
            return Ok(Vec::new());
        }
        let artifacts = self.db.list_artifacts_for_tasks(&task.dependencies)?;
        Ok(artifacts)
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_operation_artifact(
        &self,
        run_id: i64,
        task_id: i64,
        attempt_id: i64,
        role: &str,
        operation: TaskOperation,
        output: &str,
        evidence: &mut TaskEvidence,
    ) -> Result<(), FactoryError> {
        let kind = roles::artifact_kind_for(role, operation)
            .as_str()
            .to_string();
        let content = match operation {
            TaskOperation::Advisory => {
                let (report, _) = parse_advisory_report(output);
                serde_json::to_string(&report).unwrap_or_else(|_| default_artifact_content(output))
            }
            _ => {
                let report = parse_producer_report(output);
                let value = serde_json::json!({
                    "summary": report.summary,
                    "commands": report.commands,
                    "results": report.results,
                });
                serde_json::to_string(&value).unwrap_or_else(|_| default_artifact_content(output))
            }
        };
        let artifact = self.db.insert_role_artifact(
            run_id,
            Some(task_id),
            Some(attempt_id),
            role,
            Some(operation),
            &kind,
            &content,
        )?;
        evidence.artifacts.push(artifact.id);
        Ok(())
    }

    fn invoke_with_agent(
        &self,
        agent: CommandAgent,
        scope: InvocationScope<'_>,
        mission: &str,
        cancel: &AtomicBool,
    ) -> Result<Invocation, FactoryError> {
        let started = chrono::Utc::now().to_rfc3339();
        let session = self.db.insert_agent_session(&AgentSession {
            id: 0,
            run_id: scope.run_id,
            task_id: scope.task_id,
            attempt_id: scope.attempt_id,
            role: scope.role.to_string(),
            operation: scope.operation,
            agent: agent.name().to_string(),
            mode: AgentSessionMode::Automated,
            command: agent.command_line(),
            status: "running".to_string(),
            started_at: started,
            finished_at: None,
            exit_code: None,
            duration_ms: None,
            stdout: Some(String::new()),
            stderr: Some(String::new()),
        })?;
        let request = AgentRequest::new(mission, scope.working_dir);
        let timer = Instant::now();
        let mut output_error = None;
        let result = agent.run_observed(&request, cancel, |stream, chunk| {
            if output_error.is_some() {
                return;
            }
            let update = match stream {
                OutputStream::Stdout => {
                    self.db
                        .append_agent_session_output(session.id, Some(chunk), None)
                }
                OutputStream::Stderr => {
                    self.db
                        .append_agent_session_output(session.id, None, Some(chunk))
                }
            };
            if let Err(error) = update {
                output_error = Some(error);
            }
        });
        match result {
            Ok(result) => {
                let status = if result.cancelled {
                    "cancelled"
                } else if result.exit_code == Some(0) {
                    "success"
                } else {
                    "failed"
                };
                self.db.finish_agent_session(
                    session.id,
                    status,
                    result.exit_code,
                    result.duration.as_millis() as u64,
                )?;
                if let Some(error) = output_error {
                    return Err(error.into());
                }
                Ok(Invocation {
                    session_id: session.id,
                    result,
                })
            }
            Err(error) => {
                let message = error.to_string();
                self.db
                    .append_agent_session_output(session.id, None, Some(&message))?;
                self.db.finish_agent_session(
                    session.id,
                    "failed",
                    None,
                    timer.elapsed().as_millis() as u64,
                )?;
                Err(error.into())
            }
        }
    }

    pub fn cancel_run(&self, run_id: i64) -> Result<(), FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        if !matches!(run.status, RunStatus::Planning | RunStatus::Active) {
            return Err(FactoryError::InvalidRunState(
                run_id,
                run.status.as_str().to_string(),
            ));
        }
        self.db.set_run_status(run_id, RunStatus::Cancelled)?;
        Ok(())
    }

    pub fn reconcile_interrupted(&self) -> Result<Reconciliation, FactoryError> {
        let result = self.db.reconcile_interrupted()?;
        // Clean task worktrees from a previous session are reclaimed; dirty
        // ones (work-in-progress) are kept for inspection.
        let _ = self.prune_clean_worktrees();
        Ok(result)
    }

    pub fn mark_task(&self, task_id: i64, target: TaskState) -> Result<MarkOutcome, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let from = task.state;
        if !crate::workflow::Workflow::can_transition(from, target) {
            return Err(FactoryError::InvalidTransition(from, target));
        }
        self.db.set_task_state(task_id, target)?;
        let mut updated = vec![task_id];
        let run_tasks = self.db.list_tasks(task.run_id)?;
        let mut state_of: std::collections::HashMap<i64, TaskState> =
            run_tasks.iter().map(|task| (task.id, task.state)).collect();
        state_of.insert(task_id, target);
        let mut visited = std::collections::HashSet::new();
        visited.insert(task_id);
        let mut frontier = vec![task_id];
        while let Some(changed_id) = frontier.pop() {
            for dependent in run_tasks
                .iter()
                .filter(|candidate| candidate.dependencies.contains(&changed_id))
            {
                if visited.contains(&dependent.id) {
                    continue;
                }
                visited.insert(dependent.id);
                if dependent.state == TaskState::Completed {
                    continue;
                }
                let dependency_states: Vec<TaskState> = dependent
                    .dependencies
                    .iter()
                    .map(|id| state_of[id])
                    .collect();
                let next = crate::workflow::Workflow::next_state_for_dependent(&dependency_states);
                if next != dependent.state {
                    self.db.set_task_state(dependent.id, next)?;
                    state_of.insert(dependent.id, next);
                    updated.push(dependent.id);
                }
                frontier.push(dependent.id);
            }
        }
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        Ok(MarkOutcome {
            task,
            from,
            updated,
        })
    }

    pub fn list_runs(&self) -> Result<Vec<Run>, FactoryError> {
        Ok(self.db.list_runs()?)
    }

    pub fn get_run(&self, id: i64) -> Result<Option<Run>, FactoryError> {
        Ok(self.db.get_run(id)?)
    }

    pub fn list_tasks(&self, run_id: i64) -> Result<Vec<Task>, FactoryError> {
        Ok(self.db.list_tasks(run_id)?)
    }

    pub fn get_task(&self, id: i64) -> Result<Option<Task>, FactoryError> {
        Ok(self.db.get_task(id)?)
    }

    pub fn list_task_attempts(&self, run_id: i64) -> Result<Vec<TaskAttempt>, FactoryError> {
        Ok(self.db.list_task_attempts(run_id)?)
    }

    pub fn latest_task_attempt(&self, task_id: i64) -> Result<Option<TaskAttempt>, FactoryError> {
        Ok(self.db.latest_task_attempt(task_id)?)
    }

    pub fn list_agent_sessions(
        &self,
        run_id: Option<i64>,
    ) -> Result<Vec<AgentSession>, FactoryError> {
        Ok(self.db.list_agent_sessions(run_id)?)
    }

    pub fn list_role_artifacts(&self, run_id: i64) -> Result<Vec<RoleArtifact>, FactoryError> {
        Ok(self.db.list_role_artifacts(run_id)?)
    }

    pub fn list_artifacts_for_task(&self, task_id: i64) -> Result<Vec<RoleArtifact>, FactoryError> {
        Ok(self.db.list_artifacts_for_task(task_id)?)
    }

    pub fn worktree_dir(&self, task_id: i64) -> std::path::PathBuf {
        self.root
            .join(FACTORY_DIR)
            .join("worktrees")
            .join(format!("t{task_id}"))
    }

    /// The integration branch name for a run. Approved implementation-family
    /// work is integrated onto this branch; `main` is never touched.
    fn run_branch(&self, run_id: i64) -> String {
        format!("factory/run-{run_id}")
    }

    /// The diff base used for an attempt's evidence. Once the run has an
    /// integration head, evidence is measured against it (stable across rework
    /// and independent of where the worktree started); otherwise the worktree
    /// head at attempt start is used.
    fn attempt_base_sha(
        &self,
        task: &Task,
        repo: &Repo,
        worktree: &Path,
    ) -> Result<String, FactoryError> {
        if let Some(sha) = self.db.get_run_integration(task.run_id)? {
            return Ok(sha);
        }
        Ok(repo.head_sha(worktree)?)
    }

    /// Integrates an approved implementation-family task's worktree into the
    /// run's `factory/run-<id>` branch:
    ///
    /// 1. The run branch is created at the main repository head the first time.
    /// 2. Any uncommitted agent output is committed under the agent's identity
    ///    (authorship attribution) without relying on local git configuration.
    /// 3. The run branch is fast-forwarded to the worktree head; if it has
    ///    diverged, the task worktree is rebased onto it first.
    /// 4. The new head is recorded as the run's integration sha.
    ///
    /// Returns the new integration head, or `None` when the task introduced no
    /// commits (nothing was integrated).
    fn integrate_approved_task(
        &self,
        run: &Run,
        task: &Task,
        agent_name: &str,
        worktree: &Path,
    ) -> Result<Option<String>, FactoryError> {
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        let run_branch = self.run_branch(run.id);
        if !repo.branch_exists(&run_branch)? {
            let main_head = repo.head_sha(repo.root())?;
            repo.update_ref(&run_branch, &main_head)?;
        }
        if repo.has_uncommitted_changes(worktree)? {
            let message = format!(
                "factory: integrate run-{} task-{} ({agent_name})",
                run.id, task.id
            );
            repo.commit_changes(
                worktree,
                &message,
                (
                    &format!("{agent_name} via Factory {}", run.id),
                    "factory@local",
                ),
            )?;
        }
        let head = repo.head_sha(worktree)?;
        let run_head = repo.resolve_ref(&run_branch)?;
        if head == run_head {
            return Ok(None);
        }
        if !repo.is_ancestor(&run_head, &head)? {
            repo.rebase_onto_in(worktree, &run_branch)?;
            let rebased = repo.head_sha(worktree)?;
            repo.update_ref(&run_branch, &rebased)?;
            self.db.set_run_integration(run.id, Some(&rebased))?;
            return Ok(Some(rebased));
        }
        repo.update_ref(&run_branch, &head)?;
        self.db.set_run_integration(run.id, Some(&head))?;
        Ok(Some(head))
    }

    /// Removes an advisory/specialized-review task's worktree once it no longer
    /// holds changes (the product is a persisted artifact). While work-in-
    /// progress remains, the worktree is left in place for inspection.
    fn prune_inert_worktree(&self, task_id: i64) -> Result<(), FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let Some(path) = task.worktree_path.as_deref() else {
            return Ok(());
        };
        let worktree = std::path::PathBuf::from(path);
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        if !repo.has_uncommitted_changes(&worktree)? {
            self.remove_worktree(task_id, false)?;
        }
        Ok(())
    }

    /// Removes every clean Factory task worktree (`.factory/worktrees/t<N>`).
    /// Worktrees with uncommitted changes are kept. Only task worktrees under
    /// the Factory directory are ever touched; other worktrees the user created
    /// are never removed.
    fn prune_clean_worktrees(&self) -> Result<usize, FactoryError> {
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        let worktrees_root = self.root.join(FACTORY_DIR).join("worktrees");
        let mut pruned = 0;
        for info in repo.list_worktrees()? {
            let path = &info.path;
            if !path.starts_with(&worktrees_root) {
                continue;
            }
            let Some(task_id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix('t'))
                .and_then(|rest| rest.parse::<i64>().ok())
            else {
                continue;
            };
            if repo.has_uncommitted_changes(path)? {
                continue;
            }
            if self.remove_worktree(task_id, false).is_ok() {
                pruned += 1;
            }
        }
        Ok(pruned)
    }

    pub fn create_worktree(&self, task_id: i64) -> Result<std::path::PathBuf, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        if task.state != TaskState::Ready {
            return Err(FactoryError::NotReady(task_id));
        }
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        let directory = self.worktree_dir(task_id);
        let run_branch = self.run_branch(task.run_id);
        let base = if repo.branch_exists(&run_branch)? {
            Some(run_branch.as_str())
        } else {
            None
        };
        repo.add_worktree(&directory, &format!("factory/t{task_id}"), base)?;
        self.db.set_worktree_path(task_id, directory.to_str())?;
        Ok(directory)
    }

    pub fn remove_worktree(&self, task_id: i64, force: bool) -> Result<(), FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        let directory = self.worktree_dir(task_id);
        if repo.find_worktree(&directory)?.is_some() || directory.exists() {
            if force {
                repo.remove_worktree_force(&directory)?;
            } else {
                repo.remove_worktree(&directory)?;
            }
        }
        if task.worktree_path.is_some() {
            self.db.set_worktree_path(task_id, None)?;
        }
        Ok(())
    }

    pub fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>, FactoryError> {
        Ok(Repo::detect_bounded(&self.root, &self.root)?.list_worktrees()?)
    }
}

// --- Workflow stage --------------------------------------------------------

/// Answers "what kind of work is currently happening?", distinct from the
/// operational RunStatus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStage {
    Planning,
    Analysis,
    Implementation,
    Verification,
    Review,
    PostProcessing,
    Completed,
}

impl WorkflowStage {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowStage::Planning => "planning",
            WorkflowStage::Analysis => "analysis",
            WorkflowStage::Implementation => "implementation",
            WorkflowStage::Verification => "verification",
            WorkflowStage::Review => "review",
            WorkflowStage::PostProcessing => "post_processing",
            WorkflowStage::Completed => "completed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WorkflowStage::Planning => "Planning",
            WorkflowStage::Analysis => "Analysis",
            WorkflowStage::Implementation => "Implementation",
            WorkflowStage::Verification => "Verification",
            WorkflowStage::Review => "Review",
            WorkflowStage::PostProcessing => "Post-processing",
            WorkflowStage::Completed => "Completed",
        }
    }
}

/// Derives the current workflow stage from the run status and the operations
/// of the incomplete tasks.
pub fn derive_stage(run: &Run, tasks: &[Task], catalog: &RoleCatalog) -> WorkflowStage {
    if run.status == RunStatus::Planning {
        return WorkflowStage::Planning;
    }
    if run.status == RunStatus::Completed {
        return WorkflowStage::Completed;
    }
    for operation in [
        TaskOperation::PostProcess,
        TaskOperation::Review,
        TaskOperation::Verify,
        TaskOperation::Implement,
        TaskOperation::Advisory,
    ] {
        let stage = match operation {
            TaskOperation::PostProcess => WorkflowStage::PostProcessing,
            TaskOperation::Review => WorkflowStage::Review,
            TaskOperation::Verify => WorkflowStage::Verification,
            TaskOperation::Implement => WorkflowStage::Implementation,
            TaskOperation::Advisory => WorkflowStage::Analysis,
            TaskOperation::Planning => WorkflowStage::Planning,
        };
        if tasks
            .iter()
            .filter(|task| task.state != TaskState::Completed)
            .any(|task| roles::resolve_task_operation(task, catalog) == operation)
        {
            return stage;
        }
    }
    WorkflowStage::Completed
}

// --- Selection helpers -----------------------------------------------------

fn validate_task_dag(tasks: &[Task]) -> Result<(), String> {
    let ids: std::collections::HashSet<i64> = tasks.iter().map(|task| task.id).collect();
    for task in tasks {
        if task.dependencies.contains(&task.id) {
            return Err(format!("task #{} depends on itself", task.id));
        }
        if let Some(unknown) = task
            .dependencies
            .iter()
            .find(|dependency| !ids.contains(dependency))
        {
            return Err(format!(
                "task #{} depends on unknown task #{}",
                task.id, unknown
            ));
        }
    }
    let mut indegree: std::collections::HashMap<i64, usize> = tasks
        .iter()
        .map(|task| (task.id, task.dependencies.len()))
        .collect();
    let mut ready: Vec<i64> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop() {
        visited += 1;
        for task in tasks.iter().filter(|task| task.dependencies.contains(&id)) {
            let degree = indegree.get_mut(&task.id).expect("known task");
            *degree -= 1;
            if *degree == 0 {
                ready.push(task.id);
            }
        }
    }
    if visited != tasks.len() {
        return Err("the task dependency graph contains a cycle".into());
    }
    Ok(())
}

/// The direct dependency whose operation a review task evaluates: an
/// implementation task when present, otherwise a verification task, otherwise
/// any direct dependency.
fn find_evaluation_target(
    review_task: &Task,
    run_tasks: &[Task],
    catalog: &RoleCatalog,
) -> Option<i64> {
    let deps: Vec<&Task> = run_tasks
        .iter()
        .filter(|task| review_task.dependencies.contains(&task.id))
        .collect();
    let by_operation = |operation: TaskOperation| {
        deps.iter()
            .filter(|task| roles::resolve_task_operation(task, catalog) == operation)
            .max_by_key(|task| task.position)
            .map(|task| task.id)
    };
    by_operation(TaskOperation::Implement)
        .or_else(|| by_operation(TaskOperation::Verify))
        .or_else(|| {
            deps.iter()
                .max_by_key(|task| task.position)
                .map(|task| task.id)
        })
}

/// The task to rework after a review requested changes: the deepest
/// implementation task in the review's dependency ancestry (falling back to a
/// verification task), so Worker → Test Engineer → Security Auditor reworks
/// the Worker instead of bouncing the workflow.
fn find_rework_seed(review_task: &Task, run_tasks: &[Task], catalog: &RoleCatalog) -> Option<i64> {
    let ancestors = transitive_ancestors(review_task, run_tasks);
    let mut candidates: Vec<&Task> = run_tasks
        .iter()
        .filter(|task| {
            ancestors.contains(&task.id)
                && roles::resolve_task_operation(task, catalog) == TaskOperation::Implement
        })
        .collect();
    if candidates.is_empty() {
        candidates = run_tasks
            .iter()
            .filter(|task| {
                ancestors.contains(&task.id)
                    && roles::resolve_task_operation(task, catalog) == TaskOperation::Verify
            })
            .collect();
    }
    candidates
        .iter()
        .max_by_key(|task| task.position)
        .map(|task| task.id)
}

/// All task ids a task transitively depends on (its full ancestry).
fn transitive_ancestors(task: &Task, run_tasks: &[Task]) -> std::collections::HashSet<i64> {
    let mut ancestors = std::collections::HashSet::new();
    let mut frontier = task.dependencies.clone();
    while let Some(id) = frontier.pop() {
        if !ancestors.insert(id) {
            continue;
        }
        if let Some(ancestor) = run_tasks.iter().find(|candidate| candidate.id == id) {
            frontier.extend(ancestor.dependencies.iter().copied());
        }
    }
    ancestors
}

// --- Evidence helpers ------------------------------------------------------

fn collect_evidence(
    repo: &Repo,
    worktree: &Path,
    base_sha: &str,
    task: &Task,
    result: Option<&AgentResult>,
) -> Result<TaskEvidence, FactoryError> {
    let git = repo.evidence_since(worktree, base_sha)?;
    let runs = result
        .map(|result| parse_producer_report(&result.stdout))
        .unwrap_or_default();
    let diff_patch = if git.is_change() {
        repo.diff_patch(worktree, base_sha, MAX_REVIEW_DIFF_CHARS)
            .ok()
            .filter(|patch| !patch.trim().is_empty())
    } else {
        None
    };
    Ok(TaskEvidence {
        changed_files: git.changed_files,
        diff_summary: git.diff_summary,
        commit_sha: git.commit_sha,
        commands: runs.commands,
        acceptance_criteria: task.acceptance_criteria.clone(),
        worker_exit_code: result.and_then(|result| result.exit_code),
        artifacts: Vec::new(),
        diff_patch,
    })
}

fn default_artifact_content(output: &str) -> String {
    serde_json::json!({ "summary": tail(output, 4_000) }).to_string()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

fn nonempty_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(20)
        .map(str::to_string)
        .collect()
}

fn tail(value: &str, max_chars: usize) -> &str {
    if value.len() <= max_chars {
        return value;
    }
    let mut start = value.len() - max_chars;
    while !value.is_char_boundary(start) {
        start += 1;
    }
    &value[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_output_is_strict_and_structured() {
        let review = parse_review(
            r#"{"decision":"request_changes","reason":"tests fail","feedback":["fix test"]}"#,
        )
        .unwrap();
        assert_eq!(review.decision, ReviewDecision::RequestChanges);
        assert_eq!(review.feedback, vec!["fix test"]);
        assert!(parse_review("approved").is_err());
    }

    #[test]
    fn task_dag_rejects_cycles() {
        let task = |id, dependencies| Task {
            id,
            run_id: 1,
            title: format!("Task {id}"),
            objective: "test".into(),
            acceptance_criteria: vec!["works".into()],
            state: TaskState::Pending,
            position: id as i32,
            dependencies,
            worktree_path: None,
            role: None,
            operation: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(validate_task_dag(&[task(1, vec![2]), task(2, vec![1])]).is_err());
    }

    fn task_like(id: i64, deps: Vec<i64>, operation: TaskOperation) -> Task {
        Task {
            id,
            run_id: 1,
            title: id.to_string(),
            objective: "o".into(),
            acceptance_criteria: vec![],
            state: TaskState::Completed,
            position: id as i32,
            dependencies: deps,
            worktree_path: None,
            role: None,
            operation: Some(operation),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn empty_catalog() -> RoleCatalog {
        RoleCatalog::build(&std::collections::BTreeMap::new())
    }

    #[test]
    fn rework_seed_prefers_the_deepest_implementation_task() {
        let tasks = vec![
            task_like(1, vec![], TaskOperation::Advisory), // research
            task_like(2, vec![1], TaskOperation::Implement), // worker
            task_like(3, vec![2], TaskOperation::Verify),  // test engineer
            task_like(4, vec![3], TaskOperation::Review),  // security auditor
        ];
        let review = &tasks[3];
        let catalog = empty_catalog();
        assert_eq!(
            find_rework_seed(review, &tasks, &catalog),
            Some(2),
            "rework routes back to the Worker, not the validator or reviewer"
        );
        assert_eq!(
            find_evaluation_target(review, &tasks, &catalog),
            Some(3),
            "the evaluated dependency is the verification task"
        );
    }

    #[test]
    fn stage_derivation_follows_operations() {
        use crate::roles::resolve_task_operation;
        let catalog = empty_catalog();
        let mk = |state, operation| {
            let mut task = task_like(1, vec![], operation);
            task.state = state;
            task
        };
        let run = |status| Run {
            id: 1,
            objective: "o".into(),
            status,
            planner_agent: None,
            team: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let _ = resolve_task_operation; // exercised through derive_stage
        assert_eq!(
            derive_stage(
                &run(RunStatus::Active),
                &[mk(TaskState::Running, TaskOperation::Advisory)],
                &catalog
            ),
            WorkflowStage::Analysis
        );
        assert_eq!(
            derive_stage(
                &run(RunStatus::Active),
                &[mk(TaskState::Running, TaskOperation::Verify)],
                &catalog
            ),
            WorkflowStage::Verification
        );
        assert_eq!(
            derive_stage(
                &run(RunStatus::Active),
                &[mk(TaskState::Running, TaskOperation::Review)],
                &catalog
            ),
            WorkflowStage::Review
        );
        assert_eq!(
            derive_stage(
                &run(RunStatus::Active),
                &[mk(TaskState::Ready, TaskOperation::Implement)],
                &catalog
            ),
            WorkflowStage::Implementation
        );
    }

    #[test]
    fn implementation_mission_uses_the_role_aware_builder() {
        let task = Task {
            id: 7,
            run_id: 1,
            title: "Add index".into(),
            objective: "Speed up the query.".into(),
            acceptance_criteria: vec!["query plan uses the index".into()],
            state: TaskState::Ready,
            position: 0,
            dependencies: Vec::new(),
            worktree_path: None,
            role: Some("database_engineer".into()),
            operation: Some(TaskOperation::Implement),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let role = crate::roles::core_role(crate::roles::WORKER).unwrap();
        let mission = build_mission(&MissionContext {
            role: &role,
            operation: TaskOperation::Implement,
            task: &task,
            run_objective: "objective",
            upstream_artifacts: &[],
            previous_feedback: None,
            review_input: None,
            final_review: false,
        });
        assert!(mission.contains("ROLE\nWorker — "));
        assert!(mission.contains("WORKFLOW OBJECTIVE\nobjective"));
        assert!(mission.contains("TASK\nAdd index — Speed up the query."));
        assert!(mission.contains("OPERATION\nimplement"));
        assert!(mission.contains("ACCEPTANCE CRITERIA\n- query plan uses the index"));
        assert!(mission.contains("OUTPUT CONTRACT"));
        let review = ReviewResult {
            decision: ReviewDecision::RequestChanges,
            reason: "missing test".into(),
            feedback: vec!["add a regression test".into()],
        };
        let retry = build_mission(&MissionContext {
            role: &role,
            operation: TaskOperation::Implement,
            task: &task,
            run_objective: "objective",
            upstream_artifacts: &[],
            previous_feedback: Some(&review),
            review_input: None,
            final_review: false,
        });
        assert!(retry.contains("CONTEXT\nPrevious review requested changes:\nmissing test"));
        assert!(retry.contains("- add a regression test"));
    }
}
