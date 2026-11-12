//! `config init` and `config show`.

use super::config_path;
use super::doctor::shell_name;
use super::templates::template_for;
use shellmate_api::{Error, Result};
use shellmate_core::ports::SystemEnv;
use shellmate_core::present::{Presenter, Tone};
use shellmate_core::{audit, secrets, Config};
use std::path::Path;

pub fn config_init(
    explicit: Option<&Path>,
    provider: &str,
    model: &str,
    force: bool,
) -> Result<String> {
    // the shell in $SHELL picks a free key for `capture`
    let shell = shell_name(&SystemEnv);
    let text = template_for(provider, model, None, None, shell.as_deref())?;
    write_config(explicit, text, force)
}

/// Write an already rendered starting config (after checking it parses), refusing to overwrite without `force`.
pub fn write_config(explicit: Option<&Path>, text: String, force: bool) -> Result<String> {
    let path = config_path(explicit, &SystemEnv);
    if path.exists() && !force {
        return Err(Error::new(
            "config.exists",
            format!("{} already exists", path.display()),
        )
        .with_hint("pass --force to overwrite it, or edit it by hand"));
    }
    Config::parse(&text)?; // the template must always be valid
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::new("config.unwritable", format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&path, text)
        .map_err(|e| Error::new("config.unwritable", format!("{}: {e}", path.display())))?;
    Ok(format!("wrote {}\n", path.display()))
}

/// The effective configuration in words. Secrets appear only as the references written in the file.
pub fn config_show(explicit: Option<&Path>, p: &dyn Presenter) -> Result<String> {
    let path = config_path(explicit, &SystemEnv);
    let cfg = Config::load(explicit)?;
    let mut o = String::new();
    let found = if path.exists() {
        path.display().to_string()
    } else {
        format!("{} (not found; built-in defaults)", path.display())
    };
    o.push_str(&format!("config:        {found}\n"));
    o.push_str(&format!(
        "chain:         {}\n",
        cfg.provider_chain()
            .map(|c| c.join(" -> "))
            .unwrap_or_else(|_| "(none)".into())
    ));
    for (name, p) in &cfg.providers {
        let settings: Vec<String> = p
            .settings
            .iter()
            .map(|(k, v)| secrets::show_setting(k, v, &SystemEnv))
            .collect();
        o.push_str(&format!(
            "provider {name}: {} trust={:?} max_level={} {}\n",
            p.plugin,
            p.trust,
            p.effective_max_level(),
            settings.join(" ")
        ));
    }
    for (name, d) in &cfg.deciders {
        let settings: Vec<String> = d
            .settings
            .iter()
            .map(|(k, v)| secrets::show_setting(k, v, &SystemEnv))
            .collect();
        o.push_str(&format!(
            "decider {name}:  {} trust={:?} {}\n",
            d.plugin,
            d.trust,
            settings.join(" ")
        ));
    }
    let v = &cfg.validator;
    o.push_str(&format!(
        "validator:     checks={} semantic={} deciders={} llm_fallback={} min_confidence={} timeout={}\n",
        v.enabled,
        v.semantic,
        if v.deciders.is_empty() { "(none)".to_string() } else { v.deciders.join(" -> ") },
        v.llm_fallback,
        v.min_confidence,
        v.timeout
    ));
    o.push_str(&format!("context level: {}\n", cfg.context.default_level));
    o.push_str(&format!(
        "llm pipeline:  {}\n",
        cfg.llm_layers().join(" -> ")
    ));
    let keys: Vec<String> = cfg
        .input
        .shortcuts
        .iter()
        .map(|(k, a)| format!("{k}={a}"))
        .collect();
    o.push_str(&format!("shortcuts:     {}\n", keys.join(" ")));
    o.push_str(&format!(
        "conversation:  store={} idle={} max_turns={} tool_rounds={}\n",
        cfg.conversation.store,
        cfg.conversation.idle_timeout,
        cfg.conversation.max_turns,
        cfg.conversation.max_tool_rounds
    ));
    o.push_str(&format!(
        "read tool:     enabled={} approval={:?} preapproved={} roots={} max_bytes={}\n",
        cfg.tools.read.enabled,
        cfg.tools.read.approval,
        cfg.tools.read.preapproved.len(),
        cfg.tools.read.roots.len(),
        cfg.tools.read.max_bytes
    ));
    // the built-in `man` and `exec` are on without a table, so they are listed either way
    let man = shellmate_core::config::ToolEntry::default();
    let listed = ["man", "exec"].map(|n| (n, &man)).into_iter().chain(
        cfg.tools
            .other
            .iter()
            .filter(|(n, _)| !matches!(n.as_str(), "man" | "exec"))
            .map(|(n, t)| (n.as_str(), t)),
    );
    for (name, t) in listed {
        let t = cfg.tools.other.get(name).unwrap_or(t);
        o.push_str(&format!(
            "{:<14} enabled={} approval={} preapproved={}\n",
            format!("{name} tool:"),
            t.enabled,
            t.approval
                .map_or("default".to_string(), |a| format!("{a:?}").to_lowercase()),
            t.preapproved.len()
        ));
    }
    o.push_str(&format!(
        "collectors:    project={} git={} git_log_lines={}\n",
        cfg.context.project, cfg.context.git, cfg.context.git_log_lines
    ));
    if cfg.secrets.cmd_timeout != "5s" || !cfg.secrets.pass_env.is_empty() {
        o.push_str(&format!(
            "secrets:       cmd_timeout={} pass_env={}\n",
            cfg.secrets.cmd_timeout,
            if cfg.secrets.pass_env.is_empty() {
                "(none)".to_string()
            } else {
                cfg.secrets.pass_env.join(",")
            }
        ));
    }
    o.push_str(&format!(
        "audit:         enabled={} persist={} file={}\n",
        cfg.audit.enabled,
        cfg.audit.persist,
        audit::log_path(&cfg.audit, &SystemEnv).display()
    ));
    // each line is `key: value`: the key is background, the value is what to read
    let styled = o
        .lines()
        .map(|line| match line.split_once(':') {
            Some((key, value)) => format!("{}{value}\n", p.label(Tone::Muted, &format!("{key}:"))),
            None => format!("{line}\n"),
        })
        .collect::<String>();
    Ok(styled)
}
