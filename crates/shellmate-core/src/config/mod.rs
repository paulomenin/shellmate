//! Configuration (`config.md`). Strict: unknown keys are errors. Only the sections implemented so
//! far are accepted; the rest of the documented schema is added milestone by milestone.
//!
//! The schema is split by topic: `sections` (runtime, context, validator, pipeline, ui, audit,
//! conversation, input), `providers` (providers, plugins, trust) and `tools`. `load` reads and parses the
//! file, `validate` checks a parsed config, and `resolve` turns it into plugin settings.

mod load;
mod providers;
mod resolve;
mod sections;
#[cfg(test)]
mod tests;
mod tools;
mod validate;

pub use load::parse_duration;
pub use providers::*;
pub use resolve::{builtin_id, LazySettings};
pub use sections::*;
pub use tools::*;

use crate::action::Action;
use serde::Deserialize;
use shellmate_api::{Error, Result};
use std::collections::BTreeMap;

pub const KNOWN_ACTIONS: &[&str] = &["ask", "generate", "explain", "new-conversation", "capture"];

/// Actions that talk to the model (and so can have `[actions.<name>]` conversation settings).
pub const LLM_ACTIONS: &[&str] = &["ask", "generate", "explain"];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub runtime: Runtime,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub llm: Llm,
    /// Decision-model backends, configured like providers (`[deciders.<name>]`).
    #[serde(default)]
    pub deciders: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub validator: Validator,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub input: Input,
    #[serde(default)]
    pub pipeline: Pipeline,
    #[serde(default)]
    pub ui: Ui,
    #[serde(default)]
    pub conversation: ConversationConfig,
    /// Per-action overrides, keyed by action name.
    #[serde(default)]
    pub actions: BTreeMap<String, ActionConfig>,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default)]
    pub secrets: SecretsConfig,
    /// Plugin instances by id (`builtin:redact`): `enabled` and `settings`.
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginConfig>,
}

/// The `serde` default for switches that are on unless turned off.
fn default_true() -> bool {
    true
}

impl Config {
    /// How `cmd:` references run (`[secrets]`). Validated by `check_static`.
    pub fn cmd_options(&self) -> crate::secret_cmd::CmdOptions {
        crate::secret_cmd::CmdOptions {
            timeout: parse_duration(&self.secrets.cmd_timeout)
                .unwrap_or_else(|_| std::time::Duration::from_secs(5)),
            pass_env: self.secrets.pass_env.clone(),
        }
    }

    pub fn conversation_mode(&self, action: Action) -> ConversationMode {
        self.actions
            .get(action.as_str())
            .and_then(|a| a.conversation)
            .unwrap_or(ConversationMode::Auto)
    }

    /// The most specific idle timeout: per action, then global. Validated by `check_static`.
    pub fn idle_timeout(&self, action: Action) -> std::time::Duration {
        let text = self
            .actions
            .get(action.as_str())
            .and_then(|a| a.idle_timeout.as_deref())
            .unwrap_or(&self.conversation.idle_timeout);
        parse_duration(text).unwrap_or(std::time::Duration::from_secs(30 * 60))
    }

    /// Provider names in fallback order. Errors when nothing is configured.
    pub fn provider_chain(&self) -> Result<Vec<&str>> {
        let chain: Vec<&str> = if self.llm.chain.is_empty() {
            self.providers.keys().take(1).map(String::as_str).collect()
        } else {
            self.llm.chain.iter().map(String::as_str).collect()
        };
        if chain.is_empty() {
            return Err(Error::new(
                "config.no_provider",
                "shellmate isn't configured yet (no LLM provider)",
            )
            .with_hint("run `shellmate setup`, or choose a provider in the config (`shellmate config edit`)"));
        }
        Ok(chain)
    }

    /// The `llm` middleware ids to run, outermost first, `"..."` expanded to the defaults and
    /// disabled plugins removed.
    pub fn llm_layers(&self) -> Vec<String> {
        let mut out = Vec::new();
        for id in &self.pipeline.llm {
            if id == "..." {
                out.extend(DEFAULT_LLM_PIPELINE.iter().map(|s| s.to_string()));
            } else {
                out.push(id.clone());
            }
        }
        out.retain(|id| self.plugins.get(id).is_none_or(|p| p.enabled));
        out
    }
}
