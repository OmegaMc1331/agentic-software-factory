use std::fs;
use std::path::Path;
use std::process::Command;

use factory_git::source_control::{Comparison, DiffRequest, IndexRequest};
use factory_git::{GitError, Repo};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}

fn fixture(committed: bool) -> (TempDir, Repo) {
    let temp = TempDir::new().unwrap();
    git(temp.path(), &["init", "-q", "-b", "main"]);
    git(temp.path(), &["config", "user.name", "Source Test"]);
    git(
        temp.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(temp.path(), &["config", "core.autocrlf", "false"]);
    git(temp.path(), &["config", "commit.gpgsign", "false"]);
    if committed {
        fs::write(
            temp.path().join("sample.txt"),
            (1..=30).map(|n| format!("line {n}\n")).collect::<String>(),
        )
        .unwrap();
        git(temp.path(), &["add", "."]);
        git(temp.path(), &["commit", "-qm", "baseline"]);
    }
    let repo = Repo::detect(temp.path()).unwrap();
    (temp, repo)
}

fn request(repo: &Repo, paths: &[&str], unstage: bool) -> IndexRequest {
    IndexRequest {
        expected_generation: repo.source_status().unwrap().generation,
        paths: paths.iter().map(|p| p.to_string()).collect(),
        unstage,
        hunk: None,
        diff_version: None,
        trust_programs: true,
    }
}

fn diff(repo: &Repo, path: &str, staged: bool) -> factory_git::source_control::Diff {
    repo.source_diff(&DiffRequest {
        path: path.into(),
        comparison: if staged {
            Comparison::IndexHead
        } else {
            Comparison::WorkingIndex
        },
        base: None,
        target: None,
    })
    .unwrap()
}

#[test]
fn clean_modified_untracked_unicode_and_special_names_are_real() {
    let (temp, repo) = fixture(true);
    assert!(repo.source_status().unwrap().changes.is_empty());
    fs::write(temp.path().join("sample.txt"), "changed\n").unwrap();
    for name in ["space name.txt", "métrique.txt", "-leading.txt"] {
        fs::write(temp.path().join(name), "new\n").unwrap();
    }
    let status = repo.source_status().unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.changes.len(), 4);
    assert!(status
        .changes
        .iter()
        .any(|c| c.path == "sample.txt" && c.working == "M"));
    repo.source_index(&request(
        &repo,
        &["space name.txt", "métrique.txt", "-leading.txt"],
        false,
    ))
    .unwrap();
    assert_eq!(
        git(
            temp.path(),
            &[
                "-c",
                "core.quotepath=false",
                "diff",
                "--cached",
                "--name-only"
            ]
        )
        .lines()
        .count(),
        3
    );
}

#[test]
fn runtime_evidence_preserves_original_renamed_paths_and_exact_untracked_names() {
    let (temp, repo) = fixture(true);
    fs::write(
        temp.path().join(".env"),
        "private fixture value\n".repeat(30),
    )
    .unwrap();
    git(temp.path(), &["add", "-f", "--", ".env"]);
    git(
        temp.path(),
        &["commit", "-qm", "protected historical source"],
    );
    let base = git(temp.path(), &["rev-parse", "HEAD"]);
    git(temp.path(), &["mv", ".env", "public.txt"]);
    fs::write(temp.path().join(" métrique space.txt"), "new\n").unwrap();
    let staged = repo.evidence_since(temp.path(), &base).unwrap();
    for path in [".env", "public.txt", " métrique space.txt"] {
        assert!(
            staged.changed_files.iter().any(|found| found == path),
            "{path}: {:?}",
            staged.changed_files
        );
    }
    git(
        temp.path(),
        &["commit", "-qm", "rename without dropping source identity"],
    );
    let committed = repo.evidence_since(temp.path(), &base).unwrap();
    assert!(committed.changed_files.contains(&".env".into()));
    assert!(committed.changed_files.contains(&"public.txt".into()));
}

