//! ASF-F01 / ASF-F04 regressions: repository-boundary and symlink safety,
//! bounded reads, and bounded, validated ripgrep execution for content
//! search. Ripgrep behavior is made deterministic with `FACTORY_RG_BIN`
//! fakes; tests that set it are serialized under a process-wide lock because
//! CI runs test binaries with parallel threads.

use std::path::Path;
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
    _guard: MutexGuard<'static, ()>,
}

impl FakeRg {
    fn install(windows: &str, unix: &str) -> FakeRg {
        let guard = rg_env_lock();
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
        std::env::set_var("FACTORY_RG_BIN", &path);
        FakeRg {
            _dir: dir,
            _guard: guard,
        }
    }
}

impl Drop for FakeRg {
    fn drop(&mut self) {
        std::env::remove_var("FACTORY_RG_BIN");
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
        if std::os::windows::fs::symlink_file(original, link).is_ok() {
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
