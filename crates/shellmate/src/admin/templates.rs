//! The starting configs `config init` and `setup` write, one per provider preset.

use shellmate_api::{Error, Result};

/// A documented starting config for a provider preset.
pub fn template(provider: &str, model: &str) -> Result<String> {
    template_for(provider, model, None, None)
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

/// Like [`template`], and for the `custom` provider: any OpenAI-compatible server at `base_url`, with an
/// optional environment variable that holds its API key.
pub fn template_for(
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    key_env: Option<&str>,
) -> Result<String> {
    let bad = |c: char| c == '"' || c == '\n' || c == '\\' || c.is_control();
    let (trust, key, settings_head) = match provider {
        "ollama" | "lmstudio" | "localai" => ("local", None, format!("preset = \"{provider}\"")),
        "openai" => (
            "remote",
            Some("OPENAI_API_KEY"),
            "preset = \"openai\"".into(),
        ),
        "openrouter" => (
            "remote",
            Some("OPENROUTER_API_KEY"),
            "preset = \"openrouter\"".into(),
        ),
        "custom" => {
            let url = base_url.map(str::trim).unwrap_or("");
            if url.is_empty() || url.contains(bad) {
                return Err(Error::new("setup.bad_base_url", "a base URL is needed")
                    .with_hint("e.g. http://localhost:8000/v1 or https://api.example.com/v1"));
            }
            shellmate_api::check_endpoint(url)?;
            let trust = if is_loopback_url(url) {
                "local"
            } else {
                "remote"
            };
            (trust, key_env, format!("endpoint = \"{url}\""))
        }
        other => {
            return Err(Error::new(
                "setup.unknown_provider",
                format!("unknown provider `{other}`"),
            )
            .with_hint("choose one of: ollama, lmstudio, localai, openai, openrouter, custom"))
        }
    };
    if model.trim().is_empty() || model.contains(bad) {
        return Err(Error::new("setup.bad_model", "invalid model name"));
    }
    if let Some(k) = key {
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::new(
                "setup.bad_key_env",
                "the API key variable must be a name like MY_API_KEY",
            )
            .with_hint("give the name of an environment variable, never the key itself"));
        }
    }
    let key_line = key
        .map(|k| format!("api_key = \"env:{k}\"   # a reference; never paste the key here\n"))
        .unwrap_or_default();
    let level = if trust == "local" { 2 } else { 1 };
    Ok(format!(
        r#"# shellmate configuration (docs: https://github.com/paulomenin/shellmate)
version = 1

[providers.main]
plugin = "builtin:openai-compatible"
# local: runs on this machine (may see more context, redaction optional)
# remote: leaves this machine (context capped at level 1, redaction required)
trust = "{trust}"
[providers.main.settings]
{settings_head}
model = "{model}"
{key_line}# reasoning_effort = "none"   # much faster with reasoning models (e.g. gemma4)
# timeout_secs = 60

[llm]
chain = ["main"]          # add fallbacks: ["main", "backup"]

[context]
# 0 = only what you typed, 1 = + shell/OS/cwd, 2 = + last exit status and recent commands
default_level = {level}
# project = true             # level 1+: what kind of project this is (file names only)
# git = true                 # level 2+: branch, ahead/behind and change counts
# git_log_lines = 0          # also send N recent commit subjects (text others wrote; taints the request)

[input.shortcuts]
"alt+m" = "ask"
"alt+i" = "generate"
"alt+j" = "explain"
# "alt+k" = "new-conversation"
# "alt+r" = "capture"          # wraps the line as `shellmate run -- ...`; the next question can use its output (context level 2)

[conversation]
idle_timeout = "30m"       # a conversation idle this long is replaced by a new one

[validator]
# Every suggested command is checked before it reaches your input line: it must parse in your shell, its
# programs must exist, and risky commands get a note. The command is always still placed; you decide.
# To also ask a decision model whether the command matches your request (off by default):
# semantic = true
# deciders = ["checker"]      # tried in order; with none, the LLM answers the same questions
# timeout = "5s"              # raise it if the model loads slowly
# min_confidence = 0.8        # only findings at least this sure are shown
#
# [deciders.checker]
# plugin = "builtin:systemone"      # any server that speaks POST /v1/systemone (Ollama, Ollaya, hosted)
# trust = "local"
# [deciders.checker.settings]
# preset = "ollama"                 # or: endpoint = "https://..."
# model = "laya:latest"
# api_key = "env:CHECKER_API_KEY"   # only for hosted endpoints

[tools.exec]
# With `ask`, the model may ask to run ONE simple command (no pipes, redirects or globs) to look at your
# machine. Every command asks you first unless a rule below covers it. Commands such as rm, sudo or
# anything that installs or changes things always ask, even when a rule matches.
# approval = "deny" turns the tool off.
approval = "ask"
# preapproved = ["ls *", "pwd", "git status", "systemctl status *"]

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
    ))
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

    #[test]
    fn custom_provider_needs_a_base_url_and_picks_trust_from_it() {
        assert!(template_for("custom", "m", None, None).is_err());
        assert!(template_for("custom", "m", Some("http://example.com/v1"), None).is_err()); // plain http, remote
        assert!(template_for(
            "custom",
            "m",
            Some("https://x.test/v1\"\ntrust = \"local"),
            None
        )
        .is_err());
        let remote = template_for("custom", "m", Some("https://x.test/v1"), Some("X_KEY")).unwrap();
        assert!(
            remote.contains("endpoint = \"https://x.test/v1\"")
                && remote.contains("trust = \"remote\"")
        );
        assert!(remote.contains("api_key = \"env:X_KEY\""));
        Config::parse(&remote).unwrap();
        let local = template_for("custom", "m", Some("http://127.0.0.1:8000/v1"), None).unwrap();
        assert!(local.contains("trust = \"local\"") && !local.contains("\napi_key"));
        Config::parse(&local).unwrap();
        // a key given as a literal value is refused
        assert!(
            template_for("custom", "m", Some("https://x.test/v1"), Some("sk-abc.def")).is_err()
        );
        // a "local-looking" host that is not loopback stays remote
        assert!(!is_loopback_url("https://localhost.evil.test/v1"));
        assert!(!is_loopback_url("http://127.0.0.1@evil.test/v1"));
    }

    #[test]
    fn remote_templates_cap_context_and_use_env_references() {
        let t = template("openai", "m").unwrap();
        assert!(t.contains("trust = \"remote\"") && t.contains("default_level = 1"));
        assert!(t.contains("api_key = \"env:OPENAI_API_KEY\""));
    }
}
