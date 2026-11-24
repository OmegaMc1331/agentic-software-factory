use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_core::{AgentEntry, Config as FactoryConfig};
use factory_runtime::Runtime;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::files;
use crate::protocol::Service;
use crate::security::{self, ApiOptions, SecurityPolicy};

#[derive(Deserialize, Clone)]
pub struct Config {
    pub projects: BTreeMap<String, String>,
    pub node: String,
    pub python: String,
    pub lsp: String,
    pub pyright: String,
    pub probe: String,
    #[cfg(not(feature = "embedded-assets"))]
    pub assets: String,
}

pub struct Project {
    pub root: PathBuf,
    pub runtime: Runtime,
    writes: Mutex<()>,
    terminals: Mutex<Vec<i64>>,
}

pub struct ApiState {
    pub(crate) security: SecurityPolicy,
    pub config: Config,
    pub projects: BTreeMap<String, Project>,
    pub services: Mutex<HashMap<String, Arc<Service>>>,
    pub shutdown: tokio::sync::Notify,
}

pub type SharedState = Arc<ApiState>;

#[derive(Debug)]
pub struct Error(pub StatusCode, pub String);
impl Error {
    pub fn bad(text: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, text.into())
    }
    pub fn denied(text: &str) -> Self {
        Self(StatusCode::FORBIDDEN, text.into())
    }
}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self(
            if value.kind() == std::io::ErrorKind::NotFound {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            value.to_string(),
        )
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}

impl ApiState {
    pub fn project(&self, id: &str) -> Result<&Project, Error> {
        self.projects
            .get(id)
            .ok_or_else(|| Error(StatusCode::NOT_FOUND, "unknown project".into()))
    }
    pub fn cleanup(&self) {
        for service in self.services.lock().unwrap().values() {
            service.stop();
        }
        for project in self.projects.values() {
            for id in project.terminals.lock().unwrap().iter() {
                let _ = project.runtime.stop_interactive_session(*id);
            }
        }
    }
}

pub fn load(path: &str) -> Result<SharedState, Box<dyn std::error::Error>> {
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut projects = BTreeMap::new();
    for (id, folder) in &config.projects {
        let root = std::fs::canonicalize(folder)?;
        if std::fs::read_to_string(root.join(".asf-p01-fixture")).unwrap_or_default()
            != "ASF_P01_OWNED_FIXTURE_V1"
        {
            return Err("refusing a repository that is not an owned P01 fixture".into());
        }
        if !root.join(".git").exists() {
            return Err("prototype accepts only explicitly configured Git fixtures".into());
        }
        factory_core::Factory::init(&root)?;
        let mut factory_config = FactoryConfig::default();
        factory_config.agents.insert(
            "p01-trusted-pty-probe".into(),
            AgentEntry {
                kind: None,
                command: config.python.clone(),
                args: Vec::new(),
                env: BTreeMap::new(),
                prompt_transport: None,
                interactive_args: Some(vec!["-u".into(), config.probe.clone()]),
                capabilities: Vec::new(),
                max_concurrency: Some(1),
            },
        );
        factory_config.write_atomic(&root)?;
        projects.insert(
            id.clone(),
            Project {
                runtime: Runtime::new(&root)?,
                root,
                writes: Mutex::new(()),
                terminals: Mutex::new(Vec::new()),
            },
        );
    }
    let state = Arc::new(ApiState {
        security: SecurityPolicy::new(ApiOptions::default()),
        config,
        projects,
        services: Mutex::new(HashMap::new()),
        shutdown: tokio::sync::Notify::new(),
    });
    let weak = Arc::downgrade(&state);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let Some(state) = weak.upgrade() else {
                break;
            };
            for service in state.services.lock().unwrap().values() {
                if service.created.elapsed() > Duration::from_secs(180) {
                    service.stop();
                }
            }
        }
    });
    Ok(state)
}

