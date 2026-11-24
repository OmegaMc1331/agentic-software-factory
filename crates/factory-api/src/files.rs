use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_files::{Draft, FileError, FileRef, FileService, SearchRequest};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Semaphore;

use crate::app::SharedState;

static FILE_REQUESTS: Semaphore = Semaphore::const_new(8);

pub(crate) fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/files/roots", get(roots))
        .route("/api/files/tree", get(tree))
        .route("/api/files/content", get(read).put(save))
        .route("/api/files/entries", post(create).delete(delete))
        .route("/api/files/move", post(move_entry))
        .route("/api/files/git-head", get(git_head))
        .route("/api/files/changes", post(changes))
        .route("/api/files/search", post(search))
        .route("/api/files/search/:id/cancel", post(cancel_search))
        .route("/api/files/recovery", get(recovery))
        .route("/api/files/recovery/:id/content", get(recovery_content))
        .route("/api/files/recovery/:id/purge", post(purge_recovery))
        .route("/api/files/recovery/:id/restore", post(restore))
        .route(
            "/api/editor/drafts/:id",
            get(draft).put(save_draft).delete(discard_draft),
        )
        .layer(DefaultBodyLimit::max(6 * 1024 * 1024))
}

struct Error(FileError);
impl From<FileError> for Error {
    fn from(value: FileError) -> Self {
        Self(value)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.0 {
            FileError::Invalid => StatusCode::BAD_REQUEST,
            FileError::Forbidden | FileError::ReadOnly | FileError::Permission => {
                StatusCode::FORBIDDEN
            }
            FileError::NotFound | FileError::UnknownRoot | FileError::GitUnavailable => {
                StatusCode::NOT_FOUND
            }
            FileError::Exists
            | FileError::Conflict
            | FileError::DraftConflict
            | FileError::DirectoryNotEmpty
            | FileError::Uncertain => StatusCode::CONFLICT,
            FileError::Limit => StatusCode::PAYLOAD_TOO_LARGE,
            FileError::Busy => StatusCode::TOO_MANY_REQUESTS,
            FileError::Unsupported => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            FileError::Storage | FileError::Io => StatusCode::SERVICE_UNAVAILABLE,
        };
        (
            status,
            Json(json!({"error":self.0.to_string(),"code":self.0.code()})),
        )
            .into_response()
    }
}

fn service(state: &SharedState) -> Result<Arc<FileService>, Error> {
    state
        ._context
        .as_ref()
        .map(|context| context.files.clone())
        .ok_or(Error(FileError::Forbidden))
}

