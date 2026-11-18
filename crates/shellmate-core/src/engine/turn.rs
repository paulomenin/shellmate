//! What happens around the model's answer: the audit events of a request, the report for `--explain`,
//! and saving the turn in the conversation.

use super::attempt::{Attempt, Failure};
use super::request::Convo;
use super::{Engine, Outcome, ProviderSpec, Report, Stages};
use crate::conversation::{self, Active, Stored, Turn};
use crate::envelope::Envelope;
use crate::present::{Remark, Tone};
use serde_json::{json, Value};
use shellmate_api::Role;
use std::time::Instant;

/// What was sent to the provider that answered.
pub(super) struct Sent {
    pub(super) level: u8,
    /// Earlier turns left out because the provider is remote.
    pub(super) omitted: usize,
    /// The user message as sent, stored with the turn.
    pub(super) user: String,
}

/// How long each stage of one request took, measured when the stage ended (never read from a clock later, so
/// a stage cannot include the one after it). Plain values, so the report can be built and tested from them.
pub(super) struct Timing {
    /// Gathering context before the first provider was tried.
    pub(super) context_us: u64,
    /// The provider call that answered, from its start to the end of its answer. A fallback attempt that
    /// failed earlier is not in this number, only in the total.
    pub(super) llm_us: u64,
    /// Checking the suggested command.
    pub(super) validate_us: u64,
    /// The whole run, up to the finished outcome.
    pub(super) total_us: u64,
}

impl Timing {
    pub(super) fn stages(&self, first_token_us: Option<u64>) -> Stages {
        Stages {
            context_us: self.context_us,
            llm_us: self.llm_us,
            first_token_us,
            validate_us: self.validate_us,
            total_us: self.total_us,
        }
    }
}

/// Microseconds from `since` until now.
pub(super) fn micros_since(since: Instant) -> u64 {
    since.elapsed().as_micros() as u64
}

impl Engine {
    pub(super) fn audit_request_started(&self, ctx: &Value, env: &Envelope, convo: &Convo) {
        self.audit(
            "request_started",
            ctx,
            json!({
                "action": env.action,
                "providers": self.providers.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                "history_turns": convo.turns.len(),
            }),
        );
    }

    pub(super) fn audit_call_failed(
        &self,
        ctx: &Value,
        spec: &ProviderSpec,
        failure: &Failure,
        started: Instant,
        fallback: bool,
    ) {
        self.audit(
            "llm_call",
            ctx,
            json!({
                "provider": spec.name,
                "status": failure.error.code,
                "latency_ms": started.elapsed().as_millis() as u64,
                "fallback": fallback,
            }),
        );
    }

    /// The provider call and each tool call of the turn. Names, sizes and decisions only.
    pub(super) fn audit_call_ok(
        &self,
        ctx: &Value,
        spec: &ProviderSpec,
        attempt: &Attempt,
        started: Instant,
    ) {
        self.audit(
            "llm_call",
            ctx,
            json!({
                "provider": spec.name,
                "status": "ok",
                "latency_ms": started.elapsed().as_millis() as u64,
                "first_token_ms": attempt.first_token_ms,
                "input_tokens": attempt.usage.map(|u| u.input_tokens),
                "output_tokens": attempt.usage.map(|u| u.output_tokens),
            }),
        );
        for (call, decision) in &attempt.tools {
            self.audit(
                "tool_call",
                ctx,
                json!({
                    "tool": call.split_whitespace().next().unwrap_or(""),
                    "arg_bytes": call.len(),
                    "decision": decision.as_str(),
                }),
            );
        }
    }

    pub(super) fn audit_turn_finished(
        &self,
        ctx: &Value,
        level: u8,
        started: Instant,
        outcome: &Outcome,
    ) {
        self.audit(
            "turn_finished",
            ctx,
            json!({
                "outcome": "completed",
                "audience": conversation::audience_for(level),
                "level": level,
                "duration_ms": started.elapsed().as_millis() as u64,
                "buffer_set": outcome.buffer.is_some(),
            }),
        );
    }

    /// The summary of one request for `--explain` and `--timings`: no content.
    pub(super) fn report(
        &self,
        spec: &ProviderSpec,
        sent: &Sent,
        convo: &Convo,
        active: &Option<Active>,
        attempt: &Attempt,
        timing: &Timing,
    ) -> Report {
        Report {
            provider_chain: self.providers.iter().map(|p| p.name.clone()).collect(),
            trust: Some(spec.trust),
            level: sent.level,
            redaction: self.has_redactor,
            conversation: active.as_ref().map(|a| a.id.clone()),
            turn: convo.turns.len() + 1,
            omitted_turns: sent.omitted,
            tools: attempt
                .tools
                .iter()
                .map(|(c, d)| {
                    if d.went_ahead() {
                        c.clone()
                    } else {
                        format!("denied: {c}")
                    }
                })
                .collect(),
            input_tokens: attempt.usage.map(|u| u.input_tokens),
            output_tokens: attempt.usage.map(|u| u.output_tokens),
            llm_ms: timing.llm_us / 1000,
            first_token_ms: attempt.first_token_ms,
            stages: timing.stages(attempt.first_token_us),
        }
    }

    /// Store the finished turn. A store that cannot save is not an error for the request: the person is
    /// told in the trailer.
    pub(super) fn save_turn(
        &self,
        active: &mut Option<Active>,
        env: &Envelope,
        spec: &ProviderSpec,
        sent: Sent,
        attempt: Attempt,
        outcome: &mut Outcome,
    ) {
        let (Some(a), Some(store)) = (active.as_mut(), &self.store) else {
            return;
        };
        let mut messages = vec![Stored::text(Role::User, sent.user)];
        messages.extend(attempt.stored);
        messages.push(Stored::text(Role::Assistant, attempt.text));
        let turn = Turn {
            at: self.host.clock.unix_secs(),
            action: env.action.clone(),
            input: env.buffer.clone(),
            level: sent.level,
            audience: conversation::audience_for(sent.level),
            provider: spec.name.clone(),
            suggestion: outcome.buffer.clone(),
            messages,
            outcome: conversation::Outcome::Completed,
        };
        if let Err(e) = a.append(store, turn, self.cfg.conversation.max_turns) {
            outcome.notes.push(Remark::labelled(
                Tone::Warning,
                "note",
                format!("could not save this turn: {}", e.message),
            ));
        }
    }
}
