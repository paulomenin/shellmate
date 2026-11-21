use super::client::*;
use shellmate_api::{
    ChatRequest, Message, Provider, ProviderErrorKind, StreamEvent, ToolChoice, ToolRequest,
    ToolSpec, ToolSupport, Usage,
};
use shellmate_testing::{MockHttpServer, MockSseServer, SseChunk};

fn req() -> ChatRequest {
    ChatRequest {
        request_id: "r1".into(),
        messages: vec![Message::user("hi")],
        ..ChatRequest::default()
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

// ---- native tool calling ----------------------------------------------------------------------------------

fn man_spec() -> ToolSpec {
    ToolSpec {
        name: "man".into(),
        description: "Read a manual page.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "page": { "type": "string" } },
            "required": ["page"],
            "additionalProperties": false,
        }),
    }
}

fn body_of(sent: &str) -> serde_json::Value {
    serde_json::from_str(sent.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn tools_and_a_finished_call_are_sent_in_the_shape_of_the_api() {
    let server = MockHttpServer::start(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#).await;
    let mut r = req();
    r.tools = vec![man_spec()];
    r.messages = vec![
        Message::user("how do I extract"),
        Message::assistant_calling(
            "",
            vec![ToolRequest {
                id: "call_1".into(),
                name: "man".into(),
                arguments: r#"{"page":"tar"}"#.into(),
            }],
        ),
        Message::tool_result("call_1", "TAR(1) ..."),
    ];
    provider(&server.url(), None).chat(&r).await.unwrap();
    let b = body_of(&server.last_request().await);
    assert_eq!(b["tools"][0]["type"], "function");
    assert_eq!(b["tools"][0]["function"]["name"], "man");
    assert_eq!(
        b["tools"][0]["function"]["parameters"]["required"][0],
        "page"
    );
    assert!(
        b.get("tool_choice").is_none(),
        "auto is the default and is not sent"
    );
    let m = &b["messages"];
    assert_eq!(
        m[0],
        serde_json::json!({"role": "user", "content": "how do I extract"})
    );
    assert_eq!(m[1]["role"], "assistant");
    assert!(
        m[1]["content"].is_null(),
        "a message that is only calls has no text: {m}"
    );
    assert_eq!(m[1]["tool_calls"][0]["id"], "call_1");
    assert_eq!(m[1]["tool_calls"][0]["type"], "function");
    assert_eq!(m[1]["tool_calls"][0]["function"]["name"], "man");
    assert_eq!(
        m[1]["tool_calls"][0]["function"]["arguments"],
        r#"{"page":"tar"}"#
    );
    assert_eq!(
        m[2],
        serde_json::json!({"role": "tool", "tool_call_id": "call_1", "content": "TAR(1) ..."})
    );
}

#[tokio::test]
async fn a_request_without_tools_is_what_it_always_was() {
    let server = MockHttpServer::start(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#).await;
    provider(&server.url(), None).chat(&req()).await.unwrap();
    let b = body_of(&server.last_request().await);
    assert!(
        b.get("tools").is_none() && b.get("tool_choice").is_none(),
        "{b}"
    );
    assert_eq!(
        b["messages"],
        serde_json::json!([{"role": "user", "content": "hi"}])
    );
    // an assistant that said something and made no call stays a plain message
    let mut r = req();
    r.messages.push(Message::assistant("hello"));
    provider(&server.url(), None).chat(&r).await.unwrap();
    let b = body_of(&server.last_request().await);
    assert_eq!(
        b["messages"][1],
        serde_json::json!({"role": "assistant", "content": "hello"})
    );
}

#[tokio::test]
async fn the_model_can_be_told_to_answer_in_text() {
    let server = MockHttpServer::start(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#).await;
    let mut r = req();
    r.tools = vec![man_spec()];
    r.tool_choice = ToolChoice::None;
    provider(&server.url(), None).chat(&r).await.unwrap();
    let b = body_of(&server.last_request().await);
    assert_eq!(b["tool_choice"], "none");
    assert!(b["tools"].is_array(), "the tools stay listed");
    // with no tools there is nothing to choose
    r.tools.clear();
    provider(&server.url(), None).chat(&r).await.unwrap();
    assert!(body_of(&server.last_request().await)
        .get("tool_choice")
        .is_none());
}

#[tokio::test]
async fn a_whole_reply_with_calls_is_read() {
    let server = MockHttpServer::start(
        200,
        r#"{"choices":[{"message":{"content":null,"tool_calls":[
            {"id":"call_a","type":"function","function":{"name":"man","arguments":"{\"page\":\"tar\"}"}},
            {"type":"function","function":{"name":"read","arguments":{"path":"Cargo.toml"}}},
            {"id":"call_c","function":{"arguments":"{}"}}
        ]},"finish_reason":"tool_calls"}]}"#,
    )
    .await;
    let r = provider(&server.url(), None).chat(&req()).await.unwrap();
    assert_eq!(r.text, "");
    assert_eq!(
        r.tool_calls,
        [
            ToolRequest {
                id: "call_a".into(),
                name: "man".into(),
                arguments: r#"{"page":"tar"}"#.into()
            },
            // no id: one from its place; arguments sent as an object: their JSON text
            ToolRequest {
                id: "call_1".into(),
                name: "read".into(),
                arguments: r#"{"path":"Cargo.toml"}"#.into()
            },
        ],
        "a call without a name is dropped"
    );
    assert_eq!(r.finish_reason.as_deref(), Some("tool_calls"));
}

