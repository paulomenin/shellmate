use super::*;
use shellmate_api::{Decider, DecisionRequest, DecisionResponse};

struct FakeDecider {
    reply: Mutex<Option<std::result::Result<DecisionResponse, shellmate_api::ProviderError>>>,
    asked: Mutex<Vec<DecisionRequest>>,
}
impl FakeDecider {
    fn new(r: std::result::Result<DecisionResponse, shellmate_api::ProviderError>) -> Arc<Self> {
        Arc::new(Self {
            reply: Mutex::new(Some(r)),
            asked: Mutex::new(Vec::new()),
        })
    }
    fn count(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}
#[async_trait]
impl Decider for FakeDecider {
    fn info(&self) -> shellmate_api::ProviderInfo {
        shellmate_api::ProviderInfo {
            plugin: "fake".into(),
            model: "fake".into(),
            endpoint: None,
        }
    }
    async fn decide(
        &self,
        request: &DecisionRequest,
    ) -> std::result::Result<DecisionResponse, shellmate_api::ProviderError> {
        self.asked.lock().unwrap().push(request.clone());
        self.reply.lock().unwrap().take().expect("asked once")
    }
}

fn spec(name: &str, d: Arc<FakeDecider>, trust: Trust) -> DeciderSpec {
    DeciderSpec {
        name: name.into(),
        decider: d as Arc<dyn Decider>,
        trust,
    }
}

fn verdict(yes: f32, label: &str, p: f32) -> DecisionResponse {
    use shellmate_api::Answer;
    DecisionResponse {
        answers: vec![
            ("match".into(), Answer::YesNo { probability: yes }),
            (
                "risk".into(),
                Answer::Choice {
                    label: label.into(),
                    confidence: 0.5,
                    probabilities: vec![(label.into(), p)],
                },
            ),
        ],
    }
}

/// A chat model that also sits in the validator chain.
fn chat_check(name: &str, p: Arc<FakeProvider>) -> Check {
    Check::Chat(ProviderSpec {
        name: name.into(),
        provider: p as Arc<dyn Provider>,
        max_level: 1,
        trust: Trust::Local,
    })
}

fn down() -> Arc<FakeDecider> {
    FakeDecider::new(Err(shellmate_api::ProviderError::new(
        shellmate_api::ProviderErrorKind::Unavailable,
        "connection refused",
    )))
}

const DF: &str = "Here.\n```shellmate-command\ndf -h\n```";

#[tokio::test]
async fn a_confident_decider_finding_is_shown_and_the_llm_is_not_asked_again() {
    let d = FakeDecider::new(Ok(verdict(0.05, "read_only", 0.9)));
    let p = FakeProvider::replying(DF);
    let out = engine(vec![("a", p.clone(), 1)])
        .with_builtin_tools()
        .with_deciders(vec![spec("laya", d.clone(), Trust::Local)])
        .run(&env("ask", "show disk usage"))
        .await
        .unwrap();
    assert_eq!(
        out.buffer.as_deref(),
        Some("df -h"),
        "the command is still placed"
    );
    assert!(
        out.trailer_text()
            .contains("check: this may not be what you asked"),
        "{}",
        out.trailer_text()
    );
    assert_eq!(d.count(), 1);
    assert_eq!(p.seen.lock().unwrap().len(), 1, "no second LLM call");
    let q = d.asked.lock().unwrap()[0].clone();
    assert!(q
        .state
        .iter()
        .any(|(k, v)| k == "proposed_command" && v == "df -h"));
    assert!(q
        .state
        .iter()
        .any(|(k, v)| k == "request" && v == "show disk usage"));
}

#[tokio::test]
async fn an_unsure_decider_stays_silent() {
    let d = FakeDecider::new(Ok(verdict(0.4, "destructive", 0.5)));
    let out = engine(vec![("a", FakeProvider::replying(DF), 1)])
        .with_builtin_tools()
        .with_deciders(vec![spec("laya", d, Trust::Local)])
        .run(&env("ask", "show disk usage"))
        .await
        .unwrap();
    assert!(
        !out.trailer_text().contains("check:"),
        "{}",
        out.trailer_text()
    );
}

#[tokio::test]
async fn a_failing_model_falls_back_to_the_next_in_the_chain() {
    let down = down();
    let p = FakeProvider::with(vec![
        Ok(DF.to_string()),
        Ok("{\"match\": 0.02, \"risk\": \"read_only\"}".to_string()),
    ]);
    let out = engine(vec![("a", p.clone(), 1)])
        .with_builtin_tools()
        .with_checks(vec![
            Check::Decision(spec("laya", down.clone(), Trust::Local)),
            chat_check("a", p.clone()),
        ])
        .run(&env("ask", "list files"))
        .await
        .unwrap();
    assert_eq!(down.count(), 1);
    assert!(
        out.trailer_text().contains("may not be what you asked"),
        "{}",
        out.trailer_text()
    );
    // the LLM got the same questions, with the command as data
    let seen = p.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].messages[1]
        .content
        .contains("proposed_command: df -h"));
}

