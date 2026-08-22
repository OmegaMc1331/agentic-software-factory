//! Intelligent agent routing: end-to-end scenarios with synthetic evaluation
//! history. Every test uses fake shell agents and durable seeded attempts —
//! no real coding-agent credentials are involved.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use factory_core::factory_policy::{PolicyPreset, PolicyScope};
use factory_core::{
    AgentEntry, Config, Factory, RoleAssignment, RoutingConfig, RoutingMode, WorkflowTeam,
};
use factory_db::FactoryDb;
use factory_types::{AttemptStatus, RoutingDecision, TaskEvidence, TaskOperation, TaskState};
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
        max_concurrency: None,
    }
}

/// The fake worker behaviors the scenarios need.
#[derive(Clone, Copy)]
enum WorkerKind {
    /// Prints the producer report and exits 0.
    Approve,
    /// Fails the first invocation (leaving a marker file), succeeds after.
    FailOnce,
    /// Creates an untracked Rust file, then exits 1 — a failed attempt whose
    /// evidence marks the task as Rust work.
    RustFail,
}

struct RoutingFixture {
    dir: TempDir,
    factory: Factory,
    workers: Vec<String>,
}

fn script_for(kind: WorkerKind, output: &Path) -> String {
    let out = output.display();
    match kind {
        WorkerKind::Approve => {
            if cfg!(windows) {
                format!("type {out}")
            } else {
                format!("cat '{out}'")
            }
        }
        WorkerKind::FailOnce => {
            if cfg!(windows) {
                format!("if exist fail-once.txt (type {out}) else (echo x> fail-once.txt & exit 1)")
            } else {
                format!(
                    "if [ -f fail-once.txt ]; then cat '{out}'; else touch fail-once.txt; exit 1; fi"
                )
            }
        }
        WorkerKind::RustFail => {
            if cfg!(windows) {
                "echo pub fn touched() {}> probe_module.rs & exit 1".to_string()
            } else {
                "echo 'pub fn touched() {}' > probe_module.rs; exit 1".to_string()
            }
        }
    }
}

