use super::*;
use crate::ports::{MapEnv, MemFs};

fn real(path: &str) -> Host {
    Host::system().with_env(MapEnv::new().with("PATH", path).with("HOME", "/home/u"))
}

fn run_line(line: &str) -> Result<String> {
    run("k", line, &real("/usr/bin:/bin"), &CmdOptions::default())
}

#[test]
fn a_line_is_split_like_a_person_reads_it() {
    let env = MapEnv::new().with("HOME", "/home/u");
    let s = |l: &str| split("k", l, &env).unwrap();
    assert_eq!(s("pass show api/openai"), ["pass", "show", "api/openai"]);
    assert_eq!(s("  a   b\tc "), ["a", "b", "c"]);
    assert_eq!(s("a 'b c' \"d e\""), ["a", "b c", "d e"]);
    assert_eq!(s("a ''"), ["a", ""]);
    assert_eq!(s("a 'x;y|z'"), ["a", "x;y|z"], "single quotes are literal");
    assert_eq!(s(r#"a "q\"r\\s""#), ["a", r#"q"r\s"#]);
    assert_eq!(
        s("op read op://Private/OpenAI/credential"),
        ["op", "read", "op://Private/OpenAI/credential"]
    );
    assert_eq!(
        s("age -d -i ~/.age/key"),
        ["age", "-d", "-i", "/home/u/.age/key"]
    );
    assert_eq!(
        s("a x~/y '~/z'"),
        ["a", "x~/y", "~/z"],
        "only an unquoted word that starts with ~/"
    );
}

#[test]
fn shell_syntax_is_refused_and_the_line_is_not_echoed() {
    let env = MapEnv::new();
    for bad in [
        "pass show x | head -n1",
        "pass show x; id",
        "pass show x && id",
        "pass show $(id)",
        "pass show `id`",
        "pass show $HOME",
        "pass show \"$HOME\"",
        "pass show \"`id`\"",
        "pass show x > f",
        "pass show < f",
        "pass show *",
        "pass show ?",
        "pass show [a]",
        "pass show {a,b}",
        "pass show !x",
        "pass show #x",
        "pass show a\\ b",
        "pass show \"a\\nb\"",
        "pass show 'unclosed",
        "pass show \"unclosed",
        "pass show\nid",
        "pass show\rid",
        "pass show 'a\nb'",
        "pass show \x07",
        "",
        "   ",
        "''",
    ] {
        // a blank line plus a word is a valid line, so only the others get a token appended
        let mut lines = vec![bad.to_string()];
        if !bad.trim().is_empty() {
            lines.push(format!("{bad} sk-live-123"));
        }
        for line in lines {
            let e = split("providers.p.settings.api_key", &line, &env).expect_err(&line);
            assert_eq!(e.code, "config.invalid_cmd", "{line:?}");
            assert!(!e.message.contains("sk-live-123"), "{}", e.message);
            assert!(!e.message.contains("pass show"), "{}", e.message);
            assert!(e.message.contains("providers.p.settings.api_key"));
        }
    }
}

#[test]
fn a_program_with_a_relative_path_is_refused() {
    let h = Host::system().with_env(MapEnv::new().with("PATH", "/usr/bin"));
    for p in ["./x", "../x", "bin/x", "x/y", "~/x"] {
        let e = find_program("k", p, &h).unwrap_err();
        assert_eq!(e.code, "config.invalid_cmd", "{p}");
    }
}

#[test]
fn a_bare_name_is_looked_up_only_in_absolute_path_entries() {
    let host = |path: &str| {
        let fs = MemFs::new()
            .file("/opt/bin/tool", "", 0o755, 1000)
            .file("/data/plain", "", 0o644, 1000)
            .file("/here/tool", "", 0o755, 1000);
        Host::system()
            .with_env(MapEnv::new().with("PATH", path))
            .with_fs(fs)
    };
    assert_eq!(
        find_program("k", "tool", &host("/usr/bin:/opt/bin")).unwrap(),
        PathBuf::from("/opt/bin/tool")
    );
    // an empty or relative entry means "the current directory": never searched
    for path in ["", ":", ".", "here", "/usr/bin::/nowhere", "/usr/bin:."] {
        let e = find_program("k", "tool", &host(path)).unwrap_err();
        assert_eq!(e.code, "config.secret_cmd_missing", "{path:?}");
    }
    // present but not executable
    let e = find_program("k", "plain", &host("/data")).unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_missing");
    let e = find_program("k", "/data/plain", &host("/data")).unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_missing");
    // an absolute path is used as written
    assert_eq!(
        find_program("k", "/opt/bin/tool", &host("")).unwrap(),
        PathBuf::from("/opt/bin/tool")
    );
}

#[test]
fn check_looks_at_the_line_and_the_program_without_running_anything() {
    let marker = std::env::temp_dir().join(format!("sm-cmd-check-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let line = format!("/bin/sh -c 'echo key; touch {}'", marker.display());
    check("k", &line, &Host::system()).unwrap();
    assert!(!marker.exists(), "check must not run the program");
    assert_eq!(
        check("k", "/no/such/program a", &Host::system())
            .unwrap_err()
            .code,
        "config.secret_cmd_missing"
    );
    assert_eq!(
        check("k", "a | b", &Host::system()).unwrap_err().code,
        "config.invalid_cmd"
    );
}

#[test]
fn the_output_of_the_program_is_the_secret() {
    assert_eq!(run_line("/bin/sh -c 'echo sk-test'").unwrap(), "sk-test");
    assert_eq!(
        run_line("/bin/sh -c 'printf \"sk-a b\\r\\n\"'").unwrap(),
        "sk-a b"
    );
    // a bare name is found in PATH
    assert_eq!(run_line("sh -c 'echo found'").unwrap(), "found");
}

#[test]
fn the_program_gets_a_small_environment() {
    let host = |extra: Option<&str>| {
        let mut e = MapEnv::new()
            .with("PATH", "/usr/bin:/bin")
            .with("OPENAI_API_KEY", "sk-from-env")
            .with("MY_VAR", "mine");
        if let Some(x) = extra {
            e = e.with("GNUPGHOME", x);
        }
        Host::system().with_env(e)
    };
    let line = "/bin/sh -c 'echo \"${OPENAI_API_KEY:-unset}\"'";
    assert_eq!(
        run("k", line, &host(None), &CmdOptions::default()).unwrap(),
        "unset"
    );
    // the fixed list is passed on
    let line = "/bin/sh -c 'echo \"${GNUPGHOME:-unset}\"'";
    assert_eq!(
        run("k", line, &host(Some("/g")), &CmdOptions::default()).unwrap(),
        "/g"
    );
    // anything else only when it is named in `pass_env`
    let line = "/bin/sh -c 'echo \"${MY_VAR:-unset}\"'";
    assert_eq!(
        run("k", line, &host(None), &CmdOptions::default()).unwrap(),
        "unset"
    );
    let opts = CmdOptions {
        pass_env: vec!["MY_VAR".into()],
        ..Default::default()
    };
    assert_eq!(run("k", line, &host(None), &opts).unwrap(), "mine");
}

#[test]
fn the_program_has_no_stdin() {
    // `cat` returns at once on a closed stdin; with an inherited terminal it would wait for the timeout
    let e = run_line("/bin/sh -c 'cat'").unwrap_err();
    assert_eq!(e.code, "config.secret_unset");
}

#[test]
fn a_failing_program_is_an_error_and_its_stdout_is_never_shown() {
    let e = run_line("/bin/sh -c 'echo sk-leak; echo not-allowed >&2; exit 3'").unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_failed");
    assert!(e.message.contains("exit 3"), "{}", e.message);
    assert!(e.message.contains("not-allowed"));
    assert!(!e.message.contains("sk-leak"), "{}", e.message);
    // the line is not echoed, only the program's name
    let e = run_line("/bin/sh -c 'echo sk-in-args >&2; exit 1' sk-in-args").unwrap_err();
    assert!(e.message.contains("`sh`"));
    assert!(!e.message.contains("-c"), "{}", e.message);
}

#[test]
fn what_the_program_wrote_to_stderr_is_escaped_and_short() {
    let e = run_line("/bin/sh -c 'printf \"\\033[31mred\" >&2; head -c 600 /dev/zero | tr \"\\0\" a >&2; exit 1'").unwrap_err();
    assert!(!e.message.contains('\u{1b}'), "{:?}", e.message);
    assert!(e.message.len() < 400, "{}", e.message.len());
}

#[test]
fn no_output_is_an_error() {
    assert_eq!(
        run_line("/bin/sh -c 'true'").unwrap_err().code,
        "config.secret_unset"
    );
    assert_eq!(
        run_line("/bin/sh -c 'echo; echo'").unwrap_err().code,
        "config.secret_unset"
    );
}

#[test]
fn output_that_is_not_one_line_of_text_is_an_error() {
    let e = run_line("/bin/sh -c 'echo key; echo notes'").unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_failed");
    assert!(e.hint.is_some());
    assert!(!e.message.contains("notes"));
    let e = run_line("/bin/sh -c 'printf \"\\377\\376\"'").unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_failed");
}

#[test]
fn too_much_output_is_an_error_not_a_cut_key() {
    let e = run_line("/bin/sh -c 'head -c 5000 /dev/zero | tr \"\\0\" a'").unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_failed");
    assert!(e.message.contains("more than"));
    // exactly at the limit is fine
    let ok = run_line("/bin/sh -c 'head -c 4096 /dev/zero | tr \"\\0\" a'").unwrap();
    assert_eq!(ok.len(), MAX_OUTPUT);
}

#[test]
fn a_program_that_does_not_finish_is_stopped() {
    let opts = CmdOptions {
        timeout: Duration::from_millis(200),
        ..Default::default()
    };
    let e = run(
        "k",
        "/bin/sh -c 'exec sleep 30'",
        &real("/usr/bin:/bin"),
        &opts,
    )
    .unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_failed");
    assert!(e.message.contains("did not finish"));
    assert!(e.hint.unwrap().contains("cmd_timeout"));
}

#[test]
fn a_helper_left_holding_the_pipes_does_not_hang_the_lookup() {
    // the program prints the key and exits, but a background process keeps stdout open
    let line = "/bin/sh -c 'echo sk-bg; (exec sleep 30) &'";
    assert_eq!(run_line(line).unwrap(), "sk-bg");
}

#[test]
fn a_program_that_cannot_start_is_an_error() {
    let h = Host::system();
    let e = run("k", "/etc/passwd x", &h, &CmdOptions::default()).unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_missing");
    let e = run("k", "/nope/x", &h, &CmdOptions::default()).unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_missing");
}

#[test]
fn a_control_character_is_refused_inside_quotes_too() {
    let env = MapEnv::new();
    for line in [
        "a b\u{7}c",
        "a 'b\u{7}c'",
        "a \"b\u{7}c\"",
        "a 'b\nc'",
        "a \"b\nc\"",
    ] {
        let e = split("k", line, &env).unwrap_err();
        assert!(
            e.message.contains("control character"),
            "{line:?}: {}",
            e.message
        );
    }
}

#[test]
fn a_tilde_is_only_the_home_directory_at_the_start_of_an_unquoted_word() {
    let env = MapEnv::new().with("HOME", "/home/u");
    let s = |l: &str| split("k", l, &env).unwrap();
    // an empty quoted part already started the word, so the `~/` that follows is not at its start
    assert_eq!(s("a ''~/y"), ["a", "~/y"]);
    assert_eq!(s("a \"\"~/y"), ["a", "~/y"]);
    assert_eq!(s("a ~/y"), ["a", "/home/u/y"]);
    assert_eq!(s("a ~y ~"), ["a", "~y", "~"], "no slash, no expansion");
}

#[test]
fn a_key_pasted_into_the_line_is_recognised() {
    // a well-known prefix and at least 8 more characters
    for p in [
        "sk-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xox",
        "AKIA",
        "eyJ",
        "glpat-",
        "AIza",
    ] {
        assert!(looks_like_a_secret(&format!("{p}abcdefgh")), "{p}");
        assert!(
            !looks_like_a_secret(&format!("{p}abcdefg")),
            "{p} with 7 more"
        );
    }
    // a prefix is not enough, and neither is length alone
    assert!(!looks_like_a_secret("sk-"));
    assert!(!looks_like_a_secret("hello-world-abc"));
    assert!(!looks_like_a_secret("a.b.c.d.e.f.g.h.i.j.k"));

    // a long run of letters and digits, with `_` and `-` allowed
    let run32 = "a1".repeat(16);
    assert_eq!(run32.len(), 32);
    assert!(looks_like_a_secret(&run32));
    assert!(!looks_like_a_secret(&run32[..31]));
    assert!(looks_like_a_secret(&format!("{}_", &run32[..31])));
    assert!(looks_like_a_secret(&format!("{}-", &run32[..31])));
    assert!(!looks_like_a_secret(&format!("{}.", &run32[..31])));
    assert!(!looks_like_a_secret(&format!("{}/", &run32[..31])));
    // it needs both a letter and a digit
    assert!(!looks_like_a_secret(&"a".repeat(32)));
    assert!(!looks_like_a_secret(&"7".repeat(32)));
    assert!(!looks_like_a_secret(&"_".repeat(32)));
}

#[test]
fn only_the_arguments_after_the_program_are_judged_for_a_secret() {
    let env = MapEnv::new();
    assert!(has_embedded_secret("pass sk-abcdefgh", &env));
    assert!(!has_embedded_secret("pass show api/openai", &env));
    assert!(
        !has_embedded_secret(&format!("{} show", "a1".repeat(16)), &env),
        "the program name is not an argument"
    );
}

#[test]
fn output_that_arrives_after_the_program_exits_is_still_read() {
    // the program exits at once; a helper keeps the pipe and prints a moment later, inside the grace period
    let line = "/bin/sh -c '(sleep 0.05; echo sk-late) & exit 0'";
    assert_eq!(run_line(line).unwrap(), "sk-late");
}
