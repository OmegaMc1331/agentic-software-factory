use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use factory_db::{FactoryDb, SessionDuration};
use factory_types::{IntegrationOutcome, IntegrationOutcomeKind, TaskAttempt, TaskOperation};

use crate::language::detect_languages;
use crate::outcome::{
    classify_attempt, classify_task, session_is_review, Outcome, TaskClassification,
};
use crate::stats::{DurationStats, RateStats};
use crate::window::PerformanceWindow;

/// Outcome distribution of an agent's attributed tasks, including the
/// infrastructure/control outcomes that are excluded from quality rates.
/// Every category is visible so exclusions can be audited rather than hidden.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeCounts {
    pub approved: u64,
    pub first_pass_approved: u64,
    pub changes_requested: u64,
    pub agent_failed: u64,
    pub integration_conflict: u64,
    pub cancelled: u64,
    pub interrupted: u64,
    pub policy_blocked: u64,
    pub configuration_error: u64,
    pub in_progress: u64,
}

/// Downstream integration quality for an agent's approved work. These are
/// separate signals: a rebase conflict is not counted as an agent failure.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStats {
    /// Integrations that landed with a clean fast-forward.
    pub clean: u64,
    /// Integrations that landed after a stale-base rebase.
    pub rebased: u64,
    /// Integrations that failed with rebase conflicts.
    pub conflict: u64,
    /// `clean / (clean + rebased + conflict)`.
    pub clean_rate: RateStats,
    /// `conflict / (clean + rebased + conflict)`.
    pub conflict_rate: RateStats,
}

impl IntegrationStats {
    fn from_records(records: &[&TaskRecord]) -> Self {
        let mut clean = 0;
        let mut rebased = 0;
        let mut conflict = 0;
        for record in records {
            match record.integration {
                Some(IntegrationOutcomeKind::Clean) => clean += 1,
                Some(IntegrationOutcomeKind::Rebased) => rebased += 1,
                Some(IntegrationOutcomeKind::Conflict) => conflict += 1,
                None => {}
            }
        }
        let attempts = clean + rebased + conflict;
        IntegrationStats {
            clean,
            rebased,
            conflict,
            clean_rate: RateStats::new(clean, attempts),
            conflict_rate: RateStats::new(conflict, attempts),
        }
    }
}

/// All computed performance metrics for one agent over one slice of tasks.
/// There is deliberately no single opaque "agent score": consumers see the
/// components and their sample sizes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMetrics {
    pub tasks_attempted: u64,
    pub attempts: u64,
    /// Mean attempts over every attributed task.
    pub attempts_per_task: Option<f64>,
    /// Mean attempts over tasks that were eventually approved.
    pub avg_attempts_per_successful: Option<f64>,
    /// Tasks whose terminal outcome is agent-attributable — the denominator
    /// of every quality rate below.
    pub qualifying_tasks: u64,
    pub outcome_counts: OutcomeCounts,
    /// Task accepted on attempt #1 (first-pass approval).
    pub first_pass_approval: RateStats,
    /// Task eventually approved, retries included.
    pub eventual_approval: RateStats,
    /// Any attempt of the task received a request-changes decision.
    pub request_changes: RateStats,
    /// Task needed more than one attempt.
    pub retry_rate: RateStats,
    /// Task ended in an agent-caused failure.
    pub terminal_failure: RateStats,
    /// Agent execution time (worker sessions, no scheduler wait).
    pub execution_duration: DurationStats,
    /// Built-in review time spent on this agent's attempts.
    pub review_duration: DurationStats,
    /// Wall-clock first-attempt-start to last-attempt-finish per task.
    pub total_duration: DurationStats,
    pub integration: IntegrationStats,
}

/// Compact per-agent summary used by the overview list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPerformanceSummary {
    pub agent: String,
    pub metrics: AgentMetrics,
}

/// One row of a role/operation/language breakdown.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceBreakdownEntry {
    /// `role`, `operation`, or `language`.
    pub dimension: String,
    /// Stable key within the dimension (e.g. `worker`, `implement`, `rust`).
    pub key: String,
    pub metrics: AgentMetrics,
}

/// A truncated, normalized reason string with its occurrence count.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonCount {
    pub reason: String,
    pub count: u64,
}

/// First-pass approval and median execution over a window of tasks.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendWindow {
    pub label: String,
    pub first_pass: RateStats,
    pub median_execution_ms: Option<u64>,
}

