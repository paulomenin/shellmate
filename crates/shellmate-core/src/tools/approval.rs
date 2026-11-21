//! What the person is asked before the model runs a command or reads a file.

use crate::command::Risk;

/// What the model is asking to do (`Run`, `Read` or a tool's own verb).
pub use shellmate_api::ApprovalKind;

/// What the user is shown when the model wants to run a command (`approval-ux.md` §4.1).
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub kind: ApprovalKind,
    /// The exact command as it will run, or the file path for a read.
    pub command: String,
    pub cwd: Option<String>,
    pub risk: Risk,
    /// Why the classifier wants a human to look at it.
    pub reasons: Vec<String>,
    /// The person may send the command to their input line instead. Not so when the line already holds
    /// what the request is about (`explain`), because nothing would be put there.
    pub can_buffer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Run it, this once.
    Run,
    Deny,
    /// Do not run it, but put it in the user's input line.
    ToBuffer,
}

/// Asks a human. Only the client binary implements this (a prompt on the terminal); no plugin can, which is
/// what makes INV-1 hold. `None` of it means "no terminal": every request is denied.
pub trait Approver: Send + Sync {
    fn approve(&self, request: &ApprovalRequest) -> Choice;
}
