//! The policy model, precedence rules, and the effective-policy resolver.
//!
//! Policies generalize over **roles** and **agents**. Each `PolicyScope` may
//! carry a compact preset and/or explicit per-dimension rules. The resolver
//! merges them into one `EffectivePolicy` per running (role, agent) pair:
//!
//! ```text
//! Factory safety invariants          (always applied, cannot be removed)
//!         ↓
//! Role policy                       ([policies.roles.<id>])
//!         ↓
//! Agent-specific restrictions       ([policies.agents.<name>])
//!         ↓
//! EffectivePolicy
//! ```
//!
//! An agent scope may only *further restrict* a role scope (allow lists
//! intersect, deny lists union); it can never widen access. Deny rules always
//! win over allow rules. The Factory safety invariants (never modify Factory
//! state, never perform dangerous Git operations, never modify the integration
//! lane) are re-applied on top so no configuration can bypass them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::environment::{filter_configured_env, filter_environment};
use crate::path::{matches_glob, normalize_repo_relative, validate_scope};

/// Complete project-local policy configuration, stored in
/// `.factory/config.toml` under `[policies.roles.<id>]` and
/// `[policies.agents.<name>]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliciesConfig {
    #[serde(default)]
    pub roles: BTreeMap<String, PolicyScope>,
    #[serde(default)]
    pub agents: BTreeMap<String, PolicyScope>,
}

impl PoliciesConfig {
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty() && self.agents.is_empty()
    }

    pub fn role(&self, id: &str) -> Option<&PolicyScope> {
        self.roles.get(id)
    }

    pub fn agent(&self, name: &str) -> Option<&PolicyScope> {
        self.agents.get(name)
    }

    pub fn role_mut(&mut self, id: impl Into<String>) -> &mut PolicyScope {
        self.roles.entry(id.into()).or_default()
    }

    pub fn agent_mut(&mut self, name: impl Into<String>) -> &mut PolicyScope {
        self.agents.entry(name.into()).or_default()
    }

    /// Validates every declared scope, returning the first problem as a
    /// human-readable reason.
    pub fn validate(&self) -> Result<(), String> {
        for (id, scope) in &self.roles {
            scope
                .validate(&format!("policies.roles.{id}"))
                .map_err(|reason| format!("{reason} (policies.roles.{id})"))?;
        }
        for (name, scope) in &self.agents {
            scope
                .validate(&format!("policies.agents.{name}"))
                .map_err(|reason| format!("{reason} (policies.agents.{name})"))?;
        }
        Ok(())
    }

    /// Resolves the role-level effective policy (baseline + role scope),
    /// independent of any specific agent. Used by the Role Inspector to show
    /// what the role itself permits.
    pub fn effective_for_role(&self, role_id: &str) -> EffectivePolicy {
        self.effective(role_id, "")
    }

    /// Resolves the agent-level effective policy (baseline + agent scope),
    /// showing what an agent's own restrictions add on top of Factory's
    /// invariants. Used by the Agent Inspector.
    pub fn effective_for_agent(&self, agent_name: &str) -> EffectivePolicy {
        self.effective("", agent_name)
    }

    /// Resolves the effective policy for a running (role, agent) pair. This is
    /// the single resolution entry point used by execution, validation, audit,
    /// and the dashboard.
    pub fn effective(&self, role_id: &str, agent_name: &str) -> EffectivePolicy {
        let permissive = self.role(role_id).is_none() && self.agent(agent_name).is_none();
        let source = match (self.role(role_id), self.agent(agent_name)) {
            (Some(_), Some(_)) => format!("role:{role_id} + agent:{agent_name}"),
            (Some(_), None) => format!("role:{role_id}"),
            (None, Some(_)) => format!("agent:{agent_name}"),
            (None, None) => "default".to_string(),
        };
        let role = merged_scope(self.role(role_id));
        let agent = merged_scope(self.agent(agent_name));
        EffectivePolicy {
            source,
            permissive,
            filesystem: EffectiveFilesystem::resolve(role.as_ref(), agent.as_ref()),
            commands: EffectiveCommands::resolve(role.as_ref(), agent.as_ref()),
            network: EffectiveNetwork::resolve(role.as_ref(), agent.as_ref()),
            environment: EffectiveEnvironment::resolve(role.as_ref(), agent.as_ref()),
            git: EffectiveGit::resolve(role.as_ref(), agent.as_ref()),
        }
    }
}

/// One role-level or agent-level policy declaration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyScope {
    /// Optional preset that supplies defaults for the dimensions this scope
    /// does not set explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<PolicyPreset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<CommandsPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitPolicy>,
}

impl PolicyScope {
    /// Validates the scope; `context` names the TOML table for error messages.
    pub fn validate(&self, context: &str) -> Result<(), String> {
        if let Some(filesystem) = &self.filesystem {
            for pattern in filesystem.read.iter().chain(filesystem.write.iter()) {
                validate_scope(pattern)
                    .map_err(|reason| format!("invalid filesystem scope in {context}: {reason}"))?;
            }
            for pattern in &filesystem.deny_write {
                validate_scope(pattern)
                    .map_err(|reason| format!("invalid deny_write scope in {context}: {reason}"))?;
            }
        }
        Ok(())
    }
}

/// Compact policy presets used to give a role a sensible default without
/// writing a full policy. An explicit dimension on the same scope overrides
/// the preset's value for that dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyPreset {
    /// Planner / Researcher / Reviewer / Architect style: read-only.
    ReadOnly,
    /// Worker / Test Engineer style: free writes inside the task worktree,
    /// restricted local commands.
    Implementation,
    /// Documentation Writer style: README and docs only.
    Documentation,
    /// Custom review roles: read-only.
    Review,
    /// Bring your own policy: no preset defaults; any explicit dimensions
    /// still apply, otherwise the legacy default applies.
    Custom,
}

