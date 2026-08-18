//! Plan revisioning and typed, atomic plan mutations.
//!
//! The visual plan editor mutates a run's plan through [`PlanMutation`]
//! operations applied inside a single SQLite transaction. Revisions are
//! durable integers bumped on every change; editors that were opened against
//! a stale revision are rejected with an optimistic-concurrency conflict.

use serde::{Deserialize, Serialize};

use crate::artifact::TaskOperation;
use crate::plan::Plan;
use crate::task::Task;

/// Reference to a task from the editor. Draft tasks added in an unsaved
/// session carry a `clientId`; persisted tasks are addressed by database id.
/// Client ids are mapped to real ids only when a patch is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskRef {
    Id(i64),
    ClientId(String),
}

impl TaskRef {
    pub fn id(&self) -> Option<i64> {
        match self {
            TaskRef::Id(id) => Some(*id),
            TaskRef::ClientId(_) => None,
        }
    }

    pub fn client_id(&self) -> Option<&str> {
        match self {
            TaskRef::ClientId(client_id) => Some(client_id.as_str()),
            TaskRef::Id(_) => None,
        }
    }

    /// Human-readable label for diagnostics and messages.
    pub fn label(&self) -> String {
        match self {
            TaskRef::Id(id) => format!("task {id}"),
            TaskRef::ClientId(client_id) => client_id.clone(),
        }
    }
}

/// A single atomic mutation to the run plan. Applied inside one SQLite
/// transaction; the whole patch rolls back if any operation is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PlanMutation {
    /// Inserts a new draft task. `dependencies` may reference existing ids or
    /// the `clientId` of other tasks added in the same patch.
    AddTask {
        client_id: String,
        title: String,
        objective: String,
        #[serde(default)]
        acceptance_criteria: Vec<String>,
        #[serde(default)]
        dependencies: Vec<TaskRef>,
        role: Option<String>,
        operation: Option<TaskOperation>,
    },
    /// Updates an existing task. Optional fields use `Option<Option<T>>` so a
    /// `null` clears the value instead of leaving it untouched.
    UpdateTask {
        task: TaskRef,
        title: Option<String>,
        objective: Option<String>,
        acceptance_criteria: Option<Option<Vec<String>>>,
        role: Option<Option<String>>,
        operation: Option<Option<TaskOperation>>,
    },
    /// Removes a mutable task. Fails if the task has any attempt, is mid-run,
    /// or is depended on by an immutable task.
    RemoveTask { task: TaskRef },
    /// Adds a dependency edge: `task` becomes a dependent of `depends_on`.
    AddDependency { task: TaskRef, depends_on: TaskRef },
    /// Removes the dependency edge `task -> depends_on`.
    RemoveDependency { task: TaskRef, depends_on: TaskRef },
    /// Moves `task` in the plan-ordered task list (visual ordering).
    ReorderTask { task: TaskRef, position: u32 },
}

/// Batch of mutations plus the revision the editor was opened against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPatch {
    pub expected_revision: i64,
    pub mutations: Vec<PlanMutation>,
}

/// Machine-readable validation diagnostic. `task` and `field` are omitted for
/// plan-level diagnostics. The dashboard renders `message` and switches on
/// `error_code` without parsing prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDiagnostic {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub error_code: String,
    pub message: String,
}

/// How a plan revision was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRevisionSource {
    /// Original plan produced by the planner agent.
    Planner,
    /// Manual edit applied through the visual plan editor.
    Manual,
    /// Partial replan produced by the planner agent.
    Replan,
}

impl PlanRevisionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanRevisionSource::Planner => "planner",
            PlanRevisionSource::Manual => "manual",
            PlanRevisionSource::Replan => "replan",
        }
    }
}

impl std::str::FromStr for PlanRevisionSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "planner" => Ok(PlanRevisionSource::Planner),
            "manual" => Ok(PlanRevisionSource::Manual),
            "replan" => Ok(PlanRevisionSource::Replan),
            other => Err(format!("unknown plan revision source '{other}'")),
        }
    }
}

/// Full, durable snapshot of a run's plan at a given revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSnapshot {
    pub objective: String,
    pub tasks: Vec<Task>,
}

/// A durable plan revision for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRevisionRecord {
    pub id: i64,
    pub run_id: i64,
    pub revision: i64,
    pub source: PlanRevisionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_session_id: Option<i64>,
    pub created_at: String,
    pub snapshot: PlanSnapshot,
}

/// Request body for a partial replan. The scope is seeded by `seed` and
/// computed by Core from the real DAG, never trusted from the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplanRequest {
    pub expected_revision: i64,
    /// Immutable task seeding the replan scope.
    pub seed: TaskRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub plan: Plan,
}
