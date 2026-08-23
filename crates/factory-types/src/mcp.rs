use serde::{Deserialize, Serialize};

/// Factory-owned safety classification for an MCP tool. Server annotations are
/// inputs to this value, never permissions in their own right.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolClassification {
    Read,
    Write,
    Destructive,
    #[default]
    Unknown,
}

impl ToolClassification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
            Self::Unknown => "unknown",
        }
    }

    pub fn requires_exact_permission(self) -> bool {
        matches!(self, Self::Write | Self::Destructive | Self::Unknown)
    }
}

impl std::str::FromStr for ToolClassification {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "destructive" => Ok(Self::Destructive),
            "unknown" => Ok(Self::Unknown),
            _ => Err(format!("unknown MCP tool classification '{value}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolMetadata {
    pub server: String,
    pub name: String,
    pub qualified_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
    pub classification: ToolClassification,
    pub discovered_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolGrant {
    pub id: String,
    pub token_hash: String,
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    pub attempt_id: Option<i64>,
    pub session_id: Option<i64>,
    pub agent: String,
    pub role: String,
    pub operation: Option<String>,
    pub allowed_tools: Vec<String>,
    pub expires_at: String,
    pub revoked_at: Option<String>,
}

impl ToolGrant {
    pub fn active_at(&self, timestamp: &str) -> bool {
        self.revoked_at.is_none() && self.expires_at.as_str() > timestamp
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInvocation {
    pub id: i64,
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    pub attempt_id: Option<i64>,
    pub session_id: Option<i64>,
    pub agent: String,
    pub role: String,
    pub server: String,
    pub tool: String,
    pub classification: ToolClassification,
    pub status: String,
    pub started_at: String,
    pub duration_ms: Option<u64>,
    pub input_summary: String,
    pub output_summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolMetrics {
    pub calls: usize,
    pub successes: usize,
    pub failures: usize,
    pub median_duration_ms: Option<u64>,
}
