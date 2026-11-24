use std::io::{BufRead, Read, Write};

use serde_json::Value;

use crate::{IdeError, Result};

pub const MAX_MESSAGE: usize = 2 * 1024 * 1024;

pub fn read_message(input: &mut impl BufRead) -> Result<Value> {
    let mut length = None;
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let count = input.take(8193).read_until(b'\n', &mut line)?;
        total += count;
        if count == 0 {
            return Err(IdeError::Exited);
        }
        if total > 8192 || !line.ends_with(b"\r\n") {
            return Err(IdeError::Protocol);
        }
        if line == b"\r\n" {
            break;
        }
        let header = std::str::from_utf8(&line).map_err(|_| IdeError::Protocol)?;
        let (name, value) = header.trim().split_once(':').ok_or(IdeError::Protocol)?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err(IdeError::Protocol);
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| IdeError::Protocol)?,
            );
        } else if !name.eq_ignore_ascii_case("Content-Type") {
            return Err(IdeError::Protocol);
        }
    }
    let length = length
        .filter(|n| *n > 0 && *n <= MAX_MESSAGE)
        .ok_or(IdeError::Limit)?;
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    let value: Value = serde_json::from_slice(&body).map_err(|_| IdeError::Protocol)?;
    if !value.is_object() {
        return Err(IdeError::Protocol);
    }
    Ok(value)
}

pub fn write_message(output: &mut impl Write, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value).map_err(|_| IdeError::Protocol)?;
    if body.len() > MAX_MESSAGE {
        return Err(IdeError::Limit);
    }
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_bounds_duplicates_and_truncation() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &serde_json::json!({"id":1})).unwrap();
        assert_eq!(read_message(&mut bytes.as_slice()).unwrap()["id"], 1);
        for invalid in [
            "Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            "Content-Length: 9999999\r\n\r\n",
            "Content-Length: 10\r\n\r\n{}",
            "Content-Length: -1\r\n\r\n{}",
            "Content-Length: 2\n\n{}",
        ] {
            assert!(read_message(&mut invalid.as_bytes()).is_err());
        }
    }
}
