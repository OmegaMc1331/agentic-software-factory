use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};

use crate::path::{protected_component, Guard};
use crate::types::valid_token;
use crate::{native, validate_path, DirectoryPage, Entry, FileError, FileRef, FileService, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub id: String,
    pub root_id: String,
    pub query: String,
    #[serde(default)]
    pub content: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub path_filter: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub file: FileRef,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub preview: String,
    pub rank: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub complete: bool,
    pub reason: Option<String>,
    pub scanned: usize,
    pub bytes_read: usize,
    pub elapsed_ms: u128,
}

impl FileService {
    fn ignore_rules(&self, root: &crate::service::Root, directory: &str) -> Result<Gitignore> {
        let mut builder = GitignoreBuilder::new(&root.path);
        let mut current = String::new();
        for component in
            std::iter::once("").chain(directory.split('/').filter(|part| !part.is_empty()))
        {
            if !component.is_empty() {
                current = if current.is_empty() {
                    component.into()
                } else {
                    format!("{current}/{component}")
                };
            }
            let relative = if current.is_empty() {
                ".gitignore".to_owned()
            } else {
                format!("{current}/.gitignore")
            };
            match self.read_raw(root, &self.reference(root, &relative)) {
                Ok(raw) => {
                    if raw.bytes.len() > 65536 || raw.snapshot.mode != "text" {
                        return Err(FileError::Limit);
                    }
                    if let Some(content) = raw.snapshot.content {
                        for line in content.lines() {
                            builder
                                .add_line(Some(root.path.join(&relative)), line)
                                .map_err(|_| FileError::Invalid)?;
                        }
                    }
                }
                Err(FileError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        builder.build().map_err(|_| FileError::Invalid)
    }

    pub fn list(
        &self,
        root_id: &str,
        directory: &str,
        after: Option<&str>,
        show_hidden: bool,
    ) -> Result<DirectoryPage> {
        validate_path(directory, true)?;
        if after.is_some_and(|value| value.len() > 4096) {
            return Err(FileError::Invalid);
        }
        let root = self.root(root_id)?;
        let guard = Guard::root(&root.path, &root.identity)?.descend(directory)?;
        let canonical = std::fs::canonicalize(&guard.absolute)?;
        let relative = canonical
            .strip_prefix(&root.path)
            .map_err(|_| FileError::Forbidden)?
            .to_str()
            .ok_or(FileError::Forbidden)?
            .replace('\\', "/");
        validate_path(&relative, true)?;
        let ignore = self.ignore_rules(&root, &relative)?;
        let start = Instant::now();
        let mut selected = BTreeMap::new();
        let mut scanned = 0;
        let mut complete = true;
        let mut more = false;
        for entry in guard.dir.entries()? {
            scanned += 1;
            if scanned > self.limits.scan_entries
                || start.elapsed() > Duration::from_millis(self.limits.search_millis)
            {
                complete = false;
                break;
            }
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else {
                complete = false;
                continue;
            };
            let path = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            if protected_component(&name) {
                continue;
            }
            if validate_path(&path, false).is_err() {
                complete = false;
                continue;
            }
            let metadata = std::fs::symlink_metadata(guard.absolute.join(&name))?;
            let directory = metadata.is_dir() && !native::is_link(&metadata);
            let ignored = ignore
                .matched_path_or_any_parents(root.path.join(&path), directory)
                .is_ignore();
            let hidden = name.starts_with('.');
            if !show_hidden && (hidden || ignored) {
                continue;
            }
            let sort_key = format!("{}:{name}", if directory { "0" } else { "1" });
            if after.is_some_and(|cursor| sort_key.as_str() <= cursor) {
                continue;
            }
            selected.insert(sort_key, (path, name, metadata, ignored, hidden));
            if selected.len() > self.limits.directory_entries {
                selected.pop_last();
                more = true;
            }
        }
        let git = self
            .git(
                &root,
                &[
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--untracked-files=all",
                    "--ignore-submodules=all",
                    "--",
                    if relative.is_empty() { "." } else { &relative },
                ],
                262144,
            )
            .ok();
        let mut statuses = BTreeMap::new();
        if let Some(bytes) = &git {
            let mut fields = bytes.split(|byte| *byte == 0);
            while let Some(field) = fields.next() {
                if field.len() < 4 {
                    continue;
                }
                if let Ok(path) = std::str::from_utf8(&field[3..]) {
                    statuses.insert(
                        path.to_owned(),
                        String::from_utf8_lossy(&field[..2]).to_string(),
                    );
                }
                if field[..2].contains(&b'R') || field[..2].contains(&b'C') {
                    fields.next();
                }
            }
        }
        let next = if more {
            selected.keys().next_back().cloned()
        } else {
            None
        };
        let entries = selected
            .into_values()
            .map(|(path, name, metadata, ignored, hidden)| {
                let linked = native::is_link(&metadata);
                let kind = if linked {
                    "link"
                } else if metadata.is_dir() {
                    "directory"
                } else if metadata.is_file() {
                    "file"
                } else {
                    "unsupported"
                };
                let protected = linked || kind == "unsupported";
                let revision = if protected {
                    String::new()
                } else {
                    self.entry_revision(&root, &self.reference(&root, &path))
                        .map(|(_, revision)| revision)
                        .unwrap_or_default()
                };
                let git = statuses.get(&path).cloned().or_else(|| {
                    statuses
                        .keys()
                        .any(|key| key.starts_with(&format!("{path}/")))
                        .then(|| "modified_children".into())
                });
                Entry {
                    path,
                    name,
                    kind: kind.into(),
                    size: metadata.len(),
                    hidden,
                    ignored,
                    protected,
                    revision,
                    git,
                }
            })
            .collect();
        guard.verify()?;
        Ok(DirectoryPage {
            entries,
            next,
            complete,
            reason: (!complete).then(|| "directory_scan_limited".into()),
            git_available: git.is_some(),
        })
    }

    pub fn begin_search(&self, id: &str) -> Result<Arc<AtomicBool>> {
        if !valid_token(id, "q_") {
            return Err(FileError::Invalid);
        }
        let mut searches = self.searches.lock().map_err(|_| FileError::Busy)?;
        if searches.len() >= 4 {
            return Err(FileError::Busy);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        if searches.contains_key(id) {
            return Err(FileError::Busy);
        }
        searches.insert(id.into(), cancel.clone());
        Ok(cancel)
    }

    pub fn cancel_search(&self, id: &str) -> Result<()> {
        if !valid_token(id, "q_") {
            return Err(FileError::Invalid);
        }
        if let Some(cancel) = self.searches.lock().map_err(|_| FileError::Busy)?.get(id) {
            cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn search(&self, request: &SearchRequest, cancel: &AtomicBool) -> Result<SearchResponse> {
        let result = self.search_inner(request, cancel);
        if let Ok(mut searches) = self.searches.lock() {
            searches.remove(&request.id);
        }
        result
    }

    fn search_inner(&self, request: &SearchRequest, cancel: &AtomicBool) -> Result<SearchResponse> {
        if request.query.is_empty() || request.query.len() > 256 || request.path_filter.len() > 256
        {
            return Err(FileError::Invalid);
        }
        let root = self.root(&request.root_id)?;
        let regex = regex::RegexBuilder::new(&if request.regex && request.content {
            request.query.clone()
        } else {
            regex::escape(&request.query)
        })
        .case_insensitive(!request.case_sensitive)
        .size_limit(1_048_576)
        .dfa_size_limit(1_048_576)
        .nest_limit(32)
        .build()
        .map_err(|_| FileError::Invalid)?;
        let start = Instant::now();
        let mut pending = BTreeSet::from([String::new()]);
        let mut hits = Vec::new();
        let mut scanned = 0;
        let mut bytes_read = 0;
        let mut reason = None;
        let mut output = 0;
        'scan: while let Some(directory) = pending.pop_first() {
            if cancel.load(Ordering::Relaxed) {
                reason = Some("cancelled".into());
                break;
            }
            let ignore = self.ignore_rules(&root, &directory)?;
            let guard = Guard::root(&root.path, &root.identity)?.descend(&directory)?;
            let mut names = BTreeSet::new();
            for entry in guard.dir.entries()? {
                scanned += 1;
                if scanned > self.limits.scan_entries {
                    reason = Some("entry_limit".into());
                    break 'scan;
                }
                if start.elapsed() > Duration::from_millis(self.limits.search_millis) {
                    reason = Some("time_limit".into());
                    break 'scan;
                }
                if cancel.load(Ordering::Relaxed) {
                    reason = Some("cancelled".into());
                    break 'scan;
                }
                let Ok(name) = entry?.file_name().into_string() else {
                    reason = Some("unsupported_path".into());
                    continue;
                };
                names.insert(name);
            }
            for name in names {
                let path = if directory.is_empty() {
                    name.clone()
                } else {
                    format!("{directory}/{name}")
                };
                if validate_path(&path, false).is_err() {
                    continue;
                }
                let metadata = match std::fs::symlink_metadata(guard.absolute.join(&name)) {
                    Ok(metadata) => metadata,
                    Err(_) => {
                        reason = Some("filesystem_changed".into());
                        continue;
                    }
                };
                if native::is_link(&metadata) {
                    continue;
                }
                if !request.include_hidden
                    && (name.starts_with('.')
                        || ignore
                            .matched_path_or_any_parents(root.path.join(&path), metadata.is_dir())
                            .is_ignore())
                {
                    continue;
                }
                if metadata.is_dir() {
                    pending.insert(path);
                    continue;
                }
                if !metadata.is_file() || !path.contains(&request.path_filter) {
                    continue;
                }
                let reference = self.reference(&root, &path);
                if !request.content {
                    if regex.is_match(&path) {
                        let compare = if request.case_sensitive {
                            name.clone()
                        } else {
                            name.to_lowercase()
                        };
                        let needle = if request.case_sensitive {
                            request.query.clone()
                        } else {
                            request.query.to_lowercase()
                        };
                        hits.push(SearchHit {
                            file: reference,
                            line: None,
                            column: None,
                            preview: String::new(),
                            rank: if compare == needle {
                                0
                            } else if compare.starts_with(&needle) {
                                1
                            } else if compare.contains(&needle) {
                                2
                            } else {
                                3
                            },
                        });
                        if hits.len() > self.limits.search_results {
                            hits.sort_by(|a, b| {
                                (a.rank, &a.file.path).cmp(&(b.rank, &b.file.path))
                            });
                            hits.pop();
                            reason = Some("result_limit".into());
                        }
                    }
                    continue;
                }
                let lower = name.to_ascii_lowercase();
                if lower == ".env"
                    || lower.starts_with(".env.")
                    || [".pem", ".key", ".p12", ".pfx"]
                        .iter()
                        .any(|suffix| lower.ends_with(suffix))
                    || lower == "id_rsa"
                {
                    continue;
                }
                if metadata.len() > self.limits.edit_bytes as u64 {
                    continue;
                }
                if bytes_read + metadata.len() as usize > self.limits.search_bytes {
                    reason = Some("byte_limit".into());
                    break 'scan;
                }
                let raw = match self.read_raw(&root, &reference) {
                    Ok(raw) => raw,
                    Err(_) => {
                        reason = Some("filesystem_changed".into());
                        continue;
                    }
                };
                bytes_read += raw.bytes.len();
                let Some(content) = raw.snapshot.content else {
                    continue;
                };
                for (line_number, line) in content.lines().enumerate() {
                    if let Some(found) = regex.find(line) {
                        let preview = line.chars().take(400).collect::<String>();
                        output += preview.len() + path.len();
                        if output > 65536 || hits.len() >= self.limits.search_results {
                            reason = Some("result_limit".into());
                            break 'scan;
                        }
                        hits.push(SearchHit {
                            file: reference.clone(),
                            line: Some(line_number + 1),
                            column: Some(line[..found.start()].chars().count() + 1),
                            preview,
                            rank: 0,
                        });
                    }
                }
                if cancel.load(Ordering::Relaxed) {
                    reason = Some("cancelled".into());
                    break 'scan;
                }
                if start.elapsed() > Duration::from_millis(self.limits.search_millis) {
                    reason = Some("time_limit".into());
                    break 'scan;
                }
            }
        }
        hits.sort_by(|a, b| (a.rank, &a.file.path, a.line).cmp(&(b.rank, &b.file.path, b.line)));
        Ok(SearchResponse {
            hits,
            complete: reason.is_none(),
            reason,
            scanned,
            bytes_read,
            elapsed_ms: start.elapsed().as_millis(),
        })
    }
}
