//! The bounded repository index.
//!
//! A manifest of known files (path, language, size, mtime, symbols) is
//! persisted under `<factory-dir>/context/index.json`. Refresh is incremental:
//! the walk stats every file, re-extracts symbols only for files whose
//! (size, mtime) changed, and prunes removed files. The index is read-only
//! with respect to the repository itself.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Monotonic counter so concurrent `save_manifest` calls never share a tmp file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

use serde::{Deserialize, Serialize};

use crate::config::MAX_INDEX_ENTRIES;
use crate::error::{ContextError, Result};
use crate::ignore::IgnoreRules;
use crate::symbols::{language_for, extract_symbols, Symbol};

/// Bounded read cap for a single file during indexing. Larger files are still
/// indexed (metadata + search) but contribute no symbols, keeping extraction
/// cheap.
pub const MAX_INDEX_FILE_READ_BYTES: u64 = 1 << 20;

/// A single indexed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFile {
    /// Forward-slash path relative to the index root.
    pub path: String,
    pub language: Option<String>,
    pub size: u64,
    /// Modification time in Unix epoch milliseconds.
    pub mtime_ms: u128,
    pub symbols: Vec<Symbol>,
}

impl IndexedFile {
    pub fn basename(&self) -> &str {
        self.path
            .rsplit('/')
            .next()
            .unwrap_or(&self.path)
    }
}

/// Persisted state of one index root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexManifest {
    pub root: String,
    pub created_at_secs: u64,
    pub updated_at_secs: u64,
    /// Whether the repository exceeded `MAX_INDEX_ENTRIES` during the last
    /// walk (resolution still works over whatever was indexed).
    pub oversize: bool,
    pub files: Vec<IndexedFile>,
}

/// In-memory index over one root.
#[derive(Debug, Clone)]
pub struct ContextIndex {
    pub root: PathBuf,
    pub files: BTreeMap<String, IndexedFile>,
    pub oversize: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RefreshStats {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub total: usize,
    pub oversize: bool,
}

impl ContextIndex {
    pub fn from_manifest(manifest: &IndexManifest) -> Self {
        Self {
            root: PathBuf::from(&manifest.root),
            files: manifest
                .files
                .iter()
                .map(|file| (file.path.clone(), file.clone()))
                .collect(),
            oversize: manifest.oversize,
        }
    }

