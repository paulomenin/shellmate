//! The core's side of the tool API, with tools that misbehave.
//!
//! A tool describes and runs; the core enforces. These tests give the core a tool that tries to skip the
//! approval, to be pre-approved when it must not be, to hide that its output is untrusted, or to write
//! escape sequences, and check that none of it works (INV-1, INV-4). They go through `run_streaming`.

use super::*;
use crate::audit::MemoryAudit;
use crate::present::Tone;
use shellmate_api as api;
use shellmate_api::{
    ApprovalKind, ApprovalMode, ApprovalNeed, Assessment, Granted, Mark, Prepared, Refusal, Risk,
    ToBuffer, Tool, ToolAction, ToolCall as ApiCall, ToolCtx, ToolInfo, ToolNotice, ToolOutput,
    ToolSettings,
};

#[derive(Clone, Copy)]
enum Mode {
    /// Return `Allow`.
    Allow,
    /// Return `NeedsApproval`.
    Need {
        preapprovable: bool,
        to_buffer: bool,
    },
    Refuse,
}

/// A tool whose answers the test chooses. It records what reached `run`.
struct Probe {
    name: &'static str,
    approval: ApprovalMode,
    untrusted_output: bool,
    mode: Mode,
    /// What `rule_matches` answers, whatever the rule.
    rules_match: bool,
    /// Whether `run` says its output is untrusted.
    says_untrusted: bool,
    /// Whether `run` says the call failed after it was allowed.
    fails: bool,
    offered_for: Vec<ToolAction>,
    ran: Mutex<Vec<(String, Granted)>>,
    notice: ToolNotice,
}

impl Probe {
    fn new(name: &'static str, approval: ApprovalMode, mode: Mode) -> Self {
        Self {
            name,
            approval,
            untrusted_output: false,
            mode,
            rules_match: true,
            says_untrusted: false,
            fails: false,
            offered_for: vec![ToolAction::Ask],
            ran: Mutex::new(Vec::new()),
            notice: ToolNotice::new(Tone_::Success, Mark::Check, "did it"),
        }
    }

    fn ran(&self) -> Vec<(String, Granted)> {
        self.ran.lock().unwrap().clone()
    }
}

// the API's tone, under a name that does not clash with the presenter's
use shellmate_api::Tone as Tone_;

#[async_trait]
impl Tool for Probe {
    fn info(&self) -> ToolInfo {
        ToolInfo {
            name: self.name,
            permissions: vec![],
            approval: self.approval,
            untrusted_output: self.untrusted_output,
        }
    }

    fn offer(&self, action: ToolAction, _: &ToolSettings) -> Option<String> {
        self.offered_for
            .contains(&action)
            .then(|| format!("OFFER-{}\n", self.name))
    }

    async fn assess(&self, call: &ApiCall, _: &ToolCtx<'_>) -> Assessment {
        let prepared = Prepared::new(format!("payload:{}", call.rest));
        match self.mode {
            Mode::Allow => Assessment::Allow(prepared),
            Mode::Refuse => Assessment::Refuse(Refusal::denied(self.name, &call.rest, "no.")),
            Mode::Need {
                preapprovable,
                to_buffer,
            } => Assessment::NeedsApproval(Box::new(ApprovalNeed {
                kind: ApprovalKind::Other("probe".into()),
                subject: call.rest.clone(),
                cwd: None,
                risk: Risk::Medium,
                reasons: vec!["because".into()],
                preapprovable,
                rule_subject: vec![call.rest.clone()],
                prepared,
                denied: Refusal::denied(self.name, &call.rest, "denied by the person."),
                no_terminal: Refusal::denied(self.name, &call.rest, "nobody to ask."),
                to_buffer: to_buffer.then(|| ToBuffer {
                    refusal: Refusal::new(
                        ToolNotice::new(Tone_::Info, Mark::Arrow, "moved"),
                        "moved to the prompt",
                    ),
                    buffer: format!("probe {}", call.rest),
                }),
            })),
        }
    }

    fn rule_matches(&self, _: &str, _: &[String], _: &dyn api::ToolServices) -> bool {
        self.rules_match
    }

    async fn run(&self, prepared: Prepared, granted: &Granted, _: &ToolCtx<'_>) -> ToolOutput {
        let payload = prepared.downcast::<String>().expect("our own payload");
        self.ran.lock().unwrap().push((payload, granted.clone()));
        ToolOutput {
            result: "RESULT".into(),
            notice: self.notice.clone(),
            untrusted: self.says_untrusted,
            failed: self.fails,
        }
    }
}

