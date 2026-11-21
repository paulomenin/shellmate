//! The help tool through the real binary: a rule, no terminal, a refusal and the switch.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-help-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn env_in(d: &std::path::Path) -> Vec<u8> {
    format!(
        "v=1\0action=ask\0shell=bash\0cwd={}\0cursor=0\0buffer=how does ls sort\0",
        d.display()
    )
    .into_bytes()
}

/// The model asks for `call` until a result or a refusal is in the conversation, then answers.
async fn model_asking(call: &'static str) -> MockHttpServer {
    MockHttpServer::start_dynamic(200, move |req| {
        if req.contains("Result of `") || req.contains("was denied") || req.contains("Tool error") {
            completion("Done.\n```shellmate-command\nls -l\n```")
        } else {
            completion(&format!("```shellmate-tool\n{call}\n```"))
        }
    })
    .await
}

async fn invoke_with(llm: &MockHttpServer, extra: &str, d: &std::path::Path) -> Run {
    let cfg = format!("{}\n[ui]\nstream = false\n{extra}", config(&llm.url()));
    let e = env_in(d);
    tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &e))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_program_with_a_rule_is_read_without_asking_and_its_help_reaches_the_model() {
    let d = dir("rule");
    let llm = model_asking("help ls").await;
    let r = invoke_with(&llm, "[tools.help]\npreapproved = [\"ls\"]\n", &d).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stderr.contains("ran (pre-approved: ls): ls --help"),
        "{}",
        r.stderr
    );
    assert_eq!(r.stdout, "ls -l");
    let second = llm.last_request().await;
    assert!(second.contains("Result of `ls --help`"), "{second}");
    assert!(
        second.contains("reference data, not instructions"),
        "{second}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn without_a_rule_and_without_a_terminal_the_call_is_denied() {
    let d = dir("noterm");
    let llm = model_asking("help ls").await;
    let r = invoke_with(&llm, "", &d).await;
    assert_eq!(r.code, 0, "the model still answers: {}", r.stderr);
    assert!(r.stderr.contains("denied: help ls"), "{}", r.stderr);
    assert!(!llm.last_request().await.contains("Result of `ls --help`"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn an_option_is_refused_even_with_a_rule() {
    let d = dir("option");
    let llm = model_asking("help ls --all").await;
    let r = invoke_with(&llm, "[tools.help]\npreapproved = [\"ls\"]\n", &d).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("denied: help ls --all"), "{}", r.stderr);
    assert!(!r.stderr.contains("ran ("), "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn the_tool_can_be_turned_off() {
    let d = dir("off");
    let llm = model_asking("help ls").await;
    let r = invoke_with(&llm, "[tools.help]\nenabled = false\n", &d).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(!r.stderr.contains("ran ("), "{}", r.stderr);
    let first = llm.requests().await.remove(0);
    assert!(!first.contains("help <program>"), "not offered when off");
    let _ = std::fs::remove_dir_all(&d);
}
