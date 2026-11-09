//! Conversations (`conversation.md`): the turn format, which conversation a request continues, and how
//! stored turns become the history sent to a provider.

use crate::config::{ConversationMode, HistoryConfig};
use serde::{Deserialize, Serialize};
use shellmate_api::{ConversationLock, Error, Message, Registry, Result, Role, Store};
use std::sync::Arc;
use std::time::Duration;

pub const SCHEMA: u32 = 1;
/// How long a second request waits for the conversation before failing with `conversation.busy`.
const LOCK_WAIT: Duration = Duration::from_secs(2);

/// The least trusted provider a turn may be sent to again. Tags only ever get stricter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    /// Only providers marked `trust = "local"`.
    Local,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    pub role: Role,
    pub kind: Kind,
    pub content: String,
}

impl Stored {
    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            kind: Kind::Text,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Interrupted,
}

/// One request and everything that followed until the final answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub at: u64,
    pub action: String,
    /// What the input line held.
    pub input: String,
    /// The context level the answering provider was given (a summary; the context itself is not stored).
    pub level: u8,
    pub audience: Audience,
    pub provider: String,
    /// The command the turn left in the buffer, if any.
    #[serde(default)]
    pub suggestion: Option<String>,
    pub messages: Vec<Stored>,
    pub outcome: Outcome,
}

/// Audience rule: context above level 1 (last status, recent commands) is for local models only, and
/// so is whatever the answer may have repeated from it.
pub fn audience_for(level: u8) -> Audience {
    if level > 1 {
        Audience::Local
    } else {
        Audience::Any
    }
}

/// History for one provider.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct History {
    pub messages: Vec<Message>,
    /// Turns left out because the provider is less trusted than their audience.
    pub omitted_restricted: usize,
}

/// Turn the stored turns into messages: keep the recent window, drop what the provider may not see, and
/// replace old tool results by stubs. Whole turns only, so a tool call never loses its result.
pub fn assemble(turns: &[Turn], provider_is_local: bool, cfg: &HistoryConfig) -> History {
    let start = turns.len().saturating_sub(cfg.keep_recent_turns);
    let window = &turns[start..];
    let mut out = History::default();
    for (i, turn) in window.iter().enumerate() {
        if !provider_is_local && turn.audience == Audience::Local {
            out.omitted_restricted += 1;
            continue;
        }
        let age = window.len() - 1 - i; // 0 = the most recent turn
        for m in &turn.messages {
            let content = match m.kind {
                Kind::ToolResult if age >= cfg.elide_tool_results_after => {
                    format!("[tool result elided: {} bytes]", m.content.len())
                }
                _ => m.content.clone(),
            };
            out.messages.push(Message {
                role: m.role,
                content,
            });
        }
    }
    out
}

/// A conversation opened for one request. Holds the lock until dropped.
pub struct Active {
    pub id: String,
    pub key: [u8; 16],
    pub turns: Vec<Turn>,
    _lock: Box<dyn ConversationLock>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn random_key() -> Result<[u8; 16]> {
    use std::io::Read;
    let mut key = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut key))
        .map_err(|e| {
            Error::new(
                "redact.no_entropy",
                format!("cannot read /dev/urandom: {e}"),
            )
        })?;
    Ok(key)
}

/// The store the config names, or `None` for `store = "none"`.
pub fn open_store(
    cfg: &crate::Config,
    registry: &Registry,
    host: &crate::ports::Host,
) -> Result<Option<Arc<dyn Store>>> {
    if cfg.conversation.store == "none" {
        return Ok(None);
    }
    let id = crate::config::builtin_id(&cfg.conversation.store)?;
    let settings = cfg.plugin_settings(&cfg.conversation.store, host)?;
    registry.build_store(id, &settings).map(Some)
}

/// Create a conversation for `session`, make it current, and open it.
pub fn start(store: &Arc<dyn Store>, session: &str) -> Result<Active> {
    let key = random_key()?;
    let id = store.create(&serde_json::json!({
        "schema": SCHEMA,
        "session": session,
        "redaction_key": hex(&key),
    }))?;
    store.set_current(session, &id)?;
    let lock = store.lock(&id, LOCK_WAIT)?;
    Ok(Active {
        id,
        key,
        turns: Vec::new(),
        _lock: lock,
    })
}

/// Open an existing conversation (lock it and read its turns).
pub fn open(store: &Arc<dyn Store>, id: &str) -> Result<Active> {
    let lock = store.lock(id, LOCK_WAIT)?;
    let (header, raw) = store.read(id, None)?;
    let key = header["redaction_key"]
        .as_str()
        .and_then(unhex)
        .ok_or_else(|| Error::new("conversation.corrupt", "conversation has no redaction key"))?;
    let turns = raw
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    Ok(Active {
        id: id.to_string(),
        key,
        turns,
        _lock: lock,
    })
}

/// Which conversation does this request continue? `None` means stateless (`fresh`, no session). `now` is
/// Unix seconds, compared with the store's last activity for the idle timeout.
pub fn resolve(
    store: &Arc<dyn Store>,
    session: Option<&str>,
    mode: ConversationMode,
    idle: Duration,
    now: u64,
) -> Result<Option<Active>> {
    let Some(session) = session else {
        return Ok(None);
    };
    if mode == ConversationMode::Fresh {
        return Ok(None);
    }
    match store.current(session)? {
        Some(cur) => {
            let idle_for = now.saturating_sub(cur.last_activity);
            if mode == ConversationMode::Auto && idle_for > idle.as_secs() {
                start(store, session).map(Some)
            } else {
                open(store, &cur.conversation).map(Some)
            }
        }
        None => start(store, session).map(Some),
    }
}

/// Read-only counterpart of [`resolve`] for `--dry-run`: the turns and key of the conversation that
/// would be continued, or `None` when a new one would start. Creates and locks nothing.
pub fn peek(
    store: &Arc<dyn Store>,
    session: &str,
    mode: ConversationMode,
    idle: Duration,
    now: u64,
) -> Result<Option<(Vec<Turn>, [u8; 16])>> {
    if mode == ConversationMode::Fresh {
        return Ok(None);
    }
    let Some(cur) = store.current(session)? else {
        return Ok(None);
    };
    if mode == ConversationMode::Auto && now.saturating_sub(cur.last_activity) > idle.as_secs() {
        return Ok(None);
    }
    let (header, raw) = store.read(&cur.conversation, None)?;
    let Some(key) = header["redaction_key"].as_str().and_then(unhex) else {
        return Ok(None);
    };
    let turns = raw
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    Ok(Some((turns, key)))
}

impl Active {
    pub fn append(&mut self, store: &Arc<dyn Store>, turn: Turn, max_turns: usize) -> Result<()> {
        store.append_turn(
            &self.id,
            &serde_json::to_value(&turn)
                .map_err(|e| Error::new("conversation.store", e.to_string()))?,
            max_turns,
        )?;
        self.turns.push(turn);
        Ok(())
    }

    /// The command the last turn left in the buffer.
    pub fn last_suggestion(&self) -> Option<&str> {
        self.turns.last().and_then(|t| t.suggestion.as_deref())
    }
}

/// One-line description of a turn for `conversation list/show`.
pub fn summarize(turn: &Turn) -> String {
    let first = turn.input.lines().next().unwrap_or("");
    let shown: String = first.chars().take(60).collect();
    format!("{} {}", turn.action, shown)
}

#[cfg(test)]
mod tests;
