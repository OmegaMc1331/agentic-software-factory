use factory_types::{AttemptStatus, TaskAttempt, TaskOperation};

/// The centralized classification of how a task attempt (or the task it
/// belongs to) ended. Every metric in this crate is derived from this single
/// evaluator so success, failure, and exclusion rules stay consistent.
///
/// Not every non-success is an agent-quality failure: cancellations, restart
/// interruptions, policy rejections, and configuration problems are
/// infrastructure or control outcomes. They are counted separately and
/// excluded from agent-quality denominators (see `is_agent_quality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The work was accepted by review (the attempt reached `approved`).
    Approved,
    /// Review requested changes and the task ended without a later approval.
    ChangesRequested,
    /// The agent process failed (non-zero exit, crash, or invalid output).
    AgentFailed,
    /// The integration rebase conflicted; the work never landed.
    IntegrationConflict,
    /// The user cancelled the workflow while the attempt was in flight.
    Cancelled,
    /// A Factory restart interrupted the attempt.
    Interrupted,
    /// The policy engine rejected the attempt's output.
    PolicyBlocked,
    /// The agent executable or invocation was misconfigured.
    ConfigurationError,
    /// The attempt is still running or under review.
    InProgress,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Approved => "approved",
            Outcome::ChangesRequested => "changes_requested",
            Outcome::AgentFailed => "agent_failed",
            Outcome::IntegrationConflict => "integration_conflict",
            Outcome::Cancelled => "cancelled",
            Outcome::Interrupted => "interrupted",
            Outcome::PolicyBlocked => "policy_blocked",
            Outcome::ConfigurationError => "configuration_error",
            Outcome::InProgress => "in_progress",
        }
    }

    /// Whether the outcome is terminal and attributable to the agent's
    /// quality. Only these outcomes enter the denominators of approval,
    /// rework, and failure rates.
    pub fn is_agent_quality(self) -> bool {
        matches!(
            self,
            Outcome::Approved | Outcome::ChangesRequested | Outcome::AgentFailed
        )
    }
}

/// Error-string prefixes that identify configuration problems rather than
/// agent execution failures. These mirror the `Display` strings of
/// `factory_agent::AgentError` and `factory_core::AgentResolutionError`, which
/// are the durable record of why an attempt aborted; the exact list is part of
/// the contract documented in `docs/evaluations.md`.
const CONFIGURATION_ERROR_PREFIXES: &[&str] = &[
    // AgentError::ExecutableNotFound
    "executable `",
    // AgentError::InvalidExecutable
    "resolved Windows executable is invalid",
    // AgentError::Spawn
    "failed to run `",
    // AgentError::AutomatedUnavailable / AgentResolutionError::AutomatedUnavailable
    "has no non-interactive",
    // AgentError::InteractiveUnavailable
    "has no interactive invocation configured",
    // AgentError::InvalidInvocation
    "Invalid invocation for agent",
    // AgentError::RequiresTerminal
    "appears to require an interactive terminal",
    // AgentResolutionError
    "No agent is assigned to the",
    "is not available. Check the agent configuration",
    "executable installation is broken",
];

/// The error-string prefix the runtime writes when the policy engine rejects
/// an attempt's evidence.
const POLICY_BLOCKED_PREFIX: &str = "blocked by policy:";

/// Classifies one attempt from its durable status and error string.
pub fn classify_attempt(attempt: &TaskAttempt) -> Outcome {
    match attempt.status {
        AttemptStatus::Approved => Outcome::Approved,
        AttemptStatus::ChangesRequested => Outcome::ChangesRequested,
        AttemptStatus::Cancelled => Outcome::Cancelled,
        AttemptStatus::Interrupted => Outcome::Interrupted,
        AttemptStatus::Running | AttemptStatus::Reviewing => Outcome::InProgress,
        AttemptStatus::Failed => {
            let error = attempt.error.as_deref().unwrap_or("");
            if error.starts_with(POLICY_BLOCKED_PREFIX) {
                Outcome::PolicyBlocked
            } else if CONFIGURATION_ERROR_PREFIXES
                .iter()
                .any(|prefix| error.starts_with(prefix) || error.contains(&format!(" {prefix}")))
            {
                Outcome::ConfigurationError
            } else {
                Outcome::AgentFailed
            }
        }
    }
}

/// The classified result of a whole task (its attempt sequence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskClassification {
    /// The task's terminal outcome (eventual approval wins over per-attempt
    /// outcomes; a recorded integration conflict wins over an attempt stuck
    /// in review).
    pub outcome: Outcome,
    /// The task was accepted on attempt #1 without implementation rework.
    pub first_pass: bool,
    /// Any attempt of this task received a request-changes decision.
    pub had_changes_requested: bool,
    /// Whether the terminal outcome counts toward agent-quality rates.
    pub qualifying: bool,
}

/// Classifies a task from its attempts (ordered by attempt number) and
/// whether an integration conflict was recorded for it.
pub fn classify_task(attempts: &[TaskAttempt], integration_conflict: bool) -> TaskClassification {
    let mut first = Outcome::InProgress;
    let mut had_changes_requested = false;
    let mut last = Outcome::InProgress;

    for (index, attempt) in attempts.iter().enumerate() {
        let outcome = classify_attempt(attempt);
        if index == 0 {
            first = outcome;
        }
        if outcome == Outcome::ChangesRequested {
            had_changes_requested = true;
        }
        last = outcome;
    }

    let outcome = if attempts.iter().any(|a| a.status == AttemptStatus::Approved) {
        Outcome::Approved
    } else if integration_conflict {
        Outcome::IntegrationConflict
    } else {
        last
    };

    let qualifying = outcome.is_agent_quality();
    TaskClassification {
        outcome,
        first_pass: qualifying && outcome == Outcome::Approved && first == Outcome::Approved,
        had_changes_requested,
        qualifying,
    }
}

/// Splits a session's operation into execution vs review time for the attempt
/// it belongs to. A `review` session attached to a non-review attempt is the
/// built-in review of someone else's work; a `review` session on a
/// review-operation attempt is that agent's own execution.
pub fn session_is_review(
    session_operation: Option<TaskOperation>,
    attempt_operation: Option<TaskOperation>,
) -> bool {
    session_operation == Some(TaskOperation::Review)
        && attempt_operation != Some(TaskOperation::Review)
}
