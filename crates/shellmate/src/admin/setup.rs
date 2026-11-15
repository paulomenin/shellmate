//! `setup`: choose a provider (and a checker) and a shell, write the config, install the shell block and
//! run `doctor`.
//!
//! Interactive setup is a guided conversation: it looks at the machine first (key variables that are set,
//! local servers that answer), lists what it found, asks where the key lives, confirms the connection by
//! asking the server for its models, and then asks for the model name, which is never guessed. With `--yes`
//! every one of those answers is a flag and nothing is asked or contacted, unless `--verify` is given.

use super::detect::{detect, place, probe_port, Candidate, Place};
use super::doctor::{doctor, shell_name};
use super::install_shell::install_shell;
use super::keyref::KeyRef;
use super::prompter::{ask_required, choose, confirm, Prompter, Terminal};
use super::templates::{is_loopback_url, template_with, Backend};
use super::{config_cmd::write_config, config_path};
use shellmate_api::{Error, Preset, Result, PRESETS};
use shellmate_core::ports::{Env, Host, SystemEnv};
use shellmate_core::present::Tone;
use std::io::IsTerminal;
use std::path::Path;

#[cfg(test)]
mod tests;

pub struct SetupArgs {
    pub yes: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub shell: Option<String>,
    pub base_url: Option<String>,
    /// `env:NAME` or `file:PATH`.
    pub api_key: Option<String>,
    /// The name of an environment variable. The older way to say `--api-key env:NAME`.
    pub api_key_env: Option<String>,
    pub checker: Option<String>,
    pub checker_model: Option<String>,
    pub checker_url: Option<String>,
    pub checker_api_key: Option<String>,
    /// With `--yes`: also ask each server for its models, to confirm the address and the key.
    pub verify: bool,
}

/// Which of the two backends is being set up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Provider,
    /// A decision model that judges each suggested command.
    Checker,
}

/// The models a server lists: `None` when it cannot list them, and an empty list when its reply was unreadable.
type Listing = Result<Option<Vec<String>>>;

/// What setup needs from the machine, so a test can supply it.
pub(super) struct Services<'a> {
    pub env: &'a dyn Env,
    /// Does anything accept a connection on this port of this machine?
    pub probe: &'a dyn Fn(u16) -> bool,
    /// Ask the backend's server for its models. This sends the key and nothing else.
    pub connect: &'a dyn Fn(Kind, &Backend) -> Listing,
}

#[derive(Debug)]
pub(super) struct Chosen {
    pub main: Backend,
    pub checker: Option<Backend>,
}

fn cancelled() -> Error {
    Error::new("setup.cancelled", "setup cancelled")
}

const CUSTOM: &str = "custom";

fn presets_for(kind: Kind) -> Vec<&'static Preset> {
    match kind {
        Kind::Provider => PRESETS.iter().collect(),
        Kind::Checker => shellmate_builtin::decider_systemone::checker_presets(),
    }
}

fn names(kind: Kind) -> String {
    let mut n: Vec<_> = presets_for(kind).iter().map(|p| p.name).collect();
    n.push(CUSTOM);
    n.join(", ")
}

// ---- building a backend -------------------------------------------------------------------------------

fn local_backend(p: &Preset, port: u16, key: Option<KeyRef>, model: &str) -> Backend {
    let default = matches!(place(p), Place::Local { port: d } if d == port);
    Backend {
        preset: default.then(|| p.name.to_string()),
        endpoint: (!default).then(|| format!("http://localhost:{port}")),
        api_key: key.map(|k| k.as_config()),
        model: model.to_string(),
        trust: "local",
    }
}

fn remote_backend(p: &Preset, key: Option<KeyRef>, model: &str) -> Backend {
    Backend {
        preset: Some(p.name.to_string()),
        endpoint: None,
        api_key: key.map(|k| k.as_config()),
        model: model.to_string(),
        trust: "remote",
    }
}

fn custom_backend(url: &str, key: Option<KeyRef>, model: &str) -> Backend {
    Backend {
        preset: None,
        endpoint: Some(url.trim().to_string()),
        api_key: key.map(|k| k.as_config()),
        model: model.to_string(),
        trust: if is_loopback_url(url) {
            "local"
        } else {
            "remote"
        },
    }
}

// ---- the guided conversation --------------------------------------------------------------------------

