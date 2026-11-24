use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_projects::{
    ProjectEntry, ProjectError, ProjectId, ProjectLimits, ProjectManager, ProjectView,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Semaphore;
use tokio_stream::StreamExt;
use tower::ServiceExt;

use crate::app::{project_routes, ApiError, ApiState, SharedState};
use crate::security::{self, ApiOptions, SecurityPolicy};

pub struct ApplicationOptions {
    pub registry_path: PathBuf,
    pub allowed_roots: Vec<PathBuf>,
    pub default_root: Option<PathBuf>,
    pub dashboard_root: PathBuf,
    pub limits: ProjectLimits,
    pub api: ApiOptions,
}

pub struct FactoryApplication {
    pub projects: Arc<ProjectManager>,
    pub default_project: Option<ProjectId>,
    dashboard_root: PathBuf,
    pub(crate) security: SecurityPolicy,
    pub(crate) presence: Arc<crate::presence::BrowserPresence>,
    management: Arc<Semaphore>,
    requests: Arc<Semaphore>,
    pub(crate) source_requests: Arc<Semaphore>,
    project_routes: Router<SharedState>,
    pub(crate) workspaces: crate::workspaces::WorkspaceStore,
    pub(crate) closing: AtomicBool,
    pub(crate) admission: tokio::sync::RwLock<()>,
}

impl FactoryApplication {
    pub fn new(options: ApplicationOptions) -> Result<Self, ProjectError> {
        let projects = Arc::new(ProjectManager::new(
            &options.registry_path,
            &options.allowed_roots,
            options.limits.clone(),
        )?);
        let default_project = if let Some(root) = options.default_root {
            let entry = projects.register(&root, None)?;
            if let Err(error) = projects.open(&entry.project_id) {
                let fatal = matches!(&error, ProjectError::Registry(_))
                    || matches!(&error, ProjectError::Runtime(runtime) if matches!(runtime.as_ref(), factory_runtime::RuntimeError::ProjectAlreadyLocked { .. }));
                if fatal {
                    return Err(error);
                }
            }
            Some(entry.project_id)
        } else {
            None
        };
        Ok(Self {
            closing: AtomicBool::new(false),
            admission: tokio::sync::RwLock::new(()),
            presence: Arc::new(
                crate::presence::BrowserPresence::new(
                    options.registry_path.with_extension("presence.sqlite3"),
                    std::time::Duration::from_secs(presence_seconds(
                        "ASF_BROWSER_LEASE_SECONDS",
                        180,
                        5,
                        3600,
                    )?),
                    std::time::Duration::from_secs(presence_seconds(
                        "ASF_BROWSER_GRACE_SECONDS",
                        60,
                        0,
                        600,
                    )?),
                )
                .map_err(|_| {
                    ProjectError::Invalid("Browser presence storage unavailable".into())
                })?,
            ),
            workspaces: crate::workspaces::WorkspaceStore::new(
                options.registry_path.with_extension("workspaces.sqlite3"),
            ),
            projects,
            default_project,
            dashboard_root: options.dashboard_root,
            security: SecurityPolicy::new(options.api),
            management: Arc::new(Semaphore::new(options.limits.management_operations)),
            requests: Arc::new(Semaphore::new(64)),
            source_requests: Arc::new(Semaphore::new(8)),
            project_routes: project_routes().fallback(crate::app::api_not_found),
        })
    }
}

pub fn application_router(state: Arc<FactoryApplication>) -> Router {
    crate::presence::start(&state);
    Router::new()
        .merge(crate::presence::routes())
        .merge(crate::lifecycle::routes())
        .route(
            "/api/health",
            get(|| async { Json(json!({"status":"ok"})) }),
        )
        .route("/api/auth/session", post(issue_session))
        .route("/api/application", get(application_info))
        .route(
            "/api/application/preferences",
            get(crate::preferences::read)
                .put(crate::preferences::save)
                .layer(axum::extract::DefaultBodyLimit::max(4096)),
        )
        .route("/api/workspaces", get(crate::workspaces::list))
        .route(
            "/api/workspaces/:id",
            get(crate::workspaces::read).put(crate::workspaces::save),
        )
        .route("/api/projects", get(list_projects))
        .route("/api/projects/register", post(register_project))
        .route("/api/projects/create", post(create_project))
        .route("/api/projects/clone", post(clone_project))
        .route(
            "/api/projects/:project_id",
            get(get_project).put(rename_project).delete(remove_project),
        )
        .route("/api/projects/:project_id/open", post(open_project))
        .route("/api/projects/:project_id/close", post(close_project))
        .route(
            "/api/projects/:project_id/initialize",
            post(initialize_project),
        )
        .route("/api/projects/:project_id/relink", post(relink_project))
        .fallback(dispatch_project)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_application_security,
        ))
        .with_state(state)
}

