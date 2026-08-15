use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Ready,
    Running,
    Blocked,
    Failed,
    Completed,
}

impl TaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Ready => "ready",
            TaskState::Running => "running",
            TaskState::Blocked => "blocked",
            TaskState::Failed => "failed",
            TaskState::Completed => "completed",
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
            "blocked" => Ok(TaskState::Blocked),
            "failed" => Ok(TaskState::Failed),
            "completed" => Ok(TaskState::Completed),
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
    pub created_at: String,
    pub updated_at: String,
}
