use crate::app::SharedState;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_core::{Factory, FactoryError};
use factory_memory::Error as MemoryError;
use factory_types::memory::*;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

static REQUESTS: Semaphore = Semaphore::const_new(16);
static EXTRACTIONS: Semaphore = Semaphore::const_new(2);

fn database(
    state: &SharedState,
) -> Result<std::sync::MutexGuard<'_, factory_db::FactoryDb>, FactoryError> {
    state
        .db
        .lock()
        .map_err(|_| FactoryError::AgentValidationFailed("Memory database lock unavailable".into()))
}

pub(crate) fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/memory", get(list).post(create))
        .route("/api/memory/search", post(search))
        .route("/api/memory/retrieve", post(retrieve))
        .route("/api/memory/settings", get(settings).put(save_settings))
        .route("/api/memory/index/rebuild", post(rebuild))
        .route("/api/memory/sources/resolve", post(source))
        .route("/api/memory/extract", post(extract))
        .route("/api/memory/extractions", get(extractions))
        .route("/api/memory/candidates", get(candidates))
        .route("/api/memory/candidates/:id/accept", post(accept))
        .route("/api/memory/candidates/:id/reject", post(reject))
        .route("/api/memory/usage", get(usage))
        .route("/api/memory/:id", get(details).patch(update).delete(delete))
        .route("/api/memory/:id/revisions", get(revisions))
        .route("/api/memory/:id/provenance", get(provenance))
        .route("/api/memory/:id/supersede", post(supersede))
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
            FactoryError::Memory(MemoryError::Invalid(_)) => {
                (StatusCode::BAD_REQUEST, "invalid_memory")
            }
            FactoryError::Memory(MemoryError::Source) => {
                (StatusCode::FORBIDDEN, "invalid_memory_source")
            }
            FactoryError::Memory(MemoryError::Sensitive) => {
                (StatusCode::BAD_REQUEST, "sensitive_memory_content")
            }
            FactoryError::Memory(MemoryError::Policy)
            | FactoryError::PolicyBlocked(_)
            | FactoryError::Suspended => (StatusCode::FORBIDDEN, "memory_policy_denied"),
            FactoryError::Memory(MemoryError::Limit) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "memory_limit")
            }
            FactoryError::Memory(MemoryError::Duplicate(_)) => {
                (StatusCode::CONFLICT, "memory_duplicate")
            }
            FactoryError::Db(factory_db::DbError::NotFound(_))
            | FactoryError::Memory(MemoryError::Db(factory_db::DbError::NotFound(_))) => {
                (StatusCode::NOT_FOUND, "memory_not_found")
            }
            FactoryError::Db(factory_db::DbError::Conflict { .. })
            | FactoryError::Memory(MemoryError::Db(factory_db::DbError::Conflict { .. })) => {
                (StatusCode::CONFLICT, "stale_memory_revision")
            }
            FactoryError::Db(factory_db::DbError::Busy(_))
            | FactoryError::Memory(MemoryError::Db(factory_db::DbError::Busy(_)))
            | FactoryError::CapacityBusy(_) => {
                (StatusCode::TOO_MANY_REQUESTS, "memory_busy_or_limit")
            }
            FactoryError::AgentResourceExhausted(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "usage_limit_exceeded")
            }
            FactoryError::Agent(_) | FactoryError::AgentProcess(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "memory_provider_unavailable",
            ),
            _ => (
                StatusCode::SERVICE_UNAVAILABLE,
                "memory_storage_or_execution_unavailable",
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
    action: impl FnOnce(&Factory, SharedState) -> Result<Value, FactoryError> + Send + 'static,
) -> Result<Json<Value>, Error> {
    if state._context.is_none() {
        return Err(
            FactoryError::PolicyBlocked("Registered project required for memory".into()).into(),
        );
    }
    let permit = REQUESTS
        .try_acquire()
        .map_err(|_| Error(FactoryError::CapacityBusy("Memory requests".into())))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let f = state.runtime.factory()?;
        let service = f.project_memory()?;
        let value = action(&f, state)?;
        service.screen(&serde_json::to_string(&value).map_err(factory_db::DbError::from)?)?;
        Ok::<Value, FactoryError>(value)
    })
    .await
    .map_err(|_| {
        Error(FactoryError::AgentValidationFailed(
            "Memory operation interrupted".into(),
        ))
    })?
    .map(Json)
    .map_err(Into::into)
}

