use super::*;

fn argv(s: &str) -> Vec<String> {
    parse_strict(s).unwrap()
}
fn unsafe_(s: &str) -> bool {
    classify(s).verdict == Verdict::Unsafe
}

#[test]
fn simple_commands_parse() {
    assert_eq!(argv("git log -n 5"), ["git", "log", "-n", "5"]);
    assert_eq!(argv("echo 'a b' \"c d\""), ["echo", "a b", "c d"]);
    assert_eq!(argv("ls   -la\t/tmp"), ["ls", "-la", "/tmp"]);
}

#[test]
fn anything_else_is_complex() {
    for bad in [
        "ls | wc",
        "ls; id",
        "a && b",
        "echo $HOME",
        "echo `id`",
        "echo $(id)",
        "ls > f",
        "ls *",
        "ls ~",
        "FOO=1 ls",
        "cd /tmp",
        "eval ls",
        "echo \\n",
        "echo \"$x\"",
        "ls &",
        "echo é",
        "",
        "   ",
        "echo 'unterminated",
        "echo a\nb",
        "=ls",
        "%1",
    ] {
        assert!(parse_strict(bad).is_err(), "{bad:?} should be complex");
    }
}

#[test]
fn dangerous_commands_are_unsafe() {
    for bad in [
        "rm -rf /tmp/x",
        "sudo ls",
        "dd if=/dev/zero of=x",
        "chmod 777 x",
        "kill 1",
        "pkill foo",
        "systemctl restart sshd",
        "systemctl stop x",
        "service ssh restart",
        "pacman -S vim",
        "apt install x",
        "npm install x",
        "cargo install x",
        "pip install x",
        "find . -delete",
        "find . -exec rm {} ;",
        "sed -i s/a/b/ f",
        "bash -c ls",
        "python3 x.py",
        "curl http://x",
        "ssh host ls",
        "git -c core.pager=sh log",
        "git push",
        "git reset --hard",
        "xargs ls",
        "env ls",
        "cat ~/.ssh/id_rsa",
        "cat /home/u/.env",
        "ls .aws",
        "man -P sh ls",
        "ln -sf a b",
        "mv a b",
        "ls | rm",
        "ls; sudo id",
    ] {
        assert!(unsafe_(bad), "{bad} should be unsafe");
    }
}

#[test]
fn plain_reads_are_not_flagged() {
    for ok in [
        "ls -la",
        "pwd",
        "uname -a",
        "which ls",
        "git status",
        "git log -n 3",
        "git diff",
        "git branch",
        "systemctl status sshd",
        "systemctl is-active x",
        "pacman -Q",
        "pacman -Ss vim",
        "cat README.md",
        "df -h",
        "du -sh .",
        "head -n 5 f",
        "grep -rn foo src",
        "tar -tf a.tar",
        "ps aux",
        "uname",
        "date",
        "id",
        "whoami",
        "free -m",
        "journalctl -u sshd -n 20",
        "git stash list",
        "man tar",
    ] {
        let a = classify(ok);
        assert_eq!(a.verdict, Verdict::Safe, "{ok}: {:?}", a.reasons);
    }
}

#[test]
fn high_risk_is_reported() {
    assert_eq!(classify("sudo rm -rf /").risk, Risk::High);
    assert_eq!(classify("curl http://x").risk, Risk::Medium);
    assert_eq!(classify("ls").risk, Risk::Low);
    let a = classify("ls | sudo tee x");
    assert_eq!(a.verdict, Verdict::Unsafe);
    assert_eq!(a.risk, Risk::High, "{:?}", a.reasons);
}

#[test]
fn globs() {
    assert!(glob_match("ls", "ls"));
    assert!(glob_match("l*", "ls"));
    assert!(glob_match("*", "anything"));
    assert!(glob_match("a?c", "abc"));
    assert!(!glob_match("a?c", "ac"));
    assert!(!glob_match("ls", "lsx"));
}

#[test]
fn rules_match_whole_commands() {
    assert!(rule_matches("git status", &argv("git status")));
    assert!(!rule_matches("git status", &argv("git status -s")));
    assert!(!rule_matches("git status", &argv("git")));
    assert!(rule_matches("ls *", &argv("ls")));
    assert!(rule_matches("ls *", &argv("ls -la /tmp")));
    assert!(rule_matches(
        "systemctl status *",
        &argv("systemctl status sshd")
    ));
    assert!(!rule_matches(
        "systemctl status *",
        &argv("systemctl restart sshd")
    ));
    assert!(!rule_matches("", &argv("ls")));
    assert!(!rule_matches("*", &argv("ls")), "a lone `*` is not a rule");
}

