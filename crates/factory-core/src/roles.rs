use std::collections::BTreeMap;

use factory_types::{ArtifactKind, TaskOperation};
use serde::{Deserialize, Serialize};

pub const PLANNER: &str = "planner";
pub const WORKER: &str = "worker";
pub const REVIEWER: &str = "reviewer";
pub const ARCHITECT: &str = "architect";
pub const RESEARCHER: &str = "researcher";
pub const TEST_ENGINEER: &str = "test_engineer";
pub const SECURITY_AUDITOR: &str = "security_auditor";
pub const DOCUMENTATION_WRITER: &str = "documentation_writer";

pub const CORE_ROLE_IDS: [&str; 8] = [
    PLANNER,
    WORKER,
    REVIEWER,
    ARCHITECT,
    RESEARCHER,
    TEST_ENGINEER,
    SECURITY_AUDITOR,
    DOCUMENTATION_WRITER,
];

pub const PIPELINE_ROLE_IDS: [&str; 3] = [PLANNER, WORKER, REVIEWER];

pub const MAX_ROLE_INSTRUCTIONS_CHARS: usize = 16_384;
pub const MAX_ROLE_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    Planning,
    Execution,
    Review,
    Advisory,
    PostProcess,
}

impl ExecutionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionClass::Planning => "planning",
            ExecutionClass::Execution => "execution",
            ExecutionClass::Review => "review",
            ExecutionClass::Advisory => "advisory",
            ExecutionClass::PostProcess => "post_process",
        }
    }
}

impl std::str::FromStr for ExecutionClass {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "planning" => Ok(ExecutionClass::Planning),
            "execution" => Ok(ExecutionClass::Execution),
            "review" => Ok(ExecutionClass::Review),
            "advisory" => Ok(ExecutionClass::Advisory),
            "post_process" => Ok(ExecutionClass::PostProcess),
            other => Err(format!("unknown execution class '{other}'")),
        }
    }
}

/// The operations a role of this execution class may perform.
///
/// This matrix is Factory Core's invariant: a task's `operation` must belong
/// to the class of its role. Planner output that violates it is rejected and
/// repaired instead of being silently reinterpreted.
///
/// ```text
/// planning     -> planning
/// advisory     -> advisory
/// execution    -> implement, verify
/// review       -> review
/// post_process -> post_process, implement (where explicitly allowed)
/// ```
pub fn compatible_operations(class: ExecutionClass) -> &'static [TaskOperation] {
    match class {
        ExecutionClass::Planning => &[TaskOperation::Planning],
        ExecutionClass::Advisory => &[TaskOperation::Advisory],
        ExecutionClass::Execution => &[TaskOperation::Implement, TaskOperation::Verify],
        ExecutionClass::Review => &[TaskOperation::Review],
        ExecutionClass::PostProcess => &[TaskOperation::PostProcess, TaskOperation::Implement],
    }
}

/// Validates that a task operation is compatible with a role's execution
/// class. Returns the mismatch as a human-readable message when invalid.
pub fn validate_operation_compatibility(
    role_id: &str,
    class: ExecutionClass,
    operation: TaskOperation,
) -> Result<(), String> {
    if compatible_operations(class).contains(&operation) {
        Ok(())
    } else {
        let allowed = compatible_operations(class)
            .iter()
            .map(|operation| operation.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "role '{role_id}' (execution class {}) cannot perform operation '{}'; compatible operations: {allowed}",
            class.as_str(),
            operation.as_str()
        ))
    }
}

/// The operation a task defaults to when the plan does not specify one. Role
/// ids win over classes only for the one core role whose natural operation
/// differs from its class default (Test Engineer verifies, Workers implement).
pub fn default_operation_for_role(id: &str, class: ExecutionClass) -> TaskOperation {
    match id {
        TEST_ENGINEER => TaskOperation::Verify,
        _ => match class {
            ExecutionClass::Planning => TaskOperation::Planning,
            ExecutionClass::Advisory => TaskOperation::Advisory,
            ExecutionClass::Execution => TaskOperation::Implement,
            ExecutionClass::Review => TaskOperation::Review,
            ExecutionClass::PostProcess => TaskOperation::PostProcess,
        },
    }
}

/// The artifact kind a task of this role and operation persists. Advisory
/// roles map to their natural context kind; verification and review map to
/// their report kinds; explicit production operations on otherwise-advisory
/// roles persist `analysis`.
pub fn artifact_kind_for(role_id: &str, operation: TaskOperation) -> ArtifactKind {
    match (role_id, operation) {
        (RESEARCHER, _) => ArtifactKind::Research,
        (ARCHITECT, _) => ArtifactKind::Architecture,
        (_, TaskOperation::Verify) => ArtifactKind::Verification,
        (_, TaskOperation::Review) => ArtifactKind::Review,
        (_, TaskOperation::PostProcess) => ArtifactKind::DocumentationContext,
        (_, TaskOperation::Advisory) => ArtifactKind::Analysis,
        (_, TaskOperation::Implement | TaskOperation::Planning) => ArtifactKind::Analysis,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    Core,
    Custom,
}

impl RoleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RoleKind::Core => "core",
            RoleKind::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    pub execution_class: ExecutionClass,
    pub kind: RoleKind,
}

