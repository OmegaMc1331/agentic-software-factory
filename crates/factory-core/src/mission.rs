//! Role-aware mission builder.
//!
//! One place assembles the prompt for every task, parameterized by the role
//! definition, the task's semantic operation, the run objective, and the
//! consumed upstream artifacts. Operations declare their own output contract,
//! so a Researcher writes findings, a Worker reports commands, and a Security
//! Auditor returns a decision with findings — without per-role branching.

use factory_policy::EffectivePolicy;
use factory_types::{
    ReviewDecision, ReviewFinding, ReviewResult, RoleArtifact, Task, TaskEvidence, TaskOperation,
};
use serde::{Deserialize, Serialize};

use crate::roles::RoleDefinition;

/// Maximum characters of upstream artifact context injected into a mission.
/// Context above the cap is trimmed with a visible marker.
pub const MAX_UPSTREAM_CONTEXT_CHARS: usize = 32_000;
/// Maximum characters of the implementation diff given to a review role.
pub const MAX_REVIEW_DIFF_CHARS: usize = 60_000;
/// Maximum characters of the producer's raw output forwarded to a reviewer.
pub const MAX_PRODUCER_OUTPUT_CHARS: usize = 20_000;
/// Maximum characters of an individual artifact rendered into context.
pub const MAX_ARTIFACT_CHARS: usize = 12_000;

/// Everything the runtime knows about the task being executed.
#[derive(Debug, Clone)]
pub struct MissionContext<'a> {
    pub role: &'a RoleDefinition,
    pub operation: TaskOperation,
    pub task: &'a Task,
    pub run_objective: &'a str,
    pub upstream_artifacts: &'a [RoleArtifact],
    /// Rendered `REPOSITORY CONTEXT` section produced by the repository
    /// context engine, when enabled and non-empty. Absent (or empty) renders
    /// "none." so the agent knows the engine is active but found nothing.
    pub repository_context: Option<&'a str>,
    /// Feedback from the previous review attempt of this task, if any.
    pub previous_feedback: Option<&'a ReviewResult>,
    /// Evidence and diff of the implementation a review task evaluates.
    pub review_input: Option<&'a ReviewInput>,
    /// The task is reviewed by the built-in final Reviewer, which keeps its
    /// `{decision, reason, feedback}` contract instead of the findings shape
    /// used by specialized review tasks.
    pub final_review: bool,
    /// The effective policy applied to this session. Rendered as a
    /// `PERMISSIONS` section so agents see the enforced boundary; the mission
    /// text is guidance, the enforcement boundary is Factory Core.
    pub policy: Option<&'a EffectivePolicy>,
}

/// The implementation evidence a review (or final acceptance) task inspects.
#[derive(Debug, Clone)]
pub struct ReviewInput {
    pub producer_title: String,
    pub producer_role: String,
    pub evidence: TaskEvidence,
    pub producer_output: String,
    pub diff: String,
}

