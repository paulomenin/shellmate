//! Onboarding and diagnostics through the real binary: config init/show, install-shell, setup,
//! doctor, the audit log and --explain. Each test gets its own HOME so nothing touches the real one.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;
use std::process::{Command, Stdio};

fn raw(args: &[&str], home: &Scratch, envs: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_shellmate"));
    cmd.args(args)
        .env_remove("SHELLMATE_CONFIG")
        .env_remove("SHELLMATE_SESSION")
        .env("HOME", &home.0)
        .env("XDG_CONFIG_HOME", home.path("config"))
        .env("XDG_RUNTIME_DIR", home.path("run"))
        .env("XDG_STATE_HOME", home.path("state"))
        .env("SHELL", "/bin/bash")
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    std::fs::create_dir_all(home.path("run")).unwrap();
    std::fs::set_permissions(
        home.path("run"),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let out = cmd.output().unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into(),
        stderr: String::from_utf8_lossy(&out.stderr).into(),
    }
}

#[test]
fn an_unconfigured_invoke_points_at_setup() {
    let home = Scratch::new("unconf");
    let r = common::run_with(&["invoke"], "version = 1\n", &envelope("ask", "hi"), &[]);
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("isn't configured yet") && r.stderr.contains("shellmate setup"),
        "{}",
        r.stderr
    );
    drop(home);
}

#[test]
fn config_init_writes_a_valid_template_and_refuses_to_overwrite() {
    let home = Scratch::new("init");
    let path = home.str("c.toml");
    let r = raw(
        &[
            "--config",
            &path,
            "config",
            "init",
            "--provider",
            "openai",
            "--model",
            "gpt-x",
        ],
        &home,
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("model = \"gpt-x\"") && text.contains("env:OPENAI_API_KEY"));
    let again = raw(&["--config", &path, "config", "init"], &home, &[]);
    assert_eq!(again.code, 1);
    assert!(again.stderr.contains("config.exists"), "{}", again.stderr);
    let forced = raw(
        &["--config", &path, "config", "init", "--force"],
        &home,
        &[],
    );
    assert_eq!(forced.code, 0);
    // with no --provider nothing is chosen: the examples are comments and no provider is active
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# preset = \"openrouter\""), "{text}");
    assert!(!text.lines().any(|l| l.starts_with("[providers")), "{text}");
    assert!(
        !text.contains("qwen"),
        "no developer's model in the starting config"
    );
    let bad = raw(
        &[
            "--config",
            &home.str("d.toml"),
            "config",
            "init",
            "--provider",
            "nope",
            "--model",
            "m",
        ],
        &home,
        &[],
    );
    assert!(bad.stderr.contains("setup.unknown_provider"));
}

#[test]
fn config_show_lists_the_effective_settings_without_secret_values() {
    let home = Scratch::new("show");
    let path = home.str("c.toml");
    std::fs::write(
        &path,
        "version = 1\n[providers.p]\nplugin = \"builtin:openai-compatible\"\n[providers.p.settings]\npreset = \"openai\"\nmodel = \"m\"\napi_key = \"env:SM_TEST_KEY\"\n[llm]\nchain = [\"p\"]\n",
    )
    .unwrap();
    let r = raw(
        &["--config", &path, "config", "show"],
        &home,
        &[("SM_TEST_KEY", "sk-should-never-print-0123456789")],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("chain:")
            && r.stdout.contains("provider p:")
            && r.stdout.contains("env:SM_TEST_KEY"),
        "{}",
        r.stdout
    );
    assert!(
        !r.stdout.contains("sk-should-never-print"),
        "a resolved secret leaked"
    );
}

