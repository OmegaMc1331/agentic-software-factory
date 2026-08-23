mod client;
mod config;
mod error;
mod registry;
mod resolver;

pub use client::{McpCallOutput, McpClient};
pub use config::{
    validate_server_id, validate_tool_name, McpServerConfig, McpTransport,
    DEFAULT_DISCOVERY_TIMEOUT_MS, DEFAULT_MAX_CONCURRENCY, DEFAULT_MAX_RESPONSE_BYTES,
    DEFAULT_TIMEOUT_MS,
};
pub use error::{McpError, McpFailureKind, Result};
pub use registry::{
    hash_token, GrantContext, McpServerStatus, McpServerView, ToolGrantSecret, ToolRegistry,
};
pub use resolver::{render_tool_catalog, ToolResolver, MAX_MISSION_TOOLS};
