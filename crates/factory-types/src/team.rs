use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTeam {
    pub planner: String,
    pub workers: Vec<String>,
    pub reviewers: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub additional: BTreeMap<String, Vec<String>>,
}

impl WorkflowTeam {
    pub fn agents_for_role(&self, role: &str) -> &[String] {
        match role {
            "planner" => std::slice::from_ref(&self.planner),
            "worker" => &self.workers,
            "reviewer" => &self.reviewers,
            other => self.additional.get(other).map(Vec::as_slice).unwrap_or(&[]),
        }
    }

    pub fn roles(&self) -> Vec<String> {
        let mut roles = vec![
            "planner".to_string(),
            "worker".to_string(),
            "reviewer".to_string(),
        ];
        roles.extend(self.additional.keys().cloned());
        roles
    }

    pub fn task_roles(&self) -> Vec<String> {
        let mut roles = vec!["worker".to_string()];
        roles.extend(self.additional.keys().cloned());
        roles
    }

    pub fn contains_agent(&self, agent: &str) -> bool {
        self.planner == agent
            || self.workers.iter().any(|name| name == agent)
            || self.reviewers.iter().any(|name| name == agent)
            || self
                .additional
                .values()
                .any(|agents| agents.iter().any(|name| name == agent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_resolves_agents_per_role() {
        let team = WorkflowTeam {
            planner: "codex".into(),
            workers: vec!["opencode".into(), "qwen".into()],
            reviewers: vec!["claude".into()],
            additional: BTreeMap::from([(
                "security_auditor".to_string(),
                vec!["claude".to_string()],
            )]),
        };
        assert_eq!(team.agents_for_role("planner"), ["codex"]);
        assert_eq!(team.agents_for_role("worker"), ["opencode", "qwen"]);
        assert_eq!(team.agents_for_role("reviewer"), ["claude"]);
        assert_eq!(team.agents_for_role("security_auditor"), ["claude"]);
        assert!(team.agents_for_role("architect").is_empty());
        assert!(team.contains_agent("qwen"));
        assert!(!team.contains_agent("gemini"));
        assert_eq!(
            team.roles(),
            ["planner", "worker", "reviewer", "security_auditor"]
        );
        assert_eq!(team.task_roles(), ["worker", "security_auditor"]);
    }

    #[test]
    fn team_serializes_without_empty_additional() {
        let team = WorkflowTeam {
            planner: "codex".into(),
            workers: vec!["opencode".into()],
            reviewers: vec!["claude".into()],
            additional: BTreeMap::new(),
        };
        let json = serde_json::to_string(&team).unwrap();
        assert!(!json.contains("additional"));
        let parsed: WorkflowTeam = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, team);
    }
}
