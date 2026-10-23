use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use axum::http::{header, HeaderValue};
use factory_types::{AgentSession, AgentSessionMode};
use futures_util::{SinkExt, StreamExt};
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);

struct TestServer {
    address: SocketAddr,
    task: JoinHandle<std::io::Result<()>>,
}

impl TestServer {
    fn start(state: Arc<factory_api::ApiState>) -> Self {
        let listener = factory_api::bind(0).unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(factory_api::serve(state, listener));
        Self { address, task }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
struct RawResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl RawResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn set_cookies(&self) -> impl Iterator<Item = &str> {
        self.headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, value)| value.as_str())
    }
}

struct Cookies {
    header: String,
    csrf: String,
}

async fn raw_request(address: SocketAddr, request: String) -> RawResponse {
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10)).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("raw response failed: {error}"),
            }
        }
        parse_response(bytes)
    })
    .await
    .unwrap()
}

fn parse_response(bytes: Vec<u8>) -> RawResponse {
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    RawResponse {
        status,
        headers,
        body: body.to_string(),
    }
}

fn get_request(address: SocketAddr, path: &str, extra_headers: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\n{extra_headers}Connection: close\r\n\r\n"
    )
}

fn post_request(
    address: SocketAddr,
    path: &str,
    origin: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    body: &str,
) -> String {
    let cookie = cookie.map_or_else(String::new, |value| format!("Cookie: {value}\r\n"));
    let csrf = csrf.map_or_else(String::new, |value| format!("X-Factory-CSRF: {value}\r\n"));
    format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{cookie}{csrf}Connection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn bootstrap(address: SocketAddr, origin: &str) -> (RawResponse, Cookies) {
    let response = raw_request(
        address,
        post_request(address, "/api/auth/session", origin, None, None, "{}"),
    )
    .await;
    assert_eq!(response.status, 200, "{response:?}");
    let mut session = None;
    let mut csrf = None;
    for cookie in response.set_cookies() {
        let pair = cookie.split(';').next().unwrap();
        if let Some(value) = pair.strip_prefix("factory_session=") {
            session = Some(value.to_string());
        }
        if let Some(value) = pair.strip_prefix("factory_csrf=") {
            csrf = Some(value.to_string());
        }
    }
    let session = session.expect("session cookie");
    let csrf = csrf.expect("csrf cookie");
    (
        response,
        Cookies {
            header: format!("factory_session={session}; factory_csrf={csrf}"),
            csrf,
        },
    )
}

fn init_root(root: &std::path::Path) {
    factory_core::Factory::init(root).unwrap();
}

fn make_state(root: &std::path::Path) -> Arc<factory_api::ApiState> {
    Arc::new(factory_api::ApiState::new(root.to_path_buf()).unwrap())
}

#[test]
fn pty_security_child() {
    if std::env::var("FACTORY_SECURITY_PTY_CHILD").as_deref() == Ok("1") {
        std::thread::sleep(Duration::from_secs(20));
    }
}

#[test]
fn pty_terminal_child() {
    if std::env::var("FACTORY_API_TERMINAL_CHILD").as_deref() != Ok("1") {
        return;
    }
    let mut stdout = std::io::stdout();
    stdout.write_all(b"API_TERMINAL_READY\r\n").unwrap();
    stdout.flush().unwrap();
    let mut input = [0_u8; 1];
    loop {
        if std::io::Read::read(&mut std::io::stdin(), &mut input).is_err() {
            std::process::exit(1);
        }
        if input[0] == b'i' {
            stdout.write_all(b"API_TERMINAL_INPUT\r\n").unwrap();
            stdout.flush().unwrap();
            break;
        }
    }
    std::thread::sleep(Duration::from_secs(2));
}

fn configure_console_agent(root: &std::path::Path) {
    let command = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "\\\\");
    std::fs::write(
        root.join(".factory").join("config.toml"),
        format!(
            r#"
[agents.console-test]
kind = "custom"
command = "{command}"
prompt_transport = "disabled"
interactive_args = ["--exact", "pty_security_child", "--nocapture"]

[agents.console-test.env]
FACTORY_SECURITY_PTY_CHILD = "1"
"#
        ),
    )
    .unwrap();
}

