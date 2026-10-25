//! GitHub API surface: status, issue import, and delivery endpoints, tested
//! against fake `gh` scripts and a local bare remote. No real GitHub account
//! is ever contacted.

// The env lock is held across awaits on purpose: every #[tokio::test] owns an
// independent runtime, so a test blocked on the lock only parks a thread of
// its own runtime and can never deadlock the lock holder.
#![allow(clippy::await_holding_lock)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard, OnceLock};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use factory_core::{AgentEntry, Config, RoleAssignment};
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

fn gh_env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let mutex = LOCK.get_or_init(|| Mutex::new(()));
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn install_fake_gh(dir: &Path) {
    let issue_file = dir.join("issue.json");
    std::fs::write(
        &issue_file,
        r#"{"number":42,"title":"Fix refresh token race","body":"Tokens rotate.","labels":[],"state":"open","url":"https://github.com/octocat/example/issues/42","author":{"login":"octocat"},"comments":[]}"#,
    )
    .unwrap();
    let pr_list = dir.join("pr-list.json");
    std::fs::write(&pr_list, "[]").unwrap();
    let path = if cfg!(windows) {
        let script = format!(
            "@echo off\r\n\
             if \"%1\"==\"auth\" goto auth\r\n\
             if \"%1\"==\"issue\" goto issue\r\n\
             if \"%1\"==\"pr\" if \"%2\"==\"list\" goto prlist\r\n\
             if \"%1\"==\"pr\" if \"%2\"==\"create\" goto prcreate\r\n\
             exit /b 1\r\n\
             :auth\r\n\
             echo   Logged in to github.com account octocat keyring\r\n\
             exit /b 0\r\n\
             :issue\r\n\
             type \"{issue}\"\r\n\
             exit /b 0\r\n\
             :prlist\r\n\
             type \"{pr_list}\"\r\n\
             exit /b 0\r\n\
             :prcreate\r\n\
             echo https://github.com/octocat/example/pull/58\r\n\
             exit /b 0\r\n",
            issue = issue_file.display(),
            pr_list = pr_list.display(),
        );
        let path = dir.join("fake-gh.cmd");
        std::fs::write(&path, script).unwrap();
        path
    } else {
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n\
             auth) echo '  Logged in to github.com account octocat keyring' ;;\n\
             issue) cat '{issue}' ;;\n\
             pr) case \"$2\" in\n\
             list) cat '{pr_list}' ;;\n\
             create) echo https://github.com/octocat/example/pull/58 ;;\n\
             esac ;;\n\
             esac\n",
            issue = issue_file.display(),
            pr_list = pr_list.display(),
        );
        let path = dir.join("fake-gh.sh");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
        path
    };
    std::env::set_var("FACTORY_GH_BIN", &path);
}

