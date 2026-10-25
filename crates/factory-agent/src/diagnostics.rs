use serde::Serialize;

use crate::agent::{AgentError, AgentResult};

pub const WITHHELD_ARGUMENT: &str = "<value withheld>";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDiagnosticCode {
    Available,
    ExecutableNotFound,
    BrokenExecutable,
    InvalidInvocation,
    WorkflowTransportDisabled,
    InteractiveInvocationMissing,
    ProcessSpawnFailed,
    ExecutionFailed,
    InteractiveRequired,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDiagnostic {
    pub code: AgentDiagnosticCode,
    pub message: String,
    pub action: Option<String>,
}

impl AgentDiagnostic {
    pub fn new(
        code: AgentDiagnosticCode,
        message: impl Into<String>,
        action: Option<String>,
    ) -> Self {
        AgentDiagnostic {
            code,
            message: message.into(),
            action,
        }
    }

    pub fn available(message: impl Into<String>) -> Self {
        AgentDiagnostic::new(AgentDiagnosticCode::Available, message, None)
    }

    pub fn is_available(&self) -> bool {
        self.code == AgentDiagnosticCode::Available
    }
}

impl AgentDiagnosticCode {
    pub fn is_available(self) -> bool {
        self == AgentDiagnosticCode::Available
    }
}

pub fn redacted_arguments(args: &[String]) -> Vec<String> {
    args.iter().map(|_| WITHHELD_ARGUMENT.to_string()).collect()
}

pub fn invocation_summary(command: &str, args: &[String]) -> String {
    match args.len() {
        0 => command.to_string(),
        1 => format!("{command} (1 argument, values withheld)"),
        count => format!("{command} ({count} arguments, values withheld)"),
    }
}

impl AgentError {
    pub fn diagnostic(&self) -> AgentDiagnostic {
        match self {
            AgentError::ExecutableNotFound {
                command,
                path_entries,
            } => AgentDiagnostic::new(
                AgentDiagnosticCode::ExecutableNotFound,
                format!(
                    "`{command}` was not found in the PATH visible to Factory ({path_entries} entries checked)."
                ),
                Some(format!(
                    "Install `{command}` or add its directory to the PATH used to start Factory, then refresh agent availability."
                )),
            ),
            AgentError::InvalidExecutable {
                command,
                path,
                shim,
                reason,
            } => {
                let location = match shim {
                    Some(shim) => format!(
                        "`{command}` resolves through `{}` to `{}`, which cannot be launched",
                        shim.display(),
                        path.display()
                    ),
                    None => format!(
                        "`{command}` resolves to `{}`, which cannot be launched",
                        path.display()
                    ),
                };
                AgentDiagnostic::new(
                    AgentDiagnosticCode::BrokenExecutable,
                    format!("{location}: {reason}."),
                    Some(format!(
                        "Reinstall the CLI that provides `{command}` and verify it runs outside Factory."
                    )),
                )
            }
            AgentError::Spawn(command, reason) => AgentDiagnostic::new(
                AgentDiagnosticCode::ProcessSpawnFailed,
                format!("`{command}` could not be started: {reason}."),
                Some(
                    "Verify the executable, working directory, and operating-system permissions, then refresh agent availability."
                        .to_string(),
                ),
            ),
            AgentError::AutomatedUnavailable(name) => AgentDiagnostic::new(
                AgentDiagnosticCode::WorkflowTransportDisabled,
                format!("Workflow transport is disabled for `{name}`."),
                Some(
                    "Enable a prompt transport for workflow use or select another agent for automated roles."
                        .to_string(),
                ),
            ),
            AgentError::InteractiveUnavailable(name) => AgentDiagnostic::new(
                AgentDiagnosticCode::InteractiveInvocationMissing,
                format!("No interactive invocation is configured for `{name}`."),
                Some(
                    "Add interactive arguments in Settings if this agent supports interactive sessions."
                        .to_string(),
                ),
            ),
            AgentError::InvalidInvocation(name, reason) => AgentDiagnostic::new(
                AgentDiagnosticCode::InvalidInvocation,
                format!("The invocation for `{name}` is invalid: {reason}."),
                Some(
                    "Correct the configured arguments or prompt transport in Settings.".to_string(),
                ),
            ),
            AgentError::RequiresTerminal(name) => AgentDiagnostic::new(
                AgentDiagnosticCode::InteractiveRequired,
                format!("`{name}` started but requires an interactive terminal."),
                Some(format!(
                    "Configure a non-interactive workflow invocation for `{name}`."
                )),
            ),
            AgentError::ExecutionFailed { command, exit_code } => AgentDiagnostic::new(
                AgentDiagnosticCode::ExecutionFailed,
                format!("`{command}` started but exited with code {exit_code}."),
                Some(
                    "Inspect the agent session output; this is an execution failure, not an installation problem."
                        .to_string(),
                ),
            ),
            AgentError::Io(error) => AgentDiagnostic::new(
                AgentDiagnosticCode::Unknown,
                format!("The agent process could not be observed: {error}."),
                Some("Retry the session and inspect Factory logs if the problem persists.".to_string()),
            ),
        }
    }
}

impl AgentResult {
    pub fn diagnostic(&self) -> AgentDiagnostic {
        if self.cancelled {
            return AgentDiagnostic::new(
                AgentDiagnosticCode::Cancelled,
                "The agent process was cancelled.",
                None,
            );
        }
        match self.exit_code {
            Some(0) => AgentDiagnostic::available("The agent process completed successfully."),
            Some(code) => AgentDiagnostic::new(
                AgentDiagnosticCode::ExecutionFailed,
                format!("The agent process exited with code {code}."),
                Some(
                    "Inspect the agent session output; this is an execution failure, not an installation problem."
                        .to_string(),
                ),
            ),
            None => AgentDiagnostic::new(
                AgentDiagnosticCode::Unknown,
                "The agent process ended without an exit code.",
                Some("Inspect the agent session output and retry the session.".to_string()),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_argument_values_and_preserves_the_count() {
        let args = vec![
            "--model".to_string(),
            "openai/gpt-5".to_string(),
            "--api-key=secret".to_string(),
        ];
        assert_eq!(
            redacted_arguments(&args),
            vec![
                WITHHELD_ARGUMENT.to_string(),
                WITHHELD_ARGUMENT.to_string(),
                WITHHELD_ARGUMENT.to_string()
            ]
        );
        let summary = invocation_summary("codex", &args);
        assert_eq!(summary, "codex (3 arguments, values withheld)");
        assert!(!summary.contains("secret"));
        assert!(!summary.contains("openai/gpt-5"));
    }

    #[test]
    fn describes_missing_and_broken_executables_actionably() {
        let missing = AgentError::ExecutableNotFound {
            command: "ghost".to_string(),
            path_entries: 9,
        }
        .diagnostic();
        assert_eq!(missing.code, AgentDiagnosticCode::ExecutableNotFound);
        assert!(missing.message.contains("PATH visible to Factory"));
        assert!(missing.action.unwrap().contains("Install"));

        let broken = AgentError::InvalidExecutable {
            command: "fake".to_string(),
            path: "C:\\tools\\fake.exe".into(),
            shim: None,
            reason: "missing the DOS MZ header".to_string(),
        }
        .diagnostic();
        assert_eq!(broken.code, AgentDiagnosticCode::BrokenExecutable);
        assert!(broken.message.contains("cannot be launched"));
        assert!(broken.action.unwrap().contains("Reinstall"));
    }

    #[test]
    fn separates_configuration_spawn_and_execution_failures() {
        let disabled = AgentError::AutomatedUnavailable("agent".to_string()).diagnostic();
        assert_eq!(
            disabled.code,
            AgentDiagnosticCode::WorkflowTransportDisabled
        );

        let spawn =
            AgentError::Spawn("agent".to_string(), "permission denied".to_string()).diagnostic();
        assert_eq!(spawn.code, AgentDiagnosticCode::ProcessSpawnFailed);

        let execution = AgentError::ExecutionFailed {
            command: "agent".to_string(),
            exit_code: 7,
        }
        .diagnostic();
        assert_eq!(execution.code, AgentDiagnosticCode::ExecutionFailed);
        assert!(execution.message.contains("exited with code 7"));
    }
}