#[test]
fn file_stage_and_unstage_preserve_working_source_and_unrelated_index_entries() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("sample.txt"), "working\n").unwrap();
    fs::write(temp.path().join("other.txt"), "other\n").unwrap();
    repo.source_index(&request(&repo, &["other.txt", "sample.txt"], false))
        .unwrap();
    repo.source_index(&request(&repo, &["sample.txt"], true))
        .unwrap();
    assert_eq!(
        git(temp.path(), &["diff", "--cached", "--name-only"]),
        "other.txt"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("sample.txt")).unwrap(),
        "working\n"
    );
}

#[test]
fn exact_hunk_stage_and_unstage_keep_the_other_hunk_unstaged() {
    let (temp, repo) = fixture(true);
    let path = temp.path().join("sample.txt");
    let content = fs::read_to_string(&path)
        .unwrap()
        .replace("line 2\n", "first change\n")
        .replace("line 28\n", "last change\n");
    fs::write(&path, &content).unwrap();
    let preview = diff(&repo, "sample.txt", false);
    assert_eq!(preview.hunks.len(), 2);
    let mut selected = request(&repo, &["sample.txt"], false);
    selected.hunk = Some(preview.hunks[0].id.clone());
    selected.diff_version = Some(preview.version);
    repo.source_index(&selected).unwrap();
    assert!(git(temp.path(), &["diff", "--cached"]).contains("first change"));
    assert!(!git(temp.path(), &["diff", "--cached"]).contains("last change"));
    assert!(git(temp.path(), &["diff"]).contains("last change"));
    let staged = diff(&repo, "sample.txt", true);
    let mut selected = request(&repo, &["sample.txt"], true);
    selected.hunk = Some(staged.hunks[0].id.clone());
    selected.diff_version = Some(staged.version);
    repo.source_index(&selected).unwrap();
    assert!(git(temp.path(), &["diff", "--cached"]).is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), content);
}

#[test]
fn stale_hunks_and_external_index_updates_are_rejected_without_changes() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("sample.txt"), "first\n").unwrap();
    let preview = diff(&repo, "sample.txt", false);
    let mut selected = request(&repo, &["sample.txt"], false);
    selected.hunk = Some(preview.hunks[0].id.clone());
    selected.diff_version = Some(preview.version);
    fs::write(temp.path().join("sample.txt"), "external\n").unwrap();
    assert!(matches!(repo.source_index(&selected), Err(GitError::Stale)));
    let expected = request(&repo, &["sample.txt"], false);
    fs::write(temp.path().join("other.txt"), "other\n").unwrap();
    git(temp.path(), &["add", "other.txt"]);
    assert!(matches!(repo.source_index(&expected), Err(GitError::Stale)));
    assert_eq!(
        git(temp.path(), &["diff", "--cached", "--name-only"]),
        "other.txt"
    );
}

#[test]
fn standard_index_lock_contention_fails_closed() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("sample.txt"), "change\n").unwrap();
    let request = request(&repo, &["sample.txt"], false);
    fs::write(temp.path().join(".git/index.lock"), "owned externally").unwrap();
    assert!(matches!(repo.source_index(&request), Err(GitError::Busy)));
    assert_eq!(
        fs::read_to_string(temp.path().join(".git/index.lock")).unwrap(),
        "owned externally"
    );
}

#[test]
fn local_commit_contains_only_index_and_returns_verified_identity() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("sample.txt"), "staged\n").unwrap();
    repo.source_index(&request(&repo, &["sample.txt"], false))
        .unwrap();
    fs::write(temp.path().join("sample.txt"), "unstaged\n").unwrap();
    let status = repo.source_status().unwrap();
    let sha = repo
        .source_commit(&status.generation, "local change", true)
        .unwrap();
    assert_eq!(sha, git(temp.path(), &["rev-parse", "HEAD"]));
    assert_eq!(git(temp.path(), &["show", "HEAD:sample.txt"]), "staged");
    assert_eq!(
        fs::read_to_string(temp.path().join("sample.txt")).unwrap(),
        "unstaged\n"
    );
    assert!(matches!(
        repo.source_commit(&status.generation, "duplicate", true),
        Err(GitError::Stale)
    ));
    let history = repo.source_history(None, 0, 20).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].sha, sha);
    assert_eq!(history[0].subject, "local change");
}

