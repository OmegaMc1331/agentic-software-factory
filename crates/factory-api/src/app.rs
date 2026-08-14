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
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_core::{Agents, Config, ConfigError};
use factory_db::FactoryDb;
use factory_runtime::{Runtime, RuntimeError, TerminalSubscription};
use factory_types::AgentSessionMode;
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;

use crate::dashboard;
use crate::graph_workspace::{GraphWorkspace, GraphWorkspaceError, GraphWorkspaceResponse};
use crate::types::{
    AgentSessionResponse, ExecutionRolesResponse, GraphEdge, GraphNode, GraphResponse,
    RetryResponse, RunDetail, RunSummary, TaskCounts,
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
                | factory_core::FactoryError::RetryLimit(_)
                | factory_core::FactoryError::InvalidTransition(_, _)
                | factory_core::FactoryError::NotReady(_)
                | factory_core::FactoryError::Agent(_)
                | factory_core::FactoryError::AgentProcess(_)
                | factory_core::FactoryError::Git(_),
            ) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError::new(status, error.to_string())
    }
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/:id", get(get_run))
        .route("/api/runs/:id/start", post(start_run))
        .route("/api/runs/:id/cancel", post(cancel_run))
        .route("/api/tasks/:id/retry", post(retry_task))
        .route("/api/graph", get(get_graph))
        .route(
            "/api/graph/workspace",
            get(get_graph_workspace).put(put_graph_workspace),
        )
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
}

async fn create_run(
    State(state): State<SharedState>,
    body: Result<Json<CreateRunRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<factory_types::Run>), ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let run = state.runtime.create_workflow(&request.objective)?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

async fn start_run(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<(StatusCode, Json<ExecutionRolesResponse>), ApiError> {
    let roles = state.runtime.start_workflow(id)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ExecutionRolesResponse {
            worker: roles.worker,
            reviewer: roles.reviewer,
        }),
    ))
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
    Ok(Json(RunDetail {
        run,
        tasks,
        attempts,
        sessions,
    }))
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
    let config = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    config
        .validate()
        .map_err(|reason| ApiError::new(StatusCode::BAD_REQUEST, reason))?;
    config.write_atomic(&state.root).map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
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

    let mut role_to_agent: BTreeMap<String, String> = BTreeMap::new();
    let mut roles_for_agent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(config) = &config {
        for (role, entry) in &config.roles {
            role_to_agent.insert(role.clone(), entry.agent.clone());
            roles_for_agent
                .entry(entry.agent.clone())
                .or_default()
                .push(role.clone());
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
            }),
        });
    }

    for (role, agent) in &role_to_agent {
        nodes.push(GraphNode {
            id: format!("role:{role}"),
            kind: "role".into(),
            label: role.clone(),
            meta: json!({ "agent": agent }),
        });
        edges.push(GraphEdge {
            id: format!("assignment:{role}"),
            source: format!("role:{role}"),
            target: format!("agent:{agent}"),
            kind: "binds".into(),
            editable: true,
            semantic: "configuration".into(),
        });
    }

    let runs = db.list_runs()?;
    let mut total_tasks = 0usize;
    let mut task_dependencies: Vec<(String, Vec<i64>)> = Vec::new();

    for run in &runs {
        let tasks = db.list_tasks(run.id)?;
        let attempts = db.list_task_attempts(run.id)?;
        let sessions = db.list_agent_sessions(Some(run.id))?;
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
                "workerAgent": role_to_agent.get("worker"),
                "reviewerAgent": role_to_agent.get("reviewer"),
                "createdAt": run.created_at,
                "counts": TaskCounts::from_tasks(&tasks),
            }),
        });

        if let Some(planner) = &run.planner_agent {
            let target = if role_to_agent.get("planner") == Some(planner) {
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
            let (kind, source) = match session.role.as_str() {
                "worker" => ("works", format!("agent:{}", session.agent)),
                "reviewer" => ("reviews", format!("agent:{}", session.agent)),
                _ => continue,
            };
            edges.push(GraphEdge {
                id: format!("activity:{}", session.id),
                source,
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
        "roles": role_to_agent.len(),
    });

    Ok(GraphResponse {
        nodes,
        edges,
        metadata,
    })
}
