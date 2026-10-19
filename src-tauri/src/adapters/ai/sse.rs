//! Server-sent events, parsed incrementally from arbitrary byte chunks. Pure and bounded: a hostile or broken server cannot make
//! it allocate without limit.

use crate::domain::ai::{AiError, AiErrorKind, AiResult, MAX_SSE_EVENT_BYTES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    data_bytes: usize,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk; returns the events that became complete.
    pub fn feed(&mut self, bytes: &[u8]) -> AiResult<Vec<SseEvent>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some((end, skip)) = next_line_end(&self.buf) else { break };
            let line: Vec<u8> = self.buf.drain(..end + skip).take(end).collect();
            self.line(&line, &mut out)?;
        }
        if self.buf.len() > MAX_SSE_EVENT_BYTES {
            return Err(AiError::new(AiErrorKind::TooLarge, "the provider sent a single line larger than Vixalune allows"));
        }
        Ok(out)
    }

    /// The stream ended: an event without a final blank line still counts.
    pub fn finish(&mut self) -> AiResult<Option<SseEvent>> {
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            let mut out = Vec::new();
            self.line(&line, &mut out)?;
            if let Some(e) = out.into_iter().next() {
                return Ok(Some(e));
            }
        }
        Ok(self.dispatch())
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<SseEvent>) -> AiResult<()> {
        if line.is_empty() {
            if let Some(e) = self.dispatch() {
                out.push(e);
            }
            return Ok(());
        }
        if line[0] == b':' {
            return Ok(()); // comment / keep-alive
        }
        let text = String::from_utf8_lossy(line);
        let (name, value) = match text.split_once(':') {
            Some((n, v)) => (n, v.strip_prefix(' ').unwrap_or(v)),
            None => (text.as_ref(), ""),
        };
        match name {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data_bytes += value.len() + 1;
                if self.data_bytes > MAX_SSE_EVENT_BYTES {
                    return Err(AiError::new(AiErrorKind::TooLarge, "the provider sent a single event larger than Vixalune allows"));
                }
                self.data.push(value.to_string());
            }
            _ => {} // id, retry and unknown fields are ignored
        }
        Ok(())
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        let data = std::mem::take(&mut self.data);
        self.data_bytes = 0;
        if data.is_empty() {
            return None;
        }
        Some(SseEvent { event, data: data.join("\n") })
    }
}

/// Position of the next line terminator and its length. A lone `\r` at the very end waits for a possible `\n`.
fn next_line_end(buf: &[u8]) -> Option<(usize, usize)> {
    for (i, b) in buf.iter().enumerate() {
        match b {
            b'\n' => return Some((i, 1)),
            b'\r' => {
                return match buf.get(i + 1) {
                    Some(b'\n') => Some((i, 2)),
                    Some(_) => Some((i, 1)),
                    None => None,
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::new();
        let mut out = vec![];
        for c in chunks {
            out.extend(p.feed(c).unwrap());
        }
        out.extend(p.finish().unwrap());
        out
    }

    #[test]
    fn parses_named_events_comments_multiline_data_and_all_line_endings() {
        let e = all(&[b": keep-alive\n\nevent: message_start\ndata: {\"a\":1}\n\ndata: x\ndata: y\n\n"]);
        assert_eq!(e, vec![SseEvent { event: Some("message_start".into()), data: "{\"a\":1}".into() }, SseEvent { event: None, data: "x\ny".into() }]);
        assert_eq!(all(&[b"data: a\r\n\r\ndata: b\r\rdata: c\n\n"]).iter().map(|e| e.data.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    }

    #[test]
    fn events_split_across_chunks_at_any_byte_are_reassembled() {
        let whole = "event: content_block_delta\ndata: {\"text\":\"h\u{e9}llo\"}\n\ndata: [DONE]\n\n".as_bytes();
        for cut in 1..whole.len() {
            let got = all(&[&whole[..cut], &whole[cut..]]);
            assert_eq!(got.len(), 2, "cut at {cut}");
            assert_eq!(got[0].data, "{\"text\":\"h\u{e9}llo\"}");
            assert_eq!(got[1].data, "[DONE]");
        }
    }

    #[test]
    fn a_final_event_without_a_blank_line_is_not_lost() {
        assert_eq!(all(&[b"data: last"]).len(), 1);
        assert_eq!(all(&[b"data: last\n"]).len(), 1);
        assert!(all(&[b": only a comment\n"]).is_empty());
    }

    #[test]
    fn oversized_lines_and_events_are_refused() {
        let mut p = SseParser::new();
        let e = p.feed(&vec![b'a'; MAX_SSE_EVENT_BYTES + 10]).unwrap_err();
        assert_eq!(e.kind, AiErrorKind::TooLarge);
        let mut p = SseParser::new();
        let line = format!("data: {}\n", "b".repeat(300_000));
        let mut failed = false;
        for _ in 0..5 {
            if p.feed(line.as_bytes()).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "an event made of many large data lines must hit the limit");
    }

    proptest! {
        /// Whatever bytes arrive, in whatever chunks, the parser neither panics nor loses the framing of well-formed events.
        #[test]
        fn never_panics_on_arbitrary_bytes(chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..64), 0..12)) {
            let mut p = SseParser::new();
            for c in &chunks {
                let _ = p.feed(c);
            }
            let _ = p.finish();
        }

        #[test]
        fn chunking_does_not_change_the_result(payloads in prop::collection::vec("[a-z0-9 ]{1,20}", 1..6), cuts in prop::collection::vec(1usize..8, 1..10)) {
            let stream: String = payloads.iter().map(|p| format!("data: {p}\n\n")).collect();
            let bytes = stream.as_bytes();
            let mut p = SseParser::new();
            let mut got = vec![];
            let mut at = 0;
            let mut i = 0;
            while at < bytes.len() {
                let n = cuts[i % cuts.len()].min(bytes.len() - at);
                got.extend(p.feed(&bytes[at..at + n]).unwrap());
                at += n;
                i += 1;
            }
            let want: Vec<String> = payloads.iter().map(|s| s.to_string()).collect();
            prop_assert_eq!(got.iter().map(|e| e.data.clone()).collect::<Vec<_>>(), want);
        }
    }
}
