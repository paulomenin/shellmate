//! Tool calls the model makes during a turn.
//!
//! Every tool implements `shellmate_api::Tool`; `plugged` is the host that runs one, and where the core
//! enforces (INV-1, INV-4): the approver, the pre-approval rules, taint, escaping. `services` is what the
//! core lets a tool use. This file finds the tool for a call and holds what was decided about it.

mod plugged;
mod services;

pub(in crate::engine) use plugged::Plugged;

use super::Engine;
use crate::action::Action;
use crate::ports::Host;
use crate::present::{Mark, Notice, Tone};
use crate::tools::{Approver, ToolCall};

/// Everything a tool may use for one call. A tool gets no more than this.
pub(in crate::engine) struct ToolEnv<'a> {
    /// `None` (no terminal): every call that needs a person is denied.
    pub(in crate::engine) approver: Option<&'a dyn Approver>,
    /// Environment and file system the tool may look at.
    pub(in crate::engine) host: &'a Host,
    pub(in crate::engine) cwd: Option<&'a str>,
    /// The turn carries text written by others: pre-approvals are suspended (INV-4).
    pub(in crate::engine) tainted: bool,
    /// Where a result will be sent, for the approval prompt (`read` says so).
    pub(in crate::engine) dest: &'a str,
    /// A command may be sent to the input line: false for `explain`, which never puts one there.
    pub(in crate::engine) buffer_ok: bool,
}

/// The offers of every tool that is available for `action`, in the order the prompt shows them: the order
/// the tools were registered in.
pub(in crate::engine) fn offers(plugged: &[Plugged], action: Action, host: &Host) -> Vec<String> {
    let api_action = match action {
        Action::Ask => Some(shellmate_api::ToolAction::Ask),
        Action::Generate => Some(shellmate_api::ToolAction::Generate),
        Action::Explain => Some(shellmate_api::ToolAction::Explain),
        Action::NewConversation => None,
    };
    api_action
        .into_iter()
        .flat_map(|a| plugged.iter().filter_map(move |p| p.offer(a, host)))
        .collect()
}

/// The native form of every tool that is available for `action`, in the same order as [`offers`].
pub(in crate::engine) fn specs(
    plugged: &[Plugged],
    action: Action,
    host: &Host,
) -> Vec<shellmate_api::ToolSpec> {
    let api_action = match action {
        Action::Ask => Some(shellmate_api::ToolAction::Ask),
        Action::Generate => Some(shellmate_api::ToolAction::Generate),
        Action::Explain => Some(shellmate_api::ToolAction::Explain),
        Action::NewConversation => None,
    };
    api_action
        .into_iter()
        .flat_map(|a| plugged.iter().filter_map(move |p| p.spec(a, host)))
        .collect()
}

/// A native call whose arguments are not a call. It is a tool call that failed, so the model is told and the
/// turn goes on.
pub(in crate::engine) fn invalid_native_call(why: &str) -> ToolRun {
    ToolRun::new(
        Notice::new(Tone::Warning, Mark::Chevron, "(invalid tool call)"),
        format!("Tool error: {why}"),
        ToolDecision::Deny,
    )
}

impl Engine {
    /// Returns what was shown to the user, what the model gets back, and what was decided.
    pub(super) async fn run_tool(
        &self,
        body: &str,
        cwd: Option<&str>,
        tainted: bool,
        dest: &str,
        buffer_ok: bool,
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
            approver: self.approver.as_deref(),
            host: &self.host,
            cwd,
            tainted,
            dest,
            buffer_ok,
        };
        if let Some(p) = self.plugged.iter().find(|p| p.name() == call.name) {
            return p.run(&call, &env).await;
        }
        unknown_tool(&call)
    }
}

/// A call to a tool that does not exist.
fn unknown_tool(call: &ToolCall) -> ToolRun {
    ToolRun::new(
        Notice::new(
            Tone::Warning,
            Mark::Chevron,
            format!("denied: {}", call.display()),
        ),
        format!(
            "Tool call `{}` was denied: not allowed without approval: unknown tool `{}`.",
            call.display(),
            call.name
        ),
        ToolDecision::Deny,
    )
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
}
