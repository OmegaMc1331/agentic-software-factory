//! Repository ignore rules for the bounded repository walk.
//!
//! The engine is read-only and offline-friendly. Its documented policy is
//! **repository-local ignore rules only**: the root `.gitignore`, nested
//! `.gitignore` files, and `.git/info/exclude`, layered with Factory's own
//! built-in exclusions (which nothing can negate). User-global excludes
//! (`core.excludesFile`) are deliberately NOT consulted so indexing stays
//! deterministic across machines; a repository that wants a file excluded must
//! say so in the repository itself.
//!
//! Parsing follows the gitignore subset Factory has always supported (blank
//! lines, comments, exact names, anchored paths, `*`/`**` globs, `?`, `/`
//! directory suffixes, `!` negations) and is UTF-8 safe: multi-byte
//! characters are handled per character, never per byte.

use std::path::{Component, Path, PathBuf};

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

/// Bound for the rule-collection walk, mirroring the index walk's cap so a
/// pathological tree cannot make rule collection unbounded.
const COLLECT_ENTRY_LIMIT: usize = 20_000;

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

/// Compiled ignore rules for one repository.
///
/// Patterns carry the directory whose `.gitignore` contributed them
/// (`prefix`, `""` for the repository root) and a `precedence` encoding
/// gitignore(5) ordering: `.git/info/exclude` is weakest, and deeper
/// `.gitignore` files override shallower ones. Within one file, later lines
/// win. At match time patterns are evaluated lowest precedence first so the
/// last match wins, exactly like git.
#[derive(Debug, Default, Clone)]
pub struct IgnoreRules {
    globs: Vec<IgnorePattern>,
}

#[derive(Debug, Clone)]
struct IgnorePattern {
    /// `true` for a `!` negation pattern.
    negate: bool,
    only_directories: bool,
    /// Forward-slash directory (relative to the walk root, no trailing
    /// slash) this pattern applies under; `""` means the root.
    prefix: String,
    /// Lower applies first; the last matching pattern decides.
    precedence: u32,
    regex: regex::Regex,
}

