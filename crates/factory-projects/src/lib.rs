mod identity;
mod manager;
mod registry;

pub use identity::{validate_factory_paths, DirectoryIdentity, ProjectPathPolicy};
pub use manager::{ProjectContext, ProjectLimits, ProjectManager, ProjectView};
pub use registry::{application_registry_path, ProjectEntry, ProjectId, ProjectRegistry};

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("invalid project id")]
    InvalidId,
    #[error("unknown project")]
    NotFound,
    #[error("project path is outside the authorized locations")]
    Forbidden,
    #[error("invalid project: {0}")]
    Invalid(String),
    #[error("project conflict: {0}")]
    Conflict(String),
    #[error("project is not open")]
    Closed,
    #[error("project resource limit reached: {0}")]
    Limit(&'static str),
    #[error("project unavailable: {0}")]
    Unavailable(String),
    #[error("registry error; original data retained: {0}")]
    Registry(String),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Runtime(Box<factory_runtime::RuntimeError>),
    #[error(transparent)]
    Factory(Box<factory_core::FactoryError>),
}

pub type Result<T> = std::result::Result<T, ProjectError>;

impl From<factory_runtime::RuntimeError> for ProjectError {
    fn from(error: factory_runtime::RuntimeError) -> Self {
        Self::Runtime(Box::new(error))
    }
}
impl From<factory_core::FactoryError> for ProjectError {
    fn from(error: factory_core::FactoryError) -> Self {
        Self::Factory(Box::new(error))
    }
}
