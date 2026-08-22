use serde::{Deserialize, Serialize};

/// How an approved attempt's work landed on the run's integration branch.
///
/// These are downstream quality signals, not agent verdicts: a rebase conflict
/// usually reflects concurrent work rather than a coding-agent mistake, so the
/// evaluation reports integration outcomes separately from agent quality
/// metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationOutcomeKind {
    /// The run branch fast-forwarded to the attempt's head: the attempt was
    /// based on the current integration head.
    Clean,
    /// The attempt was based on a stale head and was rebased onto the current
    /// integration branch before landing.
    Rebased,
    /// The rebase failed with conflicts; the attempt never landed and the
    /// workflow stopped.
    Conflict,
}

impl IntegrationOutcomeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            IntegrationOutcomeKind::Clean => "clean",
            IntegrationOutcomeKind::Rebased => "rebased",
            IntegrationOutcomeKind::Conflict => "conflict",
        }
    }
}

impl std::str::FromStr for IntegrationOutcomeKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "clean" => Ok(IntegrationOutcomeKind::Clean),
            "rebased" => Ok(IntegrationOutcomeKind::Rebased),
            "conflict" => Ok(IntegrationOutcomeKind::Conflict),
            other => Err(format!("unknown integration outcome '{other}'")),
        }
    }
}

impl std::fmt::Display for IntegrationOutcomeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The durable record of one approved attempt's integration onto the run
/// branch. Written by the integration step regardless of outcome so the
/// evaluation can measure integration quality from real history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationOutcome {
    pub id: i64,
    pub run_id: i64,
    pub task_id: i64,
    pub attempt_id: i64,
    /// The agent whose approved work was integrated (the attempt's agent).
    pub agent: String,
    pub outcome: IntegrationOutcomeKind,
    pub created_at: String,
}
