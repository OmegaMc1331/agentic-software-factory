use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_core::{AgentEntry, Config, RoleAssignment};
use factory_types::{AgentSession, AgentSessionMode, RunStatus, TaskState};
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

fn init_root(root: &Path) {
    factory_core::Factory::init(root).unwrap();
}

fn make_state(root: &Path) -> Arc<factory_api::ApiState> {
    Arc::new(factory_api::ApiState::new(root.to_path_buf()).unwrap())
}

#[test]
fn pty_api_child() {
    if std::env::var("FACTORY_API_PTY_CHILD").as_deref() == Ok("1") {
        std::thread::sleep(Duration::from_secs(20));
    }
}

fn command_entry(script: &str) -> AgentEntry {
    if cfg!(windows) {
        AgentEntry {
            kind: None,
            command: "cmd".into(),
            args: vec!["/d".into(), "/c".into(), script.into()],
            env: BTreeMap::new(),
            prompt_transport: None,
            interactive_args: None,
            capabilities: Vec::new(),
            max_concurrency: None,
        }
    } else {
        AgentEntry {
            kind: None,
            command: "sh".into(),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::new(),
            prompt_transport: None,
            interactive_args: None,
            capabilities: Vec::new(),
            max_concurrency: None,
        }
    }
}

fn configure_test_agents(root: &Path, slow_planner: bool) {
    let plan = r#"{"objective":"API workflow","tasks":[{"id":"T1","title":"API task","objective":"exercise the workflow API","dependencies":[],"acceptanceCriteria":["review approved"]}]}"#;
    let worker_output = r#"{"commands":["test-worker"],"tests":["test-check"]}"#;
    let review_output = r#"{"decision":"approve","reason":"API evidence accepted"}"#;
    let plan_path = root.join("test-plan.json");
    let worker_path = root.join("test-worker.json");
    let reviewer_path = root.join("test-reviewer.json");
    std::fs::write(&plan_path, plan).unwrap();
    std::fs::write(&worker_path, worker_output).unwrap();
    std::fs::write(&reviewer_path, review_output).unwrap();
    let planner = if cfg!(windows) {
        if slow_planner {
            format!("ping -n 20 127.0.0.1 >nul & type {}", plan_path.display())
        } else {
            format!("type {}", plan_path.display())
        }
    } else if slow_planner {
        format!("sleep 20; cat '{}'", plan_path.display())
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
    for (name, script) in [
        ("planner-test", planner.as_str()),
        ("worker-test", worker.as_str()),
        ("reviewer-test", reviewer.as_str()),
    ] {
        config.agents.insert(name.into(), command_entry(script));
    }
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
    config.write_atomic(root).unwrap();
}

fn init_git(root: &Path) {
    std::fs::write(root.join("README.md"), "API fixture\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "factory@example.test"][..],
        &["config", "user.name", "Factory API Test"][..],
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

async fn wait_for_run(root: &Path, id: i64, expected: RunStatus) {
    // Worktree creation can be slow on loaded Windows CI hosts. Keep a finite
    // deadline, but do not make the workflow assertion depend on filesystem speed.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let status = factory_core::Factory::open(root)
            .unwrap()
            .get_run(id)
            .unwrap()
            .unwrap()
            .status;
        if status == expected {
            return;
        }
        if matches!(
            status,
            RunStatus::Failed | RunStatus::Blocked | RunStatus::Cancelled
        ) && status != expected
        {
            let factory = factory_core::Factory::open(root).unwrap();
            panic!(
                "workflow reached {status:?}; sessions: {:#?}; attempts: {:#?}",
                factory.list_agent_sessions(Some(id)).unwrap(),
                factory.list_task_attempts(id).unwrap()
            );
        }
        assert!(Instant::now() < deadline, "workflow remained {status:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn bind_returns_a_nonblocking_listener() {
    let listener = factory_api::bind(0).unwrap();
    let address = listener.local_addr().unwrap();
    let (result_tx, result_rx) = std::sync::mpsc::channel();

    let accept_thread = std::thread::spawn(move || {
        let result = listener.accept().map(|_| ()).map_err(|error| error.kind());
        result_tx.send(result).unwrap();
    });

    let result = match result_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            // Unblock and join the worker before failing, so a regression cannot hang CI.
            let _ = TcpStream::connect_timeout(&address, Duration::from_secs(1));
            let _ = result_rx.recv_timeout(Duration::from_secs(1));
            accept_thread.join().unwrap();
            panic!("accept blocked; listener was not configured as non-blocking");
        }
        Err(error) => panic!("accept worker disconnected: {error}"),
    };

    accept_thread.join().unwrap();
    assert_eq!(result.unwrap_err(), std::io::ErrorKind::WouldBlock);
}

async fn live_get(address: SocketAddr, path: &'static str, cookie: Option<String>) -> String {
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10)).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let cookie = cookie.map_or_else(String::new, |value| format!("Cookie: {value}\r\n"));
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\n{cookie}Connection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = Vec::new();
        let mut chunk = [0; 1024];
        while !http_response_is_complete(&response) {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8(response).unwrap()
    })
    .await
    .unwrap()
}

