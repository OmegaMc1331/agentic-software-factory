use super::*;
use crate::{types::nonce, SearchRequest};

struct Fixture {
    directory: tempfile::TempDir,
    service: Arc<FileService>,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join(".factory")).unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(directory.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        let service = Arc::new(
            FileService::new(
                &format!("p_{}", "a".repeat(32)),
                directory.path(),
                Limits::default(),
            )
            .unwrap(),
        );
        Self { directory, service }
    }
    fn reference(&self, path: &str) -> FileRef {
        self.service.reference(&self.service.primary, path)
    }
    fn disk(&self, path: &str) -> Vec<u8> {
        fs::read(self.directory.path().join(path)).unwrap()
    }
    fn git(&self, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(self.directory.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn initialize_git(&self) {
        self.git(&["init", "-q", "-b", "main"]);
        self.git(&["config", "user.email", "p05@example.test"]);
        self.git(&["config", "user.name", "P05 fixture"]);
        self.git(&["add", "src/main.rs"]);
        self.git(&["commit", "-qm", "fixture"]);
    }
}

#[test]
fn fs_sec_01_02_06_07_08_paths_are_strict_and_internals_are_protected() {
    let fixture = Fixture::new();
    for path in [
        "../outside",
        "src/../main.rs",
        "/absolute",
        "C:/absolute",
        "a\\b",
        "./src/main.rs",
        "src//main.rs",
        "src/main.rs:stream",
        "src/.git/config",
        ".git/config",
        ".FACTORY/db.sqlite3",
        "a\0b",
        "CON",
        "NUL.txt",
        "COM1.rs",
        "trailing.",
        "trailing ",
    ] {
        assert!(
            fixture.service.read(&fixture.reference(path)).is_err(),
            "{path:?}"
        );
        assert!(
            fixture
                .service
                .create(&fixture.reference(path), false)
                .is_err(),
            "{path:?}"
        );
    }
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}

#[cfg(unix)]
#[test]
fn nested_file_mutations_sync_the_held_directory() {
    let fixture = Fixture::new();
    let reference = fixture.reference("src/new.txt");
    fixture.service.create(&reference, false).unwrap();
    let original = fixture.service.read(&reference).unwrap();
    let saved = fixture
        .service
        .save(&reference, &original.revision, "durable nested content")
        .unwrap();
    assert_eq!(
        saved.durability.as_deref(),
        Some("file_and_directory_synced")
    );
    assert_eq!(fixture.disk("src/new.txt"), b"durable nested content");
}

#[test]
fn edit_02_03_save_preserves_utf8_crlf_bom_and_no_final_newline() {
    let fixture = Fixture::new();
    for (name, bytes, content) in [
        (
            "unicode-été.rs",
            b"\xef\xbb\xbf// UTF8\r\nfn main() {}".as_slice(),
            "// été\r\nfn main() { }",
        ),
        ("lf.py", b"x = 1\n".as_slice(), "x = 'é'\n"),
        ("none.txt", b"old".as_slice(), "new"),
    ] {
        fs::write(fixture.directory.path().join(name), bytes).unwrap();
        let reference = fixture.reference(name);
        let initial = fixture.service.read(&reference).unwrap();
        let saved = fixture
            .service
            .save(&reference, &initial.revision, content)
            .unwrap();
        let mut expected = if initial.bom {
            vec![0xef, 0xbb, 0xbf]
        } else {
            vec![]
        };
        expected.extend_from_slice(content.as_bytes());
        assert_eq!(fixture.disk(name), expected);
        assert_ne!(saved.revision, initial.revision);
        assert!(saved.durability.is_some());
        assert_eq!(saved.content.as_deref(), Some(content));
    }
}

#[test]
fn edit_10_11_two_saves_with_same_revision_have_one_winner() {
    let fixture = Fixture::new();
    let reference = fixture.reference("src/main.rs");
    let revision = fixture.service.read(&reference).unwrap().revision;
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let threads = (0..2)
        .map(|i| {
            let service = fixture.service.clone();
            let reference = reference.clone();
            let revision = revision.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service.save(&reference, &revision, &format!("writer {i}"))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results
        .iter()
        .any(|r| matches!(r, Err(FileError::Conflict | FileError::Busy))));
}

#[test]
fn fs_sec_14_edit_12_external_replacement_before_commit_is_a_conflict() {
    let fixture = Fixture::new();
    let reference = fixture.reference("src/main.rs");
    let initial = fixture.service.read(&reference).unwrap();
    let replacement = fixture.directory.path().join("agent-change");
    fs::write(&replacement, "agent version").unwrap();
    let result = fixture
        .service
        .save_hook(&reference, &initial.revision, "human version", || {
            #[cfg(windows)]
            {
                fs::remove_file(fixture.directory.path().join("src/main.rs"))?;
                fs::rename(&replacement, fixture.directory.path().join("src/main.rs"))?;
            }
            #[cfg(not(windows))]
            fs::rename(&replacement, fixture.directory.path().join("src/main.rs"))?;
            Ok(())
        });
    assert!(matches!(result, Err(FileError::Conflict)));
    assert_eq!(fixture.disk("src/main.rs"), b"agent version");
}

#[test]
fn fs_sec_15_hardlinks_cannot_alias_external_data() {
    let fixture = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("private");
    fs::write(&secret, "private outside").unwrap();
    fs::hard_link(&secret, fixture.directory.path().join("alias.rs")).unwrap();
    let reference = fixture.reference("alias.rs");
    assert!(matches!(
        fixture.service.read(&reference),
        Err(FileError::Forbidden)
    ));
    assert!(fixture
        .service
        .save(&reference, "v_fake", "overwrite")
        .is_err());
    assert_eq!(fs::read(secret).unwrap(), b"private outside");
}

#[test]
fn batched_source_guards_preserve_missing_path_and_leaf_checks() {
    let fixture = Fixture::new();
    let id = fixture.service.primary_root().id;
    let outside = tempfile::tempdir().unwrap();
    let external = outside.path().join("file.txt");
    fs::write(&external, "outside the project").unwrap();
    fs::hard_link(&external, fixture.directory.path().join("src/alias.txt")).unwrap();
    let check = |paths: &[&str]| {
        fixture.service.guard_source_paths(
            &id,
            &paths
                .iter()
                .map(|path| path.to_string())
                .collect::<Vec<_>>(),
        )
    };
    assert!(check(&["src/main.rs", "src/deleted.rs", "absent/deleted.rs"]).is_ok());
    assert!(matches!(
        check(&["src/main.rs", "src/alias.txt"]),
        Err(FileError::Forbidden)
    ));
    assert!(matches!(
        check(&["src/main.rs", ".factory/db.sqlite3"]),
        Err(FileError::Forbidden)
    ));
}

#[test]
fn fs_sec_14_atomic_replacement_after_last_check_is_rolled_back() {
    let fixture = Fixture::new();
    let reference = fixture.reference("src/main.rs");
    let revision = fixture.service.read(&reference).unwrap().revision;
    let result = fixture.service.save_stages(
        &reference,
        &revision,
        "human version",
        || Ok(()),
        || {
            let replacement = fixture.directory.path().join("agent-change");
            fs::write(&replacement, "agent won the race")?;
            #[cfg(windows)]
            fs::remove_file(fixture.directory.path().join("src/main.rs"))?;
            fs::rename(replacement, fixture.directory.path().join("src/main.rs"))?;
            Ok(())
        },
    );
    assert!(matches!(result, Err(FileError::Conflict)), "{result:?}");
    assert_eq!(fixture.disk("src/main.rs"), b"agent won the race");
    assert!(fixture.service.recovery().unwrap().is_empty());
}

#[test]
fn edit_26_read_only_save_preserves_original() {
    let fixture = Fixture::new();
    let reference = fixture.reference("src/main.rs");
    let revision = fixture.service.read(&reference).unwrap().revision;
    let original_permissions = fs::metadata(fixture.directory.path().join("src/main.rs"))
        .unwrap()
        .permissions();
    let mut permissions = fs::metadata(fixture.directory.path().join("src/main.rs"))
        .unwrap()
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(fixture.directory.path().join("src/main.rs"), permissions).unwrap();
    assert_ne!(fixture.service.read(&reference).unwrap().revision, revision);
    assert!(fixture.service.save(&reference, &revision, "lost").is_err());
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
    #[cfg(windows)]
    fs::set_permissions(
        fixture.directory.path().join("src/main.rs"),
        original_permissions,
    )
    .unwrap();
    #[cfg(not(windows))]
    let _ = original_permissions;
}

#[test]
fn moving_a_directory_cannot_hide_protected_internals() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.directory.path().join("nested/.git")).unwrap();
    fs::write(
        fixture.directory.path().join("nested/.git/config"),
        "preserved",
    )
    .unwrap();
    let reference = fixture.reference("nested");
    let revision = fixture
        .service
        .entry_revision(&fixture.service.primary, &reference)
        .unwrap()
        .1;
    assert!(matches!(
        fixture.service.move_entry(&reference, &revision, "renamed"),
        Err(FileError::Forbidden)
    ));
    assert_eq!(fixture.disk("nested/.git/config"), b"preserved");
}

