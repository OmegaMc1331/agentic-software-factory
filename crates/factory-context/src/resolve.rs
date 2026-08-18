//! The reservoir of the engine: `ContextEngine` owns the index lifecycle,
//! git-aware scope metadata, budget enforcement, and turns a resolved ranking
//! into the `ResolvedContext` that feeds both the mission and the dashboard.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use factory_git::Repo;
use serde::{Deserialize, Serialize};

use crate::config::ContextConfig;
use crate::error::Result;
use crate::ignore::IgnoreRules;
use crate::index::{
    load_manifest, normalize, now_secs, save_manifest, ContextIndex, IndexSummary, IndexedFile,
    RefreshStats,
};
use crate::rank::{self, is_test_path, query_terms_for, ContextRequest, RankedFile};
use crate::search::{self, search_index, SearchHit};
use crate::symbols::Symbol;

/// Maximum characters of a per-file excerpt embedded in a resolved context.
pub const MAX_FILE_EXCERPT_CHARS: usize = 1_600;
/// Lines of context kept on each side of an excerpt anchor.
const EXCERPT_RADIUS: usize = 3;
/// Cap on the number of related tests surfaced per task.
const RELATED_TESTS_LIMIT: usize = 4;

/// One selected file with everything the renderer and dashboard need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedFile {
    pub path: String,
    pub language: Option<String>,
    pub symbols: Vec<Symbol>,
    /// A bounded excerpt anchored near the most relevant line.
    pub excerpt: String,
    pub reasons: Vec<String>,
    pub score: i64,
}

/// A test file related to the task, with its subject when it pairs with a
/// selected file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedTest {
    pub path: String,
    pub for_target: Option<String>,
}

/// The resolved repository context for one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedContext {
    /// The engine is disabled via `[context] enabled = false`.
    pub enabled: bool,
    pub scope_dir: PathBuf,
    pub is_worktree: bool,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub base_sha: Option<String>,
    pub candidates_considered: usize,
    pub budget_files: usize,
    pub budget_chars: usize,
    /// The indexed repository was smaller than its candidates; oversize means
    /// the walk hit `MAX_INDEX_ENTRIES` before finishing.
    pub oversize: bool,
    pub selected: Vec<SelectedFile>,
    pub related_tests: Vec<RelatedTest>,
}

impl Default for ResolvedContext {
    fn default() -> Self {
        Self {
            enabled: true,
            scope_dir: PathBuf::new(),
            is_worktree: false,
            branch: None,
            head: None,
            base_sha: None,
            candidates_considered: 0,
            budget_files: 0,
            budget_chars: 0,
            oversize: false,
            selected: Vec::new(),
            related_tests: Vec::new(),
        }
    }
}

/// Owns the persisted index and resolves task contexts. Cheap to re-create
/// (the manifest is small), so callers may build one per operation.
pub struct ContextEngine {
    pub config: ContextConfig,
    pub root: PathBuf,
    pub factory_dir: PathBuf,
    index: Option<ContextIndex>,
    manifest_created_at: u64,
    ignore: IgnoreRules,
}

impl ContextEngine {
    pub fn new(root: &Path, factory_dir: &Path, config: ContextConfig) -> Self {
        let root = normalize(root);
        let ignore = match std::fs::read(root.join(".gitignore")) {
            Ok(bytes) => IgnoreRules::from_gitignore(&String::from_utf8_lossy(&bytes)),
            Err(_) => IgnoreRules::empty(),
        };
        Self {
            config,
            root,
            factory_dir: factory_dir.to_path_buf(),
            index: None,
            manifest_created_at: now_secs(),
            ignore,
        }
    }

    pub fn index_path(&self) -> PathBuf {
        self.factory_dir.join("context").join("index.json")
    }

    /// Loads the persisted index when it targets the same root, otherwise
    /// rebuilds it. The first `refresh` reconciles cached symbols.
    pub fn ensure_index(&mut self) -> Result<()> {
        if self.index.is_some() {
            return Ok(());
        }
        let path = self.index_path();
        if let Some(manifest) = load_manifest(&path)? {
            if normalize(Path::new(&manifest.root)) == self.root {
                self.manifest_created_at = manifest.created_at_secs;
                self.index = Some(ContextIndex::from_manifest(&manifest));
            }
        }
        self.refresh()?;
        Ok(())
    }

    /// Incrementally refreshes the in-memory index and persists it when the
    /// working tree changed.
    pub fn refresh(&mut self) -> Result<()> {
        let index = self.index.get_or_insert_with(|| ContextIndex {
            root: self.root.clone(),
            files: Default::default(),
            oversize: false,
        });
        let stats: RefreshStats = index.refresh(&self.root, &self.ignore)?;
        if stats.overall_changed() {
            let manifest = index.to_manifest(self.manifest_created_at, now_secs());
            save_manifest(&self.index_path(), &manifest)?;
        }
        Ok(())
    }

