use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use factory_git::source_control::{SourceTiming, Status};
use factory_git::{GitError, Repo};
use factory_proc::{Bounds, ProcCommand};

use crate::path::{protected_component, split, Guard};
use crate::store::Store;
use crate::types::{hash, nonce, valid_token};
use crate::{
    native, validate_path, Draft, DraftEnvelope, FileError, FileRef, FileSnapshot, Limits,
    MoveResult, Recovery, Result, RootInfo,
};

pub(crate) struct Root {
    pub info: RootInfo,
    pub path: PathBuf,
    pub identity: String,
    writes: Mutex<()>,
}

struct GitBinding {
    directory: PathBuf,
    common: PathBuf,
    root: Guard,
    git: Guard,
}

impl GitBinding {
    fn verify(&self) -> Result<()> {
        self.root.verify()?;
        self.git.verify()
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub struct FileService {
    pub project_id: String,
    pub limits: Limits,
    pub(crate) roots: RwLock<BTreeMap<String, Arc<Root>>>,
    pub(crate) primary: Arc<Root>,
    store: Store,
    pub(crate) searches: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
    requests: std::sync::atomic::AtomicUsize,
    git_common: Mutex<Option<(PathBuf, String)>>,
}

pub struct FilePermit(Arc<FileService>);
impl Drop for FilePermit {
    fn drop(&mut self) {
        self.0
            .requests
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

pub(crate) struct Raw {
    pub snapshot: FileSnapshot,
    pub handle: File,
    pub bytes: Vec<u8>,
    pub stable: String,
}

impl FileService {
    pub fn new(project_id: &str, path: &Path, limits: Limits) -> Result<Self> {
        if !valid_token(project_id, "p_") {
            return Err(FileError::Invalid);
        }
        limits.validate()?;
        let path = fs::canonicalize(path)?;
        let handle = native::open_directory(&path)?;
        let identity = native::key(&handle)?;
        let id = format!(
            "r_{}",
            &hash(format!(
                "{project_id}:{identity}:{}:{:?}",
                path.display(),
                handle.metadata()?.created().ok()
            ))[..32]
        );
        let primary = Arc::new(Root {
            info: RootInfo {
                id: id.clone(),
                kind: "repository".into(),
                label: "Repository".into(),
                read_only: false,
            },
            path: path.clone(),
            identity: identity.clone(),
            writes: Mutex::new(()),
        });
        Ok(Self {
            project_id: project_id.into(),
            limits,
            roots: RwLock::new(BTreeMap::from([(id, primary.clone())])),
            primary,
            store: Store::new(&path, &identity),
            searches: Mutex::new(BTreeMap::new()),
            requests: std::sync::atomic::AtomicUsize::new(0),
            git_common: Mutex::new(None),
        })
    }

    pub fn acquire(self: &Arc<Self>) -> Result<FilePermit> {
        self.requests
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |count| (count < 4).then_some(count + 1),
            )
            .map_err(|_| FileError::Busy)?;
        Ok(FilePermit(self.clone()))
    }

    pub fn primary_root(&self) -> RootInfo {
        self.primary.info.clone()
    }

    pub fn roots(&self) -> Result<Vec<RootInfo>> {
        let output = self.git(
            &self.primary,
            &["worktree", "list", "--porcelain", "-z"],
            262_144,
        )?;
        let text = String::from_utf8(output).map_err(|_| FileError::GitUnavailable)?;
        let managed = self.primary.path.join(".factory/worktrees");
        let integrations = self.primary.path.join(".factory/integrations");
        let mut found = BTreeMap::from([(self.primary.info.id.clone(), self.primary.clone())]);
        for value in text
            .split('\0')
            .filter_map(|line| line.strip_prefix("worktree "))
        {
            let Ok(candidate) = fs::canonicalize(value) else {
                continue;
            };
            let integration = candidate.starts_with(&integrations) && candidate != integrations;
            if (!candidate.starts_with(&managed) || candidate == managed) && !integration {
                continue;
            }
            let relative = candidate
                .strip_prefix(&self.primary.path)
                .map_err(|_| FileError::Forbidden)?;
            let relative = relative
                .to_str()
                .ok_or(FileError::Forbidden)?
                .replace('\\', "/");
            let guard =
                Guard::root(&self.primary.path, &self.primary.identity)?.descend(&relative)?;
            let identity = native::key(&guard.dir.try_clone()?.into_std_file())?;
            let id = format!(
                "r_{}",
                &hash(format!("{}:{identity}:{relative}", self.project_id))[..32]
            );
            let previous = self
                .roots
                .read()
                .map_err(|_| FileError::Busy)?
                .get(&id)
                .cloned();
            let root = previous.unwrap_or_else(|| {
                Arc::new(Root {
                    info: RootInfo {
                        id: id.clone(),
                        kind: if integration {
                            "integration_snapshot".into()
                        } else {
                            "task_worktree".into()
                        },
                        label: format!(
                            "{} - {}",
                            if integration {
                                "Integration snapshot"
                            } else {
                                "Agent worktree"
                            },
                            candidate.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        read_only: true,
                    },
                    path: candidate,
                    identity,
                    writes: Mutex::new(()),
                })
            });
            found.insert(id, root);
        }
        let views = found.values().map(|root| root.info.clone()).collect();
        *self.roots.write().map_err(|_| FileError::Busy)? = found;
        Ok(views)
    }

    pub(crate) fn root(&self, id: &str) -> Result<Arc<Root>> {
        if !valid_token(id, "r_") {
            return Err(FileError::UnknownRoot);
        }
        self.roots
            .read()
            .map_err(|_| FileError::Busy)?
            .get(id)
            .cloned()
            .ok_or(FileError::UnknownRoot)
    }

    pub(crate) fn reference(&self, root: &Root, path: &str) -> FileRef {
        FileRef {
            project_id: self.project_id.clone(),
            root_id: root.info.id.clone(),
            path: path.into(),
        }
    }

    fn resolve(&self, reference: &FileRef) -> Result<Arc<Root>> {
        if reference.project_id != self.project_id {
            return Err(FileError::Forbidden);
        }
        validate_path(&reference.path, false)?;
        self.root(&reference.root_id)
    }

    pub(crate) fn parent(&self, root: &Root, path: &str) -> Result<Guard> {
        let parent = Guard::root(&root.path, &root.identity)?.descend(split(path).0)?;
        let canonical = fs::canonicalize(&parent.absolute)?;
        let relative = canonical
            .strip_prefix(&root.path)
            .map_err(|_| FileError::Forbidden)?
            .to_str()
            .ok_or(FileError::Forbidden)?
            .replace('\\', "/");
        validate_path(&relative, true)?;
        Ok(parent)
    }

    fn canonical_ref(&self, root: &Root, path: &Path) -> Result<FileRef> {
        let canonical = fs::canonicalize(path)?;
        let relative = canonical
            .strip_prefix(&root.path)
            .map_err(|_| FileError::Forbidden)?
            .to_str()
            .ok_or(FileError::Forbidden)?
            .replace('\\', "/");
        validate_path(&relative, false)?;
        Ok(self.reference(root, &relative))
    }

    fn private(&self) -> Result<Guard> {
        let factory =
            Guard::root(&self.primary.path, &self.primary.identity)?.descend(".factory")?;
        match factory.dir.create_dir("editor-recovery") {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        factory.descend("editor-recovery")
    }

    pub fn read(&self, reference: &FileRef) -> Result<FileSnapshot> {
        let root = self.resolve(reference)?;
        Ok(self.read_raw(&root, reference)?.snapshot)
    }

    /// Check a selection using the same file and snapshot checks as `read`.
    /// Consecutive paths in one directory can share its held directory lease.
    pub fn guard_source_paths(&self, id: &str, paths: &[String]) -> Result<()> {
        let root = self.root(id)?;
        let workers = paths.len().div_ceil(512).clamp(1, 4);
        if workers == 1 {
            return self.guard_source_chunk(&root, paths);
        }
        // This runs inside Source Control's existing bounded blocking permit.
        // Independent checks share no mutable state and finish before any Git write.
        std::thread::scope(|scope| {
            let root = root.as_ref();
            let mut jobs = Vec::with_capacity(workers);
            for chunk in paths.chunks(paths.len().div_ceil(workers)) {
                jobs.push(
                    std::thread::Builder::new()
                        .spawn_scoped(scope, move || self.guard_source_chunk(root, chunk))?,
                );
            }
            let mut result = Ok(());
            for job in jobs {
                let chunk_result = job.join().unwrap_or(Err(FileError::Conflict));
                result = result.and(chunk_result);
            }
            result
        })
    }

    fn guard_source_chunk(&self, root: &Root, paths: &[String]) -> Result<()> {
        let mut held = None;
        for path in paths {
            validate_path(path, false)?;
            let (directory, name) = split(path);
            if !held
                .as_ref()
                .is_some_and(|(relative, _)| *relative == directory)
            {
                held = match self.parent(root, path) {
                    Ok(parent) => Some((directory, parent)),
                    Err(FileError::NotFound) => None,
                    Err(error) => return Err(error),
                };
            }
            if let Some((_, parent)) = &held {
                match self.read_at(root, parent, name) {
                    Ok(_) | Err(FileError::NotFound) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }

    pub fn process_root(&self, id: &str) -> Result<PathBuf> {
        let root = self.root(id)?;
        if root.info.read_only {
            return Err(FileError::ReadOnly);
        }
        let _guard = Guard::root(&root.path, &root.identity)?;
        Ok(root.path.clone())
    }

    pub fn inspect_root(&self, id: &str) -> Result<PathBuf> {
        let root = self.root(id)?;
        self.git_binding(&root)?.verify()?;
        Ok(root.path.clone())
    }

    pub fn observe_source<E: From<FileError> + From<GitError>>(
        &self,
        id: &str,
    ) -> std::result::Result<(RootInfo, PathBuf, PathBuf, Status), E> {
        let mut timing = SourceTiming::start("source_observation");
        let root = self.root(id)?;
        let binding = self.git_binding(&root)?;
        timing.phase("repository_binding");
        let status = Repo::source_status_for_bound_worktree(
            &root.path,
            &binding.common,
            &binding.directory,
        )?;
        timing.phase("authoritative_status");
        binding.verify()?;
        timing.phase("verify_binding");
        Ok((root.info.clone(), root.path.clone(), binding.common, status))
    }

    pub fn source_operation<T, E: From<FileError>>(
        &self,
        id: &str,
        checkout: bool,
        action: impl FnOnce(&Path) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let root = self.root(id)?;
        if root.info.read_only {
            return Err(FileError::ReadOnly.into());
        }
        let _write = root.writes.lock().map_err(|_| FileError::Busy)?;
        let guard = Guard::root(&root.path, &root.identity)?;
        self.git(&root, &["rev-parse", "--show-toplevel"], 16_384)?;
        if checkout {
            let (connection, _lease) = self.store.connect()?;
            let dirty: i64 = connection
                .query_row("SELECT count(*) FROM drafts WHERE dirty=1", [], |r| {
                    r.get(0)
                })
                .map_err(|_| FileError::Storage)?;
            if dirty > 0 {
                return Err(FileError::DraftConflict.into());
            }
        }
        let value = action(&root.path)?;
        guard.verify()?;
        Ok(value)
    }

    pub fn tool_path(&self, reference: &FileRef) -> Result<PathBuf> {
        let root = self.resolve(reference)?;
        self.read_raw(&root, reference)?;
        Ok(root.path.join(&reference.path))
    }

    pub fn tool_reference(&self, root_id: &str, path: &Path) -> Result<FileRef> {
        let root = self.root(root_id)?;
        let root_path = PathBuf::from(factory_git::git_path(&root.path));
        let source_path = PathBuf::from(factory_git::git_path(path));
        #[cfg(not(windows))]
        let relative = source_path
            .strip_prefix(&root_path)
            .map_err(|_| FileError::Forbidden)?
            .to_str()
            .ok_or(FileError::Invalid)?
            .replace('\\', "/");
        #[cfg(windows)]
        let relative = {
            let source = factory_git::git_path(&source_path).replace('\\', "/");
            let root = factory_git::git_path(&root_path).replace('\\', "/");
            if !source
                .get(..root.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&root))
            {
                return Err(FileError::Forbidden);
            }
            source
                .get(root.len()..)
                .and_then(|suffix| suffix.strip_prefix('/'))
                .ok_or(FileError::Forbidden)?
                .to_owned()
        };
        let reference = self.reference(&root, &relative);
        Ok(self.read(&reference)?.file)
    }

    pub fn launch_path(&self, reference: &FileRef) -> Result<PathBuf> {
        let root = self.resolve(reference)?;
        if root.info.read_only {
            return Err(FileError::ReadOnly);
        }
        let parent = self.parent(&root, &reference.path)?;
        let file = parent.launch_file(split(&reference.path).1)?;
        if !file.metadata()?.is_file() {
            return Err(FileError::Forbidden);
        }
        Ok(root.path.join(&reference.path))
    }

    pub(crate) fn read_raw(&self, root: &Root, reference: &FileRef) -> Result<Raw> {
        let parent = self.parent(root, &reference.path)?;
        self.read_at(root, &parent, split(&reference.path).1)
    }

    fn read_at(&self, root: &Root, parent: &Guard, name: &str) -> Result<Raw> {
        let file = parent.file(name)?;
        let canonical = self.canonical_ref(root, &parent.absolute.join(name))?;
        self.snapshot(root, canonical, file)
    }

    fn snapshot(&self, root: &Root, reference: FileRef, file: File) -> Result<Raw> {
        let before = file.metadata()?;
        let key = native::key(&file)?;
        let modified = format!("{:?}", before.modified().ok());
        let large = before.len() > self.limits.edit_bytes as u64;
        let limit = if large {
            self.limits.preview_bytes
        } else {
            self.limits.edit_bytes
        };
        let mut bytes = Vec::with_capacity((before.len() as usize).min(limit));
        (&file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
        let truncated = bytes.len() > limit;
        if truncated {
            bytes.truncate(limit);
        }
        let after = file.metadata()?;
        if before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || native::change_stamp(&before) != native::change_stamp(&after)
        {
            return Err(FileError::Conflict);
        }
        if truncated && !large {
            return Err(FileError::Limit);
        }
        let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        let text = if bytes.contains(&0) {
            None
        } else {
            let offset = if bom { 3 } else { 0 };
            match std::str::from_utf8(&bytes[offset..]) {
                Ok(text) => Some(text.to_owned()),
                Err(error) if large && error.error_len().is_none() => Some(
                    std::str::from_utf8(&bytes[offset..offset + error.valid_up_to()])
                        .map_err(|_| FileError::Unsupported)?
                        .to_owned(),
                ),
                Err(_) => None,
            }
        };
        let newline = text
            .as_ref()
            .map(|text| {
                let crlf = text
                    .as_bytes()
                    .windows(2)
                    .filter(|pair| *pair == b"\r\n")
                    .count();
                let lf = text.bytes().filter(|byte| *byte == b'\n').count();
                if crlf > 0 && crlf != lf {
                    "mixed"
                } else if crlf > 0 {
                    "crlf"
                } else {
                    "lf"
                }
            })
            .unwrap_or("none");
        let reason = if root.info.read_only {
            Some("agent_worktree".to_owned())
        } else if large {
            Some("large_file_preview".to_owned())
        } else if text.is_none() {
            Some("binary_or_unsupported_encoding".to_owned())
        } else if newline == "mixed" {
            Some("mixed_line_endings".to_owned())
        } else if before.permissions().readonly() {
            Some("filesystem_read_only".to_owned())
        } else {
            None
        };
        let stable = hash(format!(
            "{key}:{}:{modified}:{}",
            before.len(),
            hash(&bytes)
        ));
        let revision = format!(
            "v_{}",
            hash(format!(
                "{}:{}:{stable}:{}",
                reference.root_id,
                reference.path,
                native::change_stamp(&before)
            ))
        );
        let snapshot = FileSnapshot {
            file: reference,
            revision,
            content: text,
            size: before.len(),
            modified: before
                .modified()
                .ok()
                .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_millis().to_string())
                .unwrap_or_default(),
            encoding: if bytes.contains(&0) {
                "binary"
            } else if std::str::from_utf8(&bytes[if bom { 3 } else { 0 }..]).is_ok() {
                "utf-8"
            } else {
                "unsupported"
            }
            .into(),
            bom,
            newline: newline.into(),
            read_only: reason.is_some(),
            reason,
            mode: if large {
                "preview"
            } else if newline == "none" {
                "metadata"
            } else {
                "text"
            }
            .into(),
            durability: None,
        };
        Ok(Raw {
            snapshot,
            handle: file,
            bytes,
            stable,
        })
    }

    pub fn save(&self, reference: &FileRef, expected: &str, content: &str) -> Result<FileSnapshot> {
        self.save_hook(reference, expected, content, || Ok(()))
    }

    fn save_hook(
        &self,
        reference: &FileRef,
        expected: &str,
        content: &str,
        hook: impl FnOnce() -> Result<()>,
    ) -> Result<FileSnapshot> {
        self.save_stages(reference, expected, content, hook, || Ok(()))
    }
    fn save_stages(
        &self,
        reference: &FileRef,
        expected: &str,
        content: &str,
        hook: impl FnOnce() -> Result<()>,
        commit_hook: impl FnOnce() -> Result<()>,
    ) -> Result<FileSnapshot> {
        let root = self.resolve(reference)?;
        if root.info.read_only {
            return Err(FileError::ReadOnly);
        }
        if content.len() > self.limits.edit_bytes {
            return Err(FileError::Limit);
        }
        if content.contains('\0') {
            return Err(FileError::Unsupported);
        }
        let _write = root.writes.try_lock().map_err(|_| FileError::Busy)?;
        let original = self.read_raw(&root, reference)?;
        if original.snapshot.revision != expected {
            return Err(FileError::Conflict);
        }
        if original.snapshot.reason.as_deref() == Some("filesystem_read_only") {
            return Err(FileError::Permission);
        }
        if original.snapshot.read_only {
            return Err(FileError::Unsupported);
        }
        let parent = self.parent(&root, &original.snapshot.file.path)?;
        let name = split(&original.snapshot.file.path).1;
        let temporary = format!(".asf-editor-{}.tmp", nonce());
        let private = self.private()?;
        let operation = self.store.begin(
            &root.info.id,
            &original.snapshot.file.path,
            "save",
            &temporary,
        )?;
        let mut uncertain = false;
        let result = (|| {
            let mut options = cap_std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            let mut file = parent.dir.open_with(&temporary, &options)?.into_std();
            native::preserve_permissions(&original.handle, &file)?;
            let mut bytes = Vec::with_capacity(content.len() + 3);
            if original.snapshot.bom {
                bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            }
            bytes.extend_from_slice(content.as_bytes());
            if bytes.len() > self.limits.edit_bytes {
                return Err(FileError::Limit);
            }
            file.write_all(&bytes)?;
            file.sync_all()?;
            let replacement_key = native::key(&file)?;
            drop(file);
            hook()?;
            if self
                .read_raw(&root, &original.snapshot.file)?
                .snapshot
                .revision
                != expected
            {
                return Err(FileError::Conflict);
            }
            parent.verify()?;
            commit_hook()?;
            uncertain = true;
            native::exchange(
                &parent.dir,
                &parent.absolute,
                &temporary,
                name,
                &private.absolute.join(&operation.backup),
            )?;
            #[cfg(windows)]
            let (backup_parent, backup_name) = (&private, operation.backup.as_str());
            #[cfg(unix)]
            let (backup_parent, backup_name) = (&parent, temporary.as_str());
            let backup = self.snapshot(
                &root,
                original.snapshot.file.clone(),
                backup_parent.file(backup_name)?,
            )?;
            let current = self.read_raw(&root, &original.snapshot.file)?;
            if current.bytes != bytes || native::key(&current.handle)? != replacement_key {
                self.store.finish(&operation.id, "uncertain")?;
                return Err(FileError::Uncertain);
            }
            if backup.stable != original.stable {
                let backup_stable = backup.stable.clone();
                drop(backup);
                drop(current);
                native::restore(
                    &parent,
                    name,
                    backup_parent,
                    backup_name,
                    &format!("{}.local", operation.backup),
                )?;
                let restored = self.read_raw(&root, &original.snapshot.file)?;
                if restored.stable != backup_stable {
                    return Err(FileError::Uncertain);
                }
                #[cfg(windows)]
                let retained = private.file(&format!("{}.local", operation.backup))?;
                #[cfg(unix)]
                let retained = parent.file(&temporary)?;
                if native::key(&retained)? != replacement_key {
                    return Err(FileError::Uncertain);
                }
                drop(retained);
                native::remove_owned(
                    &backup_parent
                        .absolute
                        .join(format!("{}.local", operation.backup)),
                );
                native::remove_owned(&parent.absolute.join(&temporary));
                self.store.finish(&operation.id, "failed")?;
                uncertain = false;
                return Err(FileError::Conflict);
            }
            let durability = native::sync_directory(&parent.dir)?;
            self.store
                .finish(&operation.id, "committed")
                .map_err(|_| FileError::Uncertain)?;
            native::remove_owned(&backup_parent.absolute.join(backup_name));
            let mut snapshot = current.snapshot;
            snapshot.durability = Some(durability.into());
            uncertain = false;
            Ok(snapshot)
        })();
        let result = if uncertain && result.is_err() {
            Err(FileError::Uncertain)
        } else {
            result
        };
        match &result {
            Err(FileError::Uncertain) => {
                let _ = self.store.finish(&operation.id, "uncertain");
            }
            Err(_) => {
                native::remove_owned(&parent.absolute.join(&temporary));
                let _ = self.store.finish(&operation.id, "failed");
            }
            Ok(_) => {}
        }
        result
    }

    pub fn create(&self, reference: &FileRef, directory: bool) -> Result<()> {
        let root = self.resolve(reference)?;
        if root.info.read_only {
            return Err(FileError::ReadOnly);
        }
        let _write = root.writes.try_lock().map_err(|_| FileError::Busy)?;
        let parent = self.parent(&root, &reference.path)?;
        let name = split(&reference.path).1;
        if parent.dir.symlink_metadata(name).is_ok() {
            return Err(FileError::Exists);
        }
        if directory {
            parent.dir.create_dir(name)?;
        } else {
            let mut options = cap_std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            parent.dir.open_with(name, &options)?.sync_all()?;
        }
        parent.verify()?;
        native::sync_directory(&parent.dir)?;
        Ok(())
    }

    pub fn move_entry(
        &self,
        reference: &FileRef,
        expected: &str,
        destination: &str,
    ) -> Result<MoveResult> {
        validate_path(destination, false)?;
        let root = self.resolve(reference)?;
        if root.info.read_only {
            return Err(FileError::ReadOnly);
        }
        let _write = root.writes.try_lock().map_err(|_| FileError::Busy)?;
        let source = self.parent(&root, &reference.path)?;
        let target = self.parent(&root, destination)?;
        let source_name = split(&reference.path).1;
        let target_name = split(destination).1;
        let (kind, revision) = self.entry_revision(&root, reference)?;
        if revision != expected
            && self
                .read_raw(&root, reference)
                .map(|raw| raw.snapshot.revision != expected)
                .unwrap_or(true)
        {
            return Err(FileError::Conflict);
        }
        if kind == "directory" {
            self.directory_safe(&root, &reference.path)?;
        }
        let source_key = native::metadata_key(
            &source.absolute.join(source_name),
            &fs::symlink_metadata(source.absolute.join(source_name))?,
        )?;
        source.verify()?;
        target.verify()?;
        let same_case_alias = cfg!(windows)
            && reference.path.eq_ignore_ascii_case(destination)
            && reference.path != destination;
        if same_case_alias {
            let temporary = format!(".asf-editor-{}.rename", nonce());
            let journal =
                self.store
                    .begin(&root.info.id, &reference.path, "case_rename", &temporary)?;
            native::no_replace(
                &source.dir,
                &source.absolute,
                source_name,
                &source.dir,
                &source.absolute,
                &temporary,
            )?;
            if let Err(error) = native::no_replace(
                &source.dir,
                &source.absolute,
                &temporary,
                &target.dir,
                &target.absolute,
                target_name,
            ) {
                if native::no_replace(
                    &source.dir,
                    &source.absolute,
                    &temporary,
                    &source.dir,
                    &source.absolute,
                    source_name,
                )
                .is_err()
                {
                    let _ = self.store.finish(&journal.id, "uncertain");
                    return Err(FileError::Uncertain);
                }
                self.store.finish(&journal.id, "failed")?;
                return Err(error);
            }
            self.store
                .finish(&journal.id, "committed")
                .map_err(|_| FileError::Uncertain)?;
        } else {
            native::no_replace(
                &source.dir,
                &source.absolute,
                source_name,
                &target.dir,
                &target.absolute,
                target_name,
            )?;
        }
        let moved_metadata = fs::symlink_metadata(target.absolute.join(target_name))
            .map_err(|_| FileError::Uncertain)?;
        if native::metadata_key(&target.absolute.join(target_name), &moved_metadata)? != source_key
        {
            native::no_replace(
                &target.dir,
                &target.absolute,
                target_name,
                &source.dir,
                &source.absolute,
                source_name,
            )
            .map_err(|_| FileError::Uncertain)?;
            return Err(FileError::Conflict);
        }
        source.verify().map_err(|_| FileError::Uncertain)?;
        target.verify().map_err(|_| FileError::Uncertain)?;
        native::sync_directory(&source.dir)?;
        native::sync_directory(&target.dir)?;
        Ok(MoveResult {
            from: reference.clone(),
            to: self.canonical_ref(&root, &target.absolute.join(target_name))?,
            kind,
        })
    }

    pub fn delete(&self, reference: &FileRef, expected: &str, confirmed: bool) -> Result<Recovery> {
        if !confirmed {
            return Err(FileError::Forbidden);
        }
        let root = self.resolve(reference)?;
        if root.info.read_only {
            return Err(FileError::ReadOnly);
        }
        let _write = root.writes.try_lock().map_err(|_| FileError::Busy)?;
        let parent = self.parent(&root, &reference.path)?;
        let (kind, revision) = self.entry_revision(&root, reference)?;
        if revision != expected
            && self
                .read_raw(&root, reference)
                .map(|raw| raw.snapshot.revision != expected)
                .unwrap_or(true)
        {
            return Err(FileError::Conflict);
        }
        if kind == "directory"
            && parent
                .dir
                .open_dir(split(&reference.path).1)?
                .entries()?
                .next()
                .is_some()
        {
            return Err(FileError::DirectoryNotEmpty);
        }
        let private = self.private()?;
        let name = split(&reference.path).1;
        let source_key = native::metadata_key(
            &parent.absolute.join(name),
            &fs::symlink_metadata(parent.absolute.join(name))?,
        )?;
        parent.verify()?;
        let mut operation = self
            .store
            .begin(&root.info.id, &reference.path, "delete", "")?;
        native::no_replace(
            &parent.dir,
            &parent.absolute,
            split(&reference.path).1,
            &private.dir,
            &private.absolute,
            &operation.backup,
        )?;
        let moved = fs::symlink_metadata(private.absolute.join(&operation.backup))
            .map_err(|_| FileError::Uncertain)?;
        if native::metadata_key(&private.absolute.join(&operation.backup), &moved)? != source_key {
            native::no_replace(
                &private.dir,
                &private.absolute,
                &operation.backup,
                &parent.dir,
                &parent.absolute,
                name,
            )
            .map_err(|_| FileError::Uncertain)?;
            self.store.finish(&operation.id, "failed")?;
            return Err(FileError::Conflict);
        }
        native::sync_directory(&parent.dir)?;
        native::sync_directory(&private.dir)?;
        self.store
            .finish(&operation.id, "deleted")
            .map_err(|_| FileError::Uncertain)?;
        operation.phase = "deleted".into();
        Ok(operation)
    }

    pub fn recovery(&self) -> Result<Vec<Recovery>> {
        self.store.recovery()
    }

    fn recovery_entry(&self, id: &str) -> Result<Recovery> {
        if !valid_token(id, "u_") {
            return Err(FileError::Invalid);
        }
        let item = self
            .store
            .recovery()?
            .into_iter()
            .find(|item| item.id == id)
            .ok_or(FileError::NotFound)?;
        validate_path(&item.path, false)?;
        if item.backup.len() != 41
            || !item.backup.ends_with(".recovery")
            || !item.backup.as_bytes()[..32]
                .iter()
                .all(u8::is_ascii_hexdigit)
        {
            return Err(FileError::Storage);
        }
        if !item.temporary.is_empty()
            && !(item.temporary.starts_with(".asf-editor-")
                && item.temporary.len() <= 64
                && item
                    .temporary
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte)))
        {
            return Err(FileError::Storage);
        }
        Ok(item)
    }

    pub fn recovery_content(&self, id: &str) -> Result<FileSnapshot> {
        let item = self.recovery_entry(id)?;
        let root = self.root(&item.root_id)?;
        let private = self.private()?;
        let file = if private.dir.symlink_metadata(&item.backup).is_ok() {
            private.file(&item.backup)?
        } else if private
            .dir
            .symlink_metadata(format!("{}.local", item.backup))
            .is_ok()
        {
            private.file(&format!("{}.local", item.backup))?
        } else {
            if item.temporary.is_empty() {
                return Err(FileError::NotFound);
            }
            self.parent(&root, &item.path)?.file(&item.temporary)?
        };
        let mut result = self
            .snapshot(&root, self.reference(&root, &item.path), file)?
            .snapshot;
        result.read_only = true;
        result.reason = Some("retained_recovery_version".into());
        result.revision = format!("recovery:{id}:{}", result.revision);
        Ok(result)
    }

    pub fn purge_recovery(&self, id: &str, confirmed: bool) -> Result<()> {
        if !confirmed {
            return Err(FileError::Forbidden);
        }
        let item = self.recovery_entry(id)?;
        let root = self.root(&item.root_id)?;
        let _write = root.writes.try_lock().map_err(|_| FileError::Busy)?;
        let private = self.private()?;
        let mut targets = vec![
            (&private, item.backup.clone()),
            (&private, format!("{}.local", item.backup)),
        ];
        let parent = self.parent(&root, &item.path)?;
        if !item.temporary.is_empty() {
            targets.push((&parent, item.temporary));
        }
        for (guard, name) in targets {
            match guard.dir.symlink_metadata(&name) {
                Ok(metadata) => {
                    if native::is_link(&fs::symlink_metadata(guard.absolute.join(&name))?) {
                        return Err(FileError::Forbidden);
                    }
                    if metadata.is_dir() {
                        guard.dir.remove_dir(&name)?;
                    } else {
                        guard.file(&name)?;
                        guard.dir.remove_file(&name)?;
                    }
                    native::sync_directory(&guard.dir)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.store.finish(id, "purged")
    }

    pub fn restore_deleted(&self, id: &str) -> Result<FileRef> {
        if !valid_token(id, "u_") {
            return Err(FileError::Invalid);
        }
        let operation = self.recovery_entry(id)?;
        if !(operation.kind == "delete" && operation.phase == "deleted"
            || operation.kind == "case_rename")
        {
            return Err(FileError::NotFound);
        }
        let root = self.root(&operation.root_id)?;
        let _write = root.writes.try_lock().map_err(|_| FileError::Busy)?;
        validate_path(&operation.path, false)?;
        if operation.backup.len() != 41 || !operation.backup.ends_with(".recovery") {
            return Err(FileError::Storage);
        }
        let private = self.private()?;
        let parent = self.parent(&root, &operation.path)?;
        let (source, name) = if operation.kind == "case_rename" {
            (&parent, operation.temporary.as_str())
        } else {
            (&private, operation.backup.as_str())
        };
        native::no_replace(
            &source.dir,
            &source.absolute,
            name,
            &parent.dir,
            &parent.absolute,
            split(&operation.path).1,
        )?;
        native::sync_directory(&parent.dir)?;
        self.store
            .finish(id, "restored")
            .map_err(|_| FileError::Uncertain)?;
        Ok(self.reference(&root, &operation.path))
    }

    pub(crate) fn entry_revision(
        &self,
        root: &Root,
        reference: &FileRef,
    ) -> Result<(String, String)> {
        let parent = self.parent(root, &reference.path)?;
        let absolute = parent.absolute.join(split(&reference.path).1);
        let metadata = fs::symlink_metadata(&absolute)?;
        if native::is_link(&metadata) {
            return Err(FileError::Forbidden);
        }
        self.canonical_ref(root, &absolute)?;
        let key = native::metadata_key(&absolute, &metadata)?;
        if metadata.is_file() && native::links(&parent.file(split(&reference.path).1)?)? != 1 {
            return Err(FileError::Forbidden);
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(FileError::Forbidden);
        }
        Ok((
            if metadata.is_dir() {
                "directory"
            } else {
                "file"
            }
            .into(),
            format!(
                "m_{}",
                hash(format!(
                    "{key}:{}:{:?}:{}",
                    metadata.len(),
                    metadata.modified().ok(),
                    native::change_stamp(&metadata)
                ))
            ),
        ))
    }

    fn directory_safe(&self, root: &Root, path: &str) -> Result<()> {
        let start = Instant::now();
        let mut pending = vec![path.to_owned()];
        let mut count = 0;
        while let Some(path) = pending.pop() {
            let guard = Guard::root(&root.path, &root.identity)?.descend(&path)?;
            for entry in guard.dir.entries()? {
                count += 1;
                if count > self.limits.scan_entries
                    || start.elapsed() > Duration::from_millis(self.limits.search_millis)
                {
                    return Err(FileError::Limit);
                }
                let entry = entry?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| FileError::Forbidden)?;
                if protected_component(&name) {
                    return Err(FileError::Forbidden);
                }
                let next = format!("{path}/{name}");
                validate_path(&next, false)?;
                let metadata = fs::symlink_metadata(guard.absolute.join(&name))?;
                if native::is_link(&metadata) {
                    return Err(FileError::Forbidden);
                }
                if metadata.is_dir() {
                    pending.push(next);
                }
            }
        }
        Ok(())
    }

    pub fn draft(&self, id: &str) -> Result<DraftEnvelope> {
        let value = self.store.draft(id)?;
        if value.document.file.project_id != self.project_id {
            return Err(FileError::Forbidden);
        }
        validate_path(&value.document.file.path, false)?;
        Ok(value)
    }

    pub fn save_draft(&self, document: Draft, expected: u64) -> Result<DraftEnvelope> {
        self.resolve(&document.file)?;
        self.store.save_draft(document, expected, &self.limits)
    }

    pub fn discard_draft(&self, id: &str, expected: u64) -> Result<()> {
        self.store.discard(id, expected)
    }

    fn git_binding(&self, root: &Root) -> Result<GitBinding> {
        let lease = Guard::root(&root.path, &root.identity)?;
        let canonical = |value: &str| -> Result<PathBuf> {
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err(FileError::GitUnavailable);
            }
            fs::canonicalize(value).map_err(|_| FileError::GitUnavailable)
        };
        let layout = self.raw_git(
            root,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
                "--git-common-dir",
                "--git-dir",
            ],
            24_576,
            None,
        )?;
        let mut lines = std::str::from_utf8(&layout)
            .map_err(|_| FileError::GitUnavailable)?
            .lines();
        let observed_root = canonical(lines.next().ok_or(FileError::GitUnavailable)?)?;
        let common = canonical(lines.next().ok_or(FileError::GitUnavailable)?)?;
        let directory = canonical(lines.next().ok_or(FileError::GitUnavailable)?)?;
        if lines.next().is_some() || observed_root != root.path {
            return Err(FileError::Forbidden);
        }
        if !std::ptr::eq(root, self.primary.as_ref()) {
            let primary = self.raw_git(
                &self.primary,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                8192,
                None,
            )?;
            let primary = std::str::from_utf8(&primary).map_err(|_| FileError::GitUnavailable)?;
            if canonical(primary.trim_end_matches(['\r', '\n']))? != common {
                return Err(FileError::Forbidden);
            }
        }
        let common_handle = native::open_directory(&common)?;
        let key = native::key(&common_handle)?;
        {
            let mut expected = self.git_common.lock().map_err(|_| FileError::Busy)?;
            if let Some((path, identity)) = &*expected {
                if path != &common || identity != &key {
                    return Err(FileError::Forbidden);
                }
            } else {
                *expected = Some((common.clone(), key.clone()));
            }
        }
        let relative = directory
            .strip_prefix(&common)
            .map_err(|_| FileError::Forbidden)?
            .to_str()
            .ok_or(FileError::Forbidden)?
            .replace('\\', "/");
        let git = Guard::root(&common, &key)?.descend(&relative)?;
        Ok(GitBinding {
            directory,
            common,
            root: lease,
            git,
        })
    }

    pub(crate) fn git(&self, root: &Root, args: &[&str], limit: usize) -> Result<Vec<u8>> {
        let binding = self.git_binding(root)?;
        let result = self.raw_git(
            root,
            args,
            limit,
            Some((&binding.directory, &binding.common)),
        );
        binding.verify()?;
        result
    }

    fn raw_git(
        &self,
        root: &Root,
        args: &[&str],
        limit: usize,
        directories: Option<(&Path, &Path)>,
    ) -> Result<Vec<u8>> {
        let mut command = ProcCommand::new("git")
            .clear_environment()
            .arg("--no-optional-locks")
            .args([
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.untrackedCache=false",
            ])
            .arg("-C")
            .arg(factory_git::git_path(&root.path))
            .env("GIT_TERMINAL_PROMPT", "0");
        for (key, value) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_uppercase();
            if !name.starts_with("GIT_") && !name.starts_with("SSH_") {
                command = command.env(key, value);
            }
        }
        if let Some((git, common)) = directories {
            command = command
                .arg("--git-dir")
                .arg(factory_git::git_path(git))
                .arg("--work-tree")
                .arg(factory_git::git_path(&root.path))
                .env("GIT_COMMON_DIR", factory_git::git_path(common));
        }
        let output = command
            .args(args.iter().copied())
            .run(Bounds::new(Duration::from_secs(2), limit + 1, 4096))
            .map_err(|_| FileError::GitUnavailable)?;
        if !output.success() || output.stdout.len() > limit {
            return Err(FileError::GitUnavailable);
        }
        Ok(output.stdout)
    }

    pub fn git_head(&self, reference: &FileRef) -> Result<FileSnapshot> {
        let root = self.resolve(reference)?;
        self.read_raw(&root, reference)?;
        let commit = String::from_utf8(self.git(&root, &["rev-parse", "--verify", "HEAD"], 128)?)
            .map_err(|_| FileError::GitUnavailable)?
            .trim()
            .to_owned();
        if commit.len() != 40 || !commit.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(FileError::GitUnavailable);
        }
        let bytes = self.git(
            &root,
            &["cat-file", "blob", &format!("{commit}:{}", reference.path)],
            self.limits.edit_bytes,
        )?;
        if bytes.contains(&0) {
            return Err(FileError::Unsupported);
        }
        let size = bytes.len() as u64;
        let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        let content = std::str::from_utf8(&bytes[if bom { 3 } else { 0 }..])
            .map_err(|_| FileError::Unsupported)?
            .to_owned();
        let crlf = content
            .as_bytes()
            .windows(2)
            .filter(|pair| *pair == b"\r\n")
            .count();
        let lf = content.bytes().filter(|byte| *byte == b'\n').count();
        let newline = if crlf > 0 && crlf != lf {
            "mixed"
        } else if crlf > 0 {
            "crlf"
        } else {
            "lf"
        };
        Ok(FileSnapshot {
            file: reference.clone(),
            revision: format!("git:{commit}:{}", hash(&content)),
            size,
            content: Some(content),
            modified: String::new(),
            encoding: "utf-8".into(),
            bom,
            newline: newline.into(),
            read_only: true,
            reason: Some("git_revision".into()),
            mode: "text".into(),
            durability: None,
        })
    }
}
