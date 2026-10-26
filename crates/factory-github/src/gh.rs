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
use std::time::Duration;

use factory_proc::{Bounds, ProcCommand, ProcError};

use crate::error::{GitHubError, Result};
use crate::issue::GitHubIssue;

/// Which `gh` program to run. Defaults to `gh` from PATH; the
/// `FACTORY_GH_BIN` environment variable overrides it (used by tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCli {
    program: OsString,
    args_prefix: Vec<OsString>,
}

/// Deadline and output-capture bounds for one gh operation class. Read-only
/// lookups stay short; `pr create` gets a long deadline because publication is
/// slow by nature — and its timeout is an *uncertain* outcome, never failure.
struct GhBounds {
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
}

/// GitHub caps pull request bodies at 262,144 characters; Factory rejects
/// larger bodies up front instead of letting the CLI fail opaquely.
pub const MAX_PR_BODY_CHARS: usize = 262_144;

const GH_ENV_TIMEOUT_SECS: u64 = 30;
const GH_ISSUE_TIMEOUT_SECS: u64 = 60;
const GH_PR_LIST_TIMEOUT_SECS: u64 = 30;
const GH_PR_CREATE_TIMEOUT_SECS: u64 = 180;

/// `FACTORY_GH_TIMEOUT_SECS` overrides every per-operation deadline (tests use
/// it to keep hanging-CLI cases fast); absent or invalid values keep the
/// operation-specific defaults.
fn gh_timeout(default_secs: u64) -> Duration {
    let secs = std::env::var("FACTORY_GH_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(default_secs);
    Duration::from_secs(secs)
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
        let output = match self.run(
            &["auth", "status"],
            GhBounds {
                timeout: gh_timeout(GH_ENV_TIMEOUT_SECS),
                stdout_limit: 1024 * 1024,
                stderr_limit: 1024 * 1024,
            },
        ) {
            Ok(output) => output,
            Err(error) => return Err(classify_spawn_failure(error)),
        };
        if output.success() {
            let text = output.combined_text();
            return Ok(GhAuth {
                user: parse_auth_user(&text),
            });
        }
        let text = output.combined_text();
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
        let output = self.run(
            &[
                "issue",
                "view",
                &number.to_string(),
                "--repo",
                repository,
                "--json",
                fields,
            ],
            GhBounds {
                timeout: gh_timeout(GH_ISSUE_TIMEOUT_SECS),
                stdout_limit: 16 * 1024 * 1024,
                stderr_limit: 256 * 1024,
            },
        )?;
        self.require_complete_stdout(&output, "issue view")?;
        if !output.success() {
            let text = output.combined_text();
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
        self.list_pull_requests_with_state(repository, head_branch, "open")
    }

    /// Pull requests for `head_branch` in any lifecycle state (open, closed,
    /// merged). Reconciliation uses this: an attempt whose PR was created and
    /// then closed before recovery must still be discoverable.
    pub fn list_all_pull_requests(
        &self,
        repository: &str,
        head_branch: &str,
    ) -> Result<Vec<factory_types::PullRequestInfo>> {
        self.list_pull_requests_with_state(repository, head_branch, "all")
    }

    fn list_pull_requests_with_state(
        &self,
        repository: &str,
        head_branch: &str,
        state: &str,
    ) -> Result<Vec<factory_types::PullRequestInfo>> {
        let output = self.run(
            &[
                "pr",
                "list",
                "--repo",
                repository,
                "--head",
                head_branch,
                "--state",
                state,
                "--json",
                "number,url,state,isDraft",
            ],
            GhBounds {
                timeout: gh_timeout(GH_PR_LIST_TIMEOUT_SECS),
                stdout_limit: 4 * 1024 * 1024,
                stderr_limit: 256 * 1024,
            },
        )?;
        self.require_complete_stdout(&output, "pr list")?;
        if !output.success() {
            let text = output.combined_text();
            if text.contains("no pull requests found") || text.contains("No pull requests") {
                return Ok(Vec::new());
            }
            return Err(classify_gh_failure(&text, repository));
        }
        crate::pr::parse_pull_requests(&String::from_utf8_lossy(&output.stdout))
            .map_err(|error| GitHubError::Gh(format!("unreadable pull request payload: {error}")))
    }

    /// `gh pr create`: returns the created (or already existing) PR.
    ///
    /// The body travels through an exclusive-create temporary file consumed by
    /// `--body-file` — never through argv, which Windows command-line limits
    /// would truncate. The file is removed on success, failure, and timeout.
    /// A timeout after GitHub may already have created the PR is reported as
    /// [`GitHubError::Uncertain`], never as a definitive failure.
    pub fn create_pull_request(
        &self,
        repository: &str,
        base: &str,
        head: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<factory_types::PullRequestInfo> {
        if body.chars().count() > MAX_PR_BODY_CHARS {
            return Err(GitHubError::BodyTooLarge(MAX_PR_BODY_CHARS));
        }
        let mut body_file = tempfile::NamedTempFile::new().map_err(GitHubError::Io)?;
        std::io::Write::write_all(&mut body_file, body.as_bytes()).map_err(GitHubError::Io)?;
        std::io::Write::flush(&mut body_file).map_err(GitHubError::Io)?;
        let mut args: Vec<OsString> = [
            "pr",
            "create",
            "--repo",
            repository,
            "--base",
            base,
            "--head",
            head,
            "--title",
            title,
            "--body-file",
        ]
        .iter()
        .map(|value| (*value).into())
        .collect();
        args.push(body_file.path().as_os_str().to_os_string());
        if draft {
            args.push("--draft".into());
        }
        let output = self.run_os(
            &args,
            GhBounds {
                timeout: gh_timeout(GH_PR_CREATE_TIMEOUT_SECS),
                stdout_limit: 1024 * 1024,
                stderr_limit: 256 * 1024,
            },
        );
        // Dropping the handle removes the file on every path, including the
        // timeout path above where the run never returns an Output.
        drop(body_file);
        let output = match output {
            Ok(output) => output,
            // The create may have reached GitHub before the deadline killed
            // the CLI: the outcome is unknown until reconciliation.
            Err(GitHubError::TimedOut(detail)) => {
                return Err(GitHubError::Uncertain(format!(
                    "gh pr create timed out: {detail}"
                )))
            }
            Err(error) => return Err(error),
        };
        self.require_complete_stdout(&output, "pr create")?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.success() {
            let failure = output.combined_text();
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

    fn run(&self, args: &[&str], bounds: GhBounds) -> Result<factory_proc::ProcOutput> {
        let os_args: Vec<OsString> = args.iter().map(|value| (*value).into()).collect();
        self.run_os(&os_args, bounds)
    }

    fn run_os(&self, args: &[OsString], bounds: GhBounds) -> Result<factory_proc::ProcOutput> {
        let command = ProcCommand::new(&self.program)
            .args(&self.args_prefix)
            .args(args)
            .current_dir(std::env::temp_dir())
            // gh inherits the environment it needs for auth; Factory adds
            // nothing and never passes or inspects token values itself. The
            // prompt guard only stops the git underneath gh from waiting for
            // terminal credentials; keyring and helper flows are unaffected.
            .env("GIT_TERMINAL_PROMPT", "0");
        let summary = args
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        match command.run(Bounds::new(
            bounds.timeout,
            bounds.stdout_limit,
            bounds.stderr_limit,
        )) {
            Ok(output) => Ok(output),
            Err(ProcError::TimedOut { .. }) | Err(ProcError::Cancelled { .. }) => {
                Err(GitHubError::TimedOut(format!("gh {summary}")))
            }
            Err(ProcError::Spawn(error)) | Err(ProcError::Io(error)) => Err(GitHubError::Io(error)),
        }
    }

    /// JSON responses must never be parsed from truncated output: a partial
    /// payload is a hard error, not a strange but valid-looking reply.
    fn require_complete_stdout(
        &self,
        output: &factory_proc::ProcOutput,
        operation: &str,
    ) -> Result<()> {
        if output.stdout_truncated {
            return Err(GitHubError::Gh(format!(
                "gh {operation} produced more output than the capture limit; refusing to parse a truncated payload"
            )));
        }
        Ok(())
    }
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
        // auth status is read-only: a deadline miss is a definitive network
        // failure, not an uncertain outcome.
        GitHubError::TimedOut(detail) => GitHubError::Network(detail),
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
