//! Native tool calling: what the provider is sent, how a call it makes is run, and how a model is found to
//! take tools. A native call is only another way to write the line a call in text has, so most of what is
//! checked here is that the same host, parsers and approval apply.

use super::*;
use crate::engine::tool_calls::ToolDecision;
use crate::ports::ManualClock;
use shellmate_api::{
    ChatResponse, ProviderError, ProviderErrorKind, Role, ToolChoice, ToolSupport,
};
use shellmate_testing::native_call;

const NOW: u64 = 1_800_000_000;

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-native-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::at(NOW))
}

fn engine_in(
    providers: Vec<(&str, Arc<FakeProvider>)>,
    mode: crate::config::ToolCalling,
    cache: &std::path::Path,
    now: &Arc<ManualClock>,
) -> Engine {
    let mut e = engine(providers.into_iter().map(|(n, p)| (n, p, 1)).collect())
        .with_host(host_with_cache(cache, now));
    for spec in &mut e.providers {
        spec.tool_calling = mode;
    }
    e
}

fn native(provider: Arc<FakeProvider>) -> Engine {
    let d = scratch("unused");
    engine_in(
        vec![("a", provider)],
        crate::config::ToolCalling::Native,
        &d,
        &clock(),
    )
}

fn text_reply(text: &str) -> Result<ChatResponse, ProviderError> {
    Ok(ChatResponse {
        text: text.into(),
        ..ChatResponse::default()
    })
}

fn call(id: &str, name: &str, arguments: &str) -> Result<ChatResponse, ProviderError> {
    Ok(native_call(id, name, arguments))
}

fn seen(p: &FakeProvider) -> Vec<shellmate_api::ChatRequest> {
    p.seen.lock().unwrap().clone()
}

// ---- what the provider is sent ------------------------------------------------------------------------------

#[tokio::test]
async fn a_native_model_is_sent_the_tools_and_the_prompt_does_not_describe_them() {
    for action in ["ask", "generate", "explain"] {
        let p = FakeProvider::with_responses(vec![text_reply("ok")]);
        let _ = native(p.clone()).run(&env(action, "ls")).await.unwrap();
        let sent = &seen(&p)[0];
        let names: Vec<&str> = sent.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["man", "help", "exec", "read"], "{action}");
        assert_eq!(sent.tool_choice, ToolChoice::Auto);
        for tool in &sent.tools {
            assert_eq!(tool.parameters["type"], "object", "{}", tool.name);
            assert!(tool.description.contains("reference data"), "{}", tool.name);
        }
        let system = &sent.messages[0].content;
        assert!(
            !system.contains("shellmate-tool")
                && !system.contains("You can run one simple command"),
            "{action}: the tools are not described twice: {system}"
        );
    }
}

#[tokio::test]
async fn a_text_model_is_told_about_the_tools_in_the_prompt_and_sent_none() {
    let p = FakeProvider::replying("ok");
    let e = engine(vec![("a", p.clone(), 1)]);
    let _ = e.run(&env("ask", "ls")).await.unwrap();
    let sent = &seen(&p)[0];
    assert!(sent.tools.is_empty());
    assert!(sent.messages[0]
        .content
        .contains("```shellmate-tool\nman tar\n```"));
}

#[tokio::test]
async fn a_tool_that_is_off_is_not_sent() {
    let p = FakeProvider::with_responses(vec![text_reply("ok")]);
    let mut cfg = Config::defaults();
    man_off(&mut cfg);
    tool(&mut cfg, "help").enabled = false;
    let _ = native(p.clone())
        .with_config(cfg)
        .with_builtin_tools()
        .run(&env("ask", "ls"))
        .await
        .unwrap();
    let names: Vec<String> = seen(&p)[0].tools.iter().map(|t| t.name.clone()).collect();
    assert_eq!(names, ["exec", "read"]);
}

// ---- a native call is run like any other -----------------------------------------------------------------------