fn settings(rules: &[&str]) -> ToolSettings {
    ToolSettings {
        enabled: true,
        approval: api::ApprovalSetting::Ask,
        preapproved: rules.iter().map(|s| s.to_string()).collect(),
    }
}

struct Seen {
    outcome: Outcome,
    notices: Vec<crate::present::Notice>,
    /// `(tool, decision)` of each `tool_call` audit event.
    audited: Vec<(String, String)>,
    system: String,
    asked: Vec<ApprovalRequest>,
    /// What the model was sent after each tool call.
    results: Vec<String>,
}

/// The model calls `calls` one per reply (`name args`), then answers.
async fn go(
    tools: Vec<(Arc<Probe>, ToolSettings)>,
    action: &str,
    calls: &[&str],
    approver: Option<Choice>,
) -> Seen {
    let mut replies: Vec<Result<String, ProviderError>> = calls
        .iter()
        .map(|c| Ok(format!("```shellmate-tool\n{c}\n```")))
        .collect();
    replies.push(Ok("Done.".into()));
    let provider = FakeProvider::with(replies);
    let sink = Arc::new(MemoryAudit::new());
    let fake = approver.map(FakeApprover::new);
    let mut e = engine(vec![("a", provider.clone(), 1)]).with_audit(sink.clone());
    for (t, s) in tools {
        e = e.with_tool(t, s);
    }
    if let Some(a) = &fake {
        e = e.with_approver(a.clone());
    }
    let mut notices = Vec::new();
    let outcome = e
        .run_streaming(&env(action, "q"), &mut |emit| {
            if let Emit::Notice(n) = emit {
                notices.push(n.clone());
            }
        })
        .await
        .unwrap();
    let seen = provider.seen.lock().unwrap();
    Seen {
        outcome,
        notices,
        audited: sink
            .events()
            .iter()
            .filter(|v| v["event"] == "tool_call")
            .map(|v| {
                (
                    v["tool"].as_str().unwrap().to_string(),
                    v["decision"].as_str().unwrap().to_string(),
                )
            })
            .collect(),
        system: seen[0].messages[0].content.clone(),
        asked: fake
            .map(|a| a.asked.lock().unwrap().clone())
            .unwrap_or_default(),
        results: seen
            .iter()
            .skip(1)
            .map(|r| r.messages.last().unwrap().content.clone())
            .collect(),
    }
}

fn dec(tool: &str, decision: &str) -> Vec<(String, String)> {
    vec![(tool.to_string(), decision.to_string())]
}

// ---- INV-1: a tool cannot go without the person ----------------------------------------------------------

#[tokio::test]
async fn a_tool_that_needs_approval_cannot_allow_itself() {
    let t = Arc::new(Probe::new("probe", ApprovalMode::Required, Mode::Allow));
    let s = go(
        vec![(t.clone(), settings(&[]))],
        "ask",
        &["probe go"],
        Some(Choice::Run),
    )
    .await;
    assert!(t.ran().is_empty(), "run was not reached");
    assert_eq!(s.audited, dec("probe", "deny"));
    assert!(s.asked.is_empty());
    assert!(s.results[0].contains("Tool call `probe go` was denied"));
}

#[tokio::test]
async fn a_tool_that_declares_no_approval_runs_without_asking() {
    let t = Arc::new(Probe::new("probe", ApprovalMode::Never, Mode::Allow));
    let s = go(
        vec![(t.clone(), settings(&[]))],
        "ask",
        &["probe go"],
        Some(Choice::Deny),
    )
    .await;
    assert_eq!(t.ran(), [("payload:go".to_string(), Granted::NotRequired)]);
    assert_eq!(s.audited, dec("probe", "allow"));
    assert!(s.asked.is_empty());
    assert_eq!(s.results[0], "RESULT");
}

#[tokio::test]
async fn nothing_runs_when_the_person_says_no_or_nobody_can_be_asked() {
    for (choice, decision) in [
        (Some(Choice::Deny), "denied_by_user"),
        (None, "denied_no_terminal"),
    ] {
        let t = Arc::new(Probe::new(
            "probe",
            ApprovalMode::Required,
            Mode::Need {
                preapprovable: false,
                to_buffer: false,
            },
        ));
        let s = go(
            vec![(t.clone(), settings(&[]))],
            "ask",
            &["probe go"],
            choice,
        )
        .await;
        assert!(t.ran().is_empty(), "{decision}");
        assert_eq!(s.audited, dec("probe", decision));
        assert_eq!(
            s.asked.len(),
            usize::from(choice.is_some()),
            "{decision}: asked only when someone can answer"
        );
    }
}

