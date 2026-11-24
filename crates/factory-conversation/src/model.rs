use factory_files::FileRef;
use factory_types::{Plan, WorkflowTeam};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextReference {
    pub file: FileRef,
    pub revision: String,
    pub buffer: Option<String>,
    pub selection: Option<[usize; 2]>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub key: String,
    pub workspace_session_id: String,
    pub content: String,
    #[serde(default)]
    pub context: Vec<ContextReference>,
    #[serde(default)]
    pub mission: Option<MissionRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionRequest {
    pub objective: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub constraints: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub team: Option<WorkflowTeam>,
    pub recipe: Option<String>,
    pub plan: Option<Plan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Respond {
        content: String,
    },
    CreateMission {
        objective: String,
        recipe: Option<String>,
    },
    InspectMission {
        run_id: i64,
    },
    ReferenceFiles {
        paths: Vec<String>,
        content: String,
    },
    ReferenceMemories {
        memory_ids: Vec<String>,
        content: String,
    },
    Clarify {
        content: String,
    },
    Unavailable {
        content: String,
    },
}

pub fn parse_action(output: &str) -> crate::Result<Action> {
    let output = output.trim();
    let output = output
        .strip_prefix("```json")
        .and_then(|text| {
            text.strip_suffix("``` ")
                .or_else(|| text.strip_suffix("```"))
        })
        .unwrap_or(output)
        .trim();
    let action: Action=serde_json::from_str(output).map_err(|_|crate::Error::Invalid("Orchestrator returned an invalid action. No action was executed; inspect its session and retry with a new command.".into()))?;
    match &action {
        Action::Respond { content }
        | Action::Clarify { content }
        | Action::Unavailable { content }
        | Action::ReferenceFiles { content, .. }
        | Action::ReferenceMemories { content, .. }
            if content.trim().is_empty() || content.len() > 24000 =>
        {
            return Err(crate::Error::Invalid(
                "Orchestrator response is empty or exceeds the response limit.".into(),
            ))
        }
        Action::CreateMission { objective, .. }
            if objective.trim().is_empty() || objective.len() > 16000 =>
        {
            return Err(crate::Error::Invalid(
                "Mission objective is invalid.".into(),
            ))
        }
        Action::InspectMission { run_id } if *run_id <= 0 => {
            return Err(crate::Error::Invalid("Mission identity is invalid.".into()))
        }
        Action::ReferenceFiles { paths, .. } if paths.len() > 8 => return Err(crate::Error::Limit),
        Action::ReferenceMemories { memory_ids, .. }
            if memory_ids.is_empty()
                || memory_ids.len() > 12
                || memory_ids.iter().any(|id| !crate::valid_id(id, "pm_")) =>
        {
            return Err(crate::Error::Invalid(
                "Invalid project memory reference".into(),
            ))
        }
        _ => {}
    }
    Ok(action)
}