pub fn router(state: SharedState) -> Router {
    let router = Router::new()
        .route(
            "/api/health",
            get(|| async { Json(json!({"status":"ok","prototype":"P01"})) }),
        )
        .route("/api/auth/session", post(security::issue_browser_session))
        .route("/api/projects", get(projects))
        .route("/api/projects/:project/tree", get(tree))
        .route("/api/projects/:project/file", get(read).put(save))
        .route(
            "/api/projects/:project/state/:candidate",
            get(state_get).put(state_put),
        )
        .route(
            "/api/projects/:project/extensions/:candidate",
            get(extension_get).post(extension_update),
        )
        .route(
            "/api/projects/:project/protocol/:kind",
            post(protocol_start),
        )
        .route(
            "/api/projects/:project/transport/:id",
            get(protocol_socket).delete(protocol_stop),
        )
        .route("/api/projects/:project/trace/:id", get(protocol_trace))
        .route("/api/projects/:project/terminal", post(terminal_start))
        .route(
            "/api/projects/:project/terminal/:id",
            get(terminal_socket).delete(terminal_stop),
        )
        .route("/api/processes", get(processes))
        .route("/api/shutdown", post(shutdown))
        .layer(DefaultBodyLimit::max(1_048_576));
    #[cfg(not(feature = "embedded-assets"))]
    let router = router.fallback_service(tower_http::services::ServeDir::new(&state.config.assets));
    #[cfg(feature = "embedded-assets")]
    let router = router.fallback(embedded);
    router
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::enforce_security,
        ))
        .with_state(state)
}

async fn projects(State(state): State<SharedState>) -> Json<Value> {
    Json(json!(state
        .projects
        .iter()
        .map(|(id, p)| {
            let uri = url::Url::from_directory_path(&p.root).unwrap();
            json!({"id":id,"root":uri.to_file_path().unwrap(),"uri":uri.as_str()})
        })
        .collect::<Vec<_>>()))
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}
#[derive(Deserialize)]
struct SaveDocument {
    path: String,
    revision: String,
    content: String,
}

async fn tree(
    State(state): State<SharedState>,
    UrlPath(project): UrlPath<String>,
) -> Result<Json<Value>, Error> {
    Ok(Json(files::tree(&state.project(&project)?.root)?))
}
async fn read(
    State(state): State<SharedState>,
    UrlPath(project): UrlPath<String>,
    Query(query): Query<FileQuery>,
) -> Result<Json<Value>, Error> {
    Ok(Json(files::read(
        &state.project(&project)?.root,
        &query.path,
    )?))
}
async fn save(
    State(state): State<SharedState>,
    UrlPath(project): UrlPath<String>,
    Json(body): Json<SaveDocument>,
) -> Result<Json<Value>, Error> {
    let project = state.project(&project)?;
    let _lock = project.writes.lock().unwrap();
    Ok(Json(files::save(
        &project.root,
        &body.path,
        &body.revision,
        &body.content,
    )?))
}

fn state_path(project: &Project, candidate: &str, kind: &str) -> Result<PathBuf, Error> {
    if !["monaco", "codemirror"].contains(&candidate) {
        return Err(Error::bad("unknown candidate"));
    }
    let folder = project.root.join(".factory/p01");
    std::fs::create_dir_all(&folder)?;
    Ok(folder.join(format!("{kind}-{candidate}.json")))
}

fn json_read(path: &Path) -> Result<Value, Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| Error::bad("invalid persisted prototype state")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(error.into()),
    }
}

fn json_write(path: &Path, value: &Value) -> Result<(), Error> {
    use std::io::Write;
    let mut temporary = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| Error::bad("missing state parent"))?,
    )?;
    temporary.write_all(&serde_json::to_vec(value).map_err(|_| Error::bad("invalid state"))?)?;
    temporary.persist(path).map_err(|e| Error::from(e.error))?;
    Ok(())
}

