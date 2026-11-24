use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use factory_api::{application_router, ApiOptions, ApplicationOptions, FactoryApplication};
use factory_projects::ProjectLimits;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
const SESSION: &str = "s_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
struct Fixture {
    _dir: tempfile::TempDir,
    app: Arc<FactoryApplication>,
    router: Router,
    a: String,
    b: String,
    auth: (String, String),
}
async fn send(
    router: &Router,
    path: &str,
    body: Value,
    auth: Option<&(String, String)>,
    lease: Option<&str>,
    options: (&str, bool, &str),
) -> (StatusCode, Value, String) {
    let (origin, csrf, method) = options;
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:4321")
        .header("origin", origin)
        .header("content-type", "application/json");
    if let Some((cookie, token)) = auth {
        req = req.header("cookie", cookie);
        if csrf {
            req = req.header("x-factory-csrf", token);
        }
    }
    if let Some(lease) = lease {
        req = req.header("x-asf-lease", lease);
    }
    let response = router
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
        cookies,
    )
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app = Arc::new(
            FactoryApplication::new(ApplicationOptions {
                registry_path: dir.path().join("registry.sqlite3"),
                allowed_roots: vec![dir.path().into()],
                default_root: None,
                dashboard_root: dir.path().into(),
                limits: ProjectLimits::default(),
                api: ApiOptions::default(),
            })
            .unwrap(),
        );
        let mut ids = Vec::new();
        for name in ["A", "B"] {
            let root = dir.path().join(name);
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("README.md"), "isolated lifecycle fixture\n").unwrap();
            for args in [
                &["init", "-q", "-b", "main"][..],
                &["config", "user.name", "Lifecycle Test"],
                &["config", "user.email", "test@example.invalid"],
                &["add", "."],
                &["commit", "-qm", "baseline"],
            ] {
                assert!(std::process::Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .status()
                    .unwrap()
                    .success());
            }
            factory_core::Factory::init(&root).unwrap();
            factory_core::Config::default().write_atomic(&root).unwrap();
            let id = app.projects.register(&root, None).unwrap().project_id;
            app.projects.open(&id).unwrap();
            ids.push(id.to_string());
        }
        let router = application_router(app.clone());
        let (status, value, cookie) = send(
            &router,
            "/api/auth/session",
            json!({}),
            None,
            None,
            ("http://127.0.0.1:4321", false, "POST"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        Self {
            _dir: dir,
            app,
            router,
            a: ids[0].clone(),
            b: ids[1].clone(),
            auth: (cookie, value["csrfToken"].as_str().unwrap().into()),
        }
    }
    async fn call(
        &self,
        path: &str,
        body: Value,
        lease: Option<&str>,
        method: &str,
    ) -> (StatusCode, Value) {
        let (s, v, _) = send(
            &self.router,
            path,
            body,
            Some(&self.auth),
            lease,
            ("http://127.0.0.1:4321", true, method),
        )
        .await;
        (s, v)
    }
    async fn register(&self, n: u64, projects: &[&str]) -> Value {
        let(s,v)=self.call("/api/browser-clients/register",json!({"clientInstanceId":format!("bc_{n:032x}"),"bindings":projects.iter().map(|p|json!({"projectId":p,"sessionId":SESSION})).collect::<Vec<_>>()}),None,"POST").await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }
    fn run(&self, project: &str) -> i64 {
        let c = self
            .app
            .projects
            .context(&factory_projects::ProjectId::parse(project).unwrap())
            .unwrap();
        let db = c.db.lock().unwrap();
        let run = db.create_run("owned fixture", None).unwrap();
        db.bind_workspace_run(run.id, project, SESSION).unwrap();
        run.id
    }
}
#[tokio::test]
async fn explicit_close_fences_a_resume_that_has_not_committed_execution() {
    let f = Fixture::new().await;
    let lease = f.register(1, &[&f.a]).await;
    let run = f.run(&f.a);
    let context = f
        .app
        .projects
        .context(&factory_projects::ProjectId::parse(&f.a).unwrap())
        .unwrap();
    context
        .runtime
        .suspend_mission(run, 1, "initial-suspend", "user_requested_suspend")
        .unwrap();
    assert!(context
        .db
        .lock()
        .unwrap()
        .begin_mission_resume(run, 2, "pending-resume")
        .unwrap());
    let (status, view) = f
        .call(
            &format!("/api/projects/{}/workspace-sessions/{SESSION}/close", f.a),
            json!({"expectedGeneration":1,"idempotencyKey":"close-pending-resume"}),
            lease["leaseId"].as_str(),
            "POST",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["missions"][0]["state"], "suspended");
    assert_eq!(view["missions"][0]["generation"], 4);
    let db = context.db.lock().unwrap();
    assert!(db.commit_mission_resume(run, 3).is_err());
    assert!(db.list_agent_sessions(Some(run)).unwrap().is_empty());
    assert!(context.runtime.active_operations().is_empty());
}