fn call_delta(json: &str) -> SseChunk {
    SseChunk::new(
        0,
        format!("data: {{\"choices\":[{{\"delta\":{json}}}]}}\n\n"),
    )
}

async fn streamed(chunks: Vec<SseChunk>) -> shellmate_api::ChatResponse {
    let server = MockSseServer::start(chunks).await;
    shellmate_api::collect(
        provider(&server.url(), None)
            .chat_stream(&req())
            .await
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_streamed_call_is_put_together_from_its_fragments() {
    let r = streamed(vec![
        SseChunk::delta(0, "Let me check."),
        call_delta(r#"{"tool_calls":[{"index":0,"id":"call_x","type":"function","function":{"name":"man","arguments":""}}]}"#),
        call_delta(r#"{"tool_calls":[{"index":0,"function":{"arguments":"{\"pa"}}]}"#),
        call_delta(r#"{"tool_calls":[{"index":0,"function":{"arguments":"ge\":\"t"}}]}"#),
        call_delta(r#"{"tool_calls":[{"index":0,"function":{"arguments":"ar\"}"}}]}"#),
        SseChunk::new(0, "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n"),
        SseChunk::done(),
    ])
    .await;
    assert_eq!(r.text, "Let me check.");
    assert_eq!(
        r.tool_calls,
        [ToolRequest {
            id: "call_x".into(),
            name: "man".into(),
            arguments: r#"{"page":"tar"}"#.into()
        }]
    );
    assert_eq!(r.finish_reason.as_deref(), Some("tool_calls"));
}

#[tokio::test]
async fn calls_that_interleave_or_arrive_whole_keep_their_own_arguments() {
    // two calls whose fragments are mixed, numbered by index
    let r = streamed(vec![
        call_delta(r#"{"tool_calls":[{"index":0,"id":"a","function":{"name":"man","arguments":"{\"page\":"}},{"index":1,"id":"b","function":{"name":"help","arguments":"{\"program\":"}}]}"#),
        call_delta(r#"{"tool_calls":[{"index":1,"function":{"arguments":"\"git\"}"}},{"index":0,"function":{"arguments":"\"tar\"}"}}]}"#),
        SseChunk::done(),
    ])
    .await;
    assert_eq!(r.tool_calls.len(), 2);
    assert_eq!(
        (
            r.tool_calls[0].id.as_str(),
            r.tool_calls[0].arguments.as_str()
        ),
        ("a", r#"{"page":"tar"}"#)
    );
    assert_eq!(
        (
            r.tool_calls[1].id.as_str(),
            r.tool_calls[1].arguments.as_str()
        ),
        ("b", r#"{"program":"git"}"#)
    );

    // a server that sends each call whole and numbers none: one call per chunk, ids from their place
    let r = streamed(vec![
        call_delta(
            r#"{"tool_calls":[{"function":{"name":"man","arguments":"{\"page\":\"ls\"}"}}]}"#,
        ),
        call_delta(r#"{"tool_calls":[{"function":{"name":"man","arguments":{"page":"cp"}}}]}"#),
        SseChunk::done(),
    ])
    .await;
    assert_eq!(
        r.tool_calls
            .iter()
            .map(|c| (c.id.as_str(), c.arguments.as_str()))
            .collect::<Vec<_>>(),
        [
            ("call_0", r#"{"page":"ls"}"#),
            ("call_1", r#"{"page":"cp"}"#)
        ]
    );
}

#[tokio::test]
async fn a_call_without_a_name_or_with_a_wild_index_is_not_a_call() {
    let r = streamed(vec![
        call_delta(r#"{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}"#),
        call_delta(r#"{"tool_calls":[{"index":9999999,"id":"z","function":{"name":"man","arguments":"{}"}}]}"#),
        SseChunk::delta(0, "text"),
        SseChunk::done(),
    ])
    .await;
    assert!(r.tool_calls.is_empty(), "{:?}", r.tool_calls);
    assert_eq!(r.text, "text");
}

#[tokio::test]
async fn a_call_without_arguments_has_empty_ones() {
    let r = streamed(vec![
        call_delta(r#"{"tool_calls":[{"index":0,"id":"a","function":{"name":"man"}}]}"#),
        SseChunk::done(),
    ])
    .await;
    assert_eq!(r.tool_calls[0].arguments, "{}");
}

#[tokio::test]
async fn the_model_list_tells_whether_this_model_takes_tools() {
    let list = r#"{"data":[
        {"id":"m","supported_parameters":["tools","temperature"]},
        {"id":"other","supported_parameters":["temperature"]}
    ]}"#;
    let server = MockHttpServer::start(200, list).await;
    assert_eq!(
        provider(&server.url(), None).tool_support().await,
        ToolSupport::Yes
    );
    let server = MockHttpServer::start(
        200,
        r#"{"data":[{"id":"m","supported_parameters":["temperature"]}]}"#,
    )
    .await;
    assert_eq!(
        provider(&server.url(), None).tool_support().await,
        ToolSupport::No
    );
    // a list that does not say, a model that is not in it, an error and a dead server: not known
    for (status, body) in [
        (200, r#"{"data":[{"id":"m"}]}"#),
        (200, r#"{"data":[]}"#),
        (500, "oops"),
    ] {
        let server = MockHttpServer::start(status, body).await;
        assert_eq!(
            provider(&server.url(), None).tool_support().await,
            ToolSupport::Unknown,
            "{status} {body}"
        );
    }
    assert_eq!(
        provider("http://127.0.0.1:1", None).tool_support().await,
        ToolSupport::Unknown
    );
}