pub fn build_mission(context: &MissionContext<'_>) -> String {
    let mut mission = String::new();

    mission.push_str(&format!(
        "ROLE\n{} — {}\n\n{}\n",
        context.role.name,
        context.role.description,
        context.role.instructions.trim()
    ));

    mission.push_str(&format!(
        "\nWORKFLOW OBJECTIVE\n{}\n",
        context.run_objective.trim()
    ));
    mission.push_str(&format!(
        "\nTASK\n{} — {}\n",
        context.task.title.trim(),
        context.task.objective.trim()
    ));
    mission.push_str(&format!(
        "\nOPERATION\n{}\n{}",
        context.operation.as_str(),
        operation_guidance(context.operation)
    ));

    let operation_text = operation_output_contract(context.operation);
    mission.push_str("\n\nUPSTREAM CONTEXT\n");
    if context.upstream_artifacts.is_empty() {
        mission.push_str("none.");
    } else {
        mission.push_str(&render_upstream_context(context.upstream_artifacts));
    }

    mission.push_str("\n\nREPOSITORY CONTEXT\n");
    match context.repository_context {
        Some(rendered) if !rendered.trim().is_empty() => {
            mission.push_str(rendered.trim_end());
            mission.push('\n');
        }
        _ => mission.push_str("none."),
    }

    mission.push_str("\n\nACCEPTANCE CRITERIA\n");
    for criterion in &context.task.acceptance_criteria {
        mission.push_str(&format!("- {criterion}\n"));
    }
    mission.push_str(
        "The criteria above are the goal of this task. Evaluate your own work against them \
         before reporting.",
    );

    if let Some(review) = context.previous_feedback {
        mission.push_str("\n\nCONTEXT\nPrevious review requested changes:\n");
        mission.push_str(review.reason.trim());
        if !review.feedback.is_empty() {
            mission.push('\n');
            for item in &review.feedback {
                mission.push_str(&format!("- {item}\n"));
            }
        }
        mission.push_str("\nAddress this feedback; do not repeat the underlying mistake.");
    }

    if let Some(input) = context.review_input {
        mission.push_str(&render_review_input(input));
    }

    if let Some(policy) = context.policy {
        mission.push_str(&format!("\n\nPERMISSIONS\n{}", render_policy(policy)));
    }

    mission.push_str("\n\nOUTPUT CONTRACT\n");
    if context.final_review {
        mission.push_str(
            "Return one JSON object only: \
             {\"decision\": \"approve\"|\"request_changes\", \"reason\": string, \"feedback\": [string]}. \
             Approve only when the evidence and repository changes satisfy every criterion. Do not modify files.",
        );
    } else {
        mission.push_str(operation_text);
    }

    mission
}

fn operation_guidance(operation: TaskOperation) -> &'static str {
    match operation {
        TaskOperation::Planning => "You produce the plan; this is only meaningful for the Planner.",
        TaskOperation::Advisory => {
            "This is an advisory step: you produce context another task consumes. \
             You normally do not change production files. Inspect the repository and \
             dependencies with the capabilities you already own, then report findings."
        }
        TaskOperation::Implement => {
            "You make the repository changes the task requires in the current worktree. \
             You may commit your work in this worktree; if you leave work uncommitted it is \
             committed for you and integrated into the run branch automatically after approval."
        }
        TaskOperation::Verify => {
            "You add or run the tests and checks that make this task's requirements \
             verifiable, then report what ran and what passed."
        }
        TaskOperation::Review => {
            "You evaluate existing work: inspect the change, the evidence, and the \
             upstream context, then decide approve or request_changes with concrete \
             findings. You do not modify implementation files."
        }
        TaskOperation::PostProcess => {
            "You run after the required implementation and review work. Produce or \
             update documentation for what was actually implemented; do not document \
             behavior that does not exist."
        }
    }
}

fn operation_output_contract(operation: TaskOperation) -> &'static str {
    match operation {
        TaskOperation::Planning => "Return the plan JSON described in the system prompt.",
        TaskOperation::Advisory => {
            "Return one JSON object only: \
             {\"summary\": string, \"findings\": [string], \"recommendations\": [string]}. \
             Keep findings concise and evidence-backed. This output is persisted as an \
             artifact that later tasks consume."
        }
        TaskOperation::Implement | TaskOperation::PostProcess => {
            "Return one JSON object only: {\"summary\": string, \"commands\": [string]}. \
             List the commands or checks you ran."
        }
        TaskOperation::Verify => {
            "Return one JSON object only: \
             {\"summary\": string, \"commands\": [string], \"results\": [string]}. \
             Report the test commands and their results exactly as observed; do not \
             invent coverage or pass/fail outcomes."
        }
        TaskOperation::Review => {
            "Return one JSON object only: \
             {\"decision\": \"approve\"|\"request_changes\", \
             \"findings\": [{\"severity\": \"low\"|\"medium\"|\"high\"|\"critical\", \
             \"summary\": string, \"evidence\": string}]}. \
             Approve only when the evidence and repository changes satisfy the task."
        }
    }
}