#[test]
fn fs_sec_18_search_results_and_scan_are_bounded() {
    let fixture = Fixture::new();
    for i in 0..140 {
        fs::write(
            fixture.directory.path().join(format!("file-{i}.txt")),
            "needle\nneedle\n",
        )
        .unwrap();
    }
    let request = SearchRequest {
        id: format!("q_{}", nonce()),
        root_id: fixture.service.primary_root().id,
        query: "needle".into(),
        content: true,
        case_sensitive: true,
        regex: false,
        include_hidden: false,
        path_filter: String::new(),
    };
    let cancel = fixture.service.begin_search(&request.id).unwrap();
    let result = fixture.service.search(&request, &cancel).unwrap();
    assert_eq!(result.hits.len(), 100);
    assert!(!result.complete);
    assert!(result.bytes_read <= 16 * 1024 * 1024);
}

#[test]
fn draft_quotas_evict_only_clean_records_and_never_drop_dirty_work() {
    let fixture = Fixture::new();
    let limits = Limits {
        draft_records: 2,
        ..Limits::default()
    };
    let service = FileService::new(
        &fixture.service.project_id,
        fixture.directory.path(),
        limits,
    )
    .unwrap();
    let draft = |id: &str, dirty: bool| Draft {
        schema: 1,
        id: format!("d_{}", id.repeat(32)),
        file: fixture.reference("src/main.rs"),
        base_revision: "v_base".into(),
        base_content: "base".into(),
        content: "draft".into(),
        newline: "lf".into(),
        bom: false,
        dirty,
    };
    service.save_draft(draft("a", true), 0).unwrap();
    service.save_draft(draft("b", false), 0).unwrap();
    service.save_draft(draft("c", true), 0).unwrap();
    assert!(service.draft(&format!("d_{}", "b".repeat(32))).is_err());
    assert!(matches!(
        service.save_draft(draft("d", true), 0),
        Err(FileError::Limit)
    ));
    assert_eq!(
        service
            .draft(&format!("d_{}", "a".repeat(32)))
            .unwrap()
            .document
            .content,
        "draft"
    );
}

