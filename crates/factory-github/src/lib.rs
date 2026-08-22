//! GitHub integration for Agentic Software Factory.
//!
//! Scope (V1): the locally installed `gh` CLI, the project's Git remotes,
//! bounded Issue import, and Factory-owned delivery of a completed run's
//! `factory/run-<id>` branch (push + pull request).
//!
//! Not implemented here on purpose: OAuth servers, GitHub Apps, token storage,
//! webhooks, and any cloud auth backend.

pub mod delivery;
pub mod error;
pub mod gh;
pub mod issue;
pub mod pr;
pub mod remote;

pub use delivery::{create_or_link_pull_request, push_branch, DeliveryOutcome};
pub use error::{GitHubError, Result};
pub use gh::{GhAuth, GhCli};
pub use issue::{
    issue_link, objective_from_issue, parse_issue, IssueRef, MAX_COMMENTS, MAX_COMMENT_CHARS,
    MAX_ISSUE_BODY_CHARS,
};
pub use pr::{build_pr_body, default_pr_title, parse_pull_requests, PrEvidence};
pub use remote::{detect, list_remotes, parse_remote_url, remote_branch_exists, GitHubRemote};
