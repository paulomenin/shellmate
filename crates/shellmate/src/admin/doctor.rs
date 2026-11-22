//! `doctor`: checks of the config, the providers, the shell integration and the environment.
//!
//! Each check is a small function from the loaded config and the environment to a list of results, and
//! [`CHECKS`] says in which order they run. A check looks at the world through [`Host`], so a test gives
//! it a `MapEnv` and a `MemFs` instead of the machine. Only the network probe (`reachable`) and the path
//! of this executable are real.

use super::config_path;
use super::install_shell::{rc_path, BEGIN};
use serde_json::{json, Value};
use shellmate_api::{ModelKind, Registry};
use shellmate_core::config::{builtin_id, parse_duration, ModelInspection};
use shellmate_core::ports::{Env, Host};
use shellmate_core::present::{Presenter, Tone};
use shellmate_core::{audit, conversation, Config, Engine};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub remedy: Option<String>,
}

fn check(
    name: &'static str,
    status: Status,
    detail: impl Into<String>,
    remedy: Option<&str>,
) -> Check {
    Check {
        name,
        status,
        detail: detail.into(),
        remedy: remedy.map(str::to_string),
    }
}

/// What a check may look at.
struct Ctx<'a> {
    cfg: &'a Config,
    registry: &'a Registry,
    host: &'a Host,
    /// Where the config file is (or would be).
    path: &'a Path,
    /// The models that can be built, looked at once (a model that cannot is reported by the `config` check).
    models: Vec<ModelInspection>,
}

impl<'a> Ctx<'a> {
    fn new(cfg: &'a Config, registry: &'a Registry, host: &'a Host, path: &'a Path) -> Self {
        let models = cfg
            .models
            .keys()
            .filter_map(|name| cfg.inspect_model(name, registry, host).ok())
            .collect();
        Self {
            cfg,
            registry,
            host,
            path,
            models,
        }
    }

    fn model(&self, name: &str) -> Option<&ModelInspection> {
        self.models.iter().find(|m| m.name == name)
    }
}

type CheckFn = fn(&Ctx<'_>) -> Vec<Check>;

/// The checks that need a loaded config, in the order they are reported. Reading the config itself
/// comes first and stops everything else when it fails (see `doctor_checks_in`).
const CHECKS: &[CheckFn] = &[
    config_is_valid,
    providers,
    secrets,
    deciders,
    shell_and_integration,
    storage,
    terminal,
    man_tool,
];

/// `host:port` of an http(s) URL, without credentials.
fn host_port(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    Some(if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:{}", if scheme == "https" { 443 } else { 80 })
    })
}

pub(super) fn reachable(hp: &str) -> bool {
    use std::net::ToSocketAddrs;
    // try every resolved address: `localhost` is often ::1 first while the server listens on IPv4 only
    hp.to_socket_addrs().is_ok_and(|mut addrs| {
        addrs.any(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_millis(1500)).is_ok())
    })
}

pub(super) fn shell_name(env: &dyn Env) -> Option<String> {
    env.var("SHELL")
        .and_then(|s| s.rsplit('/').next().map(str::to_string))
}

/// Every check, for the real machine.
pub fn doctor_checks(explicit: Option<&Path>) -> Vec<Check> {
    doctor_checks_in(explicit, &Host::system(), &shellmate_builtin::registry())
}

fn doctor_checks_in(explicit: Option<&Path>, host: &Host, registry: &Registry) -> Vec<Check> {
    let mut v = vec![check(
        "version",
        Status::Ok,
        format!("shellmate {}", env!("CARGO_PKG_VERSION")),
        None,
    )];
    let cfg = match Config::load_from(explicit, host) {
        Ok(c) => c,
        Err(e) => {
            v.push(check(
                "config",
                Status::Fail,
                format!("{}: {}", e.code, e.message),
                e.hint.as_deref(),
            ));
            return v;
        }
    };
    let path = config_path(explicit, host.env.as_ref());
    let ctx = Ctx::new(&cfg, registry, host, &path);
    for run in CHECKS {
        v.extend(run(&ctx));
    }
    v
}

fn config_is_valid(ctx: &Ctx<'_>) -> Vec<Check> {
    let path = ctx.path;
    if ctx.host.fs.metadata(path).is_err() {
        return vec![check(
            "config",
            Status::Fail,
            format!("{} does not exist", path.display()),
            Some("run `shellmate setup`"),
        )];
    }
    vec![match Engine::inspect(ctx.cfg, ctx.registry, ctx.host) {
        Ok(_) => check(
            "config",
            Status::Ok,
            format!("{} is valid", path.display()),
            None,
        ),
        Err(e) => check(
            "config",
            Status::Fail,
            format!("{}: {}", e.code, e.message),
            e.hint.as_deref(),
        ),
    }]
}

