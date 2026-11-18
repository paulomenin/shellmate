//! Turning the config into plugin settings: secret references resolved, plugin ids checked.

use super::Config;
use crate::ports::Host;
use crate::secret_cmd::CmdOptions;
use crate::secrets;
use shellmate_api::{Error, ModelKind, Registry, Result};

/// What looking at a model without using it shows (see [`Config::inspect_model`]).
#[derive(Debug, Clone)]
pub struct ModelInspection {
    pub name: String,
    pub kind: ModelKind,
    pub info: shellmate_api::ProviderInfo,
    /// A `cmd:` secret is fetched when the model is first used.
    pub lazy: bool,
}

impl Config {
    /// What a model's plugin is given as JSON with secret references resolved: the settings of the provider it is
    /// reached through and its own, in one table. A key is never in both (`check_static`).
    pub fn resolved_model_settings(&self, model: &str, host: &Host) -> Result<serde_json::Value> {
        let m = self.model(model)?;
        let mut out = resolve_table(
            &format!("providers.{}", m.model.provider),
            &m.provider.settings,
            host,
            &self.cmd_options(),
        )?;
        let own = resolve_table(
            &format!("models.{model}"),
            &m.model.settings,
            host,
            &self.cmd_options(),
        )?;
        merge(&mut out, own);
        Ok(out)
    }

    /// The tool's own settings (everything under `[tools.<name>]` but the keys the core keeps) as JSON with
    /// secret references resolved.
    pub fn resolved_tool_settings(&self, tool: &str, host: &Host) -> Result<serde_json::Value> {
        let t = self
            .tools
            .entries
            .get(tool)
            .ok_or_else(|| Error::new("config.invalid", format!("unknown tool `{tool}`")))?;
        resolve_table(
            &format!("tools.{tool}"),
            &t.settings,
            host,
            &self.cmd_options(),
        )
    }

    pub fn plugin_settings(&self, id: &str, host: &Host) -> Result<serde_json::Value> {
        match self.plugins.get(id) {
            Some(p) => resolve_table(
                &format!("plugins.\"{id}\""),
                &p.settings,
                host,
                &self.cmd_options(),
            ),
            None => Ok(serde_json::json!({})),
        }
    }

    /// One model built from its probe settings: nothing is sent and no `cmd:` program is run, so it says what the
    /// model is and where it would send requests, and whether a secret is fetched on first use. This is the one
    /// way to look at a model without using it; `validate_with` and `doctor` both go through it.
    pub fn inspect_model(
        &self,
        name: &str,
        registry: &Registry,
        host: &Host,
    ) -> Result<ModelInspection> {
        let m = self.model(name)?;
        let id = builtin_id(&m.model.plugin)?;
        let named = |mut e: Error| {
            e.message = format!("models.{name}: {}", e.message);
            e
        };
        let kind = registry.model_kind(id).map_err(named)?;
        let settings = self.lazy_model_settings(name, host)?;
        let info = match kind {
            ModelKind::Chat => registry
                .build_provider(id, settings.probe())
                .map(|p| p.info()),
            ModelKind::Decision => registry
                .build_decider(id, settings.probe())
                .map(|d| d.info()),
        }
        .map_err(named)?;
        Ok(ModelInspection {
            name: name.to_string(),
            kind,
            info,
            lazy: settings.is_lazy(),
        })
    }

    /// Every model of the config, inspected. The first one that cannot be built is the error.
    pub fn inspect_models(&self, registry: &Registry, host: &Host) -> Result<Vec<ModelInspection>> {
        self.models
            .keys()
            .map(|name| self.inspect_model(name, registry, host))
            .collect()
    }

    /// Full check used by `config check`: instantiate every model (no network), and see that the `llm` chain
    /// holds only chat models.
    pub fn validate_with(&self, registry: &Registry, host: &Host) -> Result<()> {
        self.inspect_models(registry, host)?;
        for name in &self.llm.chain {
            let id = builtin_id(&self.model(name)?.model.plugin)?;
            if registry.model_kind(id)? != ModelKind::Chat {
                let chat: Vec<&str> = self
                    .models
                    .iter()
                    .filter(|(_, m)| {
                        builtin_id(&m.plugin)
                            .ok()
                            .and_then(|id| registry.model_kind(id).ok())
                            == Some(ModelKind::Chat)
                    })
                    .map(|(n, _)| n.as_str())
                    .collect();
                return Err(Error::new(
                    "config.wrong_kind",
                    format!(
                        "llm.chain: `{name}` is a decision model, and only a chat model can answer"
                    ),
                )
                .with_hint(if chat.is_empty() {
                    "add a chat model, for example plugin = \"builtin:openai-compatible\""
                        .to_string()
                } else {
                    format!(
                        "chat models: {} (decision models belong in [validator] chain)",
                        chat.join(", ")
                    )
                }));
            }
        }
        Ok(())
    }
}

/// Add the keys of `from` to `into`. Both are JSON objects built from tables that share no key.
fn merge(into: &mut serde_json::Value, from: serde_json::Value) {
    if let (Some(into), serde_json::Value::Object(from)) = (into.as_object_mut(), from) {
        into.extend(from);
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
    /// The config path, for messages: `providers.work.api_key`.
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
            scan(format!("providers.{n}"), &p.settings);
        }
        for (n, m) in &self.models {
            scan(format!("models.{n}"), &m.settings);
        }
        for (n, p) in &self.plugins {
            scan(format!("plugins.\"{n}\""), &p.settings);
        }
        out
    }

    /// Like [`Config::resolved_model_settings`], but a `cmd:` secret is only checked (the line splits and the
    /// program exists), not run.
    pub fn lazy_model_settings(&self, model: &str, host: &Host) -> Result<LazySettings> {
        let m = self.model(model)?;
        let mut out = lazy_table(
            &format!("providers.{}", m.model.provider),
            &m.provider.settings,
            host,
            self.cmd_options(),
        )?;
        let own = lazy_table(
            &format!("models.{model}"),
            &m.model.settings,
            host,
            self.cmd_options(),
        )?;
        merge(&mut out.probe, own.probe);
        out.pending.extend(own.pending);
        Ok(out)
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
