use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use factory_agent::{
    runtime_path_entries, AgentCapabilities, AgentConfig, AgentError, AgentKind, AgentRequest,
    AgentStatus, CommandAgent, PromptTransport, MISSION_PLACEHOLDER,
};
use factory_types::WorkflowTeam;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::roles::{
    self, is_core_role, is_pipeline_role, RoleCatalog, RoleDefinition, RoleKind,
    MAX_ROLE_INSTRUCTIONS_CHARS, MAX_ROLE_NAME_CHARS,
};

pub const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub agents: BTreeMap<String, AgentEntry>,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleDefinitionEntry>,
    #[serde(default)]
    pub role_assignments: Vec<RoleAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AgentKind>,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_transport: Option<PromptTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive_args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// A custom role definition stored under `[roles.<slug>]`.
///
/// The `agent` field only captures the legacy single-agent form
/// (`[roles.planner] agent = "codex"`); `Config::normalize` converts it into a
/// `[[role_assignments]]` entry and never writes it back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleDefinitionEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_class: Option<roles::ExecutionClass>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

impl RoleDefinitionEntry {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.execution_class.is_none()
            && self.instructions.trim().is_empty()
            && self.agent.is_none()
    }

    pub fn is_definition(&self) -> bool {
        self.name.is_some()
            || self.description.is_some()
            || self.execution_class.is_some()
            || !self.instructions.trim().is_empty()
    }

    pub fn to_definition(&self, id: &str) -> Option<RoleDefinition> {
        if !self.is_definition() {
            return None;
        }
        Some(RoleDefinition {
            id: id.to_string(),
            name: self.name.clone().unwrap_or_else(|| id.to_string()),
            description: self.description.clone().unwrap_or_default(),
            instructions: self.instructions.trim().to_string(),
            execution_class: self
                .execution_class
                .unwrap_or(roles::ExecutionClass::Execution),
            kind: RoleKind::Custom,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub role: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preferred: bool,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("no factory configuration found at {0}; run `factory init`")]
    Missing(PathBuf),
    #[error("failed to read {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("failed to parse {0}: {1}")]
    Parse(PathBuf, Box<toml::de::Error>),
    #[error("failed to write {0}: {1}")]
    Write(PathBuf, std::io::Error),
    #[error("failed to serialize {0}: {1}")]
    Serialize(PathBuf, Box<toml::ser::Error>),
}

#[derive(Debug, Error)]
pub enum AgentResolutionError {
    #[error("No agent is assigned to the {0} role. Configure one from the dashboard.")]
    NoRole(String),
    #[error("role `{0}` refers to unknown agent `{1}`; add an [agents.{1}] section")]
    UnknownAgent(String, String),
    #[error("Agent `{1}` is not assigned to the {0} role.")]
    NotAssigned(String, String),
    #[error("{0} agent `{1}` is not available. Check the agent configuration.")]
    NotAvailable(String, String),
    #[error("{0} agent `{1}` cannot start because its executable installation is broken: {2}")]
    Broken(String, String, String),
    #[error("Agent `{1}` cannot be used as {0} because it has no non-interactive invocation configured.")]
    AutomatedUnavailable(String, String),
}

impl Config {
    pub fn load(root: &Path) -> Result<Config, ConfigError> {
        let path = root.join(".factory").join(CONFIG_FILE);
        let text =
            std::fs::read_to_string(&path).map_err(|e| ConfigError::Read(path.clone(), e))?;
        let mut config: Config =
            toml::from_str(&text).map_err(|e| ConfigError::Parse(path, Box::new(e)))?;
        config.normalize();
        Ok(config)
    }

    /// Loads the configuration and, when the file still uses the legacy
    /// single-agent role form, rewrites it in place. The original file is
    /// preserved next to it as `config.toml.bak` before the atomic rewrite.
    /// When the migrated configuration fails validation, the normalized form is
    /// returned without writing so the fault surfaces at role resolution
    /// instead of blocking startup.
    pub fn load_and_migrate(root: &Path) -> Result<Config, ConfigError> {
        let path = Self::path(root);
        let original =
            std::fs::read_to_string(&path).map_err(|e| ConfigError::Read(path.clone(), e))?;
        let mut config: Config =
            toml::from_str(&original).map_err(|e| ConfigError::Parse(path.clone(), Box::new(e)))?;
        if !config.normalize() {
            return Ok(config);
        }
        if config.validate().is_err() {
            return Ok(config);
        }
        let backup = path.with_extension("toml.bak");
        std::fs::write(&backup, &original).map_err(|e| ConfigError::Write(backup.clone(), e))?;
        config.write_atomic(root)?;
        Ok(config)
    }

    pub fn path(root: &Path) -> PathBuf {
        root.join(".factory").join(CONFIG_FILE)
    }

    pub fn ensure_default(root: &Path) -> Result<PathBuf, ConfigError> {
        let path = Self::path(root);
        if !path.exists() {
            std::fs::write(&path, default_config_text())
                .map_err(|e| ConfigError::Read(path.clone(), e))?;
        }
        let _ = Config::load(root)?;
        Ok(path)
    }

    /// Converts legacy `[roles.<role>] agent = "..."` entries into
    /// `[[role_assignments]]` entries and drops empty role tables. Returns
    /// whether anything changed.
    pub fn normalize(&mut self) -> bool {
        let mut changed = false;
        let legacy: Vec<(String, String)> = self
            .roles
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .agent
                    .as_ref()
                    .map(|agent| (id.clone(), agent.clone()))
            })
            .collect();
        for (role, agent) in legacy {
            changed = true;
            if agent.is_empty() {
                continue;
            }
            if self
                .role_assignments
                .iter()
                .any(|assignment| assignment.role == role && assignment.agent == agent)
            {
                continue;
            }
            let preferred = !self
                .role_assignments
                .iter()
                .any(|assignment| assignment.role == role && assignment.preferred);
            self.role_assignments.push(RoleAssignment {
                role,
                agent,
                preferred,
            });
        }
        for entry in self.roles.values_mut() {
            if entry.agent.take().is_some() {
                changed = true;
            }
        }
        let empty_roles: Vec<String> = self
            .roles
            .iter()
            .filter(|(_, entry)| entry.is_empty())
            .map(|(id, _)| id.clone())
            .collect();
        for id in empty_roles {
            changed = true;
            self.roles.remove(&id);
        }
        let before = self.role_assignments.len();
        let mut seen = std::collections::HashSet::new();
        self.role_assignments
            .retain(|assignment| seen.insert((assignment.role.clone(), assignment.agent.clone())));
        if self.role_assignments.len() != before {
            changed = true;
        }
        self.role_assignments
            .sort_by(|a, b| (&a.role, &a.agent).cmp(&(&b.role, &b.agent)));
        changed
    }

    pub fn agent_config(&self, name: &str) -> Option<AgentConfig> {
        let entry = self.agents.get(name)?;
        let kind = entry.effective_kind();
        let args = entry.effective_args(kind);
        Some(AgentConfig {
            name: name.to_string(),
            kind,
            command: entry.command.clone(),
            args,
            env: entry.env.clone(),
            prompt_transport: entry
                .prompt_transport
                .unwrap_or_else(|| kind.prompt_transport()),
            interactive_args: entry
                .interactive_args
                .clone()
                .or_else(|| kind.supports_interactive().then(Vec::new)),
            capabilities: AgentCapabilities {
                roles: entry.capabilities.clone(),
            },
        })
    }

    pub fn catalog(&self) -> RoleCatalog {
        RoleCatalog::build(&self.roles)
    }

    pub fn assignments_for(&self, role: &str) -> Vec<&RoleAssignment> {
        self.role_assignments
            .iter()
            .filter(|assignment| assignment.role == role)
            .collect()
    }

    /// The preferred assignment for a role, or the first declared one.
    pub fn preferred_assignment(&self, role: &str) -> Option<&RoleAssignment> {
        let mut assignments = self.assignments_for(role);
        assignments.sort_by_key(|assignment| !assignment.preferred);
        assignments.into_iter().next()
    }

    pub fn agent_for_role(&self, role: &str) -> Option<String> {
        self.preferred_assignment(role)
            .map(|assignment| assignment.agent.clone())
    }

    pub fn role_infos(&self) -> Vec<RoleInfo> {
        let catalog = self.catalog();
        let mut infos = Vec::new();
        for definition in catalog.list() {
            let assignments = self.assignments_for(&definition.id);
            let available = !assignments.is_empty();
            infos.push(RoleInfo {
                id: definition.id.clone(),
                name: definition.name.clone(),
                kind: definition.kind.as_str().to_string(),
                description: definition.description.clone(),
                instructions: definition.instructions.clone(),
                execution_class: definition.execution_class.as_str().to_string(),
                assignments: assignments
                    .into_iter()
                    .map(|assignment| RoleAssignmentInfo {
                        agent: assignment.agent.clone(),
                        preferred: assignment.preferred,
                    })
                    .collect(),
                available,
            });
        }
        infos
    }

    fn known_role(&self, role: &str) -> bool {
        is_core_role(role) || self.roles.contains_key(role)
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        for (id, entry) in &self.roles {
            if !valid_name(id) {
                return Err(format!("invalid role id '{id}'"));
            }
            if is_core_role(id) {
                return Err(format!(
                    "role '{id}' is a built-in core role and cannot be redefined"
                ));
            }
            if entry.agent.is_some() {
                return Err(format!(
                    "role '{id}' uses the legacy single-agent form; assignments belong in [[role_assignments]]"
                ));
            }
            if entry.is_definition() {
                let name = entry.name.as_deref().unwrap_or("").trim();
                if name.is_empty() {
                    return Err(format!("role '{id}' has no name"));
                }
                if name.chars().count() > MAX_ROLE_NAME_CHARS {
                    return Err(format!(
                        "role '{id}' name exceeds {MAX_ROLE_NAME_CHARS} characters"
                    ));
                }
                let description = entry.description.as_deref().unwrap_or("").trim();
                if description.is_empty() {
                    return Err(format!("role '{id}' has no description"));
                }
                if entry.execution_class.is_none() {
                    return Err(format!("role '{id}' has no execution class"));
                }
                if entry.instructions.chars().count() > MAX_ROLE_INSTRUCTIONS_CHARS {
                    return Err(format!(
                        "role '{id}' instructions exceed {MAX_ROLE_INSTRUCTIONS_CHARS} characters"
                    ));
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut preferred_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for assignment in &self.role_assignments {
            if !self.known_role(&assignment.role) {
                return Err(format!(
                    "assignment refers to unknown role '{}'",
                    assignment.role
                ));
            }
            if !self.agents.contains_key(&assignment.agent) {
                return Err(format!(
                    "role '{}' refers to unknown agent '{}'",
                    assignment.role, assignment.agent
                ));
            }
            if !seen.insert((assignment.role.clone(), assignment.agent.clone())) {
                return Err(format!(
                    "role '{}' assigns agent '{}' more than once",
                    assignment.role, assignment.agent
                ));
            }
            if assignment.preferred {
                *preferred_counts
                    .entry(assignment.role.as_str())
                    .or_default() += 1;
            }
        }
        for (role, count) in preferred_counts {
            if count > 1 {
                return Err(format!(
                    "role '{role}' has {count} preferred assignments; at most one is allowed"
                ));
            }
        }
        for (name, entry) in &self.agents {
            if !valid_name(name) {
                return Err(format!("invalid agent name '{name}'"));
            }
            let command = entry.command.trim();
            if command.is_empty() {
                return Err(format!("agent '{name}' has an empty command"));
            }
            if contains_control(&entry.command) {
                return Err(format!(
                    "agent '{name}' command contains control characters"
                ));
            }
            for arg in &entry.args {
                if contains_control(arg) {
                    return Err(format!(
                        "agent '{name}' has an argument with control characters"
                    ));
                }
            }
            if let Some(interactive_args) = &entry.interactive_args {
                for arg in interactive_args {
                    if contains_control(arg) {
                        return Err(format!(
                            "agent '{name}' has an interactive argument with control characters"
                        ));
                    }
                }
            }
            let placeholder_count = entry
                .args
                .iter()
                .filter(|argument| argument.as_str() == MISSION_PLACEHOLDER)
                .count();
            if entry.args.iter().any(|argument| {
                argument.contains(MISSION_PLACEHOLDER) && argument != MISSION_PLACEHOLDER
            }) {
                return Err(format!(
                    "agent '{name}' must use {MISSION_PLACEHOLDER} as a complete argument"
                ));
            }
            if placeholder_count > 1 {
                return Err(format!(
                    "agent '{name}' has more than one {MISSION_PLACEHOLDER} argument"
                ));
            }
            let kind = entry.effective_kind();
            let transport = entry
                .prompt_transport
                .unwrap_or_else(|| kind.prompt_transport());
            if transport != PromptTransport::Argument && placeholder_count > 0 {
                return Err(format!(
                    "agent '{name}' can use {MISSION_PLACEHOLDER} only with argument prompt transport"
                ));
            }
            for (key, value) in &entry.env {
                if key.is_empty() || contains_control(key) || key.contains('=') {
                    return Err(format!(
                        "agent '{name}' has an invalid environment key '{key}'"
                    ));
                }
                if contains_control(value) {
                    return Err(format!("agent '{name}' has an invalid environment value"));
                }
            }
        }
        Ok(())
    }

    /// Validates a workflow team against this configuration. Every selected
    /// agent must be assigned to the role it is selected for.
    pub fn validate_team(&self, team: &WorkflowTeam) -> std::result::Result<(), String> {
        self.validate_team_inner(team, true)
    }

    /// Same as `validate_team`, but an empty worker or reviewer selection is
    /// allowed; used for teams captured before every role is configured.
    pub fn validate_partial_team(&self, team: &WorkflowTeam) -> std::result::Result<(), String> {
        self.validate_team_inner(team, false)
    }

    fn validate_team_inner(
        &self,
        team: &WorkflowTeam,
        require_complete: bool,
    ) -> std::result::Result<(), String> {
        let member_of = |role: &str, agent: &str| -> std::result::Result<(), String> {
            if self
                .role_assignments
                .iter()
                .any(|assignment| assignment.role == role && assignment.agent == agent)
            {
                Ok(())
            } else {
                Err(format!(
                    "agent '{agent}' is not assigned to the '{role}' role"
                ))
            }
        };
        let unique = |agents: &[String], role: &str| -> std::result::Result<(), String> {
            let mut seen = std::collections::HashSet::new();
            for agent in agents {
                if !seen.insert(agent.as_str()) {
                    return Err(format!(
                        "agent '{agent}' appears twice in the {role} selection"
                    ));
                }
            }
            Ok(())
        };
        if team.planner.trim().is_empty() {
            return Err("the team has no planner".into());
        }
        member_of("planner", &team.planner)?;
        if team.workers.is_empty() {
            if require_complete {
                return Err("the team has no workers".into());
            }
        } else {
            unique(&team.workers, "worker selection")?;
            for worker in &team.workers {
                member_of("worker", worker)?;
            }
        }
        if team.reviewers.is_empty() {
            if require_complete {
                return Err("the team has no reviewers".into());
            }
        } else {
            unique(&team.reviewers, "reviewer selection")?;
            for reviewer in &team.reviewers {
                member_of("reviewer", reviewer)?;
            }
        }
        for (role, agents) in &team.additional {
            if is_pipeline_role(role) {
                return Err(format!(
                    "role '{role}' is composed directly on the team and cannot appear as an additional role"
                ));
            }
            if !self.known_role(role) {
                return Err(format!("the team uses unknown role '{role}'"));
            }
            if agents.is_empty() {
                return Err(format!("role '{role}' has no selected agents"));
            }
            unique(agents, &format!("{role} selection"))?;
            for agent in agents {
                member_of(role, agent)?;
            }
        }
        Ok(())
    }

    /// The team captured when a workflow is created: the planner must be
    /// assigned; workers and reviewers are captured when they are configured
    /// and are required before the workflow starts.
    pub fn initial_team(&self) -> std::result::Result<WorkflowTeam, String> {
        let planner = self
            .preferred_assignment("planner")
            .map(|assignment| assignment.agent.clone())
            .ok_or_else(|| {
                "No agent is assigned to the planner role. Configure one from the dashboard."
                    .to_string()
            })?;
        Ok(WorkflowTeam {
            planner,
            workers: self
                .preferred_assignment("worker")
                .map(|assignment| vec![assignment.agent.clone()])
                .unwrap_or_default(),
            reviewers: self
                .preferred_assignment("reviewer")
                .map(|assignment| vec![assignment.agent.clone()])
                .unwrap_or_default(),
            additional: BTreeMap::new(),
        })
    }

    /// The default team for a new workflow: the preferred assignment of each
    /// pipeline role. Optional roles are never added automatically.
    pub fn default_team(&self) -> std::result::Result<WorkflowTeam, String> {
        let pick = |role: &str| -> std::result::Result<String, String> {
            self.preferred_assignment(role)
                .map(|assignment| assignment.agent.clone())
                .ok_or_else(|| {
                    format!("No agent is assigned to the {role} role. Configure one from the dashboard.")
                })
        };
        Ok(WorkflowTeam {
            planner: pick("planner")?,
            workers: vec![pick("worker")?],
            reviewers: vec![pick("reviewer")?],
            additional: BTreeMap::new(),
        })
    }

    pub fn write_atomic(&self, root: &Path) -> Result<PathBuf, ConfigError> {
        self.validate().map_err(|reason| {
            ConfigError::Write(Self::path(root), std::io::Error::other(reason))
        })?;
        let path = Self::path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write(path.clone(), e))?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| ConfigError::Serialize(path.clone(), Box::new(e)))?;
        let tmp = path.with_extension(format!("toml.tmp{}", std::process::id()));
        std::fs::write(&tmp, text).map_err(|e| ConfigError::Write(tmp.clone(), e))?;
        std::fs::rename(&tmp, &path).map_err(|e| ConfigError::Write(path.clone(), e))?;
        Ok(path)
    }
}

