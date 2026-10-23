use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use factory_agent::{
    AgentConfig, AgentKind, AgentRequest, AgentResult, CommandAgent, PromptTransport,
    MISSION_PLACEHOLDER,
};
use tempfile::TempDir;

const MODE_ENV: &str = "FACTORY_AGENT_TEST_IO_MODE";
const BULK_OUTPUT: usize = 512 * 1024;
const LARGE_MISSION: usize = 2 * 1024 * 1024;
const WATCHDOG: Duration = Duration::from_secs(20);
const CANCEL_BUDGET: Duration = Duration::from_secs(10);

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0_u64, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(*byte as u64)
    })
}

fn mission_of_length(length: usize) -> String {
    let unit = "mission-context-0123456789\n";
    let mut mission = String::with_capacity(length + unit.len());
    while mission.len() < length {
        mission.push_str(unit);
    }
    mission.truncate(length);
    mission
}

fn expected_stdin_report(mission: &str) -> String {
    let bytes = mission.as_bytes();
    format!("STDIN_LEN={} STDIN_SUM={}", bytes.len(), checksum(bytes))
}

fn write_bulk(mut writer: impl Write, byte: u8, length: usize) -> std::io::Result<()> {
    let chunk = vec![byte; 4096];
    let mut remaining = length;
    while remaining > 0 {
        let amount = remaining.min(chunk.len());
        writer.write_all(&chunk[..amount])?;
        remaining -= amount;
    }
    writer.flush()
}

fn close_stdin() {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        drop(unsafe { std::fs::File::from_raw_fd(0) });
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        let handle = std::io::stdin().as_raw_handle();
        drop(unsafe { std::fs::File::from_raw_handle(handle) });
    }
}

fn fake_child_watchdog() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        eprintln!("fake agent child exceeded its watchdog deadline");
        std::process::exit(124);
    });
}

fn read_stdin_and_report() {
    let mut payload = Vec::new();
    std::io::stdin().read_to_end(&mut payload).unwrap();
    let mut stdout = std::io::stdout().lock();
    writeln!(
        stdout,
        "STDIN_LEN={} STDIN_SUM={}",
        payload.len(),
        checksum(&payload)
    )
    .unwrap();
    stdout.flush().unwrap();
}

#[test]
fn fake_child() {
    let Some(mode) = std::env::var_os(MODE_ENV) else {
        return;
    };
    fake_child_watchdog();
    match mode.to_string_lossy().as_ref() {
        "read_stdin" => read_stdin_and_report(),
        "bulk_stdout" => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(b"STDOUT_BULK_BEGIN\n").unwrap();
            write_bulk(&mut stdout, b'o', BULK_OUTPUT).unwrap();
            stdout.write_all(b"\nSTDOUT_BULK_END\n").unwrap();
            stdout.flush().unwrap();
            drop(stdout);
            read_stdin_and_report();
        }
        "bulk_stderr" => {
            let mut stderr = std::io::stderr().lock();
            stderr.write_all(b"STDERR_BULK_BEGIN\n").unwrap();
            write_bulk(&mut stderr, b'e', BULK_OUTPUT).unwrap();
            stderr.write_all(b"\nSTDERR_BULK_END\n").unwrap();
            stderr.flush().unwrap();
            drop(stderr);
            read_stdin_and_report();
        }
        "simultaneous" => {
            let stdout_writer = std::thread::spawn(|| {
                let mut stdout = std::io::stdout().lock();
                write_bulk(&mut stdout, b'o', BULK_OUTPUT).unwrap();
                stdout.write_all(b"\nSTDOUT_DONE\n").unwrap();
                stdout.flush().unwrap();
            });
            let stderr_writer = std::thread::spawn(|| {
                let mut stderr = std::io::stderr().lock();
                write_bulk(&mut stderr, b'e', BULK_OUTPUT).unwrap();
                stderr.write_all(b"\nSTDERR_DONE\n").unwrap();
                stderr.flush().unwrap();
            });
            stdout_writer.join().unwrap();
            stderr_writer.join().unwrap();
            read_stdin_and_report();
        }
        "close_stdin" => {
            close_stdin();
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "STDIN_CLOSED").unwrap();
            stdout.flush().unwrap();
            drop(stdout);
            std::thread::sleep(Duration::from_millis(250));
        }
        "exit_early" => {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "EXITED_EARLY").unwrap();
            stdout.flush().unwrap();
            drop(stdout);
            std::process::exit(7);
        }
        "never_read" => {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "READY").unwrap();
            stdout.flush().unwrap();
            drop(stdout);
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        "flood_output" => {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "READY").unwrap();
            stdout.flush().unwrap();
            drop(stdout);
            let chunk = [b'x'; 4096];
            let mut stdout = std::io::stdout().lock();
            let mut stderr = std::io::stderr().lock();
            loop {
                stdout.write_all(&chunk).unwrap();
                stderr.write_all(&chunk).unwrap();
            }
        }
        other => panic!("unknown fake agent mode: {other}"),
    }
}