/// Builds a repository with fake planner/reviewer agents plus a worker pool,
/// and a routing configuration. Worker names are `worker-a`, `worker-b`, ...
/// in the order given.
fn routing_fixture(
    mode: RoutingMode,
    workers: &[WorkerKind],
    preferred_worker: Option<usize>,
    exploration: bool,
) -> RoutingFixture {
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    Factory::init(dir.path()).unwrap();

    let plan_path = dir.path().join("plan.json");
    std::fs::write(&plan_path, ONE_TASK_PLAN).unwrap();
    let review_path = dir.path().join("review.json");
    std::fs::write(&review_path, APPROVE).unwrap();
    let output_path = dir.path().join("worker-output.json");
    std::fs::write(&output_path, r#"{"commands":["fake"]}"#).unwrap();
    let cat = |path: &Path| -> String {
        if cfg!(windows) {
            format!("type {}", path.display())
        } else {
            format!("cat '{}'", path.display())
        }
    };

    let mut config = Config::default();
    config
        .agents
        .insert("planner-test".into(), command_entry(&cat(&plan_path)));
    config
        .agents
        .insert("reviewer-test".into(), command_entry(&cat(&review_path)));
    let mut worker_names = Vec::new();
    for (index, kind) in workers.iter().enumerate() {
        let name = format!("worker-{}", (b'a' + index as u8) as char);
        worker_names.push(name.clone());
        config.agents.insert(
            name.clone(),
            command_entry(&script_for(*kind, &output_path)),
        );
    }
    config.role_assignments.push(RoleAssignment {
        role: "planner".into(),
        agent: "planner-test".into(),
        preferred: true,
    });
    config.role_assignments.push(RoleAssignment {
        role: "reviewer".into(),
        agent: "reviewer-test".into(),
        preferred: true,
    });
    for (index, name) in worker_names.iter().enumerate() {
        config.role_assignments.push(RoleAssignment {
            role: "worker".into(),
            agent: name.clone(),
            preferred: preferred_worker == Some(index),
        });
    }
    config.routing = RoutingConfig { mode, exploration };
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    RoutingFixture {
        factory,
        dir,
        workers: worker_names,
    }
}

fn open_db(dir: &Path) -> FactoryDb {
    FactoryDb::open(&dir.join(".factory").join("db.sqlite3")).unwrap()
}

/// The synthetic outcome of one seeded (attributed) task.
enum SeedOutcome {
    Approved,
    Failed,
    Cancelled,
    /// A request-changes round rescued by approval: eventual success, no
    /// first-pass.
    ChangesThenApproved,
}

/// Seeds durable workflow history: `count` tasks attributed to `agent` on
/// the (role, operation) slice, each with `changed_file` as its evidence so
/// the language slice is populated.
fn seed(
    dir: &Path,
    agent: &str,
    role: &str,
    operation: TaskOperation,
    outcome: SeedOutcome,
    count: usize,
    changed_file: &str,
) {
    let db = open_db(dir);
    for index in 0..count {
        let run = db.create_run("history", Some("planner-test")).unwrap();
        let task = db
            .create_task(
                run.id,
                "History",
                "seeded history",
                &[],
                TaskState::Completed,
                index as i32,
                Some(role),
                Some(operation),
            )
            .unwrap();
        let evidence = TaskEvidence {
            changed_files: vec![changed_file.to_string()],
            ..TaskEvidence::default()
        };
        match outcome {
            SeedOutcome::Approved | SeedOutcome::Failed | SeedOutcome::Cancelled => {
                let (status, error) = match outcome {
                    SeedOutcome::Approved => (AttemptStatus::Approved, None),
                    SeedOutcome::Failed => (
                        AttemptStatus::Failed,
                        Some("agent process exited with code 1."),
                    ),
                    _ => (
                        AttemptStatus::Cancelled,
                        Some("Workflow cancelled while the agent was running."),
                    ),
                };
                let attempt = db
                    .create_task_attempt(task, role, Some(operation), agent, "seed", None)
                    .unwrap();
                db.finish_task_attempt(
                    attempt.id,
                    status,
                    Some(0),
                    None,
                    error,
                    Some(&evidence),
                    None,
                )
                .unwrap();
            }
            SeedOutcome::ChangesThenApproved => {
                let first = db
                    .create_task_attempt(task, role, Some(operation), agent, "seed", None)
                    .unwrap();
                db.finish_task_attempt(
                    first.id,
                    AttemptStatus::ChangesRequested,
                    Some(0),
                    None,
                    Some("rework requested"),
                    Some(&evidence),
                    None,
                )
                .unwrap();
                let second = db
                    .create_task_attempt(task, role, Some(operation), agent, "seed", None)
                    .unwrap();
                db.finish_task_attempt(
                    second.id,
                    AttemptStatus::Approved,
                    Some(0),
                    None,
                    None,
                    Some(&evidence),
                    None,
                )
                .unwrap();
            }
        }
    }
}

/// Runs a one-task workflow to completion and returns the run id.
fn run_one_task(fixture: &RoutingFixture) -> i64 {
    run_plan(fixture, ONE_TASK_PLAN)
}

fn run_plan(fixture: &RoutingFixture, plan: &str) -> i64 {
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: fixture.workers.clone(),
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let dir = fixture.dir.path();
    let plan_path = dir.join("plan.json");
    std::fs::write(&plan_path, plan).unwrap();
    let run = fixture
        .factory
        .begin_run("routing scenario", Some(team))
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
    run.id
}

fn attempts(dir: &Path, run_id: i64) -> Vec<(String, AttemptStatus)> {
    open_db(dir)
        .list_task_attempts(run_id)
        .unwrap()
        .into_iter()
        .map(|attempt| (attempt.agent, attempt.status))
        .collect()
}

fn first_task_id(dir: &Path, run_id: i64) -> i64 {
    open_db(dir).list_tasks(run_id).unwrap()[0].id
}

fn worker_decisions(dir: &Path, task_id: i64) -> Vec<RoutingDecision> {
    open_db(dir)
        .list_routing_decisions_for_task(task_id)
        .unwrap()
        .into_iter()
        .filter(|decision| decision.role.as_deref() == Some("worker"))
        .collect()
}

const APPROVE: &str = r#"{"decision":"approve","reason":"evidence accepted"}"#;
const ONE_TASK_PLAN: &str = r#"{"objective":"routing","tasks":[
{"id":"T1","title":"Only","objective":"work","dependencies":[],"acceptanceCriteria":["done"]}
]}"#;
const TWO_TASK_PLAN: &str = r#"{"objective":"routing","tasks":[
{"id":"T1","title":"First","objective":"one","dependencies":[],"acceptanceCriteria":["done"]},
{"id":"T2","title":"Second","objective":"two","dependencies":[],"acceptanceCriteria":["done"]}
]}"#;
const THREE_TASK_PLAN: &str = r#"{"objective":"routing","tasks":[
{"id":"T1","title":"First","objective":"one","dependencies":[],"acceptanceCriteria":["done"]},
{"id":"T2","title":"Second","objective":"two","dependencies":[],"acceptanceCriteria":["done"]},
{"id":"T3","title":"Third","objective":"three","dependencies":[],"acceptanceCriteria":["done"]}
]}"#;

