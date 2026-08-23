use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::routing::RoutingMode;

/// The routing selection a recipe may make. `ProjectDefault` means "use
/// whatever `[routing]` mode the project is configured with"; the other
/// variants pin the run's routing mode. A recipe can never select `manual` —
/// manual routing is per-task pinning owned by the routing engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeRouting {
    #[default]
    ProjectDefault,
    RoundRobin,
    Performance,
}

impl RecipeRouting {
    pub fn as_str(self) -> &'static str {
        match self {
            RecipeRouting::ProjectDefault => "project_default",
            RecipeRouting::RoundRobin => "round_robin",
            RecipeRouting::Performance => "performance",
        }
    }

    /// Resolves the run's routing mode: the recipe's explicit mode wins over
    /// the project default, which is supplied by the caller.
    pub fn resolve(self, project_default: RoutingMode) -> RoutingMode {
        match self {
            RecipeRouting::ProjectDefault => project_default,
            RecipeRouting::RoundRobin => RoutingMode::RoundRobin,
            RecipeRouting::Performance => RoutingMode::Performance,
        }
    }
}

impl std::str::FromStr for RecipeRouting {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "project_default" => Ok(RecipeRouting::ProjectDefault),
            "round_robin" => Ok(RecipeRouting::RoundRobin),
            "performance" => Ok(RecipeRouting::Performance),
            other => Err(format!(
                "unknown recipe routing '{other}' (expected project_default, round_robin, or \
                 performance)"
            )),
        }
    }
}

impl std::fmt::Display for RecipeRouting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Optional GitHub delivery defaults applied to a run created from a recipe.
/// Every field stays a *default*: the user can still override it when the
/// pull request is created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeDelivery {
    /// Whether the pull request form defaults to a draft PR.
    #[serde(default)]
    pub draft: bool,
}

/// A reusable workflow configuration: **how** Factory should approach work.
/// A recipe never stores the task DAG, task ids, attempts, sessions, or Git
/// state — the Planner always generates the actual plan from the objective.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    /// Stable identifier (slug). Immutable once the recipe exists.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Selected role ids, in preference order. Empty only for the built-in
    /// `custom` placeholder, which pre-fills nothing.
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub routing: RecipeRouting,
    /// Preferred per-run task concurrency (1..=32); `None` keeps the project
    /// `[runtime] max_parallel_tasks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel_tasks: Option<usize>,
    /// Role id -> existing policy preset name. Presets may only *narrow* the
    /// Policy Engine's effective scopes; they can never widen them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub policy_presets: BTreeMap<String, String>,
    /// Optional qualified MCP tool patterns. When non-empty this is an
    /// only-narrowing selection applied to every role in the run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Bounded planning context handed to the Planner alongside the objective.
    #[serde(default)]
    pub planner_guidance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<RecipeDelivery>,
    /// `true` for the read-only built-in catalog; custom recipes are editable.
    #[serde(default)]
    pub builtin: bool,
}

impl Recipe {
    /// The immutable copy persisted on a run at creation time. Later edits to
    /// the recipe never reach runs that already carry a snapshot.
    pub fn snapshot(&self) -> RecipeSnapshot {
        RecipeSnapshot {
            id: self.id.clone(),
            name: self.name.clone(),
            roles: self.roles.clone(),
            routing: self.routing,
            max_parallel_tasks: self.max_parallel_tasks,
            policy_presets: self.policy_presets.clone(),
            tools: self.tools.clone(),
            planner_guidance: self.planner_guidance.clone(),
            github: self.github,
        }
    }
}

/// The immutable recipe configuration persisted with a workflow. Stored as
/// JSON on the run; enough to reproduce and audit how the run was configured
/// even after the source recipe is edited or deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSnapshot {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub routing: RecipeRouting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel_tasks: Option<usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub policy_presets: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default)]
    pub planner_guidance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<RecipeDelivery>,
}

