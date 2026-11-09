//! The lifecycle of one request: resolve the conversation, try the providers in order, check the
//! suggestion, record the turn.

use super::request::Convo;
use super::turn::{Sent, Timing};
use super::{Action, Emit, Engine, Outcome, Report};
use crate::conversation::{self, Active};
use crate::envelope::Envelope;
use crate::present::{Remark, Tone};
use crate::response;
use crate::sanitize;
use serde_json::json;
use shellmate_api::{Error, Result};
use std::time::Instant;

impl Engine {
    /// Run the action without incremental output.
    pub async fn run(&self, env: &Envelope) -> Result<Outcome> {
        self.run_streaming(env, &mut |_| {}).await
    }

    pub(super) fn new_conversation(
        &self,
        env: &Envelope,
        on_text: &mut (dyn FnMut(Emit<'_>) + Send),
    ) -> Result<Outcome> {
        let (Some(store), Some(session)) = (&self.store, env.session.as_deref()) else {
            return Err(Error::new(
                "conversation.unavailable",
                "there is no conversation to reset here",
            )
            .with_hint("conversations need the shell integration (`shellmate init`) and a store"));
        };
        conversation::start(store, session)?;
        let text = "Started a new conversation.";
        on_text(Emit::Text(&format!("{text}\n")));
        Ok(Outcome {
            text: text.into(),
            notes: Vec::new(),
            buffer: None,
            provider: String::new(),
            report: Report::default(),
        })
    }

    /// Run the action, calling `on_text` with display text (command blocks removed) as it arrives.
    ///
    /// Fallback to the next provider happens only while nothing has been shown: once text is out it
    /// cannot be retracted, so a failure after that point is final.
    pub async fn run_streaming(
        &self,
        env: &Envelope,
        on_text: &mut (dyn FnMut(Emit<'_>) + Send),
    ) -> Result<Outcome> {
        self.check_privacy()?;
        let action: Action = env.action.parse()?;
        if action == Action::NewConversation {
            return self.new_conversation(env, on_text);
        }
        self.require_a_provider()?;
        // `generate` is command-only: the model's own words are not shown live (tool notices still are).
        let show_text = action != Action::Generate;
        let cwd = env.cwd.clone();
        let run_started = Instant::now();
        self.prefetch(env).await;
        let context_us = run_started.elapsed().as_micros() as u64;
        // The conversation stays locked for the whole turn, so two requests cannot interleave.
        let mut active = self.resolve_conversation(env, action)?;
        let convo = match &active {
            Some(a) => Convo::from_active(a),
            None => Convo::default(),
        };
        let rid = self.request_id();
        let ctx = json!({
            "session": env.session,
            "conversation": active.as_ref().map(|a| a.id.clone()),
            "request_id": rid,
        });
        self.audit_request_started(&ctx, env, &convo);
        let started = Instant::now();
        let mut last_error = None;
        for spec in &self.providers {
            let mut built = self.build_request(action, env, spec, &convo)?;
            built.request.request_id = rid.clone();
            let llm_started = Instant::now();
            let attempt = match self
                .attempt(spec, built.request, show_text, built.tainted, &cwd, on_text)
                .await
            {
                Ok(a) => a,
                Err(f) => {
                    let fallback = !f.produced && falls_back(&f.error);
                    self.audit_call_failed(&ctx, spec, &f, llm_started, fallback);
                    if fallback {
                        last_error = Some(f.error);
                        continue;
                    }
                    return Err(f.error);
                }
            };
            self.audit_call_ok(&ctx, spec, &attempt, llm_started);
            let mut outcome = finish(action, &attempt.text, &spec.name)?;
            if outcome.buffer.is_none() && matches!(action, Action::Ask | Action::Generate) {
                outcome.buffer = attempt.to_buffer.clone();
            }
            let validate_started = Instant::now();
            self.add_check_notes(action, env, &ctx, &mut outcome).await;
            let timing = Timing {
                run_started,
                context_us,
                llm_started,
                validate_started,
            };
            let sent = Sent {
                level: built.level,
                omitted: built.omitted,
                user: built.user,
            };
            outcome.report = self.report(spec, &sent, &convo, &active, &attempt, &timing);
            self.audit_turn_finished(&ctx, sent.level, started, &outcome);
            if sent.omitted > 0 {
                outcome.notes.push(Remark::labelled(
                    Tone::Info,
                    "note",
                    format!(
                        "{} earlier turn(s) were not sent to {} (restricted to local models)",
                        sent.omitted, spec.name
                    ),
                ));
            }
            self.save_turn(&mut active, env, spec, sent, attempt, &mut outcome);
            if let Some(n) = &self.store_note {
                outcome
                    .notes
                    .push(Remark::labelled(Tone::Warning, "note", n.clone()));
            }
            return Ok(outcome);
        }
        Err(self.all_providers_failed(last_error))
    }

    fn require_a_provider(&self) -> Result<()> {
        if self.providers.is_empty() {
            return Err(Error::new(
                "config.no_provider",
                "shellmate isn't configured yet (no LLM provider)",
            )
            .with_hint("run `shellmate setup`, or `shellmate config init`"));
        }
        Ok(())
    }

    /// The conversation this request continues (locked until the turn is saved), or `None` when the
    /// request is stateless.
    fn resolve_conversation(&self, env: &Envelope, action: Action) -> Result<Option<Active>> {
        match &self.store {
            Some(store) => conversation::resolve(
                store,
                env.session.as_deref(),
                self.cfg.conversation_mode(action),
                self.cfg.idle_timeout(action),
                self.host.clock.unix_secs(),
            ),
            None => Ok(None),
        }
    }

    /// Advisory notes about the suggested command, appended to the trailer. Only `ask` and `generate`
    /// suggest commands.
    async fn add_check_notes(
        &self,
        action: Action,
        env: &Envelope,
        ctx: &serde_json::Value,
        outcome: &mut Outcome,
    ) {
        if let (true, Some(cmd)) = (
            matches!(action, Action::Ask | Action::Generate),
            outcome.buffer.clone(),
        ) {
            for note in self
                .validate_suggestion(&env.buffer, &cmd, &env.shell, ctx)
                .await
            {
                outcome
                    .notes
                    .push(Remark::labelled(Tone::Warning, "check", note.0));
            }
        }
    }

    /// Every provider failed in a way that allows a fallback: report the last error.
    fn all_providers_failed(&self, last: Option<Error>) -> Error {
        let mut e = last.expect("at least one provider was tried");
        if self.providers.len() > 1 {
            e.message = format!(
                "all {} providers failed; last: {}",
                self.providers.len(),
                e.message
            );
        }
        e
    }
}

fn falls_back(e: &Error) -> bool {
    matches!(
        e.code,
        "provider.timeout" | "provider.unavailable" | "provider.rate_limited"
    )
}

fn finish(action: Action, text: &str, provider: &str) -> Result<Outcome> {
    let answer = response::parse(text);
    let mut notes = Vec::new();
    let mut buffer = None;
    match action {
        Action::Explain | Action::NewConversation => {}
        Action::Ask | Action::Generate => {
            buffer = answer.commands.first().cloned();
            // `generate` is command-only: whatever else the model wrote is not shown when there is a command.
            if action == Action::Ask && answer.commands.len() > 1 {
                let options = answer.commands[1..]
                    .iter()
                    .map(|c| sanitize::escape(c))
                    .collect();
                notes.push(
                    Remark::new(Tone::Info, format!("{}:", response::OTHER_OPTIONS_LABEL))
                        .with_items(options),
                );
            }
        }
    }
    if answer.rejected > 0 {
        notes.push(Remark::new(
            Tone::Warning,
            format!(
                "(dropped {} suggestion(s) containing control characters)",
                answer.rejected
            ),
        ));
    }
    if answer.prose.is_empty() && buffer.is_none() && notes.is_empty() {
        return Err(
            Error::new("answer.empty", "the model returned an empty answer")
                .with_hint("try again, or use a different model"),
        );
    }
    let text = if action == Action::Generate && buffer.is_some() {
        String::new()
    } else {
        answer.prose
    };
    Ok(Outcome {
        text,
        notes,
        buffer,
        provider: provider.to_string(),
        report: Report::default(),
    })
}
