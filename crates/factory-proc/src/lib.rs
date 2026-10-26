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
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How large each read from a pipe is before being handed to the capture.
const READ_BUFFER_BYTES: usize = 8192;

/// How often the supervisor polls the child and the cancellation flag.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Grace window after a forced tree termination for the pipes to close and
/// the reader threads to reach EOF before the collector is detached.
const HARD_KILL_GRACE: Duration = Duration::from_secs(5);

/// Drain wait slice: how often the cancellation flag is re-checked while
/// waiting for the reader threads.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(50);

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
        // Windows: the child and every descendant it spawns (without an
        // explicit breakaway) join a Job Object, so termination — including
        // of descendants that outlive the parent — is one kernel call and
        // survives parent exit. Unix uses the child's own process group.
        #[cfg(windows)]
        let mut job = crate::windows_job::JobGuard::assign(&child);
        #[cfg(not(windows))]
        let mut job: Option<crate::windows_job::JobHandle> = None;
        let stdout_reader = child.stdout.take().expect("stdout piped before spawn");
        let stderr_reader = child.stderr.take().expect("stderr piped before spawn");
        let stdout_handle = spawn_bounded_reader(stdout_reader, bounds.stdout_limit);
        let stderr_handle = spawn_bounded_reader(stderr_reader, bounds.stderr_limit);

        // The deadline bounds the COMPLETE operation (ASF-F03): supervision,
        // pipe draining, termination, and reader shutdown all share it.
        let deadline = Instant::now() + bounds.timeout;
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
                    terminate_tree(&mut child, &mut job, false);
                    return Err(ProcError::Io(error));
                }
            }
            if Instant::now() >= deadline {
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
            terminate_tree(&mut child, &mut job, false);
        }
        // Reap so no zombie remains.
        let reaped = child.wait().ok();

        // Bounded drain (ASF-F03): joining the reader threads directly can
        // block forever when a descendant inherited a pipe and still holds
        // it after the parent exited. A collector thread joins both readers
        // and reports through a channel we wait on with the remaining
        // deadline; if that expires, the tree is force-terminated (closing
        // every in-job/group pipe) and given a final grace window before the
        // collector is detached.
        let (sender, receiver) = mpsc::sync_channel::<(ReaderOutcome, ReaderOutcome)>(1);
        // Dropping this handle (scope exit, or a detached stuck drain)
        // detaches the collector thread; it exits once the pipes close.
        let _collector = std::thread::spawn(move || {
            let stdout = reader_outcome(stdout_handle.join());
            let stderr = reader_outcome(stderr_handle.join());
            let _ = sender.send((stdout, stderr));
        });
        // Wait for the drain in bounded slices so a cancellation flag that
        // fires while readers are stuck (a descendant holds a pipe after the
        // parent exited) is still honored within one slice.
        let (stdout, stderr) = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Deadline exhausted with readers stuck: escalate to a
                // forced kill, grant a bounded grace for the pipes to close,
                // and detach whatever still refuses to finish. Escaping the
                // kill requires deliberately leaving the process group/job —
                // a documented limit. Either way the operation exceeded its
                // deadline: a timeout, never a silent partial success.
                timed_out = true;
                break force_drain(&receiver, &mut child, &mut job);
            }
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                cancelled = true;
                break force_drain(&receiver, &mut child, &mut job);
            }
            match receiver.recv_timeout(remaining.min(DRAIN_POLL_INTERVAL)) {
                Ok(pair) => break pair,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // The collector only exits after sending; a disconnected
                // channel is a supervisor bug — surface it as a failed drain.
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    timed_out = true;
                    break force_drain(&receiver, &mut child, &mut job);
                }
            }
        };

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
        // A reader that failed mid-stream (or panicked) leaves the capture
        // incomplete; that is an I/O failure, never a silent empty success.
        if stdout.failed || stderr.failed {
            return Err(ProcError::Io(std::io::Error::other(
                "process output capture did not complete (reader failed)",
            )));
        }
        Ok(output)
    }
}

/// What a pipe-reader thread produced: captured bytes, whether the capture
/// limit truncated the stream, and whether the read ended in anything other
/// than a clean EOF.
struct ReaderOutcome {
    captured: Vec<u8>,
    truncated: bool,
    failed: bool,
}

