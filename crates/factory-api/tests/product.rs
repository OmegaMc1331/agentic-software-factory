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

struct Fixture {
    _dir: tempfile::TempDir,
    app: Arc<FactoryApplication>,
    router: Router,
    ids: Vec<factory_projects::ProjectId>,
    cookie: String,
    csrf: String,
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
        let mut ids = vec![];
        for name in ["A", "B"] {
            let root = dir.path().join(name);
            std::fs::create_dir(&root).unwrap();
            factory_core::Factory::init(&root).unwrap();
            let id = app.projects.register(&root, None).unwrap().project_id;
            app.projects.open(&id).unwrap();
            ids.push(id);
        }
        let router = application_router(app.clone());
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/session")
                    .header("host", "127.0.0.1:4321")
                    .header("origin", "http://127.0.0.1:4321")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|h| h.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        let value: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        Self {
            _dir: dir,
            app,
            router,
            ids,
            cookie,
            csrf: value["csrfToken"].as_str().unwrap().into(),
        }
    }
    async fn raw(
        &self,
        path: &str,
        method: &str,
        body: Value,
        auth: bool,
        csrf: bool,
        origin: &str,
    ) -> (StatusCode, Value) {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:4321")
            .header("origin", origin)
            .header("content-type", "application/json");
        if auth {
            r = r.header("cookie", &self.cookie);
        }
        if csrf {
            r = r.header("x-factory-csrf", &self.csrf);
        }
        let response = self
            .router
            .clone()
            .oneshot(r.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let data = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&data).unwrap_or(Value::Null))
    }
    async fn call(&self, path: &str, method: &str, body: Value) -> (StatusCode, Value) {
        self.raw(path, method, body, true, true, "http://127.0.0.1:4321")
            .await
    }
}

