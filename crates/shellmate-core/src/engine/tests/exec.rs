use super::*;

/// The model asks to run `cmd`, then answers. Returns the outcome and the requests the model saw.
async fn run_exec(
    cmd: &str,
    approver: Option<Arc<FakeApprover>>,
    preapproved: &[&str],
) -> (Outcome, Arc<FakeProvider>) {
    let p = FakeProvider::with(vec![
        Ok(exec_call(cmd)),
        Ok("Done.\n```shellmate-command\nls\n```".to_string()),
    ]);
    let mut cfg = Config::defaults();
    cfg.tools.exec.preapproved = preapproved.iter().map(|s| s.to_string()).collect();
    let mut e = engine(vec![("a", p.clone(), 1)]).with_config(cfg);
    if let Some(a) = approver {
        e = e.with_approver(a);
    }
    let out = e.run(&env("ask", "look")).await.unwrap();
    (out, p)
}

#[tokio::test]
async fn an_approved_command_runs_and_its_output_reaches_the_model() {
    let a = FakeApprover::new(Choice::Run);
    let (out, p) = run_exec("echo hello-from-exec", Some(a.clone()), &[]).await;
    assert_eq!(a.count(), 1);
    let asked = a.asked.lock().unwrap()[0].clone();
    assert_eq!(asked.command, "echo hello-from-exec");
    assert_eq!(asked.risk, crate::command::Risk::Low);
    let r = second_request(&p);
    assert!(
        r.contains("hello-from-exec") && r.contains("exit status 0"),
        "{r}"
    );
    assert!(r.contains("not instructions"));
    assert_eq!(out.report.tools, ["exec echo hello-from-exec"]);
}

