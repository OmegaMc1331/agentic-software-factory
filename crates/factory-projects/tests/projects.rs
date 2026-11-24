use std::path::Path;
use std::sync::Arc;

use factory_core::Factory;
use factory_projects::{ProjectError, ProjectId, ProjectLimits, ProjectManager};
use tempfile::TempDir;

fn repository(parent: &Path, name: &str) -> std::path::PathBuf {
    let path = parent.join(name);
    std::fs::create_dir(&path).unwrap();
    factory_git::Repo::initialize(&path).unwrap();
    Factory::init(&path).unwrap();
    path
}

fn manager(parent: &Path) -> ProjectManager {
    ProjectManager::new(
        &parent.join("application/registry.sqlite3"),
        &[parent.to_owned()],
        ProjectLimits::default(),
    )
    .unwrap()
}

#[test]
fn stable_identity_separate_databases_and_display_name() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A avec espaces λ");
    let b = repository(temp.path(), "B");
    let app = manager(temp.path());
    let first = app.register(&a, Some("A")).unwrap();
    let second = app.register(&b, Some("B")).unwrap();
    let ca = app.open(&first.project_id).unwrap();
    let cb = app.open(&second.project_id).unwrap();
    let ra = ca.db.lock().unwrap().create_run("A only", None).unwrap();
    let rb = cb.db.lock().unwrap().create_run("B only", None).unwrap();
    assert_eq!(ra.id, rb.id);
    assert_ne!(
        ca.db.lock().unwrap().get_run(1).unwrap().unwrap().objective,
        cb.db.lock().unwrap().get_run(1).unwrap().unwrap().objective
    );
    assert_eq!(
        app.rename(&first.project_id, "Renamed")
            .unwrap()
            .entry
            .project_id,
        first.project_id
    );
    assert_eq!(
        app.register(&a.join("."), None).unwrap().project_id,
        first.project_id
    );
    drop(ca);
    drop(cb);
    drop(app);
    let restarted = manager(temp.path());
    assert_eq!(
        restarted.register(&a, None).unwrap().project_id,
        first.project_id
    );
    assert_eq!(
        restarted.get(&first.project_id).unwrap().lifecycle,
        "closed"
    );
}

#[test]
fn conversation_service_lease_prevents_premature_runtime_close() {
    let temp = TempDir::new().unwrap();
    let root = repository(temp.path(), "A");
    let app = manager(temp.path());
    let project = app.register(&root, None).unwrap();
    let context = app.open(&project.project_id).unwrap();
    let observer = context.conversations.clone();
    drop(context);
    assert!(matches!(
        app.close(&project.project_id),
        Err(ProjectError::Conflict(_))
    ));
    drop(observer);
    app.close(&project.project_id).unwrap();
    let reopened = app.open(&project.project_id).unwrap();
    assert_eq!(reopened.project_id, project.project_id);
}

#[test]
fn concurrent_registration_and_open_share_one_runtime() {
    let temp = TempDir::new().unwrap();
    let root = repository(temp.path(), "A");
    let app = Arc::new(manager(temp.path()));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let app = app.clone();
            let root = root.clone();
            std::thread::spawn(move || {
                let entry = app.register(&root, None).unwrap();
                app.open(&entry.project_id).unwrap()
            })
        })
        .collect();
    let contexts: Vec<_> = workers
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(contexts
        .iter()
        .all(|context| Arc::ptr_eq(context, &contexts[0])));
    assert_eq!(app.list().unwrap().len(), 1);
    let id = contexts[0].project_id.clone();
    assert!(app.close(&id).is_err());
    drop(contexts);
    app.close(&id).unwrap();
    let second_owner = factory_runtime::Runtime::new(&root).unwrap();
    drop(second_owner);
    app.open(&id).unwrap();
}

