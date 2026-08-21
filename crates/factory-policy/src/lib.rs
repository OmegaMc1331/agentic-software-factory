//! Factory's local policy engine.
//!
//! Policies are orchestration/security controls: they decide what a role or an
//! agent is *permitted* to do, they do not claim OS-level virtualization.
//! The engine is deliberately small — one resolver, a compact model, and few
//! enforcement points in Factory Core.

pub mod environment;
pub mod path;
pub mod policy;

pub use environment::{
    filter_configured_env, filter_environment, redact_secrets, secrets_from_denied, Secrets,
};
pub use policy::{
    operation_is_mutating, validate_executable, CommandsMode, CommandsPolicy, EffectiveCommands,
    EffectiveEnvironment, EffectiveFilesystem, EffectiveGit, EffectiveNetwork, EffectivePolicy,
    EnvironmentPolicy, FilesystemPolicy, GitOperation, GitPolicy, NetworkMode, NetworkPolicy,
    PoliciesConfig, PolicyPreset, PolicyScope, PolicyView,
};
