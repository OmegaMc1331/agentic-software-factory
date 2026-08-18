use serde::{Deserialize, Serialize};

use crate::error::{ContextError, Result};

/// Budget defaults for the resolved `REPOSITORY CONTEXT` section.
pub const DEFAULT_CONTEXT_MAX_FILES: usize = 12;
pub const MAX_CONTEXT_FILES_LIMIT: usize = 200;
pub const DEFAULT_CONTEXT_MAX_CHARS: usize = 40_000;
pub const MAX_CONTEXT_CHARS_LIMIT: usize = 500_000;
/// Hard cap on the number of files the bounded index may hold. Keeps the
/// stat-based refresh cheap and prevents pathological repositories from
/// dominating a mission.
pub const MAX_INDEX_ENTRIES: usize = 50_000;

fn default_enabled() -> bool {
    true
}
fn default_max_files() -> usize {
    DEFAULT_CONTEXT_MAX_FILES
}
fn default_max_chars() -> usize {
    DEFAULT_CONTEXT_MAX_CHARS
}
fn default_include_tests() -> bool {
    true
}
fn default_include_symbols() -> bool {
    true
}

/// Tunables for the repository context engine, stored under `[context]` in
/// `.factory/config.toml`. Backward compatible: absent in older configs, so it
/// defaults to the budget defaults above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Whether the engine runs at all. When disabled the mission contains no
    /// `REPOSITORY CONTEXT` section and the dashboard/CLI inspect routes
    /// report the engine as disabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Maximum number of files included in a resolved task context.
    #[serde(default = "default_max_files")]
    pub max_files: usize,
    /// Maximum characters of the rendered repository context section.
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    /// Whether related test files are discovered and rendered.
    #[serde(default = "default_include_tests")]
    pub include_tests: bool,
    /// Whether per-file symbol lists are extracted and rendered.
    #[serde(default = "default_include_symbols")]
    pub include_symbols: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_files: default_max_files(),
            max_chars: default_max_chars(),
            include_tests: default_include_tests(),
            include_symbols: default_include_symbols(),
        }
    }
}

impl ContextConfig {
    /// Normalizes out-of-range values into valid bounds, mirroring how the
    /// runtime scheduler clamps `max_parallel_tasks`. Never fails; bound
    /// enforcement is defense-in-depth at resolve time.
    pub fn normalize(&mut self) {
        self.max_files = self.max_files.clamp(1, MAX_CONTEXT_FILES_LIMIT);
        self.max_chars = self.max_chars.clamp(1_000, MAX_CONTEXT_CHARS_LIMIT);
    }

    pub fn validate(&self) -> Result<()> {
        if self.max_files == 0 {
            return Err(ContextError::Config(
                "context.max_files must be at least 1".into(),
            ));
        }
        if self.max_chars < 100 {
            return Err(ContextError::Config(
                "context.max_chars must be at least 100".into(),
            ));
        }
        Ok(())
    }
}
