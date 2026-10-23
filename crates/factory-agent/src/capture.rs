use std::collections::VecDeque;

use factory_types::AgentOutputMetadata;

/// Maximum number of decoded bytes retained from each agent stream.
pub const DEFAULT_AGENT_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

/// Bound on the number of chunks buffered between a pipe reader and the
/// process supervisor. Combined with [`OUTPUT_READ_BUFFER_BYTES`], this caps
/// queued memory even when a child floods stdout faster than Factory drains it.
pub const OUTPUT_CHANNEL_CAPACITY: usize = 32;
pub const OUTPUT_READ_BUFFER_BYTES: usize = 8192;

#[derive(Debug, Clone)]
pub(crate) struct BoundedOutputCapture {
    limit: usize,
    head_limit: usize,
    tail_limit: usize,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedOutput {
    pub text: String,
    pub metadata: AgentOutputMetadata,
}

impl BoundedOutputCapture {
    pub(crate) fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        let head_limit = limit / 4;
        BoundedOutputCapture {
            limit,
            head_limit,
            tail_limit: limit - head_limit,
            head: Vec::new(),
            tail: VecDeque::new(),
            total: 0,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len() as u64);
        let mut remaining = bytes;

        if self.head_limit > self.head.len() {
            let take = remaining.len().min(self.head_limit - self.head.len());
            self.head.extend_from_slice(&remaining[..take]);
            remaining = &remaining[take..];
        }
        if remaining.is_empty() {
            return;
        }
        if self.tail_limit == 0 {
            return;
        }

        if remaining.len() >= self.tail_limit {
            self.tail.clear();
            self.tail
                .extend(&remaining[remaining.len() - self.tail_limit..]);
        } else {
            let overflow = (self.tail.len() + remaining.len()).saturating_sub(self.tail_limit);
            for _ in 0..overflow {
                self.tail.pop_front();
            }
            self.tail.extend(remaining);
        }
    }

    pub(crate) fn reached_limit(&self) -> bool {
        self.total >= self.limit as u64
    }

    pub(crate) fn finish(mut self) -> CapturedOutput {
        let truncated = self.total > self.limit as u64;
        let mut tail = Vec::with_capacity(self.tail.len());
        tail.extend(self.tail);
        if truncated {
            trim_incomplete_utf8_tail(&mut self.head);
            let continuation = tail
                .iter()
                .take(3)
                .take_while(|byte| **byte & 0b1100_0000 == 0b1000_0000)
                .count();
            tail.drain(..continuation);
        }

        let retained_bytes = self.head.len() + tail.len();
        let dropped = if truncated {
            self.total.saturating_sub(retained_bytes as u64)
        } else {
            0
        };
        let mut metadata = AgentOutputMetadata {
            limit_bytes: self.limit as u64,
            stdout_bytes: self.total,
            stderr_bytes: 0,
            stdout_dropped_bytes: dropped,
            stderr_dropped_bytes: 0,
            stdout_truncated: truncated,
            stderr_truncated: false,
        };
        let mut retained = Vec::with_capacity(retained_bytes);
        retained.extend_from_slice(&self.head);
        retained.extend_from_slice(&tail);
        let (text, fit_truncated) =
            fit_to_limit(String::from_utf8_lossy(&retained).into_owned(), self.limit);
        metadata.stdout_truncated |= fit_truncated;
        CapturedOutput { text, metadata }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        let mut output = String::with_capacity(bytes.len());
        let leftover = if self.pending.is_empty() {
            decode_utf8(bytes, &mut output)
        } else {
            let mut combined = std::mem::take(&mut self.pending);
            combined.extend_from_slice(bytes);
            decode_utf8(&combined, &mut output)
        };
        self.pending = leftover;
        output
    }

    pub(crate) fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

fn decode_utf8(bytes: &[u8], output: &mut String) -> Vec<u8> {
    let mut remaining = bytes;
    loop {
        match std::str::from_utf8(remaining) {
            Ok(text) => {
                output.push_str(text);
                return Vec::new();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    let text =
                        std::str::from_utf8(&remaining[..valid]).expect("validated UTF-8 prefix");
                    output.push_str(text);
                }
                match error.error_len() {
                    Some(length) => {
                        output.push('\u{FFFD}');
                        remaining = &remaining[valid + length..];
                    }
                    None => return remaining[valid..].to_vec(),
                }
            }
        }
    }
}

