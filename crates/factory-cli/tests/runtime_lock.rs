use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use factory_core::Factory;
use factory_db::FactoryDb;
use factory_types::{RunStatus, TaskOperation, TaskState};
use tempfile::TempDir;

struct SeededWorkflow {
    run_id: i64,
    task_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowSnapshot {
    run: String,
    task: String,
}

fn init_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    Factory::init(dir.path()).unwrap();
    dir
}

fn db_path(root: &Path) -> PathBuf {
    root.join(".factory").join("db.sqlite3")
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
    SeededWorkflow {
        run_id: run.id,
        task_id,
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
    }
}

fn running_snapshot() -> WorkflowSnapshot {
    WorkflowSnapshot {
        run: "planning".into(),
        task: "running".into(),
    }
}

fn factory_bin() -> &'static str {
    env!("CARGO_BIN_EXE_factory")
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Server {
    child: Option<Child>,
    port: u16,
}

impl Server {
    fn start(root: &Path) -> Self {
        let port = free_port();
        let child = Command::new(factory_bin())
            .current_dir(root)
            .args(["start", "--no-browser", "--port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut server = Self {
            child: Some(child),
            port,
        };
        server.wait_ready();
        server
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                panic!("factory start exited before becoming ready: {status}");
            }
            if health_ready(self.port) {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("factory start did not become ready on port {}", self.port);
    }
}

fn health_ready(port: u16) -> bool {
    let address = format!("127.0.0.1:{port}").parse().unwrap();
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let request =
        format!("GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok() && response.starts_with("HTTP/1.1 200")
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run_factory(root: &Path, args: &[&str]) -> Output {
    let mut child = Command::new(factory_bin())
        .current_dir(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("factory process did not exit: {args:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn second_start_on_the_same_port_is_rejected_without_reconciling() {
    let dir = init_project();
    let server = Server::start(dir.path());
    let seeded = seed_running_workflow(dir.path());
    let before = snapshot(dir.path(), &seeded);

    let output = run_factory(
        dir.path(),
        &["start", "--no-browser", "--port", &server.port.to_string()],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "second start unexpectedly succeeded"
    );
    assert!(
        stderr.contains("cannot bind the Factory API"),
        "unexpected startup error: {stderr}"
    );
    assert_eq!(snapshot(dir.path(), &seeded), before);
    drop(server);
}

#[test]
fn second_start_on_a_different_port_is_rejected_without_reconciling() {
    let dir = init_project();
    let server = Server::start(dir.path());
    let seeded = seed_running_workflow(dir.path());
    let before = snapshot(dir.path(), &seeded);

    let mut other_port = free_port();
    while other_port == server.port {
        other_port = free_port();
    }
    let output = run_factory(
        dir.path(),
        &["start", "--no-browser", "--port", &other_port.to_string()],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "second start unexpectedly succeeded"
    );
    assert!(
        stderr.contains("another Factory runtime is already serving"),
        "unexpected startup error: {stderr}"
    );
    assert_eq!(snapshot(dir.path(), &seeded), before);
    drop(server);
}

#[test]
fn an_occupied_port_is_rejected_before_startup_reconciliation() {
    let dir = init_project();
    let seeded = seed_running_workflow(dir.path());
    let before = snapshot(dir.path(), &seeded);
    let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();

    let output = run_factory(
        dir.path(),
        &["start", "--no-browser", "--port", &port.to_string()],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "start unexpectedly succeeded");
    assert!(
        stderr.contains("cannot bind the Factory API"),
        "unexpected startup error: {stderr}"
    );
    assert_eq!(
        snapshot(dir.path(), &seeded),
        before,
        "an occupied port triggered startup reconciliation"
    );
}

#[test]
fn an_occupied_port_is_rejected_before_dev_serve_reconciliation() {
    let dir = init_project();
    let seeded = seed_running_workflow(dir.path());
    let before = snapshot(dir.path(), &seeded);
    let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();

    let output = run_factory(dir.path(), &["dev", "serve", "--port", &port.to_string()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "dev serve unexpectedly succeeded");
    assert!(
        stderr.contains("cannot bind the Factory API"),
        "unexpected startup error: {stderr}"
    );
    assert_eq!(
        snapshot(dir.path(), &seeded),
        before,
        "an occupied port triggered dev serve reconciliation"
    );
}

#[test]
fn a_legitimate_fresh_start_recovers_the_interrupted_workflow() {
    let dir = init_project();
    let seeded = seed_running_workflow(dir.path());
    assert_eq!(snapshot(dir.path(), &seeded), running_snapshot());

    let server = Server::start(dir.path());
    assert_eq!(
        snapshot(dir.path(), &seeded),
        WorkflowSnapshot {
            run: "failed".into(),
            task: "failed".into(),
        }
    );
    drop(server);
}
