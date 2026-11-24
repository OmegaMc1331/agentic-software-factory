use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use factory_api::{application_router, ApiOptions, ApplicationOptions, FactoryApplication};
use factory_projects::ProjectLimits;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn send(
    router: &Router,
    method: &str,
    url: &str,
    body: Value,
    credentials: Option<&(String, String)>,
    origin: &str,
    csrf: bool,
) -> (StatusCode, Value, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(url)
        .header("host", "127.0.0.1:4321")
        .header("origin", origin)
        .header("content-type", "application/json");
    if let Some((cookie, token)) = credentials {
        request = request.header("cookie", cookie);
        if csrf {
            request = request.header("x-factory-csrf", token);
        }
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|h| h.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cookie,
    )
}

async fn credentials(router: &Router) -> (String, String) {
    let (status, value, cookie) = send(
        router,
        "POST",
        "/api/auth/session",
        json!({}),
        None,
        "http://127.0.0.1:4321",
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (cookie, value["csrfToken"].as_str().unwrap().into())
}
struct Projects {
    root: tempfile::TempDir,
    router: Router,
    a: String,
    b: String,
    credentials: (String, String),
}
impl Projects {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let app = Arc::new(
            FactoryApplication::new(ApplicationOptions {
                registry_path: root.path().join("registry.sqlite3"),
                allowed_roots: vec![root.path().to_owned()],
                default_root: None,
                dashboard_root: root.path().to_owned(),
                limits: ProjectLimits::default(),
                api: ApiOptions::default(),
            })
            .unwrap(),
        );
        let mut ids = Vec::new();
        for name in ["A", "B"] {
            let path = root.path().join(name);
            std::fs::create_dir(&path).unwrap();
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["init", "-q"])
                .status()
                .unwrap();
            assert!(status.success());
            factory_core::Factory::init(&path).unwrap();
            factory_core::Config::default().write_atomic(&path).unwrap();
            std::fs::create_dir(path.join("src")).unwrap();
            std::fs::write(path.join("src/main.rs"), format!("project {name}\n")).unwrap();
            let id = app.projects.register(&path, None).unwrap().project_id;
            app.projects.open(&id).unwrap();
            ids.push(id.to_string());
        }
        let router = application_router(app);
        let credentials = credentials(&router).await;
        Self {
            root,
            router,
            a: ids[0].clone(),
            b: ids[1].clone(),
            credentials,
        }
    }
    fn route(&self, a: bool, path: &str) -> String {
        format!(
            "/api/projects/{}/{}",
            if a { &self.a } else { &self.b },
            path
        )
    }
    async fn call(&self, a: bool, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let (status, value, _) = send(
            &self.router,
            method,
            &self.route(a, path),
            body,
            Some(&self.credentials),
            "http://127.0.0.1:4321",
            true,
        )
        .await;
        (status, value)
    }
}

fn input() -> Value {
    json!({"name":"Custom","description":"Scoped profile","providerAdapterId":"trusted","modelId":null,"reasoningEffort":null,"roles":["worker"],"instructions":"Preserve tests","permissions":{"filesystem":{"read":["**"],"write":["src/**"],"deny_write":[]}},"maxConcurrency":1,"timeoutSecs":30,"enabled":true,"automaticDelegation":false,"preferred":false})
}
fn configure(f: &Projects) {
    for name in ["A", "B"] {
        let root = f.root.path().join(name);
        let mut config = factory_core::Config::load(&root).unwrap();
        config.agents.insert(
            "trusted".into(),
            factory_core::AgentEntry {
                kind: None,
                command: "git".into(),
                args: vec!["--version".into()],
                env: std::collections::BTreeMap::from([(
                    "PRIVATE_PROVIDER_SECRET".into(),
                    "NEVER_REFLECT_THIS_VALUE".into(),
                )]),
                prompt_transport: None,
                interactive_args: None,
                capabilities: vec![],
                max_concurrency: Some(1),
            },
        );
        config.write_atomic(&root).unwrap();
    }
}

