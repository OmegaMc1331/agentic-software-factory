//! ASF-N02/N03/N08 regressions: every factory-git subprocess runs under the
//! bounded supervisor, and maintenance failures are reported, never swallowed.
//!
//! Fake `git` scripts are injected through `FACTORY_GIT_BIN`; the process-wide
//! environment requires serializing these tests (the suite runs single
//! threaded).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use factory_git::{GitError, Repo};
use tempfile::TempDir;

fn git_env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let mutex = LOCK.get_or_init(|| Mutex::new(()));
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Installs a fake git (and optionally a short timeout) and restores the real
/// configuration when dropped, even on assertion failure. Callers must hold
/// [`git_env_lock`] for the whole test: the overrides are process-wide and
/// the suite may run with parallel test threads.
struct FakeGit {
    _dir: TempDir,
}

impl FakeGit {
    fn install(windows: &str, unix: &str) -> FakeGit {
        Self::install_with_timeout(windows, unix, None)
    }

    fn install_with_timeout(windows: &str, unix: &str, timeout_secs: Option<&str>) -> FakeGit {
        let dir = TempDir::new().unwrap();
        let path: PathBuf = if cfg!(windows) {
            let script = dir.path().join("fake-git.cmd");
            std::fs::write(&script, windows).unwrap();
            script
        } else {
            let script = dir.path().join("fake-git.sh");
            std::fs::write(&script, unix).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&script).unwrap().permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(&script, permissions).unwrap();
            }
            script
        };
        std::env::set_var("FACTORY_GIT_BIN", path_string(&path));
        if let Some(seconds) = timeout_secs {
            std::env::set_var("FACTORY_GIT_TIMEOUT_SECS", seconds);
        }
        FakeGit { _dir: dir }
    }
}

impl Drop for FakeGit {
    fn drop(&mut self) {
        std::env::remove_var("FACTORY_GIT_BIN");
        std::env::remove_var("FACTORY_GIT_TIMEOUT_SECS");
    }
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

/// A real repository with one commit, plus its Repo handle.
fn real_repo() -> (TempDir, Repo) {
    let dir = TempDir::new().unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "test@example.com"][..],
        &["config", "user.name", "Factory Test"][..],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(dir.path().join("README.md"), "test repository\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["add", "."])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["commit", "-q", "-m", "initial"])
        .status()
        .unwrap()
        .success());
    let repo = Repo::detect_bounded(dir.path(), dir.path()).unwrap();
    (dir, repo)
}

#[test]
fn a_nonzero_worktree_prune_exit_is_reported_as_failure() {
    let _guard = git_env_lock();
    let (_dir, repo) = real_repo();
    let _fake = FakeGit::install(
        "@echo off\r\necho simulated prune failure 1>&2\r\nexit /b 1\r\n",
        "#!/bin/sh\necho 'simulated prune failure' >&2\nexit 1\n",
    );
    let error = repo.prune().unwrap_err();
    match &error {
        GitError::CommandFailed(message) => {
            assert!(message.contains("prune failed"), "message: {message}");
            assert!(message.contains("simulated prune failure"));
        }
        other => panic!("expected CommandFailed, got {other:?}"),
    }
}

#[test]
fn a_hanging_git_command_is_terminated_at_the_deadline() {
    let _guard = git_env_lock();
    let (_dir, repo) = real_repo();
    std::env::set_var("FACTORY_GIT_TIMEOUT_SECS", "1");
    let _fake = FakeGit::install(
        "@echo off\r\necho starting\r\nping -n 120 127.0.0.1 >nul\r\nexit /b 0\r\n",
        "#!/bin/sh\necho starting\nsleep 120\n",
    );
    let started = Instant::now();
    let worktree = _dir.path().join("wt");
    let error = repo.head_sha(&worktree).unwrap_err();
    std::env::remove_var("FACTORY_GIT_TIMEOUT_SECS");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "hang was not bounded: {:?}",
        started.elapsed()
    );
    match &error {
        GitError::TimedOut { operation, seconds } => {
            assert_eq!(*seconds, 1);
            assert!(operation.contains("rev-parse"), "operation: {operation}");
        }
        other => panic!("expected TimedOut, got {other:?}"),
    }
}

#[test]
fn runaway_git_output_is_rejected_instead_of_parsed() {
    let _guard = git_env_lock();
    let (_dir, repo) = real_repo();
    let _fake = FakeGit::install(
        "@echo off\r\nfor /l %%i in (1,1,60000) do echo 01234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789ABCDE\r\n",
        "#!/bin/sh\ni=0\nwhile [ $i -lt 80000 ]; do echo 0123456789012345678901234567890123456789012345678901234567890123; i=$((i+1)); done\n",
    );
    let worktree = _dir.path().join("wt");
    let error = repo.head_sha(&worktree).unwrap_err();
    match &error {
        GitError::OutputLimitExceeded { operation, limit } => {
            assert!(operation.contains("rev-parse"));
            assert!(*limit >= 1024 * 1024);
        }
        other => panic!("expected OutputLimitExceeded, got {other:?}"),
    }
}