fn render_upstream_context(artifacts: &[RoleArtifact]) -> String {
    let mut out = String::new();
    let mut shown = 0usize;
    let mut truncated = false;
    for artifact in artifacts {
        if shown >= MAX_UPSTREAM_CONTEXT_CHARS {
            truncated = true;
            break;
        }
        let mut block = format!(
            "\n### {} artifact from task #{} ({})\n",
            artifact.kind,
            artifact.task_id.unwrap_or_default(),
            artifact.role
        );
        let content: String = artifact.content.chars().take(MAX_ARTIFACT_CHARS).collect();
        if content.chars().count() < artifact.content.chars().count() {
            block.push_str(&content);
            block.push_str("\n…[this artifact was truncated]");
        } else {
            block.push_str(&content);
        }
        shown += block.len();
        out.push_str(&block);
        out.push('\n');
    }
    if truncated {
        out.push_str("\n…[upstream context truncated; only the artifacts above were provided]\n");
    }
    out
}

/// Renders the effective policy attached to a session. This is the same
/// resolved policy used by execution and validation; the text below is explicit
/// about what Factory enforces versus what is advisory.
fn render_policy(policy: &EffectivePolicy) -> String {
    let filesystem = if policy.filesystem.read_only() {
        "read-only (no writes)".to_string()
    } else {
        let scopes = policy.filesystem.effective_write_scopes();
        let denials = policy.filesystem.deny_scopes();
        let mut line = format!(
            "Filesystem: read {}, write {}",
            policy.filesystem.read_scopes().join(", "),
            if scopes.is_empty() {
                "none".to_string()
            } else {
                scopes.join(", ")
            }
        );
        if !denials.is_empty() {
            line.push_str(&format!(", deny {}", denials.join(", ")));
        }
        line
    };
    let commands = match policy.commands.mode {
        factory_policy::CommandsMode::Unrestricted => "unrestricted".to_string(),
        factory_policy::CommandsMode::Restricted => {
            format!(
                "restricted (allowed: {}, denied: {})",
                policy.commands.allow.join(", "),
                policy.commands.deny.join(", ")
            )
        }
        factory_policy::CommandsMode::Denied => "denied (no commands may be run)".to_string(),
    };
    let network = if policy.network.allowed() {
        "network: allow (advisory — Factory cannot sandbox the process)".to_string()
    } else {
        "network: deny (advisory — blocked only by instruction, not enforced)".to_string()
    };
    let environment = if policy.environment.filtered {
        "environment: filtered".to_string()
    } else if !policy.environment.denied.is_empty() {
        "environment: inherited, some variables denied".to_string()
    } else {
        "environment: full inheritance".to_string()
    };
    let git_allowed: Vec<String> = policy
        .git
        .allowed
        .iter()
        .map(|operation| operation.as_str().to_string())
        .collect();
    format!(
        "{filesystem}\nCommands: {commands}\n{network}\n{environment}\nGit: {}\n\
         Dangerous Git operations (push, force push, branch deletion, reset, remote \
         modification) are always denied. Factory blocks writes outside the scopes above \
         and never lets task agents touch the integration branch.",
        git_allowed.join(", "),
    )
}

fn render_review_input(input: &ReviewInput) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "\n\nCHANGE UNDER REVIEW\nProduced by: {} ({})\n",
        input.producer_title.trim(),
        input.producer_role
    ));
    text.push_str(&format!(
        "Changed files: {}\nDiff summary:\n{}\nCommit: {}\nProducer-reported commands: {}\n",
        input.evidence.changed_files.join(", "),
        input.evidence.diff_summary,
        input
            .evidence
            .commit_sha
            .as_deref()
            .unwrap_or("not committed"),
        input.evidence.commands.join(", ")
    ));
    let diff = truncate_chars(&input.diff, MAX_REVIEW_DIFF_CHARS);
    text.push_str("\nDIFF\n");
    text.push_str(&diff);
    if diff.chars().count() < input.diff.chars().count() {
        text.push_str("\n…[diff truncated]");
    }
    text.push_str("\n\nPRODUCER OUTPUT\n");
    let producer_output = truncate_chars(&input.producer_output, MAX_PRODUCER_OUTPUT_CHARS);
    text.push_str(&producer_output);
    if producer_output.chars().count() < input.producer_output.chars().count() {
        text.push_str("\n…[producer output truncated]");
    }
    text
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

