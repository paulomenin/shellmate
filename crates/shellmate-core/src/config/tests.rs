use super::*;
use crate::ports::{Host, MapEnv, MemFs};
use std::path::{Path, PathBuf};

const GOOD: &str = r#"
version = 1
[providers.local]
trust = "local"
preset = "ollama"
[models.qwen]
provider = "local"
plugin = "builtin:openai-compatible"
model = "qwen2.5-coder:7b"
[llm]
chain = ["qwen"]
"#;

#[test]
fn durations_need_a_unit() {
    assert_eq!(parse_duration("90s").unwrap().as_secs(), 90);
    assert_eq!(parse_duration("30m").unwrap().as_secs(), 1800);
    assert_eq!(parse_duration("2h").unwrap().as_secs(), 7200);
    assert_eq!(parse_duration("7d").unwrap().as_secs(), 604800);
    for bad in ["", "30", "m", "1.5h", "-5m", "5x"] {
        assert!(parse_duration(bad).is_err(), "{bad}");
    }
}

#[test]
fn conversation_settings_and_per_action_overrides() {
    let c = Config::parse(
        "version = 1\n[conversation]\nidle_timeout = \"10m\"\n[actions.explain]\nidle_timeout = \"5m\"\nconversation = \"fresh\"",
    )
    .unwrap();
    assert_eq!(c.idle_timeout(Action::Ask).as_secs(), 600);
    assert_eq!(c.idle_timeout(Action::Explain).as_secs(), 300);
    assert_eq!(
        c.conversation_mode(Action::Explain),
        ConversationMode::Fresh
    );
    assert_eq!(c.conversation_mode(Action::Ask), ConversationMode::Auto);
    let d = Config::parse("version = 1").unwrap();
    assert_eq!(d.idle_timeout(Action::Ask).as_secs(), 1800);
    assert_eq!(d.conversation.max_turns, 40);
}

