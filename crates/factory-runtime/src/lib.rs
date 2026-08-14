use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::Instant;

use factory_core::{ExecutionRoles, Factory, FactoryError};
use factory_db::FactoryDb;
use factory_types::{AgentSession, AgentSessionMode, Run};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use thiserror::Error;
use tokio::sync::mpsc;

const MAX_TERMINAL_HISTORY_BYTES: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Planning,
    Executing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveOperation {
    pub run_id: i64,
    pub kind: OperationKind,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("workflow #{0} already has an active operation")]
    AlreadyActive(i64),
    #[error("workflow #{0} is not active in this Factory process")]
    NotActive(i64),
    #[error("interactive session {0} is not active in this Factory process")]
    TerminalNotActive(i64),
    #[error("interactive terminal error: {0}")]
    Terminal(String),
    #[error(transparent)]
    Factory(#[from] FactoryError),
}

#[derive(Clone)]
pub struct Runtime {
    root: Arc<PathBuf>,
    active: Arc<Mutex<HashMap<i64, Active>>>,
    terminals: Arc<Mutex<HashMap<i64, Arc<InteractiveTerminal>>>>,
}

struct Active {
    kind: OperationKind,
    cancel: Arc<AtomicBool>,
}

struct InteractiveTerminal {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    stopping: AtomicBool,
    output: Mutex<TerminalOutput>,
}

#[derive(Default)]
struct TerminalOutput {
    history: Vec<u8>,
    subscribers: Vec<mpsc::UnboundedSender<Vec<u8>>>,
}

