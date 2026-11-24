use crate::app::SharedState;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{
    sse::{Event, KeepAlive, Sse},
    IntoResponse, Response,
};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_conversation::{ConversationService, Submission};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio_stream::wrappers::ReceiverStream;

static REQUESTS: Semaphore = Semaphore::const_new(16);
static STREAMS: Semaphore = Semaphore::const_new(32);
pub(crate) fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/conversations", get(list).post(create))
        .route("/api/conversations/status", get(status))
        .route("/api/conversations/:id", get(read).patch(update))
        .route(
            "/api/conversations/:id/messages",
            get(messages).post(submit),
        )
        .route("/api/conversations/:id/missions", get(missions))
        .route("/api/conversations/:id/events", get(events))
        .route(
            "/api/conversations/:id/commands/:command/cancel",
            post(cancel),
        )
        .layer(DefaultBodyLimit::max(128 * 1024))
}
struct Error(factory_conversation::Error);
impl From<factory_conversation::Error> for Error {
    fn from(error: factory_conversation::Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use factory_conversation::Error::*;
        let (status, code) = match &self.0 {
            Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_conversation_action"),
            Missing => (StatusCode::NOT_FOUND, "unknown_conversation"),
            Conflict => (StatusCode::CONFLICT, "conversation_conflict"),
            Limit => (StatusCode::TOO_MANY_REQUESTS, "conversation_limit"),
            Context => (StatusCode::FORBIDDEN, "context_forbidden"),
            Storage => (StatusCode::SERVICE_UNAVAILABLE, "conversation_storage"),
            Operation(_) => (StatusCode::CONFLICT, "conversation_operation"),
        };
        (
            status,
            Json(json!({"error":self.0.to_string(),"code":code})),
        )
            .into_response()
    }
}
fn service(state: &SharedState) -> Result<Arc<ConversationService>, Error> {
    state
        ._context
        .as_ref()
        .map(|c| c.conversations.clone())
        .ok_or(Error(factory_conversation::Error::Missing))
}
async fn operation<T: serde::Serialize + Send + 'static>(
    state: SharedState,
    action: impl FnOnce(Arc<ConversationService>) -> factory_conversation::Result<T> + Send + 'static,
) -> Result<Json<T>, Error> {
    let service = service(&state)?;
    let permit = REQUESTS
        .try_acquire()
        .map_err(|_| Error(factory_conversation::Error::Limit))?;
    tokio::task::spawn_blocking(move || {
        let _lease = (state, permit);
        action(service)
    })
    .await
    .map_err(|_| Error(factory_conversation::Error::Storage))?
    .map(Json)
    .map_err(Into::into)
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Page {
    before: Option<i64>,
    limit: Option<usize>,
    cursor: Option<String>,
}
fn limit(value: Option<usize>) -> Result<usize, Error> {
    let n = value.unwrap_or(50);
    if !(1..=100).contains(&n) {
        return Err(Error(factory_conversation::Error::Invalid(
            "Page limit must be 1–100.".into(),
        )));
    }
    Ok(n)
}
async fn list(
    State(state): State<SharedState>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<factory_types::Conversation>>, Error> {
    let n = limit(page.limit)?;
    operation(state, move |s| s.list(page.cursor.as_deref(), n)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    title: String,
}
async fn create(
    State(state): State<SharedState>,
    Json(body): Json<Create>,
) -> Result<Json<factory_types::Conversation>, Error> {
    operation(state, move |s| s.create(&body.title)).await
}
async fn read(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<factory_types::Conversation>, Error> {
    operation(state, move |s| s.get(&id)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    title: String,
    status: String,
    revision: i64,
}
async fn update(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Update>,
) -> Result<Json<factory_types::Conversation>, Error> {
    operation(state, move |s| {
        s.update(&id, &body.title, &body.status, body.revision)
    })
    .await
}
async fn messages(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<factory_types::ConversationMessage>>, Error> {
    let n = limit(page.limit)?;
    operation(state, move |s| s.messages(&id, page.before, n)).await
}
async fn submit(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    binding: Option<axum::Extension<crate::presence::ValidatedBrowserBindings>>,
    Json(body): Json<Submission>,
) -> Result<Response, Error> {
    if let Some(binding) = binding {
        let project = state
            ._context
            .as_ref()
            .ok_or(Error(factory_conversation::Error::Missing))?
            .project_id
            .as_str();
        if !binding
            .0
             .0
            .iter()
            .any(|b| b.project_id == project && b.session_id == body.workspace_session_id)
        {
            return Err(Error(factory_conversation::Error::Context));
        }
    }
    let Json(command) = operation(state, move |s| s.submit(&id, body)).await?;
    Ok((StatusCode::ACCEPTED, Json(command)).into_response())
}
async fn missions(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Value>>, Error> {
    operation(state, move |s| s.missions(&id)).await
}
async fn status(State(state): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(state, move |s| Ok(s.status())).await
}
async fn cancel(
    State(state): State<SharedState>,
    Path((id, command)): Path<(String, i64)>,
) -> Result<Json<Value>, Error> {
    operation(state, move |s| {
        s.cancel(&id, command)?;
        Ok(json!({"accepted":true,"completed":false}))
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: Option<i64>,
}
async fn events(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    let service = service(&state)?;
    service.get(&id)?;
    let permit = STREAMS
        .try_acquire()
        .map_err(|_| Error(factory_conversation::Error::Limit))?;
    let mut after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(cursor.after.unwrap_or(0));
    if after < 0 {
        return Err(Error(factory_conversation::Error::Invalid(
            "Invalid event cursor.".into(),
        )));
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);
    tokio::spawn(async move {
        let _leases = (permit, state);
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {_=tx.closed()=>break,_=interval.tick()=>{}}
            let s = service.clone();
            let conversation = id.clone();
            let result = tokio::task::spawn_blocking(move || {
                s.observe(&conversation)?;
                s.events(&conversation, after, 100)
            })
            .await;
            let entries = match result {
                Ok(Ok(entries)) => entries,
                _ => {
                    let _ = tx.try_send(Ok(Event::default()
                        .event("resync")
                        .data("Conversation replay unavailable; reload durable history.")));
                    break;
                }
            };
            if entries.is_empty() {
                continue;
            }
            for entry in entries {
                after = entry.id;
                let event = Event::default()
                    .event("conversation")
                    .id(entry.id.to_string())
                    .data(serde_json::to_string(&entry).unwrap_or_default());
                if tx.try_send(Ok(event)).is_err() {
                    return;
                }
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response())
}
