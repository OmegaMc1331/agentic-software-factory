use std::collections::BTreeSet;

/// Deterministic file-extension → language mapping used to label tasks by the
/// languages they actually touched. No LLM classification is involved, tasks
/// can be multi-language, and tasks whose changed files match no known
/// extension get no label at all (never a forced guess).
///
/// Keys are stable lowercase identifiers (`rust`, `typescript`, `cpp`, ...);
/// the dashboard maps them to display names. Pure configuration formats
/// (toml/yaml/json/lockfiles) carry no language signal and are ignored.
fn language_for_extension(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" | "pyi" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "scala" => "scala",
        "m" | "mm" => "objective_c",
        "sh" | "bash" | "zsh" => "shell",
        "ps1" => "powershell",
        "sql" => "sql",
        "vue" => "vue",
        "svelte" => "svelte",
        "html" | "htm" => "html",
        "css" | "scss" | "sass" => "css",
        "md" | "mdx" => "markdown",
        _ => return None,
    })
}

/// The set of languages a list of changed files evidences. Paths are taken as
/// recorded in `TaskEvidence.changed_files`; only the final extension is
/// considered and lockfiles/config files never produce a label.
pub fn detect_languages(changed_files: &[String]) -> BTreeSet<String> {
    changed_files
        .iter()
        .filter_map(|path| {
            let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
            if file.starts_with('.') {
                // dotfiles such as `.gitignore` carry no language
                return None;
            }
            let extension = file.rsplit_once('.')?.1.to_ascii_lowercase();
            language_for_extension(&extension).map(str::to_string)
        })
        .collect()
}

/// Display name for a language key, used by API consumers that do not define
/// their own label table.
pub fn language_label(key: &str) -> &str {
    match key {
        "rust" => "Rust",
        "typescript" => "TypeScript",
        "javascript" => "JavaScript",
        "python" => "Python",
        "go" => "Go",
        "java" => "Java",
        "kotlin" => "Kotlin",
        "swift" => "Swift",
        "c" => "C",
        "cpp" => "C++",
        "csharp" => "C#",
        "ruby" => "Ruby",
        "php" => "PHP",
        "scala" => "Scala",
        "objective_c" => "Objective-C",
        "shell" => "Shell",
        "powershell" => "PowerShell",
        "sql" => "SQL",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "html" => "HTML",
        "css" => "CSS",
        "markdown" => "Markdown",
        other => other,
    }
}
