use factory_policy::PolicyScope;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProfileInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub provider_adapter_id: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub roles: Vec<String>,
    #[serde(default)]
    pub instructions: String,
    pub permissions: PolicyScope,
    pub max_concurrency: usize,
    pub timeout_secs: u64,
    pub enabled: bool,
    pub automatic_delegation: bool,
    #[serde(default)]
    pub preferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub project_id: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub archived: bool,
    #[serde(flatten)]
    pub configuration: AgentProfileInput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionSnapshot {
    pub schema_version: u32,
    pub profile: AgentProfile,
    pub role: String,
    pub provider_kind: String,
    pub requested_model: Option<String>,
    pub effective_policy: serde_json::Value,
    pub policy_configuration: factory_policy::PoliciesConfig,
    pub selected_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandoffRequest {
    pub idempotency_key: String,
    pub to_profile_id: String,
    pub expected_profile_revision: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoff {
    pub id: String,
    pub project_id: String,
    pub run_id: i64,
    pub task_id: i64,
    pub idempotency_key: String,
    pub from_agent: Option<String>,
    pub from_session_id: Option<i64>,
    pub from_attempt_id: Option<i64>,
    pub target: AgentProfile,
    pub reason: String,
    pub state: String,
    pub boundary: String,
    pub requested_at: String,
    pub updated_at: String,
    pub replacement_session_id: Option<i64>,
    pub evidence: serde_json::Value,
}
