use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use factory_policy::Secrets;
use factory_types::ToolClassification;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{McpError, McpFailureKind, Result};

pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_DISCOVERY_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;
pub const MIN_TIMEOUT_MS: u64 = 100;
pub const MAX_TIMEOUT_MS: u64 = 600_000;
pub const MIN_RESPONSE_BYTES: usize = 1024;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MAX_SERVER_CONCURRENCY: usize = 32;

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_discovery_timeout_ms() -> u64 {
    DEFAULT_DISCOVERY_TIMEOUT_MS
}

fn default_max_response_bytes() -> usize {
    DEFAULT_MAX_RESPONSE_BYTES
}

fn default_max_concurrency() -> usize {
    DEFAULT_MAX_CONCURRENCY
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_discovery_timeout_ms")]
    pub discovery_timeout_ms: u64,
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: usize,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub classifications: BTreeMap<String, ToolClassification>,
}

impl McpServerConfig {
    pub fn validate(&self, id: &str, root: &Path) -> std::result::Result<(), String> {
        validate_server_id(id)?;
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(format!(
                "mcp.{id}.timeout_ms must be between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS}"
            ));
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.discovery_timeout_ms) {
            return Err(format!(
                "mcp.{id}.discovery_timeout_ms must be between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS}"
            ));
        }
        if !(MIN_RESPONSE_BYTES..=MAX_RESPONSE_BYTES).contains(&self.max_response_bytes) {
            return Err(format!(
                "mcp.{id}.max_response_bytes must be between {MIN_RESPONSE_BYTES} and {MAX_RESPONSE_BYTES}"
            ));
        }
        if !(1..=MAX_SERVER_CONCURRENCY).contains(&self.max_concurrency) {
            return Err(format!(
                "mcp.{id}.max_concurrency must be between 1 and {MAX_SERVER_CONCURRENCY}"
            ));
        }
        for (key, reference) in &self.env {
            if !valid_env_name(key) {
                return Err(format!("mcp.{id} contains invalid environment key '{key}'"));
            }
            env_reference(reference).ok_or_else(|| {
                format!("mcp.{id}.{key} must be an environment reference like ${{NAME}}")
            })?;
        }
        for (key, reference) in &self.headers {
            http::HeaderName::from_bytes(key.as_bytes())
                .map_err(|_| format!("mcp.{id} contains invalid HTTP header '{key}'"))?;
            env_reference(reference).ok_or_else(|| {
                format!("mcp.{id}.{key} must be an environment reference like ${{NAME}}")
            })?;
        }
        for name in self.classifications.keys() {
            validate_tool_name(name).map_err(|reason| format!("mcp.{id}: {reason}"))?;
        }
        match self.transport {
            McpTransport::Stdio => {
                let command = self.command.as_deref().unwrap_or("").trim();
                if command.is_empty() || contains_control(command) {
                    return Err(format!(
                        "mcp.{id}.command is required and must be structured"
                    ));
                }
                if self.url.is_some() || !self.headers.is_empty() {
                    return Err(format!(
                        "mcp.{id} stdio servers cannot define url or headers"
                    ));
                }
                if self.args.iter().any(|arg| contains_control(arg)) {
                    return Err(format!("mcp.{id}.args contains control characters"));
                }
                if let Some(cwd) = &self.cwd {
                    resolve_cwd(root, cwd).map_err(|reason| format!("mcp.{id}.cwd {reason}"))?;
                }
            }
            McpTransport::Http => {
                if self.command.is_some() || !self.args.is_empty() || self.cwd.is_some() {
                    return Err(format!(
                        "mcp.{id} HTTP servers cannot define command, args, or cwd"
                    ));
                }
                let url = self
                    .url
                    .as_deref()
                    .ok_or_else(|| format!("mcp.{id}.url is required"))?;
                let parsed = Url::parse(url).map_err(|_| format!("mcp.{id}.url is invalid"))?;
                if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {
                    return Err(format!("mcp.{id}.url must be an absolute http(s) URL"));
                }
                if !parsed.username().is_empty() || parsed.password().is_some() {
                    return Err(format!("mcp.{id}.url must not contain credentials"));
                }
                if !self.env.is_empty() {
                    return Err(format!("mcp.{id} HTTP servers cannot define process env"));
                }
            }
        }
        Ok(())
    }

    pub fn resolved_cwd(&self, root: &Path) -> Result<PathBuf> {
        match self.cwd.as_deref() {
            Some(value) => resolve_cwd(root, value)
                .map_err(|reason| McpError::new(McpFailureKind::ServerUnavailable, reason)),
            None => Ok(root.to_path_buf()),
        }
    }

    pub fn resolve_env(&self) -> Result<(BTreeMap<String, String>, Secrets)> {
        resolve_references(&self.env)
    }

    pub fn resolve_headers(&self) -> Result<(BTreeMap<String, String>, Secrets)> {
        resolve_references(&self.headers)
    }
}

