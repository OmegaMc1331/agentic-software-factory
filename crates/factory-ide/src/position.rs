use crate::{IdeError, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}

pub fn offset(text: &str, position: Position, encoding: &str) -> Result<usize> {
    let mut start = 0;
    for _ in 0..position.line {
        start += text[start..].find('\n').ok_or(IdeError::Invalid)? + 1;
    }
    let line = text[start..]
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_end_matches('\r');
    let mut units = 0;
    for (index, character) in line.char_indices() {
        if units == position.character {
            return Ok(start + index);
        }
        units += match encoding {
            "utf-8" => character.len_utf8(),
            "utf-16" => character.len_utf16(),
            "utf-32" => 1,
            _ => return Err(IdeError::Unsupported),
        };
        if units > position.character {
            return Err(IdeError::Invalid);
        }
    }
    if units == position.character {
        Ok(start + line.len())
    } else {
        Err(IdeError::Invalid)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

pub fn apply(text: &str, edits: &[TextEdit], encoding: &str) -> Result<String> {
    if edits.len() > 10_000 {
        return Err(IdeError::Limit);
    }
    let mut ranges = edits
        .iter()
        .map(|edit| {
            Ok((
                offset(text, edit.range.start, encoding)?,
                offset(text, edit.range.end, encoding)?,
                &edit.new_text,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    ranges.sort_by_key(|item| (item.0, item.1));
    let mut previous = 0;
    let mut result = String::new();
    for (start, end, inserted) in ranges {
        if start < previous || end < start {
            return Err(IdeError::Invalid);
        }
        result.push_str(&text[previous..start]);
        result.push_str(inserted);
        previous = end;
        if result.len() > 1_048_576 {
            return Err(IdeError::Limit);
        }
    }
    result.push_str(&text[previous..]);
    if result.len() > 1_048_576 {
        return Err(IdeError::Limit);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_positions_crlf_and_surrogates() {
        let text = "a😀e\u{301}\r\nβ🦀\n";
        for (encoding, width) in [("utf-8", 5), ("utf-16", 3), ("utf-32", 2)] {
            assert_eq!(
                offset(
                    text,
                    Position {
                        line: 0,
                        character: width
                    },
                    encoding
                )
                .unwrap(),
                5
            );
        }
        assert!(offset(
            text,
            Position {
                line: 0,
                character: 2
            },
            "utf-16"
        )
        .is_err());
        assert_eq!(
            offset(
                text,
                Position {
                    line: 1,
                    character: 1
                },
                "utf-16"
            )
            .unwrap(),
            text.find('🦀').unwrap()
        );
        assert!(offset(
            text,
            Position {
                line: 0,
                character: 99
            },
            "utf-8"
        )
        .is_err());
    }

    #[test]
    fn differential_unicode_positions_and_nonoverlapping_edits() {
        let alphabet = ['a', 'β', '😀', '\u{301}', '中', '🦀'];
        for length in 0..128 {
            let text: String = (0..length)
                .map(|i| alphabet[(i * 7 + length) % alphabet.len()])
                .collect();
            for encoding in ["utf-8", "utf-16", "utf-32"] {
                let mut character = 0;
                for (index, c) in text.char_indices() {
                    assert_eq!(
                        offset(&text, Position { line: 0, character }, encoding).unwrap(),
                        index
                    );
                    character += match encoding {
                        "utf-8" => c.len_utf8(),
                        "utf-16" => c.len_utf16(),
                        _ => 1,
                    };
                }
                assert_eq!(
                    offset(&text, Position { line: 0, character }, encoding).unwrap(),
                    text.len()
                );
            }
        }
        let edit = TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 1,
                },
                end: Position {
                    line: 0,
                    character: 3,
                },
            },
            new_text: "x".into(),
        };
        assert_eq!(
            apply("a😀b", std::slice::from_ref(&edit), "utf-16").unwrap(),
            "axb"
        );
        assert!(apply("a😀b", &[edit.clone(), edit], "utf-16").is_err());
    }
}
