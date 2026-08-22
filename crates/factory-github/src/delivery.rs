//! Factory-owned delivery: pushing the `factory/run-<id>` integration branch
//! to the configured GitHub remote and creating its pull request.
//!
//! This is the **only** place in Factory that constructs a `git push`, and it
//! pushes exactly one Factory-generated branch name with `--no-verify`-free,
//! non-force arguments. Agents never reach this code path; the Policy Engine
//! separately denies push-class git operations for task agents.

use std::path::Path;
use std::process::Command;

use crate::error::{GitHubError, Result};
use crate::gh::GhCli;
use factory_types::PullRequestInfo;

/// Pushes `branch` to `remote` with upstream tracking. Never force-pushes and
/// never pushes any branch it was not handed by the delivery engine.
pub fn push_branch(root: &Path, remote: &str, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["push", "--set-upstream", remote, branch])
        .output()
        .map_err(GitHubError::Io)?;
    if output.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Err(classify_push_failure(&text))
}

fn classify_push_failure(text: &str) -> GitHubError {
    let text = text.trim();
    if text.contains("![rejected]")
        || text.contains("non-fast-forward")
        || text.contains("fetch first")
    {
        return GitHubError::PushRejected(
            "the remote branch diverged; refusing to force-push. Re-import or rebase manually."
                .into(),
        );
    }
    if text.contains("Permission to")
        || text.contains("403")
        || text.contains("could not read Username")
    {
        return GitHubError::PushRejected(
            "the remote rejected the push for this account (permissions or credentials)".into(),
        );
    }
    if text.contains("Could not resolve host")
        || text.contains("Connection")
        || text.contains("timed out")
    {
        return GitHubError::Network(text.to_string());
    }
    GitHubError::PushRejected(text.to_string())
}

/// Outcome of a delivery attempt: either a freshly created PR or an existing
/// one that was detected and linked instead of duplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Created(PullRequestInfo),
    LinkedExisting(PullRequestInfo),
}

/// Checks for an open PR on `head_branch` first; creates one only when none
/// exists. Prevents duplicates across retries and browser refreshes.
pub fn create_or_link_pull_request(
    gh: &GhCli,
    repository: &str,
    base: &str,
    head_branch: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<DeliveryOutcome> {
    let existing = gh.list_pull_requests(repository, head_branch)?;
    if let Some(pr) = existing.into_iter().max_by_key(|pr| pr.number) {
        return Ok(DeliveryOutcome::LinkedExisting(pr));
    }
    Ok(DeliveryOutcome::Created(gh.create_pull_request(
        repository,
        base,
        head_branch,
        title,
        body,
        draft,
    )?))
}
