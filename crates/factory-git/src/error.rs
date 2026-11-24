use thiserror::Error;

use std::path::PathBuf;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("invalid source-control request: {0}")]
    Invalid(String),
    #[error("repository state changed; refresh before retrying")]
    Stale,
    #[error("source-control operation is not authorized: {0}")]
    Protected(String),
    #[error("repository operation is busy; no changes were applied")]
    Busy,
    #[error("Git helpers require explicit trust: {0}")]
    TrustRequired(String),
    #[error("current directory is not inside a git repository")]
    NotARepository,
    #[error("git command failed: {0}")]
    CommandFailed(String),
    #[error("git {operation} timed out after {seconds}s and was terminated")]
    TimedOut { operation: String, seconds: u64 },
    #[error("git {operation} produced more output than the {limit} byte capture limit")]
    OutputLimitExceeded { operation: String, limit: usize },
    #[error("failed to create worktree at {0}")]
    WorktreeAddFailed(PathBuf),
    #[error("worktree at {0} has uncommitted changes; commit or discard them first")]
    WorktreeDirty(PathBuf),
    #[error("failed to remove worktree at {0}")]
    WorktreeRemoveFailed(PathBuf),
    #[error("io error: {0}")]
    Io(std::io::Error),
}
