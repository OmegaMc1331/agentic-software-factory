pub mod agent;
mod capture;
pub mod diagnostics;
pub mod executable;

pub use agent::{
    AgentCapabilities, AgentConfig, AgentDiagnostics, AgentError, AgentKind, AgentRequest,
    AgentResult, AgentStatus, CommandAgent, OutputStream, ProcessInvocation, PromptTransport,
    MISSION_PLACEHOLDER,
};
pub use capture::{Utf8StreamDecoder, DEFAULT_AGENT_OUTPUT_LIMIT_BYTES};
pub use diagnostics::{
    invocation_summary, redacted_arguments, AgentDiagnostic, AgentDiagnosticCode, WITHHELD_ARGUMENT,
};
pub use executable::{
    resolve_executable, runtime_path_entries, BrokenExecutable, ExecutableResolution,
    LaunchCommand, ResolvedExecutable, ResolvedExecutableKind,
};
