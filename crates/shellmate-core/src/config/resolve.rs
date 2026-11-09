//! Turning the config into plugin settings: secret references resolved, plugin ids checked.

use super::Config;
use crate::ports::Host;
use crate::secret_cmd::CmdOptions;
use crate::secrets;
use shellmate_api::{Error, Registry, Result};

impl Config {
    /// Settings of a provider as JSON with secret references resolved.
    pub fn resolved_settings(&self, provider: &str, host: &Host) -> Result<serde_json::Value> {
        let p = self.providers.get(provider).ok_or_else(|| {
            Error::new("config.invalid", format!("unknown provider `{provider}`"))
        })?;
        resolve_table(
            &format!("providers.{provider}.settings"),
            &p.settings,
            host,
            &self.cmd_options(),
        )
    }

    /// Settings of a decider as JSON with secret references resolved.
    pub fn resolved_decider_settings(
        &self,
        decider: &str,
        host: &Host,
    ) -> Result<serde_json::Value> {
        let d = self
            .deciders
            .get(decider)
            .ok_or_else(|| Error::new("config.invalid", format!("unknown decider `{decider}`")))?;
        resolve_table(
            &format!("deciders.{decider}.settings"),
            &d.settings,
            host,
            &self.cmd_options(),
        )
    }

    pub fn plugin_settings(&self, id: &str, host: &Host) -> Result<serde_json::Value> {
        match self.plugins.get(id) {
            Some(p) => resolve_table(
                &format!("plugins.\"{id}\".settings"),
                &p.settings,
                host,
                &self.cmd_options(),
            ),
            None => Ok(serde_json::json!({})),
        }
    }

    /// Full check used by `config check`: instantiate every provider (no network).
    pub fn validate_with(&self, registry: &Registry, host: &Host) -> Result<()> {
        for name in self.providers.keys() {
            let id = builtin_id(&self.providers[name].plugin)?;
            let settings = self.lazy_settings(name, host)?;
            registry
                .build_provider(id, settings.probe())
                .map_err(|mut e| {
                    e.message = format!("providers.{name}: {}", e.message);
                    e
                })?;
        }
        for name in self.deciders.keys() {
            let id = builtin_id(&self.deciders[name].plugin)?;
            let settings = self.lazy_decider_settings(name, host)?;
            registry
                .build_decider(id, settings.probe())
                .map_err(|mut e| {
                    e.message = format!("deciders.{name}: {}", e.message);
                    e
                })?;
        }
        Ok(())
    }
}

fn resolve_table(
    path: &str,
    table: &toml::Table,
    host: &Host,
    options: &CmdOptions,
) -> Result<serde_json::Value> {
    resolve_table_with(path, table, host, options, None)
}

/// With `defer`, a `cmd:` secret is checked and noted there instead of run.
fn resolve_table_with(
    path: &str,
    table: &toml::Table,
    host: &Host,
    options: &CmdOptions,
    mut defer: Option<&mut Vec<Pending>>,
) -> Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in table {
        let json =
            serde_json::to_value(v).map_err(|e| Error::new("config.invalid", e.to_string()))?;
        let json = match (&json, secrets::is_secret_key(k)) {
            (serde_json::Value::String(s), true) => {
                let key = format!("{path}.{k}");
                match (secrets::lookup_line(&key, s)?, defer.as_deref_mut()) {
                    (Some(line), Some(pending)) => {
                        crate::secret_cmd::check(&key, &line, host)?;
                        pending.push(Pending {
                            name: k.clone(),
                            path: key,
                            line: line.into_owned(),
                        });
                        serde_json::Value::String(PENDING.into())
                    }
                    _ => serde_json::Value::String(secrets::resolve_with(&key, s, host, options)?),
                }
            }
            _ => json,
        };
        map.insert(k.clone(), json);
    }
    Ok(serde_json::Value::Object(map))
}

/// What a `cmd:` secret stands for until the program has run. A plugin built with it only reports what it
/// is (model, endpoint); the engine never sends a request with it (see `engine::lazy`).
const PENDING: &str = "<fetched on first use>";