#[tokio::test]
async fn a_denied_command_never_runs() {
    let dir = std::env::temp_dir().join(format!("sm-exec-deny-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let a = FakeApprover::new(Choice::Deny);
    let (out, p) = run_exec(&format!("mkdir {}", dir.display()), Some(a.clone()), &[]).await;
    assert_eq!(a.count(), 1);
    assert!(!dir.exists(), "denied, so it must not have run");
    assert!(second_request(&p).contains("denied by the user"));
    assert!(out.report.tools[0].starts_with("denied: "));
}

#[tokio::test]
async fn without_an_approver_nothing_runs() {
    let dir = std::env::temp_dir().join(format!("sm-exec-noapp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (_, p) = run_exec(&format!("mkdir {}", dir.display()), None, &[]).await;
    assert!(!dir.exists());
    assert!(second_request(&p).contains("no terminal"));
}

#[tokio::test]
async fn a_user_rule_runs_a_safe_command_without_asking() {
    let a = FakeApprover::new(Choice::Deny);
    let (_, p) = run_exec("echo covered", Some(a.clone()), &["echo *"]).await;
    assert_eq!(a.count(), 0, "pre-approved by the user's rule");
    assert!(second_request(&p).contains("covered"));
}

#[tokio::test]
async fn a_tainted_turn_suspends_user_rules() {
    // INV-4: a turn carrying untrusted content (captured output) must not run anything unasked
    let mut cfg = Config::defaults();
    cfg.tools.exec.preapproved = vec!["echo *".into()];
    for (tainted, asked) in [(false, 0), (true, 1)] {
        let a = FakeApprover::new(Choice::Deny);
        let e = engine(vec![("a", FakeProvider::replying("x"), 1)])
            .with_config(cfg.clone())
            .with_approver(a.clone());
        let run = e
            .run_tool("exec echo hi", None, tainted, "`a` (local)")
            .await;
        assert_eq!(a.count(), asked, "tainted={tainted}");
        assert_eq!(
            run.decision,
            if tainted {
                crate::engine::tool_calls::ToolDecision::DeniedByUser
            } else {
                crate::engine::tool_calls::ToolDecision::Preapproved
            }
        );
    }
}

#[tokio::test]
async fn the_classifier_overrules_a_user_rule() {
    let dir = std::env::temp_dir().join(format!("sm-exec-veto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("keep.txt");
    std::fs::write(&file, "x").unwrap();
    let a = FakeApprover::new(Choice::Deny);
    let (_, _) = run_exec(
        &format!("rm {}", file.display()),
        Some(a.clone()),
        &["rm *"],
    )
    .await;
    assert_eq!(a.count(), 1, "rm is flagged, so the rule does not apply");
    assert_eq!(a.asked.lock().unwrap()[0].risk, crate::command::Risk::High);
    assert!(file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn complex_commands_are_refused_without_asking() {
    for bad in [
        "ls | wc -l",
        "echo $HOME",
        "ls > out",
        "rm -rf *",
        "cd /tmp",
    ] {
        let a = FakeApprover::new(Choice::Run);
        let (_, p) = run_exec(bad, Some(a.clone()), &[]).await;
        assert_eq!(a.count(), 0, "{bad}");
        assert!(second_request(&p).contains("only simple commands"), "{bad}");
    }
}

#[tokio::test]
async fn unknown_programs_are_refused() {
    let a = FakeApprover::new(Choice::Run);
    let (_, p) = run_exec("definitely-not-a-program", Some(a.clone()), &[]).await;
    assert_eq!(a.count(), 0);
    assert!(second_request(&p).contains("not a program found in PATH"));
    let (_, p) = run_exec("./sh", Some(FakeApprover::new(Choice::Run)), &[]).await;
    assert!(second_request(&p).contains("not a program found"));
}

#[tokio::test]
async fn to_buffer_places_the_command_in_the_input_line() {
    let a = FakeApprover::new(Choice::ToBuffer);
    let p = FakeProvider::with(vec![
        Ok(exec_call("git status")),
        Ok("Run it yourself.".to_string()),
    ]);
    let out = engine(vec![("a", p, 1)])
        .with_approver(a)
        .run(&env("ask", "look"))
        .await
        .unwrap();
    assert_eq!(out.buffer.as_deref(), Some("git status"));
}

#[tokio::test]
async fn exec_can_be_turned_off_and_is_only_offered_to_ask() {
    let mut cfg = Config::defaults();
    cfg.tools.exec.approval = crate::config::ExecApproval::Deny;
    let a = FakeApprover::new(Choice::Run);
    let p = FakeProvider::with(vec![Ok(exec_call("echo x")), Ok("ok".to_string())]);
    let _ = engine(vec![("a", p.clone(), 1)])
        .with_config(cfg)
        .with_approver(a.clone())
        .run(&env("ask", "look"))
        .await
        .unwrap();
    assert_eq!(a.count(), 0);
    assert!(!p.seen.lock().unwrap()[0].messages[0]
        .content
        .contains("exec <command>"));

    let p = FakeProvider::replying("```shellmate-command\nls\n```");
    let _ = engine(vec![("a", p.clone(), 1)])
        .run(&env("generate", "list"))
        .await
        .unwrap();
    assert!(!p.seen.lock().unwrap()[0].messages[0]
        .content
        .contains("exec <command>"));
    let p = FakeProvider::replying("hi");
    let _ = engine(vec![("a", p.clone(), 1)])
        .run(&env("ask", "hi"))
        .await
        .unwrap();
    assert!(p.seen.lock().unwrap()[0].messages[0]
        .content
        .contains("exec <command>"));
}

#[tokio::test]
async fn long_output_is_capped() {
    let mut cfg = Config::defaults();
    cfg.tools.exec.max_output = 100;
    let p = FakeProvider::with(vec![Ok(exec_call("seq 1 5000")), Ok("ok".to_string())]);
    let _ = engine(vec![("a", p.clone(), 1)])
        .with_config(cfg)
        .with_approver(FakeApprover::new(Choice::Run))
        .run(&env("ask", "look"))
        .await
        .unwrap();
    let r = second_request(&p);
    assert!(
        r.contains("[output truncated]") && r.len() < 400,
        "{}",
        r.len()
    );
}