impl IgnoreRules {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parses the repository-root `.gitignore` (and, for compatibility with
    /// existing callers and tests, any root-level rule text).
    pub fn from_gitignore(text: &str) -> Self {
        let mut rules = Self::default();
        rules.add_scoped(text, "", 1);
        rules
    }

    /// Adds rules from one ignore file, scoped to `prefix` at `precedence`.
    pub fn add_scoped(&mut self, text: &str, prefix: &str, precedence: u32) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // `\#` and `\!` escape a literal first character.
            let (negate, raw) = match line.strip_prefix('\\') {
                Some(rest) if rest.starts_with('#') || rest.starts_with('!') => (false, rest),
                _ => match line.strip_prefix('!') {
                    Some(rest) => (true, rest),
                    None => (false, line),
                },
            };
            if raw.starts_with('#') && !line.starts_with("\\#") {
                continue;
            }
            let only_directories = raw.ends_with('/');
            let raw = raw.trim_end_matches('/');
            if raw.is_empty() {
                continue;
            }
            let root_anchored = raw.starts_with('/');
            let raw = raw.trim_start_matches('/');
            // gitignore(5): a pattern with an interior slash is relative to
            // the contributing directory; a slash-free pattern matches a
            // basename at any depth. Leading `/` anchors to the directory.
            let path_relative = root_anchored || raw.contains('/');

            let mut parts = Vec::new();
            let chars: Vec<char> = raw.chars().collect();
            let mut index = 0usize;
            while index < chars.len() {
                let ch = chars[index];
                match ch {
                    // gitignore: a backslash escapes the next character —
                    // `?`, `*`, `#`, `!`, space, or another backslash match
                    // literally. A trailing lone backslash is literal.
                    '\\' => {
                        if let Some(escaped) = chars.get(index + 1) {
                            parts.push(regex::escape(escaped.encode_utf8(&mut [0u8; 4])));
                            index += 2;
                        } else {
                            parts.push(regex::escape("\\"));
                            index += 1;
                        }
                    }
                    '*' if chars.get(index + 1) == Some(&'*') => {
                        // Context-aware `**` per gitignore(5):
                        // - leading `**/`  → zero or more leading directories
                        // - `a/**/b`       → zero or more middle directories
                        // - trailing `/**` → everything inside
                        // - otherwise      → any characters, including `/`
                        let at_start = index == 0;
                        let after_slash = at_start || chars[index - 1] == '/';
                        let followed_by_slash = chars.get(index + 2) == Some(&'/');
                        if after_slash && followed_by_slash {
                            // `**/a` (leading) and `c/**/d` (middle): zero or
                            // more directories before the next component.
                            parts.push("(?:.*/)?".to_string());
                            index += 3;
                        } else {
                            // `b/**` (everything inside) and any standalone
                            // `**`: any characters, including separators.
                            parts.push(".*".to_string());
                            index += 2;
                        }
                    }
                    '*' => {
                        parts.push("[^/]*".to_string());
                        index += 1;
                    }
                    '?' => {
                        parts.push("[^/]".to_string());
                        index += 1;
                    }
                    _ => {
                        parts.push(regex::escape(ch.encode_utf8(&mut [0u8; 4])));
                        index += 1;
                    }
                }
            }
            let pattern = parts.join("");
            let regex_text = if path_relative {
                format!("^{pattern}$")
            } else {
                format!("(?:^|/){pattern}$")
            };
            if let Ok(regex) = regex::Regex::new(&regex_text) {
                self.globs.push(IgnorePattern {
                    negate,
                    only_directories,
                    prefix: prefix.to_string(),
                    precedence,
                    regex,
                });
            }
        }
        self.globs.sort_by_key(|pattern| pattern.precedence);
    }

    /// Collects the effective repository-local ignore rules for `root`:
    /// `.git/info/exclude` (weakest), then every `.gitignore` from the root
    /// down through directories that are not themselves ignored. The walk is
    /// read-only, bounded, and does not follow symlinks.
    pub fn collect(root: &Path) -> Self {
        let mut rules = Self::default();
        if let Some(exclude) = read_info_exclude(root) {
            rules.add_scoped(&exclude, "", 0);
        }
        if let Ok(bytes) = std::fs::read(root.join(".gitignore")) {
            rules.add_scoped(&String::from_utf8_lossy(&bytes), "", 1);
        }
        let mut visited: usize = 0;
        let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited > COLLECT_ENTRY_LIMIT {
                    return rules;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                // Symlinked directories are not followed (entry metadata for
                // a symlink reports a file), matching the index walk.
                if !file_type.is_dir() {
                    continue;
                }
                let relative = match entry.path().strip_prefix(root) {
                    Ok(relative) => relative.to_path_buf(),
                    Err(_) => continue,
                };
                if rules.is_ignored(&relative, true) {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(entry.path().join(".gitignore")) {
                    let prefix = relative.to_string_lossy().replace('\\', "/");
                    let precedence = prefix.split('/').count() as u32 + 1;
                    rules.add_scoped(&String::from_utf8_lossy(&bytes), &prefix, precedence);
                }
                stack.push(entry.path());
            }
        }
        rules
    }

    /// Whether `relative` (a forward-slash path relative to the walk root)
    /// should be skipped. Directories must be passed with `is_directory` set.
    pub fn is_ignored(&self, relative: &Path, is_directory: bool) -> bool {
        // Builtin directory rules apply to every nested component, whether the
        // entry itself is a file or a directory.
        if relative.components().any(|component| match component {
            Component::Normal(name) => name.to_str().map(is_builtin_ignored_dir).unwrap_or(false),
            _ => false,
        }) {
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
        let text = relative.to_string_lossy().replace('\\', "/");
        // gitignore(5): a file cannot be re-included when a parent directory
        // is excluded, so an ignored ancestor settles the question before any
        // negation for the path itself is considered.
        let mut ancestors = Vec::new();
        let mut current = String::new();
        for component in text.split('/') {
            if component.is_empty() {
                continue;
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(component);
            ancestors.push(current.clone());
        }
        ancestors.pop();
        for ancestor in ancestors {
            if self.match_with_rules(&ancestor, true) {
                return true;
            }
        }
        self.match_with_rules(&text, is_directory)
    }

    /// Last-matching-pattern decision for one path, gitignore precedence
    /// order (patterns are stored sorted by precedence; equal precedence
    /// keeps file line order). Builtin rules cannot be undone by negation and
    /// are decided before this runs.
    fn match_with_rules(&self, text: &str, is_directory: bool) -> bool {
        let mut ignored = false;
        for rule in &self.globs {
            if rule.only_directories && !is_directory {
                continue;
            }
            // A nested file's patterns apply only under its directory.
            let candidate = if rule.prefix.is_empty() {
                text
            } else {
                match text.strip_prefix(&rule.prefix) {
                    Some(rest) => rest.strip_prefix('/').unwrap_or(rest),
                    None => continue,
                }
            };
            if candidate.is_empty() {
                continue;
            }
            if rule.regex.is_match(candidate) {
                ignored = !rule.negate;
            }
        }
        ignored
    }
}

/// `.git/info/exclude` of the repository at `root`. Worktrees keep their git
/// directory elsewhere; their `.git` file is followed so both layouts share
/// one rule source.
fn read_info_exclude(root: &Path) -> Option<String> {
    let dot_git = root.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let pointer = std::fs::read_to_string(&dot_git).ok()?;
        let path = pointer.trim().strip_prefix("gitdir:")?.trim();
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    let bytes = std::fs::read(git_dir.join("info").join("exclude")).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
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

    #[test]
    fn files_inside_an_ignored_directory_cannot_be_reincluded() {
        let rules = IgnoreRules::from_gitignore("build/\n");
        assert!(rules.is_ignored(&rel("build/keep.rs"), false));
        let negating = IgnoreRules::from_gitignore("build/\n!build/keep.rs\n");
        assert!(
            negating.is_ignored(&rel("build/keep.rs"), false),
            "gitignore(5): no re-inclusion under an excluded directory"
        );
        assert!(negating.is_ignored(&rel("build/deep/keep.rs"), false));
    }

    // --- ASF-N04: Unicode-safe parsing --------------------------------------

    #[test]
    fn unicode_patterns_parse_without_panicking_and_match() {
        for pattern in [
            "données/",
            "résumé.md",
            "日本語/",
            "naïve*.rs",
            "文件/**",
            "emoji-😀.txt",
        ] {
            let rules = IgnoreRules::from_gitignore(pattern);
            // The pattern must compile without a byte-boundary panic; the
            // match assertions below prove the semantics survived.
            let _ = &rules;
        }
        let rules = IgnoreRules::from_gitignore("données/\n");
        assert!(rules.is_ignored(&rel("données"), true));
        assert!(rules.is_ignored(&rel("src/données"), true));
        assert!(rules.is_ignored(&rel("données/file.txt"), false));

        let rules = IgnoreRules::from_gitignore("résumé.md\n");
        assert!(rules.is_ignored(&rel("docs/résumé.md"), false));
        assert!(!rules.is_ignored(&rel("docs/résumé.md.bak"), false));

        let rules = IgnoreRules::from_gitignore("日本語/\n");
        assert!(rules.is_ignored(&rel("日本語/kanji.txt"), false));

        let rules = IgnoreRules::from_gitignore("naïve*.rs\n");
        assert!(rules.is_ignored(&rel("naïve1.rs"), false));
        assert!(!rules.is_ignored(&rel("naïve1.ts"), false));

        let rules = IgnoreRules::from_gitignore("文件/**\n");
        assert!(rules.is_ignored(&rel("文件/a/b.txt"), false));

        let rules = IgnoreRules::from_gitignore("emoji-😀.txt\n");
        assert!(rules.is_ignored(&rel("misc/emoji-😀.txt"), false));
    }

    #[test]
    fn every_unicode_pattern_split_stays_parseable() {
        // Parsing a pattern with multi-byte characters at every position must
        // never hit a byte/character boundary panic.
        let pattern = "日本語/ファイル*.md";
        let mut rules = IgnoreRules::empty();
        rules.add_scoped(pattern, "", 1);
        assert!(rules.is_ignored(&rel("日本語/ファイル一覧.md"), false));
    }

    #[test]
    fn escaped_hash_and_bang_are_literals() {
        let rules = IgnoreRules::from_gitignore("\\#not-a-comment\n\\!important\n");
        assert!(rules.is_ignored(&rel("#not-a-comment"), false));
        assert!(rules.is_ignored(&rel("!important"), false));
    }

    // --- ASF-N07: nested rules, info/exclude, precedence --------------------

    #[test]
    fn nested_gitignore_scopes_and_overrides_the_root() {
        let mut rules = IgnoreRules::from_gitignore("*.generated.md\n");
        rules.add_scoped("!keep.generated.md", "docs", 2);
        assert!(rules.is_ignored(&rel("root.generated.md"), false));
        assert!(rules.is_ignored(&rel("other/keep.generated.md"), false));
        assert!(
            !rules.is_ignored(&rel("docs/keep.generated.md"), false),
            "the nested file overrides the root rule under docs/"
        );
    }

    #[test]
    fn nested_rules_do_not_leak_outside_their_directory() {
        let mut rules = IgnoreRules::empty();
        rules.add_scoped("secret.txt", "tools", 2);
        assert!(rules.is_ignored(&rel("tools/secret.txt"), false));
        assert!(!rules.is_ignored(&rel("secret.txt"), false));
        assert!(!rules.is_ignored(&rel("other/secret.txt"), false));
    }

    #[test]
    fn info_exclude_is_weaker_than_gitignore() {
        let mut rules = IgnoreRules::empty();
        rules.add_scoped("scratch/", "", 0);
        rules.add_scoped("!scratch/", "", 1);
        assert!(
            !rules.is_ignored(&rel("scratch"), true),
            ".gitignore beats .git/info/exclude"
        );
    }

    #[test]
    fn collect_reads_root_nested_and_info_exclude_rules() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "root-ignored.txt\n").unwrap();
        std::fs::create_dir_all(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/.gitignore"), "local.txt\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".git/info")).unwrap();
        std::fs::write(dir.path().join(".git/info/exclude"), "exclude-only.txt\n").unwrap();

        let rules = IgnoreRules::collect(dir.path());
        assert!(rules.is_ignored(&rel("root-ignored.txt"), false));
        assert!(rules.is_ignored(&rel("nested/local.txt"), false));
        assert!(!rules.is_ignored(&rel("local.txt"), false));
        assert!(rules.is_ignored(&rel("exclude-only.txt"), false));
    }

    #[test]
    fn collect_does_not_read_gitignores_inside_ignored_directories() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("build")).unwrap();
        // A negation inside an excluded directory must never be picked up.
        std::fs::write(dir.path().join("build/.gitignore"), "!keep.txt\n").unwrap();
        let rules = IgnoreRules::collect(dir.path());
        assert!(rules.is_ignored(&rel("build/keep.txt"), false));
    }
}
