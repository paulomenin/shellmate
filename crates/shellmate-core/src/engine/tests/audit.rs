//! What the engine reports to the audit sink, through the port and without touching a file. The log is
//! content-free: decisions, sizes, timings and codes, never prompts, answers or tool results.

use super::*;
use crate::audit::MemoryAudit;

const ASKED: &str = "list the big files in my private-project-xyz";
const ANSWER: &str = "Here you go.\n```shellmate-command\nls -S\n```";

fn names(events: &[serde_json::Value]) -> Vec<&str> {
    events
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect()
}

fn dump(events: &[serde_json::Value]) -> String {
    serde_json::to_string(events).unwrap()
}

#[tokio::test]
async fn a_request_reports_its_stages_in_order_and_no_content() {
    let sink = Arc::new(MemoryAudit::new());
    let e = engine(vec![("a", FakeProvider::replying(ANSWER), 1)]).with_audit(sink.clone());
    let mut env = env("ask", ASKED);
    env.session = Some("s1".into());
    e.run(&env).await.unwrap();
    let events = sink.events();
    assert_eq!(
        names(&events),
        ["request_started", "llm_call", "validation", "turn_finished"]
    );
    assert!(events.iter().all(|v| v["v"] == 1 && v["ts"].is_u64()));
    assert_eq!(events[0]["session"], "s1");
    assert_eq!(events[0]["action"], "ask");
    assert_eq!(events[1]["provider"], "a");
    assert_eq!(events[1]["status"], "ok");
    assert_eq!(events[3]["buffer_set"], true);
    let all = dump(&events);
    for secret in ["private-project-xyz", "ls -S", "Here you go"] {
        assert!(!all.contains(secret), "the log must not contain `{secret}`");
    }
}

#[tokio::test]
async fn a_failed_provider_is_recorded_with_its_code_and_the_fallback() {
    let sink = Arc::new(MemoryAudit::new());
    let down = FakeProvider::with(vec![Err(ProviderError::new(
        ProviderErrorKind::Unavailable,
        "down",
    ))]);
    let e = engine(vec![
        ("one", down, 1),
        ("two", FakeProvider::replying(ANSWER), 1),
    ])
    .with_audit(sink.clone());
    e.run(&env("ask", ASKED)).await.unwrap();
    let calls: Vec<_> = sink
        .events()
        .into_iter()
        .filter(|v| v["event"] == "llm_call")
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["provider"], "one");
    assert_eq!(calls[0]["status"], "provider.unavailable");
    assert_eq!(calls[0]["fallback"], true);
    assert_eq!(calls[1]["status"], "ok");
}

#[tokio::test]
async fn a_tool_call_records_the_decision_but_not_the_command_output() {
    let sink = Arc::new(MemoryAudit::new());
    let p = FakeProvider::with(vec![
        Ok(exec_call("echo audit-canary-output")),
        Ok("Done.".to_string()),
    ]);
    let e = engine(vec![("a", p, 1)])
        .with_approver(FakeApprover::new(Choice::Run))
        .with_audit(sink.clone());
    e.run(&env("ask", "say it")).await.unwrap();
    let events = sink.events();
    let tool = events.iter().find(|v| v["event"] == "tool_call").unwrap();
    assert_eq!(tool["tool"], "exec");
    assert_eq!(tool["decision"], "approved");
    assert!(!dump(&events).contains("audit-canary-output"));
}

#[tokio::test]
async fn without_a_sink_nothing_is_recorded_and_the_request_works() {
    let e = engine(vec![("a", FakeProvider::replying(ANSWER), 1)]);
    assert!(e.run(&env("ask", ASKED)).await.is_ok());
}

#[tokio::test]
async fn ids_and_timestamps_come_from_the_clock() {
    use crate::ports::{Clock, Host, ManualClock};
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let sink = Arc::new(MemoryAudit::with_clock(clock.clone()));
    let e = engine(vec![("a", FakeProvider::replying(ANSWER), 1)])
        .with_host(Host::system().with_clock(clock.clone()))
        .with_audit(sink.clone());
    e.run(&env("ask", ASKED)).await.unwrap();
    let events = sink.events();
    assert!(events.iter().all(|v| v["ts"] == 1_700_000_000u64));
    let id = events[0]["request_id"].as_str().unwrap();
    assert!(
        id.starts_with(&format!("{:x}-", clock.unix_nanos())),
        "{id}"
    );
}
