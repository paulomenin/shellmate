//! One provider's turn: stream the answer and, while the model asks for tools, run them and ask again.

use super::terminals::ProviderTerminal;
use super::tool_calls::ToolDecision;
use super::{Emit, Engine, ProviderSpec};
use crate::config::Trust;
use crate::conversation::{Kind, Stored};
use crate::response;
use crate::tools::ToolCall;
use shellmate_api::{ChatRequest, Error, Message, Next, Role, StreamEvent, Usage};
use std::time::Instant;

/// How the turn is run, the same for every provider that is tried.
pub(super) struct TurnOptions<'a> {
    /// Show the model's own words as they arrive (`generate` does not).
    pub(super) show_text: bool,
    /// The request carries text written by others.
    pub(super) tainted: bool,
    pub(super) cwd: &'a Option<String>,
    /// A command may be sent to the input line (not for `explain`).
    pub(super) buffer_ok: bool,
}

impl Engine {
    /// One provider's turn: stream the answer, and while the model asks for tools, run them (through
    /// the classifier and the policy) and ask again.
    pub(super) async fn attempt(
        &self,
        spec: &ProviderSpec,
        mut request: ChatRequest,
        turn: TurnOptions<'_>,
        on_text: &mut (dyn FnMut(Emit<'_>) + Send),
    ) -> std::result::Result<Attempt, Failure> {
        let TurnOptions {
            show_text,
            mut tainted,
            cwd,
            buffer_ok,
        } = turn;
        let max_rounds = self.cfg.conversation.max_tool_rounds;
        let (mut rounds, mut produced, mut at_line_start) = (0usize, false, true);
        let mut stored = Vec::new();
        let mut to_buffer: Option<String> = None;
        let (mut tools_used, mut usage, mut first_token_ms) = (Vec::new(), None::<Usage>, None);
        let mut first_token_us = None::<u64>;
        let t0 = Instant::now();
        // Secrets fetched on first use (`cmd:`): a failure here is a configuration error, and nothing has
        // been sent or shown yet.
        spec.provider.prepare().await.map_err(|error| Failure {
            error,
            produced: false,
        })?;
        loop {
            let fail = |error: Error, produced: bool| Failure { error, produced };
            let terminal = ProviderTerminal(spec.provider.as_ref());
            let mut stream = Next::new(&self.layers, &terminal)
                .run(request.clone())
                .await
                .map_err(|e| fail(e, produced))?;
            let mut full = String::new();
            let mut filter = response::DisplayFilter::default();
            loop {
                match stream.next().await {
                    Some(Ok(StreamEvent::Text(t))) => {
                        if first_token_ms.is_none() && !t.is_empty() {
                            first_token_ms = Some(t0.elapsed().as_millis() as u64);
                            first_token_us = Some(t0.elapsed().as_micros() as u64);
                        }
                        full.push_str(&t);
                        let shown = filter.push(&t);
                        if show_text && !shown.is_empty() {
                            at_line_start = shown.ends_with('\n');
                            on_text(Emit::Text(&shown));
                        }
                    }
                    Some(Ok(StreamEvent::Usage(u))) => {
                        let sum = usage.unwrap_or_default();
                        usage = Some(Usage {
                            input_tokens: sum.input_tokens + u.input_tokens,
                            output_tokens: sum.output_tokens + u.output_tokens,
                        });
                    }
                    Some(Ok(StreamEvent::Done { .. })) | None => break,
                    Some(Err(e)) => {
                        return Err(fail(Error::from(e), produced || !full.is_empty()));
                    }
                }
            }
            let tail = filter.finish();
            if show_text && !tail.is_empty() {
                at_line_start = tail.ends_with('\n');
                on_text(Emit::Text(&tail));
            }
            produced |= !full.is_empty();
            let answer = response::parse(&full);
            let Some(body) = answer.tool_calls.first().filter(|_| rounds < max_rounds) else {
                return Ok(Attempt {
                    text: full,
                    stored,
                    usage,
                    tools: tools_used,
                    to_buffer,
                    first_token_ms,
                    first_token_us,
                });
            };
            rounds += 1;
            let dest = format!(
                "`{}` ({})",
                spec.name,
                if spec.trust == Trust::Local {
                    "local"
                } else {
                    "remote, after redaction"
                }
            );
            let run = self
                .run_tool(body, cwd.as_deref(), tainted, &dest, buffer_ok)
                .await;
            tainted |= run.untrusted;
            let (mut notice, result) = (run.notice, run.result);
            if run.to_buffer.is_some() {
                to_buffer = run.to_buffer;
            }
            tools_used.push((
                ToolCall::parse(body)
                    .map(|c| c.display())
                    .unwrap_or_else(|_| "invalid".into()),
                run.decision,
            ));
            notice.gap = !at_line_start;
            on_text(Emit::Notice(&notice));
            at_line_start = true;
            request.messages.push(Message::assistant(full.clone()));
            request.messages.push(Message::user(result.clone()));
            stored.push(Stored {
                role: Role::Assistant,
                kind: Kind::ToolCall,
                content: full,
            });
            stored.push(Stored {
                role: Role::User,
                kind: Kind::ToolResult,
                content: result,
            });
        }
    }
}

pub(super) struct Attempt {
    /// The final answer text (command blocks included).
    pub(super) text: String,
    /// Tool calls and results of this turn, in order.
    pub(super) stored: Vec<Stored>,
    pub(super) usage: Option<Usage>,
    /// (tool call as shown, allowed)
    pub(super) tools: Vec<(String, ToolDecision)>,
    /// A command the user chose to run themselves (`b` at the approval prompt).
    pub(super) to_buffer: Option<String>,
    pub(super) first_token_ms: Option<u64>,
    pub(super) first_token_us: Option<u64>,
}

pub(super) struct Failure {
    pub(super) error: Error,
    /// Something already reached the user or ran, so trying another provider is not allowed.
    pub(super) produced: bool,
}
