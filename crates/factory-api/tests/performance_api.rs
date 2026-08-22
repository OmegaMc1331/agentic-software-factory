//! Read-only performance API over synthetic workflow history.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use factory_types::{
    AgentSession, AgentSessionMode, AttemptStatus, ReviewDecision, ReviewResult, TaskEvidence,
    TaskOperation, TaskState,
};
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

fn init_root(root: &Path) {
    factory_core::Factory::init(root).unwrap();
}

fn make_state(root: &Path) -> Arc<factory_api::ApiState> {
    Arc::new(factory_api::ApiState::new(root.to_path_buf()).unwrap())
}

/// Seeds one two-attempt task: a request-changes round, then approval.
fn seed_history(state: &factory_api::ApiState) {
    let db = state.db.lock().unwrap();
    let run = db.create_run("objective", Some("codex")).unwrap();
    let task = db
        .create_task(
            run.id,
            "Task",
            "objective",
            &[],
            TaskState::Completed,
            0,
            Some("worker"),
            Some(TaskOperation::Implement),
        )
        .unwrap();

    let first = db
        .create_task_attempt(
            task,
            "worker",
            Some(TaskOperation::Implement),
            "codex",
            "worktree",
            None,
        )
        .unwrap();
    db.finish_task_attempt(
        first.id,
        AttemptStatus::ChangesRequested,
        Some(0),
        None,
        Some("needs tests"),
        None,
        Some(&ReviewResult {
            decision: ReviewDecision::RequestChanges,
            reason: "needs tests".into(),
            feedback: vec![],
        }),
    )
    .unwrap();

    let second = db
        .create_task_attempt(
            task,
            "worker",
            Some(TaskOperation::Implement),
            "codex",
            "worktree",
            None,
        )
        .unwrap();
    db.insert_agent_session(&AgentSession {
        id: 0,
        run_id: Some(run.id),
        task_id: Some(task),
        attempt_id: Some(second.id),
        role: "worker".into(),
        operation: Some(TaskOperation::Implement),
        agent: "codex".into(),
        mode: AgentSessionMode::Automated,
        command: "codex exec".into(),
        status: "success".into(),
        started_at: "2026-08-20T08:00:00Z".into(),
        finished_at: Some("2026-08-20T08:01:34Z".into()),
        exit_code: Some(0),
        duration_ms: Some(94_000),
        stdout: None,
        stderr: None,
        policy_audit: None,
    })
    .unwrap();
    let evidence = TaskEvidence {
        changed_files: vec!["src/lib.rs".into()],
        diff_summary: "1 file changed".into(),
        commit_sha: None,
        commands: vec![],
        acceptance_criteria: vec![],
        worker_exit_code: Some(0),
        artifacts: vec![],
        diff_patch: None,
    };
    db.finish_task_attempt(
        second.id,
        AttemptStatus::Approved,
        Some(0),
        None,
        None,
        Some(&evidence),
        Some(&ReviewResult {
            decision: ReviewDecision::Approve,
            reason: "verified".into(),
            feedback: vec![],
        }),
    )
    .unwrap();
    db.record_integration_outcome(
        run.id,
        task,
        second.id,
        "codex",
        factory_types::IntegrationOutcomeKind::Clean,
    )
    .unwrap();
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn performance_overview_lists_agents_with_sample_sizes() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    seed_history(&state);
    let app = factory_api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/performance/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();

    assert_eq!(value["window"], json!("all_time"));
    let agents = value["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    let codex = &agents[0];
    assert_eq!(codex["agent"], "codex");
    assert_eq!(codex["metrics"]["tasksAttempted"], 1);
    assert_eq!(codex["metrics"]["attempts"], 2);
    assert_eq!(codex["metrics"]["firstPassApproval"]["successes"], 0);
    assert_eq!(codex["metrics"]["firstPassApproval"]["total"], 1);
    assert_eq!(codex["metrics"]["firstPassApproval"]["reliable"], false);
    assert_eq!(codex["metrics"]["eventualApproval"]["rate"], 1.0);
    assert_eq!(codex["metrics"]["executionDuration"]["medianMs"], 94_000);
    assert_eq!(codex["metrics"]["integration"]["clean"], 1);
    assert!(value["facets"]["languages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|language| language == "rust"));
}

#[tokio::test]
async fn performance_agent_detail_returns_breakdowns_and_trend() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    seed_history(&state);
    let app = factory_api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/performance/agents/codex")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();

    assert_eq!(value["summary"]["agent"], "codex");
    let operations = value["byOperation"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["key"], "implement");
    let languages = value["byLanguage"].as_array().unwrap();
    assert_eq!(languages[0]["key"], "rust");
    let trend = &value["trend"];
    assert_eq!(trend["recent10"]["firstPass"]["total"], 1);
    let reasons = value["reworkReasons"].as_array().unwrap();
    assert_eq!(reasons[0]["reason"], "needs tests");
}

#[tokio::test]
async fn performance_endpoints_reject_unknown_filters_and_agents() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    let app = factory_api::router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/performance/agents?window=42d")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/performance/agents/codex")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/performance/agents?operation=nonsense")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn performance_endpoints_apply_window_and_role_filters() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    seed_history(&state);
    let app = factory_api::router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/performance/agents?window=7d&role=worker&operation=implement&language=rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    // The seeded attempt timestamps (August 2026) fall inside the window.
    assert_eq!(value["agents"].as_array().unwrap().len(), 1);

    // A role nothing matches empties the list without an error.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/performance/agents?role=reviewer")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value["agents"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn performance_endpoints_are_read_only() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    seed_history(&state);
    let app = factory_api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/performance/agents")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
