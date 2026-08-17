use std::path::Path;

use factory_agent::{AgentError, AgentRequest, AgentResult, CommandAgent};
use factory_types::{Plan, TaskOperation};
use thiserror::Error;

use crate::roles::{self, ExecutionClass, RoleCatalog};

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("agent error: {0}")]
    Agent(#[from] AgentError),
    #[error("planner agent produced no output")]
    NoOutput,
    #[error("rejected invalid plan output: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct PlanOutcome {
    pub plan: Plan,
    pub agent: String,
    pub command: String,
    pub result: AgentResult,
}

pub(crate) const MAX_ATTEMPTS: u32 = 3;

/// Concise catalog entry shown to the Planner for one role available to the
/// workflow. Only id, display name, execution class, and description are sent;
/// full role instruction bodies are not injected into planning prompts.
#[derive(Debug, Clone)]
pub struct PlannerRoleInfo {
    pub id: String,
    pub name: String,
    pub execution_class: ExecutionClass,
    pub description: String,
}

pub struct Planner {
    agent: CommandAgent,
}

impl Planner {
    pub fn new(agent: CommandAgent) -> Self {
        Planner { agent }
    }

    pub fn agent_name(&self) -> &str {
        self.agent.name()
    }

    pub fn plan(&self, objective: &str, working_dir: &Path) -> Result<PlanOutcome, PlanError> {
        let mut instruction = mission(objective, &[], None);
        for attempt in 0..MAX_ATTEMPTS {
            let request = AgentRequest::new(&instruction, working_dir);
            let result = self.agent.run(&request)?;
            if result.stdout.trim().is_empty() {
                return Err(PlanError::NoOutput);
            }
            match parse_plan(&result.stdout) {
                Ok(mut plan) => {
                    if plan.objective.trim().is_empty() {
                        plan.objective = objective.to_string();
                    }
                    return Ok(PlanOutcome {
                        plan,
                        agent: self.agent.name().to_string(),
                        command: self.agent.command_line(),
                        result,
                    });
                }
                Err(reason) => {
                    if attempt + 1 >= MAX_ATTEMPTS {
                        return Err(PlanError::Invalid(reason));
                    }
                    instruction = mission(objective, &[], Some(&reason));
                }
            }
        }
        unreachable!("attempt loop always returns")
    }
}

const SYSTEM_PROMPT: &str = "You are the planner of a software factory. Convert the user's objective into a structured implementation plan. Respond with a single JSON object and nothing else. The JSON must match this schema exactly:

{
  \"objective\": string,
  \"tasks\": [
    {
      \"id\": string,
      \"title\": string,
      \"objective\": string,
      \"dependencies\": [string],
      \"acceptanceCriteria\": [string],
      \"role\": string,
      \"operation\": string
    }
  ]
}

Rules:
- task ids are short unique labels such as \"T1\", \"T2\".
- \"dependencies\" lists ids of other tasks in the same plan that must finish first.
- the first tasks have empty dependency lists.
- every task has a distinct responsibility and at least one acceptance criterion.
- \"role\" is optional; it must be the id of one of the available roles listed below. Omit it when the task needs general implementation (the Worker role).
- \"operation\" is optional; it must be compatible with the selected role's execution class:
    planning   -> planning
    advisory   -> advisory
    execution  -> implement, verify
    review     -> review
    post_process -> post_process, implement
  Omit \"operation\" to let the runtime derive it from the role.
- plan proportionally: use specialized roles only when they materially improve the workflow. Do not create tasks for Researcher, Architect, Test Engineer, Security Auditor, or Documentation Writer just because those roles exist. Examples:
  - a small CSS fix needs only a general implementation task (Worker) and its review;
  - an authentication subsystem may need an Architect task, implementation tasks, a Test Engineer task, a Security Auditor review, and a Documentation Writer task;
  - an unfamiliar dependency or API may need a Researcher task before implementation;
  - a database migration may need an Architect or Database Engineer task, verification, and a review.
- advisory roles (Researcher, Architect, custom analysts) produce knowledge or design artifacts consumed by later tasks; use them before implementation when the objective is uncertain.
- review roles (Security Auditor, custom reviewers, Reviewer) evaluate the output of earlier tasks; place them after the tasks they must review.
- post_process roles (Documentation Writer) run near workflow completion, after the implementation they document.
- do not include any text outside the JSON object.";

pub(crate) fn mission(
    objective: &str,
    available_roles: &[PlannerRoleInfo],
    rejection: Option<&str>,
) -> String {
    let mut text = format!("{SYSTEM_PROMPT}\n\nObjective: {objective}");
    if available_roles.is_empty() {
        text.push_str(
            "\n\nAvailable roles:\n- worker (Worker) [execution]: General implementation.",
        );
    } else {
        text.push_str("\n\nAvailable roles:");
        for role in available_roles {
            text.push_str(&format!(
                "\n- {} ({}) [{}]: {}",
                role.id,
                role.name,
                role.execution_class.as_str(),
                role.description
            ));
        }
        text.push_str(
            "\n\nOnly the ids listed above may appear as \"role\". Every assigned operation must be \
             compatible with the role's execution class.",
        );
    }
    if let Some(reason) = rejection {
        text.push_str("\n\nYour previous output was rejected because: ");
        text.push_str(reason);
        text.push_str("\nReturn a corrected plan that matches the schema.");
    }
    text
}

/// Builds the concise Planner catalog for a workflow from its selected team.
pub fn planners_catalog(catalog: &RoleCatalog, roles: &[String]) -> Vec<PlannerRoleInfo> {
    roles
        .iter()
        .filter_map(|role| {
            catalog.get(role).map(|definition| PlannerRoleInfo {
                id: definition.id.clone(),
                name: definition.name.clone(),
                execution_class: definition.execution_class,
                description: definition.description.clone(),
            })
        })
        .collect()
}

