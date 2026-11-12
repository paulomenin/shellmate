//! `[tools.<name>]` for tools that are not built in: the core keeps `enabled`, `approval` and
//! `preapproved`, the tool gets the rest, and a name nobody provides is an error that says so.

use super::*;
use shellmate_api as api;
use shellmate_api::{
    ApprovalMode, Assessment, Granted, Prepared, Registry, Tool, ToolAction, ToolCall as ApiCall,
    ToolCtx, ToolInfo, ToolNotice, ToolOutput, ToolSettings,
};

/// A tool that does nothing but say it ran.
struct Web {
    approval: ApprovalMode,
    /// The `approval` setting the core handed to each call.
    seen: Arc<Mutex<Vec<api::ApprovalSetting>>>,
}

#[async_trait]
impl Tool for Web {
    fn info(&self) -> ToolInfo {
        ToolInfo {
            name: "web",
            permissions: vec![api::Permission::Network],
            approval: self.approval,
            untrusted_output: true,
        }
    }
    fn offer(&self, _: ToolAction, _: &ToolSettings) -> Option<String> {
        Some("OFFER-web\n".into())
    }
    async fn assess(&self, call: &ApiCall, ctx: &ToolCtx<'_>) -> Assessment {
        self.seen.lock().unwrap().push(ctx.settings.approval);
        let prepared = Prepared::new(call.rest.clone());
        match self.approval {
            ApprovalMode::Never => Assessment::Allow(prepared),
            ApprovalMode::Required => Assessment::NeedsApproval(Box::new(api::ApprovalNeed {
                kind: api::ApprovalKind::Other("fetch".into()),
                subject: call.rest.clone(),
                cwd: None,
                risk: api::Risk::Low,
                reasons: vec![],
                preapprovable: true,
                rule_subject: vec![call.rest.clone()],
                prepared,
                denied: api::Refusal::denied("web", &call.rest, "denied by the person."),
                no_terminal: api::Refusal::denied("web", &call.rest, "nobody to ask."),
                to_buffer: None,
            })),
        }
    }
    fn rule_matches(&self, rule: &str, subject: &[String], _: &dyn api::ToolServices) -> bool {
        subject.iter().any(|s| s.starts_with(rule))
    }
    async fn run(&self, p: Prepared, _: &Granted, _: &ToolCtx<'_>) -> ToolOutput {
        ToolOutput {
            result: format!("fetched {}", p.downcast::<String>().unwrap()),
            notice: ToolNotice::new(api::Tone::Success, api::Mark::Check, "fetched"),
            untrusted: false,
        }
    }
}

/// A registry with a fake provider that makes one call to `web`, and the tool `web` that records the
/// settings it is built with.
fn registry(approval: ApprovalMode, built: Arc<Mutex<Vec<serde_json::Value>>>) -> Registry {
    registry_seen(approval, built, Arc::default())
}

fn registry_seen(
    approval: ApprovalMode,
    built: Arc<Mutex<Vec<serde_json::Value>>>,
    seen: Arc<Mutex<Vec<api::ApprovalSetting>>>,
) -> Registry {
    let mut r = Registry::new();
    r.register_provider(
        "fake",
        Box::new(|_| {
            Ok(FakeProvider::with(vec![
                Ok("```shellmate-tool\nweb https://example.org\n```".into()),
                Ok("Done.".into()),
            ]) as Arc<dyn Provider>)
        }),
    );
    r.register_tool(
        "web",
        Box::new(move |settings| {
            built.lock().unwrap().push(settings.clone());
            if settings.get("typo").is_some() {
                return Err(shellmate_api::Error::new(
                    "config.invalid",
                    "unknown key `typo`",
                ));
            }
            Ok(Arc::new(Web {
                approval,
                seen: seen.clone(),
            }))
        }),
    );
    r
}

fn config(tools: &str) -> Config {
    Config::parse(&format!(
        "version = 1\n[llm]\nchain = [\"p\"]\n[conversation]\nstore = \"none\"\n[audit]\nenabled = false\n\
         [pipeline]\nllm = []\n[providers.p]\nplugin = \"builtin:fake\"\ntrust = \"local\"\n\
         [providers.p.settings]\nmodel = \"m\"\n{tools}"
    ))
    .unwrap()
}

fn host() -> Host {
    Host::system().with_env(MapEnv::new().with("WEB_KEY", "k-123"))
}

async fn ask(e: &Engine) -> (Outcome, Vec<String>) {
    let mut notices = Vec::new();
    let o = e
        .run_streaming(&env("ask", "q"), &mut |emit| {
            if let Emit::Notice(n) = emit {
                notices.push(n.text.clone());
            }
        })
        .await
        .unwrap();
    (o, notices)
}

#[test]
fn the_core_keeps_three_keys_and_the_tool_gets_the_rest_with_secrets_resolved() {
    let built = Arc::default();
    let cfg = config(
        "[tools.web]\nenabled = true\napproval = \"ask\"\npreapproved = [\"https://example.org\"]\n\
         api_key = \"env:WEB_KEY\"\ndepth = 3\n",
    );
    let e = &cfg.tools.other["web"];
    assert_eq!(e.preapproved, ["https://example.org"]);
    Engine::from_config_in(
        &cfg,
        &registry(ApprovalMode::Required, Arc::clone(&built)),
        host(),
    )
    .unwrap();
    assert_eq!(
        built.lock().unwrap()[0],
        serde_json::json!({"api_key": "k-123", "depth": 3}),
        "no enabled, approval or preapproved, and the secret is resolved"
    );
}

