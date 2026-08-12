use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use factory_agent::{AgentCapabilities, AgentConfig, CommandAgent};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub agents: BTreeMap<String, AgentEntry>,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEntry {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleEntry {
    pub agent: String,
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
    #[error("{0} agent `{1}` is not available. Check the agent configuration.")]
    NotAvailable(String, String),
}

impl Config {
    pub fn load(root: &Path) -> Result<Config, ConfigError> {
        let path = root.join(".factory").join(CONFIG_FILE);
        let text =
            std::fs::read_to_string(&path).map_err(|e| ConfigError::Read(path.clone(), e))?;
        let config: Config =
            toml::from_str(&text).map_err(|e| ConfigError::Parse(path, Box::new(e)))?;
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

    pub fn agent_config(&self, name: &str) -> Option<AgentConfig> {
        let entry = self.agents.get(name)?;
        Some(AgentConfig {
            name: name.to_string(),
            command: entry.command.clone(),
            args: entry.args.clone(),
            env: entry.env.clone(),
            capabilities: AgentCapabilities {
                roles: entry.capabilities.clone(),
            },
        })
    }

    pub fn agent_for_role(&self, role: &str) -> Option<String> {
        self.roles.get(role).map(|r| r.agent.clone())
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        for role in self.roles.keys() {
            if !valid_name(role) {
                return Err(format!("invalid role name '{role}'"));
            }
            let agent = self
                .roles
                .get(role)
                .map(|r| r.agent.as_str())
                .unwrap_or_default();
            if agent.is_empty() {
                return Err(format!("role '{role}' has no agent assigned"));
            }
            if !self.agents.contains_key(agent) {
                return Err(format!("role '{role}' refers to unknown agent '{agent}'"));
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
# A role points to an agent by name; the same agent may fill several roles.

[agents.codex]
command = \"codex\"
args = [\"exec\"]

[agents.opencode]
command = \"opencode\"
args = [\"run\"]

[agents.claude]
command = \"claude\"
args = [\"-p\"]

[roles.planner]
agent = \"codex\"

[roles.worker]
agent = \"opencode\"

[roles.reviewer]
agent = \"claude\"
"
    .to_string()
}

pub struct Agents {
    config: Config,
}

impl Agents {
    pub fn load(root: &Path) -> Result<Agents, ConfigError> {
        Ok(Agents {
            config: Config::load(root)?,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn list(&self) -> Vec<AgentInfo> {
        let mut infos = Vec::new();
        for (name, entry) in &self.config.agents {
            let agent = CommandAgent::new(AgentConfig {
                name: name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                env: entry.env.clone(),
                capabilities: AgentCapabilities {
                    roles: entry.capabilities.clone(),
                },
            });
            infos.push(AgentInfo {
                name: name.clone(),
                command: format_command(&entry.command, &entry.args),
                args: entry.args.clone(),
                available: agent.available(),
            });
        }
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    pub fn command_agent(&self, role: &str) -> Result<CommandAgent, AgentResolutionError> {
        let name = self
            .config
            .agent_for_role(role)
            .ok_or_else(|| AgentResolutionError::NoRole(role.to_string()))?;
        let agent = self
            .config
            .agent_config(&name)
            .ok_or_else(|| AgentResolutionError::UnknownAgent(role.to_string(), name.clone()))?;
        let command = CommandAgent::new(agent);
        if !command.available() {
            return Err(AgentResolutionError::NotAvailable(capitalized(role), name));
        }
        Ok(command)
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
pub struct AgentInfo {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub available: bool,
}

fn format_command(command: &str, args: &[String]) -> String {
    let mut parts = vec![command.to_string()];
    parts.extend(args.iter().cloned());
    parts.join(" ")
}