#[tokio::test]
async fn preferences_use_real_application_storage_and_preserve_auth_scope_and_cas() {
    let f = Fixture::new().await;
    let path = "/api/application/preferences";
    let (status, mut value) = f.call(path, "GET", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    value["preferences"]["theme"] = json!("dark");
    for (auth, csrf, origin) in [
        (false, true, "http://127.0.0.1:4321"),
        (true, false, "http://127.0.0.1:4321"),
        (true, true, "https://attacker.invalid"),
    ] {
        let (status, _) = f.raw(path, "PUT", value.clone(), auth, csrf, origin).await;
        assert!(matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    let (status, saved) = f.call(path, "PUT", value.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["revision"], 1);
    assert_eq!(
        f.call(path, "PUT", value.clone()).await.0,
        StatusCode::CONFLICT
    );
    let mut malformed = saved.clone();
    malformed["preferences"]["editor_tab_size"] = json!(0);
    assert_eq!(
        f.call(path, "PUT", malformed).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut unknown = saved.clone();
    unknown["preferences"]["project_id"] = json!(f.ids[1].to_string());
    assert!(f.call(path, "PUT", unknown).await.0.is_client_error());
    let (status, current) = f.call(path, "GET", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current, saved);
    assert!(!current.to_string().contains("api_key"));
    let (status, _) = f
        .call(
            &format!("/api/projects/{}/application/preferences", f.ids[0]),
            "PUT",
            saved,
        )
        .await;
    assert!(status.is_client_error());
}

#[tokio::test]
async fn real_notification_events_and_read_state_remain_project_scoped_and_non_executing() {
    let f = Fixture::new().await;
    let a = f.app.projects.open(&f.ids[0]).unwrap();
    let run = {
        let db = a.db.lock().unwrap();
        let run = db
            .create_run("private source should not be copied", None)
            .unwrap()
            .id;
        db.set_run_status(run, factory_types::RunStatus::Failed)
            .unwrap();
        db.set_run_status(run, factory_types::RunStatus::Failed)
            .unwrap();
        run
    };
    let ap = format!("/api/projects/{}/notifications", f.ids[0]);
    let bp = format!("/api/projects/{}/notifications", f.ids[1]);
    let (status, page) = f.call(&ap, "GET", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["run_id"], run);
    assert!(!page.to_string().contains("private source"));
    assert_eq!(f.call(&bp, "GET", Value::Null).await.1["unread"], 0);
    let body = json!({"revision":1,"read":true,"dismissed":false});
    let id = page["items"][0]["id"].as_i64().unwrap();
    assert_eq!(
        f.call(&format!("{bp}/{id}"), "PATCH", body.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    let mutation = format!("{ap}/{id}");
    assert_eq!(
        f.raw(
            &mutation,
            "PATCH",
            body.clone(),
            true,
            false,
            "http://127.0.0.1:4321"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(&mutation, "PATCH", body.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(f.call(&mutation, "PATCH", body).await.0, StatusCode::OK);
    assert_eq!(
        f.call(
            &mutation,
            "PATCH",
            json!({"revision":1,"read":false,"dismissed":false})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(f.call(&ap, "GET", Value::Null).await.1["unread"], 0);
    let db = a.db.lock().unwrap();
    assert_eq!(db.list_agent_sessions(None).unwrap().len(), 0);
    assert_eq!(
        db.get_run(run).unwrap().unwrap().status,
        factory_types::RunStatus::Failed
    );
}

#[tokio::test]
async fn project_settings_are_an_authoritative_read_only_projection_without_credentials() {
    let f = Fixture::new().await;
    let a = f.app.projects.open(&f.ids[0]).unwrap();
    let mut config = factory_core::Config::default();
    let credential = "synthetic-private-setting-value";
    config.agents.insert("configured".into(),serde_json::from_value(json!({"command":"unused-test-provider","env":{"API_TOKEN":credential},"args":[credential]})).unwrap());
    config.policies = serde_json::from_value(
        json!({"ceiling":{"network":{"mode":"deny"},"commands":{"mode":"denied"}}}),
    )
    .unwrap();
    config.write_atomic(&a.root).unwrap();
    let path = format!("/api/projects/{}/settings", f.ids[0]);
    let (status, value) = f.call(&path, "GET", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["project_id"], f.ids[0].to_string());
    assert_eq!(value["agent_count"], 1);
    assert_eq!(value["policies"]["ceiling"]["network_mode"], "deny");
    assert!(!value.to_string().contains(credential));
    assert!(!value.to_string().contains("unused-test-provider"));
    assert!(f
        .call(&path, "PUT", json!({"policies":{}}))
        .await
        .0
        .is_client_error());
    let (status, b) = f
        .call(
            &format!("/api/projects/{}/settings", f.ids[1]),
            "GET",
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        b["agent_count"],
        factory_core::Config::load_with_revision(&f.app.projects.open(&f.ids[1]).unwrap().root)
            .unwrap()
            .0
            .agents
            .len()
    );
    assert!(factory_core::Config::load_with_revision(&a.root)
        .unwrap()
        .0
        .policies
        .ceiling
        .is_some());
}

#[tokio::test]
async fn a_real_save_conflict_creates_one_private_notification_without_overwriting_source() {
    let f = Fixture::new().await;
    let a = f.app.projects.open(&f.ids[0]).unwrap();
    std::fs::write(a.root.join("source.rs"), "first version\n").unwrap();
    let prefix = format!("/api/projects/{}", f.ids[0]);
    let (_, roots) = f
        .call(&format!("{prefix}/files/roots"), "GET", Value::Null)
        .await;
    let root = roots["roots"][0]["id"].as_str().unwrap();
    let (status, original) = f
        .call(
            &format!("{prefix}/files/content?root_id={root}&path=source.rs"),
            "GET",
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    std::fs::write(a.root.join("source.rs"), "external version\n").unwrap();
    let body = json!({"file":{"project_id":f.ids[0].to_string(),"root_id":root,"path":"source.rs"},"expected_revision":original["revision"],"content":"do not overwrite this version"});
    for _ in 0..2 {
        assert_eq!(
            f.call(&format!("{prefix}/files/content"), "PUT", body.clone())
                .await
                .0,
            StatusCode::CONFLICT
        );
    }
    assert_eq!(
        std::fs::read_to_string(a.root.join("source.rs")).unwrap(),
        "external version\n"
    );
    let (_, page) = f
        .call(&format!("{prefix}/notifications"), "GET", Value::Null)
        .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["event_type"], "file.save_conflict");
    assert!(!page.to_string().contains("source.rs"));
    assert!(!page.to_string().contains("external version"));
    assert_eq!(
        f.call(
            &format!("/api/projects/{}/notifications", f.ids[1]),
            "GET",
            Value::Null
        )
        .await
        .1["unread"],
        0
    );
}
