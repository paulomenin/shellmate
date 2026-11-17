use super::*;

#[tokio::test]
async fn remote_providers_require_a_redaction_layer() {
    let p = FakeProvider::replying("hi");
    let e = Engine::with_providers(remote(p.clone()), 1);
    assert_eq!(
        e.run(&env("ask", "hello")).await.unwrap_err().code,
        "privacy.no_redaction"
    );
    assert_eq!(
        e.preview(&env("ask", "hello")).await.unwrap_err().code,
        "privacy.no_redaction"
    );
    assert!(
        p.seen.lock().unwrap().is_empty(),
        "nothing may be sent without redaction"
    );

    let e =
        Engine::with_providers(remote(p.clone()), 1).with_layers(vec![Arc::new(Passthrough)], true);
    assert!(e.run(&env("ask", "hello")).await.is_ok());
}

#[tokio::test]
async fn preview_refuses_a_layer_that_does_not_pass_the_request_on() {
    let p = FakeProvider::replying("unused");
    let e = Engine::with_providers(remote(p.clone()), 1)
        .with_layers(vec![Arc::new(ShortCircuit)], true);
    let err = e.preview(&env("ask", "hello")).await.unwrap_err();
    assert_eq!(err.code, "preview.no_request");
    assert!(err.hint.is_some());
    assert!(p.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn preview_runs_the_chain_but_sends_nothing() {
    let p = FakeProvider::replying("unused");
    let e = engine(vec![("a", p.clone(), 3), ("b", p.clone(), 0)]);
    let previews = e.preview(&env("ask", "hello")).await.unwrap();
    assert_eq!(previews.len(), 2);
    assert_eq!((previews[0].level, previews[1].level), (1, 0));
    assert!(previews[0].request.messages[0].content.contains("/work/x"));
    assert!(!previews[1].request.messages[0].content.contains("/work/x"));
    assert!(p.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn generate_puts_the_command_in_the_buffer() {
    let p =
        FakeProvider::replying("Sorted by size.\n```shellmate-command\ndu -sh * | sort -h\n```");
    let out = engine(vec![("a", p, 1)])
        .run(&env("generate", "biggest files"))
        .await
        .unwrap();
    assert_eq!(out.buffer.as_deref(), Some("du -sh * | sort -h"));
    assert_eq!(out.text, "", "generate shows the command only");
    assert_eq!(out.provider, "a");
}

#[tokio::test]
async fn generate_without_a_command_shows_the_models_sentence() {
    let p = FakeProvider::replying("Which directory do you mean?");
    let out = engine(vec![("a", p, 1)])
        .run(&env("generate", "biggest files"))
        .await
        .unwrap();
    assert_eq!(out.buffer, None);
    assert_eq!(out.text, "Which directory do you mean?");
}

#[tokio::test]
async fn generate_keeps_only_the_first_command() {
    let p =
        FakeProvider::replying("```shellmate-command\nls\n```\n```shellmate-command\nls -la\n```");
    let out = engine(vec![("a", p, 1)])
        .run(&env("generate", "list"))
        .await
        .unwrap();
    assert_eq!(out.buffer.as_deref(), Some("ls"));
    assert!(out.trailer_text().is_empty(), "{}", out.trailer_text());
}

#[tokio::test]
async fn explain_never_touches_the_buffer() {
    let p = FakeProvider::replying("It lists files.\n```shellmate-command\nls -la\n```");
    let out = engine(vec![("a", p, 1)])
        .run(&env("explain", "ls"))
        .await
        .unwrap();
    assert_eq!(out.buffer, None);
    assert_eq!(out.text, "It lists files.");
}

#[tokio::test]
async fn falls_back_on_unavailable_but_not_on_auth() {
    let down = FakeProvider::with(vec![Err(ProviderError::new(
        ProviderErrorKind::Unavailable,
        "down",
    ))]);
    let up = FakeProvider::replying("hi there");
    let out = engine(vec![("down", down, 1), ("up", up, 1)])
        .run(&env("ask", "hello"))
        .await
        .unwrap();
    assert_eq!(
        (out.text.as_str(), out.provider.as_str()),
        ("hi there", "up")
    );

    let bad = FakeProvider::with(vec![Err(ProviderError::new(
        ProviderErrorKind::Auth,
        "401",
    ))]);
    let never = FakeProvider::replying("unused");
    let e = engine(vec![("bad", bad, 1), ("never", never.clone(), 1)])
        .run(&env("ask", "hello"))
        .await
        .unwrap_err();
    assert_eq!(e.code, "provider.auth");
    assert!(never.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn payload_is_rebuilt_per_provider_with_its_context_cap() {
    let first = FakeProvider::with(vec![Err(ProviderError::new(
        ProviderErrorKind::Unavailable,
        "down",
    ))]);
    let second = FakeProvider::replying("ok");
    let e = engine(vec![
        ("local", first.clone(), 3),
        ("remote", second.clone(), 0),
    ]);
    e.run(&env("ask", "hello")).await.unwrap();
    let sys_first = first.seen.lock().unwrap()[0].messages[0].content.clone();
    let sys_second = second.seen.lock().unwrap()[0].messages[0].content.clone();
    assert!(
        sys_first.contains("/work/x"),
        "level 1 sent to the first provider"
    );
    assert!(
        !sys_second.contains("/work/x"),
        "level 0 for the capped provider"
    );
}

#[tokio::test]
async fn empty_answers_and_unsafe_commands() {
    let p = FakeProvider::replying("   ");
    assert_eq!(
        engine(vec![("a", p, 1)])
            .run(&env("ask", "x"))
            .await
            .unwrap_err()
            .code,
        "answer.empty"
    );
    let p = FakeProvider::replying("```shellmate-command\nrm -rf /\u{1b}[2J\n```");
    let out = engine(vec![("a", p, 1)])
        .run(&env("generate", "x"))
        .await
        .unwrap();
    assert_eq!(
        out.buffer, None,
        "an unsafe command never reaches the buffer"
    );
    assert!(
        out.trailer_text().contains("dropped 1 suggestion"),
        "{}",
        out.trailer_text()
    );
}

#[test]
fn unknown_action_is_a_protocol_error() {
    assert_eq!("fly".parse::<Action>().unwrap_err().code, "glue.protocol");
}