#[tokio::test]
async fn handoff_errors_distinguish_configuration_ownership_and_terminal_state() {
    let f = Projects::new().await;
    configure(&f);
    let database =
        factory_db::FactoryDb::open(&f.root.path().join("A/.factory/db.sqlite3")).unwrap();
    let run = database
        .create_run("Owned permission fixture", None)
        .unwrap();
    let tasks = database
        .persist_plan(
            run.id,
            &factory_types::Plan {
                objective: run.objective,
                tasks: vec![factory_types::PlannedTask {
                    id: "source".into(),
                    title: "Source task".into(),
                    objective: "Edit authorized source".into(),
                    dependencies: vec![],
                    acceptance_criteria: vec!["Reviewed changes".into()],
                    role: Some("worker".into()),
                    operation: Some(factory_types::TaskOperation::Implement),
                }],
            },
        )
        .unwrap();
    let path = format!("runs/{}/tasks/{}/handoff", run.id, tasks[0].id);
    let mut disabled = input();
    disabled["enabled"] = json!(false);
    let (status, disabled) = f.call(true, "POST", "agent-profiles", disabled).await;
    assert_eq!(status, StatusCode::OK);
    let mut reviewer = input();
    reviewer["roles"] = json!(["reviewer"]);
    let (status, reviewer) = f.call(true, "POST", "agent-profiles", reviewer).await;
    assert_eq!(status, StatusCode::OK);
    for (profile, key, code) in [
        (&disabled, "disabled", "disabled_profile"),
        (&reviewer, "role", "unsupported_role"),
    ] {
        let (status, error) = f.call(true,"POST",&path,json!({"toProfileId":profile["id"],"expectedProfileRevision":1,"idempotencyKey":key,"reason":"Verify the authorized refusal"})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
        assert_eq!(error["code"], code);
    }
    let (status, worker) = f.call(true, "POST", "agent-profiles", input()).await;
    assert_eq!(status, StatusCode::OK);
    let request = json!({"toProfileId":worker["id"],"expectedProfileRevision":1,"idempotencyKey":"owned","reason":"No agent process should start"});
    let other = database.create_run("Other mission", None).unwrap();
    let foreign = format!("runs/{}/tasks/{}/handoff", other.id, tasks[0].id);
    let (status, error) = f.call(true, "POST", &foreign, request.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error["code"], "project_mismatch");
    database
        .set_run_status(run.id, factory_types::RunStatus::Failed)
        .unwrap();
    let (status, error) = f.call(true, "POST", &path, request).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "task_no_longer_eligible");
    assert!(database
        .list_agent_sessions(Some(run.id))
        .unwrap()
        .is_empty());
    assert_eq!(database.handoffs(run.id).unwrap().len(), 2);
}

#[tokio::test]
async fn profiles_api_uses_project_scope_and_preserves_revision_history() {
    let f = Projects::new().await;
    configure(&f);
    let (status, a) = f.call(true, "POST", "agent-profiles", input()).await;
    assert_eq!(status, StatusCode::OK, "{a}");
    assert_eq!(a["projectId"], f.a);
    let id = a["id"].as_str().unwrap();
    let path = format!("agent-profiles/{id}");
    assert_eq!(
        f.call(false, "GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    let body = json!({"expectedRevision":1,"configuration":input()});
    assert_eq!(
        f.call(false, "PATCH", &path, body.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, b) = f.call(true, "PATCH", &path, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{b}");
    assert_eq!(b["revision"], 2);
    let (status, error) = f.call(true, "PATCH", &path, body).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "stale_profile_revision");
    let (status, revs) = f
        .call(true, "GET", &format!("{path}/revisions"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revs.as_array().unwrap().len(), 2);
    assert_eq!(
        f.call(true, "DELETE", &path, json!({"expectedRevision":2}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(true, "GET", "agent-profiles", Value::Null)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        f.call(true, "GET", &format!("{path}/revisions"), Value::Null)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        f.call(false, "POST", "agent-profiles", input()).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn profile_mutations_require_authentication_csrf_and_same_origin() {
    let f = Projects::new().await;
    configure(&f);
    for (credentials, origin, csrf) in [
        (None, "http://127.0.0.1:4321", false),
        (Some(&f.credentials), "http://127.0.0.1:4321", false),
        (Some(&f.credentials), "https://evil.example", true),
    ] {
        let (status, _, _) = send(
            &f.router,
            "POST",
            &f.route(true, "agent-profiles"),
            input(),
            credentials,
            origin,
            csrf,
        )
        .await;
        assert!(!status.is_success());
    }
    assert!(f
        .call(true, "GET", "agent-profiles", Value::Null)
        .await
        .1
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn untrusted_process_settings_are_rejected_and_provider_metadata_is_redacted() {
    let f = Projects::new().await;
    configure(&f);
    let mut payload = input();
    payload["command"] = json!("arbitrary.exe");
    assert!(!f
        .call(true, "POST", "agent-profiles", payload)
        .await
        .0
        .is_success());
    let mut payload = input();
    payload["providerAdapterId"] = json!("arbitrary.exe");
    assert_eq!(
        f.call(true, "POST", "agent-profiles", payload).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut payload = input();
    payload["modelId"] = json!("gpt; --sandbox=danger-full-access");
    assert_eq!(
        f.call(true, "POST", "agent-profiles", payload).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, providers) = f
        .call(true, "GET", "agent-profiles/providers", Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{providers}");
    assert!(!providers.to_string().contains("NEVER_REFLECT_THIS_VALUE"));
    assert!(!providers.to_string().contains("PRIVATE_PROVIDER_SECRET"));
    assert_eq!(providers["providers"][0]["modelDiscovery"], false);
}
