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

/// Like [`write`] but tolerates platforms that forbid the filename
/// (literal-wildcard fixtures on Windows).
fn write_best_effort(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, content);
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

// --- ASF-F02: exact path semantics, expanded differential matrix -----------

/// Builds a repository whose ignore setup covers every rule class the
/// parser supports, with candidate paths that distinguish correct anchoring
/// from suffix-matching mistakes.
fn comprehensive_repo() -> DifferentialRepo {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    assert!(run_git(root, &["init", "-q", "-b", "main"]));

    // ASF-F02's original failing scenario first: a negated slash-containing
    // pattern must re-include exactly the named path, not deeper copies.
    write(
        &root.join(".gitignore"),
        "*.env\n\
         !dir/allowed.env\n\
         *.log\n\
         secret.txt\n\
         temp*\n\
         dir/file.txt\n\
         nested/output.log\n\
         src/generated/*.rs\n\
         /generated\n\
         /src/generated/keep.txt\n\
         build/\n\
         cache/\n\
         secrets/\n\
         !secrets/allowed.txt\n\
         !plain-keep.txt\n\
         **/anywhere/\n\
         **/drafts\n\
         logs/**\n\
         a/**/b\n\
         what\\?q\n\
         star\\*literal\n\
         hash\\#tag\n\
         bang\\!tag\n\
         escaped\\ space.txt\n\
         données/\n\
         résumé.md\n\
         日本語/\n\
         文件/**\n\
         emoji-😀.txt\n",
    );
    write(
        &root.join("nested/.gitignore"),
        "local.txt\n!nested-keep.txt\n",
    );
    write(
        &root.join("nested/deep/.gitignore"),
        "*.internal\n!deep-keep.internal\n",
    );
    write(&root.join(".git/info/exclude"), "exclude-only.txt\n");

    let mut candidates: Vec<(String, bool)> = Vec::new();
    let mut add = |path: &str, is_dir: bool| candidates.push((path.to_string(), is_dir));
    // F02 scenario.
    add("dir/allowed.env", false);
    add("nested/dir/allowed.env", false);
    add("deep/nested/dir/allowed.env", false);
    add("plain.env", false);
    // Basename patterns at several depths.
    add("a.log", false);
    add("x/y/z.log", false);
    add("secret.txt", false);
    add("nested/secret.txt", false);
    add("tempfile", false);
    add("src/temp-data.rs", false);
    add("atemp", false);
    // Slash-containing patterns relative to the root.
    add("dir/file.txt", false);
    add("other/dir/file.txt", false);
    add("nested/output.log", false);
    add("nested/deeper/output.log", false);
    add("src/generated/mod.rs", false);
    add("src/generated/deep/mod.rs", false);
    add("other/src/generated/mod.rs", false);
    // Root-anchored patterns.
    add("generated", true);
    add("nested/generated", true);
    add("src/generated/keep.txt", false);
    add("nested/src/generated/keep.txt", false);
    // Directory-only rules.
    add("build", true);
    add("x/build", true);
    add("build/keep.rs", false);
    add("x/build/keep.rs", false);
    add("cache", true);
    add("cache/deep/c.txt", false);
    add("cache.txt", false);
    // Negation and parent-directory restriction.
    add("secrets", true);
    add("secrets/allowed.txt", false);
    add("secrets/deep/allowed.txt", false);
    add("secrets/token.key", false);
    add("plain-keep.txt", false);
    add("nested/plain-keep.txt", false);
    // Leading `**` matches at any depth including the root.
    add("anywhere", true);
    add("x/anywhere", true);
    add("x/y/anywhere", true);
    add("drafts", true);
    add("x/drafts", true);
    add("drafts.txt", false);
    // Trailing `/**` matches everything inside, not the dir itself.
    add("logs", true);
    add("logs/a.txt", false);
    add("logs/d/e.txt", false);
    // Middle `/**/` matches zero or more directories.
    add("a/b", false);
    add("a/x/b", false);
    add("a/x/y/b", false);
    add("z/a/b", false);
    // Escapes and literal wildcards.
    add("what?q", false);
    add("whatxy", false);
    add("star*literal", false);
    add("starXliteral", false);
    add("hash#tag", false);
    add("hash-tag", false);
    add("bang!tag", false);
    add("bang-tag", false);
    add("escaped space.txt", false);
    add("escaped  space.txt", false);
    // Unicode.
    add("données", true);
    add("src/données", true);
    add("données/x.md", false);
    add("résumé.md", false);
    add("docs/résumé.md", false);
    add("日本語", true);
    add("日本語/kanji.txt", false);
    add("文件/a/b.txt", false);
    add("文件/top.txt", false);
    add("misc/emoji-😀.txt", false);
    // Nested gitignore scoping and precedence.
    add("nested/local.txt", false);
    add("local.txt", false);
    add("nested/nested-keep.txt", false);
    add("nested/deep/thing.internal", false);
    add("nested/deep/deep-keep.internal", false);
    add("nested/thing.internal", false);
    // info/exclude.
    add("exclude-only.txt", false);
    add("nested/exclude-only.txt", false);

    let mut fixtures: Vec<(PathBuf, bool)> = Vec::new();
    for (path, is_dir) in &candidates {
        let full = root.join(path);
        // Some literal-escape fixtures (what?q, star*literal) contain
        // characters Windows forbids in filenames; the differential oracle
        // compares pathnames, which need not exist on disk.
        if *is_dir {
            let _ = std::fs::create_dir_all(&full);
        } else {
            write_best_effort(&full, "content\n");
        }
        fixtures.push((PathBuf::from(path), *is_dir));
    }

    let empty_excludes = root.join("empty-excludes");
    std::fs::write(&empty_excludes, "").unwrap();
    DifferentialRepo {
        dir,
        candidates: fixtures,
        empty_excludes,
    }
}