impl PolicyPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyPreset::ReadOnly => "read_only",
            PolicyPreset::Implementation => "implementation",
            PolicyPreset::Documentation => "documentation",
            PolicyPreset::Review => "review",
            PolicyPreset::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Option<PolicyPreset> {
        match value.trim() {
            "read_only" => Some(PolicyPreset::ReadOnly),
            "implementation" => Some(PolicyPreset::Implementation),
            "documentation" => Some(PolicyPreset::Documentation),
            "review" => Some(PolicyPreset::Review),
            "custom" => Some(PolicyPreset::Custom),
            _ => None,
        }
    }

    fn defaults(self) -> PolicyScope {
        match self {
            PolicyPreset::ReadOnly => read_only_defaults(self),
            PolicyPreset::Implementation => PolicyScope {
                preset: Some(self),
                filesystem: Some(FilesystemPolicy {
                    read: vec!["**".to_string()],
                    write: vec!["**".to_string()],
                    deny_write: baseline_deny_write(),
                }),
                commands: Some(CommandsPolicy {
                    mode: CommandsMode::Restricted,
                    allow: default_command_allow(),
                    deny: default_denied_shells(),
                }),
                network: Some(NetworkPolicy {
                    mode: NetworkMode::Allow,
                }),
                environment: Some(EnvironmentPolicy {
                    allow: environment_essentials(),
                    deny: Vec::new(),
                }),
                git: None,
            },
            PolicyPreset::Documentation => PolicyScope {
                preset: Some(self),
                filesystem: Some(FilesystemPolicy {
                    read: vec!["**".to_string()],
                    write: vec!["README.md".to_string(), "docs/**".to_string()],
                    deny_write: baseline_deny_write(),
                }),
                commands: Some(CommandsPolicy {
                    mode: CommandsMode::Restricted,
                    allow: vec!["git".to_string()],
                    deny: default_denied_shells(),
                }),
                network: Some(NetworkPolicy {
                    mode: NetworkMode::Allow,
                }),
                environment: Some(EnvironmentPolicy {
                    allow: environment_essentials(),
                    deny: Vec::new(),
                }),
                git: None,
            },
            PolicyPreset::Review => read_only_defaults(self),
            PolicyPreset::Custom => PolicyScope {
                preset: Some(self),
                ..PolicyScope::default()
            },
        }
    }
}

fn read_only_defaults(preset: PolicyPreset) -> PolicyScope {
    PolicyScope {
        preset: Some(preset),
        filesystem: Some(FilesystemPolicy {
            read: vec!["**".to_string()],
            write: Vec::new(),
            deny_write: Vec::new(),
        }),
        commands: Some(CommandsPolicy {
            mode: CommandsMode::Restricted,
            allow: vec!["git".to_string()],
            deny: default_denied_shells(),
        }),
        network: Some(NetworkPolicy {
            mode: NetworkMode::Allow,
        }),
        environment: Some(EnvironmentPolicy {
            allow: environment_essentials(),
            deny: Vec::new(),
        }),
        git: None,
    }
}

fn baseline_deny_write() -> Vec<String> {
    vec![
        ".factory/**".to_string(),
        ".git/**".to_string(),
        ".git".to_string(),
    ]
}

fn default_command_allow() -> Vec<String> {
    vec![
        "git".into(),
        "cargo".into(),
        "npm".into(),
        "pnpm".into(),
        "yarn".into(),
        "node".into(),
        "python".into(),
        "python3".into(),
    ]
}

fn default_denied_shells() -> Vec<String> {
    vec![
        "powershell".into(),
        "cmd".into(),
        "bash".into(),
        "sh".into(),
    ]
}

fn environment_essentials() -> Vec<String> {
    vec![
        "PATH".into(),
        "HOME".into(),
        "USERPROFILE".into(),
        "HOMEDRIVE".into(),
        "HOMEPATH".into(),
        "USERNAME".into(),
        "TEMP".into(),
        "TMP".into(),
        "TMPDIR".into(),
        "SYSTEMDRIVE".into(),
        "SYSTEMROOT".into(),
        "WINDIR".into(),
        "PATHEXT".into(),
        "ComSpec".into(),
        "RUST_BACKTRACE".into(),
        "RUST_LOG".into(),
    ]
}

/// A declared filesystem policy. Paths are repository-relative globs.
///
/// A declared `filesystem` table is complete: `write` lists the exact write
/// scopes (an absent or empty `write` means no writes at all), `read` the read
/// scopes (an absent or empty `read` means nothing may be read), and
/// `deny_write` always wins over any allow list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemPolicy {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub deny_write: Vec<String>,
}