fn fake_agent(mode: &str) -> CommandAgent {
    let executable = std::env::current_exe().unwrap();
    let mut config = AgentConfig::new("fake-agent", executable.to_string_lossy().into_owned());
    config.args = vec![
        "--exact".into(),
        "fake_child".into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ];
    config.env.insert(MODE_ENV.into(), mode.to_string());
    CommandAgent::new(config)
}

fn run(agent: &CommandAgent, mission: &str, directory: &TempDir) -> AgentResult {
    agent
        .run(&AgentRequest::new(mission, directory.path()))
        .unwrap()
}

fn cancel_when_ready(agent: &CommandAgent, request: &AgentRequest) -> (AgentResult, Duration) {
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancel);
    let started = Instant::now();
    let result = agent
        .run_observed(request, &cancel, |_, chunk| {
            if chunk.contains("READY") {
                signal.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
    (result, started.elapsed())
}

#[test]
fn small_stdin_payload_is_delivered_with_normal_output() {
    let directory = TempDir::new().unwrap();
    let mission = "small mission payload";
    let result = run(&fake_agent("read_stdin"), mission, &directory);

    assert_eq!(result.exit_code, Some(0));
    assert!(!result.cancelled);
    assert!(result.stdout.contains(&expected_stdin_report(mission)));
    assert!(result.stdout.contains("STDIN_LEN=21"));
}

#[test]
fn large_stdin_payload_is_delivered_without_truncation() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let result = run(&fake_agent("read_stdin"), &mission, &directory);

    assert_eq!(result.exit_code, Some(0));
    assert!(result.stdout.contains(&expected_stdin_report(&mission)));
    assert!(mission.len() > 32 * 1024);
}

#[test]
fn substantial_stdout_before_stdin_does_not_deadlock() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let result = run(&fake_agent("bulk_stdout"), &mission, &directory);

    assert_eq!(result.exit_code, Some(0));
    assert!(result.stdout.contains("STDOUT_BULK_END"));
    assert!(result.stdout.contains(&expected_stdin_report(&mission)));
    assert!(result.stdout.len() >= BULK_OUTPUT);
}

#[test]
fn substantial_stderr_before_stdin_does_not_deadlock() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let result = run(&fake_agent("bulk_stderr"), &mission, &directory);

    assert_eq!(result.exit_code, Some(0));
    assert!(result.stderr.contains("STDERR_BULK_END"));
    assert!(result.stdout.contains(&expected_stdin_report(&mission)));
    assert!(result.stderr.len() >= BULK_OUTPUT);
}