enum Pick {
    Preset(&'static Preset, Candidate),
    Custom,
}

pub(super) fn guided(p: &mut dyn Prompter, svc: &Services) -> Result<Chosen> {
    p.say("Where should answers come from?");
    let main = pick(p, svc, Kind::Provider)?.ok_or_else(cancelled)?;
    p.say("");
    let checker = pick(p, svc, Kind::Checker)?;
    Ok(Chosen { main, checker })
}

/// List what is available for `kind`, let the person choose, and set the choice up. `None` is "no checker".
fn pick(p: &mut dyn Prompter, svc: &Services, kind: Kind) -> Result<Option<Backend>> {
    let mut found = detect(presets_for(kind), svc.env, svc.probe);
    if kind == Kind::Checker {
        // a checker is only offered when something that serves it was found
        found.retain(|c| c.detected);
        if found.is_empty() {
            p.say("No decision model was found, so there is no checker. It is optional: add one later with `shellmate config edit`.");
            return Ok(None);
        }
        p.say("A checker is a decision model that judges whether each suggested command does what you asked.");
    }
    let mut items: Vec<String> = found
        .iter()
        .map(|c| format!("{:<11} {}", c.preset, c.describe()))
        .collect();
    let (custom, none) = match kind {
        Kind::Provider => {
            items.push(format!(
                "{CUSTOM:<11} any OpenAI-compatible server (you give the address)"
            ));
            (Some(items.len() - 1), None)
        }
        Kind::Checker => {
            items.push(format!(
                "{:<11} no checker: the LLM answers the same questions",
                "none"
            ));
            (None, Some(items.len() - 1))
        }
    };
    // what was found is the default; with nothing found the person has to choose
    let default = match kind {
        Kind::Provider => found.first().filter(|c| c.detected).map(|_| 0),
        Kind::Checker => none,
    };
    let n = choose(p, "choice", &items, default)?;
    if Some(n) == none {
        return Ok(None);
    }
    let sel = if Some(n) == custom {
        Pick::Custom
    } else {
        let cand = found[n].clone();
        let preset = presets_for(kind)
            .into_iter()
            .find(|x| x.name == cand.preset)
            .expect("a candidate comes from the presets");
        Pick::Preset(preset, cand)
    };
    set_up(p, svc, kind, &sel).map(Some)
}

fn set_up(p: &mut dyn Prompter, svc: &Services, kind: Kind, sel: &Pick) -> Result<Backend> {
    let mut consented = false;
    let mut again = false;
    let (mut backend, listed) = loop {
        let mut b = gather(p, kind, sel, again, &mut consented)?;
        p.say(&format!("checking {} ...", target(&b)));
        // a model name is not needed to list models
        b.model = "-".into();
        match (svc.connect)(kind, &b) {
            Ok(ids) => break (b, ids),
            Err(e) => {
                p.say(&format!("could not confirm the connection: {}", e.message));
                if let Some(h) = &e.hint {
                    p.say(&format!("  {h}"));
                }
                let next = choose(
                    p,
                    "what now?",
                    &[
                        "try again, changing the key or the address".to_string(),
                        "continue anyway: write the config as it is".to_string(),
                        "cancel setup".to_string(),
                    ],
                    Some(0),
                )?;
                match next {
                    0 => again = true,
                    1 => break (b, None),
                    _ => return Err(cancelled()),
                }
            }
        }
    };
    let known = match &listed {
        Some(ids) if !ids.is_empty() => {
            p.say(&format!("connected; the server lists {} models", ids.len()));
            ids.clone()
        }
        Some(_) => {
            p.say("connected; its list of models could not be read");
            Vec::new()
        }
        None => {
            p.say("connected; this server does not list its models");
            Vec::new()
        }
    };
    let question = match kind {
        Kind::Provider => "model name (as the provider lists it)",
        Kind::Checker => "checker model name (as the provider lists it)",
    };
    backend.model = ask_required(p, question)?;
    if !known.is_empty() && !known.contains(&backend.model) {
        p.say("note: that name is not in the server's list. Aliases such as `~vendor/model-latest` are often not listed, so it is kept.");
    }
    Ok(backend)
}

/// Where it is and how to reach it. `again` is true after a failed check: the key is asked for even when
/// a variable was found.
///
/// A remote service has to be agreed to once, before the key is asked for and before anything is contacted.
fn gather(
    p: &mut dyn Prompter,
    kind: Kind,
    sel: &Pick,
    again: bool,
    consented: &mut bool,
) -> Result<Backend> {
    match sel {
        Pick::Preset(preset, cand) => match &cand.place {
            Place::Local { port } => {
                let chosen = ask_port(p, *port)?;
                let key = ask_key(p, None, false)?;
                Ok(local_backend(preset, chosen, key, ""))
            }
            Place::Remote { key_env } => {
                if !*consented {
                    consent(p, kind, preset.name)?;
                    *consented = true;
                }
                let var = key_env.filter(|_| cand.detected && !again);
                let key = match (var, key_env) {
                    (Some(v), _) => {
                        p.say(&format!("using the key in {v}"));
                        Some(KeyRef::Env(v.to_string()))
                    }
                    (None, Some(v)) => {
                        if !cand.detected {
                            p.say(&format!("{v} is not set in this shell."));
                        }
                        ask_key(p, Some(v), true)?
                    }
                    (None, None) => ask_key(p, None, false)?,
                };
                Ok(remote_backend(preset, key, ""))
            }
        },
        Pick::Custom => {
            let url = ask_url(p)?;
            if !is_loopback_url(&url) && !*consented {
                consent(p, kind, &url)?;
                *consented = true;
            }
            let key = ask_key(p, None, false)?;
            Ok(custom_backend(&url, key, ""))
        }
    }
}

fn ask_port(p: &mut dyn Prompter, default: u16) -> Result<u16> {
    for _ in 0..3 {
        let a = p.ask("port", &default.to_string());
        match a.trim().parse::<u16>() {
            Ok(n) if n > 0 => return Ok(n),
            _ => p.say("enter a port number from 1 to 65535"),
        }
    }
    Err(Error::new("setup.no_answer", "no valid port")
        .with_hint("run `shellmate setup` again, or pass --base-url with --yes"))
}

fn ask_url(p: &mut dyn Prompter) -> Result<String> {
    for _ in 0..3 {
        let a = ask_required(p, "base URL of the server (e.g. http://localhost:8000)")?;
        match shellmate_api::check_endpoint(a.trim()) {
            Ok(()) => return Ok(a.trim().to_string()),
            Err(e) => {
                p.say(&e.message);
                if let Some(h) = e.hint {
                    p.say(&format!("  {h}"));
                }
            }
        }
    }
    Err(Error::new("setup.no_answer", "no valid base URL")
        .with_hint("run `shellmate setup` again, or pass --base-url with --yes"))
}

/// Ask where the key lives. Only a reference is accepted, so a key pasted by mistake is refused and never
/// echoed. With `suggest`, an empty answer means that variable; without it, an empty answer means no key.
fn ask_key(p: &mut dyn Prompter, suggest: Option<&str>, needed: bool) -> Result<Option<KeyRef>> {
    let default = suggest.map(|v| format!("env:{v}")).unwrap_or_default();
    let question = if needed {
        "where is the API key? (env:NAME or file:PATH)"
    } else {
        "where is the API key? (env:NAME or file:PATH, empty for none)"
    };
    for _ in 0..3 {
        let a = p.ask(question, &default);
        if a.trim().is_empty() {
            return Ok(None);
        }
        match KeyRef::parse(&a) {
            Ok(k) => return Ok(Some(k)),
            Err(e) => {
                p.say(&e.message);
                p.say(&format!("  {}", e.hint.unwrap_or_default()));
            }
        }
    }
    Err(
        Error::new("setup.no_answer", "no valid reference to the key")
            .with_hint("run `shellmate setup` again, or pass --api-key with --yes"),
    )
}

fn target(b: &Backend) -> String {
    b.endpoint
        .clone()
        .or_else(|| b.preset.clone())
        .unwrap_or_default()
}

fn consent(p: &mut dyn Prompter, kind: Kind, name: &str) -> Result<()> {
    let sent = match kind {
        Kind::Provider => "By default shellmate sends it: what you type, the shell/OS name and your working directory.\nIt does not send your command history or environment, and secrets-looking values are replaced by placeholders first.",
        Kind::Checker => "For each suggested command it is sent your request, the command and the name of your shell, and nothing else from your context.\nSecrets-looking values are replaced by placeholders first.",
    };
    p.say(&format!(
        "\n{name} is a remote service. {sent}\nYour API key is read from where you said; it is never written to the config."
    ));
    if confirm(p, "continue?", false) {
        Ok(())
    } else {
        Err(cancelled())
    }
}

// ---- --yes: every answer is a flag --------------------------------------------------------------------

fn key_from_flags(reference: Option<&str>, env_name: Option<&str>) -> Result<Option<KeyRef>> {
    match (reference, env_name) {
        (Some(_), Some(_)) => Err(Error::new(
            "setup.bad_key_ref",
            "give --api-key or --api-key-env, not both",
        )),
        (Some(r), None) => KeyRef::parse(r).map(Some),
        (None, Some(n)) => KeyRef::parse(&format!("env:{n}")).map(Some),
        (None, None) => Ok(None),
    }
}

/// A backend from flags. A preset says where the server is; `url` replaces that, and then the preset's key
/// variable no longer applies, since a key is never sent to an address the person did not give it for.
fn backend_from_flags(
    kind: Kind,
    name: &str,
    model: &str,
    url: Option<&str>,
    key: Option<KeyRef>,
) -> Result<Backend> {
    if name == CUSTOM {
        let url = url
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                Error::new("setup.bad_base_url", "a base URL is needed")
                    .with_hint("pass --base-url (or --checker-url), e.g. http://localhost:8000")
            })?;
        shellmate_api::check_endpoint(url)?;
        return Ok(custom_backend(url, key, model));
    }
    let preset = presets_for(kind)
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| {
            Error::new(
                "setup.unknown_provider",
                format!(
                    "unknown {} `{name}`",
                    match kind {
                        Kind::Provider => "provider",
                        Kind::Checker => "checker",
                    }
                ),
            )
            .with_hint(format!("choose one of: {}", names(kind)))
        })?;
    if let Some(url) = url {
        shellmate_api::check_endpoint(url.trim())?;
        return Ok(custom_backend(url, key, model));
    }
    Ok(match place(preset) {
        Place::Local { port } => local_backend(preset, port, key, model),
        Place::Remote { key_env } => {
            let key = key.or_else(|| key_env.map(|v| KeyRef::Env(v.to_string())));
            remote_backend(preset, key, model)
        }
    })
}

