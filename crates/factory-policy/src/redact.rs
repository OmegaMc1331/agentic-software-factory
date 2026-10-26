//! Incremental secret redaction across streaming output boundaries.
//!
//! `Secrets::redact` operates on one complete string, so applying it chunk by
//! chunk to a decoded process stream misses any secret that a child process
//! happens to emit across two write boundaries. [`StreamingRedactor`] closes
//! that gap: it feeds on consecutive chunks of one stream and only returns the
//! prefix of its buffer that provably cannot become part of a known secret,
//! holding back the (bounded) suffix that a future chunk might complete.
//!
//! Security invariant: the concatenation of everything returned from
//! `push_chunk`/`finish` for a stream never contains a complete known secret,
//! regardless of where chunk boundaries fall.

use crate::environment::Secrets;

/// Replacement text for redacted values. Shared with `Secrets::redact`.
pub(crate) const REDACTION_MARKER: &str = "[REDACTED]";

/// Hard bound (in bytes) on the suffix retained between chunks. A configured
/// secret longer than this bound is still redacted whenever it appears fully
/// inside one chunk and in final whole-text redaction, but a split across
/// chunk boundaries may not be caught while streaming. Real credential and
/// environment values are orders of magnitude shorter than this bound, which
/// exists so a pathological configuration cannot grow memory without limit.
const MAX_HOLD_BYTES: usize = 1024 * 1024;

/// Stateful redactor for one output stream. Create one per stream (stdout and
/// stderr must never share an instance); feed it decoded UTF-8 chunks in
/// arrival order and finish it exactly once when the stream ends.
#[derive(Debug, Clone)]
pub struct StreamingRedactor {
    /// Secrets ordered longest-first so overlapping candidates resolve to the
    /// longest match, mirroring `Secrets::redact`.
    secrets: Vec<String>,
    pending: String,
}

impl StreamingRedactor {
    pub fn new(secrets: &Secrets) -> Self {
        let mut ordered = secrets.values().to_vec();
        ordered.sort_by_key(|value| std::cmp::Reverse(value.len()));
        ordered.dedup();
        Self {
            secrets: ordered,
            pending: String::new(),
        }
    }

    /// Feeds one decoded output chunk and returns every byte that is now
    /// provably safe to persist or forward. The returned text never contains a
    /// complete known secret, and no future chunk can turn already-returned
    /// text into a complete secret.
    pub fn push_chunk(&mut self, chunk: &str) -> String {
        if chunk.is_empty() {
            return String::new();
        }
        if self.secrets.is_empty() {
            return chunk.to_string();
        }
        self.pending.push_str(chunk);
        self.redact_complete_matches();
        let hold = self.required_hold();
        let cut = self.pending.len() - hold;
        // `hold` starts at a character boundary (it equals a prefix of a
        // valid UTF-8 secret), so `split_off(cut)` cannot panic.
        let tail = self.pending.split_off(cut);
        std::mem::replace(&mut self.pending, tail)
    }

    /// Ends the stream and returns the remaining held content. No future
    /// input exists, so whatever is left is emitted verbatim after a final
    /// in-buffer redaction pass; an unterminated secret prefix is not itself
    /// a secret and is safe to release.
    pub fn finish(&mut self) -> String {
        self.redact_complete_matches();
        std::mem::take(&mut self.pending)
    }

    /// Bytes currently held back while waiting for a possible completion.
    /// Bounded by the longest configured secret (and `MAX_HOLD_BYTES`).
    #[cfg(test)]
    pub(crate) fn retained_len(&self) -> usize {
        self.pending.len()
    }

    /// Replaces every complete occurrence of a known secret inside `pending`
    /// (leftmost-longest) without rescanning replacement markers.
    fn redact_complete_matches(&mut self) {
        if self.secrets.is_empty() || self.pending.is_empty() {
            return;
        }
        let source = std::mem::take(&mut self.pending);
        let mut result = String::with_capacity(source.len());
        let bytes = source.as_str();
        let mut pos = 0;
        while pos < bytes.len() {
            let rest = &bytes[pos..];
            let matched = self
                .secrets
                .iter()
                .find(|secret| rest.starts_with(secret.as_str()));
            match matched {
                Some(secret) => {
                    result.push_str(REDACTION_MARKER);
                    pos += secret.len();
                }
                None => {
                    let step = rest.chars().next().expect("non-empty").len_utf8();
                    result.push_str(&rest[..step]);
                    pos += step;
                }
            }
        }
        self.pending = result;
    }

