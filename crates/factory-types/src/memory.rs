use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Architecture,
    TechnicalDecisions,
    CodingConventions,
    ProjectConstraints,
    Dependencies,
    KnownIssues,
    PreviousFailures,
    ProjectPreferences,
    GeneralFacts,
}

impl MemoryCategory {
    pub const ALL: [Self; 9] = [
        Self::Architecture,
        Self::TechnicalDecisions,
        Self::CodingConventions,
        Self::ProjectConstraints,
        Self::Dependencies,
        Self::KnownIssues,
        Self::PreviousFailures,
        Self::ProjectPreferences,
        Self::GeneralFacts,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::TechnicalDecisions => "technical_decisions",
            Self::CodingConventions => "coding_conventions",
            Self::ProjectConstraints => "project_constraints",
            Self::Dependencies => "dependencies",
            Self::KnownIssues => "known_issues",
            Self::PreviousFailures => "previous_failures",
            Self::ProjectPreferences => "project_preferences",
            Self::GeneralFacts => "general_facts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Disabled,
    Stale,
    Disputed,
    Superseded,
}
impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Stale => "stale",
            Self::Disputed => "disputed",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOrigin {
    UserAuthored,
    UserEdited,
    RepositoryDerived,
    ConversationDerived,
    MissionDerived,
    ReviewDerived,
    Proposed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySource {
    pub project_id: String,
    pub reference: MemorySourceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MemorySourceRef {
    Manual {
        note: String,
    },
    RepositoryFile {
        root_id: String,
        path: String,
        revision: Option<String>,
    },
    ConversationMessage {
        conversation_id: String,
        message_id: i64,
    },
    Mission {
        run_id: i64,
    },
    TaskAttempt {
        attempt_id: i64,
    },
    AgentSession {
        session_id: i64,
    },
    Review {
        attempt_id: i64,
    },
    TestEvidence {
        attempt_id: i64,
    },
    IntegrationCommit {
        run_id: i64,
        commit_sha: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProvenance {
    pub source: MemorySource,
    pub source_revision: String,
    pub fingerprint: String,
    pub validation_status: String,
    pub checked_at: String,
    #[serde(default)]
    pub evidence_excerpt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryInput {
    pub category: MemoryCategory,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub scope: String,
    pub status: MemoryStatus,
    pub sources: Vec<MemorySource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemory {
    pub schema_version: u32,
    pub id: String,
    pub project_id: String,
    pub revision: i64,
    pub category: MemoryCategory,
    pub title: String,
    pub content: String,
    pub subject: String,
    pub scope: String,
    pub status: MemoryStatus,
    pub origin: MemoryOrigin,
    pub provenance: Vec<MemoryProvenance>,
    pub created_at: String,
    pub updated_at: String,
    pub change_reason: String,
    pub related_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMode {
    #[default]
    Disabled,
    Review,
    Automatic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySettings {
    pub revision: i64,
    pub enabled: bool,
    pub extraction_mode: ExtractionMode,
    pub categories: Vec<MemoryCategory>,
    pub retrieval_enabled: bool,
    pub max_context_records: usize,
    pub max_context_bytes: usize,
}
impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            revision: 0,
            enabled: true,
            extraction_mode: ExtractionMode::Disabled,
            categories: MemoryCategory::ALL.to_vec(),
            retrieval_enabled: true,
            max_context_records: 6,
            max_context_bytes: 12000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySearch {
    #[serde(default)]
    pub query: String,
    pub category: Option<MemoryCategory>,
    pub status: Option<MemoryStatus>,
    pub source_type: Option<String>,
    pub updated_after: Option<String>,
    pub before: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPage {
    pub items: Vec<ProjectMemory>,
    pub next_cursor: Option<i64>,
    pub search_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReference {
    pub memory_id: String,
    pub revision: i64,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryContext {
    pub schema_version: u32,
    pub project_id: String,
    pub generation: i64,
    pub settings_revision: i64,
    pub records: Vec<ProjectMemory>,
    pub truncated: bool,
    pub notice: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUsage {
    pub id: i64,
    pub project_id: String,
    pub agent_session_id: i64,
    pub role: String,
    pub settings_revision: i64,
    pub memory_generation: i64,
    pub memories: Vec<MemoryReference>,
    pub used_at: String,
    pub context_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCandidate {
    pub id: String,
    pub project_id: String,
    pub extraction_id: String,
    pub memory: ProjectMemory,
    pub state: String,
    pub issue: String,
    pub accepted_memory_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtraction {
    pub id: String,
    pub project_id: String,
    pub source_key: String,
    pub source_references: Vec<MemoryProvenance>,
    pub policy_snapshot: MemorySettings,
    pub state: String,
    pub agent_session_id: Option<i64>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
