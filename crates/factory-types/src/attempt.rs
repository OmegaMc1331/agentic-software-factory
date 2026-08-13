use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Running,
    Reviewing,
    Approved,
    ChangesRequested,
    Failed,
    Interrupted,
    Cancelled,
}

impl AttemptStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AttemptStatus::Running => "running",
            AttemptStatus::Reviewing => "reviewing",
            AttemptStatus::Approved => "approved",
            AttemptStatus::ChangesRequested => "changes_requested",
            AttemptStatus::Failed => "failed",
            AttemptStatus::Interrupted => "interrupted",
            AttemptStatus::Cancelled => "cancelled",
        }
    }
}

impl std::str::FromStr for AttemptStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "running" => Ok(AttemptStatus::Running),
            "reviewing" => Ok(AttemptStatus::Reviewing),
            "approved" => Ok(AttemptStatus::Approved),
            "changes_requested" => Ok(AttemptStatus::ChangesRequested),
            "failed" => Ok(AttemptStatus::Failed),
            "interrupted" => Ok(AttemptStatus::Interrupted),
            "cancelled" => Ok(AttemptStatus::Cancelled),
            other => Err(format!("unknown attempt status '{other}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvidence {
    pub changed_files: Vec<String>,
    pub diff_summary: String,
    pub commit_sha: Option<String>,
    pub commands: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub worker_exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResult {
    pub decision: ReviewDecision,
    pub reason: String,
    #[serde(default)]
    pub feedback: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAttempt {
    pub id: i64,
    pub task_id: i64,
    pub attempt_number: u32,
    pub agent: String,
    pub status: AttemptStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub worktree_path: String,
    pub commit_sha: Option<String>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub evidence: Option<TaskEvidence>,
    pub review: Option<ReviewResult>,
}
