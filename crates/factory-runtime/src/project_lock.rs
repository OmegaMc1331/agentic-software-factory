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
    pub(crate) fn acquire(
        canonical_root: &Path,
        requested_root: &Path,
    ) -> Result<Self, RuntimeError> {
        let lock_path = canonical_root.join(FACTORY_DIR).join(PROJECT_LOCK_FILE);
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