/// Restrained command policy: what Factory lets a role/agent run and validates
/// against its reported commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandsMode {
    #[default]
    Unrestricted,
    Restricted,
    Denied,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandsPolicy {
    #[serde(default)]
    pub mode: CommandsMode,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    #[default]
    Allow,
    Deny,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub mode: NetworkMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPolicy {
    /// When non-empty, only these variable names are forwarded from the
    /// Factory process environment. Values explicitly configured on an agent
    /// always pass unless a `deny` entry removes them.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Variables stripped even when configured or allowed elsewhere.
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    Read,
    CommitInTaskWorktree,
    Push,
    ForcePush,
    DeleteBranch,
    ResetBranch,
    ModifyRemotes,
}

impl GitOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            GitOperation::Read => "read",
            GitOperation::CommitInTaskWorktree => "commit_in_task_worktree",
            GitOperation::Push => "push",
            GitOperation::ForcePush => "force_push",
            GitOperation::DeleteBranch => "delete_branch",
            GitOperation::ResetBranch => "reset_branch",
            GitOperation::ModifyRemotes => "modify_remotes",
        }
    }

    pub fn parse(value: &str) -> Option<GitOperation> {
        match value {
            "read" => Some(Self::Read),
            "commit_in_task_worktree" | "commit" => Some(Self::CommitInTaskWorktree),
            "push" => Some(Self::Push),
            "force_push" => Some(Self::ForcePush),
            "delete_branch" => Some(Self::DeleteBranch),
            "reset_branch" => Some(Self::ResetBranch),
            "modify_remotes" => Some(Self::ModifyRemotes),
            _ => None,
        }
    }

    /// A Factory safety invariant: never permitted to any task agent.
    pub fn is_dangerous(self) -> bool {
        matches!(
            self,
            Self::Push
                | Self::ForcePush
                | Self::DeleteBranch
                | Self::ResetBranch
                | Self::ModifyRemotes
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitPolicy {
    /// Git operations the role/agent may perform, as `GitOperation::as_str`
    /// names. Dangerous operations are removed by the resolver no matter what.
    #[serde(default)]
    pub allow: Vec<String>,
}

// --- Effective policy ------------------------------------------------------

/// One resolved policy for a running (role, agent) pair. Produced by
/// `PoliciesConfig::effective`; consumed by execution, validation, audit, and
/// the dashboard — never re-implemented elsewhere.
#[derive(Debug, Clone)]
pub struct EffectivePolicy {
    pub source: String,
    pub permissive: bool,
    pub filesystem: EffectiveFilesystem,
    pub commands: EffectiveCommands,
    pub network: EffectiveNetwork,
    pub environment: EffectiveEnvironment,
    pub git: EffectiveGit,
}

/// Filesystem rules after role/agent merging.
#[derive(Debug, Clone)]
pub struct EffectiveFilesystem {
    read_scopes: Vec<String>,
    /// Present only when the agent scope restricts reads further.
    read_agent: Option<Vec<String>>,
    write_scopes: Vec<String>,
    /// Present only when the agent scope restricts writes further. An empty
    /// list means "nothing writable".
    write_agent: Option<Vec<String>>,
    deny_scopes: Vec<String>,
}

impl Default for EffectiveFilesystem {
    fn default() -> Self {
        Self {
            read_scopes: vec!["**".to_string()],
            read_agent: None,
            write_scopes: vec!["**".to_string()],
            write_agent: None,
            deny_scopes: factory_filesystem_baseline_deny(),
        }
    }
}

fn factory_filesystem_baseline_deny() -> Vec<String> {
    baseline_deny_write()
}

impl EffectiveFilesystem {
    fn resolve(role: Option<&PolicyScope>, agent: Option<&PolicyScope>) -> Self {
        let mut fs = Self::default();
        if let Some(role_fs) = role.and_then(|scope| scope.filesystem.as_ref()) {
            if !role_fs.read.is_empty() {
                fs.read_scopes = role_fs.read.clone();
            }
            fs.write_scopes = role_fs.write.clone();
            fs.deny_scopes.extend(role_fs.deny_write.iter().cloned());
        }
        if let Some(agent_fs) = agent.and_then(|scope| scope.filesystem.as_ref()) {
            if !agent_fs.read.is_empty() {
                fs.read_agent = Some(agent_fs.read.clone());
            }
            fs.write_agent = Some(agent_fs.write.clone());
            fs.deny_scopes.extend(agent_fs.deny_write.iter().cloned());
        }
        fs
    }

    /// Whether a repository path may be read.
    pub fn read_allowed(&self, repo_relative: &str) -> bool {
        self.in_allowed(repo_relative, &self.read_scopes)
            && self
                .read_agent
                .as_ref()
                .is_none_or(|scopes| self.in_allowed(repo_relative, scopes))
            && !self.in_denied(repo_relative)
    }

    /// Whether a repository-relative write to `path` is permitted.
    pub fn write_allowed(&self, repo_relative: &str) -> bool {
        self.in_allowed(repo_relative, &self.write_scopes)
            && self
                .write_agent
                .as_ref()
                .is_none_or(|scopes| self.in_allowed(repo_relative, scopes))
            && !self.in_denied(repo_relative)
    }

    /// Normalizes an evidence path and checks it. Returns the normalized path
    /// on success; `Err(reason)` when the path cannot be mapped into the
    /// repository (traversal/absolute) or is not inside a write scope.
    pub fn check_write(&self, path: &str) -> Result<String, String> {
        let normalized = normalize_repo_relative(path)
            .ok_or_else(|| format!("'{path}' is outside the repository"))?;
        if !self.write_allowed(&normalized) {
            return Err(format!(
                "'{normalized}' is not inside an allowed write scope"
            ));
        }
        Ok(normalized)
    }

    /// Checks every changed file of an attempt against the write scopes.
    pub fn write_violations(&self, changed_files: &[String]) -> Vec<String> {
        changed_files
            .iter()
            .filter_map(|file| self.check_write(file).err())
            .collect()
    }

    /// Whether any write at all is permitted (used to pre-block mutating
    /// operations on read-only policies).
    pub fn read_only(&self) -> bool {
        if self.write_scopes.is_empty() {
            return true;
        }
        match &self.write_agent {
            Some(scopes) => scopes.is_empty(),
            None => false,
        }
    }

    /// `open` (writes anywhere), `restricted` (explicit write scopes), or
    /// `read_only` — the form shown in the inspectors and stored in session
    /// audits.
    pub fn mode_name(&self) -> &'static str {
        if self.read_only() {
            "read_only"
        } else if self.write_scopes.iter().any(|scope| scope != "**") {
            "restricted"
        } else {
            "open"
        }
    }

    pub fn read_scopes(&self) -> &[String] {
        &self.read_scopes
    }

    /// The scopes listed in session audit and the UI, including the agent
    /// restriction when one applies.
    pub fn effective_write_scopes(&self) -> Vec<String> {
        match &self.write_agent {
            Some(scopes) if !scopes.is_empty() => {
                let mut combined = self.write_scopes.clone();
                for scope in scopes {
                    combined.push(format!("(agent) {scope}"));
                }
                combined
            }
            _ => self.write_scopes.clone(),
        }
    }

    pub fn deny_scopes(&self) -> &[String] {
        &self.deny_scopes
    }

    fn in_allowed(&self, path: &str, scopes: &[String]) -> bool {
        if scopes.is_empty() {
            return false;
        }
        scopes
            .iter()
            .any(|scope| matches_glob(scope, path, !cfg!(windows)))
    }

    fn in_denied(&self, path: &str) -> bool {
        self.deny_scopes
            .iter()
            .any(|scope| matches_glob(scope, path, !cfg!(windows)))
    }
}

/// Commands after role/agent merging.
#[derive(Debug, Clone)]
pub struct EffectiveCommands {
    pub mode: CommandsMode,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl Default for EffectiveCommands {
    fn default() -> Self {
        Self {
            mode: CommandsMode::Unrestricted,
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }
}

impl EffectiveCommands {
    fn resolve(role: Option<&PolicyScope>, agent: Option<&PolicyScope>) -> Self {
        let mut mode = CommandsMode::Unrestricted;
        let mut allow: Vec<String> = Vec::new();
        let mut deny: Vec<String> = Vec::new();
        for scope in [role, agent].into_iter().flatten() {
            let Some(commands) = scope.commands.as_ref() else {
                continue;
            };
            // More restrictive wins; an agent can never widen.
            mode = mode.max(commands.mode);
            if commands.mode == CommandsMode::Restricted && !commands.allow.is_empty() {
                allow = if allow.is_empty() {
                    commands.allow.clone()
                } else {
                    allow
                        .iter()
                        .filter(|candidate| {
                            commands
                                .allow
                                .iter()
                                .any(|other| candidate.eq_ignore_ascii_case(other))
                        })
                        .cloned()
                        .collect()
                };
            }
            deny.extend(commands.deny.iter().cloned());
        }
        // Restricted with nothing allowed is effectively denied.
        if mode == CommandsMode::Restricted && allow.is_empty() {
            mode = CommandsMode::Denied;
        }
        Self { mode, allow, deny }
    }

    /// Whether running `command` is permitted. The command's executable (its
    /// first whitespace-delimited token) is compared case-insensitively against
    /// the allow/deny lists; `git commit -m ...` therefore matches `git`.
    pub fn allowed(&self, command: &str) -> bool {
        let name = command_executable(command);
        if self
            .deny
            .iter()
            .any(|denied| name == command_executable(denied))
        {
            return false;
        }
        match self.mode {
            CommandsMode::Unrestricted => true,
            CommandsMode::Restricted => {
                // Dangerous Git invocations are denied even when `git` itself
                // is on the allow list (the Git permission model protects
                // push / force push / branch deletion / resets / remotes).
                if is_dangerous_git_invocation(command) {
                    return false;
                }
                self.allow
                    .iter()
                    .any(|allowed| name == command_executable(allowed))
            }
            CommandsMode::Denied => false,
        }
    }

    /// Every denied command in a reported command list.
    pub fn violations(&self, commands: &[String]) -> Vec<String> {
        if self.mode == CommandsMode::Unrestricted {
            return Vec::new();
        }
        commands
            .iter()
            .filter(|command| !self.allowed(command))
            .cloned()
            .collect()
    }
}

/// The lowercase executable name of a command: its first whitespace-delimited
/// token with any directory part removed (`git commit -m ...` → `git`).
fn command_executable(command: &str) -> String {
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or(command.trim())
        .to_lowercase();
    let basename = first.rsplit(['/', '\\']).next().unwrap_or(&first);
    basename.to_string()
}

/// Whether an invocation runs a dangerous Git subcommand (the same set the Git
/// permission model refuses in the factory safety invariants).
fn is_dangerous_git_invocation(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    let executable = parts.next().map(command_executable).unwrap_or_default();
    if !matches!(executable.as_str(), "git") {
        return false;
    }
    let subcommand = parts.next().map(|part| part.to_lowercase());
    match subcommand.as_deref() {
        Some("push" | "reset" | "remote") => true,
        Some("branch") => parts
            .next()
            .is_some_and(|flag| matches!(flag, "-d" | "-D" | "--delete")),
        _ => false,
    }
}

/// Network mode. Factory cannot reliably restrict the network of an arbitrary
/// launched process on the current OS, so every determination remains
/// advisory: the mode is recorded and injected into the mission, never claimed
/// as a sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveNetwork {
    pub mode: NetworkMode,
}

impl Default for EffectiveNetwork {
    fn default() -> Self {
        Self {
            mode: NetworkMode::Allow,
        }
    }
}

impl EffectiveNetwork {
    fn resolve(role: Option<&PolicyScope>, agent: Option<&PolicyScope>) -> Self {
        let mut mode = NetworkMode::Allow;
        for scope in [role, agent].into_iter().flatten() {
            let denies = scope
                .network
                .as_ref()
                .map(|network| network.mode == NetworkMode::Deny)
                .unwrap_or(false);
            if denies {
                mode = NetworkMode::Deny;
            }
        }
        Self { mode }
    }

    pub fn allowed(&self) -> bool {
        self.mode == NetworkMode::Allow
    }
}

/// Environment filtering rules after merging.
#[derive(Debug, Clone, Default)]
pub struct EffectiveEnvironment {
    /// Whether inheritance is restricted to an allow list.
    pub filtered: bool,
    /// Allow list when filtered; empty otherwise.
    pub allowed: Vec<String>,
    /// Keys always removed.
    pub denied: Vec<String>,
}

impl EffectiveEnvironment {
    fn resolve(role: Option<&PolicyScope>, agent: Option<&PolicyScope>) -> Self {
        let mut allowed: Vec<String> = Vec::new();
        let mut denied: Vec<String> = Vec::new();
        let mut any_restriction = false;
        for scope in [role, agent].into_iter().flatten() {
            let Some(environment) = scope.environment.as_ref() else {
                continue;
            };
            any_restriction = true;
            if !environment.allow.is_empty() {
                allowed = if allowed.is_empty() {
                    environment.allow.clone()
                } else {
                    allowed
                        .iter()
                        .filter(|candidate| {
                            environment
                                .allow
                                .iter()
                                .any(|other| candidate.eq_ignore_ascii_case(other))
                        })
                        .cloned()
                        .collect()
                };
            }
            denied.extend(environment.deny.iter().cloned());
        }
        let filtered = any_restriction && !allowed.is_empty();
        Self {
            filtered,
            allowed,
            denied,
        }
    }

    /// Whether inheritance filtering applies (vs. full inheritance).
    pub fn mode(&self) -> &'static str {
        if self.filtered {
            "filtered"
        } else {
            "full"
        }
    }

    /// Builds the exact process environment from the inherited one.
    pub fn environment(
        &self,
        inherited: impl IntoIterator<Item = (String, String)>,
    ) -> BTreeMap<String, String> {
        filter_environment(inherited, &self.allowed, &self.denied)
    }

    /// Strips denied keys from an agent's configured environment.
    pub fn filter_configured(
        &self,
        configured: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        if self.denied.is_empty() {
            configured.clone()
        } else {
            filter_configured_env(configured, &self.denied)
        }
    }
}

/// Git operations permitted to the task agent, with the Factory safety
/// invariants already removed.
#[derive(Debug, Clone)]
pub struct EffectiveGit {
    pub allowed: BTreeSet<GitOperation>,
}

fn factory_git_baseline_allowed() -> BTreeSet<GitOperation> {
    [GitOperation::Read, GitOperation::CommitInTaskWorktree]
        .into_iter()
        .collect()
}

impl Default for EffectiveGit {
    fn default() -> Self {
        Self {
            allowed: factory_git_baseline_allowed(),
        }
    }
}

impl EffectiveGit {
    fn resolve(role: Option<&PolicyScope>, agent: Option<&PolicyScope>) -> Self {
        let mut allowed = factory_git_baseline_allowed();
        let restrictions: Vec<&GitPolicy> = [role, agent]
            .into_iter()
            .flatten()
            .filter_map(|scope| scope.git.as_ref())
            .filter(|git| !git.allow.is_empty())
            .collect();
        for git in restrictions {
            let declared: BTreeSet<GitOperation> = git
                .allow
                .iter()
                .filter_map(|name| GitOperation::parse(name))
                .collect();
            // Allowed = declared ∩ current (an agent only narrows);
            // dangerous operations are dropped regardless (invariants).
            allowed = allowed
                .intersection(&declared)
                .filter(|operation| !operation.is_dangerous())
                .copied()
                .collect();
        }
        Self { allowed }
    }

    pub fn allows(&self, operation: GitOperation) -> bool {
        self.allowed.contains(&operation)
    }
}

// --- Preset merging --------------------------------------------------------

fn merged_scope(scope: Option<&PolicyScope>) -> Option<PolicyScope> {
    let scope = scope?;
    let Some(preset) = scope.preset else {
        return Some(scope.clone());
    };
    let defaults = preset.defaults();
    let _ = preset == PolicyPreset::Custom; // handled uniformly below
    Some(PolicyScope {
        preset: scope.preset,
        filesystem: scope.filesystem.clone().or(defaults.filesystem),
        commands: scope.commands.clone().or(defaults.commands),
        network: scope.network.clone().or(defaults.network),
        environment: scope.environment.clone().or(defaults.environment),
        git: scope.git.clone().or(defaults.git),
    })
}

// --- Execution-time validation ---------------------------------------------

/// Whether a task operation mutates repository files and therefore requires
/// write scopes.
pub fn operation_is_mutating(operation: &str) -> bool {
    matches!(operation, "implement" | "verify" | "post_process")
}

/// Validates a task before execution against the effective policy. Returns a
/// useful reason when the task cannot legally execute (used to block a run
/// before any agent process is launched).
pub fn validate_executable(policy: &EffectivePolicy, operation: &str) -> Result<(), String> {
    if operation_is_mutating(operation) && policy.filesystem.read_only() {
        return Err(format!(
            "cannot perform operation '{operation}' with no writable filesystem scope \
             (allowed writes: {})",
            describe_scopes(policy.filesystem.effective_write_scopes())
        ));
    }
    Ok(())
}

fn describe_scopes(scopes: Vec<String>) -> String {
    if scopes.is_empty() {
        "none".to_string()
    } else {
        scopes.join(", ")
    }
}

// --- UI view ---------------------------------------------------------------

/// Compact serializable policy summary shown in the Role/Agent inspectors,
/// derived from the effective policy by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyView {
    pub source: String,
    /// `true` when no policy is configured anywhere: legacy permissive mode.
    pub permissive: bool,
    pub filesystem_mode: String,
    pub read_scopes: Vec<String>,
    pub write_scopes: Vec<String>,
    pub deny_write_scopes: Vec<String>,
    pub commands_mode: String,
    pub commands_allow: Vec<String>,
    pub commands_deny: Vec<String>,
    pub network: String,
    /// Always `advisory`: Factory cannot guarantee process-level network
    /// isolation on the current OS.
    pub network_enforcement: String,
    pub environment_mode: String,
    pub environment_allowed: Vec<String>,
    pub environment_denied: Vec<String>,
    pub git_allowed: Vec<String>,
    pub git_denied: Vec<String>,
}

