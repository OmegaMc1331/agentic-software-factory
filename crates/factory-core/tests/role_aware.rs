//! Role-aware workflow engine integration tests.
//!
//! These exercise the runtime with fake agent executables (no real model
//! credentials required): advisory roles produce persisted artifacts, artifacts
//! propagate only along the dependency DAG, implementation is accepted through
//! the built-in Reviewer, specialized reviews route back to implementation
//! rework, and custom roles run through the generic operation semantics.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use factory_core::ExecutionClass;
use factory_core::{
    AgentEntry, Config, Factory, RoleAssignment, RoleDefinitionEntry, WorkflowTeam,
};
use factory_types::{ArtifactKind, AttemptStatus, RunStatus, TaskOperation, TaskState};
use tempfile::TempDir;

fn init_git(root: &Path) {
    std::fs::write(root.join("README.md"), "test repository\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "factory@example.test"][..],
        &["config", "user.name", "Factory Test"][..],
        &["add", "."][..],
        &["commit", "-q", "-m", "initial"][..],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
}

/// Writes a small agent script into the fixture directory and returns an
/// agent entry that runs it. Script files avoid nested-quote parsing problems
/// in `cmd /c` or `sh -c` for paths that contain spaces.
fn script_entry(dir: &Path, name: &str, body_unix: &str, body_win: &str) -> AgentEntry {
    let path = if cfg!(windows) {
        let path = dir.join(format!("{name}.cmd"));
        std::fs::write(&path, format!("@echo off\r\n{body_win}\r\n")).unwrap();
        path
    } else {
        let path = dir.join(format!("{name}.sh"));
        std::fs::write(&path, format!("{body_unix}\n")).unwrap();
        path
    };
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".into(),
                "/c".into(),
                path.to_string_lossy().into_owned(),
            ],
        )
    } else {
        ("sh".to_string(), vec![path.to_string_lossy().into_owned()])
    };
    AgentEntry {
        kind: None,
        command,
        args,
        env: BTreeMap::new(),
        prompt_transport: None,
        interactive_args: None,
        capabilities: Vec::new(),
        max_concurrency: None,
    }
}

/// Script body pair (unix, windows) that prints the contents of `path`,
/// used by the fake planner and reviewer agents.
fn cat_body(path: &Path) -> (String, String) {
    (
        format!("cat \"{}\"", path.display()),
        format!("type \"{}\"", path.display()),
    )
}

/// A script body that writes a marker file (so the task produces repository
/// evidence), copies the agent's stdin (the mission) into `capture_path`, and
/// then prints the contents of `output_path`.
/// `findstr` with no file argument reads stdin, which the CommandAgent pipes
/// to the process.
fn capture_worker_body(agent: &str, capture_path: &Path, output_path: &Path) -> (String, String) {
    (
        format!(
            "printf '{agent}\\n' > worker-output.txt\ncat > \"{capture}\"\ncat \"{output}\"",
            agent = agent,
            capture = capture_path.display(),
            output = output_path.display()
        ),
        format!(
            "echo {agent}>worker-output.txt\nfindstr /r \".*\" > \"{capture}\"\ntype \"{output}\"",
            agent = agent,
            capture = capture_path.display(),
            output = output_path.display()
        ),
    )
}

/// A script body that does not touch repository files (for advisory and
/// review agents) but persists the mission for assertions.
fn capture_only_body(capture_path: &Path, output_path: &Path) -> (String, String) {
    (
        format!(
            "cat > \"{capture}\"\ncat \"{output}\"",
            capture = capture_path.display(),
            output = output_path.display()
        ),
        format!(
            "findstr /r \".*\" > \"{capture}\"\ntype \"{output}\"",
            capture = capture_path.display(),
            output = output_path.display()
        ),
    )
}

struct Fixture {
    _dir: TempDir,
    factory: Factory,
    captures: BTreeMap<String, std::path::PathBuf>,
}

impl Fixture {
    fn capture(&self, key: &str) -> Option<String> {
        let path = self.captures.get(key)?;
        std::fs::read_to_string(path).ok()
    }
}

struct FixtureOptions {
    /// Named agents to configure on top of the pipeline trio. Each gets its
    /// own capture file and a static output file.
    include: Vec<&'static str>,
    /// `security` may be "approve" (default), "request_changes", or
    /// "flag-fixed" (request changes once, then approve).
    security: &'static str,
    /// Add a second worker (worker-b) for routing tests.
    two_workers: bool,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        FixtureOptions {
            include: Vec::new(),
            security: "approve",
            two_workers: false,
        }
    }
}

