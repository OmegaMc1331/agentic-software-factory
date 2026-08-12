use std::path::Path;
use std::process::Command;

use factory_core::Factory;
use factory_types::TaskState;
use tempfile::TempDir;

const DEFAULT_PLAN: &str = r#"{"objective":"build a calculator","tasks":[
{"id":"T1","title":"Clarify the objective","objective":"define requirements","dependencies":[],"acceptanceCriteria":["requirements explicit"]},
{"id":"T2","title":"Set up the scaffold","objective":"create project skeleton","dependencies":["T1"],"acceptanceCriteria":["project builds"]},
{"id":"T3","title":"Implement core","objective":"implement behaviour","dependencies":["T2"],"acceptanceCriteria":["core works"]},
{"id":"T4","title":"Write tests","objective":"add tests","dependencies":["T3"],"acceptanceCriteria":["tests pass"]},
{"id":"T5","title":"Document","objective":"write docs","dependencies":["T4"],"acceptanceCriteria":["docs written"]}
]}"#;

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

fn fake_planner_config(plan_path: &Path) -> String {
    let (command, args) = if cfg!(windows) {
        (
            "powershell".to_string(),
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                format!("Get-Content '{}'", plan_path.display()),
            ],
        )
    } else {
        (
            "sh".to_string(),
            vec!["-c".to_string(), format!("cat '{}'", plan_path.display())],
        )
    };
    format!(
        r#"[agents.fake]
command = "{command}"
args = {args:?}

[roles.planner]
agent = "fake"
"#
    )
}

fn with_factory_and_plan(plan: &str) -> (TempDir, Factory) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("README.md"), "repo").unwrap();
    init_git(dir.path());
    let plan_path = dir.path().join("plan.json");
    std::fs::write(&plan_path, plan).unwrap();
    let factory_dir = dir.path().join(".factory");
    std::fs::create_dir_all(&factory_dir).unwrap();
    std::fs::write(
        factory_dir.join("config.toml"),
        fake_planner_config(&plan_path),
    )
    .unwrap();
    let factory = Factory::init(dir.path(), false).unwrap();
    (dir, factory)
}

fn with_factory() -> (TempDir, Factory) {
    with_factory_and_plan(DEFAULT_PLAN)
}

#[test]
fn creates_a_planned_run_with_deterministic_states() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();

    assert_eq!(outcome.run.status.as_str(), "planned");
    assert_eq!(outcome.tasks.len(), 5);
    assert_eq!(outcome.run.planner_agent.as_deref(), Some("fake"));

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
fn persists_a_planner_agent_session() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let sessions = factory.list_agent_sessions(Some(outcome.run.id)).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].role, "planner");
    assert_eq!(sessions[0].agent, "fake");
    assert!(sessions[0]
        .stdout
        .as_deref()
        .unwrap()
        .contains("calculator"));
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

    factory.remove_worktree(t1, false).unwrap();
    assert!(!path.exists());
    let task = factory.get_task(t1).unwrap().unwrap();
    assert!(task.worktree_path.is_none());
}

#[test]
fn refuses_to_remove_a_dirty_worktree() {
    let (_dir, factory) = with_factory();
    let outcome = factory.create_run("build a calculator").unwrap();
    let t1 = outcome.tasks[0].id;

    let path = factory.create_worktree(t1).unwrap();
    std::fs::write(path.join("wip.txt"), "uncommitted").unwrap();

    let err = factory.remove_worktree(t1, false).unwrap_err();
    assert!(err.to_string().contains("uncommitted changes"));
    assert!(path.exists());

    factory.remove_worktree(t1, true).unwrap();
    assert!(!path.exists());
}

#[test]
fn init_requires_force_to_overwrite_existing_state() {
    let dir = TempDir::new().unwrap();
    assert!(Factory::init(dir.path(), false).is_ok());
    match Factory::init(dir.path(), false) {
        Ok(_) => panic!("expected second init to fail"),
        Err(e) => assert!(e.to_string().contains("already initialized")),
    }
    assert!(Factory::init(dir.path(), true).is_ok());
}

#[test]
fn init_writes_a_default_agent_configuration() {
    let dir = TempDir::new().unwrap();
    Factory::init(dir.path(), false).unwrap();
    let config = std::fs::read_to_string(dir.path().join(".factory").join("config.toml")).unwrap();
    assert!(config.contains("[agents.codex]"));
    assert!(config.contains("[roles.planner]"));
}