pub(super) fn from_args(a: &SetupArgs, svc: &Services) -> Result<Chosen> {
    let provider = a.provider.as_deref().ok_or_else(|| {
        Error::new("setup.no_provider", "no provider chosen").with_hint(format!(
            "pass --provider ({}) and --model",
            names(Kind::Provider)
        ))
    })?;
    let model = a.model.as_deref().ok_or_else(|| {
        Error::new("setup.no_model", "no model chosen")
            .with_hint("pass --model with the name of the model, as the provider lists it")
    })?;
    let key = key_from_flags(a.api_key.as_deref(), a.api_key_env.as_deref())?;
    let main = backend_from_flags(Kind::Provider, provider, model, a.base_url.as_deref(), key)?;
    let checker = match a.checker.as_deref() {
        None => {
            if a.checker_model.is_some() || a.checker_url.is_some() || a.checker_api_key.is_some() {
                return Err(
                    Error::new("setup.no_checker", "checker options without --checker")
                        .with_hint(format!("pass --checker ({})", names(Kind::Checker))),
                );
            }
            None
        }
        Some(c) => {
            let cm = a.checker_model.as_deref().ok_or_else(|| {
                Error::new("setup.no_model", "no checker model chosen")
                    .with_hint("pass --checker-model with the name of the decision model")
            })?;
            let ck = key_from_flags(a.checker_api_key.as_deref(), None)?;
            Some(backend_from_flags(
                Kind::Checker,
                c,
                cm,
                a.checker_url.as_deref(),
                ck,
            )?)
        }
    };
    if a.verify {
        for (kind, b) in [
            (Kind::Provider, Some(&main)),
            (Kind::Checker, checker.as_ref()),
        ] {
            if let Some(b) = b {
                (svc.connect)(kind, b).map_err(|e| {
                    let hint = e
                        .hint
                        .clone()
                        .unwrap_or_else(|| "check the address and the key".into());
                    Error::new(
                        e.code,
                        format!("{} did not answer: {}", target(b), e.message),
                    )
                    .with_hint(hint)
                })?;
            }
        }
    }
    Ok(Chosen { main, checker })
}

