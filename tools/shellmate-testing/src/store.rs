//! A conversation store in memory, for tests of the engine, and the contract every `Store` must meet.

use serde_json::json;
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

/// What every [`Store`] must do, checked through the trait only. Run it on a new, empty store; it is run by
/// the in-memory store here and by `builtin:store-tmpfs`, so the fake the engine tests use cannot drift from
/// the real one. Process locking is not part of it: only a real adapter can show that.
pub fn assert_store_contract(store: &dyn Store) {
    let ns =
        |turns: &[Value]| -> Vec<i64> { turns.iter().map(|t| t["n"].as_i64().unwrap()).collect() };

    // an empty store
    assert_eq!(store.current("s").unwrap(), None);
    assert!(store.list().unwrap().is_empty());

    // create keeps the header, and ids are distinct
    let a = store.create(&json!({"session": "s"})).unwrap();
    let b = store.create(&json!({"session": "s"})).unwrap();
    assert_ne!(a, b);
    let (header, turns) = store.read(&a, None).unwrap();
    assert_eq!(header["session"], "s");
    assert!(turns.is_empty());

    // turns come back oldest first, and `tail` takes the newest
    for n in 0..5 {
        store.append_turn(&a, &json!({ "n": n }), 40).unwrap();
    }
    assert_eq!(ns(&store.read(&a, None).unwrap().1), [0, 1, 2, 3, 4]);
    assert_eq!(ns(&store.read(&a, Some(2)).unwrap().1), [3, 4]);
    assert_eq!(ns(&store.read(&a, Some(99)).unwrap().1), [0, 1, 2, 3, 4]);
    assert!(store.read(&a, Some(0)).unwrap().1.is_empty());

    // the oldest turns are dropped past `max_turns`, never the header, and the limit is exact
    for n in 5..8 {
        store.append_turn(&a, &json!({ "n": n }), 3).unwrap();
    }
    let (header, turns) = store.read(&a, None).unwrap();
    assert_eq!(header["session"], "s");
    assert_eq!(ns(&turns), [5, 6, 7]);
    store.append_turn(&b, &json!({"n": 1}), 0).unwrap();
    assert!(
        store.read(&b, None).unwrap().1.is_empty(),
        "0 keeps no turn"
    );

    // list counts turns
    let listed = store.list().unwrap();
    let turns_of = |id: &str| listed.iter().find(|c| c.id == id).map(|c| c.turns);
    assert_eq!((turns_of(&a), turns_of(&b)), (Some(3), Some(0)));

    // pop returns the newest turn, then nothing once only the header is left
    assert_eq!(store.pop_turn(&a).unwrap().unwrap()["n"], 7);
    assert_eq!(ns(&store.read(&a, None).unwrap().1), [5, 6]);
    store.pop_turn(&a).unwrap();
    store.pop_turn(&a).unwrap();
    assert_eq!(store.pop_turn(&a).unwrap(), None);
    assert_eq!(store.read(&a, None).unwrap().0["session"], "s");

    // the session pointer follows a conversation until it is deleted
    store.set_current("s", &a).unwrap();
    assert_eq!(store.current("s").unwrap().unwrap().conversation, a);
    store.delete(&a).unwrap();
    assert_eq!(store.current("s").unwrap(), None);
    assert!(store.read(&a, None).is_err());
    assert!(store.append_turn(&a, &json!({}), 40).is_err());
    assert!(store.pop_turn(&a).is_err());

    // clear forgets everything
    store.set_current("s", &b).unwrap();
    store.clear().unwrap();
    assert_eq!(store.current("s").unwrap(), None);
    assert!(store.list().unwrap().is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_store_meets_the_contract() {
        assert_store_contract(&MemoryStore::new());
    }
}
