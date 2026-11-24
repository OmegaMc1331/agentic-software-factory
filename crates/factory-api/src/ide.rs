use crate::app::SharedState;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_ide::{IdeError, IdeManager, ToolProfile};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;

static REQUESTS: Semaphore = Semaphore::const_new(16);
static CONTROLS: Semaphore = Semaphore::const_new(4);

pub(crate) fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/ide/state", get(state))
        .route("/api/ide/audit", get(audit))
        .route("/api/ide/profiles", post(profile))
        .route("/api/ide/profiles/:id", post(remove_profile))
        .route("/api/ide/configurations", post(configuration))
        .route("/api/ide/lsp/start", post(start_lsp))
        .route("/api/ide/lsp/:id/events", get(lsp_events))
        .route("/api/ide/lsp/:id/:action", post(lsp_action))
        .route("/api/ide/dap/start", post(start_dap))
        .route("/api/ide/dap/:id/events", get(dap_events))
        .route("/api/ide/dap/:id/:action", post(dap_action))
        .route("/api/ide/terminals/start", post(start_terminal))
        .route("/api/ide/terminals/:id/socket", get(terminal_socket))
        .route("/api/ide/terminals/:id/:action", post(terminal_action))
        .route("/api/ide/extensions/install", post(install_extension))
        .route("/api/ide/extensions/:id/:action", post(extension_action))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
}