/// `Last 7 days` vs `Previous 7 days` comparison.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendComparison {
    pub current: TrendWindow,
    pub previous: TrendWindow,
    /// Difference of first-pass rates in percentage points, when both sides
    /// have samples.
    pub first_pass_delta_pp: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendSummary {
    /// First-pass approval over the most recent 10 and 25 qualifying tasks.
    pub recent_10: TrendWindow,
    pub recent_25: TrendWindow,
    pub weekly: Option<TrendComparison>,
}

/// Full evaluation result for a single agent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPerformanceDetail {
    pub summary: AgentPerformanceSummary,
    pub by_role: Vec<PerformanceBreakdownEntry>,
    pub by_operation: Vec<PerformanceBreakdownEntry>,
    pub by_language: Vec<PerformanceBreakdownEntry>,
    pub trend: TrendSummary,
    /// Top review reasons behind request-changes outcomes.
    pub rework_reasons: Vec<ReasonCount>,
    /// Top error strings behind agent failures.
    pub failure_reasons: Vec<ReasonCount>,
}

/// Filter values observed in the evaluated history, for building dropdowns.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceFacets {
    pub roles: Vec<String>,
    pub operations: Vec<String>,
    pub languages: Vec<String>,
}

/// The immutable workflow history reduced to one record per task, joined
/// with lean session timings and integration outcomes.
pub(crate) struct TaskRecord {
    pub(crate) agent: String,
    pub(crate) role: String,
    pub(crate) operation: Option<TaskOperation>,
    pub(crate) languages: BTreeSet<String>,
    attempts: u32,
    classification: TaskClassification,
    last_started: Option<DateTime<Utc>>,
    exec_ms: Option<u64>,
    exec_approx: bool,
    review_ms: Option<u64>,
    total_ms: Option<u64>,
    integration: Option<IntegrationOutcomeKind>,
    pub(crate) rework_reasons: Vec<String>,
    pub(crate) failure_reasons: Vec<String>,
}

/// The slice of history a query evaluates: every task (with its full attempt
/// sequence) whose latest attempt started inside the window.
pub(crate) struct History {
    records: Vec<TaskRecord>,
}

impl History {
    pub(crate) fn load(db: &FactoryDb) -> factory_db::Result<Self> {
        let attempts = db.list_task_attempts_all()?;
        let sessions = db.list_session_durations()?;
        let integrations = db.list_integration_outcomes()?;

        let mut sessions_by_attempt: HashMap<i64, Vec<&SessionDuration>> = HashMap::new();
        for session in &sessions {
            sessions_by_attempt
                .entry(session.attempt_id)
                .or_default()
                .push(session);
        }
        let mut integration_by_task: HashMap<i64, IntegrationOutcome> = HashMap::new();
        // later rows overwrite earlier ones; ids are insertion-ordered
        for outcome in integrations {
            integration_by_task.insert(outcome.task_id, outcome);
        }

        let mut attempts_by_task: BTreeMap<i64, Vec<TaskAttempt>> = BTreeMap::new();
        for attempt in attempts {
            attempts_by_task
                .entry(attempt.task_id)
                .or_default()
                .push(attempt);
        }

        let mut records = Vec::with_capacity(attempts_by_task.len());
        for (task_id, mut task_attempts) in attempts_by_task {
            task_attempts.sort_by_key(|attempt| attempt.attempt_number);
            records.push(build_record(
                task_attempts,
                &sessions_by_attempt,
                integration_by_task.get(&task_id),
            ));
        }
        Ok(History { records })
    }

    /// Records visible in `window`, before agent/role/operation/language
    /// filters. Used for facets.
    pub(crate) fn in_window(
        &self,
        window: PerformanceWindow,
        now: DateTime<Utc>,
    ) -> Vec<&TaskRecord> {
        let Some(since) = window.since(now) else {
            return self.records.iter().collect();
        };
        self.records
            .iter()
            .filter(|record| record.started_within(since))
            .collect()
    }
}

