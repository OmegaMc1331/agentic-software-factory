use factory_core::{AgentEntry, Config, Factory};
use factory_db::FactoryDb;
use factory_runtime::Runtime;
use factory_types::{
    AttemptStatus, MissionLifecycleState, Plan, PlannedTask, RunStatus, TaskState,
};
use std::{collections::BTreeMap, path::Path, process::Command, time::Duration};

const PROJECT: &str = "p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SESSION: &str = "s_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_rejects_a_conversation_permit_reserved_before_admission_closed() {
    let (temp, runtime, _) = fixture();
    let factory = runtime.factory().unwrap();
    let run = factory
        .begin_run("Late accepted conversation", None)
        .unwrap();
    let db = database(temp.path());
    db.bind_workspace_run(run.id, PROJECT, SESSION).unwrap();
    let permit = runtime.reserve_operation().unwrap();
    runtime.shutdown().unwrap();
    let plan = Plan {
        objective: "Late accepted conversation".into(),
        tasks: vec![PlannedTask {
            id: "late".into(),
            title: "Late task".into(),
            objective: "FAST_COMPLETE must never execute after shutdown".into(),
            dependencies: vec![],
            acceptance_criteria: vec!["no post-shutdown dispatch".into()],
            role: None,
            operation: None,
        }],
    };
    assert!(matches!(
        runtime.execute_conversation_mission(run.id, Some(&plan), permit),
        Err(factory_runtime::RuntimeError::ResourceLimit(
            "application shutting down"
        ))
    ));
    assert!(runtime.is_idle());
    assert!(db.list_agent_sessions(Some(run.id)).unwrap().is_empty());
    assert!(db.list_task_attempts(run.id).unwrap().is_empty());
    assert_eq!(
        db.mission_lifecycle(run.id).unwrap().unwrap().state,
        MissionLifecycleState::Suspended
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_retry_reserves_safely_settled_failure_without_replaying_history() {
    let (temp, runtime, run) = fixture();
    let mut config = Config::load_and_migrate(temp.path()).unwrap();
    config
        .agents
        .get_mut("worker")
        .unwrap()
        .env
        .insert("ASF_TEST_REFUSED_WORKER".into(), "1".into());
    config.scheduler.max_replans = 0;
    config.policies.roles.insert(
        "worker".into(),
        serde_json::from_value(serde_json::json!({"commands":{"mode":"restricted","allow":["python","git"],"deny":["curl"]}})).unwrap(),
    );
    config.write_atomic(temp.path()).unwrap();
    runtime.start_workflow(run).unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Failed
            && db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Settled
            && runtime.active_operations().is_empty()
    })
    .await;
    let db = database(temp.path());
    let original = db.list_task_attempts(run).unwrap().remove(0);
    assert_eq!(original.status, AttemptStatus::Failed);
    assert!(runtime.start_workflow(run).is_err());
    db.set_workspace_session_closed(PROJECT, SESSION, 1, true)
        .unwrap();
    assert!(runtime.retry_task(original.task_id).is_err());
    assert!(runtime.active_operations().is_empty());
    db.set_workspace_session_closed(PROJECT, SESSION, 2, false)
        .unwrap();
    config
        .agents
        .get_mut("worker")
        .unwrap()
        .env
        .remove("ASF_TEST_REFUSED_WORKER");
    config
        .agents
        .get_mut("worker")
        .unwrap()
        .env
        .insert("ASF_TEST_FAST_WORKER".into(), "1".into());
    config.write_atomic(temp.path()).unwrap();
    assert_eq!(runtime.retry_task(original.task_id).unwrap(), run);
    assert!(runtime.retry_task(original.task_id).is_err());
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
            && runtime.active_operations().is_empty()
    })
    .await;
    assert_eq!(db.get_task_attempt(original.id).unwrap().unwrap(), original);
    assert_eq!(db.list_task_attempts(run).unwrap().len(), 3);
    assert_eq!(
        db.integration_records(run)
            .unwrap()
            .iter()
            .filter(|r| r.status == "published")
            .count(),
        2
    );
    assert_eq!(db.active_dispatch_count(run).unwrap(), 0);
    assert_eq!(db.mission_lifecycle(run).unwrap().unwrap().generation, 2);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p11_resume_selects_current_memory_without_resurrecting_deleted_context() {
    use factory_types::memory::*;
    let (temp, runtime, run) = fixture();
    let memory = runtime
        .factory()
        .unwrap()
        .project_memory()
        .unwrap()
        .create(
            &MemoryInput {
                category: MemoryCategory::CodingConventions,
                title: "Result partial work convention".into(),
                content: "Result changes preserve partial work and require fresh verification."
                    .into(),
                subject: "result_convention".into(),
                scope: "implementation".into(),
                status: MemoryStatus::Active,
                sources: vec![MemorySource {
                    project_id: PROJECT.into(),
                    reference: MemorySourceRef::Manual {
                        note: "User-authored runtime test convention".into(),
                    },
                }],
            },
            "before execution",
        )
        .unwrap();
    suspend_running(&temp, &runtime, run).await;
    let db = database(temp.path());
    let fenced_factory = runtime.factory().unwrap();
    let mut extraction_settings = fenced_factory.project_memory().unwrap().settings().unwrap();
    extraction_settings.extraction_mode = ExtractionMode::Review;
    fenced_factory
        .project_memory()
        .unwrap()
        .save_settings(&extraction_settings)
        .unwrap();
    let session_count = db.list_agent_sessions(Some(run)).unwrap().len();
    let denied = fenced_factory
        .extract_project_memory(
            &[MemorySource {
                project_id: PROJECT.into(),
                reference: MemorySourceRef::Mission { run_id: run },
            }],
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap_err();
    assert!(denied.to_string().contains("Memory policy denies"));
    assert_eq!(
        db.list_agent_sessions(Some(run)).unwrap().len(),
        session_count
    );
    extraction_settings = fenced_factory.project_memory().unwrap().settings().unwrap();
    extraction_settings.extraction_mode = ExtractionMode::Disabled;
    fenced_factory
        .project_memory()
        .unwrap()
        .save_settings(&extraction_settings)
        .unwrap();
    let interrupted = db
        .list_agent_sessions(Some(run))
        .unwrap()
        .into_iter()
        .find(|s| s.status == "interrupted" && s.role == "worker")
        .unwrap();
    let original = db
        .memory_usage(PROJECT, Some(interrupted.id), None)
        .unwrap();
    assert_eq!(original[0].memories[0].memory_id, memory.id);
    let factory = runtime.factory().unwrap();
    factory
        .project_memory()
        .unwrap()
        .delete(&memory.id, memory.revision)
        .unwrap();
    let replacement = factory
        .project_memory()
        .unwrap()
        .create(
            &MemoryInput {
                category: MemoryCategory::CodingConventions,
                title: "Result current verification convention".into(),
                content: "Result partial work must be independently verified after continuation."
                    .into(),
                subject: "result_convention".into(),
                scope: "implementation".into(),
                status: MemoryStatus::Active,
                sources: vec![MemorySource {
                    project_id: PROJECT.into(),
                    reference: MemorySourceRef::Manual {
                        note: "User correction while suspended".into(),
                    },
                }],
            },
            "current revision",
        )
        .unwrap();
    assert!(runtime.active_operations().is_empty());
    let generation = db.mission_lifecycle(run).unwrap().unwrap().generation;
    runtime
        .resume_mission(run, generation, "memory-aware-manual-resume")
        .unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
    })
    .await;
    let sessions = db.list_agent_sessions(Some(run)).unwrap();
    let resumed = sessions
        .iter()
        .filter(|s| s.role == "worker")
        .max_by_key(|s| s.id)
        .unwrap();
    let supplied = db.memory_usage(PROJECT, Some(resumed.id), None).unwrap();
    assert!(!supplied[0]
        .memories
        .iter()
        .any(|m| m.memory_id == memory.id));
    assert!(supplied[0]
        .memories
        .iter()
        .any(|m| m.memory_id == replacement.id));
    assert_eq!(
        db.memory_usage(PROJECT, Some(interrupted.id), None)
            .unwrap()[0]
            .memories,
        original[0].memories
    );
    assert_eq!(db.list_task_attempts(run).unwrap().len(), 3);
}
#[test]
fn concurrent_suspend_requests_finalize_one_checkpoint_without_duplicate_events_or_owners() {
    let (temp, runtime, run) = fixture();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let runtime = runtime.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                runtime.suspend_mission(run, 1, "concurrent-suspend", "user_requested_suspend")
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        assert_eq!(
            handle.join().unwrap().unwrap().state,
            MissionLifecycleState::Suspended
        );
    }
    let db = database(temp.path());
    assert_eq!(db.mission_checkpoints(run, None).unwrap().len(), 1);
    assert!(runtime.active_operations().is_empty());
    assert!(db.list_task_attempts(run).unwrap().is_empty());
    assert!(db.list_agent_sessions(Some(run)).unwrap().is_empty());
    let connection = rusqlite::Connection::open(temp.path().join(".factory/db.sqlite3")).unwrap();
    for kind in [
        "mission.quiescing",
        "mission.checkpoint_created",
        "mission.suspended",
    ] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM scheduler_events WHERE run_id=?1 AND event_type=?2",
                    rusqlite::params![run, kind],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().into()
}
fn database(root: &Path) -> FactoryDb {
    FactoryDb::open(&root.join(".factory/db.sqlite3")).unwrap()
}
fn python() -> String {
    ["python", "python3"]
        .into_iter()
        .find(|p| {
            Command::new(p)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .unwrap()
        .into()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupt_future_foreign_or_stale_checkpoint_never_dispatches_another_worker() {
    let (temp, runtime, run) = fixture();
    suspend_running(&temp, &runtime, run).await;
    let db = database(temp.path());
    let checkpoint = db.mission_checkpoints(run, None).unwrap().remove(0);
    let fault = rusqlite::Connection::open(temp.path().join(".factory/db.sqlite3")).unwrap();
    let original = serde_json::to_value(&checkpoint).unwrap();
    let mut injected = original["worktrees"].clone();
    injected[0]["baseCommit"] = serde_json::json!("--help");
    for (n, field, value) in [
        (0, "schemaVersion", serde_json::json!(99)),
        (
            1,
            "projectId",
            serde_json::json!("p_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ),
        (2, "planRevision", serde_json::json!(999)),
        (
            3,
            "integrationCommit",
            serde_json::json!("0000000000000000000000000000000000000000"),
        ),
        (4, "worktrees", injected),
    ] {
        let mut payload = original.clone();
        payload[field] = value;
        fault
            .execute(
                "UPDATE mission_checkpoints SET payload=?1 WHERE id=?2",
                rusqlite::params![payload.to_string(), checkpoint.id],
            )
            .unwrap();
        let generation = db.mission_lifecycle(run).unwrap().unwrap().generation;
        assert!(runtime
            .resume_mission(run, generation, &format!("corrupt-{n}"))
            .is_err());
        assert!(runtime.active_operations().is_empty());
        assert_eq!(db.list_task_attempts(run).unwrap().len(), 2);
        assert_eq!(
            db.mission_lifecycle(run).unwrap().unwrap().state,
            MissionLifecycleState::RecoveryRequired
        );
    }
}

fn fixture() -> (tempfile::TempDir, Runtime, i64) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join(".gitignore"), ".factory/\n__pycache__/\n").unwrap();
    std::fs::write(root.join("result.py"), "VALUE = 0\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Lifecycle Fixture"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "baseline"]);
    Factory::init(root).unwrap();
    let script = root.join(".factory/agent.py");
    std::fs::write(&script,r#"
import json,os,pathlib,sys,time
text=sys.stdin.read()
if sys.argv[1]=='review':
 if os.environ.get('ASF_TEST_SLOW_REVIEW') and 'VALUE = 42' in text: time.sleep(30)
 print(json.dumps({'decision':'approve','reason':'controlled fixture review of real captured diff','feedback':[]}))
else:
 if os.environ.get('ASF_TEST_REFUSED_WORKER'):
  print(json.dumps({'commands':['curl --version'],'summary':'controlled command report refused by project policy','tests':[]}))
  sys.exit(0)
 task=text.split('\nTASK\n')[1].split('\nOPERATION\n')[0]
 if 'FAST_COMPLETE' in task:
  pathlib.Path('fast.py').write_text('PREDECESSOR = 1\n')
  print(json.dumps({'commands':[],'summary':'controlled predecessor implementation','tests':[]}))
  sys.exit(0)
 marker=pathlib.Path('partial-marker.txt')
 if 'ASF_LOGICAL_RESUME' in text and not marker.exists() and pathlib.Path('result.py').read_text()=='VALUE = 42\n':
  print(json.dumps({'commands':[],'summary':'preserved completed source awaits new review','tests':[]}))
  sys.exit(0)
 if not marker.exists():
  marker.write_text('owned partial work',encoding='utf8')
  pathlib.Path('result.py').write_text('VALUE = 1\n',encoding='utf8')
 else:
  assert 'ASF_LOGICAL_RESUME' in text
  assert pathlib.Path('result.py').read_text()=='VALUE = 1\n'
 if (not os.environ.get('ASF_TEST_FAST_WORKER') and 'ASF_LOGICAL_RESUME' not in text) or os.environ.get('ASF_TEST_ALWAYS_WAIT'): time.sleep(30)
 marker.unlink()
 pathlib.Path('result.py').write_text('VALUE = 42\n',encoding='utf8')
 print(json.dumps({'commands':[],'summary':'controlled source change after logical continuation','tests':[]}))
"#).unwrap();
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
                args: vec![script.to_string_lossy().into(), mode.into()],
                env: BTreeMap::from([("PYTHONDONTWRITEBYTECODE".into(), "1".into())]),
                prompt_transport: Some(factory_agent::PromptTransport::Stdin),
                interactive_args: None,
                capabilities: vec![],
                max_concurrency: Some(1),
            },
        );
        config.role_assignments.push(factory_core::RoleAssignment {
            role: role.into(),
            agent: role.into(),
            preferred: true,
        });
    }
    config.write_atomic(root).unwrap();
    let runtime = Runtime::new(root).unwrap();
    runtime.bind_project_identity(PROJECT).unwrap();
    let factory = runtime.factory().unwrap();
    let run = factory
        .begin_run("Controlled lifecycle source modification", None)
        .unwrap();
    let db = database(root);
    db.register_workspace_session(PROJECT, SESSION).unwrap();
    db.bind_workspace_run(run.id, PROJECT, SESSION).unwrap();
    factory
        .persist_authored_plan(
            run.id,
            &Plan {
                objective: "Controlled lifecycle".into(),
                tasks: vec![
                    PlannedTask {
                        id: "fast".into(),
                        title: "Predecessor".into(),
                        objective: "FAST_COMPLETE independent prerequisite".into(),
                        dependencies: vec![],
                        acceptance_criteria: vec!["predecessor captured".into()],
                        role: None,
                        operation: None,
                    },
                    PlannedTask {
                        id: "edit".into(),
                        title: "Edit implementation".into(),
                        objective: "Modify result and preserve partial work".into(),
                        dependencies: vec!["fast".into()],
                        acceptance_criteria: vec!["VALUE is 42".into()],
                        role: None,
                        operation: None,
                    },
                ],
            },
        )
        .unwrap();
    (temp, runtime, run.id)
}
async fn wait(root: &Path, run: i64, predicate: impl Fn(&FactoryDb) -> bool) {
    for _ in 0..600 {
        if predicate(&database(root)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Timed out waiting for workflow {run}");
}
async fn suspend_running(temp: &tempfile::TempDir, runtime: &Runtime, run: i64) {
    runtime.start_workflow(run).unwrap();
    wait(temp.path(), run, |db| {
        db.list_tasks(run).unwrap().iter().any(|t| {
            t.worktree_path
                .as_ref()
                .is_some_and(|p| Path::new(p).join("partial-marker.txt").exists())
        })
    })
    .await;
    runtime
        .suspend_mission(run, 1, "suspend-owned-worker", "user_requested_suspend")
        .unwrap();
    wait(temp.path(), run, |db| {
        db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Suspended
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn application_shutdown_fences_and_drains_owned_worker_with_durable_checkpoint() {
    let (temp, runtime, run) = fixture();
    runtime.start_workflow(run).unwrap();
    wait(temp.path(), run, |db| {
        db.list_tasks(run).unwrap().iter().any(|task| {
            task.worktree_path
                .as_ref()
                .is_some_and(|path| Path::new(path).join("partial-marker.txt").exists())
        })
    })
    .await;
    runtime.shutdown().unwrap();
    wait(temp.path(), run, |db| {
        db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Suspended
            && runtime.is_idle()
    })
    .await;
    assert!(runtime.is_idle());
    assert!(runtime.reserve_operation().is_err());
    let db = database(temp.path());
    let life = db.mission_lifecycle(run).unwrap().unwrap();
    assert_eq!(life.reason.as_deref(), Some("application_shutdown"));
    assert!(db
        .lifecycle_processes(run)
        .unwrap()
        .iter()
        .all(|(_, _, meta)| meta
            .as_ref()
            .is_none_or(|meta| meta.process_tree_cleanup_confirmed)));
    let tasks = db.list_tasks(run).unwrap();
    assert!(tasks
        .iter()
        .any(|task| task.state == factory_types::TaskState::Completed));
    assert!(tasks.iter().any(
        |task| task
            .worktree_path
            .as_ref()
            .is_some_and(|path| Path::new(path).join("partial-marker.txt").exists())
    ));
    drop(runtime);
    let reopened = Runtime::new(temp.path()).unwrap();
    reopened.bind_project_identity(PROJECT).unwrap();
    assert!(reopened.active_operations().is_empty());
    assert_eq!(
        database(temp.path())
            .mission_lifecycle(run)
            .unwrap()
            .unwrap()
            .state,
        MissionLifecycleState::Suspended
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_worker_is_drained_and_partial_work_is_preserved_before_resume() {
    let (temp, runtime, run) = fixture();
    suspend_running(&temp, &runtime, run).await;
    let db = database(temp.path());
    let attempt = db
        .latest_task_attempt(
            db.list_tasks(run)
                .unwrap()
                .iter()
                .find(|t| t.title == "Edit implementation")
                .unwrap()
                .id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(attempt.status, AttemptStatus::Interrupted);
    assert!(Path::new(&attempt.worktree_path)
        .join("partial-marker.txt")
        .exists());
    assert_eq!(
        std::fs::read_to_string(Path::new(&attempt.worktree_path).join("fast.py"))
            .unwrap()
            .trim(),
        "PREDECESSOR = 1"
    );
    let predecessor = db
        .list_tasks(run)
        .unwrap()
        .into_iter()
        .find(|t| t.title == "Predecessor")
        .unwrap();
    let predecessor_attempt = db.latest_task_attempt(predecessor.id).unwrap().unwrap();
    assert_eq!(predecessor.state, TaskState::Completed);
    let sessions = db.list_agent_sessions(Some(run)).unwrap();
    let meta = sessions
        .iter()
        .find(|s| s.status == "interrupted")
        .unwrap()
        .output_meta
        .as_ref()
        .unwrap();
    assert!(meta.process_tree_cleanup_confirmed);
    assert!(meta.process_finished_unix_ms.is_some());
    assert!(factory_proc::owned_process_absent(meta.process_id.unwrap()));
    assert_eq!(db.active_dispatch_count(run).unwrap(), 0);
    assert!(runtime.active_operations().is_empty());
    assert!(!db.scheduler_running(run).unwrap());
    let checkpoint = db.mission_checkpoints(run, None).unwrap();
    assert_eq!(checkpoint.len(), 1);
    assert_eq!(checkpoint[0].worktrees[0].task_id, attempt.task_id);
    runtime.resume_mission(run, 2, "resume-once").unwrap();
    runtime.resume_mission(run, 2, "resume-once").unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
    })
    .await;
    let db = database(temp.path());
    assert_eq!(db.list_task_attempts(run).unwrap().len(), 3);
    assert_eq!(
        db.latest_task_attempt(predecessor.id).unwrap().unwrap().id,
        predecessor_attempt.id
    );
    assert_eq!(
        db.integration_records(run)
            .unwrap()
            .iter()
            .filter(|r| r.status == "published")
            .count(),
        2
    );
    assert_eq!(
        git(temp.path(), &["show", "factory/run-1:result.py"]),
        "VALUE = 42"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("result.py")).unwrap(),
        "VALUE = 0\n"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_revalidates_missing_providers_and_new_read_only_policy_without_new_execution() {
    for provider_missing in [true, false] {
        let (temp, runtime, run) = fixture();
        suspend_running(&temp, &runtime, run).await;
        let db = database(temp.path());
        let mut config = Config::load_and_migrate(temp.path()).unwrap();
        if provider_missing {
            config.agents.get_mut("worker").unwrap().command = temp
                .path()
                .join("nonexistent-cli.exe")
                .to_string_lossy()
                .into_owned();
        } else {
            config
                .policies
                .roles
                .entry("worker".into())
                .or_default()
                .filesystem = Some(Default::default());
        }
        config.write_atomic(temp.path()).unwrap();
        assert!(runtime
            .resume_mission(run, 2, "changed-execution-context")
            .is_err());
        assert!(runtime.active_operations().is_empty());
        assert_eq!(db.list_task_attempts(run).unwrap().len(), 2);
        assert!(matches!(
            db.mission_lifecycle(run).unwrap().unwrap().state,
            MissionLifecycleState::Suspended | MissionLifecycleState::RecoveryRequired
        ));
        assert!(db
            .list_tasks(run)
            .unwrap()
            .iter()
            .any(|task| task.title == "Predecessor" && task.state == TaskState::Completed));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suspended_checkpoint_survives_runtime_restart_without_automatic_dispatch() {
    let (temp, runtime, run) = fixture();
    suspend_running(&temp, &runtime, run).await;
    drop(runtime);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reopened = Runtime::new(temp.path()).unwrap();
    reopened.bind_project_identity(PROJECT).unwrap();
    assert_eq!(
        database(temp.path())
            .mission_lifecycle(run)
            .unwrap()
            .unwrap()
            .state,
        MissionLifecycleState::Suspended
    );
    assert!(reopened.active_operations().is_empty());
    assert!(reopened.start_workflow(run).is_err());
    reopened.resume_mission(run, 2, "restart-resume").unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_retained_worktree_refuses_resume_without_launching_another_worker() {
    let (temp, runtime, run) = fixture();
    suspend_running(&temp, &runtime, run).await;
    let db = database(temp.path());
    let task = db
        .list_tasks(run)
        .unwrap()
        .into_iter()
        .find(|t| t.title == "Edit implementation")
        .unwrap();
    std::fs::write(
        Path::new(task.worktree_path.as_ref().unwrap()).join("result.py"),
        "unexpected external edit\n",
    )
    .unwrap();
    assert!(runtime.resume_mission(run, 2, "unsafe-resume").is_err());
    assert_eq!(
        db.mission_lifecycle(run).unwrap().unwrap().state,
        MissionLifecycleState::RecoveryRequired
    );
    assert_eq!(db.list_task_attempts(run).unwrap().len(), 2);
    assert!(runtime.active_operations().is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_close_blocks_resume_until_the_exact_session_is_reopened() {
    let (temp, runtime, run) = fixture();
    suspend_running(&temp, &runtime, run).await;
    let db = database(temp.path());
    db.set_workspace_session_closed(PROJECT, SESSION, 1, true)
        .unwrap();
    assert!(runtime.resume_mission(run, 2, "closed-resume").is_err());
    db.set_workspace_session_closed(PROJECT, SESSION, 2, false)
        .unwrap();
    runtime.resume_mission(run, 2, "reopened-resume").unwrap();
    wait(temp.path(), run, |db| {
        db.list_tasks(run)
            .unwrap()
            .iter()
            .all(|t| t.state == TaskState::Completed)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_interruption_preserves_the_implementation_and_requires_a_new_decision() {
    let (temp, runtime, run) = fixture();
    let mut config = Config::load_and_migrate(temp.path()).unwrap();
    config
        .agents
        .get_mut("worker")
        .unwrap()
        .env
        .insert("ASF_TEST_FAST_WORKER".into(), "1".into());
    config
        .agents
        .get_mut("reviewer")
        .unwrap()
        .env
        .insert("ASF_TEST_SLOW_REVIEW".into(), "1".into());
    config.write_atomic(temp.path()).unwrap();
    runtime.start_workflow(run).unwrap();
    wait(temp.path(), run, |db| {
        db.list_agent_sessions(Some(run)).unwrap().iter().any(|s| {
            s.role == "reviewer"
                && s.status == "running"
                && s.output_meta
                    .as_ref()
                    .is_some_and(|m| m.process_id.is_some())
                && s.task_id.is_some_and(|id| {
                    db.get_task(id)
                        .unwrap()
                        .is_some_and(|t| t.title == "Edit implementation")
                })
        })
    })
    .await;
    runtime
        .suspend_mission(run, 1, "review-suspend", "user_requested_suspend")
        .unwrap();
    wait(temp.path(), run, |db| {
        db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Suspended
    })
    .await;
    let db = database(temp.path());
    let original = db
        .list_agent_sessions(Some(run))
        .unwrap()
        .into_iter()
        .find(|s| s.role == "reviewer" && s.status == "interrupted")
        .unwrap();
    assert!(db
        .latest_task_attempt(original.task_id.unwrap())
        .unwrap()
        .unwrap()
        .review
        .is_none());
    config
        .agents
        .get_mut("reviewer")
        .unwrap()
        .env
        .remove("ASF_TEST_SLOW_REVIEW");
    config.write_atomic(temp.path()).unwrap();
    runtime.resume_mission(run, 2, "review-resume").unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
    })
    .await;
    assert_eq!(
        db.get_agent_session(original.id).unwrap().unwrap().status,
        "interrupted"
    );
    assert!(db
        .latest_task_attempt(original.task_id.unwrap())
        .unwrap()
        .unwrap()
        .review
        .is_some_and(|r| r.decision == factory_types::ReviewDecision::Approve));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integration_validation_suspension_resumes_a_reviewed_candidate_without_rerunning_the_worker(
) {
    let (temp, runtime, run) = fixture();
    let mut config = Config::load_and_migrate(temp.path()).unwrap();
    config
        .agents
        .get_mut("worker")
        .unwrap()
        .env
        .insert("ASF_TEST_FAST_WORKER".into(), "1".into());
    config.integration_checks=vec![factory_core::execution::ValidationCommand{program:python(),args:vec!["-B".into(),"-c".into(),"import pathlib,time; time.sleep(30 if '42' in pathlib.Path('result.py').read_text() else 0)".into()],timeout_secs:60}];
    config.write_atomic(temp.path()).unwrap();
    runtime.start_workflow(run).unwrap();
    wait(temp.path(), run, |db| {
        db.integration_records(run).unwrap().iter().any(|r| {
            r.status == "preparing"
                && db
                    .get_task(r.task_id)
                    .unwrap()
                    .is_some_and(|t| t.title == "Edit implementation")
        })
    })
    .await;
    runtime
        .suspend_mission(run, 1, "integration-suspend", "user_requested_suspend")
        .unwrap();
    wait(temp.path(), run, |db| {
        db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Suspended
    })
    .await;
    let db = database(temp.path());
    let count = db.list_task_attempts(run).unwrap().len();
    assert!(db
        .integration_records(run)
        .unwrap()
        .iter()
        .any(|r| r.status == "suspended"));
    config.integration_checks.clear();
    config.write_atomic(temp.path()).unwrap();
    runtime
        .resume_mission(run, 2, "integration-resume")
        .unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
    })
    .await;
    assert_eq!(db.list_task_attempts(run).unwrap().len(), count);
    assert_eq!(
        db.integration_records(run)
            .unwrap()
            .iter()
            .filter(|r| r.status == "published")
            .count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_handoff_survives_suspension_without_rewriting_the_interrupted_attempt() {
    let (temp, runtime, run) = fixture();
    let factory = runtime.factory().unwrap();
    let profile = factory
        .save_agent_profile(
            None,
            None,
            factory_types::AgentProfileInput {
                name: "Resume replacement".into(),
                description: String::new(),
                provider_adapter_id: "worker".into(),
                model_id: None,
                reasoning_effort: None,
                roles: vec!["worker".into()],
                instructions: "Continue only from persisted authorized evidence".into(),
                permissions: Default::default(),
                max_concurrency: 1,
                timeout_secs: 60,
                enabled: true,
                automatic_delegation: false,
                preferred: false,
            },
        )
        .unwrap();
    runtime.start_workflow(run).unwrap();
    wait(temp.path(), run, |db| {
        db.list_tasks(run).unwrap().iter().any(|t| {
            t.worktree_path
                .as_ref()
                .is_some_and(|p| Path::new(p).join("partial-marker.txt").exists())
        })
    })
    .await;
    let db = database(temp.path());
    let task = db
        .list_tasks(run)
        .unwrap()
        .into_iter()
        .find(|t| t.title == "Edit implementation")
        .unwrap();
    let attempt = db.latest_task_attempt(task.id).unwrap().unwrap();
    let handoff = runtime
        .factory()
        .unwrap()
        .request_agent_handoff(
            run,
            task.id,
            factory_types::HandoffRequest {
                to_profile_id: profile.id.clone(),
                expected_profile_revision: profile.revision,
                idempotency_key: "pending-at-suspend".into(),
                reason: "Explicit replacement after a verified boundary".into(),
            },
        )
        .unwrap();
    runtime
        .suspend_mission(run, 1, "handoff-suspend", "user_requested_suspend")
        .unwrap();
    wait(temp.path(), run, |db| {
        db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Suspended
    })
    .await;
    assert_eq!(
        db.active_handoff(task.id).unwrap().unwrap().state,
        "waiting"
    );
    let interrupted = db.get_task_attempt(attempt.id).unwrap().unwrap();
    assert_eq!(interrupted.status, AttemptStatus::Interrupted);
    runtime.resume_mission(run, 2, "handoff-resume").unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
            && db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Settled
    })
    .await;
    assert_eq!(
        db.get_task_attempt(attempt.id).unwrap().unwrap(),
        interrupted
    );
    let completed = db
        .handoffs(run)
        .unwrap()
        .into_iter()
        .find(|h| h.id == handoff.id)
        .unwrap();
    assert_eq!(completed.state, "completed");
    assert_eq!(completed.boundary, "reconciled_suspension");
    assert_eq!(
        db.get_agent_session(completed.replacement_session_id.unwrap())
            .unwrap()
            .unwrap()
            .agent,
        profile.id
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_controlled_suspensions_do_not_consume_code_quality_attempts_or_republish_completed_work(
) {
    let (temp, runtime, run) = fixture();
    let mut config = Config::load_and_migrate(temp.path()).unwrap();
    config
        .agents
        .get_mut("worker")
        .unwrap()
        .env
        .insert("ASF_TEST_ALWAYS_WAIT".into(), "1".into());
    config.write_atomic(temp.path()).unwrap();
    suspend_running(&temp, &runtime, run).await;
    let db = database(temp.path());
    let task = db
        .list_tasks(run)
        .unwrap()
        .into_iter()
        .find(|t| t.title == "Edit implementation")
        .unwrap();
    let mut previous_elapsed = db.scheduler_elapsed_millis(run).unwrap();
    for n in 0..3 {
        let generation = db.mission_lifecycle(run).unwrap().unwrap().generation;
        let previous = db
            .list_agent_sessions(Some(run))
            .unwrap()
            .iter()
            .map(|s| s.id)
            .max()
            .unwrap();
        runtime
            .resume_mission(run, generation, &format!("cycle-resume-{n}"))
            .unwrap();
        wait(temp.path(), run, |db| {
            db.list_agent_sessions(Some(run)).unwrap().iter().any(|s| {
                s.id > previous
                    && s.task_id == Some(task.id)
                    && s.status == "running"
                    && s.output_meta
                        .as_ref()
                        .is_some_and(|m| m.process_id.is_some())
            })
        })
        .await;
        let replay = runtime
            .suspend_mission(run, 1, "suspend-owned-worker", "user_requested_suspend")
            .unwrap();
        assert_eq!(replay.state, MissionLifecycleState::Active);
        assert_eq!(replay.generation, generation + 1);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(db
            .list_agent_sessions(Some(run))
            .unwrap()
            .iter()
            .any(|s| s.id > previous && s.task_id == Some(task.id) && s.status == "running"));
        runtime
            .suspend_mission(
                run,
                generation + 1,
                &format!("cycle-suspend-{n}"),
                "user_requested_suspend",
            )
            .unwrap();
        wait(temp.path(), run, |db| {
            db.mission_lifecycle(run).unwrap().unwrap().state == MissionLifecycleState::Suspended
        })
        .await;
        assert!(db.scheduler_elapsed_millis(run).unwrap() >= previous_elapsed);
        previous_elapsed = db.scheduler_elapsed_millis(run).unwrap();
        assert_eq!(db.active_dispatch_count(run).unwrap(), 0);
    }
    assert_eq!(db.task_execution_attempt_count(task.id).unwrap(), 0);
    config
        .agents
        .get_mut("worker")
        .unwrap()
        .env
        .remove("ASF_TEST_ALWAYS_WAIT");
    config.write_atomic(temp.path()).unwrap();
    let generation = db.mission_lifecycle(run).unwrap().unwrap().generation;
    runtime
        .resume_mission(run, generation, "cycles-final-resume")
        .unwrap();
    wait(temp.path(), run, |db| {
        db.get_run(run).unwrap().unwrap().status == RunStatus::Completed
    })
    .await;
    assert_eq!(db.list_task_attempts_for_task(task.id).unwrap().len(), 5);
    assert_eq!(
        db.integration_records(run)
            .unwrap()
            .iter()
            .filter(|r| r.status == "published")
            .count(),
        2
    );
}
