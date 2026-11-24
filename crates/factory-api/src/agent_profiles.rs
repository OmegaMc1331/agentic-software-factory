use crate::app::SharedState;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_core::{Factory, FactoryError};
use factory_types::{AgentProfileInput, HandoffRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

static REQUESTS: Semaphore = Semaphore::const_new(16);

pub(crate) fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/agent-profiles", get(list).post(create))
        .route("/api/agent-profiles/providers", get(providers))
        .route(
            "/api/agent-profiles/:id",
            get(details).patch(update).delete(archive),
        )
        .route("/api/agent-profiles/:id/revisions", get(revisions))
        .route("/api/runs/:id/agent-handoffs", get(handoffs))
        .route("/api/runs/:run/tasks/:task/handoff", post(handoff))
        .route("/api/agent-sessions/:id/execution-snapshot", get(snapshot))
        .route(
            "/api/agent-sessions/:id/model-observation",
            get(observation),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
}

struct Error(FactoryError);
impl From<FactoryError> for Error {
    fn from(value: FactoryError) -> Self {
        Self(value)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            FactoryError::Db(factory_db::DbError::NotFound("agent profile")) => {
                (StatusCode::NOT_FOUND, "unknown_profile")
            }
            FactoryError::Db(factory_db::DbError::Busy(reason))
                if reason.starts_with("Handoff already pending") =>
            {
                (StatusCode::CONFLICT, "handoff_already_pending")
            }
            FactoryError::Db(factory_db::DbError::Busy(reason)) if reason.starts_with("Unsafe") => {
                (StatusCode::CONFLICT, "unsafe_handoff_boundary")
            }
            FactoryError::Db(factory_db::DbError::Busy(reason))
                if reason.starts_with("No active attempt")
                    || reason.starts_with("Dispatch already claimed") =>
            {
                (StatusCode::CONFLICT, "unsafe_handoff_boundary")
            }
            FactoryError::Db(factory_db::DbError::Busy(reason))
                if reason.starts_with("Task no longer eligible") =>
            {
                (StatusCode::CONFLICT, "task_no_longer_eligible")
            }
            FactoryError::AgentValidationFailed(reason)
                if reason.contains("Unknown trusted provider") =>
            {
                (StatusCode::BAD_REQUEST, "unsupported_provider")
            }
            FactoryError::AgentValidationFailed(reason)
                if reason.contains("model") || reason.contains("reasoning") =>
            {
                (StatusCode::BAD_REQUEST, "unsupported_model_configuration")
            }
            FactoryError::AgentValidationFailed(reason)
                if reason.contains("Unsupported or duplicate role") =>
            {
                (StatusCode::BAD_REQUEST, "unsupported_role")
            }
            FactoryError::PolicyBlocked(reason) if reason.contains("PROFILE_DISABLED:") => {
                (StatusCode::FORBIDDEN, "disabled_profile")
            }
            FactoryError::PolicyBlocked(reason) if reason.contains("ROLE_UNSUPPORTED:") => {
                (StatusCode::FORBIDDEN, "unsupported_role")
            }
            FactoryError::PolicyBlocked(reason) if reason.starts_with("PROJECT_MISMATCH:") => {
                (StatusCode::FORBIDDEN, "project_mismatch")
            }
            FactoryError::PolicyBlocked(reason) if reason.starts_with("HANDOFF_UNSUPPORTED:") => (
                StatusCode::CONFLICT,
                "active_operation_cannot_be_interrupted",
            ),
            FactoryError::PolicyBlocked(_) | FactoryError::PolicyViolation(_) => {
                (StatusCode::FORBIDDEN, "permission_denied")
            }
            FactoryError::AgentValidationFailed(_) => (StatusCode::BAD_REQUEST, "invalid_profile"),
            FactoryError::TaskNotFound(_)
            | FactoryError::RunNotFound(_)
            | FactoryError::Db(factory_db::DbError::NotFound(_)) => {
                (StatusCode::NOT_FOUND, "not_found")
            }
            FactoryError::Db(factory_db::DbError::Conflict { .. }) => {
                (StatusCode::CONFLICT, "stale_profile_revision")
            }
            FactoryError::Db(factory_db::DbError::Busy(_)) | FactoryError::RoutingOverride(_) => {
                (StatusCode::CONFLICT, "agent_operation_conflict")
            }
            FactoryError::AgentResourceExhausted(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "usage_limit_exceeded")
            }
            FactoryError::Agent(_) | FactoryError::AgentProcess(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable")
            }
            FactoryError::CapacityBusy(_) => (StatusCode::TOO_MANY_REQUESTS, "capacity_exhausted"),
            _ => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent_storage_or_configuration",
            ),
        };
        (
            status,
            Json(json!({"error":self.0.to_string(),"code":code})),
        )
            .into_response()
    }
}

