use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_core::{AgentEntry, Config, RoleAssignment};
use factory_policy::ToolPolicy;
use factory_types::{McpToolMetadata, ToolClassification};
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

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
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

async fn json_body(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn metadata(name: &str, classification: ToolClassification) -> McpToolMetadata {
    McpToolMetadata {
        server: "fake".into(),
        name: name.into(),
        qualified_name: format!("fake.{name}"),
        description: format!("Fake {name} tool."),
        input_schema: json!({"type": "object"}),
        annotations: None,
        classification,
        discovered_at: "2026-08-23T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn mcp_server_crud_is_semantic_and_never_resolves_secret_values() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    let app = common::authenticated_app(state.clone()).await;
    let create = request(
        app.clone(),
        "POST",
        "/api/mcp/servers",
        Some(json!({
            "id": "fake",
            "transport": "stdio",
            "command": "definitely-not-an-executable",
            "args": [],
            "env": {"TOKEN": "${FACTORY_TEST_TOKEN}"},
            "timeout_ms": 100,
            "discovery_timeout_ms": 100,
            "max_response_bytes": 4096,
            "max_concurrency": 2
        })),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let created = json_body(create).await;
    assert_eq!(created["status"], "unknown");
    assert_eq!(created["env"]["TOKEN"], "${FACTORY_TEST_TOKEN}");

    let listed = request(app.clone(), "GET", "/api/mcp/servers", None).await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = json_body(listed).await;
    assert_eq!(listed[0]["env"]["TOKEN"], "${FACTORY_TEST_TOKEN}");

    // Diagnostic testing reports Unavailable as state; it does not crash the
    // API or expose a generic invocation surface.
    let tested = request(app.clone(), "POST", "/api/mcp/servers/fake/test", None).await;
    assert_eq!(tested.status(), StatusCode::OK);
    assert_eq!(json_body(tested).await["status"], "unavailable");
    let raw_call = request(app.clone(), "POST", "/api/mcp/call", Some(json!({}))).await;
    assert!(matches!(
        raw_call.status(),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ));

    let updated = request(
        app.clone(),
        "PUT",
        "/api/mcp/servers/fake",
        Some(json!({
            "transport": "http",
            "url": "http://127.0.0.1:9/mcp",
            "headers": {"Authorization": "${FACTORY_TEST_TOKEN}"},
            "timeout_ms": 100,
            "discovery_timeout_ms": 100,
            "max_response_bytes": 4096,
            "max_concurrency": 1
        })),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    assert_eq!(json_body(updated).await["transport"], "http");

    let deleted = request(app, "DELETE", "/api/mcp/servers/fake", None).await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn role_and_agent_inspectors_receive_effective_concrete_permissions() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let mut config = Config::load(dir.path()).unwrap();
    config.agents.clear();
    config.role_assignments.clear();
    config.agents.insert(
        "codex".into(),
        AgentEntry {
            kind: None,
            command: "codex".into(),
            args: vec!["exec".into()],
            env: Default::default(),
            prompt_transport: None,
            interactive_args: None,
            capabilities: Vec::new(),
            max_concurrency: None,
        },
    );
    config.role_assignments.push(RoleAssignment {
        role: "worker".into(),
        agent: "codex".into(),
        preferred: true,
    });
    config.policies.role_mut("worker").tools = Some(ToolPolicy {
        allow: vec!["fake.*".into(), "fake.mutate".into()],
        deny: vec!["fake.erase".into()],
    });
    config.mcp.insert(
        "fake".into(),
        factory_mcp::McpServerConfig {
            transport: factory_mcp::McpTransport::Stdio,
            command: Some("fake".into()),
            args: Vec::new(),
            cwd: None,
            url: None,
            env: Default::default(),
            headers: Default::default(),
            timeout_ms: 100,
            discovery_timeout_ms: 100,
            max_response_bytes: 4096,
            max_concurrency: 1,
            classifications: Default::default(),
        },
    );
    config.write_atomic(dir.path()).unwrap();
    let state = make_state(dir.path());
    state
        .db
        .lock()
        .unwrap()
        .replace_mcp_tools(
            "fake",
            &[
                metadata("search", ToolClassification::Read),
                metadata("mutate", ToolClassification::Write),
                metadata("erase", ToolClassification::Destructive),
                metadata("ambiguous", ToolClassification::Unknown),
            ],
        )
        .unwrap();
    let app = common::authenticated_app(state).await;

    for uri in ["/api/roles/worker/tools", "/api/agents/codex/tools"] {
        let response = request(app.clone(), "GET", uri, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let rows = json_body(response).await;
        let rows = rows.as_array().unwrap();
        let allowed = |name: &str| {
            rows.iter()
                .find(|row| row["qualifiedName"] == name)
                .unwrap()["allowed"]
                .as_bool()
                .unwrap()
        };
        assert!(allowed("fake.search"));
        assert!(allowed("fake.mutate"));
        assert!(!allowed("fake.erase"));
        assert!(!allowed("fake.ambiguous"));
    }
}
