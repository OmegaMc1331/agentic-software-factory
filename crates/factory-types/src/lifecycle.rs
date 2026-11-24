use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionLifecycleState {
    Active,
    SuspendRequested,
    Quiescing,
    Suspended,
    Reconciling,
    RecoveryRequired,
    Settled,
}

impl MissionLifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::SuspendRequested => "suspend_requested",
            Self::Quiescing => "quiescing",
            Self::Suspended => "suspended",
            Self::Reconciling => "reconciling",
            Self::RecoveryRequired => "recovery_required",
            Self::Settled => "settled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissionLifecycle {
    pub schema_version: u32,
    pub project_id: String,
    pub workspace_session_id: String,
    pub run_id: i64,
    pub generation: i64,
    pub state: MissionLifecycleState,
    pub reason: Option<String>,
    pub checkpoint_id: Option<i64>,
    pub problem: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissionCheckpoint {
    pub schema_version: u32,
    pub id: i64,
    pub project_id: String,
    pub workspace_session_id: String,
    pub run_id: i64,
    pub generation: i64,
    pub plan_revision: i64,
    pub integration_commit: Option<String>,
    pub worktrees: Vec<CheckpointWorktree>,
    pub records: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointWorktree {
    pub task_id: i64,
    pub path: String,
    pub head_commit: String,
    pub base_commit: Option<String>,
    pub status_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSessionLifecycle {
    pub project_id: String,
    pub session_id: String,
    pub generation: i64,
    pub closed: bool,
    pub missions: Vec<MissionLifecycle>,
    pub events: Vec<Value>,
}