#[test]
fn unborn_stage_unstage_and_first_commit_work() {
    let (temp, repo) = fixture(false);
    assert!(repo.source_status().unwrap().head.is_none());
    fs::write(temp.path().join("new.txt"), "new\n").unwrap();
    repo.source_index(&request(&repo, &["new.txt"], false))
        .unwrap();
    repo.source_index(&request(&repo, &["new.txt"], true))
        .unwrap();
    repo.source_index(&request(&repo, &["new.txt"], false))
        .unwrap();
    let sha = repo
        .source_commit(&repo.source_status().unwrap().generation, "first", true)
        .unwrap();
    assert_eq!(sha, git(temp.path(), &["rev-parse", "HEAD"]));
}

#[test]
fn deleted_and_renamed_files_are_preserved_in_status() {
    let (temp, repo) = fixture(true);
    git(temp.path(), &["mv", "sample.txt", "renamed file.txt"]);
    let status = repo.source_status().unwrap();
    let rename = status
        .changes
        .iter()
        .find(|c| c.path == "renamed file.txt")
        .unwrap();
    assert_eq!(rename.original_path.as_deref(), Some("sample.txt"));
    fs::remove_file(temp.path().join("renamed file.txt")).unwrap();
    assert_eq!(repo.source_status().unwrap().changes[0].working, "D");
}

#[test]
fn branches_reject_dirty_worktrees_and_protect_coordinator_refs() {
    let (temp, repo) = fixture(true);
    repo.source_branch(
        &repo.source_status().unwrap().generation,
        "feature/local",
        true,
        true,
    )
    .unwrap();
    assert!(repo
        .source_branches()
        .unwrap()
        .iter()
        .any(|b| b.name == "feature/local" && b.current));
    fs::write(temp.path().join("sample.txt"), "dirty\n").unwrap();
    assert!(repo
        .source_branch(
            &repo.source_status().unwrap().generation,
            "main",
            false,
            true
        )
        .is_err());
    assert!(repo
        .source_branch(
            &repo.source_status().unwrap().generation,
            "factory/run-1",
            true,
            true
        )
        .is_err());
    git(temp.path(), &["checkout", "--detach", "-q"]);
    assert!(repo.source_status().unwrap().branch.is_none());
}

#[test]
fn hooks_require_trust_and_failures_do_not_claim_a_commit() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("sample.txt"), "changed\n").unwrap();
    repo.source_index(&request(&repo, &["sample.txt"], false))
        .unwrap();
    let hook = temp.path().join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        "#!/bin/sh\necho 'bounded hook refusal' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let before = git(temp.path(), &["rev-parse", "HEAD"]);
    let status = repo.source_status().unwrap();
    assert!(matches!(
        repo.source_commit(&status.generation, "denied", false),
        Err(GitError::TrustRequired(_))
    ));
    assert!(repo
        .source_commit(&status.generation, "hook runs", true)
        .is_err());
    assert_eq!(before, git(temp.path(), &["rev-parse", "HEAD"]));
    assert!(!git(temp.path(), &["diff", "--cached"]).is_empty());
}