#[test]
fn expanded_differential_matrix_matches_git() {
    let repo = comprehensive_repo();
    let rules = IgnoreRules::collect(repo.dir.path());
    let mut mismatches = Vec::new();
    for (relative, is_dir) in &repo.candidates {
        let Some(git_says) = git_ignores(&repo, relative) else {
            panic!("git check-ignore failed for {}", relative.display());
        };
        let factory_says = rules.is_ignored(relative, *is_dir);
        if git_says != factory_says {
            mismatches.push(format!(
                "{} (dir={is_dir}): git={git_says} factory={factory_says}",
                relative.display()
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {} decisions diverged from git:\n{}",
        mismatches.len(),
        repo.candidates.len(),
        mismatches.join("\n")
    );
}

#[test]
fn the_original_f02_scenario_reproduces_and_is_fixed() {
    // *.env + !dir/allowed.env: only dir/allowed.env is re-included. The
    // pre-F02 parser suffix-matched the negation onto deeper paths, which
    // would re-expose ignored credential-bearing files.
    let repo = comprehensive_repo();
    let rules = IgnoreRules::collect(repo.dir.path());
    assert!(
        !rules.is_ignored(std::path::Path::new("dir/allowed.env"), false),
        "the negated exact path is re-included"
    );
    assert!(
        rules.is_ignored(std::path::Path::new("nested/dir/allowed.env"), false),
        "deeper copies of the same basename stay ignored"
    );
    assert!(
        rules.is_ignored(std::path::Path::new("deep/nested/dir/allowed.env"), false),
        "arbitrarily deep copies stay ignored"
    );
}

#[test]
fn a_ignored_credential_file_cannot_return_through_any_context_path() {
    // The F02 scenario made practical: a nested .env carrying fake
    // credentials must not resurface through indexing, search (both tool
    // paths), resolution, or rendered context.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    assert!(run_git(root, &["init", "-q", "-b", "main"]));
    write(&root.join(".gitignore"), "*.env\n!dir/allowed.env\n");
    write(
        &root.join("dir").join("allowed.env"),
        "FAKE_TOKEN=nested-credential-f02\n",
    );
    write(
        &root.join("nested").join("dir").join("allowed.env"),
        "FAKE_TOKEN=leaked-credential-f02\n",
    );
    write(
        &root.join("src").join("plain.rs"),
        "// FAKE_TOKEN mention\n",
    );

    let factory_dir = root.join(".factory");
    std::fs::create_dir_all(&factory_dir).unwrap();
    let mut engine = factory_context::ContextEngine::new(
        root,
        &factory_dir,
        factory_context::ContextConfig::default(),
    );

    // Index + content search (whichever tool path this machine uses).
    engine.ensure_index().unwrap();
    for hit in engine.search("credential-f02", 20).unwrap() {
        assert!(
            hit.path.replace('\\', "/") != "nested/dir/allowed.env",
            "ignored nested credential leaked through search: {:?}",
            hit.path
        );
    }
    // The re-included file IS visible; the deeper ignored copy is not.
    let rules = IgnoreRules::collect(root);
    let visible = factory_context::search::search_content(root, &rules, "credential-f02", 20)
        .unwrap()
        .into_iter()
        .map(|hit| hit.path)
        .collect::<Vec<_>>();
    assert!(
        visible.contains(&"dir/allowed.env".to_string()),
        "{visible:?}"
    );
    assert!(
        !visible.contains(&"nested/dir/allowed.env".to_string()),
        "{visible:?}"
    );

    // Resolution: rank candidates never include the ignored path.
    let resolved = engine.resolve(&default_request(root)).unwrap();
    for file in &resolved.selected {
        assert!(
            !file.path.replace('\\', "/").starts_with("nested/dir/"),
            "ignored path selected into context: {:?}",
            file.path
        );
    }
    for test in &resolved.related_tests {
        assert!(
            !test.path.replace('\\', "/").starts_with("nested/dir/"),
            "ignored path selected as a related test: {:?}",
            test.path
        );
    }
    let rendered = factory_context::render_repository_context(&resolved);
    assert!(
        !rendered.contains("leaked-credential-f02"),
        "ignored credential leaked into rendered context"
    );
}

#[test]
fn a_live_engine_picks_up_ignore_rule_changes_on_refresh() {
    // ASF-F02 lifecycle: a long-lived engine must not keep applying ignore
    // rules captured at construction time.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    assert!(run_git(root, &["init", "-q", "-b", "main"]));
    write(&root.join("keep.rs"), "pub fn keep() {}\n");
    write(&root.join("dropme.rs"), "pub fn dropme() {}\n");
    let factory_dir = root.join(".factory");
    std::fs::create_dir_all(&factory_dir).unwrap();

    let mut engine = factory_context::ContextEngine::new(
        root,
        &factory_dir,
        factory_context::ContextConfig::default(),
    );
    engine.ensure_index().unwrap();
    assert!(!engine.search("dropme", 10).unwrap().is_empty());

    write(&root.join(".gitignore"), "dropme.rs\n");
    engine.refresh().unwrap();
    let hits = engine.search("dropme", 10).unwrap();
    assert!(
        hits.iter()
            .all(|hit| hit.path.replace('\\', "/") != "dropme.rs"),
        "same-instance refresh must apply new ignore rules: {hits:?}"
    );

    std::fs::remove_file(root.join(".gitignore")).unwrap();
    engine.refresh().unwrap();
    assert!(
        !engine.search("dropme", 10).unwrap().is_empty(),
        "removing the rule re-includes the file on the same instance"
    );
}

/// A minimal resolution request for the exposure test.
fn default_request(root: &Path) -> factory_context::ContextRequest {
    factory_context::ContextRequest {
        scope_dir: root.to_path_buf(),
        root_dir: root.to_path_buf(),
        base_sha: None,
        role_id: Some("worker".to_string()),
        operation: Some(factory_types::TaskOperation::Implement),
        title: "Find credentials".to_string(),
        objective: "locate credential handling".to_string(),
        acceptance_criteria: Vec::new(),
        changed_files: Vec::new(),
        upstream_artifact_snippets: Vec::new(),
    }
}
