use crate::{stream_of_response, ChatRequest, ChatResponse, ChatStream, Error};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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

impl Error {
    /// [`ProviderError::falls_back`] for an error that has already been converted: the engine only has the
    /// `Error`, and the codes are the ones `From<ProviderError>` gives. A test keeps the two in step.
    pub fn allows_fallback(&self) -> bool {
        matches!(
            self.code,
            "provider.timeout" | "provider.unavailable" | "provider.rate_limited"
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

/// What a provider knows about native tool calling for its model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSupport {
    Yes,
    No,
    /// The server does not say. Never read as "no": it only means the answer is not known.
    Unknown,
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

    /// The models the server offers, to confirm the connection and the key before a request (used by
    /// `setup`). Carries no prompt or context, only the credentials. `None` means this provider cannot list
    /// models, and `Some(vec![])` that it answered but its reply could not be read (see
    /// [`parse_model_ids`](crate::parse_model_ids)). The default cannot list.
    async fn models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        Ok(None)
    }

    /// Whether the model takes tools natively (`ChatRequest::tools`). Used when the user left the choice to
    /// `auto`: the answer is cached, so a network call here happens rarely. A provider that cannot tell says
    /// [`ToolSupport::Unknown`] (the default), and the tools are then offered in the prompt text.
    async fn tool_support(&self) -> ToolSupport {
        ToolSupport::Unknown
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// Streaming variant. Providers without streaming get this default: the whole answer as one event.
    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, ProviderError> {
        self.chat(request).await.map(stream_of_response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_converted_error_allows_fallback_exactly_when_the_provider_error_does() {
        use ProviderErrorKind::*;
        for kind in [
            Timeout,
            Unavailable,
            RateLimited,
            Auth,
            BadRequest,
            ContentFiltered,
            Other,
        ] {
            let e = ProviderError::new(kind, "x");
            let allowed = e.falls_back();
            assert_eq!(Error::from(e).allows_fallback(), allowed, "{kind:?}");
        }
        // an error that did not come from a provider never falls back
        assert!(!Error::new("config.invalid", "x").allows_fallback());
    }
}
