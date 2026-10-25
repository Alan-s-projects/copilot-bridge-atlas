#[inline]
pub(crate) fn strip_sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let value = line.strip_prefix(field)?.strip_prefix(':')?;
    Some(value.strip_prefix(' ').unwrap_or(value))
}

#[derive(Default)]
pub(crate) struct SseBuffer {
    text: String,
    start: usize,
    scan: usize,
    utf8_remainder: Vec<u8>,
}

impl From<String> for SseBuffer {
    fn from(text: String) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }
}

impl SseBuffer {
    pub fn as_str(&self) -> &str {
        &self.text[self.start..]
    }

    fn compact(&mut self) {
        if self.start > 0 && self.start >= self.text.len() / 2 {
            self.text.drain(..self.start);
            self.scan -= self.start;
            self.start = 0;
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.compact();
        append_utf8_safe(&mut self.text, &mut self.utf8_remainder, bytes);
    }

    pub fn push_str(&mut self, text: &str) {
        self.compact();
        self.text.push_str(text);
    }

    pub fn flush_utf8(&mut self) {
        let remainder = std::mem::take(&mut self.utf8_remainder);
        self.push_str(&String::from_utf8_lossy(&remainder));
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.start = 0;
        self.scan = 0;
    }

    pub fn next_block(&mut self) -> Option<(&str, &'static str)> {
        let bytes = self.text.as_bytes();
        while self.scan < bytes.len() {
            let remaining = &bytes[self.scan..];
            let delimiter = if remaining.starts_with(b"\r\n\r\n") {
                Some("\r\n\r\n")
            } else if remaining.starts_with(b"\n\n") {
                Some("\n\n")
            } else {
                None
            };
            if let Some(delimiter) = delimiter {
                let start = self.start;
                let end = self.scan;
                self.scan += delimiter.len();
                self.start = self.scan;
                return Some((&self.text[start..end], delimiter));
            }
            self.scan += 1;
        }
        // Only the last three bytes can begin a delimiter split across chunks.
        self.scan = self.text.len().saturating_sub(3).max(self.start);
        None
    }
}

/// Append raw bytes to a UTF-8 `String` buffer, correctly handling multi-byte
/// characters that are split across chunk boundaries.
///
/// `remainder` accumulates trailing bytes from the previous chunk that form an
/// incomplete UTF-8 sequence (at most 3 bytes under normal operation). On each
/// call the remainder is prepended to `new_bytes`, the longest valid UTF-8
/// prefix is appended to `buffer`, and any trailing incomplete bytes are saved
/// back into `remainder` for the next call.
///
/// A defensive guard discards `remainder` via lossy conversion if it ever
/// exceeds 3 bytes, which cannot happen with well-formed UTF-8 streams.
pub(crate) fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, new_bytes: &[u8]) {
    // Build the byte slice to decode: prepend any leftover bytes from previous chunk.
    let (owned, bytes): (Option<Vec<u8>>, &[u8]) = if remainder.is_empty() {
        (None, new_bytes)
    } else {
        // Defensive guard: remainder should never exceed 3 bytes (max incomplete
        // UTF-8 sequence is 3 bytes: a 4-byte char missing its last byte). If it
        // does, the stream is producing genuinely invalid bytes; flush them lossy
        // and start fresh.
        if remainder.len() > 3 {
            buffer.push_str(&String::from_utf8_lossy(remainder));
            remainder.clear();
            (None, new_bytes)
        } else {
            let mut combined = std::mem::take(remainder);
            combined.extend_from_slice(new_bytes);
            (Some(combined), &[])
        }
    };
    let input = owned.as_deref().unwrap_or(bytes);

    // Decode loop: consume all valid UTF-8 and any genuinely invalid bytes,
    // only leaving a trailing incomplete sequence in remainder.
    let mut pos = 0;
    loop {
        match std::str::from_utf8(&input[pos..]) {
            Ok(s) => {
                buffer.push_str(s);
                // Everything consumed – remainder stays empty.
                return;
            }
            Err(e) => {
                let valid_up_to = pos + e.valid_up_to();
                let valid_slice = &input[pos..valid_up_to];
                match std::str::from_utf8(valid_slice) {
                    Ok(valid) => buffer.push_str(valid),
                    Err(_) => buffer.push_str(&String::from_utf8_lossy(valid_slice)),
                }
                if let Some(invalid_len) = e.error_len() {
                    // Genuinely invalid byte(s) – emit U+FFFD and continue.
                    buffer.push('\u{FFFD}');
                    pos = valid_up_to + invalid_len;
                } else {
                    // Incomplete trailing sequence – stash for next chunk.
                    *remainder = input[valid_up_to..].to_vec();
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{append_utf8_safe, strip_sse_field, SseBuffer};

    #[test]
    fn strip_sse_field_accepts_optional_space() {
        assert_eq!(
            strip_sse_field("data: {\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            strip_sse_field("data:{\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            strip_sse_field("event: message_start", "event"),
            Some("message_start")
        );
        assert_eq!(
            strip_sse_field("event:message_start", "event"),
            Some("message_start")
        );
        assert_eq!(strip_sse_field("id:1", "data"), None);
    }

    #[test]
    fn take_sse_block_supports_lf_delimiters() {
        let mut buffer = SseBuffer::from("data: {\"ok\":true}\n\nrest".to_string());

        assert_eq!(buffer.next_block(), Some(("data: {\"ok\":true}", "\n\n")));
        assert_eq!(buffer.as_str(), "rest");
    }

    #[test]
    fn take_sse_block_supports_crlf_delimiters() {
        let mut buffer = SseBuffer::from("data: {\"ok\":true}\r\n\r\nrest".to_string());

        assert_eq!(
            buffer.next_block(),
            Some(("data: {\"ok\":true}", "\r\n\r\n"))
        );
        assert_eq!(buffer.as_str(), "rest");
    }

    #[test]
    fn buffer_preserves_split_delimiters_and_utf8_after_compaction() {
        let mut buffer = SseBuffer::default();
        buffer.push_bytes(b"data: first\n\ndata: second\r");
        assert_eq!(buffer.next_block(), Some(("data: first", "\n\n")));
        assert_eq!(buffer.next_block(), None);
        buffer.push_bytes(b"\n\r");
        assert_eq!(buffer.next_block(), None);
        buffer.push_bytes(b"\ndata: \xe2\x82");
        assert_eq!(buffer.next_block(), Some(("data: second", "\r\n\r\n")));
        assert_eq!(buffer.next_block(), None);
        buffer.push_bytes(b"\xac\n\ntail");
        assert_eq!(buffer.next_block(), Some(("data: €", "\n\n")));
        assert_eq!(buffer.next_block(), None);
        assert_eq!(buffer.as_str(), "tail");
    }

    // ------------------------------------------------------------------
    // append_utf8_safe tests
    // ------------------------------------------------------------------

    #[test]
    fn ascii_passthrough() {
        let mut buf = String::new();
        let mut rem = Vec::new();
        append_utf8_safe(&mut buf, &mut rem, b"hello world");
        assert_eq!(buf, "hello world");
        assert!(rem.is_empty());
    }

    #[test]
    fn complete_multibyte_in_single_chunk() {
        let mut buf = String::new();
        let mut rem = Vec::new();
        append_utf8_safe(&mut buf, &mut rem, "Héllo Wørld".as_bytes());
        assert_eq!(buf, "Héllo Wørld");
        assert!(rem.is_empty());
    }

    #[test]
    fn split_multibyte_across_two_chunks() {
        // "€" = E2 82 AC (3 bytes)
        let bytes = "€".as_bytes();
        assert_eq!(bytes.len(), 3);

        let mut buf = String::new();
        let mut rem = Vec::new();

        // Chunk 1: first 2 bytes (incomplete)
        append_utf8_safe(&mut buf, &mut rem, &bytes[..2]);
        assert_eq!(buf, "");
        assert_eq!(rem.len(), 2);

        // Chunk 2: last byte completes the character
        append_utf8_safe(&mut buf, &mut rem, &bytes[2..]);
        assert_eq!(buf, "€");
        assert!(rem.is_empty());
    }

    #[test]
    fn split_four_byte_char_across_chunks() {
        // 😀 = F0 9F 98 80 (4 bytes)
        let bytes = "😀".as_bytes();
        assert_eq!(bytes.len(), 4);

        let mut buf = String::new();
        let mut rem = Vec::new();

        // Send 1 byte at a time
        append_utf8_safe(&mut buf, &mut rem, &bytes[..1]);
        assert_eq!(buf, "");
        assert_eq!(rem.len(), 1);

        append_utf8_safe(&mut buf, &mut rem, &bytes[1..2]);
        assert_eq!(buf, "");
        assert_eq!(rem.len(), 2);

        append_utf8_safe(&mut buf, &mut rem, &bytes[2..3]);
        assert_eq!(buf, "");
        assert_eq!(rem.len(), 3);

        append_utf8_safe(&mut buf, &mut rem, &bytes[3..]);
        assert_eq!(buf, "😀");
        assert!(rem.is_empty());
    }

    #[test]
    fn mixed_ascii_and_split_multibyte() {
        // "hi€" = 68 69 E2 82 AC
        let all = "hi€".as_bytes();
        assert_eq!(all.len(), 5);

        let mut buf = String::new();
        let mut rem = Vec::new();

        // Chunk 1: "hi" + first byte of "€"
        append_utf8_safe(&mut buf, &mut rem, &all[..3]);
        assert_eq!(buf, "hi");
        assert_eq!(rem.len(), 1);

        // Chunk 2: remaining 2 bytes of "€"
        append_utf8_safe(&mut buf, &mut rem, &all[3..]);
        assert_eq!(buf, "hi€");
        assert!(rem.is_empty());
    }

    #[test]
    fn multiple_split_characters_in_sequence() {
        let text = "€≈";
        let bytes = text.as_bytes(); // E2 82 AC E2 89 88

        let mut buf = String::new();
        let mut rem = Vec::new();

        // Split in the middle: first char complete + 1 byte of second
        append_utf8_safe(&mut buf, &mut rem, &bytes[..4]);
        assert_eq!(buf, "€");
        assert_eq!(rem.len(), 1);

        // Remaining 2 bytes complete second char
        append_utf8_safe(&mut buf, &mut rem, &bytes[4..]);
        assert_eq!(buf, "€≈");
        assert!(rem.is_empty());
    }

    #[test]
    fn empty_chunks_are_harmless() {
        let mut buf = String::new();
        let mut rem = Vec::new();

        append_utf8_safe(&mut buf, &mut rem, b"");
        assert_eq!(buf, "");
        assert!(rem.is_empty());

        append_utf8_safe(&mut buf, &mut rem, b"ok");
        assert_eq!(buf, "ok");

        append_utf8_safe(&mut buf, &mut rem, b"");
        assert_eq!(buf, "ok");
    }

    #[test]
    fn sse_json_with_multibyte_split_at_boundary() {
        // Simulate an SSE data line with multibyte UTF-8 split across chunks.
        let json_line = "data: {\"text\":\"€≈\"}\n\n";
        let bytes = json_line.as_bytes();

        // Find where "€" starts in the byte stream and split there.
        let euro_start = bytes.windows(3).position(|w| w == "€".as_bytes()).unwrap();
        let split_point = euro_start + 1; // split inside "€"

        let mut buf = String::new();
        let mut rem = Vec::new();

        append_utf8_safe(&mut buf, &mut rem, &bytes[..split_point]);
        append_utf8_safe(&mut buf, &mut rem, &bytes[split_point..]);

        assert_eq!(buf, json_line);
        assert!(rem.is_empty());

        // Verify the buffer can be parsed as SSE with valid JSON
        let data = strip_sse_field(buf.lines().next().unwrap(), "data").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(parsed["text"], "€≈");
    }

    #[test]
    fn invalid_bytes_flushed_immediately_not_accumulated() {
        // 0xFF is never valid in UTF-8 – it should be replaced immediately,
        // not stashed in remainder.
        let mut buf = String::new();
        let mut rem = Vec::new();

        // "hi" + invalid byte + "ok"
        append_utf8_safe(&mut buf, &mut rem, b"hi\xFFok");
        assert!(
            rem.is_empty(),
            "remainder should be empty after invalid byte"
        );
        assert!(buf.contains("hi"), "valid prefix must be present");
        assert!(buf.contains("ok"), "valid suffix must be present");
        assert!(buf.contains('\u{FFFD}'), "invalid byte must produce U+FFFD");
    }

    #[test]
    fn invalid_byte_in_slow_path_flushed_immediately() {
        let mut buf = String::new();
        let mut rem = Vec::new();

        // Prime remainder with an incomplete sequence (first byte of "€").
        append_utf8_safe(&mut buf, &mut rem, &"€".as_bytes()[..1]);
        assert_eq!(rem.len(), 1);

        // Next chunk starts with an invalid byte – the stale remainder and the
        // invalid byte should both be flushed, not accumulated.
        append_utf8_safe(&mut buf, &mut rem, b"\xFFworld");
        assert!(rem.is_empty(), "remainder should be empty");
        assert!(
            buf.contains("world"),
            "valid data after invalid byte must appear"
        );
    }

    #[test]
    fn defensive_guard_flushes_oversized_remainder() {
        let mut buf = String::new();
        let mut rem = Vec::new();

        // Manually inject 4 invalid bytes into remainder to trigger the >3 guard.
        // This can't happen with well-formed UTF-8, but tests the safety net.
        rem.extend_from_slice(b"\x80\x80\x80\x80");
        assert_eq!(rem.len(), 4);

        append_utf8_safe(&mut buf, &mut rem, b"hello");
        // The 4 invalid bytes should have been flushed lossy, then "hello" decoded.
        assert!(rem.is_empty(), "remainder must be empty after guard flush");
        assert!(
            buf.contains("hello"),
            "valid data after guard flush must appear"
        );
        // The 4 invalid bytes each produce a U+FFFD
        let replacement_count = buf.chars().filter(|&c| c == '\u{FFFD}').count();
        assert_eq!(
            replacement_count, 4,
            "each invalid byte should produce one U+FFFD"
        );
    }
}
