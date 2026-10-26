//! Bounded subprocess supervision for Factory-owned Git and GitHub commands.
//!
//! Every external command Factory runs on its own behalf (never agent
//! processes, which have their own supervisor in `factory-agent`) goes through
//! [`run_bounded`]: a fixed deadline, concurrent stdout/stderr draining with
//! hard capture limits, a null stdin so nothing waits for terminal input, and
//! process-tree termination when the deadline or a cancellation flag fires.
//! The primitive is deliberately small — structured arguments only, no shell,
//! no environment inheritance policies — and owns no knowledge of git or gh.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How large each read from a pipe is before being handed to the capture.
const READ_BUFFER_BYTES: usize = 8192;

/// How often the supervisor polls the child and the cancellation flag.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A command to run: program, arguments, working directory, and extra
/// environment entries layered on top of the inherited environment.
#[derive(Debug, Clone, Default)]
pub struct ProcCommand {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub current_dir: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
}

impl ProcCommand {
    pub fn new(program: impl Into<OsString>) -> Self {
        ProcCommand {
            program: program.into(),
            args: Vec::new(),
            current_dir: None,
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, value: impl Into<OsString>) -> Self {
        self.args.push(value.into());
        self
    }

    pub fn args<I, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<OsString>,
    {
        self.args.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Runs the command under `bounds` with no cancellation flag.
    pub fn run(&self, bounds: Bounds) -> Result<ProcOutput, ProcError> {
        self.run_with_cancel(bounds, None)
    }

    /// Runs the command under `bounds`, terminating the process tree when the
    /// deadline passes or `cancel` (when provided) becomes set.
    pub fn run_with_cancel(
        &self,
        bounds: Bounds,
        cancel: Option<&AtomicBool>,
    ) -> Result<ProcOutput, ProcError> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        // Kill the whole tree, not just the launcher: git and gh spawn helper
        // processes whose survival would outlive the deadline.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(ProcError::Spawn)?;
        let stdout_reader = child.stdout.take().expect("stdout piped before spawn");
        let stderr_reader = child.stderr.take().expect("stderr piped before spawn");
        let stdout_handle = spawn_bounded_reader(stdout_reader, bounds.stdout_limit);
        let stderr_handle = spawn_bounded_reader(stderr_reader, bounds.stderr_limit);

        let started = Instant::now();
        let mut timed_out = false;
        let mut cancelled = false;
        let mut status = None;
        loop {
            match child.try_wait() {
                Ok(Some(exited)) => {
                    status = Some(exited);
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    terminate_process_tree(&mut child);
                    return Err(ProcError::Io(error));
                }
            }
            if started.elapsed() >= bounds.timeout {
                timed_out = true;
                break;
            }
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                cancelled = true;
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        if timed_out || cancelled {
            terminate_process_tree(&mut child);
        }
        // Reap so no zombie remains, then drain the readers to EOF (the pipes
        // close once the tree is dead). The reaped status of a killed tree is
        // reported as-is; callers treat timeout/cancel as the outcome, not the
        // exit code.
        let reaped = child.wait().ok();
        let stdout = stdout_handle.join().unwrap_or_default();
        let stderr = stderr_handle.join().unwrap_or_default();
        let output = ProcOutput {
            status: status.or(reaped).expect("child reaped with a status"),
            stdout: stdout.captured,
            stderr: stderr.captured,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        };
        if timed_out {
            return Err(ProcError::TimedOut { output });
        }
        if cancelled {
            return Err(ProcError::Cancelled { output });
        }
        Ok(output)
    }
}

/// Deadline and capture limits for one supervised run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub timeout: Duration,
    /// Maximum stdout bytes retained (the head of the stream). Anything past
    /// the limit is counted and dropped, and reported as truncated.
    pub stdout_limit: usize,
    pub stderr_limit: usize,
}

impl Bounds {
    pub fn new(timeout: Duration, stdout_limit: usize, stderr_limit: usize) -> Self {
        Bounds {
            timeout,
            stdout_limit,
            stderr_limit,
        }
    }
}

/// Result of a completed (not timed out, not cancelled) run. Truncation flags
/// tell callers whether the captured bytes are the complete stream.
#[derive(Debug, Clone)]
pub struct ProcOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl ProcOutput {
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// Combined stdout+stderr as lossy UTF-8 — for error classification only.
    pub fn combined_text(&self) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        )
    }
}

#[derive(Debug)]
pub enum ProcError {
    /// The process could not be started.
    Spawn(std::io::Error),
    /// Waiting on or draining the process failed.
    Io(std::io::Error),
    /// The deadline passed; the process tree was terminated. The partial
    /// output captured before the kill is included — callers must treat any
    /// external side effect as uncertain, never as a definitive failure.
    TimedOut { output: ProcOutput },
    /// The cancellation flag fired; the process tree was terminated.
    Cancelled { output: ProcOutput },
}