#[test]
fn reliable_performance_wins() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Failed,
        12,
        "src/main.ts",
    );
    let run_id = run_one_task(&fixture);
    let attempts = attempts(dir, run_id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, "worker-a");
    assert_eq!(attempts[0].1, AttemptStatus::Approved);

    let decisions = worker_decisions(dir, first_task_id(dir, run_id));
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].selected_agent, "worker-a");
    assert_eq!(decisions[0].mode, "performance");
    assert!(decisions[0].reason.contains("reliable routing score"));
    // Failures are reliable evidence of badness: the losing agent is ranked,
    // strictly below the winner, not hidden as "insufficient data".
    let score_of = |agent: &str| {
        decisions[0]
            .candidate_scores
            .iter()
            .find(|candidate| candidate.agent == agent)
            .unwrap()
            .score
            .unwrap()
    };
    assert!(score_of("worker-a") > score_of("worker-b"));
}

#[test]
fn insufficient_data_falls_back_to_round_robin() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        true,
    );
    let dir = fixture.dir.path();
    let run_id = run_plan(&fixture, THREE_TASK_PLAN);
    let run_attempts = attempts(dir, run_id);
    let agents: Vec<&str> = run_attempts
        .iter()
        .map(|(agent, _)| agent.as_str())
        .collect();
    // Nobody is ranked: the historical capacity-aware rotation applies
    // unchanged and every decision explains the fallback.
    assert_eq!(agents, ["worker-a", "worker-b", "worker-a"]);
    for decision in worker_decisions(dir, first_task_id(dir, run_id)) {
        assert!(decision
            .reason
            .contains("Insufficient reliable performance data"));
    }
}

#[test]
fn language_slice_routes_retries_to_the_rust_specialist() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::RustFail, WorkerKind::Approve],
        Some(0), // worker-a preferred: wins the (tied) first dispatch
        false,
    );
    let dir = fixture.dir.path();
    // worker-a: reliable, but on TypeScript work only.
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    // worker-b: reliable specifically on Rust.
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.rs",
    );
    let run_id = run_one_task(&fixture);
    let attempts = attempts(dir, run_id);
    // Attempt 1 (language unknown): tie on role+operation, preferred wins.
    // It fails leaving Rust evidence. Attempt 2 (language = rust): the Rust
    // specialist outranks the preferred agent.
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].0, "worker-a");
    assert_eq!(attempts[1].0, "worker-b");
    assert_eq!(attempts[1].1, AttemptStatus::Approved);

    let decisions = worker_decisions(dir, first_task_id(dir, run_id));
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0].language, None);
    assert_eq!(decisions[1].language.as_deref(), Some("rust"));
}

#[test]
fn unreliable_specific_slice_falls_back_to_broader_reliable_slice() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    // 3 Rust tasks (below the reliability threshold) inside a reliable
    // broader history: routing must use the role+operation slice.
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        3,
        "src/main.rs",
    );
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        15,
        "src/main.ts",
    );
    let db = open_db(dir);
    let resolved = factory_eval::resolve_performance(
        &db,
        "worker-a",
        Some("worker"),
        Some(TaskOperation::Implement),
        Some("rust"),
        chrono::Utc::now(),
    )
    .unwrap()
    .expect("reliable broader slice exists");
    assert_eq!(
        resolved.level,
        factory_eval::PerformanceSliceLevel::RoleOperation
    );
    assert_eq!(resolved.sample_count(), 18);
}