pub fn default_config_text() -> String {
    "\
# Agentic Software Factory
#
# The Factory does not talk to model providers. Agents are external coding
# CLIs that you install and authenticate yourself (Codex, Claude Code,
# OpenCode, Gemini CLI, ...). The Factory only orchestrates them.
#
# Roles describe responsibilities; assignments connect a role to one or
# more agents. The same agent may fill several roles. Custom roles are
# defined under [roles.<slug>] with name, description, execution_class
# and instructions.

[agents.codex]
kind = \"codex\"
command = \"codex\"
args = [\"exec\"]

[agents.opencode]
kind = \"open_code\"
command = \"opencode\"
args = [\"run\"]

[agents.claude]
kind = \"claude_code\"
command = \"claude\"
args = [\"-p\"]

[[role_assignments]]
role = \"planner\"
agent = \"codex\"
preferred = true

[[role_assignments]]
role = \"worker\"
agent = \"opencode\"
preferred = true

[[role_assignments]]
role = \"reviewer\"
agent = \"claude\"
preferred = true
"
    .to_string()
}

pub struct Agents {
    config: Config,
}

impl Agents {
    pub fn load(root: &Path) -> Result<Agents, ConfigError> {
        Ok(Agents {
            config: Config::load_and_migrate(root)?,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn list(&self) -> Vec<AgentInfo> {
        let mut infos = Vec::new();
        for (name, entry) in &self.config.agents {
            let kind = entry.effective_kind();
            let args = entry.effective_args(kind);
            let agent = CommandAgent::new(AgentConfig {
                name: name.clone(),
                kind,
                command: entry.command.clone(),
                args: args.clone(),
                env: entry.env.clone(),
                prompt_transport: entry
                    .prompt_transport
                    .unwrap_or_else(|| kind.prompt_transport()),
                interactive_args: entry
                    .interactive_args
                    .clone()
                    .or_else(|| kind.supports_interactive().then(Vec::new)),
                capabilities: AgentCapabilities {
                    roles: entry.capabilities.clone(),
                },
            });
            let resolution = agent.resolve_executable();
            let resolution_error = resolution.as_ref().err().map(|error| error.to_string());
            let mut info = AgentInfo {
                name: name.clone(),
                command: format_command(&entry.command, &args),
                args,
                available: resolution.is_ok(),
                status: AgentStatus::Missing,
                kind,
                workflow_available: agent.workflow_available(),
                interactive_available: agent.interactive_available(),
                resolved_executable: None,
                resolution_error,
                resolution_shim: None,
                resolution_target: None,
                resolution_kind: None,
                path_entries_checked: runtime_path_entries(),
            };
            match resolution {
                Ok(resolved) => {
                    info.status = AgentStatus::Available;
                    info.resolved_executable = Some(resolved.path().to_string_lossy().into_owned());
                    if resolved.kind() == factory_agent::ResolvedExecutableKind::NpmShim {
                        info.resolution_shim = info.resolved_executable.clone();
                    }
                    info.resolution_target =
                        Some(resolved.launch_program().to_string_lossy().into_owned());
                    info.resolution_kind = Some(resolved.kind().as_str().to_string());
                    info.path_entries_checked = resolved.path_entries_checked();
                }
                Err(AgentError::InvalidExecutable { path, shim, .. }) => {
                    info.status = AgentStatus::Broken;
                    info.resolution_shim = shim
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned());
                    info.resolution_target = Some(path.to_string_lossy().into_owned());
                }
                Err(_) => {}
            }
            infos.push(info);
        }
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    pub fn command_agent(&self, role: &str) -> Result<CommandAgent, AgentResolutionError> {
        let name = self
            .config
            .agent_for_role(role)
            .ok_or_else(|| AgentResolutionError::NoRole(role.to_string()))?;
        self.command_agent_for(role, &name)
    }

    /// Resolves a specific agent that must be assigned to the given role.
    pub fn command_agent_for(
        &self,
        role: &str,
        agent: &str,
    ) -> Result<CommandAgent, AgentResolutionError> {
        let assigned = self
            .config
            .role_assignments
            .iter()
            .any(|assignment| assignment.role == role && assignment.agent == agent);
        if !assigned {
            return Err(AgentResolutionError::NotAssigned(
                role.to_string(),
                agent.to_string(),
            ));
        }
        let config = self
            .config
            .agent_config(agent)
            .ok_or_else(|| AgentResolutionError::UnknownAgent(role.to_string(), agent.into()))?;
        let command = CommandAgent::new(config);
        match command.resolve_executable() {
            Ok(_) => {}
            Err(error @ AgentError::InvalidExecutable { .. }) => {
                return Err(AgentResolutionError::Broken(
                    capitalized(role),
                    agent.to_string(),
                    error.to_string(),
                ));
            }
            Err(_) => {
                return Err(AgentResolutionError::NotAvailable(
                    capitalized(role),
                    agent.to_string(),
                ));
            }
        }
        if command
            .automated_invocation(&AgentRequest::new("validation", "."))
            .is_err()
        {
            return Err(AgentResolutionError::AutomatedUnavailable(
                capitalized(role),
                agent.to_string(),
            ));
        }
        Ok(command)
    }

    pub fn named_agent(&self, name: &str) -> Result<CommandAgent, AgentResolutionError> {
        let agent = self
            .config
            .agent_config(name)
            .ok_or_else(|| AgentResolutionError::UnknownAgent("console".into(), name.into()))?;
        let command = CommandAgent::new(agent);
        match command.resolve_executable() {
            Ok(_) => {}
            Err(error @ AgentError::InvalidExecutable { .. }) => {
                return Err(AgentResolutionError::Broken(
                    "Console".into(),
                    name.into(),
                    error.to_string(),
                ));
            }
            Err(_) => {
                return Err(AgentResolutionError::NotAvailable(
                    "Console".into(),
                    name.into(),
                ));
            }
        }
        Ok(command)
    }
}

impl AgentEntry {
    pub fn effective_kind(&self) -> AgentKind {
        self.kind
            .unwrap_or_else(|| infer_kind(&self.command, &self.args))
    }

    fn effective_args(&self, kind: AgentKind) -> Vec<String> {
        if self.args.is_empty() && kind != AgentKind::Custom {
            kind.workflow_args()
                .iter()
                .map(|arg| (*arg).into())
                .collect()
        } else {
            self.args.clone()
        }
    }
}

fn infer_kind(command: &str, args: &[String]) -> AgentKind {
    let executable = Path::new(command)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .to_ascii_lowercase();
    match executable.as_str() {
        "codex" if args.first().is_some_and(|arg| arg == "exec") => AgentKind::Codex,
        "claude" if args.iter().any(|arg| arg == "-p" || arg == "--print") => AgentKind::ClaudeCode,
        "opencode" if args.first().is_some_and(|arg| arg == "run") => AgentKind::OpenCode,
        "gemini" if args.iter().any(|arg| arg == "-p" || arg == "--prompt") => AgentKind::GeminiCli,
        "qwen" if args.iter().any(|arg| arg == "-p" || arg == "--prompt") => AgentKind::QwenCode,
        _ => AgentKind::Custom,
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn contains_control(value: &str) -> bool {
    value
        .chars()
        .any(|c| c.is_control() || matches!(c, '\n' | '\r' | '\0'))
}

fn capitalized(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub available: bool,
    pub status: AgentStatus,
    pub kind: AgentKind,
    pub workflow_available: bool,
    pub interactive_available: bool,
    pub resolved_executable: Option<String>,
    pub resolution_error: Option<String>,
    pub resolution_shim: Option<String>,
    pub resolution_target: Option<String>,
    pub resolution_kind: Option<String>,
    pub path_entries_checked: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleAssignmentInfo {
    pub agent: String,
    pub preferred: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub instructions: String,
    pub execution_class: String,
    pub assignments: Vec<RoleAssignmentInfo>,
    pub available: bool,
}

fn format_command(command: &str, args: &[String]) -> String {
    let mut parts = vec![command.to_string()];
    parts.extend(args.iter().cloned());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_entry() -> AgentEntry {
        AgentEntry {
            kind: None,
            command: "codex".into(),
            args: vec!["exec".into()],
            env: BTreeMap::new(),
            prompt_transport: None,
            interactive_args: None,
            capabilities: Vec::new(),
        }
    }

    #[test]
    fn legacy_single_agent_roles_migrate_to_assignments() {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry());
        config.agents.insert("claude".into(), agent_entry());
        config.roles.insert(
            "planner".into(),
            RoleDefinitionEntry {
                agent: Some("codex".into()),
                ..RoleDefinitionEntry::default()
            },
        );
        config.roles.insert(
            "reviewer".into(),
            RoleDefinitionEntry {
                agent: Some("claude".into()),
                ..RoleDefinitionEntry::default()
            },
        );
        assert!(config.normalize());
        assert!(config.roles.is_empty());
        assert_eq!(config.role_assignments.len(), 2);
        let planner = config
            .role_assignments
            .iter()
            .find(|assignment| assignment.role == "planner")
            .unwrap();
        assert_eq!(planner.agent, "codex");
        assert!(planner.preferred);
        assert!(config.validate().is_ok());
        assert!(!config.normalize(), "second normalize is a no-op");
    }

    #[test]
    fn empty_role_tables_are_dropped() {
        let mut config = Config::default();
        config
            .roles
            .insert("architect".into(), RoleDefinitionEntry::default());
        assert!(config.normalize());
        assert!(config.roles.is_empty());
    }

    #[test]
    fn duplicate_assignments_are_rejected() {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry());
        config.role_assignments = vec![
            RoleAssignment {
                role: "worker".into(),
                agent: "codex".into(),
                preferred: false,
            },
            RoleAssignment {
                role: "worker".into(),
                agent: "codex".into(),
                preferred: false,
            },
        ];
        assert!(config.validate().is_err());
    }

    #[test]
    fn multiple_preferred_assignments_are_rejected() {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry());
        config.agents.insert("claude".into(), agent_entry());
        config.role_assignments = vec![
            RoleAssignment {
                role: "worker".into(),
                agent: "codex".into(),
                preferred: true,
            },
            RoleAssignment {
                role: "worker".into(),
                agent: "claude".into(),
                preferred: true,
            },
        ];
        let error = config.validate().unwrap_err();
        assert!(error.contains("preferred"), "unexpected: {error}");
    }

    #[test]
    fn assignment_to_unknown_agent_or_role_is_rejected() {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry());
        config.role_assignments = vec![RoleAssignment {
            role: "worker".into(),
            agent: "ghost".into(),
            preferred: false,
        }];
        assert!(config.validate().is_err());
        config.role_assignments = vec![RoleAssignment {
            role: "ghost_role".into(),
            agent: "codex".into(),
            preferred: false,
        }];
        assert!(config.validate().is_err());
    }

    #[test]
    fn core_roles_cannot_be_redefined() {
        let mut config = Config::default();
        config.roles.insert(
            "worker".into(),
            RoleDefinitionEntry {
                name: Some("Worker".into()),
                description: Some("Override".into()),
                execution_class: Some(roles::ExecutionClass::Execution),
                instructions: String::new(),
                agent: None,
            },
        );
        let error = config.validate().unwrap_err();
        assert!(error.contains("built-in core role"), "unexpected: {error}");
    }

    #[test]
    fn custom_roles_validate_and_join_the_catalog() {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry());
        config.roles.insert(
            "database_engineer".into(),
            RoleDefinitionEntry {
                name: Some("Database Engineer".into()),
                description: Some("Designs and modifies relational database schemas.".into()),
                execution_class: Some(roles::ExecutionClass::Execution),
                instructions: "Focus on schema design and migrations.".into(),
                agent: None,
            },
        );
        config.role_assignments = vec![RoleAssignment {
            role: "database_engineer".into(),
            agent: "codex".into(),
            preferred: true,
        }];
        assert!(config.validate().is_ok());
        let catalog = config.catalog();
        let role = catalog.get("database_engineer").unwrap();
        assert_eq!(role.name, "Database Engineer");
        assert_eq!(role.kind, RoleKind::Custom);
        assert!(catalog.get("planner").is_some());
        let infos = config.role_infos();
        assert!(infos.len() >= 9);
        let info = infos
            .iter()
            .find(|info| info.id == "database_engineer")
            .unwrap();
        assert!(info.available);
        assert_eq!(info.assignments.len(), 1);
        assert!(info.assignments[0].preferred);
        let architect = infos.iter().find(|info| info.id == "architect").unwrap();
        assert!(!architect.available);
    }

    #[test]
    fn incomplete_custom_role_definitions_are_rejected() {
        let mut config = Config::default();
        config.roles.insert(
            "half_role".into(),
            RoleDefinitionEntry {
                name: Some("Half".into()),
                description: None,
                execution_class: None,
                instructions: String::new(),
                agent: None,
            },
        );
        let error = config.validate().unwrap_err();
        assert!(error.contains("no description"), "unexpected: {error}");
    }

    #[test]
    fn default_team_picks_the_preferred_assignment() {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry());
        config.agents.insert("claude".into(), agent_entry());
        config.role_assignments = vec![
            RoleAssignment {
                role: "planner".into(),
                agent: "codex".into(),
                preferred: true,
            },
            RoleAssignment {
                role: "worker".into(),
                agent: "codex".into(),
                preferred: false,
            },
            RoleAssignment {
                role: "worker".into(),
                agent: "claude".into(),
                preferred: true,
            },
            RoleAssignment {
                role: "reviewer".into(),
                agent: "claude".into(),
                preferred: true,
            },
        ];
        let team = config.default_team().unwrap();
        assert_eq!(team.planner, "codex");
        assert_eq!(team.workers, ["claude"]);
        assert_eq!(team.reviewers, ["claude"]);
        assert!(config.validate_team(&team).is_ok());
    }

    #[test]
    fn team_validation_rejects_unassigned_agents() {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry());
        config.agents.insert("claude".into(), agent_entry());
        config.role_assignments = vec![
            RoleAssignment {
                role: "planner".into(),
                agent: "codex".into(),
                preferred: true,
            },
            RoleAssignment {
                role: "worker".into(),
                agent: "codex".into(),
                preferred: true,
            },
            RoleAssignment {
                role: "reviewer".into(),
                agent: "claude".into(),
                preferred: true,
            },
        ];
        let team = WorkflowTeam {
            planner: "codex".into(),
            workers: vec!["claude".into()],
            reviewers: vec!["claude".into()],
            additional: BTreeMap::new(),
        };
        let error = config.validate_team(&team).unwrap_err();
        assert!(error.contains("not assigned"), "unexpected: {error}");
    }

    #[test]
    fn default_config_text_parses_and_validates() {
        let mut config: Config = toml::from_str(&default_config_text()).unwrap();
        assert!(config.validate().is_ok());
        assert!(!config.normalize());
        let team = config.default_team().unwrap();
        assert_eq!(team.planner, "codex");
        assert_eq!(team.workers, ["opencode"]);
        assert_eq!(team.reviewers, ["claude"]);
    }
}
