use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use factory_core::{Agents, Config, ConfigError, RoleAssignment};
use factory_db::FactoryDb;
use factory_runtime::{Runtime, RuntimeError, TerminalSubscription};
use factory_types::{AgentSessionMode, WorkflowTeam};
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;

use crate::dashboard;
use crate::graph_workspace::{GraphWorkspace, GraphWorkspaceError, GraphWorkspaceResponse};
use crate::types::{
    AgentSessionResponse, GraphEdge, GraphNode, GraphResponse, RetryResponse, RunDetail,
    RunSummary, StageStatus, TaskCounts,
};

pub struct ApiState {
    pub db: Mutex<FactoryDb>,
    pub root: PathBuf,
    pub runtime: Runtime,
}

impl ApiState {
    pub fn new(root: PathBuf) -> Result<Self, RuntimeError> {
        let runtime = Runtime::new(&root)?;
        let db = FactoryDb::open(&root.join(".factory").join("db.sqlite3"))
            .map_err(factory_core::FactoryError::from)?;
        Ok(Self {
            db: Mutex::new(db),
            root,
            runtime,
        })
    }
}

pub type SharedState = Arc<ApiState>;

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<factory_db::DbError> for ApiError {
    fn from(err: factory_db::DbError) -> Self {
        match err {
            factory_db::DbError::NotFound(_) => {
                ApiError::new(StatusCode::NOT_FOUND, err.to_string())
            }
            other => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

impl From<ConfigError> for ApiError {
    fn from(err: ConfigError) -> Self {
        let status = match err {
            ConfigError::Missing(_) | ConfigError::Parse(_, _) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError::new(status, err.to_string())
    }
}

impl From<GraphWorkspaceError> for ApiError {
    fn from(err: GraphWorkspaceError) -> Self {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }
}

impl From<RuntimeError> for ApiError {
    fn from(error: RuntimeError) -> Self {
        let status = match &error {
            RuntimeError::AlreadyActive(_) => StatusCode::CONFLICT,
            RuntimeError::NotActive(_) => StatusCode::CONFLICT,
            RuntimeError::TerminalNotActive(_) => StatusCode::CONFLICT,
            RuntimeError::Terminal(_) => StatusCode::BAD_REQUEST,
            RuntimeError::Factory(factory_core::FactoryError::RunNotFound(_))
            | RuntimeError::Factory(factory_core::FactoryError::TaskNotFound(_)) => {
                StatusCode::NOT_FOUND
            }
            RuntimeError::Factory(
                factory_core::FactoryError::EmptyObjective
                | factory_core::FactoryError::InvalidRunState(_, _)
                | factory_core::FactoryError::EmptyPlan(_)
                | factory_core::FactoryError::InvalidDag(_, _)
                | factory_core::FactoryError::TaskRoleUnavailable(_, _)
                | factory_core::FactoryError::InvalidTeam(_)
                | factory_core::FactoryError::RetryLimit(_, _)
                | factory_core::FactoryError::InvalidTransition(_, _)
                | factory_core::FactoryError::NotReady(_)
                | factory_core::FactoryError::Agent(_)
                | factory_core::FactoryError::AgentProcess(_)
                | factory_core::FactoryError::Git(_)
                | factory_core::FactoryError::GitHub(_),
            ) => StatusCode::BAD_REQUEST,
            RuntimeError::Factory(factory_core::FactoryError::NotDeliverable(_)) => {
                StatusCode::CONFLICT
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError::new(status, error.to_string())
    }
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/from-issue", post(create_run_from_issue))
        .route("/api/runs/:id", get(get_run))
        .route("/api/runs/:id/start", post(start_run))
        .route("/api/runs/:id/cancel", post(cancel_run))
        .route("/api/runs/:id/delivery", get(run_delivery))
        .route("/api/runs/:id/pr-preview", get(run_pr_preview))
        .route("/api/runs/:id/pull-request", post(create_pull_request))
        .route("/api/tasks/:id/retry", post(retry_task))
        .route("/api/runs/:id/team", put(put_run_team))
        .route("/api/runs/:id/artifacts", get(run_artifacts))
        .route("/api/tasks/:id/artifacts", get(task_artifacts))
        .route("/api/roles", get(list_roles).post(create_role))
        .route("/api/roles/:id", put(update_role).delete(delete_role))
        .route("/api/roles/:id/assignments", post(add_role_assignment))
        .route(
            "/api/roles/:id/assignments/:agent",
            delete(remove_role_assignment),
        )
        .route("/api/roles/:id/preferred", put(set_preferred_assignment))
        .route("/api/roles/:id/policy", put(put_role_policy))
        .route("/api/graph", get(get_graph))
        .route(
            "/api/graph/workspace",
            get(get_graph_workspace).put(put_graph_workspace),
        )
        .route("/api/github/status", get(github_status))
        .route("/api/agents", get(get_agents))
        .route(
            "/api/agents/:agent/sessions",
            get(list_agent_sessions).post(start_interactive_session),
        )
        .route(
            "/api/sessions/:id",
            get(get_agent_session).delete(stop_interactive_session),
        )
        .route("/api/sessions/:id/stream", get(stream_agent_session))
        .route("/api/sessions/:id/terminal", get(interactive_terminal))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/*rest", get(api_not_found))
        .fallback_service(dashboard::router(&state.root))
        .with_state(state)
}

async fn api_not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "unknown endpoint")
}

pub fn bind(port: u16) -> std::io::Result<std::net::TcpListener> {
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    let listener = std::net::TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// Serve the API and dashboard on an already-bound listener.
pub async fn serve(state: SharedState, listener: std::net::TcpListener) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, router(state)).await
}

pub async fn run_app(state: SharedState, port: u16) -> std::io::Result<()> {
    serve(state, bind(port)?).await
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn list_runs(State(state): State<SharedState>) -> Result<Json<Vec<RunSummary>>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    let runs = db.list_runs()?;
    let mut summaries = Vec::with_capacity(runs.len());
    for run in runs {
        let tasks = db.list_tasks(run.id)?;
        summaries.push(RunSummary {
            id: run.id,
            objective: run.objective.clone(),
            status: run.status.as_str().to_string(),
            planner_agent: run.planner_agent.clone(),
            created_at: run.created_at.clone(),
            counts: TaskCounts::from_tasks(&tasks),
        });
    }
    Ok(Json(summaries))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRunRequest {
    objective: String,
    #[serde(default)]
    team: Option<WorkflowTeam>,
}

async fn create_run(
    State(state): State<SharedState>,
    body: Result<Json<CreateRunRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<factory_types::Run>), ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let run = state
        .runtime
        .create_workflow(&request.objective, request.team)?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

// --- GitHub linkage and delivery -------------------------------------------

/// `gh auth status` + remote detection for the dashboard connection chip.
/// Semantic read: no GitHub mutation happens here.
async fn github_status(
    State(state): State<SharedState>,
) -> Result<Json<factory_core::github::GitHubStatus>, ApiError> {
    let root = state.root.clone();
    let status = tokio::task::spawn_blocking(
        move || -> Result<factory_core::github::GitHubStatus, ApiError> {
            let factory = factory_core::Factory::open(&root).map_err(|error| {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            })?;
            Ok(factory.github_status())
        },
    )
    .await
    .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))??;
    Ok(Json(status))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRunFromIssueRequest {
    /// `#42`, `42`, or a GitHub issue URL.
    issue: String,
    #[serde(default)]
    team: Option<WorkflowTeam>,
}

/// Imports a GitHub Issue as a workflow. The run is planned in the background;
/// nothing executes until the user explicitly starts it.
async fn create_run_from_issue(
    State(state): State<SharedState>,
    body: Result<Json<CreateRunFromIssueRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<factory_types::Run>), ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let run = state
        .runtime
        .import_workflow_from_issue(&request.issue, request.team)?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

async fn run_delivery(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<factory_core::github::DeliveryReport>, ApiError> {
    let report = run_delivery_operation(state, move |factory| factory.delivery_report(id)).await?;
    Ok(Json(report))
}

async fn run_pr_preview(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<factory_core::github::PrPreview>, ApiError> {
    let preview =
        run_delivery_operation(state, move |factory| factory.pull_request_preview(id)).await?;
    Ok(Json(preview))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePullRequestRequest {
    /// Overrides the derived title (issue title / objective first line).
    #[serde(default)]
    title: Option<String>,
    /// Overrides the deterministic evidence-based body.
    #[serde(default)]
    body: Option<String>,
    /// Factory's documented default is a normal (non-draft) pull request.
    #[serde(default)]
    draft: bool,
}

/// The single Factory-owned delivery action: push `factory/run-<id>` and
/// create (or link) its pull request. This endpoint is semantic — there is no
/// generic GitHub command passthrough.
async fn create_pull_request(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
    body: Result<Json<CreatePullRequestRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<factory_types::GitHubDelivery>), ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let delivery = run_delivery_operation(state, move |factory| {
        factory.create_pull_request(
            id,
            request.title.as_deref(),
            request.body.as_deref(),
            request.draft,
        )
    })
    .await?;
    Ok((StatusCode::CREATED, Json(delivery)))
}

/// Runs a blocking delivery operation against a freshly opened Factory.
/// `RuntimeError`'s status mapping covers not-found, not-deliverable, and the
/// actionable GitHub errors.
async fn run_delivery_operation<T, F>(state: SharedState, operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(factory_core::Factory) -> Result<T, factory_core::FactoryError> + Send + 'static,
{
    let root = state.root.clone();
    tokio::task::spawn_blocking(move || {
        let factory = factory_core::Factory::open(&root).map_err(RuntimeError::from)?;
        operation(factory).map_err(RuntimeError::from)
    })
    .await
    .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    .map_err(ApiError::from)
}

async fn start_run(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<(StatusCode, Json<WorkflowTeam>), ApiError> {
    let team = state.runtime.start_workflow(id)?;
    Ok((StatusCode::ACCEPTED, Json(team)))
}

async fn put_run_team(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
    body: Result<Json<WorkflowTeam>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<WorkflowTeam>, ApiError> {
    let team = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let team = state.runtime.update_workflow_team(id, team)?;
    Ok(Json(team))
}

async fn cancel_run(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<StatusCode, ApiError> {
    state.runtime.cancel_workflow(id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn retry_task(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<(StatusCode, Json<RetryResponse>), ApiError> {
    let run_id = state.runtime.retry_task(id)?;
    Ok((StatusCode::ACCEPTED, Json(RetryResponse { run_id })))
}

async fn get_run(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<RunDetail>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    let run = db.get_run(id)?.ok_or(ApiError::new(
        StatusCode::NOT_FOUND,
        format!("run {id} not found"),
    ))?;
    let tasks = db.list_tasks(id)?;
    let attempts = db.list_task_attempts(id)?;
    let sessions = db.list_agent_sessions(Some(id))?;
    let artifacts = db.list_role_artifacts(id)?;
    let stages = derive_stages(&tasks);
    let head = db.get_run_integration(id)?;
    let mut latest: HashMap<i64, &factory_types::TaskAttempt> = HashMap::new();
    for attempt in &attempts {
        match latest.get(&attempt.task_id) {
            Some(current) if attempt.attempt_number > current.attempt_number => {
                latest.insert(attempt.task_id, attempt);
            }
            None => {
                latest.insert(attempt.task_id, attempt);
            }
            _ => {}
        }
    }
    let integrated_tasks = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.operation,
                Some(
                    factory_types::TaskOperation::Implement
                        | factory_types::TaskOperation::Verify
                        | factory_types::TaskOperation::PostProcess
                )
            ) && latest
                .get(&task.id)
                .is_some_and(|attempt| attempt.status == factory_types::AttemptStatus::Approved)
        })
        .map(|task| task.id)
        .collect();
    Ok(Json(RunDetail {
        run,
        tasks,
        attempts,
        sessions,
        stages,
        artifacts,
        integration: crate::types::RunIntegration {
            branch: format!("factory/run-{id}"),
            head,
            integrated_tasks,
        },
    }))
}

async fn run_artifacts(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Vec<factory_types::RoleArtifact>>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    if db.get_run(id)?.is_none() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("run {id} not found"),
        ));
    }
    Ok(Json(db.list_role_artifacts(id)?))
}

async fn task_artifacts(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Vec<factory_types::RoleArtifact>>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    if db.get_task(id)?.is_none() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("task {id} not found"),
        ));
    }
    Ok(Json(db.list_artifacts_for_task(id)?))
}

/// The compact stage overview shown in the Workflow Inspector: one row per
/// operation kind present in the plan, ordered by the role-aware pipeline.
fn derive_stages(tasks: &[factory_types::Task]) -> Vec<StageStatus> {
    use factory_types::TaskOperation as Op;
    let order = [
        (Op::Advisory, "analysis", "Analysis"),
        (Op::Implement, "implementation", "Implementation"),
        (Op::Verify, "verification", "Verification"),
        (Op::Review, "review", "Review"),
        (Op::PostProcess, "post_process", "Post-processing"),
    ];
    let mut stages = Vec::new();
    for (operation, key, label) in order {
        let members: Vec<&factory_types::Task> = tasks
            .iter()
            .filter(|task| task.operation == Some(operation))
            .collect();
        if members.is_empty() {
            continue;
        }
        let completed = members
            .iter()
            .filter(|task| task.state == factory_types::TaskState::Completed)
            .count();
        let active = members.iter().any(|task| {
            matches!(
                task.state,
                factory_types::TaskState::Ready | factory_types::TaskState::Running
            )
        });
        let state = if members
            .iter()
            .all(|task| task.state == factory_types::TaskState::Completed)
        {
            "completed"
        } else if active {
            "active"
        } else {
            "pending"
        };
        stages.push(StageStatus {
            key: key.to_string(),
            label: label.to_string(),
            total: members.len(),
            completed,
            state: state.to_string(),
        });
    }
    if stages.is_empty() && !tasks.is_empty() {
        stages.push(StageStatus {
            key: "task".to_string(),
            label: "Tasks".to_string(),
            total: tasks.len(),
            completed: tasks
                .iter()
                .filter(|task| task.state == factory_types::TaskState::Completed)
                .count(),
            state: "pending".to_string(),
        });
    }
    stages
}

async fn get_agents(
    State(state): State<SharedState>,
) -> Result<Json<Vec<factory_core::AgentInfo>>, ApiError> {
    let agents = Agents::load(&state.root)?;
    Ok(Json(agents.list()))
}

async fn get_config(State(state): State<SharedState>) -> Result<Json<Config>, ApiError> {
    let config = Config::load(&state.root)?;
    Ok(Json(config))
}

async fn put_config(
    State(state): State<SharedState>,
    body: Result<Json<Config>, axum::extract::rejection::JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let mut config = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    config.normalize();
    save_config(&state, &config)?;
    Ok(StatusCode::NO_CONTENT)
}

fn load_config(state: &SharedState) -> Result<Config, ApiError> {
    Ok(Config::load(&state.root)?)
}

fn save_config(state: &SharedState, config: &Config) -> Result<(), ApiError> {
    config
        .validate()
        .map_err(|reason| ApiError::new(StatusCode::BAD_REQUEST, reason))?;
    config.write_atomic(&state.root).map_err(ApiError::from)?;
    Ok(())
}

fn role_info(config: &Config, id: &str) -> Result<factory_core::RoleInfo, ApiError> {
    config
        .role_infos()
        .into_iter()
        .find(|role| role.id == id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("role '{id}' not found")))
}

fn role_in_use(db: &FactoryDb, role: &str) -> Result<bool, factory_db::DbError> {
    for run in db.list_runs()? {
        if !matches!(
            run.status,
            factory_types::RunStatus::Planning
                | factory_types::RunStatus::Planned
                | factory_types::RunStatus::Active
                | factory_types::RunStatus::Blocked
        ) {
            continue;
        }
        if run
            .team
            .as_ref()
            .is_some_and(|team| team.additional.contains_key(role))
        {
            return Ok(true);
        }
        for task in db.list_tasks(run.id)? {
            if task.role.as_deref() == Some(role) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn list_roles(
    State(state): State<SharedState>,
) -> Result<Json<Vec<factory_core::RoleInfo>>, ApiError> {
    let config = load_config(&state)?;
    Ok(Json(config.role_infos()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRoleRequest {
    #[serde(default)]
    id: Option<String>,
    name: String,
    description: String,
    execution_class: factory_core::ExecutionClass,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    agents: Vec<String>,
    #[serde(default)]
    preferred_agent: Option<String>,
    /// Optional policy preset for the new role (`read_only`, `implementation`,
    /// `documentation`, `review`, or `custom`).
    #[serde(default)]
    policy_preset: Option<String>,
}

async fn create_role(
    State(state): State<SharedState>,
    body: Result<Json<CreateRoleRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<factory_core::RoleInfo>), ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let mut config = load_config(&state)?;
    let id = request
        .id
        .clone()
        .unwrap_or_else(|| factory_core::slugify(&request.name));
    if id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "the role needs a name or id",
        ));
    }
    if factory_core::is_core_role(&id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("role '{id}' is a built-in core role and cannot be redefined"),
        ));
    }
    if config.roles.contains_key(&id) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("role '{id}' already exists"),
        ));
    }
    config.roles.insert(
        id.clone(),
        factory_core::RoleDefinitionEntry {
            name: Some(request.name.trim().to_string()),
            description: Some(request.description.trim().to_string()),
            execution_class: Some(request.execution_class),
            instructions: request.instructions.trim().to_string(),
            agent: None,
        },
    );
    let preferred_agent = request.preferred_agent.as_deref();
    if preferred_agent.is_some() {
        config
            .role_assignments
            .retain(|assignment| !(assignment.role == id && assignment.preferred));
    }
    for agent in &request.agents {
        config.role_assignments.push(RoleAssignment {
            role: id.clone(),
            agent: agent.clone(),
            preferred: preferred_agent == Some(agent.as_str()),
        });
    }
    if let Some(preset_name) = request.policy_preset.as_deref() {
        let preset = parse_policy_preset(preset_name)?;
        config.policies.role_mut(&id).preset = Some(preset);
    }
    save_config(&state, &config)?;
    Ok((StatusCode::CREATED, Json(role_info(&config, &id)?)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRoleRequest {
    name: String,
    description: String,
    execution_class: factory_core::ExecutionClass,
    #[serde(default)]
    instructions: String,
    /// When present, replaces the role's policy preset (null clears it).
    #[serde(default)]
    policy_preset: Option<String>,
}

async fn update_role(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    body: Result<Json<UpdateRoleRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<factory_core::RoleInfo>, ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let mut config = load_config(&state)?;
    if factory_core::is_core_role(&id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "built-in core roles cannot be redefined; manage their assignments instead",
        ));
    }
    let Some(entry) = config.roles.get_mut(&id) else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("role '{id}' not found"),
        ));
    };
    *entry = factory_core::RoleDefinitionEntry {
        name: Some(request.name.trim().to_string()),
        description: Some(request.description.trim().to_string()),
        execution_class: Some(request.execution_class),
        instructions: request.instructions.trim().to_string(),
        agent: None,
    };
    // Absent preset leaves the policy untouched (clearing goes through
    // PUT /api/roles/:id/policy, which distinguishes null from absent).
    if let Some(preset_name) = request.policy_preset.as_deref() {
        let preset = parse_policy_preset(preset_name)?;
        config.policies.role_mut(&id).preset = Some(preset);
    }
    save_config(&state, &config)?;
    Ok(Json(role_info(&config, &id)?))
}

async fn delete_role(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
) -> Result<StatusCode, ApiError> {
    let mut config = load_config(&state)?;
    if factory_core::is_core_role(&id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "built-in core roles cannot be deleted; remove their assignments instead",
        ));
    }
    if config.roles.remove(&id).is_none() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("role '{id}' not found"),
        ));
    }
    {
        let db = state.db.lock().expect("db mutex poisoned");
        if role_in_use(&db, &id)? {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                format!("role '{id}' is used by an active workflow and cannot be deleted"),
            ));
        }
    }
    config
        .role_assignments
        .retain(|assignment| assignment.role != id);
    // A deleted role must not keep a policy scope around.
    config.policies.roles.remove(&id);
    save_config(&state, &config)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddAssignmentRequest {
    agent: String,
    #[serde(default)]
    preferred: bool,
}

async fn add_role_assignment(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    body: Result<Json<AddAssignmentRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<factory_core::RoleInfo>, ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let mut config = load_config(&state)?;
    if config.role_infos().iter().all(|role| role.id != id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("role '{id}' not found"),
        ));
    }
    if !config.agents.contains_key(&request.agent) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("unknown agent '{}'", request.agent),
        ));
    }
    if config
        .role_assignments
        .iter()
        .any(|assignment| assignment.role == id && assignment.agent == request.agent)
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "agent '{}' is already assigned to role '{id}'",
                request.agent
            ),
        ));
    }
    if request.preferred {
        config.role_assignments.iter_mut().for_each(|assignment| {
            if assignment.role == id {
                assignment.preferred = false;
            }
        });
    }
    config.role_assignments.push(RoleAssignment {
        role: id.clone(),
        agent: request.agent,
        preferred: request.preferred,
    });
    save_config(&state, &config)?;
    Ok(Json(role_info(&config, &id)?))
}

async fn remove_role_assignment(
    State(state): State<SharedState>,
    UrlPath((id, agent)): UrlPath<(String, String)>,
) -> Result<Json<factory_core::RoleInfo>, ApiError> {
    let mut config = load_config(&state)?;
    let before = config.role_assignments.len();
    config
        .role_assignments
        .retain(|assignment| !(assignment.role == id && assignment.agent == agent));
    if config.role_assignments.len() == before {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("agent '{agent}' is not assigned to role '{id}'"),
        ));
    }
    save_config(&state, &config)?;
    Ok(Json(role_info(&config, &id)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreferredAssignmentRequest {
    agent: String,
}

async fn set_preferred_assignment(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    body: Result<Json<PreferredAssignmentRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<factory_core::RoleInfo>, ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let mut config = load_config(&state)?;
    let mut found = false;
    for assignment in &mut config.role_assignments {
        if assignment.role != id {
            continue;
        }
        assignment.preferred = assignment.agent == request.agent;
        if assignment.preferred {
            found = true;
        }
    }
    if !found {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("agent '{}' is not assigned to role '{id}'", request.agent),
        ));
    }
    save_config(&state, &config)?;
    Ok(Json(role_info(&config, &id)?))
}

/// Parses a policy preset name accepted by the dashboard. `null` is handled by
/// the caller (it clears the preset).
fn parse_policy_preset(value: &str) -> Result<factory_policy::PolicyPreset, ApiError> {
    factory_policy::PolicyPreset::parse(value).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "unknown policy preset '{value}' (expected read_only, implementation, \
                 documentation, review, or custom)"
            ),
        )
    })
}

/// Sets (or clears) a role's policy preset. Works for core and custom roles:
/// the policy says what Factory permits the role to do, which is orthogonal to
/// the role's instructions. Clearing a preset only removes the preset; any
/// explicitly configured dimensions stay untouched.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RolePolicyRequest {
    preset: Option<String>,
}

async fn put_role_policy(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    body: Result<Json<RolePolicyRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<factory_core::RoleInfo>, ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let mut config = load_config(&state)?;
    if config.role_infos().iter().all(|role| role.id != id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("role '{id}' not found"),
        ));
    }
    match request.preset.as_deref() {
        Some(name) => {
            let preset = parse_policy_preset(name)?;
            config.policies.role_mut(&id).preset = Some(preset);
        }
        None => {
            let mut now_empty = false;
            if let Some(scope) = config.policies.roles.get_mut(&id) {
                scope.preset = None;
                now_empty = scope.filesystem.is_none()
                    && scope.commands.is_none()
                    && scope.network.is_none()
                    && scope.environment.is_none()
                    && scope.git.is_none();
            }
            if now_empty {
                config.policies.roles.remove(&id);
            }
        }
    }
    save_config(&state, &config)?;
    Ok(Json(role_info(&config, &id)?))
}

async fn get_graph_workspace(
    State(state): State<SharedState>,
) -> Result<Json<GraphWorkspaceResponse>, ApiError> {
    let mut response = GraphWorkspace::load(&state.root)?;
    let graph = build_graph(&state)?;
    let system_nodes: HashMap<String, String> = graph
        .nodes
        .into_iter()
        .map(|node| (node.id, node.kind))
        .collect();
    if response.workspace.retain_known(&system_nodes) {
        response.warning = Some(
            "Ignored stale graph positions or links for removed Factory entities.".to_string(),
        );
    }
    Ok(Json(response))
}

async fn put_graph_workspace(
    State(state): State<SharedState>,
    body: Result<Json<GraphWorkspace>, axum::extract::rejection::JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let workspace = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let graph = build_graph(&state)?;
    let system_nodes: HashMap<String, String> = graph
        .nodes
        .into_iter()
        .map(|node| (node.id, node.kind))
        .collect();
    workspace
        .validate(&system_nodes)
        .map_err(|reason| ApiError::new(StatusCode::BAD_REQUEST, reason))?;
    workspace.write_atomic(&state.root)?;
    Ok(StatusCode::NO_CONTENT)
}

fn session_response(
    state: &SharedState,
    db: &FactoryDb,
    session: factory_types::AgentSession,
) -> Result<AgentSessionResponse, ApiError> {
    let working_directory = match session.task_id {
        Some(task_id) => db
            .get_task(task_id)?
            .and_then(|task| task.worktree_path)
            .unwrap_or_else(|| state.root.to_string_lossy().into_owned()),
        None => state.root.to_string_lossy().into_owned(),
    };
    Ok(AgentSessionResponse {
        interactive: session.mode == AgentSessionMode::Interactive,
        session,
        working_directory,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartInteractiveSession {
    #[serde(default = "default_terminal_cols")]
    cols: u16,
    #[serde(default = "default_terminal_rows")]
    rows: u16,
}

fn default_terminal_cols() -> u16 {
    100
}

fn default_terminal_rows() -> u16 {
    28
}

async fn start_interactive_session(
    State(state): State<SharedState>,
    UrlPath(agent): UrlPath<String>,
    Json(request): Json<StartInteractiveSession>,
) -> Result<Json<AgentSessionResponse>, ApiError> {
    let session = state
        .runtime
        .start_interactive_session(&agent, request.cols, request.rows)?;
    let db = state.db.lock().expect("db mutex poisoned");
    Ok(Json(session_response(&state, &db, session)?))
}

async fn stop_interactive_session(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<StatusCode, ApiError> {
    let session = {
        let db = state.db.lock().expect("db mutex poisoned");
        db.get_agent_session(id)?.ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("session {id} not found"))
        })?
    };
    if session.mode != AgentSessionMode::Interactive {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("session {id} is not interactive"),
        ));
    }
    state.runtime.stop_interactive_session(id)?;
    Ok(StatusCode::ACCEPTED)
}

async fn interactive_terminal(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    {
        let db = state.db.lock().expect("db mutex poisoned");
        let session = db.get_agent_session(id)?.ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("session {id} not found"))
        })?;
        if session.mode != AgentSessionMode::Interactive {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("session {id} is not interactive"),
            ));
        }
    }
    let subscription = state.runtime.subscribe_terminal(id)?;
    Ok(upgrade
        .on_upgrade(move |socket| terminal_socket(socket, state, id, subscription))
        .into_response())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalCommand {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

async fn terminal_socket(
    mut socket: WebSocket,
    state: SharedState,
    session_id: i64,
    mut subscription: TerminalSubscription,
) {
    if !subscription.snapshot.is_empty()
        && socket
            .send(Message::Binary(subscription.snapshot))
            .await
            .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break; };
                match message {
                    Message::Text(text) => {
                        let Ok(command) = serde_json::from_str::<TerminalCommand>(&text) else {
                            let _ = socket.send(Message::Text("invalid terminal command".into())).await;
                            continue;
                        };
                        let result = match command {
                            TerminalCommand::Input { data } => state.runtime.write_terminal(session_id, data.as_bytes()),
                            TerminalCommand::Resize { cols, rows } => state.runtime.resize_terminal(session_id, cols, rows),
                        };
                        if let Err(error) = result {
                            let _ = socket.send(Message::Text(error.to_string())).await;
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(payload) => {
                        if socket.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Message::Binary(_) | Message::Pong(_) => {}
                }
            }
            output = subscription.receiver.recv() => {
                let Some(output) = output else { break; };
                if socket.send(Message::Binary(output)).await.is_err() { break; }
            }
        }
    }
}

async fn list_agent_sessions(
    State(state): State<SharedState>,
    UrlPath(agent): UrlPath<String>,
) -> Result<Json<Vec<AgentSessionResponse>>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    let sessions = db.list_agent_sessions_for_agent(&agent, 12)?;
    if sessions.is_empty() {
        let config = Config::load(&state.root)?;
        if !config.agents.contains_key(&agent) {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("agent '{agent}' not found"),
            ));
        }
    }
    let responses = sessions
        .into_iter()
        .map(|session| session_response(&state, &db, session))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(responses))
}

async fn get_agent_session(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<AgentSessionResponse>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    let session = db
        .get_agent_session(id)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("session {id} not found")))?;
    Ok(Json(session_response(&state, &db, session)?))
}

async fn stream_agent_session(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Sse<ReceiverStream<Result<Event, Infallible>>>, ApiError> {
    {
        let db = state.db.lock().expect("db mutex poisoned");
        if db.get_agent_session(id)?.is_none() {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("session {id} not found"),
            ));
        }
    }

    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move {
        let mut previous = String::new();
        loop {
            let result = {
                let db = state.db.lock().expect("db mutex poisoned");
                db.get_agent_session(id)
                    .map_err(|error| error.to_string())
                    .and_then(|session| {
                        session
                            .map(|session| {
                                session_response(&state, &db, session)
                                    .map_err(|error| error.message)
                            })
                            .transpose()
                    })
            };

            let response = match result {
                Ok(Some(response)) => response,
                Ok(None) => break,
                Err(error) => {
                    let event = Event::default().event("error").data(&error);
                    let _ = sender.send(Ok(event)).await;
                    break;
                }
            };
            let serialized = match serde_json::to_string(&response) {
                Ok(serialized) => serialized,
                Err(_) => break,
            };
            if serialized != previous {
                previous = serialized.clone();
                let event = Event::default().event("session").data(serialized);
                if sender.send(Ok(event)).await.is_err() {
                    break;
                }
            }
            if !matches!(response.session.status.as_str(), "running" | "active") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(750)).await;
        }
    });

    Ok(Sse::new(ReceiverStream::new(receiver)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

async fn get_graph(State(state): State<SharedState>) -> Result<Json<GraphResponse>, ApiError> {
    Ok(Json(build_graph(&state)?))
}

fn build_graph(state: &SharedState) -> Result<GraphResponse, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");

    let config = factory_core::Config::load(&state.root).ok();
    let agents = Agents::load(&state.root)
        .map(|a| a.list())
        .unwrap_or_default();

    let mut roles_for_agent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut planner_bound = false;
    if let Some(config) = &config {
        for assignment in &config.role_assignments {
            roles_for_agent
                .entry(assignment.agent.clone())
                .or_default()
                .push(assignment.role.clone());
            if assignment.role == "planner" {
                planner_bound = true;
            }
        }
    }

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();

    for agent in &agents {
        nodes.push(GraphNode {
            id: format!("agent:{}", agent.name),
            kind: "agent".into(),
            label: agent.name.clone(),
            meta: json!({
                "command": agent.command,
                "available": agent.available,
                "status": agent.status,
                "kind": agent.kind,
                "workflowAvailable": agent.workflow_available,
                "interactiveAvailable": agent.interactive_available,
                "resolvedExecutable": agent.resolved_executable,
                "resolutionError": agent.resolution_error,
                "resolutionShim": agent.resolution_shim,
                "resolutionTarget": agent.resolution_target,
                "resolutionKind": agent.resolution_kind,
                "pathEntriesChecked": agent.path_entries_checked,
                "roles": roles_for_agent.get(&agent.name).cloned().unwrap_or_default(),
                "permissions": agent.permissions,
            }),
        });
    }

    let role_infos = config
        .as_ref()
        .map(|config| config.role_infos())
        .unwrap_or_default();
    let visible_roles = role_infos
        .iter()
        .filter(|role| {
            factory_core::is_pipeline_role(&role.id) || role.kind == "custom" || role.available
        })
        .collect::<Vec<_>>();
    for role in &visible_roles {
        nodes.push(GraphNode {
            id: format!("role:{}", role.id),
            kind: "role".into(),
            label: role.name.clone(),
            meta: json!({
                "id": role.id,
                "name": role.name,
                "kind": role.kind,
                "description": role.description,
                "instructions": role.instructions,
                "executionClass": role.execution_class,
                "assignments": role.assignments,
                "available": role.available,
                "permissions": role.permissions,
                "policyPreset": role.policy_preset,
            }),
        });
        for assignment in &role.assignments {
            edges.push(GraphEdge {
                id: format!("assignment:{}:{}", role.id, assignment.agent),
                source: format!("role:{}", role.id),
                target: format!("agent:{}", assignment.agent),
                kind: "binds".into(),
                editable: true,
                semantic: "configuration".into(),
            });
        }
    }

    let runs = db.list_runs()?;
    let mut total_tasks = 0usize;
    let mut task_dependencies: Vec<(String, Vec<i64>)> = Vec::new();

    for run in &runs {
        let tasks = db.list_tasks(run.id)?;
        let attempts = db.list_task_attempts(run.id)?;
        let sessions = db.list_agent_sessions(Some(run.id))?;
        let github_link = db.get_run_github_link(run.id).ok().flatten();
        let delivery = db.get_delivery(run.id).ok().flatten();
        let latest_attempts: HashMap<i64, factory_types::TaskAttempt> =
            attempts
                .into_iter()
                .fold(HashMap::new(), |mut latest, attempt| {
                    latest.insert(attempt.task_id, attempt);
                    latest
                });
        total_tasks += tasks.len();
        nodes.push(GraphNode {
            id: format!("run:{}", run.id),
            kind: "run".into(),
            label: run.objective.clone(),
            meta: json!({
                "runId": run.id,
                "objective": run.objective,
                "status": run.status.as_str(),
                "plannerAgent": run.planner_agent,
                "team": run.team,
                "createdAt": run.created_at,
                "counts": TaskCounts::from_tasks(&tasks),
                "github": github_link.as_ref().map(|link| json!({
                    "issueNumber": link.issue_number,
                    "issueUrl": link.issue_url,
                    "issueTitle": link.issue_title,
                    "repository": link.repository,
                })),
                "delivery": delivery.as_ref().map(|delivery| json!({
                    "state": delivery.state.as_str(),
                    "prNumber": delivery.pull_request.as_ref().map(|pr| pr.number),
                    "prUrl": delivery.pull_request.as_ref().map(|pr| pr.url.clone()),
                })),
            }),
        });

        // Compact external-source nodes: the imported Issue before the run,
        // and the delivered pull request after it.
        if let Some(link) = &github_link {
            nodes.push(GraphNode {
                id: format!("github_issue:{}", run.id),
                kind: "github_issue".into(),
                label: format!("#{} {}", link.issue_number, link.issue_title),
                meta: json!({
                    "runId": run.id,
                    "number": link.issue_number,
                    "repository": link.repository,
                    "url": link.issue_url,
                    "title": link.issue_title,
                    "state": link.issue_state,
                    "author": link.issue_author,
                    "labels": link.issue_labels,
                }),
            });
            edges.push(GraphEdge {
                id: format!("originates:{}", run.id),
                source: format!("github_issue:{}", run.id),
                target: format!("run:{}", run.id),
                kind: "originates".into(),
                editable: false,
                semantic: "system".into(),
            });
        }
        if let Some(pr) = delivery
            .as_ref()
            .and_then(|delivery| delivery.pull_request.as_ref())
        {
            nodes.push(GraphNode {
                id: format!("github_pr:{}", run.id),
                kind: "github_pr".into(),
                label: format!("PR #{}", pr.number),
                meta: json!({
                    "runId": run.id,
                    "number": pr.number,
                    "url": pr.url,
                    "state": pr.state,
                    "isDraft": pr.is_draft,
                }),
            });
            edges.push(GraphEdge {
                id: format!("delivers:{}", run.id),
                source: format!("run:{}", run.id),
                target: format!("github_pr:{}", run.id),
                kind: "delivers".into(),
                editable: false,
                semantic: "system".into(),
            });
        }

        if let Some(planner) = &run.planner_agent {
            let planner_assigned = config.as_ref().is_some_and(|config| {
                config
                    .role_assignments
                    .iter()
                    .any(|assignment| assignment.role == "planner" && &assignment.agent == planner)
            });
            let target = if planner_bound && planner_assigned {
                "role:planner".to_string()
            } else {
                format!("agent:{planner}")
            };
            edges.push(GraphEdge {
                id: format!("run-plan:{}:{target}", run.id),
                source: format!("run:{}", run.id),
                target,
                kind: "plans".into(),
                editable: false,
                semantic: "system".into(),
            });
        }

        for task in &tasks {
            nodes.push(GraphNode {
                id: format!("task:{}", task.id),
                kind: "task".into(),
                label: task.title.clone(),
                meta: json!({
                    "taskId": task.id,
                    "runId": task.run_id,
                    "objective": task.objective,
                    "state": task.state.as_str(),
                    "position": task.position,
                    "dependencies": task.dependencies,
                    "acceptanceCriteria": task.acceptance_criteria,
                    "worktreePath": task.worktree_path,
                    "role": task.role,
                    "operation": task.operation,
                    "currentAttempt": latest_attempts.get(&task.id),
                }),
            });
            edges.push(GraphEdge {
                id: format!("contains:{}:{}", run.id, task.id),
                source: format!("run:{}", run.id),
                target: format!("task:{}", task.id),
                kind: "contains".into(),
                editable: false,
                semantic: "system".into(),
            });
            task_dependencies.push((format!("task:{}", task.id), task.dependencies.clone()));
        }

        for session in sessions
            .iter()
            .filter(|session| session.status == "running")
        {
            let Some(task_id) = session.task_id else {
                continue;
            };
            if session.role == "planner" {
                continue;
            }
            let kind = if session.role == "reviewer" {
                "reviews"
            } else {
                "works"
            };
            edges.push(GraphEdge {
                id: format!("activity:{}", session.id),
                source: format!("agent:{}", session.agent),
                target: format!("task:{task_id}"),
                kind: kind.into(),
                editable: false,
                semantic: "system".into(),
            });
        }
    }

    for (task_id, dependencies) in task_dependencies {
        for dependency in dependencies {
            edges.push(GraphEdge {
                id: format!("dependency:{dependency}:{task_id}"),
                source: format!("task:{dependency}"),
                target: task_id.clone(),
                kind: "depends".into(),
                editable: false,
                semantic: "execution".into(),
            });
        }
    }

    let metadata = json!({
        "runs": runs.len(),
        "tasks": total_tasks,
        "agents": agents.len(),
        "missingAgents": agents.iter().filter(|a| !a.available).count(),
        "roles": visible_roles.len(),
    });

    Ok(GraphResponse {
        nodes,
        edges,
        metadata,
    })
}
