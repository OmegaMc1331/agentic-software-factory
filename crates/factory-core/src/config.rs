use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use factory_agent::{AgentCapabilities, AgentConfig, CommandAgent};
use serde::Deserialize;
use thiserror::Error;

pub const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub agents: BTreeMap<String, AgentEntry>,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentEntry {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Error)]
pub enum AgentResolutionError {
    #[error("no agent configured for role `{0}`")]
    NoRole(String),
    #[error("role `{0}` refers to unknown agent `{1}`; add an [agents.{1}] section")]
    UnknownAgent(String, String),
    #[error("configured {0} agent `{1}` is not available")]
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
            return Err(AgentResolutionError::NotAvailable(role.to_string(), name));
        }
        Ok(command)
    }
}

pub struct AgentInfo {
    pub name: String,
    pub command: String,
    pub available: bool,
}

fn format_command(command: &str, args: &[String]) -> String {
    let mut parts = vec![command.to_string()];
    parts.extend(args.iter().cloned());
    parts.join(" ")
}
