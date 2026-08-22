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
use axum::http::{Request, StatusCode};
use factory_core::{AgentEntry, Config, RoleAssignment};
use http_body_util::BodyExt;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

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
    let app = factory_api::router(state);

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
    let app = factory_api::router(state);

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
    let app = factory_api::router(state);

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
    let app = factory_api::router(state);
    let run = factory_core::Factory::open(dir.path())
        .unwrap()
        .import_github_issue("#42", None)
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
    let app = factory_api::router(state);
    let factory = factory_core::Factory::open(dir.path()).unwrap();
    let run = factory.import_github_issue("#42", None).unwrap();

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