impl EffectivePolicy {
    pub fn view(&self) -> PolicyView {
        let filesystem_mode = self.filesystem.mode_name();
        PolicyView {
            source: self.source.clone(),
            permissive: self.permissive,
            filesystem_mode: filesystem_mode.to_string(),
            read_scopes: self.filesystem.read_scopes().to_vec(),
            write_scopes: self.filesystem.effective_write_scopes(),
            deny_write_scopes: self.filesystem.deny_scopes().to_vec(),
            commands_mode: match self.commands.mode {
                CommandsMode::Unrestricted => "unrestricted".to_string(),
                CommandsMode::Restricted => "restricted".to_string(),
                CommandsMode::Denied => "denied".to_string(),
            },
            commands_allow: self.commands.allow.clone(),
            commands_deny: self.commands.deny.clone(),
            network: match self.network.mode {
                NetworkMode::Allow => "allow".to_string(),
                NetworkMode::Deny => "deny".to_string(),
            },
            network_enforcement: "advisory".to_string(),
            environment_mode: self.environment.mode().to_string(),
            environment_allowed: self.environment.allowed.clone(),
            environment_denied: self.environment.denied.clone(),
            git_allowed: self
                .git
                .allowed
                .iter()
                .map(|operation| operation.as_str().to_string())
                .collect(),
            git_denied: all_dangerous_git_operations(),
        }
    }
}

