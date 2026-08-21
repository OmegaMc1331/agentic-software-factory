//! Environment filtering and secret redaction.
//!
//! When a role policy restricts the environment, Factory computes the exact
//! process environment before launch instead of letting an external agent
//! inherit the whole Factory process environment. This module owns both the
//! filtering rule and the redaction that keeps secret values out of logs.

use std::collections::BTreeMap;

/// Filters an inherited environment against an allow/deny policy.
///
/// - `allowed` empty means the caller keeps the inherited environment as-is
///   (legacy behavior); a non-empty `allowed` restricts to exactly those keys.
/// - `denied` keys are always removed, regardless of allow lists or a later
///   caller (`deny` wins over `allow`).
///
/// Keys are compared case-insensitively because environment keys are
/// case-insensitive on Windows and this is harmless elsewhere.
pub fn filter_environment(
    inherited: impl IntoIterator<Item = (String, String)>,
    allowed: &[String],
    denied: &[String],
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let allow_casefold: Vec<String> = allowed.iter().map(|value| casefold(value)).collect();
    let deny_casefold: Vec<String> = denied.iter().map(|value| casefold(value)).collect();
    for (key, value) in inherited {
        if deny_casefold.contains(&casefold(&key)) {
            continue;
        }
        if allow_casefold.is_empty() || allow_casefold.contains(&casefold(&key)) {
            result.insert(key, value);
        }
    }
    result
}

/// Applies an explicit deny list to a configured environment map (for example
/// the `[agents.<name>].env` values). Deny wins over allow; allow lists never
/// gate values a user explicitly configured on an agent.
pub fn filter_configured_env(
    configured: &BTreeMap<String, String>,
    denied: &[String],
) -> BTreeMap<String, String> {
    let deny_casefold: Vec<String> = denied.iter().map(|value| casefold(value)).collect();
    configured
        .iter()
        .filter(|(key, _)| !deny_casefold.contains(&casefold(key)))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// A set of secret values whose occurrences must never reach logs.
#[derive(Debug, Clone, Default)]
pub struct Secrets {
    values: Vec<String>,
}

impl Secrets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensures a value is redacted if it ever appears in captured output.
    /// Values shorter than `MIN_SECRET_CHARS` characters are ignored to avoid
    /// redacting noise such as single characters.
    pub fn add(&mut self, value: &str, min_chars: usize) {
        if value.len() >= min_chars && !self.values.iter().any(|known| known == value) {
            self.values.push(value.to_string());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Replaces every occurrence of a known secret with `[REDACTED]`.
    /// Longer secrets are replaced first so overlapping values keep the
    /// largest match visible as redacted.
    pub fn redact(&self, text: &str) -> String {
        if self.values.is_empty() {
            return text.to_string();
        }
        let mut ordered = self.values.clone();
        ordered.sort_by_key(|value| std::cmp::Reverse(value.len()));
        let mut redacted = text.to_string();
        for secret in &ordered {
            redacted = redacted.replace(secret.as_str(), "[REDACTED]");
        }
        redacted
    }
}

const MIN_SECRET_CHARS: usize = 4;

/// Collects redaction-worthy secret values from a filtered environment: the
/// values of every denied key (values Factory actively withheld from the
/// process are exactly the ones that must not leak back into logs).
pub fn secrets_from_denied(
    inherited: impl IntoIterator<Item = (String, String)>,
    denied: &[String],
) -> Secrets {
    let deny_casefold: Vec<String> = denied.iter().map(|value| casefold(value)).collect();
    let mut secrets = Secrets::new();
    for (key, value) in inherited {
        if deny_casefold.contains(&casefold(&key)) {
            secrets.add(&value, MIN_SECRET_CHARS);
        }
    }
    secrets
}

/// Convenience redaction of already-known secret values.
pub fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut set = Secrets::new();
    for value in secrets {
        set.add(value, MIN_SECRET_CHARS);
    }
    set.redact(text)
}

fn casefold(value: &str) -> String {
    value.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn empty_allow_keeps_the_inherited_environment() {
        let filtered = filter_environment(env(&[("PATH", "/bin"), ("HOME", "/home/me")]), &[], &[]);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn allow_list_restricts_which_keys_pass_through() {
        let inherited = env(&[
            ("PATH", "/bin"),
            ("HOME", "/home/me"),
            ("AWS_SECRET_ACCESS_KEY", "x"),
        ]);
        let allowed = vec!["PATH".to_string(), "HOME".to_string()];
        let filtered = filter_environment(inherited, &allowed, &[]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains_key("PATH"));
        assert!(filtered.contains_key("HOME"));
        assert!(!filtered.contains_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn deny_wins_over_allow() {
        let inherited = env(&[("PATH", "/bin"), ("GITHUB_TOKEN", "tok"), ("MY_KEY", "k")]);
        let allowed = vec!["PATH".to_string(), "GITHUB_TOKEN".to_string()];
        let denied = vec!["GITHUB_TOKEN".to_string()];
        let filtered = filter_environment(inherited, &allowed, &denied);
        assert!(filtered.contains_key("PATH"));
        assert!(!filtered.contains_key("GITHUB_TOKEN"));
        assert!(!filtered.contains_key("MY_KEY"));
    }

    #[test]
    fn deny_is_case_insensitive() {
        let inherited = env(&[("GitHub_Token", "tok")]);
        let filtered = filter_environment(inherited, &[], &["github_token".to_string()]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn configured_env_is_filtered_by_deny_only() {
        let configured: BTreeMap<String, String> =
            [("OPENAI_API_KEY".to_string(), "sk-secret".to_string())].into();
        let filtered = filter_configured_env(&configured, &["openai_api_key".to_string()]);
        assert!(filtered.is_empty(), "deny strips configured values too");
        let kept = filter_configured_env(&configured, &["OTHER".to_string()]);
        assert_eq!(kept.len(), 1, "unrelated deny keeps the configured value");
    }

    #[test]
    fn redaction_hides_secret_values() {
        let redacted = redact_secrets(
            "build log: token=abc-secret-value and GITHUB_TOKEN=abc-secret-value done",
            &["abc-secret-value"],
        );
        assert!(!redacted.contains("abc-secret-value"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn shorter_secrets_are_ignored_to_avoid_noise() {
        assert_eq!(redact_secrets("abc", &["abc"]), "abc");
    }

    #[test]
    fn secret_values_never_appear_in_filtered_output() {
        let inherited = env(&[("GITHUB_TOKEN", "super-secret-value-1234")]);
        let secrets = secrets_from_denied(inherited, &["GITHUB_TOKEN".to_string()]);
        assert!(!secrets.is_empty());
        let redacted = secrets.redact("agent output echoed super-secret-value-1234");
        assert!(!redacted.contains("super-secret-value-1234"));
    }
}
