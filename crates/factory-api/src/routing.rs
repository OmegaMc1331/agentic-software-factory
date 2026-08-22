//! Read-only routing explanations plus the per-task manual routing override.
//!
//! The scheduler owns routing decisions — nothing here selects an agent for a
//! dispatch. `GET routing-preview` and `GET routing-decisions` expose why an
//! agent was (or will be) chosen, and `PUT routing` pins or clears a task's
//! agent with full eligibility validation up front.

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use factory_core::FactoryError;
use factory_types::{RoutingDecision, TaskState};
use serde::{Deserialize, Serialize};

use crate::app::{ApiError, SharedState};

fn factory_error(error: FactoryError) -> ApiError {
    let status = match &error {
        FactoryError::TaskNotFound(_) | FactoryError::RunNotFound(_) => StatusCode::NOT_FOUND,
        FactoryError::NotInitialized => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    };
    ApiError::new(status, error.to_string())
}

/// `GET /api/tasks/:id/routing-preview` — what the router would do for this
/// task right now. Informational: the real selection happens at dispatch time
/// because capacity and history change.
pub(crate) async fn routing_preview(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<factory_types::RoutingPreview>, ApiError> {
    // The preview path owns no state; opening a Factory gives it the same
    // config, database, and capacity view the scheduler would use.
    let factory = factory_core::Factory::open(&state.root).map_err(factory_error)?;
    let preview = factory.routing_preview(id).map_err(factory_error)?;
    Ok(Json(preview))
}

/// `GET /api/tasks/:id/routing-decisions` — the durable audit trail of every
/// dispatch recorded for the task, oldest first.
pub(crate) async fn routing_decisions(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Vec<RoutingDecision>>, ApiError> {
    let db = state.db.lock().expect("db mutex poisoned");
    let task = db.get_task(id)?.ok_or(ApiError::new(
        StatusCode::NOT_FOUND,
        format!("task {id} not found"),
    ))?;
    let decisions = db.list_routing_decisions_for_task(task.id)?;
    Ok(Json(decisions))
}

#[derive(Deserialize)]
pub(crate) struct RoutingOverrideRequest {
    /// The agent to pin for this task, or `null` to return to automatic
    /// routing.
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingOverrideResponse {
    task_id: i64,
    agent_override: Option<String>,
}

/// `PUT /api/tasks/:id/routing` — pin (`{"agent": "codex"}`) or clear
/// (`{"agent": null}`) the task's manual routing override.
///
/// The pin must reference an existing agent assigned to the task's role;
/// policy and availability are re-checked at dispatch time. A task that is
/// already running (or finished) cannot be re-pinned.
pub(crate) async fn set_routing_override(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
    body: Result<Json<RoutingOverrideRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<RoutingOverrideResponse>, ApiError> {
    let request = body
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON body"))?
        .0;
    let agent = request.agent.filter(|agent| !agent.trim().is_empty());
    let agent = agent.as_deref().map(str::trim);

    let task = {
        let db = state.db.lock().expect("db mutex poisoned");
        let task = db.get_task(id)?.ok_or(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("task {id} not found"),
        ))?;
        match task.state {
            TaskState::Pending | TaskState::Ready | TaskState::Blocked | TaskState::Failed => {}
            other => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    format!(
                        "task {id} is {} and can no longer be pinned; only future work \
                         can carry a manual routing override",
                        other.as_str()
                    ),
                ));
            }
        }
        task
    };

    if let Some(pinned) = agent {
        let config = factory_core::Config::load(&state.root)?;
        let role = task
            .role
            .clone()
            .unwrap_or_else(|| factory_core::WORKER.to_string());
        if !config.agents.contains_key(pinned) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("unknown agent '{pinned}'"),
            ));
        }
        let assigned = config
            .assignments_for(&role)
            .iter()
            .any(|assignment| assignment.agent == pinned);
        if !assigned {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("agent '{pinned}' is not assigned to role '{role}'"),
            ));
        }
    }

    let db = state.db.lock().expect("db mutex poisoned");
    db.set_task_agent_override(id, agent)?;
    Ok(Json(RoutingOverrideResponse {
        task_id: id,
        agent_override: agent.map(str::to_string),
    }))
}
