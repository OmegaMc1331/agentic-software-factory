//! ASF-F01 / ASF-F04 regressions: repository-boundary and symlink safety,
//! bounded reads, and bounded, validated ripgrep execution for content
//! search. Ripgrep behavior is made deterministic with `FACTORY_RG_BIN`
//! fakes; tests that set it are serialized under a process-wide lock because
//! CI runs test binaries with parallel threads.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use factory_context::IgnoreRules;
use tempfile::TempDir;

fn rg_env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let mutex = LOCK.get_or_init(|| Mutex::new(()));
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Installs a fake ripgrep (and restores the real one on drop, even on
/// assertion failure).
struct FakeRg {
    _dir: TempDir,
    previous: Option<std::ffi::OsString>,
}

impl FakeRg {
    fn candidates(paths: &[PathBuf]) -> Self {
        let fake = Self::install(
            "@echo off\r\nif \"%1\"==\"--version\" (echo 14.1.0 & exit /b 0)\r\ntype \"%~dp0payload.bin\"\r\nexit /b 0\r\n",
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 14.1.0; exit 0; }\ncat \"$(dirname \"$0\")/payload.bin\"\nexit 0\n",
        );
        let mut payload = Vec::new();
        for path in paths {
            payload.extend_from_slice(path.as_os_str().as_encoded_bytes());
            payload.push(0);
        }
        std::fs::write(fake._dir.path().join("payload.bin"), payload).unwrap();
        fake
    }

    /// Callers must hold [`rg_env_lock`] for the whole test: the override
    /// is process-wide and the suite runs with parallel test threads on CI.
    fn install(windows: &str, unix: &str) -> FakeRg {
        let dir = TempDir::new().unwrap();
        let path = if cfg!(windows) {
            let script = dir.path().join("fake-rg.cmd");
            std::fs::write(&script, windows).unwrap();
            script
        } else {
            let script = dir.path().join("fake-rg.sh");
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
        let previous = std::env::var_os("FACTORY_RG_BIN");
        std::env::set_var("FACTORY_RG_BIN", &path);
        FakeRg {
            _dir: dir,
            previous,
        }
    }
}

impl Drop for FakeRg {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("FACTORY_RG_BIN", value),
            None => std::env::remove_var("FACTORY_RG_BIN"),
        }
    }
}

