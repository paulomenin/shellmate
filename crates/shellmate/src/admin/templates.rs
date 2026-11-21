//! The starting configs `config init` and `setup` write, one per provider preset.

use shellmate_api::{Error, Result};

/// A documented starting config for a provider preset, for tests (the commands pick a shell too).
#[cfg(test)]
pub fn template(provider: &str, model: &str) -> Result<String> {
    template_for(provider, model, None, None, None)
}

/// True when the base URL points at this machine, so the provider can be trusted as local.
pub fn is_loopback_url(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or("");
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(v6) = host.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

/// The key the starting config gives to `capture`. `alt+r` is readline's `revert-line` in bash, and shellmate
/// never overrides a binding, so bash gets `alt+e`. (fish already uses `alt+e` for `edit_command_buffer`.)
fn capture_key(shell: Option<&str>) -> &'static str {
    if shell == Some("bash") {
        "alt+e"
    } else {
        "alt+r"
    }
}

/// One backend as `setup` and `config init` describe it: a provider or a checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backend {
    /// A preset name; the preset says where the server is.
    pub preset: Option<String>,
    /// A base URL, instead of the preset's own.
    pub endpoint: Option<String>,
    /// Where the key lives, as written in the config (`env:NAME` or `file:PATH`).
    pub api_key: Option<String>,
    pub model: String,
    /// `local` or `remote`.
    pub trust: &'static str,
}

impl Backend {
    /// The name of the `[providers.<name>]` for this backend: the preset, or `custom` for a base URL.
    fn provider_name(&self) -> String {
        self.preset.clone().unwrap_or_else(|| "custom".to_string())
    }

    /// Whether two backends are reached the same way, so one `[providers.<name>]` serves both.
    fn same_connection(&self, other: &Backend) -> bool {
        self.preset == other.preset
            && self.endpoint == other.endpoint
            && self.api_key == other.api_key
            && self.trust == other.trust
    }

    /// The `[providers.<name>]` table: where the data goes.
    fn provider_block(&self, name: &str) -> String {
        let mut s = format!(
            "[providers.{name}]\n\
             # local: runs on this machine (may see more context, redaction optional)\n\
             # remote: leaves this machine (context capped at level 1, redaction required)\n\
             trust = \"{}\"\n",
            self.trust
        );
        if let Some(p) = &self.preset {
            s.push_str(&format!("preset = \"{p}\"\n"));
        }
        if let Some(e) = &self.endpoint {
            s.push_str(&format!("endpoint = \"{e}\"\n"));
        }
        if let Some(k) = &self.api_key {
            s.push_str(&format!(
                "api_key = \"{k}\"   # a reference; never paste the key here\n"
            ));
        }
        s.push_str("# timeout_secs = 60\n");
        s
    }

    /// The `[models.<name>]` table: what is used from the provider.
    fn model_block(&self, name: &str, provider: &str, plugin: &str) -> String {
        let mut s = format!(
            "[models.{name}]\nprovider = \"{provider}\"\nplugin = \"{plugin}\"\nmodel = \"{}\"\n",
            self.model
        );
        if plugin == "builtin:openai-compatible" {
            s.push_str("# reasoning_effort = \"none\"   # much faster with reasoning models\n");
        }
        s
    }

    /// Refuse anything that could break out of a TOML string, or that is not a reference to a key.
    fn check(&self) -> Result<()> {
        let bad = |c: char| c == '"' || c == '\n' || c == '\\' || c.is_control();
        if self.model.trim().is_empty() || self.model.contains(bad) {
            return Err(Error::new("setup.bad_model", "invalid model name"));
        }
        if let Some(p) = &self.preset {
            if p.contains(bad) {
                return Err(Error::new("setup.unknown_provider", "invalid preset name"));
            }
        }
        if let Some(e) = &self.endpoint {
            if e.trim().is_empty() || e.contains(bad) {
                return Err(Error::new("setup.bad_base_url", "a base URL is needed")
                    .with_hint("e.g. http://localhost:8000 or https://api.example.com"));
            }
            shellmate_api::check_endpoint(e.trim())?;
        }
        if let Some(k) = &self.api_key {
            // the same rule the prompt applies: a reference to a key, never the key
            super::keyref::KeyRef::parse(k).map_err(|_| {
                Error::new(
                    "setup.bad_key_env",
                    "the API key must be a reference like env:MY_API_KEY or file:/path/to/key",
                )
                .with_hint(
                    "give the name of an environment variable or a file, never the key itself",
                )
            })?;
        }
        Ok(())
    }
}

