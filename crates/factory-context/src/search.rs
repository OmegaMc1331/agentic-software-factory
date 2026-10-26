//! Repository search over the index (fast path) and file contents (ripgrep
//! with a pure-Rust fallback).
//!
//! Security and resource invariants shared by both content-search paths
//! (ASF-F01/ASF-F04):
//! - symbolic links are never followed and never read: traversal decides on
//!   `DirEntry::file_type()` (non-following), and every path returned by an
//!   external tool (ripgrep) is revalidated against the repository root, the
//!   non-symlink rule, the ignore policy, and the read-size limit before any
//!   byte is read;
//! - file reads are bounded by `MAX_INDEX_FILE_READ_BYTES` via handle-based
//!   metadata plus a `take()`d reader, so memory stays bounded even when a
//!   file grows concurrently;
//! - directory traversal visits at most `MAX_INDEX_ENTRIES` entries and
//!   intermediate match storage at most [`MAX_MATCHED_PATHS`] paths;
//! - the ripgrep subprocess runs under the `factory-proc` supervisor with a
//!   deadline and bounded stdout/stderr capture, NUL-separated output, and
//!   explicit exit-code semantics (1 = no matches, not a failure).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use factory_proc::{Bounds, ProcCommand};

use crate::error::Result;
use crate::ignore::IgnoreRules;
use crate::index::{forward_slash, normalize, ContextIndex, MAX_INDEX_FILE_READ_BYTES};
use crate::rank::query_tokens;
use crate::symbols::Symbol;

/// Upper bound on intermediate matched-path storage before the final
/// `limit` truncation, so a pathological repository cannot grow the
/// candidate list without bound.
pub const MAX_MATCHED_PATHS: usize = 1_000;

/// Deadline and capture bounds for one ripgrep invocation. The deadline is
/// overridable (`FACTORY_RG_TIMEOUT_SECS`) for tests and slow machines.
const RG_TIMEOUT_SECS: u64 = 30;

fn rg_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(
        std::env::var("FACTORY_RG_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(RG_TIMEOUT_SECS),
    )
}
const RG_STDOUT_LIMIT: usize = 1024 * 1024;
const RG_STDERR_LIMIT: usize = 64 * 1024;

/// What produced a search hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// The file path itself matched a query term.
    Path,
    /// A declared symbol (function/struct/...) matched.
    Symbol,
    /// The file contents matched.
    Content,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub match_kind: MatchKind,
    /// 1-based first matching line, when known.
    pub line: Option<usize>,
    /// A bounded snippet of the matching line, when read.
    pub snippet: Option<String>,
    /// Symbols that matched the query (Symbol hits only).
    pub symbols: Vec<Symbol>,
}

/// Fast path: matches against indexed paths and symbol names. Deterministic
/// (path/symbol priority, then path ascending) and bounded by `limit`.
pub fn search_index(index: &ContextIndex, query: &str, limit: usize) -> Vec<SearchHit> {
    let terms = query_tokens(query, 1);
    if terms.is_empty() {
        return Vec::new();
    }
    let mut path_hits: Vec<SearchHit> = Vec::new();
    let mut symbol_hits: Vec<SearchHit> = Vec::new();
    for (path, file) in &index.files {
        let lower = path.to_ascii_lowercase();
        if terms
            .iter()
            .any(|term| term.len() >= 4 && (lower.contains(term.as_str()) || term.contains(&lower)))
        {
            path_hits.push(SearchHit {
                path: path.clone(),
                match_kind: MatchKind::Path,
                line: None,
                snippet: None,
                symbols: Vec::new(),
            });
        }
        let matched_symbols: Vec<Symbol> = file
            .symbols
            .iter()
            .filter(|symbol| {
                let name = symbol.name.to_ascii_lowercase();
                terms.iter().any(|term| {
                    term.len() >= 4
                        && (name.contains(term.as_str()) || term.contains(name.as_str()))
                })
            })
            .cloned()
            .collect();
        if !matched_symbols.is_empty() {
            symbol_hits.push(SearchHit {
                path: path.clone(),
                match_kind: MatchKind::Symbol,
                line: matched_symbols.first().map(|symbol| symbol.line),
                snippet: None,
                symbols: matched_symbols,
            });
        }
    }
    path_hits.sort_by(|a, b| a.path.cmp(&b.path));
    symbol_hits.sort_by(|a, b| a.path.cmp(&b.path));
    path_hits
        .into_iter()
        .chain(symbol_hits)
        .take(limit)
        .collect()
}