/// `platform_symlink` creates a real symlink where the platform allows it
/// without elevated privileges and returns whether it exists.
fn platform_symlink(original: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link).is_ok()
    }
    #[cfg(windows)]
    {
        // Symlink creation on Windows needs developer mode or privileges;
        // junctions (`mklink /J`) never do and exercise the same reparse
        // boundary. Try a symlink first, fall back to a junction for
        // directories.
        let result = if original.is_dir() {
            std::os::windows::fs::symlink_dir(original, link)
        } else {
            std::os::windows::fs::symlink_file(original, link)
        };
        if result.is_ok() {
            return true;
        }
        if original.is_dir() {
            let output = std::process::Command::new("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(link)
                .arg(original)
                .output()
                .unwrap();
            return output.status.success();
        }
        false
    }
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn raw_candidate(root: &Path, suffix: &str) -> PathBuf {
    let mut path = root.as_os_str().to_os_string();
    path.push(std::path::MAIN_SEPARATOR_STR);
    path.push(if cfg!(windows) {
        suffix.replace('/', "\\")
    } else {
        suffix.to_string()
    });
    PathBuf::from(path)
}

fn hits(dir: &TempDir, query: &str) -> Vec<String> {
    factory_context::search::search_content(dir.path(), &IgnoreRules::empty(), query, 20)
        .unwrap()
        .into_iter()
        .map(|hit| hit.path)
        .collect()
}

// --- ASF-F01: symlink and repository-boundary safety -----------------------

#[test]
fn a_symlink_to_an_external_file_exposes_nothing() {
    let _guard = rg_env_lock();
    let dir = TempDir::new().unwrap();
    let private = dir.path().parent().unwrap().join("f01-private-file");
    std::fs::create_dir_all(&private).unwrap();
    let secret = private.join("secret.txt");
    write(&secret, "UNIQUE-EXTERNAL-SECRET-f01a\n");
    write(
        &dir.path().join("src").join("ordinary.txt"),
        "nothing interesting\n",
    );
    let link = dir.path().join("src").join("external.txt");
    if !platform_symlink(&secret, &link) {
        eprintln!("skipped: this platform/privilege cannot create file symlinks");
        return;
    }

    let results = hits(&dir, "UNIQUE-EXTERNAL-SECRET-f01a");
    assert!(
        results.is_empty(),
        "external secret leaked through the file symlink: {results:?}"
    );
    // Ordinary files in the same tree remain searchable.
    assert!(!hits(&dir, "interesting").is_empty());
    let _ = std::fs::remove_dir_all(&private);
}

#[test]
fn b_symlink_to_an_external_directory_is_not_traversed() {
    let _guard = rg_env_lock();
    let dir = TempDir::new().unwrap();
    let private = dir.path().parent().unwrap().join("f01-private-dir");
    std::fs::create_dir_all(&private).unwrap();
    write(&private.join("hidden.rs"), "HIDDEN-EXTERNAL-CONTENT-f01b\n");
    write(&dir.path().join("visible.txt"), "plain content\n");
    let link = dir.path().join("linked");
    if !platform_symlink(&private, &link) {
        eprintln!("skipped: this platform/privilege cannot create directory links");
        return;
    }

    let results = hits(&dir, "HIDDEN-EXTERNAL-CONTENT-f01b");
    assert!(
        results.is_empty(),
        "external directory traversed through the link: {results:?}"
    );
    assert!(!hits(&dir, "plain").is_empty());
    let _ = std::fs::remove_dir_all(&private);
}

#[test]
fn c_symlink_cycles_terminate_within_a_bounded_time() {
    let _guard = rg_env_lock();
    let dir = TempDir::new().unwrap();
    write(&dir.path().join("nested").join("real.txt"), "cycle probe\n");
    let nested = dir.path().join("nested");
    if !platform_symlink(dir.path(), &nested.join("back")) {
        eprintln!("skipped: this platform/privilege cannot create directory links");
        return;
    }
    // The scan must finish on its own; the old is_dir()-based traversal
    // looped forever on this fixture.
    let root = dir.path().to_path_buf();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = factory_context::search::search_content(
            &root,
            &IgnoreRules::empty(),
            "cycle probe",
            20,
        );
        let _ = sender.send(result.map(|hits| hits.len()));
    });
    let count = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("the scan must terminate instead of following the cycle")
        .unwrap();
    assert_eq!(count, 1, "exactly the real file matches");
}

#[test]
fn d_ordinary_files_with_spaces_and_unicode_stay_searchable() {
    let _guard = rg_env_lock();
    let dir = TempDir::new().unwrap();
    write(
        &dir.path().join("src").join("日本語 notes.md"),
        "café UNIQUE-ORDINARY-f01d\n",
    );
    write(
        &dir.path().join("with space.txt"),
        "UNIQUE-ORDINARY-f01d second\n",
    );
    let results = hits(&dir, "UNIQUE-ORDINARY-f01d");
    assert_eq!(results.len(), 2, "both ordinary files match: {results:?}");
    assert!(results.iter().any(|path| path.contains("日本語")));
    assert!(results.iter().any(|path| path.contains("with space")));
}

#[test]
fn e_internal_symlinks_are_not_followed_either() {
    let _guard = rg_env_lock();
    // Policy (matching the index): symlinks are never followed or read,
    // including targets that live inside the repository.
    let dir = TempDir::new().unwrap();
    write(&dir.path().join("real.txt"), "INTERNAL-LINK-TARGET-f01e\n");
    let link = dir.path().join("alias.txt");
    if !platform_symlink(&dir.path().join("real.txt"), &link) {
        eprintln!("skipped: this platform/privilege cannot create file symlinks");
        return;
    }
    // The real file matches; the alias must not add a second hit.
    let results = hits(&dir, "INTERNAL-LINK-TARGET-f01e");
    assert_eq!(
        results,
        vec!["real.txt".to_string()],
        "alias followed: {results:?}"
    );
}

