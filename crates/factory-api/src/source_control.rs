use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use factory_core::{Factory, FactoryError};
use factory_git::source_control::{digest, DiffRequest, IndexRequest, SourceTiming};
use factory_git::{GitError, Repo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::SharedState;

pub(crate) fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/source-control/status", get(status))
        .route("/api/source-control/repository", get(repository))
        .route("/api/source-control/roots", get(roots))
        .route("/api/source-control/diff", post(diff))
        .route(
            "/api/source-control/index",
            post(index).layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route("/api/source-control/commit", post(commit))
        .route("/api/source-control/branches", get(branches).post(branch))
        .route("/api/source-control/history", get(history))
        .route("/api/source-control/commit-files", get(commit_files))
        .route("/api/source-control/worktrees", get(worktrees))
        .route("/api/source-control/conflicts", get(conflict))
        .route("/api/source-control/reviews", get(reviews))
        .route("/api/source-control/reviews/:id", get(review))
        .route("/api/source-control/reviews/:id/comments", post(comment))
        .route("/api/source-control/operations", get(operations))
        .route(
            "/api/source-control/delivery/:run/update",
            post(update_pull_request),
        )
        .route("/api/source-control/github", get(github))
        .route(
            "/api/source-control/github/pulls/:number",
            get(pull_request),
        )
        .route(
            "/api/source-control/delivery/:run",
            get(delivery_preview).post(deliver),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
}

struct Error(Box<FactoryError>, Option<String>);
impl From<FactoryError> for Error {
    fn from(error: FactoryError) -> Self {
        Self(Box::new(error), None)
    }
}
impl From<GitError> for Error {
    fn from(error: GitError) -> Self {
        Self(Box::new(error.into()), None)
    }
}
impl From<factory_files::FileError> for Error {
    fn from(error: factory_files::FileError) -> Self {
        if matches!(error, factory_files::FileError::Busy) {
            return GitError::Busy.into();
        }
        Self(
            Box::new(FactoryError::PolicyBlocked(error.to_string())),
            None,
        )
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self.0.as_ref() {
            FactoryError::GitHub(factory_github::GitHubError::GhAuthRequired) => {
                (StatusCode::UNAUTHORIZED, "github_auth_required")
            }
            FactoryError::GitHub(factory_github::GitHubError::PermissionDenied(_)) => {
                (StatusCode::FORBIDDEN, "github_permission_denied")
            }
            FactoryError::GitHub(factory_github::GitHubError::RateLimited) => {
                (StatusCode::TOO_MANY_REQUESTS, "github_rate_limited")
            }
            FactoryError::GitHub(factory_github::GitHubError::Uncertain(_)) => {
                (StatusCode::CONFLICT, "github_reconciliation_required")
            }
            FactoryError::GitHub(
                factory_github::GitHubError::NotAGithubRemote
                | factory_github::GitHubError::RemoteParse(_),
            ) => (StatusCode::BAD_REQUEST, "github_repository_unavailable"),
            FactoryError::Git(GitError::Invalid(_)) => {
                (StatusCode::BAD_REQUEST, "invalid_git_request")
            }
            FactoryError::Git(GitError::Stale) => (StatusCode::CONFLICT, "stale_git_state"),
            FactoryError::Git(GitError::Busy) => (StatusCode::CONFLICT, "git_busy"),
            FactoryError::Git(GitError::TrustRequired(_)) => {
                (StatusCode::FORBIDDEN, "git_program_trust_required")
            }
            FactoryError::Git(GitError::Protected(_))
            | FactoryError::PolicyBlocked(_)
            | FactoryError::Suspended => (StatusCode::FORBIDDEN, "git_operation_denied"),
            FactoryError::Git(GitError::OutputLimitExceeded { .. }) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "git_limit")
            }
            FactoryError::Db(factory_db::DbError::NotFound(_))
            | FactoryError::RunNotFound(_)
            | FactoryError::TaskNotFound(_) => (StatusCode::NOT_FOUND, "git_evidence_not_found"),
            FactoryError::Db(factory_db::DbError::Busy(_)) => {
                (StatusCode::CONFLICT, "git_operation_conflict")
            }
            _ => (StatusCode::SERVICE_UNAVAILABLE, "git_operation_failed"),
        };
        (
            status,
            Json(json!({"code":code,"error":self.1.unwrap_or_else(||redact(&self.0.to_string()))})),
        )
            .into_response()
    }
}