#[tokio::test]
async fn the_person_says_yes_and_the_tool_runs_what_was_shown() {
    let t = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: false,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![(t.clone(), settings(&[]))],
        "ask",
        &["probe go"],
        Some(Choice::Run),
    )
    .await;
    assert_eq!(t.ran(), [("payload:go".to_string(), Granted::Person)]);
    assert_eq!(s.audited, dec("probe", "approved"));
    let a = &s.asked[0];
    assert_eq!(a.kind, ApprovalKind::Other("probe".into()));
    assert_eq!((a.command.as_str(), a.risk), ("go", Risk::Medium));
    assert_eq!(a.reasons, ["because"]);
}

#[tokio::test]
async fn taking_it_to_the_prompt_leaves_the_text_in_the_buffer_and_runs_nothing() {
    let t = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: false,
            to_buffer: true,
        },
    ));
    let s = go(
        vec![(t.clone(), settings(&[]))],
        "ask",
        &["probe go"],
        Some(Choice::ToBuffer),
    )
    .await;
    assert!(t.ran().is_empty());
    assert_eq!(s.audited, dec("probe", "to_buffer"));
    assert_eq!(s.outcome.buffer.as_deref(), Some("probe go"));
    // a tool with no such thing treats it as a no
    let t = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: false,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![(t.clone(), settings(&[]))],
        "ask",
        &["probe go"],
        Some(Choice::ToBuffer),
    )
    .await;
    assert!(t.ran().is_empty());
    assert_eq!(s.audited, dec("probe", "denied_by_user"));
    assert_eq!(s.outcome.buffer, None);
}

// ---- the user's rules, and the classifier's veto ---------------------------------------------------------

#[tokio::test]
async fn a_rule_covers_a_call_the_tool_marks_preapprovable() {
    let t = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: true,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![(t.clone(), settings(&["go*"]))],
        "ask",
        &["probe go"],
        Some(Choice::Deny),
    )
    .await;
    assert_eq!(
        t.ran(),
        [("payload:go".to_string(), Granted::Rule("go*".into()))]
    );
    assert_eq!(s.audited, dec("probe", "preapproved"));
    assert!(s.asked.is_empty());
}

#[tokio::test]
async fn a_rule_never_covers_a_call_the_tool_does_not_mark_preapprovable() {
    // the rule matches everything, and the tool's matcher says yes, but the classifier's veto stands
    let t = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: false,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![(t.clone(), settings(&["*"]))],
        "ask",
        &["probe go"],
        Some(Choice::Deny),
    )
    .await;
    assert!(t.ran().is_empty());
    assert_eq!(s.asked.len(), 1, "the person was asked");
    assert_eq!(s.audited, dec("probe", "denied_by_user"));
}

#[tokio::test]
async fn a_call_no_rule_matches_asks() {
    let mut p = Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: true,
            to_buffer: false,
        },
    );
    p.rules_match = false;
    let t = Arc::new(p);
    let s = go(
        vec![(t.clone(), settings(&["x"]))],
        "ask",
        &["probe go"],
        Some(Choice::Run),
    )
    .await;
    assert_eq!(s.asked.len(), 1);
    assert_eq!(t.ran(), [("payload:go".to_string(), Granted::Person)]);
}

// ---- INV-4: taint ----------------------------------------------------------------------------------------

#[tokio::test]
async fn a_tool_that_declares_untrusted_output_taints_even_if_it_says_it_does_not() {
    let mut dirty = Probe::new("dirty", ApprovalMode::Never, Mode::Allow);
    dirty.untrusted_output = true;
    dirty.says_untrusted = false;
    let dirty = Arc::new(dirty);
    let probe = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: true,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![(dirty, settings(&[])), (probe.clone(), settings(&["*"]))],
        "ask",
        &["dirty x", "probe go"],
        Some(Choice::Run),
    )
    .await;
    assert_eq!(
        s.asked.len(),
        1,
        "the rule was suspended after untrusted output"
    );
    assert_eq!(probe.ran(), [("payload:go".to_string(), Granted::Person)]);
}

#[tokio::test]
async fn output_a_tool_marks_untrusted_taints_too() {
    let mut dirty = Probe::new("dirty", ApprovalMode::Never, Mode::Allow);
    dirty.says_untrusted = true;
    let probe = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: true,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![
            (Arc::new(dirty), settings(&[])),
            (probe.clone(), settings(&["*"])),
        ],
        "ask",
        &["dirty x", "probe go"],
        Some(Choice::Run),
    )
    .await;
    assert_eq!(s.asked.len(), 1);
}