pub fn is_core_role(id: &str) -> bool {
    CORE_ROLE_IDS.contains(&id)
}

pub fn is_pipeline_role(id: &str) -> bool {
    PIPELINE_ROLE_IDS.contains(&id)
}

pub fn core_role(id: &str) -> Option<RoleDefinition> {
    let (name, description, execution_class, instructions) = match id {
        PLANNER => (
            "Planner",
            "Transforms the workflow objective into a valid task DAG.",
            ExecutionClass::Planning,
            "Purpose: decompose the objective into ordered tasks with clear scope.\n\
             Responsibilities:\n\
             - decompose the objective into tasks with distinct responsibilities;\n\
             - define dependencies between tasks;\n\
             - write acceptance criteria that a reviewer can verify;\n\
             - keep each task small enough for one focused implementation pass.\n\
             Boundaries:\n\
             - do not implement code;\n\
             - do not invent tasks outside the objective.\n\
             Expected output: a plan that matches the required JSON schema exactly.",
        ),
        WORKER => (
            "Worker",
            "Implements a planned task in an isolated worktree.",
            ExecutionClass::Execution,
            "Purpose: implement exactly one task and verify it locally.\n\
             Responsibilities:\n\
             - make the code changes the task requires;\n\
             - keep edits scoped to the task;\n\
             - run focused local validation (build, tests, linters) where practical;\n\
             - report evidence of what changed and what was run.\n\
             Boundaries:\n\
             - do not restructure unrelated code;\n\
             - do not weaken or skip tests to make them pass.\n\
             Expected output: implementation plus a concise JSON report with \
             `summary` and `commands`.",
        ),
        REVIEWER => (
            "Reviewer",
            "Independently evaluates task output against acceptance criteria.",
            ExecutionClass::Review,
            "Purpose: decide whether the implementation satisfies the task.\n\
             Responsibilities:\n\
             - check the diff and evidence against every acceptance criterion;\n\
             - verify claimed commands and outcomes are plausible;\n\
             - approve, or request changes with concrete, actionable feedback.\n\
             Boundaries:\n\
             - do not modify files;\n\
             - do not approve on trust without evidence.\n\
             Expected output: one JSON object with `decision`, `reason` and `feedback`.",
        ),
        ARCHITECT => (
            "Architect",
            "Analyzes architecture and technical boundaries around a change.",
            ExecutionClass::Advisory,
            "Purpose: resolve structural questions before or during implementation.\n\
             Responsibilities:\n\
             - identify component boundaries and interfaces affected by the task;\n\
             - describe data flow and technical constraints;\n\
             - propose a migration or rollout strategy when behavior changes;\n\
             - write the decision down where the task requires it.\n\
             Boundaries:\n\
             - do not silently restructure unrelated subsystems;\n\
             - implementation only when the task explicitly includes it.\n\
             Expected output: implementation or a design note the task can build on.",
        ),
        RESEARCHER => (
            "Researcher",
            "Gathers technical context another role needs.",
            ExecutionClass::Advisory,
            "Purpose: collect and summarize repository or dependency context.\n\
             Responsibilities:\n\
             - inspect the repository and existing documentation;\n\
             - investigate relevant dependency or API behavior;\n\
             - identify constraints, unknowns and prior art;\n\
             - return concise, evidence-backed findings.\n\
             Boundaries:\n\
             - do not modify production code unless the task assigns implementation.\n\
             Expected output: findings with file references, written where the task \
             requires them.",
        ),
        TEST_ENGINEER => (
            "Test Engineer",
            "Designs and runs verification for task requirements.",
            ExecutionClass::Execution,
            "Purpose: make the task's behavior verifiable.\n\
             Responsibilities:\n\
             - add or extend tests covering the acceptance criteria;\n\
             - cover regressions and relevant edge cases;\n\
             - run the affected test suites and report results.\n\
             Boundaries:\n\
             - do not lower coverage bars or mark tests as ignored;\n\
             - do not change production behavior unless the task requires it.\n\
             Expected output: tests, passing runs, and verification evidence.",
        ),
        SECURITY_AUDITOR => (
            "Security Auditor",
            "Reviews security-sensitive boundaries of a change.",
            ExecutionClass::Review,
            "Purpose: find security issues the change could introduce.\n\
             Responsibilities:\n\
             - review authentication and authorization boundaries;\n\
             - check validation of untrusted input;\n\
             - look for secret exposure and unsafe process, file or network use;\n\
             - assess dependency risks directly relevant to the change.\n\
             Boundaries:\n\
             - do not modify unrelated application behavior;\n\
             - report findings with severity and concrete evidence.\n\
             Expected output: findings, or a clear statement that the reviewed \
             boundaries hold.",
        ),
        DOCUMENTATION_WRITER => (
            "Documentation Writer",
            "Produces or updates documentation after implementation.",
            ExecutionClass::PostProcess,
            "Purpose: keep documentation consistent with the change.\n\
             Responsibilities:\n\
             - update README and usage docs affected by the change;\n\
             - refresh architecture or design notes where behavior moved;\n\
             - add migration notes for behavior or interface changes.\n\
             Boundaries:\n\
             - do not document behavior that does not exist;\n\
             - keep instructions runnable and copy-pasteable.\n\
             Expected output: documentation changes with verification evidence.",
        ),
        _ => return None,
    };
    Some(RoleDefinition {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        instructions: instructions.to_string(),
        execution_class,
        kind: RoleKind::Core,
    })
}

