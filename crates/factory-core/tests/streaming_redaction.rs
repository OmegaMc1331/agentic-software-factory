//! ASF-N01 regression: a secret emitted by an agent process across multiple
//! write boundaries must never become visible in persisted session output,
//! neither in intermediate streaming rows nor in the final row.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use factory_core::{AgentEntry, Config, Factory, RoleAssignment, WorkflowResult};
use factory_db::FactoryDb;
use factory_types::RunStatus;
use tempfile::TempDir;

const SECRET: &str = "stream-secret-7f4d2a3b";
const PLAN: &str = r#"{"objective":"streaming redaction","tasks":[{"id":"T1","title":"Emit","objective":"write fragments","dependencies":[],"acceptanceCriteria":["worker evidence exists"]}]}"#;
const REPORT: &str = r#"{"commands":["fake-worker"],"tests":["fake-test"]}"#;
const REVIEW: &str = r#"{"decision":"approve","reason":"evidence accepted"}"#;

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

fn shell_entry(script: &str) -> AgentEntry {
    let common = AgentEntry {
        kind: None,
        env: BTreeMap::new(),
        prompt_transport: None,
        interactive_args: None,
        capabilities: Vec::new(),
        max_concurrency: None,
        command: String::new(),
        args: Vec::new(),
    };
    if cfg!(windows) {
        AgentEntry {
            command: "powershell".into(),
            args: vec!["-NoProfile".into(), "-Command".into(), script.into()],
            ..common
        }
    } else {
        AgentEntry {
            command: "sh".into(),
            args: vec!["-c".into(), script.into()],
            ..common
        }
    }
}

fn cat_file(path: &Path) -> AgentEntry {
    if cfg!(windows) {
        shell_entry(&format!(
            "Get-Content -Raw -LiteralPath '{}'",
            path.display()
        ))
    } else {
        shell_entry(&format!("cat '{}'", path.display()))
    }
}

/// A worker that streams the secret in two separate writes, gated so the test
/// can deterministically observe the persisted row between the fragments.
fn gated_worker(report: &Path, gate: &Path) -> AgentEntry {
    let gate = gate.display().to_string();
    let done = format!("{gate}-done");
    if cfg!(windows) {
        shell_entry(&format!(
            "$o=[Console]::Out; $e=[Console]::Error; \
             $o.Write('log-prefix |'); $o.Flush(); \
             $e.Write('err-prefix |'); $e.Flush(); \
             while(-not (Test-Path -LiteralPath '{gate}')) {{ Start-Sleep -Milliseconds 50 }}; \
             $o.Write('stream-secret-'); $o.Flush(); \
             $e.Write('stream-secret-'); $e.Flush(); \
             while(-not (Test-Path -LiteralPath '{done}')) {{ Start-Sleep -Milliseconds 50 }}; \
             $o.Write('7f4d2a3b | finished'); $o.Write(\"`n\"); \
             $e.Write('7f4d2a3b | finished'); $e.Write(\"`n\"); \
             Get-Content -Raw -LiteralPath '{}'",
            report.display()
        ))
    } else {
        shell_entry(&format!(
            "printf 'log-prefix |'; printf 'err-prefix |' >&2; \
             while [ ! -f '{gate}' ]; do sleep 0.05; done; \
             printf 'stream-secret-'; printf 'stream-secret-' >&2; \
             while [ ! -f '{done}' ]; do sleep 0.05; done; \
             printf '7f4d2a3b | finished\\n'; printf '7f4d2a3b | finished\\n' >&2; \
             cat '{}'",
            report.display()
        ))
    }
}

/// A worker that emits only the first fragment and then blocks forever; used
/// to exercise cancellation while a secret fragment is held back.
fn stalling_worker(report: &Path, gate: &Path) -> AgentEntry {
    let gate = gate.display().to_string();
    if cfg!(windows) {
        shell_entry(&format!(
            "$o=[Console]::Out; $o.Write('log-prefix |'); $o.Flush(); \
             $o.Write('stream-secret-'); $o.Flush(); \
             while(-not (Test-Path -LiteralPath '{gate}')) {{ Start-Sleep -Milliseconds 50 }}; \
             Get-Content -Raw -LiteralPath '{}'",
            report.display()
        ))
    } else {
        shell_entry(&format!(
            "printf 'log-prefix |'; printf 'stream-secret-'; \
             while [ ! -f '{gate}' ]; do sleep 0.05; done; \
             cat '{}'",
            report.display()
        ))
    }
}

struct Fixture {
    dir: TempDir,
    gate: PathBuf,
}

impl Fixture {
    /// `worker` receives (repo root, report path, gate path) and must wait for
    /// the gate before completing the secret.
    fn new(worker: fn(&Path, &Path) -> AgentEntry) -> Self {
        let dir = TempDir::new().unwrap();
        init_git(dir.path());
        Factory::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("plan.json"), PLAN).unwrap();
        let report = dir.path().join("report.json");
        std::fs::write(&report, REPORT).unwrap();
        let review = dir.path().join("review.json");
        std::fs::write(&review, REVIEW).unwrap();
        let gate = dir.path().join("worker-gate");
        let mut config = Config::default();
        config.agents.insert(
            "planner-test".into(),
            cat_file(&dir.path().join("plan.json")),
        );
        let mut worker = worker(&report, &gate);
        worker
            .env
            .insert("FACTORY_STREAM_SECRET".into(), SECRET.into());
        config.agents.insert("worker-test".into(), worker);
        config
            .agents
            .insert("reviewer-test".into(), cat_file(&review));
        for (role, agent) in [
            ("planner", "planner-test"),
            ("worker", "worker-test"),
            ("reviewer", "reviewer-test"),
        ] {
            config.role_assignments.push(RoleAssignment {
                role: role.into(),
                agent: agent.into(),
                preferred: true,
            });
        }
        config.write_atomic(dir.path()).unwrap();
        Self { dir, gate }
    }
}

