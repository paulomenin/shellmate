//! Tools: things the model can ask shellmate to do while it answers (read a manual page, run a command,
//! read a file).
//!
//! **The core enforces, the tool describes and runs.** A tool never asks the person anything, never
//! decides that a call is pre-approved, and never touches the audit log. It answers three questions:
//!
//! 1. [`Tool::offer`]: what do I tell the model about myself for this action?
//! 2. [`Tool::assess`]: may this call happen, and what must be true first? Runs nothing.
//! 3. [`Tool::run`]: do it, for a call the core let through.
//!
//! What the core does with the answers (INV-1, INV-3, INV-4):
//!
//! - A tool whose [`ToolInfo::approval`] is [`ApprovalMode::Required`] cannot return [`Assessment::Allow`]:
//!   the core treats that as a refusal. Only the core holds the approver, so only a person can say yes.
//! - A pre-approval rule applies only when the call is [`ApprovalNeed::preapprovable`] and the turn is not
//!   tainted. The rules are the user's, read from the config by the core.
//! - A tool that declares [`ToolInfo::untrusted_output`] always taints the turn, whatever it returns.
//! - Text a tool returns is raw. The core escapes it before it reaches a terminal.

use crate::Error;
use async_trait::async_trait;
use std::any::Any;
use std::path::PathBuf;
use std::time::Duration;

/// The label of the fenced block in which the model writes a tool call.
pub const TOOL_LABEL: &str = "shellmate-tool";

/// What a tool may be allowed to do. A tool declares these in [`ToolInfo`]; the user grants them (INV-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    /// Start programs on this machine.
    RunProcesses,
    /// Read files on this machine.
    ReadFiles,
    /// Make network requests.
    Network,
}

/// Whether a person must say yes to each call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Calls are read-only and constrained enough to run without asking (a manual page).
    Never,
    /// Every call needs a person, unless a pre-approval rule of the user's covers it.
    Required,
}

/// What a tool says about itself, without being asked anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfo {
    /// The name the model writes in a `shellmate-tool` block, and the name of the `[tools.<name>]` table.
    pub name: &'static str,
    pub permissions: Vec<Permission>,
    pub approval: ApprovalMode,
    /// What the tool returns is text somebody else wrote (command output, file text). The core taints the
    /// rest of the turn and suspends pre-approvals (INV-4).
    pub untrusted_output: bool,
}

/// The actions that call a model, and so may offer tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAction {
    Ask,
    Generate,
    Explain,
}

/// The settings the core keeps for every tool. Anything else under `[tools.<name>]` goes to the tool's
/// constructor and is checked there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSettings {
    pub enabled: bool,
    pub approval: ApprovalSetting,
    /// The user's standing approvals. Only the core reads them, and it asks the tool whether one matches
    /// ([`Tool::rule_matches`]).
    pub preapproved: Vec<String>,
}

/// The `approval` key of a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalSetting {
    /// Run without asking. Valid only for a tool that declares [`ApprovalMode::Never`].
    Allow,
    /// Ask for each call.
    Ask,
    /// The tool is off.
    Deny,
}

/// One call as the model wrote it: one line, a name and its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub args: Vec<String>,
    /// Everything after the name, as written.
    pub rest: String,
}

/// Where a call happens, and what the tool is allowed to look at.
pub struct ToolCtx<'a> {
    pub settings: &'a ToolSettings,
    pub services: &'a dyn ToolServices,
    /// The working directory of the request, when the shell sent one.
    pub cwd: Option<&'a str>,
    /// The turn carries text written by others, so pre-approvals are suspended (INV-4).
    pub tainted: bool,
    /// Where a result will be sent ("the provider", a host name), for the prompt.
    pub dest: &'a str,
}

/// How a call that needed approval got it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Granted {
    /// The tool does not need approval.
    NotRequired,
    /// A pre-approval rule of the user's covered it.
    Rule(String),
    /// A person said yes at the prompt.
    Person,
}

/// How much harm a call could do, as the classifier rates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        }
    }
}

/// What the person is asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalKind {
    Run,
    Read,
    /// Any other thing; the text is the verb ("fetch").
    Other(String),
}

/// A line of the report of what happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
    Error,
    Muted,
}

/// The symbol before a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Check,
    Cross,
    Arrow,
    Chevron,
}

