//! Pull request payload parsing and deterministic PR content generation.

use factory_types::PullRequestInfo;
use serde::Deserialize;

/// Parses `gh pr list --json number,url,state,isDraft` output.
pub fn parse_pull_requests(content: &str) -> std::result::Result<Vec<PullRequestInfo>, String> {
    let raw: Vec<RawPullRequest> = serde_json::from_str(content).map_err(|e| e.to_string())?;
    Ok(raw.into_iter().map(Into::into).collect())
}

#[derive(Debug, Deserialize)]
struct RawPullRequest {
    number: i64,
    url: String,
    state: String,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
}

impl From<RawPullRequest> for PullRequestInfo {
    fn from(raw: RawPullRequest) -> PullRequestInfo {
        PullRequestInfo {
            number: raw.number,
            url: raw.url,
            state: raw.state,
            is_draft: raw.is_draft,
        }
    }
}

/// Extracts `(number, url)` from a `gh pr create` stdout URL such as
/// `https://github.com/owner/repo/pull/58`.
pub fn parse_pull_request_url(text: &str) -> Option<PullRequestInfo> {
    let text = text.trim();
    let start = text.rfind("https://github.com/")?;
    let url = &text[start..];
    let url = url.split_whitespace().next().unwrap_or(url);
    let number = url.trim_end_matches('/').rsplit('/').next()?.parse().ok()?;
    if number <= 0 {
        return None;
    }
    Some(PullRequestInfo {
        number,
        url: url.trim_end_matches('/').to_string(),
        state: "OPEN".to_string(),
        is_draft: false,
    })
}

/// The evidence Factory has about a completed workflow, used to build the PR
/// body deterministically. Every field comes from persisted state — nothing is
/// fabricated.
#[derive(Debug, Clone, Default)]
pub struct PrEvidence {
    pub objective: String,
    /// `(#id, title, role)` rows for every task in the plan.
    pub tasks: Vec<(i64, String, Option<String>)>,
    /// Verification commands actually reported by approved verify attempts.
    pub verification_commands: Vec<String>,
    /// `(role, approved)` rows for review attempts.
    pub reviews: Vec<(String, bool)>,
    /// Linked issue number, when the workflow came from an issue.
    pub issue_number: Option<i64>,
}

/// Default PR title: the linked issue's title is supplied by the caller via
/// `issue_title`; falls back to the workflow objective's first line. Never
/// LLM-generated marketing copy.
pub fn default_pr_title(objective: &str, issue_title: Option<&str>) -> String {
    let base = issue_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| objective.lines().next().unwrap_or_default().trim());
    bound(base, 100)
}

/// Builds the initial PR body from actual workflow evidence.
pub fn build_pr_body(evidence: &PrEvidence) -> String {
    let mut body = String::new();
    body.push_str("## Summary\n\n");
    let summary = evidence.objective.trim();
    body.push_str(if summary.is_empty() {
        "(no objective recorded)"
    } else {
        summary
    });
    body.push_str("\n\n## Changes\n\n");
    if evidence.tasks.is_empty() {
        body.push_str("- no tasks recorded\n");
    } else {
        for (id, title, role) in &evidence.tasks {
            match role {
                Some(role) if !role.is_empty() && role != "worker" => {
                    body.push_str(&format!("- Task #{id} ({role}): {title}\n"));
                }
                _ => body.push_str(&format!("- Task #{id}: {title}\n")),
            }
        }
    }
    body.push_str("\n## Verification\n\n");
    if evidence.verification_commands.is_empty() {
        body.push_str("- no verification commands reported by the workflow\n");
    } else {
        for command in &evidence.verification_commands {
            body.push_str(&format!("- `{command}`\n"));
        }
    }
    body.push_str("\n## Reviews\n\n");
    if evidence.reviews.is_empty() {
        body.push_str("- no review attempts recorded\n");
    } else {
        for (role, approved) in &evidence.reviews {
            if *approved {
                body.push_str(&format!("- {role} approved\n"));
            } else {
                body.push_str(&format!("- {role} requested changes\n"));
            }
        }
    }
    if let Some(number) = evidence.issue_number {
        body.push_str(&format!("\nCloses #{number}\n"));
    }
    body
}

fn bound(value: &str, max: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    trimmed.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pr_list_payloads() {
        let prs = parse_pull_requests(
            r#"[{"number":58,"url":"https://github.com/o/r/pull/58","state":"OPEN","isDraft":true}]"#,
        )
        .unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 58);
        assert!(prs[0].is_draft);
        assert_eq!(prs[0].state, "OPEN");
    }

    #[test]
    fn parses_created_pr_urls_from_gh_output() {
        let pr = parse_pull_request_url(
            "Creating pull request for feature into main in o/r\nhttps://github.com/o/r/pull/58\n",
        )
        .unwrap();
        assert_eq!(pr.number, 58);
        assert_eq!(pr.url, "https://github.com/o/r/pull/58");
        assert!(parse_pull_request_url("no url here").is_none());
    }

    #[test]
    fn pr_title_prefers_the_issue_title() {
        assert_eq!(
            default_pr_title(
                "Resolve GitHub Issue #42: Fix race\n\nlong body",
                Some("Fix refresh token race")
            ),
            "Fix refresh token race"
        );
        assert_eq!(
            default_pr_title("Short workflow objective\n\nbody", None),
            "Short workflow objective"
        );
    }

    #[test]
    fn pr_body_uses_evidence_and_never_fabricates() {
        let evidence = PrEvidence {
            objective: "Resolve GitHub Issue #42: Fix refresh token race".into(),
            tasks: vec![
                (12, "Implement token guard".into(), None),
                (
                    13,
                    "Add regression tests".into(),
                    Some("test_engineer".into()),
                ),
            ],
            verification_commands: vec!["cargo test".into()],
            reviews: vec![("reviewer".into(), true), ("security_auditor".into(), true)],
            issue_number: Some(42),
        };
        let body = build_pr_body(&evidence);
        assert!(body.contains("## Summary\n\nResolve GitHub Issue #42"));
        assert!(body.contains("- Task #12: Implement token guard"));
        assert!(body.contains("- Task #13 (test_engineer): Add regression tests"));
        assert!(body.contains("- `cargo test`"));
        assert!(body.contains("- reviewer approved"));
        assert!(body.contains("- security_auditor approved"));
        assert!(body.trim_end().ends_with("Closes #42"));
        assert!(!body.to_lowercase().contains("co-authored-by"));
    }

    #[test]
    fn pr_body_states_absent_evidence_instead_of_inventing_it() {
        let body = build_pr_body(&PrEvidence {
            objective: "o".into(),
            ..PrEvidence::default()
        });
        assert!(body.contains("- no tasks recorded"));
        assert!(body.contains("- no verification commands reported"));
        assert!(body.contains("- no review attempts recorded"));
        assert!(!body.contains("Closes #"));
    }
}
