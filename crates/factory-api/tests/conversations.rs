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

#[tokio::test]
async fn conversation_api_preserves_project_ownership_and_authorization() {
    let f = Projects::new().await;
    let (status, c) = f
        .call(true, "POST", "conversations", json!({"title":"Same title"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{c}");
    let id = c["id"].as_str().unwrap();
    let path = format!("conversations/{id}");
    assert_eq!(
        f.call(true, "GET", &path, Value::Null).await.0,
        StatusCode::OK
    );
    for suffix in ["", "/messages", "/missions", "/events"] {
        assert_eq!(
            f.call(false, "GET", &format!("{path}{suffix}"), Value::Null)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        f.call(
            false,
            "POST",
            &format!("{path}/messages"),
            json!({"content":"no","key":"no","workspace_session_id":format!("s_{}","a".repeat(32))})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for (credentials, origin, csrf) in [
        (None, "http://127.0.0.1:4321", false),
        (Some(&f.credentials), "http://127.0.0.1:4321", false),
        (Some(&f.credentials), "https://evil.example", true),
    ] {
        let (status, _, _) = send(
            &f.router,
            "POST",
            &f.route(true, "conversations"),
            json!({"title":"unauthorized"}),
            credentials,
            origin,
            csrf,
        )
        .await;
        assert!(!status.is_success());
    }
    assert_eq!(
        f.call(true, "GET", "conversations/bad-id", Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call(true, "GET", "conversations?limit=1000", Value::Null)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (status, renamed) = f
        .call(
            true,
            "PATCH",
            &path,
            json!({"title":"Renamed","status":"active","revision":1}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["revision"], 2);
    assert_eq!(
        f.call(
            true,
            "PATCH",
            &path,
            json!({"title":"stale","status":"active","revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let disk = f.read(true).await;
    let forbidden = json!({"key":"foreign","workspace_session_id":format!("s_{}","a".repeat(32)),"content":"Question","context":[{"file":f.file(false),"revision":disk["revision"]}]});
    assert_eq!(
        f.call(true, "POST", &format!("{path}/messages"), forbidden)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert!(f.root.path().join("A/.factory/db.sqlite3").is_file());
    assert_ne!(f.ar, f.br);
}

#[tokio::test]
async fn conversation_message_acceptance_is_durable_even_when_no_agent_exists() {
    let f = Projects::new().await;
    let (_, c) = f
        .call(true, "POST", "conversations", json!({"title":"No agent"}))
        .await;
    let id = c["id"].as_str().unwrap();
    let body = json!({"key":"once","workspace_session_id":format!("s_{}","a".repeat(32)),"content":"Why?"});
    let path = format!("conversations/{id}/messages");
    let (status, accepted) = f.call(true, "POST", &path, body.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let (status, again) = f.call(true, "POST", &path, body).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(again["id"], accepted["id"]);
    for _ in 0..100 {
        let (_, messages) = f.call(true, "GET", &path, Value::Null).await;
        if messages[0]["status"] == "failed" {
            assert_eq!(messages.as_array().unwrap().len(), 2);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("failed message was not recorded");
}

#[tokio::test]
async fn conversation_sse_slow_subscriber_is_bounded_and_replayable() {
    let f = Projects::new().await;
    let (_, c) = f
        .call(
            true,
            "POST",
            "conversations",
            json!({"title":"Slow subscriber"}),
        )
        .await;
    let id = c["id"].as_str().unwrap();
    let db = factory_db::FactoryDb::open(&f.root.path().join("A/.factory/db.sqlite3")).unwrap();
    for number in 0..100 {
        let (command, _) = db
            .conversation_accept(
                id,
                &format!("fixture-{number}"),
                "hash",
                "chat",
                "controlled protocol history",
                &json!({}),
            )
            .unwrap();
        db.conversation_state(
            command.id,
            "finished",
            "recorded response",
            &json!({"source":"fixture"}),
            None,
        )
        .unwrap();
    }
    let request = Request::builder()
        .uri(f.route(true, &format!("conversations/{id}/events")))
        .header("host", "127.0.0.1:4321")
        .header("origin", "http://127.0.0.1:4321")
        .header("cookie", &f.credentials.0)
        .body(Body::empty())
        .unwrap();
    let response = f.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        response.into_body().collect(),
    )
    .await
    .unwrap()
    .unwrap()
    .to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let delivered = text.matches("event: conversation").count();
    assert!((1..=16).contains(&delivered), "{delivered}");
    assert_eq!(db.conversation_events(id, 0, 100).unwrap().len(), 100);
    assert_eq!(
        f.call(
            true,
            "GET",
            &format!("conversations/{id}/messages"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn cancelled_response_replays_through_its_own_authenticated_stream() {
    let f = Projects::new().await;
    let (_, c) = f
        .call(
            true,
            "POST",
            "conversations",
            json!({"title":"Cancelled response"}),
        )
        .await;
    let id = c["id"].as_str().unwrap();
    let db = factory_db::FactoryDb::open(&f.root.path().join("A/.factory/db.sqlite3")).unwrap();
    let (command, _) = db
        .conversation_accept(id, "stopped", "hash", "chat", "cancel fixture", &json!({}))
        .unwrap();
    let cursor = db
        .conversation_events(id, 0, 100)
        .unwrap()
        .last()
        .unwrap()
        .id;
    db.conversation_finish_stopped_agent(
        command.id,
        "cancelled",
        "Interaction cancelled.",
        "user_cancelled",
        None,
    )
    .unwrap();
    assert_eq!(
        f.call(
            false,
            "POST",
            &format!("conversations/{id}/commands/{}/cancel", command.id),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let request = Request::builder()
        .uri(f.route(true, &format!("conversations/{id}/events")))
        .header("host", "127.0.0.1:4321")
        .header("origin", "http://127.0.0.1:4321")
        .header("cookie", &f.credentials.0)
        .header("last-event-id", cursor.to_string())
        .body(Body::empty())
        .unwrap();
    let response = f.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    let event = std::str::from_utf8(&event).unwrap();
    assert!(event.contains("conversation.response.cancelled"), "{event}");
    assert!(event.contains("user_cancelled"));
    assert!(!event.contains("conversation.response.failed"));
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
            factory_core::Config::default().write_atomic(&path).unwrap();
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
