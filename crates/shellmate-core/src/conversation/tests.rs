use super::*;

fn turn(n: usize, audience: Audience, tool: bool) -> Turn {
    let mut messages = vec![Stored::text(Role::User, format!("q{n}"))];
    if tool {
        messages.push(Stored {
            role: Role::Assistant,
            kind: Kind::ToolCall,
            content: "```shellmate-tool\nman ssh\n```".into(),
        });
        messages.push(Stored {
            role: Role::User,
            kind: Kind::ToolResult,
            content: "x".repeat(4096),
        });
    }
    messages.push(Stored::text(Role::Assistant, format!("a{n}")));
    Turn {
        at: 0,
        action: "ask".into(),
        input: format!("q{n}"),
        level: 1,
        audience,
        provider: "p".into(),
        suggestion: None,
        messages,
        outcome: Outcome::Completed,
    }
}

fn cfg(keep: usize, elide: usize) -> HistoryConfig {
    HistoryConfig {
        strategy: "window".into(),
        keep_recent_turns: keep,
        elide_tool_results_after: elide,
    }
}

fn texts(h: &History) -> Vec<&str> {
    h.messages.iter().map(|m| m.content.as_str()).collect()
}

#[test]
fn window_keeps_whole_recent_turns() {
    let turns: Vec<_> = (0..5).map(|n| turn(n, Audience::Any, false)).collect();
    let h = assemble(&turns, true, &cfg(2, 2));
    assert_eq!(texts(&h), ["q3", "a3", "q4", "a4"]);
}

#[test]
fn old_tool_results_become_stubs_but_stay_paired_with_their_call() {
    let turns = vec![
        turn(0, Audience::Any, true),
        turn(1, Audience::Any, true),
        turn(2, Audience::Any, true),
    ];
    let h = assemble(&turns, true, &cfg(6, 2));
    let results: Vec<_> = h
        .messages
        .iter()
        .filter(|m| m.content.starts_with('x') || m.content.starts_with("[tool result"))
        .map(|m| m.content.len())
        .collect();
    // oldest turn (age 2) elided, the two newest kept in full
    assert_eq!(
        results,
        ["[tool result elided: 4096 bytes]".len(), 4096, 4096]
    );
    assert_eq!(
        h.messages
            .iter()
            .filter(|m| m.content.contains("shellmate-tool"))
            .count(),
        3
    );
}

#[test]
fn local_only_turns_are_omitted_for_remote_providers_and_counted() {
    let turns = vec![
        turn(0, Audience::Local, false),
        turn(1, Audience::Any, false),
        turn(2, Audience::Local, false),
    ];
    let remote = assemble(&turns, false, &cfg(6, 2));
    assert_eq!(texts(&remote), ["q1", "a1"]);
    assert_eq!(remote.omitted_restricted, 2);
    let local = assemble(&turns, true, &cfg(6, 2));
    assert_eq!(local.messages.len(), 6);
    assert_eq!(local.omitted_restricted, 0);
}

#[test]
fn audience_follows_the_context_level() {
    assert_eq!(audience_for(0), Audience::Any);
    assert_eq!(audience_for(1), Audience::Any);
    assert_eq!(audience_for(2), Audience::Local);
    assert_eq!(audience_for(3), Audience::Local);
}

#[test]
fn keys_round_trip_through_hex() {
    let k = [0xab; 16];
    assert_eq!(unhex(&hex(&k)), Some(k));
    assert_eq!(unhex("zz"), None);
    assert_eq!(unhex(&"g".repeat(32)), None);
}

// ---- which conversation a request continues (the clock is a parameter) ----

use serde_json::{json, Value};
use std::sync::Mutex;

struct Held;
impl ConversationLock for Held {}

/// One stored conversation `c1`, last used at `last`; `create` hands out `c2`.
struct OneConversation {
    last: u64,
    created: Mutex<Vec<String>>,
}

impl OneConversation {
    fn store(last: u64) -> Arc<dyn Store> {
        Arc::new(Self {
            last,
            created: Mutex::new(Vec::new()),
        })
    }
}

impl Store for OneConversation {
    fn current(&self, _: &str) -> Result<Option<shellmate_api::Current>> {
        Ok(Some(shellmate_api::Current {
            conversation: "c1".into(),
            last_activity: self.last,
        }))
    }
    fn set_current(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn create(&self, _: &Value) -> Result<String> {
        self.created.lock().unwrap().push("c2".into());
        Ok("c2".into())
    }
    fn read(&self, _: &str, _: Option<usize>) -> Result<(Value, Vec<Value>)> {
        Ok((json!({"redaction_key": hex(&[7; 16])}), Vec::new()))
    }
    fn append_turn(&self, _: &str, _: &Value, _: usize) -> Result<()> {
        Ok(())
    }
    fn pop_turn(&self, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn list(&self) -> Result<Vec<shellmate_api::ConversationInfo>> {
        Ok(Vec::new())
    }
    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn clear(&self) -> Result<()> {
        Ok(())
    }
    fn lock(&self, _: &str, _: Duration) -> Result<Box<dyn ConversationLock>> {
        Ok(Box::new(Held))
    }
}

const IDLE: Duration = Duration::from_secs(600);

fn resolved(last: u64, now: u64, mode: ConversationMode) -> Option<String> {
    resolve(&OneConversation::store(last), Some("s"), mode, IDLE, now)
        .unwrap()
        .map(|a| a.id)
}

#[test]
fn an_auto_conversation_continues_until_it_has_been_idle_too_long() {
    assert_eq!(
        resolved(1000, 1000 + 600, ConversationMode::Auto).as_deref(),
        Some("c1")
    );
    assert_eq!(
        resolved(1000, 1000 + 601, ConversationMode::Auto).as_deref(),
        Some("c2")
    );
    // a clock that is behind the store never makes a conversation look idle
    assert_eq!(
        resolved(5000, 1000, ConversationMode::Auto).as_deref(),
        Some("c1")
    );
}

#[test]
fn continue_ignores_the_idle_time_and_fresh_and_no_session_are_stateless() {
    assert_eq!(
        resolved(1000, 1_000_000, ConversationMode::Continue).as_deref(),
        Some("c1")
    );
    assert_eq!(resolved(1000, 1000, ConversationMode::Fresh), None);
    let store = OneConversation::store(1000);
    assert!(resolve(&store, None, ConversationMode::Auto, IDLE, 1000)
        .unwrap()
        .is_none());
}

#[test]
fn peek_reports_nothing_for_an_idle_conversation_and_creates_nothing() {
    let store = OneConversation::store(1000);
    let live = peek(&store, "s", ConversationMode::Auto, IDLE, 1100).unwrap();
    assert_eq!(live.map(|(_, key)| key), Some([7; 16]));
    assert!(peek(&store, "s", ConversationMode::Auto, IDLE, 1000 + 601)
        .unwrap()
        .is_none());
    assert!(peek(&store, "s", ConversationMode::Fresh, IDLE, 1100)
        .unwrap()
        .is_none());
}
