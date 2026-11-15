use super::client::*;
use shellmate_api::Message;
use shellmate_api::{ChatRequest, Provider, ProviderErrorKind, StreamEvent, Usage};
use shellmate_testing::{MockHttpServer, MockSseServer, SseChunk};

fn req() -> ChatRequest {
    ChatRequest {
        request_id: "r1".into(),
        messages: vec![Message::user("hi")],
        max_tokens: None,
        temperature: None,
        redaction_key: None,
    }
}

/// The key a provider ends up with, given the environment variables `vars`.
fn key_of(settings: serde_json::Value, vars: &[(&str, &str)]) -> Option<String> {
    let lookup = |name: &str| {
        vars.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_string())
    };
    OpenAiCompatible::from_settings_with(&settings, &lookup)
        .unwrap()
        .api_key
        .map(|k| k.expose().to_string())
}

#[test]
fn a_hosted_preset_supplies_its_conventional_key_variable() {
    use serde_json::json;
    let env = [("OPENAI_API_KEY", "sk-from-env")];
    // the preset's variable is used when no api_key is configured
    assert_eq!(
        key_of(json!({"preset": "openai", "model": "m"}), &env).as_deref(),
        Some("sk-from-env")
    );
    // an explicit api_key wins
    assert_eq!(
        key_of(
            json!({"preset": "openai", "model": "m", "api_key": "explicit"}),
            &env
        )
        .as_deref(),
        Some("explicit")
    );
    // a custom endpoint never receives the preset's key
    assert_eq!(
        key_of(
            json!({"preset": "openai", "endpoint": "https://proxy.example/v1", "model": "m"}),
            &env
        ),
        None
    );
    // unset variable, or a preset without one: no key
    assert_eq!(
        key_of(json!({"preset": "openrouter", "model": "m"}), &env),
        None
    );
    assert_eq!(
        key_of(json!({"preset": "ollama", "model": "m"}), &env),
        None
    );
    // an empty variable is no key
    assert_eq!(
        key_of(
            json!({"preset": "openai", "model": "m"}),
            &[("OPENAI_API_KEY", "")]
        ),
        None
    );
}

fn provider(url: &str, key: Option<&str>) -> OpenAiCompatible {
    let mut v = serde_json::json!({ "endpoint": url, "model": "m" });
    if let Some(k) = key {
        v["api_key"] = k.into();
    }
    OpenAiCompatible::from_settings(&v).unwrap()
}