fn configure_terminal_stream_agent(root: &std::path::Path) {
    let command = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "\\\\");
    std::fs::write(
        root.join(".factory").join("config.toml"),
        format!(
            r#"
[agents.terminal-stream-test]
kind = "custom"
command = "{command}"
prompt_transport = "disabled"
interactive_args = ["--exact", "pty_terminal_child", "--nocapture"]

[agents.terminal-stream-test.env]
FACTORY_API_TERMINAL_CHILD = "1"
"#
        ),
    )
    .unwrap();
}

fn configure_runnable_agent(root: &std::path::Path) {
    let (command, args) = if cfg!(windows) {
        ("cmd", r#"["/d", "/c", "exit", "0"]"#)
    } else {
        ("sh", r#"["-c", "exit 0"]"#)
    };
    std::fs::write(
        root.join(".factory").join("config.toml"),
        format!(
            r#"
[agents.security-test]
kind = "custom"
command = "{command}"
args = {args}

[[role_assignments]]
role = "planner"
agent = "security-test"
preferred = true

[[role_assignments]]
role = "worker"
agent = "security-test"
preferred = true

[[role_assignments]]
role = "reviewer"
agent = "security-test"
preferred = true
"#
        ),
    )
    .unwrap();
}

type TerminalSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn open_terminal(
    address: SocketAddr,
    session_id: i64,
    cookie: Option<&str>,
    origin: &str,
) -> Result<(TerminalSocket, u16), Box<WsError>> {
    let mut request = format!("ws://{address}/api/sessions/{session_id}/terminal")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        header::HOST,
        HeaderValue::from_str(&address.to_string()).unwrap(),
    );
    request
        .headers_mut()
        .insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
    if let Some(cookie) = cookie {
        request
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
    }
    let (socket, response) = connect_async(request).await.map_err(Box::new)?;
    Ok((socket, response.status().as_u16()))
}

async fn connect_terminal(
    address: SocketAddr,
    session_id: i64,
    cookie: Option<&str>,
    origin: &str,
) -> Result<u16, Box<WsError>> {
    let (socket, status) = open_terminal(address, session_id, cookie, origin).await?;
    drop(socket);
    Ok(status)
}

async fn terminal_output_until(socket: &mut TerminalSocket, marker: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut answered_cursor_query = false;
    while !output.windows(marker.len()).any(|window| window == marker) {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "terminal WebSocket timed out after {} bytes: {:?}",
                    output.len(),
                    String::from_utf8_lossy(&output)
                )
            })
            .expect("terminal WebSocket closed")
            .expect("terminal WebSocket failed");
        match message {
            Message::Binary(data) => output.extend_from_slice(&data),
            Message::Close(frame) => panic!("terminal WebSocket closed early: {frame:?}"),
            Message::Text(text) => panic!("unexpected terminal text: {text}"),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
        if !answered_cursor_query && output.windows(4).any(|window| window == b"\x1b[6n") {
            socket
                .send(Message::Text(
                    r#"{"type":"input","data":"\u001b[1;1R"}"#.into(),
                ))
                .await
                .unwrap();
            answered_cursor_query = true;
        }
    }
    output
}

