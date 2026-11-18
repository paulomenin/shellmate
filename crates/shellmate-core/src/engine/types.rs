//! The types a client sees: actions, outcomes, reports and the provider specs.

use crate::config::Trust;
use crate::present::{Notice, Remark};
use shellmate_api::{ChatRequest, Decider, Provider};
use std::sync::Arc;

pub use crate::action::Action;

/// What the engine shows while it works. The client prints [`Emit::Notice`] always, and [`Emit::Text`] (the
/// model's words) only when the user streams answers: what the engine *does* (a command run, a file read, a
/// refusal) must never depend on a display preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emit<'a> {
    Text(&'a str),
    Notice(&'a Notice),
}

/// What the client should do with the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The answer's prose (command blocks removed). Already shown when it was streamed; the client prints
    /// it only if nothing was streamed. Plain text: the client wraps and sanitizes it.
    pub text: String,
    /// Notes to show after the answer (other command options, checks, things that went wrong around it).
    pub notes: Vec<Remark>,
    /// New content for the input buffer; `None` leaves it unchanged.
    pub buffer: Option<String>,
    /// Which provider answered (for `--explain`-style reports).
    pub provider: String,
    /// What happened, for `--explain`.
    pub report: Report,
}

impl Outcome {
    /// The notes as plain text, separated by blank lines, as a text-only terminal shows them.
    pub fn trailer_text(&self) -> String {
        self.notes
            .iter()
            .map(Remark::plain)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// A summary of one request for `--explain` (no content).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub provider_chain: Vec<String>,
    pub trust: Option<Trust>,
    pub level: u8,
    pub redaction: bool,
    pub conversation: Option<String>,
    pub turn: usize,
    pub omitted_turns: usize,
    /// Tool calls made, as shown (`man tar`, `denied: man -P x`).
    pub tools: Vec<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub llm_ms: u64,
    pub first_token_ms: Option<u64>,
    /// Finer timings, in microseconds, for `--timings`.
    pub stages: Stages,
}

/// Where the engine's time went, in microseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stages {
    /// Gathering context (project and git collectors).
    pub context_us: u64,
    /// The model, including tool runs and approvals, for the attempt that answered. A provider that failed
    /// earlier in the fallback chain is counted only in `total_us`.
    pub llm_us: u64,
    pub first_token_us: Option<u64>,
    /// Checking the suggested command (syntax, programs, decision model).
    pub validate_us: u64,
    /// The whole engine run, from the start of the request to the finished outcome. The stages above do not
    /// overlap, so their sum is at most this.
    pub total_us: u64,
}

/// A provider in the fallback chain with the facts the privacy rules need.
#[derive(Clone)]
pub struct ProviderSpec {
    pub name: String,
    pub provider: Arc<dyn Provider>,
    /// Highest context level this provider may receive.
    pub max_level: u8,
    pub trust: Trust,
}

/// A decision-model backend with the fact the privacy rules need.
#[derive(Clone)]
pub struct DeciderSpec {
    pub name: String,
    pub decider: Arc<dyn Decider>,
    pub trust: Trust,
}

/// One entry of the validator chain: a decision model, or a chat model that answers the same questions as JSON.
#[derive(Clone)]
pub enum Check {
    Decision(DeciderSpec),
    Chat(ProviderSpec),
}

impl Check {
    pub fn name(&self) -> &str {
        match self {
            Check::Decision(d) => &d.name,
            Check::Chat(p) => &p.name,
        }
    }

    pub fn trust(&self) -> Trust {
        match self {
            Check::Decision(d) => d.trust,
            Check::Chat(p) => p.trust,
        }
    }
}

/// The payload one provider would receive (after the whole `llm` chain), for `--dry-run`.
#[derive(Debug, Clone)]
pub struct Preview {
    pub provider: String,
    pub trust: Trust,
    pub level: u8,
    pub request: ChatRequest,
}

/// Whether one piece of context is part of what a provider receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemState {
    /// Sent (the detail says how much).
    Sent,
    /// Not sent, and why (a level, a setting, or nothing to send).
    Withheld,
}

/// One piece of context and its fate (`shellmate context`).
#[derive(Debug, Clone)]
pub struct ContextItem {
    pub name: &'static str,
    /// The context level that unlocks it.
    pub level: u8,
    pub state: ItemState,
    pub detail: String,
}

/// What one provider would receive, piece by piece. No content: sizes and reasons only.
#[derive(Debug, Clone)]
pub struct ContextReport {
    pub provider: String,
    pub trust: Trust,
    pub level: u8,
    /// The most this provider may get, by its trust.
    pub ceiling: u8,
    pub redaction: bool,
    /// Pre-approvals are suspended for this request (it carries text written by others).
    pub tainted: bool,
    pub items: Vec<ContextItem>,
}