#[tokio::test]
async fn a_chat_model_in_the_chain_answers_the_same_questions() {
    let p = FakeProvider::with(vec![
        Ok(DF.to_string()),
        Ok("{\"match\": 0.97, \"risk\": \"read_only\"}".to_string()),
    ]);
    let out = engine(vec![("a", p.clone(), 1)])
        .with_checks(vec![chat_check("a", p.clone())])
        .run(&env("ask", "show disk usage"))
        .await
        .unwrap();
    assert_eq!(p.seen.lock().unwrap().len(), 2);
    assert!(
        !out.trailer_text().contains("check:"),
        "a clean command has no notes: {}",
        out.trailer_text()
    );
}

#[tokio::test]
async fn when_nothing_in_the_chain_answers_the_check_is_skipped_visibly() {
    let p = FakeProvider::replying(DF);
    let d = down();
    let out = engine(vec![("a", p.clone(), 1)])
        .with_deciders(vec![spec("laya", d.clone(), Trust::Local)])
        .run(&env("ask", "show disk usage"))
        .await
        .unwrap();
    assert_eq!(d.count(), 1);
    assert_eq!(
        p.seen.lock().unwrap().len(),
        1,
        "the chat model of the llm chain is not asked unless it is in the validator chain"
    );
    assert!(
        out.trailer_text()
            .contains("the semantic check was skipped"),
        "{}",
        out.trailer_text()
    );
    assert!(out.trailer_text().contains("`laya`"));
    assert_eq!(out.buffer.as_deref(), Some("df -h"));
}

#[tokio::test]
async fn the_semantic_check_is_off_without_a_chain() {
    let p = FakeProvider::replying(DF);
    let out = engine(vec![("a", p.clone(), 1)])
        .run(&env("ask", "show disk usage"))
        .await
        .unwrap();
    assert_eq!(p.seen.lock().unwrap().len(), 1);
    assert!(!out.trailer_text().contains("check:"));
}

#[tokio::test]
async fn deterministic_checks_flag_missing_programs_and_bad_syntax() {
    let p = FakeProvider::replying("```shellmate-command\nlsss -la\n```");
    let out = engine(vec![("a", p, 1)])
        .run(&env("generate", "list files"))
        .await
        .unwrap();
    assert!(
        out.trailer_text()
            .contains("check: `lsss` was not found in PATH"),
        "{}",
        out.trailer_text()
    );
    assert_eq!(out.buffer.as_deref(), Some("lsss -la"));
    let p = FakeProvider::replying("```shellmate-command\nls |\n```");
    let mut e = env("generate", "list files");
    e.shell = "bash".into();
    let out = engine(vec![("a", p, 1)]).run(&e).await.unwrap();
    assert!(
        out.trailer_text().contains("does not parse in bash"),
        "{}",
        out.trailer_text()
    );
}

#[tokio::test]
async fn the_validator_can_be_turned_off_and_never_runs_for_explain() {
    let mut cfg = Config::defaults();
    cfg.validator.enabled = false;
    let p = FakeProvider::replying("```shellmate-command\nlsss -la\n```");
    let out = engine(vec![("a", p, 1)])
        .with_config(cfg)
        .with_builtin_tools()
        .run(&env("generate", "list files"))
        .await
        .unwrap();
    assert!(out.trailer_text().is_empty(), "{}", out.trailer_text());
    let p = FakeProvider::replying("It lists.\n```shellmate-command\nlsss\n```");
    let out = engine(vec![("a", p, 1)])
        .run(&env("explain", "lsss"))
        .await
        .unwrap();
    assert!(!out.trailer_text().contains("check:"));
}

#[test]
fn a_remote_decider_needs_redaction() {
    let d = FakeDecider::new(Ok(verdict(0.9, "read_only", 0.9)));
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)]).with_deciders(vec![spec(
        "hosted",
        d,
        Trust::Remote,
    )]);
    assert_eq!(e.check_privacy().unwrap_err().code, "privacy.no_redaction");
}

#[test]
fn a_remote_chat_model_in_the_chain_needs_redaction() {
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)]).with_checks(vec![Check::Chat(
        ProviderSpec {
            name: "cloud".into(),
            provider: FakeProvider::replying("x") as Arc<dyn Provider>,
            max_level: 1,
            trust: Trust::Remote,
        },
    )]);
    let err = e.check_privacy().unwrap_err();
    assert_eq!(err.code, "privacy.no_redaction");
    assert!(err.message.contains("cloud"), "{}", err.message);
}

#[tokio::test]
async fn a_remote_decider_is_not_asked_when_a_layer_swallows_the_record() {
    let d = FakeDecider::new(Ok(verdict(0.9, "read_only", 0.9)));
    let out = Engine::with_providers(
        vec![ProviderSpec {
            name: "a".into(),
            provider: FakeProvider::replying(DF) as Arc<dyn Provider>,
            max_level: 1,
            trust: Trust::Remote,
        }],
        1,
    )
    .with_layers(vec![Arc::new(ShortCircuit)], true)
    .with_deciders(vec![spec("hosted", d.clone(), Trust::Remote)])
    .run(&env("ask", "show disk usage"))
    .await;
    // the chat request is swallowed by the same layer, so the run fails or ends without a check: what matters
    // is that nothing reached the decider
    let _ = out;
    assert_eq!(d.count(), 0, "the record must not be sent unredacted");
}