// --- Structured output parsing ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryReport {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<String>,
    #[serde(default)]
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProducerReport {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    results: Vec<String>,
}

/// Parses the structured output of an advisory task. Tolerant: when the agent
/// does not return valid JSON, the raw output still becomes an analysis
/// artifact so downstream context is never silently empty.
pub fn parse_advisory_report(output: &str) -> (AdvisoryReport, bool) {
    let candidate = strip_code_fence(output);
    match serde_json::from_str::<AdvisoryReport>(candidate) {
        Ok(report) => (report, true),
        Err(_) => (
            AdvisoryReport {
                summary: tail(output, 4_000).trim().to_string(),
                findings: Vec::new(),
                recommendations: Vec::new(),
            },
            false,
        ),
    }
}

/// The commands (and, for verification, results) an implementation-family
/// agent reported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProducedRuns {
    pub summary: String,
    pub commands: Vec<String>,
    pub results: Vec<String>,
}

pub fn parse_producer_report(output: &str) -> ProducedRuns {
    let candidate = strip_code_fence(output);
    match serde_json::from_str::<ProducerReport>(candidate) {
        Ok(report) => ProducedRuns {
            summary: report.summary,
            commands: report.commands,
            results: report.results,
        },
        Err(_) => ProducedRuns::default(),
    }
}

/// Parses the final decision of the built-in Reviewer.
pub fn parse_review(output: &str) -> Result<ReviewResult, String> {
    let candidate = strip_code_fence(output);
    let review: ReviewResult =
        serde_json::from_str(candidate).map_err(|error| format!("invalid JSON: {error}"))?;
    if review.reason.trim().is_empty() {
        return Err("reason must not be empty".into());
    }
    Ok(review)
}

/// Parses the structured decision of a specialized review task and validates
/// its severity values.
pub fn parse_specialized_review(output: &str) -> Result<factory_types::SpecializedReview, String> {
    let candidate = strip_code_fence(output);
    let mut review: factory_types::SpecializedReview =
        serde_json::from_str(candidate).map_err(|error| format!("invalid JSON: {error}"))?;
    if review.decision == ReviewDecision::RequestChanges && review.findings.is_empty() {
        return Err(
            "request_changes requires at least one finding with severity and summary".into(),
        );
    }
    review
        .findings
        .retain(|finding| !finding.summary.trim().is_empty());
    Ok(review)
}

/// Renders a specialized review decision into the final Reviewer's contract
/// so inspectors and evidence can show both consistently.
pub fn review_result_from(findings: &[ReviewFinding]) -> ReviewResult {
    let decision = if findings.is_empty() {
        ReviewDecision::Approve
    } else {
        ReviewDecision::RequestChanges
    };
    let reason = if decision == ReviewDecision::Approve {
        if findings.is_empty() {
            "Specialized review approved with no findings.".to_string()
        } else {
            format!(
                "Specialized review requested changes ({} finding(s)).",
                findings.len()
            )
        }
    } else {
        let worst = findings
            .iter()
            .map(|f| f.severity)
            .max()
            .unwrap_or_default();
        format!(
            "Specialized review requested changes (highest severity: {}).",
            worst.as_str()
        )
    };
    let feedback: Vec<String> = findings
        .iter()
        .map(|finding| {
            format!(
                "[{}] {} {}",
                finding.severity.as_str(),
                finding.summary,
                if finding.evidence.trim().is_empty() {
                    String::new()
                } else {
                    format!("({})", finding.evidence.trim())
                }
            )
        })
        .collect();
    ReviewResult {
        decision,
        reason,
        feedback,
    }
}

fn strip_code_fence(content: &str) -> &str {
    let trimmed = content.trim();
    trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim()
}

