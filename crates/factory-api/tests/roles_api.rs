use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_core::{AgentEntry, Config, RoleAssignment};
use factory_types::RunStatus;
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

async fn body_text(response: axum::http::Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn init_root(root: &Path) {
    factory_core::Factory::init(root).unwrap();
}

fn make_state(root: &Path) -> Arc<factory_api::ApiState> {
    Arc::new(factory_api::ApiState::new(root.to_path_buf()).unwrap())
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

fn configure_two_agents(root: &Path) {
    let mut config = Config::default();
    config
        .agents
        .insert("codex".into(), command_entry("echo codex"));
    config
        .agents
        .insert("claude".into(), command_entry("echo claude"));
    for (role, agent) in [
        ("planner", "codex"),
        ("worker", "codex"),
        ("reviewer", "claude"),
    ] {
        config.role_assignments.push(RoleAssignment {
            role: role.into(),
            agent: agent.into(),
            preferred: true,
        });
    }
    config.write_atomic(root).unwrap();
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

#[tokio::test]
async fn roles_endpoint_lists_core_and_custom_roles() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_two_agents(dir.path());
    let app = factory_api::router(make_state(dir.path()));

    let response = request(app, "GET", "/api/roles", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let roles: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let roles = roles.as_array().unwrap();
    let ids: Vec<&str> = roles
        .iter()
        .map(|role| role["id"].as_str().unwrap())
        .collect();
    for expected in [
        "planner",
        "worker",
        "reviewer",
        "architect",
        "researcher",
        "test_engineer",
        "security_auditor",
        "documentation_writer",
    ] {
        assert!(ids.contains(&expected), "missing core role {expected}");
    }
    let worker = roles.iter().find(|role| role["id"] == "worker").unwrap();
    assert_eq!(worker["kind"], "core");
    assert_eq!(worker["executionClass"], "execution");
    assert_eq!(worker["available"], true);
    let architect = roles.iter().find(|role| role["id"] == "architect").unwrap();
    assert_eq!(architect["available"], false);
}

#[tokio::test]
async fn custom_role_lifecycle_create_assign_edit_delete() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_two_agents(dir.path());
    let app = factory_api::router(make_state(dir.path()));

    let response = request(
        app.clone(),
        "POST",
        "/api/roles",
        Some(json!({
            "name": "Database Engineer",
            "description": "Designs and modifies relational database schemas.",
            "executionClass": "execution",
            "instructions": "Focus on schema design and migrations.",
            "agents": ["codex"],
            "preferredAgent": "codex"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(role["id"], "database_engineer");
    assert_eq!(role["kind"], "custom");
    assert_eq!(role["assignments"][0]["agent"], "codex");
    assert_eq!(role["assignments"][0]["preferred"], true);

    let response = request(
        app.clone(),
        "POST",
        "/api/roles/database_engineer/assignments",
        Some(json!({ "agent": "claude" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(role["assignments"].as_array().unwrap().len(), 2);

    let response = request(
        app.clone(),
        "DELETE",
        "/api/roles/database_engineer/assignments/codex",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(role["assignments"].as_array().unwrap().len(), 1);
    assert_eq!(role["assignments"][0]["agent"], "claude");

    let response = request(
        app.clone(),
        "POST",
        "/api/roles/database_engineer/assignments",
        Some(json!({ "agent": "codex", "preferred": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let preferred: Vec<_> = role["assignments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|assignment| assignment["preferred"] == true)
        .collect();
    assert_eq!(preferred.len(), 1);

    let response = request(
        app.clone(),
        "PUT",
        "/api/roles/database_engineer",
        Some(json!({
            "name": "Database Engineer",
            "description": "Owns schema, migrations and data integrity.",
            "executionClass": "execution",
            "instructions": "Focus on schema design, migrations and integrity."
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let role: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert!(role["description"]
        .as_str()
        .unwrap()
        .contains("data integrity"));

    let response = request(app.clone(), "DELETE", "/api/roles/database_engineer", None).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let config = factory_core::Config::load(dir.path()).unwrap();
    assert!(!config.roles.contains_key("database_engineer"));
    assert!(config
        .role_assignments
        .iter()
        .all(|assignment| assignment.role != "database_engineer"));
}

#[tokio::test]
async fn core_roles_are_protected_from_redefinition_and_deletion() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_two_agents(dir.path());
    let app = factory_api::router(make_state(dir.path()));

    let response = request(
        app.clone(),
        "PUT",
        "/api/roles/worker",
        Some(json!({
            "name": "Worker",
            "description": "Override attempt",
            "executionClass": "review"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(response).await.contains("cannot be redefined"));

    let response = request(app, "DELETE", "/api/roles/worker", None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(response).await.contains("cannot be deleted"));
}

#[tokio::test]
async fn assignment_validation_rejects_unknown_and_duplicate_agents() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_two_agents(dir.path());
    let app = factory_api::router(make_state(dir.path()));

    let response = request(
        app.clone(),
        "POST",
        "/api/roles/worker/assignments",
        Some(json!({ "agent": "ghost" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = request(
        app.clone(),
        "POST",
        "/api/roles/worker/assignments",
        Some(json!({ "agent": "claude" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = request(
        app.clone(),
        "POST",
        "/api/roles/worker/assignments",
        Some(json!({ "agent": "claude" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = request(
        app,
        "POST",
        "/api/roles/ghost_role/assignments",
        Some(json!({ "agent": "claude" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn graph_shows_one_role_node_with_one_edge_per_assignment() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_two_agents(dir.path());
    let mut config = factory_core::Config::load(dir.path()).unwrap();
    config.role_assignments.push(RoleAssignment {
        role: "worker".into(),
        agent: "claude".into(),
        preferred: false,
    });
    config.write_atomic(dir.path()).unwrap();
    let app = factory_api::router(make_state(dir.path()));

    let response = request(app, "GET", "/api/graph", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let graph: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let worker_node = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "role:worker")
        .expect("worker role node");
    assert_eq!(worker_node["label"], "Worker");
    assert_eq!(worker_node["meta"]["kind"], "core");
    assert_eq!(
        worker_node["meta"]["assignments"].as_array().unwrap().len(),
        2
    );
    let binds: Vec<&serde_json::Value> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| edge["source"] == "role:worker")
        .collect();
    assert_eq!(binds.len(), 2);
    assert!(binds
        .iter()
        .any(|edge| edge["target"] == "agent:claude" && edge["id"] == "assignment:worker:claude"));
    assert!(binds
        .iter()
        .any(|edge| edge["target"] == "agent:codex" && edge["id"] == "assignment:worker:codex"));
    let codex_node = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "agent:codex")
        .unwrap();
    assert!(codex_node["meta"]["roles"].as_array().unwrap().len() >= 2);
}

async fn wait_for_run(root: &Path, id: i64, expected: RunStatus) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "run {id} never reached {expected:?}"
        );
        let state = make_state(root);
        let run = {
            let db = state.db.lock().unwrap();
            db.get_run(id).unwrap().unwrap()
        };
        if run.status == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn init_git(root: &Path) {
    std::fs::write(root.join("README.md"), "role API fixture\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "factory@example.test"][..],
        &["config", "user.name", "Factory Role API Test"][..],
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

fn configure_test_agents(root: &Path) {
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
        format!("type {}", plan_path.display())
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

#[tokio::test]
async fn run_team_can_be_updated_before_start() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_test_agents(dir.path());
    init_git(dir.path());
    let app = factory_api::router(make_state(dir.path()));

    let response = request(
        app.clone(),
        "POST",
        "/api/runs",
        Some(json!({ "objective": "Team editing" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let run_id = run["id"].as_i64().unwrap();
    assert_eq!(run["team"]["planner"], "planner-test");
    wait_for_run(dir.path(), run_id, RunStatus::Planned).await;

    let response = request(
        app.clone(),
        "PUT",
        &format!("/api/runs/{run_id}/team"),
        Some(json!({
            "planner": "planner-test",
            "workers": ["worker-test"],
            "reviewers": ["reviewer-test"]
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let team: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(team["workers"][0], "worker-test");

    let response = request(
        app.clone(),
        "PUT",
        &format!("/api/runs/{run_id}/team"),
        Some(json!({
            "planner": "ghost",
            "workers": ["worker-test"],
            "reviewers": ["reviewer-test"]
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(response)
        .await
        .contains("is not assigned to the 'planner' role"));

    let response = request(app, "POST", &format!("/api/runs/{run_id}/start"), None).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let team: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(team["planner"], "planner-test");
    assert_eq!(team["workers"][0], "worker-test");
    assert_eq!(team["reviewers"][0], "reviewer-test");
}
