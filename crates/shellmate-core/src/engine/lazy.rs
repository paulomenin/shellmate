//! A provider whose secrets are fetched on first use.
//!
//! A `cmd:` secret runs a program (`pass`, `op`, ...) that can take a noticeable time, and a fallback
//! provider that is never called should not cost that time. So the engine builds the provider twice:
//! once at start-up with a placeholder in place of the secret (this checks everything else in the
//! settings, and is what [`Provider::info`] answers from), and once when the provider is first asked to
//! do something, with the program's output. Requests only ever go through the second one.

use async_trait::async_trait;
use shellmate_api::{
    ChatRequest, ChatResponse, ChatStream, Error, Provider, ProviderError, ProviderErrorKind,
    ProviderInfo, ToolSupport,
};
use std::sync::{Arc, OnceLock};

type Make = Arc<dyn Fn() -> Result<Arc<dyn Provider>, Error> + Send + Sync>;

pub(super) struct LazyProvider {
    /// Built with placeholders: describes itself, never sends.
    probe: Arc<dyn Provider>,
    make: Make,
    ready: OnceLock<Arc<dyn Provider>>,
}

impl LazyProvider {
    pub(super) fn new(probe: Arc<dyn Provider>, make: Make) -> Self {
        Self {
            probe,
            make,
            ready: OnceLock::new(),
        }
    }

    /// The provider built with the real secrets. The programs run on a blocking thread, so the runtime
    /// keeps going while one waits for a passphrase.
    async fn ready(&self) -> Result<Arc<dyn Provider>, Error> {
        if let Some(p) = self.ready.get() {
            return Ok(p.clone());
        }
        let make = self.make.clone();
        let built = tokio::task::spawn_blocking(move || make())
            .await
            .map_err(|e| {
                Error::new(
                    "config.secret_cmd_failed",
                    format!("secret lookup stopped: {e}"),
                )
            })??;
        Ok(self.ready.get_or_init(|| built).clone())
    }
}

fn other(e: Error) -> ProviderError {
    ProviderError::new(ProviderErrorKind::Other, e.message)
}

#[async_trait]
impl Provider for LazyProvider {
    fn info(&self) -> ProviderInfo {
        self.probe.info()
    }

    async fn prepare(&self) -> Result<(), Error> {
        self.ready().await.map(|_| ())
    }

    async fn tool_support(&self) -> ToolSupport {
        match self.ready().await {
            Ok(p) => p.tool_support().await,
            Err(_) => ToolSupport::Unknown,
        }
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.ready().await.map_err(other)?.chat(request).await
    }

    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, ProviderError> {
        self.ready()
            .await
            .map_err(other)?
            .chat_stream(request)
            .await
    }
}