#[test]
fn filters_sensitive_paths_and_argument_injection_remain_denied() {
    let (temp, repo) = fixture(true);
    for path in [
        "../escape",
        ".git/config",
        ".factory/db.sqlite3",
        ".env",
        "file:stream",
        "a/../../b",
    ] {
        assert!(repo.source_index(&request(&repo, &[path], false)).is_err());
    }
    for branch in [
        "--help",
        "bad..ref",
        "x@{y",
        "factory/run-8",
        "refs/heads/main",
    ] {
        assert!(factory_git::source_control::validate_branch(branch).is_err());
    }
    git(
        temp.path(),
        &["config", "filter.evil.clean", "unexpected-program"],
    );
    fs::write(temp.path().join(".gitattributes"), "safe.txt filter=evil\n").unwrap();
    fs::write(temp.path().join("safe.txt"), "safe\n").unwrap();
    assert!(matches!(
        repo.source_index(&request(&repo, &["safe.txt"], false)),
        Err(GitError::Protected(_))
    ));
}

#[test]
fn binary_diffs_are_accurately_unavailable_for_hunk_staging() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("binary.dat"), [0, 1, 2, 3]).unwrap();
    let d = diff(&repo, "binary.dat", false);
    assert!(d.limited);
    assert!(d.before.is_some());
    assert!(d.after.is_none());
    assert!(d.hunks.is_empty());
}

#[test]
fn new_and_deleted_text_hunks_modify_only_the_index() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("new file.txt"), "new text\n").unwrap();
    let preview = diff(&repo, "new file.txt", false);
    let mut selected = request(&repo, &["new file.txt"], false);
    selected.hunk = Some(preview.hunks[0].id.clone());
    selected.diff_version = Some(preview.version);
    repo.source_index(&selected).unwrap();
    assert_eq!(git(temp.path(), &["show", ":new file.txt"]), "new text");
    let preview = diff(&repo, "new file.txt", true);
    let mut selected = request(&repo, &["new file.txt"], true);
    selected.hunk = Some(preview.hunks[0].id.clone());
    selected.diff_version = Some(preview.version);
    repo.source_index(&selected).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("new file.txt")).unwrap(),
        "new text\n"
    );
    fs::remove_file(temp.path().join("sample.txt")).unwrap();
    let preview = diff(&repo, "sample.txt", false);
    let mut selected = request(&repo, &["sample.txt"], false);
    selected.hunk = Some(preview.hunks[0].id.clone());
    selected.diff_version = Some(preview.version);
    repo.source_index(&selected).unwrap();
    assert!(git(temp.path(), &["diff", "--cached", "--name-status"]).starts_with("D"));
}

#[test]
fn historical_rename_uses_actual_original_blob_and_invalid_commit_is_not_empty_success() {
    let (temp, repo) = fixture(true);
    let base = git(temp.path(), &["rev-parse", "HEAD"]);
    let original = fs::read_to_string(temp.path().join("sample.txt")).unwrap();
    git(temp.path(), &["mv", "sample.txt", "renamed.txt"]);
    git(temp.path(), &["commit", "-qm", "rename"]);
    let head = git(temp.path(), &["rev-parse", "HEAD"]);
    let preview = repo
        .source_diff(&DiffRequest {
            path: "renamed.txt".into(),
            comparison: Comparison::Commits,
            base: Some(base),
            target: Some(head),
        })
        .unwrap();
    assert_eq!(preview.before.as_deref(), Some(original.as_str()));
    assert_eq!(preview.before, preview.after);
    assert!(repo
        .source_diff(&DiffRequest {
            path: "renamed.txt".into(),
            comparison: Comparison::Commits,
            base: Some("f".repeat(40)),
            target: Some("e".repeat(40))
        })
        .is_err());
}

#[test]
fn crlf_and_oversized_changes_do_not_pretend_to_support_safe_hunks() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("sample.txt"), "modified\r\n").unwrap();
    assert!(diff(&repo, "sample.txt", false).hunks.is_empty());
    fs::write(
        temp.path().join("oversized.txt"),
        vec![b'x'; 1024 * 1024 + 1],
    )
    .unwrap();
    let status = repo.source_status().unwrap();
    assert!(
        !status
            .changes
            .iter()
            .find(|c| c.path == "oversized.txt")
            .unwrap()
            .supported
    );
    assert!(repo
        .source_index(&request(&repo, &["oversized.txt"], false))
        .is_err());
    assert!(diff(&repo, "oversized.txt", false).limited);
}

