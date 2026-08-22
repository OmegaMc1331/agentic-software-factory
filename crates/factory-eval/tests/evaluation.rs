//! Deterministic evaluation tests over synthetic SQLite workflow history.
//! No real agents are involved: attempts, sessions, and integration outcomes
//! are seeded straight into a temp FactoryDb.

use chrono::{DateTime, Duration, Utc};
use factory_db::FactoryDb;
use factory_eval::{
    detect_languages, evaluate, evaluate_agent, language_label, performance, stats::wilson_95,
    Outcome, PerformanceQuery, PerformanceWindow,
};
use factory_types::{
    AgentSession, AgentSessionMode, AttemptStatus, IntegrationOutcomeKind, ReviewDecision,
    ReviewResult, TaskEvidence, TaskOperation, TaskState,
};
use rusqlite::Connection;
use tempfile::TempDir;

// --- synthetic history helpers ----------------------------------------------

struct HistoryBuilder {
    db: FactoryDb,
    path: std::path::PathBuf,
    run_id: i64,
    task_seq: i64,
}

impl HistoryBuilder {
    fn new() -> (Self, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let db = FactoryDb::open(&path).unwrap();
        let run = db.create_run("objective", Some("codex")).unwrap();
        (
            HistoryBuilder {
                db,
                path,
                run_id: run.id,
                task_seq: 0,
            },
            dir,
        )
    }

    /// Creates one task and drives its full attempt sequence. Attempt wall
    /// time spans the synthetic exec/review durations plus overhead; the
    /// first attempt starts at `started` and retries follow one hour apart.
    fn task(
        &mut self,
        role: &str,
        operation: TaskOperation,
        attempts: &[AttemptSpec],
        started: DateTime<Utc>,
        changed_files: &[&str],
    ) -> i64 {
        self.task_seq += 1;
        let task_id = self
            .db
            .create_task(
                self.run_id,
                &format!("Task {}", self.task_seq),
                "objective",
                &[],
                TaskState::Completed,
                self.task_seq as i32,
                Some(role),
                Some(operation),
            )
            .unwrap();

        for (index, spec) in attempts.iter().enumerate() {
            let attempt_started = spec
                .start
                .unwrap_or(started + Duration::seconds(index as i64 * 3600));
            let wall_ms =
                spec.exec_ms.map_or(70_000, |ms| ms + 10_000) + spec.review_ms.unwrap_or(0);
            let attempt = self
                .db
                .create_task_attempt(
                    task_id,
                    role,
                    Some(operation),
                    &spec.agent,
                    &format!("worktree-{task_id}-{}", index + 1),
                    None,
                )
                .unwrap();
            // create_task_attempt stamps started_at with the real clock; the
            // synthetic timeline is set through a second connection so the
            // production API stays test-free.
            Connection::open(&self.path)
                .unwrap()
                .execute(
                    "UPDATE task_attempts SET started_at = ?1, finished_at = ?2 WHERE id = ?3",
                    rusqlite::params![
                        attempt_started.to_rfc3339(),
                        (attempt_started + Duration::milliseconds(wall_ms as i64)).to_rfc3339(),
                        attempt.id
                    ],
                )
                .unwrap();

            if spec.exec_ms.is_some() || spec.review_ms.is_some() {
                self.attach_sessions(
                    &attempt,
                    attempt_started,
                    wall_ms,
                    role,
                    operation,
                    &spec.agent,
                    spec.exec_ms,
                    spec.review_ms,
                );
            }

            let evidence = (!changed_files.is_empty()).then(|| TaskEvidence {
                changed_files: changed_files.iter().map(|file| file.to_string()).collect(),
                diff_summary: "changed".into(),
                commit_sha: None,
                commands: vec![],
                acceptance_criteria: vec![],
                worker_exit_code: Some(0),
                artifacts: vec![],
                diff_patch: None,
            });

            let review = matches!(
                spec.status,
                AttemptStatus::Approved | AttemptStatus::ChangesRequested
            )
            .then(|| ReviewResult {
                decision: if spec.status == AttemptStatus::Approved {
                    ReviewDecision::Approve
                } else {
                    ReviewDecision::RequestChanges
                },
                reason: spec.error.clone().unwrap_or_default(),
                feedback: vec![],
            });

            self.db
                .finish_task_attempt(
                    attempt.id,
                    spec.status,
                    spec.exit_code,
                    None,
                    spec.error.as_deref(),
                    evidence.as_ref(),
                    review.as_ref(),
                )
                .unwrap();
        }
        task_id
    }

