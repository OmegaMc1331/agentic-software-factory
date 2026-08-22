use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Ready,
    Running,
    /// The task's latest attempt passed main review and is queued for the
    /// per-workflow, serialized integration lane.
    AwaitingIntegration,
    /// The task is currently being integrated onto the run branch.
    Integrating,
    Blocked,
    Failed,
    Completed,
    /// The task was superseded by a partial replan: it no longer advances the
    /// run but its rows and dependency edges are kept for visual reference.
    /// Never scheduled, never considered for completion or blocking.
    Superseded,
}

impl TaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Ready => "ready",
            TaskState::Running => "running",
            TaskState::AwaitingIntegration => "awaiting_integration",
            TaskState::Integrating => "integrating",
            TaskState::Blocked => "blocked",
            TaskState::Failed => "failed",
            TaskState::Completed => "completed",
            TaskState::Superseded => "superseded",
        }
    }
}

impl std::str::FromStr for TaskState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(TaskState::Pending),
            "ready" => Ok(TaskState::Ready),
            "running" => Ok(TaskState::Running),
            "awaiting_integration" => Ok(TaskState::AwaitingIntegration),
            "integrating" => Ok(TaskState::Integrating),
            "blocked" => Ok(TaskState::Blocked),
            "failed" => Ok(TaskState::Failed),
            "completed" => Ok(TaskState::Completed),
            "superseded" => Ok(TaskState::Superseded),
            other => Err(format!("unknown task state '{other}'")),
        }
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: i64,
    pub run_id: i64,
    pub title: String,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub state: TaskState,
    pub position: i32,
    pub dependencies: Vec<i64>,
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    /// The semantic operation for this task. Nullable for rows persisted by
    /// older releases; Factory Core derives a compatible default when missing.
    #[serde(default)]
    pub operation: Option<crate::artifact::TaskOperation>,
    /// The agent pinned by the user for this task (manual routing override).
    /// Honored by every routing mode; validated against role assignment,
    /// policies, and availability at dispatch time. `None` lets the router
    /// choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_override: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