/// Which ripgrep binary to run: `FACTORY_RG_BIN` overrides `rg` from PATH
/// (deterministic fake-rg tests).
fn rg_program() -> OsString {
    std::env::var_os("FACTORY_RG_BIN")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "rg".into())
}

/// Whether the real `rg` is available. Cached once per process; availability
/// of an injected override binary is probed per run (tests swap fakes).
fn ripgrep_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    if std::env::var_os("FACTORY_RG_BIN").is_some_and(|value| !value.is_empty()) {
        return probe_rg(&rg_program());
    }
    *AVAILABLE.get_or_init(|| probe_rg(OsStr::new("rg")))
}

fn probe_rg(program: &OsStr) -> bool {
    ProcCommand::new(program)
        .arg("--version")
        .run(Bounds::new(rg_timeout(), 64 * 1024, 64 * 1024))
        .map(|output| output.success())
        .unwrap_or(false)
}

fn content_glob_flags() -> Vec<String> {
    let mut flags = Vec::new();
    const DIRS: &[&str] = &[
        ".factory",
        "node_modules",
        "target",
        "dist",
        "build",
        "vendor",
        "__pycache__",
        ".venv",
        "venv",
        "coverage",
        ".pytest_cache",
        ".ruff_cache",
        ".next",
        ".git",
    ];
    for dir in DIRS {
        flags.push(format!("-g!{dir}/**"));
    }
    flags.push("-g!*.png".to_string());
    flags.push("-g!*.jpg".to_string());
    flags.push("-g!*.jpeg".to_string());
    flags.push("-g!*.gif".to_string());
    flags.push("-g!*.webp".to_string());
    flags.push("-g!*.ico".to_string());
    flags.push("-g!*.woff".to_string());
    flags.push("-g!*.woff2".to_string());
    flags.push("-g!*.ttf".to_string());
    flags.push("-g!*.pdf".to_string());
    flags.push("-g!*.zip".to_string());
    flags.push("-g!*.gz".to_string());
    flags.push("-g!*.wasm".to_string());
    flags.push("-g!*.exe".to_string());
    flags.push("-g!*.so".to_string());
    flags.push("-g!*.dll".to_string());
    flags.push("-g!*.lock".to_string());
    flags
}

/// Content search over `root`: ripgrep when available (bounded, supervised,
/// NUL-separated), pure-Rust bounded fallback otherwise and whenever the
/// tool fails, times out, or produces truncated/untrusted output. Returns
/// paths (not full diffs) plus a first-match line/snippet resolved by a
/// bounded read of each hit.
pub fn search_content(
    root: &Path,
    ignore: &IgnoreRules,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let terms = query_tokens(query, 1);
    if terms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let root = normalize(root);
    let matched_paths: Vec<PathBuf> = match ripgrep_paths(&root, ignore, query) {
        // `None` means the tool path is unusable; the fallback stays bounded.
        Some(paths) => paths,
        None => fallback_paths(&root, ignore, query),
    };

    let mut hits: Vec<SearchHit> = Vec::new();
    for path in matched_paths.into_iter().take(limit) {
        let relative = path.strip_prefix(&root).ok();
        let key = relative
            .map(forward_slash)
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let (line, snippet) = first_match_line(&path, query);
        hits.push(SearchHit {
            path: key,
            match_kind: MatchKind::Content,
            line,
            snippet,
            symbols: Vec::new(),
        });
    }
    hits.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(hits)
}

