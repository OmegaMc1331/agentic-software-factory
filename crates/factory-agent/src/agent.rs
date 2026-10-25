use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use factory_types::AgentOutputMetadata;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::capture::{
    BoundedOutputCapture, Utf8StreamDecoder, DEFAULT_AGENT_OUTPUT_LIMIT_BYTES,
    OUTPUT_CHANNEL_CAPACITY, OUTPUT_READ_BUFFER_BYTES,
};
use crate::diagnostics::{invocation_summary, AgentDiagnostic};
use crate::executable::{
    resolve_executable, runtime_path_entries, ExecutableResolution, LaunchCommand,
    ResolvedExecutable,
};

pub const MISSION_PLACEHOLDER: &str = "{mission}";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    ClaudeCode,
    OpenCode,
    GeminiCli,
    QwenCode,
    #[default]
    Custom,
}

impl AgentKind {
    pub fn default_command(self) -> Option<&'static str> {
        match self {
            AgentKind::Codex => Some("codex"),
            AgentKind::ClaudeCode => Some("claude"),
            AgentKind::OpenCode => Some("opencode"),
            AgentKind::GeminiCli => Some("gemini"),
            AgentKind::QwenCode => Some("qwen"),
            AgentKind::Custom => None,
        }
    }

    pub fn workflow_args(self) -> &'static [&'static str] {
        match self {
            AgentKind::Codex => &["exec"],
            AgentKind::ClaudeCode | AgentKind::GeminiCli | AgentKind::QwenCode => &["-p"],
            AgentKind::OpenCode => &["run"],
            AgentKind::Custom => &[],
        }
    }

    pub fn prompt_transport(self) -> PromptTransport {
        match self {
            // Codex accepts a workflow mission from stdin when `exec` has no
            // positional prompt. Keeping the mission out of argv is required
            // for large Windows workflows, while the configured args remain
            // structured and unchanged.
            AgentKind::Codex | AgentKind::Custom => PromptTransport::Stdin,
            AgentKind::ClaudeCode
            | AgentKind::OpenCode
            | AgentKind::GeminiCli
            | AgentKind::QwenCode => PromptTransport::Argument,
        }
    }

    pub fn supports_interactive(self) -> bool {
        self != AgentKind::Custom
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptTransport {
    #[default]
    Stdin,
    Argument,
    Disabled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCapabilities {
    #[serde(default)]
    pub roles: Vec<String>,
}

impl AgentCapabilities {
    pub fn supports(&self, role: &str) -> bool {
        self.roles.is_empty() || self.roles.iter().any(|r| r == role)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub name: String,
    #[serde(default)]
    pub kind: AgentKind,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub prompt_transport: PromptTransport,
    #[serde(default)]
    pub interactive_args: Option<Vec<String>>,
    #[serde(default)]
    pub capabilities: AgentCapabilities,
}

impl AgentConfig {
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        AgentConfig {
            name: name.into(),
            kind: AgentKind::Custom,
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            prompt_transport: PromptTransport::Stdin,
            interactive_args: None,
            capabilities: AgentCapabilities::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInvocation {
    pub command: String,
    pub executable: ResolvedExecutable,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub working_dir: PathBuf,
    pub stdin_payload: Option<Vec<u8>>,
}

impl ProcessInvocation {
    pub fn invocation_summary(&self) -> String {
        invocation_summary(&self.command, &self.args)
    }

    pub fn process_launch(&self) -> Result<LaunchCommand, AgentError> {
        self.executable
            .process_launch(&self.args)
            .map_err(|reason| AgentError::InvalidInvocation(self.command.clone(), reason))
    }

    pub fn pty_launch(&self) -> Result<LaunchCommand, AgentError> {
        self.executable
            .pty_launch(&self.args)
            .map_err(|reason| AgentError::InvalidInvocation(self.command.clone(), reason))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Available,
    Missing,
    Broken,
}

#[derive(Debug)]
pub struct AgentDiagnostics {
    pub status: AgentStatus,
    pub resolution: Result<ResolvedExecutable, AgentError>,
    pub workflow: AgentDiagnostic,
    pub interactive: AgentDiagnostic,
}

#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub mission: String,
    pub working_dir: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Keys removed from the process environment even when configured on the
    /// agent or passed in `env` (deny wins over allow).
    pub env_deny: Vec<String>,
    /// When set, the child process environment is *replaced* by the computed
    /// one instead of inheriting Factory's full environment. Used by the
    /// policy engine's environment filtering; the caller must pass the exact
    /// environment in `env` in that case.
    pub clear_env: bool,
}

impl AgentRequest {
    pub fn new(mission: impl Into<String>, working_dir: impl Into<PathBuf>) -> Self {
        AgentRequest {
            mission: mission.into(),
            working_dir: working_dir.into(),
            env: BTreeMap::new(),
            env_deny: Vec::new(),
            clear_env: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration: Duration,
    pub cancelled: bool,
    pub output_metadata: AgentOutputMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("executable `{command}` was not found in the PATH visible to Factory ({path_entries} entries checked)")]
    ExecutableNotFound {
        command: String,
        path_entries: usize,
    },
    #[error("`{command}` was found, but its resolved Windows executable is invalid: {path} ({reason}). Reinstall the CLI and verify `{command} --version` outside Factory")]
    InvalidExecutable {
        command: String,
        path: PathBuf,
        shim: Option<PathBuf>,
        reason: String,
    },
    #[error("failed to run `{0}`: {1}")]
    Spawn(String, String),
    #[error("`{command}` started but exited with code {exit_code}")]
    ExecutionFailed { command: String, exit_code: i32 },
    #[error("Agent `{0}` has no non-interactive workflow invocation configured.")]
    AutomatedUnavailable(String),
    #[error("Agent `{0}` has no interactive invocation configured.")]
    InteractiveUnavailable(String),
    #[error("Invalid invocation for agent `{0}`: {1}")]
    InvalidInvocation(String, String),
    #[error("Agent `{0}` appears to require an interactive terminal. Configure a non-interactive workflow invocation for this agent.")]
    RequiresTerminal(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl AgentError {
    pub fn is_configuration(&self) -> bool {
        matches!(
            self,
            AgentError::ExecutableNotFound { .. }
                | AgentError::InvalidExecutable { .. }
                | AgentError::Spawn(_, _)
                | AgentError::AutomatedUnavailable(_)
                | AgentError::InteractiveUnavailable(_)
                | AgentError::InvalidInvocation(_, _)
                | AgentError::RequiresTerminal(_)
        )
    }
}

#[derive(Debug, Clone)]
pub struct CommandAgent {
    config: AgentConfig,
    output_limit: usize,
}

impl CommandAgent {
    pub fn new(config: AgentConfig) -> Self {
        CommandAgent {
            config,
            output_limit: DEFAULT_AGENT_OUTPUT_LIMIT_BYTES,
        }
    }

    pub fn with_output_limit(mut self, limit: usize) -> Self {
        self.output_limit = limit.max(1);
        self
    }

    pub fn output_limit(&self) -> usize {
        self.output_limit
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn command(&self) -> &str {
        &self.config.command
    }

    pub fn invocation_summary(&self) -> String {
        invocation_summary(&self.config.command, &self.config.args)
    }

    pub fn kind(&self) -> AgentKind {
        self.config.kind
    }

    pub fn prompt_transport(&self) -> PromptTransport {
        self.config.prompt_transport
    }

    fn automated_invocation_parts(
        &self,
        request: &AgentRequest,
    ) -> Result<(Vec<String>, Option<Vec<u8>>), AgentError> {
        if self.config.prompt_transport == PromptTransport::Disabled {
            return Err(AgentError::AutomatedUnavailable(self.config.name.clone()));
        }
        let mut args = self.config.args.clone();
        let stdin_payload = match self.config.prompt_transport {
            PromptTransport::Stdin => Some(request.mission.as_bytes().to_vec()),
            PromptTransport::Argument => {
                let placeholders = args
                    .iter()
                    .filter(|argument| argument.as_str() == MISSION_PLACEHOLDER)
                    .count();
                if placeholders > 1 {
                    return Err(AgentError::InvalidInvocation(
                        self.config.name.clone(),
                        "workflow arguments contain more than one {mission} placeholder".into(),
                    ));
                }
                if placeholders == 1 {
                    for argument in &mut args {
                        if argument == MISSION_PLACEHOLDER {
                            *argument = request.mission.clone();
                        }
                    }
                } else {
                    args.push(request.mission.clone());
                }
                None
            }
            PromptTransport::Disabled => unreachable!(),
        };
        Ok((args, stdin_payload))
    }

    fn build_invocation(
        &self,
        executable: ResolvedExecutable,
        args: Vec<String>,
        stdin_payload: Option<Vec<u8>>,
        working_dir: PathBuf,
        env: BTreeMap<String, String>,
    ) -> ProcessInvocation {
        ProcessInvocation {
            command: self.config.command.clone(),
            executable,
            args,
            env,
            working_dir,
            stdin_payload,
        }
    }

    pub fn automated_invocation(
        &self,
        request: &AgentRequest,
    ) -> Result<ProcessInvocation, AgentError> {
        let (args, stdin_payload) = self.automated_invocation_parts(request)?;
        let executable = self.resolve_executable()?;
        Ok(self.build_invocation(
            executable,
            args,
            stdin_payload,
            request.working_dir.clone(),
            merged_env(&self.config.env, &request.env, &request.env_deny),
        ))
    }

    fn interactive_args(&self) -> Result<Vec<String>, AgentError> {
        self.config
            .interactive_args
            .clone()
            .ok_or_else(|| AgentError::InteractiveUnavailable(self.config.name.clone()))
    }

    pub fn interactive_invocation(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> Result<ProcessInvocation, AgentError> {
        let args = self.interactive_args()?;
        let executable = self.resolve_executable()?;
        Ok(self.build_invocation(
            executable,
            args,
            None,
            working_dir.into(),
            self.config.env.clone(),
        ))
    }

    pub fn diagnose(&self) -> AgentDiagnostics {
        let resolution = self.resolve_executable();
        let workflow = match self.automated_invocation_parts(&AgentRequest::new("probe", ".")) {
            Err(error) => error.diagnostic(),
            Ok((args, stdin_payload)) => match &resolution {
                Ok(executable) => {
                    let invocation = self.build_invocation(
                        executable.clone(),
                        args,
                        stdin_payload,
                        PathBuf::from("."),
                        self.config.env.clone(),
                    );
                    match invocation.process_launch() {
                        Ok(_) => AgentDiagnostic::available(
                            "Workflow can be launched with the current configuration.",
                        ),
                        Err(error) => error.diagnostic(),
                    }
                }
                Err(error) => error.diagnostic(),
            },
        };
        let interactive = match self.interactive_args() {
            Err(error) => error.diagnostic(),
            Ok(args) => match &resolution {
                Ok(executable) => {
                    let invocation = self.build_invocation(
                        executable.clone(),
                        args,
                        None,
                        PathBuf::from("."),
                        self.config.env.clone(),
                    );
                    match invocation.pty_launch() {
                        Ok(_) => AgentDiagnostic::available(
                            "Interactive sessions can be launched with the current configuration.",
                        ),
                        Err(error) => error.diagnostic(),
                    }
                }
                Err(error) => error.diagnostic(),
            },
        };
        AgentDiagnostics {
            status: status_for(&resolution),
            resolution,
            workflow,
            interactive,
        }
    }

    pub fn workflow_available(&self) -> bool {
        self.diagnose().workflow.is_available()
    }

    pub fn interactive_available(&self) -> bool {
        self.diagnose().interactive.is_available()
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    pub fn status(&self) -> AgentStatus {
        status_for(&self.resolve_executable())
    }

    pub fn available(&self) -> bool {
        matches!(self.status(), AgentStatus::Available)
    }

    pub fn resolve_executable(&self) -> Result<ResolvedExecutable, AgentError> {
        match resolve_executable(&self.config.command) {
            ExecutableResolution::Resolved(resolved) => Ok(resolved),
            ExecutableResolution::NotFound => Err(AgentError::ExecutableNotFound {
                command: self.config.command.clone(),
                path_entries: runtime_path_entries(),
            }),
            ExecutableResolution::Broken(broken) => Err(AgentError::InvalidExecutable {
                command: self.config.command.clone(),
                path: broken.path,
                shim: broken.shim,
                reason: broken.reason,
            }),
        }
    }

    pub fn run(&self, request: &AgentRequest) -> Result<AgentResult, AgentError> {
        let cancel = AtomicBool::new(false);
        self.run_observed(request, &cancel, |_, _| {})
    }

    pub fn run_observed<F>(
        &self,
        request: &AgentRequest,
        cancel: &AtomicBool,
        mut on_output: F,
    ) -> Result<AgentResult, AgentError>
    where
        F: FnMut(OutputStream, &str),
    {
        let invocation = self.automated_invocation(request)?;
        let launch = invocation.process_launch()?;
        let mut cmd = Command::new(&launch.program);
        cmd.args(&launch.args)
            .current_dir(&invocation.working_dir)
            .stdin(if invocation.stdin_payload.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        if request.clear_env {
            // Policy-managed environment: the child gets exactly the computed
            // variables, not Factory's full inherited environment.
            cmd.env_clear();
        }
        for (key, value) in &invocation.env {
            cmd.env(key, value);
        }
        let started = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| spawn_error(&self.config.command, &invocation.working_dir, &e))?;
        let stdin = child.stdin.take();
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_and_reap(&mut child);
                return Err(AgentError::Spawn(
                    self.config.command.clone(),
                    "stdout not available".into(),
                ));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_and_reap(&mut child);
                return Err(AgentError::Spawn(
                    self.config.command.clone(),
                    "stderr not available".into(),
                ));
            }
        };
        if stdin.is_none() && invocation.stdin_payload.is_some() {
            terminate_and_reap(&mut child);
            return Err(AgentError::Spawn(
                self.config.command.clone(),
                "stdin not available".into(),
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let stdout_reader = output_reader(stdout, OutputStream::Stdout, sender.clone());
        let stderr_reader = output_reader(stderr, OutputStream::Stderr, sender);
        let stdin_writer = match (stdin, invocation.stdin_payload) {
            (Some(stdin), Some(payload)) => Some(spawn_stdin_writer(stdin, payload)),
            _ => None,
        };
        let mut stdout_capture = BoundedOutputCapture::new(self.output_limit);
        let mut stderr_capture = BoundedOutputCapture::new(self.output_limit);
        let mut stdout_decoder = Utf8StreamDecoder::default();
        let mut stderr_decoder = Utf8StreamDecoder::default();
        let mut stdout_forwarding = true;
        let mut stderr_forwarding = true;
        let mut handle_text = |stream: OutputStream, chunk: Option<&[u8]>, text: &str| {
            let (capture, forwarding) = match stream {
                OutputStream::Stdout => (&mut stdout_capture, &mut stdout_forwarding),
                OutputStream::Stderr => (&mut stderr_capture, &mut stderr_forwarding),
            };
            if let Some(chunk) = chunk {
                capture.push(chunk);
            }
            if *forwarding && !text.is_empty() {
                on_output(stream, text);
                if capture.reached_limit() {
                    *forwarding = false;
                }
            }
        };
        let mut was_cancelled = false;
        let status = loop {
            let mut received = false;
            for _ in 0..64 {
                let Ok((stream, chunk)) = receiver.try_recv() else {
                    break;
                };
                received = true;
                let text = match stream {
                    OutputStream::Stdout => stdout_decoder.push(&chunk),
                    OutputStream::Stderr => stderr_decoder.push(&chunk),
                };
                handle_text(stream, Some(&chunk), &text);
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
            }
            if cancel.load(std::sync::atomic::Ordering::Relaxed) && !was_cancelled {
                was_cancelled = true;
                terminate_process_tree(&mut child);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if !received {
                std::thread::sleep(Duration::from_millis(40));
            }
        };
        while let Ok((stream, chunk)) = receiver.recv() {
            let text = match stream {
                OutputStream::Stdout => stdout_decoder.push(&chunk),
                OutputStream::Stderr => stderr_decoder.push(&chunk),
            };
            handle_text(stream, Some(&chunk), &text);
        }
        let stdout_join = stdout_reader.join();
        let stderr_join = stderr_reader.join();
        let stdin_join = stdin_writer.map(|writer| writer.join());
        let stdout_error = match stdout_join {
            Ok(result) => result.err(),
            Err(_) => {
                return Err(AgentError::Spawn(
                    self.config.command.clone(),
                    "stdout reader stopped".into(),
                ));
            }
        };
        let stderr_error = match stderr_join {
            Ok(result) => result.err(),
            Err(_) => {
                return Err(AgentError::Spawn(
                    self.config.command.clone(),
                    "stderr reader stopped".into(),
                ));
            }
        };
        let stdin_error = match stdin_join {
            Some(Ok(result)) => result.err(),
            Some(Err(_)) => {
                return Err(AgentError::Spawn(
                    self.config.command.clone(),
                    "stdin writer stopped".into(),
                ));
            }
            None => None,
        };
        let stdout_tail = stdout_decoder.finish();
        handle_text(OutputStream::Stdout, None, &stdout_tail);
        let stderr_tail = stderr_decoder.finish();
        handle_text(OutputStream::Stderr, None, &stderr_tail);
        drop(handle_text);
        let stdout_capture = stdout_capture.finish();
        let stderr_capture = stderr_capture.finish();
        let (stdout_text, stderr_text, output_metadata) =
            stdout_capture.into_streams(stderr_capture);
        let duration = started.elapsed();
        if !was_cancelled {
            if let Some(err) = stdout_error.or(stderr_error) {
                return Err(AgentError::Io(err));
            }
            if let Some(err) = stdin_error {
                if err.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(AgentError::Io(err));
                }
            }
        }
        if !was_cancelled && !status.success() && terminal_required(&stdout_text, &stderr_text) {
            return Err(AgentError::RequiresTerminal(self.config.name.clone()));
        }
        Ok(AgentResult {
            stdout: stdout_text,
            stderr: stderr_text,
            exit_code: status.code(),
            duration,
            cancelled: was_cancelled,
            output_metadata,
        })
    }
}

fn status_for(resolution: &Result<ResolvedExecutable, AgentError>) -> AgentStatus {
    match resolution {
        Ok(_) => AgentStatus::Available,
        Err(AgentError::InvalidExecutable { .. }) => AgentStatus::Broken,
        Err(_) => AgentStatus::Missing,
    }
}

fn merged_env(
    configured: &BTreeMap<String, String>,
    request: &BTreeMap<String, String>,
    denied: &[String],
) -> BTreeMap<String, String> {
    let minimized: Vec<String> = denied.iter().map(|key| key.to_lowercase()).collect();
    let allowed = |key: &str| !minimized.contains(&key.to_lowercase());
    let mut env = BTreeMap::new();
    for (key, value) in configured {
        if allowed(key) {
            env.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in request {
        if allowed(key) {
            env.insert(key.clone(), value.clone());
        }
    }
    env
}

/// Translates process-creation failures into actionable Factory diagnostics
/// while preserving the original OS error. Windows error 193
/// (ERROR_BAD_EXE_FORMAT) and 216 (ERROR_EXE_MACHINE_TYPE_MISMATCH) mean the
/// resolved file is not a runnable executable for this environment.
fn spawn_error(command: &str, working_dir: &Path, error: &std::io::Error) -> AgentError {
    let incompatible = matches!(error.raw_os_error(), Some(193) | Some(216));
    if incompatible {
        return AgentError::Spawn(
            command.to_string(),
            format!(
                "the resolved executable is not compatible with this Windows environment \
                 (os error {}: {error}). Verify the agent directly with `{command} --version` \
                 outside Factory and reinstall it if the check fails",
                error.raw_os_error().unwrap_or_default()
            ),
        );
    }
    let directory = working_dir.display();
    let invalid_directory = matches!(error.raw_os_error(), Some(267));
    let reason = match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            format!("permission denied while starting the process in `{directory}` ({error})")
        }
        std::io::ErrorKind::NotFound => {
            format!("the executable or working directory `{directory}` was not found ({error})")
        }
        _ if invalid_directory => {
            format!("the working directory `{directory}` is invalid ({error})")
        }
        _ => error.to_string(),
    };
    AgentError::Spawn(command.to_string(), reason)
}

fn terminal_required(stdout: &str, stderr: &str) -> bool {
    let output = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    [
        "stdin is not a terminal",
        "stdin is not a tty",
        "the input device is not a tty",
        "input device is not a tty",
        "requires an interactive terminal",
        "requires a tty",
    ]
    .iter()
    .any(|message| output.contains(message))
}

fn terminate_and_reap(child: &mut std::process::Child) {
    terminate_process_tree(child);
    let _ = child.wait();
}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn spawn_stdin_writer(
    mut stdin: std::process::ChildStdin,
    payload: Vec<u8>,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::spawn(move || stdin.write_all(&payload))
}

fn output_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: OutputStream,
    sender: mpsc::SyncSender<(OutputStream, Vec<u8>)>,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; OUTPUT_READ_BUFFER_BYTES];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if read == 0 {
                break;
            }
            if sender.send((stream, buffer[..read].to_vec())).is_err() {
                break;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::AgentDiagnosticCode;
    use tempfile::TempDir;

    fn shell_args(kind: &str) -> (String, Vec<String>) {
        match kind {
            "echo" => {
                if cfg!(windows) {
                    (
                        "cmd".into(),
                        vec!["/c".into(), "echo".into(), "hello".into()],
                    )
                } else {
                    ("sh".into(), vec!["-c".into(), "echo hello".into()])
                }
            }
            "stderr" => {
                if cfg!(windows) {
                    (
                        "cmd".into(),
                        vec!["/c".into(), "echo".into(), "oops".into(), "1>&2".into()],
                    )
                } else {
                    ("sh".into(), vec!["-c".into(), "echo oops >&2".into()])
                }
            }
            "fail" => {
                if cfg!(windows) {
                    ("cmd".into(), vec!["/c".into(), "exit".into(), "3".into()])
                } else {
                    ("sh".into(), vec!["-c".into(), "exit 3".into()])
                }
            }
            "cat" => {
                if cfg!(windows) {
                    ("cmd".into(), vec!["/c".into(), "more".into()])
                } else {
                    ("sh".into(), vec!["-c".into(), "cat".into()])
                }
            }
            "sleep" => {
                if cfg!(windows) {
                    (
                        "cmd".into(),
                        vec!["/d".into(), "/c".into(), "ping -n 20 127.0.0.1 >nul".into()],
                    )
                } else {
                    ("sh".into(), vec!["-c".into(), "sleep 20".into()])
                }
            }
            "tty-error" => {
                if cfg!(windows) {
                    (
                        "cmd".into(),
                        vec![
                            "/d".into(),
                            "/c".into(),
                            "echo stdin is not a terminal 1>&2 & exit /b 1".into(),
                        ],
                    )
                } else {
                    (
                        "sh".into(),
                        vec![
                            "-c".into(),
                            "echo 'stdin is not a terminal' >&2; exit 1".into(),
                        ],
                    )
                }
            }
            _ => unreachable!(),
        }
    }

    fn agent(kind: &str) -> CommandAgent {
        let (command, args) = shell_args(kind);
        let mut cfg = AgentConfig::new("test", command);
        cfg.args = args;
        CommandAgent::new(cfg)
    }

    #[test]
    fn detects_missing_executable() {
        let agent = CommandAgent::new(AgentConfig::new(
            "ghost",
            "definitely-not-a-real-factory-test-binary",
        ));
        assert!(!agent.available());
        let diagnostics = agent.diagnose();
        assert_eq!(diagnostics.status, AgentStatus::Missing);
        assert_eq!(
            diagnostics.workflow.code,
            AgentDiagnosticCode::ExecutableNotFound
        );
        assert_eq!(
            diagnostics.interactive.code,
            AgentDiagnosticCode::InteractiveInvocationMissing
        );
        let request = AgentRequest::new("mission", TempDir::new().unwrap().path());
        let err = agent.run(&request).unwrap_err();
        assert!(matches!(err, AgentError::ExecutableNotFound { .. }));
        assert_eq!(
            err.diagnostic().code,
            AgentDiagnosticCode::ExecutableNotFound
        );
    }

    #[test]
    fn missing_working_directory_is_reported_as_a_spawn_failure() {
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let agent = CommandAgent::new(AgentConfig::new("test", command));
        let directory = TempDir::new().unwrap();
        let missing = directory.path().join("does-not-exist");
        let error = agent
            .run(&AgentRequest::new("", &missing))
            .expect_err("spawn must fail");
        assert!(matches!(error, AgentError::Spawn(_, _)));
        assert_eq!(
            error.diagnostic().code,
            AgentDiagnosticCode::ProcessSpawnFailed
        );
        assert!(error.to_string().contains("working directory"));
    }

    #[test]
    fn diagnoses_disabled_workflow_and_missing_interactive_invocation() {
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let mut workflow_only = AgentConfig::new("workflow-only", command);
        workflow_only.prompt_transport = PromptTransport::Disabled;
        workflow_only.interactive_args = Some(Vec::new());
        let disabled = CommandAgent::new(workflow_only).diagnose();
        assert_eq!(disabled.status, AgentStatus::Available);
        assert_eq!(
            disabled.workflow.code,
            AgentDiagnosticCode::WorkflowTransportDisabled
        );
        assert!(disabled.workflow.action.is_some());
        assert_eq!(disabled.interactive.code, AgentDiagnosticCode::Available);

        let interactive_only = CommandAgent::new(AgentConfig::new("interactive-only", command));
        let missing = interactive_only.diagnose();
        assert_eq!(missing.workflow.code, AgentDiagnosticCode::Available);
        assert_eq!(
            missing.interactive.code,
            AgentDiagnosticCode::InteractiveInvocationMissing
        );
        assert!(missing.interactive.action.is_some());
    }

    #[test]
    fn diagnoses_invalid_mission_placeholders_before_execution() {
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let mut config = AgentConfig::new("invalid", command);
        config.args = vec![
            MISSION_PLACEHOLDER.to_string(),
            MISSION_PLACEHOLDER.to_string(),
        ];
        config.prompt_transport = PromptTransport::Argument;
        config.interactive_args = Some(Vec::new());
        let agent = CommandAgent::new(config);
        let diagnostics = agent.diagnose();
        assert_eq!(diagnostics.status, AgentStatus::Available);
        assert_eq!(
            diagnostics.workflow.code,
            AgentDiagnosticCode::InvalidInvocation
        );
        assert!(diagnostics.workflow.message.contains("placeholder"));
        assert_eq!(diagnostics.interactive.code, AgentDiagnosticCode::Available);
    }

    #[test]
    fn invocation_summaries_withhold_configured_argument_values() {
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let mut config = AgentConfig::new("secret", command);
        config.args = vec!["--api-key".into(), "super-secret-token".into()];
        let agent = CommandAgent::new(config);
        assert!(!agent.invocation_summary().contains("super-secret-token"));
        let invocation = agent
            .automated_invocation(&AgentRequest::new("", "."))
            .unwrap();
        assert!(!invocation
            .invocation_summary()
            .contains("super-secret-token"));
        assert!(invocation.invocation_summary().contains("values withheld"));
    }

    #[cfg(windows)]
    #[test]
    fn an_invalid_placeholder_exe_is_broken_not_available() {
        let directory = TempDir::new().unwrap();
        let fake = directory.path().join("fake-agent.exe");
        std::fs::write(
            &fake,
            "#!/bin/sh\nthis file is a placeholder, not an executable\n",
        )
        .unwrap();
        let agent = CommandAgent::new(AgentConfig::new(
            "fake",
            fake.to_string_lossy().into_owned(),
        ));
        assert_eq!(agent.status(), AgentStatus::Broken);
        assert!(!agent.available());
        assert!(!agent.workflow_available());
        let diagnostics = agent.diagnose();
        assert_eq!(
            diagnostics.workflow.code,
            AgentDiagnosticCode::BrokenExecutable
        );
        assert!(diagnostics
            .workflow
            .action
            .as_deref()
            .unwrap()
            .contains("Reinstall"));
        let error = agent
            .run(&AgentRequest::new("mission", directory.path()))
            .unwrap_err();
        assert!(matches!(error, AgentError::InvalidExecutable { .. }));
        assert!(error.to_string().contains("invalid"));
        assert!(!error.to_string().contains("216"));
    }

    #[cfg(windows)]
    #[test]
    fn unsafe_batch_arguments_make_workflow_availability_false_without_spawning() {
        let directory = TempDir::new().unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        std::fs::write(&shim, "@echo off\r\nexit /b 0\r\n").unwrap();
        let mut config = AgentConfig::new("fake", shim.to_string_lossy().into_owned());
        config.args = vec!["100%super-secret-token".into()];
        config.interactive_args = Some(Vec::new());
        let agent = CommandAgent::new(config);
        assert!(agent.available());
        let diagnostics = agent.diagnose();
        assert_eq!(diagnostics.status, AgentStatus::Available);
        assert_eq!(
            diagnostics.workflow.code,
            AgentDiagnosticCode::InvalidInvocation
        );
        assert!(diagnostics.workflow.message.contains("unsafe"));
        assert!(!diagnostics.workflow.message.contains("super-secret-token"));
        assert!(!agent.workflow_available());
    }

    #[cfg(windows)]
    #[test]
    fn a_broken_npm_shim_reports_an_invalid_executable_diagnostic() {
        let directory = TempDir::new().unwrap();
        let target = directory.path().join("fake-package/bin/fake-agent.exe");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "text placeholder shipped by a broken package\n").unwrap();
        let shim_path = directory.path().join("fake-agent.cmd");
        std::fs::write(
            &shim_path,
            "@ECHO off\r\n\"%dp0%\\fake-package\\bin\\fake-agent.exe\" %*\r\n",
        )
        .unwrap();

        let agent = CommandAgent::new(AgentConfig::new(
            "fake",
            shim_path.to_string_lossy().into_owned(),
        ));
        assert_eq!(agent.status(), AgentStatus::Broken);
        assert!(!agent.available());
        assert_eq!(
            agent.diagnose().workflow.code,
            AgentDiagnosticCode::BrokenExecutable
        );
        let error = agent
            .run(&AgentRequest::new("mission", directory.path()))
            .unwrap_err();
        match &error {
            AgentError::InvalidExecutable {
                command,
                path,
                shim,
                reason,
            } => {
                assert_eq!(command, &shim_path.to_string_lossy().into_owned());
                assert_eq!(path, &target);
                assert_eq!(shim.as_deref(), Some(shim_path.as_path()));
                assert!(reason.contains("Windows executable"));
            }
            other => panic!("expected InvalidExecutable, got {other:?}"),
        }
        assert!(error.to_string().contains("Reinstall the CLI"));
        assert!(!error.to_string().contains("216"));
    }

    #[test]
    fn runs_successfully_and_captures_stdout() {
        let dir = TempDir::new().unwrap();
        let result = agent("echo")
            .run(&AgentRequest::new("mission", dir.path()))
            .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("hello"));
        assert!(result.stderr.is_empty());
    }

    #[test]
    fn captures_stderr() {
        let dir = TempDir::new().unwrap();
        let result = agent("stderr")
            .run(&AgentRequest::new("mission", dir.path()))
            .unwrap();
        assert!(result.stderr.contains("oops"));
    }

    #[test]
    fn reports_a_failed_exit_status() {
        let dir = TempDir::new().unwrap();
        let result = agent("fail")
            .run(&AgentRequest::new("mission", dir.path()))
            .unwrap();
        assert_eq!(result.exit_code, Some(3));
        let diagnostic = result.diagnostic();
        assert_eq!(diagnostic.code, AgentDiagnosticCode::ExecutionFailed);
        assert!(diagnostic.message.contains("code 3"));
    }

    #[test]
    fn delivers_the_mission_through_stdin() {
        let dir = TempDir::new().unwrap();
        let result = agent("cat")
            .run(&AgentRequest::new("hello mission", dir.path()))
            .unwrap();
        assert!(result.stdout.contains("hello mission"));
    }

    #[test]
    fn delivers_the_mission_as_one_process_argument() {
        let dir = TempDir::new().unwrap();
        let (command, args) = if cfg!(windows) {
            let script = dir.path().join("print-mission.ps1");
            std::fs::write(
                &script,
                "param([string]$mission)\n[Console]::Out.Write($mission)\n",
            )
            .unwrap();
            (
                "powershell".to_string(),
                vec![
                    "-NoProfile".into(),
                    "-File".into(),
                    script.to_string_lossy().into_owned(),
                    MISSION_PLACEHOLDER.into(),
                ],
            )
        } else {
            (
                "sh".to_string(),
                vec![
                    "-c".into(),
                    "printf %s \"$1\"".into(),
                    "factory-test".into(),
                    MISSION_PLACEHOLDER.into(),
                ],
            )
        };
        let mut config = AgentConfig::new("argument-test", command);
        config.args = args;
        config.prompt_transport = PromptTransport::Argument;
        let result = CommandAgent::new(config)
            .run(&AgentRequest::new("mission with spaces", dir.path()))
            .unwrap();
        assert_eq!(result.stdout, "mission with spaces");
    }

    #[test]
    fn builds_zero_and_one_argument_invocations_without_splitting_values() {
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let mut zero = AgentConfig::new("zero", command);
        zero.prompt_transport = PromptTransport::Argument;
        let zero_invocation = CommandAgent::new(zero)
            .automated_invocation(&AgentRequest::new("mission", "."))
            .unwrap();
        assert_eq!(zero_invocation.args, vec!["mission"]);

        let mut one = AgentConfig::new("one", command);
        one.args = vec!["--model openai/gpt-5".into()];
        one.prompt_transport = PromptTransport::Argument;
        let one_invocation = CommandAgent::new(one)
            .automated_invocation(&AgentRequest::new("mission", "."))
            .unwrap();
        assert_eq!(one_invocation.args, vec!["--model openai/gpt-5", "mission"]);
    }

    #[test]
    fn preserves_order_and_replaces_a_mission_placeholder_among_arguments() {
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let mut config = AgentConfig::new("structured", command);
        config.args = vec![
            "run".into(),
            "--model".into(),
            "openai/gpt-5".into(),
            MISSION_PLACEHOLDER.into(),
            "--label hello world".into(),
        ];
        config.prompt_transport = PromptTransport::Argument;
        let invocation = CommandAgent::new(config)
            .automated_invocation(&AgentRequest::new("mission with spaces", "."))
            .unwrap();

        assert_eq!(
            invocation.args,
            vec![
                "run",
                "--model",
                "openai/gpt-5",
                "mission with spaces",
                "--label hello world"
            ]
        );
    }

    #[test]
    fn rejects_automated_use_when_workflow_transport_is_disabled() {
        let mut config = AgentConfig::new("interactive-only", "unused");
        config.prompt_transport = PromptTransport::Disabled;
        config.interactive_args = Some(Vec::new());
        let error = CommandAgent::new(config)
            .automated_invocation(&AgentRequest::new("mission", "."))
            .unwrap_err();
        assert!(matches!(error, AgentError::AutomatedUnavailable(_)));
    }

    #[test]
    fn known_profiles_define_non_interactive_invocations() {
        for (kind, command, prefix) in [
            (AgentKind::Codex, "codex", vec!["exec"]),
            (AgentKind::ClaudeCode, "claude", vec!["-p"]),
            (AgentKind::OpenCode, "opencode", vec!["run"]),
            (AgentKind::GeminiCli, "gemini", vec!["-p"]),
            (AgentKind::QwenCode, "qwen", vec!["-p"]),
        ] {
            let config = AgentConfig {
                name: command.into(),
                kind,
                command: command.into(),
                args: prefix.iter().map(|arg| (*arg).into()).collect(),
                env: BTreeMap::new(),
                prompt_transport: kind.prompt_transport(),
                interactive_args: Some(Vec::new()),
                capabilities: AgentCapabilities::default(),
            };
            assert_eq!(config.command, command);
            assert_eq!(config.args, prefix);
            assert_eq!(config.prompt_transport, kind.prompt_transport());
        }
    }

    #[test]
    fn codex_mission_uses_stdin_and_is_absent_from_argv() {
        let mission = "large codex mission";
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let config = AgentConfig {
            name: "codex".into(),
            kind: AgentKind::Codex,
            command: command.into(),
            args: vec![
                "exec".into(),
                "--model".into(),
                "openai/gpt-5".into(),
                "--sandbox".into(),
                "workspace-write".into(),
            ],
            env: BTreeMap::new(),
            prompt_transport: AgentKind::Codex.prompt_transport(),
            interactive_args: Some(Vec::new()),
            capabilities: AgentCapabilities::default(),
        };

        let invocation = CommandAgent::new(config)
            .automated_invocation(&AgentRequest::new(mission, "."))
            .unwrap();

        assert_eq!(
            invocation.args,
            vec![
                "exec",
                "--model",
                "openai/gpt-5",
                "--sandbox",
                "workspace-write"
            ]
        );
        assert!(!invocation.args.iter().any(|argument| argument == mission));
        assert_eq!(invocation.stdin_payload, Some(mission.as_bytes().to_vec()));
    }

    #[test]
    fn very_large_codex_mission_stays_out_of_argv() {
        let mission = "mission context ".repeat(100_000);
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        let config = AgentConfig {
            name: "codex".into(),
            kind: AgentKind::Codex,
            command: command.into(),
            args: vec!["exec".into(), "--model".into(), "openai/gpt-5".into()],
            env: BTreeMap::new(),
            prompt_transport: AgentKind::Codex.prompt_transport(),
            interactive_args: Some(Vec::new()),
            capabilities: AgentCapabilities::default(),
        };

        let invocation = CommandAgent::new(config)
            .automated_invocation(&AgentRequest::new(&mission, "."))
            .unwrap();

        assert!(mission.len() > 32_768);
        assert_eq!(invocation.args, vec!["exec", "--model", "openai/gpt-5"]);
        assert_eq!(
            invocation.stdin_payload.as_deref(),
            Some(mission.as_bytes())
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_cmd_found_through_path_is_the_executable_that_runs() {
        const CHILD: &str = "FACTORY_WINDOWS_CMD_RESOLUTION_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let mut config = AgentConfig::new("fake-agent", "fake-agent");
            config.prompt_transport = PromptTransport::Argument;
            let agent = CommandAgent::new(config);
            assert_eq!(agent.status(), AgentStatus::Available);
            let invocation = agent
                .automated_invocation(&AgentRequest::new("mission-value", "."))
                .unwrap();
            assert!(invocation
                .executable
                .path()
                .to_string_lossy()
                .ends_with("fake-agent.cmd"));
            let result = agent.run(&AgentRequest::new("mission-value", ".")).unwrap();
            assert!(result.stdout.contains("ARG=mission-value"));
            return;
        }

        let directory = TempDir::new().unwrap();
        std::fs::write(
            directory.path().join("fake-agent.cmd"),
            "@echo off\r\necho ARG=%~1\r\n",
        )
        .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "agent::tests::windows_cmd_found_through_path_is_the_executable_that_runs",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("PATH", directory.path())
            .env("PATHEXT", ".CMD;.EXE")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_codex_invocation_reads_a_mission_from_stdin() {
        const CHILD: &str = "FACTORY_WINDOWS_CODEX_STDIN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let config = AgentConfig {
                name: "codex".into(),
                kind: AgentKind::Codex,
                command: "codex".into(),
                args: vec!["exec".into(), "--model".into(), "test-model".into()],
                env: BTreeMap::new(),
                prompt_transport: AgentKind::Codex.prompt_transport(),
                interactive_args: Some(Vec::new()),
                capabilities: AgentCapabilities::default(),
            };
            let result = CommandAgent::new(config)
                .run(&AgentRequest::new("windows codex mission", "."))
                .unwrap();
            assert_eq!(result.exit_code, Some(0));
            assert!(result.stdout.contains("windows codex mission"));
            return;
        }

        let directory = TempDir::new().unwrap();
        std::fs::write(
            directory.path().join("codex.cmd"),
            "@echo off\r\nif /I not \"%~1\"==\"exec\" exit /b 2\r\nif /I not \"%~2\"==\"--model\" exit /b 3\r\nif /I not \"%~3\"==\"test-model\" exit /b 4\r\nif not \"%~4\"==\"\" exit /b 5\r\nset /p MISSION=\r\necho(%MISSION%\r\n",
        )
        .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "agent::tests::windows_codex_invocation_reads_a_mission_from_stdin",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("PATH", directory.path())
            .env("PATHEXT", ".CMD;.EXE")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn classifies_terminal_mode_failures() {
        let dir = TempDir::new().unwrap();
        let error = agent("tty-error")
            .run(&AgentRequest::new("mission", dir.path()))
            .unwrap_err();
        assert!(matches!(error, AgentError::RequiresTerminal(_)));
        assert!(error.is_configuration());
        assert_eq!(
            error.diagnostic().code,
            AgentDiagnosticCode::InteractiveRequired
        );
    }

    #[test]
    fn runs_in_the_requested_working_directory() {
        let dir = TempDir::new().unwrap();
        let agent = {
            let (command, args) = if cfg!(windows) {
                (
                    "powershell".to_string(),
                    vec![
                        "-NoProfile".into(),
                        "-Command".into(),
                        "(Get-Location).Path".into(),
                    ],
                )
            } else {
                ("pwd".to_string(), vec![])
            };
            let mut cfg = AgentConfig::new("pwd", command);
            cfg.args = args;
            CommandAgent::new(cfg)
        };
        let cwd = std::fs::canonicalize(dir.path()).unwrap();
        let cwd_str = cwd.display().to_string();
        let cwd_str = cwd_str
            .strip_prefix("\\\\?\\")
            .unwrap_or(&cwd_str)
            .to_string();
        let result = agent.run(&AgentRequest::new("", &cwd)).unwrap();
        assert!(result
            .stdout
            .to_lowercase()
            .contains(&cwd_str.to_lowercase()));
    }

    #[test]
    fn cancellation_terminates_the_specific_agent_process() {
        let dir = TempDir::new().unwrap();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            signal.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let started = Instant::now();
        let result = agent("sleep")
            .run_observed(&AgentRequest::new("", dir.path()), &cancel, |_, _| {})
            .unwrap();
        setter.join().unwrap();
        assert!(result.cancelled);
        assert_eq!(result.diagnostic().code, AgentDiagnosticCode::Cancelled);
        // The uncancelled agent would run for ~20s (ping -n 20 / sleep 20);
        // a generous budget keeps this stable on loaded CI runners while
        // still proving the process tree was terminated early.
        assert!(started.elapsed() < Duration::from_secs(15));
    }
}