#[tokio::test]
async fn lease_endpoints_require_authentication_origin_csrf_and_owner() {
    let f = Fixture::new().await;
    let body = json!({"clientInstanceId":"bc_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","bindings":[]});
    for (auth, origin, csrf, expected) in [
        (
            None,
            "http://127.0.0.1:4321",
            false,
            StatusCode::UNAUTHORIZED,
        ),
        (
            Some(&f.auth),
            "https://attacker.invalid",
            true,
            StatusCode::FORBIDDEN,
        ),
        (
            Some(&f.auth),
            "http://127.0.0.1:4321",
            false,
            StatusCode::FORBIDDEN,
        ),
    ] {
        let (s, _, _) = send(
            &f.router,
            "/api/browser-clients/register",
            body.clone(),
            auth,
            None,
            (origin, csrf, "POST"),
        )
        .await;
        assert_eq!(s, expected);
    }
    let lease = f.register(1, &[&f.a]).await;
    let heartbeat = json!({"clientInstanceId":format!("bc_{:032x}",2),"generation":1,"sequence":1,"bindings":[]});
    assert_eq!(
        f.call(
            &format!(
                "/api/browser-clients/{}/heartbeat",
                lease["leaseId"].as_str().unwrap()
            ),
            heartbeat,
            None,
            "POST"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(
            "/api/browser-clients/register",
            json!({"clientInstanceId":"bad","bindings":[]}),
            None,
            "POST"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn session_close_is_scoped_and_only_reopen_authorizes_resume() {
    let f = Fixture::new().await;
    let lease = f.register(1, &[&f.a, &f.b]).await;
    let lease = lease["leaseId"].as_str().unwrap();
    let ra = f.run(&f.a);
    let rb = f.run(&f.b);
    assert_eq!(ra, rb);
    let close = format!("/api/projects/{}/workspace-sessions/{SESSION}/close", f.a);
    let body = json!({"expectedGeneration":1,"idempotencyKey":"close-A"});
    let (s, v) = f.call(&close, body.clone(), Some(lease), "POST").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["closed"], true);
    assert_eq!(v["missions"][0]["state"], "suspended");
    assert_eq!(
        f.call(&close, body, Some(lease), "POST").await.0,
        StatusCode::OK
    );
    let (_, b) = f
        .call(
            &format!("/api/projects/{}/runs/{rb}/lifecycle", f.b),
            json!({}),
            None,
            "GET",
        )
        .await;
    assert_eq!(b["lifecycle"]["state"], "active");
    let resume = format!("/api/projects/{}/runs/{ra}/resume", f.a);
    assert_eq!(
        f.call(
            &resume,
            json!({"expectedGeneration":2,"idempotencyKey":"r"}),
            Some(lease),
            "POST"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let reopen = format!("/api/projects/{}/workspace-sessions/{SESSION}/reopen", f.a);
    assert_eq!(
        f.call(
            &reopen,
            json!({"expectedGeneration":2,"idempotencyKey":"open"}),
            Some(lease),
            "POST"
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, checkpoint) = f
        .call(
            &format!("/api/projects/{}/runs/{ra}/checkpoints", f.a),
            json!({}),
            None,
            "GET",
        )
        .await;
    assert_eq!(checkpoint[0]["projectId"], f.a);
    assert_eq!(checkpoint[0]["workspaceSessionId"], SESSION);
}
#[tokio::test]
async fn a_lease_for_one_project_cannot_suspend_or_resume_matching_run_id_elsewhere() {
    let f = Fixture::new().await;
    let a = f.register(1, &[&f.a]).await;
    let _b = f.register(2, &[&f.b]).await;
    let _ra = f.run(&f.a);
    let rb = f.run(&f.b);
    for action in ["suspend", "resume"] {
        let (s, _) = f
            .call(
                &format!("/api/projects/{}/runs/{rb}/{action}", f.b),
                json!({"expectedGeneration":1,"idempotencyKey":action}),
                a["leaseId"].as_str(),
                "POST",
            )
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
    let path = format!("/api/projects/{}/runs/{rb}/suspend", f.b);
    assert_eq!(
        f.call(
            &path,
            json!({"expectedGeneration":1,"idempotencyKey":"fake"}),
            Some("bl_fake"),
            "POST"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn client_release_requires_its_own_identity_and_generation() {
    let f = Fixture::new().await;
    let lease = f.register(1, &[&f.a]).await;
    let id = lease["leaseId"].as_str().unwrap();
    let body = json!({"clientInstanceId":lease["clientInstanceId"],"generation":99,"sequence":1,"bindings":[]});
    assert_eq!(
        f.call(
            &format!("/api/browser-clients/{id}/release"),
            body,
            None,
            "POST"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let body = json!({"clientInstanceId":lease["clientInstanceId"],"generation":1,"sequence":1,"bindings":[]});
    assert_eq!(
        f.call(
            &format!("/api/browser-clients/{id}/release"),
            body.clone(),
            None,
            "POST"
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            &format!("/api/browser-clients/{id}/heartbeat"),
            body,
            None,
            "POST"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
