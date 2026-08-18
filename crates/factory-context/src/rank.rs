//! Deterministic task-context ranking.
//!
//! Candidates are scored with fixed weights (no learned model), so two runs
//! against the same repository always select the same files. Every point a
//! file earns records a human-readable reason — this is what the dashboard
//! "why selected" panel and the CLI `-v` mode surface.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use factory_types::TaskOperation;

use crate::config::MAX_CONTEXT_FILES_LIMIT;
use crate::index::{ContextIndex, IndexedFile, MAX_INDEX_FILE_READ_BYTES};

/// Everything the resolver knows about the task being served. Supplied by the
/// Factory (mission builder) and by the dashboard/CLI inspector routes.
#[derive(Debug, Clone, Default)]
pub struct ContextRequest {
    /// Where the agent operates (the task worktree or the main checkout).
    pub scope_dir: PathBuf,
    /// The repository root whose index is loaded.
    pub root_dir: PathBuf,
    /// Value to attribute to the context resolution (task named "ctx-...", or
    /// None when resolving the raw index). Optimized away at render time.
    pub base_sha: Option<String>,
    pub role_id: Option<String>,
    pub operation: Option<TaskOperation>,
    pub title: String,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    /// Forward-slash repository-relative paths of the change under review.
    pub changed_files: Vec<String>,
    /// Optional keyword hints from upstream artifacts (bounded).
    pub upstream_artifact_snippets: Vec<String>,
}

const PATH_TERM_BONUS: i64 = 60;
const SYMBOL_TERM_BONUS: i64 = 40;
const CHANGED_BONUS: i64 = 500;
const TEST_RELEVANT_BONUS: i64 = 25;
const TEST_IRRELEVANT_PENALTY: i64 = 15;
const CODE_BONUS: i64 = 5;
const LINK_BONUS: i64 = 15;
/// How many top candidates participate in dependency-link expansion (reads are
/// bounded by `MAX_INDEX_FILE_READ_BYTES` each).
const LINK_POOL_LIMIT: usize = 24;
const LINK_MAX_PAIRS: usize = 300;
/// Maximum number of referenced symbols considered per candidate during link
/// expansion.
const LINK_SYMBOLS_PER_FILE: usize = 8;

/// A scored candidate with its accumulated reasons.
#[derive(Debug, Clone)]
pub struct RankedFile {
    pub file: IndexedFile,
    pub score: i64,
    pub reasons: Vec<String>,
}

impl RankedFile {
    pub fn new(file: IndexedFile) -> Self {
        Self {
            file,
            score: 0,
            reasons: Vec::new(),
        }
    }
}

/// Splits a token stream into lowercase alphanumeric tokens, breaking on
/// non-alphanumeric characters and on camelCase boundaries.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for part in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if part.is_empty() {
            continue;
        }
        split_camel(part, &mut tokens);
    }
    tokens
}

fn split_camel(word: &str, tokens: &mut Vec<String>) {
    let chars: Vec<char> = word.chars().collect();
    let mut start = 0usize;
    for i in 1..chars.len() {
        let current = chars[i];
        let previous = chars[i - 1];
        let next_is_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
        let boundary = current.is_uppercase()
            && (previous.is_lowercase() || previous.is_ascii_digit())
            || current.is_uppercase() && next_is_lower && i - start > 1;
        if boundary {
            tokens.push(chars[start..i].iter().collect::<String>().to_ascii_lowercase());
            start = i;
        }
    }
    tokens.push(chars[start..].iter().collect::<String>().to_ascii_lowercase());
}

/// Lowercase tokens of `text` that are at least `min_len` characters long.
pub fn query_tokens(text: &str, min_len: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in tokenize(text) {
        if token.chars().count() >= min_len {
            out.push(token);
        }
    }
    out
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "within", "make", "use", "used",
    "uses", "using", "should", "will", "can", "could", "would", "task", "feature", "implement",
    "implementation", "need", "needs", "ensure", "add", "support", "its", "their", "are", "was",
    "were", "not", "work", "works", "working",
];