struct Error(IdeError);
impl From<IdeError> for Error {
    fn from(error: IdeError) -> Self {
        Self(error)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.0 {
            IdeError::Invalid | IdeError::Protocol => StatusCode::BAD_REQUEST,
            IdeError::Forbidden | IdeError::Unapproved => StatusCode::FORBIDDEN,
            IdeError::NotFound => StatusCode::NOT_FOUND,
            IdeError::Conflict => StatusCode::CONFLICT,
            IdeError::Limit => StatusCode::TOO_MANY_REQUESTS,
            IdeError::Unsupported => StatusCode::NOT_IMPLEMENTED,
            IdeError::Timeout => StatusCode::GATEWAY_TIMEOUT,
            IdeError::Cancelled => StatusCode::REQUEST_TIMEOUT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        (
            status,
            Json(json!({"error":self.0.to_string(),"reason":self.0})),
        )
            .into_response()
    }
}

fn service(state: &SharedState) -> Result<Arc<IdeManager>, Error> {
    state
        ._context
        .as_ref()
        .map(|context| context.ide.clone())
        .ok_or(Error(IdeError::Forbidden))
}

async fn operation(
    state: SharedState,
    action: impl FnOnce(&IdeManager) -> factory_ide::Result<Value> + Send + 'static,
) -> Result<Json<Value>, Error> {
    execute(state, action, false).await
}
async fn execute(
    state: SharedState,
    action: impl FnOnce(&IdeManager) -> factory_ide::Result<Value> + Send + 'static,
    control: bool,
) -> Result<Json<Value>, Error> {
    let ide = service(&state)?;
    let project = if control {
        None
    } else {
        Some(ide.acquire_request()?)
    };
    let global = if control { &CONTROLS } else { &REQUESTS }
        .try_acquire()
        .map_err(|_| Error(IdeError::Limit))?;
    tokio::task::spawn_blocking(move || {
        let _lease = state;
        let _permits = (project, global);
        let result = action(&ide);
        let kind = match &result {
            Err(IdeError::Unapproved | IdeError::Forbidden) => {
                Some(factory_db::ServiceNotification::ApprovalRequired)
            }
            Err(
                IdeError::Executable | IdeError::Exited | IdeError::Timeout | IdeError::Tool(_),
            ) => Some(factory_db::ServiceNotification::IdeFailure),
            _ => None,
        };
        if let Some(kind) = kind {
            crate::notifications::record_service(
                &_lease,
                kind,
                &format!(
                    "ide:{}:{}",
                    chrono::Utc::now().timestamp() / 300,
                    match &result {
                        Err(IdeError::Unapproved | IdeError::Forbidden) => "approval",
                        _ => "failure",
                    }
                ),
            );
        }
        result
    })
    .await
    .map_err(|_| Error(IdeError::Exited))?
    .map(Json)
    .map_err(Error)
}

async fn state(State(state): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(state,|ide|Ok(json!({"profiles":ide.profiles()?,"servers":ide.servers()?,"debug_sessions":ide.debug_sessions()?,"terminals":ide.terminals()?,"configurations":ide.configurations()?,"extensions":ide.extensions.list()?}))).await
}
async fn audit(State(state): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(state, |ide| Ok(json!(ide.audit_entries()?))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRequest {
    profile: ToolProfile,
    confirmed: bool,
}
async fn profile(
    State(state): State<SharedState>,
    Json(body): Json<ProfileRequest>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        Ok(json!(ide.approve_profile(body.profile, body.confirmed)?))
    })
    .await
}
async fn remove_profile(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        ide.remove_profile(&id)?;
        Ok(json!({"removed":true}))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationRequest {
    configuration: factory_ide::dap::LaunchConfiguration,
    confirmed: bool,
}
async fn configuration(
    State(state): State<SharedState>,
    Json(body): Json<ConfigurationRequest>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        Ok(json!(ide.approve_configuration(
            body.configuration,
            body.confirmed
        )?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LspStart {
    root_id: String,
    profile_id: String,
    client_id: String,
    confirmed: bool,
}
async fn start_lsp(
    State(state): State<SharedState>,
    Json(body): Json<LspStart>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        Ok(json!(ide.start_lsp(
            &body.root_id,
            &body.profile_id,
            &body.client_id,
            body.confirmed
        )?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    #[serde(default)]
    cursor: u64,
}
async fn lsp_events(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        Ok(json!(ide.lsp(&id)?.events(query.cursor)))
    })
    .await
}
async fn dap_events(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        Ok(json!(ide.dap(&id)?.events(query.cursor)))
    })
    .await
}

async fn lsp_action(
    State(state): State<SharedState>,
    Path((id, action)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Error> {
    if action == "stop" || action == "cancel" {
        return execute(
            state,
            move |ide| {
                if action == "stop" {
                    ide.stop_lsp(&id)?;
                    Ok(json!({"stopped":true}))
                } else {
                    ide.lsp(&id)?
                        .cancel(body["request_id"].as_str().ok_or(IdeError::Invalid)?)?;
                    Ok(Value::Null)
                }
            },
            true,
        )
        .await;
    }
    operation(state, move |ide| {
        let server = ide.lsp(&id)?;
        match action.as_str() {
            "sync" => server.sync(
                serde_json::from_value(body["document"].clone()).map_err(|_| IdeError::Invalid)?,
                body["saved"] == true,
            ),
            "close" => {
                server.close(
                    &serde_json::from_value(body["file"].clone()).map_err(|_| IdeError::Invalid)?,
                )?;
                Ok(Value::Null)
            }
            "request" if body["request_id"].is_string() => server.request_token(
                body["method"].as_str().ok_or(IdeError::Invalid)?,
                body["params"].clone(),
                body["confirmed"] == true,
                body["request_id"].as_str().ok_or(IdeError::Invalid)?,
            ),
            "request" => server.request(
                body["method"].as_str().ok_or(IdeError::Invalid)?,
                body["params"].clone(),
                body["confirmed"] == true,
            ),
            "prepare-edit" => Ok(json!(server.prepare_verified_edit(&body["edit"])?)),
            "resolve" => Ok(json!(factory_ide::lsp::reference(
                &ide.files,
                &server.root_id,
                body["uri"].as_str().ok_or(IdeError::Invalid)?
            )?)),
            _ => Err(IdeError::NotFound),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DapStart {
    configuration_id: String,
    breakpoints: Value,
    confirmed: bool,
}
async fn start_dap(
    State(state): State<SharedState>,
    Json(body): Json<DapStart>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        Ok(json!(ide.start_debug(
            &body.configuration_id,
            body.breakpoints,
            body.confirmed
        )?))
    })
    .await
}
async fn dap_action(
    State(state): State<SharedState>,
    Path((id, action)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Error> {
    if action == "stop" {
        return execute(
            state,
            move |ide| {
                ide.stop_debug(&id)?;
                Ok(json!({"stopped":true}))
            },
            true,
        )
        .await;
    }
    operation(state, move |ide| {
        if action == "stop" {
            ide.stop_debug(&id)?;
            return Ok(json!({"stopped":true}));
        }
        let session = ide.dap(&id)?;
        match action.as_str() {
            "request" => session.request(
                body["command"].as_str().ok_or(IdeError::Invalid)?,
                body["arguments"].clone(),
                body["confirmed"] == true,
            ),
            "breakpoints" => session.set_breakpoints(
                &serde_json::from_value(body["file"].clone()).map_err(|_| IdeError::Invalid)?,
                body["breakpoints"].clone(),
            ),
            _ => Err(IdeError::NotFound),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalStart {
    root_id: String,
    profile_id: String,
    confirmed: bool,
    cols: u16,
    rows: u16,
}
async fn start_terminal(
    State(state): State<SharedState>,
    Json(body): Json<TerminalStart>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| {
        Ok(json!(ide.start_terminal(
            &body.root_id,
            &body.profile_id,
            body.confirmed,
            body.cols,
            body.rows
        )?))
    })
    .await
}
async fn terminal_action(
    State(state): State<SharedState>,
    Path((id, action)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Error> {
    if action == "stop" {
        return execute(
            state,
            move |ide| {
                ide.stop_terminal(&id)?;
                Ok(json!({"stopped":true}))
            },
            true,
        )
        .await;
    }
    operation(state, move |ide| {
        let terminal = ide.terminal(&id)?;
        match action.as_str() {
            "stop" => {
                ide.stop_terminal(&id)?;
                Ok(json!({"stopped":true}))
            }
            "rename" => {
                terminal.rename(body["name"].as_str().ok_or(IdeError::Invalid)?)?;
                Ok(json!(terminal.info()))
            }
            _ => Err(IdeError::NotFound),
        }
    })
    .await
}

async fn terminal_socket(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Error> {
    let ide = service(&state)?;
    let terminal = ide.terminal(&id)?;
    let subscription = terminal
        .process
        .subscribe()
        .map_err(|_| Error(IdeError::Limit))?;
    Ok(upgrade
        .max_message_size(65536)
        .on_upgrade(move |socket| terminal_stream(socket, state, terminal, subscription))
        .into_response())
}
async fn send(socket: &mut WebSocket, message: Message) -> bool {
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(5), socket.send(message)).await,
        Ok(Ok(()))
    )
}
async fn terminal_stream(
    mut socket: WebSocket,
    _state: SharedState,
    terminal: Arc<factory_ide::Terminal>,
    mut subscription: factory_runtime::TerminalSubscription,
) {
    if !send(&mut socket,Message::Text(json!({"type":"snapshot","bounded":true,"limit_bytes":1000000,"terminal":terminal.info()}).to_string())).await{return;}
    if !subscription.snapshot.is_empty()
        && !send(&mut socket, Message::Binary(subscription.snapshot)).await
    {
        return;
    }
    loop {
        tokio::select! {
            incoming=socket.recv()=>{
                let Some(Ok(message))=incoming else{break;};
                match message{
                    Message::Text(text)=>{
                        let Ok(command)=serde_json::from_str::<Value>(&text) else{break;};
                        let result=match command["type"].as_str(){
                            Some("input")=>{
                                let Some(data)=command["data"].as_str() else{break;};
                                if data.len()>65536{break;}
                                let data=data.to_owned();let terminal=terminal.clone();
                                match tokio::time::timeout(std::time::Duration::from_secs(5),tokio::task::spawn_blocking(move||terminal.process.write(data.as_bytes()))).await{Ok(Ok(result))=>result,_=>break}
                            },
                            Some("resize")=>terminal.process.resize(command["cols"].as_u64().unwrap_or(80).clamp(2,500) as u16,command["rows"].as_u64().unwrap_or(24).clamp(1,200) as u16),
                            _=>break,
                        };
                        if result.is_err(){break;}
                    },
                    Message::Close(_)=>break,
                    Message::Ping(bytes)=>{if !send(&mut socket,Message::Pong(bytes)).await{break;}},
                    _=>{},
                }
            },
            output=subscription.receiver.recv()=>{
                let Some(bytes)=output else{
                    let _=send(&mut socket,Message::Text(json!({"type":"closed","overflowed":subscription.receiver.overflowed(),"terminal":terminal.info()}).to_string())).await;break;
                };
                if !send(&mut socket,Message::Binary(bytes.to_vec())).await{break;}
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionInstall {
    manifest: factory_ide::extensions::ExtensionManifest,
    trusted: bool,
}
async fn install_extension(
    State(state): State<SharedState>,
    Json(body): Json<ExtensionInstall>,
) -> Result<Json<Value>, Error> {
    let notification_state = state.clone();
    operation(state, move |ide| {
        let result = ide.extensions.install(body.manifest, body.trusted);
        if result.is_err() {
            crate::notifications::record_service(
                &notification_state,
                factory_db::ServiceNotification::ExtensionFailure,
                &format!("extension:install:{}", chrono::Utc::now().timestamp() / 300),
            );
        }
        Ok(json!(result?))
    })
    .await
}
async fn extension_action(
    State(state): State<SharedState>,
    Path((id, action)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Error> {
    operation(state, move |ide| match action.as_str() {
        "enable" => Ok(json!(ide.extensions.enable(&id, true)?)),
        "disable" => Ok(json!(ide.extensions.enable(&id, false)?)),
        "settings" => {
            ide.extensions.settings(
                &id,
                serde_json::from_value(body["settings"].clone()).map_err(|_| IdeError::Invalid)?,
            )?;
            Ok(json!({"updated":true}))
        }
        "uninstall" => {
            ide.extensions.uninstall(&id)?;
            Ok(json!({"removed":true}))
        }
        _ => Err(IdeError::NotFound),
    })
    .await
}
