#[path = "support/worker_barrier.rs"]
mod worker_barrier;
use worker_barrier::WorkerBarrier as StressBarrier;

use factory_core::{AgentEntry, Config, Factory, ProjectExecution, TaskBudget, WorkflowResult};
use factory_types::{Plan, PlannedTask, RunStatus, SchedulerPolicy, TaskState};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8_lossy(&result.stdout).trim().into()
}

fn python() -> String {
    ["python", "python3"]
        .into_iter()
        .find(|program| {
            Command::new(program)
                .arg("--version")
                .output()
                .is_ok_and(|result| result.status.success())
        })
        .expect("Python 3 is required for controlled scheduler integration fixtures")
        .into()
}

const AGENT: &str = r#"
import json,os,pathlib,subprocess,sys,time
mode=sys.argv[1]
text=sys.stdin.read()
if mode=='auto': mode='review' if text.startswith('ROLE\nReviewer') else 'worker'
if mode=='review':
    if os.environ.get('ASF_TEST_REVIEW_MUTATE'): pathlib.Path('tampered.py').write_text('unreviewed')
    print(json.dumps({'decision':'approve','reason':'controlled fixture inspected captured evidence','feedback':[]}))
elif mode=='planner':
    if 'REPLAN_UNFINISHED' in text:
        objective='WRITE semanticRepair' if os.environ.get('ASF_TEST_SEMANTIC_REPAIR') else 'WRITE repair'
        print(json.dumps({'objective':'repair','tasks':[{'id':'repair','title':'Repair obstacle','objective':objective,'dependencies':[],'acceptanceCriteria':['correct repair'], 'role':'worker'}]}))
    else: raise RuntimeError('authored fixture does not use initial planner')
else:
    task=text.split('\nTASK\n')[1].split('\nOPERATION\n')[0]
    if 'QUOTA' in task:
        sys.stderr.write("OpenAI Codex v0.0.0-controlled-fixture\nERROR: You've hit your usage limit. Try again later.\n");sys.exit(1)
    if 'DENIED' in task:
        print(json.dumps({'permissionBlocker':{'code':'POLICY_DENIED','reason':'controlled restriction'},'commands':[]}));sys.exit(0)
    if 'FAIL' in task: sys.exit(9)
    if 'LONG' in task: time.sleep(30)
    time.sleep(0.25)
    name=task.split('WRITE ')[1].split()[0]
    barrier=pathlib.Path(__file__).with_name('stress-barrier-address')
    if barrier.exists():
        import socket
        with socket.create_connection(('127.0.0.1',int(barrier.read_text())),timeout=15) as connection:
            assert connection.recv(1)==b'G'
    if name=='semanticRepair':
        pathlib.Path('part_A.py').write_text('VALUE = 2\n')
        print(json.dumps({'commands':[], 'summary':'controlled semantic repair', 'tests':[]}))
        sys.exit(0)
    if name=='dependent':
        facts=json.loads(text.split('ASF_DEPENDENCY_FACTS\n')[1].splitlines()[0])
        assert len(facts)==2 and all(item['state']=='completed' and item['review']=='approve' and item['publishedCommit'] for item in facts)
        assert pathlib.Path('part_A.py').read_text()=='VALUE = 1\n'
        assert pathlib.Path('part_B.py').read_text()=='VALUE = 1\n'
    target='shared.py' if name in ('conflictA','conflictB') else 'part_'+name+'.py'
    pathlib.Path(target).write_text('VALUE = '+('2' if name=='conflictB' else '1')+'\n')
    print(json.dumps({'commands':[], 'summary':'controlled source modification', 'tests':[]}))
"#;

