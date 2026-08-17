use serde::{Deserialize, Serialize};

/// The kind of context an advisory or review task persists for downstream
/// tasks. The list stays compact; `content` carries the structured payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Research,
    Architecture,
    Analysis,
    Review,
    Verification,
    DocumentationContext,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Research => "research",
            ArtifactKind::Architecture => "architecture",
            ArtifactKind::Analysis => "analysis",
            ArtifactKind::Review => "review",
            ArtifactKind::Verification => "verification",
            ArtifactKind::DocumentationContext => "documentation_context",
        }
    }
}

impl std::str::FromStr for ArtifactKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "research" => Ok(ArtifactKind::Research),
            "architecture" => Ok(ArtifactKind::Architecture),
            "analysis" => Ok(ArtifactKind::Analysis),
            "review" => Ok(ArtifactKind::Review),
            "verification" => Ok(ArtifactKind::Verification),
            "documentation_context" => Ok(ArtifactKind::DocumentationContext),
            other => Err(format!("unknown artifact kind '{other}'")),
        }
    }
}

impl std::fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A persisted output of an advisory, verification, or review task.
///
/// The artifact is the durable interface between roles: a Researcher writes a
/// research artifact, an Architect writes an architecture artifact, and a
/// Worker consumes the relevant ones from its dependency ancestry. `content`
/// holds structured JSON (see the docs for each operation's output contract),
/// which keeps the model small without inventing a generic blob store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleArtifact {
    pub id: i64,
    pub run_id: i64,
    #[serde(default)]
    pub task_id: Option<i64>,
    #[serde(default)]
    pub attempt_id: Option<i64>,
    pub role: String,
    #[serde(default)]
    pub operation: Option<String>,
    pub kind: String,
    pub content: String,
    pub created_at: String,
}

/// The semantic operation a planned task performs. Operations are a compact,
/// Factory-owned enum; they are validated against the task role's execution
/// class so a Security Auditor cannot silently be used as an implementation
/// Worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOperation {
    /// The Planner itself: produces the task DAG. Never a scheduled task.
    Planning,
    /// Produces context consumed by later tasks (Researcher, Architect).
    Advisory,
    /// Makes repository changes in an isolated worktree (Worker).
    Implement,
    /// Adds or runs tests and reports results (Test Engineer).
    Verify,
    /// Evaluates existing evidence and output (Reviewer, Security Auditor).
    Review,
    /// Runs after required implementation and review work (Documentation Writer).
    PostProcess,
}

impl TaskOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskOperation::Planning => "planning",
            TaskOperation::Advisory => "advisory",
            TaskOperation::Implement => "implement",
            TaskOperation::Verify => "verify",
            TaskOperation::Review => "review",
            TaskOperation::PostProcess => "post_process",
        }
    }
}

impl std::str::FromStr for TaskOperation {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "planning" => Ok(TaskOperation::Planning),
            "advisory" => Ok(TaskOperation::Advisory),
            "implement" => Ok(TaskOperation::Implement),
            "verify" => Ok(TaskOperation::Verify),
            "review" => Ok(TaskOperation::Review),
            "post_process" => Ok(TaskOperation::PostProcess),
            other => Err(format!("unknown task operation '{other}'")),
        }
    }
}

impl std::fmt::Display for TaskOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