#[test]
fn a_misspelt_key_of_the_core_is_a_config_error_and_a_misspelt_one_of_the_tool_is_its_error() {
    assert!(Config::parse("version = 1\n[tools.web]\napproval = \"maybe\"").is_err());
    assert!(Config::parse("version = 1\n[tools.web]\nenabled = \"yes\"").is_err());
    assert!(Config::parse("version = 1\n[tools]\nweb = 3").is_err());
    let cfg = config("[tools.web]\ntypo = 1\n");
    let e = Engine::from_config_in(
        &cfg,
        &registry(ApprovalMode::Required, Arc::default()),
        host(),
    )
    .err()
    .unwrap();
    assert_eq!(e.code, "config.invalid");
    assert_eq!(e.message, "tools.web: unknown key `typo`");
}

#[test]
fn a_tool_nobody_provides_is_an_error_that_lists_the_tools_there_are() {
    let cfg = config("[tools.wbe]\nenabled = true\n");
    let e = Engine::from_config_in(
        &cfg,
        &registry(ApprovalMode::Required, Arc::default()),
        host(),
    )
    .err()
    .unwrap();
    assert_eq!(e.code, "config.unknown_plugin");
    assert_eq!(e.message, "tools.wbe: no tool plugin `wbe`");
    assert_eq!(e.hint.as_deref(), Some("available tools: web"));
}

#[test]
fn allow_is_for_tools_that_need_no_approval_only() {
    let cfg = config("[tools.web]\napproval = \"allow\"\n");
    let e = Engine::from_config_in(
        &cfg,
        &registry(ApprovalMode::Required, Arc::default()),
        host(),
    )
    .err()
    .unwrap();
    assert_eq!(e.code, "config.invalid");
    assert!(e.message.contains("tools.web.approval"), "{}", e.message);
    Engine::from_config_in(&cfg, &registry(ApprovalMode::Never, Arc::default()), host()).unwrap();
}

#[test]
fn ask_is_for_tools_that_need_approval_only() {
    let cfg = config("[tools.web]\napproval = \"ask\"\n");
    let e = Engine::from_config_in(&cfg, &registry(ApprovalMode::Never, Arc::default()), host())
        .err()
        .unwrap();
    assert_eq!(e.code, "config.invalid");
    assert!(e.message.contains("tools.web.approval"), "{}", e.message);
    Engine::from_config_in(
        &cfg,
        &registry(ApprovalMode::Required, Arc::default()),
        host(),
    )
    .unwrap();
}

#[tokio::test]
async fn a_tool_from_the_config_is_offered_and_asks_by_default() {
    let cfg = config("[tools.web]\n");
    let e = Engine::from_config_in(
        &cfg,
        &registry(ApprovalMode::Required, Arc::default()),
        host(),
    )
    .unwrap();
    let approver = FakeApprover::new(Choice::Run);
    let e = e.with_approver(approver.clone());
    let (o, notices) = ask(&e).await;
    assert_eq!(o.text, "Done.");
    assert_eq!(notices, ["fetched"]);
    assert_eq!(approver.count(), 1, "approval defaults to ask");
    assert_eq!(
        approver.asked.lock().unwrap()[0].kind,
        api::ApprovalKind::Other("fetch".into())
    );
}

#[tokio::test]
async fn a_tool_that_needs_no_approval_defaults_to_not_asking() {
    let cfg = config("[tools.web]\n");
    let e = Engine::from_config_in(&cfg, &registry(ApprovalMode::Never, Arc::default()), host())
        .unwrap();
    let approver = FakeApprover::new(Choice::Deny);
    let e = e.with_approver(approver.clone());
    let (_, notices) = ask(&e).await;
    assert_eq!(notices, ["fetched"]);
    assert_eq!(approver.count(), 0);
}

#[tokio::test]
async fn preapproved_rules_from_the_config_apply_to_the_tool_that_reads_them() {
    let cfg = config("[tools.web]\npreapproved = [\"https://example\"]\n");
    let e = Engine::from_config_in(
        &cfg,
        &registry(ApprovalMode::Required, Arc::default()),
        host(),
    )
    .unwrap();
    let approver = FakeApprover::new(Choice::Deny);
    let e = e.with_approver(approver.clone());
    let (_, notices) = ask(&e).await;
    assert_eq!(notices, ["fetched"], "covered by the rule");
    assert_eq!(approver.count(), 0);
}

#[tokio::test]
async fn a_tool_that_is_off_in_the_config_is_not_offered_and_refuses() {
    for off in ["enabled = false", "approval = \"deny\""] {
        let cfg = config(&format!("[tools.web]\n{off}\n"));
        let e =
            Engine::from_config_in(&cfg, &registry(ApprovalMode::Never, Arc::default()), host())
                .unwrap();
        let (_, notices) = ask(&e).await;
        assert_eq!(notices, ["denied: web https://example.org"], "{off}");
    }
}

#[tokio::test]
async fn the_approval_a_tool_sees_is_the_one_configured_or_its_default() {
    for (tools, mode, want) in [
        (
            "[tools.web]\n",
            ApprovalMode::Required,
            api::ApprovalSetting::Ask,
        ),
        (
            "[tools.web]\n",
            ApprovalMode::Never,
            api::ApprovalSetting::Allow,
        ),
        (
            "[tools.web]\napproval = \"ask\"\n",
            ApprovalMode::Required,
            api::ApprovalSetting::Ask,
        ),
        (
            "[tools.web]\napproval = \"allow\"\n",
            ApprovalMode::Never,
            api::ApprovalSetting::Allow,
        ),
    ] {
        let seen = Arc::default();
        let e = Engine::from_config_in(
            &config(tools),
            &registry_seen(mode, Arc::default(), Arc::clone(&seen)),
            host(),
        )
        .unwrap()
        .with_approver(FakeApprover::new(Choice::Run));
        ask(&e).await;
        assert_eq!(*seen.lock().unwrap(), [want], "{tools} {mode:?}");
    }
}
