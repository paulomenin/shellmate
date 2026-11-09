//! Building the payload for one provider, and the conversation a request continues.

use super::{Action, Engine, ProviderSpec};
use crate::config::Trust;
use crate::conversation::{self, Turn};
use crate::envelope::Envelope;
use crate::prompt::{self, Extras};
use shellmate_api::{ChatRequest, Result};

/// What the engine knows about the conversation a request continues.
#[derive(Default)]
pub(super) struct Convo {
    pub(super) turns: Vec<Turn>,
    pub(super) key: Option<[u8; 16]>,
    pub(super) last_suggestion: Option<String>,
}

impl Convo {
    pub(super) fn from_active(a: &conversation::Active) -> Self {
        Self {
            turns: a.turns.clone(),
            key: Some(a.key),
            last_suggestion: a.last_suggestion().map(str::to_string),
        }
    }
    pub(super) fn from_turns((turns, key): (Vec<Turn>, [u8; 16])) -> Self {
        Self {
            last_suggestion: turns.last().and_then(|t| t.suggestion.clone()),
            turns,
            key: Some(key),
        }
    }
}

pub(super) struct Built {
    pub(super) level: u8,
    /// The request carries untrusted content (captured output): pre-approvals are suspended.
    pub(super) tainted: bool,
    pub(super) omitted: usize,
    /// The user message as sent (stored with the turn).
    pub(super) user: String,
    pub(super) request: ChatRequest,
}

impl Engine {
    /// The payload rebuilt for this provider: context level, history, and the whole chain depend on the
    /// receiver, so nothing prepared for one provider reaches another.
    pub(super) fn build_request(
        &self,
        action: Action,
        env: &Envelope,
        spec: &ProviderSpec,
        convo: &Convo,
    ) -> Result<Built> {
        let level = self.default_level().min(spec.max_level);
        let history = conversation::assemble(
            &convo.turns,
            spec.trust == Trust::Local,
            &self.cfg.conversation.history,
        );
        let tool_offers = super::tool_calls::offers(&self.cfg, &self.policy, action);
        let extras = Extras {
            history: &history.messages,
            tool_offers: &tool_offers,
            omitted: history.omitted_restricted,
            unchanged_suggestion: convo.last_suggestion.as_deref() == Some(env.buffer.as_str()),
        };
        let ctx = self.context(env);
        // Captured output is untrusted program output: a turn that carries it is tainted (INV-4).
        let tainted = level >= 2 && (ctx.capture.is_some() || ctx.git_untrusted);
        let messages = prompt::build_with(action, &ctx, level, &extras)?;
        let user = messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(Built {
            level,
            tainted,
            omitted: history.omitted_restricted,
            user,
            request: ChatRequest {
                request_id: self.request_id(),
                messages,
                max_tokens: None,
                temperature: Some(0.2),
                redaction_key: convo.key,
            },
        })
    }

    /// Read-only view of the conversation a request would continue (`--dry-run`): nothing is created,
    /// locked or written.
    pub(super) fn peek(&self, env: &Envelope, action: Action) -> Result<Convo> {
        let (Some(store), Some(session)) = (&self.store, env.session.as_deref()) else {
            return Ok(Convo::default());
        };
        let found = conversation::peek(
            store,
            session,
            self.cfg.conversation_mode(action),
            self.cfg.idle_timeout(action),
            self.host.clock.unix_secs(),
        )?;
        Ok(found.map(Convo::from_turns).unwrap_or_default())
    }
}
