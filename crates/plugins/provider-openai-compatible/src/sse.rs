//! Reading a streamed (server-sent events) answer.

use crate::wire::*;
use async_trait::async_trait;
use serde::Deserialize;
use shellmate_api::{EventStream, ProviderError, ProviderErrorKind, StreamEvent, Usage};
use std::collections::VecDeque;

/// Server-sent events from `/chat/completions` with `stream: true`.
///
/// Framing rules (`spike-streaming.md`): buffer raw bytes and decode *complete* events only (a JSON body
/// or a UTF-8 character may be split across network chunks); events end at a blank line (`\n\n` or
/// `\r\n\r\n`); comment lines (`: keepalive`) are ignored; `[DONE]` ends the stream. Dropping the
/// stream drops the response and closes the connection, which stops generation at the provider.
pub(crate) struct SseStream {
    pub(crate) resp: reqwest::Response,
    pub(crate) buf: Vec<u8>,
    pub(crate) pending: VecDeque<Result<StreamEvent, ProviderError>>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) ended: bool,
}

#[derive(Deserialize)]
pub(crate) struct WireChunk {
    #[serde(default)]
    pub(crate) choices: Vec<WireChunkChoice>,
    pub(crate) usage: Option<WireUsage>,
}

#[derive(Deserialize)]
pub(crate) struct WireChunkChoice {
    pub(crate) delta: Option<WireDelta>,
    pub(crate) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireDelta {
    pub(crate) content: Option<String>,
}

impl SseStream {
    /// Move every complete event out of the byte buffer into `pending`.
    pub(crate) fn drain_events(&mut self) {
        loop {
            let found = find(&self.buf, b"\n\n")
                .map(|p| (p, 2))
                .into_iter()
                .chain(find(&self.buf, b"\r\n\r\n").map(|p| (p, 4)))
                .min_by_key(|(p, _)| *p);
            let Some((pos, sep)) = found else { return };
            let raw: Vec<u8> = self.buf.drain(..pos + sep).collect();
            let event = String::from_utf8_lossy(&raw[..pos]).into_owned();
            self.handle_event(&event);
        }
    }

    pub(crate) fn handle_event(&mut self, event: &str) {
        let mut data = String::new();
        for line in event.lines() {
            if let Some(d) = line.strip_prefix("data:") {
                data.push_str(d.strip_prefix(' ').unwrap_or(d));
            }
        }
        if data.is_empty() {
            return; // comment or keepalive
        }
        if data.trim() == "[DONE]" {
            self.end();
            return;
        }
        let Ok(chunk) = serde_json::from_str::<WireChunk>(&data) else {
            return; // unknown event shape: ignore rather than abort the answer
        };
        for choice in chunk.choices {
            if let Some(text) = choice
                .delta
                .and_then(|d| d.content)
                .filter(|t| !t.is_empty())
            {
                self.pending.push_back(Ok(StreamEvent::Text(text)));
            }
            if choice.finish_reason.is_some() {
                self.finish_reason = choice.finish_reason;
            }
        }
        if let Some(u) = chunk.usage {
            self.pending.push_back(Ok(StreamEvent::Usage(Usage {
                input_tokens: u.prompt_tokens.unwrap_or(0),
                output_tokens: u.completion_tokens.unwrap_or(0),
            })));
        }
    }

    pub(crate) fn end(&mut self) {
        if !self.ended {
            self.ended = true;
            self.pending.push_back(Ok(StreamEvent::Done {
                finish_reason: self.finish_reason.take(),
            }));
        }
    }
}

pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[async_trait]
impl EventStream for SseStream {
    async fn next(&mut self) -> Option<Result<StreamEvent, ProviderError>> {
        loop {
            if let Some(ev) = self.pending.pop_front() {
                return Some(ev);
            }
            if self.ended {
                return None;
            }
            match self.resp.chunk().await {
                Ok(Some(bytes)) => {
                    self.buf.extend_from_slice(&bytes);
                    self.drain_events();
                }
                Ok(None) => {
                    // connection closed: a final event without a trailing blank line still counts
                    if !self.buf.is_empty() {
                        let rest =
                            String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned();
                        self.handle_event(&rest);
                    }
                    self.end();
                }
                Err(e) => {
                    self.ended = true;
                    return Some(Err(ProviderError::new(
                        ProviderErrorKind::Unavailable,
                        format!("stream interrupted: {}", e.without_url()),
                    )));
                }
            }
        }
    }
}