impl CapturedOutput {
    pub(crate) fn into_streams(
        self,
        stderr: CapturedOutput,
    ) -> (String, String, AgentOutputMetadata) {
        let mut metadata = self.metadata;
        metadata.stderr_bytes = stderr.metadata.stdout_bytes;
        metadata.stderr_dropped_bytes = stderr.metadata.stdout_dropped_bytes;
        metadata.stderr_truncated = stderr.metadata.stdout_truncated;
        (self.text, stderr.text, metadata)
    }
}

fn trim_incomplete_utf8_tail(bytes: &mut Vec<u8>) {
    for cut in 0..=3.min(bytes.len()) {
        let end = bytes.len() - cut;
        if std::str::from_utf8(&bytes[..end]).is_ok() {
            bytes.truncate(end);
            return;
        }
    }
}

fn fit_to_limit(mut text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut start = text.len() - limit;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text.drain(..start);
    (text, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_small_and_exact_limit_output() {
        let mut small = BoundedOutputCapture::new(16);
        small.push(b"hello");
        let small = small.finish();
        assert_eq!(small.text, "hello");
        assert!(!small.metadata.stdout_truncated);
        assert_eq!(small.metadata.stdout_bytes, 5);

        let mut exact = BoundedOutputCapture::new(16);
        exact.push(&[b'x'; 16]);
        let exact = exact.finish();
        assert_eq!(exact.text, "xxxxxxxxxxxxxxxx");
        assert!(!exact.metadata.stdout_truncated);
        assert_eq!(exact.metadata.stdout_dropped_bytes, 0);
    }

    #[test]
    fn retains_head_and_tail_when_output_exceeds_limit() {
        let mut capture = BoundedOutputCapture::new(16);
        capture.push(b"HEAD");
        capture.push(&[b'm'; 100]);
        capture.push(b"TAIL");
        let output = capture.finish();
        assert_eq!(output.text.len(), 16);
        assert!(output.text.starts_with("HEAD"));
        assert!(output.text.ends_with("TAIL"));
        assert!(output.metadata.stdout_truncated);
        assert_eq!(output.metadata.stdout_bytes, 108);
        assert_eq!(
            output.metadata.stdout_dropped_bytes,
            108 - output.text.len() as u64
        );
    }

    #[test]
    fn a_multibyte_character_split_between_head_and_tail_is_not_truncation() {
        let mut capture = BoundedOutputCapture::new(16);
        capture.push("日é".as_bytes());
        let output = capture.finish();
        assert_eq!(output.text, "日é");
        assert!(!output.metadata.stdout_truncated);
        assert_eq!(output.metadata.stdout_dropped_bytes, 0);
    }

    #[test]
    fn invalid_utf8_is_bounded_and_marked_truncated() {
        let mut capture = BoundedOutputCapture::new(3);
        capture.push(&[0xff; 3]);
        let output = capture.finish();
        assert!(output.text.len() <= 3);
        assert!(output.metadata.stdout_truncated);
        assert_eq!(output.metadata.stdout_bytes, 3);
    }

    #[test]
    fn counts_are_independent_per_stream() {
        let mut stdout = BoundedOutputCapture::new(8);
        stdout.push(b"1234567890");
        let mut stderr = BoundedOutputCapture::new(8);
        stderr.push(b"err");
        let (stdout, stderr, metadata) = stdout.finish().into_streams(stderr.finish());
        assert_eq!(stdout.len(), 8);
        assert_eq!(stderr, "err");
        assert!(metadata.stdout_truncated);
        assert!(!metadata.stderr_truncated);
        assert_eq!(metadata.stdout_bytes, 10);
        assert_eq!(metadata.stderr_bytes, 3);
        assert_eq!(metadata.stdout_dropped_bytes, 2);
        assert_eq!(metadata.stderr_dropped_bytes, 0);
    }

    #[test]
    fn decoder_preserves_multibyte_characters_split_across_reads() {
        let encoded = "aé日🎉z".as_bytes();
        for split in 1..encoded.len() {
            let mut decoder = Utf8StreamDecoder::default();
            let mut text = decoder.push(&encoded[..split]);
            text.push_str(&decoder.push(&encoded[split..]));
            text.push_str(&decoder.finish());
            assert_eq!(text, "aé日🎉z");
        }
    }

    #[test]
    fn output_never_exceeds_the_configured_limit() {
        let mut capture = BoundedOutputCapture::new(64);
        for _ in 0..10_000 {
            capture.push("é".repeat(100).as_bytes());
        }
        let output = capture.finish();
        assert!(output.text.len() <= 64);
        assert!(output.text.is_char_boundary(0));
        assert!(!output.text.contains('\u{FFFD}'));
        assert!(output.metadata.stdout_truncated);
    }
}
