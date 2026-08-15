use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use factory_core::ExecutionClass;
use factory_core::{
    AgentEntry, Config, Factory, RoleAssignment, RoleDefinitionEntry, WorkflowTeam,
};
use factory_types::{AttemptStatus, RunStatus, TaskState};
use tempfile::TempDir;

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
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec!["/d".into(), "/c".into(), script.into()],
        )
    } else {
        ("sh".to_string(), vec!["-c".into(), script.into()])
    };
    AgentEntry {
        kind: None,
        command,
        args,
        env: BTreeMap::new(),
        prompt_transport: None,
        interactive_args: None,
        capabilities: Vec::new(),
    }
}

fn cat_script(path: &std::path::Path, marker: &str) -> String {
    if cfg!(windows) {
        format!("echo {marker}>worker-output.txt & type {}", path.display())
    } else {
        format!(
            "printf '{marker}\\n' > worker-output.txt; cat '{}'",
            path.display()
        )
    }
}

struct Fixture {
    _dir: TempDir,
    factory: Factory,
}

fn fixture(plan: &str, review: &str) -> Fixture {
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    Factory::init(dir.path()).unwrap();

    let plan_path = dir.path().join("plan.json");
    let review_path = dir.path().join("review.json");
    std::fs::write(&plan_path, plan).unwrap();
    std::fs::write(&review_path, review).unwrap();
    let plan_cat = if cfg!(windows) {
        format!("type {}", plan_path.display())
    } else {
        format!("cat '{}'", plan_path.display())
    };
    let review_cat = if cfg!(windows) {
        format!("type {}", review_path.display())
    } else {
        format!("cat '{}'", review_path.display())
    };

    let worker_output = dir.path().join("worker-output.json");
    std::fs::write(&worker_output, r#"{"commands":["fake"]}"#).unwrap();
    let worker = cat_script(&worker_output, "done");
    let db_output = dir.path().join("db-output.json");
    std::fs::write(&db_output, r#"{"commands":["fake-db"]}"#).unwrap();
    let db_worker = cat_script(&db_output, "db-done");

    let mut config = Config::default();
    config
        .agents
        .insert("planner-test".into(), command_entry(&plan_cat));
    config
        .agents
        .insert("worker-a".into(), command_entry(&worker));
    config
        .agents
        .insert("worker-b".into(), command_entry(&worker));
    config
        .agents
        .insert("db-test".into(), command_entry(&db_worker));
    config
        .agents
        .insert("reviewer-test".into(), command_entry(&review_cat));
    for (role, agent, preferred) in [
        ("planner", "planner-test", true),
        ("worker", "worker-a", true),
        ("worker", "worker-b", false),
        ("reviewer", "reviewer-test", true),
    ] {
        config.role_assignments.push(RoleAssignment {
            role: role.into(),
            agent: agent.into(),
            preferred,
        });
    }
    config.roles.insert(
        "database_engineer".into(),
        RoleDefinitionEntry {
            name: Some("Database Engineer".into()),
            description: Some("Designs and modifies relational database schemas.".into()),
            execution_class: Some(ExecutionClass::Execution),
            instructions: "Focus on schema design and migrations.".into(),
            agent: None,
        },
    );
    config.role_assignments.push(RoleAssignment {
        role: "database_engineer".into(),
        agent: "db-test".into(),
        preferred: true,
    });
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    Fixture { _dir: dir, factory }
}

const APPROVE: &str = r#"{"decision":"approve","reason":"evidence accepted"}"#;

const THREE_TASK_PLAN: &str = r#"{"objective":"parallel-ready plan","tasks":[
{"id":"T1","title":"First","objective":"one","dependencies":[],"acceptanceCriteria":["done"]},
{"id":"T2","title":"Second","objective":"two","dependencies":[],"acceptanceCriteria":["done"]},
{"id":"T3","title":"Third","objective":"three","dependencies":[],"acceptanceCriteria":["done"]}
]}"#;

#[test]
fn multiple_workers_route_round_robin_across_attempts() {
    let fixture = fixture(THREE_TASK_PLAN, APPROVE);
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into(), "worker-b".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let run = fixture
        .factory
        .begin_run("route workers", Some(team))
        .unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    fixture.factory.prepare_start(run.id).unwrap();
    fixture
        .factory
        .execute_active_run(run.id, &AtomicBool::new(false))
        .unwrap();

    let attempts = fixture.factory.list_task_attempts(run.id).unwrap();
    assert_eq!(attempts.len(), 3);
    let agents: Vec<&str> = attempts.iter().map(|a| a.agent.as_str()).collect();
    assert_eq!(agents, ["worker-a", "worker-b", "worker-a"]);
    assert!(attempts
        .iter()
        .all(|attempt| attempt.role.as_deref() == Some("worker")));
    assert!(attempts
        .iter()
        .all(|attempt| attempt.status == AttemptStatus::Approved));
    let sessions = fixture.factory.list_agent_sessions(Some(run.id)).unwrap();
    assert!(sessions
        .iter()
        .filter(|session| session.role == "worker")
        .all(|session| session.attempt_id.is_some()));
}