fn redact(text: &str) -> String {
    let mut secrets = factory_policy::Secrets::new();
    for (key, value) in std::env::vars() {
        let key = key.to_ascii_lowercase();
        if [
            "token",
            "secret",
            "password",
            "api_key",
            "credential",
            "cookie",
        ]
        .iter()
        .any(|pattern| key.contains(pattern))
        {
            secrets.add(&value, 4);
        }
    }
    secrets.redact(text)
}

fn project_secrets(f: &Factory) -> factory_policy::Secrets {
    let mut secrets = factory_policy::Secrets::new();
    for agent in f.agents().config().agents.values() {
        for value in agent.env.values() {
            secrets.add(value, 4);
        }
    }
    secrets
}

async fn operation(
    s: SharedState,
    action: impl FnOnce(SharedState, Factory, Arc<factory_files::FileService>) -> Result<Value, Error>
        + Send
        + 'static,
) -> Result<Json<Value>, Error> {
    let context = s
        ._context
        .as_ref()
        .ok_or_else(|| GitError::Protected("Registered project required".into()))?;
    context
        .validate()
        .map_err(|_| GitError::Protected("Project identity changed".into()))?;
    let files = context.files.clone();
    let permit = s
        .source_requests
        .clone()
        .try_acquire_owned()
        .map_err(|_| GitError::Busy)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let project = loop {
        match files.acquire() {
            Ok(permit) => break permit,
            Err(factory_files::FileError::Busy) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await
            }
            Err(error) => return Err(error.into()),
        }
    };
    tokio::task::spawn_blocking(move || {
        let _permits = (permit, project);
        let factory = s.runtime.factory()?;
        let secrets = project_secrets(&factory);
        action(s, factory, files).map(Json).map_err(|mut error| {
            error.1 = Some(secrets.redact(&redact(&error.0.to_string())));
            error
        })
    })
    .await
    .map_err(|_| {
        GitError::Protected(
            "Operation interrupted; inspect its durable audit before retrying".into(),
        )
    })?
}

fn repo(files: &factory_files::FileService, id: &str) -> Result<Repo, Error> {
    let path = files.inspect_root(id)?;
    Ok(Repo::detect_bounded(&path, &path)?)
}

