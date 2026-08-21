//! Integration tests for the policy engine: fail-before-execution blocks,
//! evidence enforcement, environment filtering, and per-session audit. All
//! workflows run against fake command agents.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use factory_core::{factory_policy, AgentEntry, Config, Factory, RoleAssignment, WorkflowResult};
use factory_types::{AttemptStatus, RunStatus, TaskState};
use tempfile::TempDir;

const PLAN: &str = r#"{"objective":"exercise the policy engine","tasks":[{"id":"T1","title":"Single task","objective":"make the change","dependencies":[],"acceptanceCriteria":["worker evidence exists"]}]}"#;

const PARALLEL_PLAN: &str = r#"{"objective":"parallel policy probe","tasks":[
{"id":"T1","title":"First task","objective":"first change","dependencies":[],"acceptanceCriteria":["done"]},
{"id":"T2","title":"Second task","objective":"second change","dependencies":[],"acceptanceCriteria":["done"]}]}"#;

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

fn command_entry(script: &str) -> AgentEntry {
    if cfg!(windows) {
        AgentEntry {
            kind: None,
            command: "cmd".into(),
            args: vec!["/d".into(), "/c".into(), script.into()],
            env: BTreeMap::new(),
            prompt_transport: None,
            interactive_args: None,
            capabilities: Vec::new(),
            max_concurrency: None,
        }
    } else {
        AgentEntry {
            kind: None,
            command: "sh".into(),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::new(),
            prompt_transport: None,
            interactive_args: None,
            capabilities: Vec::new(),
            max_concurrency: None,
        }
    }
}

struct Fixture {
    dir: TempDir,
    plan: String,
    worker_script: String,
    worker_report: String,
    policies: Option<String>,
    agent_env: BTreeMap<String, String>,
}

impl Fixture {
    fn new(worker_script: &str, worker_report: &str) -> Self {
        Self {
            dir: TempDir::new().unwrap(),
            plan: PLAN.to_string(),
            worker_script: worker_script.to_string(),
            worker_report: worker_report.to_string(),
            policies: None,
            agent_env: BTreeMap::new(),
        }
    }

    fn policies(mut self, toml: &str) -> Self {
        self.policies = Some(toml.to_string());
        self
    }

    fn plan(mut self, plan: &str) -> Self {
        self.plan = plan.to_string();
        self
    }

    fn agent_env(mut self, key: &str, value: &str) -> Self {
        self.agent_env.insert(key.to_string(), value.to_string());
        self
    }

    fn build(self) -> (TempDir, Factory) {
        let dir = self.dir;
        init_git(dir.path());
        Factory::init(dir.path()).unwrap();

        let plan_path = dir.path().join("test-plan.json");
        let worker_path = dir.path().join("test-worker.json");
        let reviewer_path = dir.path().join("test-reviewer.json");
        std::fs::write(&plan_path, self.plan).unwrap();
        std::fs::write(&worker_path, self.worker_report).unwrap();
        std::fs::write(
            &reviewer_path,
            r#"{"decision":"approve","reason":"evidence accepted"}"#,
        )
        .unwrap();

        let cat = |path: &Path| {
            if cfg!(windows) {
                format!("type {}", path.display())
            } else {
                format!("cat '{}'", path.display())
            }
        };
        let mut config = Config::default();
        config
            .agents
            .insert("planner-test".into(), command_entry(&cat(&plan_path)));
        let mut worker = command_entry(&format!(
            "{} & type {}",
            self.worker_script,
            worker_path.display()
        ));
        if !cfg!(windows) {
            worker = command_entry(&format!(
                "{}; cat '{}'",
                self.worker_script,
                worker_path.display()
            ));
        }
        worker.env = self.agent_env;
        config.agents.insert("worker-test".into(), worker);
        config
            .agents
            .insert("reviewer-test".into(), command_entry(&cat(&reviewer_path)));
        for (role, agent) in [
            ("planner", "planner-test"),
            ("worker", "worker-test"),
            ("reviewer", "reviewer-test"),
        ] {
            config.role_assignments.push(RoleAssignment {
                role: role.into(),
                agent: agent.into(),
                preferred: true,
            });
        }
        if let Some(policies) = self.policies {
            #[derive(serde::Deserialize)]
            struct Wrapper {
                policies: factory_policy::PoliciesConfig,
            }
            let wrapper: Wrapper = toml::from_str(&policies).unwrap();
            config.policies = wrapper.policies;
        }
        config.write_atomic(dir.path()).unwrap();
        let factory = Factory::open(dir.path()).unwrap();
        (dir, factory)
    }
}

/// A worker that writes one file inside its worktree, creating parent
/// directories as needed.
fn write_file_script(file: &str) -> String {
    let Some((parent, _)) = file.rsplit_once('/') else {
        return if cfg!(windows) {
            format!("echo done>{file}")
        } else {
            format!("printf 'done\\n' > {file}")
        };
    };
    if cfg!(windows) {
        format!("mkdir {parent} 2>nul & echo done>{file}")
    } else {
        format!("mkdir -p {parent} && printf 'done\\n' > {file}")
    }
}