pub fn core_roles() -> Vec<RoleDefinition> {
    CORE_ROLE_IDS
        .iter()
        .filter_map(|id| core_role(id))
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct RoleCatalog {
    definitions: BTreeMap<String, RoleDefinition>,
}

impl RoleCatalog {
    pub fn build(custom: &BTreeMap<String, crate::config::RoleDefinitionEntry>) -> RoleCatalog {
        let mut definitions = BTreeMap::new();
        for id in CORE_ROLE_IDS {
            if let Some(role) = core_role(id) {
                definitions.insert(id.to_string(), role);
            }
        }
        for (id, entry) in custom {
            if let Some(definition) = entry.to_definition(id) {
                definitions.insert(id.clone(), definition);
            }
        }
        RoleCatalog { definitions }
    }

    pub fn get(&self, id: &str) -> Option<&RoleDefinition> {
        self.definitions.get(id)
    }

    pub fn list(&self) -> Vec<&RoleDefinition> {
        self.definitions.values().collect()
    }
}

/// The effective operation of a task: its declared operation, or a compatible
/// default derived from the role's execution class. Tasks persisted by older
/// releases carry no operation and fall back here. Unknown roles resolve as
/// Worker (execution).
pub fn resolve_task_operation(task: &factory_types::Task, catalog: &RoleCatalog) -> TaskOperation {
    if let Some(operation) = task.operation {
        return operation;
    }
    let role = task.role.as_deref().unwrap_or(WORKER);
    let class = catalog
        .get(role)
        .map(|definition| definition.execution_class)
        .unwrap_or(ExecutionClass::Execution);
    default_operation_for_role(role, class)
}

/// Whether a task operation produces a persisted role artifact. Implementation
/// produces evidence; the other operations persist their structured output.
pub fn operation_persists_artifact(operation: TaskOperation) -> bool {
    !matches!(
        operation,
        TaskOperation::Planning | TaskOperation::Implement
    )
}

/// Deterministic assignment selection.
///
/// A pool with a preferred member always selects it. Otherwise members are
/// visited round-robin, driven by an externally supplied index so the choice
/// stays stable across restarts.
pub fn select_agent(pool: &[String], index: usize) -> Option<&String> {
    if pool.is_empty() {
        return None;
    }
    Some(&pool[index % pool.len()])
}

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_underscore = false;
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            pending_underscore = false;
        } else if !slug.is_empty() && !pending_underscore {
            slug.push('_');
            pending_underscore = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_roles_cover_the_expected_ids() {
        let roles = core_roles();
        let ids: Vec<&str> = roles.iter().map(|role| role.id.as_str()).collect();
        assert_eq!(ids, CORE_ROLE_IDS);
        for role in roles {
            assert_eq!(role.kind, RoleKind::Core);
            assert!(!role.description.is_empty());
            assert!(role.instructions.contains("Purpose:"));
        }
    }

    #[test]
    fn pipeline_roles_are_core() {
        for id in PIPELINE_ROLE_IDS {
            assert!(is_core_role(id));
        }
        assert!(!is_core_role("database_engineer"));
    }

    #[test]
    fn selection_is_deterministic_round_robin() {
        let pool: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let picks: Vec<&str> = (0..7)
            .map(|index| select_agent(&pool, index).unwrap().as_str())
            .collect();
        assert_eq!(picks, ["a", "b", "c", "a", "b", "c", "a"]);
        let empty: Vec<String> = Vec::new();
        assert!(select_agent(&empty, 0).is_none());
    }

    #[test]
    fn slugify_produces_stable_identifiers() {
        assert_eq!(slugify("Database Engineer"), "database_engineer");
        assert_eq!(slugify("  Security -- Auditor! "), "security_auditor");
        assert_eq!(slugify("Ünicode Team"), "nicode_team");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn execution_class_round_trips() {
        for class in [
            ExecutionClass::Planning,
            ExecutionClass::Execution,
            ExecutionClass::Review,
            ExecutionClass::Advisory,
            ExecutionClass::PostProcess,
        ] {
            assert_eq!(class.as_str().parse::<ExecutionClass>().unwrap(), class);
        }
        assert!("manager".parse::<ExecutionClass>().is_err());
    }
}
