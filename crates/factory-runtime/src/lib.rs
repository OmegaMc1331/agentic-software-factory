use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use factory_agent::Utf8StreamDecoder;
use factory_core::{Factory, FactoryError, FACTORY_DIR};
use factory_db::FactoryDb;
use factory_types::{AgentSession, AgentSessionMode, RecipeSnapshot, Run, WorkflowTeam};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use thiserror::Error;
use tokio::sync::Notify;

mod project_lock;

use project_lock::ProjectLock;

const MAX_TERMINAL_HISTORY_BYTES: usize = 1_000_000;
const MAX_TERMINAL_SUBSCRIBERS: usize = 8;
const MAX_TERMINAL_SUBSCRIBER_BYTES: usize = 2 * 1024 * 1024;
const MAX_TERMINAL_SUBSCRIBER_CHUNKS: usize = 256;

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
    #[error("interactive session {0} already has the maximum number of terminal subscribers")]
    TerminalSubscribersFull(i64),
    #[error("interactive terminal error: {0}")]
    Terminal(String),
    #[error(
        "another Factory runtime is already serving project {root} (lock: {lock}); stop that Factory process before starting a new one"
    )]
    ProjectAlreadyLocked { root: PathBuf, lock: PathBuf },
    #[error("cannot acquire the Factory project lock at {lock}: {source}")]
    ProjectLock {
        lock: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot resolve the Factory project root {root}: {source}")]
    ProjectRoot {
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Factory(#[from] FactoryError),
}

#[derive(Clone)]
pub struct Runtime {
    root: Arc<PathBuf>,
    active: Arc<Mutex<HashMap<i64, Active>>>,
    terminals: Arc<Mutex<HashMap<i64, Arc<InteractiveTerminal>>>>,
    _project_lock: Arc<ProjectLock>,
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
    output: Arc<Mutex<TerminalOutput>>,
}

#[derive(Default)]
struct TerminalOutput {
    history: Vec<u8>,
    subscribers: Vec<TerminalSubscriber>,
    next_subscriber_id: u64,
    closed: bool,
}

struct TerminalSubscriber {
    id: u64,
    queue: Arc<TerminalSubscriberQueue>,
}

struct TerminalSubscriberQueue {
    state: Mutex<TerminalSubscriberQueueState>,
    notify: Notify,
}

#[derive(Default)]
struct TerminalSubscriberQueueState {
    chunks: VecDeque<Arc<[u8]>>,
    bytes: usize,
    closed: bool,
    overflowed: bool,
}

pub struct TerminalSubscription {
    pub snapshot: Vec<u8>,
    pub receiver: TerminalReceiver,
}

pub struct TerminalReceiver {
    queue: Arc<TerminalSubscriberQueue>,
    cleanup: Option<SubscriberCleanup>,
}

struct SubscriberCleanup {
    output: Weak<Mutex<TerminalOutput>>,
    id: u64,
}

enum TerminalPoll {
    Chunk(Arc<[u8]>),
    Empty,
    Closed,
}

#[cfg(test)]
enum TerminalTryRecvError {
    Empty,
    Disconnected,
}

#[derive(Debug)]
enum TerminalSubscribeError {
    Full,
}

impl TerminalSubscriberQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(TerminalSubscriberQueueState::default()),
            notify: Notify::new(),
        }
    }

    fn enqueue(&self, chunk: Arc<[u8]>) -> bool {
        let enqueued = {
            let mut state = self.state.lock().expect("terminal queue poisoned");
            if state.closed {
                return false;
            }
            let over_bytes = state
                .bytes
                .checked_add(chunk.len())
                .is_none_or(|bytes| bytes > MAX_TERMINAL_SUBSCRIBER_BYTES);
            let over_chunks = state.chunks.len() >= MAX_TERMINAL_SUBSCRIBER_CHUNKS;
            if chunk.len() > MAX_TERMINAL_SUBSCRIBER_BYTES || over_bytes || over_chunks {
                state.overflowed = true;
                state.closed = true;
                state.chunks.clear();
                state.bytes = 0;
                false
            } else {
                state.bytes += chunk.len();
                state.chunks.push_back(chunk);
                true
            }
        };
        self.notify.notify_one();
        enqueued
    }

    fn close(&self) {
        self.state.lock().expect("terminal queue poisoned").closed = true;
        self.notify.notify_one();
    }

    fn discard(&self) {
        {
            let mut state = self.state.lock().expect("terminal queue poisoned");
            state.closed = true;
            state.chunks.clear();
            state.bytes = 0;
        }
        self.notify.notify_one();
    }

    fn poll(&self) -> TerminalPoll {
        let mut state = self.state.lock().expect("terminal queue poisoned");
        if let Some(chunk) = state.chunks.pop_front() {
            state.bytes -= chunk.len();
            TerminalPoll::Chunk(chunk)
        } else if state.closed {
            TerminalPoll::Closed
        } else {
            TerminalPoll::Empty
        }
    }

    fn overflowed(&self) -> bool {
        self.state
            .lock()
            .expect("terminal queue poisoned")
            .overflowed
    }
}