#[test]
fn unavailable_project_does_not_break_other_projects_and_can_be_relinked() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let app = manager(temp.path());
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    let moved = temp.path().join("moved");
    std::fs::rename(&a, &moved).unwrap();
    assert_eq!(app.get(&ea.project_id).unwrap().availability, "unavailable");
    assert!(app.open(&ea.project_id).is_err());
    assert!(app.open(&eb.project_id).is_ok());
    assert_eq!(app.list().unwrap().len(), 2);
    app.relink(&ea.project_id, &moved, false).unwrap();
    assert!(app.open(&ea.project_id).is_ok());
}

#[test]
fn close_remove_and_resource_limit_are_safe() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let app = ProjectManager::new(
        &temp.path().join("registry.sqlite3"),
        &[temp.path().to_owned()],
        ProjectLimits {
            open_projects: 1,
            ..ProjectLimits::default()
        },
    )
    .unwrap();
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    let context = app.open(&ea.project_id).unwrap();
    assert!(matches!(
        app.open(&eb.project_id),
        Err(ProjectError::Limit(_))
    ));
    assert!(app.remove(&ea.project_id).is_err());
    drop(context);
    app.close(&ea.project_id).unwrap();
    app.remove(&ea.project_id).unwrap();
    assert!(matches!(
        app.context(&ea.project_id),
        Err(ProjectError::NotFound)
    ));
    assert!(a.join(".factory/db.sqlite3").exists());
    assert!(app.open(&eb.project_id).is_ok());
}

#[test]
fn explicit_initialization_never_overwrites_existing_factory_data() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("uninitialized");
    std::fs::create_dir(&root).unwrap();
    factory_git::Repo::initialize(&root).unwrap();
    let app = manager(temp.path());
    let entry = app.register(&root, None).unwrap();
    assert_eq!(
        app.get(&entry.project_id).unwrap().availability,
        "not_initialized"
    );
    assert!(app.open(&entry.project_id).is_err());
    assert_eq!(
        app.get(&entry.project_id).unwrap().availability,
        "not_initialized"
    );
    app.initialize(&entry.project_id).unwrap();
    let original = std::fs::read(root.join(".factory/config.toml")).unwrap();
    assert!(app.initialize(&entry.project_id).is_err());
    assert_eq!(
        std::fs::read(root.join(".factory/config.toml")).unwrap(),
        original
    );
    assert!(app.open(&entry.project_id).is_ok());
}

#[test]
fn invalid_ids_and_out_of_scope_creation_are_rejected_without_writes() {
    for id in ["../A", "1", "p_", "p_0000000000000000000000000000000G", "A"] {
        assert!(ProjectId::parse(id).is_err());
    }
    let temp = TempDir::new().unwrap();
    let allowed = temp.path().join("allowed");
    std::fs::create_dir(&allowed).unwrap();
    let app = ProjectManager::new(
        &temp.path().join("registry.sqlite3"),
        std::slice::from_ref(&allowed),
        ProjectLimits::default(),
    )
    .unwrap();
    let destination = temp.path().join("outside");
    assert!(app.create(&destination, "No").is_err());
    assert!(!destination.exists());
    let created = app.create(&allowed.join("created"), "Created").unwrap();
    assert!(app.create(&created.canonical_root, "Again").is_err());
    assert_eq!(app.list().unwrap().len(), 1);
    assert!(app
        .clone_project("ext::evil", &allowed.join("bad"), "Bad", true, true)
        .is_err());
    assert!(!allowed.join("bad").exists());
}

#[test]
fn registry_corruption_is_not_replaced_or_treated_as_empty() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("registry.sqlite3");
    std::fs::write(&path, b"preserve corrupt registry for recovery").unwrap();
    assert!(factory_projects::ProjectRegistry::open(&path).is_err());
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"preserve corrupt registry for recovery"
    );
}

#[cfg(windows)]
#[test]
fn windows_case_and_junction_aliases_have_one_identity() {
    let temp = TempDir::new().unwrap();
    let root = repository(temp.path(), "MixedCase");
    let alias = temp.path().join("alias");
    let status = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&alias)
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let app = manager(temp.path());
    let first = app.register(&root, None).unwrap();
    assert_eq!(
        app.register(&alias, None).unwrap().project_id,
        first.project_id
    );
    assert_eq!(
        app.register(Path::new(&root.to_string_lossy().to_uppercase()), None)
            .unwrap()
            .project_id,
        first.project_id
    );
}

