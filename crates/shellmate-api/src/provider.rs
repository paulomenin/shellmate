use crate::{stream_of_response, ChatRequest, ChatResponse, ChatStream, Error};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    pub plugin: String,
    pub model: String,
    /// Where requests go (no credentials), when the provider talks to a network endpoint.
    pub endpoint: Option<String>,
}

/// Error classes that decide fallback behavior (`providers-prompts.md` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    Timeout,
    Unavailable,
    RateLimited,
    Auth,
    BadRequest,
    ContentFiltered,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub message: String,
}

impl ProviderError {
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Whether the next provider in a fallback chain may be tried.
    pub fn falls_back(&self) -> bool {
        matches!(
            self.kind,
            ProviderErrorKind::Timeout
                | ProviderErrorKind::Unavailable
                | ProviderErrorKind::RateLimited
        )
    }
}

impl From<ProviderError> for Error {
    fn from(e: ProviderError) -> Self {
        let (code, hint) = match e.kind {
            ProviderErrorKind::Timeout => (
                "provider.timeout",
                Some("raise `runtime.request_timeout` or check the endpoint"),
            ),
            ProviderErrorKind::Unavailable => (
                "provider.unavailable",
                Some("is the server running and the endpoint correct?"),
            ),
            ProviderErrorKind::RateLimited => (
                "provider.rate_limited",
                Some("wait a bit or configure a fallback provider"),
            ),
            ProviderErrorKind::Auth => (
                "provider.auth",
                Some("check the API key reference in the provider settings"),
            ),
            ProviderErrorKind::BadRequest => ("provider.bad_request", None),
            ProviderErrorKind::ContentFiltered => ("provider.content_filtered", None),
            ProviderErrorKind::Other => ("provider.error", None),
        };
        let err = Error::new(code, e.message);
        match hint {
            Some(h) => err.with_hint(h),
            None => err,
        }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn info(&self) -> ProviderInfo;

    /// Get ready for the first request. A provider whose credentials are fetched on first use (a `cmd:`
    /// secret) does that here, so a failure is reported as what it is (a configuration error with its own
    /// code and hint) and before anything is sent. The default does nothing.
    async fn prepare(&self) -> Result<(), Error> {
        Ok(())
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// Streaming variant. Providers without streaming get this default: the whole answer as one event.
    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, ProviderError> {
        self.chat(request).await.map(stream_of_response)
    }
}