impl TerminalReceiver {
    pub async fn recv(&mut self) -> Option<Arc<[u8]>> {
        loop {
            match self.queue.poll() {
                TerminalPoll::Chunk(chunk) => return Some(chunk),
                TerminalPoll::Closed => return None,
                TerminalPoll::Empty => self.queue.notify.notified().await,
            }
        }
    }

    pub fn overflowed(&self) -> bool {
        self.queue.overflowed()
    }

    #[cfg(test)]
    fn try_recv(&mut self) -> Result<Arc<[u8]>, TerminalTryRecvError> {
        match self.queue.poll() {
            TerminalPoll::Chunk(chunk) => Ok(chunk),
            TerminalPoll::Empty => Err(TerminalTryRecvError::Empty),
            TerminalPoll::Closed => Err(TerminalTryRecvError::Disconnected),
        }
    }
}

impl Drop for TerminalReceiver {
    fn drop(&mut self) {
        self.queue.discard();
        let Some(cleanup) = self.cleanup.take() else {
            return;
        };
        let Some(output) = cleanup.output.upgrade() else {
            return;
        };
        if let Ok(mut output) = output.lock() {
            output.remove_subscriber(cleanup.id);
        };
    }
}

impl TerminalOutput {
    fn publish(&mut self, chunk: &[u8]) {
        if self.closed || chunk.is_empty() {
            return;
        }
        self.history.extend_from_slice(chunk);
        if self.history.len() > MAX_TERMINAL_HISTORY_BYTES {
            let overflow = self.history.len() - MAX_TERMINAL_HISTORY_BYTES;
            self.history.drain(..overflow);
            // The retained byte window is later decoded as text, so drop a
            // leading incomplete UTF-8 fragment the cut may have produced
            // instead of letting it surface as a replacement character.
            let leading_continuation = self
                .history
                .iter()
                .take(3)
                .take_while(|byte| **byte & 0b1100_0000 == 0b1000_0000)
                .count();
            self.history.drain(..leading_continuation);
        }
        if self.subscribers.is_empty() {
            return;
        }
        let chunk: Arc<[u8]> = Arc::from(chunk);
        self.subscribers
            .retain(|subscriber| subscriber.queue.enqueue(chunk.clone()));
    }

    fn remove_subscriber(&mut self, id: u64) {
        if let Some(index) = self
            .subscribers
            .iter()
            .position(|subscriber| subscriber.id == id)
        {
            let subscriber = self.subscribers.remove(index);
            subscriber.queue.discard();
        }
    }

    fn close_subscribers(&mut self) {
        self.closed = true;
        for subscriber in self.subscribers.drain(..) {
            subscriber.queue.close();
        }
    }
}

fn subscribe_terminal_output(
    output: &Arc<Mutex<TerminalOutput>>,
) -> Result<TerminalSubscription, TerminalSubscribeError> {
    let mut state = output.lock().expect("terminal output poisoned");
    let snapshot = state.history.clone();
    if state.closed {
        let queue = Arc::new(TerminalSubscriberQueue::new());
        queue.close();
        return Ok(TerminalSubscription {
            snapshot,
            receiver: TerminalReceiver {
                queue,
                cleanup: None,
            },
        });
    }
    if state.subscribers.len() >= MAX_TERMINAL_SUBSCRIBERS {
        return Err(TerminalSubscribeError::Full);
    }
    let id = state.next_subscriber_id;
    state.next_subscriber_id = state
        .next_subscriber_id
        .checked_add(1)
        .expect("terminal subscriber id overflow");
    let queue = Arc::new(TerminalSubscriberQueue::new());
    state.subscribers.push(TerminalSubscriber {
        id,
        queue: queue.clone(),
    });
    Ok(TerminalSubscription {
        snapshot,
        receiver: TerminalReceiver {
            queue,
            cleanup: Some(SubscriberCleanup {
                output: Arc::downgrade(output),
                id,
            }),
        },
    })
}