#[test]
fn fs_sec_16_17_edit_21_22_binary_encoding_and_large_files_are_not_editable() {
    let fixture = Fixture::new();
    for (name, bytes) in [
        ("binary.bin", vec![0, 0xff, 3]),
        ("latin.bin", vec![0xff, 0xfe, 65, 66]),
        ("large.rs", vec![b'x'; 1_048_577]),
    ] {
        fs::write(fixture.directory.path().join(name), &bytes).unwrap();
        let snapshot = fixture.service.read(&fixture.reference(name)).unwrap();
        assert!(snapshot.read_only);
        assert!(snapshot
            .content
            .as_ref()
            .is_none_or(|text| text.len() <= 65536));
        assert!(fixture
            .service
            .save(&snapshot.file, &snapshot.revision, "bad")
            .is_err());
        assert_eq!(fixture.disk(name), bytes);
    }
    let reference = fixture.reference("src/main.rs");
    let snapshot = fixture.service.read(&reference).unwrap();
    assert!(matches!(
        fixture
            .service
            .save(&reference, &snapshot.revision, &"x".repeat(1_048_577)),
        Err(FileError::Limit)
    ));
}

#[test]
fn edit_13_14_15_16_create_move_delete_and_restore_are_real() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(&fixture.reference("new"), true)
        .unwrap();
    fixture
        .service
        .create(&fixture.reference("new/file.rs"), false)
        .unwrap();
    assert!(matches!(
        fixture
            .service
            .create(&fixture.reference("new/file.rs"), false),
        Err(FileError::Exists)
    ));
    let initial = fixture
        .service
        .read(&fixture.reference("new/file.rs"))
        .unwrap();
    let moved = fixture
        .service
        .move_entry(&initial.file, &initial.revision, "src/moved.rs")
        .unwrap();
    assert!(!fixture.directory.path().join("new/file.rs").exists());
    assert_eq!(moved.to.path, "src/moved.rs");
    let current = fixture.service.read(&moved.to).unwrap();
    assert!(fixture
        .service
        .delete(&current.file, &current.revision, false)
        .is_err());
    let deleted = fixture
        .service
        .delete(&current.file, &current.revision, true)
        .unwrap();
    assert!(!fixture.directory.path().join("src/moved.rs").exists());
    fixture.service.restore_deleted(&deleted.id).unwrap();
    assert_eq!(fixture.disk("src/moved.rs"), b"");
}

