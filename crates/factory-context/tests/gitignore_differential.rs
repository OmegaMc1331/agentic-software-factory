//! ASF-N07 differential suite: the Context Engine's ignore decisions are
//! compared against real `git check-ignore` on a controlled repository with
//! isolated configuration (no user-global excludes), and ignored files are
//! proven to stay out of the index, content search, and resolved context.

use std::path::{Path, PathBuf};
use std::process::Command;

use factory_context::IgnoreRules;
use tempfile::TempDir;

struct DifferentialRepo {
    dir: TempDir,
    candidates: Vec<(PathBuf, bool)>,
    empty_excludes: PathBuf,
}

fn run_git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// A repository whose ignore setup exercises every rule layer Factory honors.
fn differential_repo() -> DifferentialRepo {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    assert!(run_git(root, &["init", "-q", "-b", "main"]));

    write(
        &root.join(".gitignore"),
        "*.log\n\
         build/\n\
         /generated\n\
         secrets/\n\
         !keep.log\n\
         données/\n\
         *.tmp\n\
         !important.tmp\n",
    );
    write(&root.join(".git/info/exclude"), "local-only.txt\n");
    write(
        &root.join("nested/.gitignore"),
        "draft.md\n!final-draft.md\n",
    );
    write(&root.join("nested/deep/.gitignore"), "*.internal\n");

    // Fixture files: (relative path, is_directory). Builtin-covered names
    // (target/, *.lock, ...) are excluded from the differential because
    // Factory ignores them even without any git rule — a documented,
    // intentional divergence.
    let mut candidates: Vec<(PathBuf, bool)> = Vec::new();
    for (path, is_dir) in [
        ("app.log", false),
        ("keep.log", false),
        ("nested/app.log", false),
        ("build", true),
        ("build/artifact.rs", false),
        ("generated", true),
        ("nested/generated", false),
        ("secrets/token.txt", false),
        ("données", true),
        ("données/entrée.md", false),
        ("local-only.txt", false),
        ("nested/local-only.txt", false),
        ("nested/draft.md", false),
        ("nested/final-draft.md", false),
        ("nested/deep/thing.internal", false),
        ("nested/thing.internal", false),
        ("app.tmp", false),
        ("important.tmp", false),
        ("src/main.rs", false),
        (".hidden/file.txt", false),
    ] {
        let full = root.join(path);
        if is_dir {
            std::fs::create_dir_all(&full).unwrap();
        } else {
            write(&full, "content\n");
        }
        candidates.push((PathBuf::from(path), is_dir));
    }

    let empty_excludes = root.join("empty-excludes");
    std::fs::write(&empty_excludes, "").unwrap();
    DifferentialRepo {
        dir,
        candidates,
        empty_excludes,
    }
}

/// `git check-ignore` decision for one path under isolated configuration:
/// exit 0 means ignored, exit 1 means not ignored.
fn git_ignores(repo: &DifferentialRepo, relative: &Path) -> Option<bool> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo.dir.path())
        .arg("-c")
        .arg(format!(
            "core.excludesFile={}",
            repo.empty_excludes.display()
        ))
        .args(["check-ignore", "-q", "--"])
        .arg(relative)
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

