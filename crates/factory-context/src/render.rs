//! Renders a resolved context into the `REPOSITORY CONTEXT` mission section.
//!
//! Character budget is enforced as a hard cap with a visible truncation
//! marker, and output is deterministic (stable ordering, fixed caps on symbols
//! and reasons per file).

use crate::resolve::{RelatedTest, ResolvedContext, SelectedFile};

/// Per-file limits that keep the section readable even before the total budget
/// kicks in.
const MAX_RENDERED_SYMBOLS_PER_FILE: usize = 6;
const MAX_REASONS_PER_FILE: usize = 3;

/// The mission section header. `build_mission` in factory-core emits its own
/// section labels; this text is inserted verbatim beneath its label.
pub fn render_repository_context(resolved: &ResolvedContext) -> String {
    if !resolved.enabled || resolved.selected.is_empty() {
        return String::new();
    }
    let budget_chars = resolved.budget_chars.max(100);
    let mut bits: Vec<String> = Vec::new();

    bits.push(format!("Scope: {}", resolved.scope_dir.display()));
    if resolved.is_worktree {
        bits.push(format!(
            "Worktree: branch {}, HEAD {}",
            resolved.branch.as_deref().unwrap_or("?"),
            resolved.head.as_deref().unwrap_or("?")
        ));
    } else if let Some(branch) = resolved.branch.as_ref() {
        bits.push(format!("Branch: {branch}"));
    }
    if let Some(head) = resolved.head.as_ref() {
        bits.push(format!("HEAD: {head}"));
    }
    let mut detail = format!(
        "Relevant files ({} of {} candidates; budget {} files / {} chars):",
        resolved.selected.len(),
        resolved.candidates_considered,
        resolved.budget_files,
        resolved.budget_chars,
    );
    if resolved.oversize {
        detail.push_str(" [index truncated]");
    }
    bits.push(detail);
    for file in &resolved.selected {
        bits.push(file_line(file));
    }

    if !resolved.related_tests.is_empty() {
        bits.push("Related tests:".to_string());
        for test in &resolved.related_tests {
            bits.push(test_line(test));
        }
    }

    if let Some(first) = resolved.selected.first() {
        bits.push(format!("Excerpt of {}:", first.path));
        bits.push(first.excerpt.trim_end().to_string());
    }

    let mut out = String::new();
    for bit in &bits {
        out.push_str(bit);
        out.push('\n');
    }
    if out.chars().count() > budget_chars {
        out = out.chars().take(budget_chars).collect();
        out.push_str(&format!(
            "…[repository context truncated to {budget_chars} chars]"
        ));
        out.push('\n');
    }
    out.trim_end().to_string() + "\n"
}

fn file_line(file: &SelectedFile) -> String {
    let mut line = format!("- {}", file.path);
    if let Some(language) = file.language.as_ref() {
        line.push_str(&format!(" [{language}]"));
    }
    if !file.symbols.is_empty() {
        let mut symbols = file
            .symbols
            .iter()
            .take(MAX_RENDERED_SYMBOLS_PER_FILE)
            .map(|symbol| format!("{}@{}", symbol.name, symbol.line))
            .collect::<Vec<_>>();
        if file.symbols.len() > MAX_RENDERED_SYMBOLS_PER_FILE {
            symbols.push("…".to_string());
        }
        line.push_str(&format!(" — symbols: {}", symbols.join(", ")));
    }
    if !file.reasons.is_empty() {
        let reasons = file
            .reasons
            .iter()
            .take(MAX_REASONS_PER_FILE)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        line.push_str(&format!(" — why: {reasons}"));
    }
    line
}

fn test_line(test: &RelatedTest) -> String {
    match test.for_target.as_ref() {
        Some(target) => format!("- {} (linked to {})", test.path, target),
        None => format!("- {}", test.path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::Symbol;

    fn file(path: &str, symbols: Vec<Symbol>) -> SelectedFile {
        SelectedFile {
            path: path.into(),
            language: Some("rust".into()),
            symbols,
            excerpt: "pub fn authenticate() {}\n".into(),
            reasons: vec!["path matches task term 'auth'".into()],
            score: 100,
        }
    }

    fn resolved() -> ResolvedContext {
        ResolvedContext {
            enabled: true,
            scope_dir: "/repo".into(),
            is_worktree: true,
            branch: Some("factory/t1".into()),
            head: Some("abc123".into()),
            base_sha: None,
            candidates_considered: 40,
            budget_files: 4,
            budget_chars: 40_000,
            oversize: false,
            selected: vec![file(
                "src/auth.rs",
                vec![Symbol {
                    name: "authenticate".into(),
                    kind: "function".into(),
                    line: 7,
                }],
            )],
            related_tests: vec![RelatedTest {
                path: "tests/auth_test.rs".into(),
                for_target: Some("src/auth.rs".into()),
            }],
        }
    }

    #[test]
    fn renders_the_section_with_git_and_why() {
        let text = render_repository_context(&resolved());
        assert!(text.contains("Worktree: branch factory/t1, HEAD abc123"));
        assert!(text.contains("src/auth.rs [rust]"));
        assert!(text.contains("authenticate@7"));
        assert!(text.contains("path matches task term 'auth'"));
        assert!(text.contains("tests/auth_test.rs (linked to src/auth.rs)"));
    }

    #[test]
    fn disabled_or_empty_context_renders_nothing() {
        let mut resolved = resolved();
        resolved.enabled = false;
        assert_eq!(render_repository_context(&resolved), "");
        resolved.enabled = true;
        resolved.selected.clear();
        assert_eq!(render_repository_context(&resolved), "");
    }

    #[test]
    fn budget_truncates_with_marker() {
        let mut resolved = resolved();
        resolved.budget_chars = 120;
        let text = render_repository_context(&resolved);
        assert!(text.contains("truncated to 120 chars"));
        assert!(text.chars().count() <= 200);
        assert!(text.starts_with("Scope:"));
    }

    #[test]
    fn output_is_deterministic() {
        let a = render_repository_context(&resolved());
        let b = render_repository_context(&resolved());
        assert_eq!(a, b);
    }
}