#[test]
fn edit_27_interrupted_precommit_leaves_original_and_cleans_prepared_file() {
    let fixture = Fixture::new();
    let reference = fixture.reference("src/main.rs");
    let before = fixture.service.read(&reference).unwrap();
    assert!(fixture
        .service
        .save_hook(&reference, &before.revision, "never committed", || Err(
            FileError::Io
        ))
        .is_err());
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
    assert!(!fs::read_dir(fixture.directory.path().join("src"))
        .unwrap()
        .flatten()
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with(".asf-editor-")));
}

#[test]
fn edit_28_replaced_same_name_does_not_accept_an_old_revision() {
    let fixture = Fixture::new();
    let reference = fixture.reference("src/main.rs");
    let before = fixture.service.read(&reference).unwrap();
    fs::remove_file(fixture.directory.path().join("src/main.rs")).unwrap();
    fs::write(
        fixture.directory.path().join("src/main.rs"),
        b"fn main() {}\n",
    )
    .unwrap();
    assert!(matches!(
        fixture.service.save(&reference, &before.revision, "stale"),
        Err(FileError::Conflict)
    ));
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}

#[test]
fn fs_sec_09_10_19_project_and_root_references_are_not_authority() {
    let fixture = Fixture::new();
    let mut reference = fixture.reference("src/main.rs");
    reference.project_id = format!("p_{}", "b".repeat(32));
    assert!(matches!(
        fixture.service.read(&reference),
        Err(FileError::Forbidden)
    ));
    reference = fixture.reference("src/main.rs");
    reference.root_id = format!("r_{}", "b".repeat(32));
    assert!(matches!(
        fixture.service.read(&reference),
        Err(FileError::UnknownRoot)
    ));
}

