//! The answer appears while it is being generated, and Ctrl-C stops the request.

mod common;

use common::*;
use shellmate_testing::{MockSseServer, SseChunk};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn spawn(config_text: &str, stdin: &[u8]) -> (std::process::Child, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "sm-stream-{}-{:?}",
        std::process::id(),
        Instant::now()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.toml");
    std::fs::write(&cfg, config_text).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_shellmate"));
    cmd.arg("--config")
        .arg(&cfg)
        .arg("invoke")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    (child, dir)
}

/// Read stderr in a thread; returns a handle that reports the instant each chunk arrived.
fn watch(mut stderr: std::process::ChildStderr) -> std::sync::mpsc::Receiver<(Instant, String)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = tx.send((
                Instant::now(),
                String::from_utf8_lossy(&buf[..n]).into_owned(),
            ));
        }
    });
    rx
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_is_shown_before_the_answer_is_complete() {
    let chunks = vec![
        SseChunk::delta(0, "First part of the answer. "),
        SseChunk::delta(1500, "Second part."),
        SseChunk::done(),
    ];
    let server = MockSseServer::start(chunks).await;
    let cfg = config(&server.url());
    let (mut child, dir) =
        tokio::task::spawn_blocking(move || spawn(&cfg, &envelope("ask", "tell me something")))
            .await
            .unwrap();
    let started = Instant::now();
    let rx = watch(child.stderr.take().unwrap());
    let (mut first_seen, mut all) = (None, String::new());
    while let Ok((at, text)) = rx.recv_timeout(Duration::from_secs(10)) {
        if first_seen.is_none() && text.contains("First part") {
            first_seen = Some(at - started);
        }
        all.push_str(&text);
    }
    let status = child.wait().unwrap();
    let first = first_seen.expect("first part never shown");
    assert!(
        first < Duration::from_millis(1000),
        "first words took {first:?}: not streamed"
    );
    assert!(
        started.elapsed() > Duration::from_millis(1400),
        "the server held back the second part"
    );
    assert!(all.contains("Second part."), "{all}");
    assert_eq!(status.code(), Some(10));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigint_cancels_the_request_and_closes_the_connection() {
    let chunks: Vec<_> = (0..300).map(|_| SseChunk::delta(20, "word ")).collect();
    let server = MockSseServer::start(chunks).await;
    let cfg = config(&server.url());
    let (mut child, dir) =
        tokio::task::spawn_blocking(move || spawn(&cfg, &envelope("ask", "go on and on")))
            .await
            .unwrap();
    let pid = child.id() as i32;
    let rx = watch(child.stderr.take().unwrap());
    // wait for the first words, then interrupt
    let mut all = String::new();
    while !all.contains("word") {
        let (_, t) = rx.recv_timeout(Duration::from_secs(10)).expect("no output");
        all.push_str(&t);
    }
    unsafe { libc::kill(pid, libc::SIGINT) };
    while let Ok((_, t)) = rx.recv_timeout(Duration::from_secs(5)) {
        all.push_str(&t);
    }
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(10), "stderr: {all}");
    assert!(all.contains("[cancelled]"), "{all}");
    for _ in 0..100 {
        if server.client_disconnected() {
            std::fs::remove_dir_all(dir).ok();
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the provider connection stayed open after cancel");
}

#[tokio::test]
async fn command_blocks_are_not_echoed_but_land_in_the_buffer() {
    let text = "Biggest first.\n```shellmate-command\ndu -sh * | sort -rh\n```\nDone.";
    let chunks: Vec<_> = text
        .chars()
        .collect::<Vec<_>>()
        .chunks(5)
        .map(|c| SseChunk::delta(2, &c.iter().collect::<String>()))
        .chain([SseChunk::done()])
        .collect();
    let server = MockSseServer::start(chunks).await;
    let cfg = config(&server.url());
    let r = tokio::task::spawn_blocking(move || {
        run(&["invoke"], &cfg, &envelope("ask", "biggest files"))
    })
    .await
    .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "du -sh * | sort -rh");
    assert!(
        r.stderr.contains("Biggest first.") && r.stderr.contains("Done."),
        "{}",
        r.stderr
    );
    assert!(
        !r.stderr.contains("du -sh"),
        "the command block must not be echoed: {}",
        r.stderr
    );
    assert!(!r.stderr.contains("```"));
}