// ---- the real machine ---------------------------------------------------------------------------------

/// Ask the server for its models: build the plugin from these settings, as the engine would, and call it.
fn real_connect(kind: Kind, b: &Backend) -> Listing {
    let mut settings = serde_json::Map::new();
    if let Some(p) = &b.preset {
        settings.insert("preset".into(), p.clone().into());
    }
    if let Some(e) = &b.endpoint {
        settings.insert("endpoint".into(), e.clone().into());
    }
    settings.insert("model".into(), b.model.clone().into());
    if let Some(k) = &b.api_key {
        // the same resolution the engine does: a variable, or a file that only you can read
        let value = shellmate_core::secrets::resolve("api_key", k, &Host::system())?;
        settings.insert("api_key".into(), value.into());
    }
    let settings = serde_json::Value::Object(settings);
    let registry = shellmate_builtin::registry();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            Error::new(
                "setup.runtime",
                format!("cannot start the network runtime: {e}"),
            )
        })?;
    match kind {
        Kind::Provider => {
            let provider = registry.build_provider("openai-compatible", &settings)?;
            rt.block_on(provider.models()).map_err(Error::from)
        }
        Kind::Checker => {
            let decider = registry.build_decider("systemone", &settings)?;
            rt.block_on(decider.models()).map_err(Error::from)
        }
    }
}