const STANDARD_REPORT: &str = r#"{"commands":["fake-worker"],"tests":["fake-test"]}"#;

#[test]
fn read_only_worker_blocks_the_run_before_any_agent_runs() {
    let (_dir, factory) = Fixture::new(&write_file_script("worker-output.txt"), STANDARD_REPORT)
        .policies(
            r#"
[policies]
[policies.roles.worker]
preset = "read_only"
"#,
        )
        .build();

    let outcome = factory.create_run("blocked by policy").unwrap();
    let error = factory.prepare_start(outcome.run.id).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("cannot perform operation 'implement'"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("no writable filesystem scope"),
        "the blocker must explain the write scopes: {message}"
    );
    assert_eq!(
        factory.get_run(outcome.run.id).unwrap().unwrap().status,
        RunStatus::Blocked
    );
    // Nothing ran beyond the planner: no attempt and no worker/review session.
    assert!(factory
        .list_task_attempts(outcome.run.id)
        .unwrap()
        .is_empty());
    assert!(factory
        .list_agent_sessions(Some(outcome.run.id))
        .unwrap()
        .iter()
        .all(|session| session.role == "planner"));
    let tasks = factory.list_tasks(outcome.run.id).unwrap();
    assert_eq!(tasks[0].state, TaskState::Ready);
}

#[test]
fn a_policy_block_never_consumes_task_retries() {
    let (dir, factory) = Fixture::new(&write_file_script("worker-output.txt"), STANDARD_REPORT)
        .policies(
            r#"
[policies]
[policies.roles.worker]
preset = "read_only"
"#,
        )
        .build();

    let outcome = factory.create_run("retry budget intact").unwrap();
    assert!(factory.prepare_start(outcome.run.id).is_err());
    assert!(factory
        .list_task_attempts(outcome.run.id)
        .unwrap()
        .is_empty());

    // Fix the configuration: the same task must now run and complete with a
    // single attempt — the earlier policy block burned nothing.
    drop(factory);
    let mut config = Config::load(dir.path()).unwrap();
    config.policies.roles.remove("worker");
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();

    factory.prepare_start(outcome.run.id).unwrap();
    let result = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result, WorkflowResult::Completed);
    let attempts = factory.list_task_attempts(outcome.run.id).unwrap();
    assert_eq!(attempts.len(), 1, "the policy block consumed no retry");
    assert_eq!(attempts[0].status, AttemptStatus::Approved);
}

#[test]
fn writes_outside_the_allowed_scope_fail_the_attempt_as_a_violation() {
    let (_dir, factory) = Fixture::new(&write_file_script("README.md"), STANDARD_REPORT)
        .policies(
            r#"
[policies]
[policies.roles.worker.filesystem]
read = ["**"]
write = ["src/**"]
"#,
        )
        .build();

    let outcome = factory.create_run("write outside the scope").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let error = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("outside the allowed write scopes"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("'README.md'"),
        "the violation must name the file: {message}"
    );

    let attempts = factory.list_task_attempts(outcome.run.id).unwrap();
    assert_eq!(attempts.len(), 1, "violations are not retried");
    assert_eq!(attempts[0].status, AttemptStatus::Failed);
    assert!(attempts[0]
        .error
        .as_deref()
        .unwrap()
        .contains("blocked by policy"));
    let tasks = factory.list_tasks(outcome.run.id).unwrap();
    assert_eq!(tasks[0].state, TaskState::Failed);
}

#[test]
fn factory_state_stays_unwritable_even_with_explicitly_open_scopes() {
    let (_dir, factory) = Fixture::new(&write_file_script(".factory/leaked.txt"), STANDARD_REPORT)
        .policies(
            r#"
[policies]
[policies.roles.worker.filesystem]
read = ["**"]
write = ["**"]
"#,
        )
        .build();

    let outcome = factory.create_run("protect factory state").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let error = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap_err();
    assert!(
        error.to_string().contains(".factory"),
        "the baseline deny of Factory state must hold: {error}"
    );
    // The Git safety invariants cannot be widened by configuration either.
    let config = Config::load(_dir.path()).unwrap();
    let policy = config.effective_policy("worker", "worker-test");
    assert!(!policy.git.allows(factory_policy::GitOperation::Push));
    assert!(!policy.git.allows(factory_policy::GitOperation::ForcePush));
    assert!(!policy
        .git
        .allows(factory_policy::GitOperation::DeleteBranch));
    assert!(!policy.git.allows(factory_policy::GitOperation::ResetBranch));
    assert!(!policy
        .git
        .allows(factory_policy::GitOperation::ModifyRemotes));
}

#[test]
fn reported_commands_outside_the_policy_fail_the_attempt() {
    let (_dir, factory) = Fixture::new(
        &write_file_script("worker-output.txt"),
        r#"{"commands":["fake-worker","bash -c leak"],"tests":[]}"#,
    )
    .policies(
        r#"
[policies]
[policies.roles.worker.commands]
mode = "restricted"
allow = ["fake-worker"]
deny = ["bash"]
"#,
    )
    .build();

    let outcome = factory.create_run("command policy").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let error = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("bash -c leak"),
        "the violation must name the command: {message}"
    );
    let attempts = factory.list_task_attempts(outcome.run.id).unwrap();
    assert_eq!(attempts.len(), 1, "violations are not retried");
    assert_eq!(attempts[0].status, AttemptStatus::Failed);
}

