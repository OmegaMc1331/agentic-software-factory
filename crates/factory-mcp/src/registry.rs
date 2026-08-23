use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use factory_db::FactoryDb;
use factory_policy::EffectivePolicy;
use factory_types::{McpToolMetadata, ToolClassification, ToolGrant, ToolInvocation, ToolMetrics};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    McpCallOutput, McpClient, McpError, McpFailureKind, McpServerConfig, McpTransport, Result,
    ToolResolver,
};

const GRANT_LIFETIME_HOURS: i64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerStatus {
    Connected,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    pub id: String,
    pub transport: McpTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// References only (`${NAME}`), never resolved values.
    pub env: BTreeMap<String, String>,
    /// References only (`${NAME}`), never resolved values.
    pub headers: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub discovery_timeout_ms: u64,
    pub max_response_bytes: usize,
    pub max_concurrency: usize,
    pub classifications: BTreeMap<String, ToolClassification>,
    pub status: McpServerStatus,
    pub tool_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl McpServerView {
    pub fn from_config(
        id: impl Into<String>,
        config: &McpServerConfig,
        status: McpServerStatus,
        tool_count: usize,
        last_error: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            transport: config.transport,
            command: config.command.clone(),
            args: config.args.clone(),
            cwd: config.cwd.clone(),
            url: config.url.clone(),
            env: config.env.clone(),
            headers: config.headers.clone(),
            timeout_ms: config.timeout_ms,
            discovery_timeout_ms: config.discovery_timeout_ms,
            max_response_bytes: config.max_response_bytes,
            max_concurrency: config.max_concurrency,
            classifications: config.classifications.clone(),
            status,
            tool_count,
            last_error,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GrantContext {
    pub run_id: Option<i64>,
    pub task_id: Option<i64>,
    pub attempt_id: Option<i64>,
    pub session_id: Option<i64>,
    pub agent: String,
    pub role: String,
    pub operation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolGrantSecret {
    pub id: String,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct ToolRegistry {
    root: PathBuf,
    db_path: PathBuf,
    servers: BTreeMap<String, McpServerConfig>,
}

/// Releases a per-server lease even when an async invocation future is
/// cancelled/dropped. The transport is owned by that same future, so dropping
/// it also tears down the MCP client or child process.
struct ServerLease {
    db_path: PathBuf,
    server: String,
}

impl Drop for ServerLease {
    fn drop(&mut self) {
        if let Ok(db) = FactoryDb::open(&self.db_path) {
            let _ = db.release_mcp_server_slot(&self.server);
        }
    }
}

/// Gives every accepted invocation a terminal audit state even if its async
/// future is dropped during discovery, queueing, transport setup, or I/O.
struct InvocationGuard {
    db_path: PathBuf,
    invocation_id: i64,
    started: Instant,
}

impl Drop for InvocationGuard {
    fn drop(&mut self) {
        if let Ok(db) = FactoryDb::open(&self.db_path) {
            let _ = db.cancel_running_tool_invocation(
                self.invocation_id,
                self.started.elapsed().as_millis() as u64,
            );
        }
    }
}

impl ToolRegistry {
    pub fn new(root: impl Into<PathBuf>, servers: BTreeMap<String, McpServerConfig>) -> Self {
        let root = root.into();
        let db_path = root.join(".factory").join("db.sqlite3");
        Self {
            root,
            db_path,
            servers,
        }
    }

    pub fn servers(&self) -> &BTreeMap<String, McpServerConfig> {
        &self.servers
    }

    pub fn server_views(&self) -> Result<Vec<McpServerView>> {
        let db = self.db()?;
        let cached = db.list_mcp_tools(None).map_err(db_error)?;
        let mut counts = BTreeMap::<String, usize>::new();
        for tool in cached {
            *counts.entry(tool.server).or_default() += 1;
        }
        Ok(self
            .servers
            .iter()
            .map(|(id, config)| {
                let count = counts.get(id).copied().unwrap_or(0);
                McpServerView::from_config(
                    id,
                    config,
                    if count > 0 {
                        McpServerStatus::Connected
                    } else {
                        McpServerStatus::Unknown
                    },
                    count,
                    None,
                )
            })
            .collect())
    }

    pub fn cached_tools(&self, server_id: Option<&str>) -> Result<Vec<McpToolMetadata>> {
        let mut tools = self.db()?.list_mcp_tools(server_id).map_err(db_error)?;
        tools.retain(|tool| self.servers.contains_key(&tool.server));
        Ok(tools)
    }

    pub async fn discover_server(&self, server_id: &str) -> Result<Vec<McpToolMetadata>> {
        let config = self.servers.get(server_id).ok_or_else(|| {
            McpError::new(
                McpFailureKind::ServerUnavailable,
                format!("unknown MCP server '{server_id}'"),
            )
        })?;
        let tools = McpClient::new(&self.root)
            .discover(server_id, config)
            .await?;
        self.db()?
            .replace_mcp_tools(server_id, &tools)
            .map_err(db_error)?;
        Ok(tools)
    }

    pub fn discover_server_blocking(&self, server_id: &str) -> Result<Vec<McpToolMetadata>> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| McpError::new(McpFailureKind::ServerUnavailable, error.to_string()))?
            .block_on(self.discover_server(server_id))
    }

    /// Refreshes only servers that a role policy may reference. Failures are
    /// returned per server and never abort unrelated discovery.
    pub async fn discover_relevant(
        &self,
        policy: &EffectivePolicy,
    ) -> BTreeMap<String, Result<Vec<McpToolMetadata>>> {
        let mut results = BTreeMap::new();
        for id in self.servers.keys() {
            if policy.tools.may_allow_server(id) {
                results.insert(id.clone(), self.discover_server(id).await);
            }
        }
        results
    }

    pub fn discover_relevant_blocking(
        &self,
        policy: &EffectivePolicy,
    ) -> BTreeMap<String, Result<Vec<McpToolMetadata>>> {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(self.discover_relevant(policy)),
            Err(error) => self
                .servers
                .keys()
                .filter(|id| policy.tools.may_allow_server(id))
                .map(|id| {
                    (
                        id.clone(),
                        Err(McpError::new(
                            McpFailureKind::ServerUnavailable,
                            error.to_string(),
                        )),
                    )
                })
                .collect(),
        }
    }

    pub fn relevant_cached_tools(
        &self,
        policy: &EffectivePolicy,
        role: &str,
        operation: &str,
        objective: &str,
    ) -> Result<Vec<McpToolMetadata>> {
        let cached = self.cached_tools(None)?;
        Ok(ToolResolver
            .relevant(&cached, policy, role, operation, objective)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn create_grant(
        &self,
        context: GrantContext,
        allowed_tools: &[McpToolMetadata],
    ) -> Result<Option<ToolGrantSecret>> {
        if allowed_tools.is_empty() {
            return Ok(None);
        }
        let id = Uuid::new_v4().to_string();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let token_hash = hash_token(&token);
        let grant = ToolGrant {
            id: id.clone(),
            token_hash,
            run_id: context.run_id,
            task_id: context.task_id,
            attempt_id: context.attempt_id,
            session_id: context.session_id,
            agent: context.agent,
            role: context.role,
            operation: context.operation,
            allowed_tools: allowed_tools
                .iter()
                .map(|tool| tool.qualified_name.clone())
                .collect(),
            expires_at: (Utc::now() + TimeDelta::hours(GRANT_LIFETIME_HOURS)).to_rfc3339(),
            revoked_at: None,
        };
        self.db()?.insert_tool_grant(&grant).map_err(db_error)?;
        Ok(Some(ToolGrantSecret { id, token }))
    }

    pub fn revoke_grant(&self, id: &str) -> Result<()> {
        self.db()?.revoke_tool_grant(id).map_err(db_error)
    }

    pub fn revoke_session_grants(&self, session_id: i64) -> Result<()> {
        self.db()?
            .revoke_session_tool_grants(session_id)
            .map_err(db_error)
    }

    /// Resolves only the non-secret grant context. The raw capability is
    /// hashed before lookup and is never returned or persisted.
    pub fn grant_for_token(&self, token: &str) -> Result<Option<ToolGrant>> {
        self.db()?
            .get_tool_grant_by_hash(&hash_token(token))
            .map_err(db_error)
    }

    pub async fn execute_grant(
        &self,
        token: &str,
        qualified_name: &str,
        arguments: serde_json::Value,
        current_policy: &EffectivePolicy,
    ) -> Result<McpCallOutput> {
        let grant = self.grant_for_token(token)?.ok_or_else(|| {
            McpError::new(McpFailureKind::PermissionDenied, "invalid MCP tool grant")
        })?;
        let now = Utc::now().to_rfc3339();
        let (server_id, tool_name) = split_qualified(qualified_name)?;

        // Reject inactive or out-of-scope requests before discovery, process
        // launch, HTTP traffic, or any other interaction with the server.
        if !grant.active_at(&now)
            || !grant
                .allowed_tools
                .iter()
                .any(|allowed| allowed == qualified_name)
        {
            self.audit_rejection(&grant, server_id, tool_name, ToolClassification::Unknown)?;
            return Err(McpError::new(
                McpFailureKind::PermissionDenied,
                format!("tool '{qualified_name}' is not permitted for this task"),
            ));
        }

        let input_summary = summarize_input(&arguments);
        let started_at = Utc::now().to_rfc3339();
        let started = Instant::now();
        let cached_classification = self
            .cached_tools(Some(server_id))?
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .map(|tool| tool.classification)
            .unwrap_or(ToolClassification::Unknown);
        let invocation = self
            .db()?
            .start_tool_invocation(&ToolInvocation {
                id: 0,
                run_id: grant.run_id,
                task_id: grant.task_id,
                attempt_id: grant.attempt_id,
                session_id: grant.session_id,
                agent: grant.agent.clone(),
                role: grant.role.clone(),
                server: server_id.to_string(),
                tool: tool_name.to_string(),
                classification: cached_classification,
                status: "running".into(),
                started_at,
                duration_ms: None,
                input_summary,
                output_summary: None,
                error: None,
            })
            .map_err(db_error)?;
        let _invocation_guard = InvocationGuard {
            db_path: self.db_path.clone(),
            invocation_id: invocation.id,
            started,
        };

        let config = match self.servers.get(server_id).cloned() {
            Some(config) => config,
            None => {
                let error = McpError::new(
                    McpFailureKind::ServerUnavailable,
                    format!("MCP server '{server_id}' is no longer configured"),
                );
                self.finish_failure(invocation.id, &started, error.kind)?;
                return Err(error);
            }
        };
        let metadata = match self.ensure_tool(server_id, tool_name).await {
            Ok(metadata) => metadata,
            Err(error) => {
                self.finish_failure(invocation.id, &started, error.kind)?;
                return Err(error);
            }
        };
        self.db()?
            .update_tool_invocation_classification(invocation.id, metadata.classification)
            .map_err(db_error)?;

        let permitted = current_policy.tools.allowed(
            qualified_name,
            metadata.classification.requires_exact_permission(),
        );
        if !permitted {
            self.db()?
                .finish_tool_invocation(
                    invocation.id,
                    McpFailureKind::PermissionDenied.as_str(),
                    started.elapsed().as_millis() as u64,
                    None,
                    Some("tool is outside the active scoped grant"),
                )
                .map_err(db_error)?;
            return Err(McpError::new(
                McpFailureKind::PermissionDenied,
                format!("tool '{qualified_name}' is not permitted for this task"),
            ));
        }

        let _lease = match self.acquire_slot(server_id, &config).await {
            Ok(lease) => lease,
            Err(error) => {
                self.finish_failure(invocation.id, &started, error.kind)?;
                return Err(error);
            }
        };
        let result = McpClient::new(&self.root)
            .call(server_id, &config, tool_name, arguments)
            .await;

        match result {
            Ok(output) => {
                let summary = format!(
                    "success; {} bytes; untrusted external data",
                    serde_json::to_vec(&output.value)
                        .map(|bytes| bytes.len())
                        .unwrap_or(0)
                );
                self.db()?
                    .finish_tool_invocation(
                        invocation.id,
                        "success",
                        started.elapsed().as_millis() as u64,
                        Some(&summary),
                        None,
                    )
                    .map_err(db_error)?;
                Ok(output)
            }
            Err(error) => {
                self.finish_failure(invocation.id, &started, error.kind)?;
                Err(error)
            }
        }
    }

    pub fn metrics(&self) -> Result<ToolMetrics> {
        self.db()?.tool_metrics().map_err(db_error)
    }

    fn finish_failure(
        &self,
        invocation_id: i64,
        started: &Instant,
        kind: McpFailureKind,
    ) -> Result<()> {
        self.db()?
            .finish_tool_invocation(
                invocation_id,
                kind.as_str(),
                started.elapsed().as_millis() as u64,
                None,
                Some(failure_summary(kind)),
            )
            .map_err(db_error)
    }

    fn audit_rejection(
        &self,
        grant: &ToolGrant,
        server: &str,
        tool: &str,
        classification: ToolClassification,
    ) -> Result<()> {
        let invocation = self
            .db()?
            .start_tool_invocation(&ToolInvocation {
                id: 0,
                run_id: grant.run_id,
                task_id: grant.task_id,
                attempt_id: grant.attempt_id,
                session_id: grant.session_id,
                agent: grant.agent.clone(),
                role: grant.role.clone(),
                server: server.to_string(),
                tool: tool.to_string(),
                classification,
                status: "running".into(),
                started_at: Utc::now().to_rfc3339(),
                duration_ms: None,
                input_summary: "input withheld: permission denied".into(),
                output_summary: None,
                error: None,
            })
            .map_err(db_error)?;
        self.db()?
            .finish_tool_invocation(
                invocation.id,
                McpFailureKind::PermissionDenied.as_str(),
                0,
                None,
                Some("tool is outside the active scoped grant"),
            )
            .map_err(db_error)
    }

    async fn ensure_tool(&self, server_id: &str, tool_name: &str) -> Result<McpToolMetadata> {
        if let Some(tool) = self
            .cached_tools(Some(server_id))?
            .into_iter()
            .find(|tool| tool.name == tool_name)
        {
            return Ok(tool);
        }
        self.discover_server(server_id)
            .await?
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .ok_or_else(|| {
                McpError::new(
                    McpFailureKind::InvalidSchema,
                    format!("MCP server does not expose tool '{tool_name}'"),
                )
            })
    }

    async fn acquire_slot(&self, server_id: &str, config: &McpServerConfig) -> Result<ServerLease> {
        let deadline = Instant::now() + Duration::from_millis(config.timeout_ms);
        let stale_before =
            (Utc::now() - TimeDelta::milliseconds((config.timeout_ms * 2) as i64)).to_rfc3339();
        loop {
            if self
                .db()?
                .try_acquire_mcp_server_slot(server_id, config.max_concurrency, &stale_before)
                .map_err(db_error)?
            {
                return Ok(ServerLease {
                    db_path: self.db_path.clone(),
                    server: server_id.to_string(),
                });
            }
            if Instant::now() >= deadline {
                return Err(McpError::new(
                    McpFailureKind::Timeout,
                    format!("MCP server '{server_id}' concurrency queue timed out"),
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn db(&self) -> Result<FactoryDb> {
        FactoryDb::open(&self.db_path).map_err(db_error)
    }
}

fn split_qualified(value: &str) -> Result<(&str, &str)> {
    let (server, tool) = value.split_once('.').ok_or_else(|| {
        McpError::new(
            McpFailureKind::InvalidSchema,
            "tool name must be qualified as server.tool",
        )
    })?;
    if server.is_empty() || tool.is_empty() {
        return Err(McpError::new(
            McpFailureKind::InvalidSchema,
            "tool name must be qualified as server.tool",
        ));
    }
    crate::validate_server_id(server)
        .map_err(|message| McpError::new(McpFailureKind::InvalidSchema, message))?;
    crate::validate_tool_name(tool)
        .map_err(|message| McpError::new(McpFailureKind::InvalidSchema, message))?;
    Ok((server, tool))
}

pub fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn summarize_input(value: &serde_json::Value) -> String {
    let Some(object) = value.as_object() else {
        return "non-object input rejected".into();
    };
    format!(
        "JSON object with {} keys; names and values omitted",
        object.len()
    )
}

fn failure_summary(kind: McpFailureKind) -> &'static str {
    match kind {
        McpFailureKind::ServerUnavailable => "MCP server unavailable",
        McpFailureKind::Timeout => "MCP operation timed out",
        McpFailureKind::Cancelled => "MCP operation cancelled",
        McpFailureKind::PermissionDenied => "MCP permission denied",
        McpFailureKind::InvalidSchema => "MCP schema validation failed",
        McpFailureKind::ProtocolError => "MCP protocol error",
        McpFailureKind::ResponseTooLarge => "MCP response exceeded the configured limit",
        McpFailureKind::ToolFailure => "MCP tool reported a failure",
    }
}

fn db_error(error: factory_db::DbError) -> McpError {
    McpError::new(
        McpFailureKind::ServerUnavailable,
        format!("Factory MCP persistence failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_summaries_never_persist_values() {
        let summary = summarize_input(&serde_json::json!({
            "token": "super-secret-value",
            "query": "needle"
        }));
        assert!(summary.contains("2 keys"));
        assert!(!summary.contains("query"));
        assert!(!summary.contains("token"));
        assert!(!summary.contains("super-secret-value"));
        assert!(!summary.contains("needle"));
    }

    #[test]
    fn tokens_are_hashed_before_storage() {
        let token = "plain-capability-token";
        let hash = hash_token(token);
        assert_ne!(hash, token);
        assert!(!hash.contains(token));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn stale_cache_entries_for_unconfigured_servers_are_hidden() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".factory")).unwrap();
        let db = FactoryDb::open(&root.path().join(".factory/db.sqlite3")).unwrap();
        db.replace_mcp_tools(
            "removed",
            &[McpToolMetadata {
                server: "removed".into(),
                name: "search".into(),
                qualified_name: "removed.search".into(),
                description: "stale".into(),
                input_schema: serde_json::json!({"type": "object"}),
                annotations: None,
                classification: ToolClassification::Read,
                discovered_at: Utc::now().to_rfc3339(),
            }],
        )
        .unwrap();
        let registry = ToolRegistry::new(root.path(), BTreeMap::new());
        assert!(registry.cached_tools(None).unwrap().is_empty());
    }

    #[test]
    fn qualified_tool_names_are_bounded_and_structured() {
        assert_eq!(split_qualified("docs.search").unwrap(), ("docs", "search"));
        assert!(split_qualified("bad server.search").is_err());
        assert!(split_qualified(&format!("docs.{}", "x".repeat(129))).is_err());
    }
}
