use factory_core::{AgentEntry, Config, Factory, ProjectExecution, TaskBudget, WorkflowResult};
use factory_db::FactoryDb;
use factory_types::memory::*;
use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

const PROJECT: &str = "p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SCRIPT: &str = r#"
import json,pathlib,sys,time,os
text=sys.stdin.read()
mode=sys.argv[1]
marker='UNTRUSTED_PROJECT_MEMORY_JSON\n'
mem=json.loads(text.split(marker)[1].split('\nEND_PROJECT_MEMORY')[0]) if marker in text else {'records':[]}
if 'ASF_MEMORY_EXTRACTION_V1' in text:
 if 'QUOTA_CODEX_FIXTURE' in text or os.environ.get('ASF_TEST_MEMORY_QUOTA')=='1':
  sys.stderr.write("OpenAI Codex v0.0.0-controlled-fixture\nERROR: You've hit your usage limit. Try again later.\n");sys.exit(1)
 data=json.loads(text.split('UNTRUSTED_EVIDENCE_JSON\n')[1].split('\nEND_EVIDENCE')[0])
 if 'INVALID_EXTRACT_FIXTURE' in text: print('not valid JSON');sys.exit(0)
 quote=data['sources'][0]['content'][:40]
 print(json.dumps({'candidates':[{'category':'general_facts','title':'Database integration observation','content':'The recorded integration contains database task evidence.','subject':'integration_observation','scope':'api','citations':[{'sourceIndex':0,'quote':quote}]}]}));sys.exit(0)
if 'ASF_ORCHESTRATOR_V1' in text:
 print(json.dumps({'action':'respond','content':'Supplied project memories: '+','.join(m['id']+':'+str(m['revision']) for m in mem['records'])}));sys.exit(0)
if mode=='planner':
 print(json.dumps({'objective':'Database adapter fixture','tasks':[{'id':'database','title':'Database adapter','objective':'Implement database repository adapter fixture','dependencies':[],'acceptanceCriteria':['source change recorded'],'role':'worker'}]}))
elif mode=='reviewer':
 print(json.dumps({'decision':'approve','reason':'Controlled review inspects actual captured change; memory references '+','.join(m['id'] for m in mem['records']),'feedback':[]}))
else:
 gate=os.environ.get('ASF_TEST_MEMORY_HANDOFF_GATE')
 if gate:
  deadline=time.monotonic()+25
  while not pathlib.Path(gate).exists() and time.monotonic()<deadline: time.sleep(0.02)
  if not pathlib.Path(gate).exists(): raise RuntimeError('Controlled handoff gate deadline exceeded')
 elif 'LONG_BOUNDARY' in text: time.sleep(1.5)
 filename='replacement.py' if 'REPLACEMENT_MEMORY' in text else 'result.py'
 pathlib.Path(filename).write_text('ADAPTER = True\n',encoding='utf8')
 print(json.dumps({'commands':[],'summary':'Controlled source change; supplied memory revisions '+','.join(str(m['revision']) for m in mem['records']),'tests':[]}))
