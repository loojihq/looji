//! Server-sent events, decoded from raw bytes.
//!
//! The bytes are buffered until a whole line has arrived, and only then is the line decoded as UTF-8. A chunk
//! can end in the middle of a line, or in the middle of a multi-byte character, so decoding each chunk on its
//! own would corrupt text. Both `\n` and `\r\n` end a line.

use crate::error::AiError;

/// One event: its name if it had one, and its data lines joined with newlines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a chunk from the network and returns every event it completes. A partial line stays buffered until
    /// its end arrives in a later chunk.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, AiError> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = &line[..line.len() - 1];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let text = std::str::from_utf8(line)
                .map_err(|_| AiError::Protocol("the stream was not valid UTF-8".to_string()))?;
            self.line(text, &mut events);
        }
        Ok(events)
    }

    fn line(&mut self, line: &str, events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            // A blank line ends an event. A blank line with no data in it ends nothing.
            if !self.data.is_empty() {
                events.push(SseEvent {
                    event: self.event.take(),
                    data: self.data.join("\n"),
                });
                self.data.clear();
            }
            self.event = None;
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{SseDecoder, SseEvent};

    fn event(name: &str, data: &str) -> SseEvent {
        SseEvent {
            event: Some(name.to_string()),
            data: data.to_string(),
        }
    }

    #[test]
    fn an_event_is_complete_only_at_its_blank_line() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(b"event: ping\ndata: {}\n").unwrap().is_empty());
        assert_eq!(decoder.push(b"\n").unwrap(), vec![event("ping", "{}")]);
    }

    #[test]
    fn a_line_split_across_chunks_is_joined_before_it_is_read() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(b"event: pi").unwrap().is_empty());
        assert!(decoder.push(b"ng\ndata: {}").unwrap().is_empty());
        assert_eq!(decoder.push(b"\n\n").unwrap(), vec![event("ping", "{}")]);
    }

    #[test]
    fn a_multi_byte_character_split_across_chunks_is_not_corrupted() {
        // "é" is two bytes and "😀" is four. Split them in the middle, one byte at a time.
        let line = "event: t\ndata: café 😀\n\n".as_bytes();
        let mut decoder = SseDecoder::new();
        let mut events = Vec::new();
        for byte in line {
            events.extend(decoder.push(std::slice::from_ref(byte)).unwrap());
        }
        assert_eq!(events, vec![event("t", "café 😀")]);
    }

    #[test]
    fn crlf_line_endings_are_read_the_same_as_lf() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push(b"event: ping\r\ndata: {}\r\n\r\n").unwrap();
        assert_eq!(events, vec![event("ping", "{}")]);
    }

    #[test]
    fn several_events_in_one_chunk_come_out_in_order() {
        let mut decoder = SseDecoder::new();
        let events = decoder
            .push(b"event: a\ndata: 1\n\nevent: b\ndata: 2\n\n")
            .unwrap();
        assert_eq!(events, vec![event("a", "1"), event("b", "2")]);
    }

    #[test]
    fn comments_and_unknown_fields_are_ignored() {
        let mut decoder = SseDecoder::new();
        let events = decoder
            .push(b": keep-alive\nid: 7\nevent: a\ndata: x\n\n")
            .unwrap();
        assert_eq!(events, vec![event("a", "x")]);
    }

    #[test]
    fn invalid_utf8_on_a_complete_line_is_a_protocol_error() {
        let mut decoder = SseDecoder::new();
        assert!(matches!(
            decoder.push(b"data: \xff\n"),
            Err(crate::error::AiError::Protocol(_))
        ));
    }
}