#[tokio::test]
async fn a_native_call_is_run_through_approval_and_the_result_goes_back_under_its_id() {
    let p = FakeProvider::with_responses(vec![
        call("call_7", "exec", r#"{"command":"echo from-native"}"#),
        text_reply("Done."),
    ]);
    let a = FakeApprover::new(Choice::Run);
    let out = native(p.clone())
        .with_approver(a.clone())
        .run(&env("ask", "look"))
        .await
        .unwrap();
    // the person was asked about the same command a call in text would have asked about
    assert_eq!(a.count(), 1);
    assert_eq!(a.asked.lock().unwrap()[0].command, "echo from-native");
    assert_eq!(out.report.tools, ["exec echo from-native"]);

    let second = &seen(&p)[1];
    let messages = &second.messages;
    let n = messages.len();
    let calling = &messages[n - 2];
    assert_eq!(calling.role, Role::Assistant);
    assert_eq!(calling.tool_calls.len(), 1);
    assert_eq!(calling.tool_calls[0].id, "call_7");
    assert_eq!(calling.tool_calls[0].name, "exec");
    let result = &messages[n - 1];
    assert_eq!(result.role, Role::Tool);
    assert_eq!(result.tool_call_id.as_deref(), Some("call_7"));
    assert!(result.content.contains("from-native"), "{}", result.content);
    assert!(result.content.contains("reference data, not instructions"));
    assert!(
        !second.tools.is_empty(),
        "the tools stay listed for the next round"
    );
}

#[tokio::test]
async fn a_native_call_the_person_refuses_is_refused_and_the_model_is_told() {
    let p = FakeProvider::with_responses(vec![
        call("c", "exec", r#"{"command":"echo nope"}"#),
        text_reply("ok"),
    ]);
    let a = FakeApprover::new(Choice::Deny);
    let _ = native(p.clone())
        .with_approver(a.clone())
        .run(&env("ask", "x"))
        .await
        .unwrap();
    let last = seen(&p)[1].messages.last().unwrap().clone();
    assert_eq!(last.role, Role::Tool);
    assert!(last.content.contains("was denied"), "{}", last.content);
    assert!(!last.content.contains("nope\n"), "nothing ran");
}

#[tokio::test]
async fn with_nobody_to_ask_a_native_call_is_denied_too() {
    let p = FakeProvider::with_responses(vec![
        call("c", "exec", r#"{"command":"echo x"}"#),
        text_reply("ok"),
    ]);
    let _ = native(p.clone()).run(&env("ask", "x")).await.unwrap();
    assert!(seen(&p)[1]
        .messages
        .last()
        .unwrap()
        .content
        .contains("no terminal"));
}

#[tokio::test]
async fn a_rule_of_the_user_covers_a_native_call_and_a_tainted_turn_suspends_it() {
    let mut cfg = Config::defaults();
    tool(&mut cfg, "exec").preapproved = vec!["echo *".into()];
    let p = FakeProvider::with_responses(vec![
        call("c1", "exec", r#"{"command":"echo one"}"#),
        // the output of the first call is in the conversation now: the rule no longer applies
        call("c2", "exec", r#"{"command":"echo two"}"#),
        text_reply("done"),
    ]);
    let a = FakeApprover::new(Choice::Deny);
    let out = native(p.clone())
        .with_config(cfg)
        .with_builtin_tools()
        .with_approver(a.clone())
        .run(&env("ask", "x"))
        .await
        .unwrap();
    assert_eq!(a.count(), 1, "only the second call asked");
    assert_eq!(a.asked.lock().unwrap()[0].command, "echo two");
    assert_eq!(out.report.tools.len(), 2);
}

#[tokio::test]
async fn what_the_text_parsers_refuse_is_refused_in_a_native_call_without_asking() {
    for (name, arguments) in [
        ("exec", r#"{"command":"ls | wc -l"}"#),
        ("exec", r#"{"command":"echo $(id)"}"#),
        ("exec", r#"{"command":"echo a\nrm -rf /"}"#),
        ("man", r#"{"page":"-P cat"}"#),
        ("man", r#"{"page":"/etc/passwd"}"#),
        ("help", r#"{"program":"helpme","subcommand":["--version"]}"#),
        ("help", r#"{"program":"./helpme"}"#),
        ("read", r#"{"path":"/home/x/.ssh/id_rsa"}"#),
    ] {
        let p = FakeProvider::with_responses(vec![call("c", name, arguments), text_reply("ok")]);
        let a = FakeApprover::new(Choice::Run);
        let _ = native(p.clone())
            .with_approver(a.clone())
            .run(&env("ask", "x"))
            .await
            .unwrap();
        assert_eq!(a.count(), 0, "{name} {arguments} asked");
        let last = seen(&p)[1].messages.last().unwrap().content.clone();
        assert!(
            last.contains("denied") || last.contains("Tool error"),
            "{name} {arguments}: {last}"
        );
    }
}

#[tokio::test]
async fn arguments_that_are_not_a_call_are_a_tool_error_the_model_can_read() {
    for (name, arguments, why) in [
        ("exec", "not json", "not valid JSON"),
        ("exec", r#"["echo"]"#, "JSON object"),
        ("exec", r#"{"cmd":"echo"}"#, "unknown field"),
        ("exec", r#"{"command":["echo"]}"#, "invalid type"),
        ("exec", "{}", "missing field"),
        ("nope", "{}", "no tool called `nope`"),
    ] {
        let p = FakeProvider::with_responses(vec![call("c", name, arguments), text_reply("ok")]);
        let a = FakeApprover::new(Choice::Run);
        let out = native(p.clone())
            .with_approver(a.clone())
            .run(&env("ask", "x"))
            .await
            .unwrap();
        assert_eq!(a.count(), 0, "{arguments}");
        let last = seen(&p)[1].messages.last().unwrap().clone();
        assert_eq!(last.role, Role::Tool);
        assert!(
            last.content.starts_with("Tool error:"),
            "{arguments}: {}",
            last.content
        );
        assert!(last.content.contains(why), "{arguments}: {}", last.content);
        assert_eq!(out.report.tools, ["denied: invalid"]);
    }
    // too long to be a call anyone needs
    let long = format!(r#"{{"command":"echo {}"}}"#, "a".repeat(9000));
    let p = FakeProvider::with_responses(vec![call("c", "exec", &long), text_reply("ok")]);
    let _ = native(p.clone()).run(&env("ask", "x")).await.unwrap();
    assert!(seen(&p)[1]
        .messages
        .last()
        .unwrap()
        .content
        .contains("too long"));
}

#[tokio::test]
async fn when_a_model_makes_several_calls_only_the_first_runs_and_each_is_answered() {
    let two = ChatResponse {
        tool_calls: vec![
            shellmate_api::ToolRequest {
                id: "first".into(),
                name: "exec".into(),
                arguments: r#"{"command":"echo one"}"#.into(),
            },
            shellmate_api::ToolRequest {
                id: "second".into(),
                name: "exec".into(),
                arguments: r#"{"command":"echo two"}"#.into(),
            },
        ],
        ..ChatResponse::default()
    };
    let p = FakeProvider::with_responses(vec![Ok(two), text_reply("ok")]);
    let a = FakeApprover::new(Choice::Run);
    let out = native(p.clone())
        .with_approver(a.clone())
        .run(&env("ask", "x"))
        .await
        .unwrap();
    assert_eq!(a.count(), 1);
    assert_eq!(out.report.tools, ["exec echo one"]);
    let m = &seen(&p)[1].messages;
    let n = m.len();
    assert_eq!(
        m[n - 3].tool_calls.len(),
        2,
        "the assistant message keeps both calls"
    );
    assert_eq!(m[n - 2].tool_call_id.as_deref(), Some("first"));
    assert!(m[n - 2].content.contains("one"));
    assert_eq!(m[n - 1].tool_call_id.as_deref(), Some("second"));
    assert!(
        m[n - 1].content.starts_with("Not run:"),
        "{}",
        m[n - 1].content
    );
}

#[tokio::test]
async fn a_call_written_in_text_still_works_in_native_mode() {
    // a model that follows an earlier turn's text form instead of the tools it was sent
    let p = FakeProvider::with(vec![
        Ok("```shellmate-tool\nexec echo as-text\n```".to_string()),
        Ok("done".to_string()),
    ]);
    let a = FakeApprover::new(Choice::Run);
    let out = native(p.clone())
        .with_approver(a.clone())
        .run(&env("ask", "x"))
        .await
        .unwrap();
    assert_eq!(out.report.tools, ["exec echo as-text"]);
    let m = &seen(&p)[1].messages;
    assert!(m[m.len() - 1].role == Role::User && m[m.len() - 1].content.contains("as-text"));
}

#[tokio::test]
async fn the_last_round_must_answer_in_words() {
    let mut cfg = Config::defaults();
    cfg.conversation.max_tool_rounds = 2;
    let p = FakeProvider::with_responses(vec![
        call("a", "man", r#"{"page":"sm-no-such-page-1"}"#),
        call("b", "man", r#"{"page":"sm-no-such-page-2"}"#),
        text_reply("An answer."),
    ]);
    let out = native(p.clone())
        .with_config(cfg)
        .run(&env("ask", "x"))
        .await
        .unwrap();
    assert_eq!(out.text, "An answer.");
    let choices: Vec<ToolChoice> = seen(&p).iter().map(|r| r.tool_choice).collect();
    assert_eq!(
        choices,
        [ToolChoice::Auto, ToolChoice::Auto, ToolChoice::None]
    );
    assert!(
        !seen(&p)[2].tools.is_empty(),
        "the tools stay listed: some providers need them"
    );
}

#[tokio::test]
async fn once_a_tool_has_run_a_failure_does_not_move_on_to_another_provider() {
    let down = ProviderError::new(ProviderErrorKind::Unavailable, "down");
    // a turn that is only a call has no text, so only the call shows that something happened
    let first = FakeProvider::with_responses(vec![
        call("c", "man", r#"{"page":"sm-no-such-page"}"#),
        Err(down),
    ]);
    let second = FakeProvider::with_responses(vec![text_reply("never")]);
    let d = scratch("nofallback");
    let e = engine_in(
        vec![("a", first), ("b", second.clone())],
        crate::config::ToolCalling::Native,
        &d,
        &clock(),
    );
    assert!(e.run(&env("ask", "x")).await.is_err());
    assert!(seen(&second).is_empty(), "the second provider was asked");
}

#[tokio::test]
async fn the_history_keeps_the_text_form_so_any_model_can_replay_it() {
    // the in-memory store reports activity at second 0, so keep the clock near it
    let soon = Arc::new(ManualClock::at(100));
    let store = Arc::new(shellmate_testing::MemoryStore::new());
    let p = FakeProvider::with_responses(vec![
        call("c", "exec", r#"{"command":"echo kept"}"#),
        text_reply("Done."),
    ]);
    let e = engine_in(
        vec![("a", p)],
        crate::config::ToolCalling::Native,
        &scratch("history"),
        &soon,
    )
    .with_store(store.clone())
    .with_approver(FakeApprover::new(Choice::Run));
    let mut request = env("ask", "look");
    request.session = Some("s".into());
    e.run(&request).await.unwrap();
    let turns = store.turns("c1");
    let messages = turns[0]["messages"].as_array().unwrap();
    let kept = |kind: &str| -> String {
        messages
            .iter()
            .find(|m| m["kind"] == kind)
            .and_then(|m| m["content"].as_str())
            .unwrap()
            .to_string()
    };
    assert_eq!(kept("tool_call"), "```shellmate-tool\nexec echo kept\n```");
    assert!(kept("tool_result").contains("kept"));
    // a later request to a text model replays it as text
    let p2 = FakeProvider::replying("ok");
    let e2 = engine(vec![("a", p2.clone(), 1)])
        .with_host(host_with_cache(&scratch("replay"), &soon))
        .with_store(store);
    let mut again = env("ask", "and now");
    again.session = Some("s".into());
    e2.run(&again).await.unwrap();
    let all: Vec<String> = seen(&p2)[0]
        .messages
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert!(
        all.iter()
            .any(|c| c.contains("```shellmate-tool\nexec echo kept\n```")),
        "{all:#?}"
    );
}

// ---- finding out whether a model takes tools --------------------------------------------------------------

fn auto(p: &Arc<FakeProvider>, cache: &std::path::Path, now: &Arc<ManualClock>) -> Engine {
    engine_in(
        vec![("a", p.clone())],
        crate::config::ToolCalling::Auto,
        cache,
        now,
    )
}

#[tokio::test]
async fn auto_goes_native_when_the_server_says_the_model_takes_tools() {
    let d = scratch("yes");
    let p = FakeProvider::with_responses(vec![text_reply("ok")]);
    p.says_tools(ToolSupport::Yes);
    let _ = auto(&p, &d, &clock()).run(&env("ask", "x")).await.unwrap();
    assert!(!seen(&p)[0].tools.is_empty());
    assert_eq!(p.times_asked_about_tools(), 1);
}

#[tokio::test]
async fn auto_stays_in_text_when_the_server_says_no_or_does_not_say() {
    for says in [ToolSupport::No, ToolSupport::Unknown] {
        let d = scratch("no");
        let p = FakeProvider::with_responses(vec![text_reply("ok")]);
        p.says_tools(says);
        let _ = auto(&p, &d, &clock()).run(&env("ask", "x")).await.unwrap();
        let sent = &seen(&p)[0];
        assert!(sent.tools.is_empty(), "{says:?}");
        assert!(
            sent.messages[0].content.contains("shellmate-tool"),
            "{says:?}: told in the prompt"
        );
    }
}

#[tokio::test]
async fn the_answer_is_remembered_so_the_server_is_not_asked_on_every_request() {
    for says in [ToolSupport::Yes, ToolSupport::No, ToolSupport::Unknown] {
        let d = scratch("remember");
        let now = clock();
        let mut asked = 0;
        for _ in 0..3 {
            // every request is a new process: a new engine, with the same cache directory
            let p = FakeProvider::with_responses(vec![text_reply("ok")]);
            p.says_tools(says);
            let _ = auto(&p, &d, &now).run(&env("ask", "x")).await.unwrap();
            asked += p.times_asked_about_tools();
        }
        assert_eq!(asked, 1, "{says:?}");
        let file = d.join("shellmate/tool-support.json");
        assert!(file.exists());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[tokio::test]
async fn a_remembered_answer_expires_and_not_knowing_expires_sooner() {
    for (says, lasts) in [
        (ToolSupport::Yes, 7 * 24 * 3600),
        (ToolSupport::Unknown, 3600),
    ] {
        let d = scratch("expire");
        let now = clock();
        let ask = |now: &Arc<ManualClock>| {
            let p = FakeProvider::with_responses(vec![text_reply("ok")]);
            p.says_tools(says);
            (auto(&p, &d, now), p)
        };
        let (e, p) = ask(&now);
        e.run(&env("ask", "x")).await.unwrap();
        assert_eq!(p.times_asked_about_tools(), 1);
        now.advance(lasts - 1);
        let (e, p) = ask(&now);
        e.run(&env("ask", "x")).await.unwrap();
        assert_eq!(p.times_asked_about_tools(), 0, "{says:?} still believed");
        now.advance(2);
        let (e, p) = ask(&now);
        e.run(&env("ask", "x")).await.unwrap();
        assert_eq!(p.times_asked_about_tools(), 1, "{says:?} asked again");
    }
}

#[tokio::test]
async fn an_unreadable_cache_is_ignored() {
    let d = scratch("garbage");
    std::fs::create_dir_all(d.join("shellmate")).unwrap();
    std::fs::write(d.join("shellmate/tool-support.json"), "{ not json").unwrap();
    let p = FakeProvider::with_responses(vec![text_reply("ok")]);
    p.says_tools(ToolSupport::Yes);
    let _ = auto(&p, &d, &clock()).run(&env("ask", "x")).await.unwrap();
    assert!(!seen(&p)[0].tools.is_empty());
    // and it was replaced by a readable one
    let text = std::fs::read_to_string(d.join("shellmate/tool-support.json")).unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(&text).is_ok(),
        "{text}"
    );
}

#[tokio::test]
async fn the_config_wins_over_what_the_server_says() {
    let d = scratch("pinned");
    for (mode, says, native_expected) in [
        (crate::config::ToolCalling::Text, ToolSupport::Yes, false),
        (crate::config::ToolCalling::Native, ToolSupport::No, true),
    ] {
        let p = FakeProvider::with_responses(vec![text_reply("ok")]);
        p.says_tools(says);
        let _ = engine_in(vec![("a", p.clone())], mode, &d, &clock())
            .run(&env("ask", "x"))
            .await
            .unwrap();
        assert_eq!(!seen(&p)[0].tools.is_empty(), native_expected, "{mode:?}");
        assert_eq!(p.times_asked_about_tools(), 0, "{mode:?}: nothing to ask");
    }
}

#[tokio::test]
async fn a_dry_run_asks_nobody() {
    let d = scratch("dry");
    let p = FakeProvider::with_responses(vec![]);
    p.says_tools(ToolSupport::Yes);
    let e = auto(&p, &d, &clock());
    let previews = e.preview(&env("ask", "x")).await.unwrap();
    assert_eq!(p.times_asked_about_tools(), 0);
    assert!(
        previews[0].request.tools.is_empty(),
        "nothing is remembered, so text"
    );
    // once the answer is remembered, the preview shows what would be sent
    let p2 = FakeProvider::with_responses(vec![text_reply("ok")]);
    p2.says_tools(ToolSupport::Yes);
    let _ = auto(&p2, &d, &clock()).run(&env("ask", "x")).await.unwrap();
    let previews = auto(&p2, &d, &clock())
        .preview(&env("ask", "x"))
        .await
        .unwrap();
    assert!(!previews[0].request.tools.is_empty());
}

#[tokio::test]
async fn a_server_that_refuses_the_tools_it_advertised_is_asked_again_in_text_and_remembered() {
    let d = scratch("refused");
    let now = clock();
    let bad = ProviderError::new(
        ProviderErrorKind::BadRequest,
        "HTTP 400: tools not supported",
    );
    let p = FakeProvider::with_responses(vec![Err(bad), text_reply("Answered in text.")]);
    p.says_tools(ToolSupport::Yes);
    let out = auto(&p, &d, &now).run(&env("ask", "x")).await.unwrap();
    assert_eq!(out.text, "Answered in text.");
    let sent = seen(&p);
    assert_eq!(sent.len(), 2);
    assert!(!sent[0].tools.is_empty() && sent[1].tools.is_empty());
    assert!(
        sent[1].messages[0].content.contains("shellmate-tool"),
        "the tools are in the prompt now"
    );
    // the next request goes out in text at once, without asking the server again
    let p2 = FakeProvider::with_responses(vec![text_reply("ok")]);
    p2.says_tools(ToolSupport::Yes);
    let _ = auto(&p2, &d, &now).run(&env("ask", "x")).await.unwrap();
    assert!(seen(&p2)[0].tools.is_empty());
    assert_eq!(p2.times_asked_about_tools(), 0);
}

#[tokio::test]
async fn native_set_in_the_config_is_not_taken_back_when_the_server_refuses() {
    let bad = ProviderError::new(
        ProviderErrorKind::BadRequest,
        "HTTP 400: tools not supported",
    );
    let p = FakeProvider::with_responses(vec![Err(bad)]);
    let err = native(p.clone()).run(&env("ask", "x")).await.unwrap_err();
    assert_eq!(err.code, "provider.bad_request");
    assert_eq!(
        seen(&p).len(),
        1,
        "no second try: the user asked for native"
    );
}

#[tokio::test]
async fn a_refusal_after_a_tool_ran_is_not_retried() {
    // retrying would run the tool, and ask the person, a second time
    let d = scratch("late");
    let bad = ProviderError::new(ProviderErrorKind::BadRequest, "HTTP 400");
    let p = FakeProvider::with_responses(vec![
        call("c", "man", r#"{"page":"sm-no-such-page"}"#),
        Err(bad),
    ]);
    p.says_tools(ToolSupport::Yes);
    let err = auto(&p, &d, &clock())
        .run(&env("ask", "x"))
        .await
        .unwrap_err();
    assert_eq!(err.code, "provider.bad_request");
    assert_eq!(seen(&p).len(), 2);
}

#[tokio::test]
async fn the_decision_of_a_native_call_is_recorded_like_any_other() {
    use crate::audit::MemoryAudit;
    let sink = Arc::new(MemoryAudit::new());
    let p = FakeProvider::with_responses(vec![
        call("c", "exec", r#"{"command":"echo audited-secret-word"}"#),
        text_reply("ok"),
    ]);
    let _ = native(p)
        .with_audit(sink.clone())
        .with_approver(FakeApprover::new(Choice::Run))
        .run(&env("ask", "x"))
        .await
        .unwrap();
    let events = sink.events();
    let calls: Vec<_> = events
        .iter()
        .filter(|e| e["event"] == "tool_call")
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["tool"], "exec");
    assert_eq!(calls[0]["decision"], ToolDecision::Approved.as_str());
    let dump = serde_json::to_string(&events).unwrap();
    assert!(
        !dump.contains("audited-secret-word") && !dump.contains("Result of"),
        "{dump}"
    );
}
