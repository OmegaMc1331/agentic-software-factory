use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_db::FactoryDb;
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

fn init_root(root: &Path) {
    factory_core::Factory::init(root, false).unwrap();
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
