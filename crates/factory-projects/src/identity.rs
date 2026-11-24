use std::fs::{File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

use crate::{ProjectError, Result};

#[derive(Debug)]
pub struct DirectoryIdentity {
    pub path: PathBuf,
    pub key: String,
    _handle: File,
}

impl DirectoryIdentity {
    pub fn open(path: &Path) -> Result<Self> {
        reject_network_path(path)?;
        let canonical = std::fs::canonicalize(path)?;
        reject_network_path(&canonical)?;
        if !canonical.is_dir() || canonical.to_str().is_none() {
            return Err(ProjectError::Invalid(
                "expected a directory with a Unicode path".into(),
            ));
        }
        let handle = open_directory(&canonical)?;
        let key = file_key(&handle)?;
        Ok(Self {
            path: canonical,
            key,
            _handle: handle,
        })
    }

    pub fn verify(&self) -> Result<()> {
        let current = Self::open(&self.path)?;
        if current.key != self.key {
            return Err(ProjectError::Unavailable(
                "directory identity changed; explicit relink required".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn file_identity(path: &Path) -> Result<String> {
    file_key(&File::open(path)?).map_err(ProjectError::from)
}

#[derive(Clone)]
pub struct ProjectPathPolicy {
    roots: Vec<std::sync::Arc<DirectoryIdentity>>,
}

impl ProjectPathPolicy {
    pub fn new(roots: &[PathBuf]) -> Result<Self> {
        if roots.is_empty() {
            return Err(ProjectError::Invalid(
                "at least one authorized directory is required".into(),
            ));
        }
        let roots = roots
            .iter()
            .map(|path| DirectoryIdentity::open(path).map(std::sync::Arc::new))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { roots })
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|root| root.path.clone()).collect()
    }

    pub fn authorize(&self, path: &Path) -> Result<DirectoryIdentity> {
        let identity = DirectoryIdentity::open(path)?;
        for allowed in &self.roots {
            if identity.path.starts_with(&allowed.path) {
                allowed.verify()?;
                return Ok(identity);
            }
        }
        Err(ProjectError::Forbidden)
    }

    pub fn destination(&self, path: &Path) -> Result<PathBuf> {
        if !path.is_absolute() || path.exists() || std::fs::symlink_metadata(path).is_ok() {
            return Err(ProjectError::Invalid(
                "destination must be an absolute, nonexistent directory".into(),
            ));
        }
        let name = path
            .file_name()
            .ok_or_else(|| ProjectError::Invalid("destination requires a name".into()))?;
        let parent = self.authorize(path.parent().ok_or(ProjectError::Forbidden)?)?;
        Ok(parent.path.join(name))
    }
}

pub fn validate_factory_paths(root: &Path) -> Result<()> {
    for relative in [
        ".factory",
        ".factory/db.sqlite3",
        ".factory/config.toml",
        ".factory/runtime.lock",
    ] {
        let path = root.join(relative);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(ProjectError::Invalid(format!(
                    "{relative} must not be a symlink or reparse point"
                )))
            }
            Ok(metadata) if relative == ".factory" && !metadata.is_dir() => {
                return Err(ProjectError::Invalid(".factory must be a directory".into()))
            }
            Ok(metadata) if relative != ".factory" && !metadata.is_file() => {
                return Err(ProjectError::Invalid(format!(
                    "{relative} must be a regular file"
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn reject_network_path(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if (value.starts_with("\\\\") && !value.starts_with("\\\\?\\"))
            || value.to_ascii_lowercase().starts_with("\\\\?\\unc\\")
        {
            return Err(ProjectError::Invalid(
                "UNC project roots are not supported".into(),
            ));
        }
    }
    let _ = path;
    Ok(())
}

fn is_link(metadata: &Metadata) -> bool {
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

fn open_directory(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS);
    }
    options.open(path)
}

fn file_key(file: &File) -> std::io::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(format!(
            "windows:{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
}
