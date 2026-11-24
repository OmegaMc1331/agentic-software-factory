use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use factory_api::{application_router, ApiOptions, ApplicationOptions, FactoryApplication};
use factory_projects::ProjectLimits;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    _dir: tempfile::TempDir,
    router: Router,
    a: String,
    b: String,
    cookie: String,
    csrf: String,
    app: Arc<FactoryApplication>,
}

#[tokio::test]
async fn bounded_large_index_selection_uses_the_authenticated_project_boundary() {
    let f = Fixture::new().await;
    let root = f.root(&f.a);
    let paths: Vec<_> = (0..600)
        .map(|n| format!("selection_{n:04}_{}.txt", "bounded".repeat(15)))
        .collect();
    for path in &paths {
        std::fs::write(f._dir.path().join("A").join(path), "owned selection\n").unwrap();
    }
    let before = f.status().await;
    let body = json!({"rootId":root,"key":"large-selection","request":{"expectedGeneration":before["generation"],"paths":paths,"unstage":false,"hunk":null,"diffVersion":null,"trustPrograms":false}});
    assert!(serde_json::to_vec(&body).unwrap().len() > 64 * 1024);
    let (status, staged) = f.call(&f.a, "/index", "POST", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    assert_eq!(
        f.git(&["diff", "--cached", "--name-only"]).lines().count(),
        600
    );
    let (status, repeated) = f.call(&f.a, "/index", "POST", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(staged, repeated);
    let (status, _) = f.call(&f.b, "/index", "POST", body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.raw(
        &format!("/api/projects/{}/source-control/index", f.a),
        "POST",
        json!({"rootId":root,"key":"no-csrf","request":{"expectedGeneration":staged["generation"],"paths":paths,"unstage":true,"trustPrograms":false}}),
        true,
        false,
        "http://127.0.0.1:4321",
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        f.git(&["diff", "--cached", "--name-only"]).lines().count(),
        600
    );
    let (status, _) = f.call(
        &f.a,
        "/index",
        "POST",
        json!({"rootId":root,"key":"oversized-selection","request":{"expectedGeneration":staged["generation"],"paths":vec!["x".repeat(2048);600],"unstage":true,"trustPrograms":false}}),
    ).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        f.git(&["diff", "--cached", "--name-only"]).lines().count(),
        600
    );
}

impl Fixture {
    fn root(&self, project: &str) -> String {
        let id = factory_projects::ProjectId::parse(project).unwrap();
        self.app.projects.open(&id).unwrap().files.primary_root().id
    }
    async fn status(&self) -> Value {
        let (status, value) = self
            .call(
                &self.a,
                &format!("/status?rootId={}", self.root(&self.a)),
                "GET",
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["status"].clone()
    }
    fn git(&self, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(self._dir.path().join("A"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().into()
    }
}

#[tokio::test]
async fn source_auth_csrf_origin_project_and_worktree_boundaries_are_real() {
    let f = Fixture::new().await;
    let root = f.root(&f.a);
    let path = format!("/api/projects/{}/source-control/index", f.a);
    let body = json!({"rootId":root,"key":"auth","request":{"expectedGeneration":"stale","paths":["ARCHITECTURE.md"],"unstage":false,"hunk":null,"diffVersion":null,"trustPrograms":false}});
    for (auth, csrf, origin) in [
        (false, true, "http://127.0.0.1:4321"),
        (true, false, "http://127.0.0.1:4321"),
        (true, true, "https://attacker.invalid"),
    ] {
        let (status, _) = f.raw(&path, "POST", body.clone(), auth, csrf, origin).await;
        assert!(
            matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            "{status}"
        );
    }
    let (status, _) = f
        .call(&f.b, &format!("/status?rootId={root}"), "GET", Value::Null)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for path in ["../outside", ".git/config", ".factory/db.sqlite3", ".env"] {
        let (status,_)=f.call(&f.a,"/diff","POST",json!({"rootId":root,"diff":{"path":path,"comparison":"working_index","base":null,"target":null}})).await;
        assert!(status.is_client_error(), "{path}: {status}");
    }
    assert_eq!(f.git(&["status", "--porcelain"]), "");
    let outside = f._dir.path().join("outside.txt");
    std::fs::write(&outside, "Unrelated root content").unwrap();
    let linked = f._dir.path().join("A/linked.txt");
    std::fs::hard_link(&outside, &linked).unwrap();
    let (status, _) = f.call(&f.a, "/diff", "POST", json!({"rootId":root,"diff":{"path":"linked.txt","comparison":"working_index","base":null,"target":null}})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn actual_file_index_diff_commit_idempotency_and_stale_requests_work() {
    let f = Fixture::new().await;
    std::fs::write(
        f._dir.path().join("A/ARCHITECTURE.md"),
        "Changed architectural fact.\n",
    )
    .unwrap();
    let root = f.root(&f.a);
    let before = f.status().await;
    let request = json!({"rootId":root,"key":"stage-1","request":{"expectedGeneration":before["generation"],"paths":["ARCHITECTURE.md"],"unstage":false,"hunk":null,"diffVersion":null,"trustPrograms":true}});
    let (status, staged) = f.call(&f.a, "/index", "POST", request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let (status, repeated) = f.call(&f.a, "/index", "POST", request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(staged, repeated);
    let (status,diff)=f.call(&f.a,"/diff","POST",json!({"rootId":root,"diff":{"path":"ARCHITECTURE.md","comparison":"index_head","base":null,"target":null}})).await;
    assert_eq!(status, StatusCode::OK, "{diff}");
    assert!(diff["patch"]
        .as_str()
        .unwrap()
        .contains("Changed architectural fact"));
    let request = json!({"rootId":root,"key":"commit-1","expectedGeneration":staged["generation"],"message":"local reviewed change","trustPrograms":true});
    let (status, commit) = f.call(&f.a, "/commit", "POST", request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{commit}");
    assert_eq!(commit["commit"], f.git(&["rev-parse", "HEAD"]));
    let (status, duplicate) = f.call(&f.a, "/commit", "POST", request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(commit, duplicate);
    assert_eq!(f.git(&["rev-list", "--count", "HEAD"]), "2");
    let (status,_)=f.call(&f.a,"/commit","POST",json!({"rootId":root,"key":"stale","expectedGeneration":before["generation"],"message":"stale commit","trustPrograms":true})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, history) = f
        .call(
            &f.a,
            &format!("/history?rootId={root}&limit=1"),
            "GET",
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn managed_worktrees_and_dirty_editor_drafts_cannot_be_mutated() {
    let f = Fixture::new().await;
    let root = f.root(&f.a);
    let id = factory_projects::ProjectId::parse(&f.a).unwrap();
    let context = f.app.projects.open(&id).unwrap();
    let file = factory_files::FileRef {
        project_id: f.a.clone(),
        root_id: root.clone(),
        path: "ARCHITECTURE.md".into(),
    };
    let source = context.files.read(&file).unwrap();
    context
        .files
        .save_draft(
            factory_files::Draft {
                schema: 1,
                id: "d_11111111111111111111111111111111".into(),
                file,
                base_revision: source.revision,
                base_content: source.content.clone().unwrap(),
                content: "unsaved draft\n".into(),
                newline: "lf".into(),
                bom: false,
                dirty: true,
            },
            0,
        )
        .unwrap();
    let before = f.status().await;
    let (status,value)=f.call(&f.a,"/branches","POST",json!({"rootId":root,"key":"dirty-checkout","expectedGeneration":before["generation"],"name":"feature/safe","create":true,"trustPrograms":true,"dirtyBuffers":false})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    assert_eq!(f.git(&["branch", "--show-current"]), "main");
    let repo = factory_git::Repo::detect(&context.root).unwrap();
    let path = context.root.join(".factory/worktrees/t91");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    repo.add_worktree(&path, "factory/t91", None).unwrap();
    let managed = context
        .files
        .roots()
        .unwrap()
        .into_iter()
        .find(|r| r.read_only)
        .unwrap();
    let generation = factory_git::Repo::detect(&path)
        .unwrap()
        .source_status()
        .unwrap()
        .generation;
    let (status,_)=f.call(&f.a,"/index","POST",json!({"rootId":managed.id,"key":"denied-managed","request":{"expectedGeneration":generation,"paths":["ARCHITECTURE.md"],"unstage":false,"hunk":null,"diffVersion":null,"trustPrograms":true}})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        std::fs::read_to_string(path.join("ARCHITECTURE.md")).unwrap(),
        source.content.unwrap()
    );
}

#[tokio::test]
async fn publication_without_explicit_authorization_is_rejected_before_remote_calls() {
    let f = Fixture::new().await;
    let (status,value)=f.call(&f.a,"/delivery/1","POST",json!({"key":"denied","authorize":false,"repository":"owner/disposable","repositoryId":7,"expectedIntegration":"0".repeat(40),"base":"main","head":"factory/run-1","title":null,"body":null,"draft":true})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    assert_eq!(value["code"], "git_operation_denied");
    assert_eq!(f.git(&["remote"]), "");
}

#[tokio::test]
async fn refresh_capacity_waits_briefly_and_reports_busy_instead_of_permission_denial() {
    let f = Fixture::new().await;
    let context = f
        .app
        .projects
        .open(&factory_projects::ProjectId::parse(&f.a).unwrap())
        .unwrap();
    let held = (0..4)
        .map(|_| context.files.acquire().unwrap())
        .collect::<Vec<_>>();
    let suffix = format!("/status?rootId={}", f.root(&f.a));
    let release = async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        drop(held);
    };
    let ((status, value), _) = tokio::join!(f.call(&f.a, &suffix, "GET", Value::Null), release);
    assert_eq!(status, StatusCode::OK, "{value}");
    let _held = (0..4)
        .map(|_| context.files.acquire().unwrap())
        .collect::<Vec<_>>();
    let (status, value) = f.call(&f.a, &suffix, "GET", Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT, "{value}");
    assert_eq!(value["code"], "git_busy");
}

#[tokio::test]
async fn project_policy_denials_apply_to_manual_git_and_authenticated_remote_reads() {
    let f = Fixture::new().await;
    let path = f._dir.path().join("A");
    let mut config = factory_core::Config::default();
    config.policies.ceiling = Some(factory_policy::PolicyScope {
        preset: Some(factory_policy::PolicyPreset::ReadOnly),
        network: Some(factory_policy::NetworkPolicy {
            mode: factory_policy::NetworkMode::Deny,
        }),
        ..Default::default()
    });
    config.write_atomic(&path).unwrap();
    std::fs::write(
        path.join("ARCHITECTURE.md"),
        "User change remains unmodified by denied operation\n",
    )
    .unwrap();
    let root = f.root(&f.a);
    let before = f.status().await;
    let (status,value)=f.call(&f.a,"/index","POST",json!({"rootId":root,"key":"policy-denied","request":{"expectedGeneration":before["generation"],"paths":["ARCHITECTURE.md"],"unstage":false,"hunk":null,"diffVersion":null,"trustPrograms":true}})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    assert!(f.git(&["diff", "--cached"]).is_empty());
    let (status, value) = f.call(&f.a, "/github?page=1", "GET", Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    assert_eq!(value["code"], "git_operation_denied");
}

#[tokio::test]
async fn checkout_authorizes_changed_paths_of_the_exact_target() {
    let f = Fixture::new().await;
    let project = f._dir.path().join("A");
    let path = project.join("ARCHITECTURE.md");
    let initial = f.git(&["rev-parse", "HEAD"]);
    let original = std::fs::read(&path).unwrap();
    f.git(&["checkout", "-qb", "feature/denied"]);
    std::fs::write(&path, "Protected update\n").unwrap();
    f.git(&["add", "--", "ARCHITECTURE.md"]);
    f.git(&["commit", "-qm", "Change the protected path"]);
    f.git(&["checkout", "-q", "main"]);
    f.git(&["checkout", "-qb", "feature/permitted"]);
    std::fs::write(project.join("allowed.txt"), "Permitted update\n").unwrap();
    f.git(&["add", "--", "allowed.txt"]);
    f.git(&["commit", "-qm", "Add a permitted path"]);
    let permitted = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "-q", "main"]);

    let mut config = factory_core::Config::default();
    config.policies.role_mut("source_control").filesystem =
        Some(factory_policy::FilesystemPolicy {
            read: vec!["**".into()],
            write: vec!["**".into()],
            deny_write: vec!["ARCHITECTURE.md".into()],
        });
    config.write_atomic(&project).unwrap();
    let state = f.status().await;
    let root = f.root(&f.a);
    let request = |name: &str| {
        json!({
            "rootId": root, "key": name.replace('/', "-"),
            "expectedGeneration": state["generation"], "name": name, "create": false
        })
    };
    let (status, body) = f
        .call(&f.a, "/branches", "POST", request("feature/denied"))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "git_operation_denied");
    assert_eq!(f.git(&["rev-parse", "HEAD"]), initial);
    assert_eq!(std::fs::read(&path).unwrap(), original);

    let (status, body) = f
        .call(&f.a, "/branches", "POST", request("feature/permitted"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.git(&["rev-parse", "HEAD"]), permitted);
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[tokio::test]
async fn real_diff_feedback_preserves_reviewer_and_fences_pending_publication() {
    use factory_types::{
        AttemptStatus, ReviewDecision, ReviewResult, TaskEvidence, TaskOperation, TaskState,
    };
    let f = Fixture::new().await;
    let base = f.git(&["rev-parse", "HEAD"]);
    std::fs::write(
        f._dir.path().join("A/ARCHITECTURE.md"),
        "Versioned source for review.\n",
    )
    .unwrap();
    f.git(&["add", "ARCHITECTURE.md"]);
    f.git(&["commit", "-qm", "controlled review fixture"]);
    let head = f.git(&["rev-parse", "HEAD"]);
    let repo = factory_git::Repo::detect(&f._dir.path().join("A")).unwrap();
    let patch = repo.diff_patch(repo.root(), &base, 60_000).unwrap();
    let db = factory_db::FactoryDb::open(&repo.root().join(".factory/db.sqlite3")).unwrap();
    let run = db.create_run("Controlled human review test", None).unwrap();
    let task = db
        .create_task(
            run.id,
            "Source review",
            "Review actual code",
            &[],
            TaskState::Completed,
            0,
            Some("worker"),
            Some(TaskOperation::Implement),
        )
        .unwrap();
    let attempt = db
        .create_task_attempt(
            task,
            "worker",
            Some(TaskOperation::Implement),
            "controlled",
            repo.root().to_str().unwrap(),
            Some(&base),
        )
        .unwrap();
    let review = ReviewResult {
        decision: ReviewDecision::Approve,
        reason: "Controlled automatic review fixture".into(),
        feedback: vec![],
    };
    let evidence = TaskEvidence {
        git_snapshot: Some(factory_types::attempt::GitEvidenceSnapshot {
            base_sha: base.clone(),
            head_sha: head.clone(),
            patch_sha256: factory_git::source_control::digest(&patch),
            patch_truncated: false,
        }),
        changed_files: vec!["ARCHITECTURE.md".into()],
        diff_patch: Some(patch),
        commit_sha: Some(head.clone()),
        ..Default::default()
    };
    db.finish_task_attempt(
        attempt.id,
        AttemptStatus::Approved,
        Some(0),
        Some(&head),
        None,
        Some(&evidence),
        Some(&review),
    )
    .unwrap();
    let record = db
        .begin_integration(
            run.id,
            task,
            attempt.id,
            &base,
            &head,
            "controlled-preserved-candidate",
        )
        .unwrap();
    let anchor = json!({"base":base,"head":head,"path":"ARCHITECTURE.md","line":1,"side":"head"});
    let command = json!({"key":"human-change-request","anchor":anchor,"decision":"request_changes","body":"<img src=x onerror=alert(1)> remains inert user text"});
    let (status, comment) = f
        .call(
            &f.a,
            &format!("/reviews/{}/comments", attempt.id),
            "POST",
            command.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{comment}");
    let (status, replayed) = f
        .call(
            &f.a,
            &format!("/reviews/{}/comments", attempt.id),
            "POST",
            command,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(comment, replayed);
    assert!(db
        .update_integration_record(record, "publishing", Some(&head), &json!({"checks":[]}))
        .is_err());
    assert_eq!(
        db.get_task_attempt(attempt.id).unwrap().unwrap().review,
        Some(review.clone())
    );
    let (status, detail) = f
        .call(
            &f.a,
            &format!("/reviews/{}", attempt.id),
            "GET",
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["patchHashVerified"], true);
    assert_eq!(detail["actualPaths"], json!(["ARCHITECTURE.md"]));
    let (status, _) = f
        .call(
            &f.b,
            &format!("/reviews/{}", attempt.id),
            "GET",
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status,_)=f.call(&f.a,&format!("/reviews/{}/comments",attempt.id),"POST",json!({"key":"forged-anchor","anchor":{"base":"f".repeat(40),"head":head,"path":"ARCHITECTURE.md","line":1,"side":"head"},"decision":"comment","body":"Wrong revision"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status,_)=f.call(&f.a,&format!("/reviews/{}/comments",attempt.id),"POST",json!({"key":"human-resolve","anchor":anchor,"decision":"approve_annotation","body":"Human finding resolved; automated approval still mandatory"})).await;
    assert_eq!(status, StatusCode::OK);
    db.update_integration_record(record, "publishing", Some(&head), &json!({"checks":[]}))
        .unwrap();
    let (status,_)=f.call(&f.a,&format!("/reviews/{}/comments",attempt.id),"POST",json!({"key":"late-human-fence","anchor":anchor,"decision":"request_changes","body":"Publication is already owned"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        db.get_task_attempt(attempt.id).unwrap().unwrap().review,
        Some(review)
    );
}

#[tokio::test]
async fn historical_review_patches_cannot_bypass_current_file_authorization() {
    use factory_types::{AttemptStatus, TaskEvidence, TaskOperation, TaskState};
    for mode in ["hardlink", "protected", "renamed"] {
        let f = Fixture::new().await;
        if mode == "renamed" {
            std::fs::write(
                f._dir.path().join("A/.env"),
                "private fixture value\n".repeat(30),
            )
            .unwrap();
            f.git(&["add", "-f", "--", ".env"]);
            f.git(&["commit", "-qm", "protected original"]);
            f.git(&["mv", ".env", "public.txt"]);
        }
        let base = f.git(&["rev-parse", "HEAD"]);
        let path = match mode {
            "protected" => ".env",
            "renamed" => "public.txt",
            _ => "linked.txt",
        };
        std::fs::write(
            f._dir.path().join("A").join(path),
            if mode == "renamed" {
                format!("{}changed\n", "private fixture value\n".repeat(29))
            } else {
                "private fixture value\n".into()
            },
        )
        .unwrap();
        f.git(&["add", "-f", "--", path]);
        f.git(&["commit", "-qm", "historical protected source"]);
        let head = f.git(&["rev-parse", "HEAD"]);
        let repo = factory_git::Repo::detect(&f._dir.path().join("A")).unwrap();
        let patch = repo.diff_patch(repo.root(), &base, 60_000).unwrap();
        let db = factory_db::FactoryDb::open(&repo.root().join(".factory/db.sqlite3")).unwrap();
        let run = db
            .create_run("Protected historical evidence", None)
            .unwrap();
        let task = db
            .create_task(
                run.id,
                "Evidence",
                "Inspect source",
                &[],
                TaskState::Completed,
                0,
                Some("worker"),
                Some(TaskOperation::Implement),
            )
            .unwrap();
        let attempt = db
            .create_task_attempt(
                task,
                "worker",
                Some(TaskOperation::Implement),
                "controlled",
                repo.root().to_str().unwrap(),
                Some(&base),
            )
            .unwrap();
        let evidence = TaskEvidence {
            git_snapshot: Some(factory_types::attempt::GitEvidenceSnapshot {
                base_sha: base,
                head_sha: head.clone(),
                patch_sha256: factory_git::source_control::digest(&patch),
                patch_truncated: false,
            }),
            changed_files: vec![path.into()],
            diff_patch: Some(patch),
            commit_sha: Some(head.clone()),
            ..Default::default()
        };
        db.finish_task_attempt(
            attempt.id,
            AttemptStatus::Approved,
            Some(0),
            Some(&head),
            None,
            Some(&evidence),
            None,
        )
        .unwrap();
        if mode == "hardlink" {
            std::fs::hard_link(
                f._dir.path().join("A").join(path),
                f._dir.path().join("outside.txt"),
            )
            .unwrap();
        }
        let (status, value) = f
            .call(
                &f.a,
                &format!("/reviews/{}", attempt.id),
                "GET",
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {value}");
        assert!(!value.to_string().contains("private fixture value"));
        assert!(db
            .get_task_attempt(attempt.id)
            .unwrap()
            .unwrap()
            .evidence
            .is_some());
    }
}

#[tokio::test]
async fn first_v2_delivery_uses_preview_base_and_publishes_once_through_controlled_transport() {
    use factory_types::{
        AttemptStatus, ReviewDecision, ReviewResult, RunStatus, TaskOperation, TaskState,
    };
    struct GhEnvironment(Option<std::ffi::OsString>);
    impl Drop for GhEnvironment {
        fn drop(&mut self) {
            if let Some(previous) = &self.0 {
                std::env::set_var("FACTORY_GH_BIN", previous);
            } else {
                std::env::remove_var("FACTORY_GH_BIN");
            }
        }
    }
    let f = Fixture::new().await;
    let base = f.git(&["rev-parse", "HEAD"]);
    let bare = f._dir.path().join("controlled-remote.git");
    let init = std::process::Command::new("git")
        .args(["init", "--bare", "-q", "-b", "main"])
        .arg(&bare)
        .output()
        .unwrap();
    assert!(init.status.success());
    f.git(&["push", bare.to_str().unwrap(), "main"]);
    std::fs::write(
        f._dir.path().join("A/ARCHITECTURE.md"),
        "Controlled reviewed correction\n",
    )
    .unwrap();
    f.git(&["add", "ARCHITECTURE.md"]);
    f.git(&["commit", "-qm", "controlled reviewed correction"]);
    let head = f.git(&["rev-parse", "HEAD"]);
    let provider = f._dir.path().join("gh-provider.py");
    std::fs::write(&provider, r#"import json,pathlib,sys
p=pathlib.Path(__file__).with_name('gh-state.json')
s=json.loads(p.read_text()) if p.exists() else {'creates':0,'prs':[]}
a=sys.argv[1:]
if a[:2]==['auth','status']: print('Logged in to github.com account controlled-fixture (keyring)')
elif a and a[0]=='api': print(json.dumps({'id':7,'full_name':'owner/project','html_url':'https://github.com/owner/project','default_branch':'main','permissions':{'push':True}}))
elif a[:2]==['pr','list']: print(json.dumps(s['prs']))
elif a[:2]==['pr','create']:
 s['creates']+=1;s['prs']=[{'number':12,'url':'https://github.com/owner/project/pull/12','state':'OPEN','isDraft':'--draft' in a}];p.write_text(json.dumps(s));print(s['prs'][0]['url'])
else: raise RuntimeError('Unexpected controlled provider request')
"#).unwrap();
    let relay = f._dir.path().join("ssh-relay.py");
    std::fs::write(
        &relay,
        format!(
            r#"import json,pathlib,subprocess,sys
p=pathlib.Path(__file__).with_name('transport-calls.jsonl')
with p.open('a') as f: f.write(json.dumps(sys.argv[1:])+'\n')
operation=sys.argv[-1]
if operation.startswith('git-receive-pack '): program='receive-pack'
elif operation.startswith('git-upload-pack '): program='upload-pack'
else: raise RuntimeError('Unexpected controlled transport operation')
sys.exit(subprocess.call(['git',program,{}]))
"#,
            serde_json::to_string(bare.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let launcher = f._dir.path().join(if cfg!(windows) {
        "gh-provider.cmd"
    } else {
        "gh-provider.sh"
    });
    if cfg!(windows) {
        std::fs::write(
            &launcher,
            format!("@echo off\r\npython \"{}\" %*\r\n", provider.display()),
        )
        .unwrap();
    } else {
        std::fs::write(
            &launcher,
            format!("#!/bin/sh\nexec python3 '{}' \"$@\"\n", provider.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let _environment = GhEnvironment(std::env::var_os("FACTORY_GH_BIN"));
    std::env::set_var("FACTORY_GH_BIN", &launcher);
    f.git(&[
        "remote",
        "add",
        "origin",
        "git@github.com:owner/project.git",
    ]);
    let ssh = format!(
        "{} \"{}\"",
        if cfg!(windows) { "python" } else { "python3" },
        relay.to_string_lossy().replace('\\', "/")
    );
    f.git(&["config", "core.sshCommand", &ssh]);
    f.git(&["config", "ssh.variant", "ssh"]);
    let db = factory_db::FactoryDb::open(&f._dir.path().join("A/.factory/db.sqlite3")).unwrap();
    let run = db
        .create_run("Controlled production delivery API", None)
        .unwrap();
    let task = db
        .create_task(
            run.id,
            "Reviewed correction",
            "Correct source",
            &[],
            TaskState::Completed,
            0,
            Some("worker"),
            Some(TaskOperation::Implement),
        )
        .unwrap();
    let attempt = db
        .create_task_attempt(
            task,
            "worker",
            Some(TaskOperation::Implement),
            "controlled",
            f._dir.path().join("A").to_str().unwrap(),
            Some(&base),
        )
        .unwrap();
    db.finish_task_attempt(
        attempt.id,
        AttemptStatus::Approved,
        Some(0),
        Some(&head),
        None,
        None,
        Some(&ReviewResult {
            decision: ReviewDecision::Approve,
            reason: "Controlled approval fixture".into(),
            feedback: vec![],
        }),
    )
    .unwrap();
    let record = db
        .begin_integration(
            run.id,
            task,
            attempt.id,
            &base,
            &head,
            "controlled candidate",
        )
        .unwrap();
    db.update_integration_record(record, "published", Some(&head), &json!({"checks":[]}))
        .unwrap();
    db.set_run_integration(run.id, Some(&head)).unwrap();
    db.set_run_status(run.id, RunStatus::Completed).unwrap();
    f.git(&["branch", &format!("factory/run-{}", run.id), &head]);
    let route = format!("/delivery/{}", run.id);
    let (status, preview) = f.call(&f.a, &route, "GET", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["delivery"]["baseBranch"], Value::Null);
    assert_eq!(preview["changedFiles"], json!(["ARCHITECTURE.md"]));
    assert_eq!(preview["baseSource"], "first_integration_journal");
    assert_eq!(preview["publishedByPreview"], false);
    assert!(!f._dir.path().join("transport-calls.jsonl").exists());
    let request = json!({"key":"explicit-controlled-delivery","authorize":true,"repository":"owner/project","repositoryId":7,"expectedIntegration":head,"base":"main","head":format!("factory/run-{}",run.id),"title":"Controlled correction","body":"Controlled provider contract; no external publication","draft":true});
    let mut wrong = request.clone();
    wrong["repositoryId"] = json!(8);
    assert_eq!(
        f.call(&f.a, &route, "POST", wrong).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, started) = f.call(&f.a, &route, "POST", request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["started"], true);
    for _ in 0..200 {
        if db.get_delivery(run.id).unwrap().unwrap().state.as_str() == "published" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let delivery = db.get_delivery(run.id).unwrap().unwrap();
    assert_eq!(delivery.state.as_str(), "published", "{delivery:?}");
    assert_eq!(delivery.pull_request.unwrap().number, 12);
    let (status, duplicate) = f.call(&f.a, &route, "POST", request).await;
    assert_eq!(status, StatusCode::OK, "{duplicate}");
    assert_eq!(duplicate["replayed"], false);
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(f._dir.path().join("gh-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["creates"], 1);
    let calls = std::fs::read_to_string(f._dir.path().join("transport-calls.jsonl")).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("git-receive-pack"))
            .count(),
        1
    );
    let remote_head = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(&bare)
        .args(["rev-parse", &format!("refs/heads/factory/run-{}", run.id)])
        .output()
        .unwrap();
    assert!(remote_head.status.success());
    assert_eq!(String::from_utf8(remote_head.stdout).unwrap().trim(), head);
    assert!(f
        .git(&["-C", f._dir.path().join("B").to_str().unwrap(), "remote"])
        .is_empty());
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app = Arc::new(
            FactoryApplication::new(ApplicationOptions {
                registry_path: dir.path().join("registry.sqlite3"),
                allowed_roots: vec![dir.path().into()],
                default_root: None,
                dashboard_root: dir.path().into(),
                limits: ProjectLimits::default(),
                api: ApiOptions::default(),
            })
            .unwrap(),
        );
        let mut ids = vec![];
        for name in ["A", "B"] {
            let path = dir.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(
                path.join("ARCHITECTURE.md"),
                format!("Project {name} database access uses repository adapters.\n"),
            )
            .unwrap();
            std::fs::write(path.join(".gitignore"), ".factory/\n").unwrap();
            for args in [
                &["init", "-q", "-b", "main"][..],
                &["config", "user.name", "Memory API"],
                &["config", "user.email", "memory@example.invalid"],
                &["config", "core.autocrlf", "false"],
                &["config", "commit.gpgsign", "false"],
                &["add", "."],
                &["commit", "-qm", "baseline"],
            ] {
                assert!(std::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success());
            }
            factory_core::Factory::init(&path).unwrap();
            factory_core::Config::default().write_atomic(&path).unwrap();
            let id = app.projects.register(&path, None).unwrap().project_id;
            app.projects.open(&id).unwrap();
            ids.push(id.to_string());
        }
        let router = application_router(app.clone());
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
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        Self {
            _dir: dir,
            router,
            a: ids[0].clone(),
            b: ids[1].clone(),
            cookie,
            csrf: body["csrfToken"].as_str().unwrap().into(),
            app,
        }
    }
    async fn call(
        &self,
        project: &str,
        path: &str,
        method: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        self.raw(
            &format!("/api/projects/{project}/source-control{path}"),
            method,
            body,
            true,
            true,
            "http://127.0.0.1:4321",
        )
        .await
    }
    async fn raw(
        &self,
        path: &str,
        method: &str,
        body: Value,
        auth: bool,
        csrf: bool,
        origin: &str,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:4321")
            .header("origin", origin)
            .header("content-type", "application/json");
        if auth {
            req = req.header("cookie", &self.cookie);
        }
        if csrf {
            req = req.header("x-factory-csrf", &self.csrf);
        }
        let response = self
            .router
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let text = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&text).unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn reserved_ref_case_aliases_cannot_bypass_authenticated_mutation_guards() {
    let f = Fixture::new().await;
    let root = f.root(&f.a);
    f.git(&["branch", "factory/reserved"]);
    let initial = f.git(&["rev-parse", "HEAD"]);
    for (index, name) in ["Factory/reserved", "FACTORY/reserved", "FaCtOrY/reserved"]
        .into_iter()
        .enumerate()
    {
        let state = f.status().await;
        let (status, value) = f.call(&f.a, "/branches", "POST", json!({"rootId":root,"key":format!("case-{index}"),"expectedGeneration":state["generation"],"name":name,"create":false,"trustPrograms":false,"dirtyBuffers":false})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert_eq!(f.git(&["branch", "--show-current"]), "main");
    }
    let owned = f._dir.path().join("owned");
    f.git(&[
        "worktree",
        "add",
        "-b",
        "feature/owned",
        owned.to_str().unwrap(),
    ]);
    for (index, (name, create, expected)) in [
        ("Feature/owned", false, StatusCode::BAD_REQUEST),
        ("FEATURE/owned", true, StatusCode::BAD_REQUEST),
        ("feature/owned", false, StatusCode::FORBIDDEN),
    ]
    .into_iter()
    .enumerate()
    {
        let state = f.status().await;
        let (status, value) = f.call(&f.a, "/branches", "POST", json!({"rootId":root,"key":format!("owned-{index}"),"expectedGeneration":state["generation"],"name":name,"create":create,"trustPrograms":false,"dirtyBuffers":false})).await;
        assert_eq!(status, expected, "{value}");
        assert_eq!(f.git(&["branch", "--show-current"]), "main");
    }
    #[cfg(windows)]
    {
        let unicode = f._dir.path().join("unicode-owned");
        f.git(&[
            "worktree",
            "add",
            "-b",
            "f\u{e9}ature/owned",
            unicode.to_str().unwrap(),
        ]);
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&unicode)
            .args(["symbolic-ref", "HEAD", "refs/heads/f\u{c9}ature/owned"])
            .output()
            .unwrap()
            .status
            .success());
        let state = f.status().await;
        let (status, value) = f.call(&f.a, "/branches", "POST", json!({"rootId":root,"key":"unicode-owned","expectedGeneration":state["generation"],"name":"f\u{e9}ature/owned","create":false,"trustPrograms":false,"dirtyBuffers":false})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
        assert_eq!(f.git(&["branch", "--show-current"]), "main");
    }
    f.git(&["checkout", "-q", "-b", "FaCtOrY/current"]);
    std::fs::write(
        f._dir.path().join("A/ARCHITECTURE.md"),
        "Externally staged evidence must survive denial\n",
    )
    .unwrap();
    f.git(&["add", "ARCHITECTURE.md"]);
    let staged = f.git(&["diff", "--cached"]);
    let state = f.status().await;
    let (status, value) = f.call(&f.a, "/commit", "POST", json!({"rootId":root,"key":"case-commit","expectedGeneration":state["generation"],"message":"Denied protected alias commit","trustPrograms":false})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    assert_eq!(f.git(&["diff", "--cached"]), staged);
    assert_eq!(f.git(&["rev-parse", "HEAD"]), initial);
}
