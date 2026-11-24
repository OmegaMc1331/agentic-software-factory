use crate::config::{identifier, valid_id, ToolProfile};
use crate::peer::{EventPage, Peer};
use crate::{IdeError, Result};
use factory_files::{FileRef, FileService};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchConfiguration {
    pub id: String,
    pub name: String,
    pub profile_id: String,
    pub root_id: String,
    pub program: FileRef,
    pub arguments: Value,
}

impl LaunchConfiguration {
    pub fn validate(&self, files: &FileService) -> Result<()> {
        if !valid_id(&self.id)
            || !valid_id(&self.profile_id)
            || self.name.len() > 120
            || self.program.root_id != self.root_id
            || !self.arguments.is_object()
            || serde_json::to_vec(&self.arguments)
                .map_err(|_| IdeError::Invalid)?
                .len()
                > 32_768
        {
            return Err(IdeError::Invalid);
        }
        files.process_root(&self.root_id)?;
        files.launch_path(&self.program)?;
        for field in [
            "processId",
            "pid",
            "connect",
            "listen",
            "debugServer",
            "runtimeExecutable",
            "python",
            "pythonPath",
            "debugAdapterPython",
            "cwd",
            "program",
            "request",
            "type",
            "console",
            "env",
            "envFile",
            "preLaunchTask",
            "postDebugTask",
        ] {
            if self.arguments.get(field).is_some() {
                return Err(IdeError::Forbidden);
            }
        }
        Ok(())
    }
}

pub struct DebugSession {
    pub id: String,
    pub root_id: String,
    pub profile_id: String,
    pub configuration_id: String,
    pub capabilities: Value,
    pub peer: Arc<Peer>,
    files: Arc<FileService>,
    breakpoints: Mutex<BTreeMap<String, Value>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DebugInfo {
    pub id: String,
    pub root_id: String,
    pub profile_id: String,
    pub configuration_id: String,
    pub capabilities: Value,
    pub alive: bool,
    pub pid: u32,
}

impl DebugSession {
    pub fn start(
        files: Arc<FileService>,
        profile: &ToolProfile,
        config: &LaunchConfiguration,
        breakpoints: Value,
        confirmed: bool,
    ) -> Result<Self> {
        if !confirmed {
            return Err(IdeError::Unapproved);
        }
        config.validate(&files)?;
        let root = files.process_root(&config.root_id)?;
        let peer = Peer::spawn(profile, &root, true)?;
        let capabilities = peer.request("initialize",json!({"clientID":"asf","clientName":"ASF","adapterID":profile.id,"pathFormat":"path","linesStartAt1":true,"columnsStartAt1":true,"supportsVariableType":true,"supportsRunInTerminalRequest":false,"supportsMemoryReferences":false}))?;
        let mut arguments = profile
            .settings
            .get("launch")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or(json!({}));
        for (key, value) in config.arguments.as_object().ok_or(IdeError::Invalid)? {
            arguments[key] = value.clone();
        }
        arguments["program"] = json!(factory_git::git_path(&files.launch_path(&config.program)?));
        arguments["cwd"] = json!(factory_git::git_path(&root));
        arguments["console"] = json!("internalConsole");
        arguments["env"] = json!({});
        let launch_peer = peer.clone();
        let launch = std::thread::spawn(move || launch_peer.request("launch", arguments));
        let start = Instant::now();
        let mut initialized = false;
        while start.elapsed() < peer.timeout {
            if peer
                .events(0)
                .events
                .iter()
                .any(|e| e.message["event"] == "initialized")
            {
                initialized = true;
                break;
            }
            if !peer.is_alive() || launch.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !initialized {
            peer.kill();
            let _ = launch.join();
            return Err(IdeError::Timeout);
        }
        let session = Self {
            id: identifier(),
            root_id: config.root_id.clone(),
            profile_id: profile.id.clone(),
            configuration_id: config.id.clone(),
            capabilities,
            peer,
            files,
            breakpoints: Mutex::new(BTreeMap::new()),
        };
        if let Some(sources) = breakpoints.as_array() {
            if sources.len() > 64 {
                session.peer.kill();
                return Err(IdeError::Limit);
            }
            for source in sources {
                let file: FileRef = serde_json::from_value(source["file"].clone())
                    .map_err(|_| IdeError::Invalid)?;
                session.set_breakpoints(&file, source["breakpoints"].clone())?;
            }
        }
        if session.capabilities["supportsConfigurationDoneRequest"] == true {
            session.peer.request("configurationDone", json!({}))?;
        }
        match launch.join() {
            Ok(Ok(_)) => Ok(session),
            Ok(Err(error)) => {
                session.peer.kill();
                Err(error)
            }
            Err(_) => {
                session.peer.kill();
                Err(IdeError::Tool("debug launch failed".into()))
            }
        }
    }

    pub fn info(&self) -> DebugInfo {
        DebugInfo {
            id: self.id.clone(),
            root_id: self.root_id.clone(),
            profile_id: self.profile_id.clone(),
            configuration_id: self.configuration_id.clone(),
            capabilities: self.capabilities.clone(),
            alive: self.peer.is_alive(),
            pid: self.peer.pid,
        }
    }

    pub fn set_breakpoints(&self, file: &FileRef, points: Value) -> Result<Value> {
        if file.root_id != self.root_id
            || !points.is_array()
            || points.as_array().is_some_and(|v| v.len() > 500)
        {
            return Err(IdeError::Invalid);
        }
        if points.as_array().unwrap().iter().any(|p| {
            p["line"].as_u64().is_none_or(|n| n == 0 || n > 1_000_000)
                || p.get("condition").is_some()
                    && self.capabilities["supportsConditionalBreakpoints"] != true
        }) {
            return Err(IdeError::Unsupported);
        }
        let path = self.files.tool_path(file)?;
        let result = self.peer.request(
            "setBreakpoints",
            json!({"source":{"path":factory_git::git_path(&path)},"breakpoints":points}),
        )?;
        self.breakpoints
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .insert(file.path.clone(), points);
        Ok(result)
    }

    pub fn request(&self, command: &str, params: Value, confirmed: bool) -> Result<Value> {
        let capability = match command {
            "threads" | "stackTrace" | "scopes" | "variables" | "continue" | "pause" | "next"
            | "stepIn" | "stepOut" => None,
            "evaluate" if confirmed => Some("supportsEvaluateForHovers"),
            "exceptionInfo" => Some("supportsExceptionInfoRequest"),
            _ => return Err(IdeError::Forbidden),
        };
        if capability.is_some_and(|key| self.capabilities[key] != true) {
            return Err(IdeError::Unsupported);
        }
        let mut result = self.peer.request(command, params)?;
        if command == "stackTrace" {
            if let Some(frames) = result["stackFrames"].as_array_mut() {
                for frame in frames {
                    let file = frame["source"]["path"].as_str().and_then(|path| {
                        self.files
                            .tool_reference(&self.root_id, std::path::Path::new(path))
                            .ok()
                    });
                    frame["asfFile"] = serde_json::to_value(&file).unwrap_or(Value::Null);
                    if file.is_none() {
                        frame["source"] = json!({"name":"Source outside authorized workspace"});
                    }
                }
            }
        }
        Ok(result)
    }

    pub fn events(&self, cursor: u64) -> EventPage {
        self.peer.events(cursor)
    }

    pub fn stop(&self) {
        if self.peer.is_alive() {
            let _ = self.peer.request_deadline(
                "disconnect",
                json!({"terminateDebuggee":true}),
                Duration::from_secs(2),
            );
        }
        self.peer.kill();
    }
}