impl Runtime {
    pub fn new(root: &Path) -> Result<Self, RuntimeError> {
        let canonical_root =
            std::fs::canonicalize(root).map_err(|source| RuntimeError::ProjectRoot {
                root: root.to_path_buf(),
                source,
            })?;
        if !canonical_root.join(FACTORY_DIR).join("db.sqlite3").exists() {
            return Err(FactoryError::NotInitialized.into());
        }
        let project_lock = ProjectLock::acquire(&canonical_root, root)?;
        Factory::open(root)?.reconcile_interrupted()?;
        Ok(Self {
            root: Arc::new(root.to_path_buf()),
            active: Arc::new(Mutex::new(HashMap::new())),
            terminals: Arc::new(Mutex::new(HashMap::new())),
            _project_lock: Arc::new(project_lock),
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
            operation: None,
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
            // Interactive consoles are human-driven; the policy engine applies
            // to automated sessions, so no policy audit is recorded here.
            policy_audit: None,
            output_meta: None,
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
            output: Arc::new(Mutex::new(TerminalOutput::default())),
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
        subscribe_terminal_output(&terminal.output).map_err(|TerminalSubscribeError::Full| {
            RuntimeError::TerminalSubscribersFull(session_id)
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

    pub fn create_workflow(
        &self,
        objective: &str,
        team: Option<WorkflowTeam>,
    ) -> Result<Run, RuntimeError> {
        self.create_workflow_with_recipe(objective, team, None)
    }

    /// Creates a workflow from a recipe snapshot: the recipe pre-fills the
    /// team and its configuration is persisted immutably on the run. The
    /// Planner still generates the actual DAG from the objective.
    pub fn create_workflow_with_recipe(
        &self,
        objective: &str,
        team: Option<WorkflowTeam>,
        recipe: Option<RecipeSnapshot>,
    ) -> Result<Run, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let run = factory.begin_run_with_recipe(objective, team, recipe.as_ref())?;
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

    /// Imports a GitHub Issue as a workflow and starts planning in the
    /// background. Mirrors [`create_workflow`](Self::create_workflow): the
    /// imported run is planned but never executed until the user starts it.
    pub fn import_workflow_from_issue(
        &self,
        reference: &str,
        team: Option<WorkflowTeam>,
    ) -> Result<Run, RuntimeError> {
        self.import_workflow_from_issue_with_recipe(reference, team, None)
    }

    /// GitHub Issue + Recipe: the same delivery lane as objective workflows.
    pub fn import_workflow_from_issue_with_recipe(
        &self,
        reference: &str,
        team: Option<WorkflowTeam>,
        recipe: Option<RecipeSnapshot>,
    ) -> Result<Run, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let run = factory.import_github_issue(reference, team, recipe.as_ref())?;
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

    pub fn start_workflow(&self, run_id: i64) -> Result<WorkflowTeam, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let cancel = self.reserve(run_id, OperationKind::Executing)?;
        let team = match factory.prepare_start(run_id) {
            Ok(team) => team,
            Err(error) => {
                self.release(run_id);
                return Err(error.into());
            }
        };
        self.spawn_execution(run_id, cancel);
        Ok(team)
    }

    pub fn update_workflow_team(
        &self,
        run_id: i64,
        team: WorkflowTeam,
    ) -> Result<WorkflowTeam, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        Ok(factory.update_run_team(run_id, team)?)
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
            pump_terminal_output(
                session_id,
                &output_terminal.output,
                db.as_ref(),
                &mut reader,
            );
            output_terminal
                .output
                .lock()
                .expect("terminal output poisoned")
                .close_subscribers();
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
                // The retained history window — trimmed to UTF-8 boundaries —
                // becomes the final transcript, enforcing the byte-window
                // retention policy over the incrementally appended text
                // without reintroducing invalid character boundaries.
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

/// Drains one PTY byte stream: forwards every raw chunk to subscribers and
/// history, and persists UTF-8 text decoded incrementally so characters split
/// across read boundaries reconstruct exactly. Incomplete trailing sequences
/// stay in the decoder across database batch flushes and are finalized only
/// when the stream definitively ends (EOF or a terminal read error).
fn pump_terminal_output(
    session_id: i64,
    output: &Arc<Mutex<TerminalOutput>>,
    db: Option<&FactoryDb>,
    reader: &mut dyn Read,
) {
    let mut buffer = [0u8; 8192];
    // Each append rewrites the full stored output row, so appending per
    // read chunk makes DB write cost dominate PTY streaming; batch them.
    const DB_APPEND_BATCH_BYTES: usize = 64 * 1024;
    const DB_APPEND_BATCH_INTERVAL: Duration = Duration::from_millis(250);
    let mut utf8_decoder = Utf8StreamDecoder::default();
    let mut pending_output = String::new();
    let mut last_append = Instant::now();
    fn flush_output(
        db: Option<&FactoryDb>,
        session_id: i64,
        pending_output: &mut String,
        last_append: &mut Instant,
    ) {
        if let Some(db) = db {
            if !pending_output.is_empty() {
                let _ = db.append_agent_session_output(session_id, Some(pending_output), None);
            }
        }
        pending_output.clear();
        *last_append = Instant::now();
    }
    fn finalize_decoder(
        db: Option<&FactoryDb>,
        session_id: i64,
        utf8_decoder: &mut Utf8StreamDecoder,
        pending_output: &mut String,
        last_append: &mut Instant,
    ) {
        pending_output.push_str(&utf8_decoder.finish());
        flush_output(db, session_id, pending_output, last_append);
    }
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                let chunk = buffer[..size].to_vec();
                output
                    .lock()
                    .expect("terminal output poisoned")
                    .publish(&chunk);
                if db.is_some() {
                    pending_output.push_str(&utf8_decoder.push(&chunk));
                    if pending_output.len() >= DB_APPEND_BATCH_BYTES
                        || last_append.elapsed() >= DB_APPEND_BATCH_INTERVAL
                    {
                        flush_output(db, session_id, &mut pending_output, &mut last_append);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                finalize_decoder(
                    db,
                    session_id,
                    &mut utf8_decoder,
                    &mut pending_output,
                    &mut last_append,
                );
                if let Some(db) = db {
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
    finalize_decoder(
        db,
        session_id,
        &mut utf8_decoder,
        &mut pending_output,
        &mut last_append,
    );
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::IsTerminal;

    use factory_agent::{AgentKind, PromptTransport};
    use factory_core::{AgentEntry, Config, Factory};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn pty_child_probe() {
        if std::env::var("FACTORY_PTY_CHILD").as_deref() == Ok("1") {
            println!("TTY={}", std::io::stdin().is_terminal());
            std::io::stdout().flush().unwrap();
            let _ = std::io::Read::read(&mut std::io::stdin(), &mut [0_u8; 1]);
            std::process::exit(0);
        }
    }

    #[test]
    fn pty_stream_child() {
        if std::env::var("FACTORY_PTY_STREAM").as_deref() != Ok("1") {
            return;
        }
        let mut input = [0_u8; 1];
        let mut stdout = std::io::stdout();
        stdout.write_all(b"STREAM_READY\r\n").unwrap();
        stdout.flush().unwrap();
        let _ = std::io::Read::read(&mut std::io::stdin(), &mut input);
        let block = vec![b'x'; 8192];
        for _ in 0..280 {
            stdout.write_all(&block).unwrap();
        }
        stdout.write_all(b"STREAM_DONE").unwrap();
        stdout.flush().unwrap();
        loop {
            if std::io::Read::read(&mut std::io::stdin(), &mut input).is_err() {
                std::process::exit(1);
            }
            if input[0] == b'r' {
                break;
            }
        }
        stdout.write_all(b"STREAM_ALIVE").unwrap();
        stdout.flush().unwrap();
        std::process::exit(0);
    }

    fn new_terminal_output() -> Arc<Mutex<TerminalOutput>> {
        Arc::new(Mutex::new(TerminalOutput::default()))
    }

    fn subscribe(output: &Arc<Mutex<TerminalOutput>>) -> TerminalSubscription {
        subscribe_terminal_output(output).unwrap()
    }

    fn publish(output: &Arc<Mutex<TerminalOutput>>, chunk: &[u8]) {
        output
            .lock()
            .expect("terminal output poisoned")
            .publish(chunk);
    }

    fn drain(receiver: &mut TerminalReceiver) -> Vec<u8> {
        let mut output = Vec::new();
        loop {
            match receiver.try_recv() {
                Ok(chunk) => output.extend_from_slice(&chunk),
                Err(TerminalTryRecvError::Empty | TerminalTryRecvError::Disconnected) => {
                    return output
                }
            }
        }
    }

    fn subscriber_count(output: &Arc<Mutex<TerminalOutput>>) -> usize {
        output
            .lock()
            .expect("terminal output poisoned")
            .subscribers
            .len()
    }

    #[test]
    fn terminal_subscribers_receive_identical_binary_output_in_order() {
        let output = new_terminal_output();
        let mut first = subscribe(&output);
        let mut second = subscribe(&output);
        let chunks: [&[u8]; 4] = [
            b"\x1b[31mred\x1b[0m",
            &[0, 1, 2, 3, 255],
            b"partial-\xe2\x94",
            b"\x80-complete\r\n",
        ];
        for chunk in chunks {
            publish(&output, chunk);
        }
        assert_eq!(drain(&mut first.receiver), chunks.concat());
        assert_eq!(drain(&mut second.receiver), chunks.concat());
        assert!(!first.receiver.overflowed());
        assert!(!second.receiver.overflowed());
    }

    #[test]
    fn terminal_snapshot_and_live_output_do_not_overlap() {
        let output = new_terminal_output();
        publish(&output, b"before-live");
        let mut subscription = subscribe(&output);
        assert_eq!(subscription.snapshot, b"before-live");
        publish(&output, b"-after-live");
        assert_eq!(drain(&mut subscription.receiver), b"-after-live");
        assert_eq!(
            output.lock().expect("terminal output poisoned").history,
            b"before-live-after-live"
        );
    }

    #[test]
    fn terminal_subscriber_queue_enforces_byte_capacity() {
        let output = new_terminal_output();
        let mut subscription = subscribe(&output);
        let chunk = vec![b'a'; 100_000];
        for _ in 0..20 {
            publish(&output, &chunk);
        }
        {
            let output = output.lock().expect("terminal output poisoned");
            let queue = output.subscribers[0]
                .queue
                .state
                .lock()
                .expect("terminal queue poisoned");
            assert_eq!(queue.bytes, 2_000_000);
            assert_eq!(queue.chunks.len(), 20);
        }
        publish(&output, &chunk);
        assert!(subscription.receiver.overflowed());
        assert_eq!(subscriber_count(&output), 0);
        assert!(drain(&mut subscription.receiver).is_empty());
    }

    #[test]
    fn oversized_terminal_chunk_is_rejected_without_truncation() {
        let output = new_terminal_output();
        let mut subscription = subscribe(&output);
        publish(&output, &vec![b'x'; MAX_TERMINAL_SUBSCRIBER_BYTES + 1]);
        assert!(subscription.receiver.overflowed());
        assert_eq!(subscriber_count(&output), 0);
        assert!(drain(&mut subscription.receiver).is_empty());
    }

    #[test]
    fn terminal_subscriber_queue_enforces_message_capacity() {
        let output = new_terminal_output();
        let subscription = subscribe(&output);
        for index in 0..MAX_TERMINAL_SUBSCRIBER_CHUNKS {
            publish(&output, &[index as u8]);
        }
        {
            let output = output.lock().expect("terminal output poisoned");
            let queue = output.subscribers[0]
                .queue
                .state
                .lock()
                .expect("terminal queue poisoned");
            assert_eq!(queue.chunks.len(), MAX_TERMINAL_SUBSCRIBER_CHUNKS);
            assert_eq!(queue.bytes, MAX_TERMINAL_SUBSCRIBER_CHUNKS);
        }
        publish(&output, b"overflow");
        assert!(subscription.receiver.overflowed());
        assert_eq!(subscriber_count(&output), 0);
    }

    #[test]
    fn slow_terminal_subscriber_is_evicted_while_healthy_subscriber_continues() {
        let output = new_terminal_output();
        let mut slow = subscribe(&output);
        let mut healthy = subscribe(&output);
        let chunk = vec![b'z'; 8192];
        let mut received = Vec::new();
        for _ in 0..480 {
            publish(&output, &chunk);
            received.extend_from_slice(&drain(&mut healthy.receiver));
        }
        assert!(slow.receiver.overflowed());
        assert!(!healthy.receiver.overflowed());
        assert_eq!(subscriber_count(&output), 1);
        assert_eq!(received.len(), 480 * 8192);
        assert!(received.iter().all(|byte| *byte == b'z'));
        assert!(drain(&mut slow.receiver).is_empty());
    }

    #[test]
    fn dropping_terminal_subscription_releases_its_queue() {
        let output = new_terminal_output();
        let subscription = subscribe(&output);
        publish(&output, &vec![b'q'; 1_000_000]);
        assert_eq!(subscriber_count(&output), 1);
        drop(subscription);
        assert_eq!(subscriber_count(&output), 0);
    }

    #[test]
    fn terminal_close_drains_queued_output_before_disconnecting() {
        let output = new_terminal_output();
        let mut subscription = subscribe(&output);
        publish(&output, b"final-output");
        output
            .lock()
            .expect("terminal output poisoned")
            .close_subscribers();
        assert_eq!(drain(&mut subscription.receiver), b"final-output");
        assert!(matches!(
            subscription.receiver.try_recv(),
            Err(TerminalTryRecvError::Disconnected)
        ));
        assert!(!subscription.receiver.overflowed());
    }

    #[test]
    fn subscribing_after_terminal_close_returns_the_snapshot_and_disconnects() {
        let output = new_terminal_output();
        publish(&output, b"final-output");
        output
            .lock()
            .expect("terminal output poisoned")
            .close_subscribers();
        publish(&output, b"ignored-after-close");
        let mut subscription = subscribe(&output);
        assert_eq!(subscription.snapshot, b"final-output");
        assert_eq!(subscriber_count(&output), 0);
        assert!(matches!(
            subscription.receiver.try_recv(),
            Err(TerminalTryRecvError::Disconnected)
        ));
        assert!(!subscription.receiver.overflowed());
    }

    #[test]
    fn reconnect_receives_only_the_bounded_history_window() {
        let output = new_terminal_output();
        let all: Vec<u8> = (0..2_000_000).map(|index| (index % 251) as u8).collect();
        publish(&output, &all);
        let subscription = subscribe(&output);
        assert_eq!(subscription.snapshot.len(), MAX_TERMINAL_HISTORY_BYTES);
        assert_eq!(
            subscription.snapshot,
            all[all.len() - MAX_TERMINAL_HISTORY_BYTES..]
        );
    }

    #[test]
    fn terminal_history_remains_bounded_under_high_volume() {
        let output = new_terminal_output();
        let chunk = vec![b'h'; 8192];
        for _ in 0..500 {
            publish(&output, &chunk);
        }
        let history = output
            .lock()
            .expect("terminal output poisoned")
            .history
            .len();
        assert_eq!(history, MAX_TERMINAL_HISTORY_BYTES);
    }

    #[test]
    fn terminal_subscriber_limit_is_enforced_and_releases_on_drop() {
        let output = new_terminal_output();
        let mut subscriptions = Vec::new();
        for _ in 0..MAX_TERMINAL_SUBSCRIBERS {
            subscriptions.push(subscribe(&output));
        }
        assert!(matches!(
            subscribe_terminal_output(&output),
            Err(TerminalSubscribeError::Full)
        ));
        subscriptions.pop();
        let replacement = subscribe_terminal_output(&output).unwrap();
        assert_eq!(subscriber_count(&output), MAX_TERMINAL_SUBSCRIBERS);
        drop(replacement);
        drop(subscriptions);
        assert_eq!(subscriber_count(&output), 0);
    }

    #[test]
    fn concurrent_terminal_subscription_and_publication_do_not_deadlock() {
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering::Relaxed;

        let output = new_terminal_output();
        let done = Arc::new(AtomicBool::new(false));
        let publisher_output = output.clone();
        let publisher_done = done.clone();
        let publisher = std::thread::spawn(move || {
            let chunk = vec![b'p'; 8192];
            for _ in 0..2_000 {
                publish(&publisher_output, &chunk);
            }
            publisher_done.store(true, Relaxed);
        });
        while !done.load(Relaxed) {
            let subscription = subscribe(&output);
            publish(&output, b"racing-output");
            drop(subscription);
        }
        publisher.join().unwrap();
        assert_eq!(subscriber_count(&output), 0);
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
                max_concurrency: None,
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
                output.extend_from_slice(&chunk);
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
                max_concurrency: None,
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
                output.extend_from_slice(&chunk);
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

    #[test]
    fn slow_terminal_subscriber_does_not_stop_the_pty_or_healthy_stream() {
        let dir = TempDir::new().unwrap();
        Factory::init(dir.path()).unwrap();
        let test_executable = std::env::current_exe().unwrap();
        #[cfg(windows)]
        let command = {
            let bin = dir.path().join("fake-stream-bin");
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
            "tests::pty_stream_child".into(),
            "--nocapture".into(),
        ];
        let mut config = Config::default();
        config.agents.insert(
            "terminal-stream-test".into(),
            AgentEntry {
                kind: Some(AgentKind::Custom),
                command,
                args: Vec::new(),
                env: BTreeMap::from([("FACTORY_PTY_STREAM".into(), "1".into())]),
                prompt_transport: Some(PromptTransport::Disabled),
                interactive_args: Some(interactive_args),
                capabilities: Vec::new(),
                max_concurrency: None,
            },
        );
        config.write_atomic(dir.path()).unwrap();

        let runtime = Runtime::new(dir.path()).unwrap();
        let session = runtime
            .start_interactive_session("terminal-stream-test", 90, 24)
            .unwrap();
        let slow = runtime.subscribe_terminal(session.id).unwrap();
        let mut healthy = runtime.subscribe_terminal(session.id).unwrap();

        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        let mut output = healthy.snapshot;
        let mut answered_cursor_query = false;
        let mut started = false;
        let mut resumed = false;
        loop {
            let mut received = 0;
            while let Ok(chunk) = healthy.receiver.try_recv() {
                output.extend_from_slice(&chunk);
                received += 1;
            }
            if !answered_cursor_query && output.windows(4).any(|window| window == b"\x1b[6n") {
                runtime.write_terminal(session.id, b"\x1b[1;1R").unwrap();
                answered_cursor_query = true;
            }
            if !started && output.windows(12).any(|window| window == b"STREAM_READY") {
                runtime.write_terminal(session.id, b"g\r").unwrap();
                started = true;
            }
            if output.windows(12).any(|window| window == b"STREAM_ALIVE") {
                break;
            }
            if !resumed && output.windows(11).any(|window| window == b"STREAM_DONE") {
                runtime.write_terminal(session.id, b"r\r").unwrap();
                resumed = true;
            }
            assert!(
                Instant::now() < deadline,
                "PTY stream did not complete; received {} bytes: {:?}",
                output.len(),
                String::from_utf8_lossy(&output)
            );
            if received == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        assert!(started);
        assert!(slow.receiver.overflowed());
        assert!(!healthy.receiver.overflowed());
        assert!(output.iter().filter(|byte| **byte == b'x').count() >= 280 * 8192);
        assert!(resumed);

        let close_deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match healthy.receiver.try_recv() {
                Ok(_) => {}
                Err(TerminalTryRecvError::Disconnected) => break,
                Err(TerminalTryRecvError::Empty) => {
                    assert!(
                        Instant::now() < close_deadline,
                        "terminal subscription did not close after process exit"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }

        let db = FactoryDb::open(&dir.path().join(".factory").join("db.sqlite3")).unwrap();
        let mut saved = db.get_agent_session(session.id).unwrap().unwrap();
        for _ in 0..200 {
            if saved.status != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
            saved = db.get_agent_session(session.id).unwrap().unwrap();
        }
        assert_eq!(saved.status, "success");
        assert!(saved.stdout.unwrap_or_default().contains("STREAM_ALIVE"));
        drop(slow.receiver);
    }

    /// Yields predetermined byte slices so UTF-8 sequences are split across
    /// read calls deterministically, unlike operating-system read boundaries.
    /// Chunks larger than the caller's buffer are returned across successive
    /// reads, mirroring real reader semantics.
    struct SplittingReader {
        chunks: std::collections::VecDeque<Vec<u8>>,
    }

    impl SplittingReader {
        fn byte_by_byte(payload: &[u8]) -> Self {
            Self {
                chunks: payload.iter().map(|byte| vec![*byte]).collect(),
            }
        }

        fn from_chunks(chunks: &[&[u8]]) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
            }
        }
    }

    impl Read for SplittingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            while let Some(chunk) = self.chunks.front_mut() {
                if chunk.is_empty() {
                    self.chunks.pop_front();
                    continue;
                }
                let size = chunk.len().min(buffer.len());
                buffer[..size].copy_from_slice(&chunk[..size]);
                if size == chunk.len() {
                    self.chunks.pop_front();
                } else {
                    let remainder = chunk.split_off(size);
                    self.chunks[0] = remainder;
                }
                return Ok(size);
            }
            Ok(0)
        }
    }

    struct FailingReader {
        chunks: std::collections::VecDeque<Vec<u8>>,
    }

    impl Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let Some(chunk) = self.chunks.front_mut() else {
                return Err(std::io::Error::other("synthetic PTY failure"));
            };
            let size = chunk.len().min(buffer.len());
            buffer[..size].copy_from_slice(&chunk[..size]);
            if size == chunk.len() {
                self.chunks.pop_front();
            } else {
                let remainder = chunk.split_off(size);
                self.chunks[0] = remainder;
            }
            Ok(size)
        }
    }

    fn transcript_session(db: &FactoryDb) -> i64 {
        let session = AgentSession {
            id: 0,
            run_id: None,
            task_id: None,
            attempt_id: None,
            role: "console".into(),
            operation: None,
            agent: "test-agent".into(),
            mode: AgentSessionMode::Interactive,
            command: "test-agent".into(),
            status: "running".into(),
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            exit_code: None,
            duration_ms: None,
            stdout: Some(String::new()),
            stderr: Some(String::new()),
            policy_audit: None,
            output_meta: None,
        };
        db.insert_agent_session(&session).unwrap().id
    }

    fn session_db() -> (TempDir, FactoryDb) {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".factory")).unwrap();
        let db = FactoryDb::open(&dir.path().join(".factory/db.sqlite3")).unwrap();
        (dir, db)
    }

    fn persisted_stdout(db: &FactoryDb, session_id: i64) -> String {
        db.get_agent_session(session_id)
            .unwrap()
            .unwrap()
            .stdout
            .unwrap_or_default()
    }

    #[test]
    fn terminal_transcript_reconstructs_multibyte_characters_split_across_reads() {
        let payload = "éà日本語🎉🚀\r\n\x1b[31mansi\x1b[0m ✓\r\n";
        for split_mode in 0..2 {
            let (_dir, db) = session_db();
            let session_id = transcript_session(&db);
            let output = new_terminal_output();
            let mut reader = if split_mode == 0 {
                SplittingReader::byte_by_byte(payload.as_bytes())
            } else {
                SplittingReader::from_chunks(&[
                    b"plain start, then ",
                    &payload.as_bytes()[17..19],
                    &payload.as_bytes()[19..21],
                    &payload.as_bytes()[21..23],
                    b" tail",
                ])
            };
            pump_terminal_output(session_id, &output, Some(&db), &mut reader);
            let transcript = persisted_stdout(&db, session_id);
            if split_mode == 0 {
                assert_eq!(
                    transcript, payload,
                    "byte-by-byte reads must not corrupt text"
                );
            } else {
                assert!(transcript.starts_with("plain start, then "));
                assert!(transcript.ends_with(" tail"));
            }
            assert!(!transcript.contains('\u{FFFD}'));
            assert_eq!(
                String::from_utf8(output.lock().unwrap().history.clone()).unwrap(),
                transcript,
                "persisted transcript must match the raw history"
            );
        }
    }

    #[test]
    fn chunk_local_lossy_decoding_would_have_corrupted_the_same_stream() {
        let payload = "🎉";
        let (_dir, db) = session_db();
        let session_id = transcript_session(&db);
        let mut legacy = String::new();
        for byte in payload.as_bytes() {
            legacy.push_str(&String::from_utf8_lossy(&[*byte]));
        }
        assert_ne!(legacy, payload, "fixture must reproduce the original bug");
        assert!(legacy.contains('\u{FFFD}'));

        let output = new_terminal_output();
        let mut reader = SplittingReader::byte_by_byte(payload.as_bytes());
        pump_terminal_output(session_id, &output, Some(&db), &mut reader);
        assert_eq!(persisted_stdout(&db, session_id), payload);
    }

    #[test]
    fn terminal_transcript_survives_read_errors_and_flushes_incomplete_prefix() {
        let (_dir, db) = session_db();
        let session_id = transcript_session(&db);
        let output = new_terminal_output();
        let mut reader = FailingReader {
            chunks: std::collections::VecDeque::from(vec![b"ok ".to_vec(), vec![0xf0, 0x9f]]),
        };
        pump_terminal_output(session_id, &output, Some(&db), &mut reader);
        let session = db.get_agent_session(session_id).unwrap().unwrap();
        assert_eq!(session.stdout.unwrap_or_default(), "ok \u{FFFD}");
        assert!(session
            .stderr
            .unwrap_or_default()
            .contains("PTY read failed: synthetic PTY failure"));
    }

    #[test]
    fn terminal_transcript_keeps_characters_whole_across_batch_flushes() {
        let (_dir, db) = session_db();
        let session_id = transcript_session(&db);
        let output = new_terminal_output();
        let mut prefix = vec![b'>'; 70 * 1024];
        prefix.extend_from_slice("🚀完".as_bytes());
        let mut reader = SplittingReader::from_chunks(&[
            &prefix[..40 * 1024],
            &prefix[40 * 1024..70 * 1024 + 4],
            &prefix[70 * 1024 + 4..],
        ]);
        pump_terminal_output(session_id, &output, Some(&db), &mut reader);
        let transcript = persisted_stdout(&db, session_id);
        assert_eq!(transcript.len(), prefix.len());
        assert!(transcript.ends_with("🚀完"));
        assert!(!transcript.contains('\u{FFFD}'));
    }

    #[test]
    fn terminal_history_trimming_lands_on_utf8_boundaries() {
        let output = new_terminal_output();
        let filler = "日".repeat(1024);
        for _ in 0..((MAX_TERMINAL_HISTORY_BYTES / filler.len()) + 2) {
            publish(&output, filler.as_bytes());
        }
        let history = output.lock().unwrap().history.clone();
        assert!(
            history.len() <= MAX_TERMINAL_HISTORY_BYTES,
            "retention limit must hold"
        );
        assert!(
            std::str::from_utf8(&history).is_ok(),
            "retained window must start on a character boundary"
        );
    }

    #[test]
    fn terminal_completion_snapshot_matches_incrementally_persisted_text() {
        let (_dir, db) = session_db();
        let session_id = transcript_session(&db);
        let output = new_terminal_output();
        let payload = format!("{}🎉", "é".repeat(90_000));
        let mut reader = SplittingReader::from_chunks(&[
            &payload.as_bytes()[..100_001],
            &payload.as_bytes()[100_001..],
        ]);
        pump_terminal_output(session_id, &output, Some(&db), &mut reader);
        let incremental = persisted_stdout(&db, session_id);
        let snapshot = String::from_utf8(output.lock().unwrap().history.clone()).unwrap();
        let _ = db.set_agent_session_output(session_id, Some(&snapshot), None);
        assert_eq!(incremental, payload);
        assert_eq!(persisted_stdout(&db, session_id), incremental);
        assert!(!incremental.contains('\u{FFFD}'));
    }
}