#[test]
fn bad_conversation_settings_are_errors() {
    for bad in [
        "[conversation]\nidle_timeout = \"soon\"",
        "[conversation.history]\nstrategy = \"summarize\"",
        "[actions.new-conversation]\nconversation = \"fresh\"",
        "[actions.ask]\nidle_timeout = \"5\"",
        "[conversation]\nstore = \"tmpfs\"",
    ] {
        assert!(
            Config::parse(&format!("version = 1\n{bad}")).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn parses_a_good_config() {
    let c = Config::parse(GOOD).unwrap();
    assert_eq!(c.model_chain().unwrap(), ["qwen"]);
    assert_eq!(c.providers["local"].effective_max_level(), 3);
    assert_eq!(c.model("qwen").unwrap().provider.trust, Trust::Local);
    assert_eq!(c.input.shortcuts["alt+m"], "ask");
}

#[test]
fn unknown_keys_are_errors() {
    let e = Config::parse("version = 1\n[runtim]\nmode='x'").unwrap_err();
    assert_eq!(e.code, "config.invalid");
    assert!(e.message.contains("runtim"), "{}", e.message);
}

#[test]
fn cross_references_are_checked() {
    let e = Config::parse("version = 1\n[llm]\nchain=['nope']").unwrap_err();
    assert!(e.message.contains("nope"));
    let e = Config::parse("version = 1\n[input.shortcuts]\n'alt+x'='fly'").unwrap_err();
    assert!(e.message.contains("fly"));
    assert_eq!(
        Config::parse("version = 2").unwrap_err().code,
        "config.invalid"
    );
    assert_eq!(
        Config::parse("version = 1\n[runtime]\nmode='daemon'")
            .unwrap_err()
            .code,
        "config.unsupported"
    );
}

#[test]
fn empty_config_has_no_provider() {
    assert_eq!(
        Config::defaults().model_chain().unwrap_err().code,
        "config.no_provider"
    );
}

#[test]
fn inline_secrets_in_provider_settings_are_refused() {
    let c = Config::parse(&GOOD.replace("model =", "api_key = \"sk-inline\"\nmodel =")).unwrap();
    assert_eq!(
        c.resolved_model_settings("qwen", &Host::system())
            .unwrap_err()
            .code,
        "config.inline_secret"
    );
}

#[test]
fn pipeline_defaults_and_overrides() {
    let c = Config::parse("version = 1").unwrap();
    assert_eq!(c.llm_layers(), ["builtin:redact"]);
    let c = Config::parse("version = 1\n[pipeline]\nllm = [\"...\", \"builtin:audit\"]").unwrap();
    assert_eq!(c.llm_layers(), ["builtin:redact", "builtin:audit"]);
    let c = Config::parse("version = 1\n[plugins.\"builtin:redact\"]\nenabled = false").unwrap();
    assert!(c.llm_layers().is_empty());
    assert_eq!(
        Config::parse("version = 1\n[pipeline]\nllm = [\"redact\"]")
            .unwrap_err()
            .code,
        "config.unsupported"
    );
}

#[test]
fn missing_default_file_gives_defaults() {
    let host = Host::system()
        .with_env(MapEnv::new().with("XDG_CONFIG_HOME", "/cfg"))
        .with_fs(MemFs::new());
    assert!(Config::load_from(None, &host).unwrap().providers.is_empty());
    assert_eq!(
        Config::load_from(Some(Path::new("/cfg/explicit.toml")), &host)
            .unwrap_err()
            .code,
        "config.unreadable"
    );
}

#[test]
fn the_default_file_is_found_under_xdg_config_home_then_home() {
    let xdg = MapEnv::new()
        .with("XDG_CONFIG_HOME", "/x")
        .with("HOME", "/h");
    assert_eq!(
        Config::default_path_in(&xdg).unwrap(),
        PathBuf::from("/x/shellmate/config.toml")
    );
    let relative = MapEnv::new()
        .with("XDG_CONFIG_HOME", "relative")
        .with("HOME", "/h");
    assert_eq!(
        Config::default_path_in(&relative).unwrap(),
        PathBuf::from("/h/.config/shellmate/config.toml")
    );
    assert_eq!(Config::default_path_in(&MapEnv::new()), None);
    let host = Host::system().with_env(xdg).with_fs(MemFs::new().file(
        "/x/shellmate/config.toml",
        "version = 1\n[context]\ndefault_level = 2\n",
        0o600,
        1000,
    ));
    assert_eq!(
        Config::load_from(None, &host)
            .unwrap()
            .context
            .default_level,
        2
    );
}

#[test]
fn the_ui_section_defaults_to_automatic_colour_and_markdown() {
    use crate::present::ColorSetting;
    let c = Config::parse("version = 1").unwrap();
    assert!(c.ui.stream && c.ui.markdown);
    assert_eq!(c.ui.color, ColorSetting::Auto);
    let c = Config::parse("version = 1\n[ui]\ncolor = \"never\"\nmarkdown = false").unwrap();
    assert_eq!(c.ui.color, ColorSetting::Never);
    assert!(!c.ui.markdown);
    assert!(Config::parse("version = 1\n[ui]\ncolor = \"sometimes\"").is_err());
}

fn marker(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("sm-cfg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn cmd_config(line: &str) -> Config {
    Config::parse(&format!(
        "version = 1\n[providers.p]\ntrust = \"local\"\npreset = \"ollama\"\napi_key = \"{line}\"\n[models.p]\nprovider = \"p\"\nplugin = \"builtin:openai-compatible\"\nmodel = \"m\"\n"
    ))
    .unwrap()
}

#[test]
fn secrets_settings_have_safe_defaults_and_are_checked() {
    let c = Config::parse("version = 1").unwrap();
    assert_eq!(c.cmd_options().timeout.as_secs(), 5);
    assert!(c.cmd_options().pass_env.is_empty());
    let c = Config::parse(
        "version = 1\n[secrets]\ncmd_timeout = \"20s\"\npass_env = [\"PASSWORD_STORE_DIR\"]",
    )
    .unwrap();
    assert_eq!(c.cmd_options().timeout.as_secs(), 20);
    assert_eq!(c.cmd_options().pass_env, ["PASSWORD_STORE_DIR"]);
    for bad in [
        "cmd_timeout = \"5\"",
        "cmd_timeout = \"0s\"",
        "cmd_timeout = \"3m\"",
        "pass_env = [\"\"]",
        "pass_env = [\"1X\"]",
        "pass_env = [\"A=B\"]",
        "pass_env = [\"A B\"]",
        "timeout = \"5s\"",
    ] {
        assert!(
            Config::parse(&format!("version = 1\n[secrets]\n{bad}")).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn a_cmd_secret_is_not_run_when_the_settings_are_read_lazily() {
    let m = marker("lazy");
    let c = cmd_config(&format!(
        "cmd:/bin/sh -c 'echo sk-real; touch {}'",
        m.display()
    ));
    let lazy = c.lazy_model_settings("p", &Host::system()).unwrap();
    assert!(lazy.is_lazy());
    assert!(!m.exists(), "reading the settings must not run the program");
    let probe = lazy.probe().to_string();
    assert!(
        !probe.contains("sk-real") && !probe.contains("/bin/sh"),
        "{probe}"
    );
    assert_eq!(lazy.probe()["model"], "m");
    // completing runs it and puts the output in the same key
    let done = lazy.complete(&Host::system()).unwrap();
    assert!(m.exists());
    assert_eq!(done["api_key"], "sk-real");
    assert_eq!(done["model"], "m");
    let _ = std::fs::remove_file(&m);
}

#[test]
fn the_eager_read_runs_a_cmd_secret() {
    let m = marker("eager");
    let c = cmd_config(&format!(
        "cmd:/bin/sh -c 'echo sk-eager; touch {}'",
        m.display()
    ));
    assert_eq!(
        c.resolved_model_settings("p", &Host::system()).unwrap()["api_key"],
        "sk-eager"
    );
    assert!(m.exists());
    let _ = std::fs::remove_file(&m);
}

#[test]
fn a_bad_cmd_line_is_found_without_running_anything() {
    let m = marker("bad");
    for (line, code) in [
        (
            format!("cmd:/bin/sh -c 'touch {}' | cat", m.display()),
            "config.invalid_cmd",
        ),
        ("cmd:./local-script".to_string(), "config.invalid_cmd"),
        (
            "cmd:/no/such/program".to_string(),
            "config.secret_cmd_missing",
        ),
        ("cmd:".to_string(), "config.invalid_cmd"),
    ] {
        let e = cmd_config(&line)
            .lazy_model_settings("p", &Host::system())
            .unwrap_err();
        assert_eq!(e.code, code, "{line}");
        assert!(e.message.contains("providers.p.api_key"), "{}", e.message);
    }
    assert!(!m.exists());
}

#[test]
fn a_keyring_reference_is_checked_without_asking_the_keyring() {
    let e = cmd_config("keyring:no-account")
        .lazy_model_settings("p", &Host::system())
        .unwrap_err();
    assert_eq!(e.code, "config.invalid_keyring");
    assert!(e.message.contains("providers.p.api_key"), "{}", e.message);
    // a well-formed one is never a syntax error; it is lazy when the tool exists and "tool missing"
    // when it does not, and in neither case has anything been fetched
    match cmd_config("keyring:shellmate/openai").lazy_model_settings("p", &Host::system()) {
        Ok(lazy) => assert!(lazy.is_lazy()),
        Err(e) => assert!(
            ["config.secret_cmd_missing", "config.unsupported"].contains(&e.code),
            "{}",
            e.code
        ),
    }
}

#[test]
fn the_secrets_settings_reach_the_program() {
    let c = Config::parse(
        "version = 1\n[secrets]\npass_env = [\"SM_PASS_ME\"]\n[providers.p]\npreset = \"ollama\"\napi_key = \"cmd:/bin/sh -c 'echo $SM_PASS_ME'\"\n[models.p]\nprovider = \"p\"\nplugin = \"builtin:openai-compatible\"\nmodel = \"m\"\n",
    )
    .unwrap();
    let host = Host::system().with_env(
        MapEnv::new()
            .with("PATH", "/bin")
            .with("SM_PASS_ME", "passed"),
    );
    assert_eq!(
        c.resolved_model_settings("p", &host).unwrap()["api_key"],
        "passed"
    );
}

#[test]
fn only_secret_settings_run_programs() {
    // `model` is not a secret key: a `cmd:` value there is just text
    let c = Config::parse(
        "version = 1\n[providers.p]\npreset = \"ollama\"\n[models.p]\nprovider = \"p\"\nplugin = \"builtin:openai-compatible\"\nmodel = \"cmd:/bin/false\"\n",
    )
    .unwrap();
    let lazy = c.lazy_model_settings("p", &Host::system()).unwrap();
    assert!(!lazy.is_lazy());
    assert_eq!(lazy.probe()["model"], "cmd:/bin/false");
}

#[test]
fn a_model_and_its_provider_share_one_settings_object() {
    let c = Config::parse(
        "version = 1\n[providers.p]\npreset = \"openrouter\"\ntimeout_secs = 30\n\
         [models.chat]\nprovider = \"p\"\nplugin = \"builtin:openai-compatible\"\nmodel = \"a\"\n\
         [models.judge]\nprovider = \"p\"\nplugin = \"builtin:systemone\"\nmodel = \"b\"\n",
    )
    .unwrap();
    for (name, model) in [("chat", "a"), ("judge", "b")] {
        let s = c.resolved_model_settings(name, &Host::system()).unwrap();
        assert_eq!(s["preset"], "openrouter", "{name}");
        assert_eq!(s["timeout_secs"], 30, "{name}");
        assert_eq!(s["model"], model, "{name}");
        // the core's own keys are not handed to the plugin
        assert!(
            s.get("provider").is_none() && s.get("plugin").is_none(),
            "{s}"
        );
    }
    assert_eq!(c.model("chat").unwrap().provider.trust, Trust::Remote);
}

#[test]
fn a_table_that_points_nowhere_is_an_error() {
    let model = "[models.m]\nprovider = \"p\"\nplugin = \"builtin:x\"\nmodel = \"a\"\n";
    let e = Config::parse(&format!("version = 1\n{model}")).unwrap_err();
    assert!(e.message.contains("unknown provider `p`"), "{}", e.message);
    let e = Config::parse(&format!(
        "version = 1\n[providers.p]\npreset = \"ollama\"\n{model}[validator]\nchain = [\"nope\"]"
    ))
    .unwrap_err();
    assert!(e.message.contains("validator.chain") && e.message.contains("nope"));
    assert!(e.hint.unwrap().contains("m"));
}

#[test]
fn a_key_is_set_on_the_provider_or_on_the_model_not_both() {
    let e = Config::parse(
        "version = 1\n[providers.p]\npreset = \"ollama\"\n[models.m]\nprovider = \"p\"\nplugin = \"builtin:x\"\n\
         preset = \"openai\"\n",
    )
    .unwrap_err();
    assert!(e.message.contains("`preset`"), "{}", e.message);
}

#[test]
fn the_context_ceiling_is_the_providers_and_the_models_have_none() {
    let c = Config::parse(
        "version = 1\n[providers.p]\nmax_context_level = 2\npreset = \"ollama\"\n\
         [models.m]\nprovider = \"p\"\nplugin = \"builtin:x\"\n",
    )
    .unwrap();
    assert_eq!(c.model("m").unwrap().provider.effective_max_level(), 2);
    // a model's `max_context_level` is not a core key: it goes to the plugin, which refuses it
    let c = Config::parse(
        "version = 1\n[providers.p]\npreset = \"ollama\"\n[models.m]\nprovider = \"p\"\n\
         plugin = \"builtin:x\"\nmax_context_level = 4\n",
    )
    .unwrap();
    let s = c.resolved_model_settings("m", &Host::system()).unwrap();
    assert_eq!(s["max_context_level"], 4);
    assert!(Config::parse("version = 1\n[providers.p]\nmax_context_level = 5").is_err());
}

#[test]
fn plugin_settings_sit_next_to_enabled() {
    let c = Config::parse(
        "version = 1\n[plugins.\"builtin:store-tmpfs\"]\ndir = \"/x\"\n\
         [plugins.\"builtin:redact\"]\nenabled = false\nextra_patterns = []\n",
    )
    .unwrap();
    let s = c
        .plugin_settings("builtin:store-tmpfs", &Host::system())
        .unwrap();
    assert_eq!(s["dir"], "/x");
    assert!(s.get("enabled").is_none());
    assert!(c.llm_layers().is_empty());
    assert!(c
        .plugin_settings("builtin:redact", &Host::system())
        .unwrap()
        .get("extra_patterns")
        .is_some());
}

#[test]
fn keys_of_the_older_shape_say_where_they_went() {
    for (text, hint) in [
        (
            "[deciders.x]\nplugin = \"builtin:systemone\"",
            "[models.<name>]",
        ),
        (
            "[providers.x]\nplugin = \"builtin:openai-compatible\"",
            "[models.<name>]",
        ),
        (
            "[providers.x.settings]\nmodel = \"m\"",
            "[providers.<name>]",
        ),
        ("[validator]\nsemantic = true", "[validator] chain"),
        ("[validator]\ndeciders = [\"x\"]", "[validator] chain"),
        ("[validator]\nllm_fallback = false", "[validator] chain"),
        (
            "[plugins.\"builtin:redact\".settings]\nextra_patterns = []",
            "next to `enabled`",
        ),
    ] {
        let e = Config::parse(&format!("version = 1\n{text}")).unwrap_err();
        assert_eq!(e.code, "config.moved", "{text}: {}", e.message);
        assert!(e.hint.unwrap().contains(hint), "{text}");
    }
}
