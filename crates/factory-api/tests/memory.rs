use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use factory_api::{application_router, ApiOptions, ApplicationOptions, FactoryApplication};
use factory_memory::RepositorySourceDiagnostic;
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

type ObservedResponse = (StatusCode, Value, Option<RepositorySourceDiagnostic>);

fn report_git_fixture(root: &std::path::Path) {
    let probe = |argument| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", argument])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| output.stdout)
    };
    let discovered = probe("--is-inside-work-tree")
        .is_some_and(|output| String::from_utf8_lossy(&output).trim() == "true");
    let head = probe("HEAD").is_some_and(|output| {
        let text = String::from_utf8_lossy(&output);
        matches!(text.trim().len(), 40 | 64)
            && text.trim().bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    eprintln!(
        "ASF_QA_DETAIL:memory.source.fixture_git={}",
        u8::from(discovered)
    );
    eprintln!(
        "ASF_QA_DETAIL:memory.source.fixture_head={}",
        u8::from(head)
    );
}

fn report_response(label: &str, (status, body, detail): &ObservedResponse) {
    let code = match body["code"].as_str() {
        Some("invalid_memory_source") => "invalid_memory_source",
        Some("sensitive_memory_content") => "sensitive_memory_content",
        Some("memory_policy_denied") => "memory_policy_denied",
        Some("memory_limit") => "memory_limit",
        Some("memory_busy_or_limit") => "memory_busy_or_limit",
        Some("stale_memory_revision") => "stale_memory_revision",
        Some("memory_provider_unavailable") => "memory_provider_unavailable",
        Some("memory_storage_or_execution_unavailable") => {
            "memory_storage_or_execution_unavailable"
        }
        _ if status.is_success() => "ok",
        _ => "unknown",
    };
    eprintln!("ASF_QA_DETAIL:{label}.status={}", status.as_u16());
    eprintln!("ASF_QA_DETAIL:{label}.code={code}");
    if let Some(detail) = detail {
        eprintln!("ASF_QA_DETAIL:{label}.phase={}", detail.phase());
        eprintln!("ASF_QA_DETAIL:{label}.category={}", detail.category());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_cancels_an_owned_manual_memory_extractor_without_waiting_for_its_timeout() {
    let f = Arc::new(Fixture::new().await);
    let context = f
        .app
        .projects
        .context(&factory_projects::ProjectId::parse(&f.a).unwrap())
        .expect("ASF_QA_CHECKPOINT:memory.shutdown.context");
    let script = context.root.join(".factory/shutdown-extractor.py");
    let ready_file = script.with_extension("ready");
    let start_file = script.with_extension("started");
    let input_file = script.with_extension("input");
    std::fs::write(
        &script,
        "import sys,time\nfrom pathlib import Path\n\
         Path(__file__).with_suffix('.started').write_bytes(b'started')\n\
         assert sys.stdin.buffer.read(1)\n\
         Path(__file__).with_suffix('.input').write_bytes(b'input')\n\
         sys.stdin.buffer.read()\n\
         Path(__file__).with_suffix('.ready').write_bytes(b'ready')\n\
         time.sleep(30)\nprint('{\"candidates\":[]}')\n",
    )
    .expect("ASF_QA_CHECKPOINT:memory.shutdown.fixture_script");
    let mut config = factory_core::Config::default();
    config.agents.insert("shutdown-extractor".into(), serde_json::from_value(json!({"kind":"custom","command":if cfg!(windows) {"python"} else {"python3"},"args":[script],"env":{"PYTHONDONTWRITEBYTECODE":"1"}})).unwrap());
    config.role_assignments.push(factory_core::RoleAssignment {
        role: "planner".into(),
        agent: "shutdown-extractor".into(),
        preferred: true,
    });
    config
        .write_atomic(&context.root)
        .expect("ASF_QA_CHECKPOINT:memory.shutdown.fixture_config");
    let (_, mut settings) = f.call(&f.a, "/settings", "GET", json!({})).await;
    settings["extractionMode"] = json!("review");
    assert_eq!(
        f.call(&f.a, "/settings", "PUT", settings).await.0,
        StatusCode::OK,
        "ASF_QA_CHECKPOINT:memory.shutdown.settings"
    );
    let source = json!({"projectId":f.a,"reference":{"kind":"repository_file","rootId":context.files.primary_root().id,"path":"ARCHITECTURE.md","revision":null}});
    let lookup = f
        .call_observed(&f.a, "/sources/resolve", "POST", source.clone())
        .await;
    report_response("memory.source", &lookup);
    report_git_fixture(&context.root);
    eprintln!(
        "ASF_QA_DETAIL:memory.source.revision={}",
        u8::from(
            lookup.1["sourceRevision"]
                .as_str()
                .is_some_and(|revision| !revision.is_empty())
        )
    );
    let client = f.clone();
    let mut extraction = tokio::spawn(async move {
        client
            .call_observed(&client.a, "/extract", "POST", json!({"sources":[source]}))
            .await
    });
    // The owned child acknowledges nonempty stdin and EOF before shutdown.
    // The session receipt also proves the extractor is live and supervised.
    let ready = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !extraction.is_finished() {
            if std::fs::read(&ready_file).is_ok_and(|value| value == b"ready")
                && context
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
                    })
            {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        false
    })
    .await
    .unwrap_or(false);
    let (admitted, planner, spawn, running, supervised) = {
        let db = context.db.lock().unwrap();
        let admitted = !db.memory_extractions(&f.a).unwrap().is_empty();
        let sessions = db.list_agent_sessions(None).unwrap();
        let planner = sessions
            .iter()
            .find(|session| session.role == "planner" && session.agent == "shutdown-extractor");
        let metadata = planner.and_then(|session| session.output_meta.as_ref());
        (
            admitted,
            planner.is_some(),
            metadata.is_some_and(|value| value.process_started_unix_ms.is_some()),
            planner.is_some_and(|session| session.status == "running"),
            metadata.is_some_and(|value| value.process_tree_supervised),
        )
    };
    let early = if !ready && extraction.is_finished() {
        Some((&mut extraction).await)
    } else {
        None
    };
    if let Some(Ok(receipt)) = &early {
        report_response("memory.extract", receipt);
    }
    let started = std::fs::read(&start_file).is_ok_and(|value| value == b"started");
    let input = std::fs::read(&input_file).is_ok_and(|value| value == b"input");
    let eof = std::fs::read(&ready_file).is_ok_and(|value| value == b"ready");
    for (stage, observed) in [
        ("admitted", admitted),
        ("sources_resolved", admitted),
        ("planner_registered", planner),
        ("spawn_observed", spawn || started),
        ("spawn_completed", spawn || started),
        ("running_session", running),
        ("supervised", supervised),
        ("input", input),
        ("eof_ack", eof),
        ("early_response", early.is_some()),
    ] {
        eprintln!("ASF_QA_DETAIL:memory.ready.{stage}={}", u8::from(observed));
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
    eprintln!("ASF_QA_DETAIL:memory.ready.cancel_requested=1");
    let timely = match early {
        Some(result) => Ok(result),
        None => tokio::time::timeout(std::time::Duration::from_secs(8), &mut extraction).await,
    };
    let shutdown = tokio::time::timeout(std::time::Duration::from_secs(45), server)
        .await
        .expect("ASF_QA_CHECKPOINT:memory.shutdown.deadline")
        .expect("ASF_QA_CHECKPOINT:memory.shutdown.server_join");
    assert!(ready, "ASF_QA_CHECKPOINT:memory.shutdown.child_ready");
    let response = timely
        .expect("ASF_QA_CHECKPOINT:memory.shutdown.cancellation")
        .expect("ASF_QA_CHECKPOINT:memory.shutdown.extraction_join");
    report_response("memory.extract", &response);
    let (status, _, _) = response;
    eprintln!("ASF_QA_DETAIL:memory.ready.cancel_resolved=1");
    assert!(shutdown.is_ok(), "ASF_QA_CHECKPOINT:memory.shutdown.server");
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "ASF_QA_CHECKPOINT:memory.shutdown.interrupted_status"
    );
    let sessions = context
        .db
        .lock()
        .unwrap()
        .list_agent_sessions(None)
        .unwrap();
    assert_eq!(
        sessions.len(),
        1,
        "ASF_QA_CHECKPOINT:memory.shutdown.session_count"
    );
    assert!(
        sessions[0].status != "running",
        "ASF_QA_CHECKPOINT:memory.shutdown.session_terminal"
    );
    assert!(
        sessions[0]
            .output_meta
            .as_ref()
            .is_some_and(|meta| meta.process_tree_cleanup_confirmed),
        "ASF_QA_CHECKPOINT:memory.shutdown.process_cleanup"
    );
    eprintln!("ASF_QA_DETAIL:memory.ready.cleanup_confirmed=1");
    let records = context.db.lock().unwrap().memory_extractions(&f.a).unwrap();
    assert_eq!(
        records.len(),
        1,
        "ASF_QA_CHECKPOINT:memory.shutdown.record_count"
    );
    assert!(
        records[0].state == "failed",
        "ASF_QA_CHECKPOINT:memory.shutdown.record_state"
    );
    assert!(
        records[0]
            .error
            .as_ref()
            .is_some_and(|error| error.starts_with("INTERRUPTED")),
        "ASF_QA_CHECKPOINT:memory.shutdown.record_interrupted"
    );
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().expect("ASF_QA_CHECKPOINT:memory.fixture.tempdir");
        let app = Arc::new(
            FactoryApplication::new(ApplicationOptions {
                registry_path: dir.path().join("registry.sqlite3"),
                allowed_roots: vec![dir.path().into()],
                default_root: None,
                dashboard_root: dir.path().into(),
                limits: ProjectLimits::default(),
                api: ApiOptions::default(),
            })
            .expect("ASF_QA_CHECKPOINT:memory.fixture.application"),
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
                assert!(
                    std::process::Command::new("git")
                        .arg("-C")
                        .arg(&path)
                        .args(args)
                        .output()
                        .expect("ASF_QA_CHECKPOINT:memory.fixture.git_spawn")
                        .status
                        .success(),
                    "ASF_QA_CHECKPOINT:memory.fixture.git_status"
                );
            }
            factory_core::Factory::init(&path).unwrap();
            factory_core::Config::default().write_atomic(&path).unwrap();
            let id = app
                .projects
                .register(&path, None)
                .expect("ASF_QA_CHECKPOINT:memory.fixture.register")
                .project_id;
            app.projects
                .open(&id)
                .expect("ASF_QA_CHECKPOINT:memory.fixture.open");
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
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "ASF_QA_CHECKPOINT:memory.fixture.auth"
        );
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
        let (status, body, _) = self.call_observed(project, path, method, body).await;
        (status, body)
    }

    async fn call_observed(
        &self,
        project: &str,
        path: &str,
        method: &str,
        body: Value,
    ) -> ObservedResponse {
        self.raw_observed(
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
        let (status, body, _) = self
            .raw_observed(path, method, body, auth, csrf, origin)
            .await;
        (status, body)
    }

    async fn raw_observed(
        &self,
        path: &str,
        method: &str,
        body: Value,
        auth: bool,
        csrf: bool,
        origin: &str,
    ) -> ObservedResponse {
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
        let detail = response
            .extensions()
            .get::<RepositorySourceDiagnostic>()
            .copied();
        let text = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&text).unwrap_or(Value::Null),
            detail,
        )
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
    let (status, settings) = f.call(&f.a, "/settings", "GET", json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "ASF_QA_CHECKPOINT:memory.settings.read"
    );
    assert!(
        settings["extractionMode"] == "disabled",
        "ASF_QA_CHECKPOINT:memory.settings.default"
    );
    let (s, _) = f
        .call(&f.a, "/extract", "POST", json!({"sources":[]}))
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "ASF_QA_CHECKPOINT:memory.settings.disabled"
    );
    let mut changed = settings.clone();
    changed["maxContextBytes"] = json!(2000);
    let (s, n) = f.call(&f.a, "/settings", "PUT", changed).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "ASF_QA_CHECKPOINT:memory.settings.update"
    );
    assert!(
        n["revision"] == 1,
        "ASF_QA_CHECKPOINT:memory.settings.revision"
    );
    assert_eq!(
        f.call(&f.a, "/settings", "PUT", settings).await.0,
        StatusCode::CONFLICT,
        "ASF_QA_CHECKPOINT:memory.settings.stale"
    );
    let context = f
        .app
        .projects
        .context(&factory_projects::ProjectId::parse(&f.a).unwrap())
        .expect("ASF_QA_CHECKPOINT:memory.settings.context");
    let root = context.files.primary_root().id;
    assert!(
        !root.is_empty(),
        "ASF_QA_CHECKPOINT:memory.settings.p05_root"
    );
    let lookup=f.call_observed(&f.a,"/sources/resolve","POST",json!({"projectId":f.a,"reference":{"kind":"repository_file","rootId":root,"path":"ARCHITECTURE.md","revision":null}})).await;
    report_response("memory.source", &lookup);
    report_git_fixture(&context.root);
    let (s, p, _) = lookup;
    eprintln!(
        "ASF_QA_DETAIL:memory.source.revision={}",
        u8::from(
            p["sourceRevision"]
                .as_str()
                .is_some_and(|revision| !revision.is_empty())
        )
    );
    assert_eq!(
        s,
        StatusCode::OK,
        "ASF_QA_CHECKPOINT:memory.settings.source_resolution"
    );
    assert!(
        p["sourceRevision"]
            .as_str()
            .is_some_and(|revision| !revision.is_empty()),
        "ASF_QA_CHECKPOINT:memory.settings.source_revision"
    );
    for path in [".factory/config.toml", ".git/config", "../outside"] {
        let (s,_)=f.call(&f.a,"/sources/resolve","POST",json!({"projectId":f.a,"reference":{"kind":"repository_file","rootId":root,"path":path,"revision":null}})).await;
        assert_eq!(
            s,
            StatusCode::FORBIDDEN,
            "ASF_QA_CHECKPOINT:memory.settings.source_denied"
        );
    }
}

