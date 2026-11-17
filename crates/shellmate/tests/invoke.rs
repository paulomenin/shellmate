//! End-to-end tests of the binary: envelope on stdin -> buffer on stdout, against a mock provider.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;

#[tokio::test]
async fn generate_writes_the_command_to_stdout_with_exit_0() {
    let server = MockHttpServer::start(
        200,
        &completion("Largest first.\n```shellmate-command\ndu -sh * | sort -rh | head\n```"),
    )
    .await;
    let cfg = config(&server.url());
    let r = tokio::task::spawn_blocking(move || {
        run(
            &["invoke"],
            &cfg,
            &envelope("generate", "biggest files here"),
        )
    })
    .await
    .unwrap();
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "du -sh * | sort -rh | head");
    assert!(
        !r.stderr.contains("Largest first."),
        "generate is command-only: {}",
        r.stderr
    );
    let sent = server.last_request().await;
    assert!(sent.contains("biggest files here") && sent.contains("test-model"));
}

#[tokio::test]
async fn explain_leaves_the_buffer_alone_with_exit_10() {
    let server = MockHttpServer::start(200, &completion("It lists files.")).await;
    let cfg = config(&server.url());
    let r =
        tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &envelope("explain", "ls -la")))
            .await
            .unwrap();
    assert_eq!((r.code, r.stdout.as_str()), (10, ""));
    assert!(r.stderr.contains("It lists files."));
}

#[tokio::test]
async fn dry_run_sends_nothing() {
    let server = MockHttpServer::start(200, &completion("unused")).await;
    let cfg = config(&server.url());
    let r = tokio::task::spawn_blocking(move || {
        run(&["invoke", "--dry-run"], &cfg, &envelope("ask", "hello"))
    })
    .await
    .unwrap();
    assert_eq!(r.code, 10);
    assert!(r.stderr.contains("\"messages\"") && r.stderr.contains("hello"));
    assert_eq!(server.request_count().await, 0);
}

#[tokio::test]
async fn provider_errors_are_reported_with_a_hint_and_exit_1() {
    let server = MockHttpServer::start(401, r#"{"error":{"message":"bad key"}}"#).await;
    let cfg = config(&server.url());
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &envelope("ask", "hello")))
        .await
        .unwrap();
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("provider.auth") && r.stderr.contains("bad key"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("API key"), "hint expected: {}", r.stderr);
    assert_eq!(r.stdout, "");
}

#[test]
fn bad_input_and_config_are_errors_not_panics() {
    let r = run(&["invoke"], "version = 1\n", &envelope("ask", "hi"));
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("config.no_provider"), "{}", r.stderr);

    let r = run(&["invoke"], &config("http://127.0.0.1:1"), b"garbage");
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("glue.protocol"), "{}", r.stderr);

    let r = run(
        &["invoke"],
        &config("http://127.0.0.1:1"),
        &envelope("ask", "   "),
    );
    assert!(r.stderr.contains("input.empty"), "{}", r.stderr);
}

#[test]
fn config_check_refuses_a_typo_in_the_store_settings() {
    let dir = std::env::temp_dir().join(format!("sm-store-typo-{}", std::process::id()));
    let base = config("http://127.0.0.1:1");
    let ok = format!(
        "{base}[plugins.\"builtin:store-tmpfs\"]\ndir = \"{}\"\n",
        dir.display()
    );
    assert_eq!(run(&["config", "check"], &ok, b"").code, 0);
    let typo = format!("{ok}unknown_setting = true\n");
    let bad = run(&["config", "check"], &typo, b"");
    assert_eq!(bad.code, 1, "{}", bad.stdout);
    assert!(bad.stderr.contains("config.invalid"), "{}", bad.stderr);
    assert!(bad.stderr.contains("unknown_setting"), "{}", bad.stderr);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn config_check_refuses_a_shortcut_that_init_cannot_bind() {
    let cfg = format!(
        "{}[input.shortcuts]\n\"super+m\" = \"ask\"\n",
        config("http://127.0.0.1:1")
    );
    let check = run(&["config", "check"], &cfg, b"");
    assert_eq!(check.code, 1, "{}", check.stdout);
    assert!(
        check.stderr.contains("unknown modifier `super`"),
        "{}",
        check.stderr
    );
    let init = run(&["init", "bash"], &cfg, b"");
    assert_eq!(init.code, 1);
    assert!(
        init.stderr.contains("unknown modifier `super`"),
        "{}",
        init.stderr
    );
}

#[test]
fn config_check_and_init() {
    let ok = run(&["config", "check"], &config("http://127.0.0.1:1"), b"");
    assert_eq!(ok.code, 0, "{}", ok.stderr);
    assert!(ok.stdout.contains("provider chain: mock"));
    let bad = run(&["config", "check"], "version = 1\n[nope]\n", b"");
    assert_eq!(bad.code, 1);
    assert!(bad.stderr.contains("config.invalid"));
    let init = run(&["init", "fish"], "version = 1\n", b"");
    assert_eq!(init.code, 0);
    assert!(init.stdout.contains("__shellmate_widget") && init.stdout.contains("bind alt-m"));
}
