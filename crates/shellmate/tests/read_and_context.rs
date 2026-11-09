//! The read tool and the project/git collectors through the real binary.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-rc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn env_in(d: &std::path::Path, action: &str, buffer: &str) -> Vec<u8> {
    format!(
        "v=1\0action={action}\0shell=bash\0cwd={}\0cursor=0\0buffer={buffer}\0",
        d.display()
    )
    .into_bytes()
}

/// The model asks to read `file` until the file's text is in the conversation, then answers.
async fn model_reading(file: &'static str) -> MockHttpServer {
    MockHttpServer::start_dynamic(200, move |req| {
        if req.contains("Contents of") || req.contains("was denied") {
            completion("Done.\n```shellmate-command\nls\n```")
        } else {
            completion(&format!("```shellmate-tool\nread {file}\n```"))
        }
    })
    .await
}

#[tokio::test]
async fn a_pre_approved_file_is_read_and_its_text_reaches_the_model() {
    let d = dir("pre");
    std::fs::write(d.join("notes.txt"), "the marker line\n").unwrap();
    let llm = model_reading("notes.txt").await;
    let cfg = format!(
        "{}\n[tools.read]\npreapproved = [\"notes.txt\"]\n",
        config(&llm.url())
    );
    let e = env_in(&d, "ask", "summarize it");
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &e))
        .await
        .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "ls");
    assert!(
        r.stderr.contains("read (pre-approved: notes.txt)"),
        "{}",
        r.stderr
    );
    assert!(llm.last_request().await.contains("the marker line"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn without_a_terminal_or_a_rule_nothing_is_read() {
    let d = dir("noterm");
    std::fs::write(d.join("notes.txt"), "the marker line\n").unwrap();
    let llm = model_reading("notes.txt").await;
    let e = env_in(&d, "ask", "summarize it");
    let cfg = config(&llm.url());
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &e))
        .await
        .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    let last = llm.last_request().await;
    assert!(last.contains("no terminal"), "{last}");
    assert!(!last.contains("the marker line"));
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn credentials_are_never_read_even_under_a_match_everything_rule() {
    let d = dir("cred");
    std::fs::write(d.join(".env"), "API_TOKEN=hunter2hunter2\n").unwrap();
    let llm = model_reading(".env").await;
    let cfg = format!(
        "{}\n[tools.read]\npreapproved = [\"*\"]\n",
        config(&llm.url())
    );
    let e = env_in(&d, "ask", "what is in .env?");
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &e))
        .await
        .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(!llm.last_request().await.contains("hunter2"));
    assert!(r.stderr.contains("denied: read .env"), "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn config_check_warns_about_match_everything_read_rules_and_rejects_bad_ones() {
    let c = format!(
        "{}\n[tools.read]\npreapproved = [\"*\"]\n",
        config("http://127.0.0.1:1")
    );
    let r = run(&["config", "check"], &c, b"");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("warning: tools.read.preapproved \"*\""),
        "{}",
        r.stdout
    );
    for bad in [
        "[tools.read]\nroots = [\"relative/dir\"]\n",
        "[tools.read]\nmax_bytes = 0\n",
        "[context]\ngit_log_lines = 99\n",
    ] {
        let c = format!("{}\n{bad}", config("http://127.0.0.1:1"));
        assert_ne!(run(&["config", "check"], &c, b"").code, 0, "{bad}");
    }
}

#[test]
fn the_project_and_git_sections_follow_the_context_level() {
    let git = |d: &std::path::Path, args: &[&str]| {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(d)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false"
            ])
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false))
    };
    if !have_tool("git") {
        return;
    }
    let d = dir("ctx");
    std::fs::write(d.join("go.mod"), "module x\n").unwrap();
    git(&d, &["init", "-q", "-b", "trunk"]);
    std::fs::write(d.join("new.txt"), "n").unwrap();
    for (level, project, branch) in [(0u8, false, false), (1, true, false), (2, true, true)] {
        let c = format!(
            "{}\n[context]\ndefault_level = {level}\n",
            config("http://127.0.0.1:1")
        );
        let r = run(
            &["invoke", "--dry-run"],
            &c,
            &env_in(&d, "ask", "what now?"),
        );
        assert_eq!(r.code, 10, "{}", r.stderr);
        assert_eq!(
            r.stderr.contains("Go"),
            project,
            "level {level}: {}",
            r.stderr
        );
        assert_eq!(r.stderr.contains("branch `trunk`"), branch, "level {level}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Tool calls must be announced whatever the display preference: a pre-approved call runs without a
/// prompt, so the line under the prompt is the only way the user learns it happened.
#[tokio::test]
async fn tool_calls_are_announced_even_when_streaming_is_off() {
    let d = dir("nostream");
    std::fs::write(d.join("notes.txt"), "the marker line\n").unwrap();
    let llm = model_reading("notes.txt").await;
    for stream in [true, false] {
        let cfg = format!(
            "{}\n[ui]\nstream = {stream}\n[tools.read]\npreapproved = [\"notes.txt\"]\n",
            config(&llm.url())
        );
        let e = env_in(&d, "ask", "summarize it");
        let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &e))
            .await
            .unwrap();
        assert_eq!(r.code, 0, "stream={stream}: {}", r.stderr);
        assert!(
            r.stderr.contains("read (pre-approved: notes.txt)"),
            "stream={stream}: the read was not announced: {}",
            r.stderr
        );
        assert_eq!(r.stdout, "ls", "stream={stream}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn a_pre_approved_exec_and_a_refusal_are_announced_with_streaming_off() {
    let d = dir("nostream-exec");
    let llm = MockHttpServer::start_dynamic(200, |req| {
        if req.contains("Result of `") || req.contains("was denied") {
            completion("Done.\n```shellmate-command\nls\n```")
        } else if req.contains("exit status 0") {
            completion("Done.")
        } else {
            completion("```shellmate-tool\nexec echo announced-run\n```")
        }
    })
    .await;
    let cfg = format!(
        "{}\n[ui]\nstream = false\n[tools.exec]\npreapproved = [\"echo *\"]\n",
        config(&llm.url())
    );
    let e = env_in(&d, "ask", "say hi");
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &e))
        .await
        .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stderr
            .contains("ran (pre-approved: echo *): echo announced-run"),
        "{}",
        r.stderr
    );
    // and a refusal is announced too
    let refused = MockHttpServer::start_dynamic(200, |req| {
        if req.contains("was denied") {
            completion("Done.\n```shellmate-command\nls\n```")
        } else {
            completion("```shellmate-tool\nexec ls | wc -l\n```")
        }
    })
    .await;
    let cfg = format!("{}\n[ui]\nstream = false\n", config(&refused.url()));
    let e = env_in(&d, "ask", "count");
    let r = tokio::task::spawn_blocking(move || run(&["invoke"], &cfg, &e))
        .await
        .unwrap();
    assert!(r.stderr.contains("denied: exec ls"), "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&d);
}