async fn operation(
    state: SharedState,
    action: impl FnOnce(Factory, SharedState) -> Result<Value, FactoryError> + Send + 'static,
) -> Result<Json<Value>, Error> {
    if state._context.is_none() {
        return Err(Error(FactoryError::PolicyBlocked(
            "Project-scoped agent management requires a registered project".into(),
        )));
    }
    let permit = REQUESTS
        .try_acquire()
        .map_err(|_| Error(FactoryError::CapacityBusy("Agent API limit".into())))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let factory = state.runtime.factory()?;
        action(factory, state)
    })
    .await
    .map_err(|_| {
        Error(FactoryError::AgentValidationFailed(
            "Agent operation interrupted".into(),
        ))
    })?
    .map(Json)
    .map_err(Into::into)
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    before: Option<i64>,
    archived: Option<bool>,
}

async fn list(
    State(state): State<SharedState>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, Error> {
    operation(state, move |f, _| {
        let profiles: Vec<Value> = f
            .agent_profiles(page.archived.unwrap_or(false))?
            .into_iter()
            .map(|profile| {
                let active = f.capacity().inflight(&profile.id);
                let mut value = json!(profile);
                value["activeExecutions"] = json!(active);
                value
            })
            .collect();
        Ok(json!(profiles))
    })
    .await
}
async fn providers(State(state): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(state, |f, _| f.providers()).await
}
async fn details(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    operation(state, move |f, _| f.agent_profile_details(&id)).await
}
async fn create(
    State(state): State<SharedState>,
    Json(body): Json<AgentProfileInput>,
) -> Result<Json<Value>, Error> {
    operation(state, move |f, _| {
        Ok(json!(f.save_agent_profile(None, None, body)?))
    })
    .await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Update {
    expected_revision: i64,
    configuration: AgentProfileInput,
}
async fn update(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Update>,
) -> Result<Json<Value>, Error> {
    operation(state, move |f, _| {
        Ok(json!(f.save_agent_profile(
            Some(&id),
            Some(body.expected_revision),
            body.configuration
        )?))
    })
    .await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Revision {
    expected_revision: i64,
}
async fn archive(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Revision>,
) -> Result<Json<Value>, Error> {
    operation(state, move |f, _| {
        Ok(json!(f.archive_agent_profile(&id, body.expected_revision)?))
    })
    .await
}
async fn revisions(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, Error> {
    operation(state, move |_, s| {
        let db = s
            .db
            .lock()
            .map_err(|_| FactoryError::AgentValidationFailed("Storage lock unavailable".into()))?;
        Ok(json!(db.agent_profile_revisions(&id, page.before)?))
    })
    .await
}
async fn handoff(
    State(state): State<SharedState>,
    Path((run, task)): Path<(i64, i64)>,
    Json(body): Json<HandoffRequest>,
) -> Result<Json<Value>, Error> {
    operation(state, move |f, _| {
        Ok(json!(f.request_agent_handoff(run, task, body)?))
    })
    .await
}
async fn handoffs(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Query(page): Query<HandoffPage>,
) -> Result<Json<Value>, Error> {
    operation(state, move |_, s| {
        let db = s
            .db
            .lock()
            .map_err(|_| FactoryError::AgentValidationFailed("Storage lock unavailable".into()))?;
        db.get_run(id)?.ok_or(FactoryError::RunNotFound(id))?;
        db.reconcile_handoffs(id)?;
        Ok(json!(db.handoffs_before(id, page.before.as_deref())?))
    })
    .await
}
async fn snapshot(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, Error> {
    operation(state, move |_, s| {
        let db = s
            .db
            .lock()
            .map_err(|_| FactoryError::AgentValidationFailed("Storage lock unavailable".into()))?;
        db.get_agent_session(id)?
            .ok_or(factory_db::DbError::NotFound("agent session"))?;
        Ok(json!(db.execution_snapshot(id)?))
    })
    .await
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffPage {
    before: Option<String>,
}
async fn observation(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, Error> {
    operation(state, move |_, s| {
        let db = s
            .db
            .lock()
            .map_err(|_| FactoryError::AgentValidationFailed("Storage lock unavailable".into()))?;
        db.get_agent_session(id)?
            .ok_or(factory_db::DbError::NotFound("agent session"))?;
        Ok(json!(db.model_observation(id)?))
    })
    .await
}