/// Endpoint reachability (a TCP connect; nothing is sent).
fn providers(ctx: &Ctx<'_>) -> Vec<Check> {
    let mut v = Vec::new();
    let Ok(chain) = ctx.cfg.model_chain() else {
        return v;
    };
    for name in chain {
        let Some((model, endpoint)) = ctx
            .model(name)
            .and_then(|m| m.info.endpoint.clone().map(|e| (m.info.model.clone(), e)))
        else {
            continue;
        };
        v.push(match host_port(&endpoint) {
            Some(hp) if reachable(&hp) => check("provider", Status::Ok, format!("{name}: {model} at {hp} is reachable"), None),
            Some(hp) => check("provider", Status::Warn, format!("{name}: cannot connect to {hp}"), Some("start the server (for Ollama, `ollama serve`) or check the endpoint; a fallback provider will be used if configured")),
            None => check("provider", Status::Warn, format!("{name}: unrecognised endpoint"), None),
        });
    }
    v
}

/// `cmd:` secrets of the models in the `llm` chain and the validator chain: run each program once and say
/// whether it worked. This is the one place besides a request that runs them (`config check` only checks that
/// the line is valid). Nothing about the value is shown, not even its length.
fn secrets(ctx: &Ctx<'_>) -> Vec<Check> {
    let Ok(mut chain) = ctx.cfg.model_chain() else {
        return Vec::new();
    };
    for name in &ctx.cfg.validator.chain {
        if !chain.contains(&name.as_str()) {
            chain.push(name);
        }
    }
    let mut v = Vec::new();
    for name in chain {
        let Ok(lazy) = ctx.cfg.lazy_model_settings(name, ctx.host) else {
            continue; // `config` already reported it
        };
        if !lazy.is_lazy() {
            continue;
        }
        let started = std::time::Instant::now();
        v.push(match lazy.complete(ctx.host) {
            Ok(_) => check(
                "secret",
                Status::Ok,
                format!(
                    "{name}: the `cmd:` secret works ({} ms)",
                    started.elapsed().as_millis()
                ),
                None,
            ),
            Err(e) => check(
                "secret",
                Status::Fail,
                format!("{name}: {}: {}", e.code, e.message),
                e.hint.as_deref(),
            ),
        });
    }
    v
}

/// The models of the semantic command check: the same reachability test, nothing is sent. A chat model that
/// is also in the `llm` chain is checked there.
fn deciders(ctx: &Ctx<'_>) -> Vec<Check> {
    let cfg = ctx.cfg;
    let mut v = Vec::new();
    if !(cfg.validator.enabled && !cfg.validator.chain.is_empty()) {
        return v;
    }
    let mut decision = false;
    for name in &cfg.validator.chain {
        // the kind of a model that could not be built still comes from its plugin id: it is then reported below
        let kind = ctx.model(name).map(|m| m.kind).or_else(|| {
            let id = builtin_id(&cfg.models.get(name)?.plugin).ok()?;
            ctx.registry.model_kind(id).ok()
        });
        if kind != Some(ModelKind::Decision) {
            continue;
        }
        decision = true;
        let info = ctx.model(name).map(|m| m.info.clone());
        v.push(match info.and_then(|i| i.endpoint.map(|e| (i.model, e))) {
            Some((model, endpoint)) => match host_port(&endpoint) {
                Some(hp) if reachable(&hp) => check(
                    "decider",
                    Status::Ok,
                    format!("{name}: {model} at {hp} is reachable (the model itself is not loaded by this check)"),
                    None,
                ),
                Some(hp) => check(
                    "decider",
                    Status::Warn,
                    format!("{name}: cannot connect to {hp}"),
                    Some("start the server or fix the endpoint; the next model of [validator] chain is used until then"),
                ),
                None => check("decider", Status::Warn, format!("{name}: unrecognised endpoint"), None),
            },
            None => check(
                "decider",
                Status::Warn,
                format!("{name}: could not be built"),
                Some("run `shellmate config check`"),
            ),
        });
    }
    // a decision model that is not kept in memory takes seconds to load: the first check would time out
    if let Ok(t) = parse_duration(&cfg.validator.timeout) {
        if t < Duration::from_secs(15) && decision {
            v.push(check(
                "decider",
                Status::Warn,
                format!("validator timeout is {}; a decision model that has to be loaded first often needs longer", cfg.validator.timeout),
                Some("set [validator] timeout = \"20s\", or keep the model loaded in your model server so it is not unloaded between requests"),
            ));
        }
    }
    v
}

fn shell_and_integration(ctx: &Ctx<'_>) -> Vec<Check> {
    let mut v = Vec::new();
    match shell_name(ctx.host.env.as_ref()) {
        Some(sh) if ["fish", "zsh", "bash"].contains(&sh.as_str()) => {
            v.push(check(
                "shell",
                Status::Ok,
                format!("login shell is {sh}"),
                None,
            ));
            if let Ok(rc) = rc_path(&sh) {
                let text = ctx.host.fs.read_to_string(&rc).unwrap_or_default();
                let exe = std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                v.push(integration(&text, &exe, &rc));
            }
        }
        Some(sh) => v.push(check(
            "shell",
            Status::Warn,
            format!("login shell {sh} is not supported"),
            Some("shellmate supports fish, zsh and bash"),
        )),
        None => v.push(check("shell", Status::Warn, "$SHELL is not set", None)),
    }
    if ctx.host.env.var("SHELLMATE_SESSION").is_some() {
        v.push(check(
            "session",
            Status::Ok,
            "this shell has the integration loaded",
            None,
        ));
    }
    v
}

