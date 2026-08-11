use std::path::Path;
use std::process::Command;

use factory_core::provider::LocalProvider;
use factory_core::Factory;
use factory_models::TaskState;
use tempfile::TempDir;

fn init_git(dir: &Path) {
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Factory Test"],
        vec!["add", "."],
    ] {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    }
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", "init"])
        .status()
        .unwrap()
        .success();
    assert!(ok, "git commit failed");
}

fn with_factory() -> (TempDir, Factory) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("README.md"), "repo").unwrap();
    init_git(dir.path());
    let factory = Factory::init(dir.path(), false, Box::new(LocalProvider::new())).unwrap();
    (dir, factory)
}

#[test]
fn creates_a_planned_run_with_deterministic_states() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();

    assert_eq!(outcome.run.status.as_str(), "planned");
    assert_eq!(outcome.tasks.len(), 5);

    let t1 = &outcome.tasks[0];
    assert_eq!(t1.state, TaskState::Ready);
    assert!(t1.dependencies.is_empty());

    let t2 = &outcome.tasks[1];
    assert_eq!(t2.state, TaskState::Pending);
    assert_eq!(t2.dependencies, vec![t1.id]);

    let run = factory.get_run(outcome.run.id).unwrap().unwrap();
    assert!(run.objective.contains("calculator"));
}

#[test]
fn completing_a_task_readies_its_dependents() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let t1 = outcome.tasks[0].id;
    let t2 = outcome.tasks[1].id;

    factory.mark_task(t1, TaskState::Running).unwrap();
    let marked = factory.mark_task(t1, TaskState::Completed).unwrap();
    assert!(marked.updated.contains(&t2));

    let t2_now = factory.get_task(t2).unwrap().unwrap();
    assert_eq!(t2_now.state, TaskState::Ready);
}

#[test]
fn failing_a_task_blocks_its_transitive_dependents() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let t1 = outcome.tasks[0].id;
    let t2 = outcome.tasks[1].id;
    let t3 = outcome.tasks[2].id;

    factory.mark_task(t1, TaskState::Running).unwrap();
    factory.mark_task(t1, TaskState::Failed).unwrap();

    assert_eq!(
        factory.get_task(t2).unwrap().unwrap().state,
        TaskState::Blocked
    );
    assert_eq!(
        factory.get_task(t3).unwrap().unwrap().state,
        TaskState::Blocked
    );
}

#[test]
fn recovers_a_blocked_task_when_its_dependency_recovers() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let t1 = outcome.tasks[0].id;
    let t2 = outcome.tasks[1].id;

    factory.mark_task(t1, TaskState::Running).unwrap();
    factory.mark_task(t1, TaskState::Failed).unwrap();
    assert_eq!(
        factory.get_task(t2).unwrap().unwrap().state,
        TaskState::Blocked
    );

    factory.mark_task(t1, TaskState::Ready).unwrap();
    assert_eq!(
        factory.get_task(t2).unwrap().unwrap().state,
        TaskState::Pending
    );

    factory.mark_task(t1, TaskState::Running).unwrap();
    factory.mark_task(t1, TaskState::Completed).unwrap();
    assert_eq!(
        factory.get_task(t2).unwrap().unwrap().state,
        TaskState::Ready
    );
}

#[test]
fn rejects_invalid_transitions() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let t1 = outcome.tasks[0].id;

    factory.mark_task(t1, TaskState::Running).unwrap();
    factory.mark_task(t1, TaskState::Completed).unwrap();

    let err = factory.mark_task(t1, TaskState::Running).unwrap_err();
    assert!(err.to_string().contains("invalid state transition"));
}

#[test]
fn completed_tasks_do_not_get_re_blocked() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let t1 = outcome.tasks[0].id;
    let t2 = outcome.tasks[1].id;

    factory.mark_task(t1, TaskState::Running).unwrap();
    factory.mark_task(t1, TaskState::Completed).unwrap();
    factory.mark_task(t2, TaskState::Running).unwrap();
    factory.mark_task(t2, TaskState::Completed).unwrap();

    let t2_now = factory.get_task(t2).unwrap().unwrap();
    assert_eq!(t2_now.state, TaskState::Completed);
}

#[test]
fn worktree_requires_a_ready_task() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let pending = outcome.tasks[1].id;

    let err = factory.create_worktree(pending).unwrap_err();
    assert!(err.to_string().contains("not ready to run"));
}

#[test]
fn creates_and_removes_a_worktree_for_a_task() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let t1 = outcome.tasks[0].id;

    let path = factory.create_worktree(t1).unwrap();
    assert!(path.exists());
    let task = factory.get_task(t1).unwrap().unwrap();
    assert!(task.worktree_path.is_some());

    factory.remove_worktree(t1).unwrap();
    assert!(!path.exists());
    let task = factory.get_task(t1).unwrap().unwrap();
    assert!(task.worktree_path.is_none());
}

#[test]
fn init_requires_force_to_overwrite_existing_state() {
    let dir = TempDir::new().unwrap();
    assert!(Factory::init(dir.path(), false, Box::new(LocalProvider::new())).is_ok());
    match Factory::init(dir.path(), false, Box::new(LocalProvider::new())) {
        Ok(_) => panic!("expected second init to fail"),
        Err(e) => assert!(e.to_string().contains("already initialized")),
    }
    assert!(Factory::init(dir.path(), true, Box::new(LocalProvider::new())).is_ok());
}