fn fixture(width: usize, agent_limit: usize, global: Arc<TaskBudget>) -> (TempDir, Factory) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".gitignore"), ".factory/\n__pycache__/\n").unwrap();
    std::fs::write(dir.path().join("base.py"), "VALUE = 0\n").unwrap();
    std::fs::write(dir.path().join("shared.py"), "VALUE = 0\n").unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Scheduler Test"]);
    git(
        dir.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "baseline"]);
    Factory::init(dir.path()).unwrap();
    let script = dir.path().join(".factory/controlled.py");
    std::fs::write(&script, AGENT).unwrap();
    let mut config = Config::default();
    for (role, mode) in [
        ("planner", "planner"),
        ("worker", "worker"),
        ("reviewer", "review"),
    ] {
        config.agents.insert(
            role.into(),
            AgentEntry {
                kind: None,
                command: python(),
                args: vec![script.to_string_lossy().into_owned(), mode.into()],
                env: BTreeMap::from([("PYTHONDONTWRITEBYTECODE".into(), "1".into())]),
                prompt_transport: Some(factory_agent::PromptTransport::Stdin),
                interactive_args: None,
                capabilities: vec![],
                max_concurrency: Some(agent_limit),
            },
        );
        config.role_assignments.push(factory_core::RoleAssignment {
            role: role.into(),
            agent: role.into(),
            preferred: true,
        });
    }
    config.runtime.max_parallel_tasks = width;
    config.write_atomic(dir.path()).unwrap();
    let context = Arc::new(ProjectExecution::new(
        dir.path().to_string_lossy().into_owned(),
        global,
    ));
    let factory = Factory::open_with_execution(dir.path(), context).unwrap();
    (dir, factory)
}

