use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_core::Config;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

fn init_root(root: &Path) {
    factory_core::Factory::init(root).unwrap();
}

fn make_state(root: &Path) -> Arc<factory_api::ApiState> {
    Arc::new(factory_api::ApiState::new(root.to_path_buf()).unwrap())
}

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.oneshot(builder.body(body).unwrap()).await.unwrap()
}

async fn text(response: axum::http::Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn stdio_server(id: &str, command: &str) -> Value {
    json!({
        "id": id,
        "transport": "stdio",
        "command": command,
        "args": [],
        "timeout_ms": 60000,
        "discovery_timeout_ms": 10000,
        "max_response_bytes": 262144,
        "max_concurrency": 4
    })
}

/// GET /api/config returns the current configuration revision as an ETag.
async fn current_revision(app: axum::Router) -> String {
    let response = request(app, "GET", "/api/config", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get("etag")
        .expect("ETag on GET /api/config")
        .to_str()
        .unwrap()
        .to_string()
}

async fn put_config_with_revision(
    app: axum::Router,
    revision: &str,
    body: Value,
) -> axum::http::Response<Body> {
    request_with_if_match(app, "PUT", "/api/config", Some(body), revision).await
}

async fn request_with_if_match(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    revision: &str,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("if-match", revision);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.oneshot(builder.body(body).unwrap()).await.unwrap()
}

fn loaded(root: &Path) -> Config {
    Config::load(root).unwrap()
}

#[tokio::test]
async fn get_config_exposes_a_stable_revision_that_changes_on_write() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let first = current_revision(app.clone()).await;
    let second = current_revision(app.clone()).await;
    assert_eq!(first, second, "revision must be stable without a write");

    let response = request(
        app.clone(),
        "POST",
        "/api/roles",
        Some(json!({
            "name": "Concurrency Probe",
            "description": "Exercises revision bumping.",
            "executionClass": "advisory",
            "agents": ["codex"]
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let third = current_revision(app).await;
    assert_ne!(first, third, "revision must change after a mutation");
}

#[tokio::test]
async fn concurrent_mcp_server_creations_both_survive() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let left = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/mcp/servers",
                Some(stdio_server("alpha", "alpha-mcp")),
            )
            .await
        }
    });
    let right = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/mcp/servers",
                Some(stdio_server("beta", "beta-mcp")),
            )
            .await
        }
    });
    for outcome in [left.await.unwrap(), right.await.unwrap()] {
        assert_eq!(
            outcome.status(),
            StatusCode::CREATED,
            "both additions must succeed"
        );
    }

    let config = loaded(dir.path());
    assert!(config.mcp.contains_key("alpha"), "alpha was lost");
    assert!(config.mcp.contains_key("beta"), "beta was lost");
}

#[tokio::test]
async fn concurrent_identical_mcp_server_creations_yield_one_success_one_conflict() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let left = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/mcp/servers",
                Some(stdio_server("same", "same-mcp")),
            )
            .await
        }
    });
    let right = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/mcp/servers",
                Some(stdio_server("same", "same-mcp")),
            )
            .await
        }
    });
    let statuses = [left.await.unwrap().status(), right.await.unwrap().status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one creation may succeed, got {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1
    );
    assert!(loaded(dir.path()).mcp.contains_key("same"));
}

#[tokio::test]
async fn concurrent_role_and_mcp_changes_do_not_overwrite_each_other() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let role = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/roles",
                Some(json!({
                    "name": "Release Manager",
                    "description": "Coordinates releases.",
                    "executionClass": "advisory",
                    "agents": ["codex"]
                })),
            )
            .await
        }
    });
    let mcp = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/mcp/servers",
                Some(stdio_server("docs", "docs-mcp")),
            )
            .await
        }
    });
    assert_eq!(role.await.unwrap().status(), StatusCode::CREATED);
    assert_eq!(mcp.await.unwrap().status(), StatusCode::CREATED);

    let config = loaded(dir.path());
    assert!(
        config.roles.contains_key("release_manager"),
        "role was lost"
    );
    assert!(config.mcp.contains_key("docs"), "MCP server was lost");
}

#[tokio::test]
async fn concurrent_assignment_changes_preserve_unrelated_assignments() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    {
        let mut config = loaded(dir.path());
        config.agents.insert(
            "gemini".into(),
            factory_core::AgentEntry {
                kind: None,
                command: "gemini".into(),
                args: vec![],
                env: Default::default(),
                prompt_transport: None,
                interactive_args: None,
                capabilities: Vec::new(),
                max_concurrency: None,
            },
        );
        config.write_atomic(dir.path()).unwrap();
    }
    let app = common::authenticated_app(make_state(dir.path())).await;

    let reviewer = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/roles/reviewer/assignments",
                Some(json!({ "agent": "gemini", "preferred": false })),
            )
            .await
        }
    });
    let worker = tokio::spawn({
        let app = app.clone();
        async move {
            request(
                app,
                "POST",
                "/api/roles/worker/assignments",
                Some(json!({ "agent": "gemini", "preferred": false })),
            )
            .await
        }
    });
    for outcome in [reviewer.await.unwrap(), worker.await.unwrap()] {
        assert_eq!(outcome.status(), StatusCode::OK);
    }

    let config = loaded(dir.path());
    assert!(
        config
            .role_assignments
            .iter()
            .any(|assignment| assignment.role == "reviewer" && assignment.agent == "gemini"),
        "reviewer assignment was lost"
    );
    assert!(
        config
            .role_assignments
            .iter()
            .any(|assignment| assignment.role == "worker" && assignment.agent == "gemini"),
        "worker assignment was lost"
    );
    assert!(
        config
            .role_assignments
            .iter()
            .any(|assignment| assignment.role == "planner" && assignment.agent == "codex"),
        "planner assignment must survive"
    );
}

