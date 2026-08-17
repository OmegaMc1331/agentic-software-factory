use serde::Serialize;

use factory_types::{AgentSession, RoleArtifact, Run, Task, TaskAttempt, TaskState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCounts {
    pub pending: usize,
    pub ready: usize,
    pub running: usize,
    pub blocked: usize,
    pub failed: usize,
    pub completed: usize,
    pub total: usize,
}

impl TaskCounts {
    pub fn from_tasks(tasks: &[Task]) -> TaskCounts {
        let mut counts = TaskCounts {
            pending: 0,
            ready: 0,
            running: 0,
            blocked: 0,
            failed: 0,
            completed: 0,
            total: tasks.len(),
        };
        for task in tasks {
            match task.state {
                TaskState::Pending => counts.pending += 1,
                TaskState::Ready => counts.ready += 1,
                TaskState::Running => counts.running += 1,
                TaskState::Blocked => counts.blocked += 1,
                TaskState::Failed => counts.failed += 1,
                TaskState::Completed => counts.completed += 1,
            }
        }
        counts
    }

    pub fn progress(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        self.completed as f32 / self.total as f32
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: i64,
    pub objective: String,
    pub status: String,
    pub planner_agent: Option<String>,
    pub created_at: String,
    pub counts: TaskCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub run: Run,
    pub tasks: Vec<Task>,
    pub attempts: Vec<TaskAttempt>,
    pub sessions: Vec<AgentSession>,
    /// Derived role-aware stage overview of this workflow.
    pub stages: Vec<StageStatus>,
    /// Artifacts persisted by the workflow's advisory/verification/review tasks.
    pub artifacts: Vec<RoleArtifact>,
    /// Per-run integration branch state: where approved implementation-family
    /// work is merged and which tasks contributed commits to it.
    pub integration: RunIntegration,
}

/// The run's `factory/run-<id>` integration branch: its current head and the
/// implementation-family tasks whose latest attempt was approved and integrated
/// into it. `head` is `None` until the first implementation task is approved.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunIntegration {
    pub branch: String,
    pub head: Option<String>,
    pub integrated_tasks: Vec<i64>,
}

/// One derived workflow stage ("what kind of work is happening") with its
/// progress across the tasks that belong to it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageStatus {
    pub key: String,
    pub label: String,
    pub total: usize,
    pub completed: usize,
    /// `active` when any task in the stage is ready or running.
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryResponse {
    pub run_id: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub meta: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: String,
    pub editable: bool,
    pub semantic: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionResponse {
    #[serde(flatten)]
    pub session: AgentSession,
    pub working_directory: String,
    pub interactive: bool,
}
