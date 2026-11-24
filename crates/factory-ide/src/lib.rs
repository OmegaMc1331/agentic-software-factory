mod config;
pub mod dap;
mod error;
pub mod extensions;
pub mod framing;
pub mod lsp;
pub mod peer;
pub mod position;
mod store;

use factory_files::FileService;
use factory_runtime::{DevelopmentTerminal, ResourcePermit, RuntimeBudget};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub use config::ToolProfile;
pub use error::{IdeError, Result};
pub use store::Store as IdeStore;

struct Owned<T> {
    value: Arc<T>,
    _permit: ResourcePermit,
}
#[derive(Clone, Debug, Serialize)]
pub struct TerminalInfo {
    pub id: String,
    pub root_id: String,
    pub profile_id: String,
    pub name: String,
    pub pid: u32,
    pub exit_code: Option<u32>,
}
pub struct Terminal {
    pub process: DevelopmentTerminal,
    info: Mutex<TerminalInfo>,
}
impl Terminal {
    pub fn info(&self) -> TerminalInfo {
        let mut info = self.info.lock().unwrap_or_else(|p| p.into_inner()).clone();
        info.exit_code = self.process.exit_code();
        info
    }
    pub fn rename(&self, name: &str) -> Result<()> {
        if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err(IdeError::Invalid);
        }
        self.info.lock().map_err(|_| IdeError::Conflict)?.name = name.into();
        Ok(())
    }
}

pub struct IdeManager {
    requests: Arc<AtomicUsize>,
    pub files: Arc<FileService>,
    pub extensions: extensions::ExtensionManager,
    store: Arc<IdeStore>,
    budget: Arc<RuntimeBudget>,
    terminal_budget: Arc<RuntimeBudget>,
    lsp: Mutex<BTreeMap<String, Option<Owned<lsp::LanguageServer>>>>,
    dap: Mutex<BTreeMap<String, Option<Owned<dap::DebugSession>>>>,
    terminals: Mutex<BTreeMap<String, Arc<Terminal>>>,
}