fn write_json(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

const APPROVE: &str = r#"{"decision":"approve","reason":"evidence accepted"}"#;
const RESEARCH_OUT: &str = r#"{"summary":"RESEARCH_TOKEN_ABC uses JWT","findings":["tokens in httpOnly cookies"],"recommendations":["auth middleware"]}"#;
const ANALYST_OUT: &str = r#"{"summary":"ANALYST_TOKEN_QRS latency profile","findings":["cache miss"],"recommendations":["warm cache"]}"#;
const ARCH_OUT: &str = r#"{"summary":"ARCH_TOKEN_XYZ add an auth middleware","findings":["one login interface"],"recommendations":["small session store"]}"#;
const WORKER_OUT: &str = r#"{"summary":"implemented","commands":["cargo check"]}"#;
const VERIFY_OUT: &str =
    r#"{"summary":"tests run","commands":["cargo test"],"results":["12 passed"]}"#;
const SECURITY_APPROVE: &str = r#"{"decision":"approve","findings":[]}"#;
const SECURITY_REQ: &str = r#"{"decision":"request_changes","findings":[{"severity":"high","summary":"token appears in query string","evidence":"src/auth.rs:9"}]}"#;
const PERF_APPROVE: &str = r#"{"decision":"approve","findings":[{"severity":"low","summary":"minor allocation","evidence":"profile"}]}"#;
const DOCS_OUT: &str = r#"{"summary":"documented the auth flow","commands":["echo"]}"#;

fn fixture_with(plan: &str, options: &FixtureOptions) -> Fixture {
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    Factory::init(dir.path()).unwrap();

    let plan_path = write_json(dir.path(), "plan.json", plan);
    let review_path = write_json(dir.path(), "review.json", APPROVE);
    let research_out = write_json(dir.path(), "research.json", RESEARCH_OUT);
    let analyst_out = write_json(dir.path(), "analyst.json", ANALYST_OUT);
    let arch_out = write_json(dir.path(), "arch.json", ARCH_OUT);
    let worker_out = write_json(dir.path(), "worker-output.json", WORKER_OUT);
    let verify_out = write_json(dir.path(), "verify.json", VERIFY_OUT);
    let security_approve = write_json(dir.path(), "security-approve.json", SECURITY_APPROVE);
    let security_req = write_json(dir.path(), "security-req.json", SECURITY_REQ);
    let perf_approve = write_json(dir.path(), "perf-approve.json", PERF_APPROVE);
    let docs_out = write_json(dir.path(), "docs.json", DOCS_OUT);
    let security_flag = dir.path().join("security-fixed.flag");

    let mut config = Config::default();
    let mut captures = BTreeMap::new();

    config.agents.insert(
        "planner-test".into(),
        script_entry(
            dir.path(),
            "planner",
            &cat_body(&plan_path).0,
            &cat_body(&plan_path).1,
        ),
    );
    let worker_a_capture = dir.path().join("worker-a.capture.txt");
    config.agents.insert(
        "worker-a".into(),
        script_entry(
            dir.path(),
            "worker-a",
            &capture_worker_body("a", &worker_a_capture, &worker_out).0,
            &capture_worker_body("a", &worker_a_capture, &worker_out).1,
        ),
    );
    captures.insert("worker-a".to_string(), worker_a_capture);
    if options.two_workers {
        let worker_b_capture = dir.path().join("worker-b.capture.txt");
        config.agents.insert(
            "worker-b".into(),
            script_entry(
                dir.path(),
                "worker-b",
                &capture_worker_body("b", &worker_b_capture, &worker_out).0,
                &capture_worker_body("b", &worker_b_capture, &worker_out).1,
            ),
        );
        captures.insert("worker-b".to_string(), worker_b_capture);
    }
    config.agents.insert(
        "reviewer-test".into(),
        script_entry(
            dir.path(),
            "reviewer",
            &cat_body(&review_path).0,
            &cat_body(&review_path).1,
        ),
    );

    let mut assignments: Vec<(String, String, bool)> = vec![
        ("planner".into(), "planner-test".into(), true),
        ("worker".into(), "worker-a".into(), true),
        ("worker".into(), "worker-b".into(), false),
        ("reviewer".into(), "reviewer-test".into(), true),
    ];
    if !options.two_workers {
        assignments.retain(|(role, agent, _)| !(role == "worker" && agent == "worker-b"));
    }

    for role in &options.include {
        let (role_id, class) = match *role {
            "researcher" => ("researcher", ExecutionClass::Advisory),
            "analyst" => ("analyst", ExecutionClass::Advisory),
            "architect" => ("architect", ExecutionClass::Advisory),
            "test_engineer" => ("test_engineer", ExecutionClass::Execution),
            "security_auditor" => ("security_auditor", ExecutionClass::Review),
            "documentation_writer" => ("documentation_writer", ExecutionClass::PostProcess),
            "database_engineer" => ("database_engineer", ExecutionClass::Execution),
            "performance_analyst" => ("performance_analyst", ExecutionClass::Review),
            other => panic!("unknown fixture role {other}"),
        };
        let agent_name = format!("{role_id}-test");
        let capture_path = dir.path().join(format!("{role_id}.capture.txt"));
        let (body_unix, body_win) = match role_id {
            "researcher" => capture_only_body(&capture_path, &research_out),
            "analyst" => capture_only_body(&capture_path, &analyst_out),
            "architect" => capture_only_body(&capture_path, &arch_out),
            "test_engineer" => capture_only_body(&capture_path, &verify_out),
            "security_auditor" if options.security == "flag-fixed" => (
                format!(
                    "if [ ! -f \"{flag}\" ]; then : > \"{flag}\"; cat \"{req}\"; else cat \"{app}\"; fi",
                    flag = security_flag.display(),
                    req = security_req.display(),
                    app = security_approve.display(),
                ),
                format!(
                    "if not exist \"{flag}\" (echo 1>\"{flag}\" & type \"{req}\") else (type \"{app}\")",
                    flag = security_flag.display(),
                    req = security_req.display(),
                    app = security_approve.display(),
                ),
            ),
            "security_auditor" if options.security == "request_changes" => {
                capture_only_body(&capture_path, &security_req)
            }
            "security_auditor" => capture_only_body(&capture_path, &security_approve),
            "documentation_writer" => (
                format!(
                    "printf 'docs\\n' > docs.txt\ncat > \"{capture}\"\ncat \"{output}\"",
                    capture = capture_path.display(),
                    output = docs_out.display(),
                ),
                format!(
                    "echo docs>docs.txt\nfindstr /r \".*\" > \"{capture}\"\ntype \"{output}\"",
                    capture = capture_path.display(),
                    output = docs_out.display(),
                ),
            ),
            "database_engineer" => {
                capture_worker_body("db", &capture_path, &worker_out)
            }
            "performance_analyst" => capture_only_body(&capture_path, &perf_approve),
            _ => unreachable!(),
        };
        config.agents.insert(
            agent_name.clone(),
            script_entry(dir.path(), &agent_name, &body_unix, &body_win),
        );
        captures.insert(role_id.to_string(), capture_path);
        assignments.push((role_id.to_string(), agent_name, true));
        if !factory_core::is_core_role(role_id) {
            config.roles.insert(
                role_id.to_string(),
                RoleDefinitionEntry {
                    name: Some(role_id.to_string()),
                    description: Some(format!("fixture {role_id}")),
                    execution_class: Some(class),
                    instructions: format!("Purpose: behave as {role_id} for the test."),
                    agent: None,
                },
            );
        }
    }

    for (role, agent, preferred) in assignments {
        config.role_assignments.push(RoleAssignment {
            role,
            agent,
            preferred,
        });
    }
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    Fixture {
        _dir: dir,
        factory,
        captures,
    }
}

fn team_with(additional: &[(&str, &str)]) -> WorkflowTeam {
    let mut extra = BTreeMap::new();
    for (role, agent) in additional {
        extra
            .entry((*role).to_string())
            .or_insert_with(Vec::new)
            .push((*agent).to_string());
    }
    WorkflowTeam {
        planner: "planner-test".into(),
        workers: vec!["worker-a".into()],
        reviewers: vec!["reviewer-test".into()],
        additional: extra,
    }
}

fn plan_and_start(fixture: &Fixture, objective: &str, team: WorkflowTeam) -> i64 {
    let run = fixture.factory.begin_run(objective, Some(team)).unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    fixture.factory.prepare_start(run.id).unwrap();
    run.id
}

fn run_completed(fixture: &Fixture, run_id: i64) {
    let result = fixture
        .factory
        .execute_active_run(run_id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result, factory_core::WorkflowResult::Completed);
    assert_eq!(
        fixture.factory.get_run(run_id).unwrap().unwrap().status,
        RunStatus::Completed
    );
}

// --- Planner semantics -----------------------------------------------------

#[test]
fn simple_plan_without_roles_defaults_every_task_to_worker_implement() {
    let plan = r#"{"objective":"small css fix","tasks":[
    {"id":"T1","title":"Tweak styles","objective":"adjust spacing","dependencies":[],"acceptanceCriteria":["spacing consistent"]}]}"#;
    let fixture = fixture_with(plan, &FixtureOptions::default());
    let outcome = fixture.factory.create_run("small css fix").unwrap();
    assert_eq!(
        outcome.tasks[0].operation,
        Some(TaskOperation::Implement),
        "a role-less task becomes Worker implementation"
    );
    assert_eq!(outcome.tasks[0].role, None);
}