#[tokio::test]
async fn parses_a_successful_response_and_sends_auth() {
    let server = MockHttpServer::start(200, r#"{"choices":[{"message":{"content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1}}"#).await;
    let p = provider(&server.url(), Some("sk-test"));
    let r = p.chat(&req()).await.unwrap();
    assert_eq!(r.text, "hello");
    assert_eq!(
        r.usage,
        Some(Usage {
            input_tokens: 3,
            output_tokens: 1
        })
    );
    let seen = server.last_request().await;
    assert!(seen.starts_with("POST /v1/chat/completions"), "{seen}");
    assert!(seen
        .to_lowercase()
        .contains("authorization: bearer sk-test"));
    assert!(seen.contains("\"stream\":false"));
    assert!(
        !seen.contains("reasoning_effort"),
        "not sent unless configured"
    );
}

#[tokio::test]
async fn sends_reasoning_effort_when_configured() {
    let server = MockHttpServer::start(200, r#"{"choices":[{"message":{"content":"x"}}]}"#).await;
    let p = OpenAiCompatible::from_settings(
        &serde_json::json!({"endpoint": server.url(), "model": "m", "reasoning_effort": "none"}),
    )
    .unwrap();
    p.chat(&req()).await.unwrap();
    assert!(server
        .last_request()
        .await
        .contains("\"reasoning_effort\":\"none\""));
}

#[tokio::test]
async fn maps_http_errors_to_kinds_without_leaking_the_key() {
    for (status, kind) in [
        (401, ProviderErrorKind::Auth),
        (429, ProviderErrorKind::RateLimited),
        (503, ProviderErrorKind::Unavailable),
        (400, ProviderErrorKind::BadRequest),
    ] {
        let server = MockHttpServer::start(status, r#"{"error":{"message":"nope"}}"#).await;
        let e = provider(&server.url(), Some("sk-secret"))
            .chat(&req())
            .await
            .unwrap_err();
        assert_eq!(e.kind, kind);
        assert!(e.message.contains("nope"));
        assert!(!e.message.contains("sk-secret"));
    }
}

#[tokio::test]
async fn connection_refused_is_unavailable_and_falls_back() {
    let e = provider("http://127.0.0.1:1", None)
        .chat(&req())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ProviderErrorKind::Unavailable);
    assert!(e.falls_back());
}

async fn collect_text(p: &OpenAiCompatible) -> (String, Option<Usage>, Option<String>) {
    let r = shellmate_api::collect(p.chat_stream(&req()).await.unwrap())
        .await
        .unwrap();
    (r.text, r.usage, r.finish_reason)
}

#[tokio::test]
async fn streams_text_deltas_and_asks_for_usage() {
    let finish = SseChunk::new(
        0,
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    );
    let usage = SseChunk::new(
        0,
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
    );
    let server = MockSseServer::start(vec![
        SseChunk::new(0, ": keepalive\n\n"),
        SseChunk::delta(0, "Hel"),
        SseChunk::delta(0, "lo"),
        finish,
        usage,
        SseChunk::done(),
    ])
    .await;
    let (text, usage, finish) = collect_text(&provider(&server.url(), None)).await;
    assert_eq!(text, "Hello");
    assert_eq!(
        usage,
        Some(Usage {
            input_tokens: 4,
            output_tokens: 2
        })
    );
    assert_eq!(finish.as_deref(), Some("stop"));
    let sent = server.last_request().await;
    assert!(
        sent.contains("\"stream\":true") && sent.contains("include_usage"),
        "{sent}"
    );
}

#[tokio::test]
async fn events_and_utf8_split_across_network_chunks_are_reassembled() {
    let ev = format!(
        "data: {}\n\n",
        serde_json::json!({"choices":[{"delta":{"content":"café 日本"}}]})
    );
    let bytes = ev.as_bytes();
    // cut in the middle of the JSON *and* inside a multi-byte character
    let cut = ev.find('é').unwrap() + 1;
    let (a, b) = bytes.split_at(cut);
    let (b1, b2) = b.split_at(b.len() - 3);
    let server = MockSseServer::start(vec![
        SseChunk::new(0, a),
        SseChunk::new(5, b1),
        SseChunk::new(5, b2),
        SseChunk::done(),
    ])
    .await;
    assert_eq!(
        collect_text(&provider(&server.url(), None)).await.0,
        "café 日本"
    );
}

#[tokio::test]
async fn crlf_terminated_events_and_missing_done_work() {
    let ev = format!(
        "data: {}\r\n\r\n",
        serde_json::json!({"choices":[{"delta":{"content":"ok"}}]})
    );
    let server = MockSseServer::start(vec![SseChunk::new(0, ev)]).await;
    assert_eq!(collect_text(&provider(&server.url(), None)).await.0, "ok");
}

#[tokio::test]
async fn dropping_the_stream_closes_the_connection() {
    let chunks: Vec<_> = (0..200).map(|_| SseChunk::delta(20, "word ")).collect();
    let server = MockSseServer::start(chunks).await;
    let p = provider(&server.url(), None);
    let mut s = p.chat_stream(&req()).await.unwrap();
    assert!(matches!(s.next().await, Some(Ok(StreamEvent::Text(_)))));
    drop(s);
    for _ in 0..100 {
        if server.client_disconnected() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("server never noticed the client going away (generation would keep running)");
}

#[tokio::test]
async fn http_errors_before_the_stream_are_classified() {
    let server = MockHttpServer::start(429, r#"{"error":{"message":"slow down"}}"#).await;
    let e = provider(&server.url(), None)
        .chat_stream(&req())
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ProviderErrorKind::RateLimited);
}

#[test]
fn endpoint_rules() {
    assert!(shellmate_api::check_endpoint("https://api.example.com/v1").is_ok());
    assert!(shellmate_api::check_endpoint("http://localhost:11434/v1").is_ok());
    assert!(shellmate_api::check_endpoint("http://[::1]:8080").is_ok());
    assert_eq!(
        shellmate_api::check_endpoint("http://example.com/v1")
            .unwrap_err()
            .code,
        "config.insecure_endpoint"
    );
    assert_eq!(
        shellmate_api::check_endpoint("ftp://x").unwrap_err().code,
        "config.invalid"
    );
}

#[test]
fn presets_and_validation() {
    let ok =
        OpenAiCompatible::from_settings(&serde_json::json!({"preset": "ollama", "model": "m"}))
            .unwrap();
    assert_eq!(ok.url, "http://localhost:11434/v1/chat/completions");
    assert!(OpenAiCompatible::from_settings(&serde_json::json!({"model": "m"})).is_err());
    assert!(
        OpenAiCompatible::from_settings(&serde_json::json!({"preset": "nope", "model": "m"}))
            .is_err()
    );
    assert!(OpenAiCompatible::from_settings(
        &serde_json::json!({"preset": "ollama", "model": "m", "typo": 1})
    )
    .is_err());
}

#[tokio::test]
async fn models_lists_the_ids_and_sends_only_the_key() {
    let server = MockHttpServer::start(
        200,
        r#"{"object":"list","data":[{"id":"m-one"},{"id":"~alias/latest"}]}"#,
    )
    .await;
    let p = provider(&server.url(), Some("sk-test"));
    assert_eq!(
        p.models().await.unwrap(),
        Some(vec!["m-one".to_string(), "~alias/latest".to_string()])
    );
    let seen = server.last_request().await;
    assert!(seen.starts_with("GET /v1/models"), "{seen}");
    assert!(seen
        .to_lowercase()
        .contains("authorization: bearer sk-test"));
    // nothing but the credentials leaves: no body, no model name, no prompt
    assert!(
        !seen.contains("\"model\"") && !seen.contains("messages"),
        "{seen}"
    );
}

#[tokio::test]
async fn models_without_a_key_sends_no_authorization() {
    let server = MockHttpServer::start(200, r#"{"data":[]}"#).await;
    let p = provider(&server.url(), None);
    assert_eq!(p.models().await.unwrap(), Some(vec![]));
    assert!(!server
        .last_request()
        .await
        .to_lowercase()
        .contains("authorization"));
}

#[tokio::test]
async fn models_tells_a_rejected_key_a_wrong_url_and_an_unreadable_reply_apart() {
    let rejected = MockHttpServer::start(401, r#"{"error":{"message":"bad key"}}"#).await;
    let e = provider(&rejected.url(), Some("sk-secret"))
        .models()
        .await
        .unwrap_err();
    assert_eq!(e.kind, ProviderErrorKind::Auth);
    assert!(!e.message.contains("sk-secret"));

    let missing = MockHttpServer::start(404, "not found").await;
    let e = provider(&missing.url(), None).models().await.unwrap_err();
    assert_eq!(e.kind, ProviderErrorKind::BadRequest);
    assert!(e.message.contains("without `/v1`"), "{}", e.message);

    // connected, but nothing we can read: an empty list, not an error and not "no models"
    let odd = MockHttpServer::start(200, "<html>hello</html>").await;
    assert_eq!(
        provider(&odd.url(), None).models().await.unwrap(),
        Some(vec![])
    );
}
