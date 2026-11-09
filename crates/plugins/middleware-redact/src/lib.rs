//! Reversible redaction for the `llm` stage (`requirements.md`, "Reversible redaction as middleware").
//!
//! Down: secrets in the outgoing messages are replaced by stable placeholders `<SECRET:a91f3c>`.
//! Up: placeholders in the response are replaced by the original values locally, so suggested
//! commands stay valid and the secret itself never leaves the machine. The mapping lives only in
//! memory for the duration of the call and is never logged.
//!
//! Redaction is best-effort pattern matching: it reduces accidental leaks, it is not a DLP guarantee.
//!
//! `rules` says what is a secret, `mapping` and `redactor` do the replacing, and `middleware` plugs it
//! into the `llm` chain (including restoring placeholders in a streamed answer).

mod mapping;
mod middleware;
mod redactor;
mod rules;
#[cfg(test)]
mod tests;

pub use mapping::*;
pub use middleware::*;
pub use redactor::*;

use serde::Deserialize;
use shellmate_api::{ChatRequest, ChatStream, Error, Middleware, MiddlewareRole, Registry};
use std::sync::Arc;

pub const PLUGIN_ID: &str = "redact";

pub fn register(registry: &mut Registry) {
    registry.register_llm_middleware(
        PLUGIN_ID,
        Some(MiddlewareRole::Redactor),
        Box::new(|settings| {
            let s: Settings = serde_json::from_value(settings.clone())
                .map_err(|e| Error::new("config.invalid", format!("redact settings: {e}")))?;
            Ok(
                Arc::new(RedactMiddleware::new(Redactor::new(&s.extra_patterns)?)?)
                    as Arc<dyn Middleware<ChatRequest, ChatStream>>,
            )
        }),
    );
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Extra user rules. If the regex has a group named `secret`, only that part is replaced.
    #[serde(default)]
    pub extra_patterns: Vec<ExtraPattern>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraPattern {
    pub name: String,
    pub regex: String,
}