#[test]
fn f_broken_symlinks_are_ignored_without_error() {
    let _guard = rg_env_lock();
    let dir = TempDir::new().unwrap();
    write(&dir.path().join("ok.txt"), "still works\n");
    let broken = dir.path().join("dangling.txt");
    #[cfg(unix)]
    let created = std::os::unix::fs::symlink("/nowhere/at/all", &broken).is_ok();
    #[cfg(windows)]
    let created = std::os::windows::fs::symlink_file("nowhere-at-all", &broken).is_ok();
    if !created {
        eprintln!("skipped: this platform/privilege cannot create file symlinks");
        return;
    }
    assert!(!hits(&dir, "works").is_empty());
}

// --- ASF-F04: bounded reads, bounded ripgrep -------------------------------

#[test]
fn oversized_files_are_rejected_before_any_read_allocates() {
    let _guard = rg_env_lock();
    let dir = TempDir::new().unwrap();
    let limit = factory_context::index::MAX_INDEX_FILE_READ_BYTES as usize;
    // A file one byte past the read limit containing the needle: it must be
    // rejected by metadata, not matched after a full read.
    let big = dir.path().join("big.txt");
    let mut content = vec![b'.'; limit + 1];
    content.extend_from_slice(b"OVERSIZED-NEEDLE-f04\n");
    std::fs::write(&big, &content).unwrap();
    // Exactly at the limit: still searchable.
    let exact_bytes = format!(
        "{}AT-LIMIT-NEEDLE-f04",
        ".".repeat(limit - "AT-LIMIT-NEEDLE-f04".len())
    );
    write(&dir.path().join("exact.txt"), &exact_bytes);

    let results = hits(&dir, "OVERSIZED-NEEDLE-f04");
    assert!(results.is_empty(), "oversized file matched: {results:?}");
    let at_limit = hits(&dir, "AT-LIMIT-NEEDLE-f04");
    assert_eq!(at_limit, vec!["exact.txt".to_string()]);
}

#[test]
fn invalid_utf8_and_binary_files_do_not_break_the_scan() {
    let _guard = rg_env_lock();
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("bin.dat"), vec![0xff, 0xfe, 0x00, 0x01]).unwrap();
    // Invalid UTF-8 that would decode to the needle under lossy conversion.
    let mut bytes = b"prefix ".to_vec();
    bytes.extend_from_slice(&[0xff]);
    bytes.extend_from_slice(b" BINMATCH-f04 suffix\n");
    std::fs::write(dir.path().join("lossy.txt"), bytes).unwrap();
    write(&dir.path().join("text.txt"), "plain BINMATCH-f04\n");
    let results = hits(&dir, "BINMATCH-f04");
    assert_eq!(
        results.len(),
        2,
        "lossy text and plain text both match; binary noise does not crash: {results:?}"
    );
}

#[test]
fn a_hanging_ripgrep_falls_back_to_a_bounded_scan() {
    let _guard = rg_env_lock();
    // Real rg answers `--version` instantly; only the search itself hangs.
    let _fake = FakeRg::install(
        "@echo off\r\nif \"%1\"==\"--version\" (echo 14.1.0 & exit /b 0)\r\nping -n 120 127.0.0.1 >nul\r\n",
        "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 14.1.0; exit 0; }\nsleep 120\n",
    );
    let _timeout = RgTimeoutGuard::set("2");
    let dir = TempDir::new().unwrap();
    write(&dir.path().join("found.txt"), "FALLBACK-AFTER-HANG-f04\n");
    let started = std::time::Instant::now();
    let results = hits(&dir, "FALLBACK-AFTER-HANG-f04");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "a hung rg must not stall search (took {:?})",
        started.elapsed()
    );
    assert_eq!(results, vec!["found.txt".to_string()]);
}