fn worker_session_text(db: &FactoryDb, run_id: i64) -> (String, String) {
    let sessions = db.list_agent_sessions(Some(run_id)).unwrap();
    // The worker session appears only after planning finishes; report empty
    // output until then so polling can distinguish "not started yet".
    let Some(session) = sessions
        .iter()
        .find(|session| session.agent == "worker-test")
    else {
        return (String::new(), String::new());
    };
    (
        session.stdout.clone().unwrap_or_default(),
        session.stderr.clone().unwrap_or_default(),
    )
}

fn wait_for<F: Fn(&str, &str) -> bool>(
    db: &FactoryDb,
    run_id: i64,
    predicate: F,
    label: &str,
) -> (String, String) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (stdout, stderr) = worker_session_text(db, run_id);
        if predicate(&stdout, &stderr) {
            return (stdout, stderr);
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn observe_db(dir: &Path) -> FactoryDb {
    FactoryDb::open(&dir.join(".factory").join("db.sqlite3")).unwrap()
}

#[test]
fn a_secret_split_across_writes_never_reaches_persisted_output() {
    let fixture = Fixture::new(gated_worker);
    let observer = observe_db(fixture.dir.path());
    let factory = Factory::open(fixture.dir.path()).unwrap();
    let outcome = factory.create_run("streaming redaction").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let run_id = outcome.run.id;
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let handle =
        std::thread::spawn(move || factory.execute_active_run(run_id, &worker_cancel).unwrap());

    // Phase 1: ordinary output flows into the persisted row.
    let (stdout, stderr) = wait_for(
        &observer,
        run_id,
        |stdout, stderr| stdout.ends_with("log-prefix |") && stderr.ends_with("err-prefix |"),
        "initial fragments persisted",
    );
    assert!(!stdout.contains(SECRET) && !stderr.contains(SECRET));

    // Phase 2: the worker has now written the first secret fragment. The
    // redactor must hold it back: the persisted row must not grow with the
    // partial secret while the stream is still running.
    std::fs::write(&fixture.gate, "go").unwrap();
    let hold_deadline = Instant::now() + Duration::from_millis(750);
    while Instant::now() < hold_deadline {
        let (stdout, stderr) = worker_session_text(&observer, run_id);
        assert!(
            !stdout.contains("stream-secret-") && !stderr.contains("stream-secret-"),
            "partial secret fragment leaked into the persisted row: {stdout:?} / {stderr:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // Phase 3: complete the secret and let the workflow finish.
    std::fs::write(
        PathBuf::from(format!("{}-done", fixture.gate.display())),
        "go",
    )
    .unwrap();
    let result = handle.join().unwrap();
    assert_eq!(result, WorkflowResult::Completed);

    let (stdout, stderr) = worker_session_text(&observer, run_id);
    for text in [&stdout, &stderr] {
        assert!(
            !text.contains(SECRET),
            "final row leaked the secret: {text:?}"
        );
        assert!(
            !text.contains("stream-secret-"),
            "fragment leaked: {text:?}"
        );
        assert!(
            text.contains("[REDACTED]"),
            "expected redaction marker: {text:?}"
        );
        assert!(text.contains("-prefix |"), "ordinary output lost: {text:?}");
    }
    assert!(
        stdout.contains("log-prefix |"),
        "stdout prefix lost: {stdout:?}"
    );
    assert!(
        stderr.contains("err-prefix |"),
        "stderr prefix lost: {stderr:?}"
    );
}

#[test]
fn cancelling_mid_fragment_never_reconstructs_the_secret() {
    let fixture = Fixture::new(stalling_worker);
    let observer = observe_db(fixture.dir.path());
    let factory = Factory::open(fixture.dir.path()).unwrap();
    let outcome = factory.create_run("cancellation redaction").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let run_id = outcome.run.id;
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let handle =
        std::thread::spawn(move || factory.execute_active_run(run_id, &worker_cancel).unwrap());

    // The first fragment is held back while the worker stalls.
    wait_for(
        &observer,
        run_id,
        |stdout, _| stdout.ends_with("log-prefix |"),
        "initial fragment persisted",
    );
    let stall_deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < stall_deadline {
        let (stdout, stderr) = worker_session_text(&observer, run_id);
        assert!(
            !stdout.contains("stream-secret-") && !stderr.contains("stream-secret-"),
            "held fragment leaked while stalled: {stdout:?} / {stderr:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    cancel.store(true, Ordering::SeqCst);
    let _ = handle.join().unwrap();

    let (stdout, stderr) = worker_session_text(&observer, run_id);
    for text in [&stdout, &stderr] {
        assert!(
            !text.contains(SECRET),
            "cancelled row leaked the secret: {text:?}"
        );
    }
    // The unterminated prefix may be flushed after the stream ends (it cannot
    // be completed anymore and is not itself the secret), but never the value.
    let run = observer.get_run(run_id).unwrap().unwrap();
    assert!(
        matches!(run.status, RunStatus::Cancelled | RunStatus::Failed),
        "unexpected status after cancellation: {:?}",
        run.status
    );
}