#[test]
fn programs_resolve_through_path_only_for_bare_names() {
    let sh = resolve_program("sh", "/usr/bin:/bin").unwrap();
    assert!(sh.is_absolute());
    assert!(resolve_program("./sh", "/usr/bin:/bin").is_none());
    assert!(resolve_program("/bin/sh", "/usr/bin:/bin").is_none());
    assert!(resolve_program("definitely-not-a-program-x", "/usr/bin:/bin").is_none());
    assert!(
        resolve_program("sh", "relative/dir").is_none(),
        "relative PATH entries are ignored"
    );
}

#[test]
fn overly_broad_rules_are_reported() {
    let rules: Vec<String> = ["ls *", "*", "rm *", "sh *", "git status"]
        .map(String::from)
        .to_vec();
    let bad: Vec<_> = dangerous_rules(&rules)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    assert_eq!(bad, ["*", "rm *", "sh *"]);
}

#[test]
fn prepared_display_quotes_words_with_spaces() {
    let p = Prepared {
        program: "/bin/echo".into(),
        argv: vec!["echo".into(), "a b".into(), "c".into()],
        cwd: None,
    };
    assert_eq!(p.display(), "echo 'a b' c");
}

// ---- tests written from a mutation run: each pins behaviour a mutant could remove unnoticed ----

fn why(s: &str) -> String {
    classify(s).reasons.join(" | ")
}

#[test]
fn every_reason_a_line_is_not_simple_has_its_own_words() {
    let all = [
        (Complex::Empty, "empty"),
        (Complex::Control, "control"),
        (Complex::Operator(';'), "operator `;`"),
        (
            Complex::Expansion('$'),
            "expansion or special character `$`",
        ),
        (Complex::Backslash, "backslash"),
        (Complex::UnterminatedQuote, "unterminated quote"),
        (Complex::Assignment, "assignment"),
        (Complex::Keyword("if".into()), "keyword or builtin `if`"),
        (Complex::NonAscii, "non-ASCII"),
    ];
    for (c, words) in all {
        assert!(c.describe().contains(words), "{c:?}: {}", c.describe());
    }
}

#[test]
fn tilde_and_hash_are_literal_only_in_the_middle_of_a_plain_word() {
    assert_eq!(argv("ls a~b"), ["ls", "a~b"]);
    assert_eq!(argv("ls a#b"), ["ls", "a#b"]);
    assert_eq!(argv("ls a~"), ["ls", "a~"]);
    for bad in [
        "ls ~",
        "ls ~/x",
        "ls #x",
        "ls a=~x",
        "ls a:~x",
        "ls 'a'~",
        "ls \"a\"#",
    ] {
        assert!(parse_strict(bad).is_err(), "{bad}");
    }
    assert_eq!(parse_strict("ls ~"), Err(Complex::Expansion('~')));
}

#[test]
fn control_and_non_ascii_characters_and_stray_symbols_are_told_apart() {
    assert_eq!(parse_strict("ls \u{1}"), Err(Complex::Control));
    assert_eq!(parse_strict("ls a\u{7f}"), Err(Complex::Control));
    assert_eq!(parse_strict("ls é"), Err(Complex::NonAscii));
    assert_eq!(parse_strict("ls ^"), Err(Complex::Operator('^')));
    assert_eq!(parse_strict("ls \u{a0}"), Err(Complex::NonAscii));
    // non-ASCII is fine inside quotes, and control characters are not
    assert_eq!(argv("echo 'é'"), ["echo", "é"]);
    assert_eq!(parse_strict("echo 'a\u{1}'"), Err(Complex::Control));
}

