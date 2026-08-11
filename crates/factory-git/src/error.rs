use thiserror::Error;

use std::path::PathBuf;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("current directory is not inside a git repository")]
    NotARepository,
    #[error("git command failed: {0}")]
    CommandFailed(String),
    #[error("failed to create worktree at {0}")]
    WorktreeAddFailed(PathBuf),
    #[error("failed to remove worktree at {0}")]
    WorktreeRemoveFailed(PathBuf),
    #[error("io error: {0}")]
    Io(std::io::Error),
}
