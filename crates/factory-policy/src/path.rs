//! Repository-relative path normalization and small glob matching.
//!
//! Policy scopes are repository-relative patterns (`src/**`, `README.md`,
//! `docs/**.md`). This module normalizes evidence paths so a scope check can
//! never be reached through absolute paths, drive letters, or `..` traversal,
//! and provides the small glob matcher used for allow/deny scopes.

/// Normalizes an evidence path (as reported by git) into a canonical
/// repository-relative path.
///
/// Returns `None` when the path cannot be mapped into the repository — it is
/// absolute, uses a drive prefix, contains `..` traversal, or holds control
/// characters. Those paths can never match a scope and are treated as outside
/// the repository.
pub fn normalize_repo_relative(path: &str) -> Option<String> {
    let clean = path.replace('\\', "/");
    if contains_control(&clean) {
        return None;
    }
    if clean.starts_with('/') || is_drive_prefixed(&clean) {
        return None;
    }
    let mut components: Vec<&str> = Vec::new();
    for component in clean.split('/') {
        match component {
            "." | "" => continue,
            ".." => return None,
            other => components.push(other),
        }
    }
    if components.is_empty() {
        return None;
    }
    Some(components.join("/"))
}

fn is_drive_prefixed(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn contains_control(path: &str) -> bool {
    path.chars()
        .any(|c| c.is_control() || matches!(c, '\n' | '\r' | '\0'))
}

/// Whether a pattern string is a valid repository-relative scope. Rejects
/// absolute paths, traversal, and empty patterns.
pub fn validate_scope(pattern: &str) -> Result<(), String> {
    if pattern.trim().is_empty() {
        return Err("scope pattern must not be empty".into());
    }
    if normalize_repo_relative(pattern.trim_start_matches("./")).is_none() {
        return Err(format!(
            "scope pattern '{pattern}' must be repository-relative"
        ));
    }
    if pattern.contains("..") {
        return Err(format!(
            "scope pattern '{pattern}' must not traverse directories"
        ));
    }
    Ok(())
}

/// Matches a normalized repository-relative path against a glob pattern.
///
/// Supported syntax:
/// - `**` matches any number of components (including none)
/// - `*` matches within a single component (not `/`)
/// - `?` matches a single character within a component
/// - everything else matches literally (`.` matches itself, unlike shell globs)
///
/// `dir/**` also matches `dir` itself (a directory created by the agent).
pub fn matches_glob(pattern: &str, path_dir: &str, case_sensitive: bool) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path_dir.split('/').collect();
    match_components(&pat, &path, case_sensitive)
}

fn match_components(pattern: &[&str], path: &[&str], case_sensitive: bool) -> bool {
    // `**` consumes zero or more components.
    if let Some((head, rest)) = pattern.split_first() {
        if *head == "**" {
            // Try consuming zero components, then one at a time.
            let mut consumed = 0;
            loop {
                if match_components(rest, &path[consumed..], case_sensitive) {
                    return true;
                }
                consumed += 1;
                if consumed > path.len() {
                    return false;
                }
            }
        }
    }
    match (pattern.split_first(), path.split_first()) {
        (Some((pat_head, pat_rest)), Some((path_head, path_rest)))
            if match_component(pat_head, path_head, case_sensitive) =>
        {
            match_components(pat_rest, path_rest, case_sensitive)
        }
        _ => pattern.is_empty() && path.is_empty(),
    }
}

fn match_component(pattern: &str, text: &str, case_sensitive: bool) -> bool {
    if pattern == "*" {
        return true;
    }
    let pattern: Vec<char> = chars(pattern, case_sensitive);
    let text: Vec<char> = chars(text, case_sensitive);
    component_match(&pattern, &text)
}

fn component_match(pattern: &[char], text: &[char]) -> bool {
    // Simple recursive wildcard matching over one component.
    match (pattern, text) {
        ([], []) => true,
        ([], _) => false,
        (['*'], _) => true,
        ([p, pat_rest @ ..], [t, text_rest @ ..]) if *p == '*' => {
            component_match(pattern, text_rest) || component_match(pat_rest, text)
        }
        ([p, pat_rest @ ..], [t, text_rest @ ..]) if *p == '?' || p == t => {
            component_match(pat_rest, text_rest)
        }
        _ => false,
    }
}

fn chars(value: &str, case_sensitive: bool) -> Vec<char> {
    if case_sensitive {
        value.chars().collect()
    } else {
        value.to_lowercase().chars().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_repo_relative_paths() {
        assert_eq!(
            normalize_repo_relative("src/main.rs"),
            Some("src/main.rs".into())
        );
        assert_eq!(
            normalize_repo_relative(".\\src\\main.rs"),
            Some("src/main.rs".into())
        );
        assert_eq!(
            normalize_repo_relative("./src//main.rs"),
            Some("src/main.rs".into())
        );
        assert_eq!(
            normalize_repo_relative("src/./main.rs"),
            Some("src/main.rs".into())
        );
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert_eq!(normalize_repo_relative("../outside"), None);
        assert_eq!(normalize_repo_relative("src/../../outside"), None);
        assert_eq!(normalize_repo_relative("/absolute/path"), None);
        assert_eq!(normalize_repo_relative("C:/absolute"), None);
        assert_eq!(normalize_repo_relative("C:\\absolute"), None);
        assert_eq!(normalize_repo_relative("..\\outside"), None);
        assert_eq!(
            normalize_repo_relative("src/\u{0}bad"),
            None,
            "control characters are rejected"
        );
    }

    #[test]
    fn glob_mathches_scopes() {
        let case = true;
        assert!(matches_glob("**", "src/main.rs", case));
        assert!(matches_glob("src/**", "src/main.rs", case));
        assert!(matches_glob("src/**", "src/deep/mod.rs", case));
        assert!(matches_glob("README.md", "README.md", case));
        assert!(!matches_glob("README.md", "docs/README.md", case));
        assert!(matches_glob("docs/**", "docs/guide.md", case));
        assert!(!matches_glob("docs/**", "README.md", case));
        assert!(matches_glob("**/test_*.rs", "tests/test_a.rs", case));
        assert!(
            !matches_glob("*.rs", "src/main.rs", case),
            "single * stops at /"
        );
        assert!(!matches_glob("src/*.rs", "src/deep/mod.rs", case));
        // A directory matched by `dir/**` (agent created the directory).
        assert!(matches_glob("docs/**", "docs", case));
        assert!(!matches_glob("docs/**", "docs2", case));
        assert!(
            !matches_glob("src/**", "src2/main.rs", case),
            "component-wise prefix"
        );
    }

    #[test]
    fn glob_case_insensitivity_on_windows_semantics() {
        assert!(matches_glob("README.md", "readme.md", false));
        assert!(!matches_glob("README.md", "readme.md", true));
    }

    #[test]
    fn scope_validation_rejects_unsafe_patterns() {
        assert!(validate_scope("src/**").is_ok());
        assert!(validate_scope("../etc/passwd").is_err());
        assert!(validate_scope("/etc/passwd").is_err());
        assert!(validate_scope("C:/Windows").is_err());
        assert!(validate_scope("").is_err());
    }

    #[test]
    fn escaped_backslash_is_treated_as_normalization() {
        // Windows-style evidence path maps into the repo scope.
        assert!(matches_glob(
            "src/**",
            normalize_repo_relative("src\\main.rs").unwrap().as_str(),
            true
        ));
    }
}
