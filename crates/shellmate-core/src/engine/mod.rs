//! The engine: builds the request per provider, runs the `llm` middleware chain, falls back, and parses
//! the answer into an [`Outcome`].
//!
//! `Engine` is defined here with its construction. What it does is split by concern into child modules:
//! `context` (what the model is told about the shell), `request` (building a provider's payload),
//! `preview` (`--dry-run` and `shellmate context`), `run` (the request lifecycle), `attempt` (one
//! provider's turn), `semantic` (checking a suggested command) and `tool_calls` (`man`, `exec`, `read`).

mod attempt;
mod context;
mod lazy;
mod preview;
mod request;
mod run;
mod semantic;
mod terminals;
#[cfg(test)]
mod tests;
mod tool_calls;
mod turn;
mod types;

use crate::audit::{Audit, AuditSink};
use crate::config::{Config, Trust};
use crate::conversation;
use crate::ports::{Host, RealFs};
use crate::prompt;
use crate::tools::Approver;
use shellmate_api::{
    ChatRequest, ChatStream, Error, Middleware, MiddlewareRole, Registry, Result, Store,
};
use std::sync::{Arc, Mutex};

pub use types::{
    Action, ContextItem, ContextReport, DeciderSpec, Emit, ItemState, Outcome, Preview,
    ProviderSpec, Report, Stages,
};

/// Every tool of the registry, in the order it was registered, and any other `[tools.<name>]` (which the
/// registry refuses). Each is built from its own settings; the core keeps `enabled`, `approval` and
/// `preapproved`. A tool with no table is on, with its own approval default and no rules.
fn build_tools(cfg: &Config, registry: &Registry, host: &Host) -> Result<Vec<tool_calls::Plugged>> {
    use crate::config::ToolApproval;
    use shellmate_api::{ApprovalMode, ApprovalSetting};
    let mut plugged = Vec::new();
    let mut names: Vec<&str> = registry.tool_ids().collect();
    for n in cfg.tools.other.keys().map(String::as_str) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    for name in names {
        let entry = cfg.tools.other.get(name).cloned().unwrap_or_default();
        let named = |mut e: Error| {
            e.message = format!("tools.{name}: {}", e.message);
            e
        };
        let settings = match cfg.tools.other.contains_key(name) {
            true => cfg.resolved_tool_settings(name, host)?,
            false => serde_json::json!({}),
        };
        let tool = registry.build_tool(name, &settings).map_err(named)?;
        if let Some(r) = entry
            .preapproved
            .iter()
            .find(|r| r.trim().is_empty() || r.contains(|c: char| c.is_control()))
        {
            return Err(Error::new(
                "config.invalid",
                format!("tools.{name}.preapproved: invalid rule {r:?}"),
            ));
        }
        let needs_approval = tool.info().approval == ApprovalMode::Required;
        let approval = match entry.approval {
            Some(ToolApproval::Allow) if needs_approval => {
                return Err(Error::new(
                    "config.invalid",
                    format!(
                        "tools.{name}.approval = \"allow\": this tool needs approval for each call"
                    ),
                )
                .with_hint("use \"ask\", or list what may run without asking in `preapproved`"))
            }
            Some(ToolApproval::Ask) if !needs_approval => {
                return Err(Error::new(
                    "config.invalid",
                    format!("tools.{name}.approval = \"ask\": this tool never asks"),
                )
                .with_hint("use \"allow\" to keep it on, or \"deny\" to turn it off"))
            }
            Some(ToolApproval::Allow) => ApprovalSetting::Allow,
            Some(ToolApproval::Ask) => ApprovalSetting::Ask,
            Some(ToolApproval::Deny) => ApprovalSetting::Deny,
            None if needs_approval => ApprovalSetting::Ask,
            None => ApprovalSetting::Allow,
        };
        plugged.push(tool_calls::Plugged {
            tool,
            settings: shellmate_api::ToolSettings {
                enabled: entry.enabled,
                approval,
                preapproved: entry.preapproved,
            },
        });
    }
    Ok(plugged)
}

pub struct Engine {
    providers: Vec<ProviderSpec>,
    /// Decision models for the semantic check, in order (empty: the LLM answers, when allowed).
    deciders: Vec<DeciderSpec>,
    layers: Vec<Arc<dyn Middleware<ChatRequest, ChatStream>>>,
    /// True when an active layer has the `Redactor` role.
    has_redactor: bool,
    os: String,
    /// The environment, clock and file system the engine looks at.
    host: Host,
    cfg: Config,
    store: Option<Arc<dyn Store>>,
    /// Why the configured store could not be used (conversations are then off, with a visible note).
    store_note: Option<String>,
    /// Tools added through the plugin API, with the settings the core keeps for each.
    plugged: Vec<tool_calls::Plugged>,
    audit: Option<Arc<dyn AuditSink>>,
    /// Asks the human before a model-driven command runs. `None` (no terminal): every such request is denied.
    approver: Option<Arc<dyn Approver>>,
    /// Project and git sections for the current request, gathered once (see `prefetch`).
    gathered: Mutex<Gathered>,
}

