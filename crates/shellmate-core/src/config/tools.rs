//! The `[tools]` section: `man`, `exec` and `read`, and any other tool a plugin provides.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub read: ReadTool,
    /// Every other `[tools.<name>]`, `man` among them. The name must be a tool the registry has; that is
    /// checked when the engine is built, so a misspelt name is an error that lists the tools there are.
    #[serde(flatten)]
    pub other: BTreeMap<String, ToolEntry>,
}

impl ToolsConfig {
    /// Whether the tool is on: a tool with no table is on.
    pub fn enabled(&self, name: &str) -> bool {
        self.other.get(name).is_none_or(|t| t.enabled)
    }
}

/// One `[tools.<name>]` of a tool that is not built in. The core keeps three keys, the same for every
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

/// The `read` tool: the model may ask to read one text file. Each file asks first unless a rule covers it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadTool {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    /// `ask` or `deny`; standing approvals are `preapproved` rules.
    #[serde(default = "default_ask")]
    pub approval: ExecApproval,
    /// Paths (relative to the working directory, `*` and `?` wildcards) read without a prompt, for example
    /// `"docs/*"`. Credential-looking files are always refused.
    #[serde(default)]
    pub preapproved: Vec<String>,
    /// Longest file text sent to the model, in bytes.
    #[serde(default = "default_read_bytes")]
    pub max_bytes: usize,
    /// Directories besides the working directory that may be read (absolute, or starting with `~/`).
    #[serde(default)]
    pub roots: Vec<String>,
}

impl Default for ReadTool {
    fn default() -> Self {
        Self {
            enabled: true,
            approval: ExecApproval::Ask,
            preapproved: Vec::new(),
            max_bytes: default_read_bytes(),
            roots: Vec::new(),
        }
    }
}

fn default_read_bytes() -> usize {
    16384
}

fn default_ask() -> ExecApproval {
    ExecApproval::Ask
}

/// `ask` shows a prompt for every command that is not covered by a pre-approval rule; `deny` turns the
/// tool off. There is no `allow`: standing approvals are written as `preapproved` rules, and the classifier
/// keeps its veto over them (`approval-ux.md` §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecApproval {
    Ask,
    Deny,
}
