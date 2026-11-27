#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("Invalid file reference or operation.")]
    Invalid,
    #[error("The selected repository or worktree root is unavailable.")]
    UnknownRoot,
    #[error("This path is protected, linked, or outside the selected root.")]
    Forbidden,
    #[error("This worktree is available for read-only inspection.")]
    ReadOnly,
    #[error("The file or directory no longer exists.")]
    NotFound,
    #[error("The destination already exists; nothing was overwritten.")]
    Exists,
    #[error("The file changed. Compare the current disk version before saving again.")]
    Conflict,
    #[error("The operation exceeds the configured file-service limit.")]
    Limit,
    #[error("The file or service is busy. Retry after the other operation finishes.")]
    Busy,
    #[error("The filesystem denied this operation. The local draft is retained.")]
    Permission,
    #[error("This content cannot be edited safely as UTF-8 text.")]
    Unsupported,
    #[error("Only empty directories can be deleted by this operation.")]
    DirectoryNotEmpty,
    #[error("The write outcome needs reconciliation. Reload and compare before retrying; recovery data is retained.")]
    Uncertain,
    #[error("Editor recovery storage is unavailable or invalid. Existing records were not reset.")]
    Storage,
    #[error("The recovery record changed in another window. Keep an independent draft.")]
    DraftConflict,
    #[error("The requested Git version is unavailable.")]
    GitUnavailable,
    #[error("The filesystem operation failed. No success was acknowledged.")]
    Io,
}

impl FileError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "invalid_file_reference",
            Self::UnknownRoot => "unknown_workspace_root",
            Self::Forbidden => "file_path_forbidden",
            Self::ReadOnly => "worktree_read_only",
            Self::NotFound => "file_not_found",
            Self::Exists => "file_exists",
            Self::Conflict => "file_revision_conflict",
            Self::Limit => "file_limit",
            Self::Busy => "file_resource_busy",
            Self::Permission => "file_access_denied",
            Self::Unsupported => "unsupported_file_content",
            Self::DirectoryNotEmpty => "directory_not_empty",
            Self::Uncertain => "file_write_uncertain",
            Self::Storage => "editor_storage_unavailable",
            Self::DraftConflict => "draft_revision_conflict",
            Self::GitUnavailable => "git_version_unavailable",
            Self::Io => "filesystem_error",
        }
    }
}

impl From<std::io::Error> for FileError {
    fn from(error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        match error.kind() {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::AlreadyExists => Self::Exists,
            ErrorKind::PermissionDenied => Self::Permission,
            ErrorKind::WouldBlock => Self::Busy,
            ErrorKind::DirectoryNotEmpty => Self::DirectoryNotEmpty,
            _ if matches!(error.raw_os_error(), Some(32 | 33)) && cfg!(windows) => Self::Busy,
            _ => Self::Io,
        }
    }
}

pub type Result<T> = std::result::Result<T, FileError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileReadPhase {
    Reference,
    Parent,
    Open,
    Canonicalize,
    Containment,
    Snapshot,
}

impl FileReadPhase {
    pub fn code(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Parent => "parent",
            Self::Open => "open",
            Self::Canonicalize => "canonicalize",
            Self::Containment => "containment",
            Self::Snapshot => "snapshot",
        }
    }
}

#[derive(Debug)]
pub struct FileReadFailure {
    pub phase: FileReadPhase,
    pub error: FileError,
}

impl FileReadFailure {
    pub(crate) fn at(phase: FileReadPhase) -> impl FnOnce(FileError) -> Self {
        move |error| Self { phase, error }
    }
}
