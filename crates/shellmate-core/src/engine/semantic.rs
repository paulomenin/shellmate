//! Checking a suggested command: deterministic checks plus the semantic check, answered by a decision
//! model or, when allowed, the LLM.

use super::terminals::{ProviderTerminal, Recorder};
use super::Engine;
use crate::config::Trust;
use crate::sanitize;
use crate::validate;
use serde_json::json;
use shellmate_api::{ChatRequest, DecisionRequest, DecisionResponse, Error, Message, Next, Result};
use std::sync::Mutex;

impl Engine {
    /// Checks on the suggested command (see `validate`). Advisory only: the notes are shown, the command is
    /// still placed in the buffer. Content-free audit event.
    pub(super) async fn validate_suggestion(
        &self,
        request: &str,
        command: &str,
        shell: &str,
        ctx: &serde_json::Value,
    ) -> Vec<validate::Note> {
        let v = &self.cfg.validator;
        let (mut notes, mut risk_noted) = (Vec::new(), false);
        if v.enabled {
            (notes, risk_noted) = validate::deterministic(
                command,
                shell,
                &self.host.env.var("PATH").unwrap_or_default(),
            )
            .await;
        }
        let mut semantic = "off".to_string();
        if v.semantic {
            let limit = crate::config::parse_duration(&v.timeout)
                .unwrap_or(std::time::Duration::from_secs(5));
            let asked = validate::questions(request, command, shell);
            match tokio::time::timeout(limit, self.decide(&asked)).await {
                Ok(Ok((resp, who))) => {
                    notes.extend(validate::findings(&resp, v.min_confidence, risk_noted));
                    semantic = who;
                }
                Ok(Err(why)) => {
                    notes.push(validate::Note(format!(
                        "the semantic check was skipped: {why}"
                    )));
                    semantic = "failed".into();
                }
                Err(_) => {
                    notes.push(validate::Note(
                        "the semantic check was skipped: timed out".into(),
                    ));
                    semantic = "timeout".into();
                }
            }
        }
        if v.enabled || v.semantic {
            self.audit(
                "validation",
                ctx,
                json!({"notes": notes.len(), "semantic": semantic}),
            );
        }
        notes
    }

    /// Ask the configured decision models in order; if none answers (or none is configured) and it is
    /// allowed, let the LLM answer the same questions. Returns who answered.
    pub(super) async fn decide(
        &self,
        asked: &DecisionRequest,
    ) -> std::result::Result<(DecisionResponse, String), String> {
        let mut last = None;
        for d in &self.deciders {
            let sent = match d.trust {
                Trust::Local => asked.clone(),
                Trust::Remote => self.redacted(asked).await.map_err(|e| e.message)?,
            };
            match d.decider.decide(&sent).await {
                Ok(r) => return Ok((r, format!("decider:{}", d.name))),
                Err(e) => {
                    last = Some(format!(
                        "decider `{}`: {}",
                        d.name,
                        sanitize::escape(&e.message)
                    ))
                }
            }
        }
        if self.cfg.validator.llm_fallback && !self.providers.is_empty() {
            match self.llm_decide(asked).await {
                Ok(r) => return Ok((r, "llm".into())),
                Err(e) => last = Some(last.map_or(e.clone(), |l| format!("{l}; llm: {e}"))),
            }
        }
        Err(last.unwrap_or_else(|| {
            "no decision model is configured and the LLM fallback is off".to_string()
        }))
    }

    /// The record with each value passed through the `llm` middleware (redaction), for a remote decider.
    pub(super) async fn redacted(&self, asked: &DecisionRequest) -> Result<DecisionRequest> {
        let request = ChatRequest {
            request_id: self.request_id(),
            messages: asked
                .state
                .iter()
                .map(|(_, v)| Message::user(v.clone()))
                .collect(),
            max_tokens: None,
            temperature: None,
            redaction_key: None,
        };
        let recorder = Recorder(Mutex::new(None));
        Next::new(&self.layers, &recorder).run(request).await?;
        let seen = recorder
            .0
            .lock()
            .expect("recorder lock")
            .take()
            .ok_or_else(|| {
                Error::new(
                    "validator.redaction",
                    "the record did not pass the redaction chain",
                )
            })?;
        if seen.messages.len() != asked.state.len() {
            return Err(Error::new(
                "validator.redaction",
                "the redaction chain changed the record",
            ));
        }
        let mut out = asked.clone();
        for ((_, v), m) in out.state.iter_mut().zip(seen.messages) {
            *v = m.content;
        }
        Ok(out)
    }

    /// The same questions, answered by the first provider of the chain that can (through the `llm` chain,
    /// so redaction applies as for any request).
    pub(super) async fn llm_decide(
        &self,
        asked: &DecisionRequest,
    ) -> std::result::Result<DecisionResponse, String> {
        let mut last = "no provider".to_string();
        for spec in &self.providers {
            let request = ChatRequest {
                request_id: self.request_id(),
                messages: validate::llm_messages(asked),
                max_tokens: Some(200),
                temperature: Some(0.0),
                redaction_key: None,
            };
            if let Err(e) = spec.provider.prepare().await {
                last = e.message;
                continue;
            }
            let terminal = ProviderTerminal(spec.provider.as_ref());
            let stream = match Next::new(&self.layers, &terminal).run(request).await {
                Ok(s) => s,
                Err(e) => {
                    last = e.message;
                    continue;
                }
            };
            match shellmate_api::collect(stream).await {
                Ok(r) => match validate::parse_llm_reply(&r.text, asked) {
                    Some(resp) => return Ok(resp),
                    None => last = format!("`{}` did not answer in the expected format", spec.name),
                },
                Err(e) => last = e.message,
            }
        }
        Err(sanitize::escape(&last))
    }
}