impl RecipeSnapshot {
    /// The policy preset this snapshot narrows the given role with, when any.
    pub fn policy_preset_for(&self, role: &str) -> Option<&str> {
        self.policy_presets.get(role).map(String::as_str)
    }

    /// The run's effective task-concurrency preference, clamped to the same
    /// safety bounds the project runtime config uses.
    pub fn effective_max_parallel_tasks(&self, project_default: usize) -> usize {
        self.max_parallel_tasks
            .unwrap_or(project_default)
            .clamp(1, config_limits::MAX_PARALLEL_TASKS_LIMIT)
    }
}

/// Shared bounds mirrored from factory-core so the pure type layer can clamp
/// without a dependency edge. Kept identical to
/// `factory_core::config::MAX_PARALLEL_TASKS_LIMIT` by test.
pub mod config_limits {
    pub const MAX_PARALLEL_TASKS_LIMIT: usize = 32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_recipe() -> Recipe {
        Recipe {
            id: "backend_feature".into(),
            name: "Backend Feature".into(),
            description: "Backend implementation.".into(),
            roles: vec!["planner".into(), "worker".into(), "reviewer".into()],
            routing: RecipeRouting::Performance,
            max_parallel_tasks: Some(3),
            policy_presets: BTreeMap::from([("worker".to_string(), "implementation".into())]),
            tools: vec!["docs.*".into()],
            planner_guidance: "Prefer a compact plan.".into(),
            github: None,
            builtin: false,
        }
    }

    #[test]
    fn snapshot_copies_configuration_without_builtin_marker() {
        let recipe = sample_recipe();
        let snapshot = recipe.snapshot();
        assert_eq!(snapshot.id, recipe.id);
        assert_eq!(snapshot.roles, recipe.roles);
        assert_eq!(snapshot.routing, RecipeRouting::Performance);
        assert_eq!(snapshot.max_parallel_tasks, Some(3));
        assert_eq!(snapshot.policy_preset_for("worker"), Some("implementation"));
        assert_eq!(snapshot.tools, vec!["docs.*"]);
        assert!(snapshot.policy_preset_for("reviewer").is_none());
        // Snapshots carry configuration only; there is no task or history data
        // to copy because the model has none.
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("builtin"));
    }

    #[test]
    fn recipe_routing_parses_and_resolves() {
        assert_eq!(
            "project_default".parse::<RecipeRouting>().unwrap(),
            RecipeRouting::ProjectDefault
        );
        assert_eq!(
            "round_robin".parse::<RecipeRouting>().unwrap(),
            RecipeRouting::RoundRobin
        );
        assert_eq!(
            "performance".parse::<RecipeRouting>().unwrap(),
            RecipeRouting::Performance
        );
        assert!("manual".parse::<RecipeRouting>().is_err());
        assert_eq!(
            RecipeRouting::ProjectDefault.resolve(RoutingMode::Manual),
            RoutingMode::Manual
        );
        assert_eq!(
            RecipeRouting::RoundRobin.resolve(RoutingMode::Performance),
            RoutingMode::RoundRobin
        );
        assert_eq!(
            RecipeRouting::Performance.resolve(RoutingMode::RoundRobin),
            RoutingMode::Performance
        );
    }

    #[test]
    fn concurrency_preference_is_clamped_to_safety_bounds() {
        let mut snapshot = sample_recipe().snapshot();
        snapshot.max_parallel_tasks = Some(0);
        assert_eq!(snapshot.effective_max_parallel_tasks(4), 1);
        snapshot.max_parallel_tasks = Some(999);
        assert_eq!(snapshot.effective_max_parallel_tasks(4), 32);
        snapshot.max_parallel_tasks = None;
        assert_eq!(snapshot.effective_max_parallel_tasks(4), 4);
    }

    #[test]
    fn recipe_round_trips_through_json() {
        let recipe = sample_recipe();
        let json = serde_json::to_string(&recipe).unwrap();
        assert!(json.contains("\"maxParallelTasks\":3"));
        let parsed: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, recipe);
    }
}
