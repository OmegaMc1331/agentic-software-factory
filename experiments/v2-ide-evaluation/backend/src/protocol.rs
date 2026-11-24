use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::app::{Config, Error};

pub struct Service {
    pub id: String,
    pub project: String,
    pub kind: String,
    root: std::path::PathBuf,
    pub pid: u32,
    pub created: Instant,
    child: Mutex<Child>,
    input: Mutex<ChildStdin>,
    pub output: broadcast::Sender<Value>,
    pub trace: Mutex<VecDeque<Value>>,
    #[cfg(windows)]
    job: Mutex<Option<factory_proc::windows_job::JobGuard>>,
}

fn frame(reader: &mut impl Read) -> std::io::Result<Value> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err(std::io::Error::other("protocol header limit"));
        }
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let header = String::from_utf8(header).map_err(std::io::Error::other)?;
    let size = header
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        })
        .ok_or_else(|| std::io::Error::other("missing Content-Length"))?;
    if size > 1_048_576 {
        return Err(std::io::Error::other("protocol frame limit"));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

impl Service {
    pub fn start(
        config: &Config,
        root: &Path,
        project: &str,
        kind: &str,
    ) -> Result<Arc<Self>, Error> {
        let mut command = match kind {
            "typescript" => {
                let mut c = Command::new(&config.node);
                c.args([&config.lsp, "--stdio"]);
                c
            }
            "python" => {
                let mut c = Command::new(&config.node);
                c.args([&config.pyright, "--stdio"]);
                c
            }
            "debugpy" => {
                let mut c = Command::new(&config.python);
                c.args(["-m", "debugpy.adapter"]);
                c
            }
            _ => return Err(Error::bad("unknown fixed protocol profile")),
        };
        command
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for key in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("PYTHONUNBUFFERED", "1");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        #[cfg(windows)]
        let job = match factory_proc::windows_job::JobGuard::assign(&child) {
            Some(job) => job,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::bad("cannot establish process-tree ownership"));
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::bad("missing process output"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::bad("missing process errors"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| Error::bad("missing process input"))?;
        let (output, _) = broadcast::channel(256);
        let service = Arc::new(Self {
            id: format!("{:016x}", rand::random::<u64>()),
            project: project.to_string(),
            kind: kind.to_string(),
            root: root.to_path_buf(),
            pid: child.id(),
            created: Instant::now(),
            child: Mutex::new(child),
            input: Mutex::new(input),
            output,
            trace: Mutex::new(VecDeque::new()),
            #[cfg(windows)]
            job: Mutex::new(Some(job)),
        });
        let weak = Arc::downgrade(&service);
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            loop {
                match frame(&mut reader) {
                    Ok(mut message) => {
                        let Some(service) = weak.upgrade() else {
                            break;
                        };
                        canonicalize_uris(&service.root, &mut message);
                        service.record("server", &message);
                        let _ = service.output.send(message);
                    }
                    Err(error) => {
                        if let Some(service) = weak.upgrade() {
                            let event = json!({"type":"transport-ended","error":error.to_string(),"pid":service.pid});
                            service.record("server", &event);
                            let _ = service.output.send(event);
                        }
                        break;
                    }
                }
            }
        });
        let weak = Arc::downgrade(&service);
        std::thread::spawn(move || {
            let mut bytes = [0; 1024];
            while let Ok(size) = stderr.read(&mut bytes) {
                if size == 0 {
                    break;
                }
                let Some(service) = weak.upgrade() else {
                    break;
                };
                service.record("stderr", &json!({"type":"stderr","bytes":size}));
            }
        });
        Ok(service)
    }

    pub fn record(&self, direction: &str, message: &Value) {
        let mut trace = self.trace.lock().unwrap();
        if trace.len() == 2000 {
            trace.pop_front();
        }
        trace.push_back(json!({"direction":direction,"method":message.get("method"),"command":message.get("command"),"event":message.get("event"),"type":message.get("type"),"id":message.get("id"),"seq":message.get("seq"),"request_seq":message.get("request_seq"),"success":message.get("success"),"elapsed_ms":self.created.elapsed().as_millis()}));
    }

    pub fn send(&self, root: &Path, config: &Config, mut message: Value) -> Result<(), Error> {
        if self.kind == "debugpy" {
            let operation = message["command"].as_str().unwrap_or_default().to_string();
            if ![
                "initialize",
                "launch",
                "setBreakpoints",
                "configurationDone",
                "threads",
                "stackTrace",
                "scopes",
                "variables",
                "continue",
                "next",
                "pause",
                "disconnect",
            ]
            .contains(&operation.as_str())
            {
                return Err(Error::denied("DAP operation not in the fixed probe policy"));
            }
            if operation == "launch" {
                let native_root = url::Url::from_directory_path(root)
                    .map_err(|_| Error::bad("invalid debug root"))?
                    .to_file_path()
                    .map_err(|_| Error::bad("invalid debug path"))?;
                message["arguments"] = json!({"type":"python","request":"launch","name":"ASF P01 trusted fixture","program":native_root.join("debug.py"),"python":config.python,"cwd":native_root,"console":"internalConsole","justMyCode":false,"stopOnEntry":false});
            }
            if operation == "setBreakpoints" {
                let supplied = message["arguments"]["source"]["path"]
                    .as_str()
                    .unwrap_or_default();
                let permitted = std::fs::canonicalize(root.join("debug.py"))?;
                if std::fs::canonicalize(supplied).ok().as_ref() != Some(&permitted) {
                    return Err(Error::denied(
                        "breakpoint source is outside the fixed profile",
                    ));
                }
            }
        } else {
            let operation = message["method"].as_str().unwrap_or_default();
            let allowed = [
                "initialize",
                "initialized",
                "shutdown",
                "exit",
                "textDocument/didOpen",
                "textDocument/didChange",
                "textDocument/didClose",
                "textDocument/didSave",
                "textDocument/hover",
                "textDocument/definition",
                "textDocument/completion",
                "completionItem/resolve",
                "textDocument/references",
                "textDocument/documentSymbol",
                "textDocument/signatureHelp",
                "textDocument/semanticTokens/full",
                "textDocument/semanticTokens/range",
                "textDocument/documentHighlight",
                "textDocument/codeLens",
                "textDocument/foldingRange",
                "textDocument/inlayHint",
                "textDocument/documentLink",
                "textDocument/codeAction",
                "$/cancelRequest",
                "$/setTrace",
            ];
            if !operation.is_empty() && !allowed.contains(&operation) {
                return Err(Error::denied(&format!(
                    "LSP operation not in the prototype policy: {operation}"
                )));
            }
            if operation == "initialize" {
                let uri = url::Url::from_directory_path(root)
                    .map_err(|_| Error::bad("invalid workspace root"))?;
                if message["params"]["rootUri"]
                    .as_str()
                    .is_some_and(|value| value != uri.as_str().trim_end_matches('/'))
                    && message["params"]["rootUri"].as_str() != Some(uri.as_str())
                {
                    return Err(Error::denied("language-server root mismatch"));
                }
                message["params"]["rootUri"] = json!(uri.as_str());
                message["params"]["workspaceFolders"] =
                    json!([{"uri":uri.as_str(),"name":self.project}]);
            }
            if let Some(uri) = message
                .pointer("/params/textDocument/uri")
                .and_then(Value::as_str)
            {
                let canonical = scoped_uri(root, uri).ok_or_else(|| {
                    Error::denied("document outside project or invalid document URI")
                })?;
                message["params"]["textDocument"]["uri"] = json!(canonical);
            }
        }
        let bytes =
            serde_json::to_vec(&message).map_err(|_| Error::bad("invalid protocol JSON"))?;
        if bytes.len() > 1_048_576 {
            return Err(Error::bad("protocol request limit"));
        }
        self.record("client", &message);
        let mut input = self.input.lock().unwrap();
        write!(input, "Content-Length: {}\r\n\r\n", bytes.len())?;
        input.write_all(&bytes)?;
        input.flush()?;
        Ok(())
    }

    pub fn running(&self) -> bool {
        self.child
            .lock()
            .unwrap()
            .try_wait()
            .ok()
            .flatten()
            .is_none()
    }

    pub fn stop(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.lock().unwrap().take() {
            job.terminate();
        }
        let mut child = self.child.lock().unwrap();
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn scoped_uri(root: &Path, uri: &str) -> Option<String> {
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    let native_root = url::Url::from_directory_path(root)
        .ok()?
        .to_file_path()
        .ok()?;
    let path_text = path.to_string_lossy();
    let root_display = native_root.to_string_lossy();
    let root_text = root_display.trim_end_matches(['/', '\\']);
    let prefix = path_text.get(..root_text.len())?;
    let matches = if cfg!(windows) {
        prefix.eq_ignore_ascii_case(root_text)
    } else {
        prefix == root_text
    };
    if !matches {
        return None;
    }
    let suffix = path_text.get(root_text.len()..)?;
    if !suffix.starts_with(['/', '\\']) {
        return None;
    }
    let relative = suffix.trim_start_matches(['/', '\\']).replace('\\', "/");
    let path = crate::files::path(root, &relative).ok()?;
    Some(url::Url::from_file_path(path).ok()?.to_string())
}

fn canonicalize_uris(root: &Path, value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                canonicalize_uris(root, value);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                if ["uri", "targetUri"].contains(&key.as_str()) {
                    let canonical = value.as_str().and_then(|uri| scoped_uri(root, uri));
                    if let Some(uri) = canonical {
                        *value = json!(uri);
                    }
                } else {
                    canonicalize_uris(root, value);
                }
            }
        }
        _ => (),
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_frame_limit_and_actual_json_framing() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        bytes.extend(body);
        assert_eq!(frame(&mut bytes.as_slice()).unwrap()["id"], 1);
        assert!(frame(&mut &b"Content-Length: 2000000\r\n\r\n"[..]).is_err());
        assert!(frame(&mut vec![b'x'; 8193].as_slice()).is_err());
    }
    #[test]
    fn server_uris_are_mapped_to_the_actual_project_document_identity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::write(root.join("file.ts"), "content").unwrap();
        let canonical = url::Url::from_file_path(root.join("file.ts"))
            .unwrap()
            .to_string();
        let alternate = if cfg!(windows) {
            canonical.replacen("C:", "c%3A", 1)
        } else {
            canonical.clone()
        };
        let mut value = json!({"params":{"uri":alternate,"diagnostics":[{"message":"actual diagnostic","range":{"start":{"line":1}}}]}});
        canonicalize_uris(&root, &mut value);
        assert_eq!(value["params"]["uri"], canonical);
        assert_eq!(scoped_uri(&root, &canonical), Some(canonical.clone()));
        let sibling = url::Url::from_file_path(
            root.with_file_name(format!(
                "{}-sibling",
                root.file_name().unwrap().to_string_lossy()
            ))
            .join("file.ts"),
        )
        .unwrap()
        .to_string();
        assert!(scoped_uri(&root, &sibling).is_none());
        assert_eq!(
            value["params"]["diagnostics"][0]["range"]["start"]["line"],
            1
        );
    }
}