/// What the person sees about one call: a tone, a mark, text, and optionally a status and a few lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolNotice {
    pub tone: Tone,
    pub mark: Mark,
    pub text: String,
    /// A short status after the text, shown in brackets (`exit 0, 0.1s`).
    pub meta: Option<String>,
    /// Further lines under the text (the first lines of output).
    pub detail: Vec<String>,
}

impl ToolNotice {
    pub fn new(tone: Tone, mark: Mark, text: impl Into<String>) -> Self {
        Self {
            tone,
            mark,
            text: text.into(),
            meta: None,
            detail: Vec::new(),
        }
    }

    pub fn with_meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn with_detail(mut self, detail: Vec<String>) -> Self {
        self.detail = detail;
        self
    }
}

/// A call that does not go ahead: what the person sees and what the model is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub notice: ToolNotice,
    pub result: String,
}

impl Refusal {
    pub fn new(notice: ToolNotice, result: impl Into<String>) -> Self {
        Self {
            notice,
            result: result.into(),
        }
    }

    /// The usual form: `denied: <tool> <subject>` for the person, and a sentence with the reason for the
    /// model.
    pub fn denied(tool: &str, subject: &str, why: &str) -> Self {
        Self::new(
            ToolNotice::new(
                Tone::Warning,
                Mark::Chevron,
                format!("denied: {tool} {subject}"),
            ),
            format!("Tool call `{tool} {subject}` was denied: {why}"),
        )
    }
}

/// A call the person chose to take to their own prompt instead of letting the model run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToBuffer {
    pub refusal: Refusal,
    /// The text for the input line.
    pub buffer: String,
}

/// Something a tool prepared during [`Tool::assess`] and gets back in [`Tool::run`]. The core never looks
/// inside, so what was shown to the person is what runs.
pub struct Prepared(Box<dyn Any + Send>);

impl Prepared {
    pub fn new<T: Any + Send>(value: T) -> Self {
        Self(Box::new(value))
    }

    /// `None` when the value is not a `T`, which means the call was handed to the wrong tool.
    pub fn downcast<T: Any>(self) -> Option<T> {
        self.0.downcast::<T>().ok().map(|b| *b)
    }
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Prepared(..)")
    }
}

/// A call that needs a person. The tool supplies everything, including the wording of each way it can end,
/// so the core stays generic.
pub struct ApprovalNeed {
    pub kind: ApprovalKind,
    /// The exact command as it will run, or the file path: what the person approves.
    pub subject: String,
    pub cwd: Option<String>,
    pub risk: Risk,
    /// Why a human should look at it.
    pub reasons: Vec<String>,
    /// The classifier found nothing dangerous. Only then can a pre-approval rule apply.
    pub preapprovable: bool,
    /// What [`Tool::rule_matches`] compares a rule with: the argv of a command, the path of a file.
    pub rule_subject: Vec<String>,
    pub prepared: Prepared,
    /// The person said no.
    pub denied: Refusal,
    /// There is nobody to ask (no terminal).
    pub no_terminal: Refusal,
    /// The person took it to their own prompt. `None` when the tool has no such thing; the core then
    /// treats it as `denied`.
    pub to_buffer: Option<ToBuffer>,
}

/// What a tool says about a call, before anything runs.
pub enum Assessment {
    /// Not a call this tool accepts, or not allowed right now. Shown and told to the model.
    Refuse(Refusal),
    /// Go ahead with no approval step. Valid only for [`ApprovalMode::Never`].
    Allow(Prepared),
    NeedsApproval(Box<ApprovalNeed>),
}

/// What running a call produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// What the model gets back.
    pub result: String,
    /// What the person sees.
    pub notice: ToolNotice,
    /// This output has text somebody else wrote in it. Adds to [`ToolInfo::untrusted_output`], never
    /// removes from it.
    pub untrusted: bool,
}

/// A tool the model can call.
#[async_trait]
pub trait Tool: Send + Sync {
    fn info(&self) -> ToolInfo;

    /// The instructions that offer this tool to the model, or `None` when it is not offered for `action`.
    /// The core calls this only for a tool that is enabled. Keep it next to what the tool accepts, so the
    /// two change together.
    fn offer(&self, action: ToolAction, settings: &ToolSettings) -> Option<String>;

