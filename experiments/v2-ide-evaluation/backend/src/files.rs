use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::Error;

pub const FILE_LIMIT: u64 = 1_048_576;

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn linked(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}

pub fn path(root: &Path, relative: &str) -> Result<PathBuf, Error> {
    if relative.is_empty()
        || relative.contains([':', '\0', '\\'])
        || relative.chars().any(char::is_control)
        || relative.split('/').any(|p| {
            matches!(
                p.to_ascii_lowercase().as_str(),
                "." | ".." | ".git" | ".factory"
            ) || p.to_ascii_lowercase().starts_with(".env")
        })
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::denied("path outside the experiment scope"));
    }
    let mut candidate = root.to_path_buf();
    for part in relative.split('/') {
        candidate.push(part);
        let metadata = std::fs::symlink_metadata(&candidate)?;
        if linked(&metadata) {
            return Err(Error::denied("links and reparse points are not eligible"));
        }
    }
    let resolved = std::fs::canonicalize(candidate)?;
    if !resolved.starts_with(root) {
        return Err(Error::denied("path escaped the project"));
    }
    Ok(resolved)
}

pub fn read(root: &Path, relative: &str) -> Result<Value, Error> {
    let path = path(root, relative)?;
    let metadata = std::fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT {
        return Err(Error::bad("non-text or oversized file"));
    }
    let bytes = std::fs::read(&path)?;
    let revision = hash(&bytes);
    let content =
        String::from_utf8(bytes).map_err(|_| Error::bad("binary or invalid UTF-8 file"))?;
    Ok(
        json!({"path": relative, "content": content, "revision": revision,
        "uri": url::Url::from_file_path(path).map_err(|_| Error::bad("invalid file URI"))?.as_str()}),
    )
}

pub fn save(root: &Path, relative: &str, revision: &str, text: &str) -> Result<Value, Error> {
    if text.len() as u64 > FILE_LIMIT {
        return Err(Error::bad("document exceeds the prototype size limit"));
    }
    let path = path(root, relative)?;
    if hash(&std::fs::read(&path)?) != revision {
        return Err(Error(
            axum::http::StatusCode::CONFLICT,
            "external modification: buffer retained; reload or merge explicitly".into(),
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(
        path.parent().ok_or_else(|| Error::bad("missing parent"))?,
    )?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| Error::from(error.error))?;
    read(root, relative)
}

pub fn tree(root: &Path) -> Result<Value, Error> {
    fn walk(root: &Path, dir: &Path, result: &mut Vec<Value>, depth: usize) -> Result<(), Error> {
        if depth > 32 {
            return Err(Error::bad("tree exceeds depth limit"));
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(name.to_ascii_lowercase().as_str(), ".git" | ".factory")
                || name.to_ascii_lowercase().starts_with(".env")
            {
                continue;
            }
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if linked(&metadata) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| Error::bad("invalid tree path"))?
                .to_string_lossy()
                .replace('\\', "/");
            if result.len() >= 20_000 {
                return Err(Error::bad("tree exceeds entry limit"));
            }
            result.push(
                json!({"path":relative,"directory":metadata.is_dir(),"bytes":metadata.len()}),
            );
            if metadata.is_dir() {
                walk(root, &entry.path(), result, depth + 1)?;
            }
        }
        Ok(())
    }
    let mut result = Vec::new();
    walk(root, root, &mut result, 0)?;
    result.sort_by_key(|entry| entry["path"].as_str().unwrap_or_default().to_string());
    Ok(json!(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_secrets_and_conflicting_external_writes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::write(root.join("file.ts"), "old").unwrap();
        std::fs::write(root.join(".env"), "private").unwrap();
        for candidate in [
            "../file.ts",
            "./file.ts",
            "/file.ts",
            "file.ts:stream",
            ".env",
            ".factory/db",
            "a\\b",
        ] {
            assert!(path(&root, candidate).is_err(), "{candidate}");
        }
        let document = read(&root, "file.ts").unwrap();
        std::fs::write(root.join("file.ts"), "external").unwrap();
        assert_eq!(
            save(
                &root,
                "file.ts",
                document["revision"].as_str().unwrap(),
                "buffer"
            )
            .unwrap_err()
            .0,
            axum::http::StatusCode::CONFLICT
        );
        assert_eq!(
            std::fs::read_to_string(root.join("file.ts")).unwrap(),
            "external"
        );
    }
}