#[test]
fn local_clone_create_and_clone_failure_never_overwrite_or_false_register() {
    let temp = TempDir::new().unwrap();
    let source = repository(temp.path(), "source");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&source)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=p03@example.test",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let app = manager(temp.path());
    let cloned = temp.path().join("cloned");
    let entry = app
        .clone_project(source.to_str().unwrap(), &cloned, "Clone", true, false)
        .unwrap();
    assert!(cloned.join(".git").is_dir());
    assert!(app.open(&entry.project_id).is_ok());
    let invalid = temp.path().join("not-a-repo");
    std::fs::create_dir(&invalid).unwrap();
    let failed = temp.path().join("failed");
    assert!(app
        .clone_project(invalid.to_str().unwrap(), &failed, "Failed", true, false)
        .is_err());
    assert!(failed.exists());
    assert_eq!(app.list().unwrap().len(), 1);
    assert!(app.create(&temp.path().join("invalid-name"), " ").is_err());
    assert!(!temp.path().join("invalid-name").exists());
    assert!(app
        .clone_project(
            "https://token@example.test/secret",
            &temp.path().join("credential"),
            "No",
            false,
            true
        )
        .is_err());
}

#[test]
fn registry_write_and_runtime_open_faults_are_atomic_and_retryable() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let app = manager(temp.path());
    let connection =
        rusqlite::Connection::open(temp.path().join("application/registry.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_insert BEFORE INSERT ON projects BEGIN SELECT RAISE(ABORT,'injected registry failure'); END;").unwrap();
    assert!(app.register(&a, None).is_err());
    assert!(app.list().unwrap().is_empty());
    connection
        .execute_batch("DROP TRIGGER reject_insert;")
        .unwrap();
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_open BEFORE UPDATE OF last_opened_at ON projects BEGIN SELECT RAISE(ABORT,'injected open failure'); END;").unwrap();
    assert!(app.open(&ea.project_id).is_err());
    let owner = factory_runtime::Runtime::new(&a).unwrap();
    drop(owner);
    connection
        .execute_batch("DROP TRIGGER reject_open;")
        .unwrap();
    assert!(app.open(&eb.project_id).is_ok());
    assert!(app.open(&ea.project_id).is_ok());
}

#[test]
fn nested_repositories_shared_git_and_hardlinked_databases_are_rejected() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let nested = repository(&a, "nested");
    let b = repository(temp.path(), "B");
    let app = manager(temp.path());
    app.register(&a, None).unwrap();
    assert!(app.register(&nested, None).is_err());
    assert!(app.register(&a.join(".factory"), None).is_err());
    std::fs::remove_file(b.join(".factory/db.sqlite3")).unwrap();
    std::fs::hard_link(a.join(".factory/db.sqlite3"), b.join(".factory/db.sqlite3")).unwrap();
    assert!(app.register(&b, None).is_err());
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&a)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=p03@example.test",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(status.status.success());
    let linked = temp.path().join("linked");
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&a)
        .args(["worktree", "add", "-q", "-b", "linked"])
        .arg(&linked)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(app.register(&linked, None).is_err());
}

#[test]
fn corrupt_database_isolated_root_replacement_and_future_registry_fail_closed() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let app = manager(temp.path());
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    std::fs::write(a.join(".factory/db.sqlite3"), b"corrupt fixture retained").unwrap();
    assert!(app.open(&ea.project_id).is_err());
    assert_eq!(app.get(&ea.project_id).unwrap().availability, "unavailable");
    assert!(app.open(&eb.project_id).is_ok());
    std::fs::rename(&a, temp.path().join("old-a")).unwrap();
    let replacement = repository(temp.path(), "A");
    assert!(app.open(&ea.project_id).is_err());
    assert!(app.relink(&ea.project_id, &replacement, false).is_err());
    app.relink(&ea.project_id, &replacement, true).unwrap();
    assert!(app.open(&ea.project_id).is_ok());
    let future = temp.path().join("future.sqlite3");
    let c = rusqlite::Connection::open(&future).unwrap();
    c.execute_batch("PRAGMA user_version=99; CREATE TABLE proof(value TEXT); INSERT INTO proof VALUES('retained');").unwrap();
    drop(c);
    assert!(factory_projects::ProjectRegistry::open(&future).is_err());
    let c = rusqlite::Connection::open(&future).unwrap();
    assert_eq!(
        c.query_row("SELECT value FROM proof", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "retained"
    );
}