#[test]
fn a_failing_ripgrep_falls_back_but_a_no_match_does_not() {
    let _guard = rg_env_lock();
    // Exit 2 is a real tool failure → fallback runs and finds the file.
    // (Each fake lives in its own block: two serialization guards held at
    // once would self-deadlock the process-wide lock.)
    let dir = TempDir::new().unwrap();
    write(
        &dir.path().join("tool-failure.txt"),
        "MATCHED-ON-FALLBACK-f04\n",
    );
    {
        let _fake = FakeRg::install(
            "@echo off\r\nif \"%1\"==\"--version\" (echo 14.1.0 & exit /b 0)\r\nexit /b 2\r\n",
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 14.1.0; exit 0; }\nexit 2\n",
        );
        let results = hits(&dir, "MATCHED-ON-FALLBACK-f04");
        assert_eq!(results, vec!["tool-failure.txt".to_string()]);
    }

    // Exit 1 is ripgrep's documented "no matches": the answer is empty and
    // the (potentially expensive) fallback must not re-scan the tree.
    write(
        &dir.path().join("second.txt"),
        "MATCHED-ON-FALLBACK-f04 again\n",
    );
    {
        let _fake = FakeRg::install(
            "@echo off\r\nif \"%1\"==\"--version\" (echo 14.1.0 & exit /b 0)\r\nexit /b 1\r\n",
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 14.1.0; exit 0; }\nexit 1\n",
        );
        let results = hits(&dir, "MATCHED-ON-FALLBACK-f04");
        assert!(
            results.is_empty(),
            "a no-match exit is authoritative, not a fallback trigger: {results:?}"
        );
    }
}

#[test]
fn truncated_ripgrep_output_is_rejected_not_parsed() {
    let _guard = rg_env_lock();
    // The fake floods stdout past the capture limit with plausible-looking
    // paths; the truncated output must be discarded and the bounded
    // fallback decide.
    let dir = TempDir::new().unwrap();
    write(&dir.path().join("real.txt"), "TRUNCATION-FALLBACK-f04\n");
    let _fake = FakeRg::install(
        "@echo off\r\nif \"%1\"==\"--version\" (echo 14.1.0 & exit /b 0)\r\nfor /l %%i in (1,1,30000) do echo C:\\nowhere\\path-number-%%i-0123456789012345678901234567890123456789\r\n",
        "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 14.1.0; exit 0; }\ni=0\nwhile [ $i -lt 30000 ]; do printf '/nowhere/path-number-%d-0123456789012345678901234567890123456789\\0' $i; i=$((i+1)); done\n",
    );
    let results = hits(&dir, "TRUNCATION-FALLBACK-f04");
    assert_eq!(results, vec!["real.txt".to_string()]);
}

#[test]
fn ripgrep_paths_outside_the_root_or_through_links_are_rejected() {
    let _guard = rg_env_lock();
    // The fake rg "finds" the query in paths that must never be accepted:
    // one outside the repository, one behind a symlink, and one legit path.
    // The NUL-separated payload is prebuilt as a binary file; both fake
    // scripts just copy it to stdout byte-for-byte.
    let dir = TempDir::new().unwrap();
    write(&dir.path().join("legit.txt"), "NEEDLE-REVALIDATE-f04\n");
    let outside = dir.path().parent().unwrap().join("outside-revalidate.txt");
    write(&outside, "NEEDLE-REVALIDATE-f04\n");
    let link_target = dir.path().parent().unwrap().join("link-target.txt");
    write(&link_target, "NEEDLE-REVALIDATE-f04\n");
    let linked = dir.path().join("linked.txt");
    let have_link = platform_symlink(&link_target, &linked);

    // The supervisor receives the canonicalized root, so candidate paths
    // come back under exactly that prefix; build the payload accordingly.
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let legit = root.join("legit.txt");
    let mut payload = outside.display().to_string().into_bytes();
    payload.push(0);
    payload.extend_from_slice(linked.display().to_string().as_bytes());
    payload.push(0);
    payload.extend_from_slice(legit.display().to_string().as_bytes());
    payload.push(0);
    let payload_path = dir.path().join("rg-payload.bin");
    std::fs::write(&payload_path, &payload).unwrap();

    let _fake = if cfg!(windows) {
        FakeRg::install(
            &format!(
                "@echo off\r\nif \"%1\"==\"--version\" (echo 14.1.0 & exit /b 0)\r\ntype \"{}\"\r\nexit /b 0\r\n",
                payload_path.display()
            ),
            "",
        )
    } else {
        FakeRg::install(
            "",
            &format!(
                "#!/bin/sh\n[ \"$1\" = \"--version\" ] && {{ echo 14.1.0; exit 0; }}\ncat '{}'\nexit 0\n",
                payload_path.display()
            ),
        )
    };

    let results = hits(&dir, "NEEDLE-REVALIDATE-f04");
    assert_eq!(
        results,
        vec!["legit.txt".to_string()],
        "outside and symlinked candidates must be rejected{}: {results:?}",
        if have_link { " (symlink existed)" } else { "" },
    );
    let _ = std::fs::remove_file(&outside);
    let _ = std::fs::remove_file(&link_target);
}