/// A `cmd:` secret waiting to be fetched.
#[derive(Debug, Clone)]
struct Pending {
    /// The setting name, for the plugin's table.
    name: String,
    /// The config path, for messages: `providers.work.settings.api_key`.
    path: String,
    line: String,
}

/// Settings whose `cmd:` references have not been run yet. `probe` has everything else resolved and a
/// placeholder in the `cmd:` settings, enough to build a plugin that can describe itself. [`complete`] runs
/// the programs and gives the real settings.
///
/// [`complete`]: LazySettings::complete
#[derive(Debug, Clone)]
pub struct LazySettings {
    probe: serde_json::Value,
    pending: Vec<Pending>,
    options: CmdOptions,
}

impl LazySettings {
    pub fn probe(&self) -> &serde_json::Value {
        &self.probe
    }

    /// Some secret still has to be fetched.
    pub fn is_lazy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Run the `cmd:` programs and return the settings with their output in place.
    pub fn complete(&self, host: &Host) -> Result<serde_json::Value> {
        let mut settings = self.probe.clone();
        for p in &self.pending {
            let value =
                secrets::resolve_with(&p.path, &format!("cmd:{}", p.line), host, &self.options)?;
            settings[&p.name] = serde_json::Value::String(value);
        }
        Ok(settings)
    }
}

impl Config {
    /// Every `cmd:` secret in the config as (config path, line), for `config check` and `doctor`.
    pub fn cmd_secrets(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut scan = |path: String, table: &toml::Table| {
            for (k, v) in table {
                let line = v
                    .as_str()
                    .and_then(|s| secrets::lookup_line(k, s).ok().flatten());
                if let (true, Some(line)) = (secrets::is_secret_key(k), line) {
                    out.push((format!("{path}.{k}"), line.into_owned()));
                }
            }
        };
        for (n, p) in &self.providers {
            scan(format!("providers.{n}.settings"), &p.settings);
        }
        for (n, d) in &self.deciders {
            scan(format!("deciders.{n}.settings"), &d.settings);
        }
        for (n, p) in &self.plugins {
            scan(format!("plugins.\"{n}\".settings"), &p.settings);
        }
        out
    }

    /// Like [`Config::resolved_settings`], but a `cmd:` secret is only checked (the line splits and the
    /// program exists), not run.
    pub fn lazy_settings(&self, provider: &str, host: &Host) -> Result<LazySettings> {
        let p = self.providers.get(provider).ok_or_else(|| {
            Error::new("config.invalid", format!("unknown provider `{provider}`"))
        })?;
        lazy_table(
            &format!("providers.{provider}.settings"),
            &p.settings,
            host,
            self.cmd_options(),
        )
    }

    /// Like [`Config::resolved_decider_settings`], without running `cmd:` secrets.
    pub fn lazy_decider_settings(&self, decider: &str, host: &Host) -> Result<LazySettings> {
        let d = self
            .deciders
            .get(decider)
            .ok_or_else(|| Error::new("config.invalid", format!("unknown decider `{decider}`")))?;
        lazy_table(
            &format!("deciders.{decider}.settings"),
            &d.settings,
            host,
            self.cmd_options(),
        )
    }
}

fn lazy_table(
    path: &str,
    table: &toml::Table,
    host: &Host,
    options: CmdOptions,
) -> Result<LazySettings> {
    let mut pending = Vec::new();
    let probe = resolve_table_with(path, table, host, &options, Some(&mut pending))?;
    Ok(LazySettings {
        probe,
        pending,
        options,
    })
}

/// `builtin:foo` → `foo`.
pub fn builtin_id(plugin: &str) -> Result<&str> {
    match plugin.strip_prefix("builtin:") {
        Some(id) if !id.is_empty() => Ok(id),
        _ => Err(Error::new(
            "config.unsupported",
            format!("plugin `{plugin}` is not supported yet"),
        )
        .with_hint("only `builtin:<id>` plugins exist in this version")),
    }
}