    /// Decide what a call needs. Reads and runs nothing.
    async fn assess(&self, call: &ToolCall, ctx: &ToolCtx<'_>) -> Assessment;

    /// Whether a pre-approval `rule` of the user's covers `subject` ([`ApprovalNeed::rule_subject`]).
    /// The core asks only when the call is preapprovable and the turn is not tainted.
    fn rule_matches(&self, _rule: &str, _subject: &[String]) -> bool {
        false
    }

    /// Do the work for a call the core let through.
    async fn run(&self, prepared: Prepared, granted: &Granted, ctx: &ToolCtx<'_>) -> ToolOutput;
}

/// How a started program sees the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessEnv {
    /// A small allow-list of variables, no pager, no colours.
    Sanitized,
    /// The variables of the shell.
    Inherit,
}

/// A program to start: no shell, no stdin, its own process group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
    pub max_output: usize,
    pub env: ProcessEnv,
    /// Variables set on top of `env`.
    pub set_env: Vec<(String, String)>,
    /// Variables removed after `env` and `set_env`.
    pub remove_env: Vec<String>,
}

/// What a finished (or failed) program produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub elapsed: Duration,
}

/// The core's rating of a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRisk {
    /// Nothing known to be dangerous. This approves nothing by itself.
    pub safe: bool,
    pub risk: Risk,
    pub reasons: Vec<String>,
}

/// A file that passed every check that needs no person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
    pub path: PathBuf,
    /// How it is shown and matched against rules.
    pub shown: String,
    pub size: u64,
}

/// What was read from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileText {
    pub text: String,
    /// The size of the file, not of `text`.
    pub bytes: u64,
    pub truncated: bool,
}

/// What the core lets a tool use. These are the only ways a tool reaches the machine, so the sanitized
/// environment, the timeout, the output cap, the classifier and the path checks are the core's, not the
/// tool's.
#[async_trait]
pub trait ToolServices: Send + Sync {
    /// An environment variable of the shell.
    fn env_var(&self, name: &str) -> Option<String>;
    fn is_dir(&self, path: &str) -> bool;

    /// Split a line into words if it is one simple command (no pipes, redirects, variables, globs or
    /// substitutions). `Err` is the reason.
    fn parse_command(&self, line: &str) -> Result<Vec<String>, String>;
    /// The program for a bare name, found in the shell's `PATH`.
    fn find_program(&self, name: &str) -> Option<PathBuf>;
    /// The classifier. It can only make things stricter.
    fn classify_command(&self, line: &str) -> CommandRisk;
    /// Whether a pre-approval rule written for commands covers `argv`.
    fn command_rule_matches(&self, rule: &str, argv: &[String]) -> bool;
    async fn run_process(&self, spec: &ProcessSpec) -> ProcessOutput;
    /// `limit` for a short internal wait of the tool's own (not a time the user set), stretched when the
    /// machine is slow, which is how tests run under instrumentation.
    fn internal_limit(&self, limit: Duration) -> Duration;

    /// Resolve and vet a path: inside the working directory or an allowed root, a plain text file, nothing
    /// credential-looking. `Err` is the reason.
    fn vet_file(
        &self,
        arg: &str,
        cwd: Option<&str>,
        roots: &[String],
    ) -> Result<FileTarget, String>;
    fn read_file(&self, target: &FileTarget, max_bytes: usize) -> Result<FileText, String>;
}

/// Builds a tool from the part of `[tools.<name>]` the core does not keep (secrets already resolved).
pub type ToolCtor =
    Box<dyn Fn(&serde_json::Value) -> Result<std::sync::Arc<dyn Tool>, Error> + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prepared_value_comes_back_only_as_its_own_type() {
        let p = Prepared::new(vec!["git".to_string(), "status".to_string()]);
        assert_eq!(
            p.downcast::<Vec<String>>(),
            Some(vec!["git".to_string(), "status".to_string()])
        );
        assert_eq!(Prepared::new(5u32).downcast::<String>(), None);
    }

    #[test]
    fn the_usual_refusal_says_what_was_refused_and_why() {
        let r = Refusal::denied("exec", "rm -rf x", "the exec tool is disabled.");
        assert_eq!(r.notice.text, "denied: exec rm -rf x");
        assert_eq!(r.notice.tone, Tone::Warning);
        assert_eq!(
            r.result,
            "Tool call `exec rm -rf x` was denied: the exec tool is disabled."
        );
    }
}
