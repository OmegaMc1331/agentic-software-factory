use std::path::PathBuf;

use thiserror::Error;

/// Errors produced by the repository context engine. The engine is advisory:
/// callers render mission context as an optional enhancement, so every failure
/// is recoverable at the call site (log/ignore rather than fail the run).
#[derive(Debug, Error)]
pub enum ContextError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read persisted context index {path}: {source}")]
    IndexRead {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to write persisted context index {path}: {source}")]
    IndexWrite {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("git error: {0}")]
    Git(#[from] factory_git::GitError),
    #[error("invalid context configuration: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, ContextError>;
