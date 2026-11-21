//! The `llm` middleware: redact on the way down, restore (across chunk borders) on the way up.

use super::{mapping::*, redactor::*, rules::*};
use async_trait::async_trait;
use serde_json::Value;
use shellmate_api::{
    ChatRequest, ChatStream, EventStream, Middleware, Next, ProviderError, Result, StreamEvent,
    ToolRequest,
};
use std::collections::VecDeque;

pub struct RedactMiddleware {
    pub(crate) redactor: Redactor,
}

impl RedactMiddleware {
    pub fn new(redactor: Redactor) -> Result<Self> {
        Ok(Self { redactor })
    }
}

#[async_trait]
impl Middleware<ChatRequest, ChatStream> for RedactMiddleware {
    fn id(&self) -> &str {
        "builtin:redact"
    }

    async fn call(
        &self,
        mut request: ChatRequest,
        next: Next<'_, ChatRequest, ChatStream>,
    ) -> Result<ChatStream> {
        let mut map = Mapping::with_key(request.redaction_key);
        for m in &mut request.messages {
            m.content = self.redactor.redact(&m.content, &mut map);
            // what the model asked a tool for is part of the conversation too
            for call in &mut m.tool_calls {
                call.arguments =
                    map_arguments(&call.arguments, &mut |s| self.redactor.redact(s, &mut map));
            }
        }
        let inner = next.run(request).await?;
        Ok(Box::new(RestoreStream {
            inner,
            map,
            held: String::new(),
            pending: VecDeque::new(),
        }))
    }
}

/// Change every string in the JSON text `arguments` with `f`, and write the JSON again. The change is made
/// to the strings, not to the text, so a value that gains characters that JSON escapes (a quote, a
/// backslash) still gives valid JSON. Text that is not JSON is changed as it is.
pub(crate) fn map_arguments(arguments: &str, f: &mut dyn FnMut(&str) -> String) -> String {
    fn walk(v: &mut Value, f: &mut dyn FnMut(&str) -> String) {
        match v {
            Value::String(s) => *s = f(s),
            Value::Array(items) => items.iter_mut().for_each(|i| walk(i, f)),
            Value::Object(map) => {
                // keys are the model's words for the schema, but a key can carry a value as well
                let entries = std::mem::take(map);
                for (k, mut v) in entries {
                    walk(&mut v, f);
                    map.insert(f(&k), v);
                }
            }
            _ => {}
        }
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(mut v) => {
            walk(&mut v, f);
            v.to_string()
        }
        Err(_) => f(arguments),
    }
}

/// How many trailing bytes of `buf` could be the start of a placeholder that is still arriving
/// (`<`, `<SEC`, `<SECRET:ab`): those must wait for the next delta.
pub(crate) fn partial_placeholder_len(buf: &str) -> usize {
    let Some(i) = buf.rfind('<') else { return 0 };
    let tail = &buf[i..];
    let could_be = if tail.len() <= PREFIX.len() {
        PREFIX.starts_with(tail)
    } else {
        tail.starts_with(PREFIX)
            && tail[PREFIX.len()..].bytes().all(|b| b.is_ascii_hexdigit())
            && tail.len() < 80
    };
    if could_be {
        tail.len()
    } else {
        0
    }
}

/// Restores placeholders in a stream, holding back a possible partial placeholder between deltas.
pub(crate) struct RestoreStream {
    pub(crate) inner: ChatStream,
    pub(crate) map: Mapping,
    pub(crate) held: String,
    pub(crate) pending: VecDeque<Result<StreamEvent, ProviderError>>,
}

impl RestoreStream {
    pub(crate) fn flush(&mut self) {
        if !self.held.is_empty() {
            let text = self.map.restore(&std::mem::take(&mut self.held));
            self.pending.push_back(Ok(StreamEvent::Text(text)));
        }
    }
}

#[async_trait]
impl EventStream for RestoreStream {
    async fn next(&mut self) -> Option<Result<StreamEvent, ProviderError>> {
        loop {
            if let Some(ev) = self.pending.pop_front() {
                return Some(ev);
            }
            match self.inner.next().await {
                Some(Ok(StreamEvent::Text(t))) => {
                    self.held.push_str(&t);
                    let keep = partial_placeholder_len(&self.held);
                    let emit_len = self.held.len() - keep;
                    if emit_len > 0 {
                        let rest = self.held.split_off(emit_len);
                        let text = self.map.restore(&std::mem::replace(&mut self.held, rest));
                        return Some(Ok(StreamEvent::Text(text)));
                    }
                }
                Some(Ok(done @ StreamEvent::Done { .. })) => {
                    self.flush();
                    self.pending.push_back(Ok(done));
                }
                Some(Ok(StreamEvent::ToolCall(call))) => {
                    // the words before the call come first, then the call with its placeholders restored
                    self.flush();
                    let arguments = map_arguments(&call.arguments, &mut |s| self.map.restore(s));
                    self.pending
                        .push_back(Ok(StreamEvent::ToolCall(ToolRequest { arguments, ..call })));
                }
                Some(other) => return Some(other),
                None => {
                    self.flush();
                    return self.pending.pop_front();
                }
            }
        }
    }
}
