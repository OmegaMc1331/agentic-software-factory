use serde::Serialize;

pub type Result<T> = std::result::Result<T, IdeError>;

#[derive(Clone, Debug, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub enum IdeError {
    #[error("invalid IDE configuration or request")]
    Invalid,
    #[error("explicit trust approval is required")]
    Unapproved,
    #[error("operation is not permitted for this project or workspace root")]
    Forbidden,
    #[error("IDE resource not found in this project")]
    NotFound,
    #[error("IDE resource limit reached")]
    Limit,
    #[error("document or profile changed; review and retry")]
    Conflict,
    #[error("required executable is unavailable or has changed; approve its current version")]
    Executable,
    #[error("protocol framing or message is invalid")]
    Protocol,
    #[error("tool request timed out or was cancelled")]
    Timeout,
    #[error("tool request was cancelled")]
    Cancelled,
    #[error("tool exited or disconnected")]
    Exited,
    #[error("tool does not support this operation")]
    Unsupported,
    #[error("IDE metadata storage is unavailable")]
    Storage,
    #[error("tool refused request: {0}")]
    Tool(String),
    #[error("file service refused operation: {0}")]
    File(String),
}

impl From<factory_files::FileError> for IdeError {
    fn from(value: factory_files::FileError) -> Self {
        use factory_files::FileError;
        match value {
            FileError::Invalid => Self::Invalid,
            FileError::Forbidden | FileError::ReadOnly | FileError::Permission => Self::Forbidden,
            FileError::NotFound | FileError::UnknownRoot => Self::NotFound,
            FileError::Conflict | FileError::DraftConflict | FileError::Busy => Self::Conflict,
            FileError::Limit => Self::Limit,
            FileError::Unsupported => Self::Unsupported,
            other => Self::File(other.code().into()),
        }
    }
}

impl From<std::io::Error> for IdeError {
    fn from(_: std::io::Error) -> Self {
        Self::Exited
    }
}

impl From<rusqlite::Error> for IdeError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