#[test]
fn edit_24_agent_worktree_is_real_distinct_and_read_only() {
    let fixture = Fixture::new();
    fixture.initialize_git();
    let worktree = fixture.directory.path().join(".factory/worktrees/t1");
    fs::create_dir_all(worktree.parent().unwrap()).unwrap();
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "agent-test",
        worktree.to_str().unwrap(),
        "HEAD",
    ]);
    fs::write(worktree.join("src/main.rs"), "agent-only").unwrap();
    let roots = fixture.service.roots().unwrap();
    assert_eq!(roots.len(), 2);
    let managed = roots.iter().find(|r| r.read_only).unwrap();
    assert!(matches!(
        fixture.service.process_root(&managed.id),
        Err(FileError::ReadOnly)
    ));
    let reference = FileRef {
        project_id: fixture.service.project_id.clone(),
        root_id: managed.id.clone(),
        path: "src/main.rs".into(),
    };
    let snapshot = fixture.service.read(&reference).unwrap();
    assert_eq!(snapshot.content.as_deref(), Some("agent-only"));
    assert!(snapshot.read_only);
    assert!(matches!(
        fixture
            .service
            .save(&reference, &snapshot.revision, "human"),
        Err(FileError::ReadOnly)
    ));
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}

#[test]
fn edit_17_18_19_fs_sec_18_search_is_real_bounded_and_cancelled() {
    let fixture = Fixture::new();
    fs::write(
        fixture.directory.path().join("src/été.rs"),
        "one\nmatching café\n",
    )
    .unwrap();
    fs::write(fixture.directory.path().join(".gitignore"), "ignored/\n").unwrap();
    fs::create_dir(fixture.directory.path().join("ignored")).unwrap();
    fs::write(
        fixture.directory.path().join("ignored/secret.rs"),
        "matching secret",
    )
    .unwrap();
    let request = SearchRequest {
        id: format!("q_{}", nonce()),
        root_id: fixture.service.primary_root().id,
        query: "matching".into(),
        content: true,
        case_sensitive: true,
        regex: false,
        include_hidden: false,
        path_filter: String::new(),
    };
    let cancellation = fixture.service.begin_search(&request.id).unwrap();
    let result = fixture.service.search(&request, &cancellation).unwrap();
    assert!(result.complete);
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].line, Some(2));
    assert_eq!(result.hits[0].file.path, "src/été.rs");
    let cancellation = fixture.service.begin_search(&request.id).unwrap();
    fixture.service.cancel_search(&request.id).unwrap();
    assert_eq!(
        fixture
            .service
            .search(&request, &cancellation)
            .unwrap()
            .reason
            .as_deref(),
        Some("cancelled")
    );
}

#[test]
fn drafts_have_revisions_and_do_not_overwrite_disk() {
    let fixture = Fixture::new();
    let snapshot = fixture
        .service
        .read(&fixture.reference("src/main.rs"))
        .unwrap();
    let id = format!("d_{}", nonce());
    let draft = Draft {
        schema: 1,
        id: id.clone(),
        file: snapshot.file,
        base_revision: snapshot.revision,
        base_content: snapshot.content.unwrap(),
        content: "unsaved".into(),
        newline: "lf".into(),
        bom: false,
        dirty: true,
    };
    let saved = fixture.service.save_draft(draft.clone(), 0).unwrap();
    assert_eq!(saved.revision, 1);
    assert!(matches!(
        fixture.service.save_draft(draft, 0),
        Err(FileError::DraftConflict)
    ));
    let reopened = FileService::new(
        &fixture.service.project_id,
        fixture.directory.path(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(reopened.draft(&id).unwrap().document.content, "unsaved");
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}

#[test]
fn directory_listing_is_sorted_paginated_and_protected() {
    let fixture = Fixture::new();
    fixture.initialize_git();
    for index in (0..520).rev() {
        fs::write(
            fixture
                .directory
                .path()
                .join(format!("file-{index:04}.txt")),
            "data",
        )
        .unwrap();
    }
    let first = fixture
        .service
        .list(&fixture.service.primary_root().id, "", None, false)
        .unwrap();
    assert_eq!(first.entries.len(), 200);
    assert_eq!(first.entries[0].kind, "directory");
    assert!(!first
        .entries
        .iter()
        .any(|entry| entry.path.starts_with(".factory") || entry.path.starts_with(".git")));
    let second = fixture
        .service
        .list(
            &fixture.service.primary_root().id,
            "",
            first.next.as_deref(),
            false,
        )
        .unwrap();
    assert!(second
        .entries
        .iter()
        .all(|entry| !first.entries.iter().any(|old| old.path == entry.path)));
}

#[cfg(unix)]
#[test]
fn fs_sec_03_04_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "outside").unwrap();
    symlink(outside.path(), fixture.directory.path().join("escape")).unwrap();
    symlink(
        outside.path().join("secret"),
        fixture.directory.path().join("leaf"),
    )
    .unwrap();
    assert!(fixture
        .service
        .read(&fixture.reference("escape/secret"))
        .is_err());
    assert!(fixture.service.read(&fixture.reference("leaf")).is_err());
    assert!(fixture
        .service
        .create(&fixture.reference("escape/new"), false)
        .is_err());
}

