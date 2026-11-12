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

fn semantic_cfg(deciders: &[&str], llm_fallback: bool) -> Config {
    let mut cfg = Config::defaults();
    cfg.validator.semantic = true;
    cfg.validator.deciders = deciders.iter().map(|d| d.to_string()).collect();
    cfg.validator.llm_fallback = llm_fallback;
    cfg
}

const DF: &str = "Here.\n```shellmate-command\ndf -h\n```";

#[tokio::test]
async fn a_confident_decider_finding_is_shown_and_the_llm_is_not_asked_again() {
    let d = FakeDecider::new(Ok(verdict(0.05, "read_only", 0.9)));
    let p = FakeProvider::replying(DF);
    let out = engine(vec![("a", p.clone(), 1)])
        .with_config(semantic_cfg(&["laya"], true))
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
        .with_config(semantic_cfg(&["laya"], true))
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
async fn a_failing_decider_falls_back_to_the_next_then_the_llm() {
    let down = FakeDecider::new(Err(shellmate_api::ProviderError::new(
        shellmate_api::ProviderErrorKind::Unavailable,
        "connection refused",
    )));
    let p = FakeProvider::with(vec![
        Ok(DF.to_string()),
        Ok("{\"match\": 0.02, \"risk\": \"read_only\"}".to_string()),
    ]);
    let out = engine(vec![("a", p.clone(), 1)])
        .with_config(semantic_cfg(&["laya"], true))
        .with_builtin_tools()
        .with_deciders(vec![spec("laya", down.clone(), Trust::Local)])
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
async fn with_no_decider_configured_the_llm_answers() {
    let p = FakeProvider::with(vec![
        Ok(DF.to_string()),
        Ok("{\"match\": 0.97, \"risk\": \"read_only\"}".to_string()),
    ]);
    let out = engine(vec![("a", p.clone(), 1)])
        .with_config(semantic_cfg(&[], true))
        .with_builtin_tools()
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
async fn without_fallback_and_without_a_decider_the_check_is_skipped_visibly() {
    let p = FakeProvider::replying(DF);
    let out = engine(vec![("a", p.clone(), 1)])
        .with_config(semantic_cfg(&[], false))
        .with_builtin_tools()
        .run(&env("ask", "show disk usage"))
        .await
        .unwrap();
    assert_eq!(p.seen.lock().unwrap().len(), 1);
    assert!(
        out.trailer_text()
            .contains("the semantic check was skipped"),
        "{}",
        out.trailer_text()
    );
    assert_eq!(out.buffer.as_deref(), Some("df -h"));
}

#[tokio::test]
async fn the_semantic_check_is_off_by_default() {
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
