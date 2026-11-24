use factory_agent::{AgentKind, AgentResult};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum AuthorizationCode {
    PolicyDenied,
    ApprovalRequired,
    ApprovalUnavailable,
    HostSandboxRestricted,
    OsAccessDenied,
}

impl AuthorizationCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::PolicyDenied => "POLICY_DENIED",
            Self::ApprovalRequired => "APPROVAL_REQUIRED",
            Self::ApprovalUnavailable => "APPROVAL_UNAVAILABLE",
            Self::HostSandboxRestricted => "HOST_SANDBOX_RESTRICTED",
            Self::OsAccessDenied => "OS_ACCESS_DENIED",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ReportedBlocker {
    code: AuthorizationCode,
    reason: String,
}

pub(crate) fn resource_exhausted(kind: AgentKind, result: &AgentResult) -> bool {
    kind == AgentKind::Codex
        && result.exit_code == Some(1)
        && !result.cancelled
        && !result.output_metadata.truncated()
        && result
            .stderr
            .lines()
            .take(24)
            .any(|line| line.starts_with("OpenAI Codex v"))
        && result.stderr.lines().any(|line| {
            line.starts_with("ERROR: You've hit your usage limit.")
                || line.starts_with("ERROR: You\u{2019}ve hit your usage limit.")
        })
}