pub struct TerminalSubscription {
    pub snapshot: Vec<u8>,
    pub receiver: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl Runtime {
    pub fn new(root: &Path) -> Result<Self, RuntimeError> {
        Factory::open(root)?.reconcile_interrupted()?;
        Ok(Self {
            root: Arc::new(root.to_path_buf()),
            active: Arc::new(Mutex::new(HashMap::new())),
            terminals: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn start_interactive_session(
        &self,
        agent_name: &str,
        cols: u16,
        rows: u16,
    ) -> Result<AgentSession, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let agent = factory
            .agents()
            .named_agent(agent_name)
            .map_err(FactoryError::from)?;
        let invocation = agent
            .interactive_invocation(self.root.as_path())
            .map_err(FactoryError::from)?;
        let launch = invocation.pty_launch().map_err(FactoryError::from)?;
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(terminal_size(cols, rows))
            .map_err(|error| RuntimeError::Terminal(error.to_string()))?;
        let mut command = CommandBuilder::new(&launch.program);
        command.args(&launch.args);
        command.cwd(&invocation.working_dir);
        for (name, value) in &invocation.env {
            command.env(name, value);
        }
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| RuntimeError::Terminal(error.to_string()))?;
        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| RuntimeError::Terminal(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| RuntimeError::Terminal(error.to_string()))?;

        let session = AgentSession {
            id: 0,
            run_id: None,
            task_id: None,
            attempt_id: None,
            role: "console".into(),
            agent: agent_name.to_string(),
            mode: AgentSessionMode::Interactive,
            command: invocation.command_line(),
            status: "running".into(),
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            exit_code: None,
            duration_ms: None,
            stdout: Some(String::new()),
            stderr: Some(String::new()),
        };
        let db = FactoryDb::open(&self.root.join(".factory").join("db.sqlite3"))
            .map_err(FactoryError::from)?;
        let session = match db.insert_agent_session(&session) {
            Ok(session) => session,
            Err(error) => {
                let _ = child.kill();
                return Err(FactoryError::from(error).into());
            }
        };
        let terminal = Arc::new(InteractiveTerminal {
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(killer),
            stopping: AtomicBool::new(false),
            output: Mutex::new(TerminalOutput::default()),
        });
        self.terminals
            .lock()
            .expect("terminal mutex poisoned")
            .insert(session.id, terminal.clone());
        self.spawn_terminal_process(session.id, terminal, reader, child);
        Ok(session)
    }

    pub fn subscribe_terminal(
        &self,
        session_id: i64,
    ) -> Result<TerminalSubscription, RuntimeError> {
        let terminals = self.terminals.lock().expect("terminal mutex poisoned");
        let terminal = terminals
            .get(&session_id)
            .ok_or(RuntimeError::TerminalNotActive(session_id))?;
        let mut output = terminal.output.lock().expect("terminal output poisoned");
        let (sender, receiver) = mpsc::unbounded_channel();
        output.subscribers.push(sender);
        Ok(TerminalSubscription {
            snapshot: output.history.clone(),
            receiver,
        })
    }

    pub fn write_terminal(&self, session_id: i64, data: &[u8]) -> Result<(), RuntimeError> {
        let terminal = self.terminal(session_id)?;
        let mut writer = terminal.writer.lock().expect("terminal writer poisoned");
        let writer = writer
            .as_mut()
            .ok_or(RuntimeError::TerminalNotActive(session_id))?;
        writer
            .write_all(data)
            .and_then(|_| writer.flush())
            .map_err(|error| RuntimeError::Terminal(error.to_string()))
    }

    pub fn resize_terminal(
        &self,
        session_id: i64,
        cols: u16,
        rows: u16,
    ) -> Result<(), RuntimeError> {
        let terminal = self.terminal(session_id)?;
        let master = terminal.master.lock().expect("terminal master poisoned");
        let master = master
            .as_ref()
            .ok_or(RuntimeError::TerminalNotActive(session_id))?;
        master
            .resize(terminal_size(cols, rows))
            .map_err(|error| RuntimeError::Terminal(error.to_string()))
    }

    pub fn stop_interactive_session(&self, session_id: i64) -> Result<(), RuntimeError> {
        let terminal = self.terminal(session_id)?;
        terminal.stopping.store(true, Ordering::Relaxed);
        let mut killer = terminal.killer.lock().expect("terminal killer poisoned");
        killer
            .kill()
            .map_err(|error| RuntimeError::Terminal(error.to_string()))
    }

    pub fn create_workflow(&self, objective: &str) -> Result<Run, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let run = factory.begin_run(objective)?;
        let cancel = self.reserve(run.id, OperationKind::Planning)?;
        let runtime = self.clone();
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(factory) = Factory::open(&root) {
                let _ = factory.plan_run(run.id, &cancel);
            }
            runtime.release(run.id);
        });
        Ok(run)
    }

    pub fn start_workflow(&self, run_id: i64) -> Result<ExecutionRoles, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let cancel = self.reserve(run_id, OperationKind::Executing)?;
        let roles = match factory.prepare_start(run_id) {
            Ok(roles) => roles,
            Err(error) => {
                self.release(run_id);
                return Err(error.into());
            }
        };
        self.spawn_execution(run_id, cancel);
        Ok(roles)
    }

    pub fn retry_task(&self, task_id: i64) -> Result<i64, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let task = factory
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let cancel = self.reserve(task.run_id, OperationKind::Executing)?;
        let run_id = match factory.prepare_retry(task_id) {
            Ok(run_id) => run_id,
            Err(error) => {
                self.release(task.run_id);
                return Err(error.into());
            }
        };
        self.spawn_execution(run_id, cancel);
        Ok(run_id)
    }

    pub fn cancel_workflow(&self, run_id: i64) -> Result<(), RuntimeError> {
        let cancel = {
            let active = self.active.lock().expect("runtime mutex poisoned");
            active
                .get(&run_id)
                .map(|operation| operation.cancel.clone())
                .ok_or(RuntimeError::NotActive(run_id))?
        };
        cancel.store(true, Ordering::Relaxed);
        Factory::open(&self.root)?.cancel_run(run_id)?;
        Ok(())
    }

    pub fn active_operations(&self) -> Vec<ActiveOperation> {
        let mut operations: Vec<_> = self
            .active
            .lock()
            .expect("runtime mutex poisoned")
            .iter()
            .map(|(run_id, active)| ActiveOperation {
                run_id: *run_id,
                kind: active.kind,
            })
            .collect();
        operations.sort_by_key(|operation| operation.run_id);
        operations
    }

    fn spawn_execution(&self, run_id: i64, cancel: Arc<AtomicBool>) {
        let runtime = self.clone();
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(factory) = Factory::open(&root) {
                let _ = factory.execute_active_run(run_id, &cancel);
            }
            runtime.release(run_id);
        });
    }

    fn reserve(&self, run_id: i64, kind: OperationKind) -> Result<Arc<AtomicBool>, RuntimeError> {
        let mut active = self.active.lock().expect("runtime mutex poisoned");
        if active.contains_key(&run_id) {
            return Err(RuntimeError::AlreadyActive(run_id));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        active.insert(
            run_id,
            Active {
                kind,
                cancel: cancel.clone(),
            },
        );
        Ok(cancel)
    }

    fn release(&self, run_id: i64) {
        self.active
            .lock()
            .expect("runtime mutex poisoned")
            .remove(&run_id);
    }

    fn terminal(&self, session_id: i64) -> Result<Arc<InteractiveTerminal>, RuntimeError> {
        self.terminals
            .lock()
            .expect("terminal mutex poisoned")
            .get(&session_id)
            .cloned()
            .ok_or(RuntimeError::TerminalNotActive(session_id))
    }

    fn spawn_terminal_process(
        &self,
        session_id: i64,
        terminal: Arc<InteractiveTerminal>,
        mut reader: Box<dyn Read + Send>,
        mut child: Box<dyn portable_pty::Child + Send + Sync>,
    ) {
        let output_root = self.root.clone();
        let terminals = self.terminals.clone();
        let output_terminal = terminal.clone();
        let (output_complete, output_drained) = std_mpsc::channel();
        std::thread::spawn(move || {
            let db = FactoryDb::open(&output_root.join(".factory").join("db.sqlite3")).ok();
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        let chunk = buffer[..size].to_vec();
                        publish_terminal_output(&output_terminal, &chunk);
                        if let Some(db) = &db {
                            let text = String::from_utf8_lossy(&chunk);
                            let _ = db.append_agent_session_output(session_id, Some(&text), None);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        if let Some(db) = &db {
                            let _ = db.append_agent_session_output(
                                session_id,
                                None,
                                Some(&format!("PTY read failed: {error}")),
                            );
                        }
                        break;
                    }
                }
            }
            output_terminal
                .output
                .lock()
                .expect("terminal output poisoned")
                .subscribers
                .clear();
            let _ = output_complete.send(());
        });

        let root = self.root.clone();
        std::thread::spawn(move || {
            let timer = Instant::now();
            let status = child.wait();
            terminal
                .writer
                .lock()
                .expect("terminal writer poisoned")
                .take();
            terminal
                .master
                .lock()
                .expect("terminal master poisoned")
                .take();
            let _ = output_drained.recv();
            if let Ok(db) = FactoryDb::open(&root.join(".factory").join("db.sqlite3")) {
                let stdout = terminal
                    .output
                    .lock()
                    .expect("terminal output poisoned")
                    .history
                    .clone();
                let stdout = String::from_utf8_lossy(&stdout);
                let _ = db.set_agent_session_output(session_id, Some(&stdout), None);
                match status {
                    Ok(status) => {
                        let session_status = if terminal.stopping.load(Ordering::Relaxed) {
                            "cancelled"
                        } else if status.success() {
                            "success"
                        } else {
                            "failed"
                        };
                        let _ = db.finish_agent_session(
                            session_id,
                            session_status,
                            Some(status.exit_code() as i32),
                            timer.elapsed().as_millis() as u64,
                        );
                    }
                    Err(error) => {
                        let _ = db.append_agent_session_output(
                            session_id,
                            None,
                            Some(&format!("PTY wait failed: {error}")),
                        );
                        let _ = db.finish_agent_session(
                            session_id,
                            "failed",
                            None,
                            timer.elapsed().as_millis() as u64,
                        );
                    }
                }
            }
            terminals
                .lock()
                .expect("terminal mutex poisoned")
                .remove(&session_id);
        });
    }
}

