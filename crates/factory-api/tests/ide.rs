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

fn shell_profile() -> Value {
    #[cfg(windows)]
    let executable =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
    #[cfg(not(windows))]
    let executable = std::path::PathBuf::from("/bin/sh");
    json!({"id":"shell","name":"Controlled shell","kind":"shell","executable":executable,"arguments":[],"languages":[],"enabled":true,"timeout_ms":5000})
}

#[tokio::test]
async fn corrupt_ide_metadata_does_not_disable_files_or_v1_workflows() {
    let f = Projects::with_corrupt_ide(true).await;
    assert_eq!(
        f.call(true, "GET", "ide/state", Value::Null).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        f.call(true, "GET", "runs?limit=10", Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            false,
            "GET",
            &format!("files/content?root_id={}&path=src/main.rs", f.br),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("registry.ide.sqlite3")).unwrap(),
        "intentionally corrupt isolated fixture"
    );
}

#[tokio::test]
async fn ide_auth_csrf_origin_and_profile_approval() {
    let f = Projects::new().await;
    let body = json!({"profile":shell_profile(),"confirmed":true});
    for (credentials, origin, csrf) in [
        (None, "http://127.0.0.1:4321", false),
        (Some(&f.credentials), "http://127.0.0.1:4321", false),
        (Some(&f.credentials), "https://evil.example", true),
    ] {
        let (status, _, _) = send(
            &f.router,
            "POST",
            &f.route(true, "ide/profiles"),
            body.clone(),
            credentials,
            origin,
            csrf,
        )
        .await;
        assert!(!status.is_success());
    }
    assert_eq!(
        f.call(
            true,
            "POST",
            "ide/profiles",
            json!({"profile":shell_profile(),"confirmed":false})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(true, "POST", "ide/profiles", body).await.0,
        StatusCode::OK
    );
    let (status, state) = f.call(false, "GET", "ide/state", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["profiles"], json!([]));
    assert_eq!(
        f.call(
            true,
            "POST",
            "ide/lsp/start",
            json!({"profile_id":"shell","root_id":f.ar,"client_id":"test","confirmed":true})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn development_terminal_ownership_and_authorized_roots() {
    let f = Projects::new().await;
    f.call(
        true,
        "POST",
        "ide/profiles",
        json!({"profile":shell_profile(),"confirmed":true}),
    )
    .await;
    f.call(
        false,
        "POST",
        "ide/profiles",
        json!({"profile":shell_profile(),"confirmed":true}),
    )
    .await;
    let start = |root: &str, confirmed: bool| json!({"root_id":root,"profile_id":"shell","confirmed":confirmed,"cols":80,"rows":24});
    assert_eq!(
        f.call(true, "POST", "ide/terminals/start", start(&f.ar, false))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(true, "POST", "ide/terminals/start", start(&f.br, true))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, a) = f
        .call(true, "POST", "ide/terminals/start", start(&f.ar, true))
        .await;
    assert_eq!(status, StatusCode::OK, "{a}");
    let (status, b) = f
        .call(false, "POST", "ide/terminals/start", start(&f.br, true))
        .await;
    assert_eq!(status, StatusCode::OK, "{b}");
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();
    assert_ne!(a_id, b_id);
    assert_eq!(
        f.call(
            false,
            "POST",
            &format!("ide/terminals/{a_id}/stop"),
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call(
            true,
            "POST",
            &format!("ide/terminals/{a_id}/rename"),
            json!({"name":"A shell"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            true,
            "POST",
            &format!("ide/terminals/{a_id}/stop"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, state) = f.call(false, "GET", "ide/state", Value::Null).await;
    assert_eq!(state["terminals"][0]["id"], b_id);
    assert!(state["terminals"][0]["exit_code"].is_null());
    assert_eq!(
        f.call(
            false,
            "POST",
            &format!("ide/terminals/{b_id}/stop"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn ide_resource_ids_never_fall_back_to_another_project() {
    let f = Projects::new().await;
    for subsystem in ["lsp", "dap"] {
        for action in ["request", "stop"] {
            assert_eq!(
                f.call(
                    false,
                    "POST",
                    &format!("ide/{subsystem}/i_00000000000000000000000000000001/{action}"),
                    json!({"command":"threads","method":"textDocument/hover","params":{}})
                )
                .await
                .0,
                StatusCode::NOT_FOUND
            );
        }
    }
    let missing = "p_ffffffffffffffffffffffffffffffff";
    let (status, _, _) = send(
        &f.router,
        "GET",
        &format!("/api/projects/{missing}/ide/state"),
        Value::Null,
        Some(&f.credentials),
        "http://127.0.0.1:4321",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(f.root.path().join("A/.factory/db.sqlite3").is_file());
    let file = f.file(true);
    assert_eq!(file["root_id"], f.ar);
}

#[tokio::test]
async fn declarative_extensions_reject_backend_permissions_and_isolate_activation() {
    let f = Projects::new().await;
    let manifest = json!({"id":"asf.test","name":"Test","version":"1.0.0","publisher":"asf","description":"Test contribution","entrypoint":"contributions","extension_api_version":1,"capabilities":["views"],"permissions":["views"],"supported_platforms":["any"],"contributions":{"views":[{"id":"view","title":"Test view","text":"Hello"}]}});
    assert_eq!(
        f.call(
            true,
            "POST",
            "ide/extensions/install",
            json!({"manifest":manifest,"trusted":false})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(
            true,
            "POST",
            "ide/extensions/install",
            json!({"manifest":manifest,"trusted":true})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            true,
            "POST",
            "ide/extensions/install",
            json!({"manifest":manifest,"trusted":true})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call(false, "POST", "ide/extensions/asf.test/enable", json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call(true, "POST", "ide/extensions/asf.test/enable", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    let mut invalid = manifest.clone();
    invalid["permissions"] = json!(["process"]);
    assert_eq!(
        f.call(
            false,
            "POST",
            "ide/extensions/install",
            json!({"manifest":invalid,"trusted":true})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut invalid = manifest;
    invalid["extension_api_version"] = json!(99);
    assert_eq!(
        f.call(
            false,
            "POST",
            "ide/extensions/install",
            json!({"manifest":invalid,"trusted":true})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call(true, "POST", "ide/extensions/asf.test/disable", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(true, "POST", "ide/extensions/asf.test/uninstall", json!({}))
            .await
            .0,
        StatusCode::OK
    );
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
    router: Router,
    a: String,
    b: String,
    ar: String,
    br: String,
    credentials: (String, String),
    root: tempfile::TempDir,
}
impl Projects {
    async fn new() -> Self {
        Self::with_corrupt_ide(false).await
    }
    async fn with_corrupt_ide(corrupt: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        if corrupt {
            std::fs::write(
                root.path().join("registry.ide.sqlite3"),
                "intentionally corrupt isolated fixture",
            )
            .unwrap();
        }
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
}
