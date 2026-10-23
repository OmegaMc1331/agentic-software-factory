//! Recipes API: catalog CRUD, recipe-based workflow creation (objective and
//! GitHub Issue), immutable snapshots, and Save as Recipe. Fake shell agents
//! and a fake `gh`; no real credentials.

#![allow(clippy::await_holding_lock)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
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

mod common;

fn gh_env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let mutex = LOCK.get_or_init(|| Mutex::new(()));
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

fn init_git(root: &Path) {
    std::fs::write(root.join("README.md"), "recipes api fixture\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "factory@example.test"][..],
        &["config", "user.name", "Factory API Test"][..],
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

struct GhEnvOverride {
    previous: Option<OsString>,
}

impl Drop for GhEnvOverride {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var("FACTORY_GH_BIN", previous);
        } else {
            std::env::remove_var("FACTORY_GH_BIN");
        }
    }
}

fn install_fake_gh(dir: &Path) -> GhEnvOverride {
    let issue_file = dir.join("issue.json");
    std::fs::write(
        &issue_file,
        r#"{"number":42,"title":"Fix refresh token race","body":"Tokens rotate.","labels":[],"state":"open","url":"https://github.com/octocat/example/issues/42","author":{"login":"octocat"},"comments":[]}"#,
    )
    .unwrap();
    let path = if cfg!(windows) {
        let script = format!(
            "@echo off\r\n\
             if \"%1\"==\"auth\" goto auth\r\n\
             if \"%1\"==\"issue\" goto issue\r\n\
             if \"%1\"==\"pr\" if \"%2\"==\"list\" goto prlist\r\n\
             exit /b 1\r\n\
             :auth\r\n\
             echo   Logged in to github.com account octocat keyring\r\n\
             exit /b 0\r\n\
             :issue\r\n\
             type \"{issue}\"\r\n\
             exit /b 0\r\n\
             :prlist\r\n\
             echo []\r\n\
             exit /b 0\r\n",
            issue = issue_file.display(),
        );
        let path = dir.join("fake-gh.cmd");
        std::fs::write(&path, script).unwrap();
        path
    } else {
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n\
             auth) echo '  Logged in to github.com account octocat keyring' ;;\n\
             issue) cat '{issue}' ;;\n\
             pr) case \"$2\" in list) echo '[]' ;; esac ;;\n\
             esac\n",
            issue = issue_file.display(),
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
    let previous = std::env::var_os("FACTORY_GH_BIN");
    std::env::set_var("FACTORY_GH_BIN", &path);
    GhEnvOverride { previous }
}

