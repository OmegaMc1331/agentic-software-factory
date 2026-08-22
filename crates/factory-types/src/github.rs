//! GitHub linkage persisted on runs: the imported Issue that seeded a
//! workflow, and the delivery state of its `factory/run-<id>` integration
//! branch (push / pull request).
//!
//! Issue content is *untrusted external data*: it is stored verbatim for
//! traceability but never interpreted as Factory instructions, permission
//! changes, or system prompts. The delivery state is deliberately separate
//! from [`crate::RunStatus`]: a workflow can be `completed` while its delivery
//! is still `not_ready` or `published`.

use serde::{Deserialize, Serialize};

/// One bounded issue comment kept with an imported Issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueComment {
    pub author: String,
    pub body: String,
}

/// The GitHub Issue a workflow was created from, persisted at import time so
/// the workflow stays usable even when GitHub is later unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssueLink {
    /// Always `github`.
    pub provider: String,
    /// `owner/name`.
    pub repository: String,
    pub issue_number: i64,
    pub issue_url: String,
    /// Verbatim (bounded) issue title and body — untrusted data.
    pub issue_title: String,
    pub issue_body: String,
    pub issue_state: String,
    pub issue_author: String,
    pub issue_labels: Vec<String>,
    /// A bounded selection of comments (oldest first).
    pub issue_comments: Vec<IssueComment>,
    pub imported_at: String,
}

/// Lifecycle of delivering a completed workflow's integration branch to
/// GitHub. Small on purpose; this is not a second run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// The workflow is not eligible yet (incomplete, conflicted, or drifted).
    NotReady,
    /// Eligible: completed, integration head known and matching the local
    /// branch, no unresolved conflict.
    Ready,
    /// The `factory/run-<id>` branch is being pushed.
    Pushing,
    /// The pull request is being created.
    CreatingPr,
    /// A pull request exists (created now or detected earlier).
    Published,
    /// The last delivery attempt failed; see `error`.
    Failed,
}

impl DeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryState::NotReady => "not_ready",
            DeliveryState::Ready => "ready",
            DeliveryState::Pushing => "pushing",
            DeliveryState::CreatingPr => "creating_pr",
            DeliveryState::Published => "published",
            DeliveryState::Failed => "failed",
        }
    }
}

impl std::str::FromStr for DeliveryState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "not_ready" => Ok(DeliveryState::NotReady),
            "ready" => Ok(DeliveryState::Ready),
            "pushing" => Ok(DeliveryState::Pushing),
            "creating_pr" => Ok(DeliveryState::CreatingPr),
            "published" => Ok(DeliveryState::Published),
            "failed" => Ok(DeliveryState::Failed),
            other => Err(format!("unknown delivery state '{other}'")),
        }
    }
}

/// The pull request linked to a delivered run, when one exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestInfo {
    pub number: i64,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
}

/// Persisted delivery metadata for a run. Prevents duplicate pull requests
/// across restarts and records exactly what was pushed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubDelivery {
    pub run_id: i64,
    pub state: DeliveryState,
    /// `owner/name` resolved from the Git remote at delivery time.
    pub repository: Option<String>,
    /// The Git remote name delivery pushes to (normally `origin`).
    pub remote: Option<String>,
    pub base_branch: Option<String>,
    /// Always the Factory-owned `factory/run-<id>` branch.
    pub head_branch: String,
    /// The commit sha that was pushed; compared against the local branch head
    /// to detect drift before any publish attempt.
    pub pushed_head: Option<String>,
    pub pull_request: Option<PullRequestInfo>,
    /// Human-readable reason for the last failure, if any.
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl GitHubDelivery {
    /// A fresh, not-yet-published delivery record for a run.
    pub fn initial(run_id: i64, now: &str) -> GitHubDelivery {
        GitHubDelivery {
            run_id,
            state: DeliveryState::NotReady,
            repository: None,
            remote: None,
            base_branch: None,
            head_branch: format!("factory/run-{run_id}"),
            pushed_head: None,
            pull_request: None,
            error: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        }
    }
}