async fn state_get(
    State(state): State<SharedState>,
    UrlPath((project, candidate)): UrlPath<(String, String)>,
) -> Result<Json<Value>, Error> {
    Ok(Json(json_read(&state_path(
        state.project(&project)?,
        &candidate,
        "workspace",
    )?)?))
}
async fn state_put(
    State(state): State<SharedState>,
    UrlPath((project, candidate)): UrlPath<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Error> {
    let project = state.project(&project)?;
    for item in body["documents"]
        .as_array()
        .ok_or_else(|| Error::bad("missing document list"))?
    {
        files::path(
            &project.root,
            item["path"]
                .as_str()
                .ok_or_else(|| Error::bad("missing document path"))?,
        )?;
    }
    json_write(&state_path(project, &candidate, "workspace")?, &body)?;
    Ok(Json(json!({"stored":true})))
}

async fn extension_get(
    State(state): State<SharedState>,
    UrlPath((project, candidate)): UrlPath<(String, String)>,
) -> Result<Json<Value>, Error> {
    let status = json_read(&state_path(
        state.project(&project)?,
        &candidate,
        "extension",
    )?)?;
    Ok(Json(
        json!({"id":"yaml","package":if candidate == "monaco" {"monaco-yaml"} else {"@codemirror/lang-yaml"},
        "version":if candidate == "monaco" {"5.5.1"} else {"6.1.3"},"license":"MIT","source":"pinned npm package bundled locally","permissions":["editor-language"],"status":status}),
    ))
}

#[derive(Deserialize)]
struct ExtensionIntent {
    action: String,
    id: String,
}
async fn extension_update(
    State(state): State<SharedState>,
    UrlPath((project, candidate)): UrlPath<(String, String)>,
    Json(body): Json<ExtensionIntent>,
) -> Result<Json<Value>, Error> {
    if body.id != "yaml" {
        return Err(Error::denied(
            "only the pinned trusted YAML package is accepted",
        ));
    }
    let path = state_path(state.project(&project)?, &candidate, "extension")?;
    let previous = json_read(&path)?;
    let status = match body.action.as_str() {
        "install" => json!({"installed":true,"active":false}),
        "activate" if previous["installed"] == true => json!({"installed":true,"active":true}),
        "deactivate" if previous["installed"] == true => json!({"installed":true,"active":false}),
        "uninstall" => json!({"installed":false,"active":false}),
        _ => return Err(Error::bad("invalid extension transition")),
    };
    json_write(&path, &status)?;
    Ok(Json(status))
}

async fn protocol_start(
    State(state): State<SharedState>,
    UrlPath((project, kind)): UrlPath<(String, String)>,
) -> Result<Json<Value>, Error> {
    let root = &state.project(&project)?.root;
    let mut services = state.services.lock().unwrap();
    if services.values().filter(|s| s.running()).count() >= 4 {
        return Err(Error::denied("prototype process capacity reached"));
    }
    if services
        .values()
        .any(|s| s.project == project && s.kind == kind && s.running())
    {
        return Err(Error(
            StatusCode::CONFLICT,
            "profile already owns an active process".into(),
        ));
    }
    let service = Service::start(&state.config, root, &project, &kind)?;
    let reply = json!({"id":service.id,"pid":service.pid,"kind":kind,"lease_seconds":180});
    services.insert(service.id.clone(), service);
    Ok(Json(reply))
}

fn service(state: &SharedState, project: &str, id: &str) -> Result<Arc<Service>, Error> {
    let service = state
        .services
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or_else(|| Error::bad("unknown process lease"))?;
    if service.project != project {
        return Err(Error::denied("process belongs to another project"));
    }
    Ok(service)
}
async fn protocol_trace(
    State(state): State<SharedState>,
    UrlPath((project, id)): UrlPath<(String, String)>,
) -> Result<Json<Value>, Error> {
    let service = service(&state, &project, &id)?;
    let trace: Vec<Value> = service.trace.lock().unwrap().iter().cloned().collect();
    Ok(Json(
        json!({"pid":service.pid,"running":service.running(),"trace":trace}),
    ))
}
async fn protocol_stop(
    State(state): State<SharedState>,
    UrlPath((project, id)): UrlPath<(String, String)>,
) -> Result<Json<Value>, Error> {
    service(&state, &project, &id)?.stop();
    Ok(Json(json!({"stopped":true})))
}
async fn protocol_socket(
    State(state): State<SharedState>,
    UrlPath((project, id)): UrlPath<(String, String)>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Error> {
    let service = service(&state, &project, &id)?;
    if !service.running() {
        return Err(Error::bad("process already exited"));
    }
    Ok(upgrade
        .max_message_size(1_048_576)
        .on_upgrade(move |socket| protocol_connection(state, project, service, socket)))
}
async fn protocol_connection(
    state: SharedState,
    project: String,
    service: Arc<Service>,
    mut socket: WebSocket,
) {
    let mut output = service.output.subscribe();
    loop {
        tokio::select! {
            message = output.recv() => {
                let Ok(message) = message else { break; };
                if tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(message.to_string()))).await.ok().and_then(Result::ok).is_none() { break; }
                if message["type"] == "transport-ended" { break; }
            }
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let result = serde_json::from_str(&text).map_err(|_| Error::bad("invalid JSON")).and_then(|m| service.send(&state.project(&project).unwrap().root, &state.config, m));
                        if let Err(error) = result { let _ = socket.send(Message::Text(json!({"type":"policy-error","error":error.1}).to_string())).await; }
                    }
                    Some(Ok(Message::Ping(data))) => { let _ = socket.send(Message::Pong(data)).await; }
                    _ => break,
                }
            }
        }
    }
}

