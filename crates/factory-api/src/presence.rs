use crate::application::FactoryApplication;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use rand::RngCore;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SessionBinding {
    pub project_id: String,
    pub session_id: String,
}

#[derive(Clone)]
pub(crate) struct ValidatedBrowserBindings(pub Vec<SessionBinding>);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientLease {
    pub lease_id: String,
    pub client_instance_id: String,
    pub generation: u64,
    pub sequence: u64,
    pub issued_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub heartbeat_millis: u64,
    pub grace_millis: u64,
    pub bindings: Vec<SessionBinding>,
}

#[derive(Clone)]
struct OwnedLease {
    owner: String,
    view: ClientLease,
    expires: Instant,
    expired: bool,
}

pub(crate) struct BrowserPresence {
    path: PathBuf,
    leases: Mutex<HashMap<String, OwnedLease>>,
    absent: Mutex<HashMap<SessionBinding, Instant>>,
    pub(crate) started: std::sync::atomic::AtomicBool,
    duration: Duration,
    grace: Duration,
}

#[derive(Debug)]
pub(crate) struct PresenceError(pub StatusCode, pub &'static str);
impl IntoResponse for PresenceError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1,"code":self.1}))).into_response()
    }
}
fn storage(_: impl std::fmt::Display) -> PresenceError {
    PresenceError(
        StatusCode::SERVICE_UNAVAILABLE,
        "presence_storage_unavailable",
    )
}
fn invalid() -> PresenceError {
    PresenceError(StatusCode::BAD_REQUEST, "invalid_browser_client")
}

impl BrowserPresence {
    pub(crate) fn new(
        path: PathBuf,
        duration: Duration,
        grace: Duration,
    ) -> Result<Self, PresenceError> {
        if !(5..=3600).contains(&duration.as_secs()) || grace.as_secs() > 600 {
            return Err(invalid());
        }
        let result = Self {
            path,
            leases: Mutex::new(HashMap::new()),
            absent: Mutex::new(HashMap::new()),
            started: std::sync::atomic::AtomicBool::new(false),
            duration,
            grace,
        };
        let conn = result.connect()?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        if version == 0 {
            let tables: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if tables != 0 {
                return Err(storage("Unknown presence schema"));
            }
            conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE browser_clients(lease_id TEXT PRIMARY KEY,owner_hash TEXT NOT NULL,payload TEXT NOT NULL,status TEXT NOT NULL,updated_at TEXT NOT NULL); PRAGMA user_version=1; COMMIT;").map_err(storage)?;
        } else if version != 1 {
            return Err(storage("Unsupported presence schema"));
        }
        conn.execute(
            "UPDATE browser_clients SET status='server_restarted' WHERE status='active'",
            [],
        )
        .map_err(storage)?;
        Ok(result)
    }

    fn connect(&self) -> Result<Connection, PresenceError> {
        let conn = Connection::open(&self.path).map_err(storage)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(storage)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(storage)?;
        Ok(conn)
    }

    fn persist(&self, lease: &OwnedLease, status: &str) -> Result<(), PresenceError> {
        let conn = self.connect()?;
        conn.execute("INSERT INTO browser_clients(lease_id,owner_hash,payload,status,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(lease_id) DO UPDATE SET payload=excluded.payload,status=excluded.status,updated_at=excluded.updated_at",params![lease.view.lease_id,lease.owner,serde_json::to_string(&lease.view).map_err(storage)?,status,chrono::Utc::now().to_rfc3339()]).map_err(storage)?;
        Ok(())
    }

