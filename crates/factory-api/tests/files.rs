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
    ar: String,
    br: String,
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
        let mut roots = Vec::new();
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
            std::fs::create_dir(path.join("src")).unwrap();
            std::fs::write(path.join("src/main.rs"), format!("project {name}\n")).unwrap();
            let id = app.projects.register(&path, None).unwrap().project_id;
            let context = app.projects.open(&id).unwrap();
            roots.push(context.files.primary_root().id);
            ids.push(id.to_string());
        }
        let router = application_router(app);
        let credentials = credentials(&router).await;
        Self {
            root,
            router,
            a: ids[0].clone(),
            b: ids[1].clone(),
            ar: roots[0].clone(),
            br: roots[1].clone(),
            credentials,
        }
    }
    fn file(&self, a: bool) -> Value {
        json!({"project_id":if a{&self.a}else{&self.b},"root_id":if a{&self.ar}else{&self.br},"path":"src/main.rs"})
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
    async fn read(&self, a: bool) -> Value {
        let (status, value) = self
            .call(
                a,
                "GET",
                &format!(
                    "files/content?root_id={}&path=src/main.rs",
                    if a { &self.ar } else { &self.br }
                ),
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }
}

#[tokio::test]
async fn file_api_exact_writes_and_project_root_collisions_remain_isolated() {
    let f = Projects::new().await;
    let a = f.read(true).await;
    let b = f.read(false).await;
    assert_eq!(a["content"], "project A\n");
    assert_eq!(b["content"], "project B\n");
    let (status, value) = f
        .call(
            true,
            "PUT",
            "files/content",
            json!({"file":f.file(true),"expected_revision":a["revision"],"content":"A edited\r\n"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(
        std::fs::read(f.root.path().join("A/src/main.rs")).unwrap(),
        b"A edited\r\n"
    );
    assert_eq!(
        f.call(
            true,
            "PUT",
            "files/content",
            json!({"file":f.file(true),"expected_revision":a["revision"],"content":"stale"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call(
            true,
            "PUT",
            "files/content",
            json!({"file":f.file(false),"expected_revision":b["revision"],"content":"cross"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(
            true,
            "GET",
            &format!("files/content?root_id={}&path=src/main.rs", f.br),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(f.read(false).await["content"], "project B\n");
}

#[tokio::test]
async fn fs_sec_12_13_authentication_csrf_and_origin_are_required() {
    let f = Projects::new().await;
    let snapshot = f.read(true).await;
    let body = json!({"file":f.file(true),"expected_revision":snapshot["revision"],"content":"unauthorized"});
    let url = f.route(true, "files/content");
    for (method, credentials, origin, csrf, expected) in [
        (
            "GET",
            None,
            "http://127.0.0.1:4321",
            false,
            StatusCode::UNAUTHORIZED,
        ),
        (
            "PUT",
            Some(&f.credentials),
            "http://127.0.0.1:4321",
            false,
            StatusCode::FORBIDDEN,
        ),
        (
            "PUT",
            Some(&f.credentials),
            "http://evil.test",
            true,
            StatusCode::FORBIDDEN,
        ),
    ] {
        assert_eq!(
            send(
                &f.router,
                method,
                &url,
                body.clone(),
                credentials,
                origin,
                csrf
            )
            .await
            .0,
            expected
        );
    }
    assert_eq!(f.read(true).await["content"], "project A\n");
}

#[tokio::test]
async fn fs_sec_01_02_07_08_20_invalid_paths_fail_without_absolute_path_disclosure() {
    let f = Projects::new().await;
    for path in [
        "../B/src/main.rs",
        "C:/Windows/win.ini",
        "src/main.rs:ads",
        ".git/config",
        ".factory/db.sqlite3",
    ] {
        let body = json!({"file":{"project_id":f.a,"root_id":f.ar,"path":path},"expected_revision":"invalid","content":"bad"});
        let (status, response) = f.call(true, "PUT", "files/content", body).await;
        assert!(status.is_client_error());
        assert!(!response
            .to_string()
            .contains(&f.root.path().to_string_lossy().to_string()));
        assert!(response["code"].is_string());
    }
}

#[tokio::test]
async fn file_crud_search_drafts_and_recovery_round_trip_real_disk() {
    let f = Projects::new().await;
    let file = json!({"project_id":f.a,"root_id":f.ar,"path":"src/new.py"});
    assert_eq!(
        f.call(true, "POST", "files/entries", json!({"file":file}))
            .await
            .0,
        StatusCode::OK
    );
    let (_, read) = f
        .call(
            true,
            "GET",
            &format!("files/content?root_id={}&path=src/new.py", f.ar),
            Value::Null,
        )
        .await;
    let (_, saved) = f
        .call(
            true,
            "PUT",
            "files/content",
            json!({"file":file,"expected_revision":read["revision"],"content":"print('needle')\n"}),
        )
        .await;
    let(_,search)=f.call(true,"POST","files/search",json!({"id":"q_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","root_id":f.ar,"query":"needle","content":true,"case_sensitive":true,"regex":false,"include_hidden":false,"path_filter":""})).await;
    assert_eq!(search["hits"][0]["file"]["path"], "src/new.py");
    assert_eq!(search["hits"][0]["line"], 1);
    let id = "d_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let draft = json!({"schema":1,"id":id,"file":file,"base_revision":saved["revision"],"base_content":"print('needle')\n","content":"unsaved","newline":"lf","bom":false,"dirty":true});
    assert_eq!(
        f.call(
            true,
            "PUT",
            &format!("editor/drafts/{id}"),
            json!({"base_revision":0,"document":draft})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(false, "GET", &format!("editor/drafts/{id}"), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let(status,moved)=f.call(true,"POST","files/move",json!({"file":file,"expected_revision":saved["revision"],"destination":"src/renamed.py"})).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    let (_, read) = f
        .call(
            true,
            "GET",
            &format!("files/content?root_id={}&path=src/renamed.py", f.ar),
            Value::Null,
        )
        .await;
    let (_, deleted) = f
        .call(
            true,
            "DELETE",
            "files/entries",
            json!({"file":moved["to"],"expected_revision":read["revision"],"confirmed":true}),
        )
        .await;
    assert!(!f.root.path().join("A/src/renamed.py").exists());
    let id = deleted["id"].as_str().unwrap();
    assert_eq!(
        f.call(
            true,
            "POST",
            &format!("files/recovery/{id}/restore"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("A/src/renamed.py")).unwrap(),
        "print('needle')\n"
    );
}

#[tokio::test]
async fn legacy_non_git_project_keeps_files_available_when_worktree_discovery_fails() {
    let fixture = Projects::new().await;
    let path = fixture.root.path().join("legacy");
    std::fs::create_dir(&path).unwrap();
    factory_core::Factory::init(&path).unwrap();
    std::fs::write(path.join("source.py"), "legacy = True\n").unwrap();
    let (status, project, _) = send(
        &fixture.router,
        "POST",
        "/api/projects/register",
        json!({"path":path,"trust_project":true}),
        Some(&fixture.credentials),
        "http://127.0.0.1:4321",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let id = project["project_id"].as_str().unwrap();
    let (status, opened, _) = send(
        &fixture.router,
        "POST",
        &format!("/api/projects/{id}/open"),
        json!({}),
        Some(&fixture.credentials),
        "http://127.0.0.1:4321",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let (status, roots, _) = send(
        &fixture.router,
        "GET",
        &format!("/api/projects/{id}/files/roots"),
        Value::Null,
        Some(&fixture.credentials),
        "http://127.0.0.1:4321",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{roots}");
    assert!(roots["warning"].is_string());
    let root = roots["roots"][0]["id"].as_str().unwrap();
    let (status, file, _) = send(
        &fixture.router,
        "GET",
        &format!("/api/projects/{id}/files/content?root_id={root}&path=source.py"),
        Value::Null,
        Some(&fixture.credentials),
        "http://127.0.0.1:4321",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{file}");
    assert_eq!(file["content"], "legacy = True\n");
}
