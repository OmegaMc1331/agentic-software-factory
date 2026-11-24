use std::collections::{BTreeMap, VecDeque};
use std::io::{BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::{environment, ToolProfile};
use crate::{framing, IdeError, Result};

type Reply = mpsc::SyncSender<Result<Value>>;

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub sequence: u64,
    pub message: Value,
}
#[derive(Clone, Debug, Serialize)]
pub struct EventPage {
    pub events: Vec<Event>,
    pub cursor: u64,
    pub gap: bool,
    pub alive: bool,
}

pub struct Peer {
    child: Mutex<Option<Child>>,
    sender: mpsc::SyncSender<Value>,
    pending: Mutex<BTreeMap<u64, Reply>>,
    next: AtomicU64,
    events: Mutex<VecDeque<Event>>,
    event_sequence: AtomicU64,
    alive: AtomicBool,
    pub pid: u32,
    pub timeout: Duration,
    pub dap: bool,
    endpoint: Option<u16>,
    target_sessions: bool,
    target_requested: AtomicBool,
    terminating: AtomicBool,
    target: Mutex<Option<Arc<Peer>>>,
    parent: Option<Weak<Peer>>,
    initialization: Mutex<Value>,
    breakpoints: Mutex<BTreeMap<String, Value>>,
    settings: Value,
    #[cfg(windows)]
    job: Option<factory_proc::windows_job::JobGuard>,
}

