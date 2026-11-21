//! What a request would send, without sending it: `--dry-run` and `shellmate context`.

use super::terminals::Recorder;
use super::{Action, ContextItem, ContextReport, Engine, ItemState, Preview, ProviderSpec};
use crate::config::Trust;
use crate::envelope::Envelope;
use crate::prompt::Context;
use shellmate_api::{Error, Next, Result};

impl Engine {
    /// What each provider in the chain would receive, after redaction. Nothing is sent (`--dry-run`).
    pub async fn preview(&self, env: &Envelope) -> Result<Vec<Preview>> {
        self.check_privacy()?;
        let action: Action = env.action.parse()?;
        if action == Action::NewConversation {
            return Err(Error::new(
                "action.no_request",
                "`new-conversation` does not send a request",
            ));
        }
        if self.providers.is_empty() {
            return Err(Error::new(
                "config.no_provider",
                "shellmate isn't configured yet (no LLM provider)",
            )
            .with_hint("run `shellmate setup`, or choose a provider in the config (`shellmate config edit`)"));
        }
        let convo = self.peek(env, action)?;
        let gathered = self.prefetch(env).await;
        let mut out = Vec::new();
        for spec in &self.providers {
            // a dry run sends nothing, so it only knows what was remembered about the model
            let mode = self.tool_mode(spec, action, false).await;
            let built = self.build_request(action, env, spec, &convo, &gathered, mode)?;
            let recorder = Recorder::default();
            Next::new(&self.layers, &recorder)
                .run(built.request)
                .await?;
            let request = recorder.take().ok_or_else(|| {
                Error::new(
                    "preview.no_request",
                    format!(
                        "a middleware layer answered without passing the request on, so there is nothing to show for `{}`",
                        spec.name
                    ),
                )
                .with_hint("a layer in [pipeline] llm that does not call the next handler sends nothing; remove it to preview")
            })?;
            out.push(Preview {
                provider: spec.name.clone(),
                trust: spec.trust,
                level: built.level,
                request,
            });
        }
        Ok(out)
    }

    /// Which pieces of context each provider in the chain would receive, and why the others are held back.
    /// Nothing is sent and no content is shown (`shellmate context`).
    pub async fn context_report(&self, env: &Envelope) -> Result<Vec<ContextReport>> {
        self.check_privacy()?;
        if self.providers.is_empty() {
            return Err(Error::new(
                "config.no_provider",
                "shellmate isn't configured yet (no LLM provider)",
            )
            .with_hint("run `shellmate setup`, or choose a provider in the config (`shellmate config edit`)"));
        }
        let gathered = self.prefetch(env).await;
        let ctx = self.context(env, &gathered);
        // what there is to send is the same for every provider; what each may receive is not
        let facts = self.context_facts(env, &ctx);
        let mut out = Vec::new();
        for spec in &self.providers {
            let level = self.default_level().min(spec.max_level);
            out.push(ContextReport {
                provider: spec.name.clone(),
                trust: spec.trust,
                level,
                ceiling: spec.max_level,
                redaction: self.has_redactor,
                tainted: level >= 2 && (ctx.capture.is_some() || ctx.git_untrusted),
                items: facts
                    .iter()
                    .map(|fact| self.gate(fact, level, spec))
                    .collect(),
            });
        }
        Ok(out)
    }

    /// Whether `fact` is part of what `spec` receives at `level`, and the detail to show.
    fn gate(&self, fact: &Fact, level: u8, spec: &ProviderSpec) -> ContextItem {
        let min = fact.min;
        let (state, detail) = if level < min {
            let why = if self.default_level() < min {
                format!(
                    "needs level {min}; [context] level is {}",
                    self.default_level()
                )
            } else {
                format!(
                    "needs level {min}; {} providers are capped at {}",
                    if spec.trust == Trust::Local {
                        "local"
                    } else {
                        "remote"
                    },
                    spec.max_level
                )
            };
            (ItemState::Withheld, why)
        } else {
            match &fact.found {
                Ok(d) => (ItemState::Sent, d.clone()),
                Err(d) => (ItemState::Withheld, d.clone()),
            }
        };
        ContextItem {
            name: fact.name,
            level: min,
            state,
            detail,
        }
    }

    /// Every piece of context a request can carry, in the order `shellmate context` lists them, with what
    /// there is to send right now (a size, a count) or why there is nothing.
    fn context_facts(&self, env: &Envelope, ctx: &Context) -> Vec<Fact> {
        let c = &self.cfg.context;
        vec![
            Fact::new(
                "your typed text",
                0,
                Ok(format!("{} chars", env.buffer.chars().count())),
            ),
            Fact::new(
                "shell and system",
                1,
                Ok(format!(
                    "{}{} on {}",
                    ctx.shell,
                    ctx.shell_version
                        .as_deref()
                        .map(|v| format!(" {v}"))
                        .unwrap_or_default(),
                    ctx.os
                )),
            ),
            Fact::new(
                "working directory",
                1,
                ctx.cwd
                    .clone()
                    .ok_or_else(|| "the shell did not report one".to_string()),
            ),
            Fact::new("project type", 1, project_fact(c.project, ctx)),
            Fact::new(
                "last exit status",
                2,
                ctx.last_status
                    .map(|s| format!("status {s}"))
                    .ok_or_else(|| "the shell did not report one".into()),
            ),
            Fact::new(
                "recent commands",
                2,
                if ctx.history.is_empty() {
                    Err("none reported".into())
                } else {
                    Ok(format!(
                        "{} command line(s), up to {}",
                        ctx.history.len(),
                        self.cfg.context.history_lines
                    ))
                },
            ),
            Fact::new("git state", 2, git_fact(c.git, c.git_log_lines, ctx)),
            Fact::new(
                "captured output",
                2,
                ctx.capture
                    .as_deref()
                    .map(|s| {
                        format!(
                            "last `shellmate run`, {} bytes at most {}",
                            s.len(),
                            c.capture_bytes
                        )
                    })
                    .ok_or_else(|| "nothing captured (wrap a command with `shellmate run`)".into()),
            ),
        ]
    }
}

/// One piece of context: its name, the level that unlocks it, and what there is to send or why not.
struct Fact {
    name: &'static str,
    min: u8,
    found: std::result::Result<String, String>,
}

impl Fact {
    fn new(name: &'static str, min: u8, found: std::result::Result<String, String>) -> Self {
        Self { name, min, found }
    }
}

fn project_fact(enabled: bool, ctx: &Context) -> std::result::Result<String, String> {
    if !enabled {
        return Err("turned off ([context] project = false)".into());
    }
    ctx.project
        .as_ref()
        .map(|p| format!("{} (file names only)", p.kinds.join(", ")))
        .ok_or_else(|| "no known project files found here".into())
}

fn git_fact(enabled: bool, log_lines: usize, ctx: &Context) -> std::result::Result<String, String> {
    if !enabled {
        return Err("turned off ([context] git = false)".into());
    }
    ctx.git
        .as_deref()
        .map(|_| {
            if log_lines > 0 {
                format!("branch, change counts and {log_lines} commit subject(s)")
            } else {
                "branch and change counts".to_string()
            }
        })
        .ok_or_else(|| "not a git repository".into())
}