fn task(id: &str, objective: &str, deps: Vec<&str>) -> PlannedTask {
    PlannedTask {
        id: id.into(),
        title: id.into(),
        objective: objective.into(),
        dependencies: deps.into_iter().map(Into::into).collect(),
        acceptance_criteria: vec!["real fixture changes".into()],
        role: None,
        operation: None,
    }
}
fn plan(factory: &Factory, tasks: Vec<PlannedTask>, width: usize, replans: u32) -> i64 {
    let run = factory.begin_run("scheduler fixture", None).unwrap();
    factory
        .persist_authored_plan(
            run.id,
            &Plan {
                objective: run.objective,
                tasks,
            },
        )
        .unwrap();
    factory
        .configure_run_scheduler(
            run.id,
            SchedulerPolicy {
                max_parallel_tasks: width,
                max_project_tasks: width,
                max_replans: replans,
                ..Default::default()
            },
        )
        .unwrap();
    factory.prepare_start(run.id).unwrap();
    run.id
}
fn worker_intervals(factory: &Factory, run: i64) -> Vec<(String, String)> {
    factory
        .list_agent_sessions(Some(run))
        .unwrap()
        .into_iter()
        .filter(|session| session.role == "worker")
        .map(|session| (session.started_at, session.finished_at.unwrap()))
        .collect()
}
fn peak(intervals: &[(String, String)]) -> usize {
    intervals
        .iter()
        .map(|(start, _)| {
            intervals
                .iter()
                .filter(|(a, b)| a <= start && b > start)
                .count()
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn siblings_overlap_and_dependent_base_contains_both_published_commits() {
    let budget = Arc::new(TaskBudget::new(4));
    let (dir, factory) = fixture(2, 2, budget.clone());
    let barrier = StressBarrier::new(3);
    std::fs::write(
        dir.path().join(".factory/stress-barrier-address"),
        barrier.port.to_string(),
    )
    .unwrap();
    let run = plan(
        &factory,
        vec![
            task("A", "WRITE A", vec![]),
            task("B", "WRITE B", vec![]),
            task("C", "WRITE dependent", vec!["A", "B"]),
        ],
        2,
        0,
    );
    assert_eq!(
        factory
            .execute_active_run(run, &AtomicBool::new(false))
            .unwrap(),
        WorkflowResult::Completed
    );
    assert_eq!(peak(&worker_intervals(&factory, run)), 2);
    let tasks = factory.list_tasks(run).unwrap();
    let attempts: Vec<_> = tasks
        .iter()
        .map(|task| {
            factory
                .list_task_attempts(run)
                .unwrap()
                .into_iter()
                .find(|attempt| attempt.task_id == task.id)
                .unwrap()
        })
        .collect();
    assert_ne!(attempts[0].worktree_path, attempts[1].worktree_path);
    let dependent_base = attempts[2].source_base.as_ref().unwrap();
    assert_eq!(
        git(
            dir.path(),
            &["show", &format!("{dependent_base}:part_A.py")]
        ),
        "VALUE = 1"
    );
    assert_eq!(
        git(
            dir.path(),
            &["show", &format!("{dependent_base}:part_B.py")]
        ),
        "VALUE = 1"
    );
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
    assert_eq!(git(dir.path(), &["branch", "--show-current"]), "main");
    assert_eq!(budget.usage(), 0);
}

#[test]
fn width_one_and_agent_one_are_real_sequential_fallbacks() {
    for (width, limit) in [(1, 4), (4, 1)] {
        let budget = Arc::new(TaskBudget::new(8));
        let (_dir, factory) = fixture(width, limit, budget.clone());
        let run = plan(
            &factory,
            (0..4)
                .map(|i| task(&format!("T{i}"), &format!("WRITE p{i}"), vec![]))
                .collect(),
            width,
            0,
        );
        assert_eq!(
            factory
                .execute_active_run(run, &AtomicBool::new(false))
                .unwrap(),
            WorkflowResult::Completed
        );
        assert_eq!(peak(&worker_intervals(&factory, run)), 1);
        assert_eq!(budget.usage(), 0);
    }
}

#[test]
fn integration_conflict_preserves_outputs_and_is_not_silent_overwrite() {
    let (_dir, factory) = fixture(2, 2, Arc::new(TaskBudget::new(4)));
    let run = plan(
        &factory,
        vec![
            task("A", "WRITE conflictA", vec![]),
            task("B", "WRITE conflictB", vec![]),
        ],
        2,
        0,
    );
    let error = factory
        .execute_active_run(run, &AtomicBool::new(false))
        .unwrap_err();
    assert!(matches!(error, factory_core::FactoryError::Git(_)));
    let state = factory.scheduling_state(run).unwrap();
    let records = state["integrations"].as_array().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| record["status"] == "published")
            .count(),
        1
    );
    let conflict = records
        .iter()
        .find(|record| record["status"] == "conflict")
        .unwrap();
    assert!(Path::new(conflict["worktree_path"].as_str().unwrap()).exists());
    assert_eq!(
        factory
            .list_tasks(run)
            .unwrap()
            .iter()
            .filter(|task| task.state == TaskState::Completed)
            .count(),
        1
    );
}

#[test]
fn obstacle_invokes_real_configured_planner_and_preserves_completed_rows() {
    let (_dir, factory) = fixture(2, 2, Arc::new(TaskBudget::new(4)));
    let run = plan(
        &factory,
        vec![
            task("A", "WRITE A", vec![]),
            task("fail", "FAIL", vec!["A"]),
        ],
        2,
        1,
    );
    assert_eq!(
        factory
            .execute_active_run(run, &AtomicBool::new(false))
            .unwrap(),
        WorkflowResult::Completed
    );
    let state = factory.scheduling_state(run).unwrap();
    let revisions = state["revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 2);
    assert!(factory
        .list_tasks(run)
        .unwrap()
        .iter()
        .any(|task| task.state == TaskState::Superseded));
    assert_eq!(
        factory
            .list_agent_sessions(Some(run))
            .unwrap()
            .iter()
            .filter(|session| session.role == "planner")
            .count(),
        1
    );
    assert_eq!(
        factory.get_run(run).unwrap().unwrap().status,
        RunStatus::Completed
    );
}

#[test]
fn semantic_validation_failure_preserves_candidate_then_reviews_repair_before_publication() {
    let budget = Arc::new(TaskBudget::new(1));
    let (dir, _) = fixture(1, 1, budget.clone());
    let mut config = Config::load(dir.path()).unwrap();
    config
        .agents
        .get_mut("planner")
        .unwrap()
        .env
        .insert("ASF_TEST_SEMANTIC_REPAIR".into(), "1".into());
    config
        .integration_checks
        .push(factory_core::execution::ValidationCommand {
            program: python(),
            args: vec![
                "-B".into(),
                "-c".into(),
                "import runpy; assert runpy.run_path('part_A.py')['VALUE'] == 2, 'semantic value must be two'".into(),
            ],
            timeout_secs: 15,
        });
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open_with_execution(
        dir.path(),
        Arc::new(ProjectExecution::new(
            dir.path().to_string_lossy().into_owned(),
            budget.clone(),
        )),
    )
    .unwrap();
    let run = plan(&factory, vec![task("A", "WRITE A", vec![])], 1, 1);
    assert_eq!(
        factory
            .execute_active_run(run, &AtomicBool::new(false))
            .unwrap(),
        WorkflowResult::Completed
    );
    let state = factory.scheduling_state(run).unwrap();
    assert_eq!(state["revisions"].as_array().unwrap().len(), 2);
    let records = state["integrations"].as_array().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["status"], "validation_failed");
    let rejected = Path::new(records[0]["worktree_path"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(rejected.join("part_A.py"))
            .unwrap()
            .trim(),
        "VALUE = 1"
    );
    assert_eq!(records[1]["status"], "published");
    assert_eq!(
        git(dir.path(), &["show", "factory/run-1:part_A.py"]),
        "VALUE = 2"
    );
    let tasks = factory.list_tasks(run).unwrap();
    assert_eq!(tasks[0].state, TaskState::Superseded);
    assert_eq!(tasks[1].state, TaskState::Completed);
    let attempts = factory.list_task_attempts(run).unwrap();
    assert_eq!(attempts.len(), 2);
    for attempt in &attempts {
        assert_eq!(attempt.status, factory_types::AttemptStatus::Approved);
        assert_eq!(
            attempt.review.as_ref().unwrap().decision,
            factory_types::ReviewDecision::Approve
        );
        assert!(attempt.evidence.as_ref().unwrap().diff_patch.is_some());
    }
    assert!(state["events"].as_array().unwrap().iter().any(|event| {
        event["event_type"] == "integration_validation" && event["payload"]["exitCode"] == 1
    }));
    assert_eq!(state["activeClaims"], 0);
    assert_eq!(budget.usage(), 0);
}

#[test]
fn cancellation_drains_parallel_processes_and_releases_capacity() {
    let budget = Arc::new(TaskBudget::new(2));
    let (_dir, factory) = fixture(2, 2, budget.clone());
    let run = plan(
        &factory,
        vec![
            task("A", "LONG WRITE A", vec![]),
            task("B", "LONG WRITE B", vec![]),
        ],
        2,
        0,
    );
    let cancel = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let flag = &cancel;
        scope.spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            flag.store(true, Ordering::Release);
        });
        factory.execute_active_run(run, &cancel)
    })
    .unwrap();
    assert_eq!(result, WorkflowResult::Cancelled);
    assert_eq!(budget.usage(), 0);
    assert!(factory
        .list_agent_sessions(Some(run))
        .unwrap()
        .iter()
        .all(|session| session.finished_at.is_some()));
    assert_eq!(factory.scheduling_state(run).unwrap()["activeClaims"], 0);
}