fn all_dangerous_git_operations() -> Vec<String> {
    [
        GitOperation::Push,
        GitOperation::ForcePush,
        GitOperation::DeleteBranch,
        GitOperation::ResetBranch,
        GitOperation::ModifyRemotes,
    ]
    .iter()
    .map(|operation| operation.as_str().to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the real `.factory/config.toml` nesting: policies live under a
    /// `[policies]` table (with `roles` and `agents` sub-tables).
    #[derive(serde::Deserialize)]
    struct PoliciesWrapper {
        #[serde(default)]
        policies: PoliciesConfig,
    }

    fn config(toml: &str) -> PoliciesConfig {
        let text = format!("[policies]\n{toml}");
        let wrapper: PoliciesWrapper = toml::from_str(&text).unwrap();
        wrapper.policies
    }

    #[test]
    fn legacy_config_resolves_permissive_defaults() {
        let config = PoliciesConfig::default();
        let policy = config.effective("worker", "codex");
        assert!(policy.permissive);
        assert_eq!(policy.source, "default");
        assert!(!policy.filesystem.read_only());
        assert!(policy.filesystem.write_allowed("src/main.rs"));
        assert!(policy.filesystem.write_allowed("README.md"));
        assert_eq!(policy.commands.mode, CommandsMode::Unrestricted);
        assert!(policy.network.allowed());
        assert_eq!(policy.environment.mode(), "full");
        assert!(policy.git.allows(GitOperation::Read));
        assert!(policy.git.allows(GitOperation::CommitInTaskWorktree));
        assert!(!policy.git.allows(GitOperation::Push));
        assert!(!policy.git.allows(GitOperation::ResetBranch));
    }

    #[test]
    fn baseline_invariants_cannot_be_bypassed() {
        let config = config(
            r#"
[policies.roles.worker.git]
allow = ["push", "force_push", "delete_branch"]
"#,
        );
        let policy = config.effective("worker", "any");
        assert!(!policy.git.allows(GitOperation::Push));
        assert!(!policy.git.allows(GitOperation::ForcePush));
        assert!(!policy.git.allows(GitOperation::DeleteBranch));
    }

    #[test]
    fn filesystem_write_scopes_and_deny_override_allow() {
        let config = config(
            r#"
[policies.roles.worker.filesystem]
read = ["**"]
write = ["src/**", "tests/**"]
deny_write = ["src/legacy/**"]
"#,
        );
        let policy = config.effective("worker", "codex");
        assert!(policy.filesystem.write_allowed("src/main.rs"));
        assert!(policy.filesystem.write_allowed("tests/unit.rs"));
        assert!(!policy.filesystem.write_allowed("README.md"));
        assert!(!policy.filesystem.write_allowed("src/legacy/old.rs"));
    }

    #[test]
    fn deny_wins_over_allow_across_the_repository() {
        let config = config(
            r#"
[policies.roles.worker.filesystem]
write = ["**"]
deny_write = [".factory/**", ".github/**"]
"#,
        );
        let policy = config.effective("worker", "codex");
        assert!(policy.filesystem.write_allowed("anything.txt"));
        assert!(!policy.filesystem.write_allowed(".factory/config.toml"));
        assert!(!policy.filesystem.write_allowed(".github/workflows/ci.yml"));
    }

    #[test]
    fn read_only_roles_reject_writes() {
        let config = config(
            r#"
[policies.roles.reviewer.filesystem]
read = ["**"]
write = []
"#,
        );
        let policy = config.effective("reviewer", "claude");
        assert!(policy.filesystem.read_only());
        assert!(!policy.filesystem.write_allowed("src/main.rs"));
        assert!(
            validate_executable(&policy, "implement").is_err(),
            "a read-only role cannot legally execute implementation"
        );
        assert!(validate_executable(&policy, "review").is_ok());
    }

    #[test]
    fn path_traversal_and_absolute_paths_are_outside_write_scope() {
        let config = config(
            r#"
[policies.roles.worker.filesystem]
write = ["**"]
"#,
        );
        let policy = config.effective("worker", "codex");
        assert!(policy.filesystem.check_write("../outside.txt").is_err());
        assert!(policy.filesystem.check_write("/etc/passwd").is_err());
        assert!(policy
            .filesystem
            .check_write("C:/Windows/system.ini")
            .is_err());
        assert!(policy.filesystem.check_write("src/../../etc/x").is_err());
        assert!(policy.filesystem.check_write("src/lib.rs").is_ok());
    }

    #[test]
    fn write_violation_report_names_problem_paths() {
        let config = config(
            r#"
[policies.roles.worker.filesystem]
write = ["src/**"]
"#,
        );
        let policy = config.effective("worker", "codex");
        let violations = policy
            .filesystem
            .write_violations(&["src/ok.rs".into(), "README.md".into()]);
        assert_eq!(
            violations,
            ["'README.md' is not inside an allowed write scope"]
        );
    }

    #[test]
    fn agent_scope_further_restricts_a_role() {
        let config = config(
            r#"
[policies.roles.worker.filesystem]
write = ["src/**", "tests/**"]

[policies.agents.codex.filesystem]
write = ["src/**"]
"#,
        );
        let policy = config.effective("worker", "codex");
        assert!(policy.filesystem.write_allowed("src/main.rs"));
        assert!(
            !policy.filesystem.write_allowed("tests/unit.rs"),
            "the agent narrows the role's write scopes"
        );
        let other = config.effective("worker", "opencode");
        assert!(other.filesystem.write_allowed("tests/unit.rs"));
    }

    #[test]
    fn agent_cannot_widen_a_role() {
        let config = config(
            r#"
[policies.roles.worker.filesystem]
write = ["src/**"]

[policies.agents.codex.filesystem]
write = ["**"]
"#,
        );
        let policy = config.effective("worker", "codex");
        assert!(
            !policy.filesystem.write_allowed("README.md"),
            "the agent's wider scope cannot bypass the role restriction"
        );
    }

    #[test]
    fn preset_read_only_expands_into_a_read_only_policy() {
        let config = config(
            r#"
[policies.roles.researcher]
preset = "read_only"
"#,
        );
        let policy = config.effective("researcher", "claude");
        assert!(!policy.permissive);
        assert!(policy.filesystem.read_only());
        assert!(policy.filesystem.read_allowed("src/main.rs"));
        let env = policy.environment.environment([
            ("PATH".into(), "/bin".into()),
            ("GITHUB_TOKEN".into(), "t".into()),
        ]);
        assert_eq!(env.get("GITHUB_TOKEN"), None);
        assert_eq!(policy.environment.mode(), "filtered");
        assert!(policy.commands.allowed("git"));
        assert!(!policy.commands.allowed("bash"));
    }

    #[test]
    fn preset_implementation_allows_worktree_writes_and_restricted_commands() {
        let config = config(
            r#"
[policies.roles.worker]
preset = "implementation"
"#,
        );
        let policy = config.effective("worker", "opencode");
        assert!(!policy.filesystem.read_only());
        assert!(policy.filesystem.write_allowed("src/lib.rs"));
        assert!(policy.commands.allowed("cargo"));
        assert!(policy.commands.allowed("npm"));
        assert!(policy.commands.allowed("git"));
        assert!(!policy.commands.allowed("powershell"));
        assert!(!policy.commands.allowed("bash"));
        assert!(
            !policy.filesystem.write_allowed(".factory/config.toml"),
            "the safety invariant protects Factory state"
        );
        assert!(validate_executable(&policy, "implement").is_ok());
    }

    #[test]
    fn preset_documentation_limits_writes_to_readme_and_docs() {
        let config = config(
            r#"
[policies.roles.documentation_writer]
preset = "documentation"
"#,
        );
        let policy = config.effective("documentation_writer", "claude");
        assert!(policy.filesystem.write_allowed("README.md"));
        assert!(policy.filesystem.write_allowed("docs/guide.md"));
        assert!(!policy.filesystem.write_allowed("src/main.rs"));
        assert!(validate_executable(&policy, "implement").is_ok());
        let _ = &policy; // (spelling of the scope name above is intentional)
    }

    #[test]
    fn command_allow_and_deny_and_restricted_violations() {
        let cfg = config(
            r#"
[policies.roles.worker.commands]
mode = "restricted"
allow = ["cargo", "npm", "git"]
deny = ["powershell", "cmd", "bash"]
"#,
        );
        let policy = cfg.effective("worker", "codex");
        assert!(policy.commands.allowed("cargo"));
        assert!(policy.commands.allowed("git"));
        assert!(policy.commands.allowed("NPM"), "case-insensitive");
        assert!(!policy.commands.allowed("powershell"));
        assert!(!policy.commands.allowed("bash"));
        assert_eq!(
            policy
                .commands
                .violations(&["npm".into(), "rm -rf /".into()]),
            ["rm -rf /"]
        );
        assert_eq!(
            policy
                .commands
                .violations(&["git diff".into(), "git push origin main".into()]),
            ["git push origin main"]
        );
        let denied = config(
            r#"
[policies.roles.reader.commands]
mode = "denied"
"#,
        );
        assert_eq!(
            denied.effective("reader", "a").commands.mode,
            CommandsMode::Denied
        );
    }

    #[test]
    fn network_deny_cannot_be_widened() {
        let cfg = config(
            r#"
[policies.roles.worker.network]
mode = "allow"

[policies.agents.codex.network]
mode = "deny"
"#,
        );
        assert!(!cfg.effective("worker", "codex").network.allowed());
        let wide = config(
            r#"
[policies.roles.worker.network]
mode = "deny"

[policies.agents.codex.network]
mode = "allow"
"#,
        );
        assert!(!wide.effective("worker", "codex").network.allowed());
    }

    #[test]
    fn environment_allow_list_restricts_inheritance_and_deny_wins() {
        let config = config(
            r#"
[policies.roles.worker.environment]
allow = ["PATH", "HOME"]
deny = ["GITHUB_TOKEN"]
"#,
        );
        let inherited = vec![
            ("PATH".to_string(), "/bin".to_string()),
            ("HOME".to_string(), "/home/me".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "x".to_string()),
            ("GITHUB_TOKEN".to_string(), "tok".to_string()),
        ];
        let env = config
            .effective("worker", "codex")
            .environment
            .environment(inherited);
        assert_eq!(env.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(env.get("HOME").map(String::as_str), Some("/home/me"));
        assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!env.contains_key("GITHUB_TOKEN"));
    }

    #[test]
    fn environment_agent_scope_narrows_the_role_allow_list() {
        let config = config(
            r#"
[policies.roles.worker.environment]
allow = ["PATH", "HOME", "FOO"]

[policies.agents.codex.environment]
allow = ["PATH"]
"#,
        );
        let inherited = vec![
            ("PATH".into(), "/bin".into()),
            ("HOME".into(), "/home/me".into()),
            ("FOO".into(), "bar".into()),
        ];
        let env = config
            .effective("worker", "codex")
            .environment
            .environment(inherited);
        assert_eq!(env.len(), 1);
        assert!(env.contains_key("PATH"));
        assert!(!env.contains_key("FOO"));
    }

    #[test]
    fn configured_env_keeps_working_unless_denied() {
        let config = config(
            r#"
[policies.roles.worker.environment]
deny = ["OPENAI_API_KEY"]
"#,
        );
        let environment = config.effective("worker", "codex").environment;
        let configured: BTreeMap<String, String> =
            [("OPENAI_API_KEY".to_string(), "secret".to_string())].into();
        assert!(environment.filter_configured(&configured).is_empty());
        let configured_ok: BTreeMap<String, String> =
            [("CUSTOM_KEY".to_string(), "value".to_string())].into();
        assert_eq!(environment.filter_configured(&configured_ok).len(), 1);
    }

    #[test]
    fn custom_role_preset_custom_with_explicit_dimensions() {
        let config = config(
            r#"
[policies.roles.database_engineer]
preset = "custom"

[policies.roles.database_engineer.filesystem]
write = ["migrations/**", "src/**"]
"#,
        );
        let policy = config.effective("database_engineer", "codex");
        assert!(!policy.filesystem.read_only());
        assert!(policy.filesystem.write_allowed("migrations/0001.sql"));
        assert!(policy.filesystem.write_allowed("src/db.rs"));
        assert!(!policy.filesystem.write_allowed("README.md"));
        // Custom preset leaves other dimensions at legacy defaults.
        assert_eq!(policy.commands.mode, CommandsMode::Unrestricted);
        assert_eq!(policy.environment.mode(), "full");
    }

    #[test]
    fn policy_scope_validation_rejects_bad_patterns() {
        let bad = config(
            r#"
[policies.roles.worker.filesystem]
write = ["../escape"]
"#,
        );
        assert!(bad.validate().is_err());
        let ok = config(
            r#"
[policies.roles.worker.filesystem]
write = ["src/**"]
"#,
        );
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn operation_validation_blocks_write_less_roles() {
        let config = config(
            r#"
[policies.roles.doc_writer]
preset = "read_only"
"#,
        );
        let policy = config.effective("doc_writer", "claude");
        let error = validate_executable(&policy, "implement").unwrap_err();
        assert!(
            error.contains("no writable filesystem scope"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn view_marks_legacy_config_as_permissive() {
        let view = PoliciesConfig::default()
            .effective("worker", "codex")
            .view();
        assert!(view.permissive);
        assert_eq!(view.filesystem_mode, "open");
        assert_eq!(view.environment_mode, "full");
        assert_eq!(view.commands_mode, "unrestricted");
        assert_eq!(view.network, "allow");
        assert_eq!(view.network_enforcement, "advisory");
        assert!(view.git_allowed.contains(&"read".to_string()));
        assert!(view.git_denied.contains(&"push".to_string()));
    }

    #[test]
    fn view_reports_restricted_dimensions() {
        let cfg = config(
            r#"
[policies.roles.worker]
preset = "implementation"
"#,
        );
        let view = cfg.effective("worker", "opencode").view();
        assert!(!view.permissive);
        assert_eq!(view.source, "role:worker");
        assert_eq!(view.filesystem_mode, "open", "worktree-wide writes");
        assert_eq!(view.environment_mode, "filtered");
        assert_eq!(view.commands_mode, "restricted");
        assert!(view.write_scopes.contains(&"**".to_string()));
        let restricted = config(
            r#"
[policies.roles.doc_writer.filesystem]
write = ["README.md", "docs/**"]
"#,
        );
        let view = restricted.effective("doc_writer", "claude").view();
        assert_eq!(view.filesystem_mode, "restricted");
        assert_eq!(view.write_scopes, vec!["README.md", "docs/**"]);
    }
}