pub fn setup(explicit: Option<&Path>, a: SetupArgs) -> Result<String> {
    let interactive = !a.yes;
    if interactive && !std::io::stdin().is_terminal() {
        return Err(
            Error::new("setup.needs_tty", "interactive setup needs a terminal").with_hint(
                "pass --yes with --provider and --model (and --shell if it cannot be told)",
            ),
        );
    }
    let mut term = Terminal;
    let shell = a
        .shell
        .clone()
        .or_else(|| shell_name(&SystemEnv))
        .unwrap_or_default();
    let shell = if ["fish", "zsh", "bash"].contains(&shell.as_str()) {
        shell
    } else if interactive {
        term.ask("shell (fish, zsh, bash)", "bash")
    } else {
        return Err(
            Error::new("setup.no_shell", "cannot tell which shell to set up")
                .with_hint("pass --shell fish|zsh|bash"),
        );
    };
    let ui = crate::commands::ui::stdout();
    println!("{} {shell}", ui.label(Tone::Muted, "shell:"));

    let mut out = String::new();
    let mut reminders: Vec<String> = Vec::new();
    let path = config_path(explicit, &SystemEnv);
    if path.exists() {
        // nothing to ask: the config that is there is kept
        out.push_str(&format!(
            "{} {}\n",
            ui.label(Tone::Muted, "keeping existing"),
            path.display()
        ));
    } else {
        let svc = Services {
            env: &SystemEnv,
            probe: &probe_port,
            connect: &real_connect,
        };
        let chosen = if interactive {
            guided(&mut term, &svc)
        } else {
            from_args(&a, &svc)
        };
        let chosen = match chosen {
            Err(e) if e.code == "setup.cancelled" => {
                return Ok(ui.status(Tone::Warning, "setup cancelled"))
            }
            other => other?,
        };
        let text = template_with(&chosen.main, chosen.checker.as_ref(), Some(&shell))?;
        out.push_str(&write_config(explicit, text, false)?);
        // a key that is named but not set yet is the one thing left for the person to do
        for b in std::iter::once(&chosen.main).chain(chosen.checker.as_ref()) {
            if let Some(KeyRef::Env(name)) =
                b.api_key.as_deref().and_then(|k| KeyRef::parse(k).ok())
            {
                if SystemEnv.nonempty(&name).is_none() && !reminders.contains(&name) {
                    reminders.push(name);
                }
            }
        }
    }
    print!("{out}");
    out.clear();
    out.push_str(&install_shell(&shell, a.yes, false, false)?);
    let (report, _) = doctor(explicit, false);
    out.push('\n');
    out.push_str(&report);
    for var in reminders {
        out.push('\n');
        out.push_str(&ui.status(
            Tone::Warning,
            &format!(
                "set your key before using it, e.g. `export {var}=...` in your shell's environment"
            ),
        ));
    }
    out.push('\n');
    out.push_str(&ui.status(
        Tone::Info,
        "try it: type `find the large files here` and press alt+m",
    ));
    Ok(out)
}
