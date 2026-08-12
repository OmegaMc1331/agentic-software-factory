use serde::{Deserialize, Serialize};

use crate::task::{Task, TaskState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Planned,
    Active,
    Completed,
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Planned => "planned",
            RunStatus::Active => "active",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
        }
    }

    pub fn from_tasks(tasks: &[Task]) -> RunStatus {
        if tasks.is_empty() {
            return RunStatus::Planned;
        }
        if tasks.iter().all(|t| t.state == TaskState::Completed) {
            return RunStatus::Completed;
        }
        if tasks.iter().any(|t| t.state == TaskState::Failed) {
            return RunStatus::Failed;
        }
        let started = tasks.iter().any(|t| {
            matches!(
                t.state,
                TaskState::Running | TaskState::Completed | TaskState::Blocked
            )
        });
        if started {
            RunStatus::Active
        } else {
            RunStatus::Planned
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: i64,
    pub objective: String,
    pub status: RunStatus,
    pub planner_agent: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