#[test]
fn child_lock_holder() {
    let Some(root) = std::env::var_os("ASF_P03_LOCK_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let _runtime = factory_runtime::Runtime::new(&root).unwrap();
    std::fs::write(root.join("ready"), "ready").unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !root.join("release").exists() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn os_lock_contention_in_another_process_does_not_block_b() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let app = manager(temp.path());
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_lock_holder", "--nocapture"])
        .env("ASF_P03_LOCK_ROOT", &a)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !a.join("ready").exists() {
        if std::time::Instant::now() > until {
            child.kill().ok();
            panic!("lock-holder did not start");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.open(&ea.project_id).is_err());
    assert!(app.open(&eb.project_id).is_ok());
    std::fs::write(a.join("release"), "release").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(app.open(&ea.project_id).is_ok());
}

#[cfg(windows)]
#[test]
fn junction_cannot_escape_authorized_roots_or_redirect_factory_state() {
    let temp = TempDir::new().unwrap();
    let allowed = temp.path().join("allowed");
    std::fs::create_dir(&allowed).unwrap();
    let outside = repository(temp.path(), "outside");
    let alias = allowed.join("escape");
    let make = |a: &Path, b: &Path| {
        let output = std::process::Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(a)
            .arg(b)
            .output()
            .unwrap();
        assert!(output.status.success());
    };
    make(&alias, &outside);
    let app = ProjectManager::new(
        &temp.path().join("registry.sqlite3"),
        std::slice::from_ref(&allowed),
        ProjectLimits::default(),
    )
    .unwrap();
    assert!(app.register(&alias, None).is_err());
    let root = allowed.join("redirected");
    std::fs::create_dir(&root).unwrap();
    factory_git::Repo::initialize(&root).unwrap();
    make(&root.join(".factory"), &outside.join(".factory"));
    assert!(app.register(&root, None).is_err());
}

#[test]
fn frozen_v1_project_opens_and_restarts_without_losing_history_or_worktrees() {
    let temp = TempDir::new().unwrap();
    let root = repository(temp.path(), "legacy");
    let path = root.join(".factory/db.sqlite3");
    std::fs::remove_file(&path).unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    c.pragma_update(None, "foreign_keys", "OFF").unwrap();
    c.execute_batch(include_str!(
        "../../factory-db/tests/fixtures/asf-v1-1.0.5.sql"
    ))
    .unwrap();
    drop(c);
    let config = std::fs::read(root.join(".factory/config.toml")).unwrap();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=p03@example.test",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(status.status.success());
    let worktree = root.join(".factory/worktrees/t99");
    factory_git::Repo::detect(&root)
        .unwrap()
        .add_worktree(&worktree, "factory/t99", None)
        .unwrap();
    let app = manager(temp.path());
    let entry = app.register(&root, None).unwrap();
    for _ in 0..2 {
        let context = app.open(&entry.project_id).unwrap();
        let db = context.db.lock().unwrap();
        assert_eq!(
            db.get_run(1).unwrap().unwrap().objective,
            "SYNTHETIC V1 preservation workflow"
        );
        assert_eq!(db.list_tasks(1).unwrap().len(), 2);
        assert_eq!(db.list_task_attempts(1).unwrap().len(), 2);
        assert_eq!(db.list_agent_sessions(None).unwrap().len(), 5);
        assert_eq!(db.list_role_artifacts(1).unwrap().len(), 1);
        assert_eq!(db.get_run_github_link(1).unwrap().unwrap().issue_number, 42);
        assert!(db.get_delivery(1).unwrap().is_some());
        assert!(!db.list_plan_revisions(1).unwrap().is_empty());
        assert!(worktree.is_dir());
        assert_eq!(
            std::fs::read(root.join(".factory/config.toml")).unwrap(),
            config
        );
        drop(db);
        drop(context);
        app.close(&entry.project_id).unwrap();
    }
    drop(app);
    let restarted = manager(temp.path());
    assert_eq!(
        restarted.register(&root, None).unwrap().project_id,
        entry.project_id
    );
    assert!(restarted.open(&entry.project_id).is_ok());
    assert!(worktree.exists());
}

#[test]
fn migration_failure_keeps_verified_backup_and_other_projects_usable() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let c = rusqlite::Connection::open(a.join(".factory/db.sqlite3")).unwrap();
    c.execute("DELETE FROM schema_migrations WHERE version>=18", [])
        .unwrap();
    c.execute("INSERT INTO runs (objective,status,created_at,updated_at) VALUES('migration-proof','planned','old','old')",[]).unwrap();
    drop(c);
    let app = manager(temp.path());
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    assert!(app.open(&ea.project_id).is_err());
    assert!(app.open(&eb.project_id).is_ok());
    let c = rusqlite::Connection::open(a.join(".factory/db.sqlite3")).unwrap();
    assert_eq!(
        c.query_row("SELECT objective FROM runs WHERE id=1", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "migration-proof"
    );
    assert_eq!(
        c.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        17
    );
    let backup = std::fs::read_dir(a.join(".factory/backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let saved = rusqlite::Connection::open(backup).unwrap();
    assert_eq!(
        saved
            .query_row("SELECT objective FROM runs WHERE id=1", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "migration-proof"
    );
}

#[test]
fn registry_mutations_cannot_relink_or_remove_a_runtime_owned_by_another_manager() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let other = repository(temp.path(), "other");
    let one = manager(temp.path());
    let two = manager(temp.path());
    let entry = one.register(&a, None).unwrap();
    let context = one.open(&entry.project_id).unwrap();
    assert!(two.remove(&entry.project_id).is_err());
    assert!(two.relink(&entry.project_id, &other, true).is_err());
    assert_eq!(one.context(&entry.project_id).unwrap().root, context.root);
    drop(context);
    one.close(&entry.project_id).unwrap();
    two.relink(&entry.project_id, &other, true).unwrap();
    assert!(two.open(&entry.project_id).is_ok());
}

#[test]
fn disappearance_of_one_authorized_root_does_not_disable_other_authorized_roots() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let app = ProjectManager::new(
        &temp.path().join("registry.sqlite3"),
        &[a.clone(), b.clone()],
        ProjectLimits::default(),
    )
    .unwrap();
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    std::fs::rename(&a, temp.path().join("gone-a")).unwrap();
    assert!(app.open(&ea.project_id).is_err());
    assert!(app.open(&eb.project_id).is_ok());
    assert_eq!(app.list().unwrap().len(), 2);
}

#[test]
fn missing_registry_table_is_corruption_not_an_empty_registry() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("registry.sqlite3");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA user_version=1;").unwrap();
    drop(c);
    assert!(factory_projects::ProjectRegistry::open(&path).is_err());
    let c = rusqlite::Connection::open(path).unwrap();
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='projects'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn create_cannot_mutate_a_registered_projects_nested_directory() {
    let temp = TempDir::new().unwrap();
    let root = repository(temp.path(), "A");
    let app = manager(temp.path());
    app.register(&root, None).unwrap();
    let destination = root.join("nested");
    assert!(app.create(&destination, "Nested").is_err());
    assert!(!destination.exists());
}

#[test]
fn linked_worktree_requires_metadata_authorization_and_common_git_lock() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let linked = temp.path().join("linked");
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(&a)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=p03@example.test",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "fixture"
        ])
        .status()
        .unwrap()
        .success());
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(&a)
        .args(["worktree", "add", "-q", "-b", "linked"])
        .arg(&linked)
        .status()
        .unwrap()
        .success());
    Factory::init(&linked).unwrap();
    let restricted = ProjectManager::new(
        &temp.path().join("restricted.sqlite3"),
        std::slice::from_ref(&linked),
        ProjectLimits::default(),
    )
    .unwrap();
    assert!(restricted.register(&linked, None).is_err());
    let main_owner = factory_runtime::Runtime::new(&a).unwrap();
    let app = manager(temp.path());
    let entry = app.register(&linked, None).unwrap();
    assert!(app.open(&entry.project_id).is_err());
    drop(main_owner);
    assert!(app.open(&entry.project_id).is_ok());
}

