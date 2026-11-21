//! Providers, plugins and how far each is trusted.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    Local,
    Remote,
}

impl Trust {
    /// Default maximum context level (`config.md`: local → 3, remote → 1).
    pub fn default_max_level(self) -> u8 {
        match self {
            Trust::Local => 3,
            Trust::Remote => 1,
        }
    }
}

/// `[providers.<name>]`: a connection, the place data goes. The core keeps `trust` and `max_context_level`,
/// because they are about where the data goes. Everything else (`preset`, `endpoint`, `api_key`, `timeout_secs`)
/// is for the plugins of the models that use the connection, which refuse a key they do not know.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    #[serde(default = "default_trust")]
    pub trust: Trust,
    pub max_context_level: Option<u8>,
    #[serde(flatten)]
    pub settings: toml::Table,
}

fn default_trust() -> Trust {
    Trust::Remote
}

impl ProviderConfig {
    pub fn effective_max_level(&self) -> u8 {
        self.max_context_level
            .unwrap_or_else(|| self.trust.default_max_level())
    }
}

/// How the model is offered the tools (`[models.<name>] tool_calling`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCalling {
    /// Native when the server says the model takes tools, and in the prompt text when it says it does not or
    /// does not say. The answer is remembered, so the question is asked rarely.
    #[default]
    Auto,
    /// Always native: the provider is sent the tools and the model answers with calls.
    Native,
    /// Always in the prompt text: the model writes a `shellmate-tool` block.
    Text,
}

impl ToolCalling {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolCalling::Auto => "auto",
            ToolCalling::Native => "native",
            ToolCalling::Text => "text",
        }
    }
}

/// `[models.<name>]`: what is used from a connection. The core keeps `provider`, `plugin` and `tool_calling`; the rest (`model`,
/// `reasoning_effort`, `path`) is the plugin's. The plugin says whether the model is a chat model or a decision
/// model, so a table never has to.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    /// The `[providers.<name>]` the model is reached through.
    pub provider: String,
    /// `builtin:<id>`
    pub plugin: String,
    #[serde(default)]
    pub tool_calling: ToolCalling,
    #[serde(flatten)]
    pub settings: toml::Table,
}

/// Keys the core keeps in a `[providers.<name>]`. A plugin cannot use them for its own settings.
pub const PROVIDER_KEYS: &[&str] = &["trust", "max_context_level"];
/// Keys the core keeps in a `[models.<name>]`.
pub const MODEL_KEYS: &[&str] = &["provider", "plugin", "tool_calling"];
/// Keys the core keeps in a `[plugins."<id>"]`.
pub const PLUGIN_KEYS: &[&str] = &["enabled"];

/// `[plugins."<id>"]`: a switch, and the settings of a middleware or store plugin next to it.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub settings: toml::Table,
}