#[test]
fn policy_ineligible_candidate_is_excluded() {
    let mut fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    // worker-b is the better agent on paper...
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        20,
        "src/main.ts",
    );
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    // ...but its agent-level policy forbids writes, so implement is illegal.
    let mut config = Config::load(dir).unwrap();
    config.policies.agents.insert(
        "worker-b".into(),
        PolicyScope {
            preset: Some(PolicyPreset::ReadOnly),
            ..PolicyScope::default()
        },
    );
    config.write_atomic(dir).unwrap();
    fixture.factory = Factory::open(dir).unwrap();

    let run_id = run_one_task(&fixture);
    let attempts = attempts(dir, run_id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, "worker-a");
    let decisions = worker_decisions(dir, first_task_id(dir, run_id));
    // The policy-ineligible agent never even appears as a candidate.
    assert!(decisions[0]
        .candidate_scores
        .iter()
        .all(|candidate| candidate.agent != "worker-b"));
}

#[test]
fn unavailable_agent_is_excluded() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        20,
        "src/main.ts",
    );
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: fixture.workers.clone(),
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let run = fixture
        .factory
        .begin_run("unavailable agent", Some(team))
        .unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    fixture.factory.prepare_start(run.id).unwrap();

    // The historically better agent's installation breaks between start and
    // dispatch; routing must fall to the resolvable candidate.
    let mut config = Config::load(dir).unwrap();
    config.agents.get_mut("worker-b").unwrap().command = "definitely-not-a-real-binary-xyz".into();
    config.write_atomic(dir).unwrap();
    let factory = Factory::open(dir).unwrap();
    factory
        .execute_active_run(run.id, &AtomicBool::new(false))
        .unwrap();

    let attempts = attempts(dir, run.id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, "worker-a");
    assert_eq!(attempts[0].1, AttemptStatus::Approved);
}

#[test]
fn saturated_best_agent_falls_back_to_next_candidate() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    // A large quality gap keeps worker-a ranked first even with zero free
    // slots; the reservation loop must then take worker-b.
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        40,
        "src/main.ts",
    );
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        6,
        "src/main.ts",
    );
    // Saturate worker-a's single max_concurrency slot for the dispatch.
    let _slot = fixture.factory.capacity().acquire("worker-a");
    let run_id = run_one_task(&fixture);
    let attempts = attempts(dir, run_id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, "worker-b");
    assert_eq!(attempts[0].1, AttemptStatus::Approved);
}

#[test]
fn preferred_bonus_decides_ties_but_not_gaps() {
    // Identical reliable histories: the small preferred bonus decides.
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        Some(0),
        false,
    );
    let dir = fixture.dir.path();
    for worker in ["worker-a", "worker-b"] {
        seed(
            dir,
            worker,
            "worker",
            TaskOperation::Implement,
            SeedOutcome::Approved,
            20,
            "src/main.ts",
        );
    }
    let run_id = run_one_task(&fixture);
    assert_eq!(attempts(dir, run_id)[0].0, "worker-a");

    // A real quality gap beats the bonus: worker-b is far better.
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        Some(0),
        false,
    );
    let dir = fixture.dir.path();
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Failed,
        8,
        "src/main.ts",
    );
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        25,
        "src/main.ts",
    );
    let run_id = run_one_task(&fixture);
    assert_eq!(attempts(dir, run_id)[0].0, "worker-b");
}

#[test]
fn manual_override_pins_the_agent() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        20,
        "src/main.ts",
    );
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: fixture.workers.clone(),
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let run = fixture
        .factory
        .begin_run("manual override", Some(team))
        .unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    // Pin the historically weaker agent; the override must win.
    let task_id = first_task_id(dir, run.id);
    open_db(dir)
        .set_task_agent_override(task_id, Some("worker-a"))
        .unwrap();
    fixture.factory.prepare_start(run.id).unwrap();
    fixture
        .factory
        .execute_active_run(run.id, &AtomicBool::new(false))
        .unwrap();

    let attempts = attempts(dir, run.id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, "worker-a");
    let decisions = worker_decisions(dir, task_id);
    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].reason.contains("Manual override"));
}

#[test]
fn invalid_manual_override_blocks_with_a_clear_error() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: fixture.workers.clone(),
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let run = fixture
        .factory
        .begin_run("invalid override", Some(team))
        .unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    let task_id = first_task_id(dir, run.id);
    open_db(dir)
        .set_task_agent_override(task_id, Some("reviewer-test"))
        .unwrap();
    let error = fixture.factory.prepare_start(run.id).unwrap_err();
    assert!(
        error.to_string().contains("pins agent 'reviewer-test'"),
        "unexpected error: {error}"
    );
}

