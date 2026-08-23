use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpFailureKind {
    ServerUnavailable,
    Timeout,
    Cancelled,
    PermissionDenied,
    InvalidSchema,
    ProtocolError,
    ResponseTooLarge,
    ToolFailure,
}

impl McpFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ServerUnavailable => "server_unavailable",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::PermissionDenied => "permission_denied",
            Self::InvalidSchema => "invalid_schema",
            Self::ProtocolError => "protocol_error",
            Self::ResponseTooLarge => "response_too_large",
            Self::ToolFailure => "tool_failure",
        }
    }

    pub fn infrastructure(self) -> bool {
        matches!(
            self,
            Self::ServerUnavailable
                | Self::Timeout
                | Self::Cancelled
                | Self::ProtocolError
                | Self::ResponseTooLarge
        )
    }
}

#[derive(Debug, Error)]
#[error("{kind:?}: {message}")]
pub struct McpError {
    pub kind: McpFailureKind,
    pub message: String,
}

impl McpError {
    pub fn new(kind: McpFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, McpError>;