/// Like [`template`], and for the `custom` provider: any OpenAI-compatible server at `base_url`, with an
/// optional environment variable that holds its API key. `shell` picks a free key for `capture`.
pub fn template_for(
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    key_env: Option<&str>,
    shell: Option<&str>,
) -> Result<String> {
    let bad = |c: char| c == '"' || c == '\n' || c == '\\' || c.is_control();
    let (trust, key, preset, endpoint) = match provider {
        "ollama" | "lmstudio" | "localai" => ("local", None, Some(provider.to_string()), None),
        "openai" => (
            "remote",
            Some("OPENAI_API_KEY"),
            Some("openai".to_string()),
            None,
        ),
        "openrouter" => (
            "remote",
            Some("OPENROUTER_API_KEY"),
            Some("openrouter".to_string()),
            None,
        ),
        "custom" => {
            let url = base_url.map(str::trim).unwrap_or("");
            if url.is_empty() || url.contains(bad) {
                return Err(Error::new("setup.bad_base_url", "a base URL is needed")
                    .with_hint("e.g. http://localhost:8000 or https://api.example.com"));
            }
            shellmate_api::check_endpoint(url)?;
            let trust = if is_loopback_url(url) {
                "local"
            } else {
                "remote"
            };
            (trust, key_env, None, Some(url.to_string()))
        }
        other => {
            return Err(Error::new(
                "setup.unknown_provider",
                format!("unknown provider `{other}`"),
            )
            .with_hint("choose one of: openrouter, openai, ollama, lmstudio, localai, custom"))
        }
    };
    if let Some(k) = key {
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::new(
                "setup.bad_key_env",
                "the API key variable must be a name like MY_API_KEY",
            )
            .with_hint("give the name of an environment variable, never the key itself"));
        }
    }
    let main = Backend {
        preset,
        endpoint,
        api_key: key.map(|k| format!("env:{k}")),
        model: model.to_string(),
        trust,
    };
    template_with(&main, None, shell)
}

/// An active starting config for `main`, and for a `checker` (a decision model that judges each suggested
/// command) when there is one. A checker reached the same way as `main` shares its provider.
pub fn template_with(
    main: &Backend,
    checker: Option<&Backend>,
    shell: Option<&str>,
) -> Result<String> {
    main.check()?;
    if let Some(c) = checker {
        c.check()?;
    }
    let level: u8 = if main.trust == "local" { 2 } else { 1 };
    let common = common(level, capture_key(shell), checker.is_some());
    let main_provider = main.provider_name();
    let mut tables = format!(
        "{}\n{}",
        main.provider_block(&main_provider),
        main.model_block("main", &main_provider, "builtin:openai-compatible")
    );
    if let Some(c) = checker {
        let provider = if c.same_connection(main) {
            main_provider.clone()
        } else {
            let own = c.provider_name();
            let own = if own == main_provider {
                "checker".to_string()
            } else {
                own
            };
            tables.push('\n');
            tables.push_str(&c.provider_block(&own));
            own
        };
        tables.push('\n');
        tables.push_str(&c.model_block("checker", &provider, "builtin:systemone"));
    }
    Ok(format!(
        r#"# shellmate configuration (docs: https://github.com/paulomenin/shellmate)
version = 1

# A provider is a connection (where the data goes). A model is what you use from it.
{tables}
[llm]
chain = ["main"]          # chat models, primary first; add fallbacks: ["main", "backup"]

{common}"#
    ))
}

/// Everything after the provider section, shared by the active and the commented starting config.
/// The checker example of a config that has none yet.
const COMMENTED_CHECKER: &str = r#"#
# [models.checker]
# provider = "openrouter"           # a provider of its own, or the one the chat model uses
# plugin = "builtin:systemone"      # a decision model: any server that speaks the System One format
# model = "~typesafe/jev-latest"    # a "latest" alias: it follows the newest release
# # path = "/v1/systemone"          # where the format is served below the endpoint
#
# Or call TypeSafe directly: a provider with preset = "typesafe" (reads TYPESAFE_API_KEY) and model = "jev-latest".
# A decision model on this machine keeps the check local: a provider with preset = "ollama" and trust = "local",
# with the model name your server lists.
"#;

