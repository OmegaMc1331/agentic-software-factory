pub mod error;

pub use error::GitError;

use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T> = std::result::Result<T, GitError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEvidence {
    pub changed_files: Vec<String>,
    pub diff_summary: String,
    pub commit_sha: Option<String>,
}

impl WorktreeEvidence {
    /// Whether any repository change was recorded for the attempt.
    pub fn is_change(&self) -> bool {
        !self.changed_files.is_empty() || !self.diff_summary.is_empty()
    }
}

pub struct Repo {
    root: PathBuf,
}

impl Repo {
    pub fn detect(start: &Path) -> Result<Repo> {
        Self::discover(start, None)
    }

    pub fn detect_bounded(start: &Path, ceiling: &Path) -> Result<Repo> {
        let ceiling = normalize_path(ceiling);
        Self::discover(start, Some(&ceiling))
    }

    fn discover(start: &Path, ceiling: Option<&Path>) -> Result<Repo> {
        let out = git(start, &["rev-parse", "--show-toplevel"], ceiling)
            .map_err(|_| GitError::NotARepository)?;
        let root = normalize_path(&PathBuf::from(out.trim().trim_matches('\0')));
        if let Some(ceiling) = ceiling {
            if !root.starts_with(ceiling) {
                return Err(GitError::NotARepository);
            }
        }
        Ok(Repo { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_main_worktree(&self) -> Result<bool> {
        let git_dir = git(&self.root, &["rev-parse", "--git-dir"], None)?;
        let git_dir_path = PathBuf::from(git_dir.trim());
        let git_dir_path = if git_dir_path.is_absolute() {
            git_dir_path
        } else {
            self.root.join(git_dir_path)
        };
        let expected = self.root.join(".git");
        Ok(normalize_path(&git_dir_path) == expected)
    }

    /// Registers a worktree at `worktree_path` on `branch`.
    ///
    /// * If the branch already exists, the existing branch is checked out.
    /// * Otherwise a new branch is created. When `base` is provided and the
    ///   branch is new, it is created from `base` instead of HEAD.
    pub fn add_worktree(
        &self,
        worktree_path: &Path,
        branch: &str,
        base: Option<&str>,
    ) -> Result<()> {
        if self.find_worktree(worktree_path)?.is_some() {
            return Ok(());
        }
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.root)
            .arg("worktree")
            .arg("add")
            .arg("--quiet");
        if self.branch_exists(branch)? {
            cmd.arg(worktree_path).arg(branch);
        } else if let Some(base) = base {
            cmd.arg("-b").arg(branch).arg(worktree_path).arg(base);
        } else {
            cmd.arg("-b").arg(branch).arg(worktree_path);
        }
        let status = cmd.status().map_err(GitError::Io)?;
        if !status.success() {
            return Err(GitError::WorktreeAddFailed(worktree_path.to_path_buf()));
        }
        Ok(())
    }

    /// Whether `branch` (a short branch name) exists.
    pub fn branch_exists(&self, branch: &str) -> Result<bool> {
        let reference = format!("refs/heads/{branch}");
        Ok(git(
            &self.root,
            &["rev-parse", "--verify", "--quiet", &reference],
            None,
        )
        .is_ok())
    }

    pub fn find_worktree(&self, worktree_path: &Path) -> Result<Option<WorktreeInfo>> {
        let target = normalize_path(worktree_path);
        Ok(self
            .list_worktrees()?
            .into_iter()
            .find(|w| normalize_path(&w.path) == target))
    }

    pub fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let out = git(&self.root, &["worktree", "list", "--porcelain"], None)?;
        let mut worktrees = Vec::new();
        let mut current: Option<WorktreeInfo> = None;
        for line in out.lines() {
            if line.is_empty() {
                if let Some(w) = current.take() {
                    worktrees.push(w);
                }
                continue;
            }
            match line.split_once(' ') {
                Some(("worktree", path)) => {
                    current = Some(WorktreeInfo {
                        path: PathBuf::from(path),
                        head: None,
                        branch: None,
                    });
                }
                Some(("HEAD", sha)) => {
                    if let Some(w) = current.as_mut() {
                        w.head = Some(sha.to_string());
                    }
                }
                Some(("branch", branch)) => {
                    if let Some(w) = current.as_mut() {
                        w.branch = Some(branch.trim_start_matches("refs/heads/").to_string());
                    }
                }
                _ => {}
            }
        }
        if let Some(w) = current {
            worktrees.push(w);
        }
        Ok(worktrees)
    }

    pub fn remove_worktree(&self, worktree_path: &Path) -> Result<()> {
        if self.has_uncommitted_changes(worktree_path)? {
            return Err(GitError::WorktreeDirty(worktree_path.to_path_buf()));
        }
        self.remove_internal(worktree_path, false)
    }

    pub fn remove_worktree_force(&self, worktree_path: &Path) -> Result<()> {
        self.remove_internal(worktree_path, true)
    }

    fn remove_internal(&self, worktree_path: &Path, force: bool) -> Result<()> {
        let registered = self.find_worktree(worktree_path)?.is_some();
        if registered {
            let mut cmd = Command::new("git");
            cmd.arg("-C").arg(&self.root).args(["worktree", "remove"]);
            if force {
                cmd.arg("--force");
            }
            cmd.arg(worktree_path);
            let status = cmd.status().map_err(GitError::Io)?;
            if !status.success() {
                return Err(GitError::WorktreeRemoveFailed(worktree_path.to_path_buf()));
            }
        } else if worktree_path.exists() {
            std::fs::remove_dir_all(worktree_path).map_err(GitError::Io)?;
        }
        self.prune()?;
        Ok(())
    }

    pub fn prune(&self) -> Result<()> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.root).args(["worktree", "prune"]);
        cmd.status().map_err(GitError::Io)?;
        Ok(())
    }

    pub fn has_uncommitted_changes(&self, worktree_path: &Path) -> Result<bool> {
        if !worktree_path.exists() {
            return Ok(false);
        }
        let out = git(worktree_path, &["status", "--porcelain"], None)?;
        Ok(!out.trim().is_empty())
    }

    /// Stages all changes in `worktree_path` (including untracked files) and
    /// commits them with `message` under `identity` (`(name, email)`), using
    /// explicitly-provided git identity so the commit never depends on local
    /// repository configuration.
    ///
    /// Returns the new `HEAD` sha, or `None` when the worktree had nothing to
    /// commit.
    pub fn commit_changes(
        &self,
        worktree_path: &Path,
        message: &str,
        identity: (&str, &str),
    ) -> Result<Option<String>> {
        let (name, email) = identity;
        let dir = normalize_path(worktree_path);
        let add = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["add", "-A"])
            .status()
            .map_err(GitError::Io)?;
        if !add.success() {
            return Err(GitError::CommandFailed(
                "git add -A failed".to_string(),
            ));
        }
        let staged = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["diff", "--cached", "--quiet"])
            .status()
            .map_err(GitError::Io)?;
        if staged.code() == Some(0) {
            return Ok(None);
        }
        let out = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["commit", "-m", message])
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .output()
            .map_err(GitError::Io)?;
        if out.status.success() {
            return Ok(Some(self.head_sha(&dir)?));
        }
        Err(GitError::CommandFailed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }

    /// Resolves a branch name (or full `refs/heads/...` reference) to its
    /// commit sha. Fails when the reference does not exist.
    pub fn resolve_ref(&self, reference: &str) -> Result<String> {
        let full = if reference.starts_with("refs/heads/") {
            reference.to_string()
        } else {
            format!("refs/heads/{reference}")
        };
        Ok(git(
            &self.root,
            &["rev-parse", "--verify", "--quiet", &full],
            None,
        )?
        .trim()
        .to_string())
    }

    /// Whether `ancestor` is an ancestor of `descendant` (both short branch
    /// names or shas). Returns `false` when they are unrelated.
    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["merge-base", "--is-ancestor", ancestor, descendant])
            .status()
            .map_err(GitError::Io)?;
        match status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(GitError::CommandFailed(format!(
                "git merge-base --is-ancestor {ancestor} {descendant} failed"
            ))),
        }
    }

    /// Fast-forwards (or creates) the local branch `name` to `sha` without a
    /// checkout. Used to advance the per-run integration branch; it never
    /// touches the current branch or working tree.
    pub fn update_ref(&self, name: &str, sha: &str) -> Result<()> {
        git(
            &self.root,
            &["update-ref", &format!("refs/heads/{name}"), sha],
            None,
        )?;
        Ok(())
    }

    /// Rebases the branch checked out in `worktree_path` onto `onto` (a branch
    /// name or sha). Fails if a conflict occurs; the worktree is left mid-rebase
    /// for inspection.
    pub fn rebase_onto_in(&self, worktree_path: &Path, onto: &str) -> Result<()> {
        let status = Command::new("git")
            .arg("-C")
            .arg(normalize_path(worktree_path))
            .args(["rebase", onto])
            .status()
            .map_err(GitError::Io)?;
        if !status.success() {
            return Err(GitError::CommandFailed(format!(
                "git rebase {onto} failed in {}",
                worktree_path.display()
            )));
        }
        Ok(())
    }

    pub fn head_sha(&self, worktree_path: &Path) -> Result<String> {
        Ok(git(worktree_path, &["rev-parse", "HEAD"], None)?
            .trim()
            .to_string())
    }

    pub fn evidence_since(&self, worktree_path: &Path, base_sha: &str) -> Result<WorktreeEvidence> {
        let head = self.head_sha(worktree_path)?;
        let committed = git(
            worktree_path,
            &["diff", "--name-only", base_sha, "HEAD"],
            None,
        )?;
        let working = git(worktree_path, &["status", "--porcelain"], None)?;
        let mut changed_files = std::collections::BTreeSet::new();
        for path in committed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            changed_files.insert(path.to_string());
        }
        for line in working.lines().filter(|line| line.len() > 3) {
            let raw_path = &line[3..];
            let path = raw_path
                .rsplit_once(" -> ")
                .map_or(raw_path, |(_, target)| target)
                .trim();
            if !path.is_empty() {
                changed_files.insert(path.to_string());
            }
        }
        let mut diff_summary = git(worktree_path, &["diff", "--stat", base_sha], None)?;
        if diff_summary.trim().is_empty() && !working.trim().is_empty() {
            diff_summary = working;
        }
        Ok(WorktreeEvidence {
            changed_files: changed_files.into_iter().collect(),
            diff_summary: diff_summary.trim().to_string(),
            commit_sha: (head != base_sha).then_some(head),
        })
    }

    /// The full patch text (`git diff <base>`) of a worktree, bounded to
    /// `max_chars` characters at a character boundary. Specialized review roles
    /// receive this so they can evaluate the actual change without sharing the
    /// implementation worktree.
    pub fn diff_patch(
        &self,
        worktree_path: &Path,
        base_sha: &str,
        max_chars: usize,
    ) -> Result<String> {
        let full = git(worktree_path, &["diff", base_sha, "--", "."], None)?;
        let mut bounded = String::new();
        let mut chars = 0usize;
        for line in full.lines() {
            if chars + line.len() > max_chars {
                break;
            }
            bounded.push_str(line);
            bounded.push('\n');
            chars += line.len() + 1;
        }
        Ok(bounded)
    }
}