#[derive(Default)]
struct Gathered {
    project: Option<String>,
    git: Option<crate::collect::GitInfo>,
}

impl Engine {
    /// A fresh id for one request, from the clock and the process id.
    fn request_id(&self) -> String {
        format!(
            "{:x}-{:x}",
            self.host.clock.unix_nanos(),
            std::process::id()
        )
    }

    /// The context level a request starts from, before a provider's ceiling caps it.
    fn default_level(&self) -> u8 {
        self.cfg.context.default_level
    }

    /// The user's home directory, if the environment says where it is.
    fn home(&self) -> Option<String> {
        self.host.env.var("HOME")
    }
}

impl Engine {
    /// Build providers and the `llm` middleware chain from the config (secrets resolved, nothing sent).
    pub fn from_config(cfg: &Config, registry: &Registry) -> Result<Self> {
        Self::from_config_in(cfg, registry, Host::system())
    }

    /// Like [`Engine::from_config`], looking at `host` instead of the real machine.
    pub fn from_config_in(cfg: &Config, registry: &Registry, host: Host) -> Result<Self> {
        let mut providers = Vec::new();
        for name in cfg.provider_chain()? {
            let pc = &cfg.providers[name];
            let id = crate::config::builtin_id(&pc.plugin)?;
            let lazy = cfg.lazy_settings(name, &host)?;
            let build = registry.provider_builder(id)?;
            let named = {
                let name = name.to_string();
                move |mut e: shellmate_api::Error| {
                    e.message = format!("providers.{name}: {}", e.message);
                    e
                }
            };
            let provider = build(lazy.probe()).map_err(&named)?;
            // A `cmd:` secret is fetched when the provider is first used, not now.
            let provider: Arc<dyn shellmate_api::Provider> = if lazy.is_lazy() {
                let host = host.clone();
                Arc::new(lazy::LazyProvider::new(
                    provider,
                    Arc::new(move || build(&lazy.complete(&host)?).map_err(&named)),
                ))
            } else {
                provider
            };
            providers.push(ProviderSpec {
                name: name.to_string(),
                provider,
                max_level: pc.effective_max_level(),
                trust: pc.trust,
            });
        }
        let mut deciders = Vec::new();
        if cfg.validator.semantic {
            for name in &cfg.validator.deciders {
                let dc = &cfg.deciders[name];
                let id = crate::config::builtin_id(&dc.plugin)?;
                let decider = registry
                    .build_decider(id, &cfg.resolved_decider_settings(name, &host)?)
                    .map_err(|mut e| {
                        e.message = format!("deciders.{name}: {}", e.message);
                        e
                    })?;
                deciders.push(DeciderSpec {
                    name: name.clone(),
                    decider,
                    trust: dc.trust,
                });
            }
        }
        let mut layers = Vec::new();
        let mut has_redactor = false;
        for layer in cfg.llm_layers() {
            let id = crate::config::builtin_id(&layer)?;
            has_redactor |= registry.llm_middleware_role(id) == Some(MiddlewareRole::Redactor);
            layers.push(registry.build_llm_middleware(id, &cfg.plugin_settings(&layer, &host)?)?);
        }
        let plugged = build_tools(cfg, registry, &host)?;
        let mut engine = Self::with_providers(providers, cfg.context.default_level)
            .with_host(host.clone())
            .with_layers(layers, has_redactor);
        engine.deciders = deciders;
        engine.plugged = plugged;
        engine.cfg = cfg.clone();
        engine.audit =
            Audit::from_config(&cfg.audit, &host).map(|a| Arc::new(a) as Arc<dyn AuditSink>);
        match conversation::open_store(cfg, registry, &host) {
            Ok(store) => engine.store = store,
            // An unusable store turns conversations off (never silently another store) and says so.
            Err(e) if e.code == "conversation.store" => {
                engine.store_note = Some(format!("conversations are off: {}", e.message));
            }
            Err(e) => return Err(e),
        }
        engine.check_privacy()?;
        Ok(engine)
    }

