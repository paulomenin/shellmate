//! A conversation store in memory, for tests of the engine.

use serde_json::Value;
use shellmate_api::{ConversationInfo, ConversationLock, Current, Error, Result, Store};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

struct Held;
impl ConversationLock for Held {}

#[derive(Default)]
struct Inner {
    next: u32,
    conversations: BTreeMap<String, (Value, Vec<Value>)>,
    current: BTreeMap<String, String>,
}

/// Keeps conversations in memory. Ids are `c1`, `c2`, ... and every conversation reports activity at second
/// 0, so a test chooses whether it counts as idle by the time it gives the engine.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
    /// While set, `append_turn` fails as a full disk would.
    pub fail_appends: AtomicBool,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The turns of a conversation, oldest first.
    pub fn turns(&self, conversation: &str) -> Vec<Value> {
        let inner = self.inner.lock().expect("store lock");
        inner
            .conversations
            .get(conversation)
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    }

    /// How many conversations exist.
    pub fn conversations(&self) -> usize {
        self.inner.lock().expect("store lock").conversations.len()
    }
}

fn missing(id: &str) -> Error {
    Error::new("conversation.store", format!("no conversation `{id}`"))
}

impl Store for MemoryStore {
    fn current(&self, session: &str) -> Result<Option<Current>> {
        let inner = self.inner.lock().expect("store lock");
        Ok(inner
            .current
            .get(session)
            .filter(|c| inner.conversations.contains_key(*c))
            .map(|c| Current {
                conversation: c.clone(),
                last_activity: 0,
            }))
    }

    fn set_current(&self, session: &str, conversation: &str) -> Result<()> {
        self.inner
            .lock()
            .expect("store lock")
            .current
            .insert(session.into(), conversation.into());
        Ok(())
    }

    fn create(&self, header: &Value) -> Result<String> {
        let mut inner = self.inner.lock().expect("store lock");
        inner.next += 1;
        let id = format!("c{}", inner.next);
        inner
            .conversations
            .insert(id.clone(), (header.clone(), Vec::new()));
        Ok(id)
    }

    fn read(&self, conversation: &str, tail: Option<usize>) -> Result<(Value, Vec<Value>)> {
        let inner = self.inner.lock().expect("store lock");
        let (header, turns) = inner
            .conversations
            .get(conversation)
            .ok_or_else(|| missing(conversation))?;
        let skip = tail.map_or(0, |n| turns.len().saturating_sub(n));
        Ok((header.clone(), turns[skip..].to_vec()))
    }

    fn append_turn(&self, conversation: &str, turn: &Value, max_turns: usize) -> Result<()> {
        if self.fail_appends.load(Ordering::SeqCst) {
            return Err(Error::new("conversation.store", "disk full"));
        }
        let mut inner = self.inner.lock().expect("store lock");
        let (_, turns) = inner
            .conversations
            .get_mut(conversation)
            .ok_or_else(|| missing(conversation))?;
        turns.push(turn.clone());
        while turns.len() > max_turns {
            turns.remove(0);
        }
        Ok(())
    }

    fn pop_turn(&self, conversation: &str) -> Result<Option<Value>> {
        let mut inner = self.inner.lock().expect("store lock");
        let (_, turns) = inner
            .conversations
            .get_mut(conversation)
            .ok_or_else(|| missing(conversation))?;
        Ok(turns.pop())
    }

    fn list(&self) -> Result<Vec<ConversationInfo>> {
        let inner = self.inner.lock().expect("store lock");
        Ok(inner
            .conversations
            .iter()
            .map(|(id, (_, turns))| ConversationInfo {
                id: id.clone(),
                turns: turns.len(),
                last_activity: 0,
            })
            .collect())
    }

    fn delete(&self, conversation: &str) -> Result<()> {
        self.inner
            .lock()
            .expect("store lock")
            .conversations
            .remove(conversation);
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let mut inner = self.inner.lock().expect("store lock");
        inner.conversations.clear();
        inner.current.clear();
        Ok(())
    }

    fn lock(&self, _: &str, _: Duration) -> Result<Box<dyn ConversationLock>> {
        Ok(Box::new(Held))
    }
}