#[test]
fn read_rules_that_match_everything_are_reported_and_narrow_ones_are_not() {
    let rules: Vec<String> = [
        "*",
        "**",
        "?*",
        "*/*",
        ".",
        "./**",
        "src/*",
        "*.md",
        "a?c",
        "docs/**/*.md",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let flagged: Vec<String> = dangerous_read_rules(&rules)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    assert_eq!(flagged, ["*", "**", "?*", "*/*", ".", "./**"]);
    let (_, reason) = &dangerous_read_rules(&["*".to_string()])[0];
    assert!(reason.contains("every file"), "{reason}");
    assert!(dangerous_read_rules(&[]).is_empty());
}

#[test]
fn services_may_be_inspected_but_not_changed() {
    for ok in [
        "systemctl",
        "systemctl status ssh",
        "systemctl --user status ssh",
        "systemctl is-active x",
        "service ssh status",
        "loginctl show x",
        "systemctl show x",
    ] {
        assert!(!unsafe_(ok), "{ok}: {}", why(ok));
    }
    for bad in [
        "systemctl restart ssh",
        "systemctl stop x",
        "systemctl enable x",
        "service ssh restart",
        "service ssh",
        "service",
        "loginctl terminate-user x",
        "machinectl poweroff",
    ] {
        let a = classify(bad);
        assert!(
            a.verdict == Verdict::Unsafe && a.risk == Risk::High,
            "{bad}: {a:?}"
        );
        assert!(why(bad).contains("change services"), "{bad}");
    }
}

#[test]
fn find_and_sed_are_flagged_only_for_the_forms_that_change_things() {
    for ok in [
        "find . -name x",
        "find /var -type f -mtime -1",
        "sed -n 5p f",
        "sed s/a/b/ f",
        "sed -e p f",
    ] {
        assert!(!unsafe_(ok), "{ok}: {}", why(ok));
    }
    for bad in [
        "find . -exec ls",
        "find . -execdir ls",
        "find . -ok rm",
        "find . -okdir x",
        "find . -delete",
        "find . -fprint o",
        "find . -fprintf o x",
        "find . -fls o",
    ] {
        assert!(
            why(bad).contains("`find` with an action"),
            "{bad}: {}",
            why(bad)
        );
    }
    for bad in [
        "sed -i s/a/b/ f",
        "sed -ibak s/a/b/ f",
        "sed --in-place=.b s/a/b/ f",
        "sed -n -i p f",
    ] {
        let a = classify(bad);
        assert!(
            a.risk == Risk::High && why(bad).contains("sed -i"),
            "{bad}: {a:?}"
        );
    }
    for bad in [
        "sed s/a/b/e f",
        "sed e f",
        "sed w out f",
        "sed 'p;e' f",
        "sed 'p;w out' f",
    ] {
        let a = classify(bad);
        assert!(
            a.risk == Risk::Medium && why(bad).contains("run commands or write files"),
            "{bad}: {a:?}"
        );
    }
}

#[test]
fn git_options_that_run_programs_and_subcommands_that_change_things() {
    for bad in [
        "git -c core.pager=x log",
        "git --exec-path=/x log",
        "git --git-dir x log",
        "git --config-env=a=b log",
    ] {
        let a = classify(bad);
        assert!(
            a.risk == Risk::High && why(bad).contains("git -c"),
            "{bad}: {a:?}"
        );
    }
    for ok in [
        "git status",
        "git log --oneline",
        "git diff",
        "git branch",
        "git remote",
        "git stash list",
        "git show HEAD",
        "git -C /tmp status",
    ] {
        assert!(!unsafe_(ok), "{ok}: {}", why(ok));
    }
    for bad in [
        "git branch -d x",
        "git branch new",
        "git remote add o u",
        "git remote -v",
        "git stash",
        "git stash drop",
        "git push",
        "git commit",
        "git add .",
        "git config a b",
        "git tag v1",
        "git clone u",
    ] {
        assert!(
            why(bad).contains("changes the repository"),
            "{bad}: {}",
            why(bad)
        );
    }
}

#[test]
fn man_options_and_paths_are_flagged_but_search_options_are_not() {
    for ok in ["man ls", "man -k tar", "man -f tar", "man 5 crontab"] {
        assert!(!unsafe_(ok), "{ok}: {}", why(ok));
    }
    for bad in [
        "man -P cat ls",
        "man --pager=x ls",
        "man ./x.1",
        "man /etc/passwd",
        "man -l x",
    ] {
        assert!(
            why(bad).contains("`man` option or path"),
            "{bad}: {}",
            why(bad)
        );
    }
}

#[test]
fn package_managers_may_query_but_not_change() {
    for ok in [
        "pacman -Q",
        "pacman -Qi vim",
        "pacman -Ss vim",
        "pacman -Si vim",
        "pacman -F x",
        "pacman --version",
        "pacman -V",
        "yay -Qm",
    ] {
        assert!(!unsafe_(ok), "{ok}: {}", why(ok));
    }
    for bad in [
        "pacman -S vim",
        "pacman -Syu",
        "pacman -R vim",
        "pacman",
        "paru -S x",
        "yay -Rns x",
    ] {
        assert!(
            why(bad).contains("install, remove or upgrade"),
            "{bad}: {}",
            why(bad)
        );
    }
}

#[test]
fn ln_is_only_flagged_when_it_can_overwrite() {
    for ok in ["ln a b", "ln -s a b", "ln -sv a b"] {
        assert!(!unsafe_(ok), "{ok}: {}", why(ok));
    }
    for bad in ["ln -f a b", "ln -sf a b", "ln -fs a b"] {
        assert!(why(bad).contains("ln -f"), "{bad}: {}", why(bad));
    }
}

#[test]
fn tar_is_read_only_only_when_listing() {
    for ok in [
        "tar tf a.tar",
        "tar -tf a.tar",
        "tar tvf a.tar",
        "tar --list -f a.tar",
        "tar -t",
        "tar --version",
    ] {
        assert!(!unsafe_(ok), "{ok}: {}", why(ok));
    }
    for bad in [
        "tar xf a.tar",
        "tar -xf a.tar",
        "tar -czf a.tgz d",
        "tar cf a.tar d",
        "tar rf a.tar f",
        "tar uf a.tar f",
        "tar --extract -f a",
        "tar --create -f a d",
        "tar --append -f a f",
        "tar --update -f a f",
        "tar --delete -f a f",
        "tar --get -f a",
    ] {
        let a = classify(bad);
        assert!(
            a.risk == Risk::Medium && why(bad).contains("`tar` creates or changes files"),
            "{bad}: {a:?}"
        );
    }
}

#[test]
fn programs_that_write_files_are_flagged_by_name() {
    for prog in [
        "tee", "touch", "mkdir", "mkfifo", "mknod", "patch", "ex", "vi", "vim", "nvim", "nano",
        "emacs", "ed", "sponge", "split", "csplit", "rename", "mmv", "unzip", "zip", "gzip",
        "gunzip", "bzip2", "xz", "7z", "cpio", "ar",
    ] {
        let line = format!("{prog} x");
        let a = classify(&line);
        assert!(
            a.verdict == Verdict::Unsafe
                && a.risk == Risk::Medium
                && why(&line).contains(&format!("`{prog}` creates or changes files")),
            "{line}: {a:?}"
        );
    }
    // reading programs are not in the list
    for ok in ["cat x", "ls", "head -n 3 x", "wc -l x", "grep a x"] {
        assert!(!unsafe_(ok), "{ok}");
    }
}

#[test]
fn a_program_is_found_only_when_it_is_an_executable_file_in_an_absolute_path_directory() {
    use std::os::unix::fs::PermissionsExt;
    let d = std::env::temp_dir().join(format!("sm-resolve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("isdir")).unwrap();
    std::fs::write(d.join("tool"), "x").unwrap();
    std::fs::set_permissions(d.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(d.join("plain"), "x").unwrap();
    std::fs::set_permissions(d.join("plain"), std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(d.join("isdir"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("/nonexistent:{}", d.display());
    assert_eq!(resolve_program("tool", &path), Some(d.join("tool")));
    assert_eq!(resolve_program("plain", &path), None, "not executable");
    assert_eq!(resolve_program("isdir", &path), None, "a directory");
    assert_eq!(resolve_program("missing", &path), None);
    assert_eq!(resolve_program("", &path), None);
    assert_eq!(resolve_program("a/tool", &path), None);
    assert_eq!(resolve_program("tool", "relative/dir:."), None);
    assert_eq!(resolve_program("tool", ""), None);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn globs_backtrack_over_stars() {
    for (p, t, want) in [
        ("a*b*c", "abxbc", true),
        ("a*ab", "aab", true),
        ("*ab", "aab", true),
        ("a*b", "ab", true),
        ("a*b", "a", false),
        ("*", "", true),
        ("?", "", false),
        ("a**b", "ab", true),
        ("*a*", "xax", true),
        ("*a*b", "bab", true),
        ("*a*b", "ba", false),
        ("ab*", "ab", true),
        ("ab*", "a", false),
        ("a?c", "abc", true),
        ("a?c", "ac", false),
        ("a*a*a", "aaaa", true),
        ("a*a*a", "aa", false),
    ] {
        assert_eq!(glob_match(p, t), want, "{p} vs {t}");
    }
}

#[test]
fn rule_wildcards_and_dangerous_rules_are_explained() {
    let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
    assert!(rule_matches("ls *", &words("ls")));
    assert!(rule_matches("ls *", &words("ls -la /tmp")));
    assert!(!rule_matches("ls *", &[]));
    assert!(!rule_matches("", &words("ls")));
    assert!(rule_matches("git status", &words("git status")));
    assert!(!rule_matches("git status", &words("git status -s")));
    assert!(!rule_matches("git status", &words("git")));
    assert!(rule_matches("git s*", &words("git status")));
    let flagged = dangerous_rules(&[
        "*".into(),
        "? x".into(),
        "rm *".into(),
        "sudo ls".into(),
        "ls *".into(),
        "git status".into(),
    ]);
    let names: Vec<&str> = flagged.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(names, ["*", "? x", "rm *", "sudo ls"]);
    assert!(flagged[0].1.contains("wildcard"));
    assert!(
        flagged[2].1.contains("classifier flags"),
        "{}",
        flagged[2].1
    );
}