#[tokio::test]
async fn a_trusted_tool_does_not_taint_so_a_rule_still_applies_after_it() {
    let clean = Arc::new(Probe::new("clean", ApprovalMode::Never, Mode::Allow));
    let probe = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: true,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![(clean, settings(&[])), (probe.clone(), settings(&["*"]))],
        "ask",
        &["clean x", "probe go"],
        Some(Choice::Deny),
    )
    .await;
    assert!(s.asked.is_empty());
    assert_eq!(
        s.audited[1],
        (
            "probe go".split(' ').next().unwrap().to_string(),
            "preapproved".into()
        )
    );
}

// ---- off, offered, refused -------------------------------------------------------------------------------

#[tokio::test]
async fn a_tool_that_is_off_is_not_offered_and_does_not_run() {
    for off in [
        ToolSettings {
            enabled: false,
            ..settings(&[])
        },
        ToolSettings {
            approval: api::ApprovalSetting::Deny,
            ..settings(&[])
        },
    ] {
        let t = Arc::new(Probe::new("probe", ApprovalMode::Never, Mode::Allow));
        let s = go(
            vec![(t.clone(), off)],
            "ask",
            &["probe go"],
            Some(Choice::Run),
        )
        .await;
        assert!(!s.system.contains("OFFER-probe"));
        assert!(t.ran().is_empty());
        assert_eq!(s.audited, dec("probe", "deny"));
        assert!(
            s.results[0].contains("the probe tool is disabled."),
            "{}",
            s.results[0]
        );
    }
}

#[tokio::test]
async fn a_tool_is_offered_only_for_the_actions_it_names() {
    let t = Arc::new(Probe::new("probe", ApprovalMode::Never, Mode::Allow));
    let ask = go(vec![(t.clone(), settings(&[]))], "ask", &[], None).await;
    assert!(ask.system.contains("OFFER-probe\n"));
    let generate = go(vec![(t, settings(&[]))], "generate", &[], None).await;
    assert!(!generate.system.contains("OFFER-probe"));
}

#[tokio::test]
async fn a_refusal_is_shown_and_told_to_the_model() {
    let t = Arc::new(Probe::new("probe", ApprovalMode::Required, Mode::Refuse));
    let s = go(
        vec![(t.clone(), settings(&[]))],
        "ask",
        &["probe go"],
        Some(Choice::Run),
    )
    .await;
    assert!(t.ran().is_empty());
    assert_eq!(s.audited, dec("probe", "deny"));
    assert_eq!(s.results[0], "Tool call `probe go` was denied: no.");
    assert_eq!(s.notices[0].text, "denied: probe go");
}

// ---- what a tool writes is escaped -----------------------------------------------------------------------

#[tokio::test]
async fn text_a_tool_writes_for_the_terminal_is_escaped() {
    let mut p = Probe::new("probe", ApprovalMode::Never, Mode::Allow);
    p.notice = ToolNotice::new(Tone_::Success, Mark::Check, "a\u{1b}[31mred")
        .with_meta("m\u{7}")
        .with_detail(vec!["d\u{1b}]0;title\u{7}".into()]);
    let t = Arc::new(p);
    let s = go(vec![(t, settings(&[]))], "ask", &["probe go"], None).await;
    let n = &s.notices[0];
    assert_eq!(n.tone, Tone::Success);
    for text in std::iter::once(n.text.as_str())
        .chain(n.meta.as_deref())
        .chain(n.detail.iter().map(String::as_str))
    {
        assert!(!text.chars().any(char::is_control), "{text:?}");
    }
    assert_eq!(n.text, "a\\u{1b}[31mred");
}

#[tokio::test]
async fn a_call_that_fails_after_the_yes_is_audited_as_a_refusal_and_still_taints() {
    let mut failing = Probe::new("failing", ApprovalMode::Never, Mode::Allow);
    failing.fails = true;
    failing.untrusted_output = true;
    let failing = Arc::new(failing);
    let probe = Arc::new(Probe::new(
        "probe",
        ApprovalMode::Required,
        Mode::Need {
            preapprovable: true,
            to_buffer: false,
        },
    ));
    let s = go(
        vec![
            (failing.clone(), settings(&[])),
            (probe.clone(), settings(&["*"])),
        ],
        "ask",
        &["failing x", "probe go"],
        Some(Choice::Run),
    )
    .await;
    assert_eq!(failing.ran().len(), 1, "it did run");
    assert_eq!(s.audited[0], ("failing".to_string(), "deny".to_string()));
    assert_eq!(s.asked.len(), 1, "a tool cannot dodge taint by failing");
    assert_eq!(probe.ran(), [("payload:go".to_string(), Granted::Person)]);
}
