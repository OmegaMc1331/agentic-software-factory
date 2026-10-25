use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use chrono::Utc;
use factory_agent::{AgentError, AgentRequest, AgentResult, CommandAgent, OutputStream};
use factory_db::{FactoryDb, Reconciliation};
use factory_git::{Repo, WorktreeInfo};
use factory_mcp::{render_tool_catalog, GrantContext, ToolRegistry};
use factory_types::{
    AgentSession, AgentSessionMode, AttemptStatus, DeliveryState, GitHubDelivery, GitHubIssueLink,
    McpToolMetadata, Recipe, RecipeRouting, RecipeSnapshot, ReviewDecision, ReviewResult,
    RoleArtifact, RoutingCandidateScore, RoutingDecision, RoutingPreview, Run, RunStatus,
    SpecializedReview, Task, TaskAttempt, TaskEvidence, TaskOperation, TaskState, WorkflowTeam,
};
use thiserror::Error;

use crate::capacity::{AgentCapacity, LoadGuard};
use crate::config::{AgentResolutionError, Agents, ConfigError, RoutingConfig};
use crate::mission::{
    build_mission, parse_advisory_report, parse_producer_report, parse_review,
    parse_specialized_review, review_result_from, MissionContext, ReviewInput,
    MAX_REVIEW_DIFF_CHARS,
};
use crate::planner::{
    mission as planner_mission, normalize_plan, normalize_plan_with_operations, parse_plan,
    validate_plan_roles, PlanError, PlannerRecipeContext,
};
use crate::recipes::{self, RecipeCatalog, RecipeError};
use crate::roles::{self, RoleCatalog, RoleDefinition};
use crate::routing::{self, CandidateFacts};

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
    #[error("recipe error: {0}")]
    Recipe(#[from] RecipeError),
    #[error("task #{0} reached the retry limit of {MAX_TASK_ATTEMPTS} attempts: {1}")]
    RetryLimit(i64, String),
    /// A task cannot legally execute under its effective policy. Raised before
    /// an agent process is started; policy failures never consume task
    /// retries.
    #[error("policy blocked: {0}")]
    PolicyBlocked(String),
    /// An attempt tripped the effective policy (wrote outside scopes, ran a
    /// denied command). The attempt is failed and the run stops; a violation
    /// is not a transient execution failure and is never auto-retried.
    #[error("policy violation: {0}")]
    PolicyViolation(String),
    #[error("planning failed: {0}")]
    Plan(#[from] PlanError),
    #[error("agent process failed: {0}")]
    AgentProcess(#[from] AgentError),
    #[error(
        "{role} output exceeded the {limit_bytes}-byte capture limit (stdout {stdout_bytes} bytes, stderr {stderr_bytes} bytes); structured output was not parsed"
    )]
    AgentOutputLimit {
        role: String,
        limit_bytes: u64,
        stdout_bytes: u64,
        stderr_bytes: u64,
    },
    #[error("agent resolution: {0}")]
    Agent(#[from] AgentResolutionError),
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("database error: {0}")]
    Db(#[from] factory_db::DbError),
    #[error("git error: {0}")]
    Git(#[from] factory_git::GitError),
    #[error("GitHub error: {0}")]
    GitHub(#[from] factory_github::GitHubError),
    #[error("external tool error: {0}")]
    Mcp(#[from] factory_mcp::McpError),
    #[error("delivery not allowed: {0}")]
    NotDeliverable(String),
    #[error("task {0} is not ready to run")]
    NotReady(i64),
    #[error("routing override: {0}")]
    RoutingOverride(String),
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

/// The chosen agent of one dispatch: its name, the reserved capacity slot
/// (held until the invocation finishes), and the audit record explaining
/// the choice.
struct RoutedAgent<'a> {
    agent: CommandAgent,
    _guard: LoadGuard<'a>,
    record: RoutingRecord,
}

/// Everything a [`RoutingDecision`] row needs except the ids, which are
/// only known once the attempt exists.
#[derive(Debug, Clone)]
struct RoutingRecord {
    mode: String,
    language: Option<String>,
    candidates: Vec<RoutingCandidateScore>,
    reason: String,
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
    capacity: AgentCapacity,
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
            capacity: AgentCapacity::new(),
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
            capacity: AgentCapacity::new(),
        })
    }

    pub fn agents(&self) -> &Agents {
        &self.agents
    }

    /// The shared in-flight load tracker used by routing and dispatch. The
    /// parallel runtime (and tests) can observe and reserve agent slots
    /// through it; reservations are released when the returned guards drop.
    pub fn capacity(&self) -> &AgentCapacity {
        &self.capacity
    }

    pub fn begin_run(
        &self,
        objective: &str,
        team: Option<WorkflowTeam>,
    ) -> Result<Run, FactoryError> {
        self.begin_run_with_recipe(objective, team, None)
    }

    /// Creates a workflow, optionally from a recipe. The recipe's roles
    /// pre-fill the team (the caller may still pass an explicit team), and an
    /// immutable snapshot of the applied recipe configuration is persisted on
    /// the run — later recipe edits never mutate it.
    pub fn begin_run_with_recipe(
        &self,
        objective: &str,
        team: Option<WorkflowTeam>,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<Run, FactoryError> {
        let objective = objective.trim();
        if objective.is_empty() {
            return Err(FactoryError::EmptyObjective);
        }
        if let Some(recipe) = recipe {
            self.validate_recipe_roles_available(recipe)?;
        }
        let team = self.resolve_new_team(team, recipe)?;
        let planner = self.agents.command_agent_for("planner", &team.planner)?;
        let run =
            self.db
                .create_run_with_status(objective, Some(planner.name()), RunStatus::Planning)?;
        self.db.set_run_team(run.id, &team)?;
        if let Some(recipe) = recipe {
            self.db.set_run_recipe(run.id, recipe)?;
        }
        self.db
            .get_run(run.id)?
            .ok_or(FactoryError::RunNotFound(run.id))
    }

    /// A recipe may only use roles the project can actually resolve: every
    /// selected role needs at least one assignment. Unavailable roles produce
    /// a clear error — they are never silently replaced (e.g. by Worker).
    fn validate_recipe_roles_available(&self, recipe: &RecipeSnapshot) -> Result<(), FactoryError> {
        for role in &recipe.roles {
            if self.agents.config().assignments_for(role).is_empty() {
                return Err(FactoryError::InvalidTeam(format!(
                    "Recipe '{}' requires role '{role}' but no agent is assigned to it. Assign \
                     one in Settings or pick a different recipe.",
                    recipe.name
                )));
            }
        }
        Ok(())
    }

    fn resolve_new_team(
        &self,
        team: Option<WorkflowTeam>,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<WorkflowTeam, FactoryError> {
        let (team, complete) = match team {
            Some(team) => (team, true),
            None => match recipe.filter(|recipe| !recipe.roles.is_empty()) {
                Some(recipe) => (self.team_from_recipe(recipe)?, false),
                None => (
                    self.agents
                        .config()
                        .initial_team()
                        .map_err(FactoryError::InvalidTeam)?,
                    false,
                ),
            },
        };
        let validation = if complete {
            self.agents.config().validate_team(&team)
        } else {
            self.agents.config().validate_partial_team(&team)
        };
        validation.map_err(FactoryError::InvalidTeam)?;
        Ok(team)
    }

    /// The team a recipe pre-fills: the preferred assignment of each pipeline
    /// role, plus every additional role the recipe selects. Roles with no
    /// assignment were already rejected by
    /// [`validate_recipe_roles_available`](Self::validate_recipe_roles_available).
    fn team_from_recipe(&self, recipe: &RecipeSnapshot) -> Result<WorkflowTeam, FactoryError> {
        let config = self.agents.config();
        let preferred = |role: &str| -> Option<String> {
            config
                .preferred_assignment(role)
                .map(|assignment| assignment.agent.clone())
        };
        let planner = preferred(roles::PLANNER).ok_or_else(|| {
            FactoryError::InvalidTeam(
                "No agent is assigned to the planner role. Configure one from the dashboard."
                    .to_string(),
            )
        })?;
        let mut team = WorkflowTeam {
            planner,
            workers: preferred(roles::WORKER)
                .map(|agent| vec![agent])
                .unwrap_or_default(),
            reviewers: preferred(roles::REVIEWER)
                .map(|agent| vec![agent])
                .unwrap_or_default(),
            additional: std::collections::BTreeMap::new(),
        };
        for role in &recipe.roles {
            if roles::is_pipeline_role(role) {
                continue;
            }
            let agent = preferred(role).ok_or_else(|| {
                FactoryError::InvalidTeam(format!(
                    "Recipe '{}' requires role '{role}' but no agent is assigned to it.",
                    recipe.name
                ))
            })?;
            team.additional.insert(role.clone(), vec![agent]);
        }
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

    /// Best-effort rendered repository context for a mission. The engine never
    /// blocks a task: a disabled config or any git/index failure degrades to an
    /// empty section rather than an error.
    #[allow(clippy::too_many_arguments)]
    fn repository_context(
        &self,
        task: &Task,
        operation: TaskOperation,
        scope_dir: &Path,
        base_sha: &str,
        changed_files: &[String],
        upstream: &[RoleArtifact],
    ) -> String {
        let config = self.agents.config().context;
        if !config.enabled {
            return String::new();
        }
        let role_id = task.role.as_deref().unwrap_or(roles::WORKER);
        let request = factory_context::ContextRequest {
            scope_dir: scope_dir.to_path_buf(),
            root_dir: self.root.clone(),
            base_sha: Some(base_sha.to_string()),
            role_id: Some(role_id.to_string()),
            operation: Some(operation),
            title: task.title.clone(),
            objective: task.objective.clone(),
            acceptance_criteria: task.acceptance_criteria.clone(),
            changed_files: changed_files.to_vec(),
            upstream_artifact_snippets: upstream
                .iter()
                .rev()
                .take(6)
                .map(|artifact| artifact.content.chars().take(2000).collect())
                .collect(),
        };
        let mut engine =
            factory_context::ContextEngine::new(&self.root, &self.root.join(FACTORY_DIR), config);
        match engine.resolve(&request) {
            Ok(resolved) => factory_context::render_repository_context(&resolved),
            Err(_) => String::new(),
        }
    }

    pub fn create_run(&self, objective: &str) -> Result<RunOutcome, FactoryError> {
        let run = self.begin_run(objective, None)?;
        self.plan_run(run.id, &AtomicBool::new(false))
    }

    /// The project recipe catalog: built-in recipes plus the custom recipes
    /// from `.factory/recipes.toml`.
    pub fn recipes(&self) -> Result<RecipeCatalog, FactoryError> {
        Ok(RecipeCatalog::load(&self.root)?)
    }

    /// Resolves a recipe id from the catalog into a validated, immutable
    /// snapshot. Fails with a clear error when the recipe is unknown or one
    /// of its roles has no assigned agent.
    pub fn recipe_snapshot(&self, id: &str) -> Result<RecipeSnapshot, FactoryError> {
        let catalog = self.recipes()?;
        let recipe = catalog
            .get(id)
            .ok_or_else(|| FactoryError::Recipe(RecipeError::NotFound(id.to_string())))?;
        let snapshot = recipe.snapshot();
        self.validate_recipe_roles_available(&snapshot)?;
        Ok(snapshot)
    }

    /// Validates an inline recipe definition (an unsaved recipe submitted
    /// with the workflow) and snapshots it the same way a catalog recipe is
    /// snapshotted.
    pub fn recipe_snapshot_from_definition(
        &self,
        recipe: &Recipe,
    ) -> Result<RecipeSnapshot, FactoryError> {
        recipes::validate_recipe(recipe, self.agents.config(), false)
            .map_err(FactoryError::InvalidTeam)?;
        let snapshot = recipe.snapshot();
        self.validate_recipe_roles_available(&snapshot)?;
        Ok(snapshot)
    }

    /// Derives a reusable recipe from a run's current configuration: the
    /// team's roles, the applied recipe's routing/concurrency/guidance, and
    /// the project's per-role policy presets. Configuration only — never
    /// tasks, DAG edges, attempts, sessions, or Git state ("Save as Recipe",
    /// not "Clone Workflow"). `id`, `name`, and `description` are left for
    /// the caller to fill.
    pub fn recipe_from_run(&self, run_id: i64) -> Result<Recipe, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        let team = self.resolve_team(&run)?;
        let config = self.agents.config();
        let mut policy_presets = std::collections::BTreeMap::new();
        for role in team.roles() {
            if let Some(preset) = config.policies.role(&role).and_then(|scope| scope.preset) {
                policy_presets.insert(role.clone(), preset.as_str().to_string());
            }
        }
        let (routing, max_parallel_tasks, planner_guidance, tools) = match &run.recipe {
            Some(snapshot) => (
                snapshot.routing,
                snapshot.max_parallel_tasks,
                snapshot.planner_guidance.clone(),
                snapshot.tools.clone(),
            ),
            None => (
                RecipeRouting::ProjectDefault,
                Some(config.runtime.max_parallel_tasks),
                String::new(),
                Vec::new(),
            ),
        };
        Ok(Recipe {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            roles: team.roles(),
            routing,
            max_parallel_tasks,
            policy_presets,
            tools,
            planner_guidance,
            github: None,
            builtin: false,
        })
    }

    /// The recipe snapshot persisted on a run, when it has one.
    pub fn run_recipe(&self, run_id: i64) -> Result<Option<RecipeSnapshot>, FactoryError> {
        Ok(self.db.get_run(run_id)?.and_then(|run| run.recipe))
    }

    /// The run's effective task-concurrency preference: the recipe
    /// snapshot's bounded preference when set, else the project `[runtime]`
    /// value. The parallel scheduler treats this as its per-run width bound.
    pub fn run_max_parallel_tasks(&self, run_id: i64) -> Result<usize, FactoryError> {
        let project_default = self.agents.max_parallel_tasks();
        Ok(self
            .run_recipe(run_id)?
            .map(|recipe| recipe.effective_max_parallel_tasks(project_default))
            .unwrap_or(project_default))
    }

    /// The effective policy for a dispatch under a run's recipe snapshot.
    /// The recipe's policy presets apply as one extra only-narrowing layer:
    /// the Policy Engine (baseline invariants, role and agent scopes) stays
    /// authoritative and a recipe can never widen it.
    fn policy_with_recipe(
        &self,
        recipe: Option<&RecipeSnapshot>,
        role: &str,
        agent: &str,
    ) -> factory_policy::EffectivePolicy {
        let restriction =
            recipe.and_then(|snapshot| recipes::snapshot_policy_restriction(snapshot, role));
        self.agents
            .config()
            .policies
            .effective_restricted(role, agent, restriction.as_ref())
    }

    /// Role-level variant of [`policy_with_recipe`](Self::policy_with_recipe)
    /// used for diagnostics.
    fn role_policy_with_recipe(
        &self,
        recipe: Option<&RecipeSnapshot>,
        role: &str,
    ) -> factory_policy::EffectivePolicy {
        let restriction =
            recipe.and_then(|snapshot| recipes::snapshot_policy_restriction(snapshot, role));
        self.agents
            .config()
            .policies
            .effective_for_role_restricted(role, restriction.as_ref())
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
        let untrusted_notice = self.untrusted_issue_notice(run_id)?;
        // Recipe guidance is bounded background context for the Planner; it
        // never overrides the schema, role rules, policies, or the objective.
        let recipe_context = run.recipe.as_ref().map(|snapshot| PlannerRecipeContext {
            name: snapshot.name.clone(),
            guidance: snapshot.planner_guidance.clone(),
        });
        let mut rejection: Option<String> = None;
        for attempt in 0..crate::planner::MAX_ATTEMPTS {
            let instruction = planner_mission(
                &run.objective,
                &planner_roles,
                rejection.as_deref(),
                untrusted_notice.as_deref(),
                recipe_context.as_ref(),
            );
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
                let error = match invocation.result.exit_code {
                    Some(exit_code) => AgentError::ExecutionFailed {
                        command: planner.command().to_string(),
                        exit_code,
                    },
                    None => AgentError::Spawn(
                        planner.command().to_string(),
                        "the planner process ended without an exit code".to_string(),
                    ),
                };
                return Err(PlanError::Agent(error).into());
            }
            if let Some(error) = agent_output_limit_error("planner", &invocation.result) {
                self.db
                    .set_agent_session_status(invocation.session_id, "rejected")?;
                return Err(error);
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
        // A blocked run can be started again: policy blocks leave the run
        // stopped with its tasks untouched, and starting re-validates the
        // (possibly fixed) configuration from scratch.
        if !matches!(run.status, RunStatus::Planned | RunStatus::Blocked) {
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
        // Manual routing overrides are validated before start so an invalid
        // pin blocks with a clear error instead of surfacing mid-run.
        let catalog_for_overrides = self.agents.config().catalog();
        for task in &tasks {
            if let Some(pinned) = &task.agent_override {
                let role = task.role.as_deref().unwrap_or(roles::WORKER);
                if !team.agents_for_role(role).contains(pinned) {
                    return Err(FactoryError::RoutingOverride(format!(
                        "task #{} pins agent '{}' which is not assigned to role '{}' in this \
                         workflow's team",
                        task.id, pinned, role
                    )));
                }
                let operation = roles::resolve_task_operation(task, &catalog_for_overrides);
                let policy = self.policy_with_recipe(run.recipe.as_ref(), role, pinned);
                if let Err(reason) =
                    factory_policy::validate_executable(&policy, operation.as_str())
                {
                    return Err(FactoryError::RoutingOverride(format!(
                        "task #{} pins agent '{}' which is policy-ineligible for operation \
                         '{}': {}",
                        task.id,
                        pinned,
                        operation.as_str(),
                        reason
                    )));
                }
            }
        }
        for role in team.roles() {
            for agent in team.agents_for_role(&role) {
                self.agents.command_agent_for(&role, agent)?;
            }
        }
        let catalog = self.agents.config().catalog();
        self.validate_run_policies_run(run_id, &tasks, &team, &catalog, run.recipe.as_ref())?;
        Repo::detect_bounded(&self.root, &self.root)?;
        self.db.set_run_status(run_id, RunStatus::Active)?;
        Ok(team)
    }

    /// Pre-execution policy gate for starting (or retrying) a workflow. A task
    /// that cannot legally execute under *any* of its assigned agents' effective
    /// policies blocks the workflow with a useful reason, before any agent
    /// process is launched. Policy failures never consume task retries. The
    /// run's recipe snapshot narrows the effective policies when it carries
    /// policy presets for a role.
    fn validate_run_policies_run(
        &self,
        run_id: i64,
        tasks: &[Task],
        team: &WorkflowTeam,
        catalog: &RoleCatalog,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<(), FactoryError> {
        let mut blockers: Vec<String> = Vec::new();
        for task in tasks {
            let role = task.role.as_deref().unwrap_or(roles::WORKER);
            let operation = roles::resolve_task_operation(task, catalog);
            let agents = team.agents_for_role(role);
            if agents.is_empty() {
                continue;
            }
            let any_allowed = agents.iter().any(|agent| {
                let policy = self.policy_with_recipe(recipe, role, agent);
                factory_policy::validate_executable(&policy, operation.as_str()).is_ok()
            });
            if !any_allowed {
                let policy = self.role_policy_with_recipe(recipe, role);
                let reason = factory_policy::validate_executable(&policy, operation.as_str())
                    .err()
                    .unwrap_or_else(|| "no assigned agent allows this task".to_string());
                blockers.push(format!(
                    "task #{id} ({title}) with role '{role}' cannot perform operation \
                     '{op}': {reason} (allowed writes: {writes})",
                    id = task.id,
                    title = task.title,
                    op = operation.as_str(),
                    writes = describe_policy_scopes(self.agents.config(), role),
                ));
            }
        }
        if blockers.is_empty() {
            return Ok(());
        }
        self.db.set_run_status(run_id, RunStatus::Blocked)?;
        Err(FactoryError::PolicyBlocked(blockers.join("; ")))
    }

    /// Selects an agent from the pool that can legally execute the task under
    /// its own effective policy. Prefers the capacity-aware selection; when
    /// that agent is policy-blocked, the next pool candidate (round-robin from
    /// `index`) is tried. Returns `None` when no assigned agent can legally
    /// run the task.
    fn select_agent_for_task(
        &self,
        pool: &[String],
        index: usize,
        capacity: &AgentCapacity,
        role: &str,
        operation: TaskOperation,
        recipe: Option<&RecipeSnapshot>,
    ) -> Option<String> {
        if pool.is_empty() {
            return None;
        }
        let preferred = roles::select_agent_with_capacity(pool, index, capacity);
        if let Some(candidate) = preferred {
            let policy = self.policy_with_recipe(recipe, role, candidate);
            if factory_policy::validate_executable(&policy, operation.as_str()).is_ok() {
                return Some(candidate.clone());
            }
        }
        (0..pool.len()).find_map(|offset| {
            let candidate = &pool[(index + offset) % pool.len()];
            let policy = self.policy_with_recipe(recipe, role, candidate);
            factory_policy::validate_executable(&policy, operation.as_str())
                .ok()
                .map(|_| candidate.clone())
        })
    }

    /// Routes one dispatch to an agent. The mode comes from the run's recipe
    /// snapshot when it pins one, else the project `[routing]` configuration;
    /// the routing engine itself (candidate filtering, performance scoring,
    /// capacity, policies, overrides) stays authoritative either way.
    ///
    /// Eligibility is filtered before any scoring: a candidate must belong to
    /// the role's team pool, pass the Policy Engine for the operation, and be
    /// resolvable to an installed, automatically invocable agent. Performance
    /// then ranks only the eligible survivors; it can never resurrect an
    /// ineligible one. Capacity is reserved atomically (`try_acquire`) in
    /// performance mode so the Parallel Runtime can never oversubscribe an
    /// agent's `max_concurrency` between ranking and reservation.
    #[allow(clippy::too_many_arguments)]
    fn route_agent_for_attempt(
        &self,
        task: &Task,
        pool: &[String],
        role: &str,
        operation: TaskOperation,
        rotation_index: usize,
        language: Option<&str>,
        honor_override: bool,
    ) -> Result<RoutedAgent<'_>, FactoryError> {
        let routing_config = self.agents.config().routing;
        let recipe = self.run_recipe(task.run_id)?;
        let mode = recipe
            .as_ref()
            .map(|snapshot| snapshot.routing.resolve(routing_config.mode))
            .unwrap_or(routing_config.mode);
        let mode_str = mode.as_str().to_string();
        let preferred_agent = self
            .agents
            .config()
            .preferred_assignment(role)
            .map(|assignment| assignment.agent.clone());

        // A pinned agent short-circuits every mode. It still has to pass the
        // same eligibility gates; an invalid pin blocks the task with a clear
        // error instead of silently routing around the user's choice. The
        // built-in final review never honors a pin: the pin selects the
        // task's own role, not its reviewer.
        if let Some(pinned) = task.agent_override.clone().filter(|_| honor_override) {
            if !pool.contains(&pinned) {
                return Err(FactoryError::RoutingOverride(format!(
                    "task #{} pins agent '{}' which is not assigned to role '{}' in this \
                     workflow's team",
                    task.id, pinned, role
                )));
            }
            let policy = self.policy_with_recipe(recipe.as_ref(), role, &pinned);
            if let Err(reason) = factory_policy::validate_executable(&policy, operation.as_str()) {
                return Err(FactoryError::RoutingOverride(format!(
                    "task #{} pins agent '{}' which is policy-ineligible for operation '{}': {}",
                    task.id,
                    pinned,
                    operation.as_str(),
                    reason
                )));
            }
            let agent = self.agents.command_agent_for(role, &pinned)?;
            let name = agent.name().to_string();
            let guard = self.capacity.acquire(&name);
            let candidates = self.round_robin_notes(pool);
            return Ok(RoutedAgent {
                agent,
                _guard: guard,
                record: RoutingRecord {
                    mode: mode_str,
                    language: language.map(str::to_string),
                    candidates,
                    reason: routing::reasons::OVERRIDE.to_string(),
                },
            });
        }

        match mode {
            factory_types::RoutingMode::RoundRobin => self.route_round_robin(
                task,
                pool,
                role,
                operation,
                rotation_index,
                mode_str,
                language,
                routing::reasons::ROUND_ROBIN,
                recipe.as_ref(),
            ),
            factory_types::RoutingMode::Manual => self.route_manual(
                task,
                pool,
                role,
                operation,
                rotation_index,
                mode_str,
                language,
                preferred_agent,
                recipe.as_ref(),
            ),
            factory_types::RoutingMode::Performance => self.route_performance(
                task,
                pool,
                role,
                operation,
                rotation_index,
                mode_str,
                language,
                preferred_agent,
                routing_config,
                recipe.as_ref(),
            ),
        }
    }

    fn round_robin_notes(&self, pool: &[String]) -> Vec<RoutingCandidateScore> {
        pool.iter()
            .map(|agent| RoutingCandidateScore {
                agent: agent.clone(),
                score: None,
                reliable: false,
                note: "round-robin selection (no performance scoring)".to_string(),
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn route_round_robin(
        &self,
        task: &Task,
        pool: &[String],
        role: &str,
        operation: TaskOperation,
        rotation_index: usize,
        mode_str: String,
        language: Option<&str>,
        reason: &'static str,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<RoutedAgent<'_>, FactoryError> {
        let name = self
            .select_agent_for_task(
                pool,
                rotation_index,
                &self.capacity,
                role,
                operation,
                recipe,
            )
            .ok_or_else(|| self.no_executable_agent_error(task, role, pool, operation, recipe))?;
        let agent = self.agents.command_agent_for(role, &name)?;
        let name = agent.name().to_string();
        let guard = self.capacity.acquire(&name);
        Ok(RoutedAgent {
            agent,
            _guard: guard,
            record: RoutingRecord {
                mode: mode_str,
                language: language.map(str::to_string),
                candidates: self.round_robin_notes(pool),
                reason: reason.to_string(),
            },
        })
    }

    /// Manual mode: the role's preferred agent when it is eligible and has a
    /// free slot; otherwise the deterministic capacity-aware selection.
    #[allow(clippy::too_many_arguments)]
    fn route_manual(
        &self,
        task: &Task,
        pool: &[String],
        role: &str,
        operation: TaskOperation,
        rotation_index: usize,
        mode_str: String,
        language: Option<&str>,
        preferred_agent: Option<String>,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<RoutedAgent<'_>, FactoryError> {
        if let Some(preferred) = preferred_agent.filter(|agent| pool.contains(agent)) {
            let policy = self.policy_with_recipe(recipe, role, &preferred);
            if factory_policy::validate_executable(&policy, operation.as_str()).is_ok() {
                let limit = self.agent_concurrency_limit(&preferred);
                if let Some(guard) = self.capacity.try_acquire(&preferred, limit) {
                    // Resolving proves the agent is installed and invocable;
                    // a broken preferred agent falls through to the pool.
                    if let Ok(agent) = self.agents.command_agent_for(role, &preferred) {
                        return Ok(RoutedAgent {
                            agent,
                            _guard: guard,
                            record: RoutingRecord {
                                mode: mode_str,
                                language: language.map(str::to_string),
                                candidates: self.round_robin_notes(pool),
                                reason: routing::reasons::PREFERRED.to_string(),
                            },
                        });
                    }
                }
            }
        }
        self.route_round_robin(
            task,
            pool,
            role,
            operation,
            rotation_index,
            mode_str,
            language,
            routing::reasons::ROUND_ROBIN,
            recipe,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn route_performance(
        &self,
        task: &Task,
        pool: &[String],
        role: &str,
        operation: TaskOperation,
        rotation_index: usize,
        mode_str: String,
        language: Option<&str>,
        preferred_agent: Option<String>,
        routing_config: RoutingConfig,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<RoutedAgent<'_>, FactoryError> {
        // Agents with a failed or changes-requested attempt on this task get a
        // small deterministic retry penalty so retries may route elsewhere.
        let prior_rejections: std::collections::HashSet<String> = self
            .db
            .list_task_attempts_for_task(task.id)?
            .iter()
            .filter(|attempt| {
                matches!(
                    attempt.status,
                    AttemptStatus::Failed | AttemptStatus::ChangesRequested
                )
            })
            .map(|attempt| attempt.agent.clone())
            .collect();
        let now = Utc::now();
        // Candidate filtering comes first: policy eligibility and automated
        // resolvability decide who may be ranked at all.
        let mut facts: Vec<CandidateFacts> = Vec::new();
        let mut first_resolution_error: Option<FactoryError> = None;
        for agent in pool {
            let policy = self.policy_with_recipe(recipe, role, agent);
            if factory_policy::validate_executable(&policy, operation.as_str()).is_err() {
                continue;
            }
            if let Err(error) = self.agents.command_agent_for(role, agent) {
                first_resolution_error.get_or_insert(FactoryError::from(error));
                continue;
            }
            let performance = factory_eval::resolve_performance(
                &self.db,
                agent,
                Some(role),
                Some(operation),
                language,
                now,
            )?;
            facts.push(CandidateFacts {
                agent: agent.clone(),
                preferred: preferred_agent.as_deref() == Some(agent.as_str()),
                performance,
                observed_tasks: self.db.count_agent_attempts(agent)?,
                inflight: self.capacity.inflight(agent),
                limit: self.agent_concurrency_limit(agent),
                prior_rejection: prior_rejections.contains(agent),
            });
        }
        if facts.is_empty() {
            if let Some(error) = first_resolution_error {
                return Err(error);
            }
            // Nobody survived filtering; surface the same policy diagnostics
            // the pre-start gate produces.
            return Err(self.no_executable_agent_error(task, role, pool, operation, recipe));
        }

        let ranked = routing::rank(&facts);

        // Deterministic cold-start exploration: every Nth dispatch goes to
        // the least-observed unranked candidate so it can gather evidence.
        if let Some(explorer) = routing::exploration_pick(
            &facts,
            &ranked,
            factory_types::RoutingMode::Performance,
            routing_config.exploration,
            rotation_index,
        ) {
            let limit = self.agent_concurrency_limit(&explorer);
            if let Some(guard) = self.capacity.try_acquire(&explorer, limit) {
                let agent = self.agents.command_agent_for(role, &explorer)?;
                return Ok(RoutedAgent {
                    agent,
                    _guard: guard,
                    record: RoutingRecord {
                        mode: mode_str,
                        language: language.map(str::to_string),
                        candidates: routing::candidate_scores(&ranked),
                        reason: routing::reasons::EXPLORATION.to_string(),
                    },
                });
            }
        }

        // Reserve the best reliable candidate that currently has capacity.
        // try_acquire re-checks the limit under the capacity lock, so a
        // ranking computed from a load snapshot is always guarded by the
        // final reservation.
        let any_reliable = ranked.iter().any(|candidate| candidate.reliable);
        if any_reliable {
            for candidate in ranked.iter().filter(|c| c.reliable && c.free_slots > 0) {
                let limit = self.agent_concurrency_limit(&candidate.agent);
                if let Some(guard) = self.capacity.try_acquire(&candidate.agent, limit) {
                    let agent = self.agents.command_agent_for(role, &candidate.agent)?;
                    return Ok(RoutedAgent {
                        agent,
                        _guard: guard,
                        record: RoutingRecord {
                            mode: mode_str,
                            language: language.map(str::to_string),
                            candidates: routing::candidate_scores(&ranked),
                            reason: routing::reasons::SCORED.to_string(),
                        },
                    });
                }
            }
            // Every reliable candidate is saturated (or lost a reservation
            // race): keep the run moving on the deterministic fallback
            // instead of queueing forever behind the historical favorite.
            return self.route_round_robin(
                task,
                pool,
                role,
                operation,
                rotation_index,
                mode_str,
                language,
                routing::reasons::ALL_SATURATED,
                recipe,
            );
        }

        // No candidate has reliable performance data: never rank from thin
        // samples, fall back to the existing deterministic routing.
        self.route_round_robin(
            task,
            pool,
            role,
            operation,
            rotation_index,
            mode_str,
            language,
            routing::reasons::FALLBACK,
            recipe,
        )
    }

    fn agent_concurrency_limit(&self, agent: &str) -> u64 {
        self.agents
            .config()
            .agents
            .get(agent)
            .map(|entry| entry.concurrency() as u64)
            .unwrap_or(1)
    }

    /// Deterministic language hint for routing: the language of the files the
    /// task's previous attempts changed (retry attempts), when exactly one
    /// language is observed. Fresh tasks have no evidence yet and route on
    /// role+operation alone.
    fn routing_language(&self, task_id: i64) -> Option<String> {
        let attempt = self.db.latest_task_attempt(task_id).ok()??;
        let evidence_files = attempt
            .evidence
            .map(|evidence| evidence.changed_files)
            .unwrap_or_default();
        let languages = factory_eval::detect_languages(&evidence_files);
        let sole = languages.iter().next().cloned();
        (languages.len() == 1).then_some(sole).flatten()
    }

    fn record_routing_decision(
        &self,
        task_id: i64,
        attempt_id: Option<i64>,
        role: &str,
        operation: TaskOperation,
        routed: &RoutingRecord,
        selected_agent: &str,
    ) -> Result<(), FactoryError> {
        self.db.insert_routing_decision(&RoutingDecision {
            id: 0,
            task_id,
            attempt_id,
            mode: routed.mode.clone(),
            selected_agent: selected_agent.to_string(),
            role: Some(role.to_string()),
            operation: Some(operation),
            language: routed.language.clone(),
            candidate_scores: routed.candidates.clone(),
            reason: routed.reason.clone(),
            created_at: Utc::now().to_rfc3339(),
        })?;
        Ok(())
    }

    /// Read-only preview of what the router would do for a task right now.
    /// Informational only — the real selection happens at dispatch time when
    /// capacity is reserved.
    pub fn routing_preview(&self, task_id: i64) -> Result<RoutingPreview, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let run = self
            .db
            .get_run(task.run_id)?
            .ok_or(FactoryError::RunNotFound(task.run_id))?;
        let team = self.resolve_team(&run)?;
        let catalog = self.agents.config().catalog();
        let operation = roles::resolve_task_operation(&task, &catalog);
        let role = task
            .role
            .clone()
            .unwrap_or_else(|| roles::WORKER.to_string());
        let pool = team.agents_for_role(&role).to_vec();
        let language = self.routing_language(task.id);
        let routing_config = self.agents.config().routing;
        // The run's recipe may pin the routing mode; project_default keeps
        // whatever the project configured.
        let mode = run
            .recipe
            .as_ref()
            .map(|snapshot| snapshot.routing.resolve(routing_config.mode))
            .unwrap_or(routing_config.mode);
        let recipe = run.recipe.as_ref();
        let now = Utc::now();
        let preferred_agent = self
            .agents
            .config()
            .preferred_assignment(&role)
            .map(|assignment| assignment.agent.clone());

        let mut candidates: Vec<RoutingCandidateScore> = Vec::new();
        let mut best: Option<(String, Option<f64>)> = None;
        let mut eligible: Vec<CandidateFacts> = Vec::new();
        for agent in &pool {
            let policy = self.policy_with_recipe(recipe, &role, agent);
            if factory_policy::validate_executable(&policy, operation.as_str()).is_err() {
                candidates.push(RoutingCandidateScore {
                    agent: agent.clone(),
                    score: None,
                    reliable: false,
                    note: "policy-ineligible for this operation".to_string(),
                });
                continue;
            }
            let performance = if mode == factory_types::RoutingMode::Performance {
                factory_eval::resolve_performance(
                    &self.db,
                    agent,
                    Some(&role),
                    Some(operation),
                    language.as_deref(),
                    now,
                )?
            } else {
                None
            };
            let facts = CandidateFacts {
                agent: agent.clone(),
                preferred: preferred_agent.as_deref() == Some(agent.as_str()),
                performance,
                observed_tasks: self.db.count_agent_attempts(agent)?,
                inflight: self.capacity.inflight(agent),
                limit: self.agent_concurrency_limit(agent),
                prior_rejection: false,
            };
            eligible.push(facts);
        }
        if !eligible.is_empty() {
            let ranked = routing::rank(&eligible);
            let pick = routing::exploration_pick(
                &eligible,
                &ranked,
                mode,
                routing_config.exploration,
                self.db.count_task_attempts(task.run_id)?,
            );
            if let Some(explorer) = pick {
                best = Some((explorer, None));
            } else {
                best = ranked
                    .iter()
                    .find(|candidate| candidate.reliable && candidate.free_slots > 0)
                    .or_else(|| ranked.iter().find(|candidate| candidate.reliable))
                    .map(|candidate| (candidate.agent.clone(), candidate.score));
            }
            candidates.extend(routing::candidate_scores(&ranked));
        }
        if candidates.is_empty() {
            candidates = self.round_robin_notes(&pool);
        }

        let reason = match (&task.agent_override, mode, &best) {
            (Some(_), _, _) => routing::reasons::OVERRIDE.to_string(),
            (None, factory_types::RoutingMode::Manual, _) => {
                routing::reasons::PREFERRED.to_string()
            }
            (None, factory_types::RoutingMode::RoundRobin, _) => {
                routing::reasons::ROUND_ROBIN.to_string()
            }
            (None, factory_types::RoutingMode::Performance, Some((agent, Some(_)))) => {
                let _ = agent;
                routing::reasons::SCORED.to_string()
            }
            (None, factory_types::RoutingMode::Performance, Some((_, None))) => {
                routing::reasons::EXPLORATION.to_string()
            }
            (None, factory_types::RoutingMode::Performance, None) => {
                routing::reasons::FALLBACK.to_string()
            }
        };

        Ok(RoutingPreview {
            mode: mode.as_str().to_string(),
            task_id: task.id,
            role: Some(role),
            operation: Some(operation),
            language,
            override_agent: task.agent_override.clone(),
            likely_agent: best.map(|(agent, _)| agent),
            reason,
            candidates,
        })
    }

    /// Post-hoc evidence enforcement: files changed by an attempt must fall
    /// inside the session's effective write scopes, and reported commands
    /// inside the command policy. Legacy (policy-less) sessions are never
    /// affected: only explicitly configured restrictions are enforced. The
    /// run's recipe snapshot narrows the scopes when it carries a preset for
    /// the role.
    fn enforce_evidence_policy(
        &self,
        recipe: Option<&RecipeSnapshot>,
        role: &str,
        agent: &str,
        evidence: &TaskEvidence,
    ) -> Result<(), FactoryError> {
        let policy = self.policy_with_recipe(recipe, role, agent);
        if policy.permissive {
            return Ok(());
        }
        let write_violations = policy.filesystem.write_violations(&evidence.changed_files);
        if !write_violations.is_empty() {
            return Err(FactoryError::PolicyViolation(format!(
                "the {role} agent changed files outside the allowed write scopes: {} \
                 (allowed writes: {})",
                write_violations.join(", "),
                describe_policy_scopes(self.agents.config(), role)
            )));
        }
        let command_violations = policy.commands.violations(&evidence.commands);
        if !command_violations.is_empty() {
            return Err(FactoryError::PolicyViolation(format!(
                "the {role} agent reported commands outside the command policy: {}",
                command_violations.join(", ")
            )));
        }
        Ok(())
    }

    /// Error for a task whose assigned agents are all policy-blocked for the
    /// operation (or missing entirely).
    fn no_executable_agent_error(
        &self,
        task: &Task,
        role: &str,
        pool: &[String],
        operation: TaskOperation,
        recipe: Option<&RecipeSnapshot>,
    ) -> FactoryError {
        if pool.is_empty() {
            return FactoryError::TaskRoleUnavailable(task.id, role.to_string());
        }
        let policy = self.role_policy_with_recipe(recipe, role);
        let reason = factory_policy::validate_executable(&policy, operation.as_str())
            .err()
            .unwrap_or_else(|| "no assigned agent allows this task".to_string());
        FactoryError::PolicyBlocked(format!(
            "task #{id} ({title}) with role '{role}' cannot perform operation '{op}' \
             under any assigned agent: {reason} (allowed writes: {writes})",
            id = task.id,
            title = task.title,
            op = operation.as_str(),
            writes = describe_policy_scopes(self.agents.config(), role),
        ))
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
        // Re-check the policy gate: a retry must never start a task that still
        // cannot legally execute (no retry is consumed by a policy block).
        let tasks = self.db.list_tasks(task.run_id)?;
        let catalog = self.agents.config().catalog();
        self.validate_run_policies_run(task.run_id, &tasks, &team, &catalog, run.recipe.as_ref())?;
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
            let RoutedAgent {
                agent: worker,
                _guard: _worker_load,
                record: worker_routing,
            } = self.route_agent_for_attempt(
                &task,
                &worker_pool,
                &role,
                operation,
                worker_index,
                self.routing_language(task_id).as_deref(),
                true,
            )?;
            let worker_name = worker.name().to_string();
            let attempt = self.db.create_task_attempt(
                task_id,
                &role,
                Some(operation),
                worker.name(),
                &worktree.to_string_lossy(),
                Some(&base_sha),
            )?;
            self.record_routing_decision(
                task_id,
                Some(attempt.id),
                &role,
                operation,
                &worker_routing,
                &worker_name,
            )?;
            self.mark_task(task_id, TaskState::Running)?;

            let upstream = self.upstream_artifacts(&task)?;
            let repository_context =
                self.repository_context(&task, operation, &worktree, &base_sha, &[], &upstream);
            let untrusted_notice = self.untrusted_issue_notice(task.run_id)?;
            let worker_policy = self.policy_with_recipe(run.recipe.as_ref(), &role, &worker_name);
            let worker_instruction = build_mission(&MissionContext {
                role: role_definition,
                operation,
                task: &task,
                run_objective: &run.objective,
                untrusted_context: untrusted_notice.as_deref(),
                upstream_artifacts: &upstream,
                repository_context: Some(&repository_context),
                previous_feedback: previous_feedback.as_ref(),
                review_input: None,
                final_review: false,
                policy: Some(&worker_policy),
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
            let worker_run = match worker_run {
                Ok(run) => {
                    if let Some(error) = agent_output_limit_error(&role, &run.result) {
                        return Err(self.record_output_limit_failure(
                            run.session_id,
                            attempt.id,
                            task_id,
                            error,
                            run.result.exit_code,
                            None,
                            None,
                        )?);
                    }
                    Ok(run)
                }
                Err(error) => Err(error),
            };
            let mut evidence = collect_evidence(
                &repo,
                &worktree,
                &base_sha,
                &task,
                worker_run.as_ref().ok().map(|run| &run.result),
            )?;

            // Post-hoc policy enforcement: any file written outside the
            // session's effective write scopes (or a denied command) fails the
            // attempt without consuming a normal retry.
            if let Err(policy_error) =
                self.enforce_evidence_policy(run.recipe.as_ref(), &role, &worker_name, &evidence)
            {
                self.db.finish_task_attempt(
                    attempt.id,
                    AttemptStatus::Failed,
                    worker_run
                        .as_ref()
                        .ok()
                        .and_then(|run| run.result.exit_code),
                    evidence.commit_sha.as_deref(),
                    Some(&format!("blocked by policy: {policy_error}")),
                    Some(&evidence),
                    None,
                )?;
                self.mark_task(task_id, TaskState::Failed)?;
                return Err(policy_error);
            }

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
            let RoutedAgent {
                agent: reviewer,
                _guard: _reviewer_load,
                record: reviewer_routing,
            } = self.route_agent_for_attempt(
                &task,
                &team.reviewers,
                roles::REVIEWER,
                TaskOperation::Review,
                attempt.attempt_number.saturating_sub(1) as usize,
                None,
                false,
            )?;
            let reviewer_name = reviewer.name().to_string();
            self.record_routing_decision(
                task_id,
                Some(attempt.id),
                roles::REVIEWER,
                TaskOperation::Review,
                &reviewer_routing,
                &reviewer_name,
            )?;
            let reviewer_role = catalog
                .get(roles::REVIEWER)
                .cloned()
                .unwrap_or_else(|| roles::core_role(roles::REVIEWER).expect("core reviewer"));
            let reviewer_policy =
                self.policy_with_recipe(run.recipe.as_ref(), roles::REVIEWER, &reviewer_name);
            let review_instruction = build_mission(&MissionContext {
                role: &reviewer_role,
                operation: TaskOperation::Review,
                task: &task,
                run_objective: &run.objective,
                untrusted_context: untrusted_notice.as_deref(),
                upstream_artifacts: &upstream,
                repository_context: Some(&repository_context),
                previous_feedback: None,
                review_input: Some(&ReviewInput {
                    producer_title: task.title.clone(),
                    producer_role: role_definition.name.clone(),
                    evidence: evidence.clone(),
                    producer_output: worker_run.result.stdout.clone(),
                    diff: evidence.diff_patch.clone().unwrap_or_default(),
                }),
                final_review: true,
                policy: Some(&reviewer_policy),
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
                    if let Some(error) = agent_output_limit_error("reviewer", &run.result) {
                        return Err(self.record_output_limit_failure(
                            run.session_id,
                            attempt.id,
                            task_id,
                            error,
                            run.result.exit_code,
                            Some(&evidence),
                            evidence.commit_sha.as_deref(),
                        )?);
                    }
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
                    self.integrate_approved_task(&run, &task, attempt.id, &worker_name, &worktree)?;
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
            let RoutedAgent {
                agent,
                _guard: _advisory_load,
                record: advisory_routing,
            } = self.route_agent_for_attempt(
                &task,
                &pool,
                &role,
                TaskOperation::Advisory,
                index,
                self.routing_language(task_id).as_deref(),
                true,
            )?;
            let agent_name = agent.name().to_string();
            let attempt = self.db.create_task_attempt(
                task_id,
                &role,
                Some(TaskOperation::Advisory),
                agent.name(),
                &worktree.to_string_lossy(),
                Some(&base_sha),
            )?;
            self.record_routing_decision(
                task_id,
                Some(attempt.id),
                &role,
                TaskOperation::Advisory,
                &advisory_routing,
                &agent_name,
            )?;
            self.mark_task(task_id, TaskState::Running)?;

            let upstream = self.upstream_artifacts(&task)?;
            let repository_context = self.repository_context(
                &task,
                TaskOperation::Advisory,
                &worktree,
                &base_sha,
                &[],
                &upstream,
            );
            let advisory_policy = self.policy_with_recipe(run.recipe.as_ref(), &role, &agent_name);
            let untrusted_notice = self.untrusted_issue_notice(task.run_id)?;
            let instruction = build_mission(&MissionContext {
                role: role_definition,
                operation: TaskOperation::Advisory,
                task: &task,
                run_objective: &run.objective,
                untrusted_context: untrusted_notice.as_deref(),
                upstream_artifacts: &upstream,
                repository_context: Some(&repository_context),
                previous_feedback: None,
                review_input: None,
                final_review: false,
                policy: Some(&advisory_policy),
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
            let agent_run = match agent_run {
                Ok(run) => {
                    if let Some(error) = agent_output_limit_error(&role, &run.result) {
                        return Err(self.record_output_limit_failure(
                            run.session_id,
                            attempt.id,
                            task_id,
                            error,
                            run.result.exit_code,
                            None,
                            None,
                        )?);
                    }
                    Ok(run)
                }
                Err(error) => Err(error),
            };
            let evidence = collect_evidence(
                &repo,
                &worktree,
                &base_sha,
                &task,
                agent_run.as_ref().ok().map(|run| &run.result),
            )?;

            if let Err(policy_error) =
                self.enforce_evidence_policy(run.recipe.as_ref(), &role, &agent_name, &evidence)
            {
                self.db.finish_task_attempt(
                    attempt.id,
                    AttemptStatus::Failed,
                    agent_run.as_ref().ok().and_then(|run| run.result.exit_code),
                    evidence.commit_sha.as_deref(),
                    Some(&format!("blocked by policy: {policy_error}")),
                    Some(&evidence),
                    None,
                )?;
                self.mark_task(task_id, TaskState::Failed)?;
                return Err(policy_error);
            }

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
        let RoutedAgent {
            agent: reviewer,
            _guard: _reviewer_load,
            record: reviewer_routing,
        } = self.route_agent_for_attempt(
            &task,
            &pool,
            &role,
            TaskOperation::Review,
            index,
            self.routing_language(task_id).as_deref(),
            true,
        )?;
        let reviewer_name = reviewer.name().to_string();

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
                Some(&base_sha),
            )?;
            self.record_routing_decision(
                task_id,
                Some(attempt.id),
                &role,
                TaskOperation::Review,
                &reviewer_routing,
                &reviewer_name,
            )?;
            self.mark_task(task_id, TaskState::Running)?;

            let run_tasks = self.db.list_tasks(task.run_id)?;
            let upstream = self.upstream_artifacts(&task)?;
            let review_input = self.build_review_input(&task, &run_tasks, catalog)?;
            let repository_context = self.repository_context(
                &task,
                TaskOperation::Review,
                &worktree,
                &base_sha,
                &review_input.evidence.changed_files,
                &upstream,
            );
            let review_policy = self.policy_with_recipe(run.recipe.as_ref(), &role, &reviewer_name);
            let untrusted_notice = self.untrusted_issue_notice(task.run_id)?;
            let instruction = build_mission(&MissionContext {
                role: role_definition,
                operation: TaskOperation::Review,
                task: &task,
                run_objective: &run.objective,
                untrusted_context: untrusted_notice.as_deref(),
                upstream_artifacts: &upstream,
                repository_context: Some(&repository_context),
                previous_feedback: None,
                review_input: Some(&review_input),
                final_review: false,
                policy: Some(&review_policy),
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
            let review_run = match review_run {
                Ok(run) => {
                    if let Some(error) = agent_output_limit_error(&role, &run.result) {
                        return Err(self.record_output_limit_failure(
                            run.session_id,
                            attempt.id,
                            task_id,
                            error,
                            run.result.exit_code,
                            None,
                            None,
                        )?);
                    }
                    Ok(run)
                }
                Err(error) => Err(error),
            };
            let evidence = collect_evidence(
                &repo,
                &worktree,
                &base_sha,
                &task,
                review_run.as_ref().ok().map(|run| &run.result),
            )?;
            // A review role may only inspect: any write outside the policy is
            // a violation, and review roles never commit production work.
            if let Err(policy_error) =
                self.enforce_evidence_policy(run.recipe.as_ref(), &role, &reviewer_name, &evidence)
            {
                self.db.finish_task_attempt(
                    attempt.id,
                    AttemptStatus::Failed,
                    review_run
                        .as_ref()
                        .ok()
                        .and_then(|run| run.result.exit_code),
                    evidence.commit_sha.as_deref(),
                    Some(&format!("blocked by policy: {policy_error}")),
                    Some(&evidence),
                    None,
                )?;
                self.mark_task(task_id, TaskState::Failed)?;
                return Err(policy_error);
            }
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

    #[allow(clippy::too_many_arguments)]
    fn record_output_limit_failure(
        &self,
        session_id: i64,
        attempt_id: i64,
        task_id: i64,
        error: FactoryError,
        exit_code: Option<i32>,
        evidence: Option<&TaskEvidence>,
        commit_sha: Option<&str>,
    ) -> Result<FactoryError, FactoryError> {
        self.db.set_agent_session_status(session_id, "rejected")?;
        self.db.finish_task_attempt(
            attempt_id,
            AttemptStatus::Failed,
            exit_code,
            commit_sha,
            Some(&error.to_string()),
            evidence,
            None,
        )?;
        self.mark_task(task_id, TaskState::Failed)?;
        Ok(error)
    }

    fn invoke_with_agent(
        &self,
        agent: CommandAgent,
        scope: InvocationScope<'_>,
        mission: &str,
        cancel: &AtomicBool,
    ) -> Result<Invocation, FactoryError> {
        let started = chrono::Utc::now().to_rfc3339();
        // Resolve the one effective policy for this session (baseline → role →
        // agent, narrowed by the run's recipe preset when one applies). The
        // same policy drives environment filtering, audit, and the post-hoc
        // evidence checks; there is no second permission model.
        let recipe = scope
            .run_id
            .and_then(|run_id| self.db.get_run(run_id).ok().flatten())
            .and_then(|run| run.recipe);
        let policy = self.policy_with_recipe(recipe.as_ref(), scope.role, agent.name());
        let external_tools = self.external_tools_for_invocation(&scope, &policy);
        let policy_audit = factory_types::SessionPolicyAudit {
            source: policy.source.clone(),
            filesystem: policy.filesystem.mode_name().to_string(),
            network: if policy.network.allowed() {
                "allow"
            } else {
                "deny"
            }
            .to_string(),
            environment: policy.environment.mode().to_string(),
            write_scopes: policy.filesystem.effective_write_scopes(),
            tool_allow: policy.tools.allow_patterns(),
            tool_deny: policy.tools.deny_patterns().to_vec(),
        };
        // Environment filtering before process launch: when the policy filters
        // or denies variables, the child receives exactly the computed set
        // instead of Factory's whole environment. Denied values are withheld
        // and treated as secrets that must never reach logs.
        let needs_clear = policy.environment.filtered || !policy.environment.denied.is_empty();
        let mut secrets =
            factory_policy::secrets_from_denied(std::env::vars(), &policy.environment.denied);
        for value in agent.config().env.values() {
            secrets.add(value, 4);
        }
        let session = self.db.insert_agent_session(&AgentSession {
            id: 0,
            run_id: scope.run_id,
            task_id: scope.task_id,
            attempt_id: scope.attempt_id,
            role: scope.role.to_string(),
            operation: scope.operation,
            agent: agent.name().to_string(),
            mode: AgentSessionMode::Automated,
            command: agent.invocation_summary(),
            status: "running".to_string(),
            started_at: started,
            finished_at: None,
            exit_code: None,
            duration_ms: None,
            stdout: Some(String::new()),
            stderr: Some(String::new()),
            policy_audit: Some(policy_audit),
            output_meta: None,
        })?;
        let registry = ToolRegistry::new(&self.root, self.agents.config().mcp.clone());
        let grant = registry.create_grant(
            GrantContext {
                run_id: scope.run_id,
                task_id: scope.task_id,
                attempt_id: scope.attempt_id,
                session_id: Some(session.id),
                agent: agent.name().to_string(),
                role: scope.role.to_string(),
                operation: scope
                    .operation
                    .map(|operation| operation.as_str().to_string()),
            },
            &external_tools,
        )?;
        let mission = grant
            .as_ref()
            .map(|_| inject_external_tool_catalog(mission, &external_tools))
            .unwrap_or_else(|| mission.to_string());
        let mut request = AgentRequest::new(mission, scope.working_dir);
        if needs_clear {
            request.env = policy.environment.environment(std::env::vars());
            request.env_deny = policy.environment.denied.clone();
            request.clear_env = true;
        }
        if let Some(grant) = &grant {
            secrets.add(&grant.token, 4);
            request.env.insert(
                "FACTORY_MCP_ROOT".into(),
                self.root.to_string_lossy().to_string(),
            );
            request
                .env
                .insert("FACTORY_MCP_GRANT".into(), grant.token.clone());
        }
        let timer = Instant::now();
        let mut output_error = None;
        let result = agent.run_observed(&request, cancel, |stream, chunk| {
            if output_error.is_some() {
                return;
            }
            let chunk = secrets.redact(chunk);
            let update = match stream {
                OutputStream::Stdout => {
                    self.db
                        .append_agent_session_output(session.id, Some(&chunk), None)
                }
                OutputStream::Stderr => {
                    self.db
                        .append_agent_session_output(session.id, None, Some(&chunk))
                }
            };
            if let Err(error) = update {
                output_error = Some(error);
            }
        });
        if let Some(grant) = &grant {
            // Revocation happens before any task state transition. If the
            // agent was cancelled, its active MCP subprocess is terminated by
            // the same process-tree cancellation and this closes reuse.
            registry.revoke_grant(&grant.id)?;
        }
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
                let stdout = secrets.redact(&result.stdout);
                let stderr = secrets.redact(&result.stderr);
                self.db
                    .set_agent_session_output(session.id, Some(&stdout), Some(&stderr))?;
                self.db
                    .set_agent_session_output_metadata(session.id, Some(&result.output_metadata))?;
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

    fn external_tools_for_invocation(
        &self,
        scope: &InvocationScope<'_>,
        policy: &factory_policy::EffectivePolicy,
    ) -> Vec<McpToolMetadata> {
        if self.agents.config().mcp.is_empty()
            || policy.tools.allow_patterns().is_empty()
            || policy
                .environment
                .denied
                .iter()
                .any(|key| key.eq_ignore_ascii_case("FACTORY_MCP_GRANT"))
        {
            return Vec::new();
        }
        let registry = ToolRegistry::new(&self.root, self.agents.config().mcp.clone());
        let cached = registry.cached_tools(None).unwrap_or_default();
        for server in self.agents.config().mcp.keys() {
            if policy.tools.may_allow_server(server)
                && !cached.iter().any(|tool| &tool.server == server)
            {
                // External tooling is optional infrastructure. An unavailable
                // server removes its tools from this catalog without failing
                // the task or becoming an agent-quality failure.
                let _ = registry.discover_server_blocking(server);
            }
        }
        let objective = scope
            .task_id
            .and_then(|id| self.db.get_task(id).ok().flatten())
            .map(|task| format!("{} {}", task.title, task.objective))
            .or_else(|| {
                scope
                    .run_id
                    .and_then(|id| self.db.get_run(id).ok().flatten())
                    .map(|run| run.objective)
            })
            .unwrap_or_default();
        registry
            .relevant_cached_tools(
                policy,
                scope.role,
                scope
                    .operation
                    .map(TaskOperation::as_str)
                    .unwrap_or("unknown"),
                &objective,
            )
            .unwrap_or_default()
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
    /// Every integration attempt (clean fast-forward, stale-base rebase, or
    /// rebase conflict) is recorded as a durable `integration_outcomes` row so
    /// the evaluation engine can measure integration quality. A conflict is
    /// still surfaced as an error exactly as before.
    ///
    /// Returns the new integration head, or `None` when the task introduced no
    /// commits (nothing was integrated).
    fn integrate_approved_task(
        &self,
        run: &Run,
        task: &Task,
        attempt_id: i64,
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
            if let Err(error) = repo.rebase_onto_in(worktree, &run_branch) {
                self.db.record_integration_outcome(
                    run.id,
                    task.id,
                    attempt_id,
                    agent_name,
                    factory_types::IntegrationOutcomeKind::Conflict,
                )?;
                return Err(error.into());
            }
            let rebased = repo.head_sha(worktree)?;
            repo.update_ref(&run_branch, &rebased)?;
            self.db.set_run_integration(run.id, Some(&rebased))?;
            self.db.record_integration_outcome(
                run.id,
                task.id,
                attempt_id,
                agent_name,
                factory_types::IntegrationOutcomeKind::Rebased,
            )?;
            return Ok(Some(rebased));
        }
        repo.update_ref(&run_branch, &head)?;
        self.db.set_run_integration(run.id, Some(&head))?;
        self.db.record_integration_outcome(
            run.id,
            task.id,
            attempt_id,
            agent_name,
            factory_types::IntegrationOutcomeKind::Clean,
        )?;
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

    // --- GitHub linkage and delivery ----------------------------------------

    /// The notice marking a run's objective as containing untrusted imported
    /// Issue content, rendered into Planner and task missions.
    fn untrusted_issue_notice(&self, run_id: i64) -> Result<Option<String>, FactoryError> {
        Ok(self
            .db
            .get_run_github_link(run_id)?
            .map(|link| crate::github::untrusted_issue_notice(&link)))
    }

    /// The GitHub Issue a run was imported from, when it has one.
    pub fn github_link(&self, run_id: i64) -> Result<Option<GitHubIssueLink>, FactoryError> {
        Ok(self.db.get_run_github_link(run_id)?)
    }

    /// `gh auth status` + remote detection for the dashboard's connection
    /// display. Tokens are never read or displayed.
    pub fn github_status(&self) -> crate::github::GitHubStatus {
        let mut status = crate::github::GitHubStatus {
            connected: false,
            user: None,
            auth_error: None,
            remote_error: None,
            repository: None,
        };
        match self.github_remote() {
            Ok(remote) => {
                status.repository = Some(crate::github::GitHubRepoStatus {
                    repository: remote.repository.clone(),
                    remote: remote.remote.clone(),
                    url: remote.web_url(),
                    default_branch: remote.default_branch.clone(),
                });
            }
            Err(error) => status.remote_error = Some(error.to_string()),
        }
        match factory_github::GhCli::discovered().auth_status() {
            Ok(auth) => {
                status.connected = true;
                status.user = auth.user;
            }
            Err(error) => status.auth_error = Some(error.to_string()),
        }
        status
    }

    /// The project's GitHub remote, resolved only from Git remotes.
    pub fn github_remote(&self) -> Result<factory_github::GitHubRemote, FactoryError> {
        Repo::detect_bounded(&self.root, &self.root)?;
        Ok(factory_github::remote::detect(&self.root)?)
    }

    /// Imports a GitHub Issue as a new workflow. The issue becomes the run's
    /// objective (bounded, verbatim) and is persisted as an untrusted link;
    /// nothing executes until the user starts the workflow. Authentication
    /// and repository resolution failures return actionable errors. Recipes
    /// apply exactly like for objective-created workflows — same catalog,
    /// same snapshot, no separate GitHub recipe system.
    pub fn import_github_issue(
        &self,
        reference: &str,
        team: Option<WorkflowTeam>,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<Run, FactoryError> {
        let issue_ref = factory_github::IssueRef::parse(reference)?;
        let remote = self.github_remote()?;
        if let Some(repository) = &issue_ref.repository {
            if *repository != remote.repository {
                return Err(factory_github::GitHubError::RemoteParse(format!(
                    "the issue belongs to '{repository}' but this project tracks '{}'; Factory \
                     only imports issues from the project's own GitHub remote",
                    remote.repository
                ))
                .into());
            }
        }
        let gh = factory_github::GhCli::discovered();
        gh.auth_status()?;
        let issue = gh.view_issue(&remote.repository, issue_ref.number)?;
        let objective = factory_github::objective_from_issue(&issue);
        let run = self.begin_run_with_recipe(&objective, team, recipe)?;
        let link = factory_github::issue_link(
            &issue,
            &remote.repository,
            &remote.issue_url(issue.number),
            &chrono::Utc::now().to_rfc3339(),
        );
        self.db.set_run_github_link(run.id, &link)?;
        self.db
            .get_run(run.id)?
            .ok_or(FactoryError::RunNotFound(run.id))
    }

    /// The delivery report for a run: persisted metadata, effective state,
    /// and eligibility with concrete blockers.
    pub fn delivery_report(
        &self,
        run_id: i64,
    ) -> Result<crate::github::DeliveryReport, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        let link = self.db.get_run_github_link(run_id)?;
        let delivery = self.db.get_or_create_delivery(run_id)?;
        let integration_head = self.db.get_run_integration(run_id)?;
        let local_head = Repo::detect_bounded(&self.root, &self.root)
            .ok()
            .and_then(|repo| {
                repo.resolve_ref(&delivery.head_branch)
                    .ok()
                    .filter(|sha| !sha.is_empty())
            });
        let eligibility = crate::github::delivery_eligibility(
            &run,
            integration_head.as_deref(),
            local_head.as_deref(),
        );
        let repository = self
            .github_remote()
            .ok()
            .map(|remote| crate::github::GitHubRepoStatus {
                repository: remote.repository.clone(),
                remote: remote.remote.clone(),
                url: remote.web_url(),
                default_branch: remote.default_branch.clone(),
            });
        Ok(crate::github::DeliveryReport {
            run_id,
            state: crate::github::effective_delivery_state(&delivery, eligibility.ready),
            persisted_state: delivery.state,
            link,
            repository,
            base_branch: delivery
                .base_branch
                .clone()
                .or_else(|| self.github_remote().ok().and_then(|r| r.default_branch)),
            head_branch: delivery.head_branch.clone(),
            integration_head,
            local_head,
            pushed_head: delivery.pushed_head,
            pull_request: delivery.pull_request,
            error: delivery.error,
            eligible: eligibility.ready,
            blockers: eligibility.blockers,
        })
    }

    /// The editable pull request preview for a completed run.
    pub fn pull_request_preview(
        &self,
        run_id: i64,
    ) -> Result<crate::github::PrPreview, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        let link = self.db.get_run_github_link(run_id)?;
        let delivery = self.db.get_or_create_delivery(run_id)?;
        let integration_head = self.db.get_run_integration(run_id)?;
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        let local_head = repo
            .resolve_ref(&delivery.head_branch)
            .ok()
            .filter(|sha| !sha.is_empty());
        let eligibility = crate::github::delivery_eligibility(
            &run,
            integration_head.as_deref(),
            local_head.as_deref(),
        );
        let remote = self.github_remote()?;
        let base = remote
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".to_string());
        let tasks = self.db.list_tasks(run_id)?;
        let attempts = self.db.list_task_attempts(run_id)?;
        let mut evidence = crate::github::pr_evidence(&run, &tasks, &attempts);
        evidence.issue_number = link.as_ref().map(|link| link.issue_number);
        let issue_title = link.as_ref().map(|link| link.issue_title.clone());
        // Best-effort duplicate detection; creation re-checks authoritatively.
        let existing = factory_github::GhCli::discovered()
            .list_pull_requests(&remote.repository, &delivery.head_branch)
            .ok()
            .and_then(|prs| prs.into_iter().max_by_key(|pr| pr.number));
        // The recipe's GitHub delivery default (when the run has one) seeds
        // the draft checkbox; the user can still change it before creating.
        let draft = run
            .recipe
            .as_ref()
            .and_then(|recipe| recipe.github)
            .map(|github| github.draft)
            .unwrap_or(false);
        Ok(crate::github::PrPreview {
            run_id,
            repository: remote.repository.clone(),
            base,
            head: delivery.head_branch.clone(),
            title: factory_github::default_pr_title(&run.objective, issue_title.as_deref()),
            body: factory_github::build_pr_body(&evidence),
            draft,
            issue_number: evidence.issue_number,
            issue_url: link.as_ref().map(|link| link.issue_url.clone()),
            existing,
            eligible: eligibility.ready,
            blockers: eligibility.blockers,
        })
    }

    /// Delivers a completed workflow: pushes its `factory/run-<id>` branch to
    /// the project remote and creates (or links an existing) pull request.
    ///
    /// This is the only Factory-owned push path. Agents never reach it: the
    /// Policy Engine independently denies push-class git operations for task
    /// agents, and only this explicit user action runs here.
    pub fn create_pull_request(
        &self,
        run_id: i64,
        title: Option<&str>,
        body: Option<&str>,
        draft: bool,
    ) -> Result<GitHubDelivery, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        let mut delivery = self.db.get_or_create_delivery(run_id)?;
        // An existing PR is shown and linked, never duplicated.
        if delivery.pull_request.is_some() {
            return Ok(delivery);
        }
        let integration_head = self.db.get_run_integration(run_id)?;
        let repo = Repo::detect_bounded(&self.root, &self.root)?;
        let local_head = repo
            .resolve_ref(&delivery.head_branch)
            .ok()
            .filter(|sha| !sha.is_empty());
        // Eligibility covers completion, integration head, and branch drift;
        // each failure names its blocker.
        let eligibility = crate::github::delivery_eligibility(
            &run,
            integration_head.as_deref(),
            local_head.as_deref(),
        );
        if !eligibility.ready {
            return Err(FactoryError::NotDeliverable(
                eligibility.blockers.join("; "),
            ));
        }
        let integration_head =
            integration_head.expect("eligibility guarantees an integration head");
        let remote = self.github_remote()?;
        let base = remote
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".to_string());
        // From here on, every failure is recorded on the delivery record.
        match factory_github::remote_branch_exists(&self.root, &remote.remote, &base) {
            Ok(true) => {}
            Ok(false) => {
                return self.fail_delivery(
                    delivery,
                    factory_github::GitHubError::BaseBranchUnavailable(base),
                )
            }
            Err(error) => return self.fail_delivery(delivery, error),
        }
        let tasks = self.db.list_tasks(run_id)?;
        let attempts = self.db.list_task_attempts(run_id)?;
        let link = self.db.get_run_github_link(run_id)?;
        let mut evidence = crate::github::pr_evidence(&run, &tasks, &attempts);
        evidence.issue_number = link.as_ref().map(|link| link.issue_number);
        let issue_title = link.as_ref().map(|link| link.issue_title.clone());
        let default_title =
            factory_github::default_pr_title(&run.objective, issue_title.as_deref());
        let title = title.map(str::trim).filter(|t| !t.is_empty());
        let body = body.map(str::trim).filter(|b| !b.is_empty());
        let final_title = title.map(str::to_string).unwrap_or(default_title);
        let final_body = body
            .map(str::to_string)
            .unwrap_or_else(|| factory_github::build_pr_body(&evidence));

        delivery.state = DeliveryState::Pushing;
        delivery.repository = Some(remote.repository.clone());
        delivery.remote = Some(remote.remote.clone());
        delivery.base_branch = Some(base.clone());
        delivery.error = None;
        self.db.set_delivery(&delivery)?;
        if let Err(error) =
            factory_github::push_branch(&self.root, &remote.remote, &delivery.head_branch)
        {
            return self.fail_delivery(delivery, error);
        }

        delivery.state = DeliveryState::CreatingPr;
        self.db.set_delivery(&delivery)?;
        let gh = factory_github::GhCli::discovered();
        let outcome = match factory_github::create_or_link_pull_request(
            &gh,
            &remote.repository,
            &base,
            &delivery.head_branch,
            &final_title,
            &final_body,
            draft,
        ) {
            Ok(outcome) => outcome,
            Err(error) => return self.fail_delivery(delivery, error),
        };
        let mut pull_request = match &outcome {
            factory_github::DeliveryOutcome::Created(pr) => pr.clone(),
            factory_github::DeliveryOutcome::LinkedExisting(pr) => pr.clone(),
        };
        // `gh pr create` reports only a URL, so the requested draft flag is
        // what Factory records for freshly created PRs; linked existing PRs
        // keep the state GitHub reported.
        if matches!(outcome, factory_github::DeliveryOutcome::Created(_)) {
            pull_request.is_draft = draft;
        }
        delivery.pull_request = Some(pull_request);
        delivery.pushed_head = Some(integration_head);
        delivery.state = DeliveryState::Published;
        delivery.error = None;
        self.db.set_delivery(&delivery)?;
        Ok(delivery)
    }

    /// Records a failed delivery attempt and converts the error.
    fn fail_delivery(
        &self,
        mut delivery: GitHubDelivery,
        error: factory_github::GitHubError,
    ) -> Result<GitHubDelivery, FactoryError> {
        delivery.state = DeliveryState::Failed;
        delivery.error = Some(error.to_string());
        let _ = self.db.set_delivery(&delivery);
        Err(FactoryError::GitHub(error))
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

fn agent_output_limit_error(role: &str, result: &AgentResult) -> Option<FactoryError> {
    if !result.output_metadata.stdout_truncated {
        return None;
    }
    Some(FactoryError::AgentOutputLimit {
        role: role.to_string(),
        limit_bytes: result.output_metadata.limit_bytes,
        stdout_bytes: result.output_metadata.stdout_bytes,
        stderr_bytes: result.output_metadata.stderr_bytes,
    })
}

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

fn describe_policy_scopes(config: &crate::config::Config, role: &str) -> String {
    let scopes = config
        .effective_role_policy(role)
        .filesystem
        .effective_write_scopes();
    if scopes.is_empty() {
        "none".to_string()
    } else {
        scopes.join(", ")
    }
}

fn default_artifact_content(output: &str) -> String {
    serde_json::json!({ "summary": tail(output, 4_000) }).to_string()
}

fn inject_external_tool_catalog(mission: &str, tools: &[McpToolMetadata]) -> String {
    let references: Vec<&McpToolMetadata> = tools.iter().collect();
    let Some(catalog) = render_tool_catalog(&references) else {
        return mission.to_string();
    };
    const OUTPUT: &str = "\n\nOUTPUT CONTRACT\n";
    match mission.rsplit_once(OUTPUT) {
        Some((before, after)) => format!("{before}\n\n{catalog}{OUTPUT}{after}"),
        None => format!("{mission}\n\n{catalog}"),
    }
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
            agent_override: None,
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
            agent_override: None,
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
            plan_revision: 1,
            recipe: None,
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
            agent_override: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let role = crate::roles::core_role(crate::roles::WORKER).unwrap();
        let mission = build_mission(&MissionContext {
            role: &role,
            operation: TaskOperation::Implement,
            task: &task,
            run_objective: "objective",
            untrusted_context: None,
            upstream_artifacts: &[],
            repository_context: None,
            previous_feedback: None,
            review_input: None,
            final_review: false,
            policy: None,
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
            untrusted_context: None,
            upstream_artifacts: &[],
            repository_context: None,
            previous_feedback: Some(&review),
            review_input: None,
            final_review: false,
            policy: None,
        });
        assert!(retry.contains("CONTEXT\nPrevious review requested changes:\nmissing test"));
        assert!(retry.contains("- add a regression test"));
    }

    #[test]
    fn external_tool_catalog_precedes_the_output_contract_without_schemas() {
        let tool = McpToolMetadata {
            server: "docs".into(),
            name: "search".into(),
            qualified_name: "docs.search".into(),
            description: "Search trusted documentation sources.".into(),
            input_schema: serde_json::json!({"type": "object", "required": ["query"]}),
            annotations: None,
            classification: factory_types::ToolClassification::Read,
            discovered_at: String::new(),
        };
        let mission = inject_external_tool_catalog(
            "TASK\nInspect docs.\n\nOUTPUT CONTRACT\nReturn JSON.",
            &[tool],
        );
        assert!(mission.contains("AVAILABLE EXTERNAL TOOLS"));
        assert!(mission.find("docs.search").unwrap() < mission.find("OUTPUT CONTRACT").unwrap());
        assert!(!mission.contains("required"));
    }
}
