//! Resolving the GitHub repository from a project's Git remotes.
//!
//! The repository is *only* ever derived from `git remote -v` output — never
//! guessed from folder names. HTTPS and SSH GitHub remotes are supported.

use std::path::Path;
use std::process::Command;

use crate::error::{GitHubError, Result};

/// The GitHub repository a Factory project is connected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRemote {
    /// The Git remote name the URL came from (normally `origin`).
    pub remote: String,
    /// `owner/name`.
    pub repository: String,
    /// The remote URL as configured.
    pub url: String,
    /// The remote's default branch, when known.
    pub default_branch: Option<String>,
}

impl GitHubRemote {
    pub fn web_url(&self) -> String {
        format!("https://github.com/{}", self.repository)
    }

    pub fn issue_url(&self, number: i64) -> String {
        format!("{}/issues/{number}", self.web_url())
    }
}

/// Parses one GitHub remote URL into `owner/name`.
///
/// Accepted shapes:
/// - `https://github.com/owner/name.git` (and `http://`)
/// - `git@github.com:owner/name.git` (SSH scp syntax)
/// - `ssh://git@github.com/owner/name.git` and `ssh://git@github.com:22/owner/name.git`
///
/// Anything else (other hosts, gitlab, relative paths, empty parts) is an
/// error; the repository is never guessed.
pub fn parse_remote_url(url: &str) -> Result<(String, String)> {
    let url = url.trim();
    if url.is_empty() {
        return Err(GitHubError::RemoteParse("empty remote URL".into()));
    }
    let path = if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
    {
        rest.to_string()
    } else if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest.to_string()
    } else if let Some(rest) = url
        .strip_prefix("ssh://git@github.com/")
        .or_else(|| url.strip_prefix("ssh://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))
    {
        rest.to_string()
    } else if let Some(rest) = url.strip_prefix("ssh://git@github.com:") {
        // ssh://git@github.com:22/owner/name.git — strip the leading port.
        let without_port = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        without_port.trim_start_matches('/').to_string()
    } else {
        return Err(GitHubError::NotAGithubRemote);
    };
    let path = path.trim().trim_end_matches(".git");
    let mut parts = path.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        return Err(GitHubError::RemoteParse(format!(
            "unexpected extra path segments in remote URL '{url}'"
        )));
    }
    validate_slug(owner)?;
    validate_slug(name)?;
    Ok((owner.to_string(), name.to_string()))
}

/// GitHub owner/repository names are restricted to a safe character set;
/// rejecting anything else blocks repository URL manipulation.
fn validate_slug(value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 100
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && value != "."
        && value != "..";
    if !valid {
        return Err(GitHubError::RemoteParse(format!(
            "'{value}' is not a valid GitHub owner or repository name"
        )));
    }
    Ok(())
}

/// Lists `(name, raw url)` pairs for configured remotes. Raw config values
/// are read (not `git remote -v`) so URL rewrite rules such as
/// `url.<local>.insteadOf` cannot mask the real GitHub remote.
pub fn list_remotes(root: &Path) -> Result<Vec<(String, String)>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .output()
        .map_err(GitHubError::Io)?;
    // `--get-regexp` exits 1 when nothing matches: a repo with no remotes.
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(GitHubError::Git(format!(
            "git config --get-regexp remote urls failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let out = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut remotes = Vec::new();
    for line in out.lines() {
        let Some((key, url)) = line.trim().split_once(' ') else {
            continue;
        };
        let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|rest| rest.strip_suffix(".url"))
        else {
            continue;
        };
        if name.is_empty() || url.trim().is_empty() {
            continue;
        }
        remotes.push((name.to_string(), url.trim().to_string()));
    }
    Ok(remotes)
}

/// Detects the GitHub remote for the repository at `root`. Prefers `origin`;
/// otherwise the first remote whose URL parses as GitHub.
pub fn detect(root: &Path) -> Result<GitHubRemote> {
    let remotes = list_remotes(root)?;
    if remotes.is_empty() {
        return Err(GitHubError::NotAGithubRemote);
    }
    let mut candidates = remotes.iter().filter_map(|(name, url)| {
        parse_remote_url(url)
            .ok()
            .map(|(owner, repo)| (name.clone(), url.clone(), format!("{owner}/{repo}")))
    });
    let selected = candidates
        .find(|(name, _, _)| name == "origin")
        .or_else(|| candidates.next());
    let Some((remote, url, repository)) = selected else {
        return Err(GitHubError::NotAGithubRemote);
    };
    let default_branch = default_branch_of(root, &remote);
    Ok(GitHubRemote {
        remote,
        repository,
        url,
        default_branch,
    })
}

/// The remote's default branch, from the local `refs/remotes/<remote>/HEAD`
/// symbolic ref. `None` when the clone has no remote HEAD mapping.
fn default_branch_of(root: &Path, remote: &str) -> Option<String> {
    let reference = format!("refs/remotes/{remote}/HEAD");
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["symbolic-ref", "--short", &reference])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let prefix = format!("{remote}/");
    value.strip_prefix(&prefix).map(str::to_string)
}