#[test]
fn stdout_and_stderr_are_drained_simultaneously() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let result = run(&fake_agent("simultaneous"), &mission, &directory);

    assert_eq!(result.exit_code, Some(0));
    assert!(result.stdout.contains("STDOUT_DONE"));
    assert!(result.stderr.contains("STDERR_DONE"));
    assert!(result.stdout.contains(&expected_stdin_report(&mission)));
    assert!(result.stdout.len() >= BULK_OUTPUT);
    assert!(result.stderr.len() >= BULK_OUTPUT);
}

#[test]
fn a_child_that_closes_stdin_early_is_not_an_io_failure() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let result = run(&fake_agent("close_stdin"), &mission, &directory);

    assert_eq!(result.exit_code, Some(0));
    assert!(!result.cancelled);
    assert!(result.stdout.contains("STDIN_CLOSED"));
}

#[test]
fn a_child_that_exits_before_consuming_input_is_not_an_io_failure() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let result = run(&fake_agent("exit_early"), &mission, &directory);

    assert_eq!(result.exit_code, Some(7));
    assert!(!result.cancelled);
    assert!(result.stdout.contains("EXITED_EARLY"));
}

#[test]
fn cancellation_interrupts_a_blocked_stdin_write() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let request = AgentRequest::new(mission, directory.path());
    let (result, elapsed) = cancel_when_ready(&fake_agent("never_read"), &request);

    assert!(result.cancelled);
    assert!(result.stdout.contains("READY"));
    assert!(elapsed < CANCEL_BUDGET);
}

#[test]
fn cancellation_interrupts_continuous_output_while_stdin_is_blocked() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);
    let request = AgentRequest::new(mission, directory.path());
    let (result, elapsed) = cancel_when_ready(&fake_agent("flood_output"), &request);

    assert!(result.cancelled);
    assert!(result.stdout.contains("READY"));
    assert!(elapsed < CANCEL_BUDGET);
}

#[test]
fn repeated_cancellations_complete_without_stalling() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(LARGE_MISSION);

    for _ in 0..4 {
        let request = AgentRequest::new(mission.clone(), directory.path());
        let (result, elapsed) = cancel_when_ready(&fake_agent("never_read"), &request);
        assert!(result.cancelled);
        assert!(elapsed < CANCEL_BUDGET);
    }
}

#[test]
fn codex_stdin_transport_remains_compatible() {
    let directory = TempDir::new().unwrap();
    let mission = mission_of_length(1024 * 1024);
    let executable = std::env::current_exe().unwrap();
    let harness_args = vec![
        "--exact".to_string(),
        "fake_child".to_string(),
        "--nocapture".to_string(),
        "--test-threads=1".to_string(),
    ];
    let mut config = AgentConfig::new("codex", executable.to_string_lossy().into_owned());
    config.kind = AgentKind::Codex;
    config.args = harness_args.clone();
    config.prompt_transport = AgentKind::Codex.prompt_transport();
    config.env.insert(MODE_ENV.into(), "read_stdin".into());
    let agent = CommandAgent::new(config);

    let invocation = agent
        .automated_invocation(&AgentRequest::new(&mission, directory.path()))
        .unwrap();
    assert_eq!(invocation.args, harness_args);
    assert!(!invocation.args.iter().any(|argument| argument == &mission));
    let result = run(&agent, &mission, &directory);

    assert_eq!(result.exit_code, Some(0));
    assert!(result.stdout.contains(&expected_stdin_report(&mission)));
}

#[test]
fn argument_transport_still_uses_argv_without_stdin() {
    let executable = std::env::current_exe().unwrap();
    let mut config = AgentConfig::new("argument", executable.to_string_lossy().into_owned());
    config.args = vec!["run".into(), MISSION_PLACEHOLDER.into()];
    config.prompt_transport = PromptTransport::Argument;
    let invocation = CommandAgent::new(config)
        .automated_invocation(&AgentRequest::new("mission with spaces", "."))
        .unwrap();

    assert_eq!(invocation.args, vec!["run", "mission with spaces"]);
    assert_eq!(invocation.stdin_payload, None);
}
