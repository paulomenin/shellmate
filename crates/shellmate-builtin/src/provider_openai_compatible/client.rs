//! The non-streaming and streaming client for `/chat/completions`.

use super::{settings::*, sse::*, wire::*, *};
use crate::http::classify_status;
use async_trait::async_trait;
use shellmate_api::{
    parse_model_ids, parse_model_tool_support, resolve_api_key, resolve_endpoint, ChatRequest,
    ChatResponse, ChatStream, Error, Provider, ProviderError, ProviderErrorKind, ProviderInfo,
    Secret, ToolSupport, Usage, PRESETS,
};
use std::collections::VecDeque;
use std::time::Duration;

/// Where this plugin posts, below the base URL. The version belongs here, not in the configured endpoint, so
/// a `/v2` is a change to this plugin only.
const CHAT_PATH: &str = "/v1/chat/completions";

/// Where the server lists its models, below the base URL.
const MODELS_PATH: &str = "/v1/models";

pub struct OpenAiCompatible {
    pub(crate) client: reqwest::Client,
    pub(crate) url: String,
    pub(crate) models_url: String,
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
        let (endpoint, preset) = resolve_endpoint(
            "provider",
            PRESETS,
            s.endpoint.as_deref(),
            s.preset.as_deref(),
        )?;
        let api_key = resolve_api_key(s.api_key, preset, s.endpoint.is_some(), lookup);
        let timeout = Duration::from_secs(s.timeout_secs.unwrap_or(60));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(concat!("shellmate/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::new("provider.error", format!("http client: {e}")))?;
        Ok(Self {
            client,
            url: format!("{}{CHAT_PATH}", endpoint.trim_end_matches('/')),
            models_url: format!("{}{MODELS_PATH}", endpoint.trim_end_matches('/')),
            model: s.model,
            api_key,
            reasoning_effort: s.reasoning_effort,
        })
    }
}

impl OpenAiCompatible {
    /// The body of `GET /v1/models`.
    async fn fetch_models(&self) -> Result<String, ProviderError> {
        let mut rb = self
            .client
            .get(&self.models_url)
            .timeout(Duration::from_secs(10));
        if let Some(k) = &self.api_key {
            rb = rb.bearer_auth(k.expose());
        }
        // `without_url` keeps credentials that may be embedded in a URL out of messages.
        let resp = rb.send().await.map_err(|e| {
            let kind = if e.is_timeout() {
                ProviderErrorKind::Timeout
            } else {
                ProviderErrorKind::Unavailable
            };
            ProviderError::new(kind, e.without_url().to_string())
        })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            ProviderError::new(ProviderErrorKind::Unavailable, e.without_url().to_string())
        })?;
        if !status.is_success() {
            let mut msg = format!("HTTP {}: {}", status.as_u16(), error_message(&text));
            if status.as_u16() == 404 {
                msg.push_str(" (the endpoint should be a base URL without `/v1`)");
            }
            return Err(ProviderError::new(classify_status(status.as_u16()), msg));
        }
        Ok(text)
    }

    pub(crate) fn build_request(
        &self,
        request: &ChatRequest,
        stream: bool,
    ) -> reqwest::RequestBuilder {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": wire_messages(&request.messages),
            "stream": stream,
        });
        if !request.tools.is_empty() {
            body["tools"] = wire_tools(&request.tools).into();
            if let Some(choice) = wire_tool_choice(request.tool_choice) {
                body["tool_choice"] = choice.into();
            }
        }
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

    async fn models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        Ok(Some(parse_model_ids(&self.fetch_models().await?)))
    }

    /// What the server's model list says about tools for this model. Any failure, and a list that does not
    /// say, is `Unknown`: the answer is only used to prefer native calls, never to refuse a request.
    async fn tool_support(&self) -> ToolSupport {
        match self.fetch_models().await {
            Ok(text) => parse_model_tool_support(&text, &self.model),
            Err(_) => ToolSupport::Unknown,
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
        let message = choice.message;
        let content = message
            .as_ref()
            .and_then(|m| m.content.clone())
            .unwrap_or_default();
        let tool_calls: Vec<_> = message
            .map(|m| m.tool_calls)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .filter_map(|(i, c)| c.into_request(i))
            .collect();
        if content.is_empty()
            && tool_calls.is_empty()
            && choice.finish_reason.as_deref() == Some("content_filter")
        {
            return Err(ProviderError::new(
                ProviderErrorKind::ContentFiltered,
                "the provider filtered the response",
            ));
        }
        Ok(ChatResponse {
            text: content,
            tool_calls,
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
            calls: Vec::new(),
            ended: false,
        }))
    }
}