#[test]
fn unmerged_index_stages_are_real_and_no_side_is_automatically_chosen() {
    let (temp, repo) = fixture(true);
    git(temp.path(), &["switch", "-c", "left"]);
    fs::write(temp.path().join("sample.txt"), "left\n").unwrap();
    git(temp.path(), &["commit", "-am", "left"]);
    git(temp.path(), &["switch", "main"]);
    fs::write(temp.path().join("sample.txt"), "right\n").unwrap();
    git(temp.path(), &["commit", "-am", "right"]);
    let output = Command::new("git")
        .arg("-C")
        .arg(temp.path())
        .args(["merge", "left"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let status = repo.source_status().unwrap();
    assert!(status.changes.iter().any(|c| c.conflict));
    let stages = repo.source_conflict("sample.txt").unwrap();
    assert_eq!(stages["ours"], "right\n");
    assert_eq!(stages["theirs"], "left\n");
    assert_eq!(stages["readOnly"], true);
    let before = fs::read(temp.path().join("sample.txt")).unwrap();
    assert!(repo
        .source_index(&request(&repo, &["sample.txt"], false))
        .is_err());
    assert_eq!(before, fs::read(temp.path().join("sample.txt")).unwrap());
}

#[test]
fn read_only_status_never_executes_a_configured_clean_filter() {
    let (temp, repo) = fixture(true);
    let marker = temp.path().join("FILTER_WAS_RUN");
    let script = temp.path().join("filter.py");
    fs::write(&script,format!("from pathlib import Path\nimport sys\nPath({:?}).write_text('executed')\nsys.stdout.write(sys.stdin.read())\n",marker.to_string_lossy())).unwrap();
    fs::write(
        temp.path().join(".gitattributes"),
        "sample.txt filter=inspection-guard\n",
    )
    .unwrap();
    let program = if cfg!(windows) { "python" } else { "python3" };
    git(
        temp.path(),
        &[
            "config",
            "filter.inspection-guard.clean",
            &format!("{program} \"{}\"", script.display()),
        ],
    );
    fs::write(temp.path().join("sample.txt"), "changed raw data\n").unwrap();
    repo.source_status().unwrap();
    diff(&repo, "sample.txt", false);
    assert!(
        !marker.exists(),
        "Read-only inspection must not execute clean filters"
    );
    assert!(repo
        .source_index(&request(&repo, &["sample.txt"], false))
        .is_err());
    assert!(!marker.exists());
}

#[test]
fn disabled_helpers_do_not_require_unnecessary_program_authorization() {
    let (temp, repo) = fixture(true);
    fs::write(temp.path().join("sample.txt"), "safe plain change\n").unwrap();
    let mut selected = request(&repo, &["sample.txt"], false);
    selected.trust_programs = false;
    let staged = repo.source_index(&selected).unwrap();
    repo.source_commit(&staged.generation, "plain safe commit", false)
        .unwrap();
}

#[test]
fn runtime_worktree_waits_for_shared_lock_without_the_manual_ui_deadline() {
    let (temp, repo) = fixture(true);
    let held = repo.mutation_guard().unwrap();
    let root = temp.path().to_path_buf();
    let target = temp.path().join("runtime-worktree");
    let worker_target = target.clone();
    let (started, waiting) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let repo = Repo::detect(&root).unwrap();
        started.send(()).unwrap();
        repo.add_worktree(&worker_target, "factory/queued-runtime", None)
    });
    waiting
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(6));
    assert!(
        !worker.is_finished(),
        "Runtime mutation failed at the manual wait deadline"
    );
    assert!(!target.exists(), "Shared lock was bypassed");
    drop(held);
    worker.join().unwrap().unwrap();
    assert!(target.join("sample.txt").exists());
    assert_eq!(git(temp.path(), &["branch", "--show-current"]), "main");
    assert_eq!(
        git(&target, &["branch", "--show-current"]),
        "factory/queued-runtime"
    );
}