pub fn validate_plan_roles(
    plan: &Plan,
    allowed: &std::collections::HashSet<String>,
) -> Result<(), String> {
    for task in &plan.tasks {
        if let Some(role) = &task.role {
            if !allowed.contains(role) {
                return Err(format!(
                    "task {} uses role '{}' which is not enabled for this workflow",
                    task.id, role
                ));
            }
        }
    }
    Ok(())
}

/// Validates that every declared task operation is compatible with its role's
/// execution class. A missing operation is allowed; the runtime derives it.
/// A declared but incompatible operation is a hard error: the plan is rejected
/// and the Planner is asked to repair it.
pub fn validate_plan_operations(plan: &Plan, catalog: &RoleCatalog) -> Result<(), String> {
    for task in &plan.tasks {
        let Some(operation) = task.operation else {
            continue;
        };
        let role = task.role.as_deref().unwrap_or(roles::WORKER);
        let class = match catalog.get(role) {
            Some(definition) => definition.execution_class,
            None => ExecutionClass::Execution,
        };
        roles::validate_operation_compatibility(role, class, operation)
            .map_err(|reason| format!("task {} is invalid: {reason}", task.id))?;
    }
    Ok(())
}

pub fn parse_plan(content: &str) -> std::result::Result<Plan, String> {
    let content = strip_code_fence(content);
    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("not valid JSON: {e}"))?;
    let plan: Plan = serde_json::from_value(value).map_err(|e| format!("schema mismatch: {e}"))?;
    validate_plan(&plan)?;
    Ok(plan)
}

fn strip_code_fence(content: &str) -> &str {
    let trimmed = content.trim();
    trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim()
}

pub fn validate_plan(plan: &Plan) -> std::result::Result<(), String> {
    if plan.tasks.is_empty() {
        return Err("plan contains no tasks".into());
    }
    if plan.tasks.len() > MAX_TASKS {
        return Err(format!("plan exceeds the maximum of {MAX_TASKS} tasks"));
    }
    let mut seen = std::collections::HashSet::new();
    for task in &plan.tasks {
        if task.id.trim().is_empty() {
            return Err("a task has an empty id".into());
        }
        if task.title.trim().is_empty() {
            return Err(format!("task {} has an empty title", task.id));
        }
        if task.objective.trim().is_empty() {
            return Err(format!("task {} has an empty objective", task.id));
        }
        if task.acceptance_criteria.is_empty() {
            return Err(format!("task {} has no acceptance criteria", task.id));
        }
        if task.acceptance_criteria.iter().any(|c| c.trim().is_empty()) {
            return Err(format!(
                "task {} has an empty acceptance criterion",
                task.id
            ));
        }
        if !seen.insert(task.id.trim()) {
            return Err(format!("duplicate task id '{}'", task.id));
        }
    }
    for task in &plan.tasks {
        for dep in &task.dependencies {
            if dep.as_str() == task.id.trim() {
                return Err(format!("task {} depends on itself", task.id));
            }
            if !seen.contains(dep.as_str()) {
                return Err(format!(
                    "task {} depends on unknown task '{}'",
                    task.id, dep
                ));
            }
        }
    }
    let mut indegree: std::collections::HashMap<&str, usize> = plan
        .tasks
        .iter()
        .map(|t| (t.id.trim(), t.dependencies.len()))
        .collect();
    let mut queue: Vec<&str> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();
    let mut visited = 0usize;
    while let Some(id) = queue.pop() {
        visited += 1;
        for task in &plan.tasks {
            if task.dependencies.contains(&id.to_string()) && {
                let e = indegree.get_mut(task.id.trim()).expect("known id");
                *e -= 1;
                *e == 0
            } {
                queue.push(task.id.trim());
            }
        }
    }
    if visited != plan.tasks.len() {
        return Err("the task dependency graph contains a cycle".into());
    }
    // A plan must not assign a planning role to a scheduled task: exactly one
    // Planner produces the plan and it never appears in the task DAG.
    for task in &plan.tasks {
        if task.role.as_deref() == Some(roles::PLANNER) {
            return Err(format!(
                "task {} uses the planner role, which cannot appear as a planned task",
                task.id
            ));
        }
        if task.operation == Some(TaskOperation::Planning) {
            return Err(format!(
                "task {} uses operation 'planning'; only the Planner produces plans",
                task.id
            ));
        }
    }
    Ok(())
}

pub fn normalize_plan(mut plan: Plan) -> Plan {
    for task in &mut plan.tasks {
        task.id = task.id.trim().to_string();
        task.title = task.title.trim().to_string();
        task.objective = task.objective.trim().to_string();
        task.dependencies.sort();
        task.dependencies.dedup();
        if task.role.as_deref().is_some_and(str::is_empty) {
            task.role = None;
        }
    }
    plan
}

/// Applies the derived operation semantics to a normalized plan: missing
/// operations get a compatible default from the role's execution class, and
/// declared operations are validated against the class. Returns the plan with
/// every task carrying an explicit operation.
pub fn normalize_plan_with_operations(
    mut plan: Plan,
    catalog: &RoleCatalog,
) -> Result<Plan, String> {
    validate_plan_operations(&plan, catalog)?;
    for task in &mut plan.tasks {
        if task.operation.is_none() {
            let role = task.role.as_deref().unwrap_or(roles::WORKER);
            let class = match catalog.get(role) {
                Some(definition) => definition.execution_class,
                None => ExecutionClass::Execution,
            };
            task.operation = Some(roles::default_operation_for_role(role, class));
        }
    }
    Ok(plan)
}

const MAX_TASKS: usize = 50;
