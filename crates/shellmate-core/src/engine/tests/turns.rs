//! What a request leaves behind and what it reports: the saved turn, the report, and the notes in the
//! trailer when something around the answer goes wrong.

use super::*;
use crate::ports::{Host, ManualClock};
use crate::present::Tone;
use shellmate_testing::MemoryStore;
use std::sync::atomic::Ordering;

const ANSWER: &str = "Here you go.\n```shellmate-command\nls -S\n```";

fn in_session(action: &str, buffer: &str) -> Envelope {
    let mut e = env(action, buffer);
    e.session = Some("s".into());
    e
}

/// An engine with an in-memory store, at second 100 (the store reports activity at 0, so a conversation
/// is well inside the idle timeout).
fn with_store(replies: usize) -> (Engine, Arc<MemoryStore>, Arc<FakeProvider>) {
    let store = Arc::new(MemoryStore::new());
    let provider = FakeProvider::with((0..replies).map(|_| Ok(ANSWER.to_string())).collect());
    let e = engine(vec![("a", provider.clone(), 1)])
        .with_host(Host::system().with_clock(Arc::new(ManualClock::at(100))))
        .with_store(store.clone());
    (e, store, provider)
}

#[tokio::test]
async fn a_finished_turn_is_saved_and_the_next_request_continues_it() {
    let (e, store, provider) = with_store(2);
    let first = e.run(&in_session("ask", "list big files")).await.unwrap();
    assert_eq!(first.report.conversation.as_deref(), Some("c1"));
    assert_eq!(first.report.turn, 1);

    let turns = store.turns("c1");
    assert_eq!(turns.len(), 1);
    let t = &turns[0];
    assert_eq!(
        (
            t["action"].as_str(),
            t["input"].as_str(),
            t["provider"].as_str()
        ),
        (Some("ask"), Some("list big files"), Some("a"))
    );
    assert_eq!(t["suggestion"], "ls -S");
    assert_eq!(t["at"], 100);
    let roles: Vec<_> = t["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(roles, ["user", "assistant"]);

    let second = e
        .run(&in_session("ask", "and the small ones"))
        .await
        .unwrap();
    assert_eq!(second.report.conversation.as_deref(), Some("c1"));
    assert_eq!(second.report.turn, 2);
    assert_eq!(store.conversations(), 1);
    let seen = provider.seen.lock().unwrap();
    assert!(
        seen[1].messages.len() > seen[0].messages.len(),
        "history reaches the model"
    );
    assert!(seen[1]
        .messages
        .iter()
        .any(|m| m.content.contains("list big files")));
}

#[tokio::test]
async fn a_request_without_a_session_leaves_nothing_behind() {
    let (e, store, _) = with_store(1);
    let out = e.run(&env("ask", "list big files")).await.unwrap();
    assert_eq!(out.report.conversation, None);
    assert_eq!(out.report.turn, 1);
    assert_eq!(store.conversations(), 0);
}

#[tokio::test]
async fn a_store_that_cannot_save_is_a_note_and_the_answer_still_arrives() {
    let (e, store, _) = with_store(1);
    store.fail_appends.store(true, Ordering::SeqCst);
    let out = e.run(&in_session("ask", "list big files")).await.unwrap();
    assert_eq!(out.buffer.as_deref(), Some("ls -S"));
    assert!(
        out.trailer_text()
            .contains("note: could not save this turn: disk full"),
        "{}",
        out.trailer_text()
    );
    assert!(store.turns("c1").is_empty());
}

#[tokio::test]
async fn a_store_that_could_not_be_opened_is_a_note_on_every_answer() {
    let mut e = engine(vec![("a", FakeProvider::replying(ANSWER), 1)]);
    e.store_note = Some("conversations are off: no such store".into());
    let out = e.run(&in_session("ask", "list big files")).await.unwrap();
    assert!(
        out.trailer_text()
            .contains("note: conversations are off: no such store"),
        "{}",
        out.trailer_text()
    );
}

#[tokio::test]
async fn notes_after_the_answer_are_separate_remarks_joined_by_a_blank_line() {
    let (mut e, store, _) = with_store(1);
    store.fail_appends.store(true, Ordering::SeqCst);
    e.store_note = Some("conversations are off: x".into());
    let out = e.run(&in_session("ask", "list big files")).await.unwrap();
    let notes: Vec<_> = out
        .notes
        .iter()
        .filter(|n| n.label == Some("note"))
        .collect();
    assert_eq!(notes.len(), 2, "{:?}", out.notes);
    assert!(notes.iter().all(|n| n.tone == Tone::Warning));
    assert!(out
        .trailer_text()
        .contains("\n\nnote: conversations are off: x"));
}

fn down(message: &str) -> Arc<FakeProvider> {
    FakeProvider::with(vec![Err(ProviderError::new(
        ProviderErrorKind::Unavailable,
        message,
    ))])
}

#[tokio::test]
async fn when_every_provider_is_down_the_error_counts_them_and_keeps_the_last() {
    let e = engine(vec![
        ("one", down("down one"), 1),
        ("two", down("down two"), 1),
    ]);
    let err = e.run(&env("ask", "hello")).await.unwrap_err();
    assert_eq!(err.code, "provider.unavailable");
    assert_eq!(err.message, "all 2 providers failed; last: down two");
}

#[tokio::test]
async fn a_single_provider_that_is_down_keeps_its_own_message() {
    let e = engine(vec![("one", down("down one"), 1)]);
    let err = e.run(&env("ask", "hello")).await.unwrap_err();
    assert_eq!(err.message, "down one");
}

#[tokio::test]
async fn the_report_describes_the_request_without_content() {
    let (e, _, _) = with_store(1);
    let out = e.run(&in_session("ask", "list big files")).await.unwrap();
    let r = &out.report;
    assert_eq!(r.provider_chain, ["a"]);
    assert_eq!((r.level, r.omitted_turns, r.redaction), (1, 0, false));
    assert_eq!(r.trust, Some(Trust::Local));
    assert!(r.tools.is_empty());
    // the stages are nested: the model call and the context both fit inside the whole run
    assert!(r.stages.llm_us <= r.stages.total_us);
    assert!(r.stages.context_us <= r.stages.total_us);
    assert!(r.stages.validate_us <= r.stages.total_us);
}