    pub fn to_manifest(&self, created_at_secs: u64, updated_at_secs: u64) -> IndexManifest {
        let mut files: Vec<IndexedFile> = self.files.values().cloned().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        IndexManifest {
            root: self.root.to_string_lossy().into_owned(),
            created_at_secs,
            updated_at_secs,
            oversize: self.oversize,
            files,
        }
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn symbol_count(&self) -> usize {
        self.files
            .values()
            .map(|file| file.symbols.len())
            .sum::<usize>()
    }

    /// Incrementally refreshes the index for `root`.
    ///
    /// The walk stats every file and stops early beyond `MAX_INDEX_ENTRIES`;
    /// files whose (size, mtime) match the manifest keep their cached symbols.
    /// The caller persists afterwards when `stats.overall_changed()`.
    pub fn refresh(&mut self, root: &Path, ignore: &IgnoreRules) -> Result<RefreshStats> {
        let root = normalize(root);
        self.root = root.clone();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stats = RefreshStats::default();
        let mut oversize = false;
        let mut stack: Vec<PathBuf> = Vec::new();
        stack.push(root.clone());
        while let Some(dir) = stack.pop() {
            if seen.len() >= MAX_INDEX_ENTRIES {
                oversize = true;
                break;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries {
                if seen.len() >= MAX_INDEX_ENTRIES {
                    oversize = true;
                    break;
                }
                let Ok(entry) = entry else { continue };
                let relative = match entry.path().strip_prefix(&root) {
                    Ok(relative) => relative.to_path_buf(),
                    Err(_) => continue,
                };
                let key = forward_slash(&relative);
                if key.is_empty() {
                    continue;
                }
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(_) => continue,
                };
                if file_type.is_dir() {
                    if ignore.is_ignored(relative.as_path(), true) {
                        continue;
                    }
                    stack.push(entry.path());
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                if ignore.is_ignored(relative.as_path(), false) {
                    continue;
                }
                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                let mtime_ms = modified_millis(&metadata);
                let size = metadata.len();
                let cached = self.files.get(&key);
                if let Some(cached) = cached {
                    if cached.size == size && cached.mtime_ms == mtime_ms {
                        seen.insert(key);
                        continue;
                    }
                }
                let language = language_for(&relative);
                let symbols = match language {
                    Some(language) if size <= MAX_INDEX_FILE_READ_BYTES => match std::fs::read(entry.path()) {
                        Ok(bytes) => extract_symbols(language, &String::from_utf8_lossy(&bytes)),
                        Err(_) => Vec::new(),
                    },
                    _ => Vec::new(),
                };
                let is_new = cached.is_none();
                self.files.insert(
                    key.clone(),
                    IndexedFile {
                        path: key.clone(),
                        language: language.map(str::to_string),
                        size,
                        mtime_ms,
                        symbols,
                    },
                );
                seen.insert(key);
                if is_new {
                    stats.added += 1;
                } else {
                    stats.updated += 1;
                }
            }
        }
        self.oversize = oversize;
        stats.oversize = oversize;
        let removed: Vec<String> = self
            .files
            .keys()
            .filter(|key| !seen.contains(*key))
            .cloned()
            .collect();
        for key in removed {
            self.files.remove(&key);
            stats.removed += 1;
        }
        stats.total = self.files.len();
        Ok(stats)
    }
}

impl RefreshStats {
    pub fn overall_changed(&self) -> bool {
        self.added != 0 || self.updated != 0 || self.removed != 0
    }
}

/// Reads the persisted manifest for `index_path`, returning `None` when absent
/// or unreadable (a fresh walk always rebuilds it).
pub fn load_manifest(index_path: &Path) -> Result<Option<IndexManifest>> {
    match std::fs::read(index_path) {
        Ok(bytes) => match serde_json::from_slice::<IndexManifest>(&bytes) {
            Ok(manifest) => Ok(Some(manifest)),
            Err(source) => Err(ContextError::IndexRead {
                path: index_path.to_path_buf(),
                source,
            }),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ContextError::Io {
            path: index_path.to_path_buf(),
            source,
        }),
    }
}

/// Persists the manifest. Concurrent writers (parallel missions inside one
/// process) each get a unique tmp file, and rename-over-existing is not
/// reliable on Windows, so a failed rename falls back to a direct write
/// (last writer wins; snapshots are self-contained).
pub fn save_manifest(index_path: &Path, manifest: &IndexManifest) -> Result<()> {
    let text = serde_json::to_string_pretty(manifest).map_err(|source| ContextError::IndexWrite {
        path: index_path.to_path_buf(),
        source,
    })?;
    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ContextError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = index_path.with_extension(format!("json.tmp{}-{seq}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|source| ContextError::Io {
        path: tmp.clone(),
        source,
    })?;
    if std::fs::rename(&tmp, index_path).is_err() {
        std::fs::write(index_path, std::fs::read(&tmp).unwrap_or_default())
            .map_err(|source| ContextError::Io {
                path: index_path.to_path_buf(),
                source,
            })?;
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

/// A human-readable state snapshot for the CLI and dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSummary {
    pub root: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub oversize: bool,
    pub updated_at_secs: Option<u64>,
    pub engine_enabled: bool,
    pub index_path: String,
}

pub fn forward_slash(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    while text.starts_with("./") {
        text = text.trim_start_matches("./").to_string();
    }
    text
}

pub fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn modified_millis(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("main.rs"), "pub fn main() {}\n").unwrap();
        std::fs::write(root.join("README.md"), "# repo\n").unwrap();
        (dir, root)
    }

    #[test]
    fn walks_and_indexes_files_with_symbols() {
        let (_dir, root) = repo();
        std::fs::create_dir_all(root.join("src").join("nested")).unwrap();
        std::fs::write(
            root.join("src").join("nested").join("lib.rs"),
            "pub struct Token;\n",
        )
        .unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        let stats = index
            .refresh(&root, &IgnoreRules::empty())
            .expect("refresh works");
        assert_eq!(stats.added, 3);
        assert_eq!(index.file_count(), 3);
        let lib = index.files.get("src/nested/lib.rs").expect("indexed");
        assert_eq!(lib.language.as_deref(), Some("rust"));
        assert_eq!(lib.symbols[0].name, "Token");
    }

    #[test]
    fn refresh_is_incremental_on_mtime() {
        let (_dir, root) = repo();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        let first_stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(first_stats.added, 0);
        assert_eq!(first_stats.updated, 0);
        assert_eq!(first_stats.total, 2);

        std::fs::write(root.join("src").join("main.rs"), "pub fn renamed() {}\n").unwrap();
        let second_stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(second_stats.updated, 1);
        assert_eq!(second_stats.added, 0);
        assert_eq!(
            index.files["src/main.rs"].symbols[0].name,
            "renamed"
        );
    }

    #[test]
    fn removed_files_are_pruned() {
        let (_dir, root) = repo();
        std::fs::write(root.join("extra.rs"), "struct Extra;\n").unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(index.file_count(), 3);
        std::fs::remove_file(root.join("extra.rs")).unwrap();
        let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(stats.removed, 1);
        assert_eq!(index.file_count(), 2);
    }

    #[test]
    fn manifest_round_trips_and_skips_factory_dir() {
        let (_dir, root) = repo();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index
            .refresh(&root, &IgnoreRules::empty())
            .expect("refresh");
        let manifest = index.to_manifest(1, 1);
        let restored = ContextIndex::from_manifest(&manifest);
        assert_eq!(restored.file_count(), index.file_count());

        std::fs::create_dir_all(root.join(".factory")).unwrap();
        std::fs::write(root.join(".factory").join("x.txt"), "i").unwrap();
        let mut index2 = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index2.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert!(!index2.files.contains_key(".factory/x.txt"));
    }
}