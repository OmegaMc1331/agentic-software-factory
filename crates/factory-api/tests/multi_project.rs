use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use factory_api::{application_router, ApiOptions, ApplicationOptions, FactoryApplication};
use factory_core::{Config, RoleAssignment};
use factory_projects::{ProjectId, ProjectLimits};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    app: Arc<FactoryApplication>,
    a: PathBuf,
    b: PathBuf,
    aid: ProjectId,
    bid: ProjectId,
    _dir: TempDir,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_stops_worker_before_slow_request_drains_and_defers_checkpoint() {
    use tokio::io::AsyncWriteExt;
    let fixture = Fixture::new();
    fixture.configure(true);
    let client = fixture.client().await;
    assert_eq!(
        client
            .request(
                "POST",
                &fixture.route(true, "runs"),
                json!({"objective":"shutdown during accepted request"})
            )
            .await
            .0,
        StatusCode::ACCEPTED
    );
    wait_for(&fixture, true, 1, "planned").await;
    let context = fixture.app.projects.context(&fixture.aid).unwrap();
    let session = "s_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    {
        let db = context.db.lock().unwrap();
        db.register_workspace_session(fixture.aid.as_str(), session)
            .unwrap();
        db.bind_workspace_run(1, fixture.aid.as_str(), session)
            .unwrap();
    }
    assert_eq!(
        client
            .request("POST", &fixture.route(true, "runs/1/start"), json!({}))
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !fixture
        .a
        .join(".factory/worktrees/t1/project-proof.txt")
        .exists()
    {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let listener = factory_api::bind(0).unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(factory_api::serve_application_with_shutdown(
        fixture.app.clone(),
        listener,
        async {
            stopped.await.unwrap();
        },
    ));
    let mut request = tokio::net::TcpStream::connect(address).await.unwrap();
    request.write_all(format!("POST /api/projects/register HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nCookie: {}\r\nX-Factory-CSRF: {}\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{{", client.cookie, client.csrf).as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    stop.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !context.runtime.is_idle() {
        assert!(
            Instant::now() < deadline,
            "a stalled request delayed Worker cancellation"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    {
        let db = context.db.lock().unwrap();
        let life = db.mission_lifecycle(1).unwrap().unwrap();
        assert_eq!(life.state, factory_types::MissionLifecycleState::Quiescing);
        assert!(life.checkpoint_id.is_none());
        assert!(db
            .lifecycle_processes(1)
            .unwrap()
            .iter()
            .all(|(_, _, meta)| meta
                .as_ref()
                .is_none_or(|meta| meta.process_tree_cleanup_confirmed)));
    }
    drop(request);
    tokio::time::timeout(Duration::from_secs(15), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let life = context
        .db
        .lock()
        .unwrap()
        .mission_lifecycle(1)
        .unwrap()
        .unwrap();
    assert_eq!(life.state, factory_types::MissionLifecycleState::Suspended);
    assert!(life.checkpoint_id.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn application_shutdown_closes_http_and_project_admission_without_mutation() {
    let fixture = Fixture::new();
    let client = fixture.client().await;
    let listener = factory_api::bind(0).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(factory_api::serve_application_with_shutdown(
        fixture.app.clone(),
        listener,
        async {
            stopped.await.unwrap();
        },
    ));
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let response = application_router(fixture.app.clone())
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .header("host", "127.0.0.1:4321")
                .header("cookie", &client.cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(fixture.app.projects.budget.operation().is_err());
    assert_eq!(fixture.app.projects.list().unwrap().len(), 2);
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("project A λ");
        let b = dir.path().join("project B");
        for root in [&a, &b] {
            std::fs::create_dir(root).unwrap();
            factory_git_init(root);
            factory_core::Factory::init(root).unwrap();
        }
        let app = Arc::new(
            FactoryApplication::new(ApplicationOptions {
                registry_path: dir.path().join("application/registry.sqlite3"),
                allowed_roots: vec![dir.path().to_owned()],
                default_root: Some(a.clone()),
                dashboard_root: a.clone(),
                limits: ProjectLimits::default(),
                api: ApiOptions::default(),
            })
            .unwrap(),
        );
        let aid = app.default_project.clone().unwrap();
        let bid = app.projects.register(&b, None).unwrap().project_id;
        app.projects.open(&bid).unwrap();
        Self {
            _dir: dir,
            app,
            a,
            b,
            aid,
            bid,
        }
    }

    async fn client(&self) -> Client {
        Client::new(application_router(self.app.clone())).await
    }
    fn route(&self, a: bool, suffix: &str) -> String {
        format!(
            "/api/projects/{}/{}",
            if a { &self.aid } else { &self.bid },
            suffix
        )
    }

    fn configure(&self, slow: bool) {
        let program = self._dir.path().join(if cfg!(windows) {
            "test-agent.exe"
        } else {
            "test-agent"
        });
        let output = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/project_agent.rs"))
            .arg("-o")
            .arg(&program)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for (root, label) in [(&self.a, "A"), (&self.b, "B")] {
            let mut config = Config::default();
            for (role, name, mode) in [
                ("planner", "planner", "planner"),
                ("worker", "codex", "worker"),
                ("reviewer", "reviewer", "reviewer"),
            ] {
                let mut env = BTreeMap::new();
                env.insert("ASF_PROJECT_PROOF", label.to_string());
                if slow && role == "worker" {
                    env.insert(
                        "ASF_WORKER_GATE",
                        self._dir
                            .path()
                            .join(format!("go-{label}"))
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
                let entry = serde_json::from_value(json!({"kind":"custom","command":program,"args":[mode],"interactive_args":["terminal"],"env":env})).unwrap();
                config.agents.insert(name.into(), entry);
                config.role_assignments.push(RoleAssignment {
                    role: role.into(),
                    agent: name.into(),
                    preferred: true,
                });
            }
            config.write_atomic(root).unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for label in ["A", "B"] {
            let _ = std::fs::write(self._dir.path().join(format!("go-{label}")), "cleanup");
        }
        for id in [&self.aid, &self.bid] {
            if let Ok(context) = self.app.projects.context(id) {
                for operation in context.runtime.active_operations() {
                    let _ = context.runtime.cancel_workflow(operation.run_id);
                }
                let sessions = context
                    .db
                    .lock()
                    .unwrap()
                    .list_agent_sessions(None)
                    .unwrap();
                for session in sessions {
                    if session.mode == factory_types::AgentSessionMode::Interactive {
                        let _ = context.runtime.stop_interactive_session(session.id);
                    }
                }
            }
        }
    }
}

fn factory_git_init(root: &Path) {
    std::fs::write(
        root.join("same.txt"),
        root.file_name().unwrap().to_string_lossy().as_bytes(),
    )
    .unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.name", "P03 controlled fixture"],
        vec!["config", "user.email", "p03@example.test"],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "fixture"],
    ] {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

struct Client {
    router: Router,
    cookie: String,
    csrf: String,
}
impl Client {
    async fn new(router: Router) -> Self {
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
            router,
            cookie,
            csrf: value["csrfToken"].as_str().unwrap().into(),
        }
    }
    async fn request(&self, method: &str, url: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(url)
                    .header("host", "127.0.0.1:4321")
                    .header("origin", "http://127.0.0.1:4321")
                    .header("cookie", &self.cookie)
                    .header("x-factory-csrf", &self.csrf)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

async fn wait_for(fixture: &Fixture, a: bool, id: i64, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    let context = fixture
        .app
        .projects
        .context(if a { &fixture.aid } else { &fixture.bid })
        .unwrap();
    loop {
        let status = context
            .db
            .lock()
            .unwrap()
            .get_run(id)
            .unwrap()
            .unwrap()
            .status;
        if status.as_str() == expected {
            break;
        }
        if matches!(status.as_str(), "failed" | "cancelled" | "completed") {
            let db = context.db.lock().unwrap();
            panic!("expected {expected}, observed {status:?}; tasks={:?}; attempts={:?}; sessions={:?}", db.list_tasks(id), db.list_task_attempts(id), db.list_agent_sessions(Some(id)));
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected}, observed {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identical_run_ids_are_scoped_and_legacy_default_never_switches() {
    let fixture = Fixture::new();
    let client = fixture.client().await;
    for (id, label) in [(&fixture.aid, "only A"), (&fixture.bid, "only B")] {
        fixture
            .app
            .projects
            .context(id)
            .unwrap()
            .db
            .lock()
            .unwrap()
            .create_run(label, None)
            .unwrap();
    }
    let (status, a) = client
        .request("GET", &fixture.route(true, "runs/1"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(a["run"]["objective"], "only A");
    let (_, b) = client
        .request("GET", &fixture.route(false, "runs/1"), Value::Null)
        .await;
    assert_eq!(b["run"]["objective"], "only B");
    let (_, legacy) = client.request("GET", "/api/runs/1", Value::Null).await;
    assert_eq!(legacy["run"]["objective"], "only A");
    assert_eq!(
        client
            .request(
                "GET",
                "/api/projects/p_00000000000000000000000000000000/runs/1",
                Value::Null
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .request("GET", "/api/projects/not-valid/runs/1", Value::Null)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    for path in [
        "/api/unknown-p03".to_owned(),
        fixture.route(true, "unknown-p03"),
    ] {
        let (status, error) = client.request("GET", &path, Value::Null).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["error"], "unknown endpoint");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configuration_agents_context_and_policies_remain_project_owned() {
    let fixture = Fixture::new();
    fixture.configure(false);
    let client = fixture.client().await;
    for (a, label) in [(true, "A"), (false, "B")] {
        let (status, config) = client
            .request("GET", &fixture.route(a, "config"), Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(config["agents"]["codex"]["env"]["ASF_PROJECT_PROOF"], label);
        let (status, hits) = client
            .request(
                "GET",
                &fixture.route(a, "context/search?q=same.txt"),
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(hits["hits"][0]["path"], "same.txt");
    }
    let before = std::fs::read(fixture.b.join(".factory/config.toml")).unwrap();
    let (status, _) = client
        .request(
            "PUT",
            &fixture.route(true, "roles/worker/policy"),
            json!({"preset":"read_only"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        std::fs::read(fixture.b.join(".factory/config.toml")).unwrap(),
        before
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_real_controlled_workflows_complete_with_distinct_worktrees_and_evidence() {
    let fixture = Fixture::new();
    fixture.configure(false);
    let client = fixture.client().await;
    for a in [true, false] {
        let (status, created) = client
            .request(
                "POST",
                &fixture.route(a, "runs"),
                json!({"objective":if a {"A proof"} else {"B proof"}}),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{created}");
    }
    for a in [true, false] {
        wait_for(&fixture, a, 1, "planned").await;
        assert_eq!(
            client
                .request("POST", &fixture.route(a, "runs/1/start"), json!({}))
                .await
                .0,
            StatusCode::ACCEPTED
        );
    }
    for (a, root, label) in [(true, &fixture.a, "A"), (false, &fixture.b, "B")] {
        wait_for(&fixture, a, 1, "completed").await;
        let context = fixture
            .app
            .projects
            .context(if a { &fixture.aid } else { &fixture.bid })
            .unwrap();
        let attempts = context.db.lock().unwrap().list_task_attempts(1).unwrap();
        assert!(!attempts.is_empty());
        let proof = std::fs::read_to_string(
            Path::new(&attempts[0].worktree_path).join("project-proof.txt"),
        )
        .unwrap();
        assert!(proof.starts_with(label));
        assert!(proof.contains(root.file_name().unwrap().to_str().unwrap()));
        assert!(attempts[0].commit_sha.is_some());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_cannot_cancel_b_and_busy_project_cannot_close() {
    let fixture = Fixture::new();
    fixture.configure(true);
    let client = fixture.client().await;
    for a in [true, false] {
        assert_eq!(
            client
                .request(
                    "POST",
                    &fixture.route(a, "runs"),
                    json!({"objective":"controlled cancellation"})
                )
                .await
                .0,
            StatusCode::ACCEPTED
        );
        wait_for(&fixture, a, 1, "planned").await;
        assert_eq!(
            client
                .request("POST", &fixture.route(a, "runs/1/start"), json!({}))
                .await
                .0,
            StatusCode::ACCEPTED
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    for (id, root, label) in [
        (&fixture.aid, &fixture.a, "A"),
        (&fixture.bid, &fixture.b, "B"),
    ] {
        let proof = root.join(".factory/worktrees/t1/project-proof.txt");
        while !proof.exists() {
            assert!(
                Instant::now() < deadline,
                "worker {label} did not reach its controlled gate"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(std::fs::read_to_string(&proof).unwrap().starts_with(label));
        let context = fixture.app.projects.context(id).unwrap();
        assert!(context
            .db
            .lock()
            .unwrap()
            .list_agent_sessions(Some(1))
            .unwrap()
            .iter()
            .any(|session| session.role == "worker" && session.status == "running"));
    }
    assert_eq!(
        client
            .request(
                "POST",
                &format!("/api/projects/{}/close", fixture.aid),
                json!({})
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .request("POST", &fixture.route(true, "runs/1/cancel"), json!({}))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    wait_for(&fixture, true, 1, "cancelled").await;
    let b = fixture.app.projects.context(&fixture.bid).unwrap();
    assert!(!b.runtime.is_idle());
    assert_ne!(
        b.db.lock()
            .unwrap()
            .get_run(1)
            .unwrap()
            .unwrap()
            .status
            .as_str(),
        "cancelled"
    );
    std::fs::write(fixture._dir.path().join("go-B"), "continue").unwrap();
    wait_for(&fixture, false, 1, "completed").await;
}

#[tokio::test]
async fn project_management_requires_authentication_origin_csrf_and_explicit_trust() {
    let fixture = Fixture::new();
    let app = application_router(fixture.app.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/register")
                .header("host", "127.0.0.1:4321")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let client = fixture.client().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/register")
                .header("host", "127.0.0.1:4321")
                .header("cookie", &client.cookie)
                .header("origin", "http://127.0.0.1:4321")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"path":fixture.a,"trust_project":true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        client
            .request("POST", "/api/projects/register", json!({"path":fixture.a}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

struct Server {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start(app: Arc<FactoryApplication>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, application_router(app))
                .await
                .unwrap();
        });
        Self { address, task }
    }
    async fn raw(&self, client: &Client, method: &str, path: &str, body: Value) -> (u16, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(self.address).await.unwrap();
        let body = body.to_string();
        let request = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nCookie: {}\r\nX-Factory-CSRF: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", self.address, self.address, client.cookie, client.csrf, body.len());
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(15), stream.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let status = text.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, text)
    }
    async fn socket(
        &self,
        client: &Client,
        path: &str,
        origin: Option<&str>,
    ) -> Result<TerminalSocket, Box<tokio_tungstenite::tungstenite::Error>> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://{}{path}", self.address)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("cookie", client.cookie.parse().unwrap());
        request.headers_mut().insert(
            "origin",
            origin
                .map(str::to_owned)
                .unwrap_or_else(|| format!("http://{}", self.address))
                .parse()
                .unwrap(),
        );
        tokio_tungstenite::connect_async(request)
            .await
            .map(|(socket, _)| socket)
            .map_err(Box::new)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
type TerminalSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn output_until(socket: &mut TerminalSocket, marker: &str) -> String {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let mut bytes = Vec::new();
    let mut answered = false;
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("terminal output deadline")
            .expect("terminal closed")
            .unwrap();
        if let Message::Binary(data) = message {
            bytes.extend_from_slice(&data);
        }
        if !answered && bytes.windows(4).any(|slice| slice == b"\x1b[6n") {
            socket
                .send(Message::Text(
                    json!({"type":"input","data":"\u{1b}[1;1R"}).to_string(),
                ))
                .await
                .unwrap();
            answered = true;
        }
        let text = String::from_utf8_lossy(&bytes);
        if text.contains(marker) {
            return text.into_owned();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_tcp_server_scopes_conpty_websockets_sse_resize_reconnect_and_termination() {
    use futures_util::SinkExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message;
    let fixture = Fixture::new();
    fixture.configure(false);
    let client = fixture.client().await;
    let server = Server::start(fixture.app.clone()).await;
    for a in [true, false] {
        let (status, response) = server
            .raw(
                &client,
                "POST",
                &fixture.route(a, "agents/codex/sessions"),
                json!({"cols":80,"rows":24}),
            )
            .await;
        assert_eq!(status, 200, "{response}");
        assert!(response.contains("\"id\":1"), "{response}");
    }
    let mut a = server
        .socket(&client, &fixture.route(true, "sessions/1/terminal"), None)
        .await
        .unwrap();
    let mut b = server
        .socket(&client, &fixture.route(false, "sessions/1/terminal"), None)
        .await
        .unwrap();
    assert!(!output_until(&mut a, "terminal-A")
        .await
        .contains("terminal-B"));
    assert!(!output_until(&mut b, "terminal-B")
        .await
        .contains("terminal-A"));
    a.send(Message::Text(
        json!({"type":"resize","cols":104,"rows":30}).to_string(),
    ))
    .await
    .unwrap();
    a.send(Message::Text(
        json!({"type":"input","data":"owned-a\r\n"}).to_string(),
    ))
    .await
    .unwrap();
    assert!(output_until(&mut a, "A:owned-a").await.contains("owned-a"));
    a.close(None).await.unwrap();
    let mut a = server
        .socket(&client, &fixture.route(true, "sessions/1/terminal"), None)
        .await
        .unwrap();
    assert!(output_until(&mut a, "A:owned-a")
        .await
        .contains("terminal-A"));
    for (scope, own, other) in [
        (true, "project A", "project B"),
        (false, "project B", "project A"),
    ] {
        let mut stream = tokio::net::TcpStream::connect(server.address)
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "GET {} HTTP/1.1\r\nHost: {}\r\nCookie: {}\r\n\r\n",
                    fixture.route(scope, "sessions/1/stream"),
                    server.address,
                    client.cookie
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut all = Vec::new();
        let mut buffer = [0; 8192];
        while !String::from_utf8_lossy(&all).contains(own) {
            let count = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(count, 0);
            all.extend_from_slice(&buffer[..count]);
        }
        let text = String::from_utf8_lossy(&all);
        assert!(!text.contains(other));
        assert!(text.contains(if scope {
            fixture.aid.as_str()
        } else {
            fixture.bid.as_str()
        }));
    }
    assert!(server
        .socket(
            &client,
            &fixture.route(false, "sessions/999/terminal"),
            None
        )
        .await
        .is_err());
    assert!(server
        .socket(
            &client,
            &fixture.route(true, "sessions/1/terminal"),
            Some("https://invalid.test")
        )
        .await
        .is_err());
    let response = server
        .raw(
            &client,
            "DELETE",
            &fixture.route(true, "sessions/1"),
            Value::Null,
        )
        .await;
    assert_eq!(response.0, 202, "{}", response.1);
    b.send(Message::Text(
        json!({"type":"input","data":"still-b\r\n"}).to_string(),
    ))
    .await
    .unwrap();
    assert!(output_until(&mut b, "B:still-b").await.contains("still-b"));
    a.close(None).await.ok();
    b.close(None).await.unwrap();
    let response = server
        .raw(
            &client,
            "DELETE",
            &fixture.route(false, "sessions/1"),
            Value::Null,
        )
        .await;
    assert_eq!(response.0, 202, "{}", response.1);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if fixture
            .app
            .projects
            .context(&fixture.aid)
            .unwrap()
            .runtime
            .is_idle()
            && fixture
                .app
                .projects
                .context(&fixture.bid)
                .unwrap()
                .runtime
                .is_idle()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal processes not cleaned up"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_grants_delivery_and_deleted_projects_never_fall_back_to_another_project() {
    use factory_types::ToolGrant;
    let fixture = Fixture::new();
    let client = fixture.client().await;
    let a = fixture.app.projects.context(&fixture.aid).unwrap();
    let b = fixture.app.projects.context(&fixture.bid).unwrap();
    for (context, label) in [(&a, "A"), (&b, "B")] {
        context.db.lock().unwrap().create_run(label, None).unwrap();
    }
    let grant = ToolGrant {
        id: "grant-a".into(),
        token_hash: "hash-a".into(),
        run_id: Some(1),
        task_id: None,
        attempt_id: None,
        session_id: None,
        agent: "codex".into(),
        role: "worker".into(),
        operation: None,
        allowed_tools: vec!["A.only".into()],
        expires_at: "2099-01-01T00:00:00Z".into(),
        revoked_at: None,
    };
    a.db.lock().unwrap().insert_tool_grant(&grant).unwrap();
    assert!(b
        .db
        .lock()
        .unwrap()
        .get_tool_grant_by_hash("hash-a")
        .unwrap()
        .is_none());
    for (root, label, scope) in [(&fixture.a, "A", true), (&fixture.b, "B", false)] {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "remote",
                "add",
                "origin",
                &format!("https://github.com/fixture/{label}.git"),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let (status, delivery) = client
            .request("GET", &fixture.route(scope, "runs/1/delivery"), Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK, "{delivery}");
        assert_eq!(
            delivery["repository"]["repository"],
            format!("fixture/{label}")
        );
    }
    let before = std::fs::read(fixture.b.join(".factory/config.toml")).unwrap();
    let response=client.request("POST",&fixture.route(true,"mcp/servers"),json!({"id":"local_a","transport":"stdio","command":"not-executed","args":[],"env":{},"timeout_ms":100,"discovery_timeout_ms":100,"max_response_bytes":4096,"max_concurrency":1})).await;
    assert_eq!(response.0, StatusCode::CREATED, "{}", response.1);
    assert_eq!(
        std::fs::read(fixture.b.join(".factory/config.toml")).unwrap(),
        before
    );
    assert_eq!(
        client
            .request("GET", &fixture.route(false, "mcp/servers"), Value::Null)
            .await
            .1,
        json!([])
    );
    for scope in [true, false] {
        let (status, _) = client
            .request(
                "POST",
                &fixture.route(scope, "runs/1/pull-request"),
                json!({"title":"must not publish"}),
            )
            .await;
        assert!(!status.is_success());
    }
    drop(a);
    drop(b);
    fixture.app.projects.close(&fixture.bid).unwrap();
    fixture.app.projects.remove(&fixture.bid).unwrap();
    assert_eq!(
        client
            .request("GET", &fixture.route(false, "runs/1"), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client.request("GET", "/api/runs/1", Value::Null).await.1["run"]["objective"],
        "A"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn project_management_roundtrip_initializes_clones_renames_and_removes_without_deleting_files(
) {
    let fixture = Fixture::new();
    let client = fixture.client().await;
    let root = fixture._dir.path().join("registered");
    std::fs::create_dir(&root).unwrap();
    let result = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["init", "-q"])
        .status()
        .unwrap();
    assert!(result.success());
    let (status, entry) = client
        .request(
            "POST",
            "/api/projects/register",
            json!({"path":root,"trust_project":true}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{entry}");
    let id = entry["project_id"].as_str().unwrap();
    let route = |tail: &str| format!("/api/projects/{id}{tail}");
    assert_eq!(
        client
            .request("POST", &route("/initialize"), json!({"trust_project":true}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        client.request("POST", &route("/open"), json!({})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        client
            .request("PUT", &route(""), json!({"display_name":"Renamed"}))
            .await
            .1["project_id"],
        id
    );
    assert_eq!(
        client.request("GET", &route("/runs"), Value::Null).await.1,
        json!([])
    );
    assert_eq!(
        client.request("POST", &route("/close"), json!({})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        client.request("DELETE", &route(""), Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert!(root.join(".factory/db.sqlite3").is_file());
    let created = fixture._dir.path().join("created");
    assert_eq!(
        client
            .request(
                "POST",
                "/api/projects/create",
                json!({"destination":created,"display_name":"New","trust_project":true})
            )
            .await
            .0,
        StatusCode::CREATED
    );
    let cloned = fixture._dir.path().join("cloned");
    let (status,body)=client.request("POST","/api/projects/clone",json!({"remote":fixture.a,"destination":cloned,"display_name":"Clone","initialize":true,"trust_project":true})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(cloned.join(".factory/db.sqlite3").is_file());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_operation_budget_refuses_before_creating_project_b_run() {
    let fixture = Fixture::new();
    fixture.configure(false);
    let client = fixture.client().await;
    let permits: Vec<_> = (0..fixture.app.projects.limits.concurrent_operations)
        .map(|_| fixture.app.projects.budget.operation().unwrap())
        .collect();
    assert_eq!(
        client
            .request(
                "POST",
                &fixture.route(false, "runs"),
                json!({"objective":"must not be created"})
            )
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(fixture
        .app
        .projects
        .context(&fixture.bid)
        .unwrap()
        .db
        .lock()
        .unwrap()
        .list_runs()
        .unwrap()
        .is_empty());
    drop(permits);
    assert_eq!(
        client
            .request(
                "POST",
                &fixture.route(false, "runs"),
                json!({"objective":"budget released"})
            )
            .await
            .0,
        StatusCode::ACCEPTED
    );
    wait_for(&fixture, false, 1, "planned").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_default_fails_explicitly_and_registry_corruption_cannot_redirect_scoped_requests() {
    let dir = TempDir::new().unwrap();
    let app = Arc::new(
        FactoryApplication::new(ApplicationOptions {
            registry_path: dir.path().join("application/registry.sqlite3"),
            allowed_roots: vec![dir.path().to_owned()],
            default_root: None,
            dashboard_root: dir.path().to_owned(),
            limits: ProjectLimits::default(),
            api: ApiOptions::default(),
        })
        .unwrap(),
    );
    let client = Client::new(application_router(app)).await;
    let (status, body) = client.request("GET", "/api/runs/1", Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "no_default_project");
    let fixture = Fixture::new();
    let client = fixture.client().await;
    let connection =
        rusqlite::Connection::open(fixture._dir.path().join("application/registry.sqlite3"))
            .unwrap();
    connection
        .execute(
            "UPDATE projects SET project_id='malformed' WHERE project_id=?1",
            [fixture.bid.as_str()],
        )
        .unwrap();
    let (status, body) = client
        .request("GET", &fixture.route(false, "runs/1"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "registry_unavailable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corrupt_startup_default_is_visible_without_preventing_project_b() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("A");
    let b = dir.path().join("B");
    for root in [&a, &b] {
        std::fs::create_dir(root).unwrap();
        factory_git_init(root);
        factory_core::Factory::init(root).unwrap();
    }
    std::fs::write(a.join(".factory/db.sqlite3"), "corrupt fixture").unwrap();
    let app = Arc::new(
        FactoryApplication::new(ApplicationOptions {
            registry_path: dir.path().join("application/registry.sqlite3"),
            allowed_roots: vec![dir.path().to_owned()],
            default_root: Some(a),
            dashboard_root: dir.path().to_owned(),
            limits: ProjectLimits::default(),
            api: ApiOptions::default(),
        })
        .unwrap(),
    );
    let aid = app.default_project.clone().unwrap();
    assert_eq!(app.projects.get(&aid).unwrap().availability, "unavailable");
    let bid = app.projects.register(&b, None).unwrap().project_id;
    app.projects.open(&bid).unwrap();
    let client = Client::new(application_router(app)).await;
    assert_eq!(
        client
            .request("GET", &format!("/api/projects/{bid}/runs"), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        client.request("GET", "/api/runs", Value::Null).await.0,
        StatusCode::CONFLICT
    );
}

#[test]
fn duplicate_default_server_refuses_the_existing_runtime_owner() {
    let fixture = Fixture::new();
    let duplicate = FactoryApplication::new(ApplicationOptions {
        registry_path: fixture._dir.path().join("application/registry.sqlite3"),
        allowed_roots: vec![fixture._dir.path().to_owned()],
        default_root: Some(fixture.a.clone()),
        dashboard_root: fixture.a.clone(),
        limits: ProjectLimits::default(),
        api: ApiOptions::default(),
    });
    assert!(
        matches!(duplicate,Err(factory_projects::ProjectError::Runtime(error)) if matches!(error.as_ref(),factory_runtime::RuntimeError::ProjectAlreadyLocked {..}))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_parse_failures_never_reflect_source_secrets_in_project_errors_or_listing() {
    let fixture = Fixture::new();
    let client = fixture.client().await;
    let path = fixture.a.join(".factory/config.toml");
    let original = std::fs::read(&path).unwrap();
    fixture.app.projects.close(&fixture.aid).unwrap();
    let marker = "P03_SYNTHETIC_DO_NOT_REFLECT";
    let corrupted = format!("[agents.codex.env]\nAPI_TOKEN = \"{marker}\n");
    std::fs::write(&path, &corrupted).unwrap();
    let (status, error) = client
        .request(
            "POST",
            &format!("/api/projects/{}/open", fixture.aid),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!error.to_string().contains(marker));
    assert!(error["error"].as_str().unwrap().contains("invalid TOML"));
    let (status, listed) = client.request("GET", "/api/projects", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!listed.to_string().contains(marker));
    assert_eq!(
        client
            .request("GET", &fixture.route(false, "runs"), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    std::fs::write(&path, original).unwrap();
    fixture.app.projects.open(&fixture.aid).unwrap();
    std::fs::write(&path, &corrupted).unwrap();
    let (status, error) = client
        .request("GET", &fixture.route(true, "config"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!error.to_string().contains(marker));
    assert!(error["error"].as_str().unwrap().contains("invalid TOML"));
}
