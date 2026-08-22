//! Routing API: read-only preview/decision endpoints and the manual routing
//! override, exercised against synthetic workflow state.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_core::{AgentEntry, Config, RoleAssignment, RoutingConfig, RoutingMode};
use factory_types::{AttemptStatus, TaskOperation, TaskState};
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

fn init_root(root: &Path) {
    std::fs::write(root.join("README.md"), "test repository\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "factory@example.test"][..],
        &["config", "user.name", "Factory Test"][..],
        &["add", "."][..],
        &["commit", "-q", "-m", "initial"][..],
    ] {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    factory_core::Factory::init(root).unwrap();
}

fn command_entry(script: &str) -> AgentEntry {
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec!["/d".into(), "/c".into(), script.into()],
        )
    } else {
        ("sh".to_string(), vec!["-c".into(), script.into()])
    };
    AgentEntry {
        kind: None,
        command,
        args,
        env: BTreeMap::new(),
        prompt_transport: None,
        interactive_args: None,
        capabilities: Vec::new(),
        max_concurrency: None,
    }
}

/// A root with two fake workers, a fake reviewer/planner, and performance
/// routing enabled, plus one planned run with a single ready task.
fn performance_root() -> (TempDir, Arc<factory_api::ApiState>, i64, i64) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    init_root(&root);

    let plan = r#"{"objective":"routing api","tasks":[{"id":"T1","title":"Only","objective":"work","dependencies":[],"acceptanceCriteria":["done"]}]}"#;
    std::fs::write(root.join("plan.json"), plan).unwrap();
    std::fs::write(
        root.join("review.json"),
        r#"{"decision":"approve","reason":"ok"}"#,
    )
    .unwrap();
    std::fs::write(root.join("worker.json"), r#"{"commands":["fake"]}"#).unwrap();
    let cat = |name: &str| -> String {
        if cfg!(windows) {
            format!("type {}", root.join(name).display())
        } else {
            format!("cat '{}'", root.join(name).display())
        }
    };

    let mut config = Config::default();
    config
        .agents
        .insert("planner-test".into(), command_entry(&cat("plan.json")));
    config
        .agents
        .insert("reviewer-test".into(), command_entry(&cat("review.json")));
    config
        .agents
        .insert("worker-a".into(), command_entry(&cat("worker.json")));
    config
        .agents
        .insert("worker-b".into(), command_entry(&cat("worker.json")));
    for (role, agent, preferred) in [
        ("planner", "planner-test", true),
        ("reviewer", "reviewer-test", true),
        ("worker", "worker-a", false),
        ("worker", "worker-b", false),
    ] {
        config.role_assignments.push(RoleAssignment {
            role: role.into(),
            agent: agent.into(),
            preferred,
        });
    }
    config.routing = RoutingConfig {
        mode: RoutingMode::Performance,
        exploration: false,
    };
    config.write_atomic(&root).unwrap();

    let factory = factory_core::Factory::open(&root).unwrap();
    let team = factory_types::WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into(), "worker-b".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: BTreeMap::new(),
    };
    let run = factory.begin_run("routing api", Some(team)).unwrap();
    factory
        .plan_run(run.id, &std::sync::atomic::AtomicBool::new(false))
        .unwrap();

    let state = Arc::new(factory_api::ApiState::new(root.clone()).unwrap());
    let db = state.db.lock().unwrap();
    let tasks = db.list_tasks(run.id).unwrap();
    drop(db);
    let task_id = tasks[0].id;
    (dir, state, run.id, task_id)
}

