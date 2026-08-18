//! Bread-and-butter ignore rules for the bounded repository walk.
//!
//! The engine is read-only and offline-friendly: it honors the Factory's own
//! production directories plus standard build/vendor directories, and applies
//! a pragmatic subset of the root `.gitignore` (blank lines, comments, exact
//! names, anchored paths, `*`/`**` globs, directory `/` suffixes, and `!`
//! negations). It deliberately does not run `git check-ignore`, so the index
//! stays deterministic even when git does not work.

use std::path::{Component, Path};

/// Directory names always skipped at any depth — including the Factory's own
/// state and common build/vendor sinks.
const ALWAYS_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".factory",
    ".hg",
    ".svn",
    ".idea",
    ".vscode",
    ".next",
    ".turbo",
    ".cache",
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
    ".tmp_src",
];

/// Files always skipped because their contents are not useful repository
/// context (binaries, archives, artifacts, lockfiles). Entries may be exact
/// names or `*.ext` wildcards.
const ALWAYS_IGNORED_FILES: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "*.png",
    "*.jpg",
    "*.jpeg",
    "*.gif",
    "*.webp",
    "*.ico",
    "*.svgz",
    "*.woff",
    "*.woff2",
    "*.ttf",
    "*.otf",
    "*.eot",
    "*.pdf",
    "*.zip",
    "*.tar",
    "*.gz",
    "*.tgz",
    "*.wasm",
    "*.exe",
    "*.dll",
    "*.so",
    "*.dylib",
    "*.class",
    "*.lock",
];

pub fn is_builtin_ignored_dir(name: &str) -> bool {
    ALWAYS_IGNORED_DIRS.contains(&name)
}

pub fn is_builtin_ignored_file(name: &str) -> bool {
    ALWAYS_IGNORED_FILES.iter().any(|entry| {
        if let Some(ext) = entry.strip_prefix('*') {
            name.ends_with(ext)
        } else {
            name == *entry
        }
    })
}

/// Compiled ignore rules derived from the repository root `.gitignore`.
#[derive(Debug, Default, Clone)]
pub struct IgnoreRules {
    globs: Vec<IgnorePattern>,
}

#[derive(Debug, Clone)]
struct IgnorePattern {
    /// `true` for a `!` negation pattern.
    negate: bool,
    only_directories: bool,
    regex: regex::Regex,
}

impl IgnoreRules {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parses `.gitignore` semantics from `text`. Patterns starting with `!`
    /// re-include previously ignored paths; the last matching pattern wins, as
    /// in git.
    pub fn from_gitignore(text: &str) -> Self {
        let mut rules = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negate, raw) = match line.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            let only_directories = raw.ends_with('/');
            let raw = raw.trim_end_matches('/');
            if raw.is_empty() {
                continue;
            }
            let anchored = raw.starts_with('/');
            let raw = raw.trim_start_matches('/');

            let mut parts = Vec::new();
            let bytes = raw.as_bytes();
            let mut i = 0usize;
            while i < bytes.len() {
                match bytes[i] {
                    b'*' => {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                            parts.push(".*".to_string());
                            i += 2;
                        } else {
                            parts.push("[^/]*".to_string());
                            i += 1;
                        }
                    }
                    b'?' => {
                        parts.push("[^/]".to_string());
                        i += 1;
                    }
                    _ => {
                        parts.push(regex::escape(&raw[i..i + 1]));
                        i += 1;
                    }
                }
            }
            let pattern = parts.join("");
            // Anchored patterns bind to the root; all other patterns match the
            // tail of the relative path (git's "no slash" basename case and
            // "has slash" cases both reduce to this for a root-level file).
            let regex_text = if anchored {
                format!("^{pattern}$")
            } else {
                format!("(?:^|/){pattern}$")
            };
            if let Ok(regex) = regex::Regex::new(&regex_text) {
                rules.globs.push(IgnorePattern {
                    negate,
                    only_directories,
                    regex,
                });
            }
        }
        rules
    }

    /// Whether `relative` (a forward-slash path relative to the walk root)
    /// should be skipped. Directories must be passed with `is_directory` set.
    pub fn is_ignored(&self, relative: &Path, is_directory: bool) -> bool {
        // Builtin directory rules apply to every nested component, whether the
        // entry itself is a file or a directory.
        if relative
            .components()
            .any(|component| match component {
                Component::Normal(name) => name
                    .to_str()
                    .map(is_builtin_ignored_dir)
                    .unwrap_or(false),
                _ => false,
            })
        {
            return true;
        }
        if !is_directory
            && relative
                .file_name()
                .and_then(|name| name.to_str())
                .map(is_builtin_ignored_file)
                .unwrap_or(false)
        {
            return true;
        }
        // Builtin rules cannot be undone by gitignore negation; parsed rules
        // decide the rest with git-compatible precedence.
        let mut ignored = false;
        let text = relative.to_string_lossy().replace('\\', "/");
        for rule in &self.globs {
            if rule.only_directories && !is_directory {
                continue;
            }
            if rule.regex.is_match(&text) {
                ignored = !rule.negate;
            }
        }
        ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rel(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[test]
    fn builtin_directories_always_skipped() {
        let rules = IgnoreRules::empty();
        assert!(rules.is_ignored(&rel("node_modules"), true));
        assert!(rules.is_ignored(&rel("src/ui/node_modules/react"), true));
        assert!(rules.is_ignored(&rel(".factory/config.toml"), true));
        assert!(rules.is_ignored(&rel("target/debug/app"), true));
        assert!(!rules.is_ignored(&rel("src/main.rs"), false));
    }

    #[test]
    fn builtin_files_always_skipped() {
        let rules = IgnoreRules::empty();
        assert!(rules.is_ignored(&rel("assets/logo.png"), false));
        assert!(rules.is_ignored(&rel("Cargo.lock"), false));
        assert!(!rules.is_ignored(&rel("src/main.rs"), false));
    }

    #[test]
    fn gitignore_basename_patterns_match_at_any_depth() {
        let rules = IgnoreRules::from_gitignore("logs/\n*.log\n");
        assert!(rules.is_ignored(&rel("logs"), true));
        assert!(!rules.is_ignored(&rel("logs"), false)); // dir-only rule
        assert!(rules.is_ignored(&rel("nested/logs"), true));
        assert!(rules.is_ignored(&rel("app/out.log"), false));
        assert!(rules.is_ignored(&rel("nested/app/out.log"), false));
        assert!(!rules.is_ignored(&rel("app/main.rs"), false));
    }

    #[test]
    fn gitignore_anchored_patterns_bind_to_root() {
        let rules = IgnoreRules::from_gitignore("/generated\n");
        assert!(rules.is_ignored(&rel("generated"), true));
        assert!(!rules.is_ignored(&rel("nested/generated"), true));
    }

    #[test]
    fn gitignore_negation_reincludes_files() {
        let rules = IgnoreRules::from_gitignore("*.rs\n!important.rs\n");
        assert!(rules.is_ignored(&rel("src/lib.rs"), false));
        assert!(!rules.is_ignored(&rel("src/important.rs"), false));
    }

    #[test]
    fn gitignore_double_star_matches_across_directories() {
        let rules = IgnoreRules::from_gitignore("docs/**/drafts\n");
        assert!(rules.is_ignored(&rel("docs/x/drafts"), true));
    }

    #[test]
    fn builtin_rules_are_not_overridden_by_negation() {
        let rules = IgnoreRules::from_gitignore("!target\n");
        assert!(rules.is_ignored(&rel("target"), true));
    }
}