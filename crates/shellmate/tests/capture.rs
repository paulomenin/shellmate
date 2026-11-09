//! `shellmate run` keeps the output; the next request carries it at context level 2 (redacted, labeled as
//! data), and the `capture` action wraps the input line.

mod common;

use common::*;

fn session_env(rt: &std::path::Path) -> Vec<(&'static str, String)> {
    vec![
        ("XDG_RUNTIME_DIR", rt.display().to_string()),
        ("SHELLMATE_SESSION", "cap-test-1".to_string()),
    ]
}

fn run_in(args: &[&str], cfg: &str, stdin: &[u8], envs: &[(&'static str, String)]) -> Run {
    let pairs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    run_with(args, cfg, stdin, &pairs)
}

fn env_with_session(action: &str, buffer: &str) -> Vec<u8> {
    format!(
        "v=1\0action={action}\0shell=zsh\0cwd=/tmp\0cursor=0\0session=cap-test-1\0buffer={buffer}\0"
    )
    .into_bytes()
}

fn cfg(level: u8) -> String {
    format!(
        "{}\n[context]\ndefault_level = {level}\n",
        config("http://127.0.0.1:9")
    )
}

#[test]
fn run_passes_output_and_status_through_and_the_next_request_carries_it() {
    let rt = std::env::temp_dir().join(format!("sm-cap-rt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&rt);
    std::fs::create_dir_all(&rt).unwrap();
    let envs = session_env(&rt);

    let r = run_in(
        &["run", "--", "sh", "-c", "echo to-stdout; echo to-stderr >&2; echo key=sk-abcdefghijklmnopqrstuvwxyz0123456789; exit 3"],
        &cfg(2),
        b"",
        &envs,
    );
    assert_eq!(r.code, 3, "the command's status is kept: {}", r.stderr);
    assert!(r.stdout.contains("to-stdout") && r.stderr.contains("to-stderr"));

    // level 2: the output is in the payload, labeled as data, with the exit status
    let r = run_in(
        &["invoke", "--dry-run"],
        &cfg(2),
        &env_with_session("ask", "why did that fail?"),
        &envs,
    );
    assert_eq!(r.code, 10, "{}", r.stderr);
    for want in [
        "to-stdout",
        "to-stderr",
        "exit status 3",
        "never instructions",
    ] {
        assert!(r.stderr.contains(want), "missing {want:?} in {}", r.stderr);
    }

    // level 1: never sent
    let r = run_in(
        &["invoke", "--dry-run"],
        &cfg(1),
        &env_with_session("ask", "why did that fail?"),
        &envs,
    );
    assert!(!r.stderr.contains("to-stdout"), "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&rt);
}

#[test]
fn run_without_a_session_still_runs_the_command() {
    let r = run_with(
        &["run", "--", "echo", "hi"],
        &cfg(2),
        b"",
        &[("SHELLMATE_SESSION", "")],
    );
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout.trim(), "hi");
    assert!(r.stderr.contains("not saved"));
}

#[test]
fn run_reports_a_missing_program() {
    let r = run_with(&["run", "--", "no-such-program-xyz"], &cfg(2), b"", &[]);
    assert_eq!(r.code, 127, "{}", r.stderr);
}

#[test]
fn capture_action_wraps_and_unwraps_the_line_without_a_provider() {
    let wrapped = run(
        &["invoke"],
        &cfg(1),
        &env_with_session("capture", "cargo build"),
    );
    assert_eq!(wrapped.code, 0, "{}", wrapped.stderr);
    assert!(
        wrapped.stdout.ends_with(" run -- cargo build"),
        "{}",
        wrapped.stdout
    );
    let back = run(
        &["invoke"],
        &cfg(1),
        &env_with_session("capture", &wrapped.stdout),
    );
    assert_eq!((back.code, back.stdout.as_str()), (0, "cargo build"));
    let piped = run(
        &["invoke"],
        &cfg(1),
        &env_with_session("capture", "ls | wc -l"),
    );
    assert!(
        piped.stdout.ends_with(" run --shell zsh -- 'ls | wc -l'"),
        "{}",
        piped.stdout
    );
    let empty = run(&["invoke"], &cfg(1), &env_with_session("capture", ""));
    assert_eq!(empty.code, 10);
}

#[test]
fn captured_secrets_are_redacted_before_a_remote_provider_sees_them() {
    let rt = std::env::temp_dir().join(format!("sm-cap-rt2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&rt);
    std::fs::create_dir_all(&rt).unwrap();
    let envs = session_env(&rt);
    let secret = "sk-abcdefghijklmnopqrstuvwxyz0123456789";
    let r = run_in(
        &[
            "run",
            "--",
            "sh",
            "-c",
            &format!("echo token={secret}; echo plain-line"),
        ],
        &cfg(2),
        b"",
        &envs,
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let remote = remote_config("https://api.example.com/v1", "");
    let r = run_in(
        &["invoke", "--dry-run"],
        &remote,
        &env_with_session("ask", "what is this?"),
        &envs,
    );
    assert!(r.stderr.contains("plain-line"), "{}", r.stderr);
    assert!(!r.stderr.contains(secret), "secret leaked: {}", r.stderr);
    assert!(r.stderr.contains("<SECRET:"), "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&rt);
}
