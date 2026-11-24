use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use factory_api::{application_router, ApiOptions, ApplicationOptions, FactoryApplication};
use factory_projects::ProjectLimits;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn application(root: &std::path::Path) -> Router {
    application_router(Arc::new(
        FactoryApplication::new(ApplicationOptions {
            registry_path: root.join("registry.sqlite3"),
            allowed_roots: vec![root.to_owned()],
            default_root: None,
            dashboard_root: root.to_owned(),
            limits: ProjectLimits::default(),
            api: ApiOptions::default(),
        })
        .unwrap(),
    ))
}

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
fn update(id: &str, revision: u64) -> Value {
    json!({"base_revision":revision,"document":{"id":id,"version":2,"openProjects":[],"sessions":[],"activeProjectId":null,"activeSessionId":null,"preferences":{"theme":"dark"}}})
}

#[tokio::test]
async fn workspace_api_enforces_auth_origin_csrf_and_revision_ownership() {
    let root = tempfile::tempdir().unwrap();
    let router = application(root.path());
    let id = "w_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let url = format!("/api/workspaces/{id}");
    assert_eq!(
        send(
            &router,
            "GET",
            "/api/workspaces",
            Value::Null,
            None,
            "http://127.0.0.1:4321",
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let credentials = credentials(&router).await;
    assert_eq!(
        send(
            &router,
            "PUT",
            &url,
            update(id, 0),
            Some(&credentials),
            "http://127.0.0.1:4321",
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &router,
            "PUT",
            &url,
            update(id, 0),
            Some(&credentials),
            "https://untrusted.example",
            true
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, first, _) = send(
        &router,
        "PUT",
        &url,
        update(id, 0),
        Some(&credentials),
        "http://127.0.0.1:4321",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["revision"], 1);
    assert_eq!(
        send(
            &router,
            "PUT",
            &url,
            update(id, 0),
            Some(&credentials),
            "http://127.0.0.1:4321",
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, current, _) = send(
        &router,
        "GET",
        &url,
        Value::Null,
        Some(&credentials),
        "http://127.0.0.1:4321",
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current, first);
    let second = "w_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    assert_eq!(
        send(
            &router,
            "PUT",
            &format!("/api/workspaces/{second}"),
            update(second, 0),
            Some(&credentials),
            "http://127.0.0.1:4321",
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    let response = send(
        &router,
        "GET",
        "/api/workspaces",
        Value::Null,
        Some(&credentials),
        "http://127.0.0.1:4321",
        false,
    )
    .await;
    assert_eq!(response.1.as_array().unwrap().len(), 2);
    drop(router);
    let restarted = application(root.path());
    let credentials = self::credentials(&restarted).await;
    let response = send(
        &restarted,
        "GET",
        &url,
        Value::Null,
        Some(&credentials),
        "http://127.0.0.1:4321",
        false,
    )
    .await;
    assert_eq!(response.1, first);
}

#[tokio::test]
async fn workspace_bounds_unknown_schema_and_corruption_do_not_break_project_registry() {
    let root = tempfile::tempdir().unwrap();
    let router = application(root.path());
    let credentials = credentials(&router).await;
    let id = "w_cccccccccccccccccccccccccccccccc";
    let url = format!("/api/workspaces/{id}");
    assert_eq!(
        send(
            &router,
            "GET",
            "/api/workspaces/not-an-id",
            Value::Null,
            Some(&credentials),
            "http://127.0.0.1:4321",
            false
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(
            &router,
            "GET",
            &url,
            Value::Null,
            Some(&credentials),
            "http://127.0.0.1:4321",
            false
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut unknown = update(id, 0);
    unknown["document"]["version"] = json!(99);
    assert_eq!(
        send(
            &router,
            "PUT",
            &url,
            unknown,
            Some(&credentials),
            "http://127.0.0.1:4321",
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut forbidden = update(id, 0);
    forbidden["document"]["credentials"] = json!({"synthetic_secret":"not allowed"});
    assert_eq!(
        send(
            &router,
            "PUT",
            &url,
            forbidden,
            Some(&credentials),
            "http://127.0.0.1:4321",
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut huge = update(id, 0);
    huge["document"]["sessions"] = json!(["x".repeat(1_048_577)]);
    assert_eq!(
        send(
            &router,
            "PUT",
            &url,
            huge,
            Some(&credentials),
            "http://127.0.0.1:4321",
            true
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    send(
        &router,
        "PUT",
        &url,
        update(id, 0),
        Some(&credentials),
        "http://127.0.0.1:4321",
        true,
    )
    .await;
    let db = rusqlite::Connection::open(root.path().join("registry.workspaces.sqlite3")).unwrap();
    db.pragma_update(None, "user_version", 99).unwrap();
    assert_eq!(
        send(
            &router,
            "GET",
            &url,
            Value::Null,
            Some(&credentials),
            "http://127.0.0.1:4321",
            false
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        send(
            &router,
            "GET",
            "/api/projects",
            Value::Null,
            Some(&credentials),
            "http://127.0.0.1:4321",
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        db.query_row("SELECT revision FROM workspaces WHERE id=?1", [id], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        1
    );
}
