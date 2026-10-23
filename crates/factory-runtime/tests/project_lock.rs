use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use factory_core::Factory;
use factory_db::FactoryDb;
use factory_runtime::{Runtime, RuntimeError};
use factory_types::{AgentSession, AgentSessionMode, RunStatus, TaskOperation, TaskState};
use tempfile::TempDir;

const CHILD_ENV: &str = "FACTORY_PROJECT_LOCK_CHILD";
const CHILD_ROOT_ENV: &str = "FACTORY_PROJECT_LOCK_CHILD_ROOT";
const CHILD_READY_ENV: &str = "FACTORY_PROJECT_LOCK_CHILD_READY";
const CHILD_STOP_ENV: &str = "FACTORY_PROJECT_LOCK_CHILD_STOP";
const CHILD_CLONE_ENV: &str = "FACTORY_PROJECT_LOCK_CHILD_CLONE";

const LOCK_FILE: &str = ".factory/runtime.lock";

fn init_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    Factory::init(dir.path()).unwrap();
    dir
}

fn db_path(root: &Path) -> PathBuf {
    root.join(".factory").join("db.sqlite3")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowSnapshot {
    run: String,
    task: String,
    attempt: String,
    session: String,
}

struct SeededWorkflow {
    run_id: i64,
    task_id: i64,
    attempt_id: i64,
    session_id: i64,
}

fn seed_running_workflow(root: &Path) -> SeededWorkflow {
    let db = FactoryDb::open(&db_path(root)).unwrap();
    let run = db
        .create_run_with_status("active factory workflow", None, RunStatus::Planning)
        .unwrap();
    let task_id = db
        .create_task(
            run.id,
            "running task",
            "still being worked on",
            &[],
            TaskState::Running,
            0,
            Some("worker"),
            Some(TaskOperation::Implement),
        )
        .unwrap();
    let attempt = db
        .create_task_attempt(
            task_id,
            "worker",
            Some(TaskOperation::Implement),
            "test-agent",
            ".factory/worktrees/t1",
            None,
        )
        .unwrap();
    let session = db
        .insert_agent_session(&AgentSession {
            id: 0,
            run_id: Some(run.id),
            task_id: Some(task_id),
            attempt_id: Some(attempt.id),
            role: "worker".into(),
            operation: Some(TaskOperation::Implement),
            agent: "test-agent".into(),
            mode: AgentSessionMode::Automated,
            command: "test-agent".into(),
            status: "running".into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: None,
            exit_code: None,
            duration_ms: None,
            stdout: Some(String::new()),
            stderr: Some(String::new()),
            policy_audit: None,
        })
        .unwrap();
    SeededWorkflow {
        run_id: run.id,
        task_id,
        attempt_id: attempt.id,
        session_id: session.id,
    }
}

fn snapshot(root: &Path, seeded: &SeededWorkflow) -> WorkflowSnapshot {
    let db = FactoryDb::open(&db_path(root)).unwrap();
    WorkflowSnapshot {
        run: db
            .get_run(seeded.run_id)
            .unwrap()
            .unwrap()
            .status
            .as_str()
            .into(),
        task: db
            .get_task(seeded.task_id)
            .unwrap()
            .unwrap()
            .state
            .as_str()
            .into(),
        attempt: db
            .get_task_attempt(seeded.attempt_id)
            .unwrap()
            .unwrap()
            .status
            .as_str()
            .into(),
        session: db
            .get_agent_session(seeded.session_id)
            .unwrap()
            .unwrap()
            .status,
    }
}

fn running_snapshot() -> WorkflowSnapshot {
    WorkflowSnapshot {
        run: "planning".into(),
        task: "running".into(),
        attempt: "running".into(),
        session: "running".into(),
    }
}

struct Holder {
    child: Option<Child>,
    ready: PathBuf,
    stop: PathBuf,
}

impl Holder {
    fn spawn(root: &Path, clone_runtime: bool) -> Self {
        let ready = root.join(".lock-test-ready");
        let stop = root.join(".lock-test-stop");
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "project_lock_child_holder", "--nocapture"])
            .env(CHILD_ENV, "1")
            .env(CHILD_ROOT_ENV, root)
            .env(CHILD_READY_ENV, &ready)
            .env(CHILD_STOP_ENV, &stop)
            .env(CHILD_CLONE_ENV, if clone_runtime { "1" } else { "0" })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            child: Some(child),
            ready,
            stop,
        }
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self.ready.exists() {
                return;
            }
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                panic!("project lock holder exited before becoming ready: {status}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.as_mut().unwrap().kill();
        let _ = self.child.as_mut().unwrap().wait();
        panic!("timed out waiting for the project lock holder");
    }

    fn stop_and_wait(&mut self) {
        std::fs::write(&self.stop, b"stop").unwrap();
        let status = self.child.take().unwrap().wait().unwrap();
        assert!(status.success(), "lock holder exited with {status}");
    }

    fn kill_and_wait(&mut self) {
        let mut child = self.child.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn project_lock_child_holder() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT_ENV).unwrap());
    let ready = PathBuf::from(std::env::var_os(CHILD_READY_ENV).unwrap());
    let stop = PathBuf::from(std::env::var_os(CHILD_STOP_ENV).unwrap());
    let runtime = Runtime::new(&root).unwrap();
    let _held = if std::env::var_os(CHILD_CLONE_ENV).as_deref() == Some(std::ffi::OsStr::new("1")) {
        let clone = runtime.clone();
        drop(runtime);
        clone
    } else {
        runtime
    };
    std::fs::write(&ready, b"ready").unwrap();
    while !stop.exists() {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_already_locked(runtime: Result<Runtime, RuntimeError>) {
    match runtime {
        Err(RuntimeError::ProjectAlreadyLocked { .. }) => {}
        Err(other) => panic!("expected ProjectAlreadyLocked, got {other}"),
        Ok(_) => panic!("a second runtime acquired the same project lock"),
    }
}

#[test]
fn second_process_is_rejected_without_reconciling_and_exit_releases_the_lock() {
    let dir = init_project();
    let mut holder = Holder::spawn(dir.path(), false);
    holder.wait_ready();
    let seeded = seed_running_workflow(dir.path());

    assert_eq!(snapshot(dir.path(), &seeded), running_snapshot());
    assert_already_locked(Runtime::new(dir.path()));
    assert_eq!(
        snapshot(dir.path(), &seeded),
        running_snapshot(),
        "the rejected runtime changed active workflow state"
    );

    holder.stop_and_wait();
    assert!(
        dir.path().join(LOCK_FILE).exists(),
        "the lock file should remain as harmless diagnostics"
    );

    let runtime = Runtime::new(dir.path()).unwrap();
    assert_eq!(
        snapshot(dir.path(), &seeded),
        WorkflowSnapshot {
            run: "failed".into(),
            task: "failed".into(),
            attempt: "interrupted".into(),
            session: "interrupted".into(),
        }
    );
    drop(runtime);
}

#[test]
fn abnormal_termination_releases_the_lock_and_recovers_on_next_start() {
    let dir = init_project();
    let mut holder = Holder::spawn(dir.path(), false);
    holder.wait_ready();
    let seeded = seed_running_workflow(dir.path());
    assert_eq!(snapshot(dir.path(), &seeded), running_snapshot());

    holder.kill_and_wait();

    let runtime = Runtime::new(dir.path()).unwrap();
    assert_eq!(
        snapshot(dir.path(), &seeded),
        WorkflowSnapshot {
            run: "failed".into(),
            task: "failed".into(),
            attempt: "interrupted".into(),
            session: "interrupted".into(),
        }
    );
    drop(runtime);
}

#[test]
fn a_stale_lock_file_does_not_block_a_new_runtime() {
    let dir = init_project();
    let lock = dir.path().join(LOCK_FILE);
    std::fs::write(&lock, b"pid=999999\nroot=removed\n").unwrap();

    let runtime = Runtime::new(dir.path()).unwrap();
    drop(runtime);

    let contents = std::fs::read_to_string(&lock).unwrap();
    assert!(contents.contains(&format!("pid={}", std::process::id())));
    Runtime::new(dir.path()).unwrap();
}

#[test]
fn independent_projects_can_have_active_runtimes() {
    let first = init_project();
    let second = init_project();

    let first_runtime = Runtime::new(first.path()).unwrap();
    let second_runtime = Runtime::new(second.path()).unwrap();
    assert_already_locked(Runtime::new(first.path()));
    assert_already_locked(Runtime::new(second.path()));

    drop(first_runtime);
    drop(second_runtime);
}

#[test]
fn a_cloned_runtime_keeps_ownership_until_the_last_handle_is_dropped() {
    let dir = init_project();
    let mut holder = Holder::spawn(dir.path(), true);
    holder.wait_ready();

    assert_already_locked(Runtime::new(dir.path()));

    holder.stop_and_wait();
    Runtime::new(dir.path()).unwrap();
}

#[test]
fn a_path_alias_cannot_bypass_the_project_lock() {
    let dir = init_project();
    let mut holder = Holder::spawn(dir.path(), false);
    holder.wait_ready();

    #[cfg(unix)]
    let alias = {
        let link = dir.path().parent().unwrap().join(format!(
            "{}-alias-{}",
            dir.path().file_name().unwrap().to_string_lossy(),
            std::process::id()
        ));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        link
    };
    #[cfg(windows)]
    let alias = {
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        nested.join("..")
    };

    assert_already_locked(Runtime::new(&alias));
    holder.stop_and_wait();
    Runtime::new(&alias).unwrap();

    #[cfg(unix)]
    let _ = std::fs::remove_file(&alias);
}
