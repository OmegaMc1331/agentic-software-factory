use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use factory_files::{FileRef, FileService};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::config::{identifier, ToolProfile};
use crate::peer::{EventPage, Peer};
use crate::position::{self, TextEdit};
use crate::{IdeError, Result};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub file: FileRef,
    pub base_revision: String,
    pub version: i64,
    pub text: String,
    pub language_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditPlan {
    pub file: FileRef,
    pub expected_revision: String,
    pub document_version: Option<i64>,
    pub before: String,
    pub after: String,
}

pub struct LanguageServer {
    pub id: String,
    pub root_id: String,
    pub profile: String,
    pub client_id: String,
    pub capabilities: Value,
    pub peer: Arc<Peer>,
    files: Arc<FileService>,
    documents: Mutex<BTreeMap<String, Document>>,
    encoding: String,
    sync_kind: u64,
    edit_guards: Mutex<VecDeque<(String, BTreeMap<String, i64>)>>,
    cancellations: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ServerInfo {
    pub id: String,
    pub root_id: String,
    pub profile: String,
    pub client_id: String,
    pub capabilities: Value,
    pub alive: bool,
    pub pid: u32,
    pub position_encoding: String,
    pub documents: Vec<Value>,
    pub diagnostic_versions: Vec<Value>,
}

pub fn file_uri(files: &FileService, file: &FileRef) -> Result<String> {
    let path = files.tool_path(file)?;
    Url::from_file_path(factory_git::git_path(&path))
        .map(|url| url.to_string())
        .map_err(|_| IdeError::Forbidden)
}

pub fn reference(files: &FileService, root: &str, uri: &str) -> Result<FileRef> {
    let url = Url::parse(uri).map_err(|_| IdeError::Forbidden)?;
    if url.scheme() != "file"
        || url.query().is_some()
        || url.fragment().is_some()
        || url
            .host_str()
            .is_some_and(|host| !host.is_empty() && host != "localhost")
    {
        return Err(IdeError::Forbidden);
    }
    let path = url.to_file_path().map_err(|_| IdeError::Forbidden)?;
    files.process_root(root)?;
    Ok(files.tool_reference(root, &path)?)
}

impl LanguageServer {
    pub fn start(
        files: Arc<FileService>,
        root_id: &str,
        profile: &ToolProfile,
        client_id: &str,
    ) -> Result<Self> {
        if !crate::config::valid_id(client_id) {
            return Err(IdeError::Invalid);
        }
        let root = files.process_root(root_id)?;
        let root_uri = Url::from_directory_path(factory_git::git_path(&root))
            .map_err(|_| IdeError::Forbidden)?
            .to_string();
        let peer = Peer::spawn(profile, &root, false)?;
        let initialized = peer.request("initialize", json!({
            "processId":std::process::id(),"clientInfo":{"name":"ASF","version":env!("CARGO_PKG_VERSION")},
            "rootUri":root_uri,"workspaceFolders":[{"uri":root_uri,"name":"workspace"}],
            "capabilities":{
                "general":{"positionEncodings":["utf-16"]},
                "workspace":{"applyEdit":true,"workspaceEdit":{"documentChanges":true,"failureHandling":"abort"},"configuration":true,"workspaceFolders":true},
                "textDocument":{
                    "synchronization":{"didSave":true},
                    "publishDiagnostics":{"versionSupport":true,"relatedInformation":true},
                    "hover":{"contentFormat":["plaintext","markdown"]},
                    "completion":{"completionItem":{"snippetSupport":false,"resolveSupport":{"properties":["documentation","detail","additionalTextEdits"]}}},
                    "signatureHelp":{"signatureInformation":{"documentationFormat":["plaintext","markdown"]}},
                    "definition":{"linkSupport":true},"references":{},"documentSymbol":{"hierarchicalDocumentSymbolSupport":true},
                    "rename":{"prepareSupport":true},"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix","refactor","source"]}}},
                    "formatting":{},"rangeFormatting":{}
                }
            }, "initializationOptions":profile.initialization_options
        }))?;
        let capabilities = initialized["capabilities"].clone();
        let encoding = capabilities["positionEncoding"]
            .as_str()
            .unwrap_or("utf-16")
            .to_owned();
        if encoding != "utf-16" {
            peer.kill();
            return Err(IdeError::Unsupported);
        }
        let sync_kind = capabilities["textDocumentSync"]
            .as_u64()
            .or_else(|| capabilities["textDocumentSync"]["change"].as_u64())
            .unwrap_or(0);
        peer.notify("initialized", json!({}))?;
        peer.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":profile.settings}),
        )?;
        Ok(Self {
            id: identifier(),
            root_id: root_id.into(),
            profile: profile.id.clone(),
            client_id: client_id.into(),
            capabilities,
            peer,
            files,
            documents: Mutex::new(BTreeMap::new()),
            encoding,
            sync_kind,
            edit_guards: Mutex::new(VecDeque::new()),
            cancellations: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn info(&self) -> ServerInfo {
        ServerInfo {
            id: self.id.clone(),
            root_id: self.root_id.clone(),
            profile: self.profile.clone(),
            client_id: self.client_id.clone(),
            capabilities: self.capabilities.clone(),
            alive: self.peer.is_alive(),
            pid: self.peer.pid,
            position_encoding: self.encoding.clone(),
            documents: self.documents.lock().map(|documents|documents.values().map(|document|json!({"file":document.file,"version":document.version,"content_sha256":crate::config::digest(&document.text)})).collect()).unwrap_or_default(),
            diagnostic_versions: self.peer.events(0).events.iter().filter(|event|event.message["method"]=="textDocument/publishDiagnostics").map(|event|json!({"version":event.message["params"]["version"],"count":event.message["params"]["diagnostics"].as_array().map(Vec::len)})).collect(),
        }
    }

    pub fn sync(&self, document: Document, saved: bool) -> Result<Value> {
        if document.file.root_id != self.root_id
            || document.text.len() > self.files.limits.edit_bytes
            || document.version < 1
            || document.language_id.len() > 64
        {
            return Err(IdeError::Invalid);
        }
        let uri = file_uri(&self.files, &document.file)?;
        let disk = self.files.read(&document.file)?;
        if disk.read_only {
            return Err(IdeError::Forbidden);
        }
        if saved
            && (disk.revision != document.base_revision
                || disk.content.as_deref() != Some(document.text.as_str()))
        {
            return Err(IdeError::Conflict);
        }
        let mut documents = self.documents.lock().map_err(|_| IdeError::Conflict)?;
        if let Some(previous) = documents.get(&uri) {
            if document.version < previous.version
                || document.version == previous.version && document.text != previous.text
            {
                return Err(IdeError::Conflict);
            }
            if document.version > previous.version {
                if self.sync_kind == 0 && previous.text != document.text {
                    return Err(IdeError::Unsupported);
                }
                let changes = if self.sync_kind == 2 {
                    let lines: Vec<_> = previous.text.split('\n').collect();
                    json!([{"range":{"start":{"line":0,"character":0},"end":{"line":lines.len()-1,"character":lines.last().unwrap_or(&"").trim_end_matches('\r').encode_utf16().count()}},"text":document.text}])
                } else {
                    json!([{"text":document.text}])
                };
                if self.sync_kind != 0 {
                    self.peer.notify("textDocument/didChange",json!({"textDocument":{"uri":uri,"version":document.version},"contentChanges":changes}))?;
                }
            }
        } else {
            if documents.len() >= 128 {
                return Err(IdeError::Limit);
            }
            self.peer.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":document.language_id,"version":document.version,"text":document.text}}))?;
        }
        if saved {
            let options = &self.capabilities["textDocumentSync"]["save"];
            if options == true || options.is_object() {
                let mut params = json!({"textDocument":{"uri":uri}});
                if options["includeText"] == true {
                    params["text"] = json!(document.text);
                }
                self.peer.notify("textDocument/didSave", params)?;
            }
        }
        documents.insert(uri.clone(), document);
        Ok(json!({"uri":uri}))
    }

    pub fn close(&self, file: &FileRef) -> Result<()> {
        let mut documents = self.documents.lock().map_err(|_| IdeError::Conflict)?;
        let uri = documents
            .iter()
            .find(|(_, document)| &document.file == file)
            .map(|(uri, _)| uri.clone())
            .ok_or(IdeError::NotFound)?;
        documents.remove(&uri);
        self.peer
            .notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}))
    }

    pub fn request(&self, method: &str, params: Value, confirmed: bool) -> Result<Value> {
        self.request_inner(method, params, confirmed, None)
    }

    pub fn request_token(
        &self,
        method: &str,
        params: Value,
        confirmed: bool,
        token: &str,
    ) -> Result<Value> {
        if !crate::config::valid_id(token) {
            return Err(IdeError::Invalid);
        }
        let flag = Arc::new(AtomicBool::new(false));
        {
            let mut entries = self.cancellations.lock().map_err(|_| IdeError::Conflict)?;
            if entries.len() >= 64 || entries.contains_key(token) {
                return Err(IdeError::Limit);
            }
            entries.insert(token.into(), flag.clone());
        }
        let result = self.request_inner(method, params, confirmed, Some(&flag));
        self.cancellations
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .remove(token);
        result
    }

    pub fn cancel(&self, token: &str) -> Result<()> {
        self.cancellations
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .get(token)
            .ok_or(IdeError::NotFound)?
            .store(true, Ordering::Release);
        Ok(())
    }

    fn request_inner(
        &self,
        method: &str,
        params: Value,
        confirmed: bool,
        cancelled: Option<&AtomicBool>,
    ) -> Result<Value> {
        if ![
            "textDocument/hover",
            "textDocument/completion",
            "completionItem/resolve",
            "textDocument/definition",
            "textDocument/references",
            "textDocument/documentSymbol",
            "workspace/symbol",
            "workspaceSymbol/resolve",
            "textDocument/prepareRename",
            "textDocument/rename",
            "textDocument/codeAction",
            "codeAction/resolve",
            "textDocument/formatting",
            "textDocument/rangeFormatting",
            "textDocument/signatureHelp",
            "textDocument/diagnostic",
        ]
        .contains(&method)
        {
            return Err(IdeError::Forbidden);
        }
        if let Some(uri) = params["textDocument"]["uri"].as_str() {
            reference(&self.files, &self.root_id, uri)?;
            if !self
                .documents
                .lock()
                .map_err(|_| IdeError::Conflict)?
                .contains_key(uri)
            {
                return Err(IdeError::Conflict);
            }
        }
        if method == "textDocument/rename" && !confirmed {
            return Err(IdeError::Unapproved);
        }
        let versions = self
            .documents
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .iter()
            .map(|(uri, document)| (uri.clone(), document.version))
            .collect::<BTreeMap<_, _>>();
        let result = self
            .peer
            .request_cancellable(method, params.clone(), cancelled)?;
        let mut edits = Vec::new();
        match method {
            "textDocument/formatting" | "textDocument/rangeFormatting" if result.is_array() => {
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .ok_or(IdeError::Invalid)?;
                edits.push(json!({"changes":{uri:result}}));
            }
            "textDocument/rename" if result.is_object() => edits.push(result.clone()),
            "textDocument/codeAction" => {
                if let Some(actions) = result.as_array() {
                    for action in actions {
                        if action["edit"].is_object() {
                            edits.push(action["edit"].clone());
                        }
                    }
                }
            }
            "codeAction/resolve" if result["edit"].is_object() => {
                edits.push(result["edit"].clone())
            }
            _ => {}
        }
        if !edits.is_empty() {
            let current = self.documents.lock().map_err(|_| IdeError::Conflict)?;
            if versions.iter().any(|(uri, version)| {
                current
                    .get(uri)
                    .is_none_or(|document| document.version != *version)
            }) {
                return Err(IdeError::Conflict);
            }
            let mut guards = self.edit_guards.lock().map_err(|_| IdeError::Conflict)?;
            for edit in edits {
                guards.push_back((crate::config::digest(edit.to_string()), versions.clone()));
            }
            while guards.len() > 64 {
                guards.pop_front();
            }
        }
        Ok(result)
    }

    pub fn events(&self, cursor: u64) -> EventPage {
        let mut page = self.peer.events(cursor);
        let documents = self.documents.lock().unwrap_or_else(|p| p.into_inner());
        page.events.retain_mut(|event| {
            if event.message["method"] == "textDocument/publishDiagnostics" {
                let params = &event.message["params"];
                let Some(uri) = params["uri"].as_str() else {
                    return false;
                };
                let file = reference(&self.files, &self.root_id, uri).ok();
                let Some((canonical, document)) = documents
                    .iter()
                    .find(|(key, doc)| key.as_str() == uri || file.as_ref() == Some(&doc.file))
                else {
                    return false;
                };
                let current = params["version"]
                    .as_i64()
                    .is_none_or(|version| version == document.version);
                event.message["params"]["uri"] = json!(canonical);
                return current;
            }
            true
        });
        page
    }

    pub fn prepare_edit(&self, edit: &Value) -> Result<Vec<EditPlan>> {
        let mut edits = BTreeMap::<String, (Option<i64>, Vec<TextEdit>)>::new();
        if let Some(changes) = edit["changes"].as_object() {
            for (uri, value) in changes {
                edits.insert(
                    uri.clone(),
                    (
                        None,
                        serde_json::from_value(value.clone()).map_err(|_| IdeError::Invalid)?,
                    ),
                );
            }
        }
        if let Some(changes) = edit["documentChanges"].as_array() {
            for change in changes {
                if change.get("kind").is_some() {
                    return Err(IdeError::Forbidden);
                }
                let uri = change["textDocument"]["uri"]
                    .as_str()
                    .ok_or(IdeError::Invalid)?;
                if edits.contains_key(uri) {
                    return Err(IdeError::Invalid);
                }
                edits.insert(
                    uri.into(),
                    (
                        change["textDocument"]["version"].as_i64(),
                        serde_json::from_value(change["edits"].clone())
                            .map_err(|_| IdeError::Invalid)?,
                    ),
                );
            }
        }
        if edits.is_empty() || edits.len() > 32 {
            return Err(IdeError::Limit);
        }
        let documents = self.documents.lock().map_err(|_| IdeError::Conflict)?;
        let mut plans = Vec::new();
        for (uri, (expected_version, edits)) in edits {
            let file = reference(&self.files, &self.root_id, &uri)?;
            let snapshot = self.files.read(&file)?;
            if snapshot.read_only {
                return Err(IdeError::Forbidden);
            }
            let (before, version) =
                if let Some(document) = documents.values().find(|document| document.file == file) {
                    if snapshot.revision != document.base_revision
                        || expected_version.is_some_and(|v| v != document.version)
                    {
                        return Err(IdeError::Conflict);
                    }
                    (document.text.clone(), Some(document.version))
                } else {
                    if expected_version.is_some() {
                        return Err(IdeError::Conflict);
                    }
                    (snapshot.content.ok_or(IdeError::Unsupported)?, None)
                };
            let after = position::apply(&before, &edits, &self.encoding)?;
            plans.push(EditPlan {
                file,
                expected_revision: snapshot.revision,
                document_version: version,
                before,
                after,
            });
        }
        Ok(plans)
    }

    pub fn prepare_verified_edit(&self, edit: &Value) -> Result<Vec<EditPlan>> {
        let fingerprint = crate::config::digest(edit.to_string());
        let versions = self
            .edit_guards
            .lock()
            .map_err(|_| IdeError::Conflict)?
            .iter()
            .rev()
            .find(|(hash, _)| hash == &fingerprint)
            .map(|(_, versions)| versions.clone())
            .ok_or(IdeError::Conflict)?;
        {
            let current = self.documents.lock().map_err(|_| IdeError::Conflict)?;
            if versions.iter().any(|(uri, version)| {
                current
                    .get(uri)
                    .is_none_or(|document| document.version != *version)
            }) {
                return Err(IdeError::Conflict);
            }
        }
        let plans = self.prepare_edit(edit)?;
        for plan in &plans {
            let uri = file_uri(&self.files, &plan.file)?;
            if plan.document_version.is_none()
                || versions.get(&uri).copied() != plan.document_version
            {
                return Err(IdeError::Conflict);
            }
        }
        Ok(plans)
    }

    pub fn stop(&self) {
        if self.peer.is_alive() {
            let _ = self.peer.request_deadline(
                "shutdown",
                Value::Null,
                std::time::Duration::from_secs(2),
            );
            let _ = self.peer.notify("exit", Value::Null);
            for _ in 0..10 {
                if !self.peer.is_alive() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        self.peer.kill();
    }
}