fn git(dir: &Path, args: &[&str], ceiling: Option<&Path>) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(normalize_path(dir)).args(args);
    if let Some(ceiling) = ceiling {
        let mut value = ceiling.display().to_string();
        if let Ok(existing) = std::env::var("GIT_CEILING_DIRECTORIES") {
            if !existing.is_empty() {
                value.push(';');
                value.push_str(&existing);
            }
        }
        cmd.env("GIT_CEILING_DIRECTORIES", value);
    }
    let out = cmd.output().map_err(GitError::Io)?;
    if !out.status.success() {
        return Err(GitError::CommandFailed(format!(
            "git {} failed",
            args.join(" ")
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn normalize_path(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use crate::{git, Repo};

    fn init_repo(dir: &Path) {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "-q", "-b", "main"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", "user.email", "test@example.com"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", "user.name", "Factory Test"])
            .status()
            .unwrap();
        std::fs::write(dir.join("README.md"), "test repo").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-q", "-m", "init"])
            .status()
            .unwrap();
    }

    #[test]
    fn detects_the_repository_root() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(repo.root(), root);
        assert!(repo.is_main_worktree().unwrap());
    }

    #[test]
    fn detects_non_repository() {
        let dir = TempDir::new().unwrap();
        assert!(Repo::detect_bounded(dir.path(), dir.path()).is_err());
    }

    #[test]
    fn creates_locates_and_removes_a_worktree() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();

        let worktree = dir.path().join(".factory").join("worktrees").join("t1");
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();

        let found = repo.find_worktree(&worktree).unwrap();
        assert!(found.is_some());
        let info = found.unwrap();
        assert_eq!(info.branch.as_deref(), Some("factory/t1"));
        assert!(worktree.join("README.md").exists());

        repo.remove_worktree(&worktree).unwrap();
        assert!(repo.find_worktree(&worktree).unwrap().is_none());
        assert!(!worktree.exists());
    }

    #[test]
    fn adding_an_existing_worktree_path_is_a_no_op() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();
        assert_eq!(repo.list_worktrees().unwrap().len(), 2);
    }

    #[test]
    fn reattaches_an_existing_branch_after_the_worktree_is_removed() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let wt_a = dir.path().join("wt-a");
        let wt_b = dir.path().join("wt-b");
        repo.add_worktree(&wt_a, "factory/t1", None).unwrap();
        repo.remove_worktree(&wt_a).unwrap();
        repo.add_worktree(&wt_b, "factory/t1", None).unwrap();
        let info = repo.find_worktree(&wt_b).unwrap().unwrap();
        assert_eq!(info.branch.as_deref(), Some("factory/t1"));
        assert_eq!(repo.list_worktrees().unwrap().len(), 2);
    }

    #[test]
    fn creates_worktree_from_explicit_base() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();

        std::fs::write(dir.path().join("more.txt"), "more").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["commit", "-q", "-m", "second"])
            .status()
            .unwrap();
        repo.update_ref("factory/run-1", &repo.head_sha(dir.path()).unwrap())
            .unwrap();

        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", Some("factory/run-1"))
            .unwrap();
        assert_eq!(repo.head_sha(&worktree).unwrap(), repo.head_sha(dir.path()).unwrap());
        assert!(worktree.join("more.txt").exists());
    }

    #[test]
    fn commits_uncommitted_changes_with_identity() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();
        std::fs::write(worktree.join("feature.txt"), "hello").unwrap();

        let sha = repo
            .commit_changes(
                &worktree,
                "factory: integrate run-1 task-1 (builder)",
                ("Builder Agent", "factory@local"),
            )
            .unwrap()
            .expect("a commit should be created");
        assert_eq!(repo.head_sha(&worktree).unwrap(), sha);
        assert!(!repo.has_uncommitted_changes(&worktree).unwrap());

        let author = git(&worktree, &["show", "-s", "--format=%an <%ae>", "HEAD"], None).unwrap();
        assert_eq!(author.trim(), "Builder Agent <factory@local>");
    }

    #[test]
    fn commit_changes_yields_none_when_clean() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();

        let result = repo
            .commit_changes(&worktree, "nothing here", ("A", "a@local"))
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn creates_and_fast_forwards_a_run_branch() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();

        assert!(!repo.branch_exists("factory/run-1").unwrap());
        repo.update_ref("factory/run-1", &repo.head_sha(dir.path()).unwrap())
            .unwrap();
        assert!(repo.branch_exists("factory/run-1").unwrap());
        assert_eq!(
            repo.resolve_ref("factory/run-1").unwrap(),
            repo.head_sha(dir.path()).unwrap()
        );

        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", Some("factory/run-1"))
            .unwrap();
        std::fs::write(worktree.join("feature.txt"), "hello").unwrap();
        repo.commit_changes(&worktree, "feature work", ("Worker", "w@local"))
            .unwrap();
        let task_head = repo.head_sha(&worktree).unwrap();

        assert!(repo.is_ancestor(&repo.resolve_ref("factory/run-1").unwrap(), &task_head).unwrap());
        repo.update_ref("factory/run-1", &task_head).unwrap();
        assert_eq!(repo.resolve_ref("factory/run-1").unwrap(), task_head);
        assert!(repo.is_ancestor("main", &repo.resolve_ref("factory/run-1").unwrap()).unwrap());
    }

    #[test]
    fn rebases_a_task_branch_onto_the_run_branch() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        repo.update_ref("factory/run-1", &repo.head_sha(dir.path()).unwrap())
            .unwrap();

        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", Some("factory/run-1"))
            .unwrap();
        std::fs::write(worktree.join("feature.txt"), "hello").unwrap();
        repo.commit_changes(&worktree, "feature work", ("Worker", "w@local"))
            .unwrap();

        std::fs::write(dir.path().join("runtime.txt"), "runtime").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["commit", "-q", "-m", "runtime change"])
            .status()
            .unwrap();
        repo.update_ref("factory/run-1", &repo.head_sha(dir.path()).unwrap())
            .unwrap();

        let task_head = repo.head_sha(&worktree).unwrap();
        assert!(!repo.is_ancestor(&repo.resolve_ref("factory/run-1").unwrap(), &task_head).unwrap());

        repo.rebase_onto_in(&worktree, "factory/run-1").unwrap();
        let rebased = repo.head_sha(&worktree).unwrap();
        assert!(repo.is_ancestor(&repo.resolve_ref("factory/run-1").unwrap(), &rebased).unwrap());
    }

    #[test]
    fn detects_uncommitted_changes() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();
        assert!(!repo.has_uncommitted_changes(&worktree).unwrap());
        std::fs::write(worktree.join("new-file.txt"), "hello").unwrap();
        assert!(repo.has_uncommitted_changes(&worktree).unwrap());
    }

    #[test]
    fn refuses_to_remove_a_dirty_worktree() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();
        std::fs::write(worktree.join("wip.txt"), "uncommitted").unwrap();

        let err = repo.remove_worktree(&worktree).unwrap_err();
        assert!(matches!(err, crate::GitError::WorktreeDirty(_)));
        assert!(worktree.exists());
        assert!(repo.find_worktree(&worktree).unwrap().is_some());
    }

    #[test]
    fn force_removes_a_dirty_worktree() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1", None).unwrap();
        std::fs::write(worktree.join("wip.txt"), "uncommitted").unwrap();

        repo.remove_worktree_force(&worktree).unwrap();
        assert!(!worktree.exists());
        assert!(repo.find_worktree(&worktree).unwrap().is_none());
    }
}