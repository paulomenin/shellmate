//! The non-streaming and streaming client for `/chat/completions`.

use super::{presets::*, settings::*, sse::*, wire::*, *};
use async_trait::async_trait;
use shellmate_api::{
    ChatRequest, ChatResponse, ChatStream, Error, Provider, ProviderError, ProviderErrorKind,
    ProviderInfo, Secret, Usage,
};
use std::collections::VecDeque;
use std::time::Duration;

pub struct OpenAiCompatible {
    pub(crate) client: reqwest::Client,
    pub(crate) url: String,
    pub(crate) model: String,
    pub(crate) api_key: Option<Secret>,
    pub(crate) reasoning_effort: Option<String>,
}

impl OpenAiCompatible {
    /// Build from the settings table. A hosted preset's conventional key variable is read from the process
    /// environment.
    pub fn from_settings(value: &serde_json::Value) -> Result<Self, Error> {
        Self::from_settings_with(value, &|name| std::env::var(name).ok())
    }

    /// Like [`OpenAiCompatible::from_settings`], reading environment variables through `lookup`.
    pub fn from_settings_with(
        value: &serde_json::Value,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, Error> {
        let s: Settings = serde_json::from_value(value.clone())
            .map_err(|e| Error::new("config.invalid", format!("provider settings: {e}")))?;
        let preset = s
            .preset
            .as_deref()
            .and_then(|n| PRESETS.iter().find(|p| p.name == n));
        let endpoint = match (&s.endpoint, &s.preset) {
            (Some(e), _) => e.clone(),
            (None, Some(p)) => preset.map(|p| p.endpoint.to_string()).ok_or_else(|| {
                let names: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
                Error::new("config.invalid", format!("unknown preset `{p}`"))
                    .with_hint(format!("presets: {}", names.join(", ")))
            })?,
            (None, None) => {
                return Err(
                    Error::new("config.invalid", "provider needs `preset` or `endpoint`")
                        .with_hint("e.g. preset = \"ollama\""),
                )
            }
        };
        shellmate_api::check_endpoint(&endpoint)?;
        // An explicit `api_key` always wins. Otherwise a hosted preset supplies its conventional variable, but
        // only while the endpoint is the preset's own: a custom endpoint never receives a key it was not given.
        let api_key = match (s.api_key, preset, &s.endpoint) {
            (Some(k), _, _) => Some(k),
            (
                None,
                Some(Preset {
                    key_env: Some(var), ..
                }),
                None,
            ) => lookup(var).filter(|v| !v.is_empty()).map(Secret::new),
            _ => None,
        };
        let timeout = Duration::from_secs(s.timeout_secs.unwrap_or(60));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(concat!("shellmate/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::new("provider.error", format!("http client: {e}")))?;
        Ok(Self {
            client,
            url: format!("{}/chat/completions", endpoint.trim_end_matches('/')),
            model: s.model,
            api_key,
            reasoning_effort: s.reasoning_effort,
        })
    }
}

impl OpenAiCompatible {
    pub(crate) fn build_request(
        &self,
        request: &ChatRequest,
        stream: bool,
    ) -> reqwest::RequestBuilder {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": request.messages,
            "stream": stream,
        });
        if stream {
            body["stream_options"] = serde_json::json!({ "include_usage": true });
        }
        if let Some(m) = request.max_tokens {
            body["max_tokens"] = m.into();
        }
        if let Some(t) = request.temperature {
            body["temperature"] = t.into();
        }
        if let Some(r) = &self.reasoning_effort {
            body["reasoning_effort"] = r.as_str().into();
        }
        let mut http = self.client.post(&self.url).json(&body);
        if let Some(key) = &self.api_key {
            http = http.bearer_auth(key.expose());
        }
        http
    }

    /// Send and turn HTTP-level failures into classified errors; the body is only read on failure.
    pub(crate) async fn send(
        &self,
        request: &ChatRequest,
        stream: bool,
    ) -> Result<reqwest::Response, ProviderError> {
        let resp = self
            .build_request(request, stream)
            .send()
            .await
            .map_err(|e| {
                let kind = if e.is_timeout() {
                    ProviderErrorKind::Timeout
                } else {
                    ProviderErrorKind::Unavailable
                };
                // `without_url` keeps credentials that may be embedded in a URL out of messages.
                ProviderError::new(kind, e.without_url().to_string())
            })?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let text = resp.text().await.unwrap_or_default();
        Err(ProviderError::new(
            classify_status(status.as_u16()),
            format!("HTTP {}: {}", status.as_u16(), error_message(&text)),
        ))
    }
}

#[async_trait]
impl Provider for OpenAiCompatible {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            plugin: PLUGIN_ID.into(),
            model: self.model.clone(),
            endpoint: Some(self.url.clone()),
        }
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let resp = self.send(request, false).await?;
        let text = resp.text().await.map_err(|e| {
            ProviderError::new(ProviderErrorKind::Unavailable, e.without_url().to_string())
        })?;
        let wire: WireResponse = serde_json::from_str(&text).map_err(|e| {
            ProviderError::new(
                ProviderErrorKind::Other,
                format!("unexpected response format: {e}"),
            )
        })?;
        let choice = wire.choices.into_iter().next().ok_or_else(|| {
            ProviderError::new(ProviderErrorKind::Other, "response has no choices")
        })?;
        let content = choice.message.and_then(|m| m.content).unwrap_or_default();
        if content.is_empty() && choice.finish_reason.as_deref() == Some("content_filter") {
            return Err(ProviderError::new(
                ProviderErrorKind::ContentFiltered,
                "the provider filtered the response",
            ));
        }
        Ok(ChatResponse {
            text: content,
            usage: wire.usage.map(|u| Usage {
                input_tokens: u.prompt_tokens.unwrap_or(0),
                output_tokens: u.completion_tokens.unwrap_or(0),
            }),
            finish_reason: choice.finish_reason,
        })
    }

    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, ProviderError> {
        let resp = self.send(request, true).await?;
        Ok(Box::new(SseStream {
            resp,
            buf: Vec::new(),
            pending: VecDeque::new(),
            finish_reason: None,
            ended: false,
        }))
    }
}
