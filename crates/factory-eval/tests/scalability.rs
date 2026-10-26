//! ASF-N06 scalability benchmark for the evaluation engine.
//!
//! Not part of CI: run explicitly with
//! `cargo test -p factory-eval --test scalability -- --ignored --nocapture`.
//!
//! Populates a synthetic history (runs, tasks, attempts with realistic
//! evidence payloads, worker/reviewer sessions, integration outcomes) at
//! increasing scales and measures the raw evaluation reads, the data volume
//! they transfer, and the end-to-end `evaluate` latency.

use std::time::Instant;

use factory_db::FactoryDb;
use factory_eval::{evaluate, evaluate_agent, PerformanceQuery, PerformanceWindow};
use factory_types::{
    AgentSessionMode, AttemptStatus, IntegrationOutcomeKind, ReviewDecision, ReviewResult,
    TaskEvidence, TaskOperation, TaskState,
};
use tempfile::TempDir;

/// A realistic diff patch: the review evidence captured for every approved
/// attempt is bounded at 60 KB in production; this ~24 KB stand-in keeps the
/// synthetic data representative without slowing population itself down.
fn diff_patch() -> String {
    "@@ -1,4 +1,6 @@\n+impl FastPath {\n+    fn resolve(&self) -> u32 { self.value }\n+}\n"
        .repeat(600)
}

const AGENTS: &[&str] = &["coder-a", "coder-b", "reviewer-x", "reviewer-y"];

fn populate(attempt_budget: usize) -> (TempDir, FactoryDb) {
    let dir = TempDir::new().unwrap();
    let db = FactoryDb::open(&dir.path().join("bench.sqlite3")).unwrap();
    let mut created = 0usize;
    let mut run_id = 0i64;
    let mut position = 0i32;
    let mut task_index = 0usize;
    while created < attempt_budget {
        if task_index.is_multiple_of(10) {
            run_id = db
                .create_run_with_status("bench run", None, factory_types::RunStatus::Completed)
                .unwrap()
                .id;
            position = 0;
        }
        task_index += 1;
        let task_id = db
            .create_task(
                run_id,
                "benchmark task",
                "benchmark objective",
                &["passes".to_string()],
                TaskState::Completed,
                position,
                Some("worker"),
                Some(TaskOperation::Implement),
            )
            .unwrap();
        position += 1;
        // ~15% of tasks get a retry with a first failed/reviewed attempt.
        let retries = if task_index.is_multiple_of(7) { 1 } else { 0 };
        for round in 0..=retries {
            if created >= attempt_budget {
                break;
            }
            let agent = AGENTS[(task_index + round) % AGENTS.len()];
            let attempt = db
                .create_task_attempt(
                    task_id,
                    "worker",
                    Some(TaskOperation::Implement),
                    agent,
                    "wt",
                    None,
                )
                .unwrap();
            let evidence = TaskEvidence {
                changed_files: vec![
                    format!("src/module{}/main.rs", task_index % 40),
                    "lib/core/text.rs".to_string(),
                    format!("tests/module{}_test.rs", task_index % 40),
                ],
                diff_summary: "3 files changed, 120 insertions".to_string(),
                commit_sha: Some("abc123".to_string()),
                commands: vec!["cargo test".to_string()],
                acceptance_criteria: vec!["passes".to_string()],
                worker_exit_code: Some(0),
                artifacts: Vec::new(),
                diff_patch: Some(diff_patch()),
            };
            let final_round = round == retries;
            let (status, review) = if final_round {
                (
                    AttemptStatus::Approved,
                    Some(ReviewResult {
                        decision: ReviewDecision::Approve,
                        reason: "evidence accepted".to_string(),
                        feedback: Vec::new(),
                    }),
                )
            } else if round % 2 == 0 {
                (
                    AttemptStatus::ChangesRequested,
                    Some(ReviewResult {
                        decision: ReviewDecision::RequestChanges,
                        reason: "needs clearer tests".to_string(),
                        feedback: Vec::new(),
                    }),
                )
            } else {
                (AttemptStatus::Failed, None)
            };
            db.finish_task_attempt(
                attempt.id,
                status,
                Some(0),
                Some("abc123"),
                (status == AttemptStatus::Failed).then_some("agent hit a tool error"),
                Some(&evidence),
                review.as_ref(),
            )
            .unwrap();
            // Worker + reviewer sessions per attempt.
            for (role, operation, duration) in [
                ("worker", Some(TaskOperation::Implement), 90_000u64),
                ("reviewer", Some(TaskOperation::Review), 40_000),
            ] {
                db.insert_agent_session(&factory_types::AgentSession {
                    id: 0,
                    run_id: Some(run_id),
                    task_id: Some(task_id),
                    attempt_id: Some(attempt.id),
                    role: role.to_string(),
                    operation,
                    agent: agent.to_string(),
                    mode: AgentSessionMode::Automated,
                    command: format!("{agent} --run"),
                    status: "success".to_string(),
                    started_at: "2026-10-01T10:00:00Z".to_string(),
                    finished_at: None,
                    exit_code: Some(0),
                    duration_ms: Some(duration),
                    stdout: None,
                    stderr: None,
                    policy_audit: None,
                    output_meta: None,
                })
                .unwrap();
            }
            if final_round && !task_index.is_multiple_of(5) {
                db.record_integration_outcome(
                    run_id,
                    task_id,
                    attempt.id,
                    agent,
                    if task_index.is_multiple_of(11) {
                        IntegrationOutcomeKind::Conflict
                    } else {
                        IntegrationOutcomeKind::Clean
                    },
                )
                .unwrap();
            }
            created += 1;
        }
    }
    (dir, db)
}

#[test]
#[ignore = "ASF-N06 benchmark: run with --ignored --nocapture"]
fn evaluation_scalability_benchmark() {
    let query = PerformanceQuery {
        window: PerformanceWindow::AllTime,
        agent: None,
        role: None,
        operation: None,
        language: None,
    };
    println!("scale | population | raw reads | evidence bytes | evaluate() | evaluate_agent()");
    for scale in [100usize, 1_000, 10_000, 100_000] {
        let populate_started = Instant::now();
        let (_dir, db) = populate(scale);
        let population = populate_started.elapsed();

        let reads_started = Instant::now();
        let attempts = db.list_task_attempts_all().unwrap();
        let sessions = db.list_session_durations().unwrap();
        let integrations = db.list_integration_outcomes().unwrap();
        let reads = reads_started.elapsed();
        let evidence_bytes: usize = attempts
            .iter()
            .map(|attempt| {
                attempt.evidence.as_ref().map_or(0, |evidence| {
                    evidence.diff_patch.as_deref().map_or(0, str::len)
                })
            })
            .sum();
        drop(attempts);
        drop(sessions);
        drop(integrations);

        let evaluate_started = Instant::now();
        let report = evaluate(&db, &query, chrono::Utc::now()).unwrap();
        let evaluate_latency = evaluate_started.elapsed();
        let agent_started = Instant::now();
        let agent_detail = evaluate_agent(&db, AGENTS[0], &query, chrono::Utc::now()).unwrap();
        let agent_latency = agent_started.elapsed();

        println!(
            "{scale:>6} | {:>9.2?} | {:>9.2?} | {:>13.1} MB | {:>9.2?} | {:>9.2?} | agents={}",
            population,
            reads,
            evidence_bytes as f64 / (1024.0 * 1024.0),
            evaluate_latency,
            agent_latency,
            report.agents.len(),
        );
        let _ = agent_detail;
    }
}