    /// Longest suffix of `pending` that is a proper prefix of some known
    /// secret: exactly the bytes a future chunk could turn into a secret.
    fn required_hold(&self) -> usize {
        let mut hold = 0;
        for secret in &self.secrets {
            // A one-character secret cannot span a boundary; it is fully
            // handled by the in-buffer pass.
            if secret.len() < 2 {
                continue;
            }
            // Every character start except the first is a proper-prefix
            // boundary; take the longest one that matches a pending suffix.
            for (index, _) in secret.char_indices().skip(1) {
                if index > self.pending.len() || index > MAX_HOLD_BYTES {
                    continue;
                }
                if self.pending.ends_with(&secret[..index]) {
                    hold = hold.max(index);
                }
            }
        }
        hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor(secrets: &[&str]) -> StreamingRedactor {
        let mut set = Secrets::new();
        for value in secrets {
            set.add(value, 1);
        }
        StreamingRedactor::new(&set)
    }

    fn stream_all(secrets: &[&str], chunks: &[&str]) -> String {
        let mut redactor = redactor(secrets);
        let mut text = String::new();
        for chunk in chunks {
            text.push_str(&redactor.push_chunk(chunk));
        }
        text.push_str(&redactor.finish());
        text
    }

    #[test]
    fn a_secret_split_across_two_chunks_is_redacted() {
        let out = stream_all(
            &["example-secret-12345"],
            &["alpha example-secret-", "12345 beta"],
        );
        assert!(
            !out.contains("example-secret-12345")
                && !out.contains("example-secret-")
                && !out.contains("12345"),
            "leaked fragments: {out:?}"
        );
        assert!(out.contains("[REDACTED]"));
        assert!(out.starts_with("alpha ") && out.ends_with(" beta"));
    }

    #[test]
    fn a_secret_split_into_many_fragments_is_redacted() {
        let secret = "multi-fragment-secret-value";
        let chunks: Vec<String> = secret
            .chars()
            .map(|c| c.to_string())
            .chain([" tail".to_string()])
            .collect();
        let refs: Vec<&str> = chunks.iter().map(|c| c.as_str()).collect();
        let out = stream_all(&[secret], &refs);
        assert!(!out.contains(secret), "leaked: {out:?}");
        assert!(out.contains("[REDACTED]") && out.ends_with(" tail"));
    }

    #[test]
    fn every_possible_split_point_stays_secret_free() {
        let secret = "boundary-exhaustive-secret";
        let whole = format!("head {secret} tail");
        for split in 0..whole.len() {
            if !whole.is_char_boundary(split) {
                continue;
            }
            let out = stream_all(&[secret], &[&whole[..split], &whole[split..]]);
            assert!(!out.contains(secret), "split {split} leaked: {out:?}");
        }
    }

    #[test]
    fn adjacent_secrets_are_both_redacted() {
        let out = stream_all(&["secret-aaa", "secret-bbb"], &["x secret-aaasecret-bbb y"]);
        assert!(
            !out.contains("secret-aaa") && !out.contains("secret-bbb"),
            "{out:?}"
        );
        assert_eq!(out, "x [REDACTED][REDACTED] y");
    }

    #[test]
    fn overlapping_secrets_never_reconstruct_either_value() {
        let out = stream_all(&["key-1234", "2345"], &["key-12345"]);
        assert!(
            !out.contains("key-1234") && !out.contains("2345"),
            "{out:?}"
        );
    }

    #[test]
    fn a_secret_that_is_a_prefix_of_another_stays_safe_when_completed_later() {
        // "token" is complete after chunk 1 and is redacted immediately; the
        // later "-value" completion cannot resurrect the longer secret.
        let out = stream_all(&["token", "token-value"], &["a token", "-value b"]);
        assert!(
            !out.contains("token-value") && !out.contains("token"),
            "{out:?}"
        );
    }

    #[test]
    fn a_secret_at_the_start_of_the_stream_is_redacted() {
        let out = stream_all(&["head-secret-99"], &["head-secret-99", " rest"]);
        assert_eq!(out, "[REDACTED] rest");
    }

    #[test]
    fn a_secret_at_the_end_of_the_stream_is_redacted() {
        let out = stream_all(&["tail-secret-99"], &["rest ", "tail-secret-99"]);
        assert_eq!(out, "rest [REDACTED]");
    }

    #[test]
    fn an_unterminated_secret_prefix_flushes_safely_at_end_of_stream() {
        let mut redactor = redactor(&["never-completed-secret"]);
        let mut out = redactor.push_chunk("data never-completed");
        out.push_str(&redactor.finish());
        // The stream ended mid-secret: the fragment is released verbatim
        // because it cannot be completed and is not itself the secret.
        assert_eq!(out, "data never-completed");
    }

    #[test]
    fn unicode_secrets_split_at_character_boundaries_are_redacted() {
        let secret = "tokén-秘密-🔒-value";
        let whole = format!("lead {secret} trail");
        for split in whole.char_indices() {
            let out = stream_all(&[secret], &[&whole[..split.0], &whole[split.0..]]);
            assert!(!out.contains(secret), "split {split:?} leaked: {out:?}");
        }
    }

    #[test]
    fn multibyte_characters_in_ordinary_output_are_preserved() {
        let out = stream_all(&["plain-secret"], &["日本語テキスト", "🎉 émoji 🚀"]);
        assert_eq!(out, "日本語テキスト🎉 émoji 🚀");
    }

    #[test]
    fn empty_chunks_and_no_secrets_pass_through_unchanged() {
        let mut none = redactor(&[]);
        assert_eq!(none.push_chunk(""), "");
        assert_eq!(none.push_chunk("plain"), "plain");
        assert_eq!(none.finish(), "");

        let mut some = redactor(&["secret-value"]);
        assert_eq!(some.push_chunk(""), "");
        assert_eq!(some.push_chunk("plain"), "plain");
        assert_eq!(some.finish(), "");
    }

    #[test]
    fn retained_buffer_stays_bounded_under_a_flooded_stream() {
        let mut redactor = redactor(&["flooded-secret-marker"]);
        let mut max_retained = 0;
        for round in 0..5_000 {
            let chunk = if round % 3 == 0 {
                "flooded-secret-mar"
            } else {
                "ordinary text "
            };
            redactor.push_chunk(chunk);
            max_retained = max_retained.max(redactor.retained_len());
        }
        redactor.finish();
        assert!(
            max_retained < "flooded-secret-marker".len(),
            "held back {max_retained} bytes"
        );
    }

    #[test]
    fn deterministic_pseudo_random_splits_never_leak() {
        // LCG split points over a long mixed stream; covers splits inside and
        // between secrets with surrounding noise.
        let secrets = ["alpha-secret-11", "beta-secret-22", "gamma-🔑-secret-33"];
        let mut expected = String::new();
        let mut stream = String::new();
        for index in 0..60 {
            let secret = secrets[index % secrets.len()];
            let piece = format!("n{index} {secret} m{index} ");
            stream.push_str(&piece);
            expected.push_str(&format!("n{index} [REDACTED] m{index} "));
        }
        let mut state: u64 = 0x2545F4914F6CDD1D;
        let mut redactor = redactor(&secrets);
        let mut received = String::new();
        let mut rest = stream.as_str();
        while !rest.is_empty() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let take = (state as usize % 29 + 1).min(rest.len());
            let mut cut = take;
            while cut < rest.len() && !rest.is_char_boundary(cut) {
                cut += 1;
            }
            let (chunk, tail) = rest.split_at(cut);
            received.push_str(&redactor.push_chunk(chunk));
            rest = tail;
        }
        received.push_str(&redactor.finish());
        for secret in secrets {
            assert!(!received.contains(secret), "leaked {secret}: {received:?}");
        }
        assert_eq!(received, expected);
    }

    #[test]
    fn stdout_and_stderr_states_are_independent() {
        let mut stdout = redactor(&["stream-secret-value"]);
        let mut stderr = redactor(&["stream-secret-value"]);
        let out_a = stdout.push_chunk("out stream-secret-");
        let err_a = stderr.push_chunk("err stream-sec");
        let out_b = stdout.push_chunk("value done");
        let err_b = stderr.push_chunk("ret-value done");
        assert_eq!(
            format!("{out_a}{out_b}{}", stdout.finish()),
            "out [REDACTED] done"
        );
        assert_eq!(
            format!("{err_a}{err_b}{}", stderr.finish()),
            "err [REDACTED] done"
        );
    }

    #[test]
    fn secrets_longer_than_the_hold_bound_are_redacted_within_a_chunk() {
        let long_secret = "s".repeat(MAX_HOLD_BYTES + 64);
        let mut redactor = redactor(&[long_secret.as_str()]);
        let out = redactor.push_chunk(&long_secret);
        assert_eq!(out, "[REDACTED]");
        assert!(redactor.retained_len() <= MAX_HOLD_BYTES);
        assert_eq!(redactor.finish(), "");
    }
}