/// Escalates to a forced tree termination and waits a bounded grace for the
/// reader collector to finish; whatever remains stuck is detached (the
/// collector and any surviving reader thread are leaked deliberately: their
/// memory is bounded by the capture limits, and a reader can only be stuck
/// if a process outside the group/job still holds the pipe).
fn force_drain(
    receiver: &mpsc::Receiver<(ReaderOutcome, ReaderOutcome)>,
    child: &mut std::process::Child,
    job: &mut Option<crate::windows_job::JobHandle>,
) -> (ReaderOutcome, ReaderOutcome) {
    terminate_tree(child, job, true);
    match receiver.recv_timeout(HARD_KILL_GRACE) {
        Ok(pair) => pair,
        Err(_) => (
            ReaderOutcome {
                captured: Vec::new(),
                truncated: false,
                failed: true,
            },
            ReaderOutcome {
                captured: Vec::new(),
                truncated: false,
                failed: true,
            },
        ),
    }
}

fn reader_outcome(
    joined: std::result::Result<Captured, Box<dyn std::any::Any + Send>>,
) -> ReaderOutcome {
    match joined {
        Ok(captured) => ReaderOutcome {
            captured: captured.captured,
            truncated: captured.truncated,
            failed: captured.read_failed,
        },
        // A panicked reader is a supervisor bug; surface it as a failure
        // instead of pretending the stream was empty.
        Err(_) => ReaderOutcome {
            captured: Vec::new(),
            truncated: false,
            failed: true,
        },
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
    read_failed: bool,
}

fn spawn_bounded_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> std::thread::JoinHandle<Captured> {
    std::thread::spawn(move || {
        let mut result = Captured {
            captured: Vec::new(),
            truncated: false,
            read_failed: false,
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
                Err(_) => {
                    result.read_failed = true;
                    return result;
                }
            }
        }
    })
}

/// Terminates a process and every descendant it spawned. Best effort: called
/// only after a timeout or cancellation, when leaving the tree running would
/// be strictly worse than an imperfect kill.
///
/// `hard` selects the escalation stage: graceful first (SIGTERM on Unix, job
/// termination on Windows), then forced (SIGKILL) when readers remain stuck
/// because a descendant kept a pipe open after the parent exited.
#[cfg(unix)]
fn terminate_tree(
    child: &mut std::process::Child,
    job: &mut Option<crate::windows_job::JobHandle>,
    hard: bool,
) {
    let _ = job;
    // The child leads its own process group (set at spawn), so signaling the
    // negative pid reaches the whole tree; the group persists while any
    // descendant lives, even after the leader (the child) has exited.
    let signal = if hard { "-KILL" } else { "-TERM" };
    let _ = Command::new("kill")
        .args([signal, "--", &format!("-{}", child.id())])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(windows)]
