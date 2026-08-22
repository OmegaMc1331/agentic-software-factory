//! Evaluation & agent performance.
//!
//! This crate measures how each configured coding agent actually performs
//! inside Factory workflows by deriving metrics from the immutable workflow
//! history (`task_attempts`, `agent_sessions`, `integration_outcomes`).
//! It observes and reports; it never influences task execution — the
//! scheduler's deterministic capacity-aware selection is untouched.
//!
//! Principles:
//!
//! * real durable evidence only — no LLM judging another LLM, no fabricated
//!   token or cost numbers;
//! * one centralized outcome classifier shared by every metric
//!   ([`outcome`]);
//! * first-pass approval kept separate from eventual approval;
//! * sample sizes always exposed, rates carry Wilson 95% intervals, and
//!   small samples are flagged unreliable instead of presented as truth;
//! * infrastructure/control outcomes (cancellation, restart interruption,
//!   policy rejection, configuration errors) are counted but excluded from
//!   agent-quality denominators;
//! * everything stays local in `.factory`.
//!
//! See `docs/evaluations.md` for the exact metric definitions.

pub mod language;
pub mod outcome;
pub mod stats;
pub mod summary;
pub mod window;

pub use language::{detect_languages, language_label};
pub use outcome::{classify_attempt, classify_task, Outcome, TaskClassification};
pub use stats::{
    DurationStats, RateStats, MIN_RELIABLE_DURATION_SAMPLES, MIN_RELIABLE_RATE_SAMPLES,
};
pub use summary::{
    AgentMetrics, AgentPerformanceDetail, AgentPerformanceSummary, IntegrationStats, OutcomeCounts,
    PerformanceBreakdownEntry, PerformanceFacets, ReasonCount, TrendComparison, TrendSummary,
    TrendWindow,
};
pub use window::PerformanceWindow;

use chrono::{DateTime, Utc};
use factory_db::{DbError, FactoryDb};
use factory_types::TaskOperation;

use summary::{collect_reasons, record_matches, History};

/// The semantic slice of history to evaluate. Filters are simple fixed
/// dimensions — not a general query language.
#[derive(Debug, Clone, Default)]
pub struct PerformanceQuery {
    pub window: PerformanceWindow,
    pub agent: Option<String>,
    pub role: Option<String>,
    pub operation: Option<TaskOperation>,
    pub language: Option<String>,
}

/// The result of evaluating the whole history for one window.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationReport {
    pub window: PerformanceWindow,
    /// Agents sorted by tasks attempted (desc), then name.
    pub agents: Vec<AgentPerformanceSummary>,
    /// Filter values observed in this window's history.
    pub facets: PerformanceFacets,
}

/// Evaluates every agent present in the workflow history.
///
/// A task belongs to the window when its latest attempt started inside it;
/// the task's full attempt history is then evaluated (so first-pass vs
/// eventual-success distinctions stay correct across window boundaries).
pub fn evaluate(
    db: &FactoryDb,
    query: &PerformanceQuery,
    now: DateTime<Utc>,
) -> Result<EvaluationReport, DbError> {
    let history = History::load(db)?;
    let windowed = history.in_window(query.window, now);

    let mut by_agent: std::collections::BTreeMap<String, Vec<&summary::TaskRecord>> =
        std::collections::BTreeMap::new();
    for record in &windowed {
        if !record_matches(
            record,
            query.agent.as_deref(),
            query.role.as_deref(),
            query.operation,
            query.language.as_deref(),
        ) {
            continue;
        }
        by_agent
            .entry(record.agent.clone())
            .or_default()
            .push(*record);
    }

    let mut agents: Vec<AgentPerformanceSummary> = by_agent
        .into_iter()
        .filter(|(agent, _)| {
            // An agent filter with no history still yields its (empty) entry.
            query.agent.as_deref().is_none_or(|filter| filter == agent)
        })
        .map(|(agent, records)| AgentPerformanceSummary {
            agent,
            metrics: summary::aggregate(&records),
        })
        .collect();
    agents.sort_by(|a, b| {
        b.metrics
            .tasks_attempted
            .cmp(&a.metrics.tasks_attempted)
            .then_with(|| a.agent.cmp(&b.agent))
    });

    Ok(EvaluationReport {
        window: query.window,
        agents,
        facets: summary::facets(&windowed),
    })
}