    #[allow(clippy::too_many_arguments)]
    fn attach_sessions(
        &self,
        attempt: &factory_types::TaskAttempt,
        started: DateTime<Utc>,
        wall_ms: u64,
        role: &str,
        operation: TaskOperation,
        agent: &str,
        exec_ms: Option<u64>,
        review_ms: Option<u64>,
    ) {
        let insert = |role_name: &str, op: Option<TaskOperation>, duration: Option<u64>| {
            self.db
                .insert_agent_session(&AgentSession {
                    id: 0,
                    run_id: Some(self.run_id),
                    task_id: Some(attempt.task_id),
                    attempt_id: Some(attempt.id),
                    role: role_name.to_string(),
                    operation: op,
                    agent: agent.to_string(),
                    mode: AgentSessionMode::Automated,
                    command: "agent run".into(),
                    status: "success".into(),
                    started_at: started.to_rfc3339(),
                    finished_at: Some(
                        (started + Duration::milliseconds(wall_ms as i64)).to_rfc3339(),
                    ),
                    exit_code: Some(0),
                    duration_ms: duration,
                    stdout: None,
                    stderr: None,
                    policy_audit: None,
                })
                .unwrap();
        };
        insert(role, Some(operation), exec_ms);
        if review_ms.is_some() {
            insert("reviewer", Some(TaskOperation::Review), review_ms);
        }
    }

    fn record_integration(
        &mut self,
        task_id: i64,
        attempt_id: i64,
        agent: &str,
        kind: IntegrationOutcomeKind,
    ) {
        self.db
            .record_integration_outcome(self.run_id, task_id, attempt_id, agent, kind)
            .unwrap();
    }
}

#[derive(Clone)]
struct AttemptSpec {
    agent: String,
    status: AttemptStatus,
    error: Option<String>,
    exit_code: Option<i32>,
    exec_ms: Option<u64>,
    review_ms: Option<u64>,
    /// Overrides the default `started + index * 1h` timeline for this
    /// attempt (retries that happen days later, for window tests).
    start: Option<DateTime<Utc>>,
}

impl AttemptSpec {
    fn by(agent: &str, status: AttemptStatus) -> Self {
        AttemptSpec {
            agent: agent.to_string(),
            status,
            error: None,
            exit_code: None,
            exec_ms: None,
            review_ms: None,
            start: None,
        }
    }
    fn error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }
    fn exit(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }
    fn durations(mut self, exec_ms: u64, review_ms: u64) -> Self {
        self.exec_ms = Some(exec_ms);
        self.review_ms = Some(review_ms);
        self
    }
    fn starting_at(mut self, start: DateTime<Utc>) -> Self {
        self.start = Some(start);
        self
    }
}

fn query() -> PerformanceQuery {
    PerformanceQuery::default()
}

trait AgentMetricsLookup {
    fn metrics_for<'a>(&'a self, agent: &str) -> &'a factory_eval::AgentMetrics;
}

impl AgentMetricsLookup for factory_eval::EvaluationReport {
    fn metrics_for<'a>(&'a self, agent: &str) -> &'a factory_eval::AgentMetrics {
        &self
            .agents
            .iter()
            .find(|summary| summary.agent == agent)
            .unwrap_or_else(|| panic!("agent {agent} missing"))
            .metrics
    }
}

// --- unit-level checks --------------------------------------------------------

#[test]
fn wilson_interval_matches_reference_values() {
    // 93/100 successes at 95% confidence.
    let (low, high) = wilson_95(93, 100);
    assert!((low - 0.8625).abs() < 0.001, "low {low}");
    assert!((high - 0.9657).abs() < 0.001, "high {high}");
    // Degenerate samples stay inside [0, 1].
    let (low, high) = wilson_95(0, 1);
    assert_eq!(low, 0.0);
    assert!(high > 0.0 && high < 0.95);
}