fn presence_seconds(name: &str, default: u64, min: u64, max: u64) -> Result<u64, ProjectError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|v| (*v >= min) && (*v <= max))
            .ok_or_else(|| ProjectError::Invalid(format!("{name} must be {min}..={max}"))),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(_) => Err(ProjectError::Invalid(format!("Invalid {name}"))),
    }
}

pub async fn serve_application(
    state: Arc<FactoryApplication>,
    listener: std::net::TcpListener,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, application_router(state)).await
}

pub async fn serve_application_with_shutdown(
    state: Arc<FactoryApplication>,
    listener: std::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let router = application_router(state.clone());
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let early_result = tokio::select! {
        result = &mut server => Some(result),
        _ = shutdown => None,
    };
    state.closing.store(true, Ordering::Release);
    state.projects.close_admission();
    let projects = state.projects.clone();
    let cancellation = tokio::task::spawn_blocking(move || projects.begin_shutdown()).await;
    let admission =
        tokio::time::timeout(std::time::Duration::from_secs(15), state.admission.write()).await;
    let cleanup = if let Ok(_admission) = admission {
        let projects = state.projects.clone();
        tokio::task::spawn_blocking(move || {
            projects.finish_shutdown(std::time::Duration::from_secs(45))
        })
        .await
        .map_err(std::io::Error::other)
        .and_then(|result| result.map_err(std::io::Error::other))
    } else {
        Err(std::io::Error::other(
            "Accepted requests did not drain; checkpoints deferred and recovery required",
        ))
    };
    let _ = stop.send(());
    let result = if let Some(result) = early_result {
        result.map_err(std::io::Error::other)?
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(2), &mut server).await {
            Ok(result) => result.map_err(std::io::Error::other)?,
            Err(_) => {
                server.abort();
                let _ = server.await;
                Ok(())
            }
        }
    };
    cancellation
        .map_err(std::io::Error::other)?
        .map_err(std::io::Error::other)?;
    cleanup?;
    result
}

async fn enforce_application_security(
    State(state): State<Arc<FactoryApplication>>,
    request: Request,
    next: Next,
) -> Response {
    let _admission = state.admission.read().await;
    if state.closing.load(Ordering::Acquire) {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"code":"application_shutting_down","error":"Application is shutting down; no operation was accepted"}))).into_response();
    }
    security::enforce_policy(&state.security, request, next).await
}

async fn issue_session(
    State(state): State<Arc<FactoryApplication>>,
    headers: HeaderMap,
) -> Response {
    security::issue_policy_session(&state.security, headers)
}

async fn application_info(State(state): State<Arc<FactoryApplication>>) -> Json<serde_json::Value> {
    Json(
        json!({"default_project_id":state.default_project,"authorized_roots":state.projects.policy.roots(),"limits":state.projects.limits,"project_request_limit":64}),
    )
}

struct ManagementError(ProjectError);
impl From<ProjectError> for ManagementError {
    fn from(value: ProjectError) -> Self {
        Self(value)
    }
}
impl IntoResponse for ManagementError {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            ProjectError::InvalidId | ProjectError::Invalid(_) => {
                (StatusCode::BAD_REQUEST, "invalid_project")
            }
            ProjectError::NotFound => (StatusCode::NOT_FOUND, "unknown_project"),
            ProjectError::Forbidden => (StatusCode::FORBIDDEN, "project_path_forbidden"),
            ProjectError::Conflict(_) | ProjectError::Closed => {
                (StatusCode::CONFLICT, "project_conflict")
            }
            ProjectError::Limit(_) => (StatusCode::TOO_MANY_REQUESTS, "project_limit"),
            ProjectError::Unavailable(_)
            | ProjectError::Io(_)
            | ProjectError::Runtime(_)
            | ProjectError::Factory(_) => (StatusCode::CONFLICT, "project_unavailable"),
            ProjectError::Registry(_) => (StatusCode::SERVICE_UNAVAILABLE, "registry_unavailable"),
        };
        (
            status,
            Json(json!({"error":self.0.to_string(),"code":code})),
        )
            .into_response()
    }
}