#[cfg(windows)]
#[test]
fn fs_sec_05_windows_junction_is_rejected() {
    use std::os::windows::process::CommandExt;
    let fixture = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "outside").unwrap();
    let link = fixture.directory.path().join("escape");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fixture
        .service
        .read(&fixture.reference("escape/secret"))
        .is_err());
    assert!(fixture
        .service
        .create(&fixture.reference("escape/new"), false)
        .is_err());
    assert!(!outside.path().join("new").exists());
}

#[cfg(windows)]
#[test]
fn windows_case_only_rename_preserves_content() {
    let fixture = Fixture::new();
    let snapshot = fixture
        .service
        .read(&fixture.reference("src/main.rs"))
        .unwrap();
    let result = fixture
        .service
        .move_entry(&snapshot.file, &snapshot.revision, "src/Main.rs")
        .unwrap();
    assert!(result.to.path.ends_with("Main.rs"));
    assert_eq!(fixture.disk("src/Main.rs"), b"fn main() {}\n");
}

#[test]
fn recovery_versions_can_be_inspected_and_only_explicitly_purged() {
    let fixture = Fixture::new();
    let file = fixture.reference("src/main.rs");
    let revision = fixture.service.read(&file).unwrap().revision;
    let deleted = fixture.service.delete(&file, &revision, true).unwrap();
    let retained = fixture.service.recovery_content(&deleted.id).unwrap();
    assert!(retained.read_only);
    assert_eq!(retained.content.as_deref(), Some("fn main() {}\n"));
    assert!(fixture.service.purge_recovery(&deleted.id, false).is_err());
    fixture.service.purge_recovery(&deleted.id, true).unwrap();
    assert!(fixture.service.recovery().unwrap().is_empty());
    assert!(!fixture.directory.path().join("src/main.rs").exists());
}

#[test]
fn corrupt_draft_storage_does_not_reset_or_overwrite_user_data() {
    let fixture = Fixture::new();
    let path = fixture
        .directory
        .path()
        .join(".factory/editor-state.sqlite3");
    fs::write(&path, b"not a sqlite database").unwrap();
    assert!(fixture
        .service
        .draft(&format!("d_{}", "a".repeat(32)))
        .is_err());
    assert_eq!(fs::read(path).unwrap(), b"not a sqlite database");
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}

#[test]
fn git_head_identifies_the_committed_version_without_changing_disk() {
    let fixture = Fixture::new();
    fixture.initialize_git();
    fs::write(fixture.directory.path().join("src/main.rs"), "dirty disk").unwrap();
    let result = fixture
        .service
        .git_head(&fixture.reference("src/main.rs"))
        .unwrap();
    assert!(result.revision.starts_with("git:"));
    assert!(result.read_only);
    assert_eq!(result.content.as_deref(), Some("fn main() {}\n"));
    assert_eq!(fixture.disk("src/main.rs"), b"dirty disk");
}