/// Ripgrep-based candidates, or `None` when the tool path must not be
/// trusted (missing, failed, timed out, truncated, or unusable output) and
/// the bounded fallback should run instead.
fn ripgrep_paths(root: &Path, ignore: &IgnoreRules, query: &str) -> Option<Vec<PathBuf>> {
    if !ripgrep_available() {
        return None;
    }
    let command = ProcCommand::new(rg_program())
        .arg("--files-with-matches")
        .arg("--hidden")
        // Factory's IgnoreRules is the single exclusion policy: rg's own
        // ignore handling (which would also consult this machine's global
        // excludes) is disabled so results match the fallback scanner and
        // the index exactly.
        .arg("--no-ignore")
        .arg("-i")
        .arg("-m")
        .arg("1")
        // NUL-separated paths: filenames may contain spaces, quotes, or
        // (on Unix) newlines.
        .arg("--null")
        .args(content_glob_flags())
        .arg(query)
        .arg(root);
    let output = command.run(Bounds::new(rg_timeout(), RG_STDOUT_LIMIT, RG_STDERR_LIMIT));
    let output = match output {
        Ok(output) => output,
        // A deadline miss or spawn failure says nothing about matches; the
        // bounded fallback decides.
        Err(_) => return None,
    };
    // Truncated path output must never be treated as a complete result set.
    if output.stdout_truncated {
        return None;
    }
    match output.status.code() {
        // 0: matches found — validate every returned path.
        Some(0) => {}
        // 1: ripgrep's documented "no matches" — not a tool failure. Do NOT
        // fall back: the answer is authoritative and empty.
        Some(1) => return Some(Vec::new()),
        // 2 or anything else: real tool failure — fall back.
        _ => return None,
    }
    let mut paths = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let os_raw: OsString = os_from_bytes(raw);
        let path = PathBuf::from(os_raw);
        if accept_candidate(root, ignore, &path) {
            paths.push(path);
            if paths.len() >= MAX_MATCHED_PATHS {
                break;
            }
        }
    }
    Some(paths)
}

/// Converts raw ripgrep bytes into a path losslessly on Unix (arbitrary
/// bytes are legal path content) and via UTF-8 on Windows.
#[cfg(unix)]
fn os_from_bytes(raw: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(raw.to_vec())
}

#[cfg(windows)]
fn os_from_bytes(raw: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(raw).into_owned())
}

/// Boundary revalidation for every externally produced candidate (ASF-F01):
/// a path is accepted only if it lies inside the canonical root, is not a
/// symbolic link, honors the ignore policy, and is small enough to read.
/// String-prefix containment alone proves nothing — `strip_prefix` is the
/// first check, not the only one.
fn accept_candidate(root: &Path, ignore: &IgnoreRules, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.as_os_str().is_empty() {
        return false;
    }
    // Non-following metadata: the entry itself must not be a link, whatever
    // it points at or where it resides.
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() => {}
        _ => return false,
    }
    if ignore.is_ignored(relative, false) {
        return false;
    }
    // Pre-read size check from the same non-following metadata.
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.len() <= MAX_INDEX_FILE_READ_BYTES => {}
        _ => return false,
    }
    true
}

/// Pure-Rust bounded fallback: a directory walk that decides on
/// `DirEntry::file_type()` (never follows or reads symbolic links, never
/// traverses linked directories, so link cycles cannot loop), skips ignored
/// and built-in paths, bounds visited entries, and reads every file through
/// a size-checked bounded reader.
fn fallback_paths(root: &Path, ignore: &IgnoreRules, query: &str) -> Vec<PathBuf> {
    let query = query.to_ascii_lowercase();
    let mut found: Vec<PathBuf> = Vec::new();
    let mut visited: usize = 0;
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            // Traversal budget mirrors the index walk so a pathological
            // tree cannot make search unbounded; the result is then a
            // bounded partial scan, not an exhaustive one.
            if visited > crate::config::MAX_INDEX_ENTRIES {
                return found;
            }
            // ASF-F01: decide on the entry's own type — a symlink is never
            // a directory to traverse and never a file to read.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if file_type.is_dir() {
                if !ignore.is_ignored(relative, true) {
                    stack.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if ignore.is_ignored(relative, false) {
                continue;
            }
            if file_matches_bounded(&path, &query) {
                found.push(path);
                if found.len() >= MAX_MATCHED_PATHS {
                    return found;
                }
            }
        }
    }
    found
}

