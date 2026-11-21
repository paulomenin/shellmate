//! Tool dispatch: what is offered for each action, what a call to a tool nobody provides gets, and the
//! names the audit log records.

use super::*;
use crate::engine::tool_calls::ToolDecision;

#[tokio::test]
async fn an_unknown_tool_is_denied_and_runs_nothing() {
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)]);
    let run = e
        .run_tool("curl http://example.com", None, false, "`a`", true)
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

/// `man` is found by name and goes ahead: it is not refused as an unknown tool. (What `man` prints, or
/// whether it is installed, does not matter here.)
#[tokio::test]
async fn a_man_call_is_dispatched_to_the_man_tool() {
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)])
        .with_config(Config::defaults())
        .with_builtin_tools();
    let run = e
        .run_tool("man sm-no-such-page", None, false, "`a`", true)
        .await;
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
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)])
        .with_config(cfg.clone())
        .with_builtin_tools();
    crate::engine::tool_calls::offers(&e.plugged, action, &e.host)
        .iter()
        .map(|text| {
            if text.contains("manual page") {
                "man"
            } else if text.contains("help text of a program") {
                "help"
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
fn every_tool_is_offered_for_every_action_that_asks_something() {
    use crate::action::Action;
    let cfg = Config::defaults();
    for action in [Action::Ask, Action::Generate, Action::Explain] {
        assert_eq!(offered(&cfg, action), ["man", "help", "exec", "read"]);
    }
    // a new conversation asks nothing
    assert!(offered(&cfg, Action::NewConversation).is_empty());
}

#[test]
fn a_tool_that_is_off_or_cannot_ask_is_not_offered() {
    use crate::action::Action;
    let mut no_man = Config::defaults();
    man_off(&mut no_man);
    assert_eq!(offered(&no_man, Action::Ask), ["help", "exec", "read"]);
    let mut no_exec = Config::defaults();
    tool(&mut no_exec, "exec").enabled = false;
    assert_eq!(offered(&no_exec, Action::Ask), ["man", "help", "read"]);
    let mut exec_denied = Config::defaults();
    tool(&mut exec_denied, "exec").approval = Some(crate::config::ToolApproval::Deny);
    assert_eq!(offered(&exec_denied, Action::Ask), ["man", "help", "read"]);
    let mut read_denied = Config::defaults();
    tool(&mut read_denied, "read").approval = Some(crate::config::ToolApproval::Deny);
    assert_eq!(offered(&read_denied, Action::Ask), ["man", "help", "exec"]);
    let mut none = Config::defaults();
    man_off(&mut none);
    tool(&mut none, "help").enabled = false;
    tool(&mut none, "exec").enabled = false;
    tool(&mut none, "read").enabled = false;
    assert!(offered(&none, Action::Ask).is_empty());
}

#[test]
fn every_offer_names_the_block_the_model_must_use_and_calls_results_data() {
    let e = engine(vec![("a", FakeProvider::replying("x"), 1)])
        .with_config(Config::defaults())
        .with_builtin_tools();
    let all = crate::engine::tool_calls::offers(&e.plugged, crate::action::Action::Ask, &e.host);
    assert_eq!(all.len(), 4);
    for text in &all {
        assert!(text.contains(crate::response::TOOL_LABEL), "{text}");
        assert!(text.contains("reference data"), "{text}");
        assert!(text.ends_with('\n'), "each offer ends its own line");
    }
}

#[test]
fn man_is_offered_only_where_man_is_installed() {
    use crate::action::Action;
    let with_path = |path: &str| {
        let e = engine(vec![("a", FakeProvider::replying("x"), 1)])
            .with_host(Host::system().with_env(MapEnv::new().with("PATH", path)))
            .with_builtin_tools();
        crate::engine::tool_calls::offers(&e.plugged, Action::Ask, &e.host)
            .iter()
            .map(|t| t.contains("manual page"))
            .collect::<Vec<_>>()
    };
    // no `man` anywhere on PATH: only `help`, `exec` and `read` are offered, and the model is not told about man
    assert_eq!(with_path("/nonexistent"), [false, false, false]);
    // a relative PATH entry never counts
    assert_eq!(with_path("."), [false, false, false]);
    // with one, it comes first, as before
    let dir = std::env::temp_dir().join(format!("shellmate-man-offer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let man = dir.join("man");
    std::fs::write(&man, "#!/bin/sh\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&man, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        with_path(&dir.display().to_string()),
        [true, false, false, false]
    );
    // a file that is not executable is not a program
    std::fs::set_permissions(&man, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(with_path(&dir.display().to_string()), [false, false, false]);
    let _ = std::fs::remove_dir_all(&dir);
}
