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
    router: Router,
    a: String,
    b: String,
    cookie: String,
    csrf: String,
    app: Arc<FactoryApplication>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_cancels_an_owned_manual_memory_extractor_without_waiting_for_its_timeout() {
    let f = Arc::new(Fixture::new().await);
    let context = f
        .app
        .projects
        .context(&factory_projects::ProjectId::parse(&f.a).unwrap())
        .unwrap();
    let script = context.root.join(".factory/shutdown-extractor.py");
    std::fs::write(
        &script,
        "import sys,time\nsys.stdin.read()\ntime.sleep(30)\nprint('{\"candidates\":[]}')\n",
    )
    .unwrap();
    let mut config = factory_core::Config::default();
    config.agents.insert("shutdown-extractor".into(), serde_json::from_value(json!({"kind":"custom","command":if cfg!(windows) {"python"} else {"python3"},"args":[script],"env":{"PYTHONDONTWRITEBYTECODE":"1"}})).unwrap());
    config.role_assignments.push(factory_core::RoleAssignment {
        role: "planner".into(),
        agent: "shutdown-extractor".into(),
        preferred: true,
    });
    config.write_atomic(&context.root).unwrap();
    let (_, mut settings) = f.call(&f.a, "/settings", "GET", json!({})).await;
    settings["extractionMode"] = json!("review");
    assert_eq!(
        f.call(&f.a, "/settings", "PUT", settings).await.0,
        StatusCode::OK
    );
    let source = json!({"projectId":f.a,"reference":{"kind":"repository_file","rootId":context.files.primary_root().id,"path":"ARCHITECTURE.md","revision":null}});
    let client = f.clone();
    let mut extraction = tokio::spawn(async move {
        client
            .call(&client.a, "/extract", "POST", json!({"sources":[source]}))
            .await
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let started = context
            .db
            .lock()
            .unwrap()
            .list_agent_sessions(None)
            .unwrap()
            .iter()
            .any(|session| {
                session.role == "planner"
                    && session.status == "running"
                    && session
                        .output_meta
                        .as_ref()
                        .is_some_and(|metadata| metadata.process_tree_supervised)
            });
        if started {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "owned extractor did not start"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(factory_api::serve_application_with_shutdown(
        f.app.clone(),
        factory_api::bind(0).unwrap(),
        async {
            let _ = stopped.await;
        },
    ));
    stop.send(()).unwrap();
    let timely = tokio::time::timeout(std::time::Duration::from_secs(8), &mut extraction).await;
    let cancelled_promptly = timely.is_ok();
    let (status, body) = match timely {
        Ok(result) => result.unwrap(),
        Err(_) => extraction.await.unwrap(),
    };
    let shutdown = tokio::time::timeout(std::time::Duration::from_secs(45), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        cancelled_promptly,
        "application shutdown ignored the extractor cancellation boundary"
    );
    assert!(shutdown.is_ok(), "{shutdown:?}");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let sessions = context
        .db
        .lock()
        .unwrap()
        .list_agent_sessions(None)
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_ne!(sessions[0].status, "running");
    assert!(
        sessions[0]
            .output_meta
            .as_ref()
            .unwrap()
            .process_tree_cleanup_confirmed
    );
    let records = context.db.lock().unwrap().memory_extractions(&f.a).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, "failed");
    assert!(records[0]
        .error
        .as_ref()
        .unwrap()
        .starts_with("INTERRUPTED"));
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
            let path = dir.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(
                path.join("ARCHITECTURE.md"),
                format!("Project {name} database access uses repository adapters.\n"),
            )
            .unwrap();
            for args in [
                &["init", "-q", "-b", "main"][..],
                &["config", "user.name", "Memory API"],
                &["config", "user.email", "memory@example.invalid"],
                &["add", "."],
                &["commit", "-qm", "baseline"],
            ] {
                assert!(std::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success());
            }
            factory_core::Factory::init(&path).unwrap();
            factory_core::Config::default().write_atomic(&path).unwrap();
            let id = app.projects.register(&path, None).unwrap().project_id;
            app.projects.open(&id).unwrap();
            ids.push(id.to_string());
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
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        Self {
            _dir: dir,
            router,
            a: ids[0].clone(),
            b: ids[1].clone(),
            cookie,
            csrf: body["csrfToken"].as_str().unwrap().into(),
            app,
        }
    }
    async fn call(
        &self,
        project: &str,
        path: &str,
        method: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        self.raw(
            &format!("/api/projects/{project}/memory{path}"),
            method,
            body,
            true,
            true,
            "http://127.0.0.1:4321",
        )
        .await
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
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:4321")
            .header("origin", origin)
            .header("content-type", "application/json");
        if auth {
            req = req.header("cookie", &self.cookie);
        }
        if csrf {
            req = req.header("x-factory-csrf", &self.csrf);
        }
        let response = self
            .router
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let text = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&text).unwrap_or(Value::Null))
    }
    fn input(&self, project: &str) -> Value {
        json!({"category":"coding_conventions","title":"Database convention","content":"Database access uses repository adapters.","subject":"database_access","scope":"api","status":"active","sources":[{"projectId":project,"reference":{"kind":"manual","note":"Explicit user entry"}}]})
    }
    async fn create(&self) -> Value {
        let (status, m) = self
            .call(
                &self.a,
                "",
                "POST",
                json!({"memory":self.input(&self.a),"reason":"manual","expectedRevision":null}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{m}");
        m
    }
}
#[tokio::test]
async fn security_authentication_csrf_origin_and_payload_bounds_remain_authoritative() {
    let f = Fixture::new().await;
    let path = format!("/api/projects/{}/memory", f.a);
    let body = json!({"memory":f.input(&f.a)});
    for (auth, csrf, origin) in [
        (false, true, "http://127.0.0.1:4321"),
        (true, false, "http://127.0.0.1:4321"),
        (true, true, "https://attacker.invalid"),
    ] {
        let (s, _) = f.raw(&path, "POST", body.clone(), auth, csrf, origin).await;
        assert!(!s.is_success());
    }
    let (s, _) = f
        .call(
            &f.a,
            "",
            "POST",
            json!({"memory":f.input(&f.a),"shell":"unsafe"}),
        )
        .await;
    assert!(!s.is_success());
    let (s, _) = f
        .call(
            &f.a,
            "",
            "POST",
            json!({"memory":f.input(&f.a),"reason":"x".repeat(70000)}),
        )
        .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
}
#[tokio::test]
async fn sec_01_04_cross_project_read_edit_delete_and_source_forgery_are_rejected() {
    let f = Fixture::new().await;
    let m = f.create().await;
    let path = format!("/{}", m["id"].as_str().unwrap());
    for method in ["GET", "PATCH", "DELETE"] {
        let body = if method == "PATCH" {
            json!({"memory":f.input(&f.b),"expectedRevision":1})
        } else {
            json!({"expectedRevision":1})
        };
        let (status, _) = f.call(&f.b, &path, method, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    let mut forged = f.input(&f.a);
    forged["sources"][0]["projectId"] = json!(f.b);
    let (s, v) = f.call(&f.a, "", "POST", json!({"memory":forged})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (_, value) = f
        .call(
            &f.b,
            "/retrieve",
            "POST",
            json!({"query":"database adapters"}),
        )
        .await;
    assert!(value["records"].as_array().unwrap().is_empty());
}
#[tokio::test]
async fn mem_crud_cas_disable_search_and_genuine_deletion_are_backend_operations() {
    let f = Fixture::new().await;
    let m = f.create().await;
    let id = m["id"].as_str().unwrap();
    let mut input = f.input(&f.a);
    input["title"] = json!("Renamed database decision");
    let (s, n) = f
        .call(
            &f.a,
            &format!("/{id}"),
            "PATCH",
            json!({"memory":input,"expectedRevision":1,"reason":"rename"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{n}");
    assert_eq!(n["id"], m["id"]);
    assert_eq!(n["revision"], 2);
    let (s, _) = f
        .call(
            &f.a,
            &format!("/{id}"),
            "PATCH",
            json!({"memory":input,"expectedRevision":1}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    input["status"] = json!("disabled");
    let (s, _) = f
        .call(
            &f.a,
            &format!("/{id}"),
            "PATCH",
            json!({"memory":input,"expectedRevision":2}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, r) = f
        .call(
            &f.a,
            "/retrieve",
            "POST",
            json!({"query":"database adapters"}),
        )
        .await;
    assert!(r["records"].as_array().unwrap().is_empty());
    let (_, h) = f
        .call(&f.a, &format!("/{id}/revisions"), "GET", json!({}))
        .await;
    assert_eq!(h.as_array().unwrap().len(), 3);
    let (s, _) = f
        .call(
            &f.a,
            &format!("/{id}"),
            "DELETE",
            json!({"expectedRevision":3}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    for path in [format!("/{id}"), format!("/{id}/revisions")] {
        assert_eq!(
            f.call(&f.a, &path, "GET", json!({})).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (_, list) = f
        .call(&f.a, "/search", "POST", json!({"query":"database"}))
        .await;
    assert!(list["items"].as_array().unwrap().is_empty());
}
#[tokio::test]
async fn settings_are_revisioned_extraction_is_disabled_and_source_reads_use_p05() {
    let f = Fixture::new().await;
    let (_, settings) = f.call(&f.a, "/settings", "GET", json!({})).await;
    assert_eq!(settings["extractionMode"], "disabled");
    let (s, _) = f
        .call(&f.a, "/extract", "POST", json!({"sources":[]}))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let mut changed = settings.clone();
    changed["maxContextBytes"] = json!(2000);
    let (s, n) = f.call(&f.a, "/settings", "PUT", changed).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(n["revision"], 1);
    assert_eq!(
        f.call(&f.a, "/settings", "PUT", settings).await.0,
        StatusCode::CONFLICT
    );
    let context = f
        .app
        .projects
        .context(&factory_projects::ProjectId::parse(&f.a).unwrap())
        .unwrap();
    let root = context.files.primary_root().id;
    let (_,p)=f.call(&f.a,"/sources/resolve","POST",json!({"projectId":f.a,"reference":{"kind":"repository_file","rootId":root,"path":"ARCHITECTURE.md","revision":null}})).await;
    assert!(!p["sourceRevision"].as_str().unwrap().is_empty());
    for path in [".factory/config.toml", ".git/config", "../outside"] {
        let (s,_)=f.call(&f.a,"/sources/resolve","POST",json!({"projectId":f.a,"reference":{"kind":"repository_file","rootId":root,"path":path,"revision":null}})).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
}
