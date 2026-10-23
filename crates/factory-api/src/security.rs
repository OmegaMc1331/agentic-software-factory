use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::header;
use axum::http::uri::{Authority, Uri};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::app::SharedState;

pub(crate) const SESSION_COOKIE: &str = "factory_session";
pub(crate) const CSRF_COOKIE: &str = "factory_csrf";
const CSRF_HEADER: &str = "x-factory-csrf";
const SESSION_SECONDS: u64 = 12 * 60 * 60;
const CLOCK_SKEW_SECONDS: u64 = 60;
const HMAC_BLOCK_BYTES: usize = 64;
const HMAC_DIGEST_BYTES: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct ApiOptions {
    dev_origins: Vec<AllowedOrigin>,
    session_ttl: Option<Duration>,
}

impl ApiOptions {
    pub fn with_dev_origin(mut self, origin: &str) -> Result<Self, String> {
        let parsed = AllowedOrigin::parse(origin).ok_or_else(|| {
            "development origin must be an exact loopback http(s) origin".to_string()
        })?;
        if !self.dev_origins.contains(&parsed) {
            self.dev_origins.push(parsed);
        }
        Ok(self)
    }

    pub fn with_session_ttl(mut self, ttl: Duration) -> Self {
        self.session_ttl = Some(ttl);
        self
    }

    fn ttl(&self) -> Duration {
        self.session_ttl
            .unwrap_or_else(|| Duration::from_secs(SESSION_SECONDS))
    }
}

#[derive(Clone)]
pub(crate) struct SecurityPolicy {
    secret: Arc<[u8; HMAC_DIGEST_BYTES]>,
    dev_origins: Arc<Vec<AllowedOrigin>>,
    ttl: Duration,
}

struct SessionIssue {
    session: String,
    csrf: String,
}

impl SecurityPolicy {
    pub(crate) fn new(options: ApiOptions) -> Self {
        let mut secret = [0u8; HMAC_DIGEST_BYTES];
        OsRng.fill_bytes(&mut secret);
        let ttl = options.ttl();
        Self {
            secret: Arc::new(secret),
            dev_origins: Arc::new(options.dev_origins),
            ttl,
        }
    }

    fn issue_session_with_csrf(&self, existing_csrf: Option<String>) -> SessionIssue {
        let issued = unix_seconds();
        let mut nonce = [0u8; 16];
        OsRng.fill_bytes(&mut nonce);
        let payload = format!("v1.{issued}.{}", hex(&nonce));
        let signature = self.sign(&payload);
        let csrf = existing_csrf.unwrap_or_else(|| {
            let mut csrf = [0u8; HMAC_DIGEST_BYTES];
            OsRng.fill_bytes(&mut csrf);
            hex(&csrf)
        });
        SessionIssue {
            session: format!("{payload}.{signature}"),
            csrf,
        }
    }

    fn validate_session(&self, token: &str) -> bool {
        let mut parts = token.split('.');
        let (Some("v1"), Some(issued), Some(nonce), Some(signature), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return false;
        };
        let Ok(issued) = issued.parse::<u64>() else {
            return false;
        };
        if nonce.len() != 32 || signature.len() != HMAC_DIGEST_BYTES * 2 {
            return false;
        }
        let now = unix_seconds();
        if issued > now.saturating_add(CLOCK_SKEW_SECONDS) {
            return false;
        }
        let expires = issued.saturating_add(self.ttl.as_secs());
        if now >= expires {
            return false;
        }
        let payload = format!("v1.{issued}.{nonce}");
        constant_time_eq(signature.as_bytes(), self.sign(&payload).as_bytes())
    }

    fn sign(&self, message: &str) -> String {
        hmac_sha256(self.secret.as_slice(), message.as_bytes())
    }

