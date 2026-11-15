//! Provider plugin for OpenAI-compatible `/chat/completions` APIs.
//!
//! `settings` says where to connect (the presets are shared, in `shellmate-api`), `client` makes the requests, `wire` reads the replies and
//! classifies errors, and `sse` reads a streamed answer.

mod client;
mod settings;
mod sse;
#[cfg(test)]
mod tests;
mod wire;

pub use client::*;
pub use settings::*;

use shellmate_api::{Provider, Registry};
use std::sync::Arc;

pub const PLUGIN_ID: &str = "openai-compatible";

pub fn register(registry: &mut Registry) {
    registry.register_provider(
        PLUGIN_ID,
        Box::new(|settings| {
            let provider = OpenAiCompatible::from_settings(settings)?;
            Ok(Arc::new(provider) as Arc<dyn Provider>)
        }),
    );
}
