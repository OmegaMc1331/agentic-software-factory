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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: i64,
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    pub attempt_id: Option<i64>,
    pub role: String,
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
}
