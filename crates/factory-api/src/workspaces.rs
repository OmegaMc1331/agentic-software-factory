use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::application::FactoryApplication;

pub(crate) struct WorkspaceStore {
    path: PathBuf,
    requests: Arc<tokio::sync::Semaphore>,
}

#[derive(Debug)]
pub(crate) enum WorkspaceError {
    Invalid,
    Missing,
    Conflict,
    Limit,
    Storage,
    Busy,
}

impl IntoResponse for WorkspaceError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "Invalid workspace document or identity."),
            Self::Missing => (StatusCode::NOT_FOUND, "Workspace not found. Its local recovery copy is retained."),
            Self::Conflict => (StatusCode::CONFLICT, "Another window saved this workspace. Reload or save an independent copy."),
            Self::Limit => (StatusCode::PAYLOAD_TOO_LARGE, "Workspace storage limit reached. Close or archive unused views."),
            Self::Storage => (StatusCode::SERVICE_UNAVAILABLE, "Workspace storage is unavailable. Existing data is retained; restore a verified backup offline."),
            Self::Busy => (StatusCode::TOO_MANY_REQUESTS, "Workspace storage is busy. Local presentation changes are retained; retry saving."),
        };
        (status, Json(json!({"error":message}))).into_response()
    }
}

impl From<rusqlite::Error> for WorkspaceError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

#[derive(Serialize)]
pub(crate) struct WorkspaceDocument {
    revision: u64,
    document: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveWorkspace {
    base_revision: u64,
    document: Value,
}

fn valid_id(id: &str) -> bool {
    id.len() == 34
        && id.starts_with("w_")
        && id[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl WorkspaceStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            requests: Arc::new(tokio::sync::Semaphore::new(8)),
        }
    }

    pub(crate) fn permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit, WorkspaceError> {
        self.requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| WorkspaceError::Busy)
    }

    pub(crate) fn connect(&self) -> Result<Connection, WorkspaceError> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let initial: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if initial < 2 {
            let transaction =
                rusqlite::Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
            let version: i64 = transaction.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if version == 0 {
                let tables: i64 = transaction.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )?;
                if tables != 0 {
                    return Err(WorkspaceError::Storage);
                }
                transaction.execute_batch("CREATE TABLE workspaces(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, document TEXT NOT NULL, updated_at TEXT NOT NULL);")?;
            } else if version != 1 && version != 2 {
                return Err(WorkspaceError::Storage);
            }
            if version < 2 {
                transaction.execute_batch("CREATE TABLE application_preferences(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,payload TEXT NOT NULL);PRAGMA user_version=2;")?;
            }
            transaction.commit()?;
        } else if initial != 2 {
            return Err(WorkspaceError::Storage);
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(connection)
    }

    fn read(&self, id: &str) -> Result<WorkspaceDocument, WorkspaceError> {
        if !valid_id(id) {
            return Err(WorkspaceError::Invalid);
        }
        let connection = self.connect()?;
        let row: Option<(u64, String)> = connection
            .query_row(
                "SELECT revision, document FROM workspaces WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (revision, text) = row.ok_or(WorkspaceError::Missing)?;
        Ok(WorkspaceDocument {
            revision,
            document: serde_json::from_str(&text).map_err(|_| WorkspaceError::Storage)?,
        })
    }

    fn save(&self, id: &str, update: SaveWorkspace) -> Result<WorkspaceDocument, WorkspaceError> {
        if !valid_id(id)
            || update.document.get("id").and_then(Value::as_str) != Some(id)
            || !matches!(
                update.document.get("version").and_then(Value::as_u64),
                Some(1..=3)
            )
        {
            return Err(WorkspaceError::Invalid);
        }
        let object = update.document.as_object().ok_or(WorkspaceError::Invalid)?;
        if object.keys().any(|key| {
            ![
                "id",
                "version",
                "openProjects",
                "sessions",
                "activeSessionId",
                "activeProjectId",
                "preferences",
            ]
            .contains(&key.as_str())
        }) {
            return Err(WorkspaceError::Invalid);
        }
        let serialized =
            serde_json::to_string(&update.document).map_err(|_| WorkspaceError::Invalid)?;
        if serialized.len() > 1_048_576 {
            return Err(WorkspaceError::Limit);
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<u64> = transaction
            .query_row("SELECT revision FROM workspaces WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        if previous.unwrap_or(0) != update.base_revision {
            return Err(WorkspaceError::Conflict);
        }
        if previous.is_none() {
            let count: i64 =
                transaction.query_row("SELECT count(*) FROM workspaces", [], |r| r.get(0))?;
            if count >= 64 {
                return Err(WorkspaceError::Limit);
            }
        }
        let revision = update
            .base_revision
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(WorkspaceError::Invalid)?;
        transaction.execute("INSERT INTO workspaces(id,revision,document,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document=excluded.document,updated_at=excluded.updated_at", params![id,revision,serialized,chrono::Utc::now().to_rfc3339()])?;
        transaction.commit()?;
        Ok(WorkspaceDocument {
            revision,
            document: update.document,
        })
    }
}

pub(crate) async fn list(
    State(app): State<Arc<FactoryApplication>>,
) -> Result<Json<Value>, WorkspaceError> {
    let permit = app.workspaces.permit()?;
    tokio::task::spawn_blocking(move || {
        let _permit=permit;
        let connection = app.workspaces.connect()?;
        let mut statement = connection.prepare("SELECT id, revision, updated_at FROM workspaces ORDER BY updated_at DESC, id LIMIT 64")?;
        let rows = statement.query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"revision":r.get::<_,u64>(1)?,"updatedAt":r.get::<_,String>(2)?})))?;
        let values: Result<Vec<_>, _> = rows.collect();
        Ok(Json(json!(values?)))
    }).await.map_err(|_| WorkspaceError::Storage)?
}

pub(crate) async fn read(
    State(app): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceDocument>, WorkspaceError> {
    let permit = app.workspaces.permit()?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        app.workspaces.read(&id)
    })
    .await
    .map_err(|_| WorkspaceError::Storage)?
    .map(Json)
}

pub(crate) async fn save(
    State(app): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
    Json(update): Json<SaveWorkspace>,
) -> Result<Json<WorkspaceDocument>, WorkspaceError> {
    let permit = app.workspaces.permit()?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        app.workspaces.save(&id, update)
    })
    .await
    .map_err(|_| WorkspaceError::Storage)?
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(id: &str) -> Value {
        json!({"version":2,"id":id,"sessions":[],"openProjects":[],"activeProjectId":null,"activeSessionId":null,"preferences":{"theme":"system"}})
    }
    #[test]
    fn workspace_persistence_conflicts_and_corruption_preserve_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("workspaces.sqlite3");
        let store = WorkspaceStore::new(path.clone());
        let id = "w_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(matches!(store.read(id), Err(WorkspaceError::Missing)));
        store
            .save(
                id,
                SaveWorkspace {
                    base_revision: 0,
                    document: document(id),
                },
            )
            .unwrap();
        assert!(matches!(
            store.save(
                id,
                SaveWorkspace {
                    base_revision: 0,
                    document: document(id)
                }
            ),
            Err(WorkspaceError::Conflict)
        ));
        let reopened = WorkspaceStore::new(path.clone());
        assert_eq!(reopened.read(id).unwrap().revision, 1);
        assert!(matches!(
            store.save(
                "../invalid",
                SaveWorkspace {
                    base_revision: 1,
                    document: document(id)
                }
            ),
            Err(WorkspaceError::Invalid)
        ));
        Connection::open(path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        assert!(matches!(store.read(id), Err(WorkspaceError::Storage)));
    }
}
