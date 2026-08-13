use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use factory_core::{Agents, Config, ConfigError};
use factory_db::FactoryDb;
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;

use crate::dashboard;
use crate::graph_workspace::{GraphWorkspace, GraphWorkspaceError, GraphWorkspaceResponse};
use crate::types::{
    AgentSessionResponse, GraphEdge, GraphNode, GraphResponse, RunDetail, RunSummary, TaskCounts,
};

pub struct ApiState {
    pub db: Mutex<FactoryDb>,
    pub root: PathBuf,
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

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/runs", get(list_runs))
        .route("/api/runs/:id", get(get_run))
        .route("/api/graph", get(get_graph))
        .route(
            "/api/graph/workspace",
            get(get_graph_workspace).put(put_graph_workspace),
        )
        .route("/api/agents", get(get_agents))
        .route("/api/agents/:agent/sessions", get(list_agent_sessions))
        .route("/api/sessions/:id", get(get_agent_session))
        .route("/api/sessions/:id/stream", get(stream_agent_session))
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
    Ok(Json(RunDetail { run, tasks }))
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
        session,
        working_directory,
        interactive: false,
    })
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
        total_tasks += tasks.len();
        nodes.push(GraphNode {
            id: format!("run:{}", run.id),
            kind: "run".into(),
            label: format!("Run #{}", run.id),
            meta: json!({
                "objective": run.objective,
                "status": run.status.as_str(),
                "plannerAgent": run.planner_agent,
                "createdAt": run.created_at,
                "counts": TaskCounts::from_tasks(&tasks),
            }),
        });

        if let Some(planner) = &run.planner_agent {
            let target = role_to_agent
                .iter()
                .find(|(_, agent)| *agent == planner)
                .map(|(role, _)| format!("role:{role}"))
                .unwrap_or_else(|| format!("agent:{planner}"));
            edges.push(GraphEdge {
                id: format!("run-use:{}:{target}", run.id),
                source: format!("run:{}", run.id),
                target,
                kind: "uses".into(),
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
                    "worktreePath": task.worktree_path,
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