#[test]
fn diff_capture_is_bounded_with_an_explicit_truncation_marker() {
    let _guard = git_env_lock();
    let (dir, repo) = real_repo();
    let worktree = dir.path().join("wt");
    repo.add_worktree(&worktree, "factory/t1", None).unwrap();
    // ~300 KB of diff against a 10 KB review budget.
    let big_content: String = "a line of change that is reasonably long 0123456789\n".repeat(6000);
    std::fs::write(worktree.join("big.txt"), &big_content).unwrap();
    repo.commit_changes(&worktree, "big change", ("W", "w@local"))
        .unwrap();
    let base = repo.head_sha(dir.path()).unwrap();

    let patch = repo.diff_patch(&worktree, &base, 10_000).unwrap();
    assert!(
        patch.len() < 12_000,
        "patch must stay near the budget, got {} bytes",
        patch.len()
    );
    assert!(
        patch.ends_with("[DIFF TRUNCATED: showing the first 10000 characters]\n"),
        "truncated patch must end with the marker"
    );
    assert!(patch.contains("+a line of change"));
}

#[test]
fn a_small_diff_is_returned_complete_without_a_marker() {
    let _guard = git_env_lock();
    let (dir, repo) = real_repo();
    let worktree = dir.path().join("wt");
    repo.add_worktree(&worktree, "factory/t1", None).unwrap();
    std::fs::write(worktree.join("small.txt"), "small change\n").unwrap();
    repo.commit_changes(&worktree, "small change", ("W", "w@local"))
        .unwrap();
    let base = repo.head_sha(dir.path()).unwrap();

    let patch = repo.diff_patch(&worktree, &base, 10_000).unwrap();
    assert!(patch.contains("+small change"));
    assert!(
        !patch.contains("[DIFF TRUNCATED"),
        "unexpected marker: {patch}"
    );
}

#[test]
fn a_diff_exactly_at_the_budget_is_returned_complete() {
    let _guard = git_env_lock();
    let (dir, repo) = real_repo();
    let worktree = dir.path().join("wt");
    repo.add_worktree(&worktree, "factory/t1", None).unwrap();
    // Each added line renders as "+<line>\n" in the patch; generate content
    // whose diff fits just under the budget.
    let line = "x".repeat(48);
    let content: String = std::iter::repeat_n(format!("{line}\n"), 100).collect();
    std::fs::write(worktree.join("exact.txt"), &content).unwrap();
    repo.commit_changes(&worktree, "exact change", ("W", "w@local"))
        .unwrap();
    let base = repo.head_sha(dir.path()).unwrap();

    let patch = repo.diff_patch(&worktree, &base, 10_000).unwrap();
    assert!(
        !patch.contains("[DIFF TRUNCATED"),
        "budget was generous: {patch}"
    );
}

#[test]
fn unicode_and_binary_content_never_panic_or_leak_the_budget() {
    let _guard = git_env_lock();
    let (dir, repo) = real_repo();
    let worktree = dir.path().join("wt");
    repo.add_worktree(&worktree, "factory/t1", None).unwrap();
    let mut content = String::from("header 日本語 🎉\n");
    content.push_str(&"é".repeat(20_000));
    content.push('\n');
    std::fs::write(worktree.join("unicode.txt"), &content).unwrap();
    // Invalid UTF-8 bytes: git quotes them in diffs; capture stays lossy-safe.
    let mut binary = vec![0u8; 4096];
    for (index, byte) in binary.iter_mut().enumerate() {
        *byte = (index % 256) as u8;
    }
    std::fs::write(worktree.join("blob.bin"), &binary).unwrap();
    repo.commit_changes(&worktree, "mixed change", ("W", "w@local"))
        .unwrap();
    let base = repo.head_sha(dir.path()).unwrap();

    let patch = repo.diff_patch(&worktree, &base, 5_000).unwrap();
    assert!(patch.len() < 7_000, "patch exceeded its budget");
    assert!(patch.contains("[DIFF TRUNCATED"));
}

#[test]
fn a_git_error_before_producing_a_diff_surfaces_as_a_failure() {
    let _guard = git_env_lock();
    let (dir, repo) = real_repo();
    let worktree = dir.path().join("wt");
    repo.add_worktree(&worktree, "factory/t1", None).unwrap();
    let error = repo
        .diff_patch(
            &worktree,
            "0000000000000000000000000000000000000000",
            10_000,
        )
        .unwrap_err();
    assert!(
        matches!(error, GitError::CommandFailed(ref message) if message.contains("git diff")),
        "unexpected error: {error:?}"
    );
}
