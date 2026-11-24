use crate::{application::FactoryApplication, presence::SessionBinding};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use factory_types::{MissionLifecycle, MissionLifecycleState};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

struct Error(StatusCode, String);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":self.1,"code":"lifecycle_refused"})),
        )
            .into_response()
    }
}
fn failure(error: impl std::fmt::Display) -> Error {
    Error(StatusCode::CONFLICT, error.to_string())
}
fn context(
    app: &FactoryApplication,
    project: &str,
) -> Result<Arc<factory_projects::ProjectContext>, Error> {
    let id = factory_projects::ProjectId::parse(project).map_err(failure)?;
    let c = app.projects.context(&id).map_err(failure)?;
    c.validate().map_err(failure)?;
    Ok(c)
}
fn authorize(
    app: &FactoryApplication,
    headers: &HeaderMap,
    binding: &SessionBinding,
) -> Result<(), Error> {
    let owner = app.security.authenticated_identity(headers).ok_or(Error(
        StatusCode::UNAUTHORIZED,
        "Authentication required".into(),
    ))?;
    let lease = headers
        .get("x-asf-lease")
        .and_then(|h| h.to_str().ok())
        .ok_or(Error(
            StatusCode::FORBIDDEN,
            "A current browser lease for this workspace session is required".into(),
        ))?;
    if !app.presence.owns_binding(&owner, lease, binding) {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "Browser lease does not own this workspace session".into(),
        ));
    }
    Ok(())
}