    fn register(
        &self,
        owner: String,
        client: String,
        bindings: Vec<SessionBinding>,
    ) -> Result<ClientLease, PresenceError> {
        if client.len() != 35
            || !client.starts_with("bc_")
            || !client[3..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(invalid());
        }
        let mut leases = self.leases.lock().map_err(storage)?;
        if let Some(existing) = leases.values().find(|l| {
            l.owner == owner
                && l.view.client_instance_id == client
                && !l.expired
                && l.expires > Instant::now()
        }) {
            return Ok(existing.view.clone());
        }
        let total: i64 = self
            .connect()?
            .query_row("SELECT count(*) FROM browser_clients", [], |r| r.get(0))
            .map_err(storage)?;
        if leases.len() >= 4096
            || total >= 10_000
            || leases.values().filter(|l| !l.expired).count() >= 256
        {
            return Err(PresenceError(
                StatusCode::TOO_MANY_REQUESTS,
                "browser_client_limit",
            ));
        }
        let mut random = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let id = format!(
            "bl_{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let lease = OwnedLease {
            owner,
            view: ClientLease {
                lease_id: id.clone(),
                client_instance_id: client,
                generation: 1,
                sequence: 0,
                issued_at: chrono::Utc::now().to_rfc3339(),
                last_seen_at: chrono::Utc::now().to_rfc3339(),
                expires_at: (chrono::Utc::now()
                    + chrono::Duration::from_std(self.duration).map_err(storage)?)
                .to_rfc3339(),
                heartbeat_millis: (self.duration.as_millis() as u64 / 6).clamp(1000, 30_000),
                grace_millis: self.grace.as_millis() as u64,
                bindings,
            },
            expires: Instant::now() + self.duration,
            expired: false,
        };
        self.persist(&lease, "active")?;
        let view = lease.view.clone();
        leases.insert(id, lease);
        Ok(view)
    }

    fn renew(&self, owner: &str, id: &str, body: Renew) -> Result<ClientLease, PresenceError> {
        let mut leases = self.leases.lock().map_err(storage)?;
        let lease = leases.get_mut(id).ok_or(PresenceError(
            StatusCode::NOT_FOUND,
            "unknown_browser_lease",
        ))?;
        if lease.owner != owner || lease.view.client_instance_id != body.client_instance_id {
            return Err(PresenceError(
                StatusCode::FORBIDDEN,
                "browser_lease_owner_mismatch",
            ));
        }
        if lease.expired
            || lease.expires <= Instant::now()
            || lease.view.generation != body.generation
        {
            return Err(PresenceError(StatusCode::CONFLICT, "browser_lease_expired"));
        }
        if body.sequence < lease.view.sequence {
            return Err(PresenceError(
                StatusCode::CONFLICT,
                "stale_browser_heartbeat",
            ));
        }
        if body.sequence == lease.view.sequence {
            return Ok(lease.view.clone());
        }
        if body.sequence != lease.view.sequence + 1 {
            return Err(PresenceError(
                StatusCode::CONFLICT,
                "invalid_browser_sequence",
            ));
        }
        let mut candidate = lease.clone();
        candidate.view.sequence = body.sequence;
        candidate.view.last_seen_at = chrono::Utc::now().to_rfc3339();
        candidate.view.bindings = body.bindings;
        candidate.expires = Instant::now() + self.duration;
        candidate.view.expires_at = (chrono::Utc::now()
            + chrono::Duration::from_std(self.duration).map_err(storage)?)
        .to_rfc3339();
        self.persist(&candidate, "active")?;
        *lease = candidate;
        Ok(lease.view.clone())
    }

    pub(crate) fn owns_binding(&self, owner: &str, lease: &str, binding: &SessionBinding) -> bool {
        self.leases.lock().is_ok_and(|leases| {
            leases.get(lease).is_some_and(|l| {
                l.owner == owner
                    && !l.expired
                    && l.expires > Instant::now()
                    && l.view.bindings.contains(binding)
            })
        })
    }

    pub(crate) fn client_bindings(&self, owner: &str, lease: &str) -> Option<Vec<SessionBinding>> {
        self.leases
            .lock()
            .ok()?
            .get(lease)
            .filter(|l| l.owner == owner && !l.expired && l.expires > Instant::now())
            .map(|l| l.view.bindings.clone())
    }

    fn release(&self, owner: &str, id: &str, body: Renew) -> Result<(), PresenceError> {
        let mut leases = self.leases.lock().map_err(storage)?;
        let lease = leases.get_mut(id).ok_or(PresenceError(
            StatusCode::NOT_FOUND,
            "unknown_browser_lease",
        ))?;
        if lease.owner != owner
            || lease.view.client_instance_id != body.client_instance_id
            || lease.view.generation != body.generation
        {
            return Err(PresenceError(
                StatusCode::FORBIDDEN,
                "browser_lease_owner_mismatch",
            ));
        }
        let mut candidate = lease.clone();
        candidate.expired = true;
        candidate.expires = Instant::now();
        self.persist(&candidate, "released")?;
        *lease = candidate;
        Ok(())
    }

