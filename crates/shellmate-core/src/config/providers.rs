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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// `builtin:<id>`
    pub plugin: String,
    #[serde(default = "default_trust")]
    pub trust: Trust,
    pub max_context_level: Option<u8>,
    #[serde(default)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub settings: toml::Table,
}
