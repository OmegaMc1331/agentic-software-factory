//! Repository context engine for Agentic Software Factory.
//!
//! Builds a bounded, persisted index of a repository (paths, languages,
//! regex-extracted symbols), searches it with ripgrep (pure-Rust fallback),
//! and deterministically resolves the most relevant files, symbols, and
//! related tests for a task — all read-only with respect to the repository.
//! The rendered `REPOSITORY CONTEXT` section is injected into agent missions
//! by factory-core; the dashboard Task Inspector and the `factory dev context`
//! CLI surface the same resolution for inspection.
//!
//! ## Security
//!
//! The engine never executes repository code, never modifies repository files
//! (the only writable path is `<factory>/.factory/context/index.json`), never
//! follows symlinks, prunes the entire search space through ignore rules, and
//! caps every read (`MAX_INDEX_FILE_READ_BYTES`) and every output budget
//! (`ContextConfig.max_files` / `max_chars`).

pub mod config;
pub mod error;
pub mod ignore;
pub mod index;
pub mod rank;
pub mod render;
pub mod resolve;
pub mod search;
pub mod symbols;

pub use config::{
    ContextConfig, DEFAULT_CONTEXT_MAX_CHARS, DEFAULT_CONTEXT_MAX_FILES, MAX_CONTEXT_CHARS_LIMIT,
    MAX_CONTEXT_FILES_LIMIT, MAX_INDEX_ENTRIES,
};
pub use error::{ContextError, Result};
pub use ignore::IgnoreRules;
pub use index::{ContextIndex, IndexManifest, IndexSummary, IndexedFile, RefreshStats};
pub use rank::ContextRequest;
pub use render::render_repository_context;
pub use resolve::{
    ContextEngine, RelatedTest, ResolvedContext, SelectedFile, MAX_FILE_EXCERPT_CHARS,
};
pub use search::{MatchKind, SearchHit};
pub use symbols::{extract_symbols, language_for, Symbol};

/// The directory (relative to a repository root) where the persisted context
/// index lives.
pub const CONTEXT_STATE_DIR: &str = "context";