/// Primary ranking terms from the task text, deduplicated and deterministic.
pub fn query_terms_for(request: &ContextRequest) -> Vec<String> {
    let mut set = BTreeSet::new();
    let mut consider = |text: &str, min_len: usize| {
        for token in query_tokens(text, min_len) {
            if !STOPWORDS.contains(&token.as_str()) {
                set.insert(token);
            }
        }
    };
    consider(&request.title, 3);
    consider(&request.objective, 3);
    for criterion in &request.acceptance_criteria {
        consider(criterion, 3);
    }
    for snippet in request
        .upstream_artifact_snippets
        .iter()
        .take(6)
    {
        consider(&snippet.chars().take(500).collect::<String>(), 4);
    }
    set.into_iter().collect()
}

/// Whether the request is about testing/verification, which makes test files
/// relevant context rather than noise.
fn testing_intent(request: &ContextRequest) -> bool {
    if request.operation == Some(TaskOperation::Verify)
        || request
            .role_id
            .as_deref()
            .is_some_and(|role| role.contains("test") || role == "tester")
    {
        return true;
    }
    let words = [
        request.title.as_str(),
        request.objective.as_str(),
    ];
    let mut tokens = words.into_iter().flat_map(query_tokens_min_3);
    tokens.any(|token| {
        matches!(token.as_str(), "test" | "tests" | "testing" | "spec" | "specs" | "verify")
    })
}

fn query_tokens_min_3(text: &str) -> Vec<String> {
    query_tokens(text, 3)
}

/// Whether a repository-relative path is a test file.
pub fn is_test_path(key: &str) -> bool {
    let basename = key.rsplit('/').next().unwrap_or(key);
    if basename.contains(".test.") || basename.contains(".spec.") {
        return true;
    }
    if basename.starts_with("test_")
        || basename.ends_with("_test")
        || basename == "tests.rs"
        || basename == "tests.py"
        || basename == "test.rs"
        || basename == "test.py"
    {
        return true;
    }
    key.split('/')
        .any(|part| part == "tests" || part == "__tests__" || part == "test")
}

/// Strips test markers from a basename so related tests can be paired with the
/// subject they exercise (e.g. `auth_test.rs` → `auth`).
fn test_subject(basename: &str) -> String {
    let stem = basename.rsplit('/').next().unwrap_or(basename);
    let mut stem = stem
        .to_string()
        .replace(".test.", ".")
        .replace(".spec.", ".");
    for marker in ["_test", "_spec", "Test", "Spec"] {
        if stem.ends_with(marker) {
            stem.truncate(stem.len() - marker.len());
            break;
        }
    }
    if stem.starts_with("test_") {
        stem = stem.trim_start_matches("test_").to_string();
    }
    stem.to_ascii_lowercase()
}

/// Scores every indexed file against the request. Deterministic and cheap —
/// operates purely on paths and symbol lists, never reading file contents.
pub fn rank_candidates(index: &ContextIndex, request: &ContextRequest) -> Vec<RankedFile> {
    let terms = query_terms_for(request);
    let testing = testing_intent(request);
    let changed: BTreeSet<&String> = request.changed_files.iter().collect();
    let mut ranked: Vec<RankedFile> = Vec::with_capacity(index.files.len());
    for entry in index.files.values() {
        let mut file = RankedFile::new(entry.clone());
        let path_lower = file.file.path.to_ascii_lowercase();
        for term in &terms {
            if path_lower.contains(term) {
                file.score += PATH_TERM_BONUS;
                file.reasons
                    .push(format!("path matches task term '{term}'"));
            }
        }
        if !file.file.symbols.is_empty() {
            let mut matched_symbols = 0usize;
            for symbol in &file.file.symbols {
                let name = symbol.name.to_ascii_lowercase();
                if terms.iter().any(|term| {
                    term.len() >= 4 && (name.contains(term.as_str()) || term.contains(name.as_str()))
                }) {
                    matched_symbols += 1;
                }
            }
            if matched_symbols > 0 {
                file.score += SYMBOL_TERM_BONUS + (matched_symbols as i64);
                file.reasons
                    .push(format!("symbols match task terms ({matched_symbols})"));
            }
        }
        if changed.contains(&file.file.path) {
            file.score += CHANGED_BONUS;
            file.reasons
                .push("changed in this attempt".to_string());
        }
        let is_test = is_test_path(&file.file.path);
        if is_test {
            if testing {
                file.score += TEST_RELEVANT_BONUS;
                file.reasons.push("related test file".to_string());
            } else {
                file.score -= TEST_IRRELEVANT_PENALTY;
            }
        }
        if file.file.language.is_some() {
            file.score += CODE_BONUS;
        }
        ranked.push(file);
    }
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.file.path.cmp(&b.file.path))
    });
    ranked.truncate(MAX_CONTEXT_FILES_LIMIT); // defensive upper bound
    ranked
}

