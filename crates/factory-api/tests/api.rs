use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_db::FactoryDb;
use factory_types::AgentSession;
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

fn init_root(root: &Path) {
    factory_core::Factory::init(root).unwrap();
}

fn make_state(root: &Path) -> Arc<factory_api::ApiState> {
    let db = FactoryDb::open(&root.join(".factory").join("db.sqlite3")).unwrap();
    Arc::new(factory_api::ApiState {
        db: Mutex::new(db),
        root: root.to_path_buf(),
    })
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

async fn live_get(address: SocketAddr, path: &'static str) -> String {
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
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

    let health = live_get(address, "/api/health").await;
    assert!(health.starts_with("HTTP/1.1 200 OK"));
    assert!(health.contains(r#"{"status":"ok"}"#));

    let runs = live_get(address, "/api/runs").await;
    assert!(runs.starts_with("HTTP/1.1 200 OK"));
    let runs_body = runs.split_once("\r\n\r\n").unwrap().1;
    assert!(serde_json::from_str::<serde_json::Value>(runs_body)
        .unwrap()
        .is_array());

    let health_again = live_get(address, "/api/health").await;
    assert!(health_again.starts_with("HTTP/1.1 200 OK"));
    assert!(health_again.contains(r#"{"status":"ok"}"#));

    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn config_round_trips_through_put_and_get() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = factory_api::router(make_state(dir.path()));

    let config = json!({
        "agents": {
            "codex": { "command": "codex", "args": ["exec"], "env": { "TEST": "1" } }
        },
        "roles": { "planner": { "agent": "codex" } }
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(config.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
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
    assert_eq!(value["roles"]["planner"]["agent"], "codex");
    assert_eq!(value["agents"]["codex"]["args"][0], "exec");
}

#[tokio::test]
async fn role_assignment_can_be_reassigned_to_another_configured_agent() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = factory_api::router(make_state(dir.path()));

    let config = json!({
        "agents": {
            "codex": { "command": "codex" },
            "opencode": { "command": "opencode" }
        },
        "roles": { "planner": { "agent": "codex" } }
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(config.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let reassigned = json!({
        "agents": {
            "codex": { "command": "codex" },
            "opencode": { "command": "opencode" }
        },
        "roles": { "planner": { "agent": "opencode" } }
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(reassigned.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
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
    let app = factory_api::router(make_state(dir.path()));

    let config = json!({
        "agents": {},
        "roles": { "planner": { "agent": "ghost" } }
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(config.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
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
    let app = factory_api::router(make_state(dir.path()));

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
}

#[cfg(feature = "embedded-dashboard")]
#[tokio::test]
async fn serves_the_embedded_dashboard_and_spa_fallback() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = factory_api::router(make_state(dir.path()));

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

    let app = factory_api::router(make_state(dir.path()));
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
    let app = factory_api::router(make_state(dir.path()));

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
    let app = factory_api::router(make_state(dir.path()));

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
            role: "worker".to_string(),
            agent: "codex".to_string(),
            command: "codex exec".to_string(),
            status: "success".to_string(),
            started_at: "2026-08-13T08:00:00Z".to_string(),
            finished_at: Some("2026-08-13T08:00:02Z".to_string()),
            exit_code: Some(0),
            duration_ms: Some(2_000),
            stdout: Some("compiled\n".to_string()),
            stderr: Some("warning\n".to_string()),
        })
        .unwrap()
    };
    let app = factory_api::router(state);

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
async fn configured_idle_agent_has_an_empty_session_history() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = factory_api::router(make_state(dir.path()));

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
