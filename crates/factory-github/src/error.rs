use thiserror::Error;

pub type Result<T> = std::result::Result<T, GitHubError>;

/// Actionable GitHub failures. Callers surface these verbatim; nothing is
/// collapsed into a generic "GitHub failed".
#[derive(Debug, Error)]
pub enum GitHubError {
    #[error(
        "GitHub CLI not found. Install gh (https://cli.github.com) and make sure `gh` is on PATH."
    )]
    GhNotInstalled,
    #[error(
        "GitHub authentication required. Run `gh auth login` in a terminal, then retry. Factory \
         never reads, stores, or displays GitHub tokens."
    )]
    GhAuthRequired,
    #[error("GitHub CLI failed: {0}")]
    Gh(String),
    #[error("the repository has no GitHub remote; add one (git remote add origin <url>) or run Factory inside a GitHub clone")]
    NotAGithubRemote,
    #[error("cannot determine the GitHub repository: {0}")]
    RemoteParse(String),
    #[error("issue not found: {0}")]
    IssueNotFound(String),
    #[error("invalid issue reference: {0}")]
    InvalidIssueRef(String),
    #[error("network unavailable while contacting GitHub: {0}")]
    Network(String),
    #[error("push rejected by the remote: {0}")]
    PushRejected(String),
    #[error("GitHub permission denied for repository {0}; check the account's access and `gh auth status`")]
    PermissionDenied(String),
    #[error("a pull request already exists for this branch")]
    PullRequestExists,
    #[error("base branch '{0}' is unavailable on the remote")]
    BaseBranchUnavailable(String),
    #[error("branch drift: {0}")]
    BranchDrift(String),
    #[error("git error: {0}")]
    Git(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
