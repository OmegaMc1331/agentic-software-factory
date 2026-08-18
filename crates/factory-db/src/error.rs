use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no {0} found")]
    NotFound(&'static str),
    /// The editor's expected plan revision no longer matches the stored one:
    /// another writer bumped the plan first.
    #[error("plan changed since it was read (expected revision {expected}, current {current})")]
    Conflict { expected: i64, current: i64 },
}