/// Evaluates one agent in depth: summary plus role/operation/language
/// breakdowns, trends, and rework/failure reasons.
///
/// Returns `None` when the agent has no attributed tasks in the window — the
/// caller decides whether that is a 404 or an empty summary.
pub fn evaluate_agent(
    db: &FactoryDb,
    agent: &str,
    query: &PerformanceQuery,
    now: DateTime<Utc>,
) -> Result<Option<AgentPerformanceDetail>, DbError> {
    let history = History::load(db)?;
    let windowed = history.in_window(query.window, now);

    let filtered: Vec<&summary::TaskRecord> = windowed
        .iter()
        .filter(|record| {
            record_matches(
                record,
                Some(agent),
                query.role.as_deref(),
                query.operation,
                query.language.as_deref(),
            )
        })
        .copied()
        .collect();
    if filtered.is_empty() {
        return Ok(None);
    }

    // Trends span window boundaries by design; apply only the
    // role/operation/language filters there.
    let unwindowed = history.in_window(PerformanceWindow::AllTime, now);
    let trend_records: Vec<&summary::TaskRecord> = unwindowed
        .iter()
        .filter(|record| {
            record_matches(
                record,
                Some(agent),
                query.role.as_deref(),
                query.operation,
                query.language.as_deref(),
            )
        })
        .copied()
        .collect();

    let by_role = summary::breakdown("role", &filtered, |record| vec![record.role.clone()]);
    let by_operation = summary::breakdown("operation", &filtered, |record| {
        vec![record
            .operation
            .map(|operation| operation.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string())]
    });
    let by_language = summary::breakdown("language", &filtered, |record| {
        record.languages.iter().cloned().collect()
    });

    Ok(Some(AgentPerformanceDetail {
        summary: AgentPerformanceSummary {
            agent: agent.to_string(),
            metrics: summary::aggregate(&filtered),
        },
        by_role,
        by_operation,
        by_language,
        trend: summary::trend_summary(&trend_records, now),
        rework_reasons: collect_reasons(&filtered, |record| &record.rework_reasons, 8),
        failure_reasons: collect_reasons(&filtered, |record| &record.failure_reasons, 8),
    }))
}

/// The future hook for intelligent agent routing: a deterministic,
/// read-only answer to `performance(agent, role, operation, language?)`.
///
/// The scheduler does NOT call this yet — current deterministic
/// capacity-aware routing is unchanged. The function exists so a later
/// milestone can consume measured performance without re-deriving it.
pub fn performance(
    db: &FactoryDb,
    agent: &str,
    role: Option<&str>,
    operation: Option<TaskOperation>,
    language: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Option<AgentMetrics>, DbError> {
    let query = PerformanceQuery {
        window: PerformanceWindow::AllTime,
        agent: Some(agent.to_string()),
        role: role.map(str::to_string),
        operation,
        language: language.map(str::to_string),
    };
    Ok(evaluate_agent(db, agent, &query, now)?.map(|detail| detail.summary.metrics))
}

/// Which slice of an agent's history a resolved performance value came from.
/// Ordered from most specific to least specific; see [`resolve_performance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceSliceLevel {
    RoleOperationLanguage,
    RoleOperation,
    Role,
    Global,
}

impl PerformanceSliceLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            PerformanceSliceLevel::RoleOperationLanguage => "role+operation+language",
            PerformanceSliceLevel::RoleOperation => "role+operation",
            PerformanceSliceLevel::Role => "role",
            PerformanceSliceLevel::Global => "global",
        }
    }
}

/// A reliable performance slice for one agent: the metrics plus the hierarchy
/// level they were resolved from.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPerformance {
    pub level: PerformanceSliceLevel,
    pub metrics: AgentMetrics,
}

impl ResolvedPerformance {
    /// Quality rate denominators (qualifying tasks) back every quality rate,
    /// so this is the sample size a routing decision was based on.
    pub fn sample_count(&self) -> u64 {
        self.metrics.qualifying_tasks
    }
}

/// Resolves an agent's performance from the most specific reliable slice of
/// its history, falling back through the hierarchy:
///
/// ```text
/// agent + role + operation + language   (when a language is known)
/// agent + role + operation
/// agent + role
/// agent (global)
/// ```
///
/// A slice is used only when its quality rates meet the evaluation
/// reliability requirement (`MIN_RELIABLE_RATE_SAMPLES` qualifying samples),
/// so tiny highly-specific samples never mask large reliable broader ones.
/// Returns `None` when no level has reliable data — the caller must treat the
/// agent as unranked rather than guessing. This is the single performance
/// entry point the scheduler uses; it reuses `evaluate_agent` so no metric
/// formula is duplicated in routing.
pub fn resolve_performance(
    db: &FactoryDb,
    agent: &str,
    role: Option<&str>,
    operation: Option<TaskOperation>,
    language: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Option<ResolvedPerformance>, DbError> {
    /// One hierarchy step: the level plus the filters that select it.
    type Level<'a> = (
        PerformanceSliceLevel,
        Option<&'a str>,
        Option<TaskOperation>,
        Option<&'a str>,
    );
    let mut levels: Vec<Level> = Vec::new();
    if let (Some(_), Some(role), Some(operation)) = (language, role, operation) {
        levels.push((
            PerformanceSliceLevel::RoleOperationLanguage,
            Some(role),
            Some(operation),
            language,
        ));
    }
    if let (Some(role), Some(operation)) = (role, operation) {
        levels.push((
            PerformanceSliceLevel::RoleOperation,
            Some(role),
            Some(operation),
            None,
        ));
    }
    if let Some(role) = role {
        levels.push((PerformanceSliceLevel::Role, Some(role), None, None));
    }
    levels.push((PerformanceSliceLevel::Global, None, None, None));
    for (level, role, operation, language) in levels {
        if let Some(metrics) = performance(db, agent, role, operation, language, now)? {
            if metrics.eventual_approval.reliable {
                return Ok(Some(ResolvedPerformance { level, metrics }));
            }
        }
    }
    Ok(None)
}

/// Convenience for callers that want "now" semantics.
pub fn evaluate_now(db: &FactoryDb, query: &PerformanceQuery) -> Result<EvaluationReport, DbError> {
    evaluate(db, query, Utc::now())
}
