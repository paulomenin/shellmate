//! Reading a streamed (server-sent events) answer.

use super::wire::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use shellmate_api::{
    EventStream, ProviderError, ProviderErrorKind, StreamEvent, ToolRequest, Usage,
};
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
    /// Calls being put together from the fragments of the deltas, in the order the server numbers them.
    pub(crate) calls: Vec<PartialCall>,
    pub(crate) ended: bool,
}

/// One tool call whose id, name and arguments arrive in pieces.
#[derive(Default)]
pub(crate) struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
pub(crate) struct WireDeltaCall {
    /// Which call the fragment belongs to. A server that leaves it out sends each call whole, or the
    /// fragments of one call in a row.
    index: Option<usize>,
    id: Option<String>,
    #[serde(default)]
    function: WireFunction,
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
    #[serde(default)]
    pub(crate) tool_calls: Vec<WireDeltaCall>,
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
            if let Some(delta) = choice.delta {
                for call in delta.tool_calls {
                    self.add_fragment(call);
                }
                if let Some(text) = delta.content.filter(|t| !t.is_empty()) {
                    self.pending.push_back(Ok(StreamEvent::Text(text)));
                }
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

    /// Put a fragment of a call with the others of the same call.
    fn add_fragment(&mut self, f: WireDeltaCall) {
        let starts_one = f.id.as_deref().is_some_and(|i| !i.is_empty())
            || f.function.name.as_deref().is_some_and(|n| !n.is_empty());
        let at = f.index.unwrap_or(if starts_one || self.calls.is_empty() {
            self.calls.len()
        } else {
            self.calls.len() - 1
        });
        // a server's numbers are small; never let a hostile one allocate a long list
        if at >= 16 {
            return;
        }
        while self.calls.len() <= at {
            self.calls.push(PartialCall::default());
        }
        let call = &mut self.calls[at];
        if let Some(id) = f.id {
            call.id = id;
        }
        if let Some(name) = f.function.name {
            call.name.push_str(&name);
        }
        match f.function.arguments {
            Some(Value::String(s)) => call.arguments.push_str(&s),
            Some(Value::Null) | None => {}
            // a server that sends the arguments as an object sends them whole
            Some(other) => call.arguments = other.to_string(),
        }
    }

    pub(crate) fn end(&mut self) {
        if !self.ended {
            self.ended = true;
            for (i, call) in std::mem::take(&mut self.calls).into_iter().enumerate() {
                if call.name.is_empty() {
                    continue; // fragments of something that never got a name are not a call
                }
                self.pending
                    .push_back(Ok(StreamEvent::ToolCall(ToolRequest {
                        id: if call.id.is_empty() {
                            format!("call_{i}")
                        } else {
                            call.id
                        },
                        name: call.name,
                        arguments: if call.arguments.is_empty() {
                            "{}".into()
                        } else {
                            call.arguments
                        },
                    })));
            }
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