fn common(level: u8, capture_key: &str, checker: bool) -> String {
    let (validator_on, decider_block) = if checker {
        (
            "chain = [\"checker\"]          # models that judge the command, tried in order; a chat model works too\n",
            "",
        )
    } else {
        (
            "# chain = [\"checker\"]          # models that judge the command, tried in order; a chat model works too\n",
            COMMENTED_CHECKER,
        )
    };
    format!(
        r#"[context]
# 0 = only what you typed, 1 = + shell/OS/cwd, 2 = + last exit status and recent commands
default_level = {level}
# project = true             # level 1+: what kind of project this is (file names only)
# git = true                 # level 2+: branch, ahead/behind and change counts
# git_log_lines = 0          # also send N recent commit subjects (text others wrote; taints the request)

[input.shortcuts]
"alt+m" = "ask"
"alt+i" = "generate"
"alt+j" = "explain"
"alt+k" = "new-conversation"
"{capture_key}" = "capture"       # wraps the line as `shellmate run -- ...`; the next question can use its output (context level 2)
# shellmate never overrides a key that is already bound: alt+r is revert-line in bash, alt+e is edit_command_buffer in fish.

[conversation]
idle_timeout = "30m"       # a conversation idle this long is replaced by a new one

[validator]
# Every suggested command is checked before it reaches your input line: it must parse in your shell, its
# programs must exist, and risky commands get a note. The command is always still placed; you decide.
# To also ask a model whether the command matches your request (off by default, so the chain is empty):
{validator_on}# timeout = "5s"              # raise it if the model loads slowly
# min_confidence = 0.8        # only findings at least this sure are shown
{decider_block}
[tools.exec]
# With `ask`, the model may ask to run ONE simple command (no pipes, redirects or globs) to look at your
# machine. Every command asks you first unless a rule below covers it. Commands such as rm, sudo or
# anything that installs or changes things always ask, even when a rule matches.
# approval = "deny" turns the tool off.
approval = "ask"
# preapproved = ["ls *", "pwd", "git status", "systemctl status *"]
# Let the model read a program's help without asking (name each program; a bare `*` is not accepted):
# preapproved = ["git --help", "git * --help", "docker --help", "docker * --help"]

[tools.help]
# With `ask`, the model may ask for the --help text of a program that has no man page (`help git commit`
# runs `git commit --help`; it cannot pass anything else). Every call asks you first unless a rule names the
# program. The classifier can still force a question, and the text counts as untrusted.
# approval = "deny" turns the tool off.
approval = "ask"
# preapproved = ["git", "docker", "cargo"]

[tools.read]
# With `ask`, the model may ask to read ONE text file in the working directory (a config, a log, source).
# Each file asks first, and the prompt says where the text goes. Credentials and binary files are never read.
# approval = "deny" turns the tool off.
approval = "ask"
# preapproved = ["docs/*", "*.md"]          # paths read without a prompt (never credentials)
# roots = ["~/notes"]                       # directories besides the working directory

[audit]
persist = false            # true keeps the audit log across logins (retention applies)
"#
    )
}