#[test]
fn protected_ref_case_aliases_are_rejected_before_mutation() {
    let (temp, repo) = fixture(true);
    git(temp.path(), &["branch", "factory/reserved"]);
    let initial = git(temp.path(), &["rev-parse", "HEAD"]);
    for name in [
        "Factory/reserved",
        "FACTORY/reserved",
        "FaCtOrY/new",
        "Refs/heads/main",
    ] {
        for create in [false, true] {
            assert!(
                matches!(
                    repo.source_branch(
                        &repo.source_status().unwrap().generation,
                        name,
                        create,
                        false
                    ),
                    Err(GitError::Invalid(_))
                ),
                "Reserved namespace alias was accepted: {name}"
            );
            assert_eq!(git(temp.path(), &["branch", "--show-current"]), "main");
            assert_eq!(git(temp.path(), &["rev-parse", "HEAD"]), initial);
        }
    }
    git(temp.path(), &["checkout", "-q", "-b", "FaCtOrY/current"]);
    fs::write(
        temp.path().join("sample.txt"),
        "externally staged partial work\n",
    )
    .unwrap();
    git(temp.path(), &["add", "sample.txt"]);
    let staged = git(temp.path(), &["diff", "--cached"]);
    assert!(matches!(
        repo.source_index(&request(&repo, &["sample.txt"], true)),
        Err(GitError::Protected(_))
    ));
    assert!(matches!(
        repo.source_commit(
            &repo.source_status().unwrap().generation,
            "Denied reserved ref commit",
            false
        ),
        Err(GitError::Protected(_))
    ));
    assert_eq!(git(temp.path(), &["diff", "--cached"]), staged);
    assert_eq!(git(temp.path(), &["rev-parse", "HEAD"]), initial);
}

#[test]
fn branch_spelling_and_registered_worktree_ownership_are_enforced() {
    let (temp, repo) = fixture(true);
    let other = TempDir::new().unwrap();
    let worktree = other.path().join("owned");
    git(
        temp.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/owned",
            worktree.to_str().unwrap(),
        ],
    );
    for (name, create) in [
        ("Feature/owned", false),
        ("FEATURE/owned", true),
        ("feature/owned", false),
    ] {
        assert!(repo
            .source_branch(
                &repo.source_status().unwrap().generation,
                name,
                create,
                true
            )
            .is_err());
        assert_eq!(git(temp.path(), &["branch", "--show-current"]), "main");
    }
    let refs = repo.source_branches().unwrap();
    assert!(refs
        .iter()
        .any(|branch| branch.name == "feature/owned" && !branch.worktree.is_empty()));
    if cfg!(windows) {
        git(
            temp.path(),
            &["symbolic-ref", "HEAD", "refs/heads/Feature/owned"],
        );
        fs::write(temp.path().join("sample.txt"), "preserved partial work\n").unwrap();
        let head = git(temp.path(), &["rev-parse", "HEAD"]);
        assert!(matches!(
            repo.source_index(&request(&repo, &["sample.txt"], false)),
            Err(GitError::Protected(_))
        ));
        assert_eq!(
            fs::read_to_string(temp.path().join("sample.txt")).unwrap(),
            "preserved partial work\n"
        );
        assert_eq!(git(temp.path(), &["rev-parse", "HEAD"]), head);
    }
    git(
        temp.path(),
        &["symbolic-ref", "HEAD", "refs/heads/feature/owned"],
    );
    fs::write(temp.path().join("sample.txt"), "preserved partial work\n").unwrap();
    assert!(matches!(
        repo.source_index(&request(&repo, &["sample.txt"], false)),
        Err(GitError::Protected(_))
    ));
}

