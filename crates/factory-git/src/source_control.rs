use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use fs4::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{git_path, git_program, git_timeout, GitError, Repo, Result};
use factory_proc::{Bounds, ProcCommand};

const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const TEXT_LIMIT: usize = 1024 * 1024;
const ENTRY_LIMIT: usize = 10_000;
const PACKED_ADD_MIN_PATHS: usize = 256;

pub struct SourceTiming {
    operation: &'static str,
    last: Option<Instant>,
}

impl SourceTiming {
    pub fn start(operation: &'static str) -> Self {
        Self {
            operation,
            last: (std::env::var_os("ASF_GIT_TIMINGS").as_deref()
                == Some(std::ffi::OsStr::new("1")))
            .then(Instant::now),
        }
    }

    pub fn phase(&mut self, phase: &'static str) {
        if let Some(last) = self.last.as_mut() {
            let now = Instant::now();
            let _ = writeln!(
                std::io::stderr().lock(),
                "asf_git_timing operation={} phase={phase} elapsed_us={}",
                self.operation,
                now.duration_since(*last).as_micros()
            );
            *last = now;
        }
    }
}

enum SourceCommand<'a> {
    Standard,
    Input(File),
    PackedAdd,
    BoundStatus(&'a Path),
}

pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub original_path: Option<String>,
    pub index: String,
    pub working: String,
    pub conflict: bool,
    pub supported: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
    pub index_revision: String,
    pub generation: String,
    pub changes: Vec<Change>,
    pub operation: Option<String>,
    pub observation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    WorkingIndex,
    IndexHead,
    WorkingHead,
    Commits,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiffRequest {
    pub path: String,
    pub comparison: Comparison,
    pub base: Option<String>,
    pub target: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub id: String,
    pub header: String,
    pub patch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub path: String,
    pub comparison: Comparison,
    pub base: String,
    pub target: String,
    pub generation: String,
    pub version: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub patch: String,
    pub hunks: Vec<Hunk>,
    pub limited: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexRequest {
    pub expected_generation: String,
    pub paths: Vec<String>,
    pub unstage: bool,
    pub hunk: Option<String>,
    pub diff_version: Option<String>,
    #[serde(default)]
    pub trust_programs: bool,
}

impl IndexRequest {
    pub fn validate_selection(&self) -> Result<()> {
        if self.paths.is_empty()
            || self.paths.len() > ENTRY_LIMIT
            || self.paths.iter().map(String::len).sum::<usize>() > TEXT_LIMIT
            || self.hunk.is_some() && self.paths.len() != 1
        {
            return Err(GitError::Invalid(
                "Select between 1 and 10000 paths within 1 MiB; a hunk belongs to one path".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub commit: String,
    pub current: bool,
    pub upstream: String,
    pub tracking: String,
    pub worktree: String,
    pub protected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: String,
    pub subject: String,
}

pub struct MutationGuard(File);

impl Drop for MutationGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

struct IndexGuard {
    path: PathBuf,
    file: Option<File>,
    published: bool,
}

impl IndexGuard {
    fn acquire(index: &Path) -> Result<Self> {
        let path = index.with_file_name(format!(
            "{}.lock",
            index.file_name().unwrap_or_default().to_string_lossy()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    GitError::Busy
                } else {
                    GitError::Io(error)
                }
            })?;
        Ok(Self {
            path,
            file: Some(file),
            published: false,
        })
    }

    fn publish(mut self, index: &Path, content: &[u8]) -> Result<()> {
        let mut file = self.file.take().ok_or(GitError::Busy)?;
        file.write_all(content).map_err(GitError::Io)?;
        file.sync_all().map_err(GitError::Io)?;
        drop(file);
        fs::rename(&self.path, index).map_err(GitError::Io)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for IndexGuard {
    fn drop(&mut self) {
        self.file.take();
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Repo {
    pub fn mutation_guard(&self) -> Result<MutationGuard> {
        self.mutation_guard_with_timeout(std::time::Duration::from_secs(5))
    }

    pub(crate) fn runtime_mutation_guard(&self) -> Result<MutationGuard> {
        self.mutation_guard_with_timeout(git_timeout())
    }

    fn mutation_guard_with_timeout(&self, timeout: std::time::Duration) -> Result<MutationGuard> {
        let path = self.common_directory()?.join("asf-git-operation.lock");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| linked(&metadata)) {
            return Err(GitError::Protected("Git operation lock is a link".into()));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(GitError::Io)?;
        let started = std::time::Instant::now();
        loop {
            match FileExt::try_lock(&file) {
                Ok(()) => break,
                Err(fs4::TryLockError::WouldBlock) if started.elapsed() < timeout => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                Err(_) => return Err(GitError::Busy),
            }
        }
        Ok(MutationGuard(file))
    }

    fn sc_command(&self, args: &[&str], index: Option<&Path>) -> Result<factory_proc::ProcOutput> {
        self.sc_command_with(args, index, SourceCommand::Standard)
    }

    fn sc_command_with(
        &self,
        args: &[&str],
        index: Option<&Path>,
        mode: SourceCommand<'_>,
    ) -> Result<factory_proc::ProcOutput> {
        let mut command = ProcCommand::new(git_program())
            .clear_environment()
            .arg("-C")
            .arg(git_path(&self.root))
            .args([
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.untrackedCache=false",
                "-c",
                "submodule.recurse=false",
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .env("LC_ALL", "C");
        for (key, value) in std::env::vars_os() {
            let key_text = key.to_string_lossy().to_ascii_uppercase();
            if !key_text.starts_with("GIT_") && !key_text.starts_with("SSH_") {
                command = command.env(key, value);
            }
        }
        if let Some(index) = index {
            command = command.env("GIT_INDEX_FILE", git_path(index));
        }
        if let SourceCommand::BoundStatus(directory) = mode {
            command = command
                .arg("--git-dir")
                .arg(git_path(directory))
                .arg("--work-tree")
                .arg(git_path(&self.root))
                .env("GIT_COMMON_DIR", git_path(&self.common));
        }
        if matches!(mode, SourceCommand::PackedAdd) {
            command = command.args(["-c", "core.bigFileThreshold=0"]);
        }
        if !matches!(
            args.first().copied(),
            Some(
                "add"
                    | "apply"
                    | "reset"
                    | "rm"
                    | "commit"
                    | "switch"
                    | "rev-parse"
                    | "rev-list"
                    | "for-each-ref"
                    | "check-attr"
            )
        ) {
            let configuration = command
                .clone()
                .args([
                    "config",
                    "--null",
                    "--name-only",
                    "--get-regexp",
                    "^filter\\.",
                ])
                .run(Bounds::new(git_timeout(), 64 * 1024, 16 * 1024))
                .map_err(|_| {
                    GitError::Protected(
                        "Cannot verify Git filter configuration before safe inspection".into(),
                    )
                })?;
            if !matches!(configuration.status.code(), Some(0 | 1)) || configuration.stdout_truncated
            {
                return Err(GitError::Protected(
                    "Cannot safely inspect Git filter configuration".into(),
                ));
            }
            for key in configuration
                .stdout
                .split(|byte| *byte == 0)
                .filter(|value| !value.is_empty())
            {
                let key = std::str::from_utf8(key)
                    .map_err(|_| GitError::Invalid("Non-UTF-8 filter configuration".into()))?;
                if !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
                {
                    return Err(GitError::Protected(
                        "Unsupported Git filter driver identifier".into(),
                    ));
                }
                let value = if key.ends_with(".required") {
                    "false"
                } else {
                    ""
                };
                command = command.arg("-c").arg(format!("{key}={value}"));
            }
        }
        command = command.args(args);
        let bounds = Bounds::new(git_timeout(), OUTPUT_LIMIT, 16 * 1024);
        let output = match mode {
            SourceCommand::Input(file) => command.run_with_input_file(bounds, file),
            _ => command.run(bounds),
        }
        .map_err(|error| match error {
            factory_proc::ProcError::Spawn(error) | factory_proc::ProcError::Io(error) => {
                GitError::Io(error)
            }
            _ => GitError::TimedOut {
                operation: args.first().copied().unwrap_or("source control").into(),
                seconds: git_timeout().as_secs(),
            },
        })?;
        if output.stdout_truncated {
            return Err(GitError::OutputLimitExceeded {
                operation: args.first().copied().unwrap_or("inspection").into(),
                limit: OUTPUT_LIMIT,
            });
        }
        Ok(output)
    }

    fn sc(&self, args: &[&str], index: Option<&Path>) -> Result<Vec<u8>> {
        self.sc_with(args, index, SourceCommand::Standard)
    }

    fn sc_with(
        &self,
        args: &[&str],
        index: Option<&Path>,
        mode: SourceCommand<'_>,
    ) -> Result<Vec<u8>> {
        let output = self.sc_command_with(args, index, mode)?;
        if !output.success() {
            return Err(GitError::CommandFailed(format!(
                "{} exited {:?}: {}",
                args.first().unwrap_or(&"git"),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output.stdout)
    }

    fn sc_text(&self, args: &[&str]) -> Result<String> {
        String::from_utf8(self.sc(args, None)?)
            .map_err(|_| GitError::Invalid("Git returned non-UTF-8 metadata".into()))
    }

    fn sc_path(&self, name: &str) -> Result<PathBuf> {
        let text = self.sc_text(&["rev-parse", "--path-format=absolute", "--git-path", name])?;
        Ok(PathBuf::from(text.trim()))
    }

    pub fn source_status(&self) -> Result<Status> {
        self.source_status_at(&self.sc_path("index")?)
    }

    fn source_status_at(&self, index: &Path) -> Result<Status> {
        self.observe_source_status(index, SourceCommand::Standard)
    }

    /// Observe a worktree whose root and Git directory identities are held by the caller.
    /// Git uses the bound directories throughout the filter probe and the status command.
    pub fn source_status_for_bound_worktree(
        root: &Path,
        common: &Path,
        directory: &Path,
    ) -> Result<Status> {
        let repo = Self {
            root: root.to_path_buf(),
            common: common.to_path_buf(),
        };
        repo.observe_source_status(
            &directory.join("index"),
            SourceCommand::BoundStatus(directory),
        )
    }

    fn observe_source_status(&self, index: &Path, mode: SourceCommand<'_>) -> Result<Status> {
        let mut timing = SourceTiming::start("source_status");
        let raw = self.sc_with(
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--ignore-submodules=all",
            ],
            None,
            mode,
        )?;
        timing.phase("git_porcelain_and_filter_probe");
        let mut status = parse_status(&raw)?;
        let index_bytes = read_optional(index, OUTPUT_LIMIT)?;
        status.index_revision = digest(&index_bytes);
        let mut hash = Sha256::new();
        hash.update(&raw);
        hash.update(&index_bytes);
        let mut remaining = 64 * 1024 * 1024;
        for change in &mut status.changes {
            change.supported =
                validate_path(&change.path).is_ok() && check_path(&self.root, &change.path).is_ok();
            if !change.supported {
                continue;
            }
            let path = self.root.join(&change.path);
            match fs::symlink_metadata(&path) {
                Ok(meta) if !linked(&meta) && meta.is_file() => {
                    hash.update(meta.len().to_le_bytes());
                    hash.update(format!("{:?}", meta.modified().ok()));
                    if meta.len() <= TEXT_LIMIT as u64 && remaining >= meta.len() as usize {
                        let data = read_optional(&path, TEXT_LIMIT)?;
                        remaining -= data.len();
                        hash.update(data);
                    } else {
                        change.supported = false;
                    }
                }
                Ok(_) => change.supported = false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(GitError::Io(error)),
            }
        }
        status.generation = format!("{:x}", hash.finalize());
        for name in [
            "MERGE_HEAD",
            "rebase-merge",
            "rebase-apply",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
        ] {
            if index
                .parent()
                .is_some_and(|parent| parent.join(name).exists())
            {
                status.operation = Some(name.into());
                break;
            }
        }
        timing.phase("index_and_working_fingerprint");
        Ok(status)
    }

    fn source_blob(&self, revision: &str, path: &str) -> Result<Vec<u8>> {
        let object = format!("{revision}:{path}");
        let output = self.sc_command(&["cat-file", "-s", &object], None)?;
        if !output.success() {
            return Ok(Vec::new());
        }
        let size = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<usize>()
            .map_err(|_| GitError::Invalid("Invalid Git object size".into()))?;
        if size > TEXT_LIMIT {
            return Err(GitError::OutputLimitExceeded {
                operation: "blob".into(),
                limit: TEXT_LIMIT,
            });
        }
        self.sc(&["cat-file", "blob", &object], None)
    }

    pub fn source_diff(&self, request: &DiffRequest) -> Result<Diff> {
        validate_path(&request.path)?;
        check_path(&self.root, &request.path)?;
        let status = self.source_status()?;
        let head = status.head.clone();
        let original_path = status
            .changes
            .iter()
            .find(|change| change.path == request.path)
            .and_then(|change| change.original_path.as_deref())
            .unwrap_or(&request.path);
        validate_path(original_path)?;
        let (base, target, before, after) = match request.comparison {
            Comparison::WorkingIndex => (
                format!("index:{}", status.index_revision),
                "working_tree".into(),
                self.source_blob("", &request.path),
                read_optional(&self.root.join(&request.path), TEXT_LIMIT),
            ),
            Comparison::IndexHead => (
                head.clone().unwrap_or_else(|| "unborn".into()),
                format!("index:{}", status.index_revision),
                head.as_ref()
                    .map_or(Ok(Vec::new()), |h| self.source_blob(h, original_path)),
                self.source_blob("", &request.path),
            ),
            Comparison::WorkingHead => (
                head.clone().unwrap_or_else(|| "unborn".into()),
                "working_tree".into(),
                head.as_ref()
                    .map_or(Ok(Vec::new()), |h| self.source_blob(h, original_path)),
                read_optional(&self.root.join(&request.path), TEXT_LIMIT),
            ),
            Comparison::Commits => {
                let base = request
                    .base
                    .as_deref()
                    .ok_or_else(|| GitError::Invalid("Base commit required".into()))?;
                let target = request
                    .target
                    .as_deref()
                    .ok_or_else(|| GitError::Invalid("Target commit required".into()))?;
                validate_sha(base)?;
                validate_sha(target)?;
                self.sc(&["cat-file", "-e", &format!("{base}^{{commit}}")], None)?;
                self.sc(&["cat-file", "-e", &format!("{target}^{{commit}}")], None)?;
                let original = self.source_original_path(base, target, &request.path)?;
                (
                    base.into(),
                    target.into(),
                    self.source_blob(base, original.as_deref().unwrap_or(&request.path)),
                    self.source_blob(target, &request.path),
                )
            }
        };
        let mut limited = false;
        let mut reason = None;
        let mut text = |result: Result<Vec<u8>>| -> Result<Option<String>> {
            match result {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(value) if !value.contains('\0') => Ok(Some(value)),
                    _ => {
                        limited = true;
                        reason = Some("Binary content; text/hunk staging unavailable".into());
                        Ok(None)
                    }
                },
                Err(GitError::OutputLimitExceeded { .. }) => {
                    limited = true;
                    reason = Some("File exceeds the 1 MiB text comparison limit".into());
                    Ok(None)
                }
                Err(error) => Err(error),
            }
        };
        let before = text(before)?;
        let after = text(after)?;
        let patch = if !limited {
            let a = tempfile::NamedTempFile::new().map_err(GitError::Io)?;
            let b = tempfile::NamedTempFile::new().map_err(GitError::Io)?;
            fs::write(a.path(), before.as_deref().unwrap_or("")).map_err(GitError::Io)?;
            fs::write(b.path(), after.as_deref().unwrap_or("")).map_err(GitError::Io)?;
            let output = self.sc_command(
                &[
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--unified=3",
                    "--",
                    &git_path(a.path()),
                    &git_path(b.path()),
                ],
                None,
            )?;
            if !matches!(output.status.code(), Some(0 | 1)) {
                return Err(GitError::CommandFailed("Text comparison failed".into()));
            }
            let raw = String::from_utf8(output.stdout)
                .map_err(|_| GitError::Invalid("Non-UTF-8 diff".into()))?;
            let body = raw.find("@@ ").map(|start| &raw[start..]).unwrap_or("");
            if body.is_empty() {
                String::new()
            } else {
                let change = status
                    .changes
                    .iter()
                    .find(|change| change.path == request.path);
                let added = matches!(request.comparison, Comparison::WorkingIndex)
                    && change.is_some_and(|c| c.index == "?")
                    || matches!(request.comparison, Comparison::IndexHead)
                        && change.is_some_and(|c| c.index == "A");
                let deleted = matches!(request.comparison, Comparison::WorkingIndex)
                    && change.is_some_and(|c| c.working == "D")
                    || matches!(request.comparison, Comparison::IndexHead)
                        && change.is_some_and(|c| c.index == "D");
                let left = if added {
                    "/dev/null".into()
                } else {
                    format!("a/{}", request.path)
                };
                let right = if deleted {
                    "/dev/null".into()
                } else {
                    format!("b/{}", request.path)
                };
                format!("--- {left}\n+++ {right}\n{body}")
            }
        } else {
            String::new()
        };
        let version = digest(format!("{}\0{base}\0{target}\0{patch}", status.generation));
        let eligible = !limited
            && !status
                .changes
                .iter()
                .any(|c| c.path == request.path && (c.conflict || c.original_path.is_some()))
            && before
                .as_ref()
                .is_some_and(|v| !v.contains('\r') && (v.is_empty() || v.ends_with('\n')))
            && after
                .as_ref()
                .is_some_and(|v| !v.contains('\r') && (v.is_empty() || v.ends_with('\n')));
        let hunks = if eligible {
            split_hunks(&patch, &version)
        } else {
            Vec::new()
        };
        Ok(Diff {
            path: request.path.clone(),
            comparison: request.comparison.clone(),
            base,
            target,
            generation: status.generation,
            version,
            before,
            after,
            patch,
            hunks,
            limited,
            reason,
        })
    }

    pub fn source_index(&self, request: &IndexRequest) -> Result<Status> {
        request.validate_selection()?;
        let mut timing = SourceTiming::start("index");
        let _common = self.mutation_guard()?;
        timing.phase("common_lock");
        let index = self.sc_path("index")?;
        let guard = IndexGuard::acquire(&index)?;
        timing.phase("index_lock");
        let status = self.source_status_at(&index)?;
        self.source_writable(&status)?;
        timing.phase("snapshot_and_ownership");
        if status.generation != request.expected_generation {
            return Err(GitError::Stale);
        }
        let supported: HashSet<&str> = status
            .changes
            .iter()
            .filter(|change| change.supported && !change.conflict)
            .flat_map(|change| {
                std::iter::once(change.path.as_str()).chain(change.original_path.as_deref())
            })
            .collect();
        for path in &request.paths {
            validate_path(path)?;
            check_path(&self.root, path)?;
            if !supported.contains(path.as_str()) {
                return Err(GitError::Protected("Selected path has no supported, fully fingerprinted change; refresh or use an authorized external Git client for oversized paths".into()));
            }
        }
        timing.phase("paths");
        self.source_filters(&request.paths, None)?;
        self.source_trust(request.trust_programs)?;
        timing.phase("attributes_and_trust");
        let directory = tempfile::tempdir_in(index.parent().ok_or(GitError::NotARepository)?)
            .map_err(GitError::Io)?;
        let alternate = directory.path().join("index");
        if index.exists() {
            fs::copy(&index, &alternate).map_err(GitError::Io)?;
        }
        if let Some(hunk) = &request.hunk {
            let diff = self.source_diff(&DiffRequest {
                path: request.paths[0].clone(),
                comparison: if request.unstage {
                    Comparison::IndexHead
                } else {
                    Comparison::WorkingIndex
                },
                base: None,
                target: None,
            })?;
            if request.diff_version.as_ref() != Some(&diff.version) {
                return Err(GitError::Stale);
            }
            let selected = diff
                .hunks
                .iter()
                .find(|h| &h.id == hunk)
                .ok_or(GitError::Stale)?;
            let patch = directory.path().join("selected.patch");
            fs::write(&patch, &selected.patch).map_err(GitError::Io)?;
            let mut args = vec!["apply", "--cached", "--whitespace=nowarn"];
            if request.unstage {
                args.push("--reverse");
            }
            args.extend(["--", patch.to_str().ok_or(GitError::NotARepository)?]);
            self.sc(&args, Some(&alternate))?;
        } else {
            let pathspec = directory.path().join("selected.paths");
            let mut selected = Vec::new();
            for path in &request.paths {
                selected.extend_from_slice(path.as_bytes());
                selected.push(0);
            }
            fs::write(&pathspec, selected).map_err(GitError::Io)?;
            let pathspec = format!("--pathspec-from-file={}", git_path(&pathspec));
            let mut args = if request.unstage {
                if status.head.is_some() {
                    vec!["reset", "--quiet", "HEAD"]
                } else {
                    vec!["rm", "--cached", "--quiet", "--ignore-unmatch"]
                }
            } else {
                vec!["add", "--all"]
            };
            args.extend([pathspec.as_str(), "--pathspec-file-nul"]);
            let mode = if !request.unstage && request.paths.len() >= PACKED_ADD_MIN_PATHS {
                SourceCommand::PackedAdd
            } else {
                SourceCommand::Standard
            };
            self.sc_with(&args, Some(&alternate), mode)?;
        }
        timing.phase("git_mutation");
        if self.source_status_at(&index)?.generation != status.generation {
            return Err(GitError::Stale);
        }
        timing.phase("generation_recheck");
        let bytes = read_optional(&alternate, OUTPUT_LIMIT)?;
        guard.publish(&index, &bytes)?;
        timing.phase("index_publication");
        let result = self.source_status_at(&index);
        timing.phase("response_snapshot");
        result
    }

    fn source_writable(&self, status: &Status) -> Result<()> {
        if let Some(branch) = &status.branch {
            if protected_branch(branch) {
                return Err(GitError::Protected("ASF owns this branch".into()));
            }
            if status.head.is_some() {
                let reference = format!("refs/heads/{branch}");
                let refs = self.sc_text(&[
                    "for-each-ref",
                    "--count=2",
                    "--format=%(refname)",
                    &reference,
                ])?;
                if !refs.lines().any(|value| value == reference) {
                    return Err(GitError::Protected(
                        "Current branch spelling does not match its stored ref".into(),
                    ));
                }
            }
            if self.source_branch_owners(branch)? > 1 {
                return Err(GitError::Protected(
                    "Branch ownership is shared or ambiguous across worktrees".into(),
                ));
            }
        }
        if status.operation.is_some() || status.changes.iter().any(|c| c.conflict) {
            return Err(GitError::Protected(
                "An unresolved Git operation requires its owning recovery workflow".into(),
            ));
        }
        Ok(())
    }

    fn source_branch_owners(&self, branch: &str) -> Result<usize> {
        let reference = format!("refs/heads/{branch}");
        let catalog = self.sc_text(&[
            "for-each-ref",
            "--count=501",
            "--format=%(refname)",
            "refs/heads/",
        ])?;
        let refs: Vec<_> = catalog.lines().collect();
        if refs.len() > 500 {
            return Err(GitError::OutputLimitExceeded {
                operation: "worktree branch identities".into(),
                limit: 500,
            });
        }
        let output = self.sc(&["worktree", "list", "--porcelain", "-z"], None)?;
        let mut unborn = false;
        let mut owners = 0;
        for field in output.split(|byte| *byte == 0) {
            if field.starts_with(b"worktree ") {
                unborn = false;
            } else if let Some(head) = field.strip_prefix(b"HEAD ") {
                unborn = !head.is_empty() && head.iter().all(|byte| *byte == b'0');
            } else if let Some(owner) = field.strip_prefix(b"branch ") {
                let owner = std::str::from_utf8(owner)
                    .map_err(|_| GitError::Invalid("Non-UTF-8 worktree ref".into()))?;
                if !unborn && !refs.contains(&owner) {
                    return Err(GitError::Protected("A registered worktree has a noncanonical or missing branch ref; reconcile Git ownership first".into()));
                }
                owners += usize::from(owner.eq_ignore_ascii_case(&reference));
            }
        }
        Ok(owners)
    }

    pub fn source_trust(&self, trusted: bool) -> Result<()> {
        let config = self.sc_command(&["config", "--get-regexp", "^(filter\\.|core\\.hooksPath|core\\.fsmonitor|commit\\.gpgsign|gpg\\.|core\\.sshCommand)"], None)?;
        let settings = match config.status.code() {
            Some(0) => String::from_utf8(config.stdout)
                .map_err(|_| GitError::Invalid("Non-UTF-8 trust configuration".into()))?,
            Some(1) => String::new(),
            _ => {
                return Err(GitError::Protected(
                    "Cannot verify configured Git programs".into(),
                ))
            }
        };
        let hooks = self.sc_path("hooks")?;
        let active_hooks = fs::read_dir(&hooks)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .any(|entry| {
                !entry.file_name().to_string_lossy().ends_with(".sample")
                    && entry
                        .file_type()
                        .is_ok_and(|t| t.is_file() || t.is_symlink())
            });
        let helper_settings = settings.lines().any(|line| {
            !line.starts_with("filter.")
                && !matches!(line, "commit.gpgsign false" | "core.fsmonitor false")
        });
        if !trusted && (active_hooks || helper_settings) {
            return Err(GitError::TrustRequired("Configured hooks, signing or helper programs may execute code. Review project trust before authorizing this operation".into()));
        }
        let tracked = self.sc(&["ls-files", "-z"], None)?;
        let tracked = String::from_utf8(tracked)
            .map_err(|_| GitError::Invalid("Non-UTF-8 tracked paths".into()))?;
        let paths: Vec<String> = tracked
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(String::from)
            .collect();
        if paths.len() > ENTRY_LIMIT {
            return Err(GitError::OutputLimitExceeded {
                operation: "filter trust inspection".into(),
                limit: ENTRY_LIMIT,
            });
        }
        self.source_filters(&paths, None)?;
        Ok(())
    }

    fn source_filters(&self, paths: &[String], source: Option<&str>) -> Result<()> {
        let mut start = 0;
        while start < paths.len() {
            let mut end = start;
            let mut input = Vec::new();
            while end < paths.len() {
                let path = &paths[end];
                if path.len() >= TEXT_LIMIT || path.as_bytes().contains(&0) {
                    return Err(GitError::Invalid(
                        "Path exceeds bounded Git attribute input".into(),
                    ));
                }
                if input.len() + path.len() + 1 > TEXT_LIMIT {
                    break;
                }
                input.extend_from_slice(path.as_bytes());
                input.push(0);
                end += 1;
            }
            let chunk = &paths[start..end];
            let source = source.map(|value| format!("--source={value}"));
            let mut args = vec!["check-attr", "-z", "--stdin"];
            if let Some(source) = &source {
                args.push(source);
            }
            args.push("filter");
            let mut file = tempfile::tempfile().map_err(GitError::Io)?;
            file.write_all(&input).map_err(GitError::Io)?;
            file.rewind().map_err(GitError::Io)?;
            let output = self.sc_with(&args, None, SourceCommand::Input(file))?;
            let body = output
                .strip_suffix(&[0])
                .ok_or_else(|| GitError::Invalid("Incomplete Git attribute response".into()))?;
            let fields: Vec<_> = body.split(|b| *b == 0).collect();
            let (rows, remainder) = fields.as_chunks::<3>();
            if !remainder.is_empty()
                || rows.len() != chunk.len()
                || rows
                    .iter()
                    .zip(chunk)
                    .any(|(row, path)| row[0] != path.as_bytes() || row[1] != b"filter")
            {
                return Err(GitError::Invalid(
                    "Git attribute response does not match the inspected paths".into(),
                ));
            }
            if rows
                .iter()
                .any(|triple| !matches!(triple[2], b"unspecified" | b"unset"))
            {
                return Err(GitError::Protected("The selected files use clean/smudge filters; this manual operation does not authorize them".into()));
            }
            start = end;
        }
        Ok(())
    }

    pub fn source_commit(&self, expected: &str, message: &str, trusted: bool) -> Result<String> {
        self.source_commit_authorized(expected, message, trusted, |_| Ok(()))
    }

    pub fn source_commit_authorized(
        &self,
        expected: &str,
        message: &str,
        trusted: bool,
        authorize: impl Fn(&str) -> Result<()>,
    ) -> Result<String> {
        let mut timing = SourceTiming::start("commit");
        let _common = self.mutation_guard()?;
        timing.phase("common_lock");
        let index = self.sc_path("index")?;
        let guard = IndexGuard::acquire(&index)?;
        timing.phase("index_lock");
        let status = self.source_status_at(&index)?;
        self.source_writable(&status)?;
        timing.phase("snapshot_and_ownership");
        if status.generation != expected {
            return Err(GitError::Stale);
        }
        if message.trim().is_empty() || message.len() > 8192 || message.contains('\0') {
            return Err(GitError::Invalid(
                "Commit message must contain 1-8192 bytes".into(),
            ));
        }
        let staged: Vec<_> = status
            .changes
            .iter()
            .filter(|c| c.index != "." && c.index != "?")
            .collect();
        if staged.is_empty() {
            return Err(GitError::Invalid("No staged changes".into()));
        }
        for change in staged {
            validate_path(&change.path)?;
            authorize(&change.path)?;
            if let Some(path) = &change.original_path {
                validate_path(path)?;
                authorize(path)?;
            }
        }
        self.source_trust(trusted)?;
        timing.phase("policy_and_trust");
        if self.source_status_at(&index)?.generation != expected {
            return Err(GitError::Stale);
        }
        timing.phase("generation_recheck");
        let directory = tempfile::tempdir_in(index.parent().ok_or(GitError::NotARepository)?)
            .map_err(GitError::Io)?;
        let alternate = directory.path().join("index");
        fs::copy(&index, &alternate).map_err(GitError::Io)?;
        self.sc(&["commit", "-m", message, "--"], Some(&alternate))?;
        timing.phase("git_commit");
        let sha = self.sc_text(&["rev-parse", "HEAD"])?.trim().to_string();
        validate_sha(&sha)?;
        let parent = self.sc_text(&["rev-list", "--parents", "-n", "1", &sha])?;
        if status
            .head
            .as_ref()
            .is_some_and(|head| parent.split_whitespace().nth(1) != Some(head))
        {
            return Err(GitError::Protected(
                "Commit parent changed; inspect actual HEAD before retrying".into(),
            ));
        }
        guard.publish(&index, &read_optional(&alternate, OUTPUT_LIMIT)?)?;
        timing.phase("verification_and_index_publication");
        Ok(sha)
    }

    pub fn source_branches(&self) -> Result<Vec<Branch>> {
        let output = self.sc_text(&["for-each-ref", "--count=501", "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track)%00%(worktreepath)", "refs/heads/"])?;
        let mut branches = Vec::new();
        for line in output.lines() {
            let fields: Vec<_> = line.split('\0').collect();
            if fields.len() != 6 {
                return Err(GitError::Invalid("Unrepresentable branch metadata".into()));
            }
            branches.push(Branch {
                name: fields[0]
                    .strip_prefix("refs/heads/")
                    .ok_or_else(|| GitError::Invalid("Unexpected branch namespace".into()))?
                    .into(),
                commit: fields[1].into(),
                current: fields[2] == "*",
                upstream: fields[3].into(),
                tracking: fields[4].into(),
                worktree: fields[5].into(),
                protected: protected_branch(
                    fields[0].strip_prefix("refs/heads/").unwrap_or("refs/"),
                ),
            });
        }
        if branches.len() > 500 {
            return Err(GitError::OutputLimitExceeded {
                operation: "branches".into(),
                limit: 500,
            });
        }
        Ok(branches)
    }

    pub fn source_branch(
        &self,
        expected: &str,
        branch: &str,
        create: bool,
        trusted: bool,
    ) -> Result<Status> {
        self.source_branch_with_target_check(expected, branch, create, trusted, |_, _, _| Ok(()))
    }

    pub fn source_branch_with_target_check(
        &self,
        expected: &str,
        branch: &str,
        create: bool,
        trusted: bool,
        check_target: impl FnOnce(&Self, &Status, &str) -> Result<()>,
    ) -> Result<Status> {
        validate_branch(branch)?;
        let _common = self.mutation_guard()?;
        let status = self.source_status()?;
        self.source_writable(&status)?;
        if status.generation != expected {
            return Err(GitError::Stale);
        }
        if !status.changes.is_empty() {
            return Err(GitError::Protected(
                "Save and reconcile all staged, unstaged and untracked changes before checkout"
                    .into(),
            ));
        }
        let branches = self.source_branches()?;
        let target = if create {
            if branches
                .iter()
                .any(|value| value.name.eq_ignore_ascii_case(branch))
            {
                return Err(GitError::Invalid(
                    "Branch already exists or has a case alias".into(),
                ));
            }
            status.head.clone().ok_or_else(|| {
                GitError::Invalid("Initial commit required before branch creation".into())
            })?
        } else {
            let selected = branches
                .into_iter()
                .find(|value| value.name == branch)
                .ok_or_else(|| {
                    GitError::Invalid("Select an exact existing local branch name".into())
                })?;
            if status.branch.as_deref() != Some(branch) && self.source_branch_owners(branch)? > 0 {
                return Err(GitError::Protected(
                    "Branch is owned by another worktree".into(),
                ));
            }
            check_target(self, &status, &selected.commit)?;
            selected.commit
        };
        let paths = self.sc(&["ls-tree", "-r", "--name-only", "-z", &target], None)?;
        let paths: Vec<String> = String::from_utf8(paths)
            .map_err(|_| GitError::Invalid("Non-UTF-8 tree paths".into()))?
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect();
        if paths.len() > ENTRY_LIMIT {
            return Err(GitError::OutputLimitExceeded {
                operation: "checkout files".into(),
                limit: ENTRY_LIMIT,
            });
        }
        for path in &paths {
            validate_path(path)?;
            check_path(&self.root, path)?;
        }
        self.source_filters(&paths, Some(&target))?;
        self.source_trust(trusted)?;
        if create {
            self.sc(&["switch", "--no-guess", "-c", branch], None)?;
        } else {
            if self.resolve_ref(branch)? != target {
                return Err(GitError::Stale);
            }
            self.sc(&["switch", "--no-guess", "--", branch], None)?;
        }
        self.source_status()
    }

    pub fn source_history(
        &self,
        start: Option<&str>,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<Commit>> {
        if !(1..=100).contains(&limit) || skip > 100_000 {
            return Err(GitError::Invalid("History bounds exceeded".into()));
        }
        if let Some(start) = start {
            validate_sha(start)?;
        }
        let head = self.sc_command(&["rev-parse", "--verify", "--quiet", "HEAD"], None)?;
        if head.status.code() == Some(1) {
            return Ok(Vec::new());
        }
        if !head.success() {
            return Err(GitError::CommandFailed(
                "Cannot inspect history HEAD".into(),
            ));
        }
        let output = self.sc_text(&[
            "log",
            "-z",
            "--no-show-signature",
            "--format=%H%x00%P%x00%an%x00%aI%x00%s",
            &format!("--skip={skip}"),
            &format!("--max-count={limit}"),
            start.unwrap_or("HEAD"),
            "--",
        ])?;
        let fields: Vec<_> = output.split('\0').collect();
        let mut commits = Vec::new();
        for chunk in fields.chunks(5) {
            if chunk.len() < 5 {
                break;
            }
            commits.push(Commit {
                sha: chunk[0].into(),
                parents: chunk[1].split_whitespace().map(String::from).collect(),
                author: chunk[2].into(),
                timestamp: chunk[3].into(),
                subject: chunk[4].into(),
            });
        }
        Ok(commits)
    }

    pub fn source_changed_paths(&self, base: &str, target: &str) -> Result<Vec<String>> {
        validate_sha(base)?;
        validate_sha(target)?;
        let raw = self.sc(
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=all",
                "--no-renames",
                "--name-only",
                "-z",
                base,
                target,
                "--",
            ],
            None,
        )?;
        raw.split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| {
                String::from_utf8(p.to_vec())
                    .map_err(|_| GitError::Invalid("Non-UTF-8 path".into()))
            })
            .collect()
    }

    fn source_original_path(&self, base: &str, target: &str, path: &str) -> Result<Option<String>> {
        let raw = self.sc(
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=all",
                "--name-status",
                "--find-renames",
                "-z",
                base,
                target,
                "--",
            ],
            None,
        )?;
        let mut fields = raw
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty());
        while let Some(kind) = fields.next() {
            let Some(first) = fields.next() else {
                return Err(GitError::Invalid("Invalid changed-path metadata".into()));
            };
            if matches!(kind.first(), Some(b'R' | b'C')) {
                let second = fields
                    .next()
                    .ok_or_else(|| GitError::Invalid("Missing renamed path".into()))?;
                if second == path.as_bytes() {
                    let original = String::from_utf8(first.to_vec())
                        .map_err(|_| GitError::Invalid("Non-UTF-8 original path".into()))?;
                    validate_path(&original)?;
                    return Ok(Some(original));
                }
            }
        }
        Ok(None)
    }

    pub fn source_conflict(&self, path: &str) -> Result<serde_json::Value> {
        validate_path(path)?;
        check_path(&self.root, path)?;
        let status = self.source_status()?;
        if !status
            .changes
            .iter()
            .any(|change| change.path == path && change.conflict)
        {
            return Err(GitError::Invalid(
                "No actual unmerged index entry exists for this path".into(),
            ));
        }
        let read = |stage: &str| -> Result<Option<String>> {
            let bytes = self.source_blob(stage, path)?;
            Ok(String::from_utf8(bytes)
                .ok()
                .filter(|text| !text.contains('\0')))
        };
        Ok(
            serde_json::json!({"path":path,"generation":status.generation,"indexRevision":status.index_revision,"operation":status.operation,"base":read(":1")?,"ours":read(":2")?,"theirs":read(":3")?,"readOnly":true,"resolution":"Use the owning integration coordinator's authorized repair/replan workflow; no side is automatically selected"}),
        )
    }
}

pub fn validate_sha(value: &str) -> Result<()> {
    if ![40, 64].contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(GitError::Invalid("Full commit identity required".into()));
    }
    Ok(())
}

pub fn protected_branch(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.starts_with("factory/") || value.starts_with("refs/")
}

pub fn validate_branch(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 200
        || value.starts_with(['-', '/'])
        || value.ends_with(['.', '/'])
        || value.contains(['\\', ' ', '~', '^', ':', '?', '*', '['])
        || value.contains("..")
        || value.contains("@{")
        || value.contains("//")
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|p| p.starts_with('.') || p.ends_with(".lock"))
        || protected_branch(value)
    {
        return Err(GitError::Invalid("Invalid or ASF-owned branch name".into()));
    }
    Ok(())
}

pub fn validate_path(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 2048
        || value.starts_with('/')
        || value.contains(['\\', '\0', ':'])
        || value.chars().any(char::is_control)
    {
        return Err(GitError::Invalid("Unsupported source path".into()));
    }
    for part in value.split('/') {
        let lower = part.to_ascii_lowercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.contains(['"', '<', '>', '|', '?', '*'])
        {
            return Err(GitError::Invalid("Unsupported portable source path".into()));
        }
        if matches!(
            lower.as_str(),
            ".git"
                | ".factory"
                | ".gitmodules"
                | ".env"
                | ".npmrc"
                | ".netrc"
                | "credentials"
                | "id_rsa"
                | "id_ed25519"
        ) || lower.starts_with(".asf-editor-")
            || lower.starts_with(".env.")
            || lower.ends_with(".pem")
            || lower.ends_with(".key")
        {
            return Err(GitError::Protected("Internal or sensitive path".into()));
        }
    }
    Ok(())
}

fn check_path(root: &Path, relative: &str) -> Result<()> {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if linked(&metadata) => {
                return Err(GitError::Protected("Linked/reparse source path".into()))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(GitError::Io(error)),
        }
    }
    Ok(())
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn read_optional(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(GitError::Io(error)),
    };
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(GitError::Io)?;
    if bytes.len() > limit {
        return Err(GitError::OutputLimitExceeded {
            operation: "source read".into(),
            limit,
        });
    }
    Ok(bytes)
}

fn parse_status(raw: &[u8]) -> Result<Status> {
    let text = std::str::from_utf8(raw)
        .map_err(|_| GitError::Invalid("Non-UTF-8 Git path; operation unavailable".into()))?;
    let mut result = Status {
        head: None,
        branch: None,
        upstream: None,
        ahead: None,
        behind: None,
        index_revision: String::new(),
        generation: String::new(),
        changes: Vec::new(),
        operation: None,
        observation:
            "Bounded Git/index/changed-file observation; not an atomic filesystem snapshot".into(),
    };
    let mut records = text.split('\0');
    while let Some(record) = records.next() {
        if let Some(head) = record.strip_prefix("# branch.oid ") {
            if head != "(initial)" {
                result.head = Some(head.into());
            }
        } else if let Some(branch) = record.strip_prefix("# branch.head ") {
            if branch != "(detached)" {
                result.branch = Some(branch.into());
            }
        } else if let Some(upstream) = record.strip_prefix("# branch.upstream ") {
            result.upstream = Some(upstream.into());
        } else if let Some(ab) = record.strip_prefix("# branch.ab ") {
            let mut parts = ab.split_whitespace();
            result.ahead = parts
                .next()
                .and_then(|s| s.strip_prefix('+'))
                .and_then(|s| s.parse().ok());
            result.behind = parts
                .next()
                .and_then(|s| s.strip_prefix('-'))
                .and_then(|s| s.parse().ok());
        } else if record.starts_with(['1', '2', 'u']) {
            let rename = record.starts_with('2');
            let conflict = record.starts_with('u');
            let fields: Vec<_> = record
                .splitn(
                    if rename {
                        10
                    } else if conflict {
                        11
                    } else {
                        9
                    },
                    ' ',
                )
                .collect();
            let xy = fields
                .get(1)
                .ok_or_else(|| GitError::Invalid("Malformed Git status".into()))?;
            if xy.len() != 2 {
                return Err(GitError::Invalid("Malformed Git status flags".into()));
            }
            let path = fields.last().unwrap_or(&"").to_string();
            result.changes.push(Change {
                path,
                original_path: if rename {
                    Some(
                        records
                            .next()
                            .ok_or_else(|| GitError::Invalid("Missing rename source".into()))?
                            .into(),
                    )
                } else {
                    None
                },
                index: xy[0..1].into(),
                working: xy[1..2].into(),
                conflict,
                supported: false,
            });
        } else if let Some(path) = record.strip_prefix("? ") {
            result.changes.push(Change {
                path: path.into(),
                original_path: None,
                index: "?".into(),
                working: "?".into(),
                conflict: false,
                supported: false,
            });
        }
        if result.changes.len() > ENTRY_LIMIT {
            return Err(GitError::OutputLimitExceeded {
                operation: "status".into(),
                limit: ENTRY_LIMIT,
            });
        }
    }
    Ok(result)
}

fn split_hunks(patch: &str, version: &str) -> Vec<Hunk> {
    let Some(start) = patch.find("@@ ") else {
        return Vec::new();
    };
    let header = &patch[..start];
    let rest = &patch[start..];
    let mut boundaries = vec![0];
    boundaries.extend(rest.match_indices("\n@@ ").map(|(i, _)| i + 1));
    boundaries.push(rest.len());
    boundaries
        .windows(2)
        .enumerate()
        .map(|(index, range)| {
            let body = &rest[range[0]..range[1]];
            Hunk {
                id: digest(format!("{version}:{index}:{body}")),
                header: body.lines().next().unwrap_or("").into(),
                patch: format!("{header}{body}"),
            }
        })
        .collect()
}