async fn live_bootstrap(address: SocketAddr) -> String {
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10)).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            stream,
            "POST /api/auth/session HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
        let mut response = Vec::new();
        let mut chunk = [0; 1024];
        while !http_response_is_complete(&response) {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
        }
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let mut session = None;
        let mut csrf = None;
        for line in response.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if !name.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            let value = value.trim();
            if let Some(value) = value.strip_prefix("factory_session=") {
                session = Some(value.split(';').next().unwrap().to_string());
            }
            if let Some(value) = value.strip_prefix("factory_csrf=") {
                csrf = Some(value.split(';').next().unwrap().to_string());
            }
        }
        format!(
            "factory_session={}; factory_csrf={}",
            session.expect("session cookie"),
            csrf.expect("csrf cookie")
        )
    })
    .await
    .unwrap()
}

fn http_response_is_complete(response: &[u8]) -> bool {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let content_length = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    content_length.is_some_and(|length| response.len() >= header_end + 4 + length)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_server_handles_sequential_api_requests() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let listener = factory_api::bind(0).unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(factory_api::serve(make_state(dir.path()), listener));

    let health = live_get(address, "/api/health", None).await;
    assert!(health.starts_with("HTTP/1.1 200 OK"));
    assert!(health.contains(r#"{"status":"ok"}"#));

    let unauthorized = live_get(address, "/api/runs", None).await;
    assert!(unauthorized.starts_with("HTTP/1.1 401 Unauthorized"));

    let cookie = live_bootstrap(address).await;
    let runs = live_get(address, "/api/runs", Some(cookie)).await;
    assert!(runs.starts_with("HTTP/1.1 200 OK"));
    let runs_body = runs.split_once("\r\n\r\n").unwrap().1;
    assert!(serde_json::from_str::<serde_json::Value>(runs_body)
        .unwrap()
        .is_array());

    let health_again = live_get(address, "/api/health", None).await;
    assert!(health_again.starts_with("HTTP/1.1 200 OK"));
    assert!(health_again.contains(r#"{"status":"ok"}"#));

    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

async fn put_config(app: axum::Router, config: serde_json::Value) -> axum::http::Response<Body> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let etag = response
        .headers()
        .get("etag")
        .expect("configuration revision ETag")
        .to_str()
        .unwrap()
        .to_string();
    app.oneshot(
        Request::builder()
            .method("PUT")
            .uri("/api/config")
            .header("content-type", "application/json")
            .header("if-match", etag)
            .body(Body::from(config.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn config_round_trips_through_put_and_get() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    let app = common::authenticated_app(state.clone()).await;

    let config = json!({
        "agents": {
            "codex": { "command": "codex", "args": ["exec"], "env": { "TEST": "1" } }
        },
        "roles": {},
        "role_assignments": [
            { "role": "planner", "agent": "codex", "preferred": true }
        ]
    });
    let response = put_config(app.clone(), config).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let config = factory_core::Config::load(dir.path()).unwrap();
    assert_eq!(config.agent_for_role("planner").as_deref(), Some("codex"));
    let entry = config.agents.get("codex").unwrap();
    assert_eq!(entry.args, vec!["exec".to_string()]);
    assert_eq!(entry.env.get("TEST").map(|s| s.as_str()), Some("1"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value["role_assignments"][0]["role"], "planner");
    assert_eq!(value["role_assignments"][0]["agent"], "codex");
    assert_eq!(value["role_assignments"][0]["preferred"], true);
    assert_eq!(value["agents"]["codex"]["args"][0], "exec");
}

#[tokio::test]
async fn editing_agent_config_preserves_structured_fields_and_rejects_invalid_writes_atomically() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let initial = json!({
        "agents": {
            "codex": {
                "kind": "custom",
                "command": "opencode",
                "args": ["run", "--model", "openai/gpt-5", "{mission}", "--label hello world"],
                "env": { "FACTORY_TEST": "1" },
                "prompt_transport": "argument",
                "interactive_args": ["--interactive"],
                "capabilities": ["planning"],
                "max_concurrency": 4
            }
        },
        "roles": {},
        "role_assignments": [
            { "role": "planner", "agent": "codex", "preferred": true }
        ]
    });
    let response = put_config(app.clone(), initial).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let renamed = json!({
        "agents": {
            "renamed-codex": {
                "kind": "custom",
                "command": "opencode",
                "args": ["run", "--model", "openai/gpt-5", "{mission}", "--label hello world"],
                "env": { "FACTORY_TEST": "1" },
                "prompt_transport": "argument",
                "interactive_args": ["--interactive"],
                "capabilities": ["planning"],
                "max_concurrency": 4
            }
        },
        "roles": {},
        "role_assignments": [
            { "role": "planner", "agent": "renamed-codex", "preferred": true }
        ]
    });
    let response = put_config(app.clone(), renamed).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let config = Config::load(dir.path()).unwrap();
    assert!(!config.agents.contains_key("codex"));
    assert_eq!(
        config.agent_for_role("planner").as_deref(),
        Some("renamed-codex")
    );
    let entry = config.agents.get("renamed-codex").unwrap();
    assert_eq!(entry.command, "opencode");
    assert_eq!(
        entry.args,
        vec![
            "run",
            "--model",
            "openai/gpt-5",
            "{mission}",
            "--label hello world"
        ]
    );
    assert_eq!(entry.env.get("FACTORY_TEST").map(String::as_str), Some("1"));
    assert_eq!(entry.max_concurrency, Some(4));

    let invalid = json!({
        "agents": {
            "renamed-codex": {
                "command": "changed",
                "args": ["different"],
                "max_concurrency": 33
            }
        },
        "roles": {},
        "role_assignments": [
            { "role": "planner", "agent": "renamed-codex", "preferred": true }
        ]
    });
    let response = put_config(app, invalid).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let config = Config::load(dir.path()).unwrap();
    let entry = config.agents.get("renamed-codex").unwrap();
    assert_eq!(entry.command, "opencode");
    assert_eq!(entry.args[4], "--label hello world");
    assert_eq!(entry.max_concurrency, Some(4));
}

#[tokio::test]
async fn role_assignment_can_be_reassigned_to_another_configured_agent() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let config = json!({
        "agents": {
            "codex": { "command": "codex" },
            "opencode": { "command": "opencode" }
        },
        "roles": {},
        "role_assignments": [
            { "role": "planner", "agent": "codex", "preferred": true }
        ]
    });
    let response = put_config(app.clone(), config).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let reassigned = json!({
        "agents": {
            "codex": { "command": "codex" },
            "opencode": { "command": "opencode" }
        },
        "roles": {},
        "role_assignments": [
            { "role": "planner", "agent": "opencode", "preferred": true }
        ]
    });
    let response = put_config(app, reassigned).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let config = factory_core::Config::load(dir.path()).unwrap();
    assert_eq!(
        config.agent_for_role("planner").as_deref(),
        Some("opencode")
    );
}

#[tokio::test]
async fn rejects_invalid_configuration_writes() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let config = json!({
        "agents": {},
        "roles": {},
        "role_assignments": [ { "role": "planner", "agent": "ghost" } ]
    });
    let response = put_config(app.clone(), config).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = body_text(response).await;
    assert!(text.contains("unknown agent 'ghost'"));

    let config = factory_core::Config::load(dir.path()).unwrap();
    assert!(config.agents.contains_key("codex"));
}

#[tokio::test]
async fn lists_agents_with_availability() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let names: Vec<&str> = value
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|agent| agent["name"].as_str())
        .collect();
    assert!(names.contains(&"codex"));
    assert!(value.as_array().unwrap()[0]["available"].is_boolean());
    assert!(value.as_array().unwrap()[0]["pathEntriesChecked"].is_number());
    assert!(value.as_array().unwrap()[0]
        .as_object()
        .unwrap()
        .contains_key("resolvedExecutable"));
    assert!(value.as_array().unwrap()[0]["promptTransport"].is_string());
    assert!(value.as_array().unwrap()[0]["invocationSummary"].is_string());
    assert!(value.as_array().unwrap()[0]["workflowDiagnostic"].is_object());
    assert!(value.as_array().unwrap()[0]["interactiveDiagnostic"].is_object());
}

#[tokio::test]
async fn agent_diagnostics_are_actionable_and_redacted() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let marker = dir.path().join("agent-launched.txt");
    let known_good = if cfg!(windows) { "cmd" } else { "sh" };
    let launcher_args = if cfg!(windows) {
        json!(["/d", "/c", format!("echo launched> {}", marker.display())])
    } else {
        json!(["-c", format!("touch '{}'", marker.display())])
    };
    let config = json!({
        "agents": {
            "missing": {
                "command": "definitely-not-a-real-factory-test-binary",
                "interactive_args": []
            },
            "disabled": {
                "command": known_good,
                "prompt_transport": "disabled",
                "interactive_args": []
            },
            "secret": {
                "command": known_good,
                "args": ["--api-key", "super-secret-token"],
                "env": { "API_TOKEN": "env-secret-value" },
                "prompt_transport": "argument",
                "interactive_args": []
            },
            "launcher": {
                "command": known_good,
                "args": launcher_args,
                "interactive_args": []
            }
        },
        "roles": {},
        "role_assignments": []
    });
    let response = put_config(app.clone(), config).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    let agents = value.as_array().unwrap();
    let find = |name: &str| agents.iter().find(|agent| agent["name"] == name).unwrap();

    let missing = find("missing");
    assert_eq!(missing["available"], false);
    assert_eq!(missing["status"], "missing");
    assert_eq!(
        missing["workflowDiagnostic"]["code"],
        "executable_not_found"
    );
    assert_eq!(
        missing["interactiveDiagnostic"]["code"],
        "executable_not_found"
    );
    assert!(missing["workflowDiagnostic"]["message"]
        .as_str()
        .unwrap()
        .contains("PATH visible to Factory"));
    assert!(missing["workflowDiagnostic"]["action"]
        .as_str()
        .unwrap()
        .contains("Install"));

    let disabled = find("disabled");
    assert_eq!(disabled["available"], true);
    assert_eq!(disabled["promptTransport"], "disabled");
    assert_eq!(disabled["promptTransportConfigured"], true);
    assert_eq!(disabled["workflowAvailable"], false);
    assert_eq!(
        disabled["workflowDiagnostic"]["code"],
        "workflow_transport_disabled"
    );
    assert_eq!(disabled["interactiveAvailable"], true);
    assert_eq!(disabled["interactiveDiagnostic"]["code"], "available");

    let secret = find("secret");
    assert_eq!(secret["promptTransport"], "argument");
    assert_eq!(secret["argumentCount"], 2);
    assert_eq!(
        secret["args"],
        json!(["<value withheld>", "<value withheld>"])
    );
    assert!(secret["invocationSummary"]
        .as_str()
        .unwrap()
        .contains("2 arguments, values withheld"));
    assert!(!body.contains("super-secret-token"));
    assert!(!body.contains("env-secret-value"));

    assert!(
        !marker.exists(),
        "availability checks must not launch agents"
    );
}

#[cfg(feature = "embedded-dashboard")]
#[tokio::test]
async fn serves_the_embedded_dashboard_and_spa_fallback() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("id=\"root\""));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("id=\"root\""));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[cfg(not(feature = "embedded-dashboard"))]
#[tokio::test]
async fn serves_the_dashboard_index_from_dist() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let dist = dir.path().join("apps").join("dashboard").join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), "<h1>dashboard works</h1>").unwrap();
    std::fs::write(dist.join("asset.js"), "// nothing").unwrap();

    let app = common::authenticated_app(make_state(dir.path())).await;
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("dashboard works"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/asset.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn graph_workspace_round_trips_and_rejects_unknown_endpoints() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/graph/workspace")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["nodes"], json!({}));

    let workspace = json!({
        "version": 1,
        "nodes": {
            "agent:codex": { "x": 418, "y": 216 },
            "group:production": { "x": 260, "y": 180 }
        },
        "customNodes": [
            { "id": "group:production", "kind": "group", "label": "Production" }
        ],
        "edges": [
            {
                "id": "edge:membership:one",
                "source": "agent:codex",
                "target": "group:production",
                "kind": "membership"
            }
        ]
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/graph/workspace")
                .header("content-type", "application/json")
                .body(Body::from(workspace.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(dir.path().join(".factory").join("graph.json").exists());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/graph/workspace")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value["nodes"]["agent:codex"]["x"], 418.0);
    assert_eq!(value["edges"][0]["kind"], "membership");

    let invalid = json!({
        "version": 1,
        "nodes": {},
        "customNodes": [],
        "edges": [{
            "id": "edge:custom:missing",
            "source": "agent:codex",
            "target": "agent:missing",
            "kind": "custom"
        }]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/graph/workspace")
                .header("content-type", "application/json")
                .body(Body::from(invalid.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(response).await.contains("unknown target"));
}

#[tokio::test]
async fn malformed_graph_workspace_returns_a_safe_empty_layout() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    std::fs::write(dir.path().join(".factory").join("graph.json"), "{not json").unwrap();
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/graph/workspace")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value["nodes"], json!({}));
    assert!(value["warning"]
        .as_str()
        .unwrap()
        .contains("malformed graph workspace"));
}

#[tokio::test]
async fn agent_session_endpoints_return_persisted_output_and_close_completed_streams() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    let session = {
        let db = state.db.lock().unwrap();
        db.insert_agent_session(&AgentSession {
            id: 0,
            run_id: None,
            task_id: None,
            attempt_id: None,
            role: "worker".to_string(),
            operation: None,
            agent: "codex".to_string(),
            mode: AgentSessionMode::Automated,
            command: "codex exec".to_string(),
            status: "success".to_string(),
            started_at: "2026-08-13T08:00:00Z".to_string(),
            finished_at: Some("2026-08-13T08:00:02Z".to_string()),
            exit_code: Some(0),
            duration_ms: Some(2_000),
            stdout: Some("compiled\n".to_string()),
            stderr: Some("warning\n".to_string()),
            policy_audit: None,
            output_meta: None,
        })
        .unwrap()
    };
    let app = common::authenticated_app(state).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/codex/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value[0]["id"], session.id);
    assert_eq!(value[0]["stdout"], "compiled\n");
    assert_eq!(value[0]["stderr"], "warning\n");
    assert_eq!(value[0]["interactive"], false);
    assert_eq!(
        value[0]["workingDirectory"],
        dir.path().to_string_lossy().as_ref()
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}/stream", session.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let stream = body_text(response).await;
    assert!(stream.contains("event: session"));
    assert!(stream.contains("compiled"));
    assert!(stream.contains("warning"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(body_text(response)
        .await
        .contains("session 999999 not found"));
}

#[tokio::test]
async fn interactive_session_endpoint_only_starts_a_configured_agent() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let command = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "\\\\");
    let arguments = r#"["--exact", "pty_api_child", "--nocapture"]"#;
    std::fs::write(
        dir.path().join(".factory").join("config.toml"),
        format!(
            r#"
[agents.console-test]
kind = "custom"
command = "{command}"
prompt_transport = "disabled"
interactive_args = {arguments}

[agents.console-test.env]
FACTORY_API_PTY_CHILD = "1"
"#
        ),
    )
    .unwrap();
    let state = make_state(dir.path());
    let app = common::authenticated_app(state.clone()).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/console-test/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"cols":90,"rows":24}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value["mode"], "interactive");
    assert_eq!(value["interactive"], true);
    let session_id = value["id"].as_i64().unwrap();

    let _ = state.runtime.stop_interactive_session(session_id);
}

#[tokio::test]
async fn configured_idle_agent_has_an_empty_session_history() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agents/codex/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, "[]");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agents/unknown/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn run_list_keeps_the_legacy_array_and_adds_a_paginated_envelope() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    {
        let db = state.db.lock().unwrap();
        let empty = db.create_run("empty run", Some("planner")).unwrap();
        assert_eq!(empty.id, 1);
        let mixed = db.create_run("mixed run", Some("planner")).unwrap();
        for (position, task_state) in [
            TaskState::Completed,
            TaskState::Superseded,
            TaskState::Ready,
        ]
        .into_iter()
        .enumerate()
        {
            db.create_task(
                mixed.id,
                &format!("task {position}"),
                "objective",
                &[],
                task_state,
                position as i32,
                None,
                None,
            )
            .unwrap();
        }
    }
    let app = common::authenticated_app(state).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let legacy: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert!(legacy.is_array());
    assert_eq!(legacy.as_array().unwrap().len(), 2);
    assert_eq!(legacy[0]["id"], 2);
    assert_eq!(legacy[0]["counts"]["total"], 3);
    assert_eq!(legacy[0]["counts"]["completed"], 1);
    assert_eq!(legacy[0]["counts"]["superseded"], 1);
    assert_eq!(legacy[0]["counts"]["ready"], 1);
    assert_eq!(legacy[1]["id"], 1);
    assert_eq!(legacy[1]["counts"]["total"], 0);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["id"], 2);
    assert_eq!(page["items"][0]["counts"]["total"], 3);
    assert_eq!(page["nextCursor"], 2);
    assert_eq!(page["hasMore"], true);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs?limit=1&before=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["id"], 1);
    assert_eq!(page["items"][0]["counts"]["total"], 0);
    assert_eq!(page["nextCursor"], serde_json::Value::Null);
    assert_eq!(page["hasMore"], false);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/runs/2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(detail["run"]["id"], 2);
    assert_eq!(detail["tasks"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn run_list_rejects_invalid_pagination_parameters() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    let app = common::authenticated_app(state).await;

    for uri in [
        "/api/runs?limit=0",
        "/api/runs?limit=101",
        "/api/runs?limit=nope",
        "/api/runs?limit=",
        "/api/runs?before=0",
        "/api/runs?before=-1",
        "/api/runs?before=nope",
        "/api/runs?before=",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let body: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert!(body["error"].is_string(), "{uri}: {body}");
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/runs?before=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 0);
    assert_eq!(page["nextCursor"], serde_json::Value::Null);
    assert_eq!(page["hasMore"], false);
}

#[tokio::test]
async fn run_list_paginates_a_large_history_without_gaps_or_duplicates() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    {
        let db = state.db.lock().unwrap();
        for index in 0..1_000 {
            let run = db
                .create_run(&format!("run {index}"), Some("planner"))
                .unwrap();
            db.create_task(
                run.id,
                "task",
                "objective",
                &[],
                if index % 2 == 0 {
                    TaskState::Completed
                } else {
                    TaskState::Pending
                },
                0,
                None,
                None,
            )
            .unwrap();
        }
    }
    let app = common::authenticated_app(state).await;

    let mut seen = Vec::new();
    let mut before = None;
    loop {
        let uri = match before {
            Some(cursor) => format!("/api/runs?limit=25&before={cursor}"),
            None => "/api/runs?limit=25".to_string(),
        };
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let page: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
        let items = page["items"].as_array().unwrap();
        assert!(items.len() <= 25);
        for item in items {
            seen.push(item["id"].as_i64().unwrap());
        }
        if !page["hasMore"].as_bool().unwrap() {
            assert_eq!(page["nextCursor"], serde_json::Value::Null);
            break;
        }
        before = Some(page["nextCursor"].as_i64().unwrap());
    }

    assert_eq!(seen.len(), 1_000);
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 1_000);
    assert_eq!(seen.first(), Some(&1_000));
    assert_eq!(seen.last(), Some(&1));
    assert!(seen.windows(2).all(|pair| pair[0] > pair[1]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_api_plans_and_executes_through_factory_core() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_test_agents(dir.path(), false);
    init_git(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"objective": "Exercise the API"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let run_id = run["id"].as_i64().unwrap();
    assert_eq!(run["status"], "planning");
    wait_for_run(dir.path(), run_id, RunStatus::Planned).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/runs/{run_id}/start"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let team: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(team["planner"], "planner-test");
    assert_eq!(team["workers"][0], "worker-test");
    assert_eq!(team["reviewers"][0], "reviewer-test");
    wait_for_run(dir.path(), run_id, RunStatus::Completed).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(detail["run"]["status"], "completed");
    assert_eq!(detail["tasks"][0]["state"], "completed");
    assert_eq!(detail["attempts"][0]["status"], "approved");
    assert_eq!(detail["sessions"].as_array().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn role_aware_artifacts_and_stages_are_exposed() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    init_git(dir.path());
    let plan = r#"{"objective":"API roles","tasks":[
    {"id":"T1","title":"Research","objective":"understand","dependencies":[],"acceptanceCriteria":["findings"],"role":"researcher","operation":"advisory"},
    {"id":"T2","title":"Implement","objective":"build","dependencies":["T1"],"acceptanceCriteria":["works"]}]}"#;
    let plan_path = dir.path().join("plan.json");
    let research_path = dir.path().join("research.json");
    std::fs::write(&plan_path, plan).unwrap();
    std::fs::write(
        &research_path,
        r#"{"summary":"JWT findings","findings":["httpOnly"],"recommendations":["middleware"]}"#,
    )
    .unwrap();
    let worker_output = dir.path().join("worker-output.json");
    std::fs::write(&worker_output, r#"{"commands":["cargo check"]}"#).unwrap();
    let reviewer_output = dir.path().join("reviewer.json");
    std::fs::write(&reviewer_output, r#"{"decision":"approve","reason":"ok"}"#).unwrap();

    let mut config = Config::default();
    let insert_agent = |config: &mut Config, name: &str, script: String| {
        config.agents.insert(name.into(), command_entry(&script));
    };
    // The fake agents print fixture files the same way on every platform:
    // `cat` on unix, `type` on Windows. Producers also touch a marker file so
    // implementation tasks produce real repository evidence.
    let cat = |path: &Path| -> String {
        if cfg!(windows) {
            format!("type {}", path.display())
        } else {
            format!("cat '{}'", path.display())
        }
    };
    let producer = |marker: &str, file: &str, path: &Path| -> String {
        if cfg!(windows) {
            format!("echo {marker}>{file} & type {}", path.display())
        } else {
            format!("printf '{marker}\\n' > {file}; cat '{}'", path.display())
        }
    };
    insert_agent(&mut config, "planner-api", cat(&plan_path));
    insert_agent(
        &mut config,
        "researcher-api",
        producer("found", "research.txt", &research_path),
    );
    insert_agent(
        &mut config,
        "worker-api",
        producer("done", "worker-output.txt", &worker_output),
    );
    insert_agent(&mut config, "reviewer-api", cat(&reviewer_output));
    for (role, agent, preferred) in [
        ("planner", "planner-api", true),
        ("worker", "worker-api", true),
        ("reviewer", "reviewer-api", true),
        ("researcher", "researcher-api", true),
    ] {
        config.role_assignments.push(RoleAssignment {
            role: role.into(),
            agent: agent.into(),
            preferred,
        });
    }
    config.write_atomic(dir.path()).unwrap();

    let app = common::authenticated_app(make_state(dir.path())).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "objective": "API roles",
                        "team": {
                            "planner": "planner-api",
                            "workers": ["worker-api"],
                            "reviewers": ["reviewer-api"],
                            "additional": { "researcher": ["researcher-api"] }
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let run: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let run_id = run["id"].as_i64().unwrap();
    wait_for_run(dir.path(), run_id, RunStatus::Planned).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/runs/{run_id}/start"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_run(dir.path(), run_id, RunStatus::Completed).await;

    // Run detail exposes derived stages and persisted artifacts.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let detail: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let stages = detail["stages"].as_array().unwrap();
    assert!(
        stages.iter().any(|stage| stage["key"] == "analysis"),
        "advisory stage derived: {stages:?}"
    );
    assert!(stages.iter().any(|stage| stage["key"] == "implementation"));
    assert_eq!(detail["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(detail["artifacts"][0]["kind"], "research");
    assert_eq!(detail["tasks"][0]["operation"], "advisory");
    assert_eq!(detail["tasks"][1]["operation"], "implement");
    assert_eq!(detail["attempts"][0]["operation"], "advisory");

    // The dedicated artifact endpoints serve the same records.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{run_id}/artifacts"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let artifacts: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(artifacts.as_array().unwrap().len(), 1);
    assert!(artifacts[0]["content"].to_string().contains("httpOnly"));

    let research_task_id = detail["tasks"][0]["id"].as_i64().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/tasks/{research_task_id}/artifacts"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let artifacts: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(artifacts.as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_cancel_stops_a_known_planning_session() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_test_agents(dir.path(), true);
    let state = make_state(dir.path());
    let app = common::authenticated_app(state.clone()).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"objective": "Cancel planning"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let run: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let run_id = run["id"].as_i64().unwrap();

    let session_deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let sessions = factory_core::Factory::open(dir.path())
            .unwrap()
            .list_agent_sessions(Some(run_id))
            .unwrap();
        if sessions.iter().any(|session| session.status == "running") {
            break;
        }
        assert!(
            Instant::now() < session_deadline,
            "planner session did not start"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/runs/{run_id}/cancel"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    wait_for_run(dir.path(), run_id, RunStatus::Cancelled).await;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !state.runtime.active_operations().is_empty() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/runs/{run_id}/start"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(response).await.contains("while it is cancelled"));
}

#[tokio::test]
async fn role_policy_endpoint_sets_and_clears_presets() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/roles/worker/policy")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "preset": "read_only" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(role["permissions"]["filesystemMode"], "read_only");
    assert_eq!(
        role["permissions"]["writeScopes"].as_array().unwrap().len(),
        0
    );

    // The preset is persisted as a project-local policy in config.toml.
    let config = factory_core::Config::load(dir.path()).unwrap();
    let policy = config.effective_policy("worker", "codex");
    assert!(policy.filesystem.read_only());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/roles/worker/policy")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "preset": null }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(
        role["permissions"]["filesystemMode"], "open",
        "clearing the preset removes the role scope entirely"
    );
    assert_eq!(role["permissions"]["permissive"], true);

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/roles/worker/policy")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "preset": " sideways" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn custom_roles_can_select_a_policy_preset_at_creation() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/roles")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "Docs Specialist",
                        "description": "Writes documentation.",
                        "executionClass": "post_process",
                        "instructions": "Update the docs.",
                        "policyPreset": "documentation"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(role["permissions"]["filesystemMode"], "restricted");
    assert_eq!(
        role["permissions"]["writeScopes"],
        json!(["README.md", "docs/**"])
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/roles/docs_specialist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let config = factory_core::Config::load(dir.path()).unwrap();
    assert!(
        !config.policies.roles.contains_key("docs_specialist"),
        "deleting a role must not leave its policy behind"
    );
}

#[tokio::test]
async fn graph_exposes_effective_permissions_for_agents_and_roles() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/graph")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let graph: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let worker_role = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "role:worker")
        .expect("worker role node");
    assert_eq!(worker_role["meta"]["permissions"]["filesystemMode"], "open");
    assert_eq!(
        worker_role["meta"]["permissions"]["gitDenied"],
        json!([
            "push",
            "force_push",
            "delete_branch",
            "reset_branch",
            "modify_remotes"
        ])
    );
    let agent_node = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "agent")
        .expect("at least one agent node");
    assert_eq!(
        agent_node["meta"]["permissions"]["networkEnforcement"], "advisory",
        "the graph must not claim sandbox-level network isolation"
    );
}