fn ws_status(result: Result<u16, Box<WsError>>) -> u16 {
    match result {
        Ok(status) => status,
        Err(error) => match *error {
            WsError::Http(response) => response.status().as_u16(),
            error => panic!("unexpected websocket error: {error}"),
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_loopback_requests_succeed_but_unauthenticated_requests_do_not() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let server = TestServer::start(make_state(dir.path()));

    let health = raw_request(
        server.address,
        format!(
            "GET /api/health HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            server.address
        ),
    )
    .await;
    assert_eq!(health.status, 200);

    let unauthorized =
        raw_request(server.address, get_request(server.address, "/api/runs", "")).await;
    assert_eq!(unauthorized.status, 401);
    assert_eq!(unauthorized.header("cache-control"), Some("no-store"));
    assert_eq!(
        unauthorized.header("x-content-type-options"),
        Some("nosniff")
    );

    let invalid = raw_request(
        server.address,
        get_request(
            server.address,
            "/api/runs",
            "Cookie: factory_session=not-a-session; factory_csrf=also-invalid\r\n",
        ),
    )
    .await;
    assert_eq!(invalid.status, 401);

    let (bootstrap, cookies) =
        bootstrap(server.address, &format!("http://{}", server.address)).await;
    assert!(bootstrap
        .set_cookies()
        .any(|cookie| cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict")));
    assert!(bootstrap
        .set_cookies()
        .all(|cookie| !cookie.contains("Domain=")));

    let authorized = raw_request(
        server.address,
        get_request(
            server.address,
            "/api/runs",
            &format!("Cookie: {}\r\n", cookies.header),
        ),
    )
    .await;
    assert_eq!(authorized.status, 200);
    assert_eq!(authorized.body.trim(), "[]");
    assert_eq!(authorized.header("cache-control"), Some("no-store"));

    let without_origin = raw_request(
        server.address,
        format!(
            "GET /api/runs HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\nConnection: close\r\n\r\n",
            server.address, cookies.header
        ),
    )
    .await;
    assert_eq!(without_origin.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_origins_csrf_and_dns_rebinding_hosts_are_rejected() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let server = TestServer::start(make_state(dir.path()));
    let (_, cookies) = bootstrap(server.address, &format!("http://{}", server.address)).await;

    let cross_origin = raw_request(
        server.address,
        format!(
            "GET /api/runs HTTP/1.1\r\nHost: {}\r\nOrigin: http://evil.example\r\nCookie: {}\r\nConnection: close\r\n\r\n",
            server.address,
            cookies.header
        ),
    )
    .await;
    assert_eq!(cross_origin.status, 403);

    let malicious_bootstrap = raw_request(
        server.address,
        post_request(
            server.address,
            "/api/auth/session",
            "http://evil.example",
            None,
            None,
            "{}",
        ),
    )
    .await;
    assert_eq!(malicious_bootstrap.status, 403);

    let no_origin_bootstrap = raw_request(
        server.address,
        format!(
            "POST /api/auth/session HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            server.address
        ),
    )
    .await;
    assert_eq!(no_origin_bootstrap.status, 403);

    let form_bootstrap = raw_request(
        server.address,
        format!(
            "POST /api/auth/session HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Type: text/plain\r\nContent-Length: 3\r\nConnection: close\r\n\r\na=b",
            server.address, server.address
        ),
    )
    .await;
    assert_eq!(form_bootstrap.status, 415);

    let cross_origin_mutation = raw_request(
        server.address,
        post_request(
            server.address,
            "/api/runs",
            "http://evil.example",
            Some(&cookies.header),
            Some(&cookies.csrf),
            r#"{"objective":"must not run"}"#,
        ),
    )
    .await;
    assert_eq!(cross_origin_mutation.status, 403);

    let missing_csrf = raw_request(
        server.address,
        post_request(
            server.address,
            "/api/runs",
            &format!("http://{}", server.address),
            Some(&cookies.header),
            None,
            r#"{"objective":"must not run"}"#,
        ),
    )
    .await;
    assert_eq!(missing_csrf.status, 403);

    let after = raw_request(
        server.address,
        get_request(
            server.address,
            "/api/runs",
            &format!("Cookie: {}\r\n", cookies.header),
        ),
    )
    .await;
    assert_eq!(after.status, 200);
    assert_eq!(after.body.trim(), "[]");

    let rebound = raw_request(
        server.address,
        format!(
            "GET /api/runs HTTP/1.1\r\nHost: evil.example:{}\r\nOrigin: http://evil.example:{}\r\nCookie: {}\r\nConnection: close\r\n\r\n",
            server.address.port(),
            server.address.port(),
            cookies.header
        ),
    )
    .await;
    assert_eq!(rebound.status, 400);

    let rebound_without_origin = raw_request(
        server.address,
        format!(
            "GET /api/runs HTTP/1.1\r\nHost: evil.example:{}\r\nCookie: {}\r\nConnection: close\r\n\r\n",
            server.address.port(),
            cookies.header
        ),
    )
    .await;
    assert_eq!(rebound_without_origin.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_websocket_handshakes_succeed_and_unauthorized_ones_fail_before_upgrade() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_console_agent(dir.path());
    let state = make_state(dir.path());
    let server = TestServer::start(state.clone());
    let (_, cookies) = bootstrap(server.address, &format!("http://{}", server.address)).await;
    let session = state
        .runtime
        .start_interactive_session("console-test", 90, 24)
        .unwrap();
    let origin = format!("http://{}", server.address);

    assert_eq!(
        ws_status(
            connect_terminal(server.address, session.id, Some(&cookies.header), &origin).await
        ),
        101
    );

    assert_eq!(
        ws_status(connect_terminal(server.address, session.id, None, &origin).await),
        401
    );

    assert_eq!(
        ws_status(
            connect_terminal(
                server.address,
                session.id,
                Some(&cookies.header),
                "http://evil.example"
            )
            .await
        ),
        403
    );

    let _ = state.runtime.stop_interactive_session(session.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_terminal_websocket_streams_output_and_accepts_input_and_resize() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_terminal_stream_agent(dir.path());
    let state = make_state(dir.path());
    let server = TestServer::start(state.clone());
    let origin = format!("http://{}", server.address);
    let (_, cookies) = bootstrap(server.address, &origin).await;
    let session = state
        .runtime
        .start_interactive_session("terminal-stream-test", 90, 24)
        .unwrap();
    let (mut socket, status) =
        open_terminal(server.address, session.id, Some(&cookies.header), &origin)
            .await
            .unwrap();
    assert_eq!(status, 101);

    let initial = terminal_output_until(&mut socket, b"API_TERMINAL_READY").await;
    assert!(String::from_utf8_lossy(&initial).contains("API_TERMINAL_READY"));

    socket
        .send(Message::Text(
            r#"{"type":"resize","cols":120,"rows":40}"#.into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(r#"{"type":"input","data":"i\r"}"#.into()))
        .await
        .unwrap();
    let echoed = terminal_output_until(&mut socket, b"API_TERMINAL_INPUT").await;
    assert!(String::from_utf8_lossy(&echoed).contains("API_TERMINAL_INPUT"));

    socket.close(None).await.unwrap();
    let _ = state.runtime.stop_interactive_session(session.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse_streaming_requires_and_accepts_a_session() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let state = make_state(dir.path());
    let session = {
        let db = state.db.lock().unwrap();
        db.insert_agent_session(&AgentSession {
            id: 0,
            run_id: None,
            task_id: None,
            attempt_id: None,
            role: "worker".to_string(),
            operation: None,
            agent: "codex".to_string(),
            mode: AgentSessionMode::Automated,
            command: "codex exec".to_string(),
            status: "success".to_string(),
            started_at: "2026-08-13T08:00:00Z".to_string(),
            finished_at: Some("2026-08-13T08:00:02Z".to_string()),
            exit_code: Some(0),
            duration_ms: Some(2_000),
            stdout: Some("compiled\n".to_string()),
            stderr: Some("warning\n".to_string()),
            policy_audit: None,
            output_meta: None,
        })
        .unwrap()
    };
    let server = TestServer::start(state);
    let origin = format!("http://{}", server.address);
    let (_, cookies) = bootstrap(server.address, &origin).await;

    let unauthorized = raw_request(
        server.address,
        get_request(
            server.address,
            &format!("/api/sessions/{}/stream", session.id),
            "",
        ),
    )
    .await;
    assert_eq!(unauthorized.status, 401);

    let authorized = raw_request(
        server.address,
        get_request(
            server.address,
            &format!("/api/sessions/{}/stream", session.id),
            &format!("Cookie: {}\r\n", cookies.header),
        ),
    )
    .await;
    assert_eq!(authorized.status, 200);
    assert!(authorized
        .header("content-type")
        .is_some_and(|value| value.starts_with("text/event-stream")));
    assert!(authorized.body.contains("event: session"));
    assert!(authorized.body.contains("compiled"));
}

#[tokio::test]
async fn session_refresh_preserves_the_csrf_token_for_other_tabs() {
    let root = TempDir::new().unwrap();
    init_root(root.path());
    let server = TestServer::start(make_state(root.path()));
    let origin = format!("http://{}", server.address);
    let (_, first) = bootstrap(server.address, &origin).await;

    let response = raw_request(
        server.address,
        post_request(
            server.address,
            "/api/auth/session",
            &origin,
            Some(&first.header),
            None,
            "{}",
        ),
    )
    .await;
    assert_eq!(response.status, 200, "{response:?}");
    let mut csrf = None;
    for cookie in response.set_cookies() {
        if let Some(value) = cookie
            .split(';')
            .next()
            .and_then(|pair| pair.strip_prefix("factory_csrf="))
        {
            csrf = Some(value.to_string());
        }
    }
    assert_eq!(csrf.as_deref(), Some(first.csrf.as_str()));
}

#[tokio::test]
async fn encoded_api_paths_cannot_bypass_authentication() {
    let root = TempDir::new().unwrap();
    init_root(root.path());
    let server = TestServer::start(make_state(root.path()));

    for path in [
        "/api%2Fruns",
        "/api%2Fauth%2Fsession",
        "/%61pi/runs",
        "/API/runs",
    ] {
        let response = raw_request(server.address, get_request(server.address, path, "")).await;
        let dashboard_fallback = response
            .header("content-type")
            .is_some_and(|value| value.starts_with("text/html"));
        assert!(
            response.status != 200 || dashboard_fallback,
            "{path} was not protected: {response:?}"
        );
        assert!(
            !response
                .set_cookies()
                .any(|cookie| cookie.starts_with("factory_session=")),
            "{path} issued a session: {response:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_and_disk_dashboard_remains_public_but_api_stays_protected() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let server = TestServer::start(make_state(dir.path()));

    let dashboard = raw_request(
        server.address,
        format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            server.address
        ),
    )
    .await;
    assert_eq!(dashboard.status, 200);
    assert!(!dashboard.body.contains("factory_session"));

    let api = raw_request(
        server.address,
        format!(
            "GET /api/not-a-route HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            server.address
        ),
    )
    .await;
    assert_eq!(api.status, 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vite_development_origin_requires_explicit_exact_configuration() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let options = factory_api::ApiOptions::default()
        .with_dev_origin("http://localhost:5173")
        .unwrap();
    let state = Arc::new(
        factory_api::ApiState::new_with_options(dir.path().to_path_buf(), options).unwrap(),
    );
    let server = TestServer::start(state);

    let (_, cookies) = bootstrap(server.address, "http://localhost:5173").await;
    let proxied = raw_request(
        server.address,
        format!(
            "GET /api/runs HTTP/1.1\r\nHost: {}\r\nOrigin: http://localhost:5173\r\nCookie: {}\r\nConnection: close\r\n\r\n",
            server.address,
            cookies.header
        ),
    )
    .await;
    assert_eq!(proxied.status, 200);

    let other_port = raw_request(
        server.address,
        format!(
            "GET /api/runs HTTP/1.1\r\nHost: {}\r\nOrigin: http://localhost:5174\r\nCookie: {}\r\nConnection: close\r\n\r\n",
            server.address,
            cookies.header
        ),
    )
    .await;
    assert_eq!(other_port.status, 403);

    let no_development_config = TempDir::new().unwrap();
    init_root(no_development_config.path());
    let strict_server = TestServer::start(make_state(no_development_config.path()));
    let rejected = raw_request(
        strict_server.address,
        post_request(
            strict_server.address,
            "/api/auth/session",
            "http://localhost:5173",
            None,
            None,
            "{}",
        ),
    )
    .await;
    assert_eq!(rejected.status, 403);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_sessions_are_rejected() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    let options = factory_api::ApiOptions::default().with_session_ttl(Duration::ZERO);
    let state = Arc::new(
        factory_api::ApiState::new_with_options(dir.path().to_path_buf(), options).unwrap(),
    );
    let server = TestServer::start(state);
    let origin = format!("http://{}", server.address);
    let (_, cookies) = bootstrap(server.address, &origin).await;

    let response = raw_request(
        server.address,
        get_request(
            server.address,
            "/api/runs",
            &format!("Cookie: {}\r\n", cookies.header),
        ),
    )
    .await;
    assert_eq!(response.status, 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_mutation_with_csrf_and_missing_origin_still_works() {
    let dir = TempDir::new().unwrap();
    init_root(dir.path());
    configure_runnable_agent(dir.path());
    let server = TestServer::start(make_state(dir.path()));
    let origin = format!("http://{}", server.address);
    let (_, cookies) = bootstrap(server.address, &origin).await;

    let response = raw_request(
        server.address,
        format!(
            "POST /api/runs HTTP/1.1\r\nHost: {}\r\nOrigin: {origin}\r\nCookie: {}\r\nX-Factory-CSRF: {}\r\nContent-Type: application/json\r\nContent-Length: 26\r\nConnection: close\r\n\r\n{{\"objective\":\"secure run\"}}",
            server.address,
            cookies.header,
            cookies.csrf
        ),
    )
    .await;
    assert_eq!(response.status, 202, "{response:?}");

    let response = raw_request(
        server.address,
        format!(
            "POST /api/runs HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\nX-Factory-CSRF: {}\r\nContent-Type: application/json\r\nContent-Length: 26\r\nConnection: close\r\n\r\n{{\"objective\":\"secure run\"}}",
            server.address,
            cookies.header,
            cookies.csrf
        ),
    )
    .await;
    assert_eq!(response.status, 202, "{response:?}");
}

#[test]
fn session_ttl_constant_matches_documented_default() {
    assert_eq!(SESSION_TTL, Duration::from_secs(12 * 60 * 60));
}