pub(crate) fn failure(
    kind: AgentKind,
    result: &AgentResult,
    requires_writes: bool,
    accepts_report: bool,
) -> Result<Option<String>, String> {
    if result.cancelled || result.output_metadata.truncated() {
        return Ok(None);
    }
    if kind == AgentKind::Codex {
        let header: Vec<_> = result.stderr.lines().take(24).map(str::trim).collect();
        if requires_writes
            && header.iter().any(|line| line.starts_with("OpenAI Codex v"))
            && header.contains(&"sandbox: read-only")
        {
            return Ok(Some("POLICY_DENIED [agent_cli_native]: Codex reported a read-only sandbox for an implementation task. Inspect its Windows sandbox backend and managed configuration; ASF did not override permissions.".into()));
        }
        for line in result.stderr.lines() {
            let Some((timestamp, diagnostic)) =
                line.split_once(" ERROR codex_core::tools::router: ")
            else {
                continue;
            };
            if chrono::DateTime::parse_from_rfc3339(timestamp.trim()).is_ok()
                && diagnostic.starts_with("error=exec_command failed:")
                && diagnostic.contains("rejected: blocked by policy")
            {
                return Ok(Some("POLICY_DENIED [agent_cli_native]: Codex rejected a command before execution. The linked agent session retains the diagnostic. This non-interactive transport does not grant approvals; inspect the CLI policy before an explicit retry.".into()));
            }
        }
    }
    if !accepts_report {
        return Ok(None);
    }
    let text = result.stdout.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(None);
    };
    let Some(blocker) = value
        .get("permissionBlocker")
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    let blocker: ReportedBlocker = serde_json::from_value(blocker.clone()).map_err(|_| {
        "Invalid permissionBlocker: expected a known code and reason only.".to_string()
    })?;
    if blocker.reason.trim().is_empty() || blocker.reason.len() > 2000 {
        return Err("Invalid permissionBlocker: reason must contain 1-2000 UTF-8 bytes.".into());
    }
    Ok(Some(format!(
        "{} [agent_report; denying layer unverified]: {}. Execution stopped without granting permission or automatically retrying.",
        blocker.code.as_str(),
        blocker.reason.trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_types::AgentOutputMetadata;
    use std::time::Duration;

    fn result(stdout: &str, stderr: &str) -> AgentResult {
        AgentResult {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code: Some(0),
            duration: Duration::ZERO,
            cancelled: false,
            output_metadata: AgentOutputMetadata::default(),
        }
    }

    #[test]
    fn codex_readonly_header_is_a_blocker_only_for_write_operations() {
        let r = result(
            "{}",
            "OpenAI Codex v0.165.0  \n--------\nsandbox: read-only  \n",
        );
        assert!(failure(AgentKind::Codex, &r, true, true)
            .unwrap()
            .unwrap()
            .starts_with("POLICY_DENIED [agent_cli_native]"));
        assert_eq!(failure(AgentKind::Codex, &r, false, false).unwrap(), None);
        assert_eq!(failure(AgentKind::Custom, &r, true, true).unwrap(), None);
    }

    #[test]
    fn native_command_rejection_is_not_attributed_to_a_human_reviewer() {
        let r = result("{}", "2026-10-30T14:36:27.137018Z ERROR codex_core::tools::router: error=exec_command failed: CreateProcess rejected: blocked by policy\n");
        assert!(failure(AgentKind::Codex, &r, false, false)
            .unwrap()
            .unwrap()
            .contains("before execution"));
        assert_eq!(failure(AgentKind::Custom, &r, false, false).unwrap(), None);
        let prose = result(
            "The source says rejected: blocked by policy",
            "rejected: blocked by policy",
        );
        assert_eq!(failure(AgentKind::Codex, &prose, true, true).unwrap(), None);
    }

    #[test]
    fn reported_blockers_are_finite_and_do_not_claim_verified_host_attribution() {
        for code in [
            "POLICY_DENIED",
            "APPROVAL_REQUIRED",
            "APPROVAL_UNAVAILABLE",
            "HOST_SANDBOX_RESTRICTED",
            "OS_ACCESS_DENIED",
        ] {
            let r = result(
                &format!(
                    r#"{{"permissionBlocker":{{"code":"{code}","reason":"Required operation refused"}}}}"#
                ),
                "",
            );
            let failure = failure(AgentKind::Custom, &r, true, true).unwrap().unwrap();
            assert!(failure.starts_with(code));
            assert!(failure.contains("denying layer unverified"));
            assert!(failure.contains("without granting permission"));
        }
    }

    #[test]
    fn malformed_reported_authorization_cannot_select_a_command_or_policy() {
        for content in [
            r#"{"code":"RUN_SHELL","reason":"x"}"#,
            r#"{"code":"POLICY_DENIED","reason":"x","command":"override"}"#,
            r#"{"code":"POLICY_DENIED","reason":""}"#,
        ] {
            let r = result(&format!(r#"{{"permissionBlocker":{content}}}"#), "");
            assert!(failure(AgentKind::Custom, &r, true, true).is_err());
        }
    }

    #[test]
    fn truncated_or_cancelled_output_cannot_be_a_valid_permission_report() {
        let mut r = result(
            r#"{"permissionBlocker":{"code":"POLICY_DENIED","reason":"x"}}"#,
            "",
        );
        r.output_metadata.stdout_truncated = true;
        assert_eq!(failure(AgentKind::Custom, &r, true, true).unwrap(), None);
        r.output_metadata.stdout_truncated = false;
        r.cancelled = true;
        assert_eq!(failure(AgentKind::Custom, &r, true, true).unwrap(), None);
    }

    #[test]
    fn exhausted_native_usage_is_distinct_from_code_and_policy_failures() {
        let mut r = result(
            "",
            "OpenAI Codex v0.165.0\nERROR: You\u{2019}ve hit your usage limit. Try again later.\n",
        );
        assert!(!resource_exhausted(AgentKind::Codex, &r));
        r.exit_code = Some(1);
        assert!(resource_exhausted(AgentKind::Codex, &r));
        assert!(!resource_exhausted(AgentKind::Custom, &r));
        assert_eq!(failure(AgentKind::Codex, &r, false, false).unwrap(), None);
        r.output_metadata.stderr_truncated = true;
        assert!(!resource_exhausted(AgentKind::Codex, &r));
        r.output_metadata.stderr_truncated = false;
        r.cancelled = true;
        assert!(!resource_exhausted(AgentKind::Codex, &r));
    }
}
