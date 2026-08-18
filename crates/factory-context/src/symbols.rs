//! Language detection and regex-based symbol extraction.
//!
//! v1 uses per-language regular expressions behind a tiny rule table. This is
//! intentionally conservative: the extractor only needs to surface *relevant*
//! definitions to the mission, not a perfect AST. The `extract_symbols`
//! signature is the seam where a tree-sitter-backed extractor can slot in
//! later without touching the rest of the engine.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// A single named definition located in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    /// Coarse category: "function", "struct", "class", "type", ...
    pub kind: String,
    /// 1-based line number.
    pub line: usize,
}

/// The maximum number of symbols extracted per file. Ranking operates on this
/// list, so keeping it bounded keeps candidate scoring cheap and deterministic.
pub const MAX_SYMBOLS_PER_FILE: usize = 256;

struct Rule {
    kind: &'static str,
    regex: Regex,
}

type RuleSet = Vec<Rule>;

static RULES: OnceLock<HashMap<&'static str, RuleSet>> = OnceLock::new();

fn rules() -> &'static HashMap<&'static str, RuleSet> {
    RULES.get_or_init(|| {
        let mut map = HashMap::new();
        let entries: Vec<(&'static str, Vec<(&'static str, &'static str)>)> = vec![
            (
                "rust",
                vec![
                    ("function", r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("struct", r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("enum", r"^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("trait", r"^\s*(?:pub(?:\([^)]*\))?\s+)?trait\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("module", r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("impl", r"^\s*(?:unsafe\s+)?impl\s+(?:<[^>]*>\s*)?([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("type", r"^\s*(?:pub(?:\([^)]*\))?\s+)?type\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("const", r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z_][A-Z0-9_]*)"),
                    ("static", r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+([A-Z_][A-Z0-9_]*)"),
                ],
            ),
            (
                "typescript",
                vec![
                    ("function", r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][A-Za-z0-9_$]*)"),
                    ("class", r"^\s*(?:export\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)"),
                    ("interface", r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][A-Za-z0-9_$]*)"),
                    ("type", r"^\s*(?:export\s+)?type\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*="),
                    ("enum", r"^\s*(?:export\s+)?(?:const\s+)?enum\s+([A-Za-z_$][A-Za-z0-9_$]*)"),
                    ("function", r"^\s*(?:export\s+)?const\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:async\s*)?(?:\()"),
                    ("function", r"^\s*(?:export\s+)?const\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:async\s*)?[A-Za-z_$]"),
                ],
            ),
            (
                "python",
                vec![
                    ("function", r"^\s*(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                    ("class", r"^\s*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"),
                ],
            ),
            (
                "go",
                vec![
                    ("method", r"^func\s*\([^)]*\)\s+([A-Za-z_]\w*)"),
                    ("function", r"^func\s+([A-Za-z_]\w*)"),
                    ("type", r"^type\s+([A-Z]\w*)"),
                    ("const", r"^const\s+([A-Z_]\w*)"),
                    ("var", r"^var\s+([A-Z_]\w*)"),
                ],
            ),
        ];
        for (language, pattern_set) in entries {
            let rule_set = pattern_set
                .into_iter()
                .map(|(kind, pattern)| Rule {
                    kind,
                    // `(?m)` makes `^` match at every line start so symbol
                    // rules apply to whole multi-line files, not just line 1.
                    regex: Regex::new(&format!("(?m){pattern}")).expect("static symbol regex"),
                })
                .collect();
            map.insert(language, rule_set);
        }
        map
    })
}

/// Detects the extraction language for a file path, or `None` for files whose
/// contents are not worth symbol extraction (documentation, markup, data).
pub fn language_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some("rust"),
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "py" => Some("python"),
        "go" => Some("go"),
        _ => None,
    }
}

/// Extracts the named definitions of `content` for `language`. Symbols are
/// sorted by line then kind order, deduplicated (name + line), and bounded to
/// `MAX_SYMBOLS_PER_FILE`.
pub fn extract_symbols(language: &str, content: &str) -> Vec<Symbol> {
    let Some(rule_set) = rules().get(language) else {
        return Vec::new();
    };
    let mut symbols: Vec<Symbol> = Vec::new();
    for rule in rule_set {
        for captures in rule.regex.captures_iter(content) {
            let Some(name) = captures.get(1) else {
                continue;
            };
            let line = content[..name.start()]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1;
            symbols.push(Symbol {
                name: name.as_str().to_string(),
                kind: rule.kind.to_string(),
                line,
            });
        }
    }
    symbols.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    // First occurrence per name wins (sorted, so the earliest line), keeping
    // the list deterministic for ranking and rendering.
    let mut seen = std::collections::HashSet::new();
    symbols.retain(|symbol| seen.insert(symbol.name.clone()));
    symbols.truncate(MAX_SYMBOLS_PER_FILE);
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_languages_by_extension() {
        assert_eq!(language_for(Path::new("src/main.rs")), Some("rust"));
        assert_eq!(language_for(Path::new("app/api.tsx")), Some("typescript"));
        assert_eq!(language_for(Path::new("views/index.svelte")), None);
        assert_eq!(language_for(Path::new("README.md")), None);
    }

    #[test]
    fn extracts_rust_symbols_with_lines_and_dedup() {
        let content = "mod auth;\n\npub fn login() {}\nfn login() {}\npub struct Session {}\n";
        let symbols = extract_symbols("rust", content);
        let names: Vec<(&str, usize)> = symbols.iter().map(|s| (s.name.as_str(), s.line)).collect();
        assert_eq!(names, vec![("auth", 1), ("login", 3), ("Session", 5)]);
        assert_eq!(symbols[1].kind, "function");
        assert_eq!(symbols[2].kind, "struct");
    }

    #[test]
    fn extracts_typescript_declarations() {
        let content = "export interface User { id: string }\nexport type Id = string;\nexport function load() {}\nconst parse = (s: string) => s;\n";
        let symbols = extract_symbols("typescript", content);
        assert!(symbols
            .iter()
            .any(|s| s.name == "User" && s.kind == "interface"));
        assert!(symbols.iter().any(|s| s.name == "Id" && s.kind == "type"));
        assert!(symbols
            .iter()
            .any(|s| s.name == "load" && s.kind == "function"));
        assert!(symbols.iter().any(|s| s.name == "parse"));
    }

    #[test]
    fn extracts_python_def_and_class() {
        let content = "import os\n\ndef parse_config(path: str) -> dict:\n    pass\n\nclass Engine:\n    pass\n";
        let symbols = extract_symbols("python", content);
        assert!(symbols
            .iter()
            .any(|s| s.name == "parse_config" && s.line == 3));
        assert!(symbols
            .iter()
            .any(|s| s.name == "Engine" && s.line == 3 + 3));
    }

    #[test]
    fn extracts_go_functions_and_methods() {
        let content = "func main() {}\nfunc (e *Engine) Start() {}\ntype Engine struct {}\n";
        let symbols = extract_symbols("go", content);
        assert!(symbols
            .iter()
            .any(|s| s.name == "main" && s.kind == "function"));
        assert!(symbols
            .iter()
            .any(|s| s.name == "Start" && s.kind == "method"));
        assert!(symbols
            .iter()
            .any(|s| s.name == "Engine" && s.kind == "type"));
    }

    #[test]
    fn unknown_language_yields_no_symbols() {
        assert!(extract_symbols("markdown", "## Head").is_empty());
    }
}