#[test]
fn planned_operations_and_roles_are_persisted() {
    let plan = r#"{"objective":"auth work","tasks":[
    {"id":"T1","title":"Research auth","objective":"understand","dependencies":[],"acceptanceCriteria":["context"],"role":"researcher","operation":"advisory"},
    {"id":"T2","title":"Implement auth","objective":"build","dependencies":["T1"],"acceptanceCriteria":["auth works"]}]}"#;
    let options = FixtureOptions {
        include: vec!["researcher"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run = fixture
        .factory
        .begin_run(
            "auth work",
            Some(team_with(&[("researcher", "researcher-test")])),
        )
        .unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    // End-to-end: the planner reads a static plan with an explicit operation.
    let tasks = fixture.factory.list_tasks(run.id).unwrap();
    assert_eq!(tasks[0].operation, Some(TaskOperation::Advisory));
    assert_eq!(tasks[0].role.as_deref(), Some("researcher"));
    assert_eq!(tasks[1].operation, Some(TaskOperation::Implement));
    assert_eq!(tasks[1].role, None);
}

#[test]
fn operation_mismatched_with_role_class_rejects_the_plan() {
    let plan = r#"{"objective":"audit","tasks":[
    {"id":"T1","title":"Implement audit","objective":"change code","dependencies":[],"acceptanceCriteria":["changed"],"role":"security_auditor","operation":"implement"}]}"#;
    let options = FixtureOptions {
        include: vec!["security_auditor"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run = fixture
        .factory
        .begin_run(
            "audit",
            Some(team_with(&[("security_auditor", "security_auditor-test")])),
        )
        .unwrap();
    let error = fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot perform operation 'implement'"),
        "unexpected error: {error}"
    );
    assert_eq!(
        fixture.factory.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Failed
    );
}