fn kinds() -> Registry {
    let mut r = Registry::new();
    r.register_provider(
        "chat",
        Box::new(|_| Ok(FakeProvider::replying("x") as Arc<dyn Provider>)),
    );
    r.register_decider(
        "decide",
        Box::new(|_| Ok(FakeDecider::new(Ok(verdict(0.9, "read_only", 0.9))) as Arc<dyn Decider>)),
    );
    r
}

const TWO_KINDS: &str = "version = 1\n[pipeline]\nllm = []\n[conversation]\nstore = \"none\"\n\
    [providers.p]\ntrust = \"local\"\n\
    [models.chat]\nprovider = \"p\"\nplugin = \"builtin:chat\"\nmodel = \"c\"\n\
    [models.judge]\nprovider = \"p\"\nplugin = \"builtin:decide\"\nmodel = \"d\"\n";

#[test]
fn the_validator_chain_takes_both_kinds_and_the_llm_chain_only_chat_models() {
    let ok = Config::parse(&format!(
        "{TWO_KINDS}[llm]\nchain = [\"chat\"]\n[validator]\nchain = [\"judge\", \"chat\"]\n"
    ))
    .unwrap();
    let e = Engine::from_config_in(&ok, &kinds(), Host::system()).unwrap();
    assert_eq!(e.checks.len(), 2);
    assert!(matches!(e.checks[0], Check::Decision(_)));
    assert!(matches!(e.checks[1], Check::Chat(_)));

    let bad = Config::parse(&format!("{TWO_KINDS}[llm]\nchain = [\"judge\"]\n")).unwrap();
    let err = Engine::from_config_in(&bad, &kinds(), Host::system())
        .err()
        .unwrap();
    assert_eq!(err.code, "config.wrong_kind");
    assert!(err.message.contains("judge"), "{}", err.message);
    // `config check` says the same, and lists the chat models
    let err = bad.validate_with(&kinds(), &Host::system()).unwrap_err();
    assert_eq!(err.code, "config.wrong_kind");
    let hint = err.hint.unwrap();
    assert!(hint.contains("chat models: chat"), "{hint}");
    assert!(!hint.contains("judge"), "{hint}");
}

#[tokio::test]
async fn a_remote_decider_is_asked_through_the_redaction_chain() {
    let d = FakeDecider::new(Ok(verdict(0.9, "read_only", 0.9)));
    let out = Engine::with_providers(
        vec![ProviderSpec {
            name: "a".into(),
            provider: FakeProvider::replying(DF) as Arc<dyn Provider>,
            max_level: 1,
            trust: Trust::Remote,
        }],
        1,
    )
    .with_layers(vec![Arc::new(Passthrough)], true)
    .with_deciders(vec![spec("hosted", d.clone(), Trust::Remote)])
    .run(&env("ask", "show disk usage"))
    .await
    .unwrap();
    assert_eq!(out.buffer.as_deref(), Some("df -h"));
    assert_eq!(d.count(), 1, "the remote decider was asked");
    let q = d.asked.lock().unwrap()[0].clone();
    assert!(q
        .state
        .iter()
        .any(|(k, v)| k == "proposed_command" && v == "df -h"));
}

/// A decision model that takes a while, so that the check is a real part of the run. Nothing asserts how long.
struct SlowDecider(Arc<FakeDecider>);

#[async_trait]
impl Decider for SlowDecider {
    fn info(&self) -> shellmate_api::ProviderInfo {
        self.0.info()
    }
    async fn decide(
        &self,
        request: &DecisionRequest,
    ) -> std::result::Result<DecisionResponse, shellmate_api::ProviderError> {
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        self.0.decide(request).await
    }
}

#[tokio::test]
async fn the_stages_of_a_run_do_not_overlap() {
    let d = FakeDecider::new(Ok(verdict(0.9, "read_only", 0.9)));
    let out = engine(vec![("a", FakeProvider::replying(DF), 1)])
        .with_deciders(vec![DeciderSpec {
            name: "slow".into(),
            decider: Arc::new(SlowDecider(d.clone())) as Arc<dyn Decider>,
            trust: Trust::Local,
        }])
        .run(&env("ask", "show disk usage"))
        .await
        .unwrap();
    assert_eq!(d.count(), 1, "the check ran");
    let s = &out.report.stages;
    // the check took a while, and the model's stage must not contain it: the three stages are disjoint
    // stretches of the run, so they cannot add up to more than the whole
    assert!(
        s.context_us + s.llm_us + s.validate_us <= s.total_us,
        "{s:?}"
    );
    assert_eq!(out.report.llm_ms, s.llm_us / 1000);
}
