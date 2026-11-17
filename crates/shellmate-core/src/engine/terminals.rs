//! The innermost handlers of the `llm` middleware chain.

use async_trait::async_trait;
use shellmate_api::{ChatRequest, ChatStream, Error, Provider, Result, Terminal};
use std::sync::Mutex;

pub(super) struct ProviderTerminal<'a>(pub(super) &'a dyn Provider);

#[async_trait]
impl Terminal<ChatRequest, ChatStream> for ProviderTerminal<'_> {
    async fn call(&self, request: ChatRequest) -> Result<ChatStream> {
        self.0.chat_stream(&request).await.map_err(Error::from)
    }
}

/// Records the request that reaches the end of the chain instead of sending it anywhere. A middleware may
/// answer by itself without passing the request on, so there can be none.
#[derive(Default)]
pub(super) struct Recorder(Mutex<Option<ChatRequest>>);

impl Recorder {
    /// The request that arrived, if any layer passed one on.
    pub(super) fn take(&self) -> Option<ChatRequest> {
        self.0.lock().expect("recorder lock").take()
    }
}

#[async_trait]
impl Terminal<ChatRequest, ChatStream> for Recorder {
    async fn call(&self, request: ChatRequest) -> Result<ChatStream> {
        *self.0.lock().expect("recorder lock") = Some(request);
        Ok(shellmate_api::stream_from(Vec::new()))
    }
}
