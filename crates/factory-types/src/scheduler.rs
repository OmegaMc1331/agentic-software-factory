use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerPolicy {
    pub max_parallel_tasks: usize,
    pub max_project_tasks: usize,
    pub max_replans: u32,
    pub max_total_tasks: usize,
    pub max_duration_secs: u64,
}

impl Default for SchedulerPolicy {
    fn default() -> Self {
        Self {
            max_parallel_tasks: 4,
            max_project_tasks: 4,
            max_replans: 2,
            max_total_tasks: 128,
            max_duration_secs: 3600,
        }
    }
}

impl SchedulerPolicy {
    pub fn legacy() -> Self {
        Self {
            max_parallel_tasks: 1,
            max_replans: 0,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=32).contains(&self.max_parallel_tasks)
            || !(1..=32).contains(&self.max_project_tasks)
            || self.max_replans > 8
            || !(1..=512).contains(&self.max_total_tasks)
            || !(1..=86400).contains(&self.max_duration_secs)
        {
            return Err("Scheduler bounds: task widths 1-32, replans 0-8, total tasks 1-512, duration 1-86400 seconds.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerEvent {
    pub id: i64,
    pub schema_version: u32,
    pub run_id: i64,
    pub task_id: Option<i64>,
    pub revision: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationRecord {
    pub id: i64,
    pub run_id: i64,
    pub task_id: i64,
    pub attempt_id: i64,
    pub before_sha: String,
    pub source_sha: String,
    pub candidate_sha: Option<String>,
    pub worktree_path: String,
    pub status: String,
    pub details: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}
