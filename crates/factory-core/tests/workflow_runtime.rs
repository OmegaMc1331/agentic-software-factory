use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use factory_core::{AgentEntry, Config, Factory, RoleEntry, WorkflowResult, MAX_TASK_ATTEMPTS};
use factory_types::{AttemptStatus, RunStatus, TaskState};
use tempfile::TempDir;

const PLAN: &str = r#"{"objective":"ship the test workflow","tasks":[{"id":"T1","title":"First task","objective":"write the first change","dependencies":[],"acceptanceCriteria":["worker evidence exists"]},{"id":"T2","title":"Second task","objective":"write the second change","dependencies":["T1"],"acceptanceCriteria":["reviewer approves"]}]}"#;

fn init_git(root: &Path) {
    std::fs::write(root.join("README.md"), "test repository\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "factory@example.test"][..],
        &["config", "user.name", "Factory Test"][..],
        &["add", "."][..],
        &["commit", "-q", "-m", "initial"][..],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
}

fn command_entry(script: &str) -> AgentEntry {
    if cfg!(windows) {
        AgentEntry {
            command: "cmd".into(),
            args: vec!["/d".into(), "/c".into(), script.into()],
            env: BTreeMap::new(),
            capabilities: Vec::new(),
        }
    } else {
        AgentEntry {
            command: "sh".into(),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::new(),
            capabilities: Vec::new(),
        }
    }
}

fn fixture(review_decision: &str) -> (TempDir, Factory) {
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    Factory::init(dir.path()).unwrap();

    let plan_path = dir.path().join("test-plan.json");
    let worker_path = dir.path().join("test-worker.json");
    let reviewer_path = dir.path().join("test-reviewer.json");
    std::fs::write(&plan_path, PLAN).unwrap();
    std::fs::write(
        &worker_path,
        r#"{"commands":["fake-worker"],"tests":["fake-test"]}"#,
    )
    .unwrap();
    let review = if review_decision == "approve" {
        r#"{"decision":"approve","reason":"evidence accepted"}"#
    } else {
        r#"{"decision":"request_changes","reason":"needs revision","feedback":["adjust the change"]}"#
    };
    std::fs::write(&reviewer_path, review).unwrap();

    let planner = if cfg!(windows) {
        format!("type {}", plan_path.display())
    } else {
        format!("cat '{}'", plan_path.display())
    };
    let worker = if cfg!(windows) {
        format!(
            "echo done>worker-output.txt & type {}",
            worker_path.display()
        )
    } else {
        format!(
            "printf 'done\\n' > worker-output.txt; cat '{}'",
            worker_path.display()
        )
    };
    let reviewer = if cfg!(windows) {
        format!("type {}", reviewer_path.display())
    } else {
        format!("cat '{}'", reviewer_path.display())
    };

    let mut config = Config::default();
    config
        .agents
        .insert("planner-test".into(), command_entry(&planner));
    config
        .agents
        .insert("worker-test".into(), command_entry(&worker));
    config
        .agents
        .insert("reviewer-test".into(), command_entry(&reviewer));
    for (role, agent) in [
        ("planner", "planner-test"),
        ("worker", "worker-test"),
        ("reviewer", "reviewer-test"),
    ] {
        config.roles.insert(
            role.into(),
            RoleEntry {
                agent: agent.into(),
            },
        );
    }
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    (dir, factory)
}

#[test]
fn planner_success_persists_a_planned_workflow_and_session() {
    let (_dir, factory) = fixture("approve");
    let run = factory.begin_run("ship the workflow").unwrap();
    assert_eq!(run.status, RunStatus::Planning);

    let outcome = factory.plan_run(run.id, &AtomicBool::new(false)).unwrap();
    assert_eq!(outcome.run.status, RunStatus::Planned);
    assert_eq!(outcome.tasks.len(), 2);
    assert_eq!(outcome.tasks[0].state, TaskState::Ready);
    assert_eq!(outcome.tasks[1].dependencies, vec![outcome.tasks[0].id]);

    let sessions = factory.list_agent_sessions(Some(run.id)).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].role, "planner");
    assert_eq!(sessions[0].status, "success");
}