#[test]
fn language_detection_is_deterministic_and_multi_language() {
    let languages = detect_languages(&[
        "src/main.rs".to_string(),
        "src/lib.ts".into(),
        "components/App.tsx".into(),
        "Cargo.toml".into(),
        "package-lock.json".into(),
        ".gitignore".into(),
    ]);
    let keys: Vec<&str> = languages.iter().map(String::as_str).collect();
    assert_eq!(keys, vec!["rust", "typescript"]);
    assert_eq!(language_label("typescript"), "TypeScript");
    assert_eq!(language_label("cpp"), "C++");
    // No recognizable evidence means no label, never a guess.
    assert!(detect_languages(&["Cargo.toml".to_string()]).is_empty());
}

#[test]
fn outcome_classification_separates_agent_failures_from_exclusions() {
    let agent_failure = factory_types::TaskAttempt {
        error: Some("Worker process exited with code 1.".into()),
        status: AttemptStatus::Failed,
        ..minimal_attempt()
    };
    assert_eq!(
        factory_eval::classify_attempt(&agent_failure),
        Outcome::AgentFailed
    );

    let policy = factory_types::TaskAttempt {
        error: Some("blocked by policy: write outside scope".into()),
        status: AttemptStatus::Failed,
        ..minimal_attempt()
    };
    assert_eq!(
        factory_eval::classify_attempt(&policy),
        Outcome::PolicyBlocked
    );

    let missing_executable = factory_types::TaskAttempt {
        error: Some(
            "executable `codex` was not found in the PATH visible to Factory (12 entries checked)"
                .into(),
        ),
        status: AttemptStatus::Failed,
        ..minimal_attempt()
    };
    assert_eq!(
        factory_eval::classify_attempt(&missing_executable),
        Outcome::ConfigurationError
    );

    let cancelled = factory_types::TaskAttempt {
        error: Some("Workflow cancelled while the agent was running.".into()),
        status: AttemptStatus::Cancelled,
        ..minimal_attempt()
    };
    assert_eq!(
        factory_eval::classify_attempt(&cancelled),
        Outcome::Cancelled
    );
}

fn minimal_attempt() -> factory_types::TaskAttempt {
    factory_types::TaskAttempt {
        id: 1,
        task_id: 1,
        attempt_number: 1,
        agent: "codex".into(),
        role: Some("worker".into()),
        operation: Some(TaskOperation::Implement),
        status: AttemptStatus::Running,
        started_at: "2026-01-01T00:00:00Z".into(),
        finished_at: None,
        worktree_path: "w".into(),
        source_base: None,
        commit_sha: None,
        exit_code: None,
        error: None,
        evidence: None,
        review: None,
    }
}

// --- history-level checks -----------------------------------------------------

#[test]
fn first_pass_and_eventual_success_are_distinct() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();

    // codex: 8 immediate approvals, 2 approvals that needed a rework round.
    for index in 0..8 {
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 20_000)],
            now - Duration::days(20 - index as i64),
            &["src/lib.rs"],
        );
    }
    for _ in 0..2 {
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[
                AttemptSpec::by("codex", AttemptStatus::ChangesRequested)
                    .error("needs tests")
                    .durations(80_000, 15_000),
                AttemptSpec::by("codex", AttemptStatus::Approved).durations(70_000, 15_000),
            ],
            now - Duration::days(5),
            &["src/lib.rs"],
        );
    }

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let codex = report.metrics_for("codex");
    assert_eq!(codex.tasks_attempted, 10);
    assert_eq!(codex.attempts, 12);
    assert_eq!(codex.qualifying_tasks, 10);
    assert_eq!(codex.first_pass_approval.successes, 8);
    assert_eq!(codex.first_pass_approval.rate, Some(0.8));
    assert_eq!(codex.eventual_approval.rate, Some(1.0));
    assert_eq!(codex.outcome_counts.approved, 10);
    assert_eq!(codex.outcome_counts.first_pass_approved, 8);
    assert_eq!(codex.request_changes.rate, Some(0.2));
    assert_eq!(codex.retry_rate.rate, Some(0.2));
    assert_eq!(codex.terminal_failure.rate, Some(0.0));
    assert_eq!(codex.avg_attempts_per_successful, Some(1.2));
    assert!(codex.first_pass_approval.reliable, "n=10 is reliable");
}