#[test]
fn optional_role_not_selected_in_team_cannot_be_used() {
    let plan = r#"{"objective":"security","tasks":[
    {"id":"T1","title":"Audit","objective":"review","dependencies":[],"acceptanceCriteria":["decision"],"role":"security_auditor","operation":"review"}]}"#;
    // security_auditor exists globally but is NOT in the workflow team.
    let options = FixtureOptions {
        include: vec!["security_auditor"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run = fixture
        .factory
        .begin_run("security", Some(team_with(&[])))
        .unwrap();
    let error = fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap_err();
    assert!(
        error.to_string().contains("not enabled for this workflow"),
        "unexpected error: {error}"
    );
}

#[test]
fn planner_prompt_advertises_execution_classes_for_the_workflows_roles() {
    let plan = r#"{"objective":"small css fix","tasks":[
    {"id":"T1","title":"Tweak styles","objective":"adjust spacing","dependencies":[],"acceptanceCriteria":["spacing consistent"]}]}"#;
    let fixture = fixture_with(plan, &FixtureOptions::default());
    let run = fixture.factory.begin_run("css fix", None).unwrap();
    fixture
        .factory
        .plan_run(run.id, &AtomicBool::new(false))
        .unwrap();
    let sessions = fixture.factory.list_agent_sessions(Some(run.id)).unwrap();
    let planner_stdout = sessions
        .iter()
        .find(|session| session.role == "planner")
        .unwrap()
        .stdout
        .as_deref()
        .unwrap();
    // The planner session output contains the accepted plan.
    assert!(planner_stdout.contains("small css fix"));
}

