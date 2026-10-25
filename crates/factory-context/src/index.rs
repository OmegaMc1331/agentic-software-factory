//! The bounded repository index.
//!
//! A manifest of known files (path, language, size, mtime, content
//! fingerprint, symbols) is persisted under `<factory-dir>/context/index.json`.
//! Refresh is incremental: the walk stats every file, and re-extracts symbols
//! only for files whose content actually changed. For symbol-eligible files
//! that is decided by a SHA-256 fingerprint (metadata alone cannot prove a
//! same-length, same-mtime edit unchanged); other files keep the cheap
//! (size, mtime) check. Removed files are pruned. The index is read-only
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
use crate::symbols::{extract_symbols, language_for, Symbol};

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
    /// SHA-256 of the indexed content, for symbol-eligible files only.
    /// Metadata alone cannot prove content unchanged (a same-length edit can
    /// preserve size and mtime), so cached symbols are keyed by content.
    /// `None` on legacy manifests (re-populated on the next refresh) and for
    /// files that contribute no symbols.
    #[serde(default)]
    pub content_hash: Option<String>,
    pub symbols: Vec<Symbol>,
}

impl IndexedFile {
    pub fn basename(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
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
                let language = language_for(&relative);
                // Only files that contribute extracted symbols carry
                // content-derived cached state; for them, metadata equality
                // is necessary but not sufficient (ASF-R01: a same-length
                // replacement can preserve size and mtime). Everything else
                // keeps the cheap metadata check.
                let symbol_eligible = language.is_some() && size <= MAX_INDEX_FILE_READ_BYTES;
                let cached = self.files.get(&key);
                if let Some(cached) = cached {
                    let metadata_match = cached.size == size && cached.mtime_ms == mtime_ms;
                    if metadata_match && !symbol_eligible {
                        seen.insert(key);
                        continue;
                    }
                    if metadata_match {
                        // Verify by content: hash the current bytes and
                        // compare with the cached fingerprint. A missing
                        // fingerprint (legacy manifest) fails this check on
                        // purpose, so old entries are re-read and backfilled
                        // exactly once.
                        match read_stable(&entry.path()) {
                            Some((bytes, _stat))
                                if cached.content_hash.as_deref()
                                    == Some(sha256_hex(&bytes).as_str()) =>
                            {
                                seen.insert(key);
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
                let (symbols, content_hash, size, mtime_ms) = if symbol_eligible {
                    match read_stable(&entry.path()) {
                        Some((bytes, stat)) => {
                            let hash = sha256_hex(&bytes);
                            let symbols = extract_symbols(
                                language.expect("eligible implies a language"),
                                &String::from_utf8_lossy(&bytes),
                            );
                            (symbols, Some(hash), stat.len(), modified_millis(&stat))
                        }
                        // Unreadable or mid-write: record metadata with no
                        // content-derived state, like a read failure always
                        // did; the next refresh re-reads it.
                        None => (Vec::new(), None, size, mtime_ms),
                    }
                } else {
                    (Vec::new(), None, size, mtime_ms)
                };
                let is_new = cached.is_none();
                self.files.insert(
                    key.clone(),
                    IndexedFile {
                        path: key.clone(),
                        language: language.map(str::to_string),
                        size,
                        mtime_ms,
                        content_hash,
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

/// Reads a file's bytes with a bounded stability recheck: if the metadata
/// moved between stat and read (another process writing concurrently), one
/// retry uses the newer bytes. The returned metadata describes exactly the
/// returned bytes, so the fingerprint, symbols, and recorded size/mtime stay
/// consistent. Two unstable attempts yield `None`.
fn read_stable(path: &Path) -> Option<(Vec<u8>, std::fs::Metadata)> {
    for _ in 0..2 {
        let before = std::fs::metadata(path).ok()?;
        let bytes = std::fs::read(path).ok()?;
        let after = std::fs::metadata(path).ok()?;
        if before.len() == after.len() && modified_millis(&before) == modified_millis(&after) {
            return Some((bytes, after));
        }
    }
    None
}

/// SHA-256 as lowercase hex. Cache invalidation identity, not a signature.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
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
    let text =
        serde_json::to_string_pretty(manifest).map_err(|source| ContextError::IndexWrite {
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
        std::fs::write(index_path, std::fs::read(&tmp).unwrap_or_default()).map_err(|source| {
            ContextError::Io {
                path: index_path.to_path_buf(),
                source,
            }
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
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "renamed");
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

    /// ASF-R01 reproduction: a same-length content replacement with the mtime
    /// explicitly restored must not be treated as unchanged. `alpha` and
    /// `bravo` have identical byte length, and `filetime` restores the exact
    /// original mtime, so (size, mtime) alone cannot distinguish the files.
    #[test]
    fn same_size_same_mtime_replacement_refreshes_symbols() {
        let (_dir, root) = repo();
        let file = root.join("src").join("main.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "alpha");

        let metadata = std::fs::metadata(&file).unwrap();
        let original_mtime = filetime::FileTime::from_last_modification_time(&metadata);
        std::fs::write(&file, "pub fn bravo() {}\n").unwrap();
        filetime::set_file_mtime(&file, original_mtime).unwrap();

        let after = std::fs::metadata(&file).unwrap();
        assert_eq!(
            after.len(),
            metadata.len(),
            "fixture sanity: replacement has identical byte length"
        );
        assert_eq!(
            filetime::FileTime::from_last_modification_time(&after),
            original_mtime,
            "fixture sanity: modification time was restored exactly"
        );

        let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(stats.updated, 1, "the content change must be detected");
        let symbols = &index.files["src/main.rs"].symbols;
        assert!(
            symbols.iter().any(|symbol| symbol.name == "bravo"),
            "the current symbol must be discovered: {symbols:?}"
        );
        assert!(
            !symbols.iter().any(|symbol| symbol.name == "alpha"),
            "the removed symbol must not survive: {symbols:?}"
        );
    }

    /// ASF-R01: a same-length replacement whose mtime DID move is detected by
    /// the metadata check alone.
    #[test]
    fn same_size_replacement_with_changed_mtime_updates() {
        let (_dir, root) = repo();
        let file = root.join("src").join("main.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        // Different content, different length is unnecessary: metadata moved.
        std::fs::write(&file, "pub fn bravo() {}\n").unwrap();
        let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(stats.updated, 1);
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "bravo");
    }

    #[test]
    fn renamed_files_are_removed_and_re_added() {
        let (_dir, root) = repo();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        std::fs::rename(
            root.join("src").join("main.rs"),
            root.join("src").join("app.rs"),
        )
        .unwrap();
        let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(stats.removed, 1);
        assert_eq!(stats.added, 1);
        assert!(!index.files.contains_key("src/main.rs"));
        assert_eq!(index.files["src/app.rs"].symbols[0].name, "main");
    }

    #[test]
    fn multiple_same_mtime_changes_are_all_detected() {
        let (_dir, root) = repo();
        std::fs::write(root.join("src").join("extra.rs"), "pub fn alpha() {}\n").unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();

        let main = root.join("src").join("main.rs");
        let extra = root.join("src").join("extra.rs");
        for (path, replacement) in [
            (main, "pub fn bravo() {}\n"),
            (extra, "pub fn delta() {}\n"),
        ] {
            let metadata = std::fs::metadata(&path).unwrap();
            let mtime = filetime::FileTime::from_last_modification_time(&metadata);
            std::fs::write(&path, replacement).unwrap();
            filetime::set_file_mtime(path, mtime).unwrap();
        }
        let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(stats.updated, 2, "both hidden edits must be detected");
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "bravo");
        assert_eq!(index.files["src/extra.rs"].symbols[0].name, "delta");
    }

    /// A manifest written before fingerprints existed (no `content_hash`
    /// field) must still deserialize, must not be trusted as verified, gets
    /// backfilled exactly once, and is stable afterwards.
    #[test]
    fn legacy_manifest_without_fingerprints_backfills_once() {
        let (_dir, root) = repo();
        let legacy_manifest = serde_json::json!({
            "root": root.to_string_lossy(),
            "created_at_secs": 1,
            "updated_at_secs": 1,
            "oversize": false,
            "files": [{
                "path": "src/main.rs",
                "language": "rust",
                "size": 17,
                "mtime_ms": 1,
                "symbols": [{
                    "name": "main",
                    "kind": "function",
                    "line": 1
                }]
            }]
        });
        let manifest: IndexManifest =
            serde_json::from_value(legacy_manifest).expect("legacy manifest deserializes");
        assert_eq!(manifest.files[0].content_hash, None);
        let mut index = ContextIndex::from_manifest(&manifest);

        // The legacy entry claims an impossible mtime; the fingerprint
        // mismatch (None vs Some) forces a re-read even if it matched.
        let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert!(stats.updated >= 1, "legacy entries are re-verified");
        assert_eq!(
            index.files["src/main.rs"].content_hash.as_deref(),
            Some(sha256_hex(b"pub fn main() {}\n").as_str())
        );
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "main");

        // Backfilled fingerprint now verifies: further refreshes are stable.
        let again = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(again.updated, 0, "backfilled entries settle");

        // ...and still catch a hidden edit afterwards.
        let file = root.join("src").join("main.rs");
        let metadata = std::fs::metadata(&file).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(&metadata);
        std::fs::write(&file, "pub fn bravo() {}\n").unwrap();
        filetime::set_file_mtime(&file, mtime).unwrap();
        let third = index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(third.updated, 1);
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "bravo");
    }

    /// Symbol-eligible files at the read cap are fingerprinted; files beyond
    /// it contribute no content-derived state and keep the metadata check.
    #[test]
    fn oversized_files_carry_no_fingerprint_but_stay_indexed() {
        let (_dir, root) = repo();
        let oversized = root.join("big.rs");
        let fill = "x".repeat(MAX_INDEX_FILE_READ_BYTES as usize + 1);
        std::fs::write(&oversized, fill).unwrap();
        let at_cap = root.join("cap.rs");
        std::fs::write(&at_cap, "y".repeat(MAX_INDEX_FILE_READ_BYTES as usize)).unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert!(index.files.contains_key("big.rs"));
        assert_eq!(index.files["big.rs"].content_hash, None);
        assert_eq!(index.files["big.rs"].symbols.len(), 0);
        assert!(index.files["cap.rs"].content_hash.is_some());
        assert_eq!(index.files["cap.rs"].symbols.len(), 0);
    }

    #[test]
    fn unicode_content_is_fingerprinted_and_reextracted() {
        let (_dir, root) = repo();
        let file = root.join("src").join("main.rs");
        // Identifiers are ASCII by design; unicode appears in comments and
        // must still participate in the content fingerprint.
        std::fs::write(&file, "pub fn alpha() {} // éclaire\n").unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "alpha");
        let hash_before = index.files["src/main.rs"].content_hash.clone();
        // Same byte length (both comments are 8 bytes), different content,
        // same mtime.
        let metadata = std::fs::metadata(&file).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(&metadata);
        std::fs::write(&file, "pub fn bravo() {} // éclaîr\n").unwrap();
        filetime::set_file_mtime(&file, mtime).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().len(),
            metadata.len(),
            "fixture sanity: same byte length"
        );
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "bravo");
        assert_ne!(
            index.files["src/main.rs"].content_hash, hash_before,
            "the hidden unicode edit must change the fingerprint"
        );
    }

    /// Symbol locations must refresh with content, not just names.
    #[test]
    fn changed_symbol_locations_refresh() {
        let (_dir, root) = repo();
        let file = root.join("src").join("main.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(index.files["src/main.rs"].symbols[0].line, 1);
        let metadata = std::fs::metadata(&file).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(&metadata);
        // Same length (18 bytes); the declaration moves to line 3.
        std::fs::write(&file, "//a\n//b\nfn x() {}\n").unwrap();
        filetime::set_file_mtime(&file, mtime).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().len(),
            "pub fn alpha() {}\n".len() as u64,
            "fixture sanity: same byte length"
        );
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(index.files["src/main.rs"].symbols[0].name, "x");
        assert_eq!(index.files["src/main.rs"].symbols[0].line, 3);
    }

    /// Unchanged content must not report false changes — the fingerprint
    /// settles after any backfill and repeated refreshes stay clean.
    #[test]
    fn repeated_refreshes_are_stable_and_clean() {
        let (_dir, root) = repo();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        for _ in 0..3 {
            let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
            assert_eq!(stats.added, 0);
            assert_eq!(stats.updated, 0);
            assert_eq!(stats.removed, 0);
        }
        assert!(!index
            .refresh(&root, &IgnoreRules::empty())
            .unwrap()
            .overall_changed());
    }

    /// The manifest persists and reloads fingerprints faithfully.
    #[test]
    fn fingerprints_survive_manifest_round_trip() {
        let (_dir, root) = repo();
        let mut index = ContextIndex {
            root: root.clone(),
            files: BTreeMap::new(),
            oversize: false,
        };
        index.refresh(&root, &IgnoreRules::empty()).unwrap();
        let manifest = index.to_manifest(11, 22);
        assert_eq!(manifest.created_at_secs, 11);
        let main_entry = manifest
            .files
            .iter()
            .find(|file| file.path == "src/main.rs")
            .expect("main.rs in manifest");
        assert!(main_entry.content_hash.is_some());
        let text = serde_json::to_string(&manifest).unwrap();
        let reloaded: IndexManifest = serde_json::from_str(&text).unwrap();
        let mut restored = ContextIndex::from_manifest(&reloaded);
        assert_eq!(restored.files["src/main.rs"], index.files["src/main.rs"]);
        // And the restored index verifies cleanly against unchanged files.
        let stats = restored.refresh(&root, &IgnoreRules::empty()).unwrap();
        assert_eq!(stats.updated, 0);
    }

    /// ASF-R01 diagnostic: measures refresh cost on representative repos.
    /// Ignored in CI (wall-clock); run explicitly with
    /// `cargo test -p factory-context -- --ignored --nocapture`.
    #[test]
    #[ignore = "diagnostic timing measurement"]
    fn refresh_performance_diagnostic() {
        for (file_count, per_file) in [(100, 256_usize), (1000, 256)] {
            let dir = tempfile::TempDir::new().unwrap();
            let root = dir.path().to_path_buf();
            for i in 0..file_count {
                let name = format!("mod{i:04}.rs");
                std::fs::write(
                    root.join(name),
                    format!("pub fn f{i}() {{}}\n{}", "x".repeat(per_file - 14)),
                )
                .unwrap();
            }
            let mut index = ContextIndex {
                root: root.clone(),
                files: BTreeMap::new(),
                oversize: false,
            };
            let build_start = std::time::Instant::now();
            index.refresh(&root, &IgnoreRules::empty()).unwrap();
            let build = build_start.elapsed();
            let verify_start = std::time::Instant::now();
            let stats = index.refresh(&root, &IgnoreRules::empty()).unwrap();
            let verify = verify_start.elapsed();
            assert_eq!(stats.updated, 0);
            println!(
                "DIAG {file_count} files: initial refresh {build:?}, unchanged re-refresh {verify:?} \
                 (every eligible file is re-read and hashed to verify its fingerprint)"
            );
        }
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