async fn get(state: &Arc<factory_api::ApiState>, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = factory_api::router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn put(
    state: &Arc<factory_api::ApiState>,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = factory_api::router(state.clone())
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn routing_preview_reports_mode_role_and_candidates() {
    let (_dir, state, _run_id, task_id) = performance_root();
    let (status, body) = get(&state, &format!("/api/tasks/{task_id}/routing-preview")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "performance");
    assert_eq!(body["role"], "worker");
    assert_eq!(body["operation"], "implement");
    assert_eq!(body["overrideAgent"], serde_json::Value::Null);
    let candidates = body["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    for candidate in candidates {
        assert!(candidate["agent"].is_string());
        // No history: nothing is ranked yet.
        assert_eq!(candidate["reliable"], false);
    }
}

#[tokio::test]
async fn routing_override_pins_and_clears() {
    let (_dir, state, _run_id, task_id) = performance_root();
    let (status, body) = put(
        &state,
        &format!("/api/tasks/{task_id}/routing"),
        json!({ "agent": "worker-b" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["agentOverride"], "worker-b");

    let (_, preview) = get(&state, &format!("/api/tasks/{task_id}/routing-preview")).await;
    assert_eq!(preview["overrideAgent"], "worker-b");

    let (status, body) = put(
        &state,
        &format!("/api/tasks/{task_id}/routing"),
        json!({ "agent": null }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["agentOverride"], serde_json::Value::Null);
}

#[tokio::test]
async fn routing_override_validates_role_membership() {
    let (_dir, state, _run_id, task_id) = performance_root();
    // reviewer-test is a real agent but not assigned to the worker role.
    let (status, body) = put(
        &state,
        &format!("/api/tasks/{task_id}/routing"),
        json!({ "agent": "reviewer-test" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("not assigned to role"),
        "unexpected error: {body}"
    );

    let (status, body) = put(
        &state,
        &format!("/api/tasks/{task_id}/routing"),
        json!({ "agent": "ghost" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("unknown agent"));
}

#[tokio::test]
async fn routing_override_rejects_running_tasks() {
    let (_dir, state, _run_id, task_id) = performance_root();
    {
        let db = state.db.lock().unwrap();
        db.set_task_state(task_id, TaskState::Running).unwrap();
    }
    let (status, body) = put(
        &state,
        &format!("/api/tasks/{task_id}/routing"),
        json!({ "agent": "worker-a" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"].as_str().unwrap().contains("running"),
        "unexpected error: {body}"
    );
}

#[tokio::test]
async fn routing_decisions_are_auditable_after_a_dispatch() {
    let (dir, state, run_id, task_id) = performance_root();
    // Seed reliable history so the dispatch is performance-ranked, then run
    // the workflow through the runtime-backed factory.
    {
        let db = state.db.lock().unwrap();
        for index in 0..12 {
            let history_run = db.create_run("history", Some("planner-test")).unwrap();
            let task = db
                .create_task(
                    history_run.id,
                    "History",
                    "seeded",
                    &[],
                    TaskState::Completed,
                    index,
                    Some("worker"),
                    Some(TaskOperation::Implement),
                )
                .unwrap();
            let attempt = db
                .create_task_attempt(
                    task,
                    "worker",
                    Some(TaskOperation::Implement),
                    "worker-a",
                    "seed",
                    None,
                )
                .unwrap();
            db.finish_task_attempt(
                attempt.id,
                AttemptStatus::Approved,
                Some(0),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        }
    }
    let factory = factory_core::Factory::open(dir.path()).unwrap();
    factory.prepare_start(run_id).unwrap();
    factory
        .execute_active_run(run_id, &std::sync::atomic::AtomicBool::new(false))
        .unwrap();

    let (status, body) = get(&state, &format!("/api/tasks/{task_id}/routing-decisions")).await;
    assert_eq!(status, StatusCode::OK);
    let decisions = body.as_array().unwrap();
    // Worker dispatch + built-in review dispatch, both auditable.
    assert_eq!(decisions.len(), 2);
    let worker_decision = decisions
        .iter()
        .find(|decision| decision["role"] == "worker")
        .unwrap();
    assert_eq!(worker_decision["selectedAgent"], "worker-a");
    assert_eq!(worker_decision["mode"], "performance");
    assert!(worker_decision["reason"]
        .as_str()
        .unwrap()
        .contains("score"));
    assert!(worker_decision["attemptId"].is_number());
}

#[tokio::test]
async fn routing_preview_of_missing_task_is_404() {
    let (_dir, state, _run_id, _task_id) = performance_root();
    let (status, _) = get(&state, "/api/tasks/9999/routing-preview").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&state, "/api/tasks/9999/routing-decisions").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