#[test]
fn restart_reconciles_each_project_without_replaying_uncertain_delivery() {
    let temp = TempDir::new().unwrap();
    let a = repository(temp.path(), "A");
    let b = repository(temp.path(), "B");
    let app = manager(temp.path());
    let ea = app.register(&a, None).unwrap();
    let eb = app.register(&b, None).unwrap();
    for (root, label) in [(&a, "A"), (&b, "B")] {
        let db = factory_db::FactoryDb::open(&root.join(".factory/db.sqlite3")).unwrap();
        db.create_run(label, None).unwrap();
        db.get_or_create_delivery(1).unwrap();
        drop(db);
        let c = rusqlite::Connection::open(root.join(".factory/db.sqlite3")).unwrap();
        c.execute_batch("UPDATE runs SET status='active'; UPDATE github_deliveries SET state='pushing',operation_id='controlled-uncertain',recovery_required=1;").unwrap();
    }
    drop(app);
    let restored = manager(temp.path());
    assert_eq!(restored.list().unwrap().len(), 2);
    for (id, label) in [(&ea.project_id, "A"), (&eb.project_id, "B")] {
        let context = restored.open(id).unwrap();
        let db = context.db.lock().unwrap();
        let run = db.get_run(1).unwrap().unwrap();
        assert_eq!(run.objective, label);
        assert_eq!(run.status.as_str(), "failed");
        let delivery = db.get_delivery(1).unwrap().unwrap();
        assert_eq!(delivery.state.as_str(), "interrupted");
        assert!(delivery.recovery_required);
        assert!(context.runtime.is_idle());
        assert!(context.runtime.delivery_operation(1).is_none());
    }
}

