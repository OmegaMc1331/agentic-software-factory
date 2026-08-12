use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: AgentCapabilities,
}

impl AgentConfig {
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        AgentConfig {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            capabilities: AgentCapabilities::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Available,
    Missing,
}

#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub mission: String,
    pub working_dir: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl AgentRequest {
    pub fn new(mission: impl Into<String>, working_dir: impl Into<PathBuf>) -> Self {
        AgentRequest {
            mission: mission.into(),
            working_dir: working_dir.into(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration: Duration,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("executable `{0}` not found; check that it is installed and on PATH")]
    ExecutableNotFound(String),
    #[error("failed to run `{0}`: {1}")]
    Spawn(String, String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct CommandAgent {
    config: AgentConfig,
}

impl CommandAgent {
    pub fn new(config: AgentConfig) -> Self {
        CommandAgent { config }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn command(&self) -> &str {
        &self.config.command
    }

    pub fn command_line(&self) -> String {
        let mut parts = vec![self.config.command.clone()];
        parts.extend(self.config.args.iter().cloned());
        parts.join(" ")
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    pub fn status(&self) -> AgentStatus {
        if executable_exists(&self.config.command) {
            AgentStatus::Available
        } else {
            AgentStatus::Missing
        }
    }

    pub fn available(&self) -> bool {
        matches!(self.status(), AgentStatus::Available)
    }

    pub fn run(&self, request: &AgentRequest) -> Result<AgentResult, AgentError> {
        if !executable_exists(&self.config.command) {
            return Err(AgentError::ExecutableNotFound(self.config.command.clone()));
        }
        let mut cmd = Command::new(&self.config.command);
        cmd.args(&self.config.args)
            .current_dir(&request.working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }
        for (key, value) in &request.env {
            cmd.env(key, value);
        }
        let started = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| AgentError::Spawn(self.config.command.clone(), e.to_string()))?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            AgentError::Spawn(self.config.command.clone(), "stdin not available".into())
        })?;
        let write_error = stdin.write_all(request.mission.as_bytes()).err();
        drop(stdin);
        let output = child.wait_with_output()?;
        let duration = started.elapsed();
        if let Some(err) = write_error {
            if err.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(AgentError::Io(err));
            }
        }
        Ok(AgentResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code(),
            duration,
        })
    }
}

fn executable_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }
    let path_value = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_value) {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            let pathext =
                std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
            if candidate.extension().is_none() {
                for ext in pathext.split(';').filter(|s| !s.is_empty()) {
                    let ext = ext.trim_start_matches('.');
                    if candidate.with_extension(ext).is_file() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let request = AgentRequest::new("mission", TempDir::new().unwrap().path());
        let err = agent.run(&request).unwrap_err();
        assert!(matches!(err, AgentError::ExecutableNotFound(_)));
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
}
