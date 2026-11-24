use crate::app::{ApiError, SharedState};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) fn record_service(
    state: &SharedState,
    kind: factory_db::ServiceNotification,
    source: &str,
) {
    if state._context.is_none() {
        return;
    }
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    if let Ok(db) = state.db.lock() {
        if db.notification_service(kind, &digest).is_err() {
            eprintln!(
                "ASF notification persistence failed; original operation outcome is unchanged"
            );
        }
    }
}

pub(crate) fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/notifications", get(list))
        .route("/api/notifications/:id", axum::routing::patch(update))
        .layer(DefaultBodyLimit::max(4096))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    before: Option<i64>,
    limit: Option<usize>,
    #[serde(default)]
    archived: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    revision: i64,
    read: bool,
    dismissed: bool,
}

async fn list(
    State(state): State<SharedState>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, ApiError> {
    let context = state._context.as_ref().ok_or_else(|| {
        ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "Registered project required",
        )
    })?;
    let project = context.project_id.to_string();
    tokio::task::spawn_blocking(move || {
        let db = state.db.lock().map_err(|_| {
            ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "Notification storage unavailable",
            )
        })?;
        let mut value = db
            .notification_page(page.before, page.limit.unwrap_or(50), page.archived)
            .map_err(notification_error)?;
        value["project_id"] = json!(project);
        Ok(Json(value))
    })
    .await
    .map_err(|_| {
        ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "Notification request interrupted",
        )
    })?
}

async fn update(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(update): Json<Update>,
) -> Result<Json<Value>, ApiError> {
    if state._context.is_none() || id <= 0 || update.revision <= 0 {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid notification identity",
        ));
    }
    tokio::task::spawn_blocking(move || {
        let db = state.db.lock().map_err(|_| {
            ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "Notification storage unavailable",
            )
        })?;
        db.notification_update(id, update.revision, update.read, update.dismissed)
            .map(Json)
            .map_err(notification_error)
    })
    .await
    .map_err(|_| {
        ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "Notification request interrupted",
        )
    })?
}

fn notification_error(error: factory_db::DbError) -> ApiError {
    let status = match error {
        factory_db::DbError::NotFound(_) => axum::http::StatusCode::NOT_FOUND,
        factory_db::DbError::Conflict { .. } => axum::http::StatusCode::CONFLICT,
        _ => axum::http::StatusCode::SERVICE_UNAVAILABLE,
    };
    ApiError::new(
        status,
        "Notification unavailable or changed. Refresh the project notification list.",
    )
}
