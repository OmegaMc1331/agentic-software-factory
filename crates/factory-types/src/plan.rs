use serde::{Deserialize, Serialize};

use crate::artifact::TaskOperation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedTask {
    pub id: String,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The operation this task performs. The Planner may specify it; when
    /// absent the runtime derives a compatible default from the role's
    /// execution class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<TaskOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub objective: String,
    pub tasks: Vec<PlannedTask>,
}