/// A starting config with no provider chosen: the examples are comments, to be uncommented one at a time.
/// It is a valid config. Until a provider is set up, `invoke` says so.
pub fn template_commented(shell: Option<&str>) -> String {
    let common = common(1, capture_key(shell), false);
    format!(
        r#"# shellmate configuration (docs: https://github.com/paulomenin/shellmate)
version = 1

# Choose where answers come from: uncomment ONE provider and its model below, and the [llm] chain, or run
# `shellmate setup`. A provider is a connection (where the data goes). A model is what you use from it.
# Until a provider is set up, shellmate says so and does nothing else.

# A hosted model through OpenRouter. The key is read from OPENROUTER_API_KEY; export it in your shell.
# A remote provider sees at most context level 1, and redaction must be on (it is by default).
#
# [providers.openrouter]
# trust = "remote"
# preset = "openrouter"
# api_key = "env:MY_KEY_VARIABLE"     # to use another key variable
#
# [models.main]
# provider = "openrouter"
# plugin = "builtin:openai-compatible"
# model = "~openai/gpt-luna-latest"   # a "latest" alias: it follows the newest release, and so does its price

# A model on this machine (Ollama, LM Studio or LocalAI): nothing leaves your computer.
#
# [providers.local]
# trust = "local"
# preset = "ollama"                   # or lmstudio, localai; or: endpoint = "http://localhost:8000"
#
# [models.local]
# provider = "local"
# plugin = "builtin:openai-compatible"
# model = "the-model-your-server-lists"

# [llm]
# chain = ["main"]          # chat models, primary first; add fallbacks: ["main", "local"]

{common}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellmate_core::ports::Host;
    use shellmate_core::Config;
    #[test]
    fn every_template_is_a_valid_config_without_inline_secrets() {
        for p in ["ollama", "lmstudio", "localai", "openai", "openrouter"] {
            let t = template(p, "some-model").unwrap();
            let c = Config::parse(&t).unwrap_or_else(|e| panic!("{p}: {}", e.message));
            c.validate_with(&shellmate_builtin::registry(), &Host::system())
                .ok(); // env refs may be unset here
            assert!(!t.contains("sk-"), "{p}");
        }
        assert!(template("nope", "m").is_err());
        assert!(template("ollama", "bad\"model").is_err());
    }

    /// Strip the leading `# ` from the lines of the example blocks the commented template offers.
    fn uncomment(text: &str) -> String {
        let mut on = false;
        let mut out = String::new();
        for line in text.lines() {
            let start = line.starts_with("# [providers.")
                || line.starts_with("# [models.")
                || line.starts_with("# [llm]")
                || line.starts_with("# chain = [\"checker\"]");
            if start {
                on = true;
            }
            let live = on
                && line.starts_with("# ")
                && !line.starts_with("# A ")
                && !line.starts_with("# Or ");
            if live {
                out.push_str(&line[2..]);
            } else {
                on = false;
                out.push_str(line);
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn the_commented_template_is_valid_and_its_examples_work_once_uncommented() {
        let plain = template_commented(Some("bash"));
        let c = Config::parse(&plain).unwrap();
        assert!(c.providers.is_empty() && c.models.is_empty());
        assert!(
            !plain.contains("qwen"),
            "no developer's model in the starting config"
        );
        assert!(
            plain.contains("~openai/gpt-luna-latest") && plain.contains("~typesafe/jev-latest")
        );
        assert!(
            plain.contains("preset = \"typesafe\"") && plain.contains("model = \"jev-latest\"")
        );
        assert!(
            plain.contains("\"alt+e\" = \"capture\""),
            "the shell picks the capture key"
        );

        let live = Config::parse(&uncomment(&plain)).unwrap();
        assert!(live.providers.contains_key("openrouter") && live.providers.contains_key("local"));
        assert!(live.models.contains_key("main") && live.models.contains_key("local"));
        assert!(live.models.contains_key("checker"));
        assert_eq!(live.validator.chain, ["checker"]);
        // the examples build the plugins they name, with no key value anywhere in the text
        let registry = shellmate_builtin::registry();
        live.validate_with(&registry, &Host::system()).ok(); // env refs may be unset here
        assert!(!uncomment(&plain).contains("sk-"));
    }

    #[test]
    fn custom_provider_needs_a_base_url_and_picks_trust_from_it() {
        assert!(template_for("custom", "m", None, None, None).is_err());
        assert!(template_for("custom", "m", Some("http://example.com/v1"), None, None).is_err()); // plain http, remote
        assert!(template_for(
            "custom",
            "m",
            Some("https://x.test/v1\"\ntrust = \"local"),
            None,
            None
        )
        .is_err());
        let remote = template_for(
            "custom",
            "m",
            Some("https://x.test/v1"),
            Some("X_KEY"),
            None,
        )
        .unwrap();
        assert!(
            remote.contains("endpoint = \"https://x.test/v1\"")
                && remote.contains("trust = \"remote\"")
        );
        assert!(remote.contains("api_key = \"env:X_KEY\""));
        Config::parse(&remote).unwrap();
        let local =
            template_for("custom", "m", Some("http://127.0.0.1:8000/v1"), None, None).unwrap();
        assert!(local.contains("trust = \"local\"") && !local.contains("\napi_key"));
        Config::parse(&local).unwrap();
        // a key given as a literal value is refused
        assert!(template_for(
            "custom",
            "m",
            Some("https://x.test/v1"),
            Some("sk-abc.def"),
            None
        )
        .is_err());
        // a "local-looking" host that is not loopback stays remote
        assert!(!is_loopback_url("https://localhost.evil.test/v1"));
        assert!(!is_loopback_url("http://127.0.0.1@evil.test/v1"));
    }

    #[test]
    fn the_starting_config_binds_every_action_with_a_free_capture_key() {
        for (shell, key) in [
            (None, "alt+r"),
            (Some("fish"), "alt+r"),
            (Some("zsh"), "alt+r"),
            (Some("bash"), "alt+e"),
        ] {
            let t = template_for("ollama", "m", None, None, shell).unwrap();
            let c = Config::parse(&t).unwrap();
            let actions: std::collections::BTreeMap<_, _> = c
                .input
                .shortcuts
                .iter()
                .map(|(k, a)| (a.as_str(), k.as_str()))
                .collect();
            for a in ["ask", "generate", "explain", "new-conversation"] {
                assert!(actions.contains_key(a), "{shell:?}: {a}");
            }
            assert_eq!(actions["capture"], key, "{shell:?}");
        }
    }

    #[test]
    fn remote_templates_cap_context_and_use_env_references() {
        let t = template("openai", "m").unwrap();
        assert!(t.contains("trust = \"remote\"") && t.contains("default_level = 1"));
        assert!(t.contains("api_key = \"env:OPENAI_API_KEY\""));
    }
}