fn terminate_tree(
    child: &mut std::process::Child,
    job: &mut Option<crate::windows_job::JobHandle>,
    hard: bool,
) {
    let _ = hard; // TerminateJobObject is already forceful; no TERM stage.
                  // The Job Object (when assignment succeeded) reaches every descendant in
                  // one kernel call, including ones that outlived the parent — taskkill on
                  // the parent PID cannot, once the parent has exited.
    if let Some(job) = job.take() {
        job.terminate();
        drop(job);
    } else {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Placeholder for platforms without Job Objects; `JobHandle` is never
/// constructed there and termination uses the process group instead.
#[cfg(not(windows))]
pub mod windows_job {
    #[derive(Debug)]
    pub enum JobHandle {}
}

#[cfg(windows)]
pub mod windows_job {
    //! Windows Job Object supervision: the child and every descendant it
    //! spawns (unless they explicitly break away) join one job, so
    //! terminating the job kills the whole tree — including descendants
    //! that outlive the parent and would otherwise keep inherited pipes
    //! open. `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` makes the kernel clean up
    //! even if this process dies unexpectedly.

    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// Platform alias used by the supervisor's termination plumbing.
    pub type JobHandle = JobGuard;

    /// Owned handle to a job object the child was assigned to. Terminating
    /// or dropping it kills every process in the job.
    pub struct JobGuard {
        handle: *mut core::ffi::c_void,
    }

    // The handle is a plain kernel object owned exclusively by this guard;
    // the child only references the job, it does not alias the handle.
    unsafe impl Send for JobGuard {}
    unsafe impl Sync for JobGuard {}

    impl JobGuard {
        /// Creates a kill-on-close job and assigns `child` to it. `None`
        /// when the platform calls fail (callers fall back to taskkill).
        pub fn assign(child: &std::process::Child) -> Option<JobGuard> {
            unsafe {
                let job = CreateJobObjectW(core::ptr::null(), core::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = core::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let configured = windows_sys::Win32::System::JobObjects::SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if configured == 0 {
                    CloseHandle(job);
                    return None;
                }
                if AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
                    CloseHandle(job);
                    return None;
                }
                Some(JobGuard { handle: job })
            }
        }

        /// Forcefully terminates every process in the job.
        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.handle, 1);
            }
        }
    }

    impl Drop for JobGuard {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
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

    /// Linux runners can transiently refuse to exec a freshly written script
    /// (`ETXTBSY` — the kernel's writeback still holds the inode) when many
    /// tests write and spawn scripts concurrently. Real binaries from PATH
    /// never hit this; retrying the spawn briefly is deterministic-safe.
    fn run_resilient(
        command: ProcCommand,
        bounds: Bounds,
        cancel: Option<&AtomicBool>,
    ) -> std::result::Result<ProcOutput, ProcError> {
        let mut attempts = 0;
        loop {
            match command.run_with_cancel(bounds, cancel) {
                Err(ProcError::Spawn(error))
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 40 =>
                {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(25));
                }
                other => return other,
            }
        }
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
        let output = run_resilient(ProcCommand::new(program), fast_bounds(), None).unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "oops");
        assert!(!output.stdout_truncated && !output.stderr_truncated);
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        let error = run_resilient(
            ProcCommand::new("definitely-not-a-real-program-xyz"),
            fast_bounds(),
            None,
        )
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
        let error = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_millis(400), 1024, 1024),
            None,
        )
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
        let output = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_secs(20), 4096, 1024),
            None,
        )
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
        let output = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_secs(20), 1024, 2048),
            None,
        )
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
        let output = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_secs(10), 1024, 1024),
            None,
        )
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
                    run_resilient(
                        ProcCommand::new(program),
                        Bounds::new(Duration::from_secs(30), 1024, 1024),
                        Some(cancel_flag),
                    )
                })
            };
            std::thread::sleep(Duration::from_millis(300));
            let started = Instant::now();
            cancel.store(true, Ordering::SeqCst);
            let error = handle.join().unwrap().unwrap_err();
            assert!(
                matches!(error, ProcError::Cancelled { .. }),
                "unexpected outcome: {error:?}"
            );
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
        let error = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_millis(700), 1024, 1024),
            None,
        )
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
        let output = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_secs(5), 1024, 1024),
            None,
        )
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
        let output = run_resilient(
            ProcCommand::new(program)
                .env("FACTORY_PROC_PROBE", "probe-value")
                .current_dir(&fixture.dir),
            fast_bounds(),
            None,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "probe-value"
        );
    }

    // --- ASF-F03: bounded lifecycle with descendants holding pipes ---------

    /// Builds parent/child scripts where the parent exits immediately and
    /// the descendant keeps writing to `streams` ("out", "err", or "both")
    /// and appends to a heartbeat file until killed. `trap_term` makes the
    /// descendant ignore SIGTERM (Unix escalation coverage).
    fn descendant_fixture(dir: &std::path::Path, streams: &str, trap_term: bool) -> OsString {
        let heartbeat = dir.join("heartbeat.log");
        let heartbeat_arg = heartbeat.display().to_string();
        if cfg!(windows) {
            let noise = match streams {
                "out" => {
                    "echo held-stdout
"
                }
                "err" => {
                    "echo held-stderr 1>&2
"
                }
                _ => {
                    "echo held-both
echo held-both-err 1>&2
"
                }
            };
            let _ = trap_term; // TerminateJobObject cannot be trapped.
            std::fs::write(
                dir.join("child.cmd"),
                format!(
                    "@echo off
{noise}
:loop
echo x>>\"{heartbeat_arg}\"
ping -n 1 127.0.0.1 >nul
goto loop
"
                ),
            )
            .unwrap();
            std::fs::write(
                dir.join("parent.cmd"),
                "@echo off
start /b \"\" \"%~dp0child.cmd\"
exit /b 0
",
            )
            .unwrap();
            dir.join("parent.cmd").into_os_string()
        } else {
            let noise = match streams {
                "out" => {
                    "echo held-stdout
"
                }
                "err" => {
                    "echo held-stderr >&2
"
                }
                _ => {
                    "echo held-both
echo held-both-err >&2
"
                }
            };
            let trap = if trap_term {
                "trap '' TERM
"
            } else {
                ""
            };
            std::fs::write(
                dir.join("child.sh"),
                format!(
                    "#!/bin/sh
{trap}{noise}
while true; do echo x >> '{heartbeat_arg}'; sleep 0.1; done
"
                ),
            )
            .unwrap();
            std::fs::write(
                dir.join("parent.sh"),
                "#!/bin/sh
\"$(dirname \"$0\")/child.sh\" &
exit 0
",
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for name in ["child.sh", "parent.sh"] {
                    let path = dir.join(name);
                    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
                    permissions.set_mode(0o755);
                    std::fs::set_permissions(&path, permissions).unwrap();
                }
            }
            dir.join("parent.sh").into_os_string()
        }
    }

    /// The heartbeat must stop growing: the descendant died with the tree.
    fn descendant_is_dead(dir: &std::path::Path) {
        let heartbeat = dir.join("heartbeat.log");
        std::thread::sleep(Duration::from_millis(500));
        let first = std::fs::read_to_string(&heartbeat)
            .unwrap_or_default()
            .len();
        std::thread::sleep(Duration::from_millis(900));
        let second = std::fs::read_to_string(&heartbeat)
            .unwrap_or_default()
            .len();
        assert_eq!(
            first, second,
            "a descendant survived termination and kept the pipe open"
        );
    }

    #[test]
    fn a_deadline_bounds_the_operation_when_a_descendant_holds_stdout() {
        let fixture = fixture();
        let program = descendant_fixture(&fixture.dir, "out", false);
        let started = Instant::now();
        let error = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_millis(700), 4096, 4096),
            None,
        )
        .unwrap_err();
        let ProcError::TimedOut { output } = &error else {
            panic!("expected a bounded timeout, got {error:?}");
        };
        // The deadline covers the whole operation: supervision + drain +
        // cleanup, with a documented grace for pipe closure.
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "descendant-held stdout was not bounded: {:?}",
            started.elapsed()
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("held-stdout"),
            "partial stdout captured before the kill: {:?}",
            output.stdout
        );
        descendant_is_dead(&fixture.dir);
    }

    #[test]
    fn a_deadline_bounds_the_operation_when_a_descendant_holds_stderr() {
        let fixture = fixture();
        let program = descendant_fixture(&fixture.dir, "err", false);
        let started = Instant::now();
        let error = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_millis(700), 4096, 4096),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, ProcError::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(10));
        descendant_is_dead(&fixture.dir);
    }

    #[test]
    fn a_deadline_bounds_the_operation_when_a_descendant_holds_both_pipes() {
        let fixture = fixture();
        let program = descendant_fixture(&fixture.dir, "both", false);
        let started = Instant::now();
        let error = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_millis(700), 4096, 4096),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, ProcError::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(10));
        descendant_is_dead(&fixture.dir);
    }

    #[test]
    fn cancellation_is_bounded_when_a_descendant_holds_the_pipes() {
        let fixture = fixture();
        let program = descendant_fixture(&fixture.dir, "both", false);
        let cancel = AtomicBool::new(false);
        let started = std::thread::scope(|scope| {
            let handle = {
                let cancel_flag = &cancel;
                scope.spawn(move || {
                    run_resilient(
                        ProcCommand::new(program),
                        Bounds::new(Duration::from_secs(30), 4096, 4096),
                        Some(cancel_flag),
                    )
                })
            };
            std::thread::sleep(Duration::from_millis(400));
            let started = Instant::now();
            cancel.store(true, Ordering::SeqCst);
            let error = handle.join().unwrap().unwrap_err();
            assert!(
                matches!(error, ProcError::Cancelled { .. }),
                "unexpected outcome: {error:?}"
            );
            started
        });
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "cancellation was not bounded: {:?}",
            started.elapsed()
        );
        descendant_is_dead(&fixture.dir);
    }

    #[test]
    fn a_signal_ignoring_descendant_is_still_killed_by_escalation() {
        if cfg!(windows) {
            // TerminateJobObject cannot be trapped; the job-coverage
            // assertion is already exercised by the stdout/stderr tests.
            return;
        }
        let fixture = fixture();
        let program = descendant_fixture(&fixture.dir, "out", true);
        let started = Instant::now();
        let error = run_resilient(
            ProcCommand::new(program),
            Bounds::new(Duration::from_millis(700), 4096, 4096),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, ProcError::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(10));
        descendant_is_dead(&fixture.dir);
    }
}