/// Reads the contents of the top-link-pool candidates and boosts files that
/// reference (or are referenced by) one another. The reads are bounded, and
/// the pair order is deterministic (sorted pool).
pub fn expand_links(pool: &mut [RankedFile], root: &Path) {
    let n = pool.len().min(LINK_POOL_LIMIT);
    if n < 2 {
        return;
    }
    let mut contents: Vec<String> = Vec::with_capacity(n);
    for file in pool.iter().take(n) {
        let path = root.join(&file.file.path);
        contents.push(std::fs::read(&path).ok().and_then(|bytes| {
            (bytes.len() as u64 <= MAX_INDEX_FILE_READ_BYTES)
                .then(|| String::from_utf8_lossy(&bytes).to_ascii_lowercase())
        }).unwrap_or_default());
    }
    let mut pairs = 0usize;
    for i in 0..n {
        if pairs >= LINK_MAX_PAIRS {
            break;
        }
        for j in (i + 1)..n {
            if pairs >= LINK_MAX_PAIRS {
                break;
            }
            pairs += 1;
            let references = link_tokens(&pool[i].file).into_iter().any(|token| {
                pool[j].file.path.to_ascii_lowercase().contains(&token)
                    || contents[j].contains(&token)
            });
            let referenced_by = link_tokens(&pool[j].file).into_iter().any(|token| {
                pool[i].file.path.to_ascii_lowercase().contains(&token)
                    || contents[i].contains(&token)
            });
            if references {
                pool[i].score += LINK_BONUS;
                pool[i].reasons.push(format!("references {}", pool[j].file.path));
            }
            if referenced_by {
                pool[j].score += LINK_BONUS;
                pool[j].reasons.push(format!("referenced by {}", pool[i].file.path));
            }
        }
    }
    pool.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.file.path.cmp(&b.file.path))
    });
}

fn link_tokens(file: &IndexedFile) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut stem = file
        .basename()
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file.basename())
        .to_ascii_lowercase();
    if !stem.is_empty() {
        tokens.push(stem);
    }
    // Also carry the basename without its subject marker when it is a test.
    if is_test_path(&file.path) {
        stem = test_subject(file.basename());
        tokens.push(stem);
    }
    tokens.extend(
        file.symbols
            .iter()
            .take(LINK_SYMBOLS_PER_FILE)
            .map(|symbol| symbol.name.to_ascii_lowercase()),
    );
    tokens
        .into_iter()
        .filter(|token| token.len() >= 4)
        .collect()
}