"#;
fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
fn fixture() -> (tempfile::TempDir, Arc<ProjectExecution>, Factory) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".gitignore"), ".factory/\n__pycache__/\n").unwrap();
    std::fs::write(
        dir.path().join("ARCHITECTURE.md"),
        "Database access uses repository adapters.\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("result.py"), "ADAPTER = False\n").unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Memory Runtime Test"]);
    git(
        dir.path(),
        &["config", "user.email", "memory@example.invalid"],
    );
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "baseline"]);
    Factory::init(dir.path()).unwrap();
    let script = dir.path().join(".factory/memory-agent.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let python = ["python", "python3"]
        .into_iter()
        .find(|p| {
            Command::new(p)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .unwrap();
    let mut config = Config::default();
    for role in ["planner", "worker", "reviewer"] {
        config.agents.insert(
            role.into(),
            AgentEntry {
                kind: None,
                command: python.into(),
                args: vec![script.to_string_lossy().into_owned(), role.into()],
                env: BTreeMap::from([("PYTHONDONTWRITEBYTECODE".into(), "1".into())]),
                prompt_transport: Some(factory_agent::PromptTransport::Stdin),
                interactive_args: None,
                capabilities: vec![],
                max_concurrency: Some(2),
            },
        );
        config.role_assignments.push(factory_core::RoleAssignment {
            role: role.into(),
            agent: role.into(),
            preferred: true,
        });
    }
    config.policies.roles.insert(
        "reviewer".into(),
        factory_policy::PolicyPreset::Review.scope(),
    );
    config.write_atomic(dir.path()).unwrap();
    let context = Arc::new(ProjectExecution::new(
        dir.path().to_string_lossy().into_owned(),
        Arc::new(TaskBudget::new(2)),
    ));
    context.bind_project_id(PROJECT).unwrap();
    let f = Factory::open_with_execution(dir.path(), context.clone()).unwrap();
    (dir, context, f)
}
fn database(root: &Path) -> FactoryDb {
    FactoryDb::open(&root.join(".factory/db.sqlite3")).unwrap()
}
fn memory(f: &Factory) -> ProjectMemory {
    f.project_memory()
        .unwrap()
        .create(
            &MemoryInput {
                category: MemoryCategory::CodingConventions,
                title: "Database repository adapters".into(),
                content: "Use repository adapters for database access.".into(),
                subject: "data_access".into(),
                scope: "api".into(),
                status: MemoryStatus::Active,
                sources: vec![MemorySource {
                    project_id: PROJECT.into(),
                    reference: MemorySourceRef::Manual {
                        note: "Explicit test project convention".into(),
                    },
                }],
            },
            "test fixture",
        )
        .unwrap()
}
fn input(m: &ProjectMemory) -> MemoryInput {
    MemoryInput {
        category: m.category,
        title: m.title.clone(),
        content: m.content.clone(),
        subject: m.subject.clone(),
        scope: m.scope.clone(),
        status: m.status,
        sources: m.provenance.iter().map(|p| p.source.clone()).collect(),
    }
}

#[test]
fn actual_command_invocations_receive_current_revision_and_keep_prior_audit_immutable() {
    let (dir, _context, f) = fixture();
    let m = memory(&f);
    let cancel = AtomicBool::new(false);
    let (first, response) = f
        .converse_with_memory_query("ASF_ORCHESTRATOR_V1", "database adapters", &cancel)
        .unwrap();
    assert!(response.contains(&m.id));
    let mut d = input(&m);
    d.content.push_str(" Verify compatibility.");
    let n = f
        .project_memory()
        .unwrap()
        .update(&m.id, 1, &d, "correct")
        .unwrap();
    let (second, response) = f
        .converse_with_memory_query("ASF_ORCHESTRATOR_V1", "database adapters", &cancel)
        .unwrap();
    assert!(response.contains(":2"));
    d.status = MemoryStatus::Disabled;
    f.project_memory()
        .unwrap()
        .update(&m.id, n.revision, &d, "disable")
        .unwrap();
    let (third, response) = f
        .converse_with_memory_query("ASF_ORCHESTRATOR_V1", "database adapters", &cancel)
        .unwrap();
    assert!(!response.contains(&m.id));
    let db = database(dir.path());
    assert_eq!(
        db.memory_usage(PROJECT, Some(first), None).unwrap()[0].memories[0].revision,
        1
    );
    assert_eq!(
        db.memory_usage(PROJECT, Some(second), None).unwrap()[0].memories[0].revision,
        2
    );
    assert!(db.memory_usage(PROJECT, Some(third), None).unwrap()[0]
        .memories
        .is_empty());
}
#[test]
fn planner_worker_and_read_only_reviewer_receive_audited_memory_through_existing_engine() {
    let (dir, _context, f) = fixture();
    let m = memory(&f);
    let run = f
        .begin_run("Implement database repository adapter", None)
        .unwrap();
    let cancel = AtomicBool::new(false);
    f.plan_run(run.id, &cancel).unwrap();
    f.prepare_start(run.id).unwrap();
    assert_eq!(
        f.execute_active_run(run.id, &cancel).unwrap(),
        WorkflowResult::Completed
    );
    let db = database(dir.path());
    let sessions = db.list_agent_sessions(Some(run.id)).unwrap();
    for role in ["planner", "worker", "reviewer"] {
        let session = sessions.iter().find(|s| s.role == role).unwrap();
        let usage = db.memory_usage(PROJECT, Some(session.id), None).unwrap();
        assert_eq!(usage[0].memories[0].memory_id, m.id);
        if role == "reviewer" {
            assert_eq!(
                session.policy_audit.as_ref().unwrap().filesystem,
                "read_only"
            );
        }
    }
    let attempt = db
        .list_task_attempts(run.id)
        .unwrap()
        .into_iter()
        .find(|a| a.review.is_some())
        .unwrap();
    let service = f.project_memory().unwrap();
    for reference in [
        MemorySourceRef::Mission { run_id: run.id },
        MemorySourceRef::TaskAttempt {
            attempt_id: attempt.id,
        },
        MemorySourceRef::Review {
            attempt_id: attempt.id,
        },
        MemorySourceRef::AgentSession {
            session_id: sessions[0].id,
        },
        MemorySourceRef::IntegrationCommit {
            run_id: run.id,
            commit_sha: db.get_run_integration(run.id).unwrap().unwrap(),
        },
    ] {
        let source = MemorySource {
            project_id: PROJECT.into(),
            reference,
        };
        let evidence = service
            .resolve_source(&source, None)
            .unwrap_or_else(|e| panic!("{source:?}: {e}"));
        assert!(!evidence.provenance.source_revision.is_empty());
        assert!(!evidence.content.is_empty());
        let forged = MemorySource {
            project_id: "p_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            ..source
        };
        assert!(service.resolve_source(&forged, None).is_err());
    }
    assert!(db.get_run_integration(run.id).unwrap().is_some());
    assert!(db.list_task_attempts(run.id).unwrap().iter().any(|a| a
        .review
        .as_ref()
        .is_some_and(|r| r.decision == factory_types::ReviewDecision::Approve)));
}
#[test]
fn extractor_quota_is_classified_linked_and_never_automatically_retried() {
    let (dir, context, f) = fixture();
    let mut config = Config::load(dir.path()).unwrap();
    config.agents.get_mut("planner").unwrap().kind = Some(factory_agent::AgentKind::Codex);
    config.write_atomic(dir.path()).unwrap();
    drop(f);
    let f = Factory::open_with_execution(dir.path(), context).unwrap();
    let mut settings = f.project_memory().unwrap().settings().unwrap();
    settings.extraction_mode = ExtractionMode::Review;
    f.project_memory()
        .unwrap()
        .save_settings(&settings)
        .unwrap();
    std::fs::write(dir.path().join("ARCHITECTURE.md"), "QUOTA_CODEX_FIXTURE").unwrap();
    let root = factory_files_source_root(&f, dir.path());
    let source = MemorySource {
        project_id: PROJECT.into(),
        reference: MemorySourceRef::RepositoryFile {
            root_id: root,
            path: "ARCHITECTURE.md".into(),
            revision: None,
        },
    };
    assert!(matches!(
        f.extract_project_memory(std::slice::from_ref(&source), &AtomicBool::new(false)),
        Err(factory_core::FactoryError::AgentResourceExhausted(_))
    ));
    let db = database(dir.path());
    let e = &db.memory_extractions(PROJECT).unwrap()[0];
    assert!(e
        .error
        .as_deref()
        .unwrap()
        .starts_with("PROVIDER_USAGE_LIMIT"));
    assert!(e.agent_session_id.is_some());
    let count = db.list_agent_sessions(None).unwrap().len();
    let again = f
        .extract_project_memory(&[source], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(again["state"], "already_recorded");
    assert_eq!(db.list_agent_sessions(None).unwrap().len(), count);
}
fn factory_files_source_root(f: &Factory, root: &Path) -> String {
    let _ = f.project_memory().unwrap();
    factory_files::FileService::new(PROJECT, root, factory_files::Limits::default())
        .unwrap()
        .primary_root()
        .id
}
#[test]
fn safe_profile_handoff_retrieves_new_revision_without_rewriting_original_attempt() {
    let (dir, context, f) = fixture();
    let m = memory(&f);
    struct ReleaseGate(std::path::PathBuf);
    impl Drop for ReleaseGate {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.0, "release owned fixture");
        }
    }
    let gate = ReleaseGate(dir.path().join(".factory/memory-handoff-gate"));
    let mut config = Config::load(dir.path()).unwrap();
    config.agents.get_mut("worker").unwrap().env.insert(
        "ASF_TEST_MEMORY_HANDOFF_GATE".into(),
        gate.0.to_string_lossy().into_owned(),
    );
    config.write_atomic(dir.path()).unwrap();
    drop(f);
    let f = Factory::open_with_execution(dir.path(), context.clone()).unwrap();
    let profile =
        |name: &str, instructions: &str, preferred: bool| factory_types::AgentProfileInput {
            name: name.into(),
            description: String::new(),
            provider_adapter_id: "worker".into(),
            model_id: None,
            reasoning_effort: None,
            roles: vec!["worker".into()],
            instructions: instructions.into(),
            permissions: factory_policy::PolicyScope::default(),
            max_concurrency: 2,
            timeout_secs: 30,
            enabled: true,
            automatic_delegation: true,
            preferred,
        };
    let a = f
        .save_agent_profile(None, None, profile("Memory A", "", true))
        .unwrap();
    let b = f
        .save_agent_profile(None, None, profile("Memory B", "REPLACEMENT_MEMORY", false))
        .unwrap();
    let current = Factory::open_with_execution(dir.path(), context.clone()).unwrap();
    let run = current
        .begin_run("Database adapter LONG_BOUNDARY", None)
        .unwrap();
    let cancel = AtomicBool::new(false);
    current.plan_run(run.id, &cancel).unwrap();
    current.prepare_start(run.id).unwrap();
    let task = current.list_tasks(run.id).unwrap()[0].id;
    let root = dir.path().to_owned();
    let running = context.clone();
    let handle = std::thread::spawn(move || {
        Factory::open_with_execution(&root, running)
            .unwrap()
            .execute_active_run(run.id, &AtomicBool::new(false))
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if database(dir.path())
            .list_agent_sessions(Some(run.id))
            .unwrap()
            .iter()
            .any(|s| s.role == "worker" && s.status == "running")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut d = input(&m);
    d.content.push_str(" New continuation convention.");
    current
        .project_memory()
        .unwrap()
        .update(&m.id, 1, &d, "new handoff context")
        .unwrap();
    current
        .request_agent_handoff(
            run.id,
            task,
            factory_types::HandoffRequest {
                idempotency_key: "memory-handoff".into(),
                to_profile_id: b.id.clone(),
                expected_profile_revision: b.revision,
                reason: "Continue with current memory".into(),
            },
        )
        .unwrap();
    std::fs::write(&gate.0, "authorized handoff recorded").unwrap();
    assert_eq!(handle.join().unwrap().unwrap(), WorkflowResult::Completed);
    let db = database(dir.path());
    let workers = db
        .list_agent_sessions(Some(run.id))
        .unwrap()
        .into_iter()
        .filter(|s| s.role == "worker")
        .collect::<Vec<_>>();
    assert_eq!(workers.len(), 2);
    assert_eq!(workers[0].agent, a.id);
    assert_eq!(workers[1].agent, b.id);
    assert_eq!(
        db.memory_usage(PROJECT, Some(workers[0].id), None).unwrap()[0].memories[0].revision,
        1
    );
    assert_eq!(
        db.memory_usage(PROJECT, Some(workers[1].id), None).unwrap()[0].memories[0].revision,
        2
    );
    assert_eq!(db.handoffs(run.id).unwrap()[0].state, "completed");
}

#[test]
fn automatic_completion_extracts_once_without_corrupting_mission_on_provider_refusal() {
    for quota in [false, true] {
        let (dir, context, old) = fixture();
        drop(old);
        if quota {
            let mut config = Config::load(dir.path()).unwrap();
            let planner = config.agents.get_mut("planner").unwrap();
            planner.kind = Some(factory_agent::AgentKind::Codex);
            planner
                .env
                .insert("ASF_TEST_MEMORY_QUOTA".into(), "1".into());
            config.write_atomic(dir.path()).unwrap();
        }
        let f = Factory::open_with_execution(dir.path(), context).unwrap();
        let mut settings = f.project_memory().unwrap().settings().unwrap();
        settings.extraction_mode = ExtractionMode::Review;
        f.project_memory()
            .unwrap()
            .save_settings(&settings)
            .unwrap();
        let run = f
            .begin_run("Implement database repository adapter", None)
            .unwrap();
        let cancel = AtomicBool::new(false);
        f.plan_run(run.id, &cancel).unwrap();
        f.prepare_start(run.id).unwrap();
        assert_eq!(
            f.execute_active_run(run.id, &cancel).unwrap(),
            WorkflowResult::Completed
        );
        let db = database(dir.path());
        let extractions = db.memory_extractions(PROJECT).unwrap();
        assert_eq!(extractions.len(), 1);
        assert_eq!(
            extractions[0].state,
            if quota { "failed" } else { "completed" }
        );
        if quota {
            assert!(extractions[0]
                .error
                .as_deref()
                .unwrap()
                .starts_with("PROVIDER_USAGE_LIMIT"));
        } else {
            assert_eq!(db.memory_candidates(PROJECT, None).unwrap().0.len(), 1);
        }
        let sessions = db.list_agent_sessions(Some(run.id)).unwrap();
        assert_eq!(sessions.iter().filter(|s| s.role == "planner").count(), 2);
        let repeated = f
            .extract_project_memory(
                &[MemorySource {
                    project_id: PROJECT.into(),
                    reference: MemorySourceRef::IntegrationCommit {
                        run_id: run.id,
                        commit_sha: db.get_run_integration(run.id).unwrap().unwrap(),
                    },
                }],
                &cancel,
            )
            .unwrap();
        assert_eq!(repeated["state"], "already_recorded");
        assert_eq!(db.list_agent_sessions(None).unwrap().len(), sessions.len());
        assert_eq!(
            db.get_run(run.id).unwrap().unwrap().status,
            factory_types::RunStatus::Completed
        );
    }
}

#[test]
fn extractor_cannot_receive_file_evidence_outside_its_resolved_read_policy() {
    for profile_restriction in [false, true] {
        let (dir, context, old) = fixture();
        drop(old);
        std::fs::write(
            dir.path().join("PUBLIC.md"),
            "Public database adapter testing convention.",
        )
        .unwrap();
        let restriction = factory_policy::FilesystemPolicy {
            read: vec!["PUBLIC.md".into()],
            write: vec![],
            deny_write: vec![],
        };
        if !profile_restriction {
            let mut config = Config::load(dir.path()).unwrap();
            config
                .policies
                .roles
                .entry("planner".into())
                .or_default()
                .filesystem = Some(restriction.clone());
            config.write_atomic(dir.path()).unwrap();
        }
        if profile_restriction {
            let f = Factory::open_with_execution(dir.path(), context.clone()).unwrap();
            f.save_agent_profile(
                None,
                None,
                factory_types::AgentProfileInput {
                    name: "Limited extractor".into(),
                    description: String::new(),
                    provider_adapter_id: "planner".into(),
                    model_id: None,
                    reasoning_effort: None,
                    roles: vec!["planner".into()],
                    instructions: String::new(),
                    permissions: factory_policy::PolicyScope {
                        filesystem: Some(restriction),
                        ..Default::default()
                    },
                    max_concurrency: 1,
                    timeout_secs: 30,
                    enabled: true,
                    automatic_delegation: true,
                    preferred: true,
                },
            )
            .unwrap();
        }
        let f = Factory::open_with_execution(dir.path(), context).unwrap();
        let mut settings = f.project_memory().unwrap().settings().unwrap();
        settings.extraction_mode = ExtractionMode::Review;
        f.project_memory()
            .unwrap()
            .save_settings(&settings)
            .unwrap();
        let root = factory_files_source_root(&f, dir.path());
        let denied = MemorySource {
            project_id: PROJECT.into(),
            reference: MemorySourceRef::RepositoryFile {
                root_id: root.clone(),
                path: "ARCHITECTURE.md".into(),
                revision: None,
            },
        };
        let outcome = f.extract_project_memory(&[denied], &AtomicBool::new(false));
        assert!(
            matches!(
                outcome,
                Err(factory_core::FactoryError::Memory(
                    factory_memory::Error::Policy
                ))
            ),
            "denied file reached the extractor: {outcome:?}"
        );
        let db = database(dir.path());
        let sessions = db.list_agent_sessions(None).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "failed");
        assert!(sessions[0]
            .output_meta
            .as_ref()
            .and_then(|m| m.process_id)
            .is_none());
        assert!(sessions[0].stdout.as_deref().unwrap_or("").is_empty());
        assert_eq!(db.memory_extractions(PROJECT).unwrap()[0].state, "failed");
        assert_eq!(
            db.memory_extractions(PROJECT).unwrap()[0].agent_session_id,
            Some(sessions[0].id)
        );
        assert!(db.memory_candidates(PROJECT, None).unwrap().0.is_empty());
        let allowed = MemorySource {
            project_id: PROJECT.into(),
            reference: MemorySourceRef::RepositoryFile {
                root_id: root,
                path: "PUBLIC.md".into(),
                revision: None,
            },
        };
        let outcome = f.extract_project_memory(&[allowed], &AtomicBool::new(false));
        assert!(
            outcome.is_ok(),
            "allowed source was rejected (profile restriction {profile_restriction}): {outcome:?}"
        );
        assert_eq!(
            db.list_agent_sessions(None)
                .unwrap()
                .iter()
                .filter(|s| s.status == "success")
                .count(),
            1
        );
    }
}
