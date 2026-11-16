//! Finding, reading and parsing the config file.

use super::Config;
use crate::ports::{Env, Host, SystemEnv};
use shellmate_api::{Error, Result};
use std::path::{Path, PathBuf};

impl Config {
    /// `$XDG_CONFIG_HOME/shellmate/config.toml`, falling back to `~/.config/...`.
    pub fn default_path() -> Option<PathBuf> {
        Self::default_path_in(&SystemEnv)
    }

    pub fn default_path_in(env: &dyn Env) -> Option<PathBuf> {
        let base = env
            .var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| env.var("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("shellmate").join("config.toml"))
    }

    /// Load from `path` (or the default path) on the real machine.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        Self::load_from(path, &Host::system())
    }

    /// Load from `path` (or the default path). A missing default file yields the built-in defaults.
    pub fn load_from(path: Option<&Path>, host: &Host) -> Result<Self> {
        let (path, explicit) = match path {
            Some(p) => (p.to_path_buf(), true),
            None => match Self::default_path_in(host.env.as_ref()) {
                Some(p) => (p, false),
                None => return Ok(Self::defaults()),
            },
        };
        match host.fs.read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|mut e| {
                e.message = format!("{}: {}", path.display(), e.message);
                e
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => Ok(Self::defaults()),
            Err(e) => Err(Error::new(
                "config.unreadable",
                format!("{}: {e}", path.display()),
            )),
        }
    }

    pub fn defaults() -> Self {
        Self::parse("version = 1").expect("built-in defaults are valid")
    }

    pub fn parse(text: &str) -> Result<Self> {
        if let Ok(raw) = text.parse::<toml::Table>() {
            if let Some(moved) = moved_key(&raw) {
                return Err(moved);
            }
        }
        let cfg: Config =
            toml::from_str(text).map_err(|e| Error::new("config.invalid", e.to_string()))?;
        cfg.check_static()?;
        Ok(cfg)
    }
}

pub use shellmate_api::parse_duration;

/// A key of the older config shape, with where it lives now. Without this the loader would only say "unknown
/// field" or hand the key to a plugin that has never heard of it.
fn moved_key(raw: &toml::Table) -> Option<Error> {
    let moved = |what: &str, now: &str| {
        Some(
            Error::new("config.moved", format!("{what} has moved"))
                .with_hint(format!("{now} (see the Configuration page)")),
        )
    };
    let table = |v: Option<&toml::Value>| v.and_then(toml::Value::as_table).cloned();
    if raw.contains_key("deciders") {
        return moved(
            "`[deciders.<name>]`",
            "a decision model is a `[models.<name>]` with `plugin = \"builtin:systemone\"`, reached through a `[providers.<name>]`",
        );
    }
    for (name, p) in table(raw.get("providers")).unwrap_or_default() {
        let p = p.as_table()?;
        if p.contains_key("plugin") {
            return moved(
                &format!("`plugin` of `[providers.{name}]`"),
                "set `plugin`, `model` and `provider` in a `[models.<name>]`; a provider is only the connection",
            );
        }
        if p.contains_key("settings") {
            return moved(
                &format!("`[providers.{name}.settings]`"),
                "write `preset`, `endpoint`, `api_key` and `timeout_secs` directly in `[providers.<name>]`",
            );
        }
    }
    for key in ["semantic", "deciders", "llm_fallback"] {
        if table(raw.get("validator")).is_some_and(|v| v.contains_key(key)) {
            return moved(
                &format!("`[validator] {key}`"),
                "`[validator] chain = [\"model\", ...]` lists decision and chat models in order; an empty chain means no semantic check",
            );
        }
    }
    for (id, p) in table(raw.get("plugins")).unwrap_or_default() {
        if p.as_table()?.contains_key("settings") {
            return moved(
                &format!("`[plugins.\"{id}\".settings]`"),
                "write the settings directly in `[plugins.\"<id>\"]`, next to `enabled`",
            );
        }
    }
    None
}
