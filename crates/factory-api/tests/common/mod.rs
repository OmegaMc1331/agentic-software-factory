#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

const TEST_HOST: &str = "127.0.0.1:4321";
const TEST_ORIGIN: &str = "http://127.0.0.1:4321";

pub async fn authenticated_app(state: Arc<factory_api::ApiState>) -> Router {
    let (session, csrf) = bootstrap(&state).await;
    let app = factory_api::router(state);
    app.layer(middleware::from_fn(
        move |mut request: Request, next: Next| {
            let session = session.clone();
            let csrf = csrf.clone();
            async move {
                if !request.headers().contains_key(header::HOST) {
                    request
                        .headers_mut()
                        .insert(header::HOST, HeaderValue::from_static(TEST_HOST));
                }
                if !request.headers().contains_key(header::ORIGIN) {
                    request
                        .headers_mut()
                        .insert(header::ORIGIN, HeaderValue::from_static(TEST_ORIGIN));
                }
                if !request.headers().contains_key(header::COOKIE) {
                    request.headers_mut().insert(
                        header::COOKIE,
                        HeaderValue::from_str(&format!(
                            "factory_session={session}; factory_csrf={csrf}"
                        ))
                        .expect("test cookie"),
                    );
                }
                if matches!(
                    *request.method(),
                    Method::POST | Method::PUT | Method::PATCH | Method::DELETE
                ) && !request
                    .headers()
                    .contains_key(header::HeaderName::from_static("x-factory-csrf"))
                {
                    request.headers_mut().insert(
                        header::HeaderName::from_static("x-factory-csrf"),
                        HeaderValue::from_str(&csrf).expect("test csrf"),
                    );
                }
                next.run(request).await
            }
        },
    ))
}

async fn bootstrap(state: &Arc<factory_api::ApiState>) -> (String, String) {
    let response = factory_api::router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/session")
                .header(header::HOST, TEST_HOST)
                .header(header::ORIGIN, TEST_ORIGIN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("bootstrap request"),
        )
        .await
        .expect("bootstrap response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        status.is_success(),
        "bootstrap failed: {status} {}",
        String::from_utf8_lossy(&body)
    );

    let mut session = None;
    let mut csrf = None;
    for value in headers.get_all(header::SET_COOKIE) {
        let value = value.to_str().expect("cookie header");
        let pair = value.split(';').next().expect("cookie pair");
        if let Some(value) = pair.strip_prefix("factory_session=") {
            session = Some(value.to_string());
        }
        if let Some(value) = pair.strip_prefix("factory_csrf=") {
            csrf = Some(value.to_string());
        }
    }
    (session.expect("session cookie"), csrf.expect("csrf cookie"))
}

pub async fn response_json(response: Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}