pub(crate) fn routes() -> Router<Arc<FactoryApplication>> {
    Router::new()
        .route(
            "/api/projects/:project/workspace-sessions/:session/lifecycle",
            get(session),
        )
        .route(
            "/api/projects/:project/workspace-sessions/:session/close",
            post(close),
        )
        .route(
            "/api/projects/:project/workspace-sessions/:session/reopen",
            post(reopen),
        )
        .route("/api/projects/:project/runs/:run/lifecycle", get(mission))
        .route(
            "/api/projects/:project/runs/:run/checkpoints",
            get(checkpoints),
        )
        .route("/api/projects/:project/runs/:run/suspend", post(suspend))
        .route("/api/projects/:project/runs/:run/resume", post(resume))
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .layer(axum::middleware::from_fn(crate::presence::limit))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Command {
    expected_generation: i64,
    idempotency_key: String,
}

async fn session(
    State(app): State<Arc<FactoryApplication>>,
    Path((project, session)): Path<(String, String)>,
) -> Result<Json<Value>, Error> {
    tokio::task::spawn_blocking(move || {
        let c = context(&app, &project)?;
        let view =
            c.db.lock()
                .map_err(failure)?
                .workspace_session_lifecycle(&project, &session)
                .map_err(failure)?;
        Ok(Json(json!(view)))
    })
    .await
    .map_err(failure)?
}

async fn change_session(
    app: Arc<FactoryApplication>,
    headers: HeaderMap,
    project: String,
    session: String,
    command: Command,
    closed: bool,
) -> Result<Json<Value>, Error> {
    authorize(
        &app,
        &headers,
        &SessionBinding {
            project_id: project.clone(),
            session_id: session.clone(),
        },
    )?;
    tokio::task::spawn_blocking(move || {
        let c = context(&app, &project)?;
        let view = c
            .db
            .lock()
            .map_err(failure)?
            .set_workspace_session_closed(&project, &session, command.expected_generation, closed)
            .map_err(failure)?;
        if closed {
            for mission in &view.missions {
                if matches!(
                    mission.state,
                    MissionLifecycleState::Active
                        | MissionLifecycleState::SuspendRequested
                        | MissionLifecycleState::Quiescing
                        | MissionLifecycleState::Reconciling
                ) {
                    c.runtime
                        .suspend_mission(
                            mission.run_id,
                            mission.generation,
                            &format!("close-{}-{}", view.generation, mission.run_id),
                            "explicit_session_close",
                        )
                        .map_err(failure)?;
                }
            }
        }
        let result =
            c.db.lock()
                .map_err(failure)?
                .workspace_session_lifecycle(&project, &session)
                .map_err(failure)?;
        Ok(Json(json!(result)))
    })
    .await
    .map_err(failure)?
}
async fn close(
    State(app): State<Arc<FactoryApplication>>,
    Path((p, s)): Path<(String, String)>,
    headers: HeaderMap,
    Json(c): Json<Command>,
) -> Result<Json<Value>, Error> {
    change_session(app, headers, p, s, c, true).await
}
async fn reopen(
    State(app): State<Arc<FactoryApplication>>,
    Path((p, s)): Path<(String, String)>,
    headers: HeaderMap,
    Json(c): Json<Command>,
) -> Result<Json<Value>, Error> {
    change_session(app, headers, p, s, c, false).await
}

fn owned_mission(
    c: &factory_projects::ProjectContext,
    project: &str,
    run: i64,
) -> Result<Option<MissionLifecycle>, Error> {
    let life =
        c.db.lock()
            .map_err(failure)?
            .mission_lifecycle(run)
            .map_err(failure)?;
    if life.as_ref().is_some_and(|l| l.project_id != project) {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "Mission project mismatch".into(),
        ));
    }
    Ok(life)
}
async fn mission(
    State(app): State<Arc<FactoryApplication>>,
    Path((p, r)): Path<(String, i64)>,
) -> Result<Json<Value>, Error> {
    tokio::task::spawn_blocking(move||{let c=context(&app,&p)?;let life=owned_mission(&c,&p,r)?;let processes=c.db.lock().map_err(failure)?.lifecycle_processes(r).map_err(failure)?.into_iter().rev().take(32).map(|(id,status,metadata)|json!({"sessionId":id,"status":status,"metadata":metadata})).collect::<Vec<_>>();Ok(Json(json!({"lifecycle":life,"resumePolicy":"manual","legacy":life.is_none(),"processes":processes})))}).await.map_err(failure)?
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    before: Option<i64>,
}
async fn checkpoints(
    State(app): State<Arc<FactoryApplication>>,
    Path((p, r)): Path<(String, i64)>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, Error> {
    tokio::task::spawn_blocking(move || {
        let c = context(&app, &p)?;
        owned_mission(&c, &p, r)?.ok_or_else(|| failure("Not a browser-managed mission"))?;
        let rows =
            c.db.lock()
                .map_err(failure)?
                .mission_checkpoints(r, page.before)
                .map_err(failure)?;
        Ok(Json(json!(rows)))
    })
    .await
    .map_err(failure)?
}
async fn change_mission(
    app: Arc<FactoryApplication>,
    headers: HeaderMap,
    p: String,
    r: i64,
    command: Command,
    resume: bool,
) -> Result<Json<Value>, Error> {
    tokio::task::spawn_blocking(move || {
        let c = context(&app, &p)?;
        let life = owned_mission(&c, &p, r)?
            .ok_or_else(|| failure("Legacy workflows retain their original lifecycle"))?;
        authorize(
            &app,
            &headers,
            &SessionBinding {
                project_id: p,
                session_id: life.workspace_session_id,
            },
        )?;
        let result = if resume {
            c.runtime
                .resume_mission(r, command.expected_generation, &command.idempotency_key)
        } else {
            c.runtime.suspend_mission(
                r,
                command.expected_generation,
                &command.idempotency_key,
                "user_requested_suspend",
            )
        }
        .map_err(failure)?;
        Ok(Json(json!(result)))
    })
    .await
    .map_err(failure)?
}
async fn suspend(
    State(app): State<Arc<FactoryApplication>>,
    Path((p, r)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(c): Json<Command>,
) -> Result<Json<Value>, Error> {
    change_mission(app, headers, p, r, c, false).await
}
async fn resume(
    State(app): State<Arc<FactoryApplication>>,
    Path((p, r)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(c): Json<Command>,
) -> Result<Json<Value>, Error> {
    change_mission(app, headers, p, r, c, true).await
}
