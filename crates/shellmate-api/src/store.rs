//! Conversation storage (`conversation.md` §7.3). The core owns the turn format and hands the store
//! opaque JSON values; a store only has to be ordered, atomic per append, and lockable.

use crate::{Error, Result};
use serde_json::Value;
use std::time::Duration;

/// Held while a process works on a conversation; the lock is released on drop.
pub trait ConversationLock: Send {}

/// Which conversation a session points at, and when it last changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub conversation: String,
    /// Seconds since the Unix epoch of the last append (or of creation when empty).
    pub last_activity: u64,
}

/// One stored conversation as `list` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationInfo {
    pub id: String,
    pub turns: usize,
    pub last_activity: u64,
}

pub trait Store: Send + Sync {
    /// The session's current conversation, if it has one that still exists.
    fn current(&self, session: &str) -> Result<Option<Current>>;
    fn set_current(&self, session: &str, conversation: &str) -> Result<()>;
    /// Create a conversation with an opaque `header` and return its id.
    fn create(&self, header: &Value) -> Result<String>;
    /// The header and the last `tail` turns (all when `None`), oldest first.
    fn read(&self, conversation: &str, tail: Option<usize>) -> Result<(Value, Vec<Value>)>;
    /// Append atomically; when more than `max_turns` would remain, the oldest are dropped.
    fn append_turn(&self, conversation: &str, turn: &Value, max_turns: usize) -> Result<()>;
    /// Remove and return the most recent turn (`undo`).
    fn pop_turn(&self, conversation: &str) -> Result<Option<Value>>;
    fn list(&self) -> Result<Vec<ConversationInfo>>;
    fn delete(&self, conversation: &str) -> Result<()>;
    /// Forget every conversation and session pointer.
    fn clear(&self) -> Result<()>;
    /// Exclusive access across processes. Fails with `conversation.busy` after `timeout`.
    fn lock(&self, conversation: &str, timeout: Duration) -> Result<Box<dyn ConversationLock>>;
}

pub fn busy() -> Error {
    Error::new(
        "conversation.busy",
        "another request is using this conversation",
    )
    .with_hint("wait for it to finish, or start a new conversation")
}
