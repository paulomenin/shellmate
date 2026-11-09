//! `shellmate conversation ...`: inspect and manage the session's conversations.

use clap::Subcommand;
use shellmate_api::{Error, Result, Role, Store};
use shellmate_core::conversation::{self, Kind, Turn};
use shellmate_core::present::{Presenter, Tone};
use shellmate_core::{sanitize, Config};
use std::sync::Arc;

#[derive(Subcommand)]
pub enum ConvoCmd {
    /// Start a new conversation for this shell session
    New,
    /// Print the current conversation, including tool calls
    Show,
    /// List stored conversations (the current one is marked)
    List,
    /// Make a stored conversation the current one
    Resume { id: String },
    /// Forget the last turn of the current conversation
    Undo,
    /// Delete the current conversation, or all of them
    Clear {
        #[arg(long)]
        all: bool,
    },
}

fn store(cfg: &Config) -> Result<Arc<dyn Store>> {
    conversation::open_store(
        cfg,
        &shellmate_builtin::registry(),
        &shellmate_core::ports::Host::system(),
    )?
    .ok_or_else(|| {
        Error::new(
            "conversation.unavailable",
            "conversations are turned off (`conversation.store = \"none\"`)",
        )
    })
}

fn session(explicit: Option<&str>) -> Result<String> {
    explicit.map(str::to_string).ok_or_else(|| {
        Error::new(
            "conversation.no_session",
            "this shell has no shellmate session",
        )
        .with_hint("load the shell integration (`shellmate init <shell>`), or pass --session")
    })
}

/// Escape what the model or a tool wrote; keep newlines, tabs and backslashes readable.
fn safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if sanitize::is_buffer_safe(c) {
                c.to_string()
            } else {
                sanitize::escape_char(c).unwrap_or_default()
            }
        })
        .collect()
}

fn age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

fn render(id: &str, turns: &[Turn], p: &dyn Presenter) -> String {
    let mut out = format!(
        "{} {id} ({} turns)\n",
        p.label(Tone::Info, "conversation"),
        turns.len()
    );
    for (i, t) in turns.iter().enumerate() {
        let head = format!(
            "--- turn {} · {} · {} · context level {} · {:?}",
            i + 1,
            t.action,
            t.provider,
            t.level,
            t.audience
        );
        out.push_str(&format!("\n{}\n", p.label(Tone::Muted, &head)));
        for m in &t.messages {
            let (label, tone) = match (m.role, m.kind) {
                (Role::User, Kind::ToolResult) => ("tool result", Tone::Muted),
                (Role::Assistant, Kind::ToolCall) => {
                    ("assistant (asked for a tool)", Tone::Warning)
                }
                (Role::User, _) => ("you", Tone::Info),
                (_, _) => ("assistant", Tone::Success),
            };
            out.push_str(&format!(
                "{}\n{}\n",
                p.label(tone, &format!("[{label}]")),
                safe(&m.content)
            ));
        }
    }
    out
}

pub fn run(
    cmd: ConvoCmd,
    cfg: &Config,
    session_id: Option<&str>,
    p: &dyn Presenter,
) -> Result<String> {
    let store = store(cfg)?;
    match cmd {
        ConvoCmd::New => {
            let s = session(session_id)?;
            let active = conversation::start(&store, &s)?;
            Ok(format!(
                "{} conversation {}\n",
                p.label(Tone::Success, "started"),
                active.id
            ))
        }
        ConvoCmd::Show => {
            let s = session(session_id)?;
            let Some(cur) = store.current(&s)? else {
                return Ok("no conversation yet in this session\n".into());
            };
            let (_, raw) = store.read(&cur.conversation, None)?;
            let turns: Vec<Turn> = raw
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect();
            Ok(render(&cur.conversation, &turns, p))
        }
        ConvoCmd::List => {
            let current = session_id
                .and_then(|s| store.current(s).ok().flatten())
                .map(|c| c.conversation);
            let now = shellmate_core::ports::Clock::unix_secs(&shellmate_core::ports::SystemClock);
            let mut out = String::new();
            for c in store.list()? {
                let mark = if current.as_deref() == Some(c.id.as_str()) {
                    "*"
                } else {
                    " "
                };
                let id = if mark == "*" {
                    p.label(Tone::Success, &c.id)
                } else {
                    c.id.clone()
                };
                out.push_str(&format!(
                    "{mark} {id}  {} turns  {}\n",
                    c.turns,
                    p.label(Tone::Muted, &age(now.saturating_sub(c.last_activity)))
                ));
            }
            if out.is_empty() {
                out.push_str("no conversations\n");
            }
            Ok(out)
        }
        ConvoCmd::Resume { id } => {
            let s = session(session_id)?;
            store.read(&id, Some(0))?; // must exist
            store.set_current(&s, &id)?;
            Ok(format!(
                "{} conversation {id}\n",
                p.label(Tone::Success, "resumed")
            ))
        }
        ConvoCmd::Undo => {
            let s = session(session_id)?;
            let Some(cur) = store.current(&s)? else {
                return Ok("nothing to undo\n".into());
            };
            let _lock = store.lock(&cur.conversation, std::time::Duration::from_secs(2))?;
            match store.pop_turn(&cur.conversation)? {
                Some(v) => {
                    let what = serde_json::from_value::<Turn>(v)
                        .map(|t| conversation::summarize(&t))
                        .unwrap_or_else(|_| "last turn".into());
                    Ok(format!(
                        "{} {}\n",
                        p.label(Tone::Success, "forgot:"),
                        safe(&what)
                    ))
                }
                None => Ok("nothing to undo\n".into()),
            }
        }
        ConvoCmd::Clear { all } => {
            if all {
                store.clear()?;
                return Ok(format!(
                    "{} all conversations\n",
                    p.label(Tone::Success, "deleted")
                ));
            }
            let s = session(session_id)?;
            match store.current(&s)? {
                Some(cur) => {
                    store.delete(&cur.conversation)?;
                    Ok(format!(
                        "{} conversation {}\n",
                        p.label(Tone::Success, "deleted"),
                        cur.conversation
                    ))
                }
                None => Ok("no conversation to delete\n".into()),
            }
        }
    }
}