#[cfg(windows)]
#[test]
fn unicode_worktree_aliases_require_canonical_ref_ownership() {
    let (temp, repo) = fixture(true);
    let other = TempDir::new().unwrap();
    let worktree = other.path().join("unicode-owned");
    git(
        temp.path(),
        &[
            "worktree",
            "add",
            "-b",
            "f\u{e9}ature/owned",
            worktree.to_str().unwrap(),
        ],
    );
    git(
        &worktree,
        &["symbolic-ref", "HEAD", "refs/heads/f\u{c9}ature/owned"],
    );
    let initial = git(temp.path(), &["rev-parse", "HEAD"]);
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), initial);
    assert!(matches!(
        repo.source_branch(
            &repo.source_status().unwrap().generation,
            "f\u{e9}ature/owned",
            false,
            true
        ),
        Err(GitError::Protected(_))
    ));
    assert_eq!(git(temp.path(), &["branch", "--show-current"]), "main");
    fs::write(
        temp.path().join("sample.txt"),
        "preserved during uncertain ownership\n",
    )
    .unwrap();
    assert!(matches!(
        repo.source_index(&request(&repo, &["sample.txt"], false)),
        Err(GitError::Protected(_))
    ));
    assert_eq!(git(temp.path(), &["rev-parse", "HEAD"]), initial);
    assert!(git(temp.path(), &["diff", "--cached"]).is_empty());
    assert_eq!(
        fs::read_to_string(temp.path().join("sample.txt")).unwrap(),
        "preserved during uncertain ownership\n"
    );
}

#[test]
fn large_filter_batches_inspect_every_path_and_preserve_the_index_on_refusal() {
    let (temp, repo) = fixture(true);
    for n in 0..1100 {
        fs::write(
            temp.path().join(format!("batch_{n:04}.txt")),
            "owned fixture\n",
        )
        .unwrap();
    }
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "many tracked paths"]);
    repo.source_trust(false).unwrap();
    fs::write(
        temp.path().join(".gitattributes"),
        "batch_1099.txt filter=unapproved\n",
    )
    .unwrap();
    let index = git(temp.path(), &["diff", "--cached"]);
    let head = git(temp.path(), &["rev-parse", "HEAD"]);
    assert!(matches!(
        repo.source_trust(false),
        Err(GitError::Protected(_))
    ));
    assert_eq!(git(temp.path(), &["diff", "--cached"]), index);
    assert_eq!(git(temp.path(), &["rev-parse", "HEAD"]), head);
}

#[test]
fn large_literal_selections_preserve_working_files_and_unrelated_index_entries() {
    for committed in [false, true] {
        let (temp, repo) = fixture(committed);
        fs::write(temp.path().join("unrelated.txt"), "already staged\n").unwrap();
        git(temp.path(), &["add", "--", "unrelated.txt"]);
        let paths: Vec<_> = (0..600)
            .map(|n| format!("-m\u{e9}trique {n:04}_{}.txt", "bounded".repeat(15)))
            .collect();
        for path in &paths {
            fs::write(temp.path().join(path), "selected source\n").unwrap();
        }
        let names: Vec<_> = paths.iter().map(String::as_str).collect();
        repo.source_index(&request(&repo, &names, false)).unwrap();
        assert_eq!(
            git(temp.path(), &["diff", "--cached", "--name-only", "-z"])
                .split('\0')
                .filter(|p| !p.is_empty())
                .count(),
            601
        );
        let stale = request(&repo, &names, true);
        fs::write(temp.path().join(&paths[599]), "newer source\n").unwrap();
        assert!(matches!(repo.source_index(&stale), Err(GitError::Stale)));
        fs::write(temp.path().join(&paths[599]), "selected source\n").unwrap();
        repo.source_index(&request(&repo, &names, true)).unwrap();
        assert_eq!(
            git(temp.path(), &["diff", "--cached", "--name-only"]),
            "unrelated.txt"
        );
        for path in &paths {
            assert_eq!(
                fs::read_to_string(temp.path().join(path)).unwrap(),
                "selected source\n"
            );
        }
        fs::write(
            temp.path().join(".gitattributes"),
            format!("\"{}\" filter=not-authorized\n", paths[599]),
        )
        .unwrap();
        assert!(matches!(
            repo.source_index(&request(&repo, &names, false)),
            Err(GitError::Protected(_))
        ));
        assert_eq!(
            git(temp.path(), &["diff", "--cached", "--name-only"]),
            "unrelated.txt"
        );
        let mut excessive = request(&repo, &["unrelated.txt"], true);
        excessive.paths = vec!["unrelated.txt".into(); 10_001];
        assert!(matches!(
            repo.source_index(&excessive),
            Err(GitError::Invalid(_))
        ));
        excessive.paths = vec!["x".repeat(2048); 600];
        assert!(matches!(
            repo.source_index(&excessive),
            Err(GitError::Invalid(_))
        ));
    }
}

