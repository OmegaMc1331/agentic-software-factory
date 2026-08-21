use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionMode {
    Automated,
    Interactive,
}

impl AgentSessionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automated => "automated",
            Self::Interactive => "interactive",
        }
    }
}

impl std::str::FromStr for AgentSessionMode {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "automated" => Ok(Self::Automated),
            "interactive" => Ok(Self::Interactive),
            _ => Err(()),
        }
    }
}

/// Compact policy audit snapshot persisted with an automated AgentSession:
/// enough to know which policy applied, without storing any secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPolicyAudit {
    /// Where the policy came from (`role:worker`, `role:worker + agent:codex`,
    /// `default` for the permissive legacy policy).
    pub source: String,
    /// `open`, `restricted`, or `read_only`.
    pub filesystem: String,
    /// `allow` or `deny` (enforcement advisory).
    pub network: String,
    /// `filtered` or `full`.
    pub environment: String,
    /// Effective write scopes at session start.
    #[serde(default)]
    pub write_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: i64,
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    pub attempt_id: Option<i64>,
    pub role: String,
    /// The semantic operation performed by this session when it belongs to a
    /// planned task; persisted for historical observability.
    #[serde(default)]
    pub operation: Option<crate::artifact::TaskOperation>,
    pub agent: String,
    pub mode: AgentSessionMode,
    pub command: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    /// Which policy applied to this automated session (null for sessions
    /// recorded before the policy engine or for interactive consoles).
    #[serde(default)]
    pub policy_audit: Option<SessionPolicyAudit>,
}
