use super::*;
use shellmate_core::ports::{MapEnv, MemFs};

fn ctx_with<R>(toml: &str, env: MapEnv, fs: MemFs, f: impl FnOnce(&Ctx<'_>) -> R) -> R {
    let cfg = Config::parse(toml).unwrap();
    let registry = shellmate_builtin::registry();
    let host = Host::system().with_env(env).with_fs(fs);
    f(&Ctx {
        cfg: &cfg,
        registry: &registry,
        host: &host,
        path: Path::new("/cfg/shellmate/config.toml"),
    })
}

fn plain() -> shellmate_core::present::PlainPresenter {
    shellmate_core::present::PlainPresenter::new(shellmate_core::present::Caps::plain(80, true))
}

fn one(checks: Vec<Check>) -> Check {
    assert_eq!(checks.len(), 1, "{checks:?}");
    checks.into_iter().next().unwrap()
}

#[test]
fn host_and_port_are_extracted_without_credentials() {
    assert_eq!(
        host_port("http://localhost:11434/v1").as_deref(),
        Some("localhost:11434")
    );
    assert_eq!(
        host_port("https://api.openai.com/v1").as_deref(),
        Some("api.openai.com:443")
    );
    assert_eq!(
        host_port("https://u:pw@host.example/v1").as_deref(),
        Some("host.example:443")
    );
    assert_eq!(host_port("nonsense"), None);
}

#[test]
fn an_unreadable_config_is_one_failure_and_nothing_after_it() {
    let host = Host::system()
        .with_env(MapEnv::new())
        .with_fs(MemFs::new().file("/c.toml", "version = 1\nbogus = 2\n", 0o600, 1));
    let v = doctor_checks_in(
        Some(Path::new("/c.toml")),
        &host,
        &shellmate_builtin::registry(),
    );
    assert_eq!(v.len(), 2, "{v:?}");
    assert_eq!((v[0].name, v[1].name), ("version", "config"));
    assert_eq!(v[1].status, Status::Fail);
}

#[test]
fn a_missing_config_file_fails_with_the_setup_remedy() {
    let c = one(ctx_with(
        "version = 1",
        MapEnv::new(),
        MemFs::new(),
        config_is_valid,
    ));
    assert_eq!(c.status, Status::Fail);
    assert_eq!(c.remedy.as_deref(), Some("run `shellmate setup`"));
}

#[test]
fn a_valid_config_file_passes() {
    let fs = MemFs::new().file("/cfg/shellmate/config.toml", "version = 1", 0o600, 1);
    let toml = crate::admin::templates::template("ollama", "m").unwrap();
    let c = one(ctx_with(&toml, MapEnv::new(), fs, config_is_valid));
    assert_eq!(c.status, Status::Ok, "{c:?}");
    // the same file with no provider is not a working setup
    let fs = MemFs::new().file("/cfg/shellmate/config.toml", "version = 1", 0o600, 1);
    let c = one(ctx_with("version = 1", MapEnv::new(), fs, config_is_valid));
    assert_eq!(c.status, Status::Fail);
    assert!(c.detail.contains("config.no_provider"));
}

#[test]
fn the_terminal_check_wants_a_utf8_locale() {
    let run = |env: MapEnv| one(ctx_with("version = 1", env, MemFs::new(), terminal)).status;
    assert_eq!(run(MapEnv::new().with("LANG", "en_US.UTF-8")), Status::Ok);
    assert_eq!(
        run(MapEnv::new()
            .with("LC_ALL", "C")
            .with("LANG", "en_US.UTF-8")),
        Status::Warn,
        "LC_ALL wins"
    );
    assert_eq!(run(MapEnv::new()), Status::Warn);
}

#[test]
fn the_shell_check_knows_three_shells() {
    let run = |env: MapEnv| {
        let v = ctx_with("version = 1", env, MemFs::new(), |c| {
            // only the part that does not read the real home directory
            shell_name(c.host.env.as_ref())
        });
        v
    };
    assert_eq!(
        run(MapEnv::new().with("SHELL", "/usr/bin/fish")).as_deref(),
        Some("fish")
    );
    assert_eq!(run(MapEnv::new()), None);
    let tcsh = ctx_with(
        "version = 1",
        MapEnv::new().with("SHELL", "/bin/tcsh"),
        MemFs::new(),
        shell_and_integration,
    );
    assert_eq!(tcsh[0].status, Status::Warn);
    assert!(tcsh[0].detail.contains("tcsh"));
    let unset = ctx_with(
        "version = 1",
        MapEnv::new(),
        MemFs::new(),
        shell_and_integration,
    );
    assert_eq!(unset[0].detail, "$SHELL is not set");
    let session = ctx_with(
        "version = 1",
        MapEnv::new().with("SHELLMATE_SESSION", "s"),
        MemFs::new(),
        shell_and_integration,
    );
    assert!(session.iter().any(|c| c.name == "session"));
}

#[test]
fn the_integration_check_tells_the_four_states_apart() {
    let rc = Path::new("/home/u/.bashrc");
    let s = |text: &str| integration(text, "/bin/sm", rc).status;
    assert_eq!(
        s(&format!("{BEGIN}\neval \"$('/bin/sm' init bash)\"\n")),
        Status::Ok
    );
    let other = integration(&format!("{BEGIN}\n'/old/sm'\n"), "/bin/sm", rc);
    assert_eq!(other.status, Status::Warn);
    assert!(other.detail.contains("another binary"));
    assert_eq!(s("eval \"$(shellmate init bash)\""), Status::Ok);
    let none = integration("export A=1", "/bin/sm", rc);
    assert_eq!(none.status, Status::Warn);
    assert_eq!(
        none.remedy.as_deref(),
        Some("run `shellmate install-shell`")
    );
}

#[test]
fn a_man_tool_needs_a_man_binary_on_the_path() {
    let with = MemFs::new().file("/usr/bin/man", "", 0o755, 0);
    let env = MapEnv::new().with("PATH", "/nope:/usr/bin");
    assert_eq!(
        one(ctx_with("version = 1", env.clone(), with, man_tool)).status,
        Status::Ok
    );
    let c = one(ctx_with("version = 1", env.clone(), MemFs::new(), man_tool));
    assert_eq!(c.status, Status::Warn);
    let off = "version = 1\n[tools.man]\nenabled = false\n";
    assert!(ctx_with(off, env, MemFs::new(), man_tool).is_empty());
}

#[test]
fn decision_models_are_only_checked_when_they_are_in_the_validator_chain() {
    assert!(ctx_with("version = 1", MapEnv::new(), MemFs::new(), deciders).is_empty());
    let models =
        "version = 1\n[providers.p]\ntrust = \"local\"\nendpoint = \"http://127.0.0.1:9\"\n\
         [models.judge]\nprovider = \"p\"\nplugin = \"builtin:systemone\"\nmodel = \"m\"\n";
    // defined but not in the chain: nothing to check
    assert!(ctx_with(models, MapEnv::new(), MemFs::new(), deciders).is_empty());
    let on = format!("{models}[validator]\nchain = [\"judge\"]\n");
    let c = ctx_with(&on, MapEnv::new(), MemFs::new(), deciders);
    assert!(
        c.iter()
            .any(|c| c.name == "decider" && c.detail.contains("judge")),
        "{c:?}"
    );
}

#[test]
fn the_report_lists_remedies_only_for_what_is_not_ok() {
    let checks = vec![
        check("a", Status::Ok, "fine", Some("ignored")),
        check("b", Status::Warn, "hmm", Some("do this")),
    ];
    let (text, failed) = render(&checks, false, &plain());
    assert!(!failed);
    assert!(text.contains("OK    a") && text.contains("-> do this") && !text.contains("ignored"));
    let (json, _) = render(&[check("c", Status::Fail, "x", None)], true, &plain());
    assert!(json.contains("\"status\": \"fail\""));
    assert!(render(&[check("c", Status::Fail, "x", None)], false, &plain()).1);
}

#[test]
fn the_status_words_take_the_colour_of_their_meaning_and_the_text_is_unchanged() {
    use shellmate_core::present::{presenter, Caps, Level};
    let checks = vec![
        check("a", Status::Ok, "fine", None),
        check("b", Status::Warn, "hmm", Some("do this")),
        check("c", Status::Fail, "bad", None),
    ];
    let colour = presenter(Caps {
        width: 80,
        unicode: true,
        level: Level::Ansi16,
        markdown: true,
    });
    let (styled, failed) = render(&checks, false, &*colour);
    assert!(failed);
    assert!(
        styled.contains("\u{1b}[1m\u{1b}[32mOK   \u{1b}[0m"),
        "{styled:?}"
    );
    assert!(styled.contains("\u{1b}[33mWARN \u{1b}[0m"), "{styled:?}");
    assert!(
        styled.contains("\u{1b}[1m\u{1b}[31mFAIL \u{1b}[0m"),
        "{styled:?}"
    );
    let mut unstyled = String::new();
    let mut rest = styled.as_str();
    while let Some(i) = rest.find("\u{1b}[") {
        unstyled.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find('m').unwrap();
        rest = &rest[end + 1..];
    }
    unstyled.push_str(rest);
    assert_eq!(unstyled, render(&checks, false, &plain()).0);
}