#[derive(Default)]
struct Captured {
    captured: Vec<u8>,
    truncated: bool,
}

fn spawn_bounded_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> std::thread::JoinHandle<Captured> {
    std::thread::spawn(move || {
        let mut result = Captured {
            captured: Vec::new(),
            truncated: false,
        };
        let mut buffer = [0_u8; READ_BUFFER_BYTES];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return result,
                Ok(read) => {
                    let room = limit.saturating_sub(result.captured.len());
                    if read > room {
                        result.truncated = true;
                    }
                    result.captured.extend_from_slice(&buffer[..room.min(read)]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return result,
            }
        }
    })
}

/// Terminates a process and every descendant it spawned. Best effort: called
/// only after a timeout or cancellation, when leaving the tree running would
/// be strictly worse than an imperfect kill.
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
        // The child leads its own process group (set at spawn), so signaling
        // the negative pid reaches the whole tree.
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct Fixture {
        _dir: tempfile::TempDir,
        dir: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::TempDir::new().unwrap();
        Fixture {
            dir: dir.path().to_path_buf(),
            _dir: dir,
        }
    }

    fn script(dir: &Path, name: &str, windows: &str, unix: &str) -> OsString {
        if cfg!(windows) {
            let path = dir.join(format!("{name}.cmd"));
            std::fs::write(&path, windows).unwrap();
            path.into_os_string()
        } else {
            let path = dir.join(format!("{name}.sh"));
            std::fs::write(&path, unix).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&path).unwrap().permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(&path, permissions).unwrap();
            }
            path.into_os_string()
        }
    }

    fn fast_bounds() -> Bounds {
        Bounds::new(Duration::from_secs(10), 64 * 1024, 64 * 1024)
    }

    #[test]
    fn captures_successful_output_and_exit_status() {
        let fixture = fixture();
        let program = script(
            &fixture.dir,
            "ok",
            "@echo off\r\necho hello\r\necho oops 1>&2\r\nexit /b 3\r\n",
            "#!/bin/sh\necho hello\necho oops >&2\nexit 3\n",
        );
        let output = ProcCommand::new(program).run(fast_bounds()).unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "oops");
        assert!(!output.stdout_truncated && !output.stderr_truncated);
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        let error = ProcCommand::new("definitely-not-a-real-program-xyz")
            .run(fast_bounds())
            .unwrap_err();
        assert!(matches!(error, ProcError::Spawn(_)));
    }

    #[test]
    fn a_command_that_never_exits_is_killed_at_the_deadline() {
        let fixture = fixture();
        let program = script(
            &fixture.dir,
            "hang",
            "@echo off\r\necho started\r\nping -n 60 127.0.0.1 >nul\r\n",
            "#!/bin/sh\necho started\nsleep 60\n",
        );
        let started = Instant::now();
        let error = ProcCommand::new(program)
            .run(Bounds::new(Duration::from_millis(400), 1024, 1024))
            .unwrap_err();
        let ProcError::TimedOut { output } = error else {
            panic!("expected a timeout, got {error:?}");
        };
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("started"),
            "partial output captured before the kill: {:?}",
            output.stdout
        );
    }

    #[test]
    fn unbounded_stdout_is_captured_up_to_the_limit_and_flagged() {
        let fixture = fixture();
        // ~80 KB of output against a 4 KB capture limit.
        let program = script(
            &fixture.dir,
            "flood",
            "@echo off\r\nfor /l %%i in (1,1,2000) do echo 0123456789012345678901234567890123456789\r\n",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 2000 ]; do echo 0123456789012345678901234567890123456789; i=$((i+1)); done\n",
        );
        let output = ProcCommand::new(program)
            .run(Bounds::new(Duration::from_secs(20), 4096, 1024))
            .unwrap();
        assert!(output.stdout_truncated, "flood must be flagged");
        assert_eq!(output.stdout.len(), 4096);
        assert!(output.stdout.starts_with(b"0123456789"));
    }

    #[test]
    fn unbounded_stderr_is_captured_up_to_the_limit_and_flagged() {
        let fixture = fixture();
        let program = script(
            &fixture.dir,
            "flooderr",
            "@echo off\r\nfor /l %%i in (1,1,2000) do echo noise 1>&2\r\n",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 2000 ]; do echo noise >&2; i=$((i+1)); done\n",
        );
        let output = ProcCommand::new(program)
            .run(Bounds::new(Duration::from_secs(20), 1024, 2048))
            .unwrap();
        assert!(output.stderr_truncated);
        assert!(output.stderr.len() <= 2048);
        assert!(!output.stdout_truncated);
    }

    #[test]
    fn a_slow_command_that_finishes_inside_the_deadline_succeeds() {
        let fixture = fixture();
        let program = script(
            &fixture.dir,
            "slow",
            "@echo off\r\nping -n 2 127.0.0.1 >nul\r\necho done\r\n",
            "#!/bin/sh\nsleep 1\necho done\n",
        );
        let output = ProcCommand::new(program)
            .run(Bounds::new(Duration::from_secs(10), 1024, 1024))
            .unwrap();
        assert!(output.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("done"));
    }

    #[test]
    fn cancellation_terminates_the_tree_and_reports_cancelled() {
        let fixture = fixture();
        let program = script(
            &fixture.dir,
            "stall",
            "@echo off\r\nping -n 60 127.0.0.1 >nul\r\n",
            "#!/bin/sh\nsleep 60\n",
        );
        let cancel = AtomicBool::new(false);
        let started = std::thread::scope(|scope| {
            let handle = {
                let cancel_flag = &cancel;
                scope.spawn(move || {
                    ProcCommand::new(program).run_with_cancel(
                        Bounds::new(Duration::from_secs(30), 1024, 1024),
                        Some(cancel_flag),
                    )
                })
            };
            std::thread::sleep(Duration::from_millis(300));
            let started = Instant::now();
            cancel.store(true, Ordering::SeqCst);
            let error = handle.join().unwrap().unwrap_err();
            assert!(matches!(error, ProcError::Cancelled { .. }));
            started
        });
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn descendants_are_terminated_with_the_tree() {
        let fixture = fixture();
        let heartbeat = fixture.dir.join("heartbeat.log");
        let heartbeat_arg = heartbeat.display().to_string();
        let program = if cfg!(windows) {
            std::fs::write(
                fixture.dir.join("child.cmd"),
                format!(
                    "@echo off\r\n:loop\r\necho x>>\"{heartbeat_arg}\"\r\nping -n 1 127.0.0.1 >nul\r\ngoto loop\r\n"
                ),
            )
            .unwrap();
            script(
                &fixture.dir,
                "parent",
                "@echo off\r\nstart /b \"\" \"%~dp0child.cmd\"\r\nping -n 60 127.0.0.1 >nul\r\n",
                "",
            )
        } else {
            std::fs::write(
                fixture.dir.join("child.sh"),
                format!("#!/bin/sh\nwhile true; do echo x >> '{heartbeat_arg}'; sleep 0.1; done\n"),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let child = fixture.dir.join("child.sh");
                let mut permissions = std::fs::metadata(&child).unwrap().permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(&child, permissions).unwrap();
            }
            script(
                &fixture.dir,
                "parent",
                "",
                &format!(
                    "#!/bin/sh\n'{}/child.sh' &\nsleep 60\n",
                    fixture.dir.display()
                ),
            )
        };
        let error = ProcCommand::new(program)
            .run(Bounds::new(Duration::from_millis(700), 1024, 1024))
            .unwrap_err();
        assert!(matches!(error, ProcError::TimedOut { .. }));
        // The descendant must die with the tree: its heartbeat stops growing.
        std::thread::sleep(Duration::from_millis(500));
        let first = std::fs::read_to_string(&heartbeat)
            .unwrap_or_default()
            .len();
        std::thread::sleep(Duration::from_millis(600));
        let second = std::fs::read_to_string(&heartbeat)
            .unwrap_or_default()
            .len();
        assert_eq!(
            first, second,
            "descendant kept running after the process tree was terminated"
        );
    }

    #[test]
    fn stdin_is_never_connected_so_commands_cannot_wait_for_input() {
        let fixture = fixture();
        // A command that reads stdin must see immediate EOF, not block.
        let program = script(
            &fixture.dir,
            "reads",
            "@echo off\r\nset /p value=\r\necho got-%value%\r\n",
            "#!/bin/sh\nread value\necho \"got-$value\"\n",
        );
        let output = ProcCommand::new(program)
            .run(Bounds::new(Duration::from_secs(5), 1024, 1024))
            .unwrap();
        assert!(
            output.success(),
            "a stdin reader must exit cleanly on EOF: {:?}",
            output.status
        );
    }

    #[test]
    fn environment_entries_and_working_directory_apply() {
        let fixture = fixture();
        let program = script(
            &fixture.dir,
            "envprobe",
            "@echo off\r\necho %FACTORY_PROC_PROBE%\r\n",
            "#!/bin/sh\necho \"$FACTORY_PROC_PROBE\"\n",
        );
        let output = ProcCommand::new(program)
            .env("FACTORY_PROC_PROBE", "probe-value")
            .current_dir(&fixture.dir)
            .run(fast_bounds())
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "probe-value"
        );
    }
}