#[test]
fn packed_selection_matches_native_index_and_preserves_line_endings() {
    let (temp, repo) = fixture(true);
    let (reference, _) = fixture(true);
    let paths = (0..270)
        .map(|n| format!("-caf\u{e9} {n:03}.txt"))
        .collect::<Vec<_>>();
    let mut paths = paths;
    for root in [temp.path(), reference.path()] {
        fs::write(root.join(".gitattributes"), "*-convert.txt text eol=lf\n").unwrap();
        git(root, &["add", "--", ".gitattributes"]);
        git(root, &["commit", "-qm", "text conversion rules"]);
        for (n, path) in paths.iter().enumerate() {
            fs::write(root.join(path), format!("line {n}\n")).unwrap();
        }
        fs::write(
            root.join("one-convert.txt"),
            "preserve the working copy\r\n",
        )
        .unwrap();
        fs::write(root.join("sample.txt"), "unrelated change\n").unwrap();
    }
    paths.push("one-convert.txt".into());
    if cfg!(unix) {
        let name = "line\nbreak.txt";
        fs::write(temp.path().join(name), "preserve this unsupported name\n").unwrap();
        let portable = repo.source_status().unwrap();
        assert!(portable
            .changes
            .iter()
            .any(|change| change.path == name && !change.supported));
        assert!(matches!(
            repo.source_index(&request(&repo, &[name], false)),
            Err(GitError::Invalid(_))
        ));
        assert_eq!(
            fs::read_to_string(temp.path().join(name)).unwrap(),
            "preserve this unsupported name\n"
        );
    }
    let selected = paths.iter().map(String::as_str).collect::<Vec<_>>();
    let staging = repo
        .source_index(&request(&repo, &selected, false))
        .unwrap();
    let mut args = vec!["add", "--all", "--"];
    args.extend(selected);
    git(reference.path(), &args);
    let reference_tree = git(reference.path(), &["write-tree"]);
    assert_eq!(git(temp.path(), &["write-tree"]), reference_tree);
    assert_eq!(
        fs::read(temp.path().join("one-convert.txt")).unwrap(),
        b"preserve the working copy\r\n"
    );
    if cfg!(windows) {
        assert!(fs::read_dir(temp.path().join(".git/objects/pack"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|suffix| suffix == "pack")));
    }
    let refreshed = repo.source_status().unwrap();
    assert_eq!(staging.changes.len(), refreshed.changes.len());
    let head = git(temp.path(), &["rev-parse", "HEAD"]);
    let refused =
        repo.source_commit_authorized(&refreshed.generation, "policy probe", false, |path| {
            if path == "one-convert.txt" {
                Err(GitError::Protected("fixture path policy".into()))
            } else {
                Ok(())
            }
        });
    assert!(matches!(refused, Err(GitError::Protected(_))));
    assert_eq!(git(temp.path(), &["rev-parse", "HEAD"]), head);
    let sha = repo
        .source_commit(&refreshed.generation, "exact index", false)
        .unwrap();
    assert_eq!(
        git(temp.path(), &["rev-parse", &format!("{sha}^{{tree}}")]),
        reference_tree
    );
    assert_eq!(git(temp.path(), &["diff", "--name-only"]), "sample.txt");
}
