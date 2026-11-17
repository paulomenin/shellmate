#[tokio::test]
async fn an_approved_git_command_cannot_run_the_repositorys_fsmonitor() {
    if !crate::needs::have("git") {
        return;
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let git = crate::command::resolve_program("git", &path).unwrap();
    let d = std::env::temp_dir().join(format!("sm-exec-fsm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let g = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&d)
            .args(args)
            .output()
            .unwrap()
            .status
            .success())
    };
    g(&["init", "-q"]);
    let marker = d.join("PWNED");
    let hook = d.join("hook.sh");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    g(&["config", "core.fsmonitor", &hook.display().to_string()]);
    let prepared = crate::command::Prepared {
        program: git,
        argv: vec!["git".into(), "status".into()],
        cwd: Some(d.clone()),
    };
    for env in [
        shellmate_api::ProcessEnv::Sanitized,
        shellmate_api::ProcessEnv::Inherit,
    ] {
        let r = run_exec(
            &prepared,
            Duration::from_secs(10),
            4096,
            env,
            &crate::ports::SystemEnv,
        )
        .await;
        assert_eq!(r.exit, Some(0), "{}", r.stderr);
        assert!(
            !marker.exists(),
            "the repository's fsmonitor program ran ({env:?})"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

use super::*;
use std::time::Duration;

fn call(s: &str) -> ToolCall {
    ToolCall::parse(s).unwrap()
}

#[test]
fn parses_single_line_calls_only() {
    assert_eq!(call("man  tar").args, ["tar"]);
    assert!(ToolCall::parse("").is_err());
    assert!(ToolCall::parse("man a\nman b").is_err());
}

#[test]
fn man_output_is_cleaned() {
    let raw = "N\u{8}NA\u{8}AM\u{8}ME\u{8}E\n\n\n\n   tar - archive\u{1b}[1m  \n";
    assert_eq!(clean_man(raw), "NAME\n\n   tar - archive");
}

// ---- tests written from a mutation run: each pins behaviour a mutant could remove unnoticed ----

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `script` (shell text) through `sh`, as `exec` would run a program, with the real environment.
async fn exec_script(
    script: &str,
    timeout: Duration,
    cap: usize,
    env: shellmate_api::ProcessEnv,
) -> ExecResult {
    exec_script_with(script, timeout, cap, env, &crate::ports::SystemEnv).await
}

/// Like [`exec_script`], with `vars` as the environment the sanitized child is built from.
async fn exec_script_with(
    script: &str,
    timeout: Duration,
    cap: usize,
    env: shellmate_api::ProcessEnv,
    vars: &dyn crate::ports::Env,
) -> ExecResult {
    let prepared = crate::command::Prepared {
        program: "/bin/sh".into(),
        argv: vec!["sh".into(), "-c".into(), script.into()],
        cwd: None,
    };
    run_exec(&prepared, timeout, cap, env, vars).await
}

#[tokio::test]
async fn the_sanitized_environment_copies_only_the_allow_list_from_the_port() {
    use crate::ports::MapEnv;
    let vars = MapEnv::new()
        .with("PATH", "/usr/bin:/bin")
        .with("HOME", "/home/someone")
        .with("LANG", "C")
        .with("SM_TEST_SECRET_TOKEN", "hunter2-do-not-leak")
        .with("AWS_SECRET_ACCESS_KEY", "also-secret");
    let sanitized = exec_script_with(
        "env",
        Duration::from_secs(10),
        1 << 16,
        shellmate_api::ProcessEnv::Sanitized,
        &vars,
    )
    .await;
    let out = &sanitized.stdout;
    for hidden in [
        "SM_TEST_SECRET_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
        "hunter2",
        "also-secret",
    ] {
        assert!(!out.contains(hidden), "`{hidden}` reached the child: {out}");
    }
    for kept in [
        "PATH=/usr/bin:/bin",
        "HOME=/home/someone",
        "LANG=C",
        "TERM=dumb",
        "NO_COLOR=1",
        "PAGER=cat",
        "GIT_PAGER=cat",
        "GIT_TERMINAL_PROMPT=0",
        // the repository cannot name a program for git to run
        "GIT_CONFIG_KEY_0=core.fsmonitor",
        "GIT_CONFIG_VALUE_0=false",
    ] {
        assert!(out.contains(kept), "{kept} missing from {out}");
    }
    // a variable the port does not have is simply absent, not read from the machine
    assert!(!out.contains("USER="), "{out}");
}

#[tokio::test]
async fn the_inherited_environment_is_the_real_one_and_the_sanitized_one_is_not() {
    // `cargo test` sets this in every test process; it is not on the allow list.
    let marker = "CARGO_MANIFEST_DIR";
    let value = std::env::var(marker).expect("run the tests with cargo");
    let run = |env| exec_script("env", Duration::from_secs(10), 1 << 16, env);
    let inherited = run(shellmate_api::ProcessEnv::Inherit).await;
    assert!(inherited.stdout.contains(&format!("{marker}={value}")));
    let sanitized = run(shellmate_api::ProcessEnv::Sanitized).await;
    assert!(!sanitized.stdout.contains(marker), "{}", sanitized.stdout);
    // either way the repository cannot name a program for git to run
    assert!(inherited.stdout.contains("GIT_CONFIG_VALUE_0=false"));
    assert!(sanitized.stdout.contains("GIT_CONFIG_KEY_0=core.fsmonitor"));
}

/// True once the process is gone or is only a zombie (give the kernel a moment to reap it).
async fn gone(pid: libc::pid_t) -> bool {
    for _ in 0..100 {
        let alive = unsafe { libc::kill(pid, 0) } == 0
            && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map_or(true, |s| s.contains(") Z "));
        if !alive {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    false
}

/// The pid a script wrote to `file`, once it has written the whole line.
async fn pid_written_to(file: &std::path::Path) -> libc::pid_t {
    for _ in 0..500 {
        if let Ok(s) = std::fs::read_to_string(file) {
            if s.ends_with('\n') {
                return s.trim().parse().expect("a pid");
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the script never wrote {}", file.display());
}

#[tokio::test]
async fn a_timeout_stops_the_whole_process_group_not_just_the_leader() {
    let d = scratch("group");
    let pidfile = d.join("pid");
    let script = format!("sleep 60 & echo $! > {}; wait", pidfile.display());
    let r = exec_script(
        &script,
        Duration::from_millis(700),
        4096,
        shellmate_api::ProcessEnv::Sanitized,
    )
    .await;
    assert!(r.timed_out);
    let pid: libc::pid_t = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = !gone(pid).await;
    assert!(!alive, "the background process {pid} survived the timeout");
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn dropping_the_future_stops_the_whole_process_group() {
    let d = scratch("cancel");
    let pidfile = d.join("pid");
    let script = format!("sleep 300 & echo $! > {}; wait", pidfile.display());
    let pid = tokio::select! {
        _ = exec_script(
            &script,
            Duration::from_secs(120),
            4096,
            shellmate_api::ProcessEnv::Sanitized,
        ) => panic!("the command was not expected to finish"),
        // the background child is running once its pid is written; leaving this branch drops the other future
        pid = pid_written_to(&pidfile) => pid,
    };
    assert!(
        gone(pid).await,
        "the background process {pid} survived the cancellation"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn output_over_the_cap_is_cut_and_flagged_but_exactly_the_cap_is_not() {
    let at = exec_script(
        "printf aaaaaaaaaa",
        Duration::from_secs(10),
        10,
        shellmate_api::ProcessEnv::Sanitized,
    )
    .await;
    assert_eq!((at.stdout.len(), at.truncated), (10, false));
    let over = exec_script(
        "printf aaaaaaaaaaa",
        Duration::from_secs(10),
        10,
        shellmate_api::ProcessEnv::Sanitized,
    )
    .await;
    assert_eq!((over.stdout.len(), over.truncated), (10, true));
    let err = exec_script(
        "printf bbbbbbbbbbb >&2",
        Duration::from_secs(10),
        10,
        shellmate_api::ProcessEnv::Sanitized,
    )
    .await;
    assert_eq!((err.stderr.len(), err.truncated), (10, true));
    let err_at = exec_script(
        "printf bbbbbbbbbb >&2",
        Duration::from_secs(10),
        10,
        shellmate_api::ProcessEnv::Sanitized,
    )
    .await;
    assert_eq!((err_at.stderr.len(), err_at.truncated), (10, false));
}

#[test]
fn man_output_loses_control_characters_but_keeps_tabs_and_lines() {
    assert_eq!(clean_man("a\u{7}b\tc\u{1}\rd\ne"), "ab\tcd\ne");
    assert_eq!(clean_man("x\u{1b}[31;1mred\u{1b}[0my"), "xredy");
    // a lone escape is dropped, text after it stays
    assert_eq!(clean_man("a\u{1b}b"), "ab");
    // more than one blank line collapses to one; a single blank line stays
    assert_eq!(clean_man("a\n\nb\n\n\n\nc"), "a\n\nb\n\nc");
    assert_eq!(clean_man("  a  \n"), "a");
}

#[test]
fn terminal_codes_are_removed_from_file_text() {
    assert_eq!(strip_terminal_codes("a\u{1b}[31mred\u{1b}[0mb"), "aredb");
    assert_eq!(strip_terminal_codes("x\u{1b}]0;title\u{7}y"), "xy");
    assert_eq!(strip_terminal_codes("x\u{1b}]0;title\u{1b}\\y"), "xy");
    assert_eq!(strip_terminal_codes("a\u{1b}cb"), "ab");
    assert_eq!(strip_terminal_codes("l1\nl2\tt\u{0}\u{8}\r!"), "l1\nl2\tt!");
    assert_eq!(strip_terminal_codes("bell\u{7}"), "bell");
    // the sequence ends at its final byte only: ordinary text right after stays
    assert_eq!(strip_terminal_codes("\u{1b}[2Jtext"), "text");
}

#[test]
fn a_file_is_cut_at_the_limit_and_flagged_only_beyond_it() {
    let d = scratch("limit");
    let f = d.join("t.txt");
    std::fs::write(&f, "abcdefghij").unwrap();
    let t = ReadTarget {
        path: f.clone(),
        shown: "t.txt".into(),
        size: 10,
    };
    let at = read_file(&t, 10).unwrap();
    assert_eq!(
        (at.text.as_str(), at.truncated, at.bytes),
        ("abcdefghij", false, 10)
    );
    let over = read_file(&t, 9).unwrap();
    assert_eq!((over.text.as_str(), over.truncated), ("abcdefghi", true));
    std::fs::write(&f, b"ab\0cd").unwrap();
    assert!(read_file(&t, 100).unwrap_err().contains("binary"));
    // a directory is not a regular file
    let dt = ReadTarget {
        path: d.clone(),
        shown: "dir".into(),
        size: 0,
    };
    assert!(read_file(&dt, 10).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn paths_resolve_inside_the_roots_and_say_nothing_about_outside_ones() {
    let d = scratch("roots");
    let other = scratch("roots-other");
    std::fs::write(d.join("a.txt"), "x").unwrap();
    std::fs::write(other.join("b.txt"), "y").unwrap();
    std::fs::create_dir_all(d.join("sub")).unwrap();
    let cwd = d.display().to_string();
    let ok = prepare_read("a.txt", Some(&cwd), None, &[]).unwrap();
    assert_eq!((ok.shown.as_str(), ok.size), ("a.txt", 1));
    // outside and missing give the same answer, so the model cannot probe
    let outside_path = format!("{}/b.txt", other.display());
    let outside = prepare_read(&outside_path, Some(&cwd), None, &[]).unwrap_err();
    let missing = prepare_read("nope.txt", Some(&cwd), None, &[]).unwrap_err();
    assert_eq!(
        outside.replace(&outside_path, "X"),
        missing.replace("nope.txt", "X")
    );
    assert!(!outside.contains("configured roots"));
    // a configured root opens it, and the message then mentions the roots
    let roots = vec![other.display().to_string()];
    let far = prepare_read(
        &format!("{}/b.txt", other.display()),
        Some(&cwd),
        None,
        &roots,
    )
    .unwrap();
    assert_eq!(far.shown, far.path.to_string_lossy());
    assert!(prepare_read("nope.txt", Some(&cwd), None, &roots)
        .unwrap_err()
        .contains("configured roots"));
    // directories, two words and shell syntax are refused, each with its own reason
    assert!(prepare_read("sub", Some(&cwd), None, &[])
        .unwrap_err()
        .contains("not a regular file"));
    assert!(prepare_read("a.txt b.txt", Some(&cwd), None, &[])
        .unwrap_err()
        .contains("exactly one path"));
    assert!(prepare_read("$HOME", Some(&cwd), None, &[])
        .unwrap_err()
        .contains("one plain word"));
    assert!(prepare_read("a.txt", None, None, &[])
        .unwrap_err()
        .contains("no working directory"));
    // `~` cannot be typed in the path itself (the strict grammar refuses it) but configured roots may use it
    assert!(prepare_read("~/a.txt", Some(&cwd), None, &[])
        .unwrap_err()
        .contains("one plain word"));
    let home = other.parent().unwrap().display().to_string();
    let tilde_root = vec![format!(
        "~/{}",
        other.file_name().unwrap().to_string_lossy()
    )];
    let target = format!("{}/b.txt", other.display());
    assert!(prepare_read(&target, Some(&cwd), Some(&home), &tilde_root).is_ok());
    assert!(prepare_read(&target, Some(&cwd), None, &tilde_root).is_err());
    assert!(prepare_read(&target, Some(&cwd), Some(""), &tilde_root).is_err());
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&other);
}
