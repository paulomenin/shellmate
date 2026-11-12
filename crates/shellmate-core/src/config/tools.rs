//! The `[tools]` section: one table per tool (`man`, `exec`, `read`, and any other a plugin provides).

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsConfig {
    /// Every `[tools.<name>]`. The name must be a tool the registry has; that is checked when the engine
    /// is built, so a misspelt name is an error that lists the tools there are.
    #[serde(flatten)]
    pub entries: BTreeMap<String, ToolEntry>,
}

impl ToolsConfig {
    /// Whether the tool is on: a tool with no table is on.
    pub fn enabled(&self, name: &str) -> bool {
        self.entries.get(name).is_none_or(|t| t.enabled)
    }
}

/// One `[tools.<name>]`. The core keeps three keys, the same for every
/// tool. Everything else is the tool's own and goes to it untouched (secret references resolved); the tool
/// refuses a key it does not know.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolEntry {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    /// `ask` for every call, `allow` for none (only a tool that is read-only and constrained enough to need
    /// no approval accepts it), or `deny` to turn it off. Left out: the tool's own default.
    #[serde(default)]
    pub approval: Option<ToolApproval>,
    /// Standing approvals, written by the user. What a rule means is up to the tool, but a call the
    /// classifier flags is never covered by one, and neither is any call in a turn with untrusted text.
    #[serde(default)]
    pub preapproved: Vec<String>,
    #[serde(flatten)]
    pub settings: toml::Table,
}

impl Default for ToolEntry {
    /// What a tool has when its table is not in the file: on, with its own approval default, no rules.
    fn default() -> Self {
        Self {
            enabled: true,
            approval: None,
            preapproved: Vec::new(),
            settings: toml::Table::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolApproval {
    Allow,
    Ask,
    Deny,
}
