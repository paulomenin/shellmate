//! One provider's turn: stream the answer and, while the model asks for tools, run them and ask again.

use super::terminals::ProviderTerminal;
use super::tool_calls::{invalid_native_call, ToolDecision};
use super::{Emit, Engine, ProviderSpec};
use crate::config::Trust;
use crate::conversation::{Kind, Stored};
use crate::response;
use crate::tools::ToolCall;
use shellmate_api::{
    ChatRequest, Error, Message, Next, Role, StreamEvent, ToolChoice, ToolRequest, Usage,
};
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
            // the last round answers in words, so a model that keeps calling tools still ends the turn
            if rounds >= max_rounds && !request.tools.is_empty() {
                request.tool_choice = ToolChoice::None;
            }
            let fail = |error: Error, produced: bool| Failure { error, produced };
            let terminal = ProviderTerminal(spec.provider.as_ref());
            let mut stream = Next::new(&self.layers, &terminal)
                .run(request.clone())
                .await
                .map_err(|e| fail(e, produced))?;
            let mut full = String::new();
            let mut native_calls: Vec<ToolRequest> = Vec::new();
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
                    Some(Ok(StreamEvent::ToolCall(call))) => native_calls.push(call),
                    Some(Ok(StreamEvent::Usage(u))) => {
                        let sum = usage.unwrap_or_default();
                        usage = Some(Usage {
                            input_tokens: sum.input_tokens + u.input_tokens,
                            output_tokens: sum.output_tokens + u.output_tokens,
                        });
                    }
                    Some(Ok(StreamEvent::Done { .. })) | None => break,
                    Some(Err(e)) => {
                        return Err(fail(
                            Error::from(e),
                            produced || !full.is_empty() || !native_calls.is_empty(),
                        ));
                    }
                }
            }
            let tail = filter.finish();
            if show_text && !tail.is_empty() {
                at_line_start = tail.ends_with('\n');
                on_text(Emit::Text(&tail));
            }
            let answer = response::parse(&full);
            // The call to run: one the provider carried natively, or one written in the text. Either way it
            // is the same line from here on, so it meets the same parser, classifier and approval.
            let pending: Option<Result<String, String>> = if rounds >= max_rounds {
                None
            } else if let Some(call) = native_calls.first() {
                Some(self.native_line(call))
            } else {
                answer.tool_calls.first().map(|b| Ok(b.clone()))
            };
            let Some(pending) = pending else {
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
            // a tool is about to run (or be refused) for this turn, so another provider may not redo it
            produced = true;
            let dest = format!(
                "`{}` ({})",
                spec.name,
                if spec.trust == Trust::Local {
                    "local"
                } else {
                    "remote, after redaction"
                }
            );
            let run = match &pending {
                Ok(body) => {
                    self.run_tool(body, cwd.as_deref(), tainted, &dest, buffer_ok)
                        .await
                }
                Err(why) => invalid_native_call(why),
            };
            tainted |= run.untrusted;
            let (mut notice, result) = (run.notice, run.result);
            if run.to_buffer.is_some() {
                to_buffer = run.to_buffer;
            }
            tools_used.push((
                pending
                    .as_ref()
                    .ok()
                    .and_then(|body| ToolCall::parse(body).ok())
                    .map(|c| c.display())
                    .unwrap_or_else(|| "invalid".into()),
                run.decision,
            ));
            notice.gap = !at_line_start;
            on_text(Emit::Notice(&notice));
            at_line_start = true;
            // What the conversation keeps is always the text form, so a turn can be replayed to any model, in
            // either mode. What the provider is sent back is the form it speaks.
            let kept = if native_calls.is_empty() {
                request.messages.push(Message::assistant(full.clone()));
                request.messages.push(Message::user(result.clone()));
                full
            } else {
                for (i, call) in native_calls.iter().enumerate() {
                    // every call needs an answer, or the provider refuses the next request; only the first runs
                    let content = if i == 0 {
                        result.clone()
                    } else {
                        "Not run: only one tool call is handled at a time. Ask again if you still need it."
                            .to_string()
                    };
                    if i == 0 {
                        request.messages.push(Message::assistant_calling(
                            full.clone(),
                            native_calls.clone(),
                        ));
                    }
                    request
                        .messages
                        .push(Message::tool_result(call.id.clone(), content));
                }
                let line = match &pending {
                    Ok(body) => body.clone(),
                    Err(_) => format!(
                        "{} {}",
                        native_calls[0].name,
                        native_calls[0].arguments.replace(['\n', '\r'], " ")
                    ),
                };
                let said = if full.trim().is_empty() {
                    String::new()
                } else {
                    format!("{}\n", full.trim_end())
                };
                format!("{said}```{}\n{line}\n```", crate::response::TOOL_LABEL)
            };
            stored.push(Stored {
                role: Role::Assistant,
                kind: Kind::ToolCall,
                content: kept,
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
