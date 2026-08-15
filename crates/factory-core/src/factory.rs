use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use factory_agent::{AgentError, AgentRequest, AgentResult, CommandAgent, OutputStream};
use factory_db::{FactoryDb, Reconciliation};
use factory_git::{Repo, WorktreeInfo};
use factory_types::{
    AgentSession, AgentSessionMode, AttemptStatus, ReviewDecision, ReviewResult, Run, RunStatus,
    Task, TaskAttempt, TaskEvidence, TaskState, WorkflowTeam,
};
use serde::Deserialize;
use thiserror::Error;

use crate::config::{AgentResolutionError, Agents, ConfigError};
use crate::planner::{
    mission as planner_mission, normalize_plan, parse_plan, validate_plan_roles, PlanError,
};
use crate::roles::{self, RoleDefinition};

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
    #[error("task #{0} reached the retry limit of {MAX_TASK_ATTEMPTS} attempts")]
    RetryLimit(i64),
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
        let available_roles: Vec<(String, String)> = team
            .task_roles()
            .into_iter()
            .map(|role| {
                let description = catalog
                    .get(&role)
                    .map(|definition| definition.description.clone())
                    .unwrap_or_default();
                (role, description)
            })
            .collect();
        let allowed_roles: std::collections::HashSet<String> = available_roles
            .iter()
            .map(|(role, _)| role.clone())
            .collect();
        let available_roles: Vec<(&str, &str)> = available_roles
            .iter()
            .map(|(role, description)| (role.as_str(), description.as_str()))
            .collect();
        let mut rejection: Option<String> = None;
        for attempt in 0..crate::planner::MAX_ATTEMPTS {
            let instruction =
                planner_mission(&run.objective, &available_roles, rejection.as_deref());
            let invocation = self.invoke_with_agent(
                planner.clone(),
                InvocationScope {
                    run_id: Some(run_id),
                    task_id: None,
                    attempt_id: None,
                    role: "planner",
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
                Ok(plan)
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
            return Err(FactoryError::RetryLimit(task_id));
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

    fn execute_task(&self, task_id: i64, cancel: &AtomicBool) -> Result<bool, FactoryError> {
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
            let base_sha = repo.head_sha(&worktree)?;
            let run = self
                .db
                .get_run(task.run_id)?
                .ok_or(FactoryError::RunNotFound(task.run_id))?;
            let team = self.resolve_team(&run)?;
            let role = task
                .role
                .clone()
                .unwrap_or_else(|| roles::WORKER.to_string());
            let catalog = self.agents.config().catalog();
            let role_definition = catalog
                .get(&role)
                .cloned()
                .or_else(|| roles::core_role(roles::WORKER))
                .ok_or_else(|| FactoryError::TaskRoleUnavailable(task.id, role.clone()))?;
            let worker_pool = team.agents_for_role(&role).to_vec();
            let worker_index = self.db.count_task_attempts(task.run_id)?;
            let worker_name = roles::select_agent(&worker_pool, worker_index)
                .ok_or_else(|| FactoryError::TaskRoleUnavailable(task.id, role.clone()))?
                .clone();
            let worker = self.agents.command_agent_for(&role, &worker_name)?;
            let attempt = self.db.create_task_attempt(
                task_id,
                &role,
                worker.name(),
                &worktree.to_string_lossy(),
            )?;
            self.mark_task(task_id, TaskState::Running)?;

            let worker_instruction =
                worker_mission(&task, &role_definition, previous_feedback.as_ref());
            let worker_run = self.invoke_with_agent(
                worker,
                InvocationScope {
                    run_id: Some(task.run_id),
                    task_id: Some(task.id),
                    attempt_id: Some(attempt.id),
                    role: &role,
                    working_dir: &worktree,
                },
                &worker_instruction,
                cancel,
            );
            let evidence = collect_evidence(
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
                        Some("Workflow cancelled while the Worker was running."),
                        Some(&evidence),
                        None,
                    )?;
                    self.mark_task(task_id, TaskState::Failed)?;
                    return Ok(false);
                }
                Ok(run) if run.result.exit_code == Some(0) => run,
                Ok(run) => {
                    let error = run.result.exit_code.map_or_else(
                        || "Worker process ended without an exit code.".to_string(),
                        |code| format!("Worker process exited with code {code}."),
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
            let review_instruction = reviewer_mission(
                &task,
                &role_definition,
                &evidence,
                &worker_run.result.stdout,
            );
            let review_run = self.invoke_with_agent(
                reviewer,
                InvocationScope {
                    run_id: Some(task.run_id),
                    task_id: Some(task.id),
                    attempt_id: Some(attempt.id),
                    role: "reviewer",
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
                self.db.finish_task_attempt(
                    attempt.id,
                    AttemptStatus::Approved,
                    worker_run.result.exit_code,
                    evidence.commit_sha.as_deref(),
                    None,
                    Some(&evidence),
                    Some(&review),
                )?;
                self.mark_task(task_id, TaskState::Completed)?;
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
        Ok(self.db.reconcile_interrupted()?)
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

    pub fn worktree_dir(&self, task_id: i64) -> std::path::PathBuf {
        self.root
            .join(FACTORY_DIR)
            .join("worktrees")
            .join(format!("t{task_id}"))
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
        repo.add_worktree(&directory, &format!("factory/t{task_id}"))?;
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

fn worker_mission(
    task: &Task,
    role: &RoleDefinition,
    previous_review: Option<&ReviewResult>,
) -> String {
    let criteria = task
        .acceptance_criteria
        .iter()
        .map(|criterion| format!("- {criterion}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut mission = format!(
        "ROLE\n{} — {}\n\n{}\n\nOBJECTIVE\n{}\n\nTASK\n{}\n",
        role.name,
        role.description,
        role.instructions.trim(),
        task.objective.trim(),
        task.title.trim()
    );
    mission.push_str(
        "\nCONSTRAINTS\n\
- Work only in the current git worktree.\n\
- Do not modify Factory orchestration state (.factory).\n\
- These constraints take precedence over any role instruction.",
    );
    if let Some(review) = previous_review {
        mission.push_str("\n\nCONTEXT\nPrevious review requested changes:\n");
        mission.push_str(&review.reason);
        if !review.feedback.is_empty() {
            mission.push('\n');
            mission.push_str(
                &review
                    .feedback
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
    }
    mission.push_str(&format!("\n\nACCEPTANCE CRITERIA\n{criteria}"));
    mission.push_str(
        "\n\nOUTPUT CONTRACT\n\
At the end, report a concise JSON object with keys `summary` and `commands` \
(an array of the commands or tests you ran).",
    );
    mission
}

fn reviewer_mission(
    task: &Task,
    worker_role: &RoleDefinition,
    evidence: &TaskEvidence,
    worker_output: &str,
) -> String {
    let worker_output = tail(worker_output, 20_000);
    let criteria = task
        .acceptance_criteria
        .iter()
        .map(|criterion| format!("- {criterion}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "ROLE\nReviewer — Independently evaluates task output against acceptance criteria.\n\
The {} produced the change below.\n\nOBJECTIVE\n{}\n\nTASK\n{}\n\nACCEPTANCE CRITERIA\n{}\n\n\
EVIDENCE\nChanged files: {}\nDiff summary:\n{}\nCommit: {}\nWorker-reported commands: {}\n\n\
WORKER OUTPUT\n{}\n\n\
OUTPUT CONTRACT\n\
Return one JSON object only: {{\"decision\":\"approve\"|\"request_changes\",\"reason\":string,\"feedback\":[string]}}.\n\
Approve only when the evidence and repository changes satisfy the task. Do not modify files.",
        worker_role.name,
        task.objective.trim(),
        task.title.trim(),
        criteria,
        evidence.changed_files.join(", "),
        evidence.diff_summary,
        evidence.commit_sha.as_deref().unwrap_or("not committed"),
        evidence.commands.join(", "),
        worker_output
    )
}

fn collect_evidence(
    repo: &Repo,
    worktree: &Path,
    base_sha: &str,
    task: &Task,
    result: Option<&AgentResult>,
) -> Result<TaskEvidence, FactoryError> {
    let git = repo.evidence_since(worktree, base_sha)?;
    Ok(TaskEvidence {
        changed_files: git.changed_files,
        diff_summary: git.diff_summary,
        commit_sha: git.commit_sha,
        commands: result
            .map(|result| parse_worker_commands(&result.stdout))
            .unwrap_or_default(),
        acceptance_criteria: task.acceptance_criteria.clone(),
        worker_exit_code: result.and_then(|result| result.exit_code),
    })
}

#[derive(Deserialize)]
struct WorkerReport {
    #[serde(default)]
    commands: Vec<String>,
}

fn parse_worker_commands(output: &str) -> Vec<String> {
    let trimmed = output.trim();
    let candidate = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    serde_json::from_str::<WorkerReport>(candidate)
        .map(|report| report.commands)
        .unwrap_or_default()
}

fn parse_review(output: &str) -> Result<ReviewResult, String> {
    let trimmed = output.trim();
    let candidate = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    let review: ReviewResult =
        serde_json::from_str(candidate).map_err(|error| format!("invalid JSON: {error}"))?;
    if review.reason.trim().is_empty() {
        return Err("reason must not be empty".into());
    }
    Ok(review)
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
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(validate_task_dag(&[task(1, vec![2]), task(2, vec![1])]).is_err());
    }

    #[test]
    fn worker_mission_keeps_role_instructions_and_factory_constraints_separate() {
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
            created_at: String::new(),
            updated_at: String::new(),
        };
        let role = crate::roles::core_role(crate::roles::WORKER).unwrap();
        let mission = worker_mission(&task, &role, None);
        assert!(mission.contains("ROLE\nWorker — "));
        assert!(mission.contains("OBJECTIVE\nSpeed up the query."));
        assert!(mission.contains("TASK\nAdd index"));
        assert!(mission.contains("CONSTRAINTS"));
        assert!(mission.contains("Do not modify Factory orchestration state"));
        assert!(mission.contains("take precedence over any role instruction"));
        assert!(mission.contains("ACCEPTANCE CRITERIA\n- query plan uses the index"));
        assert!(mission.contains("OUTPUT CONTRACT"));
        let review = ReviewResult {
            decision: ReviewDecision::RequestChanges,
            reason: "missing test".into(),
            feedback: vec!["add a regression test".into()],
        };
        let retry = worker_mission(&task, &role, Some(&review));
        assert!(retry.contains("CONTEXT\nPrevious review requested changes:\nmissing test"));
        assert!(retry.contains("- add a regression test"));
    }

    #[test]
    fn reviewer_mission_names_the_worker_role() {
        let task = Task {
            id: 7,
            run_id: 1,
            title: "Add index".into(),
            objective: "Speed up the query.".into(),
            acceptance_criteria: vec!["query plan uses the index".into()],
            state: TaskState::Running,
            position: 0,
            dependencies: Vec::new(),
            worktree_path: None,
            role: Some("database_engineer".into()),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let evidence = TaskEvidence {
            changed_files: vec!["migrations/1.sql".into()],
            diff_summary: "1 file changed".into(),
            commit_sha: Some("abc".into()),
            commands: vec!["cargo test".into()],
            acceptance_criteria: Vec::new(),
            worker_exit_code: Some(0),
        };
        let role = crate::roles::core_role(crate::roles::TEST_ENGINEER).unwrap();
        let mission = reviewer_mission(&task, &role, &evidence, "worker said hi");
        assert!(mission.contains("The Test Engineer produced the change below."));
        assert!(mission.contains("OUTPUT CONTRACT"));
        assert!(mission.contains("approve"));
    }
}
