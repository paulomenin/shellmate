//! Native tool calling through the real binary: a mock server that speaks the OpenAI tool protocol.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;
use std::os::unix::fs::PermissionsExt;

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-native-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn env_in(d: &std::path::Path, action: &str) -> Vec<u8> {
    format!(
        "v=1\0action={action}\0shell=bash\0cwd={}\0cursor=0\0buffer=extract it\0",
        d.display()
    )
    .into_bytes()
}

/// A `man` that prints a recognizable page, first on PATH.
fn fake_man(d: &std::path::Path) -> String {
    let bin = d.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("man");
    std::fs::write(&script, "#!/bin/sh\necho \"FAKE-MAN-PAGE for $*\"\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// The config with the mode of the model set.
fn with_mode(mode: &str, d: &std::path::Path, url: &str, extra: &str) -> String {
    format!(
        "{}\n[ui]\nstream = false\n{extra}",
        conv_config(url, &d.join("store").display().to_string(), "")
    )
    .replace(
        "tool_calling = \"text\"",
        &format!("tool_calling = \"{mode}\""),
    )
}

async fn invoke(cfg: String, stdin: Vec<u8>, envs: Vec<(String, String)>) -> Run {
    tokio::task::spawn_blocking(move || {
        let envs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        run_with(&["invoke"], &cfg, &stdin, &envs)
    })
    .await
    .unwrap()
}

/// The model calls `man tar` natively until the result is in the conversation, then answers.
async fn model_calling_man() -> MockHttpServer {
    MockHttpServer::start_dynamic(200, |raw| {
        if raw.contains("\"role\":\"tool\"") {
            completion("Use this.\n```shellmate-command\ntar -xzf archive.tgz\n```")
        } else {
            native_completion("call_1", "man", r#"{"page":"tar"}"#)
        }
    })
    .await
}

fn body(request: &str) -> serde_json::Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn a_native_model_is_sent_the_tools_and_its_call_is_run_and_answered() {
    let d = dir("man");
    let llm = model_calling_man().await;
    let cfg = with_mode("native", &d, &llm.url(), "");
    let r = invoke(
        cfg,
        env_in(&d, "generate"),
        vec![("PATH".into(), fake_man(&d))],
    )
    .await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "tar -xzf archive.tgz");
    assert!(
        r.stderr.contains("> man tar"),
        "the call is shown: {}",
        r.stderr
    );
    let requests = llm.requests().await;
    assert_eq!(
        requests.len(),
        2,
        "no request to list models in a pinned mode"
    );
    let first = body(&requests[0]);
    let names: Vec<&str> = first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["man", "help", "exec", "read"]);
    assert!(
        !first["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("shellmate-tool"),
        "the tools are not described in the prompt as well"
    );
    let second = body(&requests[1]);
    let messages = second["messages"].as_array().unwrap();
    let n = messages.len();
    assert_eq!(messages[n - 2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(messages[n - 2]["tool_calls"][0]["function"]["name"], "man");
    assert_eq!(messages[n - 1]["role"], "tool");
    assert_eq!(messages[n - 1]["tool_call_id"], "call_1");
    assert!(messages[n - 1]["content"]
        .as_str()
        .unwrap()
        .contains("FAKE-MAN-PAGE for tar"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn a_native_call_for_exec_asks_and_with_nobody_to_ask_it_is_denied() {
    let d = dir("exec");
    let llm = MockHttpServer::start_dynamic(200, |raw| {
        if raw.contains("\"role\":\"tool\"") {
            completion("Done.\n```shellmate-command\nls\n```")
        } else {
            native_completion("x", "exec", r#"{"command":"echo from-native"}"#)
        }
    })
    .await;
    // no terminal and no rule: denied, and the model is told
    let r = invoke(
        with_mode("native", &d, &llm.url(), ""),
        env_in(&d, "ask"),
        vec![],
    )
    .await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stderr.contains("denied: exec echo from-native"),
        "{}",
        r.stderr
    );
    let last = body(&llm.last_request().await);
    assert!(
        last["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("no terminal"),
    );
    // with a rule of the user's, the same call runs without asking
    let r = invoke(
        with_mode(
            "native",
            &d,
            &llm.url(),
            "[tools.exec]\npreapproved = [\"echo *\"]\n",
        ),
        env_in(&d, "ask"),
        vec![],
    )
    .await;
    assert!(
        r.stderr
            .contains("ran (pre-approved: echo *): echo from-native"),
        "{}",
        r.stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// A server whose model list says what it says about tools, and that otherwise answers as above.
async fn server_listing(supported: &'static str) -> MockHttpServer {
    MockHttpServer::start_dynamic(200, move |raw| {
        if raw.starts_with("GET /v1/models") {
            format!(r#"{{"data":[{{"id":"test-model"{supported}}}]}}"#)
        } else if raw.contains("\"role\":\"tool\"") {
            completion("Use this.\n```shellmate-command\ntar -xzf archive.tgz\n```")
        } else if raw.contains("\"tools\"") {
            native_completion("call_1", "man", r#"{"page":"tar"}"#)
        } else if raw.contains("```shellmate-tool") && !raw.contains("FAKE-MAN-PAGE") {
            completion("```shellmate-tool\nman tar\n```")
        } else {
            completion("Use this.\n```shellmate-command\ntar -xzf archive.tgz\n```")
        }
    })
    .await
}

fn gets(requests: &[String]) -> usize {
    requests
        .iter()
        .filter(|r| r.starts_with("GET /v1/models"))
        .count()
}

#[tokio::test]
async fn auto_asks_the_server_once_then_uses_the_native_form_it_found() {
    let d = dir("auto-yes");
    let llm = server_listing(r#","supported_parameters":["tools","temperature"]"#).await;
    let cache = d.join("cache");
    let envs = || {
        vec![
            ("PATH".to_string(), fake_man(&d)),
            ("XDG_CACHE_HOME".into(), cache.display().to_string()),
        ]
    };
    // `auto` is what a model gets when the config says nothing
    let cfg = with_mode("auto", &d, &llm.url(), "");
    let r = invoke(cfg.clone(), env_in(&d, "generate"), envs()).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "tar -xzf archive.tgz");
    let all = llm.requests().await;
    assert_eq!(gets(&all), 1);
    assert!(
        all.iter().any(|r| r.contains("\"tools\"")),
        "native was used"
    );
    // the next request, a new process, does not ask again
    let r = invoke(cfg, env_in(&d, "generate"), envs()).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(gets(&llm.requests().await), 1, "the answer was remembered");
    assert!(cache.join("shellmate/tool-support.json").exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn auto_stays_in_text_when_the_server_says_no_or_says_nothing_and_does_not_ask_again() {
    for (name, listing) in [
        ("no", r#","supported_parameters":["temperature"]"#),
        ("silent", ""),
    ] {
        let d = dir(&format!("auto-{name}"));
        let llm = server_listing(listing).await;
        let cache = d.join("cache");
        let envs = || {
            vec![
                ("PATH".to_string(), fake_man(&d)),
                ("XDG_CACHE_HOME".into(), cache.display().to_string()),
            ]
        };
        let cfg = with_mode("auto", &d, &llm.url(), "");
        for _ in 0..2 {
            let r = invoke(cfg.clone(), env_in(&d, "generate"), envs()).await;
            assert_eq!(r.code, 0, "{name}: {}", r.stderr);
            assert_eq!(r.stdout, "tar -xzf archive.tgz", "{name}");
        }
        let all = llm.requests().await;
        assert_eq!(gets(&all), 1, "{name}: asked once, remembered");
        assert!(
            !all.iter().any(|r| r.contains("\"tools\"")),
            "{name}: no request carried tools"
        );
        assert!(
            all.iter().any(|r| r.contains("```shellmate-tool")),
            "{name}: the tools were in the prompt"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[tokio::test]
async fn a_model_with_no_setting_is_in_auto() {
    // the config says nothing about tool_calling: the default is to ask, which is not what `text` does
    let d = dir("default");
    let llm = server_listing(r#","supported_parameters":["tools"]"#).await;
    let cfg = with_mode("text", &d, &llm.url(), "").replace("tool_calling = \"text\"\n", "");
    assert!(!cfg.contains("tool_calling"));
    let r = invoke(
        cfg,
        env_in(&d, "generate"),
        vec![("PATH".into(), fake_man(&d))],
    )
    .await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(gets(&llm.requests().await), 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn a_dry_run_sends_nothing_and_shows_the_tools_once_the_model_is_known_to_take_them() {
    let d = dir("dry");
    let llm = server_listing(r#","supported_parameters":["tools"]"#).await;
    let cache = d.join("cache");
    let envs = || {
        vec![
            ("PATH".to_string(), fake_man(&d)),
            ("XDG_CACHE_HOME".to_string(), cache.display().to_string()),
        ]
    };
    let cfg = with_mode("auto", &d, &llm.url(), "");
    // nothing is remembered yet, and a dry run sends nothing, not even the question: it shows the text form
    let dry = invoke_args(
        &cfg,
        &["invoke", "--dry-run"],
        env_in(&d, "generate"),
        envs(),
    )
    .await;
    assert_eq!(dry.code, 10, "{}", dry.stderr);
    assert_eq!(llm.request_count().await, 0);
    assert!(dry.stderr.contains("shellmate-tool") && !dry.stderr.contains("\"tools\""));
    // a real request finds out, and remembers
    let r = invoke(cfg.clone(), env_in(&d, "generate"), envs()).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    let before = llm.request_count().await;
    let dry = invoke_args(
        &cfg,
        &["invoke", "--dry-run"],
        env_in(&d, "generate"),
        envs(),
    )
    .await;
    assert_eq!(dry.code, 10, "{}", dry.stderr);
    assert_eq!(llm.request_count().await, before, "still nothing sent");
    assert!(
        dry.stderr.contains("\"tools\""),
        "the preview shows what would be sent: {}",
        dry.stderr
    );
    assert!(!dry.stderr.contains("```shellmate-tool"));
    let _ = std::fs::remove_dir_all(&d);
}

async fn invoke_args(cfg: &str, args: &[&str], stdin: Vec<u8>, envs: Vec<(String, String)>) -> Run {
    let cfg = cfg.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let envs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        run_with(&args, &cfg, &stdin, &envs)
    })
    .await
    .unwrap()
}
