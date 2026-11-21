//! Streaming responses. Runtime-agnostic: a hand-rolled async `next()` trait instead of a futures `Stream`.

use crate::{ChatResponse, ProviderError, ToolRequest, Usage};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// A piece of the answer text.
    Text(String),
    /// A complete native tool call. Providers send it whole (they put the argument fragments together
    /// first), after the text and before `Done`.
    ToolCall(ToolRequest),
    Usage(Usage),
    /// End of the answer; `finish_reason` as reported by the provider.
    Done {
        finish_reason: Option<String>,
    },
}

/// A response being produced. Dropping it cancels the request (the connection is closed).
#[async_trait]
pub trait EventStream: Send {
    /// `None` after the stream ended.
    async fn next(&mut self) -> Option<Result<StreamEvent, ProviderError>>;
}

/// What the `llm` stage returns: a boxed stream of events.
pub type ChatStream = Box<dyn EventStream>;

/// A stream over a fixed list of events (non-streaming providers, tests).
pub struct VecStream {
    events: std::vec::IntoIter<Result<StreamEvent, ProviderError>>,
}

#[async_trait]
impl EventStream for VecStream {
    async fn next(&mut self) -> Option<Result<StreamEvent, ProviderError>> {
        self.events.next()
    }
}

pub fn stream_from(events: Vec<Result<StreamEvent, ProviderError>>) -> ChatStream {
    Box::new(VecStream {
        events: events.into_iter(),
    })
}

/// The events of an already complete response.
pub fn stream_of_response(r: ChatResponse) -> ChatStream {
    let mut ev = Vec::new();
    if !r.text.is_empty() {
        ev.push(Ok(StreamEvent::Text(r.text)));
    }
    for call in r.tool_calls {
        ev.push(Ok(StreamEvent::ToolCall(call)));
    }
    if let Some(u) = r.usage {
        ev.push(Ok(StreamEvent::Usage(u)));
    }
    ev.push(Ok(StreamEvent::Done {
        finish_reason: r.finish_reason,
    }));
    stream_from(ev)
}

/// Drain a stream into a complete response.
pub async fn collect(mut stream: ChatStream) -> Result<ChatResponse, ProviderError> {
    let mut out = ChatResponse {
        text: String::new(),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: None,
    };
    while let Some(ev) = stream.next().await {
        match ev? {
            StreamEvent::Text(t) => out.text.push_str(&t),
            StreamEvent::ToolCall(c) => out.tool_calls.push(c),
            StreamEvent::Usage(u) => out.usage = Some(u),
            StreamEvent::Done { finish_reason } => out.finish_reason = finish_reason,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collect_joins_text_and_keeps_metadata() {
        let s = stream_from(vec![
            Ok(StreamEvent::Text("he".into())),
            Ok(StreamEvent::Text("llo".into())),
            Ok(StreamEvent::Usage(Usage {
                input_tokens: 1,
                output_tokens: 2,
            })),
            Ok(StreamEvent::Done {
                finish_reason: Some("stop".into()),
            }),
        ]);
        let r = collect(s).await.unwrap();
        assert_eq!(
            (r.text.as_str(), r.finish_reason.as_deref()),
            ("hello", Some("stop"))
        );
        assert_eq!(
            r.usage,
            Some(Usage {
                input_tokens: 1,
                output_tokens: 2
            })
        );
    }

    #[tokio::test]
    async fn errors_propagate() {
        let s = stream_from(vec![
            Ok(StreamEvent::Text("a".into())),
            Err(ProviderError::new(
                crate::ProviderErrorKind::Unavailable,
                "cut",
            )),
        ]);
        assert_eq!(collect(s).await.unwrap_err().message, "cut");
    }
}