async fn terminal_start(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, Error> {
    let project = state.project(&id)?;
    if !project.terminals.lock().unwrap().is_empty() {
        return Err(Error(
            StatusCode::CONFLICT,
            "one trusted PTY per project in this experiment".into(),
        ));
    }
    let session = project
        .runtime
        .start_interactive_session("p01-trusted-pty-probe", 90, 24)
        .map_err(|e| Error::bad(&e.to_string()))?;
    project.terminals.lock().unwrap().push(session.id);
    Ok(Json(
        json!({"id":session.id,"profile":"trusted non-shell PTY probe","owner":"factory-runtime"}),
    ))
}
async fn terminal_stop(
    State(state): State<SharedState>,
    UrlPath((project, id)): UrlPath<(String, i64)>,
) -> Result<Json<Value>, Error> {
    let project = state.project(&project)?;
    let diagnostic = project
        .runtime
        .stop_interactive_session(id)
        .err()
        .map(|error| error.to_string());
    for _ in 0..100 {
        let factory = factory_core::Factory::open(&project.root)
            .map_err(|error| Error::bad(&error.to_string()))?;
        let sessions = factory
            .list_agent_sessions(None)
            .map_err(|error| Error::bad(&error.to_string()))?;
        if let Some(session) = sessions
            .into_iter()
            .find(|session| session.id == id && session.finished_at.is_some())
        {
            project
                .terminals
                .lock()
                .unwrap()
                .retain(|session| *session != id);
            return Ok(Json(
                json!({"stopped":true,"observed_status":session.status,"exit_code":session.exit_code,"upstream_termination_diagnostic":diagnostic,"reconciled_from_durable_session":true}),
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(Error::bad(
        "PTY exit not observed; termination outcome remains uncertain",
    ))
}
async fn terminal_socket(
    State(state): State<SharedState>,
    UrlPath((project, id)): UrlPath<(String, i64)>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Error> {
    let subscription = state
        .project(&project)?
        .runtime
        .subscribe_terminal(id)
        .map_err(|e| Error::bad(&e.to_string()))?;
    Ok(upgrade.max_message_size(8192).on_upgrade(move |mut socket| async move {
        let mut receiver = subscription.receiver;
        let _ = socket.send(Message::Text(json!({"type":"output","data":String::from_utf8_lossy(&subscription.snapshot)}).to_string())).await;
        loop {
            tokio::select! {
                chunk = receiver.recv() => {
                    let Some(chunk) = chunk else { break; };
                    if socket.send(Message::Text(json!({"type":"output","data":String::from_utf8_lossy(&chunk)}).to_string())).await.is_err() { break; }
                }
                message = socket.recv() => {
                    let Some(Ok(Message::Text(text))) = message else { break; };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else { break; };
                    let runtime = &state.project(&project).unwrap().runtime;
                    match value["type"].as_str() {
                        Some("input") => { let _ = runtime.write_terminal(id, value["data"].as_str().unwrap_or_default().as_bytes()); }
                        Some("resize") => { let _ = runtime.resize_terminal(id, value["cols"].as_u64().unwrap_or(90).clamp(10, 300) as u16, value["rows"].as_u64().unwrap_or(24).clamp(5, 100) as u16); }
                        _ => break,
                    }
                }
            }
        }
    }))
}

async fn processes(State(state): State<SharedState>) -> Json<Value> {
    Json(
        json!({"backend_pid":std::process::id(),"protocol":state.services.lock().unwrap().values().map(|s| json!({"id":s.id,"pid":s.pid,"kind":s.kind,"project":s.project,"running":s.running()})).collect::<Vec<_>>()}),
    )
}
async fn shutdown(State(state): State<SharedState>) -> Json<Value> {
    state.cleanup();
    state.shutdown.notify_one();
    Json(json!({"stopping":true}))
}

#[cfg(feature = "embedded-assets")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../../.factory/v2/p01/dist/"]
struct Assets;
#[cfg(feature = "embedded-assets")]
async fn embedded(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.ends_with('/') {
        format!("{path}index.html")
    } else {
        path.to_string()
    };
    match <Assets as rust_embed::RustEmbed>::get(&path) {
        Some(asset) => (
            [("content-type", asset.metadata.mimetype().to_string())],
            asset.data.into_owned(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