#[test]
fn agent_failures_terminal_and_within_retries_are_agent_failures() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();

    // opencode fails once then gets approved; qwen fails all three attempts.
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[
            AttemptSpec::by("opencode", AttemptStatus::Failed)
                .error("Worker process exited with code 2.")
                .exit(2),
            AttemptSpec::by("opencode", AttemptStatus::Approved).durations(50_000, 10_000),
        ],
        now - Duration::days(3),
        &[],
    );
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[
            AttemptSpec::by("qwen", AttemptStatus::Failed)
                .error("Worker process exited with code 1.")
                .exit(1),
            AttemptSpec::by("qwen", AttemptStatus::Failed)
                .error("Worker process exited with code 1.")
                .exit(1),
            AttemptSpec::by("qwen", AttemptStatus::Failed)
                .error("Worker process exited with code 1.")
                .exit(1),
        ],
        now - Duration::days(3),
        &[],
    );

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let opencode = report.metrics_for("opencode");
    assert_eq!(opencode.tasks_attempted, 1);
    assert_eq!(opencode.eventual_approval.rate, Some(1.0));
    assert_eq!(opencode.first_pass_approval.rate, Some(0.0));
    assert_eq!(opencode.attempts_per_task, Some(2.0));

    let qwen = report.metrics_for("qwen");
    assert_eq!(qwen.outcome_counts.agent_failed, 1);
    assert_eq!(qwen.terminal_failure.rate, Some(1.0));
    assert_eq!(qwen.outcome_counts.changes_requested, 0);
}

#[test]
fn cancellations_policy_and_configuration_failures_are_excluded() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();

    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 10_000)],
        now - Duration::days(2),
        &[],
    );
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Cancelled)
            .error("Workflow cancelled while the agent was running.")],
        now - Duration::days(2),
        &[],
    );
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Interrupted)
            .error("Factory stopped while this attempt was running.")],
        now - Duration::days(2),
        &[],
    );
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Failed)
            .error("blocked by policy: write outside scope")],
        now - Duration::days(2),
        &[],
    );
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Failed).error(
            "executable `codex` was not found in the PATH visible to Factory (0 entries checked)",
        )],
        now - Duration::days(2),
        &[],
    );

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let codex = report.metrics_for("codex");
    assert_eq!(codex.tasks_attempted, 5);
    // Only the approved task qualifies; the four exclusions do not deflate
    // the approval rate.
    assert_eq!(codex.qualifying_tasks, 1);
    assert_eq!(codex.first_pass_approval.rate, Some(1.0));
    assert_eq!(codex.outcome_counts.cancelled, 1);
    assert_eq!(codex.outcome_counts.interrupted, 1);
    assert_eq!(codex.outcome_counts.policy_blocked, 1);
    assert_eq!(codex.outcome_counts.configuration_error, 1);
    assert_eq!(codex.outcome_counts.agent_failed, 0);
}

#[test]
fn durations_separate_execution_review_and_total() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    for seconds in [40u64, 60, 80, 100, 120, 140] {
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(seconds * 1000, 25_000)],
            now - Duration::days(1),
            &[],
        );
    }

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let codex = report.metrics_for("codex");
    let execution = &codex.execution_duration;
    assert_eq!(execution.samples, 6);
    assert_eq!(execution.median_ms, Some(90_000));
    // nearest-rank p95 of 6 samples = ceil(0.95*6) = 6th value
    assert_eq!(execution.p95_ms, Some(140_000));
    assert_eq!(execution.approximate_samples, 0);
    assert_eq!(codex.review_duration.median_ms, Some(25_000));
    assert!(execution.reliable && codex.review_duration.reliable);
    // Total duration is wall-clock across attempts, so it includes review
    // time and overhead beyond pure execution.
    assert!(codex.total_duration.median_ms.unwrap() > 90_000);
}

#[test]
fn legacy_attempts_without_session_timers_fall_back_to_wall_time() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Approved)],
        now - Duration::days(40),
        &[],
    );

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let execution = &report.metrics_for("codex").execution_duration;
    assert_eq!(execution.samples, 1);
    assert_eq!(execution.approximate_samples, 1);
    assert!(
        execution.median_ms.unwrap() >= 70_000,
        "wall time of the synthetic attempt"
    );
}