#[test]
fn agent_restrictions_narrow_the_role_at_execution_time() {
    // The role allows writes anywhere; the agent scope narrows writes to src/**.
    // The worker writes a README file the role permits but the agent does not.
    let (_dir, factory) = Fixture::new(&write_file_script("README.md"), STANDARD_REPORT)
        .policies(
            r#"
[policies]
[policies.roles.worker.filesystem]
read = ["**"]
write = ["**"]

[policies.agents.worker-test.filesystem]
read = ["**"]
write = ["src/**"]
"#,
        )
        .build();

    let outcome = factory.create_run("agent scope applies").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let error = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("outside the allowed write scopes"),
        "unexpected error: {error}"
    );
}

#[test]
fn environment_is_filtered_before_launch_and_secrets_never_reach_logs() {
    // The agent configures a secret; the policy denies it and restricts
    // inheritance to an allow list. The worker echoes the probe variable that
    // the allow list must have withheld.
    std::env::set_var("FACTORY_POLICY_PROBE", "probe-leaked-value");
    let echo_probe = if cfg!(windows) {
        "echo probe=%FACTORY_POLICY_PROBE%".to_string()
    } else {
        "echo probe=$FACTORY_POLICY_PROBE".to_string()
    };
    let (dir, factory) = Fixture::new(&echo_probe, STANDARD_REPORT)
        .agent_env("FACTORY_SECRET_TOKEN", "super-secret-value-4321")
        .policies(
            r#"
[policies]
[policies.roles.worker.environment]
allow = ["PATH", "HOME", "USERPROFILE", "USERNAME", "TEMP", "TMP", "TMPDIR",
         "SYSTEMROOT", "SYSTEMDRIVE", "WINDIR", "HOMEDRIVE", "HOMEPATH",
         "ComSpec", "PATHEXT", "RUST_BACKTRACE"]
deny = ["FACTORY_SECRET_TOKEN"]
"#,
        )
        .build();

    let outcome = factory.create_run("environment filtering").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let result = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result, WorkflowResult::Completed);

    let sessions = factory.list_agent_sessions(Some(outcome.run.id)).unwrap();
    let worker_session = sessions
        .iter()
        .find(|session| session.role == "worker")
        .expect("worker session");
    let stdout = worker_session.stdout.as_deref().unwrap_or("");
    assert!(
        stdout.contains("probe="),
        "the echo ran but its output is missing: {stdout}"
    );
    assert!(
        !stdout.contains("probe-leaked-value"),
        "the allow list must withhold non-essential variables: {stdout}"
    );
    assert!(
        !stdout.contains("super-secret-value-4321"),
        "a denied secret must never be logged: {stdout}"
    );
    // The full database log (all sessions) is also secret-free.
    for session in &sessions {
        for text in [session.stdout.as_deref(), session.stderr.as_deref()]
            .into_iter()
            .flatten()
        {
            assert!(!text.contains("super-secret-value-4321"));
        }
    }
    // The session audit records which policy applied, compactly.
    let audit = worker_session
        .policy_audit
        .as_ref()
        .expect("audit recorded");
    assert_eq!(audit.source, "role:worker");
    assert_eq!(audit.environment, "filtered");
    assert!(!audit.write_scopes.is_empty());
    // The persisted config never stored the secret either (it is in agent env,
    // which is expected), but the audit itself must stay value-free.
    let audit_text = serde_json::to_string(audit).unwrap();
    assert!(!audit_text.contains("super-secret-value-4321"));
    let _ = dir;
}

#[test]
fn independent_tasks_each_carry_their_own_policy_audit() {
    let (_dir, factory) =
        Fixture::new(&write_file_script("out/worker-output.txt"), STANDARD_REPORT)
            .plan(PARALLEL_PLAN)
            .policies(
                r#"
[policies]
[policies.roles.worker.filesystem]
read = ["**"]
write = ["out/**"]
"#,
            )
            .build();

    let outcome = factory.create_run("parallel isolation").unwrap();
    factory.prepare_start(outcome.run.id).unwrap();
    let result = factory
        .execute_active_run(outcome.run.id, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result, WorkflowResult::Completed);

    let attempts = factory.list_task_attempts(outcome.run.id).unwrap();
    assert_eq!(attempts.len(), 2);
    assert!(attempts
        .iter()
        .all(|attempt| attempt.status == AttemptStatus::Approved));
    let sessions = factory.list_agent_sessions(Some(outcome.run.id)).unwrap();
    let worker_sessions: Vec<_> = sessions
        .iter()
        .filter(|session| session.role == "worker")
        .collect();
    assert_eq!(worker_sessions.len(), 2);
    for session in &worker_sessions {
        let audit = session.policy_audit.as_ref().expect("audit per session");
        assert_eq!(audit.filesystem, "restricted");
        assert_eq!(audit.write_scopes, vec!["out/**".to_string()]);
    }
}