async fn list(
    State(s): State<SharedState>,
    Query(q): Query<MemorySearch>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| Ok(json!(f.project_memory()?.search(&q)?))).await
}
async fn search(
    State(s): State<SharedState>,
    Json(q): Json<MemorySearch>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| Ok(json!(f.project_memory()?.search(&q)?))).await
}
async fn details(
    State(s): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| Ok(json!(f.project_memory()?.get(&id)?))).await
}
async fn provenance(
    State(s): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| {
        Ok(json!(f.project_memory()?.get(&id)?.provenance))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Write {
    memory: MemoryInput,
    expected_revision: Option<i64>,
    #[serde(default)]
    reason: String,
}
async fn create(
    State(s): State<SharedState>,
    Json(body): Json<Write>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| {
        if body.expected_revision.is_some() {
            return Err(
                MemoryError::Invalid("A new memory has no expected revision".into()).into(),
            );
        }
        Ok(json!(f
            .project_memory()?
            .create(&body.memory, &body.reason)?))
    })
    .await
}
async fn update(
    State(s): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Write>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| {
        Ok(json!(f.project_memory()?.update(
            &id,
            body.expected_revision
                .ok_or_else(|| MemoryError::Invalid("Expected revision is required".into()))?,
            &body.memory,
            &body.reason
        )?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Revision {
    expected_revision: i64,
}
async fn delete(
    State(s): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Revision>,
) -> Result<Json<Value>, Error> {
    operation(s,move|f,_|{f.project_memory()?.delete(&id,body.expected_revision)?;Ok(json!({"deleted":true,"notice":"Memory, revisions, search entries and related queued claims removed. Invocation reference hashes, independent mission logs and existing backups are retained."}))}).await
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Page {
    before: Option<i64>,
    session_id: Option<i64>,
}
async fn revisions(
    State(s): State<SharedState>,
    Path(id): Path<String>,
    Query(q): Query<Page>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_f, state| {
        Ok(json!(database(&state)?.memory_revisions(
            &state
                ._context
                .as_ref()
                .expect("bound project")
                .project_id
                .to_string(),
            &id,
            q.before
        )?))
    })
    .await
}
async fn settings(State(s): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| Ok(json!(f.project_memory()?.settings()?))).await
}
async fn save_settings(
    State(s): State<SharedState>,
    Json(body): Json<MemorySettings>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| {
        Ok(json!(f.project_memory()?.save_settings(&body)?))
    })
    .await
}
async fn rebuild(State(s): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(s,move|_f,state|Ok(json!({"indexed":database(&state)?.memory_rebuild_index(&state._context.as_ref().expect("bound project").project_id.to_string())?}))).await
}
async fn source(
    State(s): State<SharedState>,
    Json(body): Json<MemorySource>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| {
        Ok(json!(
            f.project_memory()?.resolve_source(&body, None)?.provenance
        ))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Retrieval {
    query: String,
}
async fn retrieve(
    State(s): State<SharedState>,
    Json(body): Json<Retrieval>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| {
        Ok(json!(f.project_memory()?.retrieve(
            &body.query,
            "interactive",
            None
        )?))
    })
    .await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Extraction {
    sources: Vec<MemorySource>,
}
async fn extract(
    State(s): State<SharedState>,
    Json(body): Json<Extraction>,
) -> Result<Json<Value>, Error> {
    let permit = EXTRACTIONS
        .try_acquire()
        .map_err(|_| Error(FactoryError::CapacityBusy("Memory extractors".into())))?;
    let resource = s
        .runtime
        .reserve_operation()
        .map_err(|_| Error(FactoryError::CapacityBusy("Application operations".into())))?;
    operation(s, move |f, _| {
        let _permit = permit;
        f.extract_project_memory(&body.sources, resource.cancellation())
    })
    .await
}
async fn candidates(
    State(s): State<SharedState>,
    Query(q): Query<Page>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_f, state| {
        let (items, next) = database(&state)?.memory_candidates(
            &state
                ._context
                .as_ref()
                .expect("bound project")
                .project_id
                .to_string(),
            q.before,
        )?;
        Ok(json!({"items":items,"nextCursor":next}))
    })
    .await
}
async fn extractions(State(s): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(s, move |_f, state| {
        Ok(json!(database(&state)?.memory_extractions(
            &state
                ._context
                .as_ref()
                .expect("bound project")
                .project_id
                .to_string()
        )?))
    })
    .await
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Acceptance {
    memory: Option<MemoryInput>,
}
async fn accept(
    State(s): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Acceptance>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, _| {
        Ok(json!(f
            .project_memory()?
            .accept_candidate(&id, body.memory.as_ref())?))
    })
    .await
}
async fn reject(
    State(s): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_f, state| {
        Ok(json!(database(&state)?.memory_decide_candidate(
            &state
                ._context
                .as_ref()
                .expect("bound project")
                .project_id
                .to_string(),
            &id,
            None
        )?))
    })
    .await
}
async fn usage(State(s): State<SharedState>, Query(q): Query<Page>) -> Result<Json<Value>, Error> {
    operation(s, move |_f, state| {
        Ok(json!(database(&state)?.memory_usage(
            &state
                ._context
                .as_ref()
                .expect("bound project")
                .project_id
                .to_string(),
            q.session_id,
            q.before
        )?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Supersede {
    expected_revision: i64,
    target_id: String,
    target_revision: i64,
}
async fn supersede(
    State(s): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Supersede>,
) -> Result<Json<Value>, Error> {
    operation(s, move |f, state| {
        let service = f.project_memory()?;
        let old = service.get(&id)?;
        let target = service.get(&body.target_id)?;
        if target.status == MemoryStatus::Stale {
            return Err(MemoryError::Invalid(
                "Refresh changed source evidence before confirming this claim".into(),
            )
            .into());
        }
        let _ = old;
        Ok(json!(database(&state)?.memory_resolve(
            &state
                ._context
                .as_ref()
                .expect("bound project")
                .project_id
                .to_string(),
            &id,
            &body.target_id,
            body.expected_revision,
            body.target_revision
        )?))
    })
    .await
}
