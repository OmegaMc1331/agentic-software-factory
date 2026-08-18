//! Repository search over the index (fast path) and file contents (ripgrep
//! with a pure-Rust fallback).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::error::Result;
use crate::ignore::IgnoreRules;
use crate::index::{ContextIndex, MAX_INDEX_FILE_READ_BYTES, forward_slash, normalize};
use crate::rank::query_tokens;
use crate::symbols::Symbol;

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
    for file in index.files.values() {
        let lower = file.path.to_ascii_lowercase();
        if terms.iter().any(|term| lower.contains(term.as_str())) {
            path_hits.push(SearchHit {
                path: file.path.clone(),
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
                path: file.path.clone(),
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

/// Whether ripgrep is available. Cached once per process.
fn ripgrep_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("rg")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    })
}

fn content_glob_flags() -> Vec<String> {
    let mut flags = Vec::new();
    const DIRS: &[&str] = &[
        ".factory", "node_modules", "target", "dist", "build", "vendor",
        "__pycache__", ".venv", "venv", "coverage", ".pytest_cache", ".ruff_cache", ".next", ".git",
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

/// Content search over `root`. Uses ripgrep when installed; otherwise falls
/// back to a bounded pure-Rust substring scan. Returns paths (not full diffs)
/// plus a first-match line/snippet resolved by reading each hit.
pub fn search_content(
    root: &Path,
    ignore: &IgnoreRules,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let terms = query_tokens(query, 1);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let matched_paths: Vec<PathBuf> = if ripgrep_available() {
        let mut command = Command::new("rg");
        command
            .arg("--files-with-matches")
            .arg("--hidden")
            .arg("-i")
            .arg("-m")
            .arg("1")
            .args(content_glob_flags())
            .arg(query)
            .arg(normalize(root));
        match command.output() {
            Ok(out) if out.status.success() => {
                let root = normalize(root);
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    // Keep builtin/vendor rules consistent between ripgrep and
                    // the pure-Rust fallback: a file nested inside an ignored
                    // directory is never a hit.
                    .filter(|path| {
                        path.strip_prefix(&root)
                            .map(|relative| !ignore.is_ignored(relative, path.is_dir()))
                            .unwrap_or(true)
                    })
                    .collect()
            }
            Ok(_) | Err(_) => fallback_paths(root, ignore, query),
        }
    } else {
        fallback_paths(root, ignore, query)
    };

    let mut hits: Vec<SearchHit> = Vec::new();
    for path in matched_paths.into_iter().take(limit) {
        let relative = path.strip_prefix(normalize(root)).ok();
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

fn fallback_paths(root: &Path, ignore: &IgnoreRules, query: &str) -> Vec<PathBuf> {
    let query = query.to_ascii_lowercase();
    let root = normalize(root);
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(&root).unwrap_or(&path);
            if ignore.is_ignored(relative, path.is_dir()) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.len() as u64 > MAX_INDEX_FILE_READ_BYTES {
                continue;
            }
            if String::from_utf8_lossy(&bytes).to_ascii_lowercase().contains(&query) {
                found.push(path);
            }
        }
    }
    found
}

fn first_match_line(path: &Path, query: &str) -> (Option<usize>, Option<String>) {
    let Ok(bytes) = std::fs::read(path) else {
        return (None, None);
    };
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
                let symbols = language.map(|l| extract_symbols(l, text)).unwrap_or_default();
                (
                    path.to_string(),
                    crate::index::IndexedFile {
                        path: path.to_string(),
                        language: language.map(|l| l.to_string()),
                        size: text.len() as u64,
                        mtime_ms: 0,
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