fn terminal_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols: cols.clamp(20, 500),
        rows: rows.clamp(5, 200),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn publish_terminal_output(terminal: &InteractiveTerminal, chunk: &[u8]) {
    let mut output = terminal.output.lock().expect("terminal output poisoned");
    output.history.extend_from_slice(chunk);
    if output.history.len() > MAX_TERMINAL_HISTORY_BYTES {
        let overflow = output.history.len() - MAX_TERMINAL_HISTORY_BYTES;
        output.history.drain(..overflow);
    }
    output
        .subscribers
        .retain(|subscriber| subscriber.send(chunk.to_vec()).is_ok());
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{IsTerminal, Read as _};

    use factory_agent::{AgentKind, PromptTransport};
    use factory_core::{AgentEntry, Config, Factory};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn pty_child_probe() {
        if std::env::var("FACTORY_PTY_CHILD").as_deref() == Ok("1") {
            println!("TTY={}", std::io::stdin().is_terminal());
            std::io::stdout().flush().unwrap();
            let _ = std::io::stdin().read(&mut [0_u8; 1]);
            std::process::exit(0);
        }
    }

    #[test]
    fn interactive_session_has_a_real_terminal_and_persists_its_mode() {
        let dir = TempDir::new().unwrap();
        Factory::init(dir.path()).unwrap();
        let test_executable = std::env::current_exe().unwrap();
        #[cfg(windows)]
        let command = {
            let bin = dir.path().join("fake-npm-bin");
            std::fs::create_dir_all(&bin).unwrap();
            let native = bin.join("fake-agent.exe");
            std::fs::copy(&test_executable, &native).unwrap();
            let shim = bin.join("fake-agent.cmd");
            std::fs::write(&shim, "@ECHO off\r\n\"%dp0%\\fake-agent.exe\" %*\r\n").unwrap();
            shim.to_string_lossy().into_owned()
        };
        #[cfg(not(windows))]
        let command = test_executable.to_string_lossy().into_owned();
        let interactive_args = vec![
            "--exact".into(),
            "tests::pty_child_probe".into(),
            "--nocapture".into(),
        ];
        let mut config = Config::default();
        config.agents.insert(
            "terminal-test".into(),
            AgentEntry {
                kind: Some(AgentKind::Custom),
                command,
                args: Vec::new(),
                env: BTreeMap::from([("FACTORY_PTY_CHILD".into(), "1".into())]),
                prompt_transport: Some(PromptTransport::Disabled),
                interactive_args: Some(interactive_args),
                capabilities: Vec::new(),
            },
        );
        config.write_atomic(dir.path()).unwrap();

        let runtime = Runtime::new(dir.path()).unwrap();
        let session = runtime
            .start_interactive_session("terminal-test", 90, 24)
            .unwrap();
        assert_eq!(session.mode, AgentSessionMode::Interactive);
        runtime.resize_terminal(session.id, 100, 30).unwrap();
        let mut subscription = runtime.subscribe_terminal(session.id).unwrap();
        let mut output = subscription.snapshot;
        let mut answered_cursor_query = false;
        for _ in 0..100 {
            while let Ok(chunk) = subscription.receiver.try_recv() {
                output.extend(chunk);
            }
            if !answered_cursor_query && output.windows(4).any(|window| window == b"\x1b[6n") {
                runtime.write_terminal(session.id, b"\x1b[1;1R").unwrap();
                answered_cursor_query = true;
            }
            if String::from_utf8_lossy(&output).contains("TTY=true") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("TTY=true"), "PTY output was: {output:?}");
        let _ = runtime.stop_interactive_session(session.id);

        let db = FactoryDb::open(&dir.path().join(".factory").join("db.sqlite3")).unwrap();
        let mut saved = db.get_agent_session(session.id).unwrap().unwrap();
        for _ in 0..100 {
            if saved.status != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            saved = db.get_agent_session(session.id).unwrap().unwrap();
        }
        assert_eq!(saved.mode, AgentSessionMode::Interactive);
        assert_ne!(saved.status, "running");
        assert!(saved.stdout.unwrap_or_default().contains("TTY=true"));
    }

    #[cfg(windows)]
    #[test]
    fn interactive_generic_batch_agent_runs_through_the_pty() {
        let dir = TempDir::new().unwrap();
        Factory::init(dir.path()).unwrap();
        let bin = dir.path().join("agent-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let batch = bin.join("fake-agent.cmd");
        std::fs::write(&batch, "@echo off\r\necho BATCH_PTY_OK\r\n").unwrap();
        let mut config = Config::default();
        config.agents.insert(
            "batch-test".into(),
            AgentEntry {
                kind: Some(AgentKind::Custom),
                command: batch.to_string_lossy().into_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                prompt_transport: Some(PromptTransport::Disabled),
                interactive_args: Some(Vec::new()),
                capabilities: Vec::new(),
            },
        );
        config.write_atomic(dir.path()).unwrap();

        let runtime = Runtime::new(dir.path()).unwrap();
        let session = runtime
            .start_interactive_session("batch-test", 90, 24)
            .unwrap();
        let mut subscription = runtime.subscribe_terminal(session.id).unwrap();
        let mut output = subscription.snapshot;
        let mut answered_cursor_query = false;
        for _ in 0..100 {
            while let Ok(chunk) = subscription.receiver.try_recv() {
                output.extend(chunk);
            }
            if !answered_cursor_query && output.windows(4).any(|window| window == b"\x1b[6n") {
                runtime.write_terminal(session.id, b"\x1b[1;1R").unwrap();
                answered_cursor_query = true;
            }
            if String::from_utf8_lossy(&output).contains("BATCH_PTY_OK") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = String::from_utf8_lossy(&output);
        assert!(
            output.contains("BATCH_PTY_OK"),
            "PTY output was: {output:?}"
        );
        let _ = runtime.stop_interactive_session(session.id);

        let db = FactoryDb::open(&dir.path().join(".factory").join("db.sqlite3")).unwrap();
        let mut saved = db.get_agent_session(session.id).unwrap().unwrap();
        for _ in 0..100 {
            if saved.status != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            saved = db.get_agent_session(session.id).unwrap().unwrap();
        }
        assert_ne!(saved.status, "running");
        assert!(saved.stdout.unwrap_or_default().contains("BATCH_PTY_OK"));
    }
}
