use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{IdeError, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolProfile {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default = "stdio")]
    pub transport: String,
    pub executable: PathBuf,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub file_patterns: Vec<String>,
    #[serde(default)]
    pub initialization_options: Value,
    #[serde(default)]
    pub settings: Value,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub fingerprint: String,
}

fn timeout() -> u64 {
    15_000
}
fn stdio() -> String {
    "stdio".into()
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

pub fn identifier() -> String {
    format!("i_{:032x}", rand::random::<u128>())
}

impl ToolProfile {
    pub fn approve(&mut self, approved: bool) -> Result<()> {
        if !approved {
            return Err(IdeError::Unapproved);
        }
        if !valid_id(&self.id)
            || self.name.is_empty()
            || self.name.len() > 120
            || !["lsp", "dap", "shell"].contains(&self.kind.as_str())
            || !["stdio", "tcp"].contains(&self.transport.as_str())
            || self.transport == "tcp" && self.kind != "dap"
            || self.arguments.len() > 64
            || self
                .arguments
                .iter()
                .any(|s| s.len() > 4096 || s.contains('\0'))
            || self.languages.len() > 32
            || self.file_patterns.len() > 32
            || !(500..=120_000).contains(&self.timeout_ms)
            || serde_json::to_vec(&self.settings)
                .map_err(|_| IdeError::Invalid)?
                .len()
                > 65_536
            || serde_json::to_vec(&self.initialization_options)
                .map_err(|_| IdeError::Invalid)?
                .len()
                > 65_536
        {
            return Err(IdeError::Invalid);
        }
        self.executable = resolve_executable(&self.executable)?;
        self.fingerprint = self.current_fingerprint()?;
        Ok(())
    }

    pub fn verify(&self, kind: &str) -> Result<()> {
        if !self.enabled || self.kind != kind {
            return Err(IdeError::Forbidden);
        }
        if self.current_fingerprint()? != self.fingerprint {
            return Err(IdeError::Executable);
        }
        Ok(())
    }

    fn current_fingerprint(&self) -> Result<String> {
        let metadata = std::fs::metadata(&self.executable).map_err(|_| IdeError::Executable)?;
        if !metadata.is_file() || metadata.len() > 256 * 1024 * 1024 {
            return Err(IdeError::Executable);
        }
        let mut items = vec![hash_file(&self.executable, 256 * 1024 * 1024)?];
        for argument in &self.arguments {
            let path = Path::new(argument);
            if path.is_absolute() && path.is_file() {
                if std::fs::metadata(path)
                    .map_err(|_| IdeError::Executable)?
                    .len()
                    > 64 * 1024 * 1024
                {
                    return Err(IdeError::Executable);
                }
                items.push(hash_file(path, 64 * 1024 * 1024)?);
            }
        }
        Ok(digest(items.join(":")))
    }
}

fn hash_file(path: &Path, limit: u64) -> Result<String> {
    let file = std::fs::File::open(path).map_err(|_| IdeError::Executable)?;
    let mut reader = file.take(limit + 1);
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    let mut total = 0;
    loop {
        let count = reader.read(&mut bytes).map_err(|_| IdeError::Executable)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err(IdeError::Limit);
        }
        digest.update(&bytes[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn resolve_executable(path: &Path) -> Result<PathBuf> {
    let resolved = if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        if path.components().count() != 1 {
            return Err(IdeError::Executable);
        }
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).find_map(|dir| {
            let candidate = dir.join(path);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            if candidate.with_extension("exe").is_file() {
                return Some(candidate.with_extension("exe"));
            }
            None
        })
    }
    .ok_or(IdeError::Executable)?;
    let resolved = std::fs::canonicalize(resolved).map_err(|_| IdeError::Executable)?;
    #[cfg(windows)]
    if !resolved
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Err(IdeError::Executable);
    }
    Ok(resolved)
}

pub fn environment() -> BTreeMap<String, String> {
    let mut values: BTreeMap<_, _> = [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "LANG",
        "LC_ALL",
        "RUSTUP_HOME",
        "CARGO_HOME",
        "PATHEXT",
        "COMSPEC",
        "SystemDrive",
        "COMPUTERNAME",
        "USERNAME",
        "USERDOMAIN",
        "ProgramData",
        "ALLUSERSPROFILE",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "CommonProgramFiles",
        "CommonProgramFiles(x86)",
        "CommonProgramW6432",
        "OS",
        "PROCESSOR_ARCHITECTURE",
        "NUMBER_OF_PROCESSORS",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|value| (key.into(), value)))
    .collect();
    values.insert("PYTHONUNBUFFERED".into(), "1".into());
    values
}
