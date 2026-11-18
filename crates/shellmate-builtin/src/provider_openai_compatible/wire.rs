//! The JSON the API sends back, and how HTTP errors map to error kinds.

use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct WireResponse {
    pub(crate) choices: Vec<WireChoice>,
    pub(crate) usage: Option<WireUsage>,
}

#[derive(Deserialize)]
pub(crate) struct WireChoice {
    pub(crate) message: Option<WireMessage>,
    pub(crate) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireMessage {
    pub(crate) content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireUsage {
    pub(crate) prompt_tokens: Option<u32>,
    pub(crate) completion_tokens: Option<u32>,
}

/// Pull `error.message` out of an error body, bounded in size; never echo the request.
pub(crate) fn error_message(body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned());
    msg.chars().take(300).collect()
}