fn init_github_clone(dir: &Path) {
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
    init_git(dir);
    for args in [
        vec![
            "remote".to_string(),
            "add".to_string(),
            "origin".to_string(),
            "https://github.com/octocat/example.git".to_string(),
        ],
        vec![
            "config".to_string(),
            format!("url.{}.insteadOf", bare.display()),
            "https://github.com/octocat/example.git".to_string(),
        ],
        vec![
            "push".to_string(),
            "-q".to_string(),
            "-u".to_string(),
            "origin".to_string(),
            "main".to_string(),
        ],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
}

/// A fixture with planner/worker/reviewer/architect/test_engineer agents so
/// the built-in recipes resolve. Tests that need `gh` install it explicitly.
fn fixture() -> (TempDir, Arc<factory_api::ApiState>) {
    let dir = TempDir::new().unwrap();
    init_github_clone(dir.path());
    factory_core::Factory::init(dir.path()).unwrap();

    let plan_path = dir.path().join("plan.json");
    std::fs::write(
        &plan_path,
        r#"{"objective":"recipe api workflow","tasks":[{"id":"T1","title":"Fix","objective":"fix it","dependencies":[],"acceptanceCriteria":["done"]}]}"#,
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
        .insert("agent-test".into(), command_entry(&cat));
    for role in [
        "planner",
        "worker",
        "reviewer",
        "architect",
        "test_engineer",
    ] {
        config.role_assignments.push(RoleAssignment {
            role: role.into(),
            agent: "agent-test".into(),
            preferred: true,
        });
    }
    config.write_atomic(dir.path()).unwrap();

    let state = Arc::new(factory_api::ApiState::new(dir.path().to_path_buf()).unwrap());
    (dir, state)
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn lists_built_in_and_custom_recipes() {
    let (dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .clone()
        .oneshot(Request::get("/api/recipes").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let recipes = body_json(response).await;
    let ids: Vec<&str> = recipes
        .as_array()
        .unwrap()
        .iter()
        .map(|recipe| recipe["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["standard", "thorough", "security", "bugfix", "docs", "custom"]
    );
    assert!(recipes
        .as_array()
        .unwrap()
        .iter()
        .all(|recipe| recipe["builtin"].as_bool().unwrap()));

    // Backward compatibility: no recipes.toml was required to get here.
    assert!(!dir.path().join(".factory").join("recipes.toml").exists());
}

#[tokio::test]
async fn custom_recipe_crud_round_trips() {
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let create = json!({
        "name": "Backend Feature",
        "description": "Backend implementation with verification.",
        "roles": ["planner", "worker", "test_engineer", "reviewer"],
        "routing": "performance",
        "maxParallelTasks": 3,
        "plannerGuidance": "Prefer a compact dependency-aware plan.",
    });
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/recipes")
                .header("content-type", "application/json")
                .body(Body::from(create.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let recipe = body_json(response).await;
    assert_eq!(recipe["id"], "backend_feature");
    assert_eq!(recipe["routing"], "performance");
    assert!(!recipe["builtin"].as_bool().unwrap());

    // Duplicate ids are rejected.
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/recipes")
                .header("content-type", "application/json")
                .body(Body::from(create.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Update (rename) the custom recipe.
    let update = json!({
        "name": "Backend Feature v2",
        "roles": ["planner", "worker", "reviewer"],
        "routing": "round_robin",
    });
    let response = app
        .clone()
        .oneshot(
            Request::put("/api/recipes/backend_feature")
                .header("content-type", "application/json")
                .body(Body::from(update.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let recipe = body_json(response).await;
    assert_eq!(recipe["name"], "Backend Feature v2");
    assert_eq!(recipe["routing"], "round_robin");

    // Built-ins are read-only.
    for (method, path) in [
        ("PUT", "/api/recipes/security"),
        ("DELETE", "/api/recipes/security"),
        ("PUT", "/api/recipes/custom"),
        ("DELETE", "/api/recipes/custom"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"name": "X", "roles": ["planner"]}).to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{method} {path}"
        );
    }

    // Unknown recipes are 404.
    let response = app
        .clone()
        .oneshot(
            Request::put("/api/recipes/missing")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name": "X", "roles": ["planner"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Delete the custom recipe.
    let response = app
        .clone()
        .oneshot(
            Request::delete("/api/recipes/backend_feature")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = app
        .oneshot(Request::get("/api/recipes").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let recipes = body_json(response).await;
    assert!(recipes
        .as_array()
        .unwrap()
        .iter()
        .all(|recipe| recipe["id"] != "backend_feature"));
}

#[tokio::test]
async fn invalid_recipe_payloads_are_rejected_atomically() {
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    // Unknown role.
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/recipes")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name": "Ghost", "roles": ["planner", "ghost_role"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = body_text(response).await;
    assert!(text.contains("ghost_role"), "unexpected: {text}");

    // Unknown policy preset.
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/recipes")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "Bad Policy",
                        "roles": ["planner", "worker"],
                        "policyPresets": {"worker": "superuser"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Out-of-bounds concurrency.
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/recipes")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "Huge",
                        "roles": ["planner", "worker"],
                        "maxParallelTasks": 99
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn run_creation_accepts_a_recipe_id_and_snapshots_it() {
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"objective": "fix the bug", "recipe": "bugfix"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run = body_json(response).await;
    let snapshot = &run["recipe"];
    assert_eq!(snapshot["id"], "bugfix");
    assert_eq!(snapshot["name"], "Bug Fix");
    assert!(snapshot["plannerGuidance"]
        .as_str()
        .unwrap()
        .contains("regression"));
    let roles = snapshot["roles"].as_array().unwrap();
    assert!(roles.iter().any(|role| role == "test_engineer"));

    // The team was pre-filled from the recipe's roles.
    let team = &run["team"];
    assert_eq!(team["planner"], "agent-test");
    assert_eq!(team["additional"]["test_engineer"][0], "agent-test");

    // Unknown recipe ids are 404, and nothing is created.
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"objective": "x", "recipe": "does_not_exist"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn run_creation_accepts_an_inline_recipe_definition() {
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .oneshot(
            Request::post("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "objective": "improve the api",
                        "recipe": {
                            "name": "Inline Run",
                            "roles": ["planner", "worker", "reviewer"],
                            "routing": "performance",
                            "maxParallelTasks": 2
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run = body_json(response).await;
    assert_eq!(run["recipe"]["id"], "inline_run");
    assert_eq!(run["recipe"]["routing"], "performance");
    assert_eq!(run["recipe"]["maxParallelTasks"], 2);
}

#[tokio::test]
async fn inline_recipe_requires_available_roles() {
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .oneshot(
            Request::post("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "objective": "document the api",
                        "recipe": {
                            "name": "Inline Docs Run",
                            "roles": ["planner", "documentation_writer", "reviewer"]
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = body_text(response).await;
    assert!(
        text.contains("documentation_writer"),
        "the error names the unavailable role: {text}"
    );
}

#[tokio::test]
async fn github_issue_workflows_accept_a_recipe() {
    let _guard = gh_env_lock();
    let (dir, state) = fixture();
    let _gh = install_fake_gh(dir.path());
    let app = common::authenticated_app(state).await;

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/runs/from-issue")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"issue": "#42", "recipe": "bugfix"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run = body_json(response).await;
    assert_eq!(run["recipe"]["id"], "bugfix");
    assert!(run["objective"].as_str().unwrap().contains("refresh token"));

    // Same catalog, same errors — no separate GitHub recipe system.
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/runs/from-issue")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"issue": "#42", "recipe": "security"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = body_text(response).await;
    assert!(text.contains("security_auditor"), "unexpected: {text}");
}

#[tokio::test]
async fn save_as_recipe_derives_configuration_without_history() {
    let (_dir, state) = fixture();
    let app = common::authenticated_app(state).await;

    // Create and plan a workflow (background planning writes one task).
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"objective": "recipe api workflow", "recipe": "bugfix"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let run = body_json(response).await;
    let run_id = run["id"].as_i64().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/runs/{run_id}/recipe"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name": "Saved Bugfix", "description": "From a run."}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let recipe = body_json(response).await;
    assert_eq!(recipe["id"], "saved_bugfix");
    assert_eq!(recipe["roles"].as_array().unwrap().len(), 4);
    // Configuration keys only.
    assert!(recipe.get("tasks").is_none());
    assert!(recipe.get("dependencies").is_none());

    // The saved recipe exists in the catalog and carries no history.
    let response = app
        .clone()
        .oneshot(Request::get("/api/recipes").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let text = body_text(response).await;
    assert!(text.contains("saved_bugfix"));
    assert!(text.contains("plannerGuidance"));
    assert!(!text.contains("attempt"), "no history in recipes: {text}");
}

#[tokio::test]
async fn existing_runs_without_recipes_expose_none() {
    let (_guard, state) = fixture();
    let app = common::authenticated_app(state).await;

    let response = app
        .oneshot(
            Request::post("/api/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"objective": "plain old workflow"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let run = body_json(response).await;
    assert!(run.get("recipe").is_none() || run["recipe"].is_null());
}