fn file_guard(files: &factory_files::FileService, root: &str, path: &str) -> Result<(), Error> {
    match files.read(&factory_files::FileRef {
        project_id: files.project_id.clone(),
        root_id: root.into(),
        path: path.into(),
    }) {
        Ok(_) | Err(factory_files::FileError::NotFound) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn path_policy(f: &Factory, path: &str, write: bool) -> Result<(), GitError> {
    let policy = f.agents().config().policies.effective("source_control", "");
    check_path_policy(&policy, path, write)
}

fn check_path_policy(
    policy: &factory_policy::EffectivePolicy,
    path: &str,
    write: bool,
) -> Result<(), GitError> {
    if write
        && !policy
            .git
            .allows(factory_policy::GitOperation::CommitInTaskWorktree)
    {
        return Err(GitError::Protected(
            "Project Git policy denies manual index/commit mutations".into(),
        ));
    }
    let result = if write {
        policy.filesystem.check_write(path)
    } else {
        if policy.filesystem.read_allowed(path) {
            Ok(path.to_string())
        } else {
            Err("Path is outside the project read policy".into())
        }
    };
    result.map(|_| ()).map_err(GitError::Protected)
}

fn command_policy(f: &Factory, command: &str, trust_programs: bool) -> Result<(), Error> {
    let policies = &f.agents().config().policies;
    for scope in [policies.ceiling.as_ref(), policies.role("source_control")]
        .into_iter()
        .flatten()
    {
        if let Some(policy) = &scope.commands {
            let matches = |pattern: &String| {
                factory_policy::path::matches_glob(pattern, command, !cfg!(windows))
            };
            if policy.mode == factory_policy::CommandsMode::Denied
                || policy.deny.iter().any(matches)
                || policy.mode == factory_policy::CommandsMode::Restricted
                    && !policy.allow.iter().any(matches)
            {
                return Err(GitError::Protected(
                    "Project command policy denies the selected typed Git operation".into(),
                )
                .into());
            }
            if trust_programs
                && (policy.mode != factory_policy::CommandsMode::Unrestricted
                    || !policy.deny.is_empty())
            {
                return Err(GitError::Protected("Configured helper execution is unavailable under this restrictive command ceiling".into()).into());
            }
        }
    }
    Ok(())
}

fn mutation(
    context: (&Factory, &SharedState, &factory_files::FileService),
    root: &str,
    identity: (&str, &str),
    body: &Value,
    checkout: bool,
    action: impl FnOnce(&Repo) -> Result<Value, Error>,
) -> Result<Value, Error> {
    let (f, s, files) = context;
    let (key, kind) = identity;
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
    {
        return Err(GitError::Invalid("Idempotency key required".into()).into());
    }
    let fingerprint = digest(
        serde_json::to_vec(body)
            .map_err(factory_db::DbError::from)
            .map_err(FactoryError::from)?,
    );
    {
        let db = s.db.lock().map_err(|_| GitError::Busy)?;
        if let Some(previous) = db.source_operation(key).map_err(FactoryError::from)? {
            if previous["requestHash"] != fingerprint
                || previous["rootId"] != root
                || previous["kind"] != kind
            {
                return Err(GitError::Stale.into());
            }
            if previous["state"] == "completed" {
                return Ok(previous["result"].clone());
            }
            return Err(GitError::Protected("A retained operation has failed or has an uncertain outcome; inspect the audit and actual Git state before using a new key".into()).into());
        }
        if !db
            .claim_source_operation(key, &fingerprint, kind, root)
            .map_err(FactoryError::from)?
        {
            return Err(GitError::Busy.into());
        }
    }
    let result = files.source_operation(root, checkout, |path| {
        let repo = Repo::detect_bounded(path, path)?;
        action(&repo)
    });
    let (state, payload) = match &result {
        Ok(value) => ("completed", value.clone()),
        Err(error) => (
            "failed",
            json!({"error":project_secrets(f).redact(&redact(&error.0.to_string()))}),
        ),
    };
    s.db.lock()
        .map_err(|_| GitError::Busy)?
        .finish_source_operation(key, state, &payload)
        .map_err(FactoryError::from)?;
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootQuery {
    root_id: String,
}
async fn status(
    State(s): State<SharedState>,
    Query(q): Query<RootQuery>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, _, files| {
        let (_, _, common, status) = files.observe_source::<Error>(&q.root_id)?;
        Ok(json!({"schema":1,"projectId":files.project_id,"rootId":q.root_id,"gitCommonIdentity":digest(common.to_string_lossy().as_bytes()),"status":status}))
    }).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryQuery {
    root_id: Option<String>,
}

fn observed_repository(files: &factory_files::FileService, id: &str) -> Result<Value, Error> {
    let (root, path, common, status) = files.observe_source::<Error>(id)?;
    Ok(json!({
        "root": root,
        "rootPath": factory_git::git_path(&path),
        "gitCommonIdentity": digest(common.to_string_lossy().as_bytes()),
        "status": status,
    }))
}

async fn repository(
    State(s): State<SharedState>,
    Query(q): Query<RepositoryQuery>,
) -> Result<Json<Value>, Error> {
    let mut timing = SourceTiming::start("source_repository");
    operation(s, move |_, _, files| {
        timing.phase("project_admission");
        let primary = files.primary_root();
        let id = q.root_id.as_deref().unwrap_or(&primary.id);
        if id != primary.id {
            files.roots()?;
        }
        timing.phase("selected_root");
        let response = observed_repository(&files, id)?;
        timing.phase("observation_and_response");
        Ok(response)
    })
    .await
}

async fn roots(State(s): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(s, move |_, _, files| Ok(json!(files.roots()?))).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiffBody {
    root_id: String,
    diff: DiffRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConflictQuery {
    root_id: String,
    path: String,
}
async fn conflict(
    State(s): State<SharedState>,
    Query(q): Query<ConflictQuery>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, f, files| {
        path_policy(&f, &q.path, false)?;
        file_guard(&files, &q.root_id, &q.path)?;
        Ok(repo(&files, &q.root_id)?.source_conflict(&q.path)?)
    })
    .await
}
async fn diff(
    State(s): State<SharedState>,
    Json(body): Json<DiffBody>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, f, files| {
        path_policy(&f, &body.diff.path, false)?;
        file_guard(&files, &body.root_id, &body.diff.path)?;
        Ok(json!(repo(&files, &body.root_id)?.source_diff(&body.diff)?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IndexBody {
    root_id: String,
    key: String,
    request: IndexRequest,
}
async fn index(
    State(s): State<SharedState>,
    Json(body): Json<IndexBody>,
) -> Result<Json<Value>, Error> {
    let mut timing = SourceTiming::start("index_api");
    operation(s, move |s, f, files| {
        timing.phase("project_admission");
        body.request.validate_selection()?;
        command_policy(
            &f,
            if body.request.unstage {
                "git reset --mixed"
            } else {
                "git add"
            },
            body.request.trust_programs,
        )?;
        let policy = f.agents().config().policies.effective("source_control", "");
        for path in &body.request.paths {
            check_path_policy(&policy, path, true)?;
        }
        timing.phase("path_policy");
        files.guard_source_paths(&body.root_id, &body.request.paths)?;
        timing.phase("file_guards");
        let payload = json!(&body.request);
        let result = mutation(
            (&f, &s, &files),
            &body.root_id,
            (&body.key, "index"),
            &payload,
            false,
            |repo| {
                timing.phase("journal_and_repository");
                let status = repo.source_index(&body.request)?;
                timing.phase("repository_operation");
                Ok(json!(status))
            },
        );
        timing.phase("serialization_and_journal");
        result
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitBody {
    root_id: String,
    key: String,
    expected_generation: String,
    message: String,
    #[serde(default)]
    trust_programs: bool,
}
async fn commit(
    State(s): State<SharedState>,
    Json(body): Json<CommitBody>,
) -> Result<Json<Value>, Error> {
    operation(s,move|s,f,files|{
        command_policy(&f,"git commit",body.trust_programs)?;
        let payload=json!({"generation":body.expected_generation,"message":body.message,"trustPrograms":body.trust_programs});
        mutation((&f,&s,&files),&body.root_id,(&body.key,"commit"),&payload,false,|repo|{
            let sha=repo.source_commit_authorized(&body.expected_generation,&body.message,body.trust_programs,|path|path_policy(&f,path,true))?;
            Ok(json!({"commit":sha,"status":repo.source_status()?}))
        })
    }).await
}
async fn branches(
    State(s): State<SharedState>,
    Query(q): Query<RootQuery>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, _, files| {
        Ok(json!(repo(&files, &q.root_id)?.source_branches()?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BranchBody {
    root_id: String,
    key: String,
    expected_generation: String,
    name: String,
    create: bool,
    #[serde(default)]
    trust_programs: bool,
    #[serde(default)]
    dirty_buffers: bool,
}
async fn branch(
    State(s): State<SharedState>,
    Json(body): Json<BranchBody>,
) -> Result<Json<Value>, Error> {
    operation(s, move |s, f, files| {
        command_policy(&f, "git switch", body.trust_programs)?;
        if body.dirty_buffers {
            return Err(GitError::Protected(
                "Unsaved editor buffers must be saved or explicitly retained before checkout"
                    .into(),
            )
            .into());
        }
        let policy = f.agents().config().policies.effective("source_control", "");
        if policy.filesystem.read_only()
            || !policy.git.allows(factory_policy::GitOperation::CommitInTaskWorktree)
        {
            return Err(GitError::Protected("Project write/Git policy prevents checkout".into()).into());
        }
        let payload = json!({"generation":body.expected_generation,"name":body.name,"create":body.create,"trustPrograms":body.trust_programs});
        mutation(
            (&f, &s, &files),
            &body.root_id,
            (&body.key, "branch"),
            &payload,
            true,
            |repo| {
                Ok(json!(repo.source_branch_with_target_check(
                    &body.expected_generation,
                    &body.name,
                    body.create,
                    body.trust_programs,
                    |repo, status, target| {
                        let head = status.head.as_deref().ok_or(GitError::Stale)?;
                        for path in repo.source_changed_paths(head, target)? {
                            path_policy(&f, &path, true)?;
                        }
                        Ok(())
                    },
                )?))
            },
        )
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryQuery {
    root_id: String,
    start: Option<String>,
    skip: Option<usize>,
    limit: Option<usize>,
}
async fn history(
    State(s): State<SharedState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, _, files| {
        Ok(json!(repo(&files, &q.root_id)?.source_history(
            q.start.as_deref(),
            q.skip.unwrap_or(0),
            q.limit.unwrap_or(25)
        )?))
    })
    .await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitFiles {
    root_id: String,
    base: String,
    target: String,
}
async fn commit_files(
    State(s): State<SharedState>,
    Query(q): Query<CommitFiles>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, _, files| {
        Ok(json!(
            repo(&files, &q.root_id)?.source_changed_paths(&q.base, &q.target)?
        ))
    })
    .await
}
async fn worktrees(State(s): State<SharedState>) -> Result<Json<Value>, Error> {
    operation(s, move |_, _, files| {
        let values = files
            .roots()?
            .iter()
            .map(|root| observed_repository(&files, &root.id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!(values))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewQuery {
    run_id: Option<i64>,
    before: Option<i64>,
}
async fn reviews(
    State(s): State<SharedState>,
    Query(q): Query<ReviewQuery>,
) -> Result<Json<Value>, Error> {
    operation(s, move |s, _, _| {
        let items =
            s.db.lock()
                .map_err(|_| GitError::Busy)?
                .source_review_list(q.run_id, q.before.unwrap_or(i64::MAX))
                .map_err(FactoryError::from)?;
        let next = if items.len() == 100 {
            items.last().and_then(|item| item["attempt"]["id"].as_i64())
        } else {
            None
        };
        Ok(json!({"items":items,"bounded":true,"pageLimit":100,"nextCursor":next}))
    })
    .await
}
async fn review(State(s): State<SharedState>, Path(id): Path<i64>) -> Result<Json<Value>, Error> {
    operation(s, move |_, f, files| {
        let detail = f.review_evidence(id)?;
        let root = files.primary_root().id;
        let paths = detail["actualPaths"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(
                detail["attempt"]["evidence"]["changedFiles"]
                    .as_array()
                    .into_iter()
                    .flatten(),
            )
            .filter_map(Value::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        for path in paths {
            factory_git::source_control::validate_path(path)?;
            path_policy(&f, path, false)?;
            file_guard(&files, &root, path)?;
        }
        Ok(detail)
    })
    .await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Comment {
    key: String,
    anchor: Value,
    decision: String,
    body: String,
}
async fn comment(
    State(s): State<SharedState>,
    Path(id): Path<i64>,
    Json(body): Json<Comment>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, f, _| {
        Ok(f.comment_on_review(id, &body.key, &body.anchor, &body.decision, &body.body)?)
    })
    .await
}
#[derive(Deserialize)]
struct Before {
    before: Option<i64>,
}
async fn operations(
    State(s): State<SharedState>,
    Query(q): Query<Before>,
) -> Result<Json<Value>, Error> {
    operation(s, move |s, _, _| {
        Ok(json!(s
            .db
            .lock()
            .map_err(|_| GitError::Busy)?
            .source_operations(q.before.unwrap_or(i64::MAX))
            .map_err(FactoryError::from)?))
    })
    .await
}

#[derive(Deserialize)]
struct RemotePage {
    page: Option<usize>,
}
fn remote_read_policy(f: &Factory) -> Result<(), Error> {
    if !f
        .agents()
        .config()
        .policies
        .effective("source_control", "")
        .network
        .allowed()
    {
        return Err(GitError::Protected(
            "Project Source Control network policy denies GitHub access".into(),
        )
        .into());
    }
    command_policy(f, "gh api", false)
}
async fn github(
    State(s): State<SharedState>,
    Query(q): Query<RemotePage>,
) -> Result<Json<Value>, Error> {
    operation(s,move|_,f,_|{
        remote_read_policy(&f)?;
        let remote=f.github_remote()?;
        let gh=factory_github::GhCli::discovered();
        gh.auth_status().map_err(FactoryError::from)?;
        Ok(json!({"repository":gh.repository_metadata(&remote.repository).map_err(FactoryError::from)?,"remote":remote.remote,"pullRequests":gh.pull_requests_page(&remote.repository,q.page.unwrap_or(1)).map_err(FactoryError::from)?,"observedAt":chrono::Utc::now().to_rfc3339()}))
    }).await
}
async fn pull_request(
    State(s): State<SharedState>,
    Path(number): Path<i64>,
    Query(q): Query<RemotePage>,
) -> Result<Json<Value>, Error> {
    operation(s, move |_, f, _| {
        remote_read_policy(&f)?;
        let remote = f.github_remote()?;
        let gh = factory_github::GhCli::discovered();
        gh.auth_status().map_err(FactoryError::from)?;
        Ok(gh
            .pull_request_details(&remote.repository, number, q.page.unwrap_or(1))
            .map_err(FactoryError::from)?)
    })
    .await
}
async fn delivery_preview(
    State(s): State<SharedState>,
    Path(run): Path<i64>,
) -> Result<Json<Value>, Error> {
    operation(s,move|s,f,_|{
        remote_read_policy(&f)?;
        let preview=f.pull_request_preview(run)?;
        let report=f.delivery_report(run)?;
        let gh=factory_github::GhCli::discovered();gh.auth_status().map_err(FactoryError::from)?;
        let repository=gh.repository_metadata(&preview.repository).map_err(FactoryError::from)?;
        let changes=f.source_delivery_changes(run)?;
        let authorization=f.authorize_manual_delivery(run).err().map(|error|error.to_string());
        let authorization=authorization.or_else(||f.github_remote().ok().and_then(|remote|factory_github::remote::verify_push_destination(&s.root,&remote.remote,&remote.repository).err().map(|error|error.to_string())));
        let authorization=authorization.or_else(||(changes["changedFilesComplete"]!=true).then(||"The integration comparison base is unavailable; inspect its retained evidence before publication".to_owned()));
        Ok(json!({"preview":preview,"delivery":report,"repository":repository,"changedFiles":changes["changedFiles"],"changedFilesComplete":changes["changedFilesComplete"],"comparisonBase":changes["comparisonBase"],"baseSource":changes["baseSource"],"authorizationBlocker":authorization,"externalActions":["Push the recorded integration branch without force","Create or link a pull request in the verified repository","Configured Git hooks, credential helpers and SSH programs execute as trusted local code under the process deadline"],"requiresExplicitAuthorization":true,"publishedByPreview":false}))
    }).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Deliver {
    key: String,
    authorize: bool,
    repository: String,
    repository_id: i64,
    expected_integration: String,
    base: String,
    head: String,
    title: Option<String>,
    body: Option<String>,
    draft: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdatePullRequest {
    key: String,
    authorize: bool,
    expected_integration: String,
    expected_remote_updated_at: String,
    repository_id: i64,
    title: String,
    body: String,
    draft: bool,
}

async fn update_pull_request(
    State(s): State<SharedState>,
    Path(run): Path<i64>,
    Json(body): Json<UpdatePullRequest>,
) -> Result<Json<Value>, Error> {
    operation(s,move|s,f,_|{
        if !body.authorize {return Err(GitError::Protected("Explicit PR update authorization is required".into()).into());}
        f.authorize_manual_delivery(run)?;
        remote_read_policy(&f)?;
        command_policy(&f,"gh pr edit",false)?;
        let report=f.delivery_report(run)?;
        let pr=report.pull_request.as_ref().ok_or_else(||GitError::Protected("Only the run's recorded published PR can be updated".into()))?;
        let remote=f.github_remote()?;
        if report.integration_head.as_deref()!=Some(&body.expected_integration)||report.local_head.as_deref()!=Some(&body.expected_integration)||report.repository.as_ref().map(|r|r.repository.as_str())!=Some(remote.repository.as_str()) {return Err(GitError::Stale.into());}
        if body.key.is_empty()||body.key.len()>128||body.title.trim().is_empty()||body.title.len()>256||body.body.len()>48_000 {return Err(GitError::Invalid("Invalid bounded PR update".into()).into());}
        let gh=factory_github::GhCli::discovered();gh.auth_status().map_err(FactoryError::from)?;
        let identity=gh.repository_metadata(&remote.repository).map_err(FactoryError::from)?;
        if identity["id"]!=body.repository_id||identity["permissions"]["push"]!=true {return Err(GitError::Protected("GitHub repository identity/permission mismatch".into()).into());}
        let current=gh.pull_request_details(&remote.repository,pr.number,1).map_err(FactoryError::from)?;
        if current["pullRequest"]["headSha"]!=body.expected_integration||current["pullRequest"]["head"]!=report.head_branch||current["pullRequest"]["headRepository"]!=remote.repository {return Err(GitError::Stale.into());}
        let fingerprint=digest(format!("{run}:{}:{}:{}:{}:{}:{}",body.repository_id,body.expected_integration,body.expected_remote_updated_at,body.title,body.body,body.draft));
        let _permit=s.runtime.reserve_operation().map_err(|error|GitError::Protected(error.to_string()))?;
        {
            let db=s.db.lock().map_err(|_|GitError::Busy)?;
            if let Some(prior)=db.source_operation(&body.key).map_err(FactoryError::from)? {
                if prior["requestHash"]!=fingerprint{return Err(GitError::Stale.into());}
                if current["pullRequest"]["title"]==body.title&&current["body"]==body.body&&current["pullRequest"]["draft"]==body.draft {return Ok(json!({"reconciled":true,"remote":current,"audit":prior}));}
                return Err(GitError::Protected("Prior PR update may be partial. Actual remote state is different; inspect it before explicitly authorizing another operation".into()).into());
            }
            if current["pullRequest"]["updatedAt"]!=body.expected_remote_updated_at {return Err(GitError::Stale.into());}
            if !db.claim_source_operation(&body.key,&fingerprint,"pull_request_update","integration").map_err(FactoryError::from)?{return Err(GitError::Busy.into());}
        }
        let result=gh.update_pull_request(&remote.repository,pr.number,&body.title,&body.body,body.draft);
        let (state,value)=match &result {
            Ok(value)=>("completed",json!({"repositoryId":body.repository_id,"number":pr.number,"remote":value})),
            Err(error)=>("uncertain",json!({"error":redact(&error.to_string()),"repositoryId":body.repository_id,"number":pr.number})),
        };
        s.db.lock().map_err(|_|GitError::Busy)?.finish_source_operation(&body.key,state,&value).map_err(FactoryError::from)?;
        result.map_err(FactoryError::from)?;
        Ok(value)
    }).await
}
async fn deliver(
    State(s): State<SharedState>,
    Path(run): Path<i64>,
    Json(body): Json<Deliver>,
) -> Result<Json<Value>, Error> {
    operation(s, move |s, f, _| {
        if !body.authorize {
            return Err(GitError::Protected(
                "Explicit repository-scoped publication authorization required".into(),
            )
            .into());
        }
        f.authorize_manual_delivery(run)?;
        remote_read_policy(&f)?;
        if f.source_delivery_changes(run)?["changedFilesComplete"] != true {
            return Err(GitError::Protected(
                "A verified integration comparison base is required before publication".into(),
            )
            .into());
        }
        let preview = f.pull_request_preview(run)?;
        let report = f.delivery_report(run)?;
        if report.integration_head.as_deref() != Some(&body.expected_integration)
            || report.local_head.as_deref() != Some(&body.expected_integration)
            || report.head_branch != body.head
            || preview.base != body.base
            || report.repository.as_ref().map(|r| r.repository.as_str()) != Some(&body.repository)
        {
            return Err(GitError::Stale.into());
        }
        let actual_remote = f.github_remote()?;
        factory_github::remote::verify_push_destination(
            &s.root,
            &actual_remote.remote,
            &body.repository,
        )
        .map_err(FactoryError::from)?;
        command_policy(&f, "git push", true)?;
        let gh = factory_github::GhCli::discovered();
        gh.auth_status().map_err(FactoryError::from)?;
        let repository = gh
            .repository_metadata(&body.repository)
            .map_err(FactoryError::from)?;
        if repository["id"] != body.repository_id || repository["permissions"]["push"] != true {
            return Err(GitError::Protected(
                "Authenticated GitHub identity or push permission mismatch".into(),
            )
            .into());
        }
        if body.key.is_empty() || body.key.len() > 128 {
            return Err(GitError::Invalid("Delivery idempotency key required".into()).into());
        }
        let fingerprint = digest(format!(
            "{run}:{}:{}:{}:{}:{}:{}:{:?}:{:?}",
            body.repository_id,
            body.repository,
            body.expected_integration,
            body.base,
            body.head,
            body.draft,
            body.title,
            body.body
        ));
        {
            let db = s.db.lock().map_err(|_| GitError::Busy)?;
            if let Some(prior) = db.source_operation(&body.key).map_err(FactoryError::from)? {
                if prior["requestHash"] != fingerprint {
                    return Err(GitError::Stale.into());
                }
                return Ok(
                    json!({"audit":prior,"delivery":f.delivery_report(run)?,"replayed":false}),
                );
            }
            if !db
                .claim_source_operation(&body.key, &fingerprint, "delivery", "integration")
                .map_err(FactoryError::from)?
            {
                return Err(GitError::Busy.into());
            }
        }
        let result: Result<Value, Error> = (|| {
            let permit = s
                .runtime
                .reserve_operation()
                .map_err(|error| GitError::Protected(error.to_string()))?;
            match f.begin_delivery(run, body.title.as_deref(), body.body.as_deref(), body.draft)? {
                factory_core::DeliveryClaim::AlreadyPublished(delivery) => {
                    Ok(json!({"delivery":delivery,"started":false}))
                }
                factory_core::DeliveryClaim::Claimed(delivery, request) => {
                    let owner = delivery.operation_id.as_deref().ok_or(GitError::Busy)?;
                    s.runtime
                        .start_delivery_reserved(run, owner, request, permit)
                        .map_err(|error| GitError::Protected(error.to_string()))?;
                    Ok(json!({"delivery":delivery,"started":true}))
                }
            }
        })();
        let (state, value) = match &result {
            Ok(value) => ("completed", value.clone()),
            Err(error) => (
                "failed",
                json!({"error":project_secrets(&f).redact(&redact(&error.0.to_string()))}),
            ),
        };
        s.db.lock()
            .map_err(|_| GitError::Busy)?
            .finish_source_operation(&body.key, state, &value)
            .map_err(FactoryError::from)?;
        result
    })
    .await
}

#[cfg(test)]
mod tests {
    use crate::{application_router, ApiOptions, ApplicationOptions, FactoryApplication};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Router,
    };
    use http_body_util::BodyExt;
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn application(root: &std::path::Path, name: &str) -> Arc<FactoryApplication> {
        Arc::new(
            FactoryApplication::new(ApplicationOptions {
                registry_path: root.join(format!("{name}.sqlite3")),
                allowed_roots: vec![root.into()],
                default_root: None,
                dashboard_root: root.into(),
                limits: factory_projects::ProjectLimits::default(),
                api: ApiOptions::default(),
            })
            .unwrap(),
        )
    }

    fn project(app: &FactoryApplication, root: &std::path::Path) -> (String, String) {
        std::fs::create_dir(root).unwrap();
        std::fs::write(root.join(".gitignore"), ".factory/\n").unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(root)
            .output()
            .unwrap()
            .status
            .success());
        factory_core::Factory::init(root).unwrap();
        let id = app.projects.register(root, None).unwrap().project_id;
        let context = app.projects.open(&id).unwrap();
        (id.to_string(), context.files.primary_root().id)
    }

    async fn authenticated(router: &Router) -> String {
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
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn status(
        router: &Router,
        project: &(String, String),
        cookie: &str,
    ) -> (StatusCode, Value) {
        get_source(
            router,
            &project.0,
            &format!("status?rootId={}", project.1),
            cookie,
        )
        .await
    }

    async fn get_source(
        router: &Router,
        project: &str,
        resource: &str,
        cookie: &str,
    ) -> (StatusCode, Value) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/projects/{project}/source-control/{resource}"))
                    .header("host", "127.0.0.1:4321")
                    .header("origin", "http://127.0.0.1:4321")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let code = response.status();
        (
            code,
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn selected_repository_matches_status_without_a_catalog_and_fences_foreign_roots() {
        let temp = tempfile::tempdir().unwrap();
        let app = application(temp.path(), "application");
        let first = project(&app, &temp.path().join("first"));
        let second = project(&app, &temp.path().join("second"));
        std::fs::write(temp.path().join("first/local.txt"), "first project").unwrap();
        let router = application_router(app);
        let cookie = authenticated(&router).await;

        let (code, selected) = get_source(&router, &first.0, "repository", &cookie).await;
        assert_eq!(code, StatusCode::OK, "{selected}");
        assert_eq!(selected["root"]["id"], first.1);
        assert_eq!(selected["root"]["read_only"], false);
        assert!(selected["status"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["path"] == "local.txt"));
        let (code, indexed) = status(&router, &first, &cookie).await;
        assert_eq!(code, StatusCode::OK, "{indexed}");
        assert_eq!(selected["status"], indexed["status"]);
        assert_eq!(selected["gitCommonIdentity"], indexed["gitCommonIdentity"]);

        let (code, foreign) = get_source(
            &router,
            &second.0,
            &format!("repository?rootId={}", first.1),
            &cookie,
        )
        .await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{foreign}");
        assert!(foreign.get("status").is_none());
    }

    #[tokio::test]
    async fn application_budget_is_shared_between_projects_but_not_independent_applications() {
        let temp = tempfile::tempdir().unwrap();
        let a = application(temp.path(), "app-a");
        let b = application(temp.path(), "app-b");
        let a1 = project(&a, &temp.path().join("a1"));
        let a2 = project(&a, &temp.path().join("a2"));
        let b1 = project(&b, &temp.path().join("b1"));
        let ar = application_router(a.clone());
        let br = application_router(b.clone());
        let ac = authenticated(&ar).await;
        let bc = authenticated(&br).await;
        let held = a.source_requests.clone().try_acquire_many_owned(8).unwrap();
        for project in [&a1, &a2] {
            let (code, value) = status(&ar, project, &ac).await;
            assert_eq!(code, StatusCode::CONFLICT, "{value}");
            assert_eq!(value["code"], "git_busy");
        }
        let (code, value) = status(&br, &b1, &bc).await;
        assert_eq!(code, StatusCode::OK, "{value}");
        drop(held);
        let (code, value) = status(&ar, &a2, &ac).await;
        assert_eq!(code, StatusCode::OK, "{value}");
        assert_eq!(a.source_requests.available_permits(), 8);
    }
}
