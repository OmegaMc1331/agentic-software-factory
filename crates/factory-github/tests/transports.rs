use std::path::Path;
use std::process::Command;

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
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn delivery_pushes_the_approved_commit_without_moving_refs_or_following_tags() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let bare = dir.path().join("remote.git");
    std::fs::create_dir(&work).unwrap();
    std::fs::create_dir(&bare).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    git(&bare, &["init", "--bare", "-q"]);
    git(&work, &["config", "user.name", "Transport Fixture"]);
    git(&work, &["config", "user.email", "fixture@example.invalid"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    git(&work, &["config", "push.followTags", "true"]);
    git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
    std::fs::write(work.join("source.txt"), "approved\n").unwrap();
    git(&work, &["add", "source.txt"]);
    git(&work, &["commit", "-qm", "approved"]);
    let approved = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["branch", "factory/run-1"]);
    git(&work, &["tag", "-a", "do-not-publish", "-m", "private tag"]);
    std::fs::write(work.join("source.txt"), "later unapproved\n").unwrap();
    git(&work, &["commit", "-qam", "later"]);
    let later = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["branch", "-f", "factory/run-1", &later]);
    factory_github::delivery::push_branch_at(&work, "origin", "factory/run-1", &approved).unwrap();
    assert_eq!(
        git(&bare, &["rev-parse", "refs/heads/factory/run-1"]),
        approved
    );
    assert!(git(&bare, &["tag", "--list"]).is_empty());
    assert_eq!(git(&work, &["rev-parse", "factory/run-1"]), later);
    factory_github::delivery::push_branch_at(&work, "origin", "factory/run-1", &later).unwrap();
    assert!(
        factory_github::delivery::push_branch_at(&work, "origin", "factory/run-1", &approved)
            .is_err()
    );
    assert_eq!(
        git(&bare, &["rev-parse", "refs/heads/factory/run-1"]),
        later
    );
    assert!(
        factory_github::delivery::push_branch_at(&work, "--all", "factory/run-1", &approved)
            .is_err()
    );
    assert!(factory_github::delivery::push_branch_at(&work, "origin", "main", &approved).is_err());
}

#[test]
fn v2_destination_guard_rejects_rewrites_insecure_extra_and_foreign_push_urls() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owner/project.git",
        ],
    );
    let verify =
        || factory_github::remote::verify_push_destination(dir.path(), "origin", "owner/project");
    verify().unwrap();
    git(
        dir.path(),
        &[
            "config",
            "remote.origin.pushurl",
            "https://github.com/other/project.git",
        ],
    );
    assert!(verify().is_err());
    git(
        dir.path(),
        &[
            "config",
            "remote.origin.pushurl",
            "http://github.com/owner/project.git",
        ],
    );
    assert!(verify().is_err());
    git(
        dir.path(),
        &[
            "config",
            "remote.origin.pushurl",
            "https://github.com/owner/project.git",
        ],
    );
    git(
        dir.path(),
        &[
            "config",
            "--add",
            "remote.origin.pushurl",
            "git@github.com:owner/project.git",
        ],
    );
    assert!(verify().is_err());
    git(
        dir.path(),
        &["config", "--unset-all", "remote.origin.pushurl"],
    );
    git(
        dir.path(),
        &[
            "config",
            "url.file:///unapproved/.pushInsteadOf",
            "https://github.com/",
        ],
    );
    assert!(verify().is_err());
    assert!(
        factory_github::remote::verify_push_destination(dir.path(), "--all", "owner/project")
            .is_err()
    );
}