fn init_github_clone(dir: &Path) -> PathBuf {
    let bare = dir.join("remote.git");
    assert!(Command::new("git")
        .arg("init")
        .arg("-q")
        .arg("--bare")
        .arg("-b")
        .arg("main")
        .arg(&bare)
        .status()
        .unwrap()
        .success());
    std::fs::write(dir.join("README.md"), "api fixture\n").unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "factory@example.test"],
        vec!["config", "user.name", "Factory API Test"],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "initial"],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
    let steps: Vec<Vec<String>> = vec![
        vec![
            "remote".into(),
            "add".into(),
            "origin".into(),
            "https://github.com/octocat/example.git".into(),
        ],
        vec![
            "config".into(),
            format!("url.{}.insteadOf", bare.display()),
            "https://github.com/octocat/example.git".into(),
        ],
        vec![
            "push".into(),
            "-q".into(),
            "-u".into(),
            "origin".into(),
            "main".into(),
        ],
    ];
    for args in &steps {
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    bare
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

fn fixture() -> (TempDir, Arc<factory_api::ApiState>) {
    let dir = TempDir::new().unwrap();
    init_github_clone(dir.path());
    factory_core::Factory::init(dir.path()).unwrap();

    let plan_path = dir.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{"objective":"API issue workflow","tasks":[{"id":"T1","title":"Fix","objective":"fix it","dependencies":[],"acceptanceCriteria":["done"]}]}"#,
    )
    .unwrap();
    let cat = if cfg!(windows) {
        format!("type {}", plan_path.display())
    } else {
        format!("cat '{}'", plan_path.display())
    };
    let mut config = Config::default();
    config
        .agents
        .insert("planner-test".into(), command_entry(&cat));
    for role in ["planner", "worker", "reviewer"] {
        config.role_assignments.push(RoleAssignment {
            role: role.into(),
            agent: "planner-test".into(),
            preferred: true,
        });
    }
    config.write_atomic(dir.path()).unwrap();

    install_fake_gh(dir.path());
    let state = Arc::new(factory_api::ApiState::new(dir.path().to_path_buf()).unwrap());
    (dir, state)
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn github_status_reports_the_connected_account_and_repository() {
    let _guard = gh_env_lock();
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/github/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(value["connected"], true);
    assert_eq!(value["user"], "octocat");
    assert_eq!(value["repository"]["repository"], "octocat/example");
    assert_eq!(value["repository"]["remote"], "origin");
}

#[tokio::test]
async fn from_issue_creates_a_workflow_with_a_persisted_untrusted_link() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs/from-issue")
                .header("content-type", "application/json")
                .body(Body::from(json!({"issue": "#42"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    let run_id = run["id"].as_i64().unwrap();
    assert_eq!(run["status"], "planning");

    let link = factory_core::Factory::open(dir.path())
        .unwrap()
        .github_link(run_id)
        .unwrap()
        .expect("link persisted");
    assert_eq!(link.issue_number, 42);
    assert_eq!(link.repository, "octocat/example");
    assert_eq!(link.provider, "github");
}

#[tokio::test]
async fn from_issue_rejects_invalid_references() {
    let _guard = gh_env_lock();
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs/from-issue")
                .header("content-type", "application/json")
                .body(Body::from(json!({"issue": "not-a-ref"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = body_text(response).await;
    assert!(text.contains("invalid issue reference"), "error: {text}");
}

#[tokio::test]
async fn delivery_endpoints_report_ineligibility_and_unknown_runs() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let app = common::authenticated_app(state).await;
    let run = factory_core::Factory::open(dir.path())
        .unwrap()
        .import_github_issue("#42", None, None)
        .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{}/delivery", run.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(report["state"], "not_ready");
    assert_eq!(report["eligible"], false);
    assert_eq!(report["link"]["issueNumber"], 42);
    assert_eq!(report["headBranch"], format!("factory/run-{}", run.id));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{}/pr-preview", run.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(preview["eligible"], false);
    assert!(preview["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| blocker.as_str().unwrap().contains("completed")));

    // Incomplete workflows cannot publish: the semantic delivery endpoint
    // refuses instead of creating a partial PR.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/runs/{}/pull-request", run.id))
                .header("content-type", "application/json")
                .body(Body::from(json!({"title": "t", "body": "b"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let text = body_text(response).await;
    assert!(
        text.contains("delivery requires completed"),
        "error: {text}"
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/runs/999999/delivery")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn graph_exposes_compact_github_issue_and_pr_nodes() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let app = common::authenticated_app(state).await;
    let factory = factory_core::Factory::open(dir.path()).unwrap();
    let run = factory.import_github_issue("#42", None, None).unwrap();

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
    let issue_node = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "github_issue")
        .expect("issue node");
    assert_eq!(issue_node["id"], format!("github_issue:{}", run.id));
    assert_eq!(issue_node["meta"]["number"], 42);
    assert_eq!(issue_node["meta"]["url"], link_issue_url());
    assert!(graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|edge| edge["kind"] == "originates"
            && edge["source"] == issue_node["id"]
            && edge["target"] == format!("run:{}", run.id)));
    // No PR yet: the delivered node must not appear.
    assert!(!graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["kind"] == "github_pr"));
}

fn link_issue_url() -> &'static str {
    "https://github.com/octocat/example/issues/42"
}

// --- ASF-18: durable asynchronous delivery operations -----------------------

/// Drives the fixture workflow to completion through the core Factory (the
/// API's runtime holds the project lock; `Factory::open` stays lock-free).
async fn completed_delivery_run(dir: &Path) -> i64 {
    let plan_path = dir.join("plan.json");
    let worker_path = dir.join("worker.json");
    let reviewer_path = dir.join("reviewer.json");
    std::fs::write(&worker_path, r#"{"commands":["cargo test"]}"#).unwrap();
    std::fs::write(
        &reviewer_path,
        r#"{"decision":"approve","reason":"lgtm","feedback":[]}"#,
    )
    .unwrap();
    let cat = |path: &Path| -> String {
        if cfg!(windows) {
            format!("type {}", path.display())
        } else {
            format!("cat '{}'", path.display())
        }
    };
    let worker = if cfg!(windows) {
        format!(
            "echo done>worker-output.txt & type {}",
            worker_path.display()
        )
    } else {
        format!(
            "echo done > worker-output.txt; cat '{}'",
            worker_path.display()
        )
    };
    let mut config = Config::default();
    config
        .agents
        .insert("planner-test".into(), command_entry(&cat(&plan_path)));
    config
        .agents
        .insert("worker-test".into(), command_entry(&worker));
    config
        .agents
        .insert("reviewer-test".into(), command_entry(&cat(&reviewer_path)));
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
    config.write_atomic(dir).unwrap();

    // Open only after the config exists: the Factory snapshots it at open.
    let factory = factory_core::Factory::open(dir).unwrap();
    let run = factory.import_github_issue("#42", None, None).unwrap();
    factory
        .plan_run(run.id, &std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    factory.prepare_start(run.id).unwrap();
    let result = factory
        .execute_active_run(run.id, &std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert!(matches!(result, factory_core::WorkflowResult::Completed));
    run.id
}

async fn delivery_report(app: &Router, run_id: i64) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{run_id}/delivery"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    common::response_json(response).await
}

async fn post_delivery(app: &Router, run_id: i64) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/runs/{run_id}/pull-request"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"title": "ASF-18 api title", "body": "api body"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, common::response_json(response).await)
}

#[tokio::test]
async fn pull_request_runs_asynchronously_and_publishes() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let run_id = completed_delivery_run(dir.path()).await;
    let app = common::authenticated_app(state).await;

    let (status, body) = post_delivery(&app, run_id).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    assert_eq!(body["state"], "pushing");
    assert!(
        body["operationId"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "the accepted response carries the durable operation id: {body}"
    );

    // The operation is Factory-owned: poll the durable status to completion.
    let mut published = None;
    for _ in 0..100 {
        let report = delivery_report(&app, run_id).await;
        if report["pullRequest"].is_object() {
            published = Some(report);
            break;
        }
        assert_eq!(report["operationActive"], true, "while running: {report}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let report = published.unwrap_or_else(|| panic!("delivery never published"));
    assert_eq!(report["state"], "published");
    assert_eq!(report["persistedState"], "published");
    assert_eq!(report["pullRequest"]["number"], 58);
    assert_eq!(report["retrySafe"], false);
    assert_eq!(report["recoveryRequired"], false);
}

#[tokio::test]
async fn repeat_request_after_publication_returns_the_existing_pr() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let run_id = completed_delivery_run(dir.path()).await;
    let app = common::authenticated_app(state).await;

    let (first_status, _) = post_delivery(&app, run_id).await;
    assert_eq!(first_status, StatusCode::ACCEPTED);
    let mut done = false;
    for _ in 0..100 {
        if delivery_report(&app, run_id).await["pullRequest"].is_object() {
            done = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(done, "delivery never published");

    let (status, body) = post_delivery(&app, run_id).await;
    assert_eq!(status, StatusCode::OK, "idempotent repeat: {body}");
    assert_eq!(body["pullRequest"]["number"], 58);
}

#[tokio::test]
async fn concurrent_requests_launch_exactly_one_operation() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let run_id = completed_delivery_run(dir.path()).await;
    let app = common::authenticated_app(state).await;
    let second = app.clone();

    let (left, right) = tokio::join!(post_delivery(&app, run_id), post_delivery(&second, run_id));
    let mut accepted = 0;
    let mut conflicts = 0;
    for (status, body) in [left, right] {
        match status {
            StatusCode::ACCEPTED => accepted += 1,
            StatusCode::CONFLICT => {
                conflicts += 1;
                assert!(
                    body["error"].to_string().contains("delivery operation"),
                    "conflict explains itself: {body}"
                );
            }
            other => panic!("unexpected status {other}: {body}"),
        }
    }
    assert_eq!(accepted, 1, "exactly one request may start the operation");
    assert_eq!(conflicts, 1);

    let mut published = false;
    for _ in 0..100 {
        if delivery_report(&app, run_id).await["pullRequest"].is_object() {
            published = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(published, "delivery never published");
}

#[tokio::test]
async fn delivery_endpoints_require_authentication() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let run_id = completed_delivery_run(dir.path()).await;
    let app = factory_api::router(state);

    for uri in [
        format!("/api/runs/{run_id}/pull-request"),
        format!("/api/runs/{run_id}/delivery/reconcile"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header("host", "127.0.0.1:4321")
                    .header("origin", "http://127.0.0.1:4321")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn interrupted_delivery_is_inspectable_blocked_then_reconciled() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let run_id = completed_delivery_run(dir.path()).await;

    // Simulate a crash: the operation is claimed, then startup demotion marks
    // it interrupted with an unknown outcome.
    let factory = factory_core::Factory::open(dir.path()).unwrap();
    factory
        .begin_delivery(run_id, Some("title"), Some("body"), false)
        .unwrap();
    factory.demote_interrupted_deliveries().unwrap();
    drop(factory);

    let app = common::authenticated_app(state).await;
    let report = delivery_report(&app, run_id).await;
    assert_eq!(report["state"], "interrupted");
    assert_eq!(report["persistedState"], "interrupted");
    assert_eq!(report["recoveryRequired"], true);
    assert_eq!(report["retrySafe"], false, "unknown outcome blocks retry");

    let (status, body) = post_delivery(&app, run_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "no blind retry: {body}");

    // GitHub reveals the PR the crashed attempt created.
    std::fs::write(
        dir.path().join("pr-list.json"),
        r#"[{"number":91,"url":"https://github.com/octocat/example/pull/91","state":"OPEN","isDraft":false}]"#,
    )
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/runs/{run_id}/delivery/reconcile"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let outcome = common::response_json(response).await;
    assert!(
        outcome.to_string().contains("linkedExisting"),
        "outcome: {outcome}"
    );

    let report = delivery_report(&app, run_id).await;
    assert_eq!(report["state"], "published");
    assert_eq!(report["pullRequest"]["number"], 91);
    assert_eq!(report["recoveryRequired"], false);
}

// --- ASF-R02: slow `gh pr create` vs client timeouts ------------------------

/// Installs a fake `gh` whose `pr create` blocks until a release file
/// appears, then reports PR #58. The gate is a file the test writes —
/// deterministic synchronization, no timing races. Every create invocation
/// is logged so duplicate publication is observable.
fn install_gated_fake_gh(dir: &Path) {
    let issue_file = dir.join("issue.json");
    std::fs::write(
        &issue_file,
        r#"{"number":42,"title":"Fix refresh token race","body":"Tokens rotate.","labels":[],"state":"open","url":"https://github.com/octocat/example/issues/42","author":{"login":"octocat"},"comments":[]}"#,
    )
    .unwrap();
    let pr_list = dir.join("pr-list.json");
    std::fs::write(&pr_list, "[]").unwrap();
    let create_log = dir.join("gh-create.log");
    let release = dir.join("gh-create-release");
    let path = if cfg!(windows) {
        let script = format!(
            "@echo off\r\n\
             if \"%1\"==\"auth\" goto auth\r\n\
             if \"%1\"==\"issue\" goto issue\r\n\
             if \"%1\"==\"pr\" if \"%2\"==\"list\" goto prlist\r\n\
             if \"%1\"==\"pr\" if \"%2\"==\"create\" goto prcreate\r\n\
             exit /b 1\r\n\
             :auth\r\n\
             echo   Logged in to github.com account octocat keyring\r\n\
             exit /b 0\r\n\
             :issue\r\n\
             type \"{issue}\"\r\n\
             exit /b 0\r\n\
             :prlist\r\n\
             type \"{pr_list}\"\r\n\
             exit /b 0\r\n\
             :prcreate\r\n\
             echo create>>\"{create_log}\"\r\n\
             set /a tries=0\r\n\
             :waitloop\r\n\
             if exist \"{release}\" goto done\r\n\
             choice /t 1 /d y /n >nul\r\n\
             set /a tries+=1\r\n\
             if %tries% lss 60 goto waitloop\r\n\
             echo gate never released 1>&2\r\n\
             exit /b 1\r\n\
             :done\r\n\
             echo https://github.com/octocat/example/pull/58\r\n\
             exit /b 0\r\n",
            issue = issue_file.display(),
            pr_list = pr_list.display(),
            create_log = create_log.display(),
            release = release.display(),
        );
        let path = dir.join("fake-gh-gated.cmd");
        std::fs::write(&path, script).unwrap();
        path
    } else {
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n\
             auth) echo '  Logged in to github.com account octocat keyring' ;;\n\
             issue) cat '{issue}' ;;\n\
             pr) case \"$2\" in\n\
             list) cat '{pr_list}' ;;\n\
             create) echo create >> '{create_log}'; \
             tries=0; \
             while [ ! -f '{release}' ] && [ \"$tries\" -lt 600 ]; do \
             sleep 0.1; tries=$((tries+1)); done; \
             if [ ! -f '{release}' ]; then echo 'gate never released' >&2; exit 1; fi; \
             echo https://github.com/octocat/example/pull/58 ;;\n\
             esac ;;\n\
             esac\n",
            issue = issue_file.display(),
            pr_list = pr_list.display(),
            create_log = create_log.display(),
            release = release.display(),
        );
        let path = dir.join("fake-gh-gated.sh");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
        path
    };
    std::env::set_var("FACTORY_GH_BIN", &path);
}

fn gated_creates(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("gh-create.log"))
        .map(|content| {
            content
                .lines()
                .filter(|line| line.trim() == "create")
                .count()
        })
        .unwrap_or(0)
}

fn release_gated_create(dir: &Path) {
    std::fs::write(dir.join("gh-create-release"), "").unwrap();
}

/// Polls the durable delivery status until `ready` returns true.
async fn wait_for_delivery(
    app: &Router,
    run_id: i64,
    ready: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    for _ in 0..300 {
        let report = delivery_report(app, run_id).await;
        if ready(&report) {
            return report;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("delivery status never satisfied the condition");
}

/// ASF-R02 primary scenario: `gh pr create` slower than any HTTP request
/// lifetime. The POST must acknowledge (202) while the operation runs, the
/// status must show it active, a duplicate submission must be rejected by
/// the server, and after the gate releases exactly one PR exists. The client
/// losing interest mid-flight changes nothing server-side.
#[tokio::test]
async fn slow_pr_create_is_acknowledged_active_and_never_duplicated() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    install_gated_fake_gh(dir.path());
    let run_id = completed_delivery_run(dir.path()).await;
    let app = common::authenticated_app(state).await;

    // 1. The operation is acknowledged immediately — publication is NOT done.
    let (status, body) = post_delivery(&app, run_id).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    assert_eq!(body["state"], "pushing");
    assert!(body["operationId"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));

    // 2. Publication proceeds into the blocked create; status shows active.
    let active = wait_for_delivery(&app, run_id, |report| {
        report["operationActive"] == serde_json::Value::Bool(true)
            && (report["persistedState"] == "pushing" || report["persistedState"] == "creating_pr")
    })
    .await;
    assert!(active["retrySafe"] == serde_json::Value::Bool(false));

    // 3. A second client (tab, retry after "timeout") is rejected by the
    //    durable claim — server-side, not by a disabled button.
    let (dup_status, dup_body) = post_delivery(&app, run_id).await;
    assert_eq!(dup_status, StatusCode::CONFLICT, "body: {dup_body}");
    assert_eq!(
        gated_creates(dir.path()),
        1,
        "still exactly one create attempt"
    );

    // 4. Release the slow command; the outcome is recoverable via status.
    release_gated_create(dir.path());
    let published =
        wait_for_delivery(&app, run_id, |report| report["pullRequest"].is_object()).await;
    assert_eq!(published["state"], "published");
    assert_eq!(published["pullRequest"]["number"], 58);
    assert_eq!(
        published["pullRequest"]["url"],
        "https://github.com/octocat/example/pull/58"
    );
    assert_eq!(gated_creates(dir.path()), 1, "no duplicate PR was created");

    // 5. A repeat request returns the existing PR and creates nothing.
    let (repeat_status, repeat_body) = post_delivery(&app, run_id).await;
    assert_eq!(repeat_status, StatusCode::OK, "body: {repeat_body}");
    assert_eq!(repeat_body["pullRequest"]["number"], 58);
    assert_eq!(gated_creates(dir.path()), 1);
}

/// A minimal HTTP/1.1 client over a raw TcpStream, for tests that need the
/// transport to behave like a browser (including losing responses).
struct RawHttp {
    stream: tokio::net::TcpStream,
}

impl RawHttp {
    async fn connect(addr: std::net::SocketAddr) -> RawHttp {
        RawHttp {
            stream: tokio::net::TcpStream::connect(addr).await.unwrap(),
        }
    }

    async fn send(
        &mut self,
        method: &str,
        target: &str,
        host: &str,
        cookie: &str,
        csrf: Option<&str>,
        body: &str,
    ) {
        use tokio::io::AsyncWriteExt;
        let csrf_header = csrf
            .map(|value| format!("x-factory-csrf: {value}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "{method} {target} HTTP/1.1\r\n\
             host: {host}\r\n\
             origin: http://{host}\r\n\
             cookie: {cookie}\r\n\
             {csrf_header}\
             content-type: application/json\r\n\
             content-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len(),
        );
        self.stream.write_all(request.as_bytes()).await.unwrap();
        self.stream.flush().await.unwrap();
    }

    /// Reads the full response; returns (status, raw response text).
    async fn read_response(mut self) -> (u16, String) {
        use tokio::io::AsyncReadExt;
        let mut buffer = Vec::new();
        self.stream.read_to_end(&mut buffer).await.unwrap();
        let text = String::from_utf8_lossy(&buffer).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or_default();
        (status, text)
    }

    fn body_of(text: &str) -> &str {
        text.split("\r\n\r\n").nth(1).unwrap_or_default()
    }

    /// The lost-acknowledgement transport: request fully sent, response
    /// never read, socket dropped.
    fn abandon(self) {
        drop(self.stream);
    }
}

/// ASF-R02 lost-response scenario over a real TCP server: the client sends
/// the create POST and drops the connection before the acknowledgement
/// arrives. The server-side operation must still run to completion, stay
/// discoverable through a fresh connection, reject a duplicate submission,
/// and publish exactly one PR.
#[tokio::test]
async fn lost_acknowledgement_does_not_lose_the_operation() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    install_gated_fake_gh(dir.path());
    let run_id = completed_delivery_run(dir.path()).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        let app = factory_api::router(server_state);
        axum::serve(listener, app).await.unwrap();
    });

    // Bootstrap a real browser session over TCP.
    let (session, csrf) = {
        let mut client = RawHttp::connect(addr).await;
        client
            .send(
                "POST",
                "/api/auth/session",
                &addr.to_string(),
                "",
                None,
                "{}",
            )
            .await;
        let (status, text) = client.read_response().await;
        assert_eq!(status, 200, "bootstrap: {text}");
        let mut session = None;
        let mut csrf = None;
        for line in text.split("\r\n") {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("set-cookie: factory_session=") {
                session = line
                    .split(';')
                    .next()
                    .and_then(|pair| pair.split('=').nth(1).map(str::to_string));
            }
            if lower.starts_with("set-cookie: factory_csrf=") {
                csrf = line
                    .split(';')
                    .next()
                    .and_then(|pair| pair.split('=').nth(1).map(str::to_string));
            }
        }
        // The bootstrap body repeats the CSRF token; either source works.
        let csrf = csrf.or_else(|| {
            text.split('"')
                .find(|part| part.len() == 64 && part.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
        });
        (session.expect("session cookie"), csrf.expect("csrf token"))
    };
    let cookie = format!("factory_session={session}; factory_csrf={csrf}");
    let host = addr.to_string();

    // The client sends the create request, then loses the response.
    let mut lost = RawHttp::connect(addr).await;
    lost.send(
        "POST",
        &format!("/api/runs/{run_id}/pull-request"),
        &host,
        &cookie,
        Some(&csrf),
        r#"{"title":"lost ack","body":"body"}"#,
    )
    .await;
    lost.abandon();

    // The dropped request may or may not have been processed before the
    // transport died - HTTP permits both. Recovery must converge safely
    // either way: poll briefly for the claim; if it never appears, the
    // request was a clean no-op and a fresh POST claims freshly.
    for _ in 0..50 {
        let mut probe = RawHttp::connect(addr).await;
        probe
            .send(
                "GET",
                &format!("/api/runs/{run_id}/delivery"),
                &host,
                &cookie,
                None,
                "",
            )
            .await;
        let (probe_status, probe_body) = probe.read_response().await;
        assert_eq!(probe_status, 200, "status: {probe_body}");
        let report: serde_json::Value =
            serde_json::from_str(RawHttp::body_of(&probe_body).trim_start_matches('\u{feff}'))
                .unwrap_or_else(|error| panic!("delivery body {probe_body:?}: {error}"));
        if report["operationActive"] == serde_json::Value::Bool(true)
            || report["pullRequest"].is_object()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // A second submission is safe in every branch: 409 while the lost
    // request's operation is active, 200 if it already published, or a
    // fresh 202 acceptance when the lost request never processed. The
    // retry reuses the same payload the lost request carried.
    let mut duplicate = RawHttp::connect(addr).await;
    duplicate
        .send(
            "POST",
            &format!("/api/runs/{run_id}/pull-request"),
            &host,
            &cookie,
            Some(&csrf),
            r#"{"title":"lost ack","body":"body"}"#,
        )
        .await;
    let (dup_status, dup_body) = duplicate.read_response().await;
    assert!(
        dup_status == 409 || dup_status == 200 || dup_status == 202,
        "duplicate resolves safely: {dup_status} {dup_body}"
    );

    // Release the slow create; the outcome is recoverable by any client.
    release_gated_create(dir.path());
    let recovered = RawHttp::connect(addr).await;
    for _ in 0..300 {
        let mut poll = RawHttp::connect(addr).await;
        poll.send(
            "GET",
            &format!("/api/runs/{run_id}/delivery"),
            &host,
            &cookie,
            None,
            "",
        )
        .await;
        let (_, body) = poll.read_response().await;
        if let Ok(report) = serde_json::from_str::<serde_json::Value>(
            RawHttp::body_of(&body).trim_start_matches('\u{feff}'),
        ) {
            if report["pullRequest"].is_object() {
                let _ = recovered;
                assert_eq!(report["pullRequest"]["number"], 58);
                assert_eq!(gated_creates(dir.path()), 1);
                server.abort();
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    server.abort();
    panic!("delivery never published after the lost acknowledgement");
}