async fn management<T: Send + 'static>(
    state: Arc<FactoryApplication>,
    action: impl FnOnce(&ProjectManager) -> Result<T, ProjectError> + Send + 'static,
) -> Result<T, ManagementError> {
    let permit = state
        .management
        .clone()
        .try_acquire_owned()
        .map_err(|_| ProjectError::Limit("management operations"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        action(&state.projects)
    })
    .await
    .map_err(|_| ProjectError::Unavailable("project operation stopped unexpectedly".into()))?
    .map_err(Into::into)
}

async fn list_projects(
    State(state): State<Arc<FactoryApplication>>,
) -> Result<Json<Vec<ProjectView>>, ManagementError> {
    Ok(Json(management(state, |projects| projects.list()).await?))
}
async fn get_project(
    State(state): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectView>, ManagementError> {
    let id = ProjectId::parse(&id)?;
    Ok(Json(
        management(state, move |projects| projects.get(&id)).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterRequest {
    path: PathBuf,
    display_name: Option<String>,
    #[serde(default)]
    trust_project: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    destination: PathBuf,
    display_name: String,
    #[serde(default)]
    trust_project: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloneRequest {
    remote: String,
    destination: PathBuf,
    display_name: String,
    #[serde(default)]
    initialize: bool,
    #[serde(default)]
    allow_network: bool,
    #[serde(default)]
    trust_project: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustRequest {
    #[serde(default)]
    trust_project: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameRequest {
    display_name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelinkRequest {
    path: PathBuf,
    #[serde(default)]
    accept_identity_change: bool,
    #[serde(default)]
    trust_project: bool,
}

fn trust(value: bool) -> Result<(), ProjectError> {
    if value {
        Ok(())
    } else {
        Err(ProjectError::Invalid("explicit trust_project acknowledgement is required before loading local repository tools".into()))
    }
}

async fn register_project(
    State(state): State<Arc<FactoryApplication>>,
    Json(body): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<ProjectEntry>), ManagementError> {
    trust(body.trust_project)?;
    Ok((
        StatusCode::CREATED,
        Json(
            management(state, move |projects| {
                projects.register(&body.path, body.display_name.as_deref())
            })
            .await?,
        ),
    ))
}
async fn create_project(
    State(state): State<Arc<FactoryApplication>>,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<ProjectEntry>), ManagementError> {
    trust(body.trust_project)?;
    Ok((
        StatusCode::CREATED,
        Json(
            management(state, move |projects| {
                projects.create(&body.destination, &body.display_name)
            })
            .await?,
        ),
    ))
}
async fn clone_project(
    State(state): State<Arc<FactoryApplication>>,
    Json(body): Json<CloneRequest>,
) -> Result<(StatusCode, Json<ProjectEntry>), ManagementError> {
    trust(body.trust_project)?;
    Ok((
        StatusCode::CREATED,
        Json(
            management(state, move |projects| {
                projects.clone_project(
                    &body.remote,
                    &body.destination,
                    &body.display_name,
                    body.initialize,
                    body.allow_network,
                )
            })
            .await?,
        ),
    ))
}
async fn open_project(
    State(state): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectView>, ManagementError> {
    let id = ProjectId::parse(&id)?;
    Ok(Json(
        management(state, move |projects| {
            projects.open(&id)?;
            projects.get(&id)
        })
        .await?,
    ))
}
async fn close_project(
    State(state): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectView>, ManagementError> {
    let id = ProjectId::parse(&id)?;
    Ok(Json(
        management(state, move |projects| {
            projects.close(&id)?;
            projects.get(&id)
        })
        .await?,
    ))
}
async fn rename_project(
    State(state): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> Result<Json<ProjectView>, ManagementError> {
    let id = ProjectId::parse(&id)?;
    Ok(Json(
        management(state, move |projects| {
            projects.rename(&id, &body.display_name)
        })
        .await?,
    ))
}
async fn remove_project(
    State(state): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ManagementError> {
    let id = ProjectId::parse(&id)?;
    management(state, move |projects| projects.remove(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn initialize_project(
    State(state): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
    Json(body): Json<TrustRequest>,
) -> Result<Json<ProjectView>, ManagementError> {
    trust(body.trust_project)?;
    let id = ProjectId::parse(&id)?;
    Ok(Json(
        management(state, move |projects| projects.initialize(&id)).await?,
    ))
}
async fn relink_project(
    State(state): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
    Json(body): Json<RelinkRequest>,
) -> Result<Json<ProjectView>, ManagementError> {
    trust(body.trust_project)?;
    let id = ProjectId::parse(&id)?;
    Ok(Json(
        management(state, move |projects| {
            projects.relink(&id, &body.path, body.accept_identity_change)
        })
        .await?,
    ))
}

async fn dispatch_project(
    State(state): State<Arc<FactoryApplication>>,
    mut request: Request,
) -> Response {
    let path = request.uri().path().to_owned();
    if !path.starts_with("/api/") {
        return crate::dashboard::router(&state.dashboard_root)
            .oneshot(request)
            .await
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let (project_id, child_path) = if let Some(scoped) = path.strip_prefix("/api/projects/") {
        let Some((id, tail)) = scoped.split_once('/') else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let id = match ProjectId::parse(id) {
            Ok(id) => id,
            Err(error) => return ManagementError(error).into_response(),
        };
        (id, format!("/api/{tail}"))
    } else {
        let Some(id) = &state.default_project else {
            return (StatusCode::CONFLICT, Json(json!({"error":"no default project is configured; use an explicit project route","code":"no_default_project"}))).into_response();
        };
        (id.clone(), path)
    };
    let permit = match state.requests.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return ManagementError(ProjectError::Limit("project requests")).into_response(),
    };
    let manager = state.projects.clone();
    let lookup = project_id.clone();
    let context = match tokio::task::spawn_blocking(move || manager.context(&lookup)).await {
        Ok(Ok(context)) => context,
        Ok(Err(error)) => return ManagementError(error).into_response(),
        Err(_) => {
            return ManagementError(ProjectError::Unavailable("project lookup failed".into()))
                .into_response()
        }
    };
    let child_uri = match request.uri().query() {
        Some(query) => format!("{child_path}?{query}"),
        None => child_path,
    };
    let Ok(uri) = child_uri.parse() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *request.uri_mut() = uri;
    if let Some(lease) = request
        .headers()
        .get("x-asf-lease")
        .and_then(|h| h.to_str().ok())
    {
        let Some(owner) = state.security.authenticated_identity(request.headers()) else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        let bindings = state.presence.client_bindings(&owner, lease);
        let Some(bindings) = bindings else {
            return (StatusCode::FORBIDDEN,Json(json!({"error":"Browser lease expired or belongs to another client","code":"browser_lease_refused"}))).into_response();
        };
        request
            .extensions_mut()
            .insert(crate::presence::ValidatedBrowserBindings(bindings));
    }
    let project_state = Arc::new(ApiState {
        db: context.db.clone(),
        root: context.root.clone(),
        runtime: context.runtime.clone(),
        security: state.security.clone(),
        _context: Some(context.clone()),
        source_requests: state.source_requests.clone(),
    });
    let response = state
        .project_routes
        .clone()
        .with_state(project_state)
        .oneshot(request)
        .await
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        "x-factory-project",
        HeaderValue::from_str(project_id.as_str()).expect("validated project id"),
    );
    let stream = body.into_data_stream().map(move |item| {
        let _lease = (&context, &permit);
        item
    });
    Response::from_parts(parts, Body::from_stream(stream))
}

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    q: String,
}

pub(crate) async fn search_context(
    State(state): State<SharedState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if query.q.len() > 512 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "query too long"));
    }
    tokio::task::spawn_blocking(move || {
        let config = factory_core::Config::load(&state.root)?;
        let mut engine = factory_context::ContextEngine::new(&state.root, &state.root.join(".factory"), config.context);
        let hits = engine.search(&query.q, 30).map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, error.to_string()))?;
        Ok(Json(json!({"hits":hits.into_iter().map(|hit| json!({"path":hit.path,"line":hit.line,"snippet":hit.snippet})).collect::<Vec<_>>()})))
    }).await.map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "context lookup failed"))?
}
