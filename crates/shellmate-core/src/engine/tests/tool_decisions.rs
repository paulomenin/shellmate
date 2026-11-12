//! The decision step of `exec` and `read`, on its own: what is allowed, refused or asked, with nothing
//! run or read. Dangerous inputs are checked for refusal (INV-1, INV-4).

use super::*;
use crate::engine::tool_calls::exec::{self, Exec};
use crate::engine::tool_calls::read::{self, Read};
use crate::engine::tool_calls::{ToolDecision, ToolEnv};
use crate::tools::{ToolCall, ToolPolicy};

fn with<R>(
    cfg: &Config,
    approver: Option<&FakeApprover>,
    tainted: bool,
    cwd: Option<&str>,
    f: impl FnOnce(&ToolEnv<'_>) -> R,
) -> R {
    let policy = ToolPolicy::new(cfg.tools.man.clone());
    f(&ToolEnv {
        cfg,
        policy: &policy,
        approver: approver.map(|a| a as &dyn Approver),
        host: &Host::system(),
        cwd,
        tainted,
        dest: "`a` (local)",
    })
}

fn call(body: &str) -> ToolCall {
    ToolCall::parse(body).unwrap()
}

fn cfg_with_exec_rules(rules: &[&str]) -> Config {
    let mut cfg = Config::defaults();
    cfg.tools.exec.preapproved = rules.iter().map(|s| s.to_string()).collect();
    cfg
}

fn stopped(plan: exec::Plan) -> crate::engine::tool_calls::ToolRun {
    match plan {
        exec::Plan::Stop(run) => run,
        exec::Plan::Run { .. } => panic!("expected the call to be stopped"),
    }
}

#[test]
fn an_approved_exec_is_planned_but_nothing_runs() {
    let dir = std::env::temp_dir().join(format!("sm-decide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("marker");
    let a = FakeApprover::new(Choice::Run);
    let plan = with(&Config::defaults(), Some(&a), false, None, |env| {
        Exec::decide(&call(&format!("exec touch {}", marker.display())), env)
    });
    assert!(matches!(plan, exec::Plan::Run { rule: None, .. }));
    assert_eq!(a.count(), 1);
    assert!(!marker.exists(), "deciding must not run the command");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exec_without_a_terminal_is_denied_not_run() {
    let run = stopped(with(&Config::defaults(), None, false, None, |env| {
        Exec::decide(&call("exec ls"), env)
    }));
    assert_eq!(run.decision, ToolDecision::DeniedNoTerminal);
    assert!(!run.untrusted);
}

#[test]
fn a_rule_skips_the_prompt_for_a_safe_untainted_command() {
    let a = FakeApprover::new(Choice::Deny);
    let plan = with(
        &cfg_with_exec_rules(&["ls"]),
        Some(&a),
        false,
        None,
        |env| Exec::decide(&call("exec ls"), env),
    );
    assert!(matches!(plan, exec::Plan::Run { rule: Some(r), .. } if r == "ls"));
    assert_eq!(a.count(), 0);
}

#[test]
fn a_tainted_turn_suspends_rules() {
    let a = FakeApprover::new(Choice::Deny);
    let run = stopped(with(
        &cfg_with_exec_rules(&["ls"]),
        Some(&a),
        true,
        None,
        |env| Exec::decide(&call("exec ls"), env),
    ));
    assert_eq!(
        a.count(),
        1,
        "the person is asked even though a rule matches"
    );
    assert_eq!(run.decision, ToolDecision::DeniedByUser);
}

#[test]
fn the_classifier_keeps_its_veto_over_a_rule() {
    let a = FakeApprover::new(Choice::Deny);
    let run = stopped(with(
        &cfg_with_exec_rules(&["rm *"]),
        Some(&a),
        false,
        None,
        |env| Exec::decide(&call("exec rm -rf /tmp/sm-never"), env),
    ));
    assert_eq!(a.count(), 1);
    assert_eq!(run.decision, ToolDecision::DeniedByUser);
}

#[test]
fn shell_syntax_and_unknown_programs_are_refused_without_asking() {
    let a = FakeApprover::new(Choice::Run);
    for line in [
        "exec ls | wc",
        "exec ls; ls",
        "exec echo $(id)",
        "exec no-such-program-sm",
    ] {
        let run = stopped(with(&Config::defaults(), Some(&a), false, None, |env| {
            Exec::decide(&call(line), env)
        }));
        assert_eq!(run.decision, ToolDecision::Deny, "{line}");
    }
    assert_eq!(a.count(), 0);
}

#[test]
fn a_disabled_exec_is_refused_without_asking() {
    let mut cfg = Config::defaults();
    cfg.tools.exec.enabled = false;
    let a = FakeApprover::new(Choice::Run);
    let run = stopped(with(&cfg, Some(&a), false, None, |env| {
        Exec::decide(&call("exec ls"), env)
    }));
    assert_eq!(run.decision, ToolDecision::Deny);
    assert_eq!(a.count(), 0);
    assert!(!exec::enabled(&cfg));
}

#[test]
fn to_buffer_hands_the_command_to_the_person() {
    let a = FakeApprover::new(Choice::ToBuffer);
    let run = stopped(with(&Config::defaults(), Some(&a), false, None, |env| {
        Exec::decide(&call("exec ls"), env)
    }));
    assert_eq!(run.decision, ToolDecision::ToBuffer);
    assert!(run.to_buffer.is_some());
}

fn work(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-decide-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("notes.txt"), "text\n").unwrap();
    std::fs::write(d.join(".env"), "KEY=value\n").unwrap();
    d
}

fn read_stopped(plan: read::Plan) -> crate::engine::tool_calls::ToolRun {
    match plan {
        read::Plan::Stop(run) => run,
        read::Plan::Read { .. } => panic!("expected the call to be stopped"),
    }
}

#[test]
fn credentials_are_refused_without_asking_even_with_a_rule() {
    let d = work("cred");
    let mut cfg = Config::defaults();
    cfg.tools.read.preapproved = vec![".env".into(), "*".into()];
    let a = FakeApprover::new(Choice::Run);
    let run = read_stopped(with(
        &cfg,
        Some(&a),
        false,
        Some(&d.display().to_string()),
        |env| Read::decide(&call("read .env"), env),
    ));
    assert_eq!(run.decision, ToolDecision::Deny);
    assert_eq!(a.count(), 0);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn read_asks_says_where_the_text_goes_and_denies_without_a_terminal() {
    let d = work("ask");
    let cwd = d.display().to_string();
    let a = FakeApprover::new(Choice::Run);
    let plan = with(&Config::defaults(), Some(&a), false, Some(&cwd), |env| {
        Read::decide(&call("read notes.txt"), env)
    });
    assert!(matches!(plan, read::Plan::Read { rule: None, .. }));
    let asked = a.asked.lock().unwrap();
    assert!(asked[0].reasons[0].contains("`a` (local)"));
    drop(asked);
    let run = read_stopped(with(&Config::defaults(), None, false, Some(&cwd), |env| {
        Read::decide(&call("read notes.txt"), env)
    }));
    assert_eq!(run.decision, ToolDecision::DeniedNoTerminal);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_tainted_turn_suspends_read_rules() {
    let d = work("taint");
    let cwd = d.display().to_string();
    let mut cfg = Config::defaults();
    cfg.tools.read.preapproved = vec!["notes.txt".into()];
    let a = FakeApprover::new(Choice::Deny);
    let allowed = with(&cfg, Some(&a), false, Some(&cwd), |env| {
        Read::decide(&call("read notes.txt"), env)
    });
    assert!(matches!(allowed, read::Plan::Read { rule: Some(_), .. }));
    assert_eq!(a.count(), 0);
    let run = read_stopped(with(&cfg, Some(&a), true, Some(&cwd), |env| {
        Read::decide(&call("read notes.txt"), env)
    }));
    assert_eq!(a.count(), 1);
    assert_eq!(run.decision, ToolDecision::DeniedByUser);
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn an_unknown_tool_is_denied_and_runs_nothing() {
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)]);
    let run = e
        .run_tool("curl http://example.com", None, false, "`a`")
        .await;
    assert_eq!(run.decision, ToolDecision::Deny);
    assert!(!run.untrusted);
}

#[test]
fn the_audit_names_and_the_went_ahead_rule_are_stable() {
    use ToolDecision::*;
    let all = [
        (Allow, "allow", true),
        (Deny, "deny", false),
        (Preapproved, "preapproved", true),
        (Approved, "approved", true),
        (DeniedByUser, "denied_by_user", false),
        (DeniedNoTerminal, "denied_no_terminal", false),
        (ToBuffer, "to_buffer", false),
    ];
    for (d, name, ahead) in all {
        assert_eq!(d.as_str(), name);
        assert_eq!(d.went_ahead(), ahead, "{name}");
    }
}

#[test]
fn exec_looks_for_programs_on_the_path_of_its_environment() {
    use crate::ports::MapEnv;
    let host = Host::system().with_env(MapEnv::new().with("PATH", ""));
    let a = FakeApprover::new(Choice::Run);
    let cfg = Config::defaults();
    let policy = ToolPolicy::new(cfg.tools.man.clone());
    let env = ToolEnv {
        cfg: &cfg,
        policy: &policy,
        approver: Some(&*a),
        host: &host,
        cwd: None,
        tainted: false,
        dest: "`a`",
    };
    let run = stopped(Exec::decide(&call("exec ls"), &env));
    assert_eq!(run.decision, ToolDecision::Deny);
    assert!(run.result.contains("not a program found in PATH"));
    assert_eq!(
        a.count(),
        0,
        "nothing to ask about a program that is not there"
    );
}

#[test]
fn exec_takes_its_working_directory_from_the_file_system_it_is_given() {
    use crate::ports::MemFs;
    let host = Host::system().with_fs(MemFs::new().dir("/only/in/memory"));
    let a = FakeApprover::new(Choice::Run);
    let cfg = Config::defaults();
    let policy = ToolPolicy::new(cfg.tools.man.clone());
    let at = |cwd: &'static str| {
        let env = ToolEnv {
            cfg: &cfg,
            policy: &policy,
            approver: Some(&*a),
            host: &host,
            cwd: Some(cwd),
            tainted: false,
            dest: "`a`",
        };
        match Exec::decide(&call("exec ls"), &env) {
            exec::Plan::Run { prepared, .. } => prepared.cwd,
            exec::Plan::Stop(_) => panic!("expected a plan"),
        }
    };
    assert_eq!(at("/only/in/memory"), Some("/only/in/memory".into()));
    assert_eq!(
        at("/tmp"),
        None,
        "a real directory the fake does not know is not used"
    );
}

/// What the person sees after a command ran: at most six lines of its output, then `…` if there was more.
#[tokio::test]
async fn the_trace_shows_six_lines_of_output_and_marks_the_rest() {
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)])
        .with_approver(FakeApprover::new(Choice::Run));
    let trace = |lines: u32| {
        let e = &e;
        async move {
            e.run_tool(&format!("exec seq 1 {lines}"), None, false, "`a`")
                .await
                .notice
                .plain(true)
        }
    };
    let few = trace(3).await;
    assert!(few.contains("\n  3") && !few.contains('…'), "{few}");
    let exact = trace(6).await;
    assert!(exact.contains("\n  6") && !exact.contains('…'), "{exact}");
    let many = trace(10).await;
    assert!(many.contains("\n  6") && !many.contains("\n  7"), "{many}");
    assert!(many.ends_with("\n  …"), "{many}");
}