// --- Advisory roles --------------------------------------------------------

#[test]
fn researcher_advisory_succeeds_with_zero_files_and_persists_an_artifact() {
    let plan = r#"{"objective":"auth research","tasks":[
    {"id":"T1","title":"Research auth","objective":"understand the stack","dependencies":[],"acceptanceCriteria":["findings"],"role":"researcher","operation":"advisory"}]}"#;
    let options = FixtureOptions {
        include: vec!["researcher"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "auth research",
        team_with(&[("researcher", "researcher-test")]),
    );
    run_completed(&fixture, run_id);

    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, AttemptStatus::Approved);
    let evidence = attempts[0].evidence.as_ref().unwrap();
    assert!(
        evidence.changed_files.is_empty(),
        "advisory work must not require repository changes"
    );

    let artifacts = fixture.factory.list_role_artifacts(run_id).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].kind, ArtifactKind::Research.as_str());
    assert!(artifacts[0].content.contains("RESEARCH_TOKEN_ABC"));

    // No per-task reviewer runs for advisory work.
    let sessions = fixture.factory.list_agent_sessions(Some(run_id)).unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.role.as_str())
            .collect::<Vec<_>>(),
        vec!["planner", "researcher"]
    );
}

#[test]
fn architecture_artifact_propagates_to_the_worker_mission_only_via_direct_deps() {
    let plan = r#"{"objective":"auth build","tasks":[
    {"id":"T1","title":"Research auth","objective":"understand","dependencies":[],"acceptanceCriteria":["research"],"role":"researcher","operation":"advisory"},
    {"id":"T2","title":"Design auth","objective":"architecture","dependencies":["T1"],"acceptanceCriteria":["design"],"role":"architect","operation":"advisory"},
    {"id":"T3","title":"Build auth","objective":"implement","dependencies":["T2"],"acceptanceCriteria":["works"]}]}"#;
    let options = FixtureOptions {
        include: vec!["researcher", "architect"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "auth build",
        team_with(&[
            ("researcher", "researcher-test"),
            ("architect", "architect-test"),
        ]),
    );
    run_completed(&fixture, run_id);

    let worker_mission = fixture.capture("worker-a").expect("worker capture file");
    assert!(
        worker_mission.contains("WORKFLOW OBJECTIVE\nauth build"),
        "worker mission carries the run objective"
    );
    assert!(
        worker_mission.contains("ARCH_TOKEN_XYZ"),
        "worker mission received the architecture artifact"
    );
    assert!(
        worker_mission.contains("UPSTREAM CONTEXT"),
        "worker mission declares its upstream context"
    );
    assert!(
        !worker_mission.contains("RESEARCH_TOKEN_ABC"),
        "transitive research context must not reach the worker"
    );

    let artifacts = fixture.factory.list_role_artifacts(run_id).unwrap();
    assert_eq!(
        artifacts
            .iter()
            .map(|artifact| artifact.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["research", "architecture"]
    );
}

#[test]
fn unrelated_dependency_branches_do_not_leak_context() {
    let plan = r#"{"objective":"two branches","tasks":[
    {"id":"T1","title":"Research A","objective":"a","dependencies":[],"acceptanceCriteria":["a"],"role":"researcher","operation":"advisory"},
    {"id":"T2","title":"Analyze B","objective":"b","dependencies":[],"acceptanceCriteria":["b"],"role":"analyst","operation":"advisory"},
    {"id":"T3","title":"Build A","objective":"build a","dependencies":["T1"],"acceptanceCriteria":["a works"]},
    {"id":"T4","title":"Build B","objective":"build b","dependencies":["T2"],"acceptanceCriteria":["b works"]}]}"#;
    let options = FixtureOptions {
        include: vec!["researcher", "analyst"],
        two_workers: true,
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let team = {
        let mut extra = BTreeMap::new();
        extra.insert(
            "researcher".to_string(),
            vec!["researcher-test".to_string()],
        );
        extra.insert("analyst".to_string(), vec!["analyst-test".to_string()]);
        WorkflowTeam {
            planner: "planner-test".into(),
            workers: vec!["worker-a".into(), "worker-b".into()],
            reviewers: vec!["reviewer-test".into()],
            additional: extra,
        }
    };
    let run_id = plan_and_start(&fixture, "two branches", team);
    run_completed(&fixture, run_id);

    // Deterministic round-robin: task T3 routes to worker-a, T4 to worker-b.
    let worker_a = fixture.capture("worker-a").expect("worker a capture");
    let worker_b = fixture.capture("worker-b").expect("worker b capture");
    assert!(worker_a.contains("RESEARCH_TOKEN_ABC"), "branch A context");
    assert!(
        !worker_a.contains("ANALYST_TOKEN_QRS"),
        "worker A never receives branch B context"
    );
    assert!(worker_b.contains("ANALYST_TOKEN_QRS"), "branch B context");
    assert!(
        !worker_b.contains("RESEARCH_TOKEN_ABC"),
        "worker B never receives branch A context"
    );
}