#[test]
fn windows_filter_by_latest_attempt_and_keep_full_attempt_history() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();

    // Recent task: started 2 days ago, approved first pass.
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 10_000)],
        now - Duration::days(2),
        &[],
    );
    // Old task: entirely outside both windows.
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Failed)
            .error("Worker process exited with code 1.")
            .exit(1)],
        now - Duration::days(45),
        &[],
    );
    // Crossed task: first attempt 10 days ago (changes requested), retry
    // 3 days ago (approved). Belongs to the 7d window with its full history.
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[
            AttemptSpec::by("codex", AttemptStatus::ChangesRequested)
                .error("polish it")
                .starting_at(now - Duration::days(10)),
            AttemptSpec::by("codex", AttemptStatus::Approved)
                .durations(50_000, 10_000)
                .starting_at(now - Duration::days(3)),
        ],
        now - Duration::days(10),
        &[],
    );

    let all = evaluate(&builder.db, &query(), now).unwrap();
    assert_eq!(all.metrics_for("codex").tasks_attempted, 3);

    let mut week = query();
    week.window = PerformanceWindow::Last7Days;
    let week_report = evaluate(&builder.db, &week, now).unwrap();
    let codex = week_report.metrics_for("codex");
    assert_eq!(codex.tasks_attempted, 2);
    // The crossed task keeps its two attempts: not a fake first-pass.
    assert_eq!(codex.attempts, 3);
    assert_eq!(codex.first_pass_approval.rate, Some(0.5));

    let mut month = query();
    month.window = PerformanceWindow::Last30Days;
    let month_report = evaluate(&builder.db, &month, now).unwrap();
    assert_eq!(month_report.metrics_for("codex").tasks_attempted, 2);
}

#[test]
fn role_operation_and_language_breakdowns_expose_specialization() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();

    for _ in 0..6 {
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 10_000)],
            now - Duration::days(1),
            &["src/lib.rs"],
        );
    }
    // codex is a poor reviewer on review-operation tasks.
    for _ in 0..4 {
        builder.task(
            "security_auditor",
            TaskOperation::Review,
            &[AttemptSpec::by("codex", AttemptStatus::ChangesRequested).error("unclear findings")],
            now - Duration::days(1),
            &[],
        );
    }
    // TypeScript work for another agent.
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("opencode", AttemptStatus::Approved).durations(80_000, 10_000)],
        now - Duration::days(1),
        &["src/app.ts", "src/component.tsx"],
    );

    let detail = evaluate_agent(&builder.db, "codex", &query(), now)
        .unwrap()
        .expect("codex has history");
    let implement = detail
        .by_operation
        .iter()
        .find(|entry| entry.key == "implement")
        .unwrap();
    assert_eq!(implement.metrics.tasks_attempted, 6);
    assert_eq!(implement.metrics.first_pass_approval.rate, Some(1.0));
    let review = detail
        .by_operation
        .iter()
        .find(|entry| entry.key == "review")
        .unwrap();
    assert_eq!(review.metrics.tasks_attempted, 4);
    assert_eq!(review.metrics.first_pass_approval.rate, Some(0.0));

    let worker_role = detail
        .by_role
        .iter()
        .find(|entry| entry.key == "worker")
        .unwrap();
    assert_eq!(worker_role.metrics.tasks_attempted, 6);

    let rust = detail
        .by_language
        .iter()
        .find(|entry| entry.key == "rust")
        .unwrap();
    assert_eq!(rust.metrics.tasks_attempted, 6);

    // Language filter on the overview data.
    let mut rust_only = query();
    rust_only.language = Some("rust".into());
    let report = evaluate(&builder.db, &rust_only, now).unwrap();
    assert_eq!(report.metrics_for("codex").tasks_attempted, 6);
    assert!(report
        .agents
        .iter()
        .all(|summary| summary.agent != "opencode"));

    // Facets expose the observed filter values.
    let report = evaluate(&builder.db, &query(), now).unwrap();
    assert!(report.facets.languages.contains(&"rust".to_string()));
    assert!(report.facets.languages.contains(&"typescript".to_string()));
    assert!(report.facets.operations.contains(&"implement".to_string()));
    assert!(report
        .facets
        .roles
        .contains(&"security_auditor".to_string()));
}

#[test]
fn multi_language_tasks_count_in_every_language_bucket() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 10_000)],
        now - Duration::days(1),
        &["src/main.rs", "web/app.ts"],
    );

    let detail = evaluate_agent(&builder.db, "codex", &query(), now)
        .unwrap()
        .unwrap();
    let keys: Vec<&str> = detail.by_language.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, vec!["rust", "typescript"]);
}