impl Peer {
    pub fn spawn(profile: &ToolProfile, root: &std::path::Path, dap: bool) -> Result<Arc<Self>> {
        profile.verify(if dap { "dap" } else { "lsp" })?;
        let mut command = Command::new(factory_git::git_path(&profile.executable));
        let port = if profile.transport == "tcp" {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
            Some(listener.local_addr()?.port())
        } else {
            None
        };
        let arguments: Vec<_> = profile
            .arguments
            .iter()
            .map(|arg| arg.replace("${port}", &port.unwrap_or(0).to_string()))
            .collect();
        command
            .args(&arguments)
            .current_dir(factory_git::git_path(root))
            .env_clear()
            .envs(environment())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|_| IdeError::Executable)?;
        #[cfg(windows)]
        let job = match factory_proc::windows_job::JobGuard::assign(&child) {
            Some(job) => job,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(IdeError::Forbidden);
            }
        };
        let input = child.stdin.take().ok_or(IdeError::Exited)?;
        let output = child.stdout.take().ok_or(IdeError::Exited)?;
        let stderr = child.stderr.take().ok_or(IdeError::Exited)?;
        let (input, output): (Box<dyn Write + Send>, Box<dyn Read + Send>) =
            if let Some(port) = port {
                let started = std::time::Instant::now();
                let stream = loop {
                    match std::net::TcpStream::connect(("127.0.0.1", port)) {
                        Ok(stream) => break stream,
                        Err(_)
                            if started.elapsed() < Duration::from_millis(profile.timeout_ms)
                                && child.try_wait()?.is_none() =>
                        {
                            std::thread::sleep(Duration::from_millis(20))
                        }
                        Err(_) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(IdeError::Timeout);
                        }
                    }
                };
                stream.set_nodelay(true)?;
                drop(input);
                std::thread::spawn(move || {
                    let mut output = output;
                    let _ = std::io::copy(&mut output, &mut std::io::sink());
                });
                (Box::new(stream.try_clone()?), Box::new(stream))
            } else {
                (Box::new(input), Box::new(output))
            };
        let (sender, receiver) = mpsc::sync_channel(128);
        let peer = Arc::new(Self {
            pid: child.id(),
            child: Mutex::new(Some(child)),
            sender,
            pending: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            events: Mutex::new(VecDeque::new()),
            event_sequence: AtomicU64::new(0),
            alive: AtomicBool::new(true),
            timeout: Duration::from_millis(profile.timeout_ms),
            dap,
            endpoint: port,
            target_sessions: profile.settings["target_sessions"] == true,
            target_requested: AtomicBool::new(false),
            terminating: AtomicBool::new(false),
            target: Mutex::new(None),
            parent: None,
            initialization: Mutex::new(Value::Null),
            breakpoints: Mutex::new(BTreeMap::new()),
            settings: profile.settings.clone(),
            #[cfg(windows)]
            job: Some(job),
        });
        wire(&peer, input, output, receiver);
        let weak = Arc::downgrade(&peer);
        std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut bytes = [0; 4096];
            let mut discarded = 0u64;
            while let Ok(size) = stderr.read(&mut bytes) {
                if size == 0 {
                    break;
                }
                discarded += size as u64;
                if let Some(peer) = weak.upgrade() {
                    if discarded <= 4096 {
                        peer.emit(json!({"type":"tool_stderr","bytes":size,"message":"Tool wrote to stderr; raw output withheld from IDE events"}));
                    }
                } else {
                    break;
                }
            }
        });
        let weak = Arc::downgrade(&peer);
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let Some(peer) = weak.upgrade() else {
                    break;
                };
                let exited = peer
                    .child
                    .lock()
                    .ok()
                    .and_then(|mut child| {
                        child
                            .as_mut()
                            .and_then(|child| child.try_wait().ok().flatten())
                    })
                    .is_some();
                if exited || started.elapsed() > Duration::from_secs(7200) {
                    peer.failed();
                    break;
                }
                if !peer.is_alive() {
                    break;
                }
            }
        });
        Ok(peer)
    }

    pub fn emit(&self, message: Value) {
        if let Some(parent) = self.parent.as_ref().and_then(Weak::upgrade) {
            parent.emit(message.clone());
        }
        let sequence = self.event_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut events) = self.events.lock() {
            events.push_back(Event { sequence, message });
            let mut bytes: usize = events
                .iter()
                .map(|event| event.message.to_string().len())
                .sum();
            while events.len() > 128 || bytes > 4 * 1024 * 1024 {
                if let Some(removed) = events.pop_front() {
                    bytes = bytes.saturating_sub(removed.message.to_string().len());
                } else {
                    break;
                }
            }
        }
    }

    pub fn events(&self, cursor: u64) -> EventPage {
        let events = self
            .events
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        EventPage {
            gap: events.front().is_some_and(|e| cursor + 1 < e.sequence),
            events: events
                .iter()
                .filter(|event| event.sequence > cursor)
                .cloned()
                .collect(),
            cursor: self.event_sequence.load(Ordering::SeqCst),
            alive: self.is_alive(),
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_cancellable(method, params, None)
    }

    pub fn request_cancellable(
        &self,
        method: &str,
        params: Value,
        cancelled: Option<&AtomicBool>,
    ) -> Result<Value> {
        self.exchange(method, params, cancelled, self.timeout)
    }
    pub fn request_deadline(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.exchange(method, params, None, timeout.min(self.timeout))
    }
    fn exchange(
        &self,
        method: &str,
        params: Value,
        cancelled: Option<&AtomicBool>,
        timeout: Duration,
    ) -> Result<Value> {
        if self.dap
            && [
                "threads",
                "stackTrace",
                "scopes",
                "variables",
                "continue",
                "pause",
                "next",
                "stepIn",
                "stepOut",
                "evaluate",
                "exceptionInfo",
                "setBreakpoints",
            ]
            .contains(&method)
        {
            let target = self.target.lock().map_err(|_| IdeError::Conflict)?.clone();
            if let Some(target) = target {
                return target.request(method, params);
            }
        }
        if !self.is_alive() {
            return Err(IdeError::Exited);
        }
        if method == "initialize" {
            *self.initialization.lock().map_err(|_| IdeError::Conflict)? = params.clone();
        }
        if self.dap && method == "setBreakpoints" {
            let key = params["source"]["path"]
                .as_str()
                .ok_or(IdeError::Invalid)?
                .to_owned();
            self.breakpoints
                .lock()
                .map_err(|_| IdeError::Conflict)?
                .insert(key, params.clone());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (reply, receiver) = mpsc::sync_channel(1);
        {
            let mut pending = self.pending.lock().map_err(|_| IdeError::Exited)?;
            if pending.len() >= 64 {
                return Err(IdeError::Limit);
            }
            pending.insert(id, reply);
        }
        let value = if self.dap {
            json!({"seq":id,"type":"request","command":method,"arguments":params})
        } else {
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        };
        if self.send(value).is_err() {
            self.pending
                .lock()
                .map_err(|_| IdeError::Exited)?
                .remove(&id);
            return Err(IdeError::Exited);
        }
        let started = std::time::Instant::now();
        let result = loop {
            if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                break Err(IdeError::Cancelled);
            }
            if started.elapsed() >= timeout {
                break Err(IdeError::Timeout);
            }
            match receiver.recv_timeout(
                Duration::from_millis(50).min(timeout.saturating_sub(started.elapsed())),
            ) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err(IdeError::Exited),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        };
        self.pending
            .lock()
            .map_err(|_| IdeError::Exited)?
            .remove(&id);
        if matches!(result, Err(IdeError::Timeout | IdeError::Cancelled)) && !self.dap {
            let _ = self.notify("$/cancelRequest", json!({"id":id}));
        }
        result
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }

    fn send(&self, value: Value) -> Result<()> {
        if serde_json::to_vec(&value)
            .map_err(|_| IdeError::Protocol)?
            .len()
            > framing::MAX_MESSAGE
        {
            return Err(IdeError::Limit);
        }
        self.sender.try_send(value).map_err(|_| IdeError::Limit)
    }

    fn received(self: &Arc<Self>, value: Value) -> Result<()> {
        if !self.dap
            && (value["jsonrpc"] != "2.0"
                || value
                    .get("method")
                    .is_some_and(|method| !method.is_string())
                || value
                    .get("id")
                    .is_some_and(|id| !id.is_number() && !id.is_string() && !id.is_null()))
        {
            return Err(IdeError::Protocol);
        }
        if self.dap
            && (!value["seq"].is_u64()
                || !["request", "response", "event"]
                    .contains(&value["type"].as_str().unwrap_or("")))
        {
            return Err(IdeError::Protocol);
        }
        if self.dap && value["type"] == "response"
            || !self.dap && value.get("method").is_none() && value.get("id").is_some()
        {
            let id = if self.dap {
                &value["request_seq"]
            } else {
                &value["id"]
            };
            let id = id.as_u64().ok_or(IdeError::Protocol)?;
            if let Some(reply) = self
                .pending
                .lock()
                .map_err(|_| IdeError::Exited)?
                .remove(&id)
            {
                let result = if self.dap {
                    if value["success"] == true {
                        Ok(value["body"].clone())
                    } else {
                        Err(IdeError::Tool(safe_message(
                            value["message"]
                                .as_str()
                                .unwrap_or("debug adapter rejected request"),
                        )))
                    }
                } else if value.get("error").is_some() {
                    Err(IdeError::Tool(safe_message(
                        value["error"]["message"]
                            .as_str()
                            .unwrap_or("language server rejected request"),
                    )))
                } else {
                    Ok(value["result"].clone())
                };
                let _ = reply.try_send(result);
            }
        } else if !self.dap && value.get("id").is_some() {
            let id = value["id"].clone();
            let response = match value["method"].as_str().unwrap_or("") {
                "workspace/configuration" => {
                    json!({"jsonrpc":"2.0","id":id,"result":value["params"]["items"].as_array().map(|items|items.iter().take(128).map(|item|{let mut result=&self.settings;for key in item["section"].as_str().unwrap_or("").split('.').filter(|key|!key.is_empty()){result=&result[key];}result.clone()}).collect::<Vec<_>>())})
                }
                "workspace/workspaceFolders" => {
                    json!({"jsonrpc":"2.0","id":id,"result":self.initialization.lock().map_err(|_|IdeError::Conflict)?["workspaceFolders"]})
                }
                "window/workDoneProgress/create" => json!({"jsonrpc":"2.0","id":id,"result":null}),
                "workspace/applyEdit" => {
                    self.emit(json!({"method":"asf/workspaceEdit","params":value["params"]}));
                    json!({"jsonrpc":"2.0","id":id,"result":{"applied":false,"failureReason":"ASF requires explicit review of workspace edits"}})
                }
                _ => {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Server request not authorized"}})
                }
            };
            self.send(response)?;
        } else if self.dap && value["type"] == "request" {
            if value["command"] == "startDebugging"
                && self.target_sessions
                && self.endpoint.is_some()
                && !self.target_requested.swap(true, Ordering::AcqRel)
            {
                let parent = self.clone();
                std::thread::spawn(move || {
                    let result = parent.start_target(&value["arguments"]);
                    let _=parent.send(json!({"seq":parent.next.fetch_add(1,Ordering::SeqCst),"type":"response","request_seq":value["seq"],"command":value["command"],"success":result.is_ok(),"message":result.err().map(|e|e.to_string())}));
                });
            } else {
                self.emit(json!({"type":"asf_reverse_request_refused","command":value["command"]}));
                self.send(json!({"seq":self.next.fetch_add(1,Ordering::SeqCst),"type":"response","request_seq":value["seq"],"command":value["command"],"success":false,"message":"Reverse process launch is not authorized"}))?;
            }
        } else {
            let terminated = self.dap && value["event"] == "terminated";
            self.emit(value);
            if terminated {
                let owner = self
                    .parent
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .unwrap_or_else(|| self.clone());
                if !owner.terminating.swap(true, Ordering::AcqRel) {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(100));
                        owner.failed();
                    });
                }
            }
        }
        Ok(())
    }

    fn failed(&self) {
        self.failed_with(IdeError::Exited)
    }
    fn failed_with(&self, error: IdeError) {
        if self.alive.swap(false, Ordering::SeqCst) {
            self.emit(json!({"type":"asf_exited","reason":error.to_string()}));
            if let Ok(mut pending) = self.pending.lock() {
                for (_, reply) in std::mem::take(&mut *pending) {
                    let _ = reply.try_send(Err(error.clone()));
                }
            }
            self.kill();
        }
    }

    pub fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Ok(mut pending) = self.pending.lock() {
            for (_, reply) in std::mem::take(&mut *pending) {
                let _ = reply.try_send(Err(IdeError::Exited));
            }
        }
        if let Ok(target) = self.target.lock() {
            if let Some(target) = target.as_ref() {
                target.kill();
            }
        }
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(self.pid as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    fn start_target(self: &Arc<Self>, arguments: &Value) -> Result<()> {
        let configuration = &arguments["configuration"];
        let object = configuration.as_object().ok_or(IdeError::Forbidden)?;
        if object
            .keys()
            .any(|key| !["type", "name", "__pendingTargetId"].contains(&key.as_str()))
            || configuration["__pendingTargetId"]
                .as_str()
                .is_none_or(|s| s.is_empty() || s.len() > 200)
            || !["launch", "attach"].contains(&arguments["request"].as_str().unwrap_or(""))
        {
            return Err(IdeError::Forbidden);
        }
        let mut target_slot = self.target.lock().map_err(|_| IdeError::Conflict)?;
        if target_slot.is_some() {
            return Err(IdeError::Limit);
        }
        let stream =
            std::net::TcpStream::connect(("127.0.0.1", self.endpoint.ok_or(IdeError::Forbidden)?))?;
        stream.set_nodelay(true)?;
        let input = Box::new(stream.try_clone()?);
        let output = Box::new(stream);
        let (sender, receiver) = mpsc::sync_channel(128);
        let child = Arc::new(Self {
            child: Mutex::new(None),
            sender,
            pending: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            events: Mutex::new(VecDeque::new()),
            event_sequence: AtomicU64::new(0),
            alive: AtomicBool::new(true),
            pid: self.pid,
            timeout: self.timeout,
            dap: true,
            endpoint: None,
            target_sessions: false,
            target_requested: AtomicBool::new(false),
            terminating: AtomicBool::new(false),
            target: Mutex::new(None),
            parent: Some(Arc::downgrade(self)),
            initialization: Mutex::new(Value::Null),
            breakpoints: Mutex::new(BTreeMap::new()),
            settings: self.settings.clone(),
            #[cfg(windows)]
            job: None,
        });
        wire(&child, input, output, receiver);
        *target_slot = Some(child.clone());
        drop(target_slot);
        child.request(
            "initialize",
            self.initialization
                .lock()
                .map_err(|_| IdeError::Conflict)?
                .clone(),
        )?;
        let launch_child = child.clone();
        let request = arguments["request"].as_str().unwrap_or("launch").to_owned();
        let configuration = configuration.clone();
        let launching = std::thread::spawn(move || launch_child.request(&request, configuration));
        let started = std::time::Instant::now();
        while !child
            .events(0)
            .events
            .iter()
            .any(|e| e.message["event"] == "initialized")
        {
            if started.elapsed() > self.timeout || !child.is_alive() {
                child.kill();
                return Err(IdeError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let points = self
            .breakpoints
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for points in points {
            child.request("setBreakpoints", points)?;
        }
        child.request("configurationDone", json!({}))?;
        launching.join().map_err(|_| IdeError::Exited)??;
        Ok(())
    }
}

fn wire(
    peer: &Arc<Peer>,
    mut input: Box<dyn Write + Send>,
    output: Box<dyn Read + Send>,
    receiver: mpsc::Receiver<Value>,
) {
    let weak = Arc::downgrade(peer);
    std::thread::spawn(move || {
        while let Ok(message) = receiver.recv() {
            if framing::write_message(&mut input, &message).is_err() {
                break;
            }
        }
        if let Some(peer) = weak.upgrade() {
            peer.failed();
        }
    });
    let weak = Arc::downgrade(peer);
    std::thread::spawn(move || read_loop(weak, BufReader::new(output)));
}

fn read_loop(peer: Weak<Peer>, mut reader: impl std::io::BufRead) {
    loop {
        let message = framing::read_message(&mut reader);
        let Some(peer) = peer.upgrade() else {
            break;
        };
        match message {
            Ok(message) => {
                if peer.received(message).is_err() {
                    peer.failed();
                    break;
                }
            }
            Err(error) => {
                peer.failed_with(error);
                break;
            }
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.kill();
    }
}

fn safe_message(message: &str) -> String {
    let mut message: String = message
        .chars()
        .filter(|c| !c.is_control())
        .take(500)
        .collect();
    for (key, value) in std::env::vars() {
        let key = key.to_ascii_uppercase();
        if value.len() >= 4
            && ["TOKEN", "SECRET", "PASSWORD", "API_KEY"]
                .iter()
                .any(|word| key.contains(word))
        {
            message = message.replace(&value, "[redacted]");
        }
    }
    message
}