    fn origin_allowed(&self, origin: &AllowedOrigin, host: &LoopbackAuthority) -> bool {
        origin.is_same_origin(host) || self.dev_origins.iter().any(|allowed| allowed == origin)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AllowedOrigin {
    scheme: String,
    host: String,
    port: u16,
}

impl AllowedOrigin {
    fn parse(value: &str) -> Option<Self> {
        let uri = Uri::from_str(value).ok()?;
        let scheme = uri.scheme_str()?.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return None;
        }
        if uri.query().is_some() || value.contains('#') {
            return None;
        }
        if !matches!(uri.path(), "" | "/") {
            return None;
        }
        let authority = parse_loopback_authority(uri.authority()?.as_str())?;
        Some(Self {
            scheme,
            host: authority.host,
            port: authority.port,
        })
    }

    fn is_same_origin(&self, host: &LoopbackAuthority) -> bool {
        self.scheme == "http" && self.host == host.host && self.port == host.port
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LoopbackAuthority {
    host: String,
    port: u16,
}

fn parse_loopback_authority(value: &str) -> Option<LoopbackAuthority> {
    if value.contains('@')
        || value.contains('/')
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
    {
        return None;
    }
    let authority = Authority::from_str(value).ok()?;
    let authority_host = authority.host();
    let host = authority_host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return None;
    }
    let port = match &authority.as_str()[authority_host.len()..] {
        "" => 80,
        suffix => suffix.strip_prefix(':')?.parse::<u16>().ok()?,
    };
    if port == 0 {
        return None;
    }
    Some(LoopbackAuthority { host, port })
}

fn single_header(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value.to_str().ok()
}

fn request_host(headers: &HeaderMap) -> Option<LoopbackAuthority> {
    parse_loopback_authority(single_header(headers, header::HOST)?)
}

fn request_origin(headers: &HeaderMap) -> Result<Option<AllowedOrigin>, ()> {
    let mut values = headers.get_all(header::ORIGIN).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    let value = value.to_str().map_err(|_| ())?;
    AllowedOrigin::parse(value).map(Some).ok_or(())
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    for header in headers.get_all(header::COOKIE).iter() {
        let Ok(value) = header.to_str() else {
            continue;
        };
        for cookie in value.split(';') {
            let cookie = cookie.trim();
            if let Some(value) = cookie.strip_prefix(name).and_then(|v| v.strip_prefix('=')) {
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

fn denial(status: StatusCode, message: &str) -> Response {
    let mut response = (status, Json(json!({ "error": message }))).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(crate) async fn enforce_security(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    let headers = request.headers();

    let Some(host) = request_host(headers) else {
        return denial(StatusCode::BAD_REQUEST, "unapproved Host header");
    };
    let origin = match request_origin(headers) {
        Ok(origin) => origin,
        Err(()) => return denial(StatusCode::FORBIDDEN, "unapproved Origin header"),
    };
    if let Some(origin) = &origin {
        if !state.security.origin_allowed(origin, &host) {
            return denial(StatusCode::FORBIDDEN, "unapproved Origin header");
        }
    }

    if path == "/api/auth/session" {
        if method == Method::POST {
            if origin.is_none() {
                return denial(
                    StatusCode::FORBIDDEN,
                    "browser authentication requires a same-origin request",
                );
            }
            let content_type = single_header(headers, header::CONTENT_TYPE)
                .and_then(|value| value.split(';').next())
                .map(str::trim);
            if !content_type.is_some_and(|value| value.eq_ignore_ascii_case("application/json")) {
                return denial(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "expected application/json",
                );
            }
        }
    } else if path != "/api/health" && is_api_path(&path) {
        let authenticated = cookie_value(headers, SESSION_COOKIE)
            .is_some_and(|token| state.security.validate_session(token));
        if !authenticated {
            return denial(StatusCode::UNAUTHORIZED, "authentication required");
        }
        if matches!(
            method,
            Method::POST | Method::PUT | Method::PATCH | Method::DELETE
        ) {
            let cookie = cookie_value(headers, CSRF_COOKIE)
                .ok_or_else(|| denial(StatusCode::FORBIDDEN, "CSRF validation failed"));
            let cookie = match cookie {
                Ok(cookie) => cookie,
                Err(response) => return response,
            };
            let header = single_header(headers, header::HeaderName::from_static(CSRF_HEADER))
                .ok_or_else(|| denial(StatusCode::FORBIDDEN, "CSRF validation failed"));
            let header = match header {
                Ok(header) => header,
                Err(response) => return response,
            };
            if !constant_time_eq(cookie.as_bytes(), header.as_bytes()) {
                return denial(StatusCode::FORBIDDEN, "CSRF validation failed");
            }
        }
    }

    let mut response = next.run(request).await;
    if is_api_path(&path) {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response.headers_mut().insert(
            header::HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
    }
    response
}

pub(crate) async fn issue_browser_session(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Response {
    let existing_csrf = cookie_value(&headers, CSRF_COOKIE)
        .filter(|value| {
            value.len() == HMAC_DIGEST_BYTES * 2
                && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .map(str::to_owned);
    let session = state.security.issue_session_with_csrf(existing_csrf);
    let ttl = state.security.ttl.as_secs();
    let session_cookie = format!(
        "{SESSION_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/api; Max-Age={ttl}",
        session.session
    );
    let csrf_cookie = format!(
        "{CSRF_COOKIE}={}; SameSite=Strict; Path=/; Max-Age={ttl}",
        session.csrf
    );
    let mut response = Json(json!({ "csrfToken": session.csrf })).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&session_cookie) {
        headers.append(header::SET_COOKIE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&csrf_cookie) {
        headers.append(header::SET_COOKIE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> String {
    let mut normalized_key = [0u8; HMAC_BLOCK_BYTES];
    if key.len() > HMAC_BLOCK_BYTES {
        normalized_key[..HMAC_DIGEST_BYTES].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; HMAC_BLOCK_BYTES];
    let mut outer_pad = [0x5cu8; HMAC_BLOCK_BYTES];
    for (index, byte) in normalized_key.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let inner = Sha256::new()
        .chain_update(inner_pad)
        .chain_update(message)
        .finalize();
    let outer = Sha256::new()
        .chain_update(outer_pad)
        .chain_update(inner)
        .finalize();
    hex(&outer)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_exact_and_rebinding_hosts_are_rejected() {
        for valid in ["127.0.0.1:4321", "localhost:4321", "[::1]:4321"] {
            assert!(parse_loopback_authority(valid).is_some(), "{valid}");
        }
        for invalid in [
            "evil.example:4321",
            "127.0.0.1.evil.example:4321",
            "localhost.evil.example:4321",
            "0.0.0.0:4321",
            "user@127.0.0.1:4321",
            "localhost:4321.evil",
            "localhost:not-a-port",
            "localhost:",
            "localhost:0",
        ] {
            assert!(parse_loopback_authority(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231() {
        assert_eq!(
            hmac_sha256(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn sessions_are_random_and_expire() {
        let policy =
            SecurityPolicy::new(ApiOptions::default().with_session_ttl(Duration::from_secs(60)));
        let first = policy.issue_session_with_csrf(None);
        let second = policy.issue_session_with_csrf(None);
        assert_ne!(first.session, second.session);
        assert_ne!(first.csrf, second.csrf);
        assert!(policy.validate_session(&first.session));

        let expired_policy =
            SecurityPolicy::new(ApiOptions::default().with_session_ttl(Duration::ZERO));
        let expired = expired_policy.issue_session_with_csrf(None);
        assert!(!expired_policy.validate_session(&expired.session));
        assert!(!policy.validate_session("v1.invalid"));
    }

    #[test]
    fn development_origins_must_be_exact_loopback_origins() {
        assert!(ApiOptions::default()
            .with_dev_origin("http://localhost:5173")
            .is_ok());
        for invalid in [
            "http://evil.example:5173",
            "http://localhost:5173/app",
            "*",
            "http://localhost:5173/path",
        ] {
            assert!(ApiOptions::default().with_dev_origin(invalid).is_err());
        }
    }
}
