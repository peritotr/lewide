//! Server-sent events, as every provider here streams its replies.

/// One event: the `event:` name if any, and its `data:` lines joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Turns a byte stream that arrives in arbitrary pieces into events.
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    /// Adds received bytes and returns the events they complete. Bytes are
    /// kept as bytes until an event is whole, so a multi-byte character split
    /// across two pieces is never decoded in halves.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some((end, separator)) = find_blank_line(&self.buffer) {
            let block: Vec<u8> = self.buffer.drain(..end + separator).collect();
            if let Some(event) = parse_block(&String::from_utf8_lossy(&block[..end])) {
                events.push(event);
            }
        }
        events
    }
}

/// Position and length of the first blank line (`\n\n` or `\r\n\r\n`).
fn find_blank_line(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn parse_block(block: &str) -> Option<SseEvent> {
    let mut event = None;
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        } else if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_owned());
        }
        // Comment lines (":keep-alive") and other fields are ignored.
    }
    if data.is_empty() {
        return None;
    }
    Some(SseEvent {
        event,
        data: data.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(parser: &mut SseParser, pieces: &[&[u8]]) -> Vec<SseEvent> {
        pieces.iter().flat_map(|p| parser.push(p)).collect()
    }

    #[test]
    fn parses_events_with_and_without_names() {
        let mut parser = SseParser::default();
        let events = parser.push(b"event: ping\ndata: {\"a\":1}\n\ndata: [DONE]\n\n");
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: Some("ping".into()),
                    data: "{\"a\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "[DONE]".into()
                },
            ]
        );
    }

    #[test]
    fn waits_for_the_end_of_an_event() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"data: {\"text\":\"hel").is_empty());
        assert!(parser.push(b"lo\"}\n").is_empty());
        assert_eq!(parser.push(b"\n")[0].data, "{\"text\":\"hello\"}");
    }

    #[test]
    fn a_character_split_across_pieces_survives() {
        let whole = "data: ğüş ✓\n\n".as_bytes();
        for cut in 1..whole.len() - 1 {
            let mut parser = SseParser::default();
            let events = all(&mut parser, &[&whole[..cut], &whole[cut..]]);
            assert_eq!(events.len(), 1, "cut at {cut}");
            assert_eq!(events[0].data, "ğüş ✓", "cut at {cut}");
        }
    }

    #[test]
    fn handles_crlf_comments_and_multi_line_data() {
        let mut parser = SseParser::default();
        let events = parser.push(b": keep-alive\r\n\r\ndata: one\r\ndata: two\r\n\r\n");
        assert_eq!(
            events,
            vec![SseEvent {
                event: None,
                data: "one\ntwo".into()
            }]
        );
    }
}