#[cfg(windows)]
#[test]
fn unc_and_device_unc_variants_are_refused_before_network_lookup() {
    for value in [
        r"\\nonexistent.invalid\share",
        r"\\?\UNC\nonexistent.invalid\share",
        r"\\?\unc\nonexistent.invalid\share",
    ] {
        assert!(matches!(
            factory_projects::DirectoryIdentity::open(Path::new(value)),
            Err(ProjectError::Invalid(_))
        ));
    }
}

#[test]
fn cloning_initialized_v1_data_preserves_it_instead_of_initializing_over_it() {
    let temp = TempDir::new().unwrap();
    let source = repository(temp.path(), "source");
    std::fs::write(
        source.join(".gitattributes"),
        ".factory/config.toml -text\n",
    )
    .unwrap();
    let db = factory_db::FactoryDb::open(&source.join(".factory/db.sqlite3")).unwrap();
    db.create_run("preserved clone history", None).unwrap();
    drop(db);
    for args in [
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=p03@example.test",
            "commit",
            "-q",
            "-m",
            "controlled committed V1 data",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&source)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let before = std::fs::read(source.join(".factory/config.toml")).unwrap();
    let app = manager(temp.path());
    let destination = temp.path().join("clone");
    let entry = app
        .clone_project(
            source.to_str().unwrap(),
            &destination,
            "Preserved",
            true,
            false,
        )
        .unwrap();
    assert_eq!(
        std::fs::read(destination.join(".factory/config.toml")).unwrap(),
        before
    );
    let context = app.open(&entry.project_id).unwrap();
    assert_eq!(
        context
            .db
            .lock()
            .unwrap()
            .get_run(1)
            .unwrap()
            .unwrap()
            .objective,
        "preserved clone history"
    );
}