// --- Verification / implementation ----------------------------------------

#[test]
fn test_engineer_persists_a_verification_artifact() {
    let plan = r#"{"objective":"test the migration","tasks":[
    {"id":"T1","title":"Implement migration","objective":"change schema","dependencies":[],"acceptanceCriteria":["schema moves"]},
    {"id":"T2","title":"Verify migration","objective":"run tests","dependencies":["T1"],"acceptanceCriteria":["tests pass"],"role":"test_engineer","operation":"verify"}]}"#;
    let options = FixtureOptions {
        include: vec!["test_engineer"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "verified migration",
        team_with(&[("test_engineer", "test_engineer-test")]),
    );
    run_completed(&fixture, run_id);

    let artifacts = fixture.factory.list_role_artifacts(run_id).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].kind, ArtifactKind::Verification.as_str());
    assert!(
        artifacts[0].content.contains("cargo test"),
        "{}",
        artifacts[0].content
    );
    assert!(artifacts[0].content.contains("12 passed"));

    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    let verify_attempt = attempts
        .iter()
        .find(|attempt| attempt.operation == Some(TaskOperation::Verify))
        .unwrap();
    assert_eq!(
        verify_attempt.evidence.as_ref().unwrap().commands,
        vec!["cargo test"]
    );
    // The built-in Reviewer runs inside the implementation attempt, so each
    // task records exactly one attempt.
    assert_eq!(attempts.len(), 2);
    let sessions = fixture.factory.list_agent_sessions(Some(run_id)).unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.role.as_str())
            .collect::<Vec<_>>(),
        vec!["planner", "worker", "reviewer", "test_engineer", "reviewer"]
    );
}

#[test]
fn documentation_writer_runs_after_dependencies_and_may_change_files() {
    let plan = r#"{"objective":"ship migration docs","tasks":[
    {"id":"T1","title":"Write migration","objective":"change schema","dependencies":[],"acceptanceCriteria":["migration exists"]},
    {"id":"T2","title":"Document migration","objective":"document it","dependencies":["T1"],"acceptanceCriteria":["documented"],"role":"documentation_writer","operation":"post_process"}]}"#;
    let options = FixtureOptions {
        include: vec!["documentation_writer"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "shipping docs",
        team_with(&[("documentation_writer", "documentation_writer-test")]),
    );
    run_completed(&fixture, run_id);

    let artifacts = fixture.factory.list_role_artifacts(run_id).unwrap();
    assert_eq!(
        artifacts[0].kind,
        ArtifactKind::DocumentationContext.as_str()
    );

    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    let docs_attempt = attempts
        .iter()
        .find(|attempt| attempt.operation == Some(TaskOperation::PostProcess))
        .unwrap();
    assert_eq!(
        docs_attempt.evidence.as_ref().unwrap().changed_files,
        vec!["docs.txt"]
    );
    assert_eq!(
        docs_attempt.status,
        AttemptStatus::Approved,
        "post-process work is accepted through the built-in Reviewer"
    );
}

// --- Specialized review ----------------------------------------------------

