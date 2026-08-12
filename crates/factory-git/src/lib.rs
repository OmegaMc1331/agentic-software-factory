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

    pub fn add_worktree(&self, worktree_path: &Path, branch: &str) -> Result<()> {
        if self.find_worktree(worktree_path)?.is_some() {
            return Ok(());
        }
        let args = ["worktree", "add", "-b", branch, "--quiet"];
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.root).args(args).arg(worktree_path);
        let status = cmd.status().map_err(GitError::Io)?;
        if !status.success() {
            return Err(GitError::WorktreeAddFailed(worktree_path.to_path_buf()));
        }
        Ok(())
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

    use crate::Repo;

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
        repo.add_worktree(&worktree, "factory/t1").unwrap();

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
    fn adding_an_existing_worktree_is_a_no_op() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1").unwrap();
        repo.add_worktree(&worktree, "factory/t1").unwrap();
        assert_eq!(repo.list_worktrees().unwrap().len(), 2);
    }

    #[test]
    fn detects_uncommitted_changes() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
        let worktree = dir.path().join("wt");
        repo.add_worktree(&worktree, "factory/t1").unwrap();
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
        repo.add_worktree(&worktree, "factory/t1").unwrap();
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
        repo.add_worktree(&worktree, "factory/t1").unwrap();
        std::fs::write(worktree.join("wip.txt"), "uncommitted").unwrap();

        repo.remove_worktree_force(&worktree).unwrap();
        assert!(!worktree.exists());
        assert!(repo.find_worktree(&worktree).unwrap().is_none());
    }
}