impl IdeManager {
    pub fn new(
        files: Arc<FileService>,
        store: Arc<IdeStore>,
        budget: Arc<RuntimeBudget>,
        terminal_budget: Arc<RuntimeBudget>,
    ) -> Self {
        Self {
            requests: Arc::new(AtomicUsize::new(0)),
            extensions: extensions::ExtensionManager::new(&files.project_id, store.clone()),
            files,
            store,
            budget,
            terminal_budget,
            lsp: Mutex::new(BTreeMap::new()),
            dap: Mutex::new(BTreeMap::new()),
            terminals: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn profiles(&self) -> Result<Vec<ToolProfile>> {
        self.store.list(&self.files.project_id, "profiles")
    }
    pub fn approve_profile(&self, mut profile: ToolProfile, approved: bool) -> Result<ToolProfile> {
        if self
            .servers()?
            .iter()
            .any(|server| server.profile == profile.id && server.alive)
            || self
                .debug_sessions()?
                .iter()
                .any(|session| session.profile_id == profile.id && session.alive)
            || self
                .terminals()?
                .iter()
                .any(|terminal| terminal.profile_id == profile.id && terminal.exit_code.is_none())
        {
            return Err(IdeError::Conflict);
        }
        profile.approve(approved)?;
        self.audit("approve_profile", &profile.id)?;
        self.store
            .put(&self.files.project_id, "profiles", &profile.id, &profile)?;
        Ok(profile)
    }
    pub fn profile(&self, id: &str, kind: &str) -> Result<ToolProfile> {
        let profile: ToolProfile = self.store.get(&self.files.project_id, "profiles", id)?;
        profile.verify(kind)?;
        Ok(profile)
    }
    pub fn remove_profile(&self, id: &str) -> Result<()> {
        if self.servers()?.iter().any(|s| s.profile == id)
            || self.debug_sessions()?.iter().any(|s| s.profile_id == id)
            || self
                .terminals()?
                .iter()
                .any(|s| s.profile_id == id && s.exit_code.is_none())
        {
            return Err(IdeError::Conflict);
        }
        self.audit("remove_profile", id)?;
        self.store.remove(&self.files.project_id, "profiles", id)
    }
    pub fn servers(&self) -> Result<Vec<lsp::ServerInfo>> {
        Ok(self
            .lsp
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .filter_map(|s| s.as_ref().map(|s| s.value.info()))
            .collect())
    }
    pub fn start_lsp(
        &self,
        root: &str,
        profile: &str,
        client: &str,
        confirmed: bool,
    ) -> Result<lsp::ServerInfo> {
        if !confirmed {
            return Err(IdeError::Unapproved);
        }
        let key = format!("{root}:{profile}:{client}");
        {
            let mut entries = self.lsp.lock().map_err(|_| IdeError::Conflict)?;
            entries.retain(|_, entry| {
                entry
                    .as_ref()
                    .is_none_or(|entry| entry.value.peer.is_alive())
            });
            if let Some(Some(entry)) = entries.get(&key) {
                if entry.value.peer.is_alive() {
                    return Ok(entry.value.info());
                }
                entries.remove(&key);
            }
            if entries.contains_key(&key) {
                return Err(IdeError::Conflict);
            }
            if entries.len() >= 4 {
                return Err(IdeError::Limit);
            }
            entries.insert(key.clone(), None);
        }
        let result = (|| {
            let permit = self.budget.operation().map_err(|_| IdeError::Limit)?;
            let profile = self.profile(profile, "lsp")?;
            self.audit(
                "request_lsp_start",
                &format!("{root}:{}:{client}", profile.id),
            )?;
            let server = Arc::new(lsp::LanguageServer::start(
                self.files.clone(),
                root,
                &profile,
                client,
            )?);
            Ok(Owned {
                value: server,
                _permit: permit,
            })
        })();
        let mut entries = self.lsp.lock().map_err(|_| IdeError::Conflict)?;
        match result {
            Ok(server) => {
                let info = server.value.info();
                entries.insert(key, Some(server));
                Ok(info)
            }
            Err(error) => {
                entries.remove(&key);
                Err(error)
            }
        }
    }
    pub fn lsp(&self, id: &str) -> Result<Arc<lsp::LanguageServer>> {
        self.lsp
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .filter_map(|s| s.as_ref())
            .find(|s| s.value.id == id)
            .map(|s| s.value.clone())
            .ok_or(IdeError::NotFound)
    }
    pub fn stop_lsp(&self, id: &str) -> Result<()> {
        self.audit("request_lsp_stop", id)?;
        let server = {
            let mut entries = self.lsp.lock().map_err(|_| IdeError::Conflict)?;
            let key = entries
                .iter()
                .find(|(_, entry)| entry.as_ref().is_some_and(|e| e.value.id == id))
                .map(|(key, _)| key.clone())
                .ok_or(IdeError::NotFound)?;
            entries.remove(&key).flatten().ok_or(IdeError::NotFound)?
        };
        server.value.stop();
        Ok(())
    }
    pub fn configurations(&self) -> Result<Vec<dap::LaunchConfiguration>> {
        self.store.list(&self.files.project_id, "launch")
    }
    pub fn approve_configuration(
        &self,
        config: dap::LaunchConfiguration,
        confirmed: bool,
    ) -> Result<dap::LaunchConfiguration> {
        if !confirmed {
            return Err(IdeError::Unapproved);
        }
        config.validate(&self.files)?;
        self.audit("approve_debug_configuration", &config.id)?;
        self.profile(&config.profile_id, "dap")?;
        self.store
            .put(&self.files.project_id, "launch", &config.id, &config)?;
        Ok(config)
    }
    pub fn debug_sessions(&self) -> Result<Vec<dap::DebugInfo>> {
        Ok(self
            .dap
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .filter_map(|s| s.as_ref().map(|s| s.value.info()))
            .collect())
    }
    pub fn start_debug(&self, id: &str, points: Value, confirmed: bool) -> Result<dap::DebugInfo> {
        let key = config::identifier();
        {
            let mut entries = self.dap.lock().map_err(|_| IdeError::Conflict)?;
            entries.retain(|_, entry| {
                entry
                    .as_ref()
                    .is_none_or(|entry| entry.value.peer.is_alive())
            });
            if entries.len() >= 2 {
                return Err(IdeError::Limit);
            }
            entries.insert(key.clone(), None);
        }
        let result = (|| {
            let permit = self.budget.operation().map_err(|_| IdeError::Limit)?;
            let config: dap::LaunchConfiguration =
                self.store.get(&self.files.project_id, "launch", id)?;
            let profile = self.profile(&config.profile_id, "dap")?;
            self.audit("request_debug_launch", &config.id)?;
            let session = Arc::new(dap::DebugSession::start(
                self.files.clone(),
                &profile,
                &config,
                points,
                confirmed,
            )?);
            Ok(Owned {
                value: session,
                _permit: permit,
            })
        })();
        let mut entries = self.dap.lock().map_err(|_| IdeError::Conflict)?;
        match result {
            Ok(session) => {
                let info = session.value.info();
                entries.insert(key, Some(session));
                Ok(info)
            }
            Err(error) => {
                entries.remove(&key);
                Err(error)
            }
        }
    }
    pub fn dap(&self, id: &str) -> Result<Arc<dap::DebugSession>> {
        self.dap
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .filter_map(|s| s.as_ref())
            .find(|s| s.value.id == id)
            .map(|s| s.value.clone())
            .ok_or(IdeError::NotFound)
    }
    pub fn stop_debug(&self, id: &str) -> Result<()> {
        self.audit("request_debug_stop", id)?;
        let session = {
            let mut entries = self.dap.lock().map_err(|_| IdeError::Conflict)?;
            let key = entries
                .iter()
                .find(|(_, entry)| entry.as_ref().is_some_and(|e| e.value.id == id))
                .map(|(key, _)| key.clone())
                .ok_or(IdeError::NotFound)?;
            entries.remove(&key).flatten().ok_or(IdeError::NotFound)?
        };
        session.value.stop();
        Ok(())
    }
    pub fn terminals(&self) -> Result<Vec<TerminalInfo>> {
        Ok(self
            .terminals
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .map(|t| t.info())
            .collect())
    }
    pub fn start_terminal(
        &self,
        root: &str,
        profile: &str,
        confirmed: bool,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalInfo> {
        if !confirmed {
            return Err(IdeError::Unapproved);
        }
        let root_path = self.files.process_root(root)?;
        let profile = self.profile(profile, "shell")?;
        self.audit("request_terminal_start", &format!("{root}:{}", profile.id))?;
        let mut terminals = self.terminals.lock().map_err(|_| IdeError::Conflict)?;
        terminals.retain(|_, t| t.process.exit_code().is_none());
        if terminals.len() >= 4 {
            return Err(IdeError::Limit);
        }
        let process = DevelopmentTerminal::start(
            &self.terminal_budget,
            &profile.executable,
            &profile.arguments,
            &root_path,
            &config::environment(),
            cols,
            rows,
        )
        .map_err(|_| IdeError::Executable)?;
        let info = TerminalInfo {
            id: config::identifier(),
            root_id: root.into(),
            profile_id: profile.id,
            name: profile.name,
            pid: process.pid,
            exit_code: None,
        };
        terminals.insert(
            info.id.clone(),
            Arc::new(Terminal {
                process,
                info: Mutex::new(info.clone()),
            }),
        );
        Ok(info)
    }
    pub fn terminal(&self, id: &str) -> Result<Arc<Terminal>> {
        self.terminals
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .get(id)
            .cloned()
            .ok_or(IdeError::NotFound)
    }
    pub fn stop_terminal(&self, id: &str) -> Result<()> {
        self.audit("request_terminal_stop", id)?;
        self.terminal(id)?
            .process
            .stop()
            .map_err(|_| IdeError::Exited)
    }
    pub fn is_idle(&self) -> bool {
        self.servers().is_ok_and(|v| v.iter().all(|s| !s.alive))
            && self
                .debug_sessions()
                .is_ok_and(|v| v.iter().all(|s| !s.alive))
            && self
                .terminals()
                .is_ok_and(|v| v.iter().all(|s| s.exit_code.is_some()))
    }
    pub fn shutdown(&self) -> Result<()> {
        for entry in self
            .lsp
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .flatten()
        {
            entry.value.peer.kill();
        }
        for entry in self
            .dap
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
            .flatten()
        {
            entry.value.peer.kill();
        }
        let mut failed = false;
        for entry in self
            .terminals
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .values()
        {
            failed |= entry.process.stop().is_err();
        }
        if failed {
            return Err(IdeError::Exited);
        }
        Ok(())
    }
    pub fn audit(&self, action: &str, resource: &str) -> Result<()> {
        self.store.audit(&self.files.project_id, action, resource)
    }
    pub fn audit_entries(&self) -> Result<Vec<Value>> {
        self.store.audit_entries(&self.files.project_id)
    }
    pub fn acquire_request(&self) -> Result<IdeRequestPermit> {
        self.requests
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < 4).then_some(used + 1)
            })
            .map_err(|_| IdeError::Limit)?;
        Ok(IdeRequestPermit(self.requests.clone()))
    }
}
pub struct IdeRequestPermit(Arc<AtomicUsize>);
impl Drop for IdeRequestPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for IdeManager {
    fn drop(&mut self) {
        if let Ok(entries) = self.lsp.lock() {
            for entry in entries.values().flatten() {
                entry.value.peer.kill();
            }
        }
        if let Ok(entries) = self.dap.lock() {
            for entry in entries.values().flatten() {
                entry.value.peer.kill();
            }
        }
        if let Ok(entries) = self.terminals.lock() {
            for entry in entries.values() {
                let _ = entry.process.stop();
            }
        }
    }
}