#[test]
fn retry_penalty_routes_the_second_attempt_elsewhere() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::FailOnce, WorkerKind::Approve],
        Some(0), // preferred bonus wins the tied first dispatch
        false,
    );
    let dir = fixture.dir.path();
    for worker in ["worker-a", "worker-b"] {
        seed(
            dir,
            worker,
            "worker",
            TaskOperation::Implement,
            SeedOutcome::Approved,
            20,
            "src/main.ts",
        );
    }
    let run_id = run_one_task(&fixture);
    let attempts = attempts(dir, run_id);
    // Attempt 1: the preferred agent fails once. The deterministic retry
    // penalty outweighs the preferred bonus, so attempt 2 moves on.
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].0, "worker-a");
    assert_eq!(attempts[1].0, "worker-b");
    assert_eq!(attempts[1].1, AttemptStatus::Approved);
}

#[test]
fn first_pass_history_outranks_rescue_heavy_history() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    // Both agents eventually approve everything, but worker-b needs a
    // request-changes round every time: first-pass quality must rank the
    // cleaner agent first.
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::ChangesThenApproved,
        12,
        "src/main.ts",
    );
    let run_id = run_one_task(&fixture);
    let attempts = attempts(dir, run_id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, "worker-a");
}

#[test]
fn cold_start_exploration_then_exploitation() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        true,
    );
    let dir = fixture.dir.path();
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        20,
        "src/main.ts",
    );
    // worker-b has no history at all: every fifth dispatch explores it while
    // it is under-sampled; the rest exploit the known agent.
    let run_id = run_plan(&fixture, TWO_TASK_PLAN);
    let run_attempts = attempts(dir, run_id);
    let agents: Vec<&str> = run_attempts
        .iter()
        .map(|(agent, _)| agent.as_str())
        .collect();
    assert_eq!(agents, ["worker-b", "worker-a"]);
    let decisions = worker_decisions(dir, first_task_id(dir, run_id));
    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].reason.contains("Exploration"));
}

#[test]
fn exploration_can_be_disabled() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        20,
        "src/main.ts",
    );
    let run_id = run_plan(&fixture, TWO_TASK_PLAN);
    let run_attempts = attempts(dir, run_id);
    let agents: Vec<&str> = run_attempts
        .iter()
        .map(|(agent, _)| agent.as_str())
        .collect();
    assert_eq!(agents, ["worker-a", "worker-a"]);
}

#[test]
fn identical_histories_stay_deterministic() {
    // Identical reliable histories: the role's preferred assignment (the
    // explicitly flagged one, or the first declared) receives the small
    // deterministic bonus and wins — regardless of pool order.
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    for worker in ["worker-a", "worker-b"] {
        seed(
            dir,
            worker,
            "worker",
            TaskOperation::Implement,
            SeedOutcome::Approved,
            20,
            "src/main.ts",
        );
    }
    let run_id = run_one_task(&fixture);
    assert_eq!(attempts(dir, run_id)[0].0, "worker-a");

    // Same seeds, reversed team pool: the choice must not change.
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    for worker in ["worker-a", "worker-b"] {
        seed(
            dir,
            worker,
            "worker",
            TaskOperation::Implement,
            SeedOutcome::Approved,
            20,
            "src/main.ts",
        );
    }
    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-b".into(), "worker-a".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    std::fs::write(fixture.dir.path().join("plan.json"), ONE_TASK_PLAN).unwrap();
    let run = fixture
        .factory
        .begin_run("reversed pool", Some(team))
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
    assert_eq!(attempts(fixture.dir.path(), run.id)[0].0, "worker-a");
}