    pub(crate) fn tick(&self, application: &FactoryApplication) -> Result<(), PresenceError> {
        let now = Instant::now();
        let mut leases = self.leases.lock().map_err(storage)?;
        for lease in leases
            .values_mut()
            .filter(|l| !l.expired && l.expires <= now)
        {
            lease.expired = true;
            self.persist(lease, "expired")?;
        }
        let present = leases
            .values()
            .filter(|l| !l.expired && l.expires > now)
            .flat_map(|l| l.view.bindings.iter().cloned())
            .collect::<std::collections::HashSet<_>>();
        leases.retain(|_, l| {
            !l.expired || now.saturating_duration_since(l.expires) < Duration::from_secs(3600)
        });
        drop(leases);
        let contexts = application.projects.open_contexts();
        let mut absent = self.absent.lock().map_err(storage)?;
        let mut considered = std::collections::HashSet::new();
        for context in contexts {
            if context.validate().is_err() {
                continue;
            }
            let Ok(db) = factory_db::FactoryDb::open(&context.root.join(".factory/db.sqlite3"))
            else {
                continue;
            };
            let Ok(runs) = db.lifecycle_runs() else {
                continue;
            };
            for run in runs {
                let Ok(Some(life)) = db.mission_lifecycle(run) else {
                    continue;
                };
                if life.state != factory_types::MissionLifecycleState::Active {
                    continue;
                }
                let binding = SessionBinding {
                    project_id: context.project_id.to_string(),
                    session_id: life.workspace_session_id.clone(),
                };
                let Ok(session) =
                    db.workspace_session_lifecycle(&binding.project_id, &binding.session_id)
                else {
                    continue;
                };
                if session.closed {
                    context
                        .runtime
                        .suspend_mission(
                            run,
                            life.generation,
                            &format!("closed-session-{}", life.generation),
                            "explicit_session_close",
                        )
                        .ok();
                    continue;
                }
                considered.insert(binding.clone());
                if present.contains(&binding) {
                    absent.remove(&binding);
                    continue;
                }
                let since = absent.entry(binding.clone()).or_insert(now);
                if now.saturating_duration_since(*since) >= self.grace {
                    let leases = self.leases.lock().map_err(storage)?;
                    if leases.values().any(|l| {
                        !l.expired
                            && l.expires > Instant::now()
                            && l.view.bindings.contains(&binding)
                    }) {
                        absent.remove(&binding);
                        continue;
                    }
                    let requested = db.request_mission_suspend(
                        run,
                        life.generation,
                        &format!("absence-{}", life.generation),
                        "last_browser_client_expired",
                    );
                    drop(leases);
                    if requested.is_err() {
                        continue;
                    }
                    context
                        .runtime
                        .suspend_mission(
                            run,
                            life.generation,
                            &format!("absence-{}", life.generation),
                            "last_browser_client_expired",
                        )
                        .ok();
                }
            }
        }
        absent.retain(|binding, _| considered.contains(binding));
        self.connect()?.execute("DELETE FROM browser_clients WHERE status<>'active' AND julianday(updated_at)<julianday('now','-7 days')",[]).map_err(storage)?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Register {
    client_instance_id: String,
    bindings: Vec<SessionBinding>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Renew {
    client_instance_id: String,
    generation: u64,
    sequence: u64,
    bindings: Vec<SessionBinding>,
}

fn owner(app: &FactoryApplication, headers: &HeaderMap) -> Result<String, PresenceError> {
    app.security
        .authenticated_identity(headers)
        .ok_or(PresenceError(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        ))
}

fn validate_bindings(
    app: &FactoryApplication,
    bindings: &[SessionBinding],
) -> Result<(), PresenceError> {
    if bindings.len() > 256 {
        return Err(invalid());
    }
    for binding in bindings {
        let id = factory_projects::ProjectId::parse(&binding.project_id).map_err(|_| invalid())?;
        let context = app
            .projects
            .context(&id)
            .map_err(|_| PresenceError(StatusCode::CONFLICT, "project_unavailable"))?;
        context
            .db
            .lock()
            .map_err(storage)?
            .register_workspace_session(&binding.project_id, &binding.session_id)
            .map_err(|_| invalid())?;
    }
    Ok(())
}

pub(crate) fn routes() -> Router<Arc<FactoryApplication>> {
    Router::new()
        .route("/api/browser-clients/register", post(register))
        .route("/api/browser-clients/:id/heartbeat", post(heartbeat))
        .route("/api/browser-clients/:id/release", post(release))
        .layer(axum::extract::DefaultBodyLimit::max(32 * 1024))
        .layer(axum::middleware::from_fn(limit))
}

static REQUESTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);
pub(crate) async fn limit(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Ok(_permit) = REQUESTS.try_acquire() else {
        return PresenceError(StatusCode::TOO_MANY_REQUESTS, "lifecycle_busy").into_response();
    };
    next.run(request).await
}

async fn register(
    State(app): State<Arc<FactoryApplication>>,
    headers: HeaderMap,
    Json(body): Json<Register>,
) -> Result<Json<ClientLease>, PresenceError> {
    let owner = owner(&app, &headers)?;
    tokio::task::spawn_blocking(move || {
        validate_bindings(&app, &body.bindings)?;
        app.presence
            .register(owner, body.client_instance_id, body.bindings)
            .map(Json)
    })
    .await
    .map_err(storage)?
}
async fn heartbeat(
    State(app): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Renew>,
) -> Result<Json<ClientLease>, PresenceError> {
    let owner = owner(&app, &headers)?;
    tokio::task::spawn_blocking(move || {
        validate_bindings(&app, &body.bindings)?;
        app.presence.renew(&owner, &id, body).map(Json)
    })
    .await
    .map_err(storage)?
}
async fn release(
    State(app): State<Arc<FactoryApplication>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Renew>,
) -> Result<StatusCode, PresenceError> {
    let owner = owner(&app, &headers)?;
    tokio::task::spawn_blocking(move || {
        app.presence
            .release(&owner, &id, body)
            .map(|_| StatusCode::OK)
    })
    .await
    .map_err(storage)?
}

pub(crate) fn start(app: &Arc<FactoryApplication>) {
    if tokio::runtime::Handle::try_current().is_err()
        || app
            .presence
            .started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return;
    }
    let weak = Arc::downgrade(app);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(app) = weak.upgrade() else {
                return;
            };
            let _admission = app.admission.read().await;
            if app.closing.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let tick = app.clone();
            let _ = tokio::task::spawn_blocking(move || tick.presence.tick(&tick)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, BrowserPresence) {
        let dir = tempfile::tempdir().unwrap();
        let presence = BrowserPresence::new(
            dir.path().join("presence.sqlite3"),
            Duration::from_secs(5),
            Duration::from_secs(1),
        )
        .unwrap();
        (dir, presence)
    }
    fn client(n: u64) -> String {
        format!("bc_{n:032x}")
    }
    fn renew(lease: &ClientLease, sequence: u64) -> Renew {
        Renew {
            client_instance_id: lease.client_instance_id.clone(),
            generation: lease.generation,
            sequence,
            bindings: lease.bindings.clone(),
        }
    }
    #[test]
    fn registration_and_ordered_heartbeat_are_authenticated_and_bounded() {
        let (_dir, p) = store();
        let lease = p.register("owner".into(), client(1), vec![]).unwrap();
        assert!(p.renew("other", &lease.lease_id, renew(&lease, 1)).is_err());
        assert!(p
            .renew(
                "owner",
                &lease.lease_id,
                Renew {
                    client_instance_id: client(2),
                    ..renew(&lease, 1)
                }
            )
            .is_err());
        assert_eq!(
            p.renew("owner", &lease.lease_id, renew(&lease, 1))
                .unwrap()
                .sequence,
            1
        );
        assert!(p.renew("owner", &lease.lease_id, renew(&lease, 3)).is_err());
        assert!(p.renew("owner", &lease.lease_id, renew(&lease, 0)).is_err());
    }
    #[test]
    fn duplicated_heartbeat_is_idempotent_and_does_not_extend_presence() {
        let (_dir, p) = store();
        let lease = p.register("owner".into(), client(1), vec![]).unwrap();
        let first = p.renew("owner", &lease.lease_id, renew(&lease, 1)).unwrap();
        let expires = p.leases.lock().unwrap()[&lease.lease_id].expires;
        let repeated = p.renew("owner", &lease.lease_id, renew(&lease, 1)).unwrap();
        assert_eq!(first.expires_at, repeated.expires_at);
        assert_eq!(p.leases.lock().unwrap()[&lease.lease_id].expires, expires);
    }
    #[test]
    fn expired_and_revoked_leases_cannot_be_reactivated_by_stale_heartbeat() {
        let (_dir, p) = store();
        let lease = p.register("owner".into(), client(1), vec![]).unwrap();
        p.leases
            .lock()
            .unwrap()
            .get_mut(&lease.lease_id)
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        assert!(p.renew("owner", &lease.lease_id, renew(&lease, 1)).is_err());
        let new = p.register("owner".into(), client(1), vec![]).unwrap();
        assert_ne!(new.lease_id, lease.lease_id);
        p.release("owner", &new.lease_id, renew(&new, 0)).unwrap();
        assert!(p.renew("owner", &new.lease_id, renew(&new, 1)).is_err());
    }
    #[test]
    fn separate_windows_have_independent_leases_and_exact_session_bindings() {
        let (_dir, p) = store();
        let a = SessionBinding {
            project_id: "A".into(),
            session_id: "X".into(),
        };
        let b = SessionBinding {
            project_id: "B".into(),
            session_id: "X".into(),
        };
        let l1 = p
            .register("owner".into(), client(1), vec![a.clone()])
            .unwrap();
        let l2 = p
            .register("owner".into(), client(2), vec![b.clone()])
            .unwrap();
        p.release("owner", &l1.lease_id, renew(&l1, 0)).unwrap();
        assert!(!p.owns_binding("owner", &l1.lease_id, &a));
        assert!(p.owns_binding("owner", &l2.lease_id, &b));
        assert!(!p.owns_binding("owner", &l2.lease_id, &a));
    }
    #[test]
    fn restart_invalidates_prior_presence_without_reusing_authentication_or_lease() {
        let (dir, p) = store();
        let l = p.register("owner".into(), client(1), vec![]).unwrap();
        drop(p);
        let p = BrowserPresence::new(
            dir.path().join("presence.sqlite3"),
            Duration::from_secs(5),
            Duration::ZERO,
        )
        .unwrap();
        assert!(p.renew("owner", &l.lease_id, renew(&l, 1)).is_err());
        let status: String = p
            .connect()
            .unwrap()
            .query_row(
                "SELECT status FROM browser_clients WHERE lease_id=?1",
                [l.lease_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "server_restarted");
    }
    #[test]
    fn malformed_clients_and_unbounded_timing_are_refused() {
        let (dir, p) = store();
        assert!(p.register("owner".into(), "bad".into(), vec![]).is_err());
        assert!(BrowserPresence::new(
            dir.path().join("bad.sqlite"),
            Duration::from_secs(1),
            Duration::ZERO
        )
        .is_err());
        assert!(BrowserPresence::new(
            dir.path().join("bad2.sqlite"),
            Duration::from_secs(5),
            Duration::from_secs(601)
        )
        .is_err());
    }

    fn application() -> (
        tempfile::TempDir,
        FactoryApplication,
        Vec<Arc<factory_projects::ProjectContext>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let app = FactoryApplication::new(crate::application::ApplicationOptions {
            registry_path: dir.path().join("registry.sqlite3"),
            allowed_roots: vec![dir.path().into()],
            default_root: None,
            dashboard_root: dir.path().into(),
            limits: factory_projects::ProjectLimits::default(),
            api: crate::security::ApiOptions::default(),
        })
        .unwrap();
        let mut contexts = Vec::new();
        for name in ["A", "B"] {
            let root = dir.path().join(name);
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("baseline.txt"), "baseline").unwrap();
            for args in [
                &["init", "-q", "-b", "main"][..],
                &["config", "user.name", "Presence Test"],
                &["config", "user.email", "test@example.invalid"],
                &["add", "."],
                &["commit", "-qm", "baseline"],
            ] {
                assert!(std::process::Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success());
            }
            factory_core::Factory::init(&root).unwrap();
            factory_core::Config::default().write_atomic(&root).unwrap();
            let id = app.projects.register(&root, None).unwrap().project_id;
            let context = app.projects.open(&id).unwrap();
            {
                let db = context.db.lock().unwrap();
                db.register_workspace_session(id.as_str(), "s_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .unwrap();
                let run = db.create_run("presence fixture", None).unwrap();
                db.bind_workspace_run(run.id, id.as_str(), "s_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .unwrap();
            }
            contexts.push(context);
        }
        (dir, app, contexts)
    }

    #[test]
    fn final_client_grace_reload_and_project_isolation_use_server_state() {
        let (dir, app, contexts) = application();
        let p = BrowserPresence::new(
            dir.path().join("test-presence.sqlite3"),
            Duration::from_secs(5),
            Duration::from_secs(3),
        )
        .unwrap();
        let a = SessionBinding {
            project_id: contexts[0].project_id.to_string(),
            session_id: "s_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        };
        let b = SessionBinding {
            project_id: contexts[1].project_id.to_string(),
            session_id: a.session_id.clone(),
        };
        let l1 = p
            .register("owner".into(), client(1), vec![a.clone()])
            .unwrap();
        let l2 = p
            .register("owner".into(), client(2), vec![a.clone(), b.clone()])
            .unwrap();
        p.release("owner", &l1.lease_id, renew(&l1, 0)).unwrap();
        p.tick(&app).unwrap();
        assert_eq!(
            contexts[0]
                .db
                .lock()
                .unwrap()
                .mission_lifecycle(1)
                .unwrap()
                .unwrap()
                .state,
            factory_types::MissionLifecycleState::Active
        );
        p.release("owner", &l2.lease_id, renew(&l2, 0)).unwrap();
        p.tick(&app).unwrap();
        assert_eq!(
            contexts[0]
                .db
                .lock()
                .unwrap()
                .mission_lifecycle(1)
                .unwrap()
                .unwrap()
                .state,
            factory_types::MissionLifecycleState::Active
        );
        let reopened = p
            .register("owner".into(), client(3), vec![a.clone(), b.clone()])
            .unwrap();
        p.tick(&app).unwrap();
        assert!(!p.absent.lock().unwrap().contains_key(&a));
        p.renew(
            "owner",
            &reopened.lease_id,
            Renew {
                bindings: vec![b.clone()],
                ..renew(&reopened, 1)
            },
        )
        .unwrap();
        p.tick(&app).unwrap();
        p.absent
            .lock()
            .unwrap()
            .insert(a.clone(), Instant::now() - Duration::from_secs(4));
        p.tick(&app).unwrap();
        assert_eq!(
            contexts[0]
                .db
                .lock()
                .unwrap()
                .mission_lifecycle(1)
                .unwrap()
                .unwrap()
                .state,
            factory_types::MissionLifecycleState::Suspended
        );
        assert_eq!(
            contexts[1]
                .db
                .lock()
                .unwrap()
                .mission_lifecycle(1)
                .unwrap()
                .unwrap()
                .state,
            factory_types::MissionLifecycleState::Active
        );
        p.release("owner", &reopened.lease_id, renew(&reopened, 1))
            .unwrap();
        p.tick(&app).unwrap();
        p.absent
            .lock()
            .unwrap()
            .insert(b, Instant::now() - Duration::from_secs(4));
        p.tick(&app).unwrap();
        assert_eq!(
            contexts[1]
                .db
                .lock()
                .unwrap()
                .mission_lifecycle(1)
                .unwrap()
                .unwrap()
                .state,
            factory_types::MissionLifecycleState::Suspended
        );
    }

    #[test]
    fn unavailable_lifecycle_in_one_project_does_not_block_another_projects_absence_fence() {
        let (dir, app, contexts) = application();
        let p = BrowserPresence::new(
            dir.path().join("isolated-presence.sqlite3"),
            Duration::from_secs(5),
            Duration::ZERO,
        )
        .unwrap();
        let connection = Connection::open(contexts[0].root.join(".factory/db.sqlite3")).unwrap();
        connection
            .execute(
                "UPDATE mission_lifecycle SET state='unknown_future_state' WHERE run_id=1",
                [],
            )
            .unwrap();
        p.tick(&app).unwrap();
        assert_eq!(
            contexts[1]
                .db
                .lock()
                .unwrap()
                .mission_lifecycle(1)
                .unwrap()
                .unwrap()
                .state,
            factory_types::MissionLifecycleState::Suspended
        );
        assert!(contexts[0].db.lock().unwrap().mission_lifecycle(1).is_err());
    }

    #[test]
    fn twenty_clients_renew_without_creating_additional_leases_and_failures_do_not_extend() {
        let (dir, p) = store();
        let mut leases = Vec::new();
        for id in 0..20 {
            leases.push(p.register("owner".into(), client(id), vec![]).unwrap());
        }
        for lease in &leases {
            p.renew("owner", &lease.lease_id, renew(lease, 1)).unwrap();
        }
        assert_eq!(p.leases.lock().unwrap().len(), 20);
        let before = p.leases.lock().unwrap()[&leases[0].lease_id].expires;
        let connection = Connection::open(dir.path().join("presence.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER refuse_presence_update BEFORE UPDATE ON browser_clients BEGIN SELECT RAISE(FAIL,'controlled storage failure'); END;").unwrap();
        assert!(p
            .renew("owner", &leases[0].lease_id, renew(&leases[0], 2))
            .is_err());
        assert_eq!(
            p.leases.lock().unwrap()[&leases[0].lease_id].expires,
            before
        );
    }
}
