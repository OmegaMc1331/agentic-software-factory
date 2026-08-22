//! Adapter over the locally installed, locally authenticated `gh` CLI.
//!
//! Constraints (deliberate):
//! - every invocation uses structured process arguments — no `sh -c`, no
//!   `cmd /c`, no string-interpolated shell commands;
//! - authentication state comes from `gh auth status` only — Factory never
//!   reads gh's config files, keyrings, or tokens, and never displays them;
//! - the binary under test is injectable so CI runs against a fake `gh`
//!   instead of a real GitHub account.

use std::ffi::OsString;
use std::process::Command;

use crate::error::{GitHubError, Result};
use crate::issue::GitHubIssue;

/// Which `gh` program to run. Defaults to `gh` from PATH; the
/// `FACTORY_GH_BIN` environment variable overrides it (used by tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCli {
    program: OsString,
    args_prefix: Vec<OsString>,
}

/// Authentication state as reported by `gh auth status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhAuth {
    pub user: Option<String>,
}

impl GhCli {
    pub fn discovered() -> GhCli {
        match std::env::var_os("FACTORY_GH_BIN") {
            Some(path) if !path.is_empty() => GhCli::wrap(path, Vec::new()),
            // An absent or empty override means "plain gh from PATH".
            None => GhCli {
                program: "gh".into(),
                args_prefix: Vec::new(),
            },
            Some(_) => GhCli {
                program: "gh".into(),
                args_prefix: Vec::new(),
            },
        }
    }

    /// Runs `program` directly.
    pub fn with_program(program: impl Into<OsString>) -> GhCli {
        GhCli {
            program: program.into(),
            args_prefix: Vec::new(),
        }
    }

    /// Wraps `program` behind a fixed argument prefix (test scaffolding for
    /// shell-script fakes; production code always runs `gh` directly).
    pub fn wrap(program: impl Into<OsString>, prefix: Vec<OsString>) -> GhCli {
        GhCli {
            program: program.into(),
            args_prefix: prefix,
        }
    }

    /// `gh auth status`: connected account, without touching tokens.
    pub fn auth_status(&self) -> Result<GhAuth> {
        let out = self.run(&["auth", "status"]);
        let output = match out {
            Ok(output) => output,
            Err(error) => return Err(classify_spawn_failure(error)),
        };
        if output.status.success() {
            let text = combined(&output);
            return Ok(GhAuth {
                user: parse_auth_user(&text),
            });
        }
        let text = combined(&output);
        if text.contains("not logged in")
            || text.contains("no accounts")
            || text.contains("auth login")
            || text.contains("To get started")
        {
            return Err(GitHubError::GhAuthRequired);
        }
        Err(GitHubError::Gh(text.trim().to_string()))
    }

    /// `gh issue view`: the Issue's core fields plus a bounded selection of
    /// comments (fetched in the same call, capped by [`MAX_COMMENTS`]).
    pub fn view_issue(&self, repository: &str, number: i64) -> Result<GitHubIssue> {
        let reference = format!("{repository}/{number}");
        let fields = "number,title,body,labels,state,url,author,comments";
        let output = self.run(&[
            "issue",
            "view",
            &number.to_string(),
            "--repo",
            repository,
            "--json",
            fields,
        ])?;
        if !output.status.success() {
            let text = combined(&output);
            if text.contains("not found") || text.contains("Could not resolve") {
                return Err(GitHubError::IssueNotFound(reference));
            }
            return Err(classify_gh_failure(&text, repository));
        }
        crate::issue::parse_issue(&String::from_utf8_lossy(&output.stdout))
            .map_err(|error| GitHubError::Gh(format!("unreadable issue payload: {error}")))
    }

    /// Open pull requests for `head_branch`, newest first.
    pub fn list_pull_requests(
        &self,
        repository: &str,
        head_branch: &str,
    ) -> Result<Vec<factory_types::PullRequestInfo>> {
        let output = self.run(&[
            "pr",
            "list",
            "--repo",
            repository,
            "--head",
            head_branch,
            "--state",
            "open",
            "--json",
            "number,url,state,isDraft",
        ])?;
        if !output.status.success() {
            let text = combined(&output);
            if text.contains("no pull requests found") || text.contains("No pull requests") {
                return Ok(Vec::new());
            }
            return Err(classify_gh_failure(&text, repository));
        }
        crate::pr::parse_pull_requests(&String::from_utf8_lossy(&output.stdout))
            .map_err(|error| GitHubError::Gh(format!("unreadable pull request payload: {error}")))
    }