#[test]
fn install_shell_adds_updates_and_removes_a_marked_block_with_a_backup() {
    let home = Scratch::new("rc");
    let rc = home.path(".bashrc");
    std::fs::write(&rc, "export KEEP=1\n").unwrap();
    let dry = raw(&["install-shell", "bash", "--dry-run"], &home, &[]);
    assert!(dry.stdout.contains("init bash"), "{}", dry.stdout);
    assert_eq!(
        std::fs::read_to_string(&rc).unwrap(),
        "export KEEP=1\n",
        "dry-run changes nothing"
    );

    let no_yes = raw(&["install-shell", "bash"], &home, &[]);
    assert_eq!(
        no_yes.code, 1,
        "without --yes and without a terminal it must refuse"
    );
    assert_eq!(std::fs::read_to_string(&rc).unwrap(), "export KEEP=1\n");

    let r = raw(&["install-shell", "bash", "--yes"], &home, &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let text = std::fs::read_to_string(&rc).unwrap();
    assert!(
        text.starts_with("export KEEP=1")
            && text.contains("# >>> shellmate >>>")
            && text.contains("init bash)")
    );
    assert_eq!(
        std::fs::read_to_string(home.path(".bashrc.shellmate-backup")).unwrap(),
        "export KEEP=1\n"
    );
    let again = raw(&["install-shell", "bash", "--yes"], &home, &[]);
    assert!(again.stdout.contains("nothing to change"));

    let rm = raw(&["install-shell", "bash", "--yes", "--remove"], &home, &[]);
    assert_eq!(rm.code, 0);
    assert_eq!(
        std::fs::read_to_string(&rc).unwrap().trim(),
        "export KEEP=1"
    );
}

#[test]
fn setup_yes_writes_config_installs_the_shell_and_reports() {
    let home = Scratch::new("setup");
    let r = raw(
        &[
            "setup",
            "--yes",
            "--provider",
            "ollama",
            "--model",
            "m1",
            "--shell",
            "fish",
        ],
        &home,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let cfg = home.path("config/shellmate/config.toml");
    assert!(std::fs::read_to_string(cfg)
        .unwrap()
        .contains("model = \"m1\""));
    let rc = std::fs::read_to_string(home.path("config/fish/config.fish")).unwrap();
    assert!(rc.contains("init fish | source"));
    assert!(
        r.stdout.contains("try it:") && r.stdout.contains("config"),
        "{}",
        r.stdout
    );
    // re-running keeps the config
    let again = raw(
        &["setup", "--yes", "--provider", "ollama", "--shell", "fish"],
        &home,
        &[],
    );
    assert!(
        again.stdout.contains("keeping existing"),
        "{}",
        again.stdout
    );
    // missing information is an error, not a guess
    let none = raw(&["setup", "--yes", "--shell", "nu"], &home, &[]);
    assert_ne!(none.code, 0);
}

#[test]
fn doctor_reports_failures_with_remedies_and_exits_non_zero() {
    let home = Scratch::new("doc");
    let r = raw(&["doctor"], &home, &[]);
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert!(
        r.stdout.contains("FAIL") && r.stdout.contains("shellmate setup"),
        "{}",
        r.stdout
    );
    let j = raw(&["doctor", "--json"], &home, &[]);
    let v: serde_json::Value = serde_json::from_str(&j.stdout).unwrap();
    assert!(v
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "config" && c["status"] == "fail"));
}

#[tokio::test]
async fn doctor_is_green_for_a_working_setup_and_warns_about_an_unreachable_provider() {
    let server = MockHttpServer::start(200, &completion("x")).await;
    let home = Scratch::new("docok");
    let path = home.str("c.toml");
    std::fs::write(&path, config(&server.url())).unwrap();
    let p = path.clone();
    let h = home.0.clone();
    let r = tokio::task::spawn_blocking(move || {
        let home = Scratch(h.clone());
        let r = raw(&["--config", &p, "doctor"], &home, &[]);
        std::mem::forget(home);
        r
    })
    .await
    .unwrap();
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(
        r.stdout.contains("is reachable") && r.stdout.contains("is valid"),
        "{}",
        r.stdout
    );

    std::fs::write(&path, config("http://127.0.0.1:9")).unwrap();
    let down = raw(&["--config", &path, "doctor"], &home, &[]);
    assert_eq!(
        down.code, 0,
        "an unreachable provider is a warning, not a failure"
    );
    assert!(
        down.stdout.contains("WARN") && down.stdout.contains("cannot connect"),
        "{}",
        down.stdout
    );
}

#[tokio::test]
async fn the_audit_log_records_decisions_but_never_content() {
    let server = MockHttpServer::start(200, &completion("the secret answer text")).await;
    let home = Scratch::new("audit");
    let cfg = conv_config(&server.url(), &home.str("store"), "");
    let (c2, h) = (cfg.clone(), home.0.clone());
    let run_it = tokio::task::spawn_blocking(move || {
        let home = Scratch(h);
        let r = common::run_with(
            &["invoke", "--explain"],
            &c2,
            &envelope_s("ask", "my private question about zebras", "sess9"),
            &[("XDG_RUNTIME_DIR", &home.str("run"))],
        );
        std::mem::forget(home);
        r
    });
    std::fs::create_dir_all(home.path("run")).unwrap();
    std::fs::set_permissions(
        home.path("run"),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let r = run_it.await.unwrap();
    assert_eq!(r.code, 10, "{}", r.stderr);
    // --explain prints facts, no content
    assert!(
        r.stderr.contains("provider:")
            && r.stderr.contains("context:")
            && r.stderr.contains("timing:"),
        "{}",
        r.stderr
    );

    let log = std::fs::read_to_string(home.path("run/shellmate/audit.jsonl")).unwrap();
    for ev in ["request_started", "llm_call", "turn_finished"] {
        assert!(log.contains(ev), "missing {ev}:\n{log}");
    }
    assert!(log.contains("sess9"));
    assert!(
        !log.contains("zebras") && !log.contains("secret answer"),
        "content leaked into the audit log:\n{log}"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(home.path("run/shellmate/audit.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );

    let cfgp = home.str("a.toml");
    std::fs::write(&cfgp, &cfg).unwrap();
    let show = raw(&["--config", &cfgp, "audit", "show"], &home, &[]);
    assert!(
        show.stdout.contains("llm_call") && show.stdout.contains("provider=\"mock\""),
        "{}",
        show.stdout
    );
    let export = raw(&["--config", &cfgp, "audit", "export"], &home, &[]);
    let v: serde_json::Value = serde_json::from_str(&export.stdout).unwrap();
    assert!(v.as_array().unwrap().len() >= 3);
    let clear = raw(&["--config", &cfgp, "audit", "clear"], &home, &[]);
    assert!(clear.stdout.contains("deleted"));
    assert!(raw(&["--config", &cfgp, "audit", "show"], &home, &[])
        .stdout
        .contains("no audit events"));
}

#[tokio::test]
async fn the_audit_log_can_be_turned_off() {
    let server = MockHttpServer::start(200, &completion("fine")).await;
    let home = Scratch::new("auditoff");
    let cfg = conv_config(
        &server.url(),
        &home.str("store"),
        "[audit]\nenabled = false",
    );
    let h = home.0.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(h.join("run")).unwrap();
        std::fs::set_permissions(
            h.join("run"),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        common::run_with(
            &["invoke"],
            &cfg,
            &envelope("ask", "hi"),
            &[("XDG_RUNTIME_DIR", h.join("run").to_str().unwrap())],
        )
    })
    .await
    .unwrap();
    assert!(!home.path("run/shellmate/audit.jsonl").exists());
}

fn with_decider(llm: &str, decider: &str, timeout: &str) -> String {
    format!(
        "{}\n[validator]\nsemantic = true\ndeciders = [\"laya\"]\ntimeout = \"{timeout}\"\n\
         [deciders.laya]\nplugin = \"builtin:systemone\"\ntrust = \"local\"\n\
         [deciders.laya.settings]\nendpoint = \"{decider}\"\nmodel = \"laya\"\n",
        config(llm)
    )
}

#[tokio::test]
async fn doctor_checks_the_decision_model_and_warns_about_a_short_timeout() {
    let llm = MockHttpServer::start(200, &completion("x")).await;
    let decider = MockHttpServer::start(200, "{}").await;
    let home = Scratch::new("docdec");
    let path = home.str("c.toml");

    std::fs::write(&path, with_decider(&llm.url(), &decider.url(), "20s")).unwrap();
    let ok = raw(&["--config", &path, "doctor"], &home, &[]);
    assert!(
        ok.stdout.contains("laya: laya at") && ok.stdout.contains("is reachable"),
        "{}",
        ok.stdout
    );
    assert!(!ok.stdout.contains("validator timeout"), "{}", ok.stdout);

    std::fs::write(&path, with_decider(&llm.url(), &decider.url(), "5s")).unwrap();
    let slow = raw(&["--config", &path, "doctor"], &home, &[]);
    assert!(
        slow.stdout.contains("validator timeout is 5s")
            && slow.stdout.contains("OLLAMA_KEEP_ALIVE"),
        "{}",
        slow.stdout
    );

    std::fs::write(&path, with_decider(&llm.url(), "http://127.0.0.1:9", "20s")).unwrap();
    let down = raw(&["--config", &path, "doctor"], &home, &[]);
    assert_eq!(down.code, 0, "an unreachable decider is only a warning");
    assert!(
        down.stdout.contains("laya: cannot connect to 127.0.0.1:9"),
        "{}",
        down.stdout
    );
}

#[tokio::test]
async fn context_shows_what_each_level_sends_and_nothing_is_sent() {
    let llm = MockHttpServer::start(200, &completion("x")).await;
    let home = Scratch::new("ctx");
    let path = home.str("c.toml");
    // a remote provider capped at level 1: git and captured output must show as withheld
    let remote =
        remote_config(&llm.url(), "").replace("max_context_level = 2", "max_context_level = 1");
    std::fs::write(&path, remote).unwrap();
    let r = raw(
        &["--config", &path, "context", "why", "is", "it", "slow"],
        &home,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("provider `cloud` (remote)"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("context level 1 (ceiling 1)"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("L0 your typed text") && r.stdout.contains("14 chars"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("git state")
            && r.stdout
                .contains("needs level 2; remote providers are capped at 1"),
        "{}",
        r.stdout
    );
    assert!(
        !r.stdout.contains("payload for"),
        "no payload without --full"
    );
    assert!(
        llm.last_request().await.is_empty(),
        "context must not contact the provider"
    );

    let full = raw(
        &["--config", &path, "context", "--full", "hello"],
        &home,
        &[],
    );
    assert!(
        full.stdout.contains("payload for `cloud`") && full.stdout.contains("hello"),
        "{}",
        full.stdout
    );
}

#[tokio::test]
async fn timings_report_where_the_time_went_as_text_and_as_json() {
    let llm = MockHttpServer::start(200, &completion("Here.\n```shellmate-command\nls\n```")).await;
    let c = config(&llm.url());
    let c2 = c.clone();
    let (text, json) = tokio::task::spawn_blocking(move || {
        (
            run(&["invoke", "--timings"], &c, &envelope("ask", "list files")),
            run(
                &["invoke", "--timings=json"],
                &c2,
                &envelope("ask", "list files"),
            ),
        )
    })
    .await
    .unwrap();
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert_eq!(text.stdout, "ls", "timings must never reach stdout");
    for part in [
        "startup ",
        "context ",
        "llm ",
        "checks ",
        "overhead ",
        "total ",
    ] {
        assert!(text.stderr.contains(part), "{part}: {}", text.stderr);
    }
    let line = json
        .stderr
        .lines()
        .find(|l| l.starts_with("{\"timings\""))
        .unwrap_or_else(|| panic!("no json line: {}", json.stderr));
    let v: serde_json::Value = serde_json::from_str(line).unwrap();
    let t = &v["timings"];
    for k in [
        "startup_ms",
        "context_ms",
        "llm_ms",
        "validate_ms",
        "engine_ms",
        "overhead_ms",
        "process_ms",
    ] {
        assert!(t[k].is_f64() || t[k].is_u64(), "{k} missing in {line}");
    }
    assert!(t["process_ms"].as_f64() >= t["engine_ms"].as_f64());
    assert_eq!(json.stdout, "ls");
}

#[test]
fn check_reports_parse_errors_missing_programs_and_risk_without_running_anything() {
    let home = Scratch::new("check");
    let c = |args: &[&str]| raw(args, &home, &[]);
    let ok = c(&["check", "--shell", "bash", "ls", "-la"]);
    assert_eq!(
        (ok.code, ok.stdout.trim()),
        (0, "no findings"),
        "{}",
        ok.stdout
    );
    let bad = c(&["check", "--shell", "bash", "ls", "|"]);
    assert_eq!(bad.code, 1);
    assert!(
        bad.stdout.contains("does not parse in bash"),
        "{}",
        bad.stdout
    );
    let risky = c(&["check", "--shell", "bash", "--json", "rm", "-rf", "build"]);
    let v: serde_json::Value = serde_json::from_str(risky.stdout.trim()).unwrap();
    assert_eq!(
        (risky.code, v["risk"].as_str(), v["parses"].as_bool()),
        (0, Some("high"), Some(true))
    );
    let missing = c(&[
        "check",
        "--shell",
        "bash",
        "--json",
        "no-such-program-xyz",
        "--x",
    ]);
    let v: serde_json::Value = serde_json::from_str(missing.stdout.trim()).unwrap();
    assert_eq!(v["missing"][0], "no-such-program-xyz");
    // the line is never executed
    let marker = std::env::temp_dir().join(format!("sm-check-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    c(&[
        "check",
        "--shell",
        "bash",
        &format!("touch {}", marker.display()),
    ]);
    assert!(!marker.exists(), "check must not run the line");
}

/// An editor for tests: `sh <script>` appends a line to the file it is given and records its arguments.
fn editor_script(home: &Scratch, appended: &str, exit: i32) -> String {
    let script = home.path("editor.sh");
    std::fs::write(
        &script,
        format!(
            "for last; do :; done\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{appended}' >> \"$last\"\nexit {exit}\n",
            home.str("args")
        ),
    )
    .unwrap();
    format!("sh {}", script.display())
}

fn config_with_provider(home: &Scratch) -> String {
    let path = home.str("c.toml");
    let r = raw(
        &[
            "--config",
            &path,
            "config",
            "init",
            "--provider",
            "ollama",
            "--model",
            "m",
        ],
        home,
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    path
}

#[test]
fn config_edit_opens_the_file_in_the_editor_and_checks_it_afterwards() {
    let home = Scratch::new("edit-ok");
    let path = config_with_provider(&home);
    let editor = editor_script(&home, "# edited", 0);
    let r = raw(
        &["--config", &path, "config", "edit"],
        &home,
        &[("EDITOR", &editor), ("VISUAL", "")],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .ends_with("# edited\n"));
    assert_eq!(
        std::fs::read_to_string(home.path("args")).unwrap().trim(),
        path
    );
    assert!(r.stdout.contains("config ok"), "{}", r.stdout);
}

#[test]
fn config_edit_prefers_visual_and_keeps_the_editors_own_arguments() {
    let home = Scratch::new("edit-visual");
    let path = config_with_provider(&home);
    let editor = editor_script(&home, "# from visual", 0);
    let r = raw(
        &["--config", &path, "config", "edit"],
        &home,
        &[("VISUAL", &format!("{editor} --wait")), ("EDITOR", "false")],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let args = std::fs::read_to_string(home.path("args")).unwrap();
    assert_eq!(args.lines().collect::<Vec<_>>(), ["--wait", path.as_str()]);
}

#[test]
fn config_edit_runs_the_config_check_and_reports_a_file_that_no_longer_loads() {
    let home = Scratch::new("edit-bad");
    let path = config_with_provider(&home);
    let editor = editor_script(&home, "this is = = not toml", 0);
    let r = raw(
        &["--config", &path, "config", "edit"],
        &home,
        &[("EDITOR", &editor), ("VISUAL", "")],
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("config."), "{}", r.stderr);
    assert!(r.stderr.contains("your edit is kept"), "{}", r.stderr);
    // the person's edit is kept, not rolled back
    assert!(std::fs::read_to_string(&path).unwrap().contains("not toml"));
}

#[test]
fn config_edit_fails_when_there_is_no_file_or_no_editor_to_start() {
    let home = Scratch::new("edit-none");
    let path = home.str("missing.toml");
    let r = raw(
        &["--config", &path, "config", "edit"],
        &home,
        &[("EDITOR", "true")],
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("config init"), "{}", r.stderr);
    assert!(!home.path("missing.toml").exists(), "nothing is created");

    let path = config_with_provider(&home);
    let r = raw(
        &["--config", &path, "config", "edit"],
        &home,
        &[("EDITOR", "/nonexistent/editor"), ("VISUAL", "")],
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("could not start"), "{}", r.stderr);

    // an editor that fails is an error, and the config is not judged
    let failing = editor_script(&home, "# x", 3);
    let r = raw(
        &["--config", &path, "config", "edit"],
        &home,
        &[("EDITOR", &failing), ("VISUAL", "")],
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("exited with"), "{}", r.stderr);
}

#[test]
fn a_bare_config_init_is_valid_and_leaves_the_provider_to_you() {
    let home = Scratch::new("init-bare");
    let path = home.str("c.toml");
    assert_eq!(
        raw(&["--config", &path, "config", "init"], &home, &[]).code,
        0
    );
    // the file loads, and the unconfigured state is explained, not a crash
    let r = raw(&["--config", &path, "config", "check"], &home, &[]);
    assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("shellmate setup"), "{}", r.stderr);
    // --model only makes sense with --provider, and the other way round
    let r = raw(
        &[
            "--config",
            &home.str("d.toml"),
            "config",
            "init",
            "--model",
            "m",
        ],
        &home,
        &[],
    );
    assert_ne!(r.code, 0);
    assert!(!home.path("d.toml").exists());
    // a provider without a model is refused too: the model is never guessed
    let r = raw(
        &[
            "--config",
            &home.str("e.toml"),
            "config",
            "init",
            "--provider",
            "openrouter",
        ],
        &home,
        &[],
    );
    assert_ne!(r.code, 0);
    assert!(!home.path("e.toml").exists());
    let r = raw(
        &[
            "--config",
            &home.str("f.toml"),
            "config",
            "init",
            "--provider",
            "openrouter",
            "--model",
            "my/typed-model",
        ],
        &home,
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(std::fs::read_to_string(home.path("f.toml"))
        .unwrap()
        .contains("model = \"my/typed-model\""));
}

#[tokio::test]
async fn setup_yes_verify_asks_each_server_with_the_key_and_writes_the_checker() {
    let server = MockHttpServer::start(200, r#"{"data":[{"id":"m"}]}"#).await;
    let url = server.url();
    let home = Scratch::new("setup-verify");
    let h = home.0.clone();
    let r = tokio::task::spawn_blocking(move || {
        let home = Scratch(h);
        let r = raw(
            &[
                "setup",
                "--yes",
                "--shell",
                "bash",
                "--provider",
                "custom",
                "--base-url",
                &url,
                "--model",
                "m",
                "--api-key",
                "env:SETUP_TEST_KEY",
                "--checker",
                "custom",
                "--checker-url",
                &url,
                "--checker-model",
                "jev",
                "--checker-api-key",
                "env:SETUP_TEST_KEY",
                "--verify",
            ],
            &home,
            &[("SETUP_TEST_KEY", "sk-test-123")],
        );
        std::mem::forget(home);
        r
    })
    .await
    .unwrap();
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let cfg = std::fs::read_to_string(home.path("config/shellmate/config.toml")).unwrap();
    assert!(
        cfg.contains("[deciders.checker]") && cfg.contains("semantic = true"),
        "{cfg}"
    );
    assert!(cfg.contains("api_key = \"env:SETUP_TEST_KEY\""), "{cfg}");
    assert!(
        !cfg.contains("sk-test-123"),
        "the key must never be written"
    );
    // both servers were asked for their models with the key, and were sent nothing else (the empty
    // connections are the reachability probes of the `doctor` that setup ends with)
    let real: Vec<String> = server
        .requests()
        .await
        .into_iter()
        .filter(|r| !r.is_empty())
        .collect();
    assert_eq!(real.len(), 2, "{real:?}");
    for seen in &real {
        assert!(seen.starts_with("GET /v1/models"), "{seen}");
        assert!(
            seen.to_lowercase()
                .contains("authorization: bearer sk-test-123"),
            "{seen}"
        );
        assert!(
            !seen.contains("messages") && !seen.contains("questions"),
            "{seen}"
        );
    }
}

#[tokio::test]
async fn setup_yes_verify_stops_before_writing_when_the_key_is_rejected() {
    let server = MockHttpServer::start(401, r#"{"error":{"message":"bad key"}}"#).await;
    let url = server.url();
    let home = Scratch::new("setup-rejected");
    let h = home.0.clone();
    let r = tokio::task::spawn_blocking(move || {
        let home = Scratch(h);
        let r = raw(
            &[
                "setup",
                "--yes",
                "--shell",
                "bash",
                "--provider",
                "custom",
                "--base-url",
                &url,
                "--model",
                "m",
                "--api-key",
                "env:SETUP_TEST_KEY",
                "--verify",
            ],
            &home,
            &[("SETUP_TEST_KEY", "sk-test-123")],
        );
        std::mem::forget(home);
        r
    })
    .await
    .unwrap();
    assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("did not answer") && r.stderr.contains("401"),
        "{}",
        r.stderr
    );
    assert!(!r.stderr.contains("sk-test-123") && !r.stdout.contains("sk-test-123"));
    assert!(
        !home.path("config/shellmate/config.toml").exists(),
        "nothing is written"
    );
    assert!(
        !home.path(".bashrc").exists(),
        "the shell is left alone too"
    );
}

#[test]
fn setup_yes_without_verify_contacts_nothing_and_refuses_a_pasted_key() {
    let home = Scratch::new("setup-offline");
    // a closed port: if setup tried to connect, it would fail
    let r = raw(
        &[
            "setup",
            "--yes",
            "--shell",
            "bash",
            "--provider",
            "custom",
            "--base-url",
            "http://127.0.0.1:9",
            "--model",
            "m",
            "--api-key",
            "env:SETUP_TEST_KEY",
        ],
        &home,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("SETUP_TEST_KEY"),
        "reminds you to set the variable: {}",
        r.stdout
    );
    let pasted = raw(
        &[
            "setup",
            "--yes",
            "--shell",
            "bash",
            "--provider",
            "openrouter",
            "--model",
            "m",
            "--api-key",
            "sk-or-v1-0123456789abcdef",
            "--config",
            &home.str("other.toml"),
        ],
        &home,
        &[],
    );
    assert_eq!(pasted.code, 1);
    assert!(
        pasted.stderr.contains("setup.bad_key_ref") || pasted.stderr.contains("not a reference"),
        "{}",
        pasted.stderr
    );
    assert!(!pasted.stderr.contains("sk-or-v1"), "{}", pasted.stderr);
}