fn tail(value: &str, max_chars: usize) -> &str {
    if value.len() <= max_chars {
        return value;
    }
    let mut start = value.len() - max_chars;
    while !value.is_char_boundary(start) {
        start += 1;
    }
    &value[start..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roles;
    use factory_types::{ReviewSeverity, Task, TaskState};

    fn task(title: &str, role: Option<&str>) -> Task {
        Task {
            id: 7,
            run_id: 1,
            title: title.to_string(),
            objective: "Make it work.".into(),
            acceptance_criteria: vec!["works".into()],
            state: TaskState::Ready,
            position: 0,
            dependencies: Vec::new(),
            worktree_path: None,
            role: role.map(str::to_string),
            operation: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn advisory_mission_has_an_output_contract_and_no_false_capabilities() {
        let role = roles::core_role(roles::RESEARCHER).unwrap();
        let mission = build_mission(&MissionContext {
            role: &role,
            operation: TaskOperation::Advisory,
            task: &task("Research auth", Some(roles::RESEARCHER)),
            run_objective: "Authenticate users",
            upstream_artifacts: &[],
            repository_context: None,
            previous_feedback: None,
            review_input: None,
            final_review: false,
            policy: None,
        });
        assert!(mission.contains("ROLE\nResearcher — "));
        assert!(mission.contains("WORKFLOW OBJECTIVE\nAuthenticate users"));
        assert!(mission.contains("OPERATION\nadvisory"));
        assert!(mission.contains("UPSTREAM CONTEXT\nnone."));
        assert!(mission.contains("REPOSITORY CONTEXT\nnone."));
        assert!(mission.contains("\"findings\": [string]"));
        assert!(mission.contains("do not change production files"));
    }

    #[test]
    fn reviewer_mission_receives_diff_and_evidence() {
        let role = roles::core_role(roles::SECURITY_AUDITOR).unwrap();
        let input = ReviewInput {
            producer_title: "Implement auth".into(),
            producer_role: "worker".into(),
            evidence: TaskEvidence {
                changed_files: vec!["src/auth.rs".into()],
                diff_summary: "1 file changed".into(),
                commit_sha: Some("abc".into()),
                commands: vec!["cargo test".into()],
                acceptance_criteria: Vec::new(),
                worker_exit_code: Some(0),
                artifacts: vec![],
                diff_patch: Some("diff --git a/src/auth.rs".into()),
            },
            producer_output: "done".into(),
            diff: "diff --git a/src/auth.rs b/src/auth.rs".into(),
        };
        let mission = build_mission(&MissionContext {
            role: &role,
            operation: TaskOperation::Review,
            task: &task("Audit auth", Some(roles::SECURITY_AUDITOR)),
            run_objective: "Authenticate users",
            upstream_artifacts: &[],
            repository_context: None,
            previous_feedback: None,
            review_input: Some(&input),
            final_review: false,
            policy: None,
        });
        assert!(mission.contains("CHANGE UNDER REVIEW"));
        assert!(mission.contains("Produced by: Implement auth (worker)"));
        assert!(mission.contains("DIFF"));
        assert!(mission.contains("src/auth.rs"));
        assert!(mission.contains("\"severity\": \"low\"|\"medium\"|\"high\"|\"critical\""));
    }

    #[test]
    fn upstream_context_is_bounded_and_marked() {
        let artifact = RoleArtifact {
            id: 1,
            run_id: 1,
            task_id: Some(3),
            attempt_id: None,
            role: "researcher".into(),
            operation: None,
            kind: "research".into(),
            content: "{\"summary\":\"context\"}".into(),
            created_at: String::new(),
        };
        let rendered = render_upstream_context(&[artifact]);
        assert!(rendered.contains("research artifact from task #3 (researcher)"));
        assert!(rendered.contains("\"summary\":\"context\""));
    }

    #[test]
    fn parses_both_review_contracts() {
        assert!(parse_review(r#"{"decision":"approve","reason":"ok"}"#).is_ok());
        assert!(parse_review("approved").is_err());

        let ok = parse_specialized_review(r#"{"decision":"approve","findings":[]}"#).unwrap();
        assert_eq!(ok.decision, ReviewDecision::Approve);

        let changes = parse_specialized_review(
            r#"{"decision":"request_changes","findings":[{"severity":"high","summary":"x","evidence":"y"}]}"#,
        )
        .unwrap();
        assert_eq!(changes.decision, ReviewDecision::RequestChanges);
        assert_eq!(changes.findings[0].severity, ReviewSeverity::High);

        assert!(
            parse_specialized_review(r#"{"decision":"request_changes","findings":[]}"#).is_err()
        );
    }

    #[test]
    fn specialized_review_renders_to_the_final_review_contract() {
        let findings = vec![ReviewFinding {
            severity: ReviewSeverity::High,
            summary: "unchecked input".into(),
            evidence: "src/auth.rs:12".into(),
        }];
        let result = review_result_from(&findings);
        assert_eq!(result.decision, ReviewDecision::RequestChanges);
        assert!(result.reason.contains("requested changes"));
        assert!(result.feedback[0].contains("high"));
        assert!(result.feedback[0].contains("unchecked input"));

        let approve = review_result_from(&[]);
        assert_eq!(approve.decision, ReviewDecision::Approve);
    }

    #[test]
    fn permission_section_renders_the_effective_policy() {
        use factory_policy::PoliciesConfig;
        #[derive(serde::Deserialize)]
        struct Wrapper {
            #[serde(default)]
            policies: PoliciesConfig,
        }
        let wrapper: Wrapper = toml::from_str(
            r#"
[policies.roles.doc_writer]
preset = "documentation"
"#,
        )
        .unwrap();
        let policy = wrapper.policies.effective("doc_writer", "claude");
        let role = crate::roles::core_role(crate::roles::DOCUMENTATION_WRITER).unwrap();
        let mission = build_mission(&MissionContext {
            role: &role,
            operation: TaskOperation::PostProcess,
            task: &task("T", None),
            run_objective: "o",
            upstream_artifacts: &[],
            repository_context: None,
            previous_feedback: None,
            review_input: None,
            final_review: false,
            policy: Some(&policy),
        });
        assert!(
            mission.contains("PERMISSIONS"),
            "mission shows the enforced boundary"
        );
        assert!(mission.contains("write README.md, docs/**"));
        assert!(mission.contains("push, force push, branch deletion, reset, remote modification"));
        // No policy => no PERMISSIONS section (legacy missions unchanged).
        let bare = build_mission(&MissionContext {
            role: &role,
            operation: TaskOperation::PostProcess,
            task: &task("T", None),
            run_objective: "o",
            upstream_artifacts: &[],
            repository_context: None,
            previous_feedback: None,
            review_input: None,
            final_review: false,
            policy: None,
        });
        assert!(!bare.contains("PERMISSIONS"));
    }

    #[test]
    fn advisory_tolerates_non_json_output() {
        let (report, parsed) = parse_advisory_report("I inspected the repo…");
        assert!(!parsed);
        assert!(!report.summary.is_empty());
        let (report, parsed) =
            parse_advisory_report(r#"{"summary":"done","findings":["a"],"recommendations":[]}"#);
        assert!(parsed);
        assert_eq!(report.summary, "done");
    }

    #[test]
    fn producer_report_keeps_commands_and_results_separate() {
        let runs = parse_producer_report(
            r#"{"summary":"ok","commands":["cargo test"],"results":["12 passed"]}"#,
        );
        assert_eq!(runs.commands, vec!["cargo test"]);
        assert_eq!(runs.results, vec!["12 passed"]);
        assert!(parse_producer_report("garbage").commands.is_empty());
    }

    #[test]
    fn all_operation_contracts_render() {
        let role = roles::core_role(roles::WORKER).unwrap();
        for operation in [
            TaskOperation::Advisory,
            TaskOperation::Implement,
            TaskOperation::Verify,
            TaskOperation::Review,
            TaskOperation::PostProcess,
        ] {
            let mission = build_mission(&MissionContext {
                role: &role,
                operation,
                task: &task("T", None),
                run_objective: "o",
                upstream_artifacts: &[],
                repository_context: None,
                previous_feedback: None,
                review_input: None,
                final_review: false,
                policy: None,
            });
            assert!(mission.contains("OUTPUT CONTRACT"), "{operation}");
        }
    }
}
