use std::fs::{self, File};
use std::path::{Path, PathBuf};

use cap_std::fs::Dir;

use crate::{native, FileError, Result};

pub fn validate_path(path: &str, allow_root: bool) -> Result<()> {
    if path.is_empty() && allow_root {
        return Ok(());
    }
    if path.is_empty()
        || path.len() > 2048
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return Err(FileError::Invalid);
    }
    for component in path.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.contains([':', '<', '>', '"', '|', '?', '*'])
            || component.len() > 255
        {
            return Err(FileError::Invalid);
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        }) {
            return Err(FileError::Invalid);
        }
        if protected_component(component) {
            return Err(FileError::Forbidden);
        }
    }
    Ok(())
}

pub(crate) fn protected_component(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value == ".git"
        || value == ".factory"
        || value.starts_with(".asf-editor-")
        || value == ".gitmodules"
}

pub(crate) struct Guard {
    pub dir: Dir,
    pub absolute: PathBuf,
    _leases: Vec<File>,
}

impl Guard {
    pub fn root(path: &Path, identity: &str) -> Result<Self> {
        let handle = native::open_directory(path)?;
        if native::key(&handle)? != identity {
            return Err(FileError::UnknownRoot);
        }
        Ok(Self {
            dir: Dir::from_std_file(handle.try_clone()?),
            absolute: path.to_path_buf(),
            _leases: vec![handle],
        })
    }

    pub fn descend(mut self, relative: &str) -> Result<Self> {
        if relative.is_empty() {
            return Ok(self);
        }
        for component in relative.split('/') {
            let path = self.absolute.join(component);
            let before = fs::symlink_metadata(&path)?;
            if native::is_link(&before) || !before.is_dir() {
                return Err(FileError::Forbidden);
            }
            let child = self.dir.open_dir(component)?;
            let handle = child.try_clone()?.into_std_file();
            let after = fs::symlink_metadata(&path)?;
            if native::is_link(&after)
                || native::key(&handle)? != native::metadata_key(&path, &after)?
            {
                return Err(FileError::Conflict);
            }
            let lease = native::open_directory(&path)?;
            if native::key(&lease)? != native::key(&handle)? {
                return Err(FileError::Conflict);
            }
            self._leases.push(lease);
            self.dir = child;
            self.absolute = path;
        }
        Ok(self)
    }

    pub fn verify(&self) -> Result<()> {
        let current = native::open_directory(&self.absolute)?;
        if native::key(&current)? != native::key(&self.dir.try_clone()?.into_std_file())? {
            return Err(FileError::Conflict);
        }
        Ok(())
    }

    pub fn file(&self, name: &str) -> Result<File> {
        self.open_leaf(name, false)
    }

    pub fn launch_file(&self, name: &str) -> Result<File> {
        self.open_leaf(name, true)
    }

    fn open_leaf(&self, name: &str, hardlinks: bool) -> Result<File> {
        let path = self.absolute.join(name);
        let before = fs::symlink_metadata(&path)?;
        if native::is_link(&before) || !before.is_file() {
            return Err(FileError::Forbidden);
        }
        let file = native::open_file(&self.dir, name, &path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || native::is_link(&metadata)
            || !hardlinks && native::links(&file)? != 1
        {
            return Err(FileError::Forbidden);
        }
        let after = fs::symlink_metadata(&path)?;
        if native::is_link(&after) || native::metadata_key(&path, &after)? != native::key(&file)? {
            return Err(FileError::Conflict);
        }
        self.verify()?;
        Ok(file)
    }
}

pub(crate) fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}