#[tokio::test]
async fn rejected_p05_source_retains_typed_diagnostics_and_blocks_extraction() {
    let f = Fixture::new().await;
    let context = f
        .app
        .projects
        .context(&factory_projects::ProjectId::parse(&f.a).unwrap())
        .unwrap();
    let root = context.files.primary_root().id;
    let source = |root: &str, path: &str, revision: Value| {
        json!({"projectId":f.a,"reference":{
            "kind":"repository_file","rootId":root,"path":path,"revision":revision
        }})
    };
    for (reference, phase, category) in [
        (
            source("unrecognized-root", "ARCHITECTURE.md", Value::Null),
            "root",
            "unknown_workspace_root",
        ),
        (
            source(&root, "absent.md", Value::Null),
            "open",
            "file_not_found",
        ),
        (
            source(&root, "ARCHITECTURE.md", json!("stale")),
            "revision",
            "file_revision_conflict",
        ),
    ] {
        let (status, body, observed) = f
            .call_observed(&f.a, "/sources/resolve", "POST", reference)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "invalid_memory_source");
        assert_eq!(body.as_object().unwrap().len(), 2);
        let observed = observed.unwrap();
        assert_eq!((observed.phase(), observed.category()), (phase, category));
    }
    let (_, mut settings) = f.call(&f.a, "/settings", "GET", json!({})).await;
    settings["extractionMode"] = json!("review");
    assert_eq!(
        f.call(&f.a, "/settings", "PUT", settings).await.0,
        StatusCode::OK
    );
    let (status, body, observed) = f
        .call_observed(
            &f.a,
            "/extract",
            "POST",
            json!({"sources":[source(&root, "absent.md", Value::Null)]}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "invalid_memory_source");
    assert_eq!(observed.unwrap().category(), "file_not_found");
    let db = context.db.lock().unwrap();
    assert!(db.memory_extractions(&f.a).unwrap().is_empty());
    assert!(db.list_agent_sessions(None).unwrap().is_empty());
}