    /// For tests and embedding. Providers in fallback order.
    pub fn with_providers(providers: Vec<ProviderSpec>, default_level: u8) -> Self {
        Self {
            providers,
            deciders: Vec::new(),
            gathered: Mutex::new(Gathered::default()),
            layers: Vec::new(),
            has_redactor: false,
            os: prompt::detect_os(&RealFs),
            host: Host::system(),
            cfg: {
                let mut cfg = Config::defaults();
                cfg.context.default_level = default_level;
                cfg
            },
            store: None,
            store_note: None,
            audit: None,
            approver: None,
            plugged: Vec::new(),
        }
    }

    /// Look at `host` (environment, clock, file system) instead of the real machine. Also re-detects the OS.
    pub fn with_host(mut self, host: Host) -> Self {
        self.os = prompt::detect_os(host.fs.as_ref());
        self.host = host;
        self
    }

    /// Decision models for the semantic check (tests and embedding; `from_config` does this from the config).
    pub fn with_deciders(mut self, deciders: Vec<DeciderSpec>) -> Self {
        self.deciders = deciders;
        self
    }

    /// Add a tool that implements the plugin API (tests and embedding). The core keeps its settings and
    /// enforces approval, pre-approval, taint and escaping; the tool describes and runs.
    pub fn with_tool(
        mut self,
        tool: Arc<dyn shellmate_api::Tool>,
        settings: shellmate_api::ToolSettings,
    ) -> Self {
        self.plugged.push(tool_calls::Plugged { tool, settings });
        self
    }

    /// Who is asked before a model-driven command runs (the client's terminal prompt).
    pub fn with_approver(mut self, approver: Arc<dyn Approver>) -> Self {
        self.approver = Some(approver);
        self
    }

    /// Write audit events to `audit` (tests and embedding; `from_config` does this from the config).
    pub fn with_audit(mut self, audit: impl AuditSink + 'static) -> Self {
        self.audit = Some(Arc::new(audit));
        self
    }

    pub(super) fn audit(&self, event: &str, ctx: &serde_json::Value, fields: serde_json::Value) {
        if let Some(a) = &self.audit {
            a.record(event, ctx, fields);
        }
    }

    /// Conversations in `store` (tests and embedding; `from_config` does this from the config).
    pub fn with_store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Replace the settings used for conversations and tools (tests and embedding). The tools that
    /// implement the plugin API are built from the config by [`Engine::with_tools_from`].
    pub fn with_config(mut self, cfg: Config) -> Self {
        self.cfg = cfg;
        self
    }

    /// The user's `preapproved` rules that are too broad to be safe, as `(tool, rule, why)`. They still
    /// work, because the classifier keeps its veto; `config check` reports them.
    pub fn risky_tool_rules(&self) -> Vec<(String, String, String)> {
        self.plugged
            .iter()
            .flat_map(|p| {
                p.risky_rules(&self.host)
                    .into_iter()
                    .map(|(rule, why)| (p.name().to_string(), rule, why))
            })
            .collect()
    }

    /// Build the tools of the registry from the engine's config and host (tests and embedding;
    /// `from_config` does this itself). They replace the tools added so far.
    pub fn with_tools_from(mut self, registry: &Registry) -> Result<Self> {
        self.plugged = build_tools(&self.cfg, registry, &self.host)?;
        Ok(self)
    }

    pub fn with_os(mut self, os: &str) -> Self {
        self.os = os.to_string();
        self
    }

    /// Install `llm` stage middleware, outermost first.
    pub fn with_layers(
        mut self,
        layers: Vec<Arc<dyn Middleware<ChatRequest, ChatStream>>>,
        has_redactor: bool,
    ) -> Self {
        self.layers = layers;
        self.has_redactor = has_redactor;
        self
    }

    /// INV-2: nothing may reach a remote provider unless a redaction layer is active. Checked when the
    /// engine is built and again before every run, so no construction path can skip it.
    pub fn check_privacy(&self) -> Result<()> {
        if let Some(d) = self.deciders.iter().find(|d| d.trust == Trust::Remote) {
            if !self.has_redactor {
                return Err(Error::new("privacy.no_redaction", format!("decider `{}` is remote but no redaction layer is active", d.name))
                    .with_hint("enable `builtin:redact` in [pipeline].llm, or set trust = \"local\" if the decider runs on this machine"));
            }
        }
        if let Some(p) = self.providers.iter().find(|p| p.trust == Trust::Remote) {
            if !self.has_redactor {
                return Err(Error::new("privacy.no_redaction", format!("provider `{}` is remote but no redaction layer is active", p.name))
                    .with_hint("enable `builtin:redact` in [pipeline].llm, or set trust = \"local\" if the provider runs on this machine"));
            }
        }
        Ok(())
    }
}