/// Reads at most `MAX_INDEX_FILE_READ_BYTES + 1` bytes of `path` through an
/// open handle and reports whether the (lowercased, lossy) content contains
/// `needle`. Memory stays bounded even when the file grows concurrently:
/// the handle's own metadata gates the allocation, and `take()` caps the
/// read; a file that crosses the limit mid-read is rejected, not parsed.
fn file_matches_bounded(path: &Path, needle: &str) -> bool {
    bounded_bytes(path).is_some_and(|bytes| {
        String::from_utf8_lossy(&bytes)
            .to_ascii_lowercase()
            .contains(needle)
    })
}

/// Opens `path`, checks the handle's own size, and reads up to one byte past
/// the read limit. `Some(bytes)` longer than the limit means the file is
/// oversized (or grew past the limit while reading) and must be rejected by
/// the caller.
fn bounded_bytes(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if metadata.len() > MAX_INDEX_FILE_READ_BYTES {
        return None;
    }
    // Preallocate at most the limit (+1): the allocation itself stays
    // bounded even if the file grows between the metadata check and read.
    let mut buffer = Vec::with_capacity(
        metadata
            .len()
            .saturating_add(1)
            .min(MAX_INDEX_FILE_READ_BYTES + 1) as usize,
    );
    let mut limited = file.take(MAX_INDEX_FILE_READ_BYTES + 1);
    limited.read_to_end(&mut buffer).ok()?;
    Some(buffer)
}

fn first_match_line(path: &Path, query: &str) -> (Option<usize>, Option<String>) {
    let Some(bytes) = bounded_bytes(path) else {
        return (None, None);
    };
    // One byte past the limit means oversized: no snippet.
    if bytes.len() as u64 > MAX_INDEX_FILE_READ_BYTES {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&bytes);
    let lower = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    if !lower.contains(&needle) {
        return (None, None);
    }
    for (index, line) in text.lines().enumerate() {
        if line.to_ascii_lowercase().contains(&needle) {
            let cleaned: String = line.chars().take_while(|c| *c != '\r').collect();
            let mut snippet: String = if cleaned.chars().count() > 200 {
                format!("{}…", cleaned.chars().take(199).collect::<String>())
            } else {
                cleaned
            };
            snippet = snippet.trim_end().to_string();
            return (Some(index + 1), Some(snippet));
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::{extract_symbols, language_for};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn index_with(content: std::collections::BTreeMap<&str, &str>) -> ContextIndex {
        let files = content
            .into_iter()
            .map(|(path, text)| {
                let language = language_for(Path::new(path));
                let symbols = language
                    .map(|l| extract_symbols(l, text))
                    .unwrap_or_default();
                (
                    path.to_string(),
                    crate::index::IndexedFile {
                        path: path.to_string(),
                        language: language.map(|l| l.to_string()),
                        size: text.len() as u64,
                        mtime_ms: 0,
                        content_hash: None,
                        symbols,
                    },
                )
            })
            .collect();
        ContextIndex {
            root: PathBuf::from("/repo"),
            files,
            oversize: false,
        }
    }

    #[test]
    fn index_search_finds_path_and_symbol_matches() {
        let index = index_with(std::collections::BTreeMap::from([
            ("src/auth.rs", "pub fn authenticate() {}\n"),
            ("src/db.rs", "pub fn open() {}\n"),
        ]));
        let hits = search_index(&index, "auth", 10);
        assert_eq!(hits.len(), 2);
        assert!(hits[0].path.ends_with("auth.rs"));
        assert!(hits[1].match_kind == MatchKind::Symbol);
    }

    #[test]
    fn content_search_falls_back_to_rust_scan() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "nothing\n").unwrap();
        let hits = search_content(dir.path(), &IgnoreRules::empty(), "hello", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "a.txt");
        assert_eq!(hits[0].line, Some(1));
    }

    #[test]
    fn builtin_dir_contents_are_not_scanned() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(
            dir.path().join("node_modules").join("x.js"),
            "hidden secret content\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("ok.js"), "not a match\n").unwrap();
        let hits = search_content(dir.path(), &IgnoreRules::empty(), "secret", 10).unwrap();
        assert!(hits.is_empty());
    }
}
