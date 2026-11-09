//! The `[runtime]`, `[context]`, `[validator]`, `[pipeline]`, `[ui]`, `[audit]`, `[conversation]`, `[actions]`
//! and `[input]` sections.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    #[serde(default = "default_mode")]
    pub mode: String,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            mode: default_mode(),
        }
    }
}

fn default_mode() -> String {
    "oneshot".into()
}

/// Checks on a suggested command before it reaches the input line. They only ever add notes; the command is
/// still placed in the buffer and you decide.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validator {
    /// Deterministic checks: the program exists, the line parses in your shell, the classifier's risk.
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    /// Also ask a decision model whether the command matches the request and how risky it is.
    #[serde(default)]
    pub semantic: bool,
    /// `[deciders.<name>]` to try, in order. Empty means the LLM answers the same questions.
    #[serde(default)]
    pub deciders: Vec<String>,
    /// With no decider (or none reachable), let the LLM provider chain answer the questions.
    #[serde(default = "super::default_true")]
    pub llm_fallback: bool,
    #[serde(default = "default_validator_timeout")]
    pub timeout: String,
    /// A semantic finding is shown only when the model is at least this sure (0.5..=1).
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
}

impl Default for Validator {
    fn default() -> Self {
        Self {
            enabled: true,
            semantic: false,
            deciders: Vec::new(),
            llm_fallback: true,
            timeout: default_validator_timeout(),
            min_confidence: default_min_confidence(),
        }
    }
}

fn default_validator_timeout() -> String {
    "5s".into()
}

fn default_min_confidence() -> f32 {
    0.8
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Llm {
    /// Primary first, then fallbacks in order.
    #[serde(default)]
    pub chain: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextConfig {
    #[serde(default = "default_level")]
    pub default_level: u8,
    /// How many recent commands are sent at context level 2 and above.
    #[serde(default = "default_history_lines")]
    pub history_lines: usize,
    /// Most bytes of captured command output (`shellmate run`) sent at level 2 and above.
    #[serde(default = "default_capture_bytes")]
    pub capture_bytes: usize,
    /// Level 1 and above: which kinds of project the working directory is (file names only).
    #[serde(default = "super::default_true")]
    pub project: bool,
    /// Level 2 and above: branch, ahead/behind and change counts of the git repository.
    #[serde(default = "super::default_true")]
    pub git: bool,
    /// Also send this many recent commit subjects (0 = none). Subjects are text other people wrote, so a
    /// request that carries them is tainted and suspends `exec` pre-approvals.
    #[serde(default)]
    pub git_log_lines: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            default_level: default_level(),
            history_lines: default_history_lines(),
            capture_bytes: default_capture_bytes(),
            project: true,
            git: true,
            git_log_lines: 0,
        }
    }
}

fn default_history_lines() -> usize {
    5
}

fn default_capture_bytes() -> usize {
    8192
}

/// Middleware order per stage, outermost first. `"..."` stands for the built-in defaults.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    #[serde(default = "dots")]
    pub llm: Vec<String>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self { llm: dots() }
    }
}

fn dots() -> Vec<String> {
    vec!["...".to_string()]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ui {
    /// Show the answer as it is generated.
    #[serde(default = "super::default_true")]
    pub stream: bool,
    /// Colour and styling: `auto` decides from the terminal and the environment (`NO_COLOR`, `CLICOLOR`,
    /// `CLICOLOR_FORCE`, `TERM`), `always` and `never` decide for you.
    #[serde(default)]
    pub color: crate::present::ColorSetting,
    /// Format the markdown in answers (bold, lists, code) when the terminal can show it.
    #[serde(default = "super::default_true")]
    pub markdown: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            stream: true,
            color: crate::present::ColorSetting::Auto,
            markdown: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    /// Keep the log across logins (`$XDG_STATE_HOME`) instead of on tmpfs.
    #[serde(default)]
    pub persist: bool,
    #[serde(default = "default_retention")]
    pub retention: String,
    #[serde(default = "default_audit_bytes")]
    pub max_bytes: u64,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            persist: false,
            retention: default_retention(),
            max_bytes: default_audit_bytes(),
        }
    }
}

/// `[secrets]`: how `cmd:` references run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretsConfig {
    /// How long a `cmd:` program may take, for example `10s`.
    #[serde(default = "default_cmd_timeout")]
    pub cmd_timeout: String,
    /// Environment variables handed on to a `cmd:` program, besides the fixed list in
    /// `secret_cmd::BASE_ENV` (for example `PASSWORD_STORE_DIR`, `OP_SERVICE_ACCOUNT_TOKEN`).
    #[serde(default)]
    pub pass_env: Vec<String>,
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self {
            cmd_timeout: default_cmd_timeout(),
            pass_env: Vec::new(),
        }
    }
}

fn default_cmd_timeout() -> String {
    "5s".into()
}

fn default_retention() -> String {
    "30d".into()
}

fn default_audit_bytes() -> u64 {
    10 * 1024 * 1024
}

/// How a request relates to the session's conversation (`conversation.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationMode {
    /// Continue the current conversation unless it has been idle too long.
    Auto,
    /// Always continue (create one if there is none).
    Continue,
    /// Stateless: read nothing, write nothing.
    Fresh,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationConfig {
    /// A store plugin id (`builtin:store-tmpfs`) or `none`.
    #[serde(default = "default_store")]
    pub store: String,
    #[serde(default = "default_idle")]
    pub idle_timeout: String,
    /// Hard cap per conversation; the oldest turns are dropped from storage.
    #[serde(default = "default_max_turns")]
    pub max_turns: usize,
    #[serde(default = "default_tool_rounds")]
    pub max_tool_rounds: usize,
    #[serde(default)]
    pub history: HistoryConfig,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            store: default_store(),
            idle_timeout: default_idle(),
            max_turns: default_max_turns(),
            max_tool_rounds: default_tool_rounds(),
            history: HistoryConfig::default(),
        }
    }
}

fn default_store() -> String {
    "builtin:store-tmpfs".into()
}

fn default_idle() -> String {
    "30m".into()
}

fn default_max_turns() -> usize {
    40
}

fn default_tool_rounds() -> usize {
    8
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryConfig {
    /// Only `window` exists so far.
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default = "default_keep")]
    pub keep_recent_turns: usize,
    #[serde(default = "default_elide")]
    pub elide_tool_results_after: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            strategy: default_strategy(),
            keep_recent_turns: default_keep(),
            elide_tool_results_after: default_elide(),
        }
    }
}

fn default_strategy() -> String {
    "window".into()
}

fn default_keep() -> usize {
    6
}

fn default_elide() -> usize {
    2
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionConfig {
    pub conversation: Option<ConversationMode>,
    pub idle_timeout: Option<String>,
}

/// Built-in `llm` middleware, outermost first.
pub const DEFAULT_LLM_PIPELINE: &[&str] = &["builtin:redact"];

fn default_level() -> u8 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default = "default_shortcuts")]
    pub shortcuts: BTreeMap<String, String>,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            shortcuts: default_shortcuts(),
        }
    }
}

fn default_shortcuts() -> BTreeMap<String, String> {
    BTreeMap::from([("alt+m".to_string(), "ask".to_string())])
}
