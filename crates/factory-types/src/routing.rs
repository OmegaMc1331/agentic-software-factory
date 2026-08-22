use serde::{Deserialize, Serialize};

use crate::artifact::TaskOperation;

/// How the scheduler picks the agent that runs a task. Existing projects
/// default to [`RoutingMode::RoundRobin`], which preserves the historical
/// deterministic capacity-aware selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    /// Least-loaded pool member with round-robin tie-breaking (the original
    /// Factory behavior).
    #[default]
    RoundRobin,
    /// Deterministic score computed from reliable `factory-eval` history,
    /// falling back to round-robin whenever the evidence is insufficient.
    Performance,
    /// Explicit selection: each task's pinned agent, or the role's preferred
    /// assignment when no pin exists.
    Manual,
}

impl RoutingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RoutingMode::RoundRobin => "round_robin",
            RoutingMode::Performance => "performance",
            RoutingMode::Manual => "manual",
        }
    }
}

impl std::str::FromStr for RoutingMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "round_robin" => Ok(RoutingMode::RoundRobin),
            "performance" => Ok(RoutingMode::Performance),
            "manual" => Ok(RoutingMode::Manual),
            other => Err(format!(
                "unknown routing mode '{other}' (expected round_robin, performance, or manual)"
            )),
        }
    }
}

impl std::fmt::Display for RoutingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One candidate in a routing decision: the agent, its routing score when the
/// performance evidence was reliable enough to rank it, and a short note that
/// names the evidence slice the score came from (or why there is no score).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingCandidateScore {
    pub agent: String,
    /// The deterministic routing score in `[0, 1+]`; `None` when the agent's
    /// history is too thin to rank (`insufficient data`).
    pub score: Option<f64>,
    pub reliable: bool,
    pub note: String,
}

/// A compact, durable audit record of why a specific agent was selected for
/// one dispatch. One row is written per agent invocation (a retried task
/// produces one row per attempt; a worker attempt plus its built-in review
/// produces two rows distinguished by role/operation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingDecision {
    pub id: i64,
    pub task_id: i64,
    pub attempt_id: Option<i64>,
    /// The configured routing mode that produced this decision
    /// (`round_robin`, `performance`, or `manual`).
    pub mode: String,
    pub selected_agent: String,
    pub role: Option<String>,
    pub operation: Option<TaskOperation>,
    pub language: Option<String>,
    pub candidate_scores: Vec<RoutingCandidateScore>,
    pub reason: String,
    pub created_at: String,
}

/// What the router would do for a task right now. Informational only: the
/// real selection happens at dispatch time, when capacity is reserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingPreview {
    pub mode: String,
    pub task_id: i64,
    pub role: Option<String>,
    pub operation: Option<TaskOperation>,
    pub language: Option<String>,
    /// The agent pinned by the user, when one is set.
    pub override_agent: Option<String>,
    /// The agent the router would currently select.
    pub likely_agent: Option<String>,
    pub reason: String,
    pub candidates: Vec<RoutingCandidateScore>,
}
