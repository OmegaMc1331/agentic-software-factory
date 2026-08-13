use serde::{Deserialize, Serialize};

use crate::task::{Task, TaskState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Planning,
    Planned,
    Active,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Planning => "planning",
            RunStatus::Planned => "planned",
            RunStatus::Active => "active",
            RunStatus::Blocked => "blocked",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
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
        if tasks.iter().any(|t| t.state == TaskState::Blocked)
            && !tasks
                .iter()
                .any(|t| matches!(t.state, TaskState::Ready | TaskState::Running))
        {
            return RunStatus::Blocked;
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

impl std::str::FromStr for RunStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "planning" => Ok(RunStatus::Planning),
            "planned" => Ok(RunStatus::Planned),
            "active" => Ok(RunStatus::Active),
            "blocked" => Ok(RunStatus::Blocked),
            "completed" => Ok(RunStatus::Completed),
            "failed" => Ok(RunStatus::Failed),
            "cancelled" => Ok(RunStatus::Cancelled),
            other => Err(format!("unknown run status '{other}'")),
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