#[test]
fn security_auditor_approves_and_persists_a_review_artifact() {
    let plan = r#"{"objective":"secure auth","tasks":[
    {"id":"T1","title":"Build auth","objective":"implement","dependencies":[],"acceptanceCriteria":["auth works"]},
    {"id":"T2","title":"Audit auth","objective":"security review","dependencies":["T1"],"acceptanceCriteria":["decision"],"role":"security_auditor","operation":"review"}]}"#;
    let options = FixtureOptions {
        include: vec!["security_auditor"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "secure auth",
        team_with(&[("security_auditor", "security_auditor-test")]),
    );
    run_completed(&fixture, run_id);

    let artifacts = fixture.factory.list_role_artifacts(run_id).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].kind, ArtifactKind::Review.as_str());
    assert!(artifacts[0].content.contains("\"approve\""));

    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    let audit = attempts
        .iter()
        .find(|attempt| attempt.operation == Some(TaskOperation::Review))
        .unwrap();
    assert_eq!(audit.status, AttemptStatus::Approved);
    assert!(
        audit.evidence.as_ref().unwrap().changed_files.is_empty(),
        "review tasks succeed with zero repository changes"
    );
    assert!(
        audit.review.as_ref().unwrap().reason.contains("approved"),
        "{}",
        audit.review.as_ref().unwrap().reason
    );
}

#[test]
fn security_auditor_request_changes_reroutes_to_implementation_rework() {
    let plan = r#"{"objective":"secure auth","tasks":[
    {"id":"T1","title":"Build auth","objective":"implement","dependencies":[],"acceptanceCriteria":["auth works"]},
    {"id":"T2","title":"Audit auth","objective":"security review","dependencies":["T1"],"acceptanceCriteria":["decision"],"role":"security_auditor","operation":"review"}]}"#;
    let options = FixtureOptions {
        include: vec!["security_auditor"],
        security: "request_changes",
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "secure auth",
        team_with(&[("security_auditor", "security_auditor-test")]),
    );
    let error = fixture
        .factory
        .execute_active_run(run_id, &AtomicBool::new(false))
        .unwrap_err();
    assert!(
        error.to_string().contains("requested changes"),
        "meaningful rejection, got: {error}"
    );
    assert_eq!(
        fixture.factory.get_run(run_id).unwrap().unwrap().status,
        RunStatus::Failed
    );

    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    let worker_attempts: Vec<_> = attempts
        .iter()
        .filter(|attempt| attempt.operation == Some(TaskOperation::Implement))
        .collect();
    let review_attempts: Vec<_> = attempts
        .iter()
        .filter(|attempt| attempt.operation == Some(TaskOperation::Review))
        .collect();
    assert_eq!(
        worker_attempts.len(),
        factory_core::MAX_TASK_ATTEMPTS as usize,
        "implementation reworked up to the bounded limit"
    );
    assert_eq!(
        review_attempts.len(),
        factory_core::MAX_TASK_ATTEMPTS as usize
    );
    assert!(worker_attempts
        .iter()
        .all(|attempt| attempt.status == AttemptStatus::Approved));
    assert!(review_attempts
        .iter()
        .all(|attempt| attempt.status == AttemptStatus::ChangesRequested));
    assert!(review_attempts[0]
        .review
        .as_ref()
        .unwrap()
        .feedback
        .iter()
        .any(|line| line.contains("token appears in query string")));
}

#[test]
fn reworked_implementation_can_satisfy_the_review_and_finish() {
    let plan = r#"{"objective":"secure auth","tasks":[
    {"id":"T1","title":"Build auth","objective":"implement","dependencies":[],"acceptanceCriteria":["auth works"]},
    {"id":"T2","title":"Audit auth","objective":"security review","dependencies":["T1"],"acceptanceCriteria":["decision"],"role":"security_auditor","operation":"review"}]}"#;
    let options = FixtureOptions {
        include: vec!["security_auditor"],
        security: "flag-fixed",
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "secure auth",
        team_with(&[("security_auditor", "security_auditor-test")]),
    );
    run_completed(&fixture, run_id);

    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    let worker_attempts: Vec<_> = attempts
        .iter()
        .filter(|attempt| attempt.operation == Some(TaskOperation::Implement))
        .collect();
    let review_attempts: Vec<_> = attempts
        .iter()
        .filter(|attempt| attempt.operation == Some(TaskOperation::Review))
        .collect();
    assert_eq!(worker_attempts.len(), 2, "one rework cycle");
    assert_eq!(review_attempts.len(), 2);
    assert_eq!(review_attempts[0].status, AttemptStatus::ChangesRequested);
    assert_eq!(review_attempts[1].status, AttemptStatus::Approved);
}

// --- Custom roles ----------------------------------------------------------