#[test]
fn legacy_runs_have_no_new_deadline_and_explicit_deadlines_drain_processes() {
    let budget = Arc::new(TaskBudget::new(1));
    let (_dir, factory) = fixture(1, 1, budget.clone());
    let legacy = factory.begin_run("legacy compatibility", None).unwrap();
    factory
        .persist_authored_plan(
            legacy.id,
            &Plan {
                objective: legacy.objective,
                tasks: vec![task("legacy", "WRITE legacy", vec![])],
            },
        )
        .unwrap();
    factory.prepare_start(legacy.id).unwrap();
    assert_eq!(
        factory
            .scheduling_state(legacy.id)
            .unwrap()
            .get("deadlineSeconds"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(
        factory
            .execute_active_run(legacy.id, &AtomicBool::new(false))
            .unwrap(),
        WorkflowResult::Completed
    );
    let run = plan(&factory, vec![task("A", "LONG WRITE A", vec![])], 1, 0);
    factory
        .configure_run_scheduler(
            run,
            SchedulerPolicy {
                max_parallel_tasks: 1,
                max_project_tasks: 1,
                max_replans: 0,
                max_duration_secs: 15,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(
        factory.execute_active_run(run, &AtomicBool::new(false)),
        Err(factory_core::FactoryError::SchedulerLimit(_))
    ));
    let sessions = factory.list_agent_sessions(Some(run)).unwrap();
    assert!(sessions.iter().any(|session| session.role == "worker"
        && session
            .output_meta
            .as_ref()
            .is_some_and(|meta| meta.process_id.is_some())));
    assert!(sessions.iter().all(|session| session.finished_at.is_some()));
    let state = factory.scheduling_state(run).unwrap();
    assert_eq!(state["deadlineSeconds"], 15);
    assert_eq!(state["activeClaims"], 0);
    assert_eq!(state["ownerActive"], false);
    assert_eq!(budget.usage(), 0);
}

#[test]
fn policy_setup_failure_releases_ownership_without_launching_agents() {
    let (dir, factory) = fixture(1, 1, Arc::new(TaskBudget::new(1)));
    let run = plan(&factory, vec![task("A", "WRITE A", vec![])], 1, 0);
    let database = dir.path().join(".factory/db.sqlite3");
    let corrupt = Command::new(python())
        .args([
            "-c",
            "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute(\"UPDATE scheduler_policies SET policy='{'\"); c.commit()",
        ])
        .arg(&database)
        .output()
        .unwrap();
    assert!(corrupt.status.success());
    assert!(factory
        .execute_active_run(run, &AtomicBool::new(false))
        .is_err());
    assert!(factory.list_agent_sessions(Some(run)).unwrap().is_empty());
    let inspected = Command::new(python())
        .args([
            "-c",
            "import json,sqlite3,sys; c=sqlite3.connect(sys.argv[1]); print(json.dumps(c.execute('SELECT COUNT(*),SUM(finished_at IS NULL) FROM scheduler_owners').fetchone()))",
        ])
        .arg(database)
        .output()
        .unwrap();
    assert!(inspected.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&inspected.stdout).unwrap(),
        serde_json::json!([1, 0])
    );
}

#[test]
fn global_capacity_is_shared_across_projects_and_numeric_collisions() {
    let budget = Arc::new(TaskBudget::new(2));
    let (_a, factory_a) = fixture(4, 4, budget.clone());
    let (_b, factory_b) = fixture(4, 4, budget.clone());
    let make = |factory: &Factory| {
        plan(
            factory,
            (0..4)
                .map(|i| task(&format!("T{i}"), &format!("WRITE f{i}"), vec![]))
                .collect(),
            4,
            0,
        )
    };
    let a = make(&factory_a);
    let b = make(&factory_b);
    assert_eq!(a, b);
    let first = std::thread::spawn(move || {
        factory_a
            .execute_active_run(a, &AtomicBool::new(false))
            .unwrap();
        worker_intervals(&factory_a, a)
    });
    let second = std::thread::spawn(move || {
        factory_b
            .execute_active_run(b, &AtomicBool::new(false))
            .unwrap();
        worker_intervals(&factory_b, b)
    });
    let mut intervals = first.join().unwrap();
    intervals.extend(second.join().unwrap());
    assert!(peak(&intervals) <= 2);
    assert_eq!(intervals.len(), 8);
    assert_eq!(budget.usage(), 0);
}

#[test]
fn bounded_stress_two_four_eight_has_no_duplicate_attempts_or_capacity_leaks() {
    for width in [2, 4, 8] {
        for iteration in 0..2 {
            let budget = Arc::new(TaskBudget::new(width));
            let (dir, factory) = fixture(width, width, budget.clone());
            let barrier = StressBarrier::new(width + 2);
            std::fs::write(
                dir.path().join(".factory/stress-barrier-address"),
                barrier.port.to_string(),
            )
            .unwrap();
            let run = plan(
                &factory,
                (0..width + 2)
                    .map(|i| task(&format!("S{i}"), &format!("WRITE stress{i}"), vec![]))
                    .collect(),
                width,
                0,
            );
            let start = std::time::Instant::now();
            let result = factory.execute_active_run(run, &AtomicBool::new(false));
            if !matches!(result, Ok(WorkflowResult::Completed)) {
                let state = factory.scheduling_state(run).unwrap();
                let retained = dir.keep();
                panic!(
                    "Stress width {width} iteration {iteration}: {result:?}; preserved {}\n{state}",
                    retained.display()
                );
            }
            let intervals = worker_intervals(&factory, run);
            assert!(
                peak(&intervals) >= 2 && peak(&intervals) <= width,
                "width {width}, observed {}",
                peak(&intervals)
            );
            assert_eq!(intervals.len(), width + 2);
            assert_eq!(factory.list_task_attempts(run).unwrap().len(), width + 2);
            let state = factory.scheduling_state(run).unwrap();
            assert_eq!(state["activeClaims"], 0);
            assert_eq!(budget.usage(), 0);
            assert_eq!(
                state["integrations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|record| record["status"] == "published")
                    .count(),
                width + 2
            );
            println!("P08_CONTROLLED_METRIC width={width} iteration={iteration} tasks={} peak={} elapsed_ms={}",width+2,peak(&intervals),start.elapsed().as_millis());
        }
    }
}

#[test]
fn deterministic_policy_denial_does_not_invoke_recovery_planner() {
    let budget = Arc::new(TaskBudget::new(2));
    let (_dir, factory) = fixture(2, 2, budget.clone());
    let run = plan(
        &factory,
        vec![task("denied", "DENIED WRITE bad", vec![])],
        2,
        2,
    );
    assert!(factory
        .execute_active_run(run, &AtomicBool::new(false))
        .is_err());
    assert_eq!(factory.list_task_attempts(run).unwrap().len(), 1);
    assert_eq!(
        factory
            .list_agent_sessions(Some(run))
            .unwrap()
            .iter()
            .filter(|session| session.role == "planner" || session.role == "reviewer")
            .count(),
        0
    );
    assert_eq!(
        factory.get_run(run).unwrap().unwrap().status,
        RunStatus::Blocked
    );
    assert_eq!(budget.usage(), 0);
}

#[test]
fn exhausted_cli_capacity_stops_without_retries_or_a_recovery_planner() {
    let budget = Arc::new(TaskBudget::new(2));
    let (dir, factory) = fixture(2, 2, budget.clone());
    let mut config = Config::load(dir.path()).unwrap();
    config.agents.get_mut("worker").unwrap().kind = Some(factory_agent::AgentKind::Codex);
    config.write_atomic(dir.path()).unwrap();
    drop(factory);
    let factory = Factory::open_with_execution(
        dir.path(),
        Arc::new(ProjectExecution::new(
            dir.path().to_string_lossy().into_owned(),
            budget.clone(),
        )),
    )
    .unwrap();
    let run = plan(
        &factory,
        vec![
            task("quota", "QUOTA WRITE unavailable", vec![]),
            task("long", "LONG WRITE collateral", vec![]),
        ],
        2,
        2,
    );
    let result = factory.execute_active_run(run, &AtomicBool::new(false));
    assert!(
        matches!(
            result,
            Err(factory_core::FactoryError::AgentResourceExhausted(_))
        ),
        "{result:?}"
    );
    let attempts = factory.list_task_attempts(run).unwrap();
    let unavailable = attempts
        .iter()
        .filter(|attempt| {
            attempt
                .error
                .as_deref()
                .is_some_and(|error| error.contains("AGENT_RESOURCE_EXHAUSTED"))
        })
        .count();
    assert_eq!(unavailable, 1);
    assert!(factory
        .list_agent_sessions(Some(run))
        .unwrap()
        .iter()
        .all(|session| session.role == "worker"));
    assert_eq!(
        factory.get_run(run).unwrap().unwrap().status,
        RunStatus::Blocked
    );
    assert_eq!(factory.scheduling_state(run).unwrap()["activeClaims"], 0);
    assert_eq!(budget.usage(), 0);
}

#[test]
fn interrupted_attempts_are_not_blindly_replayed() {
    let (dir, factory) = fixture(2, 2, Arc::new(TaskBudget::new(2)));
    let run = plan(&factory, vec![task("A", "WRITE A", vec![])], 2, 0);
    let task = factory.list_tasks(run).unwrap()[0].id;
    let path = factory.create_worktree(task).unwrap();
    drop(factory);
    let db = factory_db::FactoryDb::open(&dir.path().join(".factory/db.sqlite3")).unwrap();
    db.create_task_attempt(
        task,
        "worker",
        Some(factory_types::TaskOperation::Implement),
        "worker",
        &path.to_string_lossy(),
        Some("captured-base"),
    )
    .unwrap();
    db.set_task_state(task, TaskState::Running).unwrap();
    drop(db);
    let factory = Factory::open(dir.path()).unwrap();
    factory
        .reconcile_interrupted_preserving_worktrees()
        .unwrap();
    assert_eq!(
        factory.list_task_attempts(run).unwrap()[0].status,
        factory_types::AttemptStatus::Interrupted
    );
    assert!(factory
        .prepare_retry(task)
        .unwrap_err()
        .to_string()
        .contains("ownership is uncertain"));
    assert!(path.exists());
}

#[test]
fn duplicate_scheduler_control_cannot_fail_the_actual_owner() {
    let (dir, factory) = fixture(2, 2, Arc::new(TaskBudget::new(2)));
    let run = plan(&factory, vec![task("A", "WRITE A", vec![])], 2, 0);
    let db = factory_db::FactoryDb::open(&dir.path().join(".factory/db.sqlite3")).unwrap();
    db.acquire_scheduler(run, "legitimate-owner").unwrap();
    assert!(matches!(
        factory.execute_active_run(run, &AtomicBool::new(false)),
        Err(factory_core::FactoryError::CapacityBusy(_))
    ));
    assert_eq!(
        factory.get_run(run).unwrap().unwrap().status,
        RunStatus::Active
    );
    assert!(factory.list_agent_sessions(Some(run)).unwrap().is_empty());
    db.finish_scheduler("legitimate-owner", "finished").unwrap();
}

#[test]
fn crash_after_ref_publication_reconciles_without_replaying_git_or_worker() {
    let (dir, factory) = fixture(2, 2, Arc::new(TaskBudget::new(2)));
    let run = plan(
        &factory,
        vec![
            task("A", "WRITE A", vec![]),
            task("B", "WRITE B", vec!["A"]),
        ],
        2,
        0,
    );
    let tasks = factory.list_tasks(run).unwrap();
    let path = factory.create_worktree(tasks[0].id).unwrap();
    let repo = factory_git::Repo::detect_bounded(dir.path(), dir.path()).unwrap();
    let before = repo.head_sha(&path).unwrap();
    std::fs::write(path.join("published.py"), "VALUE = 1\n").unwrap();
    repo.commit_changes(
        &path,
        "controlled reviewed output",
        ("Fixture", "test@example.invalid"),
    )
    .unwrap();
    let source = repo.head_sha(&path).unwrap();
    let db = factory_db::FactoryDb::open(&dir.path().join(".factory/db.sqlite3")).unwrap();
    let attempt = db
        .create_task_attempt(
            tasks[0].id,
            "worker",
            Some(factory_types::TaskOperation::Implement),
            "worker",
            &path.to_string_lossy(),
            Some(&before),
        )
        .unwrap();
    db.finish_task_attempt(
        attempt.id,
        factory_types::AttemptStatus::Approved,
        Some(0),
        Some(&source),
        None,
        Some(&factory_types::TaskEvidence::default()),
        Some(&factory_types::ReviewResult {
            decision: factory_types::ReviewDecision::Approve,
            reason: "controlled approval".into(),
            feedback: vec![],
        }),
    )
    .unwrap();
    db.set_task_state(tasks[0].id, TaskState::Integrating)
        .unwrap();
    db.set_run_status(run, RunStatus::Active).unwrap();
    db.acquire_scheduler(run, "crashed-owner").unwrap();
    let record = db
        .begin_integration(
            run,
            tasks[0].id,
            attempt.id,
            &before,
            &source,
            &path.to_string_lossy(),
        )
        .unwrap();
    db.update_integration_record(
        record,
        "publishing",
        Some(&source),
        &serde_json::json!({"checks":[]}),
    )
    .unwrap();
    repo.compare_update_ref(&format!("factory/run-{run}"), &source, None)
        .unwrap();
    factory
        .reconcile_interrupted_preserving_worktrees()
        .unwrap();
    assert_eq!(db.integration_records(run).unwrap()[0].status, "published");
    assert_eq!(
        db.get_run_integration(run).unwrap().as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        factory.list_tasks(run).unwrap()[0].state,
        TaskState::Completed
    );
    assert_eq!(factory.list_tasks(run).unwrap()[1].state, TaskState::Ready);
    assert!(factory.list_agent_sessions(Some(run)).unwrap().is_empty());
    factory
        .reconcile_interrupted_preserving_worktrees()
        .unwrap();
    assert_eq!(
        repo.resolve_ref(&format!("factory/run-{run}")).unwrap(),
        source
    );
    assert_eq!(db.integration_records(run).unwrap().len(), 1);
}

#[test]
fn reviewer_cannot_change_the_captured_source_and_approve_it() {
    let (dir, _) = fixture(2, 2, Arc::new(TaskBudget::new(2)));
    let mut config = Config::load(dir.path()).unwrap();
    config
        .agents
        .get_mut("reviewer")
        .unwrap()
        .env
        .insert("ASF_TEST_REVIEW_MUTATE".into(), "1".into());
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    let run = plan(&factory, vec![task("A", "WRITE A", vec![])], 2, 1);
    assert!(matches!(
        factory.execute_active_run(run, &AtomicBool::new(false)),
        Err(factory_core::FactoryError::PolicyViolation(_))
    ));
    assert_eq!(
        factory.list_task_attempts(run).unwrap()[0].status,
        factory_types::AttemptStatus::Failed
    );
    assert!(factory.scheduling_state(run).unwrap()["integrations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        git(dir.path(), &["show", "factory/run-1:base.py"]),
        "VALUE = 0"
    );
}

#[test]
fn checks_cannot_publish_dirty_or_committed_changes_after_review() {
    for commit in [false, true] {
        let (dir, _) = fixture(1, 1, Arc::new(TaskBudget::new(1)));
        let mut config = Config::load(dir.path()).unwrap();
        let code = if commit {
            "from pathlib import Path; import subprocess; Path('tampered.py').write_text('unexpected'); subprocess.run(['git','add','tampered.py'],check=True); subprocess.run(['git','commit','-qm','unreviewed check mutation'],check=True)"
        } else {
            "from pathlib import Path; Path('tampered.py').write_text('unexpected')"
        };
        config
            .integration_checks
            .push(factory_core::execution::ValidationCommand {
                program: python(),
                args: vec!["-c".into(), code.into()],
                timeout_secs: 15,
            });
        config.write_atomic(dir.path()).unwrap();
        let factory = Factory::open(dir.path()).unwrap();
        let run = plan(&factory, vec![task("A", "WRITE A", vec![])], 1, 1);
        assert!(matches!(
            factory.execute_active_run(run, &AtomicBool::new(false)),
            Err(factory_core::FactoryError::PolicyViolation(_))
        ));
        let state = factory.scheduling_state(run).unwrap();
        assert_eq!(state["integrations"][0]["status"], "policy_blocked");
        assert_eq!(
            git(dir.path(), &["show", "factory/run-1:base.py"]),
            "VALUE = 0"
        );
        assert!(run_command_missing_file(
            dir.path(),
            "factory/run-1:tampered.py"
        ));
    }
}
fn run_command_missing_file(root: &Path, reference: &str) -> bool {
    !Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "-e", reference])
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn one_agent_shared_between_worker_and_reviewer_does_not_deadlock() {
    let (dir, _) = fixture(2, 1, Arc::new(TaskBudget::new(2)));
    let mut config = Config::load(dir.path()).unwrap();
    config.agents.get_mut("worker").unwrap().args[1] = "auto".into();
    for assignment in &mut config.role_assignments {
        if assignment.role == "reviewer" {
            assignment.agent = "worker".into();
        }
    }
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    let run = plan(
        &factory,
        vec![task("A", "WRITE A", vec![]), task("B", "WRITE B", vec![])],
        2,
        0,
    );
    assert_eq!(
        factory
            .execute_active_run(run, &AtomicBool::new(false))
            .unwrap(),
        WorkflowResult::Completed
    );
    assert_eq!(factory.capacity().inflight("worker"), 0);
}