#[test]
fn planner_process_failure_marks_the_persisted_workflow_failed() {
    let (dir, _factory) = fixture("approve");
    let mut config = Config::load(dir.path()).unwrap();
    config.agents.insert(
        "planner-test".into(),
        command_entry(if cfg!(windows) { "exit /b 7" } else { "exit 7" }),
    );
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    let run = factory.begin_run("fail while planning").unwrap();

    assert!(factory.plan_run(run.id, &AtomicBool::new(false)).is_err());
    assert_eq!(
        factory.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Failed
    );
    let sessions = factory.list_agent_sessions(Some(run.id)).unwrap();
    assert_eq!(sessions[0].exit_code, Some(7));
    assert_eq!(sessions[0].status, "failed");
}

#[test]
fn sequential_scheduler_records_evidence_reviews_and_completion() {
    let (_dir, factory) = fixture("approve");
    let outcome = factory.create_run("execute in order").unwrap();
    let roles = factory.prepare_start(outcome.run.id).unwrap();
    assert_eq!(roles.worker, "worker-test");
    assert_eq!(roles.reviewer, "reviewer-test");

    let result = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result, WorkflowResult::Completed);
    assert_eq!(
        factory.get_run(outcome.run.id).unwrap().unwrap().status,
        RunStatus::Completed
    );

    let attempts = factory.list_task_attempts(outcome.run.id).unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].task_id, outcome.tasks[0].id);
    assert_eq!(attempts[1].task_id, outcome.tasks[1].id);
    assert!(attempts
        .iter()
        .all(|attempt| attempt.status == AttemptStatus::Approved));
    assert!(attempts.iter().all(|attempt| attempt
        .evidence
        .as_ref()
        .is_some_and(|evidence| evidence.changed_files == vec!["worker-output.txt"])));
    assert!(attempts.iter().all(|attempt| attempt
        .review
        .as_ref()
        .is_some_and(|review| review.reason == "evidence accepted")));

    let sessions = factory.list_agent_sessions(Some(outcome.run.id)).unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.role.as_str())
            .collect::<Vec<_>>(),
        vec!["planner", "worker", "reviewer", "worker", "reviewer"]
    );
    assert!(sessions
        .iter()
        .filter(|session| session.role != "planner")
        .all(|session| session.attempt_id.is_some()));
}

#[test]
fn reviewer_retries_are_bounded_and_cannot_be_reopened_after_the_limit() {
    let (_dir, factory) = fixture("request_changes");
    let outcome = factory.create_run("exercise retries").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();

    factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        factory.get_run(outcome.run.id).unwrap().unwrap().status,
        RunStatus::Failed
    );
    let attempts = factory.list_task_attempts(outcome.run.id).unwrap();
    assert_eq!(attempts.len(), MAX_TASK_ATTEMPTS as usize);
    assert!(attempts
        .iter()
        .all(|attempt| attempt.status == AttemptStatus::ChangesRequested));
    assert!(factory
        .prepare_retry(outcome.tasks[0].id)
        .unwrap_err()
        .to_string()
        .contains("retry limit"));
}

#[test]
fn start_requires_configured_worker_and_reviewer_roles() {
    let (dir, factory) = fixture("approve");
    let outcome = factory.create_run("check start eligibility").unwrap();
    drop(factory);
    let mut config = Config::load(dir.path()).unwrap();
    config.roles.remove("worker");
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();

    let error = factory.prepare_start(outcome.run.id).unwrap_err();
    assert!(error.to_string().contains("worker role"));
    assert_eq!(
        factory.get_run(outcome.run.id).unwrap().unwrap().status,
        RunStatus::Planned
    );
}

#[test]
fn execution_errors_do_not_leave_the_workflow_active() {
    let (dir, factory) = fixture("approve");
    let outcome = factory.create_run("record an execution failure").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    std::fs::remove_dir_all(dir.path().join(".git")).unwrap();

    assert!(factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .is_err());
    assert_eq!(
        factory.get_run(outcome.run.id).unwrap().unwrap().status,
        RunStatus::Failed
    );
}