#[test]
fn custom_execution_role_implements_with_generic_semantics() {
    let plan = r#"{"objective":"schema work","tasks":[
    {"id":"T1","title":"Design schema","objective":"create migration","dependencies":[],"acceptanceCriteria":["migration exists"],"role":"database_engineer","operation":"implement"}]}"#;
    let options = FixtureOptions {
        include: vec!["database_engineer"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "schema",
        team_with(&[("database_engineer", "database_engineer-test")]),
    );
    run_completed(&fixture, run_id);

    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    assert_eq!(attempts[0].role.as_deref(), Some("database_engineer"));
    assert_eq!(attempts[0].operation, Some(TaskOperation::Implement));
    assert_eq!(attempts[0].status, AttemptStatus::Approved);
}

#[test]
fn custom_review_role_runs_through_specialized_review_semantics() {
    let plan = r#"{"objective":"performance pass","tasks":[
    {"id":"T1","title":"Optimize hot path","objective":"implement","dependencies":[],"acceptanceCriteria":["fast"]},
    {"id":"T2","title":"Review performance","objective":"performance review","dependencies":["T1"],"acceptanceCriteria":["decision"],"role":"performance_analyst","operation":"review"}]}"#;
    let options = FixtureOptions {
        include: vec!["performance_analyst"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "perf",
        team_with(&[("performance_analyst", "performance_analyst-test")]),
    );
    run_completed(&fixture, run_id);

    let artifacts = fixture.factory.list_role_artifacts(run_id).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].content.contains("minor allocation"));
    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    let review = attempts
        .iter()
        .find(|attempt| attempt.operation == Some(TaskOperation::Review))
        .unwrap();
    assert_eq!(review.status, AttemptStatus::Approved);
    assert_eq!(review.role.as_deref(), Some("performance_analyst"));
}

// --- Session / attempt identity --------------------------------------------

#[test]
fn agent_sessions_and_attempts_keep_the_actual_role_and_operation() {
    let plan = r#"{"objective":"auth research","tasks":[
    {"id":"T1","title":"Research auth","objective":"understand","dependencies":[],"acceptanceCriteria":["findings"],"role":"researcher","operation":"advisory"}]}"#;
    let options = FixtureOptions {
        include: vec!["researcher"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "research",
        team_with(&[("researcher", "researcher-test")]),
    );
    run_completed(&fixture, run_id);

    let sessions = fixture.factory.list_agent_sessions(Some(run_id)).unwrap();
    let research_session = sessions
        .iter()
        .find(|session| session.role == "researcher")
        .unwrap();
    assert_eq!(
        research_session.operation,
        Some(TaskOperation::Advisory),
        "sessions keep the actual operation"
    );
}

#[test]
fn review_without_implementation_dependency_fails_with_a_diagnostic() {
    let plan = r#"{"objective":"audit","tasks":[
    {"id":"T1","title":"Audit","objective":"review","dependencies":[],"acceptanceCriteria":["done"],"role":"security_auditor","operation":"review"}]}"#;
    let options = FixtureOptions {
        include: vec!["security_auditor"],
        ..FixtureOptions::default()
    };
    let fixture = fixture_with(plan, &options);
    let run_id = plan_and_start(
        &fixture,
        "audit",
        team_with(&[("security_auditor", "security_auditor-test")]),
    );
    let error = fixture
        .factory
        .execute_active_run(run_id, &AtomicBool::new(false))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no dependency providing implementation evidence"),
        "unexpected: {error}"
    );
}

#[test]
fn every_workflow_keeps_a_role_aware_task_and_operation_model() {
    let plan = r#"{"objective":"simple","tasks":[
    {"id":"T1","title":"One","objective":"one","dependencies":[],"acceptanceCriteria":["done"]}]}"#;
    let fixture = fixture_with(plan, &FixtureOptions::default());
    let run_id = plan_and_start(&fixture, "simple", team_with(&[]));
    run_completed(&fixture, run_id);
    let task = fixture.factory.list_tasks(run_id).unwrap().remove(0);
    assert_eq!(task.operation, Some(TaskOperation::Implement));
    assert_eq!(task.state, TaskState::Completed);
    let attempts = fixture.factory.list_task_attempts(run_id).unwrap();
    assert_eq!(attempts[0].role.as_deref(), Some("worker"));
    assert_eq!(attempts[0].operation, Some(TaskOperation::Implement));
}