    /// `gh pr create`: returns the created (or already existing) PR.
    pub fn create_pull_request(
        &self,
        repository: &str,
        base: &str,
        head: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<factory_types::PullRequestInfo> {
        let mut args: Vec<OsString> = [
            "pr", "create", "--repo", repository, "--base", base, "--head", head, "--title", title,
            "--body", body,
        ]
        .iter()
        .map(|value| (*value).into())
        .collect();
        if draft {
            args.push("--draft".into());
        }
        let output = self.run_os(&args)?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success() {
            let failure = combined(&output);
            if failure.contains("already exists") {
                return Err(GitHubError::PullRequestExists);
            }
            if failure.contains("no commits between") || failure.contains("No commits between") {
                return Err(GitHubError::PushRejected(
                    "the head branch has no changes against the base branch".into(),
                ));
            }
            return Err(classify_gh_failure(&failure, repository));
        }
        crate::pr::parse_pull_request_url(&text)
            .ok_or_else(|| GitHubError::Gh(format!("gh did not report a pull request URL: {text}")))
    }

    fn run(&self, args: &[&str]) -> Result<std::process::Output> {
        let os_args: Vec<OsString> = args.iter().map(|value| (*value).into()).collect();
        self.run_os(&os_args)
    }

    fn run_os(&self, args: &[OsString]) -> Result<std::process::Output> {
        let mut command = Command::new(&self.program);
        command.args(&self.args_prefix).args(args);
        command.current_dir(std::env::temp_dir());
        // gh inherits the environment it needs for auth; Factory adds nothing
        // and never passes or inspects token values itself.
        command.output().map_err(GitHubError::Io)
    }
}

/// Combined stdout+stderr, trimmed — `gh auth status` prints to either stream
/// depending on version; account discovery must read both.
fn combined(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{stdout}{stderr}")
}

/// Extracts the account name from `gh auth status` text such as
/// `✓ Logged in to github.com account octocat (keyring)`.
pub fn parse_auth_user(text: &str) -> Option<String> {
    let marker = "account ";
    let mut search = 0usize;
    while let Some(index) = text[search..].find(marker) {
        let start = search + index + marker.len();
        let user: String = text[start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(c, '(' | ')'))
            .collect();
        if !user.is_empty() {
            return Some(user);
        }
        search = start;
    }
    None
}

fn classify_spawn_failure(error: GitHubError) -> GitHubError {
    match error {
        GitHubError::Io(inner) if inner.kind() == std::io::ErrorKind::NotFound => {
            GitHubError::GhNotInstalled
        }
        other => other,
    }
}

/// Maps gh failure text onto actionable errors.
pub(crate) fn classify_gh_failure(text: &str, repository: &str) -> GitHubError {
    let text = text.trim();
    if text.contains("authentication required")
        || text.contains("not logged in")
        || text.contains("HTTP 401")
    {
        return GitHubError::GhAuthRequired;
    }
    if text.contains("HTTP 403") || text.contains("permission denied") || text.contains("HTTP 404")
    {
        return GitHubError::PermissionDenied(repository.to_string());
    }
    if text.contains("could not resolve")
        || text.contains("dial tcp")
        || text.contains("network")
        || text.contains("timed out")
        || text.contains("connection refused")
    {
        return GitHubError::Network(text.to_string());
    }
    GitHubError::Gh(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_logged_in_account_without_touching_tokens() {
        let text = "github.com\n  ✓ Logged in to github.com account octocat (keyring)\n";
        assert_eq!(parse_auth_user(text).as_deref(), Some("octocat"));
    }

    #[test]
    fn auth_user_is_none_without_an_account_line() {
        assert_eq!(
            parse_auth_user("github.com\n  ✗ Logged in to github.com"),
            None
        );
        assert_eq!(parse_auth_user(""), None);
    }
}