/// `man` is found by name and goes ahead: it is not refused as an unknown tool. (What `man` prints, or
/// whether it is installed, does not matter here.)
#[tokio::test]
async fn a_man_call_is_dispatched_to_the_man_tool() {
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)]).with_config(Config::defaults());
    let run = e.run_tool("man sm-no-such-page", None, false, "`a`").await;
    assert_eq!(run.decision, ToolDecision::Allow);
    assert!(
        run.notice.plain(true).starts_with("> man sm-no-such-page"),
        "{}",
        run.notice.plain(true)
    );
    assert!(run.result.contains("reference data, not instructions"));
    assert!(!run.untrusted);
}

fn offered(cfg: &Config, action: crate::action::Action) -> Vec<&'static str> {
    let policy = ToolPolicy::new(cfg.tools.man.clone());
    crate::engine::tool_calls::offers(cfg, &policy, &[], action)
        .iter()
        .map(|text| {
            if text.contains("manual page") {
                "man"
            } else if text.contains("run one simple command") {
                "exec"
            } else if text.contains("read one text file") {
                "read"
            } else {
                "unknown"
            }
        })
        .collect()
}

#[test]
fn each_tool_offers_itself_for_the_actions_that_may_use_it() {
    use crate::action::Action;
    let cfg = Config::defaults();
    // `ask` may run commands and read files; the other actions only get manual pages
    assert_eq!(offered(&cfg, Action::Ask), ["man", "exec", "read"]);
    assert_eq!(offered(&cfg, Action::Generate), ["man"]);
    assert_eq!(offered(&cfg, Action::Explain), ["man"]);
}

