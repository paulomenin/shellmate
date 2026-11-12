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
        let cfg: Config =
            toml::from_str(text).map_err(|e| Error::new("config.invalid", e.to_string()))?;
        cfg.check_static()?;
        Ok(cfg)
    }
}

pub use shellmate_api::parse_duration;
