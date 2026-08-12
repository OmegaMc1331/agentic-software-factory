use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use factory_core::{Agents, Config, ConfigError};
use factory_db::FactoryDb;
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

use crate::types::{GraphEdge, GraphNode, GraphResponse, RunDetail, RunSummary, TaskCounts};

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

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/runs", get(list_runs))
        .route("/api/runs/:id", get(get_run))
        .route("/api/graph", get(get_graph))
        .route("/api/agents", get(get_agents))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/*rest", get(api_not_found))
        .fallback_service(dashboard_service(&state.root))
        .with_state(state)
}

async fn api_not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "unknown endpoint")
}

pub async fn run_app(state: SharedState, port: u16) -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(state)).await
}

fn dashboard_service(root: &Path) -> Router {
    let dir = find_dashboard_dir(root).unwrap_or_else(dashboard_stub_dir);
    Router::new().fallback_service(
        ServeDir::new(&dir).not_found_service(ServeFile::new(dir.join("index.html"))),
    )
}

fn dashboard_stub_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("factory-dashboard-stub");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><meta charset=\"utf-8\"><title>Agentic Software Factory</title>\
         <style>body{font-family:system-ui,sans-serif;background:#0f1115;color:#d7dce3;margin:48px auto;max-width:560px;line-height:1.6}</style>\
         <h1>Dashboard not built</h1>\
         <p>The dashboard has not been built yet. From the project root run:</p>\
         <pre>cd apps/dashboard\nnpm install\nnpm run build</pre>\
         <p>Then restart <code>factory start</code>.</p>",
    )
    .ok();
    dir
}

fn find_dashboard_dir(start: &Path) -> Option<PathBuf> {
    let mut cursor = Some(start.to_path_buf());
    while let Some(dir) = cursor {
        let candidate = dir.join("apps").join("dashboard").join("dist");
        if candidate.join("index.html").is_file() {
            return Some(candidate);
        }
        cursor = dir.parent().map(Path::to_path_buf);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let mut cursor = Some(parent.to_path_buf());
            while let Some(dir) = cursor {
                let candidate = dir.join("apps").join("dashboard").join("dist");
                if candidate.join("index.html").is_file() {
                    return Some(candidate);
                }
                cursor = dir.parent().map(Path::to_path_buf);
            }
        }
    }
    None
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

async fn get_graph(State(state): State<SharedState>) -> Result<Json<GraphResponse>, ApiError> {
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
            source: format!("role:{role}"),
            target: format!("agent:{agent}"),
            kind: "binds".into(),
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
                source: format!("run:{}", run.id),
                target,
                kind: "uses".into(),
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
                source: format!("run:{}", run.id),
                target: format!("task:{}", task.id),
                kind: "contains".into(),
            });
            task_dependencies.push((format!("task:{}", task.id), task.dependencies.clone()));
        }
    }

    for (task_id, dependencies) in task_dependencies {
        for dependency in dependencies {
            edges.push(GraphEdge {
                source: format!("task:{dependency}"),
                target: task_id.clone(),
                kind: "depends".into(),
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

    Ok(Json(GraphResponse {
        nodes,
        edges,
        metadata,
    }))
}
