//! The settings table of a provider.

use serde::Deserialize;
use shellmate_api::Secret;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub preset: Option<String>,
    pub endpoint: Option<String>,
    pub model: String,
    pub api_key: Option<Secret>,
    pub timeout_secs: Option<u64>,
    /// Sent as `reasoning_effort` ("none", "low", ...). Reasoning models spend most of their time thinking
    /// silently; for short shell answers "none" is typically 2-3x faster.
    pub reasoning_effort: Option<String>,
}
