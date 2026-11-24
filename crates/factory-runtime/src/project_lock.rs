use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use factory_core::FACTORY_DIR;

use crate::RuntimeError;

pub(crate) const PROJECT_LOCK_FILE: &str = "runtime.lock";

pub(crate) struct ProjectLock {
    _file: File,
}

impl ProjectLock {
    pub fn acquire(canonical_root: &Path, requested_root: &Path) -> Result<Self, RuntimeError> {
        Self::acquire_file(
            canonical_root.join(FACTORY_DIR).join(PROJECT_LOCK_FILE),
            requested_root,
        )
    }

    fn acquire_file(
        lock_path: std::path::PathBuf,
        requested_root: &Path,
    ) -> Result<Self, RuntimeError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| RuntimeError::ProjectLock {
                lock: lock_path.clone(),
                source,
            })?;

        match fs4::FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(fs4::TryLockError::WouldBlock) => {
                return Err(RuntimeError::ProjectAlreadyLocked {
                    root: requested_root.to_path_buf(),
                    lock: lock_path,
                });
            }
            Err(fs4::TryLockError::Error(source)) => {
                return Err(RuntimeError::ProjectLock {
                    lock: lock_path,
                    source,
                });
            }
        }

        let _ = write_lock_metadata(&mut file, requested_root);
        Ok(Self { _file: file })
    }
}

fn write_lock_metadata(file: &mut File, root: &Path) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    writeln!(file, "pid={}", std::process::id())?;
    writeln!(file, "root={}", root.display())?;
    writeln!(file, "acquired_at={}", chrono::Utc::now().to_rfc3339())?;
    file.flush()
}

pub struct ProjectOwnership {
    _project: ProjectLock,
    _git: Option<ProjectLock>,
}

impl ProjectOwnership {
    pub fn acquire(root: &Path) -> Result<Self, RuntimeError> {
        let canonical =
            std::fs::canonicalize(root).map_err(|source| RuntimeError::ProjectRoot {
                root: root.to_owned(),
                source,
            })?;
        let project = ProjectLock::acquire(&canonical, root)?;
        let git = match factory_git::Repo::detect_bounded(&canonical, &canonical) {
            Ok(repo) => {
                let common = repo
                    .common_directory()
                    .map_err(factory_core::FactoryError::from)?;
                Some(ProjectLock::acquire_file(
                    common.join("asf-runtime.lock"),
                    root,
                )?)
            }
            Err(error) if canonical.join(".git").exists() => {
                return Err(factory_core::FactoryError::from(error).into())
            }
            Err(_) => None,
        };
        Ok(Self {
            _project: project,
            _git: git,
        })
    }
}
