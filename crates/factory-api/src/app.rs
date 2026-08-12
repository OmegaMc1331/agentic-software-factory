use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use factory_db::FactoryDb;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::types::{RunDetail, RunSummary, TaskCounts};

pub struct ApiState {
    pub db: Mutex<FactoryDb>,
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

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/runs", get(list_runs))
        .route("/api/runs/:id", get(get_run))
        .with_state(state)
        .layer(CorsLayer::permissive())
}

pub async fn run_app(state: SharedState, port: u16) -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(state)).await
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
    Path(id): Path<i64>,
) -> Result<Json<RunDetail>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    let run = db.get_run(id)?.ok_or(ApiError::new(
        StatusCode::NOT_FOUND,
        format!("run {id} not found"),
    ))?;
    let tasks = db.list_tasks(id)?;
    Ok(Json(RunDetail { run, tasks }))
}