async fn operation<T: Send + 'static>(
    state: SharedState,
    action: impl FnOnce(&FileService) -> factory_files::Result<T> + Send + 'static,
) -> Result<Json<T>, Error> {
    let service = service(&state)?;
    let project = service.acquire()?;
    let global = FILE_REQUESTS
        .try_acquire()
        .map_err(|_| Error(FileError::Busy))?;
    tokio::task::spawn_blocking(move || {
        let _leases = (project, global, state);
        action(&service).map(Json).map_err(Error)
    })
    .await
    .map_err(|_| Error(FileError::Io))?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileQuery {
    root_id: String,
    path: String,
}
impl FileQuery {
    fn reference(self, service: &FileService) -> FileRef {
        FileRef {
            project_id: service.project_id.clone(),
            root_id: self.root_id,
            path: self.path,
        }
    }
}

async fn roots(State(state): State<SharedState>) -> Result<Json<serde_json::Value>, Error> {
    operation(state, |service| {
        let (roots,warning)=match service.roots(){Ok(roots)=>(roots,None),Err(FileError::GitUnavailable)=>(vec![service.primary_root()],Some("Git worktree discovery is unavailable. Primary repository files remain available.")),Err(error)=>return Err(error)};
        Ok(json!({"roots":roots,"warning":warning,"limits":service.limits,"worktree_policy":"read_only"}))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeQuery {
    root_id: String,
    #[serde(default)]
    path: String,
    after: Option<String>,
    #[serde(default)]
    hidden: bool,
}
async fn tree(
    State(state): State<SharedState>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<factory_files::DirectoryPage>, Error> {
    operation(state, move |service| {
        service.list(
            &query.root_id,
            &query.path,
            query.after.as_deref(),
            query.hidden,
        )
    })
    .await
}
async fn read(
    State(state): State<SharedState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<factory_files::FileSnapshot>, Error> {
    operation(state, move |service| {
        service.read(&query.reference(service))
    })
    .await
}
async fn git_head(
    State(state): State<SharedState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<factory_files::FileSnapshot>, Error> {
    operation(state, move |service| {
        service.git_head(&query.reference(service))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    file: FileRef,
    expected_revision: String,
    content: String,
}
async fn save(
    State(state): State<SharedState>,
    Json(body): Json<Save>,
) -> Result<Json<factory_files::FileSnapshot>, Error> {
    let notification_state = state.clone();
    operation(state, move |service| {
        let result = service.save(&body.file, &body.expected_revision, &body.content);
        if matches!(&result, Err(FileError::Conflict)) {
            crate::notifications::record_service(
                &notification_state,
                factory_db::ServiceNotification::SaveConflict,
                &format!(
                    "{}\0{}\0{}",
                    body.file.root_id, body.file.path, body.expected_revision
                ),
            );
        }
        result
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    file: FileRef,
    #[serde(default)]
    directory: bool,
}
async fn create(
    State(state): State<SharedState>,
    Json(body): Json<Create>,
) -> Result<Json<serde_json::Value>, Error> {
    operation(state, move |service| {
        service.create(&body.file, body.directory)?;
        Ok(json!({"created":body.file}))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    file: FileRef,
    expected_revision: String,
    destination: String,
}
async fn move_entry(
    State(state): State<SharedState>,
    Json(body): Json<Move>,
) -> Result<Json<factory_files::MoveResult>, Error> {
    operation(state, move |service| {
        service.move_entry(&body.file, &body.expected_revision, &body.destination)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    file: FileRef,
    expected_revision: String,
    confirmed: bool,
}
async fn delete(
    State(state): State<SharedState>,
    Json(body): Json<Delete>,
) -> Result<Json<factory_files::Recovery>, Error> {
    operation(state, move |service| {
        service.delete(&body.file, &body.expected_revision, body.confirmed)
    })
    .await
}
async fn recovery(
    State(state): State<SharedState>,
) -> Result<Json<Vec<factory_files::Recovery>>, Error> {
    operation(state, |service| service.recovery()).await
}
async fn restore(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<FileRef>, Error> {
    operation(state, move |service| service.restore_deleted(&id)).await
}

async fn recovery_content(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<factory_files::FileSnapshot>, Error> {
    operation(state, move |service| service.recovery_content(&id)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PurgeRecovery {
    confirmed: bool,
}
async fn purge_recovery(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<PurgeRecovery>,
) -> Result<Json<serde_json::Value>, Error> {
    operation(state, move |service| {
        service.purge_recovery(&id, body.confirmed)?;
        Ok(json!({"purged":id}))
    })
    .await
}

async fn draft(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<factory_files::DraftEnvelope>, Error> {
    operation(state, move |service| service.draft(&id)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveDraft {
    base_revision: u64,
    document: Draft,
}
async fn save_draft(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<SaveDraft>,
) -> Result<Json<factory_files::DraftEnvelope>, Error> {
    operation(state, move |service| {
        if id != body.document.id {
            return Err(FileError::Invalid);
        }
        service.save_draft(body.document, body.base_revision)
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Discard {
    revision: u64,
    confirmed: bool,
}
async fn discard_draft(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Discard>,
) -> Result<Json<serde_json::Value>, Error> {
    operation(state, move |service| {
        if !body.confirmed {
            return Err(FileError::Forbidden);
        }
        service.discard_draft(&id, body.revision)?;
        Ok(json!({"discarded":id}))
    })
    .await
}

async fn search(
    State(state): State<SharedState>,
    Json(body): Json<SearchRequest>,
) -> Result<Json<factory_files::SearchResponse>, Error> {
    operation(state, move |service| {
        let cancel = service.begin_search(&body.id)?;
        service.search(&body, &cancel)
    })
    .await
}
async fn cancel_search(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Error> {
    service(&state)?.cancel_search(&id)?;
    Ok(Json(json!({"cancelled":id})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeItem {
    file: FileRef,
    revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Changes {
    items: Vec<ChangeItem>,
}
async fn changes(
    State(state): State<SharedState>,
    Json(body): Json<Changes>,
) -> Result<Json<serde_json::Value>, Error> {
    if body.items.len() > 8 {
        return Err(Error(FileError::Limit));
    }
    operation(state,move|service|{
        let items=body.items.into_iter().map(|item|match service.read(&item.file){
            Ok(snapshot)=>json!({"file":snapshot.file,"status":if snapshot.revision==item.revision{"unchanged"}else{"changed"},"revision":snapshot.revision}),
            Err(error)=>json!({"file":item.file,"status":error.code()}),
        }).collect::<Vec<_>>();Ok(json!({"changes":items,"complete":true}))
    }).await
}
