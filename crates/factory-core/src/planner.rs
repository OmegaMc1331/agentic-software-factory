use std::path::Path;

use factory_agent::{AgentError, AgentRequest, AgentResult, CommandAgent};
use factory_types::Plan;
use thiserror::Error;

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
        let mut instruction = mission(objective, None);
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
                    instruction = mission(objective, Some(&reason));
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
      \"acceptanceCriteria\": [string]
    }
  ]
}

Rules:
- task ids are short unique labels such as \"T1\", \"T2\".
- \"dependencies\" lists ids of other tasks in the same plan that must finish first.
- the first tasks have empty dependency lists.
- every task has a distinct responsibility and at least one acceptance criterion.
- do not include any text outside the JSON object.";

pub(crate) fn mission(objective: &str, rejection: Option<&str>) -> String {
    let mut text = format!("{SYSTEM_PROMPT}\n\nObjective: {objective}");
    if let Some(reason) = rejection {
        text.push_str("\n\nYour previous output was rejected because: ");
        text.push_str(reason);
        text.push_str("\nReturn a corrected plan that matches the schema.");
    }
    text
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
    Ok(())
}

pub fn normalize_plan(mut plan: Plan) -> Plan {
    for task in &mut plan.tasks {
        task.id = task.id.trim().to_string();
        task.title = task.title.trim().to_string();
        task.objective = task.objective.trim().to_string();
        task.dependencies.sort();
        task.dependencies.dedup();
    }
    plan
}

const MAX_TASKS: usize = 50;