#[test]
fn task_role_routes_through_its_own_pool_and_persists_the_role() {
    let plan = r#"{"objective":"schema work","tasks":[
{"id":"T1","title":"Design schema","objective":"design","dependencies":[],"acceptanceCriteria":["schema designed"],"role":"database_engineer"}]}"#;
    let fixture = fixture(plan, APPROVE);
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::from([(
            "database_engineer".to_string(),
            vec!["db-test".to_string()],
        )]),
    };
    let run = fixture
        .factory
        .begin_run("custom role", Some(team))
        .unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    fixture.factory.prepare_start(run.id).unwrap();
    fixture
        .factory
        .execute_active_run(run.id, &AtomicBool::new(false))
        .unwrap();

    let attempts = fixture.factory.list_task_attempts(run.id).unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].agent, "db-test");
    assert_eq!(attempts[0].role.as_deref(), Some("database_engineer"));
    let sessions = fixture.factory.list_agent_sessions(Some(run.id)).unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.role.as_str())
            .collect::<Vec<_>>(),
        ["planner", "database_engineer", "reviewer"]
    );
    let tasks = fixture.factory.list_tasks(run.id).unwrap();
    assert_eq!(tasks[0].role.as_deref(), Some("database_engineer"));
}

#[test]
fn unknown_task_roles_are_rejected_during_planning() {
    let plan = r#"{"objective":"bad role","tasks":[
{"id":"T1","title":"Ghost","objective":"work","dependencies":[],"acceptanceCriteria":["done"],"role":"ghost_role"}]}"#;
    let fixture = fixture(plan, APPROVE);
    let run = fixture.factory.begin_run("bad role", None).unwrap();
    let error = fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("role 'ghost_role' which is not enabled for this workflow"));
    assert_eq!(
        fixture.factory.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Failed
    );
}

#[test]
fn unassigned_role_blocks_start_with_a_diagnostic() {
    let plan = r#"{"objective":"schema work","tasks":[
{"id":"T1","title":"Design schema","objective":"design","dependencies":[],"acceptanceCriteria":["schema designed"],"role":"database_engineer"}]}"#;
    let (dir, factory) = {
        let fixture = fixture(plan, APPROVE);
        (fixture._dir, fixture.factory)
    };
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::from([(
            "database_engineer".to_string(),
            vec!["db-test".to_string()],
        )]),
    };
    let run = factory.begin_run("custom role", Some(team)).unwrap();
    factory.plan_run(run.id, &AtomicBool::new(false)).unwrap();
    drop(factory);

    let mut config = Config::load(dir.path()).unwrap();
    config
        .role_assignments
        .retain(|assignment| assignment.role != "database_engineer");
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();

    let error = factory.prepare_start(run.id).unwrap_err();
    assert!(error
        .to_string()
        .contains("agent 'db-test' is not assigned to the 'database_engineer' role"));
    assert_eq!(
        factory.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Planned
    );
}

#[test]
fn team_snapshot_survives_global_config_changes() {
    let fixture = fixture(THREE_TASK_PLAN, APPROVE);
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-b".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let run = fixture
        .factory
        .begin_run("explicit team", Some(team.clone()))
        .unwrap();
    let stored = fixture.factory.get_run(run.id).unwrap().unwrap().team;
    assert_eq!(stored, Some(team));
    assert_eq!(
        fixture
            .factory
            .get_run(run.id)
            .unwrap()
            .unwrap()
            .planner_agent,
        Some("planner-test".to_string())
    );
    let default_run = fixture.factory.begin_run("default team", None).unwrap();
    let default_team = fixture
        .factory
        .get_run(default_run.id)
        .unwrap()
        .unwrap()
        .team;
    assert_eq!(
        default_team.map(|team| team.workers),
        Some(vec!["worker-a".to_string()])
    );
}

#[test]
fn retry_rotation_moves_to_the_next_reviewer_and_worker() {
    let request_changes = r#"{"decision":"request_changes","reason":"needs work"}"#;
    let fixture = fixture(THREE_TASK_PLAN, request_changes);
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into(), "worker-b".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let run = fixture.factory.begin_run("rotate", Some(team)).unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    fixture.factory.prepare_start(run.id).unwrap();
    fixture
        .factory
        .execute_active_run(run.id, &AtomicBool::new(false))
        .unwrap();
    let attempts = fixture.factory.list_task_attempts(run.id).unwrap();
    assert_eq!(attempts.len(), 3);
    let agents: Vec<&str> = attempts.iter().map(|a| a.agent.as_str()).collect();
    assert_eq!(agents, ["worker-a", "worker-b", "worker-a"]);
    assert!(attempts
        .iter()
        .all(|attempt| attempt.status == AttemptStatus::ChangesRequested));
}

#[test]
fn update_run_team_is_locked_once_active() {
    let fixture = fixture(THREE_TASK_PLAN, APPROVE);
    let run = fixture.factory.begin_run("locked team", None).unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-b".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    fixture.factory.update_run_team(run.id, team).unwrap();
    fixture.factory.prepare_start(run.id).unwrap();
    let locked = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let error = fixture.factory.update_run_team(run.id, locked).unwrap_err();
    assert!(error.to_string().contains("cannot be started while"));
    let task = fixture.factory.list_tasks(run.id).unwrap().remove(0);
    fixture
        .factory
        .mark_task(task.id, TaskState::Running)
        .unwrap();
    let _ = fixture.factory.mark_task(task.id, TaskState::Failed);
}
