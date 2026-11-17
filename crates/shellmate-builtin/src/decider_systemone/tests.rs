use super::*;
use shellmate_testing::MockHttpServer;

fn req() -> DecisionRequest {
    DecisionRequest {
        state: vec![("request".into(), "show disk usage".into())],
        questions: vec![
            (
                "match".into(),
                Question::YesNo {
                    instructions: "Does it match?".into(),
                },
            ),
            (
                "risk".into(),
                Question::Choice {
                    instructions: "How risky?".into(),
                    criteria: vec![
                        ("safe".into(), "reads only".into()),
                        ("danger".into(), "destroys data".into()),
                    ],
                },
            ),
        ],
    }
}

fn settings(url: &str) -> Value {
    json!({"endpoint": url, "model": "laya"})
}

#[test]
fn the_wire_request_has_the_expected_shape() {
    let w = wire_request("laya", &req());
    assert_eq!(w["model"], "laya");
    assert_eq!(w["state"]["request"], "show disk usage");
    assert_eq!(w["questions"]["match"]["type"], "noul");
    assert_eq!(w["questions"]["risk"]["type"], "choice");
    assert_eq!(
        w["questions"]["risk"]["criteria"]["danger"],
        "destroys data"
    );
}

#[tokio::test]
async fn models_lists_ids_with_the_key_only_and_names_a_wrong_url() {
    let server = MockHttpServer::start(200, r#"{"data":[{"id":"jev-latest"}]}"#).await;
    let d = SystemOne::from_settings(
        &json!({"model": "m", "endpoint": server.url(), "api_key": "sk-test"}),
    )
    .unwrap();
    assert_eq!(
        d.models().await.unwrap(),
        Some(vec!["jev-latest".to_string()])
    );
    let seen = server.last_request().await;
    assert!(seen.starts_with("GET /v1/models"), "{seen}");
    assert!(seen
        .to_lowercase()
        .contains("authorization: bearer sk-test"));
    assert!(
        !seen.contains("questions") && !seen.contains("state"),
        "{seen}"
    );

    let rejected = MockHttpServer::start(401, "no").await;
    let d = SystemOne::from_settings(&json!({"model": "m", "endpoint": rejected.url()})).unwrap();
    assert_eq!(d.models().await.unwrap_err().kind, ProviderErrorKind::Auth);

    let missing = MockHttpServer::start(404, "nope").await;
    let d = SystemOne::from_settings(&json!({"model": "m", "endpoint": missing.url()})).unwrap();
    assert!(d
        .models()
        .await
        .unwrap_err()
        .message
        .contains("without `/v1`"));
}

#[test]
fn only_hosts_known_to_serve_the_format_are_offered_as_checkers() {
    let names: Vec<_> = checker_presets().iter().map(|p| p.name).collect();
    assert_eq!(names, ["openrouter", "ollama", "typesafe"]);
}

#[test]
fn the_typesafe_preset_is_for_deciders_and_reads_its_own_key_variable() {
    let env = |n: &str| (n == "TYPESAFE_API_KEY").then(|| "ts-env".to_string());
    let d =
        SystemOne::from_settings_with(&json!({"model": "jev-latest", "preset": "typesafe"}), &env)
            .unwrap();
    assert_eq!(d.url, "https://api.typesafe.ai/v1/systemone");
    assert_eq!(
        d.api_key.map(|k| k.expose().to_string()).as_deref(),
        Some("ts-env")
    );
    // a provider does not know it: it would not serve chat completions there
    assert!(!PRESETS.iter().any(|p| p.name == "typesafe"));
}

#[test]
fn the_path_comes_from_the_setting_then_the_preset_then_the_default() {
    let url = |v: Value| SystemOne::from_settings(&v).unwrap().url;
    assert_eq!(
        url(json!({"model": "m", "preset": "openrouter"})),
        "https://openrouter.ai/api/alpha/decisions"
    );
    assert_eq!(
        url(json!({"model": "m", "preset": "ollama"})),
        "http://localhost:11434/v1/systemone"
    );
    assert_eq!(
        url(json!({"model": "m", "preset": "openrouter", "path": "/v2/decide"})),
        "https://openrouter.ai/api/v2/decide"
    );
    // a custom endpoint does not inherit the preset's path
    assert_eq!(
        url(json!({"model": "m", "preset": "openrouter", "endpoint": "https://proxy.example"})),
        "https://proxy.example/v1/systemone"
    );
    assert_eq!(
        url(json!({"model": "m", "endpoint": "https://proxy.example/", "path": "/x"})),
        "https://proxy.example/x"
    );
}

#[test]
fn a_path_that_could_change_the_host_or_the_query_is_refused() {
    for bad in [
        "v1/systemone",
        "//evil.example/x",
        "/a/../b",
        "/x?key=1",
        "/x#frag",
        "http://evil.example",
        "/a b",
        "/a\\b",
        "",
    ] {
        let r = SystemOne::from_settings(
            &json!({"model": "m", "endpoint": "https://ok.example", "path": bad}),
        );
        assert!(r.is_err(), "{bad:?} should be refused");
    }
}

#[test]
fn a_hosted_preset_supplies_its_key_variable_and_a_custom_endpoint_never_gets_it() {
    let env = |n: &str| (n == "OPENAI_API_KEY").then(|| "sk-env".to_string());
    let key = |v: Value| {
        SystemOne::from_settings_with(&v, &env)
            .unwrap()
            .api_key
            .map(|k| k.expose().to_string())
    };
    assert_eq!(
        key(json!({"model": "m", "preset": "openai"})).as_deref(),
        Some("sk-env")
    );
    assert_eq!(
        key(json!({"model": "m", "preset": "openai", "api_key": "own"})).as_deref(),
        Some("own")
    );
    assert_eq!(
        key(json!({"model": "m", "preset": "openai", "endpoint": "https://proxy.example"})),
        None
    );
    assert_eq!(key(json!({"model": "m", "preset": "ollama"})), None);
}

#[tokio::test]
async fn answers_are_parsed_and_the_endpoint_path_is_added() {
    let body = json!({"model":"laya:en","answers":{
        "match":{"type":"noul","noul":0.12},
        "risk":{"type":"choice","choice":"safe","confidence":0.8,"probabilities":{"safe":0.9,"danger":0.1}}},
        "usage":{"input_tokens":5,"output_tokens":0}})
    .to_string();
    let server = MockHttpServer::start(200, &body).await;
    let d = SystemOne::from_settings(&settings(&server.url())).unwrap();
    let r = d.decide(&req()).await.unwrap();
    assert_eq!(r.get("match"), Some(&Answer::YesNo { probability: 0.12 }));
    let Some(Answer::Choice {
        label,
        confidence,
        probabilities,
    }) = r.get("risk")
    else {
        panic!()
    };
    assert_eq!((label.as_str(), *confidence), ("safe", 0.8));
    assert_eq!(probabilities[1], ("danger".to_string(), 0.1));
    assert!(server.last_request().await.contains("/v1/systemone"));
}

#[tokio::test]
async fn unreadable_answers_are_errors_not_decisions() {
    for body in [
        r#"{"answers":{}}"#.to_string(),
        r#"{"answers":{"match":{"noul":0.1},"risk":{"choice":"made-up"}}}"#.to_string(),
        "not json".to_string(),
    ] {
        let server = MockHttpServer::start(200, &body).await;
        let d = SystemOne::from_settings(&settings(&server.url())).unwrap();
        assert!(d.decide(&req()).await.is_err(), "{body}");
    }
    let server = MockHttpServer::start(404, r#"{"error":{"message":"model not found"}}"#).await;
    let d = SystemOne::from_settings(&settings(&server.url())).unwrap();
    let e = d.decide(&req()).await.unwrap_err();
    assert_eq!(e.kind, ProviderErrorKind::BadRequest);
    assert!(e.message.contains("model not found"));
}

#[test]
fn settings_are_validated() {
    assert!(SystemOne::from_settings(&json!({"model": "m"})).is_err());
    assert!(SystemOne::from_settings(&json!({"model": "m", "preset": "nope"})).is_err());
    // the presets are the ones providers use, and the hosts that only serve decision models
    assert!(PRESETS.iter().all(|p| DECIDER_PRESETS.contains(p)));
    for p in &DECIDER_PRESETS {
        let d = SystemOne::from_settings(&json!({"model": "m", "preset": p.name})).unwrap();
        let path = preset_path(p.name).unwrap_or(DEFAULT_PATH);
        assert_eq!(d.url, format!("{}{path}", p.endpoint), "{}", p.name);
    }
    assert!(
        SystemOne::from_settings(&json!({"model": "m", "endpoint": "http://10.1.1.1:1"})).is_err()
    );
    assert!(SystemOne::from_settings(
        &json!({"model": "m", "endpoint": "https://x.example", "typo": 1})
    )
    .is_err());
}
