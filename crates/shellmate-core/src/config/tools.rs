//! The `[tools]` section: `man`, `exec` and `read`.

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolsConfig {
    #[serde(default)]
    pub man: ManTool,
    #[serde(default)]
    pub exec: ExecTool,
    #[serde(default)]
    pub read: ReadTool,
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

/// `ask` shows a prompt for every command that is not covered by a pre-approval rule; `deny` turns the
/// tool off. There is no `allow`: standing approvals are written as `preapproved` rules, and the classifier
/// keeps its veto over them (`approval-ux.md` §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecApproval {
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecEnv {
    Sanitized,
    Inherit,
}

/// The `exec` tool: the model may ask to run a simple command to look at the machine.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecTool {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default = "default_ask")]
    pub approval: ExecApproval,
    /// Commands that run without a prompt, written by the user (see `command::rule_matches`).
    #[serde(default)]
    pub preapproved: Vec<String>,
    #[serde(default = "default_exec_timeout")]
    pub timeout: String,
    /// Longest stdout and longest stderr kept, in bytes.
    #[serde(default = "default_exec_output")]
    pub max_output: usize,
    #[serde(default = "default_exec_env")]
    pub env: ExecEnv,
}

impl Default for ExecTool {
    fn default() -> Self {
        Self {
            enabled: true,
            approval: ExecApproval::Ask,
            preapproved: Vec::new(),
            timeout: default_exec_timeout(),
            max_output: default_exec_output(),
            env: ExecEnv::Sanitized,
        }
    }
}

fn default_ask() -> ExecApproval {
    ExecApproval::Ask
}

fn default_exec_timeout() -> String {
    "30s".into()
}

fn default_exec_output() -> usize {
    16 * 1024
}

fn default_exec_env() -> ExecEnv {
    ExecEnv::Sanitized
}

/// What may happen to a tool call without asking (`approval-ux.md`). `ask` arrives with the approval
/// prompt (M5); until then a call that would need a prompt is denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Approval {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManTool {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default = "default_allow")]
    pub approval: Approval,
    /// Longest manual text sent to the model.
    #[serde(default = "default_man_bytes")]
    pub max_bytes: usize,
}

impl Default for ManTool {
    fn default() -> Self {
        Self {
            enabled: true,
            approval: Approval::Allow,
            max_bytes: default_man_bytes(),
        }
    }
}

fn default_allow() -> Approval {
    Approval::Allow
}

fn default_man_bytes() -> usize {
    8192
}