fn build_record(
    attempts: Vec<TaskAttempt>,
    sessions_by_attempt: &HashMap<i64, Vec<&SessionDuration>>,
    integration: Option<&IntegrationOutcome>,
) -> TaskRecord {
    let agent = attempts[0].agent.clone();
    let role = attempts
        .iter()
        .find_map(|attempt| attempt.role.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let operation = attempts.iter().find_map(|attempt| attempt.operation);

    let mut languages = BTreeSet::new();
    let mut rework_reasons = Vec::new();
    let mut failure_reasons = Vec::new();

    let mut exec_ms: u64 = 0;
    let mut exec_samples = 0u64;
    let mut exec_approx = false;
    let mut review_ms: u64 = 0;
    let mut wall_exec_ms: u64 = 0;
    let mut wall_samples = 0u64;

    let mut first_started: Option<DateTime<Utc>> = None;
    let mut last_finished: Option<DateTime<Utc>> = None;
    let mut last_started: Option<DateTime<Utc>> = None;

    for attempt in &attempts {
        if let Some(evidence) = &attempt.evidence {
            languages.extend(detect_languages(&evidence.changed_files));
        }
        if let Some(started) = parse_timestamp(&attempt.started_at) {
            first_started = Some(first_started.map_or(started, |first| first.min(started)));
            last_started = Some(last_started.map_or(started, |last| last.max(started)));
        }
        if let Some(finished) = attempt.finished_at.as_deref().and_then(parse_timestamp) {
            last_finished = Some(last_finished.map_or(finished, |last| last.max(finished)));
        }

        match classify_attempt(attempt) {
            Outcome::ChangesRequested => {
                if let Some(reason) = attempt.error.as_deref().map(normalize_reason) {
                    rework_reasons.push(reason);
                }
            }
            Outcome::AgentFailed => {
                if let Some(reason) = attempt.error.as_deref().map(normalize_reason) {
                    failure_reasons.push(reason);
                }
            }
            _ => {}
        }

        let mut attempt_review_ms = 0u64;
        let mut attempt_exec_ms = 0u64;
        let mut timed_exec_session = false;
        if let Some(sessions) = sessions_by_attempt.get(&attempt.id) {
            for session in sessions {
                let Some(duration) = session.duration_ms else {
                    continue;
                };
                if session_is_review(session.operation, attempt.operation) {
                    attempt_review_ms += duration;
                } else {
                    attempt_exec_ms += duration;
                    timed_exec_session = true;
                }
            }
        }
        review_ms += attempt_review_ms;
        if attempt_exec_ms > 0 {
            exec_ms += attempt_exec_ms;
            exec_samples += 1;
        } else if !timed_exec_session {
            // No worker session timer survived (legacy rows): fall back to
            // the attempt's wall time, flagged as approximate.
            if let (Some(started), Some(finished)) = (
                parse_timestamp(&attempt.started_at),
                attempt.finished_at.as_deref().and_then(parse_timestamp),
            ) {
                if finished > started {
                    wall_exec_ms += ((finished - started).num_milliseconds().max(0)) as u64;
                    wall_samples += 1;
                }
            }
        }
    }

    let exec_ms = if exec_samples > 0 {
        Some(exec_ms)
    } else if wall_samples > 0 {
        exec_approx = true;
        Some(wall_exec_ms)
    } else {
        None
    };

    let total_ms = match (first_started, last_finished) {
        (Some(first), Some(last)) if last > first => {
            Some(((last - first).num_milliseconds().max(0)) as u64)
        }
        _ => None,
    };

    let integration_conflict =
        integration.is_some_and(|outcome| outcome.outcome == IntegrationOutcomeKind::Conflict);
    let classification = classify_task(&attempts, integration_conflict);

    TaskRecord {
        agent,
        role,
        operation,
        languages,
        attempts: attempts.len() as u32,
        classification,
        last_started,
        exec_ms,
        exec_approx,
        review_ms: (review_ms > 0).then_some(review_ms),
        total_ms,
        integration: integration.map(|outcome| outcome.outcome),
        rework_reasons,
        failure_reasons,
    }
}

impl TaskRecord {
    fn started_within(&self, since: DateTime<Utc>) -> bool {
        self.last_started.is_some_and(|started| started >= since)
    }
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

/// First line, trimmed, capped so one verbose reviewer does not dominate the
/// reason table.
fn normalize_reason(reason: &str) -> String {
    let first_line = reason.lines().next().unwrap_or("").trim();
    let mut normalized: String = first_line.chars().take(180).collect();
    if normalized.chars().count() < first_line.chars().count() {
        normalized.push('…');
    }
    normalized
}

pub(crate) fn record_matches(
    record: &TaskRecord,
    agent: Option<&str>,
    role: Option<&str>,
    operation: Option<TaskOperation>,
    language: Option<&str>,
) -> bool {
    if let Some(agent) = agent {
        if record.agent != agent {
            return false;
        }
    }
    if let Some(role) = role {
        if record.role != role {
            return false;
        }
    }
    if let Some(operation) = operation {
        if record.operation != Some(operation) {
            return false;
        }
    }
    if let Some(language) = language {
        if !record.languages.contains(language) {
            return false;
        }
    }
    true
}

pub(crate) fn aggregate(records: &[&TaskRecord]) -> AgentMetrics {
    let mut counts = OutcomeCounts::default();
    let mut qualifying = 0u64;
    let mut first_pass = 0u64;
    let mut had_changes = 0u64;
    let mut retried = 0u64;
    let mut attempts_total = 0u64;
    let mut successful_attempts: Vec<u32> = Vec::new();

    let mut exec_samples: Vec<u64> = Vec::new();
    let mut exec_approx = 0u64;
    let mut review_samples: Vec<u64> = Vec::new();
    let mut total_samples: Vec<u64> = Vec::new();

    for record in records {
        attempts_total += record.attempts as u64;
        match record.classification.outcome {
            Outcome::Approved => {
                counts.approved += 1;
                counts.first_pass_approved += u64::from(record.classification.first_pass);
                successful_attempts.push(record.attempts);
            }
            Outcome::ChangesRequested => counts.changes_requested += 1,
            Outcome::AgentFailed => counts.agent_failed += 1,
            Outcome::IntegrationConflict => counts.integration_conflict += 1,
            Outcome::Cancelled => counts.cancelled += 1,
            Outcome::Interrupted => counts.interrupted += 1,
            Outcome::PolicyBlocked => counts.policy_blocked += 1,
            Outcome::ConfigurationError => counts.configuration_error += 1,
            Outcome::InProgress => counts.in_progress += 1,
        }
        if record.classification.qualifying {
            qualifying += 1;
            first_pass += u64::from(record.classification.first_pass);
            had_changes += u64::from(record.classification.had_changes_requested);
            retried += u64::from(record.attempts > 1);
        }
        if let Some(exec) = record.exec_ms {
            exec_samples.push(exec);
            exec_approx += u64::from(record.exec_approx);
        }
        if let Some(review) = record.review_ms {
            review_samples.push(review);
        }
        if let Some(total) = record.total_ms {
            total_samples.push(total);
        }
    }

    let tasks = records.len() as u64;
    let avg_attempts_per_successful = (!successful_attempts.is_empty()).then(|| {
        successful_attempts
            .iter()
            .map(|&attempts| attempts as f64)
            .sum::<f64>()
            / successful_attempts.len() as f64
    });

    AgentMetrics {
        tasks_attempted: tasks,
        attempts: attempts_total,
        attempts_per_task: (tasks > 0).then(|| attempts_total as f64 / tasks as f64),
        avg_attempts_per_successful,
        qualifying_tasks: qualifying,
        outcome_counts: counts,
        first_pass_approval: RateStats::new(first_pass, qualifying),
        eventual_approval: RateStats::new(counts.approved, qualifying),
        request_changes: RateStats::new(had_changes, qualifying),
        retry_rate: RateStats::new(retried, qualifying),
        terminal_failure: RateStats::new(counts.agent_failed, qualifying),
        execution_duration: DurationStats::from_samples_ms(&exec_samples, exec_approx),
        review_duration: DurationStats::from_samples_ms(&review_samples, 0),
        total_duration: DurationStats::from_samples_ms(&total_samples, 0),
        integration: IntegrationStats::from_records(records),
    }
}

pub(crate) fn facets(records: &[&TaskRecord]) -> PerformanceFacets {
    let mut roles = BTreeSet::new();
    let mut operations = BTreeSet::new();
    let mut languages = BTreeSet::new();
    for record in records {
        roles.insert(record.role.clone());
        if let Some(operation) = record.operation {
            operations.insert(operation.as_str().to_string());
        }
        languages.extend(record.languages.iter().cloned());
    }
    PerformanceFacets {
        roles: roles.into_iter().collect(),
        operations: operations.into_iter().collect(),
        languages: languages.into_iter().collect(),
    }
}

pub(crate) fn breakdown(
    dimension: &str,
    records: &[&TaskRecord],
    key_of: impl Fn(&TaskRecord) -> Vec<String>,
) -> Vec<PerformanceBreakdownEntry> {
    let mut grouped: BTreeMap<String, Vec<&TaskRecord>> = BTreeMap::new();
    for record in records {
        for key in key_of(record) {
            grouped.entry(key).or_default().push(record);
        }
    }
    let mut entries: Vec<PerformanceBreakdownEntry> = grouped
        .into_iter()
        .map(|(key, group)| PerformanceBreakdownEntry {
            dimension: dimension.to_string(),
            key,
            metrics: aggregate(&group),
        })
        .collect();
    entries.sort_by(|a, b| {
        b.metrics
            .tasks_attempted
            .cmp(&a.metrics.tasks_attempted)
            .then_with(|| a.key.cmp(&b.key))
    });
    entries
}

pub(crate) fn top_reasons(reasons: &[String], limit: usize) -> Vec<ReasonCount> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for reason in reasons {
        *counts.entry(reason.clone()).or_default() += 1;
    }
    let mut counts: Vec<ReasonCount> = counts
        .into_iter()
        .map(|(reason, count)| ReasonCount { reason, count })
        .collect();
    counts.sort_by_key(|reason| std::cmp::Reverse((reason.count, reason.reason.clone())));
    counts.truncate(limit);
    counts
}

fn trend_window(records: &[&TaskRecord], label: &str) -> TrendWindow {
    let samples: Vec<u64> = records.iter().filter_map(|record| record.exec_ms).collect();
    TrendWindow {
        label: label.to_string(),
        first_pass: RateStats::new(
            records
                .iter()
                .filter(|record| record.classification.first_pass)
                .count() as u64,
            records.len() as u64,
        ),
        median_execution_ms: DurationStats::from_samples_ms(&samples, 0).median_ms,
    }
}

/// Trends are computed over the agent's filtered history ignoring the window
/// filter (the 7d-vs-previous-7d comparison inherently spans windows).
/// Qualifying tasks are ordered most recent first; tasks without parseable
/// timestamps sort last.
pub(crate) fn trend_summary(records: &[&TaskRecord], now: DateTime<Utc>) -> TrendSummary {
    let mut qualifying: Vec<&TaskRecord> = records
        .iter()
        .filter(|record| record.classification.qualifying)
        .copied()
        .collect();
    qualifying.sort_by_key(|record| std::cmp::Reverse(record.last_started));

    let recent_10 = trend_window(&qualifying[..qualifying.len().min(10)], "Recent 10 tasks");
    let recent_25 = trend_window(&qualifying[..qualifying.len().min(25)], "Recent 25 tasks");

    let week_start = now - chrono::Duration::days(7);
    let two_weeks_start = now - chrono::Duration::days(14);
    let current: Vec<&TaskRecord> = qualifying
        .iter()
        .filter(|record| record.started_within(week_start))
        .copied()
        .collect();
    let previous: Vec<&TaskRecord> = qualifying
        .iter()
        .filter(|record| {
            record
                .last_started
                .is_some_and(|started| started < week_start && started >= two_weeks_start)
        })
        .copied()
        .collect();

    let weekly = (!current.is_empty() || !previous.is_empty()).then(|| {
        let current_window = trend_window(&current, "Last 7 days");
        let previous_window = trend_window(&previous, "Previous 7 days");
        let first_pass_delta_pp = match (
            current_window.first_pass.rate,
            previous_window.first_pass.rate,
        ) {
            (Some(current), Some(previous)) => Some((current - previous) * 100.0),
            _ => None,
        };
        TrendComparison {
            current: current_window,
            previous: previous_window,
            first_pass_delta_pp,
        }
    });

    TrendSummary {
        recent_10,
        recent_25,
        weekly,
    }
}

/// Collect reasons from records, capped to the top `limit`.
pub(crate) fn collect_reasons(
    records: &[&TaskRecord],
    select: impl Fn(&TaskRecord) -> &Vec<String>,
    limit: usize,
) -> Vec<ReasonCount> {
    let mut reasons = Vec::new();
    for record in records {
        reasons.extend(select(record).iter().cloned());
    }
    top_reasons(&reasons, limit)
}
