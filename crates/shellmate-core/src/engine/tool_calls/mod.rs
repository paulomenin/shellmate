//! Tool calls the model makes during a turn.
//!
//! Each tool implements [`Tool`]. `exec` and `read` need a human's approval (INV-1) and taint the turn
//! with what they return (INV-4), so each splits its work in two: `decide` answers "may this happen?"
//! (parse, classify, pre-approval, ask the person) and runs nothing, and `execute` does the work for a
//! call that was allowed. `man` is read-only and has no approval step.

pub(in crate::engine) mod exec;
mod man;
mod plugged;
pub(in crate::engine) mod read;
mod services;

pub(in crate::engine) use plugged::Plugged;

use super::Engine;
use crate::action::Action;
use crate::config::Config;
use crate::ports::Host;
use crate::present::{Mark, Notice, Tone};
use crate::tools::{Approver, ToolCall, ToolPolicy};
use async_trait::async_trait;

/// Everything a tool may use for one call. A tool gets no more than this.
pub(in crate::engine) struct ToolEnv<'a> {
    pub(in crate::engine) cfg: &'a Config,
    pub(in crate::engine) policy: &'a ToolPolicy,
    /// `None` (no terminal): every call that needs a person is denied.
    pub(in crate::engine) approver: Option<&'a dyn Approver>,
    /// Environment and file system the tool may look at.
    pub(in crate::engine) host: &'a Host,
    pub(in crate::engine) cwd: Option<&'a str>,
    /// The turn carries text written by others: pre-approvals are suspended (INV-4).
    pub(in crate::engine) tainted: bool,
    /// Where a result will be sent, for the approval prompt (`read` says so).
    pub(in crate::engine) dest: &'a str,
}

/// A tool the model can call.
#[async_trait]
pub(in crate::engine) trait Tool: Sync {
    /// The name the model uses in a `shellmate-tool` block.
    fn name(&self) -> &'static str;
    /// The instructions that offer this tool to the model for `action`, or `None` when it is not offered:
    /// turned off, not allowed to ask, or not an action that may use tools. Kept next to the tool so that
    /// what the model is told and what the tool accepts change together.
    fn offer(&self, cfg: &Config, policy: &ToolPolicy, action: Action) -> Option<String>;
    /// Decide and, if allowed, run one call. Always returns what to show, what the model gets back, and
    /// what was decided; a refusal is a `ToolRun`, not an error.
    async fn run(&self, call: &ToolCall, env: &ToolEnv<'_>) -> ToolRun;
}

const TOOLS: [&dyn Tool; 3] = [&man::Man, &exec::Exec, &read::Read];

/// The offers of every tool that is available for `action`, in the order the prompt shows them: the
/// built-in tools, then the others in the order they were added.
pub(in crate::engine) fn offers(
    cfg: &Config,
    policy: &ToolPolicy,
    plugged: &[Plugged],
    action: Action,
) -> Vec<String> {
    let api_action = match action {
        Action::Ask => Some(shellmate_api::ToolAction::Ask),
        Action::Generate => Some(shellmate_api::ToolAction::Generate),
        Action::Explain => Some(shellmate_api::ToolAction::Explain),
        Action::NewConversation => None,
    };
    TOOLS
        .iter()
        .filter_map(|t| t.offer(cfg, policy, action))
        .chain(
            api_action
                .into_iter()
                .flat_map(|a| plugged.iter().filter_map(move |p| p.offer(a))),
        )
        .collect()
}

fn find(name: &str) -> Option<&'static dyn Tool> {
    TOOLS.iter().copied().find(|t| t.name() == name)
}

impl Engine {
    /// Returns what was shown to the user, what the model gets back, and what was decided.
    pub(super) async fn run_tool(
        &self,
        body: &str,
        cwd: Option<&str>,
        tainted: bool,
        dest: &str,
    ) -> ToolRun {
        let call = match ToolCall::parse(body) {
            Ok(c) => c,
            Err(e) => {
                return ToolRun::new(
                    Notice::new(Tone::Warning, Mark::Chevron, "(invalid tool call)"),
                    format!("Tool error: {e}"),
                    ToolDecision::Deny,
                )
            }
        };
        let env = ToolEnv {
            cfg: &self.cfg,
            policy: &self.policy,
            approver: self.approver.as_deref(),
            host: &self.host,
            cwd,
            tainted,
            dest,
        };
        if let Some(p) = self.plugged.iter().find(|p| p.name() == call.name) {
            return p.run(&call, &env).await;
        }
        match find(&call.name) {
            Some(tool) => tool.run(&call, &env).await,
            // Not a tool we have: the policy gives the reason, as it does for a refused `man`.
            None => man::denied_by_policy(&call, &env),
        }
    }
}

/// What was decided about one tool call. [`ToolDecision::as_str`] is what the audit log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolDecision {
    /// A tool that needs no approval (`man`) ran.
    Allow,
    /// Refused without asking: disabled, not allowed, or not a call we accept.
    Deny,
    /// A pre-approval rule allowed it.
    Preapproved,
    /// The person said yes.
    Approved,
    DeniedByUser,
    /// Nobody could be asked (no terminal).
    DeniedNoTerminal,
    /// The person took the command to their own prompt instead.
    ToBuffer,
}

impl ToolDecision {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Preapproved => "preapproved",
            Self::Approved => "approved",
            Self::DeniedByUser => "denied_by_user",
            Self::DeniedNoTerminal => "denied_no_terminal",
            Self::ToBuffer => "to_buffer",
        }
    }

    /// The call went ahead (it is not a refusal or a denial).
    pub(super) fn went_ahead(self) -> bool {
        matches!(self, Self::Allow | Self::Preapproved | Self::Approved)
    }
}

/// The outcome of one tool call inside a turn.
pub(super) struct ToolRun {
    /// What is shown to the user.
    pub(super) notice: Notice,
    /// What the model gets back.
    pub(super) result: String,
    pub(super) decision: ToolDecision,
    pub(super) to_buffer: Option<String>,
    /// The result carries content the user did not write (command output, file text): the rest of the turn
    /// is tainted and pre-approvals stop applying (INV-4).
    pub(super) untrusted: bool,
}

impl ToolRun {
    pub(super) fn new(notice: Notice, result: String, decision: ToolDecision) -> Self {
        Self {
            notice,
            result,
            decision,
            to_buffer: None,
            untrusted: false,
        }
    }

    pub(super) fn untrusted(mut self) -> Self {
        self.untrusted = true;
        self
    }

    /// A refusal for `tool` (`exec`, `read`): shown as denied, and the model is told why.
    pub(super) fn refused(tool: &str, rest: &str, why: &str, decision: ToolDecision) -> Self {
        Self::new(
            Notice::new(
                Tone::Warning,
                Mark::Chevron,
                format!("denied: {tool} {}", crate::sanitize::escape(rest)),
            ),
            format!("Tool call `{tool} {rest}` was denied: {why}"),
            decision,
        )
    }
}