pub fn validate_server_id(id: &str) -> std::result::Result<(), String> {
    if id.is_empty()
        || id.chars().count() > 64
        || id
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-')))
    {
        return Err(format!("invalid MCP server id '{id}'"));
    }
    Ok(())
}

pub fn validate_tool_name(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
        return Err(format!("invalid MCP tool name '{name}'"));
    }
    Ok(())
}

fn valid_env_name(name: &str) -> bool {
    let mut characters = name.chars();
    name.len() <= 128
        && characters
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn env_reference(value: &str) -> Option<&str> {
    value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .filter(|name| valid_env_name(name))
}

fn resolve_references(
    references: &BTreeMap<String, String>,
) -> Result<(BTreeMap<String, String>, Secrets)> {
    let mut resolved = BTreeMap::new();
    let mut secrets = Secrets::new();
    for (target, reference) in references {
        let source = env_reference(reference).ok_or_else(|| {
            McpError::new(
                McpFailureKind::ServerUnavailable,
                format!("invalid environment reference for {target}"),
            )
        })?;
        let value = std::env::var(source).map_err(|_| {
            McpError::new(
                McpFailureKind::ServerUnavailable,
                format!("required environment variable {source} is not set"),
            )
        })?;
        // Explicit MCP env/header references are secrets regardless of length.
        secrets.add(&value, 1);
        resolved.insert(target.clone(), value);
    }
    Ok((resolved, secrets))
}

fn resolve_cwd(root: &Path, value: &str) -> std::result::Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("must remain repository-relative".into());
    }
    let joined = root.join(path);
    if !joined.is_dir() {
        return Err("must name an existing repository directory".into());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "cannot resolve the repository root")?;
    let canonical_joined = joined
        .canonicalize()
        .map_err(|_| "cannot resolve the configured directory")?;
    if !canonical_joined.starts_with(&canonical_root) {
        return Err("must not resolve outside the repository".into());
    }
    Ok(canonical_joined)
}

fn contains_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio() -> McpServerConfig {
        McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("fake-mcp".into()),
            args: Vec::new(),
            cwd: None,
            url: None,
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            discovery_timeout_ms: DEFAULT_DISCOVERY_TIMEOUT_MS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            classifications: BTreeMap::new(),
        }
    }

    #[test]
    fn validates_structured_stdio_and_secret_references() {
        let mut config = stdio();
        config.env.insert("TOKEN".into(), "${GITHUB_TOKEN}".into());
        assert!(config.validate("github", Path::new("repo")).is_ok());
        config.env.insert("BAD".into(), "plaintext".into());
        assert!(config.validate("github", Path::new("repo")).is_err());
    }

    #[test]
    fn even_short_explicit_mcp_secrets_are_redacted() {
        std::env::set_var("FACTORY_MCP_SHORT_SECRET", "x");
        let (_, secrets) = resolve_references(&BTreeMap::from([(
            "TOKEN".into(),
            "${FACTORY_MCP_SHORT_SECRET}".into(),
        )]))
        .unwrap();
        assert_eq!(secrets.redact("value=x"), "value=[REDACTED]");
        std::env::remove_var("FACTORY_MCP_SHORT_SECRET");
    }

    #[test]
    fn rejects_cwd_traversal_and_http_credentials() {
        let mut config = stdio();
        config.cwd = Some("../outside".into());
        assert!(config.validate("docs", Path::new("repo")).is_err());
        let mut config = stdio();
        config.transport = McpTransport::Http;
        config.command = None;
        config.url = Some("https://user:secret@example.com/mcp".into());
        assert!(config.validate("docs", Path::new("repo")).is_err());
    }

    #[test]
    fn accepts_standard_http_header_names_with_environment_references() {
        let mut config = stdio();
        config.transport = McpTransport::Http;
        config.command = None;
        config.url = Some("http://127.0.0.1:3001/mcp".into());
        config
            .headers
            .insert("X-API-Key".into(), "${DOCS_API_KEY}".into());
        assert!(config.validate("docs", Path::new("repo")).is_ok());
    }

    #[test]
    fn cwd_must_exist_and_resolve_inside_the_repository() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("inside")).unwrap();
        let mut config = stdio();
        config.cwd = Some("inside".into());
        assert!(config.validate("docs", root.path()).is_ok());

        config.cwd = Some("missing".into());
        assert!(config.validate("docs", root.path()).is_err());

        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("escape");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok();
        if linked {
            config.cwd = Some("escape".into());
            assert!(config.validate("docs", root.path()).is_err());
        }
    }
}
