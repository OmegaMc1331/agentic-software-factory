//! Factory-owned delivery: pushing the `factory/run-<id>` integration branch
//! to the configured GitHub remote and creating its pull request.
//!
//! This is the **only** place in Factory that constructs a `git push`, and it
//! pushes exactly one Factory-generated branch name with `--no-verify`-free,
//! non-force arguments. Agents never reach this code path; the Policy Engine
//! separately denies push-class git operations for task agents.

use std::path::Path;
use std::time::Duration;

use factory_proc::{Bounds, ProcCommand, ProcError};

use crate::error::{GitHubError, Result};
use crate::gh::GhCli;
use factory_types::PullRequestInfo;

/// Deadline for one network git operation (push, ls-remote). Generous for slow
/// links; a permanently wedged command is terminated and classified.
const GIT_NETWORK_TIMEOUT_SECS: u64 = 120;

fn git_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("FACTORY_GH_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(GIT_NETWORK_TIMEOUT_SECS),
    )
}

/// Runs git against the network under the bounded supervisor. Output limits
/// bound captured remote diagnostics; credentials never appear in argv.
fn run_network_git(root: &Path, args: &[&str]) -> Result<factory_proc::ProcOutput> {
    let summary = args.join(" ");
    match ProcCommand::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .run(Bounds::new(git_timeout(), 4 * 1024 * 1024, 1024 * 1024))
    {
        Ok(output) => Ok(output),
        // The remote may have accepted the push before the deadline killed
        // git: the outcome stays unknown until reconciliation.
        Err(ProcError::TimedOut { .. }) | Err(ProcError::Cancelled { .. }) => {
            Err(GitHubError::Uncertain(format!("git {summary} timed out")))
        }
        Err(ProcError::Spawn(error)) | Err(ProcError::Io(error)) => Err(GitHubError::Io(error)),
    }
}

/// Pushes `branch` to `remote` with upstream tracking. Never force-pushes and
/// never pushes any branch it was not handed by the delivery engine.
pub fn push_branch(root: &Path, remote: &str, branch: &str) -> Result<()> {
    let output = run_network_git(root, &["push", "--set-upstream", remote, branch])?;
    if output.success() {
        return Ok(());
    }
    Err(classify_push_failure(&output.combined_text()))
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
    // The remote may have accepted the push before the failure; the outcome
    // is unknown until reconciliation checks the remote branch.
    GitHubError::Uncertain(text.to_string())
}

/// The commit the remote currently holds for `branch`, if the branch exists.
/// Read-only (`git ls-remote`); used to confirm or refute an uncertain push
/// without ever pushing.
pub fn remote_branch_sha(root: &Path, remote: &str, branch: &str) -> Result<Option<String>> {
    let want = format!("refs/heads/{branch}");
    let output = run_network_git(root, &["ls-remote", remote, &want])?;
    if !output.success() {
        return Err(classify_push_failure(&output.combined_text()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .find_map(|line| line.split_once('\t'))
        .filter(|(sha, reference)| !sha.is_empty() && *reference == want)
        .map(|(sha, _)| sha.to_string()))
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
    match gh.create_pull_request(repository, base, head_branch, title, body, draft) {
        Ok(pr) => Ok(DeliveryOutcome::Created(pr)),
        // An unrecognized `gh pr create` failure cannot prove the PR was not
        // created (GitHub may have accepted it before the CLI failed); the
        // delivery outcome stays unknown until reconciliation.
        Err(GitHubError::Gh(detail)) => Err(GitHubError::Uncertain(detail)),
        Err(error) => Err(error),
    }
}