    pub fn index_summary(&mut self) -> Result<IndexSummary> {
        self.ensure_index()?;
        let index = self.index.as_ref().expect("index loaded");
        Ok(IndexSummary {
            root: index.root.to_string_lossy().into_owned(),
            file_count: index.file_count(),
            symbol_count: index.symbol_count(),
            oversize: index.oversize,
            updated_at_secs: Some(now_secs()),
            engine_enabled: self.config.enabled,
            index_path: self.index_path().to_string_lossy().into_owned(),
        })
    }

    /// Combines the index fast path with content search. Deterministic and
    /// bounded: index hits (path/symbol) first, then content hits.
    pub fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }
        self.ensure_index()?;
        let index = self.index.as_ref().expect("index loaded");
        let mut hits = search_index(index, query, limit);
        if hits.len() < limit {
            let content_hits = search::search_content(&self.root, &self.ignore, query, limit)?;
            for content_hit in content_hits {
                if hits.iter().any(|hit| hit.path == content_hit.path) {
                    continue;
                }
                hits.push(content_hit);
                if hits.len() >= limit {
                    break;
                }
            }
        }
        hits.truncate(limit);
        Ok(hits)
    }

    /// Resolves the deterministic task context for `request`. Never fails on
    /// git trouble (scope metadata degrades to `None`) and never blocks on
    /// agent availability.
    pub fn resolve(&mut self, request: &ContextRequest) -> Result<ResolvedContext> {
        if !self.config.enabled {
            return Ok(ResolvedContext {
                enabled: false,
                ..ResolvedContext::default()
            });
        }
        self.ensure_index()?;
        let index = self.index.as_ref().expect("index loaded");

        let (is_worktree, branch, head, base_sha) =
            scope_git_info(&request.scope_dir, request.base_sha.as_deref());

        let mut ranked = rank::rank_candidates(index, request);
        ranked.retain(|file| file.score > 0);
        rank::expand_links(&mut ranked, &self.root);

        // Test files are surfaced as *related* tests, never as selected
        // working files, so the budget stays on the code being changed and the
        // inspector shows both lists without overlap.
        let budget_files = self.config.max_files.min(ranked.len());
        let selected_ranked: Vec<RankedFile> = ranked
            .iter()
            .filter(|file| !is_test_path(&file.file.path))
            .take(budget_files)
            .cloned()
            .collect();
        let selected_keys: BTreeSet<String> = selected_ranked
            .iter()
            .map(|file| file.file.path.clone())
            .collect();

        let mut selected: Vec<SelectedFile> = Vec::with_capacity(selected_ranked.len());
        let anchor_terms = query_terms_for(request);
        for ranked_file in &selected_ranked {
            let span = extract_excerpt(&self.root, &ranked_file.file, &anchor_terms);
            selected.push(SelectedFile {
                path: ranked_file.file.path.clone(),
                language: ranked_file.file.language.clone(),
                symbols: ranked_file.file.symbols.clone(),
                excerpt: span,
                reasons: ranked_file.reasons.clone(),
                score: ranked_file.score,
            });
        }

        let related_tests = if self.config.include_tests {
            rank::related_tests(&ranked, &selected_keys, RELATED_TESTS_LIMIT)
                .into_iter()
                .map(|(file, target)| RelatedTest {
                    path: file.file.path,
                    for_target: target,
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(ResolvedContext {
            enabled: true,
            scope_dir: normalize(&request.scope_dir),
            is_worktree,
            branch,
            head,
            base_sha,
            candidates_considered: index.file_count(),
            budget_files: self.config.max_files,
            budget_chars: self.config.max_chars,
            oversize: index.oversize,
            selected,
            related_tests,
        })
    }
}

/// Best-effort git metadata for the resolution scope. Worktrees report their
/// branch and the worktree's own head; failures degrade to `None` rather than
/// abort the resolution.
fn scope_git_info(
    scope_dir: &Path,
    base_sha: Option<&str>,
) -> (bool, Option<String>, Option<String>, Option<String>) {
    let scope = normalize(scope_dir);
    let repo = match Repo::detect(&scope) {
        Ok(repo) => repo,
        Err(_) => return (false, None, None, base_sha.map(str::to_string)),
    };
    let is_worktree = repo.is_main_worktree().map(|main| !main).unwrap_or(false);
    let branch = Repo::detect(scope_dir)
        .ok()
        .and_then(|main_repo| main_repo.list_worktrees().ok())
        .and_then(|worktrees| {
            worktrees
                .into_iter()
                .find(|info| normalize(&info.path) == scope)
                .and_then(|info| info.branch)
        });
    let head = repo.head_sha(&scope).ok();
    (is_worktree, branch, head, base_sha.map(str::to_string))
}

/// Reads a file (bounded) and produces an excerpt anchored near the first
/// matching symbol line, otherwise the first matching query-term line,
/// otherwise the file head.
fn extract_excerpt(root: &Path, file: &IndexedFile, anchor_terms: &[String]) -> String {
    let path = root.join(&file.path);
    let Ok(bytes) = std::fs::read(&path) else {
        return String::new();
    };
    if bytes.len() as u64 > crate::index::MAX_INDEX_FILE_READ_BYTES {
        return String::new();
    }
    let content = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = content.lines().collect();
    let anchor = file
        .symbols
        .first()
        .map(|symbol| symbol.line)
        .or_else(|| {
            let lower = content.to_ascii_lowercase();
            anchor_terms
                .iter()
                .find(|term| lower.contains(term.as_str()))
                .and_then(|term| {
                    lines.iter().position(|line| {
                        line.to_ascii_lowercase()
                            .contains(&term.to_ascii_lowercase())
                    })
                })
                .map(|index| index + 1)
        })
        .unwrap_or(1);
    let start = anchor.saturating_sub(EXCERPT_RADIUS + 1);
    let mut excerpt = String::new();
    let mut chars = 0usize;
    for line in lines.iter().skip(start).take(EXCERPT_RADIUS * 2 + 1) {
        if chars >= MAX_FILE_EXCERPT_CHARS {
            break;
        }
        excerpt.push_str(line);
        excerpt.push('\n');
        chars += line.chars().count() + 1;
    }
    if chars >= MAX_FILE_EXCERPT_CHARS {
        excerpt.push('…');
    }
    excerpt
}

/// Detects test files in the resolved pool (used by the dashboard's related
/// tests helpers and for debugging).
pub fn is_test(path: &str) -> bool {
    is_test_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ContextConfig;
    use tempfile::TempDir;

    fn engine(root: &Path) -> ContextEngine {
        ContextEngine::new(root, &root.join(".factory"), ContextConfig::default())
    }

    fn seed(root: &Path) {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(
            root.join("src").join("auth.rs"),
            "// line 1\n// line 2\n// line 3\npub fn authenticate() {}\n",
        )
        .unwrap();
        std::fs::write(root.join("src").join("db.rs"), "pub fn open() {}\n").unwrap();
        std::fs::write(
            root.join("tests").join("auth_test.rs"),
            "use crate::authenticate;\n",
        )
        .unwrap();
    }

    #[test]
    fn resolve_selects_relevant_files_and_related_tests() {
        let dir = TempDir::new().unwrap();
        seed(dir.path());
        let mut engine = engine(dir.path());
        let resolved = engine
            .resolve(&ContextRequest {
                scope_dir: dir.path().to_path_buf(),
                root_dir: dir.path().to_path_buf(),
                title: "authenticate users".into(),
                operation: Some(factory_types::TaskOperation::Verify),
                ..Default::default()
            })
            .expect("resolution succeeds");
        assert!(resolved.enabled);
        assert!(resolved
            .selected
            .iter()
            .any(|file| file.path == "src/auth.rs"));
        assert!(resolved
            .related_tests
            .iter()
            .any(|test| test.path == "tests/auth_test.rs"));
        assert!(!resolved.selected.is_empty());
    }

    #[test]
    fn disabled_engine_returns_inert_context() {
        let dir = TempDir::new().unwrap();
        seed(dir.path());
        let config = ContextConfig {
            enabled: false,
            ..ContextConfig::default()
        };
        let mut engine = ContextEngine::new(dir.path(), &dir.path().join(".factory"), config);
        let resolved = engine
            .resolve(&ContextRequest {
                scope_dir: dir.path().to_path_buf(),
                root_dir: dir.path().to_path_buf(),
                ..Default::default()
            })
            .expect("resolution succeeds even when disabled");
        assert!(!resolved.enabled);
        assert!(resolved.selected.is_empty());
    }

    #[test]
    fn budget_is_enforced_on_selection() {
        let dir = TempDir::new().unwrap();
        seed(dir.path());
        let config = ContextConfig {
            enabled: true,
            max_files: 1,
            ..ContextConfig::default()
        };
        let mut engine = ContextEngine::new(dir.path(), &dir.path().join(".factory"), config);
        let resolved = engine
            .resolve(&ContextRequest {
                scope_dir: dir.path().to_path_buf(),
                root_dir: dir.path().to_path_buf(),
                title: "authenticate + db + open".into(),
                ..Default::default()
            })
            .expect("resolution succeeds");
        assert!(resolved.selected.len() <= 1);
        assert_eq!(resolved.budget_files, 1);
    }

    #[test]
    fn search_combines_index_and_content() {
        let dir = TempDir::new().unwrap();
        seed(dir.path());
        let mut engine = engine(dir.path());
        let hits = engine.search("authenticate", 10).expect("search works");
        assert!(!hits.is_empty());
    }
}
