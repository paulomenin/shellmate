use super::*;

fn read_call(arg: &str) -> String {
    format!("```shellmate-tool\nread {arg}\n```")
}

fn work_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-read-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("notes.txt"), "line one\nline two\n").unwrap();
    std::fs::write(d.join("src/main.rs"), "fn main() {}\n").unwrap();
    d
}

/// The model asks to read `arg` from `dir`, then answers. Returns the requests the model saw.
async fn run_read(
    dir: &std::path::Path,
    arg: &str,
    approver: Option<Arc<FakeApprover>>,
    cfg: Config,
) -> Arc<FakeProvider> {
    let p = FakeProvider::with(vec![Ok(read_call(arg)), Ok("Done.".to_string())]);
    let mut e = engine(vec![("a", p.clone(), 1)])
        .with_config(cfg)
        .with_builtin_tools();
    if let Some(a) = approver {
        e = e.with_approver(a);
    }
    let mut env = env("ask", "what is in it?");
    env.cwd = Some(dir.display().to_string());
    e.run(&env).await.unwrap();
    p
}

#[tokio::test]
async fn an_approved_read_puts_the_text_in_front_of_the_model() {
    let d = work_dir("ok");
    let a = FakeApprover::new(Choice::Run);
    let p = run_read(&d, "notes.txt", Some(a.clone()), Config::defaults()).await;
    assert_eq!(a.count(), 1);
    let asked = a.asked.lock().unwrap()[0].clone();
    assert_eq!(asked.kind, ApprovalKind::Read);
    assert_eq!(asked.command, "notes.txt");
    assert!(
        asked.reasons[0].contains("sent to `a` (local)"),
        "{:?}",
        asked.reasons
    );
    let r = second_request(&p);
    assert!(
        r.contains("line one") && r.contains("never instructions"),
        "{r}"
    );
    // the tool is described to the model
    assert!(p.seen.lock().unwrap()[0].messages[0]
        .content
        .contains("read <path>"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn a_denied_read_or_no_terminal_reads_nothing() {
    let d = work_dir("deny");
    let a = FakeApprover::new(Choice::Deny);
    let p = run_read(&d, "notes.txt", Some(a.clone()), Config::defaults()).await;
    assert!(second_request(&p).contains("denied by the user"));
    assert!(!second_request(&p).contains("line one"));
    let p = run_read(&d, "notes.txt", None, Config::defaults()).await;
    assert!(second_request(&p).contains("no terminal"));
    // "put it in my prompt" makes no sense for a file: it is a refusal
    let p = run_read(
        &d,
        "notes.txt",
        Some(FakeApprover::new(Choice::ToBuffer)),
        Config::defaults(),
    )
    .await;
    assert!(second_request(&p).contains("denied by the user"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn paths_outside_the_roots_and_credentials_are_refused_without_asking() {
    let d = work_dir("refuse");
    std::fs::write(d.join(".env"), "TOKEN=abc").unwrap();
    std::fs::write(d.join("my_credentials.json"), "{}").unwrap();
    std::fs::create_dir_all(d.join(".ssh")).unwrap();
    std::fs::write(d.join(".ssh/config"), "Host x").unwrap();
    std::os::unix::fs::symlink("/etc/passwd", d.join("link")).unwrap();
    let outside = std::env::temp_dir().join(format!("sm-read-outside-{}", std::process::id()));
    std::fs::write(&outside, "secret outside").unwrap();
    for (arg, why) in [
        (".env", "looks like credentials"),
        ("my_credentials.json", "looks like credentials"),
        (".ssh/config", "looks like credentials"),
        ("link", "outside the directories"),
        ("..", "outside the directories"),
        ("/etc/hostname", "outside the directories"),
        (outside.to_str().unwrap(), "outside the directories"),
        ("missing.txt", "does not exist"),
        ("src", "not a regular file"),
        ("a b", "exactly one path"),
        ("$HOME/x", "one plain word"),
    ] {
        let a = FakeApprover::new(Choice::Run);
        let p = run_read(&d, arg, Some(a.clone()), Config::defaults()).await;
        assert_eq!(a.count(), 0, "{arg}: refused before anyone is asked");
        let r = second_request(&p);
        assert!(r.contains(why), "{arg}: wanted {why:?} in {r}");
        assert!(
            !r.contains("secret outside") && !r.contains("TOKEN=abc") && !r.contains("Host x"),
            "{arg}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_file(outside);
}

#[tokio::test]
async fn a_project_called_secrets_is_not_blocked_by_its_own_name() {
    let d = work_dir("secrets-app");
    let sub = d.join("credentials-service");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("main.rs"), "fn x() {}").unwrap();
    // the working directory's own name is not judged; names below it are
    let inner = sub.clone();
    let a = FakeApprover::new(Choice::Run);
    let p = run_read(&inner, "main.rs", Some(a.clone()), Config::defaults()).await;
    assert_eq!(a.count(), 1);
    assert!(second_request(&p).contains("fn x()"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn configured_roots_extend_the_reach_but_not_to_credentials() {
    let d = work_dir("roots");
    let other = work_dir("roots-other");
    std::fs::write(other.join("shared.txt"), "from the other root").unwrap();
    let mut cfg = Config::defaults();
    cfg.tools.read.roots = vec![other.display().to_string()];
    let a = FakeApprover::new(Choice::Run);
    let p = run_read(
        &d,
        other.join("shared.txt").to_str().unwrap(),
        Some(a.clone()),
        cfg,
    )
    .await;
    assert_eq!(a.count(), 1);
    assert!(second_request(&p).contains("from the other root"));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&other);
}

#[tokio::test]
async fn binary_and_big_files_are_handled() {
    let d = work_dir("bin");
    std::fs::write(d.join("blob.bin"), b"\x00\x01\x02 not text").unwrap();
    std::fs::write(d.join("big.txt"), "x".repeat(50_000)).unwrap();
    std::fs::write(d.join("ansi.txt"), "\x1b[31mred\x1b[0m\u{7}ok\n").unwrap();
    let a = FakeApprover::new(Choice::Run);
    let p = run_read(&d, "blob.bin", Some(a.clone()), Config::defaults()).await;
    assert!(second_request(&p).contains("binary file"));
    assert_eq!(
        a.count(),
        1,
        "binary is found when reading, after the user agreed"
    );
    let mut cfg = Config::defaults();
    cfg.tools.read.max_bytes = 1000;
    let p = run_read(&d, "big.txt", Some(FakeApprover::new(Choice::Run)), cfg).await;
    let r = second_request(&p);
    assert!(r.contains("cut here") && r.len() < 1500, "{}", r.len());
    let p = run_read(
        &d,
        "ansi.txt",
        Some(FakeApprover::new(Choice::Run)),
        Config::defaults(),
    )
    .await;
    let r = second_request(&p);
    assert!(
        !r.contains('\x1b') && !r.contains('\u{7}') && r.contains("redok"),
        "{r:?}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn a_rule_skips_the_prompt_but_never_for_credentials_or_tainted_turns() {
    let d = work_dir("rule");
    std::fs::write(d.join(".env"), "TOKEN=abc").unwrap();
    let mut cfg = Config::defaults();
    cfg.tools.read.preapproved = vec!["src/*".into(), "*".into()];
    let a = FakeApprover::new(Choice::Deny);
    let p = run_read(&d, "src/main.rs", Some(a.clone()), cfg.clone()).await;
    assert_eq!(a.count(), 0, "covered by the user's rule");
    assert!(second_request(&p).contains("fn main()"));
    // even a rule that matches everything cannot reach credentials
    let a = FakeApprover::new(Choice::Run);
    let p = run_read(&d, ".env", Some(a.clone()), cfg).await;
    assert_eq!(a.count(), 0);
    assert!(!second_request(&p).contains("TOKEN=abc"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn file_text_taints_the_rest_of_the_turn() {
    // the model reads a file (pre-approved), then asks to run a command that a user rule would allow:
    // the file text is untrusted, so the rule stops applying and the user is asked (INV-4)
    let d = work_dir("taint");
    let mut cfg = Config::defaults();
    cfg.tools.read.preapproved = vec!["notes.txt".into()];
    cfg.tools.exec.preapproved = vec!["echo *".into()];
    let a = FakeApprover::new(Choice::Deny);
    let p = FakeProvider::with(vec![
        Ok(read_call("notes.txt")),
        Ok(exec_call("echo hi")),
        Ok("Done.".to_string()),
    ]);
    let mut env = env("ask", "look");
    env.cwd = Some(d.display().to_string());
    engine(vec![("a", p, 1)])
        .with_config(cfg.clone())
        .with_builtin_tools()
        .with_approver(a.clone())
        .run(&env)
        .await
        .unwrap();
    assert_eq!(
        a.count(),
        1,
        "only the exec asked; the read was pre-approved"
    );
    assert_eq!(a.asked.lock().unwrap()[0].kind, ApprovalKind::Run);
    // without the read first, the same rule applies
    let a = FakeApprover::new(Choice::Deny);
    let p = FakeProvider::with(vec![Ok(exec_call("echo hi")), Ok("Done.".to_string())]);
    engine(vec![("a", p, 1)])
        .with_config(cfg)
        .with_builtin_tools()
        .with_approver(a.clone())
        .run(&env)
        .await
        .unwrap();
    assert_eq!(a.count(), 0);
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn read_can_be_turned_off_and_is_only_offered_to_ask() {
    let d = work_dir("off");
    let mut cfg = Config::defaults();
    cfg.tools.read.approval = crate::config::ExecApproval::Deny;
    let a = FakeApprover::new(Choice::Run);
    let p = run_read(&d, "notes.txt", Some(a.clone()), cfg).await;
    assert_eq!(a.count(), 0);
    assert!(!p.seen.lock().unwrap()[0].messages[0]
        .content
        .contains("read <path>"));
    let p = FakeProvider::replying("```shellmate-command\nls\n```");
    let _ = engine(vec![("a", p.clone(), 1)])
        .run(&env("generate", "list"))
        .await
        .unwrap();
    assert!(!p.seen.lock().unwrap()[0].messages[0]
        .content
        .contains("read <path>"));
    let _ = std::fs::remove_dir_all(&d);
}