/// Whether `branch` exists on `remote` (any head ref matches).
pub fn remote_branch_exists(root: &Path, remote: &str, branch: &str) -> Result<bool> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-remote", "--heads", remote, branch])
        .output()
        .map_err(GitHubError::Io)?;
    if !out.status.success() {
        return Err(GitHubError::Network(format!(
            "git ls-remote {} {} failed: {}",
            remote,
            branch,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_remotes() {
        assert_eq!(
            parse_remote_url("https://github.com/OmegaMc1331/example.git").unwrap(),
            ("OmegaMc1331".into(), "example".into())
        );
        assert_eq!(
            parse_remote_url("https://github.com/a/b").unwrap(),
            ("a".into(), "b".into())
        );
    }

    #[test]
    fn parses_ssh_remotes() {
        assert_eq!(
            parse_remote_url("git@github.com:owner/repo.git").unwrap(),
            ("owner".into(), "repo".into())
        );
        assert_eq!(
            parse_remote_url("ssh://git@github.com/owner/repo.git").unwrap(),
            ("owner".into(), "repo".into())
        );
        assert_eq!(
            parse_remote_url("ssh://git@github.com:22/owner/repo.git").unwrap(),
            ("owner".into(), "repo".into())
        );
    }

    #[test]
    fn rejects_non_github_and_malformed_remotes() {
        assert!(matches!(
            parse_remote_url("https://gitlab.com/a/b.git"),
            Err(GitHubError::NotAGithubRemote)
        ));
        assert!(matches!(
            parse_remote_url("/local/path/repo.git"),
            Err(GitHubError::NotAGithubRemote)
        ));
        assert!(matches!(
            parse_remote_url("https://github.com/only-owner"),
            Err(GitHubError::RemoteParse(_))
        ));
        assert!(matches!(
            parse_remote_url("https://github.com/a/b/c"),
            Err(GitHubError::RemoteParse(_))
        ));
        assert!(matches!(
            parse_remote_url("https://github.com/../escape"),
            Err(GitHubError::RemoteParse(_))
        ));
        assert!(matches!(
            parse_remote_url("https://github.com/a/b;rm -rf /"),
            Err(GitHubError::RemoteParse(_))
        ));
        assert!(matches!(
            parse_remote_url(""),
            Err(GitHubError::RemoteParse(_))
        ));
    }

    #[test]
    fn remote_helpers_build_github_urls() {
        let remote = GitHubRemote {
            remote: "origin".into(),
            repository: "owner/repo".into(),
            url: "git@github.com:owner/repo.git".into(),
            default_branch: Some("main".into()),
        };
        assert_eq!(remote.web_url(), "https://github.com/owner/repo");
        assert_eq!(
            remote.issue_url(42),
            "https://github.com/owner/repo/issues/42"
        );
    }
}
