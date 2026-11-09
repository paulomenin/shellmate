//! Colour and markdown through the real binary: what the terminal gets, and what the shell's buffer (stdout)
//! never gets.

mod common;

use common::*;
use shellmate_testing::{MockHttpServer, MockSseServer, SseChunk};

/// `s` without its styles (`ESC [ digits and ; m`).
fn strip(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\u{1b}' && b.get(i + 1) == Some(&'[') {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == ';') {
                j += 1;
            }
            if b.get(j) == Some(&'m') {
                i = j + 1;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

const BOLD_ANSWER: &str = "Use **du** with `sort`.\n```shellmate-command\nls\n```";

async fn answer(reply: &str, extra_cfg: &str, envs: &[(&str, &str)]) -> Run {
    let server = MockHttpServer::start(200, &completion(reply)).await;
    invoke_on(&config(&server.url()), extra_cfg, envs, "ask").await
}

async fn invoke_on(cfg: &str, extra_cfg: &str, envs: &[(&str, &str)], action: &str) -> Run {
    let cfg = format!("{cfg}{extra_cfg}");
    let envs: Vec<(String, String)> = envs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let stdin = envelope(action, "how");
    tokio::task::spawn_blocking(move || {
        let refs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        run_with(&["invoke"], &cfg, &stdin, &refs)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn without_a_terminal_the_answer_is_plain_text_with_its_markers() {
    let r = answer(BOLD_ANSWER, "", &[]).await;
    assert_eq!(r.stderr, "\r\u{1b}[KUse **du** with `sort`.\n");
    assert_eq!(r.stdout, "ls");
}

#[tokio::test]
async fn forced_colour_formats_the_markdown_and_never_touches_the_buffer() {
    let r = answer(BOLD_ANSWER, "", &[("CLICOLOR_FORCE", "1")]).await;
    assert_eq!(strip(&r.stderr), "\r\u{1b}[KUse du with sort.\n");
    assert!(r.stderr.contains("\u{1b}[1mdu\u{1b}[0m"), "{:?}", r.stderr);
    assert!(
        r.stderr.contains("\u{1b}[35msort\u{1b}[0m"),
        "{:?}",
        r.stderr
    );
    assert_eq!(r.stdout, "ls", "the buffer for the shell carries no style");
    assert_eq!(r.code, 0);
}

#[tokio::test]
async fn no_color_removes_the_colour_request_of_a_forced_environment() {
    let r = answer(
        BOLD_ANSWER,
        "",
        &[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")],
    )
    .await;
    assert_eq!(r.stderr, "\r\u{1b}[KUse **du** with `sort`.\n");
}

#[tokio::test]
async fn the_config_decides_over_the_environment() {
    let always = answer(BOLD_ANSWER, "[ui]\ncolor = \"always\"\n", &[]).await;
    assert!(always.stderr.contains("\u{1b}[1mdu"), "{:?}", always.stderr);
    let never = answer(
        BOLD_ANSWER,
        "[ui]\ncolor = \"never\"\n",
        &[("CLICOLOR_FORCE", "1")],
    )
    .await;
    assert_eq!(never.stderr, "\r\u{1b}[KUse **du** with `sort`.\n");
}

#[tokio::test]
async fn markdown_can_be_turned_off_while_keeping_colour() {
    let r = answer(
        BOLD_ANSWER,
        "[ui]\ncolor = \"always\"\nmarkdown = false\n",
        &[],
    )
    .await;
    assert_eq!(strip(&r.stderr), "\r\u{1b}[KUse **du** with `sort`.\n");
    assert!(!r.stderr.contains("\u{1b}[1mdu"), "{:?}", r.stderr);
}

#[tokio::test]
async fn an_error_is_red_and_says_the_same_as_before() {
    let server = MockHttpServer::start(401, r#"{"error":{"message":"bad key"}}"#).await;
    let r = invoke_on(
        &config(&server.url()),
        "[ui]\ncolor = \"always\"\n",
        &[],
        "ask",
    )
    .await;
    assert!(r.stderr.contains("\u{1b}[31m"), "{:?}", r.stderr);
    assert_eq!(
        strip(&r.stderr),
        "✘ provider.auth: HTTP 401: bad key\n↳ check the API key reference in the provider settings\n"
    );
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout, "");
}

#[tokio::test]
async fn a_refused_tool_call_is_a_yellow_notice() {
    let server = MockHttpServer::start_dynamic(200, |raw: &str| {
        if raw.contains("was denied") {
            completion("Done.\n```shellmate-command\nls\n```")
        } else {
            completion("Looking.\n```shellmate-tool\nexec ls\n```")
        }
    })
    .await;
    let r = invoke_on(
        &config(&server.url()),
        "[ui]\ncolor = \"always\"\n",
        &[],
        "ask",
    )
    .await;
    assert!(r.stderr.contains("\u{1b}[33m>"), "{:?}", r.stderr);
    assert_eq!(
        strip(&r.stderr),
        "\r\u{1b}[KLooking.\n> denied: exec ls\nDone.\n"
    );
}

#[tokio::test]
async fn a_span_split_across_streamed_pieces_is_still_formatted() {
    let server = MockSseServer::start(vec![
        SseChunk::delta(0, "Use **bo"),
        SseChunk::delta(0, "ld** and `co"),
        SseChunk::delta(0, "de` now\n"),
        SseChunk::done(),
    ])
    .await;
    let r = invoke_on(
        &config(&server.url()),
        "[ui]\ncolor = \"always\"\n",
        &[],
        "explain",
    )
    .await;
    assert_eq!(strip(&r.stderr), "\r\u{1b}[KUse bold and code now\n");
    assert!(
        r.stderr.contains("\u{1b}[1mbold\u{1b}[0m"),
        "{:?}",
        r.stderr
    );
    assert!(
        r.stderr.contains("\u{1b}[35mcode\u{1b}[0m"),
        "{:?}",
        r.stderr
    );
}

fn render(args: &[&str], input: &str) -> Run {
    let mut a = vec!["render"];
    a.extend_from_slice(args);
    run_with(
        &a,
        "version = 1\n",
        input.as_bytes(),
        &[("LANG", "en_US.UTF-8")],
    )
}

#[test]
fn render_shows_markdown_the_way_an_answer_is_shown() {
    let md = "# Title\n\n- **one**\n- two\n";
    let colour = render(&["--color", "16", "--width", "30"], md);
    assert_eq!(strip(&colour.stdout), "Title\n\n• one\n• two\n");
    assert!(colour.stdout.contains("\u{1b}[1m"), "{:?}", colour.stdout);
    let mono = render(&["--color", "mono"], md);
    assert!(
        mono.stdout.contains("\u{1b}[1m") && !mono.stdout.contains("\u{1b}[3"),
        "{:?}",
        mono.stdout
    );
    let plain = render(&["--color", "plain"], md);
    assert_eq!(
        plain.stdout,
        "# Title\n\n- one\n- two\n".replace("one", "**one**")
    );
    let ascii = render(&["--color", "16", "--ascii"], md);
    assert_eq!(strip(&ascii.stdout), "Title\n\n- one\n- two\n");
    let raw = render(&["--color", "16", "--no-markdown"], md);
    assert_eq!(strip(&raw.stdout), md);
}

#[test]
fn render_cannot_be_made_to_write_an_escape_of_the_input() {
    let r = render(&["--color", "16"], "x \u{1b}[2J \u{1b}]0;t\u{7} **b**\n");
    let text = strip(&r.stdout);
    assert!(
        !text.contains('\u{1b}') && !text.contains('\u{7}'),
        "{:?}",
        r.stdout
    );
    assert!(text.contains("\\u{1b}[2J"));
}

#[test]
fn render_says_so_when_it_cannot_read_the_file() {
    let r = run_with(
        &["render", "/nonexistent/sm-file.md"],
        "version = 1\n",
        b"",
        &[],
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("render.unreadable"), "{}", r.stderr);
}

// ---- the commands that print a report ----

fn cmd(args: &[&str], envs: &[(&str, &str)]) -> Run {
    run_with(args, &config("http://127.0.0.1:9"), b"", envs)
}

const FORCE: &[(&str, &str)] = &[("CLICOLOR_FORCE", "1"), ("TERM", "xterm")];
const PLAIN: &[(&str, &str)] = &[("TERM", "xterm")];

#[test]
fn check_colours_its_findings_by_severity_and_says_the_same_in_plain_text() {
    let plain = cmd(&["check", "--shell", "bash", "--", "rm -rf /"], PLAIN);
    assert!(!plain.stdout.contains('\u{1b}'), "{:?}", plain.stdout);
    let rich = cmd(&["check", "--shell", "bash", "--", "rm -rf /"], FORCE);
    assert_eq!(strip(&rich.stdout), plain.stdout);
    assert!(
        rich.stdout.contains("\u{1b}[31m"),
        "high risk is red: {:?}",
        rich.stdout
    );
    assert_eq!(rich.code, plain.code);

    let ok = cmd(&["check", "--shell", "bash", "--", "true"], FORCE);
    assert!(
        ok.stdout.contains("\u{1b}[32m"),
        "no findings is green: {:?}",
        ok.stdout
    );
    assert_eq!(strip(&ok.stdout), "no findings\n");

    let missing = cmd(
        &["check", "--shell", "bash", "--", "no-such-prog-xyz"],
        FORCE,
    );
    assert!(
        missing.stdout.contains("\u{1b}[33m"),
        "{:?}",
        missing.stdout
    );
}

#[test]
fn check_json_is_never_styled() {
    let r = cmd(
        &["check", "--shell", "bash", "--json", "--", "rm -rf /"],
        FORCE,
    );
    assert!(!r.stdout.contains('\u{1b}'), "{:?}", r.stdout);
}

#[test]
fn config_check_and_context_use_colour_only_when_asked_for() {
    for args in [
        &["config", "check"][..],
        &["context"][..],
        &["config", "show"][..],
    ] {
        let plain = cmd(args, PLAIN);
        let rich = cmd(args, FORCE);
        assert!(
            !plain.stdout.contains('\u{1b}'),
            "{args:?}: {:?}",
            plain.stdout
        );
        assert!(
            rich.stdout.contains('\u{1b}'),
            "{args:?}: {:?}",
            rich.stdout
        );
        let same_dir = |s: &str| s.replace(|c: char| c.is_ascii_digit(), "#");
        assert_eq!(
            same_dir(&strip(&rich.stdout)),
            same_dir(&plain.stdout),
            "{args:?}"
        );
        let off = cmd(
            args,
            &[
                ("CLICOLOR_FORCE", "1"),
                ("NO_COLOR", "1"),
                ("TERM", "xterm"),
            ],
        );
        assert!(
            !off.stdout.contains("\u{1b}[3"),
            "{args:?}: no colour: {:?}",
            off.stdout
        );
    }
}

#[test]
fn a_command_error_is_red_with_colour_and_plain_without() {
    let plain = cmd(&["conversation", "resume", "nope"], PLAIN);
    let rich = cmd(&["conversation", "resume", "nope"], FORCE);
    assert_ne!(plain.code, 0);
    assert!(!plain.stderr.contains('\u{1b}'), "{:?}", plain.stderr);
    assert_eq!(strip(&rich.stderr), plain.stderr);
}