#[test]
fn fs_sec_11_git_reads_refuse_a_worktree_redirected_to_another_project() {
    let fixture = Fixture::new();
    fixture.initialize_git();
    let foreign = Fixture::new();
    foreign.initialize_git();
    let worktree = fixture.directory.path().join(".factory/worktrees/t7");
    fs::create_dir_all(worktree.parent().unwrap()).unwrap();
    fixture.git(&[
        "worktree",
        "add",
        "--detach",
        worktree.to_str().unwrap(),
        "HEAD",
    ]);
    let root = fixture
        .service
        .roots()
        .unwrap()
        .into_iter()
        .find(|root| root.kind == "task_worktree")
        .unwrap();
    fs::write(
        worktree.join(".git"),
        format!(
            "gitdir: {}\n",
            foreign.directory.path().join(".git").display()
        ),
    )
    .unwrap();
    let file = FileRef {
        project_id: fixture.service.project_id.clone(),
        root_id: root.id,
        path: "src/main.rs".into(),
    };
    assert!(matches!(
        fixture.service.git_head(&file),
        Err(FileError::Forbidden)
    ));
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}

#[cfg(windows)]
#[test]
fn windows_exclusive_file_lock_refuses_a_save_without_false_success() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    let file = fixture.reference("src/main.rs");
    let revision = fixture.service.read(&file).unwrap().revision;
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(fixture.directory.path().join("src/main.rs"))
        .unwrap();
    assert!(fixture
        .service
        .save(&file, &revision, "must not save")
        .is_err());
    drop(lock);
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}

#[cfg(windows)]
#[test]
fn fs_sec_03_04_windows_symbolic_links_are_rejected_when_available() {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    let fixture = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "outside").unwrap();
    if let Err(error) = symlink_file(
        outside.path().join("secret"),
        fixture.directory.path().join("leaf"),
    ) {
        if error.raw_os_error() == Some(1314) {
            eprintln!("NOT EXECUTED: Windows symlink creation privilege unavailable; junction test covers available reparse creation.");
            return;
        }
        panic!("{error}");
    }
    symlink_dir(outside.path(), fixture.directory.path().join("middle")).unwrap();
    for name in ["leaf", "middle/secret"] {
        assert!(fixture.service.read(&fixture.reference(name)).is_err());
        assert!(fixture
            .service
            .save(&fixture.reference(name), "v_stale", "bad")
            .is_err());
    }
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "outside"
    );
}

#[cfg(unix)]
#[test]
fn unix_executable_bits_are_preserved() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let fixture = Fixture::new();
    let path = fixture.directory.path().join("src/main.rs");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let snapshot = fixture
        .service
        .read(&fixture.reference("src/main.rs"))
        .unwrap();
    fixture
        .service
        .save(&snapshot.file, &snapshot.revision, "changed\n")
        .unwrap();
    assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o755);
}

#[test]
fn p08_integration_snapshots_are_authorized_distinct_and_read_only() {
    let fixture = Fixture::new();
    fixture.initialize_git();
    let snapshot = fixture.directory.path().join(".factory/integrations/r1-a1");
    fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "factory/integration-1-1",
        snapshot.to_str().unwrap(),
        "HEAD",
    ]);
    fs::write(snapshot.join("src/main.rs"), "integration snapshot\n").unwrap();
    let roots = fixture.service.roots().unwrap();
    let root = roots
        .iter()
        .find(|root| root.kind == "integration_snapshot")
        .unwrap();
    assert!(root.read_only);
    assert!(root.label.contains("Integration snapshot"));
    let reference = FileRef {
        project_id: fixture.service.project_id.clone(),
        root_id: root.id.clone(),
        path: "src/main.rs".into(),
    };
    let document = fixture.service.read(&reference).unwrap();
    assert_eq!(document.content.as_deref(), Some("integration snapshot\n"));
    assert!(matches!(
        fixture
            .service
            .save(&reference, &document.revision, "overwrite"),
        Err(FileError::ReadOnly)
    ));
    assert!(matches!(
        fixture.service.process_root(&root.id),
        Err(FileError::ReadOnly)
    ));
    assert_eq!(fixture.disk("src/main.rs"), b"fn main() {}\n");
}
