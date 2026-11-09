//! MVP success criterion 3: nothing sensitive reaches a remote provider unredacted, and placeholders
//! are restored locally. Verified against a recording mock provider, through the real binary.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;

const SECRETS: &[&str] = &[
    "sk-live-0123456789abcdefghijKLMN",
    "hunter2hunter2",
    "s3cr3t-db-pass",
    "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
];

fn secret_envelope(action: &str) -> Vec<u8> {
    envelope_with_history(
        action,
        "send a request with OPENAI_API_KEY=sk-live-0123456789abcdefghijKLMN to my server",
        &[
            (
                "0",
                "git clone https://alice:hunter2hunter2@git.example.com/x.git",
            ),
            ("1", "mysql --password s3cr3t-db-pass -h db"),
            (
                "0",
                "export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            ),
        ],
    )
}

/// Answers with a command that uses the first placeholder the provider received.
fn echo_first_placeholder(raw: &str) -> String {
    let ph = raw
        .split("<SECRET:")
        .nth(1)
        .and_then(|s| s.split('>').next())
        .map(|t| format!("<SECRET:{t}>"))
        .unwrap_or_else(|| "NO-PLACEHOLDER".into());
    completion(&format!("Here you go.\n```shellmate-command\ncurl -H \"Authorization: {ph}\" https://example.com\n```"))
}

#[tokio::test]
async fn secrets_in_buffer_and_history_never_reach_a_remote_provider() {
    let server = MockHttpServer::start_dynamic(200, echo_first_placeholder).await;
    let cfg = remote_config(&server.url(), "");
    let r =
        tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &secret_envelope("generate")))
            .await
            .unwrap();
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);

    let wire = server.last_request().await;
    for s in SECRETS {
        assert!(
            !wire.contains(s),
            "secret `{s}` leaked to the provider:\n{wire}"
        );
    }
    assert!(
        wire.contains("<SECRET:"),
        "expected placeholders on the wire"
    );
    assert!(
        wire.contains("git clone") && wire.contains("mysql"),
        "level 2 history should still be sent (redacted)"
    );

    // placeholders are put back locally: the suggested command is usable and contains the original value
    assert!(
        SECRETS.iter().any(|s| r.stdout.contains(s)),
        "response not restored: {}",
        r.stdout
    );
    assert!(
        !r.stdout.contains("<SECRET:"),
        "placeholder left in the buffer: {}",
        r.stdout
    );
}

#[tokio::test]
async fn dry_run_shows_placeholders_never_the_secrets() {
    let server = MockHttpServer::start(200, &completion("unused")).await;
    let cfg = remote_config(&server.url(), "");
    let r = tokio::task::spawn_blocking(move || {
        run(&["invoke", "--dry-run"], &cfg, &secret_envelope("ask"))
    })
    .await
    .unwrap();
    assert_eq!(r.code, 10);
    for s in SECRETS {
        assert!(!r.stderr.contains(s), "dry-run printed `{s}`");
    }
    assert!(
        r.stderr.contains("placeholder(s) in the payload") && r.stderr.contains("<SECRET:"),
        "{}",
        r.stderr
    );
    assert_eq!(server.request_count().await, 0);
}

#[tokio::test]
async fn a_remote_provider_without_redaction_is_refused_before_any_request() {
    let server = MockHttpServer::start(200, &completion("unused")).await;
    let cfg = remote_config(
        &server.url(),
        "[plugins.\"builtin:redact\"]\nenabled = false\n",
    );
    let (c2, e2) = (cfg.clone(), secret_envelope("ask"));
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &c2, &e2))
        .await
        .unwrap();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("privacy.no_redaction"), "{}", r.stderr);
    assert_eq!(server.request_count().await, 0, "nothing may be sent");
    // and `config check` catches it up front
    let c = tokio::task::spawn_blocking(move || run(&["config", "check"], &cfg, b""))
        .await
        .unwrap();
    assert_eq!(c.code, 1);
    assert!(c.stderr.contains("privacy.no_redaction"));
}

#[tokio::test]
async fn local_providers_may_run_without_redaction() {
    let server = MockHttpServer::start(200, &completion("fine")).await;
    let cfg = format!(
        "{}[plugins.\"builtin:redact\"]\nenabled = false\n",
        config(&server.url())
    );
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &envelope("ask", "hello")))
        .await
        .unwrap();
    assert_eq!(r.code, 10, "stderr: {}", r.stderr);
    assert!(r.stderr.contains("fine"));
}

#[tokio::test]
async fn context_level_caps_what_a_provider_gets() {
    let server = MockHttpServer::start(200, &completion("ok")).await;
    // remote default max level is 1: no history even though default_level = 2
    let cfg = remote_config(&server.url(), "").replace("max_context_level = 2\n", "");
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &secret_envelope("ask")))
        .await
        .unwrap();
    assert_eq!(r.code, 10, "stderr: {}", r.stderr);
    let wire = server.last_request().await;
    assert!(
        !wire.contains("git clone") && !wire.contains("mysql"),
        "history sent to a level-1 provider:\n{wire}"
    );
}
