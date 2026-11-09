//! `cmd:` secrets through the real binary: the key reaches the provider, the program runs on use and not
//! on `config check`, a failure is reported as what it is, and a key pasted into the line is not echoed.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;

fn cfg(url: &str, key: &str) -> String {
    format!(
        "version = 1\n[audit]\nenabled = false\n[providers.mock]\nplugin = \"builtin:openai-compatible\"\ntrust = \"local\"\n[providers.mock.settings]\nendpoint = \"{url}\"\nmodel = \"m\"\napi_key = \"{key}\"\n[llm]\nchain = [\"mock\"]\n"
    )
}

#[tokio::test]
async fn the_output_of_the_program_is_sent_as_the_bearer_token() {
    let server = MockHttpServer::start(200, &completion("hello")).await;
    let config = cfg(&server.url(), "cmd:/bin/sh -c 'echo sk-from-the-program'");
    let r =
        tokio::task::spawn_blocking(move || run(&["invoke"], &config, &envelope("explain", "ls")))
            .await
            .unwrap();
    assert_eq!(
        r.code, 10,
        "explain leaves the input line unchanged: {}",
        r.stderr
    );
    assert!(r.stderr.contains("hello"), "the answer: {}", r.stderr);
    let raw = server.last_request().await.to_lowercase();
    assert!(
        raw.contains("authorization: bearer sk-from-the-program"),
        "{raw}"
    );
}

#[tokio::test]
async fn a_failing_program_stops_the_request_before_anything_is_sent() {
    let server = MockHttpServer::start(200, &completion("hello")).await;
    let config = cfg(&server.url(), "cmd:/bin/sh -c 'echo no-way >&2; exit 7'");
    let r =
        tokio::task::spawn_blocking(move || run(&["invoke"], &config, &envelope("explain", "ls")))
            .await
            .unwrap();
    assert_ne!(r.code, 0);
    assert!(
        r.stderr.contains("config.secret_cmd_failed"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("no-way"), "{}", r.stderr); // the program's own stderr, to say why
    assert_eq!(server.request_count().await, 0);
}

#[tokio::test]
async fn config_check_does_not_run_the_program_and_a_bad_line_is_an_error() {
    let marker = Scratch::new("check");
    let line = format!("cmd:/bin/sh -c 'touch {}; echo k'", marker.str("ran"));
    let config = cfg("http://127.0.0.1:9", &line);
    let r = tokio::task::spawn_blocking(move || run(&["config", "check"], &config, b""))
        .await
        .unwrap();
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(
        !marker.path("ran").exists(),
        "config check must not run the program"
    );

    for (bad, code) in [
        ("cmd:pass show x | head -n1", "config.invalid_cmd"),
        ("cmd:./script", "config.invalid_cmd"),
        ("cmd:/no/such/program", "config.secret_cmd_missing"),
    ] {
        let config = cfg("http://127.0.0.1:9", bad);
        let r = tokio::task::spawn_blocking(move || run(&["config", "check"], &config, b""))
            .await
            .unwrap();
        assert_ne!(r.code, 0, "{bad}");
        assert!(r.stderr.contains(code), "{bad}: {}", r.stderr);
        assert!(r.stderr.contains("providers.mock.settings.api_key"));
    }
}

#[tokio::test]
async fn a_key_written_in_the_line_is_warned_about_and_never_shown() {
    let key = "sk-abcdef0123456789abcdef";
    let config = cfg("http://127.0.0.1:9", &format!("cmd:/bin/echo {key}"));
    let c2 = config.clone();
    let check = tokio::task::spawn_blocking(move || run(&["config", "check"], &c2, b""))
        .await
        .unwrap();
    assert_eq!(check.code, 0, "{}", check.stderr);
    assert!(
        check.stdout.contains("looks like a key"),
        "{}",
        check.stdout
    );
    assert!(!check.stdout.contains(key) && !check.stderr.contains(key));

    let show = tokio::task::spawn_blocking(move || run(&["config", "show"], &config, b""))
        .await
        .unwrap();
    assert_eq!(show.code, 0, "{}", show.stderr);
    assert!(show.stdout.contains("/bin/echo"), "{}", show.stdout);
    assert!(!show.stdout.contains(key), "{}", show.stdout);
}

#[tokio::test]
async fn config_show_prints_an_ordinary_line_as_written() {
    let config = cfg(
        "http://127.0.0.1:9",
        "cmd:/bin/sh -c 'pass show api/openai'",
    );
    let show = tokio::task::spawn_blocking(move || run(&["config", "show"], &config, b""))
        .await
        .unwrap();
    assert_eq!(show.code, 0, "{}", show.stderr);
    assert!(
        show.stdout.contains("pass show api/openai"),
        "{}",
        show.stdout
    );
}

#[tokio::test]
async fn doctor_runs_the_program_once_and_says_whether_it_worked() {
    let marker = Scratch::new("doctor");
    let line = format!(
        "cmd:/bin/sh -c 'echo sk-doctor-key; echo x >> {}'",
        marker.str("count")
    );
    let config = cfg("http://127.0.0.1:9", &line);
    let r = tokio::task::spawn_blocking(move || run(&["doctor"], &config, b""))
        .await
        .unwrap();
    assert!(
        r.stdout.contains("`cmd:` secret works"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert!(!r.stdout.contains("sk-doctor-key") && !r.stderr.contains("sk-doctor-key"));
    let runs = std::fs::read_to_string(marker.path("count")).unwrap();
    assert_eq!(runs.lines().count(), 1, "doctor should run it once");

    let config = cfg("http://127.0.0.1:9", "cmd:/bin/sh -c 'exit 2'");
    let r = tokio::task::spawn_blocking(move || run(&["doctor"], &config, b""))
        .await
        .unwrap();
    assert!(
        r.stdout.contains("config.secret_cmd_failed"),
        "{}",
        r.stdout
    );
}
