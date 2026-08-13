use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_db::FactoryDb;
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