/// Pairs test files with the selected subject they appear to exercise.
pub fn related_tests(
    ranked: &[RankedFile],
    selected_keys: &BTreeSet<String>,
    limit: usize,
) -> Vec<(RankedFile, Option<String>)> {
    let mut tests: Vec<(RankedFile, Option<String>)> = Vec::new();
    for file in ranked {
        if file.score <= 0 || !is_test_path(&file.file.path) {
            continue;
        }
        if selected_keys.contains(&file.file.path) {
            continue;
        }
        let subject = test_subject(file.file.basename());
        let target = ranked
            .iter()
            .filter(|candidate| candidate.score > 0 && !is_test_path(&candidate.file.path))
            .filter(|candidate| selected_keys.contains(&candidate.file.path))
            .map(|candidate| &candidate.file)
            .find(|candidate| {
                let stem = candidate
                    .basename()
                    .rsplit_once('.')
                    .map(|(stem, _)| stem)
                    .unwrap_or(candidate.basename())
                    .to_ascii_lowercase();
                stem == subject
                    || subject.is_empty()
                    || stem.contains(&subject)
                    || subject.contains(&stem)
            })
            .map(|candidate| candidate.path.clone());
        tests.push((file.clone(), target));
        if tests.len() >= limit {
            break;
        }
    }
    tests
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::Symbol;
    use std::collections::BTreeMap;

    fn entry(path: &str) -> IndexedFile {
        IndexedFile {
            path: path.to_string(),
            language: Some("rust".into()),
            size: 10,
            mtime_ms: 0,
            symbols: match path {
                "src/auth.rs" => vec![Symbol {
                    name: "authenticate".into(),
                    kind: "function".into(),
                    line: 4,
                }],
                "src/db.rs" => vec![Symbol {
                    name: "open".into(),
                    kind: "function".into(),
                    line: 2,
                }],
                _ => Vec::new(),
            },
        }
    }

    fn index() -> ContextIndex {
        let files: BTreeMap<String, IndexedFile> = ["src/auth.rs", "src/db.rs", "src/main.rs"]
            .into_iter()
            .map(|path| (path.to_string(), entry(path)))
            .collect();
        ContextIndex {
            root: PathBuf::from("/repo"),
            files,
            oversize: false,
        }
    }

    #[test]
    fn ranking_is_deterministic_and_term_sensitive() {
        let index = index();
        let request = ContextRequest {
            title: "Authenticate customers".into(),
            objective: "add login flow".into(),
            scope_dir: PathBuf::from("/repo"),
            root_dir: PathBuf::from("/repo"),
            ..Default::default()
        };
        let ranked = rank_candidates(&index, &request);
        assert_eq!(ranked[0].file.path, "src/auth.rs");
        assert!(ranked[0].score > ranked[1].score);
        // Two identical runs select identical files.
        let again = rank_candidates(&index, &request);
        assert_eq!(
            ranked.iter().map(|r| r.file.path.clone()).collect::<Vec<_>>(),
            again.iter().map(|r| r.file.path.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn changed_files_dominate_the_ranking() {
        let index = index();
        let request = ContextRequest {
            changed_files: vec!["src/db.rs".into()],
            ..Default::default()
        };
        let ranked = rank_candidates(&index, &request);
        assert_eq!(ranked[0].file.path, "src/db.rs");
        assert!(ranked[0].reasons.iter().any(|r| r.contains("changed")));
    }

    #[test]
    fn test_files_are_ranked_by_intent() {
        let mut index = index();
        index.files.insert(
            "tests/auth_test.rs".to_string(),
            IndexedFile {
                path: "tests/auth_test.rs".into(),
                language: Some("rust".into()),
                size: 10,
                mtime_ms: 0,
                symbols: Vec::new(),
            },
        );
        let verify = ContextRequest {
            operation: Some(TaskOperation::Verify),
            ..Default::default()
        };
        let ranked = rank_candidates(&index, &verify);
        let test = ranked
            .iter()
            .find(|r| r.file.path == "tests/auth_test.rs")
            .expect("test file ranked");
        assert!(test.score > 0);
        let plain = ContextRequest::default();
        let without_intent = rank_candidates(&index, &plain);
        let test = without_intent
            .iter()
            .find(|r| r.file.path == "tests/auth_test.rs")
            .expect("test file ranked");
        assert!(test.score < 0);
    }

    #[test]
    fn related_tests_pair_with_subjects() {
        let ranked = vec![
            RankedFile {
                file: entry("src/auth.rs"),
                score: 100,
                reasons: Vec::new(),
            },
            RankedFile {
                file: IndexedFile {
                    path: "tests/auth_test.rs".into(),
                    language: Some("rust".into()),
                    size: 10,
                    mtime_ms: 0,
                    symbols: Vec::new(),
                },
                score: 25,
                reasons: Vec::new(),
            },
        ];
        let selected: BTreeSet<String> = BTreeSet::from(["src/auth.rs".to_string()]);
        let tests = related_tests(&ranked, &selected, 4);
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].0.file.path, "tests/auth_test.rs");
        assert_eq!(tests[0].1.as_deref(), Some("src/auth.rs"));
    }

    #[test]
    fn tokenize_handles_camel_and_snake() {
        assert_eq!(tokenize("acceptanceCriteria for LoginFlow"), vec![
            "acceptance", "criteria", "for", "login", "flow"
        ]);
        assert_eq!(tokenize("auth.rs"), vec!["auth", "rs"]);
    }
}