#[test]
fn integration_outcomes_are_tracked_without_blaming_the_agent() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();

    // Task 1: approved, integrated cleanly.
    let clean_task = builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 10_000)],
        now - Duration::days(2),
        &[],
    );
    let clean_attempt = builder.db.latest_task_attempt(clean_task).unwrap().unwrap();
    builder.record_integration(
        clean_task,
        clean_attempt.id,
        "codex",
        IntegrationOutcomeKind::Clean,
    );

    // Task 2: approved by the worker but the integration rebase conflicted;
    // the attempt itself is left in `reviewing` exactly like the runtime does.
    let conflict_task = builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Reviewing)],
        now - Duration::days(1),
        &[],
    );
    let conflict_attempt = builder
        .db
        .latest_task_attempt(conflict_task)
        .unwrap()
        .unwrap();
    builder.record_integration(
        conflict_task,
        conflict_attempt.id,
        "codex",
        IntegrationOutcomeKind::Conflict,
    );

    // Task 3: approved from a stale base, rebased onto the run head.
    let rebased_task = builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(80_000, 10_000)],
        now - Duration::days(1),
        &[],
    );
    let rebased_attempt = builder
        .db
        .latest_task_attempt(rebased_task)
        .unwrap()
        .unwrap();
    builder.record_integration(
        rebased_task,
        rebased_attempt.id,
        "codex",
        IntegrationOutcomeKind::Rebased,
    );

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let codex = report.metrics_for("codex");
    assert_eq!(codex.integration.clean, 1);
    assert_eq!(codex.integration.rebased, 1);
    assert_eq!(codex.integration.conflict, 1);
    assert_eq!(codex.integration.conflict_rate.rate, Some(1.0 / 3.0));
    // The conflicted task is classified as an integration conflict, not an
    // agent failure, and is excluded from quality denominators.
    assert_eq!(codex.outcome_counts.integration_conflict, 1);
    assert_eq!(codex.outcome_counts.agent_failed, 0);
    assert_eq!(codex.outcome_counts.in_progress, 0);
    assert_eq!(codex.qualifying_tasks, 2);
    assert_eq!(codex.first_pass_approval.rate, Some(1.0));
}

#[test]
fn attribution_goes_to_the_first_attempts_agent() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[
            AttemptSpec::by("codex", AttemptStatus::Failed)
                .error("Worker process exited with code 1.")
                .exit(1),
            AttemptSpec::by("opencode", AttemptStatus::Approved).durations(60_000, 10_000),
        ],
        now - Duration::days(1),
        &[],
    );

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let codex = report.metrics_for("codex");
    assert_eq!(codex.tasks_attempted, 1);
    // codex attempted the task; opencode's rescue is eventual success under
    // codex's attribution, but never a first pass for codex.
    assert_eq!(codex.first_pass_approval.rate, Some(0.0));
    assert_eq!(codex.eventual_approval.rate, Some(1.0));
    assert!(report
        .agents
        .iter()
        .all(|summary| summary.agent != "opencode"));
}

#[test]
fn trends_compare_recent_tasks_and_week_over_week() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();

    // This week: 4/5 first pass.
    for index in 0..5 {
        let spec = if index == 4 {
            AttemptSpec::by("codex", AttemptStatus::ChangesRequested).error("fix")
        } else {
            AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 10_000)
        };
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[spec],
            now - Duration::days(index as i64 + 1),
            &[],
        );
    }
    // Last week: 1/5 first pass.
    for index in 0..5 {
        let spec = if index == 0 {
            AttemptSpec::by("codex", AttemptStatus::Approved).durations(80_000, 10_000)
        } else {
            AttemptSpec::by("codex", AttemptStatus::ChangesRequested).error("fix")
        };
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[spec],
            now - Duration::days(index as i64 + 8),
            &[],
        );
    }

    let detail = evaluate_agent(&builder.db, "codex", &query(), now)
        .unwrap()
        .unwrap();
    let trend = &detail.trend;
    assert_eq!(trend.recent_10.first_pass.successes, 5);
    assert_eq!(trend.recent_10.first_pass.total, 10);
    let weekly = trend.weekly.as_ref().expect("both weeks have data");
    assert_eq!(weekly.current.first_pass.rate, Some(0.8));
    assert_eq!(weekly.previous.first_pass.rate, Some(0.2));
    let delta = weekly.first_pass_delta_pp.expect("both rates known");
    assert!((delta - 60.0).abs() < 1e-9, "delta {delta}");
}