#[test]
fn v01_parent_traversal_cannot_expose_private_content() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let repository = fixture.path().join("repository");
    write(&repository.join("src/main.rs"), "pub fn main() {}\n");
    write(
        &fixture.path().join("private/secret.txt"),
        "V01_SYNTHETIC_PRIVATE_VALUE\n",
    );
    let root = repository.canonicalize().unwrap();
    let _fake = FakeRg::candidates(&[raw_candidate(&root, "../private/secret.txt")]);
    assert_private_content_absent(&root, "V01_SYNTHETIC_PRIVATE_VALUE");
}

#[test]
fn v01_intermediate_links_cannot_expose_content() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let repository = fixture.path().join("repository");
    write(&repository.join("src/main.rs"), "pub fn main() {}\n");
    write(
        &fixture.path().join("private/secret.txt"),
        "V01_SYNTHETIC_LINK_VALUE\n",
    );
    let root = repository.canonicalize().unwrap();
    assert!(platform_symlink(
        &fixture.path().join("private"),
        &root.join("linked")
    ));
    let _fake = FakeRg::candidates(&[root.join("linked/secret.txt")]);
    assert_private_content_absent(&root, "V01_SYNTHETIC_LINK_VALUE");
}

fn assert_private_content_absent(root: &Path, secret: &str) {
    let rules = IgnoreRules::collect(root);
    let hits = factory_context::search::search_content(root, &rules, secret, 20).unwrap();
    assert!(hits.is_empty(), "private path or snippet exposed: {hits:?}");
    let mut engine = factory_context::ContextEngine::new(
        root,
        &root.join(".factory"),
        factory_context::ContextConfig::default(),
    );
    assert!(engine.search(secret, 20).unwrap().is_empty());
    let context = engine
        .resolve(&factory_context::ContextRequest {
            scope_dir: root.to_path_buf(),
            root_dir: root.to_path_buf(),
            title: secret.to_string(),
            changed_files: vec!["../private/secret.txt".into(), "linked/secret.txt".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(context
        .selected
        .iter()
        .all(|file| { !file.path.contains("secret.txt") && !file.excerpt.contains(secret) }));
    assert!(!factory_context::render_repository_context(&context).contains(secret));
}

#[test]
fn v03_candidate_permutations_select_the_same_ten_of_one_hundred() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let paths: Vec<_> = (1..=100)
        .map(|i| {
            let path = root.join(format!("src/file{i:03}.rs"));
            write(&path, "V03_MATCH\n");
            path
        })
        .collect();
    let expected: Vec<_> = (1..=10).map(|i| format!("src/file{i:03}.rs")).collect();
    let reversed: Vec<_> = paths.iter().rev().cloned().collect();
    let shuffled: Vec<_> = (0..100).map(|i| paths[(i * 37) % 100].clone()).collect();
    for candidates in [&paths, &reversed, &shuffled] {
        let _fake = FakeRg::candidates(candidates);
        let hits =
            factory_context::search::search_content(&root, &IgnoreRules::empty(), "V03_MATCH", 10)
                .unwrap();
        assert_eq!(
            hits.into_iter().map(|hit| hit.path).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn v01_adversarial_candidates_share_the_full_context_ignore_and_boundary_policy() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let repository = fixture.path().join("repository");
    let secret = "BOUNDARY_NEEDLE SYNTHETIC_PRIVATE_V01";
    write(
        &repository.join("src/日本語 notes.rs"),
        "BOUNDARY_NEEDLE public\n",
    );
    write(&repository.join(".gitignore"), "*.env\n");
    write(&repository.join("src/.gitignore"), "hidden.txt\n");
    write(&repository.join("src/hidden.txt"), secret);
    write(&repository.join("credentials.env"), secret);
    write(&fixture.path().join("private/secret.txt"), secret);
    write(&fixture.path().join("repository-extra/secret.txt"), secret);
    write(
        &repository.join("real/internal.rs"),
        "BOUNDARY_NEEDLE public internal\n",
    );
    let root = repository.canonicalize().unwrap();
    let big = format!(
        "{secret}{}",
        ".".repeat(factory_context::index::MAX_INDEX_FILE_READ_BYTES as usize)
    );
    write(&root.join("oversized.rs"), &big);
    write(&root.join("oversized.env"), &big);
    assert!(platform_symlink(
        &root.join("real"),
        &root.join("internal-link")
    ));
    assert!(platform_symlink(
        &fixture.path().join("private"),
        &root.join("outside-link")
    ));
    assert!(platform_symlink(&root, &root.join("real/cycle")));
    let mut candidates = vec![
        root.join("src/日本語 notes.rs"),
        root.join("real/internal.rs"),
        fixture.path().join("private/secret.txt"),
        fixture.path().join("repository-extra/secret.txt"),
        raw_candidate(&root, "../private/secret.txt"),
        raw_candidate(&root, "src/../../private/secret.txt"),
        raw_candidate(&root, "./src/日本語 notes.rs"),
        raw_candidate(&root, "src/./日本語 notes.rs"),
        PathBuf::from("src/日本語 notes.rs"),
        root.join("src"),
        root.clone(),
        root.join("missing.txt"),
        root.join("internal-link/internal.rs"),
        root.join("outside-link/secret.txt"),
        root.join("real/cycle/src/日本語 notes.rs"),
        root.join("src/hidden.txt"),
        root.join("credentials.env"),
        root.join("oversized.rs"),
        root.join("oversized.env"),
    ];
    let leaf = root.join("leaf.txt");
    if platform_symlink(&fixture.path().join("private/secret.txt"), &leaf) {
        candidates.push(leaf);
    }
    for fallback in [false, true] {
        let _fake = if fallback {
            FakeRg::install("@exit /b 2\r\n", "#!/bin/sh\nexit 2\n")
        } else {
            FakeRg::candidates(&candidates)
        };
        let rules = IgnoreRules::collect(&root);
        let hits =
            factory_context::search::search_content(&root, &rules, "BOUNDARY_NEEDLE", 100).unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.path.as_str()).collect::<Vec<_>>(),
            ["real/internal.rs", "src/日本語 notes.rs"]
        );
        assert!(hits.iter().all(|hit| !hit
            .snippet
            .as_deref()
            .unwrap_or_default()
            .contains("SYNTHETIC_PRIVATE")));
        let mut engine = factory_context::ContextEngine::new(
            &root,
            &root.join(".factory"),
            factory_context::ContextConfig::default(),
        );
        let mut index = factory_context::ContextIndex {
            root: root.clone(),
            files: Default::default(),
            oversize: false,
        };
        index.refresh(&root, &rules).unwrap();
        for path in index.files.keys() {
            assert!(
                !path.contains("link")
                    && !path.contains("cycle")
                    && !path.ends_with(".env")
                    && !path.ends_with("hidden.txt")
            );
        }
        let context = engine
            .resolve(&factory_context::ContextRequest {
                scope_dir: root.clone(),
                root_dir: root.clone(),
                title: "BOUNDARY_NEEDLE".into(),
                changed_files: candidates
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
                ..Default::default()
            })
            .unwrap();
        let rendered = factory_context::render_repository_context(&context);
        assert!(!rendered.contains("SYNTHETIC_PRIVATE"));
    }
}

#[test]
fn v03_more_than_one_thousand_candidates_are_deduplicated_before_selection() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let maximum = factory_context::search::MAX_MATCHED_PATHS;
    let paths: Vec<_> = (0..maximum + 51)
        .rev()
        .map(|i| {
            let path = root.join(format!("src/file{i:04}.rs"));
            write(&path, "DETERMINISTIC_NEEDLE\n");
            path
        })
        .collect();
    let mut candidates = paths.clone();
    candidates.extend(paths.iter().cloned());
    let expected: Vec<_> = (0..maximum).map(|i| format!("src/file{i:04}.rs")).collect();
    for fallback in [false, true] {
        let _fake = if fallback {
            FakeRg::install("@exit /b 2\r\n", "#!/bin/sh\nexit 2\n")
        } else {
            FakeRg::candidates(&candidates)
        };
        let hits = factory_context::search::search_content(
            &root,
            &IgnoreRules::empty(),
            "DETERMINISTIC_NEEDLE",
            usize::MAX,
        )
        .unwrap();
        assert_eq!(
            hits.into_iter().map(|hit| hit.path).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn v03_ripgrep_and_fallback_agree_on_unicode_case_spaces_and_nested_paths() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let names = [
        "Alpha.rs",
        "beta.rs",
        "a space.rs",
        "nested/Zebra.rs",
        "nested/apple.rs",
        "日本語/éclair.rs",
    ];
    let mut candidates: Vec<_> = names
        .iter()
        .rev()
        .map(|name| {
            let path = root.join(name);
            write(&path, "ORDER_NEEDLE\n");
            path
        })
        .collect();
    candidates.push(raw_candidate(&root, "nested//apple.rs"));
    candidates.push(root.join("Alpha.rs"));
    write(&root.join("ignored.rs"), "ORDER_NEEDLE\n");
    candidates.push(root.join("ignored.rs"));
    assert!(platform_symlink(&root.join("nested"), &root.join("alias")));
    candidates.push(root.join("alias/apple.rs"));
    let ignore = IgnoreRules::from_gitignore("ignored.rs\n");
    let mut expected = names.to_vec();
    expected.sort();
    expected.truncate(4);
    for fallback in [false, true] {
        let _fake = if fallback {
            FakeRg::install("@exit /b 2\r\n", "#!/bin/sh\nexit 2\n")
        } else {
            FakeRg::candidates(&candidates)
        };
        for _ in 0..2 {
            let hits =
                factory_context::search::search_content(&root, &ignore, "ORDER_NEEDLE", 4).unwrap();
            assert_eq!(
                hits.into_iter().map(|hit| hit.path).collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[test]
fn v01_unterminated_ripgrep_output_falls_back_instead_of_parsing_a_partial_path() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    write(&root.join("real.rs"), "UNTERMINATED_NEEDLE\n");
    let fake = FakeRg::candidates(&[root.join("missing.rs")]);
    std::fs::write(
        fake._dir.path().join("payload.bin"),
        root.join("missing.rs").as_os_str().as_encoded_bytes(),
    )
    .unwrap();
    let hits = factory_context::search::search_content(
        &root,
        &IgnoreRules::empty(),
        "UNTERMINATED_NEEDLE",
        10,
    )
    .unwrap();
    assert_eq!(
        hits.into_iter().map(|hit| hit.path).collect::<Vec<_>>(),
        ["real.rs"]
    );
}

#[cfg(unix)]
#[test]
fn v01_unix_newlines_and_backslashes_are_lossless_and_non_utf8_paths_fail_closed() {
    use std::os::unix::ffi::OsStringExt;
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let names = ["line\nbreak.rs", "literal\\backslash.rs"];
    let mut candidates: Vec<_> = names
        .iter()
        .map(|name| {
            let path = root.join(name);
            write(&path, "UNIX_NEEDLE\n");
            path
        })
        .collect();
    let non_utf8 = root.join(std::ffi::OsString::from_vec(b"invalid-\xff.rs".to_vec()));
    match std::fs::write(&non_utf8, "UNIX_NEEDLE SYNTHETIC_PRIVATE_NON_UTF8\n") {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == Some(libc::EILSEQ) => {
            eprintln!("filesystem rejects non-UTF-8 names; the raw candidate is still tested");
        }
        Err(error) => panic!("cannot create non-UTF-8 fixture: {error}"),
    }
    write(
        &root.join("invalid-\u{fffd}.rs"),
        "SYNTHETIC_PRIVATE_NON_UTF8_ALIAS\n",
    );
    candidates.push(non_utf8);
    for fallback in [false, true] {
        let _fake = if fallback {
            FakeRg::install("", "#!/bin/sh\nexit 2\n")
        } else {
            FakeRg::candidates(&candidates)
        };
        let hits = factory_context::search::search_content(
            &root,
            &IgnoreRules::empty(),
            "UNIX_NEEDLE",
            10,
        )
        .unwrap();
        assert_eq!(
            hits.into_iter().map(|hit| hit.path).collect::<Vec<_>>(),
            names
        );
    }
}

#[cfg(windows)]
#[test]
fn v01_windows_junction_reparse_attribute_is_rejected_at_every_depth() {
    use std::os::windows::fs::MetadataExt;
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let repository = fixture.path().join("repository");
    write(&repository.join("src/main.rs"), "ordinary\n");
    let private = fixture.path().join("private");
    write(
        &private.join("secret.txt"),
        "V01_JUNCTION_SYNTHETIC_SECRET\n",
    );
    let junction = repository.join("junction");
    let output = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&private)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(
        std::fs::symlink_metadata(&junction)
            .unwrap()
            .file_attributes()
            & 0x400,
        0
    );
    let root = repository.canonicalize().unwrap();
    let _fake = FakeRg::candidates(&[root.join("junction/secret.txt")]);
    assert_private_content_absent(&root, "V01_JUNCTION_SYNTHETIC_SECRET");
}

#[cfg(windows)]
#[test]
fn v01_windows_case_aliases_cannot_bypass_ignores_or_duplicate_hits() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    write(&root.join("ordinary.rs"), "CASE_NEEDLE public\n");
    write(
        &root.join("credentials.env"),
        "CASE_NEEDLE SYNTHETIC_PRIVATE_CASE\n",
    );
    let _fake = FakeRg::candidates(&[
        root.join("ORDINARY.RS"),
        root.join("ordinary.rs"),
        root.join("CREDENTIALS.ENV"),
    ]);
    let rules = IgnoreRules::from_gitignore("*.env\n");
    let hits = factory_context::search::search_content(&root, &rules, "CASE_NEEDLE", 10).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].path, "ordinary.rs");
    assert!(!hits[0]
        .snippet
        .as_deref()
        .unwrap()
        .contains("SYNTHETIC_PRIVATE"));
}

#[test]
fn v01_a_cached_directory_replaced_by_a_link_is_rejected_before_search_and_resolution() {
    let _guard = rg_env_lock();
    let fixture = TempDir::new().unwrap();
    let repository = fixture.path().join("repository");
    write(
        &repository.join("src/auth.rs"),
        "pub fn cached_authenticate() {}\n",
    );
    let root = repository.canonicalize().unwrap();
    let mut engine = factory_context::ContextEngine::new(
        &root,
        &root.join(".factory"),
        factory_context::ContextConfig::default(),
    );
    engine.ensure_index().unwrap();
    let private = fixture.path().join("private");
    std::fs::rename(root.join("src"), &private).unwrap();
    write(
        &private.join("auth.rs"),
        "pub fn cached_authenticate() {} // SYNTHETIC_PRIVATE_REPLACED\n",
    );
    assert!(platform_symlink(&private, &root.join("src")));
    let _fake = FakeRg::candidates(&[root.join("src/auth.rs")]);
    assert!(engine.search("cached_authenticate", 20).unwrap().is_empty());
    let context = engine
        .resolve(&factory_context::ContextRequest {
            scope_dir: root.clone(),
            root_dir: root.clone(),
            title: "cached_authenticate".into(),
            changed_files: vec!["src/auth.rs".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(context.selected.is_empty());
    assert!(!factory_context::render_repository_context(&context)
        .contains("SYNTHETIC_PRIVATE_REPLACED"));
}

/// Bounds the rg deadline while alive (keeps hanging fakes fast); restores
/// the production deadline on drop.
struct RgTimeoutGuard;

impl RgTimeoutGuard {
    fn set(secs: &str) -> Self {
        std::env::set_var("FACTORY_RG_TIMEOUT_SECS", secs);
        RgTimeoutGuard
    }
}

impl Drop for RgTimeoutGuard {
    fn drop(&mut self) {
        std::env::remove_var("FACTORY_RG_TIMEOUT_SECS");
    }
}
