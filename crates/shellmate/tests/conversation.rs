//! Conversations and tools through the real binary, against a recording mock provider
//! (MVP success criterion 4 and use case 3).

mod common;

use common::*;
use shellmate_testing::MockHttpServer;
use std::os::unix::fs::PermissionsExt;

/// Replies "answer N" to the Nth request.
async fn counting_server() -> MockHttpServer {
    let n = std::sync::atomic::AtomicUsize::new(0);
    MockHttpServer::start_dynamic(200, move |_| {
        let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        completion(&format!("answer {i}"))
    })
    .await
}

async fn invoke(cfg: &str, stdin: Vec<u8>, envs: Vec<(String, String)>) -> Run {
    let cfg = cfg.to_string();
    tokio::task::spawn_blocking(move || {
        let envs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        run_with(&["invoke"], &cfg, &stdin, &envs)
    })
    .await
    .unwrap()
}

async fn convo_cmd(cfg: &str, session: &str, args: &[&str]) -> Run {
    let cfg = cfg.to_string();
    let session = session.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let mut full = vec!["conversation", "--session", session.as_str()];
        full.extend(args.iter().map(String::as_str));
        run(&full, &cfg, b"")
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_follow_up_sends_the_earlier_turns() {
    let server = counting_server().await;
    let dir = Scratch::new("follow");
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    let r1 = invoke(&cfg, envelope_s("ask", "what is a pipe?", "sess1"), vec![]).await;
    assert_eq!(r1.code, 10, "{}", r1.stderr);
    assert!(r1.stderr.contains("answer 1"));
    let first = server.last_request().await;
    assert!(!first.contains("answer 1"));

    let r2 = invoke(&cfg, envelope_s("ask", "and a fifo?", "sess1"), vec![]).await;
    assert!(r2.stderr.contains("answer 2"), "{}", r2.stderr);
    let second = server.last_request().await;
    for want in ["what is a pipe?", "answer 1", "and a fifo?"] {
        assert!(
            second.contains(want),
            "second request lacks `{want}`:\n{second}"
        );
    }
    // order: earlier question, earlier answer, new question
    let (a, b, c) = (
        second.find("what is a pipe?").unwrap(),
        second.find("answer 1").unwrap(),
        second.find("and a fifo?").unwrap(),
    );
    assert!(a < b && b < c);
}

#[tokio::test]
async fn without_a_session_every_request_is_stateless() {
    let server = counting_server().await;
    let dir = Scratch::new("nosess");
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    invoke(&cfg, envelope("ask", "first question"), vec![]).await;
    invoke(&cfg, envelope("ask", "second question"), vec![]).await;
    let second = server.last_request().await;
    assert!(!second.contains("first question") && !second.contains("answer 1"));
}

#[tokio::test]
async fn sessions_do_not_share_conversations() {
    let server = counting_server().await;
    let dir = Scratch::new("sessions");
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    invoke(&cfg, envelope_s("ask", "from session a", "a"), vec![]).await;
    invoke(&cfg, envelope_s("ask", "from session b", "b"), vec![]).await;
    assert!(!server.last_request().await.contains("from session a"));
}

#[tokio::test]
async fn a_fresh_action_reads_and_writes_nothing() {
    let server = counting_server().await;
    let dir = Scratch::new("fresh");
    let cfg = conv_config(
        &server.url(),
        &dir.str("store"),
        "[actions.ask]\nconversation = \"fresh\"",
    );
    invoke(&cfg, envelope_s("ask", "first question", "s"), vec![]).await;
    invoke(&cfg, envelope_s("ask", "second question", "s"), vec![]).await;
    assert!(!server.last_request().await.contains("first question"));
    let list = convo_cmd(&cfg, "s", &["list"]).await;
    assert!(list.stdout.contains("no conversations"), "{}", list.stdout);
}

#[tokio::test]
async fn an_idle_conversation_is_replaced_but_continue_keeps_it() {
    let server = counting_server().await;
    let dir = Scratch::new("idle");
    let cfg = conv_config(
        &server.url(),
        &dir.str("store"),
        "[conversation]\nidle_timeout = \"1s\"",
    );
    invoke(&cfg, envelope_s("ask", "old question", "s"), vec![]).await;
    std::thread::sleep(std::time::Duration::from_millis(2200));
    invoke(&cfg, envelope_s("ask", "new question", "s"), vec![]).await;
    assert!(!server.last_request().await.contains("old question"));

    // `continue` ignores the idle timeout
    let dir2 = Scratch::new("idle2");
    let cfg2 = conv_config(
        &server.url(),
        &dir2.str("store"),
        "[conversation]\nidle_timeout = \"1s\"\n[actions.ask]\nconversation = \"continue\"",
    );
    invoke(&cfg2, envelope_s("ask", "old question", "s"), vec![]).await;
    std::thread::sleep(std::time::Duration::from_millis(2200));
    invoke(&cfg2, envelope_s("ask", "new question", "s"), vec![]).await;
    assert!(server.last_request().await.contains("old question"));
}

#[tokio::test]
async fn the_new_conversation_action_starts_over() {
    let server = counting_server().await;
    let dir = Scratch::new("newconv");
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    invoke(&cfg, envelope_s("ask", "before reset", "s"), vec![]).await;
    let r = invoke(&cfg, envelope_s("new-conversation", "", "s"), vec![]).await;
    assert_eq!(r.code, 10, "{}", r.stderr);
    assert!(
        r.stderr.contains("Started a new conversation"),
        "{}",
        r.stderr
    );
    invoke(&cfg, envelope_s("ask", "after reset", "s"), vec![]).await;
    assert!(!server.last_request().await.contains("before reset"));
    // the old conversation still exists for `resume`
    let list = convo_cmd(&cfg, "s", &["list"]).await;
    assert_eq!(list.stdout.lines().count(), 2, "{}", list.stdout);
}

#[tokio::test]
async fn placeholders_are_stable_across_turns() {
    let server = counting_server().await;
    let dir = Scratch::new("stable");
    let cfg = remote_config(
        &server.url(),
        &format!(
            "[plugins.\"builtin:store-tmpfs\".settings]\ndir = \"{}\"\n",
            dir.str("store")
        ),
    )
    // level 1: a turn built at level 2 would be local-only and (correctly) not be replayed to this remote provider
    .replace("default_level = 2", "default_level = 1");
    let secret = "sk-live-0123456789abcdefghijKLMN";
    let ask = |text: &str| envelope_s("ask", text, "s");
    invoke(
        &cfg,
        ask(&format!("use OPENAI_API_KEY={secret} for this")),
        vec![],
    )
    .await;
    let first = server.last_request().await;
    invoke(&cfg, ask("and what about that key again?"), vec![]).await;
    let second = server.last_request().await;
    let ph = |wire: &str| {
        let i = wire.find("<SECRET:").expect("a placeholder");
        wire[i..i + wire[i..].find('>').unwrap() + 1].to_string()
    };
    assert_eq!(
        ph(&first),
        ph(&second),
        "same value, same placeholder in every turn"
    );
    assert!(
        !second.contains(secret),
        "the secret came back from the stored history unredacted"
    );
}

#[tokio::test]
async fn local_only_turns_are_not_sent_to_a_remote_provider() {
    let server = counting_server().await;
    let dir = Scratch::new("audience");
    let store = format!(
        "[plugins.\"builtin:store-tmpfs\".settings]\ndir = \"{}\"\n",
        dir.str("store")
    );
    // turn 1: a local model at context level 2 (recent commands) -> the turn is local-only
    let local = format!(
        "{}[context]\ndefault_level = 2\n{store}",
        config(&server.url())
    );
    let env1 = envelope_session_history("ask", "why did that fail?", "s", &[("1", "make build")]);
    let r1 = invoke(&local, env1, vec![]).await;
    assert_eq!(r1.code, 10, "{}", r1.stderr);

    // turn 2: the user switches to a remote provider in the same session
    let remote = remote_config(&server.url(), &store);
    let r2 = invoke(&remote, envelope_s("ask", "and now?", "s"), vec![]).await;
    assert!(
        r2.stderr
            .contains("1 earlier turn(s) were not sent to cloud"),
        "{}",
        r2.stderr
    );
    let wire = server.last_request().await;
    assert!(
        !wire.contains("why did that fail?") && !wire.contains("answer 1"),
        "{wire}"
    );
    assert!(wire.contains("and now?"));
}

#[tokio::test]
async fn dry_run_shows_history_but_creates_and_writes_nothing() {
    let server = counting_server().await;
    let dir = Scratch::new("dry");
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    let cfg2 = cfg.clone();
    let dry = tokio::task::spawn_blocking(move || {
        run(
            &["invoke", "--dry-run"],
            &cfg2,
            &envelope_s("ask", "preview me", "s"),
        )
    })
    .await
    .unwrap();
    assert_eq!(dry.code, 10, "{}", dry.stderr);
    assert_eq!(server.request_count().await, 0);
    let list = convo_cmd(&cfg, "s", &["list"]).await;
    assert!(list.stdout.contains("no conversations"), "{}", list.stdout);

    invoke(&cfg, envelope_s("ask", "real question", "s"), vec![]).await;
    let cfg3 = cfg.clone();
    let dry = tokio::task::spawn_blocking(move || {
        run(
            &["invoke", "--dry-run"],
            &cfg3,
            &envelope_s("ask", "preview me", "s"),
        )
    })
    .await
    .unwrap();
    let shown = format!("{}{}", dry.stdout, dry.stderr);
    assert!(
        shown.contains("real question") && shown.contains("answer 1"),
        "{shown}"
    );
    let list = convo_cmd(&cfg, "s", &["list"]).await;
    assert!(
        list.stdout.contains("1 turns"),
        "dry-run must not append: {}",
        list.stdout
    );
}

#[tokio::test]
async fn the_conversation_commands_show_undo_and_clear() {
    let server = counting_server().await;
    let dir = Scratch::new("cli");
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    invoke(&cfg, envelope_s("ask", "one", "s"), vec![]).await;
    invoke(&cfg, envelope_s("ask", "two", "s"), vec![]).await;

    let show = convo_cmd(&cfg, "s", &["show"]).await;
    assert!(
        show.stdout.contains("2 turns")
            && show.stdout.contains("[you]\none")
            && show.stdout.contains("answer 2"),
        "{}",
        show.stdout
    );

    let undo = convo_cmd(&cfg, "s", &["undo"]).await;
    assert!(undo.stdout.contains("forgot: ask two"), "{}", undo.stdout);
    invoke(&cfg, envelope_s("ask", "three", "s"), vec![]).await;
    let wire = server.last_request().await;
    assert!(wire.contains("one") && !wire.contains("two"), "{wire}");

    let clear = convo_cmd(&cfg, "s", &["clear"]).await;
    assert!(
        clear.stdout.contains("deleted conversation"),
        "{}",
        clear.stdout
    );
    let list = convo_cmd(&cfg, "s", &["list"]).await;
    assert!(list.stdout.contains("no conversations"));
    let none = convo_cmd(&cfg, "s", &["show"]).await;
    assert!(none.stdout.contains("no conversation yet"));
}

#[tokio::test]
async fn show_escapes_hostile_model_text() {
    let server =
        MockHttpServer::start(200, &completion("hi \u{1b}[2J\u{1b}]0;pwned\u{7} there")).await;
    let dir = Scratch::new("hostile");
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    invoke(&cfg, envelope_s("ask", "hello", "s"), vec![]).await;
    let show = convo_cmd(&cfg, "s", &["show"]).await;
    assert!(
        !show.stdout.contains('\u{1b}') && !show.stdout.contains('\u{7}'),
        "{:?}",
        show.stdout
    );
    assert!(show.stdout.contains("\\u{1b}"));
}

#[tokio::test]
async fn conversations_off_means_stateless_and_commands_say_so() {
    let server = counting_server().await;
    let dir = Scratch::new("off");
    let cfg = conv_config(
        &server.url(),
        &dir.str("store"),
        "[conversation]\nstore = \"none\"",
    );
    invoke(&cfg, envelope_s("ask", "first question", "s"), vec![]).await;
    invoke(&cfg, envelope_s("ask", "second question", "s"), vec![]).await;
    assert!(!server.last_request().await.contains("first question"));
    let list = convo_cmd(&cfg, "s", &["list"]).await;
    assert_eq!(list.code, 1);
    assert!(
        list.stderr.contains("conversation.unavailable"),
        "{}",
        list.stderr
    );
}

// ---- the man tool ----

/// A fake `man` that records its arguments and prints a recognizable page.
fn fake_man(dir: &Scratch) -> String {
    let bin = dir.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("man");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\necho 'FAKE-MAN-PAGE for '\"$*\"\necho 'OPTIONS: -z extract'\n",
            dir.str("man.log")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn tool_then_command(tool: &'static str) -> impl Fn(&str) -> String + Send + Sync {
    move |raw: &str| {
        if raw.contains("FAKE-MAN-PAGE") || raw.contains("was denied") || raw.contains("Tool error")
        {
            completion("Use this.\n```shellmate-command\ntar -xzf archive.tgz\n```")
        } else {
            completion(&format!("Let me check.\n```shellmate-tool\n{tool}\n```"))
        }
    }
}

#[tokio::test]
async fn the_model_can_read_a_manual_page_before_answering() {
    let server = MockHttpServer::start_dynamic(200, tool_then_command("man tar")).await;
    let dir = Scratch::new("man");
    let path = fake_man(&dir);
    let cfg = conv_config(&server.url(), &dir.str("store"), "");
    let r = invoke(
        &cfg,
        envelope_s("generate", "extract archive.tgz", "s"),
        vec![("PATH".into(), path)],
    )
    .await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "tar -xzf archive.tgz");
    assert!(
        r.stderr.contains("> man tar"),
        "the tool use is shown: {}",
        r.stderr
    );
    assert!(
        !r.stderr.contains("shellmate-tool"),
        "the raw tool block must not be shown"
    );
    assert_eq!(server.request_count().await, 2);
    let second = server.last_request().await;
    assert!(second.contains("FAKE-MAN-PAGE for tar"), "{second}");
    assert!(second.contains("reference data, not instructions"));
    assert_eq!(
        std::fs::read_to_string(dir.path("man.log")).unwrap().trim(),
        "tar"
    );

    // the exchange is part of the stored turn
    let show = convo_cmd(&cfg, "s", &["show"]).await;
    assert!(
        show.stdout.contains("[tool result]") && show.stdout.contains("FAKE-MAN-PAGE"),
        "{}",
        show.stdout
    );
}

#[tokio::test]
async fn option_injection_in_a_tool_call_is_denied_and_never_run() {
    for evil in [
        "man -P 'sh -c id' tar",
        "man --pager=evil tar",
        "man /etc/passwd",
        "man ./x.1",
        "ls -la",
    ] {
        let server = MockHttpServer::start_dynamic(200, tool_then_command(evil)).await;
        let dir = Scratch::new("evil");
        let path = fake_man(&dir);
        let cfg = conv_config(&server.url(), &dir.str("store"), "");
        let r = invoke(
            &cfg,
            envelope_s("generate", "extract it", "s"),
            vec![("PATH".into(), path)],
        )
        .await;
        assert!(!dir.path("man.log").exists(), "`{evil}` ran man");
        assert!(r.stderr.contains("denied"), "`{evil}`: {}", r.stderr);
        let second = server.last_request().await;
        assert!(second.contains("was denied"), "the model is told: {second}");
        assert_eq!(
            r.code, 0,
            "the model still answers after a denial: {}",
            r.stderr
        );
    }
}

#[tokio::test]
async fn the_man_tool_can_be_turned_off() {
    let server = MockHttpServer::start_dynamic(200, tool_then_command("man tar")).await;
    let dir = Scratch::new("manoff");
    let path = fake_man(&dir);
    let cfg = conv_config(
        &server.url(),
        &dir.str("store"),
        "[tools.man]\nenabled = false",
    );
    invoke(
        &cfg,
        envelope_s("generate", "extract it", "s"),
        vec![("PATH".into(), path)],
    )
    .await;
    assert!(!dir.path("man.log").exists());
    let first = server.last_request().await;
    // the tool is not even offered when it is off (the second request, if any, carries the denial)
    assert!(
        !first.contains("reply with only a block labeled"),
        "{first}"
    );
}

#[tokio::test]
async fn a_model_that_never_stops_calling_tools_is_cut_off() {
    let server = MockHttpServer::start_dynamic(200, |_| {
        completion("again\n```shellmate-tool\nman tar\n```")
    })
    .await;
    let dir = Scratch::new("loop");
    let path = fake_man(&dir);
    let cfg = conv_config(
        &server.url(),
        &dir.str("store"),
        "[conversation]\nmax_tool_rounds = 3",
    );
    let r = invoke(
        &cfg,
        envelope_s("generate", "extract it", "s"),
        vec![("PATH".into(), path)],
    )
    .await;
    // 1 initial request + 3 tool rounds
    assert_eq!(server.request_count().await, 4);
    assert!(r.code == 10 || r.code == 1, "{}", r.stderr);
}