#[test]
fn cancelled_history_never_ranks_an_agent() {
    let fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    seed(
        dir,
        "worker-a",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    // Twelve cancellations are visible history but never qualifying quality
    // evidence: worker-b stays unranked and cannot be selected by score.
    seed(
        dir,
        "worker-b",
        "worker",
        TaskOperation::Implement,
        SeedOutcome::Cancelled,
        12,
        "src/main.ts",
    );
    let run_id = run_plan(&fixture, TWO_TASK_PLAN);
    let run_attempts = attempts(dir, run_id);
    let agents: Vec<&str> = run_attempts
        .iter()
        .map(|(agent, _)| agent.as_str())
        .collect();
    assert_eq!(agents, ["worker-a", "worker-a"]);
    let decisions = worker_decisions(dir, first_task_id(dir, run_id));
    let worker_b = decisions[0]
        .candidate_scores
        .iter()
        .find(|candidate| candidate.agent == "worker-b")
        .unwrap();
    assert!(!worker_b.reliable);
    assert!(worker_b.note.contains("insufficient data"));
}

#[test]
fn reviewer_role_routes_by_performance() {
    let mut fixture = routing_fixture(
        RoutingMode::Performance,
        &[WorkerKind::Approve, WorkerKind::Approve],
        None,
        false,
    );
    let dir = fixture.dir.path();
    // Two reviewers on the team: one reliable, one reliably bad.
    let mut config = Config::load(dir).unwrap();
    config.agents.insert(
        "reviewer-good".into(),
        command_entry(&script_for(WorkerKind::Approve, &dir.join("review.json"))),
    );
    config.agents.insert(
        "reviewer-bad".into(),
        command_entry(&script_for(WorkerKind::Approve, &dir.join("review.json"))),
    );
    config.role_assignments.push(RoleAssignment {
        role: "reviewer".into(),
        agent: "reviewer-good".into(),
        preferred: false,
    });
    config.role_assignments.push(RoleAssignment {
        role: "reviewer".into(),
        agent: "reviewer-bad".into(),
        preferred: false,
    });
    config.write_atomic(dir).unwrap();
    fixture.factory = Factory::open(dir).unwrap();
    fixture.workers = vec!["worker-a".into(), "worker-b".into()];

    seed(
        dir,
        "reviewer-good",
        "reviewer",
        TaskOperation::Review,
        SeedOutcome::Approved,
        12,
        "src/main.ts",
    );
    seed(
        dir,
        "reviewer-bad",
        "reviewer",
        TaskOperation::Review,
        SeedOutcome::Failed,
        12,
        "src/main.ts",
    );

    let team = WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into()],
        reviewers: vec!["reviewer-bad".into(), "reviewer-good".into()],
        additional: BTreeMap::new(),
    };
    std::fs::write(dir.join("plan.json"), ONE_TASK_PLAN).unwrap();
    let run = fixture
        .factory
        .begin_run("reviewer routing", Some(team))
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

    assert_eq!(attempts(dir, run.id)[0].1, AttemptStatus::Approved);
    let review_decisions: Vec<RoutingDecision> = open_db(dir)
        .list_routing_decisions_for_task(first_task_id(dir, run.id))
        .unwrap()
        .into_iter()
        .filter(|decision| decision.role.as_deref() == Some("reviewer"))
        .collect();
    assert_eq!(review_decisions.len(), 1);
    assert_eq!(review_decisions[0].selected_agent, "reviewer-good");
}

#[test]
fn routing_config_parses_and_defaults_to_round_robin() {
    let default: Config = toml::from_str("").unwrap();
    assert_eq!(default.routing.mode, RoutingMode::RoundRobin);
    assert!(default.routing.exploration);

    let configured: Config =
        toml::from_str("[routing]\nmode = \"performance\"\nexploration = false\n").unwrap();
    assert_eq!(configured.routing.mode, RoutingMode::Performance);
    assert!(!configured.routing.exploration);

    let manual: Config = toml::from_str("[routing]\nmode = \"manual\"\n").unwrap();
    assert_eq!(manual.routing.mode, RoutingMode::Manual);

    assert!(toml::from_str::<Config>("[routing]\nmode = \"telepathic\"\n").is_err());
}

#[test]
fn manual_mode_uses_the_preferred_agent() {
    let fixture = routing_fixture(
        RoutingMode::Manual,
        &[WorkerKind::Approve, WorkerKind::Approve],
        Some(1),
        false,
    );
    let run_id = run_one_task(&fixture);
    let attempts = attempts(fixture.dir.path(), run_id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, "worker-b");
    let decisions = worker_decisions(
        fixture.dir.path(),
        first_task_id(fixture.dir.path(), run_id),
    );
    assert!(decisions[0].reason.contains("preferred agent"));
}
