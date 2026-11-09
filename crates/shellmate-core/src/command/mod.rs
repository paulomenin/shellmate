//! Commands the model wants to run (`approval-ux.md` §4.2 and §4.5): a strict parser for simple commands,
//! a rule-based classifier, and the matcher for the user's pre-approval rules.
//!
//! Everything here is deterministic and conservative. The classifier can only make things stricter: a
//! `Safe` verdict never approves anything, and anything it does not understand needs a human.
//!
//! One file per concern: `parse` (the strict grammar), `classify` and `tables` (risk), `rules` (the user's
//! pre-approvals) and `program` (a command ready to run).

mod classify;
mod parse;
mod program;
mod rules;
mod tables;
#[cfg(test)]
mod tests;

pub use classify::*;
pub use parse::*;
pub use program::*;
pub use rules::*;