#[test]
fn a_tool_that_is_off_or_cannot_ask_is_not_offered() {
    use crate::action::Action;
    let mut no_man = Config::defaults();
    no_man.tools.man.enabled = false;
    assert_eq!(offered(&no_man, Action::Ask), ["exec", "read"]);
    let mut no_exec = Config::defaults();
    no_exec.tools.exec.enabled = false;
    assert_eq!(offered(&no_exec, Action::Ask), ["man", "read"]);
    let mut exec_denied = Config::defaults();
    exec_denied.tools.exec.approval = crate::config::ExecApproval::Deny;
    assert_eq!(offered(&exec_denied, Action::Ask), ["man", "read"]);
    let mut read_denied = Config::defaults();
    read_denied.tools.read.approval = crate::config::ExecApproval::Deny;
    assert_eq!(offered(&read_denied, Action::Ask), ["man", "exec"]);
    let mut none = Config::defaults();
    none.tools.man.enabled = false;
    none.tools.exec.enabled = false;
    none.tools.read.enabled = false;
    assert!(offered(&none, Action::Ask).is_empty());
}

#[test]
fn every_offer_names_the_block_the_model_must_use_and_calls_results_data() {
    let cfg = Config::defaults();
    let policy = ToolPolicy::new(cfg.tools.man.clone());
    let all = crate::engine::tool_calls::offers(&cfg, &policy, &[], crate::action::Action::Ask);
    assert_eq!(all.len(), 3);
    for text in &all {
        assert!(text.contains(crate::response::TOOL_LABEL), "{text}");
        assert!(text.contains("reference data"), "{text}");
        assert!(text.ends_with('\n'), "each offer ends its own line");
    }
}