/// Whether the rc file `text` loads this executable.
fn integration(text: &str, exe: &str, rc: &Path) -> Check {
    if text.contains(BEGIN) && text.contains(exe) {
        check(
            "integration",
            Status::Ok,
            format!("installed in {}", rc.display()),
            None,
        )
    } else if text.contains(BEGIN) {
        check(
            "integration",
            Status::Warn,
            format!("{} points at another binary", rc.display()),
            Some("run `shellmate install-shell` to update it"),
        )
    } else if text.contains("shellmate") {
        check(
            "integration",
            Status::Ok,
            format!(
                "a shellmate line exists in {} (not managed by install-shell)",
                rc.display()
            ),
            None,
        )
    } else {
        check(
            "integration",
            Status::Warn,
            format!("not found in {}", rc.display()),
            Some("run `shellmate install-shell`"),
        )
    }
}

/// The conversation store and the audit log.
fn storage(ctx: &Ctx<'_>) -> Vec<Check> {
    let conversations = match conversation::open_store(ctx.cfg, ctx.registry, ctx.host) {
        Ok(Some(_)) => check("conversations", Status::Ok, "store is usable", None),
        Ok(None) => check(
            "conversations",
            Status::Ok,
            "turned off (store = \"none\")",
            None,
        ),
        Err(e) => check(
            "conversations",
            Status::Warn,
            e.message,
            Some("follow-ups are off until this is fixed"),
        ),
    };
    let audit_log = if ctx.cfg.audit.enabled {
        let ap = audit::log_path(&ctx.cfg.audit, ctx.host.env.as_ref());
        check(
            "audit",
            Status::Ok,
            format!("log at {}", ap.display()),
            None,
        )
    } else {
        check("audit", Status::Ok, "turned off", None)
    };
    vec![conversations, audit_log]
}

fn terminal(ctx: &Ctx<'_>) -> Vec<Check> {
    let utf8 = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| ctx.host.env.var(k))
        .find(|v| !v.is_empty())
        .is_some_and(|v| v.to_lowercase().contains("utf"));
    vec![if utf8 {
        check("terminal", Status::Ok, "UTF-8 locale", None)
    } else {
        check(
            "terminal",
            Status::Warn,
            "locale is not UTF-8; output falls back to ASCII",
            Some("set LANG to a UTF-8 locale"),
        )
    }]
}

fn man_tool(ctx: &Ctx<'_>) -> Vec<Check> {
    if !ctx.cfg.tools.enabled("man") {
        return Vec::new();
    }
    let have = ctx.host.env.var("PATH").is_some_and(|p| {
        p.split(':').any(|d| {
            ctx.host
                .fs
                .metadata(&Path::new(d).join("man"))
                .is_ok_and(|m| m.is_file)
        })
    });
    vec![if have {
        check("man tool", Status::Ok, "`man` found", None)
    } else {
        check(
            "man tool",
            Status::Warn,
            "`man` is not installed; the model cannot read manual pages",
            Some("install man-db/mandoc, or set [tools.man] enabled = false"),
        )
    }]
}

pub fn doctor(explicit: Option<&Path>, json: bool) -> (String, bool) {
    // the report is read in a terminal, so it follows the user's `[ui]` settings
    if let Ok(cfg) = Config::load(explicit) {
        crate::commands::ui::use_config(&cfg);
    }
    let checks = doctor_checks(explicit);
    render(&checks, json, &*crate::commands::ui::stdout())
}

fn render(checks: &[Check], json: bool, p: &dyn Presenter) -> (String, bool) {
    let failed = checks.iter().any(|c| c.status == Status::Fail);
    let label = |s: Status| match s {
        Status::Ok => "ok",
        Status::Warn => "warn",
        Status::Fail => "fail",
    };
    if json {
        let arr: Vec<Value> = checks
            .iter()
            .map(|c| json!({"name": c.name, "status": label(c.status), "detail": c.detail, "remedy": c.remedy}))
            .collect();
        return (
            serde_json::to_string_pretty(&arr).unwrap_or_default() + "\n",
            failed,
        );
    }
    let tone = |s: Status| match s {
        Status::Ok => Tone::Success,
        Status::Warn => Tone::Warning,
        Status::Fail => Tone::Error,
    };
    let mut o = String::new();
    for c in checks {
        o.push_str(&format!(
            "{} {:<14} {}\n",
            p.label(
                tone(c.status),
                &format!("{:<5}", label(c.status).to_uppercase())
            ),
            c.name,
            c.detail
        ));
        if let (Some(r), true) = (&c.remedy, c.status != Status::Ok) {
            o.push_str(&format!(
                "      {}\n",
                p.label(Tone::Muted, &format!("-> {r}"))
            ));
        }
    }
    (o, failed)
}

#[cfg(test)]
mod tests;