#[test]
fn ignore_decisions_match_git_check_ignore() {
    let repo = differential_repo();
    let rules = IgnoreRules::collect(repo.dir.path());
    let mut mismatches = Vec::new();
    for (relative, is_dir) in &repo.candidates {
        let Some(git_says) = git_ignores(&repo, relative) else {
            panic!("git check-ignore failed for {}", relative.display());
        };
        let factory_says = rules.is_ignored(relative, *is_dir);
        if git_says != factory_says {
            mismatches.push(format!(
                "{}: git={git_says} factory={factory_says}",
                relative.display()
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "ignore decisions diverged from git:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn ignored_files_stay_out_of_search_and_index() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    assert!(run_git(root, &["init", "-q", "-b", "main"]));
    write(&root.join(".gitignore"), "secrets/\n*.scratch\n");
    // The ignored file carries fake credentials; the normal file shares the
    // search term so the search itself demonstrably runs.
    write(
        &root.join("secrets/creds.txt"),
        "FAKE_TOKEN=super-secret-fake-value-xyz\n",
    );
    write(&root.join("notes.scratch"), "FAKE_TOKEN also here\n");
    write(
        &root.join("src/notes.txt"),
        "FAKE_TOKEN mentioned in docs\n",
    );

    let rules = IgnoreRules::collect(root);
    let hits = factory_context::search::search_content(root, &rules, "FAKE_TOKEN", 10).unwrap();
    assert!(
        !hits.is_empty(),
        "the search must find the non-ignored match"
    );
    for hit in &hits {
        assert!(
            !hit.path.starts_with("secrets/"),
            "ignored directory leaked into search: {}",
            hit.path
        );
        assert!(
            !hit.path.ends_with(".scratch"),
            "ignored file leaked into search: {}",
            hit.path
        );
    }

    let factory_dir = root.join(".factory");
    std::fs::create_dir_all(&factory_dir).unwrap();
    let mut engine = factory_context::ContextEngine::new(
        root,
        &factory_dir,
        factory_context::ContextConfig::default(),
    );
    engine.ensure_index().unwrap();
    let mut indexed: Vec<String> = Vec::new();
    // search() reuses the index; a hit for the term proves the index walk
    // ran, and neither ignored path may appear.
    let hits = engine.search("FAKE_TOKEN", 10).unwrap();
    for hit in &hits {
        indexed.push(hit.path.clone());
    }
    for path in &indexed {
        assert!(
            !path.starts_with("secrets/") && !path.ends_with(".scratch"),
            "ignored path leaked through the index: {path}"
        );
    }
    assert!(
        indexed
            .iter()
            .any(|path| path.replace('\\', "/") == "src/notes.txt"),
        "the non-ignored hit must be present: {indexed:?}"
    );
}

#[test]
fn changing_ignore_rules_between_refreshes_updates_the_index() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    assert!(run_git(root, &["init", "-q", "-b", "main"]));
    write(&root.join("keep.rs"), "pub fn keep() {}\n");
    write(&root.join("dropped.rs"), "pub fn dropped() {}\n");

    let factory_dir = root.join(".factory");
    std::fs::create_dir_all(&factory_dir).unwrap();

    // First refresh: both files indexed.
    let mut engine = factory_context::ContextEngine::new(
        root,
        &factory_dir,
        factory_context::ContextConfig::default(),
    );
    engine.ensure_index().unwrap();
    assert!(!engine.search("dropped", 10).unwrap().is_empty());

    // Now the repository starts ignoring dropped.rs; a fresh engine (the
    // normal per-operation construction) must drop it from the index and
    // search, and re-include it when the rule disappears again.
    write(&root.join(".gitignore"), "dropped.rs\n");
    let mut engine = factory_context::ContextEngine::new(
        root,
        &factory_dir,
        factory_context::ContextConfig::default(),
    );
    engine.ensure_index().unwrap();
    let hits = engine.search("dropped", 10).unwrap();
    assert!(
        hits.iter()
            .all(|hit| hit.path.replace('\\', "/") != "dropped.rs"),
        "newly ignored file must leave the index: {hits:?}"
    );
    let keep_hits = engine.search("keep", 10).unwrap();
    assert!(!keep_hits.is_empty(), "included files stay indexed");

    std::fs::remove_file(root.join(".gitignore")).unwrap();
    let mut engine = factory_context::ContextEngine::new(
        root,
        &factory_dir,
        factory_context::ContextConfig::default(),
    );
    engine.ensure_index().unwrap();
    assert!(
        !engine.search("dropped", 10).unwrap().is_empty(),
        "newly unignored file becomes indexable again"
    );
}