#[tokio::test]
async fn stale_full_config_put_is_rejected_and_leaves_the_file_unchanged() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let stale = current_revision(app.clone()).await;

    let response = request(
        app.clone(),
        "POST",
        "/api/mcp/servers",
        Some(stdio_server("docs", "docs-mcp")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let fresh_revision = current_revision(app.clone()).await;
    assert_ne!(stale, fresh_revision);

    let stale_config: Value = {
        let response = request(app.clone(), "GET", "/api/config", None).await;
        serde_json::from_str(&text(response).await).unwrap()
    };
    let mut stale_config = stale_config;
    // Drop the server added in between: this document is based on the old state.
    stale_config["mcp"]
        .as_object_mut()
        .expect("mcp map")
        .remove("docs");

    let response = put_config_with_revision(app.clone(), &stale, stale_config).await;
    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
    let error = text(response).await;
    assert!(error.contains("modified"), "unexpected error body: {error}");

    let config = loaded(dir.path());
    assert!(
        config.mcp.contains_key("docs"),
        "stale PUT must not drop newer changes"
    );

    // The current revision still replaces successfully.
    let mut current: Value = {
        let response = request(app.clone(), "GET", "/api/config", None).await;
        serde_json::from_str(&text(response).await).unwrap()
    };
    let mut extra = stdio_server("extra", "extra-mcp");
    extra.as_object_mut().expect("mutation object").remove("id");
    current["mcp"]
        .as_object_mut()
        .expect("mcp map")
        .insert("extra".into(), extra);
    let response = put_config_with_revision(app.clone(), &fresh_revision, current).await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "{}",
        text(response).await
    );
    assert!(loaded(dir.path()).mcp.contains_key("extra"));
}

#[tokio::test]
async fn unversioned_full_config_put_is_rejected() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let body: Value = {
        let response = request(app.clone(), "GET", "/api/config", None).await;
        serde_json::from_str(&text(response).await).unwrap()
    };
    let response = request(app, "PUT", "/api/config", Some(body)).await;
    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
    let error = text(response).await;
    assert!(error.contains("If-Match"), "unexpected error body: {error}");
}

#[tokio::test]
async fn rejected_validations_leave_the_file_and_revision_unchanged() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;
    let revision_before = current_revision(app.clone()).await;

    let mut body: Value = {
        let response = request(app.clone(), "GET", "/api/config", None).await;
        serde_json::from_str(&text(response).await).unwrap()
    };
    body["role_assignments"] = json!([{ "role": "planner", "agent": "ghost" }]);
    let response = put_config_with_revision(app.clone(), &revision_before, body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(text(response).await.contains("unknown agent 'ghost'"));

    let revision_after = current_revision(app).await;
    assert_eq!(
        revision_before, revision_after,
        "a rejected write must not change the revision"
    );
    let config = loaded(dir.path());
    assert!(config.agents.contains_key("codex"));
    assert!(config
        .role_assignments
        .iter()
        .any(|assignment| assignment.role == "planner" && assignment.agent == "codex"));
}

#[tokio::test]
async fn successful_config_put_returns_a_new_revision() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;
    let before = current_revision(app.clone()).await;

    let mut body: Value = {
        let response = request(app.clone(), "GET", "/api/config", None).await;
        serde_json::from_str(&text(response).await).unwrap()
    };
    body["agents"].as_object_mut().expect("agents map").insert(
        "extra".into(),
        json!({ "command": "extra-agent", "args": [] }),
    );
    let response = put_config_with_revision(app.clone(), &before, body).await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "{}",
        text(response).await
    );
    let etag = response
        .headers()
        .get("etag")
        .expect("new revision ETag")
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(before, etag);
    assert_eq!(etag, current_revision(app).await);
}

#[tokio::test]
async fn independent_project_roots_do_not_share_revisions_or_locks() {
    let left = TempDir::new().unwrap();
    let right = TempDir::new().unwrap();
    init_root(left.path());
    init_root(right.path());
    let left_app = common::authenticated_app(make_state(left.path())).await;
    let right_app = common::authenticated_app(make_state(right.path())).await;

    let left_revision_task = tokio::spawn(current_revision(left_app.clone()));
    let right_revision_task = tokio::spawn(current_revision(right_app.clone()));
    let left_revision = left_revision_task.await.unwrap();
    let right_revision = right_revision_task.await.unwrap();
    // Same default content: revisions match until one project changes.
    assert_eq!(left_revision, right_revision);

    let response = request(
        left_app.clone(),
        "POST",
        "/api/mcp/servers",
        Some(stdio_server("left-only", "left-mcp")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = put_config_with_revision(right_app.clone(), &right_revision, {
        let response = request(right_app.clone(), "GET", "/api/config", None).await;
        serde_json::from_str::<Value>(&text(response).await).unwrap()
    })
    .await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "mutations in one project must not disturb another project"
    );
    assert!(loaded(left.path()).mcp.contains_key("left-only"));
    assert!(!loaded(right.path()).mcp.contains_key("left-only"));
}

#[tokio::test]
async fn parallel_writes_leave_valid_toml_and_no_temporary_files_behind() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let app = common::authenticated_app(make_state(dir.path())).await;

    let mut tasks = Vec::new();
    for index in 0..12 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            request(
                app,
                "POST",
                "/api/mcp/servers",
                Some(stdio_server(&format!("srv{index}"), "bulk-mcp")),
            )
            .await
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().status(), StatusCode::CREATED);
    }

    let config = loaded(dir.path());
    for index in 0..12 {
        assert!(config.mcp.contains_key(&format!("srv{index}")));
    }
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join(".factory"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files leaked: {leftovers:?}"
    );
}
