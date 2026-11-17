//! Checking a suggested command: deterministic checks plus the semantic check, answered by a decision
//! model or, when allowed, the LLM.

use super::terminals::{ProviderTerminal, Recorder};
use super::{Check, Engine, ProviderSpec};
use crate::config::Trust;
use crate::sanitize;
use crate::validate;
use serde_json::json;
use shellmate_api::{ChatRequest, DecisionRequest, DecisionResponse, Error, Message, Next, Result};

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
        let asking = !self.checks.is_empty();
        if asking {
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
        if v.enabled || asking {
            self.audit(
                "validation",
                ctx,
                json!({"notes": notes.len(), "semantic": semantic}),
            );
        }
        notes
    }

    /// Ask the models of the validator chain in order: a decision model with `decide`, a chat model with the
    /// same questions as a JSON reply. Returns who answered.
    pub(super) async fn decide(
        &self,
        asked: &DecisionRequest,
    ) -> std::result::Result<(DecisionResponse, String), String> {
        let mut last: Option<String> = None;
        for check in &self.checks {
            let answered = match check {
                Check::Decision(d) => {
                    let sent = match d.trust {
                        Trust::Local => asked.clone(),
                        Trust::Remote => self.redacted(asked).await.map_err(|e| e.message)?,
                    };
                    d.decider
                        .decide(&sent)
                        .await
                        .map_err(|e| sanitize::escape(&e.message))
                }
                Check::Chat(p) => self.llm_decide(p, asked).await,
            };
            match answered {
                Ok(r) => return Ok((r, format!("model:{}", check.name()))),
                Err(why) => {
                    let why = format!("`{}`: {why}", check.name());
                    last = Some(last.map_or(why.clone(), |l| format!("{l}; {why}")));
                }
            }
        }
        Err(last.unwrap_or_else(|| "no model is configured for it".to_string()))
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
        let recorder = Recorder::default();
        Next::new(&self.layers, &recorder).run(request).await?;
        let seen = recorder.take().ok_or_else(|| {
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

    /// The same questions, answered by a chat model through the `llm` chain, so redaction applies as for any
    /// request.
    pub(super) async fn llm_decide(
        &self,
        spec: &ProviderSpec,
        asked: &DecisionRequest,
    ) -> std::result::Result<DecisionResponse, String> {
        let request = ChatRequest {
            request_id: self.request_id(),
            messages: validate::llm_messages(asked),
            max_tokens: Some(200),
            temperature: Some(0.0),
            redaction_key: None,
        };
        spec.provider
            .prepare()
            .await
            .map_err(|e| sanitize::escape(&e.message))?;
        let terminal = ProviderTerminal(spec.provider.as_ref());
        let stream = Next::new(&self.layers, &terminal)
            .run(request)
            .await
            .map_err(|e| sanitize::escape(&e.message))?;
        let reply = shellmate_api::collect(stream)
            .await
            .map_err(|e| sanitize::escape(&e.message))?;
        validate::parse_llm_reply(&reply.text, asked)
            .ok_or_else(|| "did not answer in the expected format".to_string())
    }
}