#[test]
fn reasons_surface_rework_and_failure_causes() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    for _ in 0..3 {
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[AttemptSpec::by("codex", AttemptStatus::ChangesRequested)
                .error("missing unit tests for parser")],
            now - Duration::days(1),
            &[],
        );
    }
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Failed)
            .error("Worker process exited with code 3.")
            .exit(3)],
        now - Duration::days(1),
        &[],
    );

    let detail = evaluate_agent(&builder.db, "codex", &query(), now)
        .unwrap()
        .unwrap();
    let top_rework = detail.rework_reasons.first().expect("rework reasons");
    assert_eq!(top_rework.count, 3);
    assert!(top_rework.reason.contains("missing unit tests"));
    let top_failure = detail.failure_reasons.first().expect("failure reasons");
    assert!(top_failure.reason.contains("exited with code 3"));
}

#[test]
fn small_samples_are_flagged_unreliable() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    for _ in 0..2 {
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[AttemptSpec::by("qwen", AttemptStatus::Approved)],
            now - Duration::days(1),
            &[],
        );
    }

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let first_pass = &report.metrics_for("qwen").first_pass_approval;
    assert_eq!(first_pass.total, 2);
    assert_eq!(first_pass.rate, Some(1.0));
    assert!(
        !first_pass.reliable,
        "n=2 must not be presented as reliable"
    );
}

#[test]
fn parallel_sessions_across_attempts_are_attributed_correctly() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    // Two tasks executed in parallel with overlapping timestamps: durations
    // must not leak between them.
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(120_000, 30_000)],
        now - Duration::days(1),
        &[],
    );
    builder.task(
        "worker",
        TaskOperation::Implement,
        &[AttemptSpec::by("opencode", AttemptStatus::Approved).durations(40_000, 20_000)],
        now - Duration::days(1),
        &[],
    );

    let report = evaluate(&builder.db, &query(), now).unwrap();
    assert_eq!(
        report.metrics_for("codex").execution_duration.median_ms,
        Some(120_000)
    );
    assert_eq!(
        report.metrics_for("opencode").execution_duration.median_ms,
        Some(40_000)
    );
    assert_eq!(
        report.metrics_for("codex").review_duration.median_ms,
        Some(30_000)
    );
}

#[test]
fn review_operation_sessions_count_as_execution_not_review_time() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    // A specialized-review task: its single review-operation session is the
    // agent's own execution time.
    builder.task(
        "security_auditor",
        TaskOperation::Review,
        &[AttemptSpec::by("claude", AttemptStatus::Approved).durations(45_000, 0)],
        now - Duration::days(1),
        &[],
    );

    let report = evaluate(&builder.db, &query(), now).unwrap();
    let claude = report.metrics_for("claude");
    assert_eq!(claude.execution_duration.median_ms, Some(45_000));
    assert_eq!(claude.review_duration.samples, 0);
}

#[test]
fn performance_function_answers_the_future_routing_question() {
    let (mut builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    for _ in 0..4 {
        builder.task(
            "worker",
            TaskOperation::Implement,
            &[AttemptSpec::by("codex", AttemptStatus::Approved).durations(90_000, 10_000)],
            now - Duration::days(1),
            &["src/lib.rs"],
        );
    }

    let metrics = performance(
        &builder.db,
        "codex",
        Some("worker"),
        Some(TaskOperation::Implement),
        Some("rust"),
        now,
    )
    .unwrap()
    .expect("metrics exist");
    assert_eq!(metrics.tasks_attempted, 4);
    assert_eq!(metrics.first_pass_approval.rate, Some(1.0));

    let none = performance(&builder.db, "codex", Some("reviewer"), None, None, now).unwrap();
    assert!(none.is_none(), "no reviewer history for codex");
}

#[test]
fn evaluate_agent_returns_none_for_unknown_agents() {
    let (builder, _dir) = HistoryBuilder::new();
    let now = Utc::now();
    let detail = evaluate_agent(&builder.db, "ghost", &query(), now).unwrap();
    assert!(detail.is_none());
}
