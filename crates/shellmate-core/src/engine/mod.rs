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
mod native;
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
    ChatRequest, ChatStream, Error, Middleware, MiddlewareRole, ModelKind, Registry, Result, Store,
};
use std::sync::Arc;

pub use types::{
    Action, Check, ContextItem, ContextReport, DeciderSpec, Emit, ItemState, Outcome, Preview,
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
    for n in cfg.tools.entries.keys().map(String::as_str) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    for name in names {
        let entry = cfg.tools.entries.get(name).cloned().unwrap_or_default();
        let named = |mut e: Error| {
            e.message = format!("tools.{name}: {}", e.message);
            e
        };
        let settings = match cfg.tools.entries.contains_key(name) {
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
    /// The semantic check: decision and chat models, tried in order (empty: there is no semantic check).
    checks: Vec<Check>,
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
}

/// The project and git sections of one request, collected once by `prefetch` and passed along with it. They
/// belong to the request, not to the engine, so overlapping requests cannot replace each other's facts.
#[derive(Default)]
struct Gathered {
    project: Option<crate::collect::ProjectInfo>,
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
        for name in cfg.model_chain()? {
            providers.push(Self::chat_model(cfg, registry, &host, name)?);
        }
        let mut checks = Vec::new();
        for name in &cfg.validator.chain {
            let id = crate::config::builtin_id(&cfg.model(name)?.model.plugin)?;
            checks.push(match registry.model_kind(id)? {
                ModelKind::Chat => Check::Chat(Self::chat_model(cfg, registry, &host, name)?),
                ModelKind::Decision => {
                    Check::Decision(Self::decision_model(cfg, registry, &host, name)?)
                }
            });
        }
        let (layers, has_redactor) = build_layers(cfg, registry, &host)?;
        let plugged = build_tools(cfg, registry, &host)?;
        let mut engine = Self::with_providers(providers, cfg.context.default_level)
            .with_host(host.clone())
            .with_layers(layers, has_redactor);
        engine.checks = checks;
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

    /// What `config check` and `doctor` learn from a config without starting an engine: every model is built
    /// from its probe settings (no `cmd:` program is run, nothing is sent), the middleware and tools are built
    /// and their settings checked, the store is opened as the engine would, and the privacy rule (a remote model
    /// needs an active redactor) is applied to the models of both chains. No audit log is opened and nothing is
    /// kept. [`Engine::from_config`] enforces the same privacy rule when it builds the real thing.
    pub fn inspect(cfg: &Config, registry: &Registry, host: &Host) -> Result<Inspection> {
        cfg.validate_with(registry, host)?;
        let (_layers, has_redactor) = build_layers(cfg, registry, host)?;
        let plugged = build_tools(cfg, registry, host)?;
        match conversation::open_store(cfg, registry, host) {
            Ok(_) => {}
            // an unusable store only turns conversations off; `doctor` says why
            Err(e) if e.code == "conversation.store" => {}
            Err(e) => return Err(e),
        }
        let chain: Vec<&str> = cfg.model_chain()?;
        let mut models = Vec::new();
        for name in cfg
            .validator
            .chain
            .iter()
            .map(String::as_str)
            .chain(chain.iter().copied())
        {
            models.push((name, cfg.model(name)?.provider.trust));
        }
        privacy_check(models, has_redactor)?;
        Ok(Inspection {
            chain: chain.into_iter().map(String::from).collect(),
            risky_tool_rules: risky_rules(&plugged, host),
        })
    }

    /// A chat model of the config as the engine uses it: built with a placeholder for a `cmd:` secret, and again
    /// with its output when it is first used. Its trust and context ceiling are its provider's.
    fn chat_model(
        cfg: &Config,
        registry: &Registry,
        host: &Host,
        name: &str,
    ) -> Result<ProviderSpec> {
        let m = cfg.model(name)?;
        let id = crate::config::builtin_id(&m.model.plugin)?;
        if registry.model_kind(id)? != ModelKind::Chat {
            return Err(Error::new(
                "config.wrong_kind",
                format!("`{name}` is a decision model, and only a chat model can answer"),
            )
            .with_hint("decision models belong in [validator] chain"));
        }
        let lazy = cfg.lazy_model_settings(name, host)?;
        let build = registry.provider_builder(id)?;
        let named = {
            let name = name.to_string();
            move |mut e: shellmate_api::Error| {
                e.message = format!("models.{name}: {}", e.message);
                e
            }
        };
        let provider = build(lazy.probe()).map_err(&named)?;
        // A `cmd:` secret is fetched when the model is first used, not now.
        let provider: Arc<dyn shellmate_api::Provider> = if lazy.is_lazy() {
            let host = host.clone();
            Arc::new(lazy::LazyProvider::new(
                provider,
                Arc::new(move || build(&lazy.complete(&host)?).map_err(&named)),
            ))
        } else {
            provider
        };
        Ok(ProviderSpec {
            name: name.to_string(),
            provider,
            max_level: m.provider.effective_max_level(),
            trust: m.provider.trust,
            tool_calling: m.model.tool_calling,
        })
    }

    /// A decision model of the config as the engine uses it.
    fn decision_model(
        cfg: &Config,
        registry: &Registry,
        host: &Host,
        name: &str,
    ) -> Result<DeciderSpec> {
        let m = cfg.model(name)?;
        let id = crate::config::builtin_id(&m.model.plugin)?;
        let decider = registry
            .build_decider(id, &cfg.resolved_model_settings(name, host)?)
            .map_err(|mut e| {
                e.message = format!("models.{name}: {}", e.message);
                e
            })?;
        Ok(DeciderSpec {
            name: name.to_string(),
            decider,
            trust: m.provider.trust,
        })
    }

    /// For tests and embedding. Providers in fallback order.
    pub fn with_providers(providers: Vec<ProviderSpec>, default_level: u8) -> Self {
        Self {
            providers,
            checks: Vec::new(),
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
        self.checks = deciders.into_iter().map(Check::Decision).collect();
        self
    }

    /// The semantic check's chain, decision and chat models in order (tests and embedding).
    pub fn with_checks(mut self, checks: Vec<Check>) -> Self {
        self.checks = checks;
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
    /// implement the plugin API are built from the config by [`Engine::with_tools_from`], so call this
    /// first: tools built earlier keep the settings they were built with.
    pub fn with_config(mut self, cfg: Config) -> Self {
        self.cfg = cfg;
        self
    }

    /// The user's `preapproved` rules that are too broad to be safe, as `(tool, rule, why)`. They still
    /// work, because the classifier keeps its veto; `config check` reports them.
    pub fn risky_tool_rules(&self) -> Vec<(String, String, String)> {
        risky_rules(&self.plugged, &self.host)
    }

    /// Build the tools of the registry from the engine's config and host (tests and embedding;
    /// `from_config` does this itself). They replace the tools added so far. A later [`Engine::with_config`]
    /// does not rebuild them.
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
        privacy_check(
            self.checks
                .iter()
                .map(|c| (c.name(), c.trust()))
                .chain(self.providers.iter().map(|p| (p.name.as_str(), p.trust))),
            self.has_redactor,
        )
    }
}

/// What [`Engine::inspect`] found.
#[derive(Debug, Clone)]
pub struct Inspection {
    /// The chat models of `[llm] chain`, in order.
    pub chain: Vec<String>,
    /// `preapproved` rules that are too broad to be safe, as `(tool, rule, why)`.
    pub risky_tool_rules: Vec<(String, String, String)>,
}

/// INV-2: a model whose provider is remote is refused unless a redactor is active. The same rule for the running
/// engine and for `config check`, which looks at the config without building one.
fn privacy_check<'a>(
    models: impl IntoIterator<Item = (&'a str, Trust)>,
    has_redactor: bool,
) -> Result<()> {
    if has_redactor {
        return Ok(());
    }
    match models.into_iter().find(|(_, trust)| *trust == Trust::Remote) {
        Some((name, _)) => Err(Error::new("privacy.no_redaction", format!("model `{name}` is remote but no redaction layer is active"))
            .with_hint("enable `builtin:redact` in [pipeline].llm, or set trust = \"local\" on its provider if the model runs on this machine")),
        None => Ok(()),
    }
}

type Layers = Vec<Arc<dyn Middleware<ChatRequest, ChatStream>>>;

/// The `llm` middleware chain of the config, and whether one of the layers is a redactor.
fn build_layers(cfg: &Config, registry: &Registry, host: &Host) -> Result<(Layers, bool)> {
    let mut layers = Vec::new();
    let mut has_redactor = false;
    for layer in cfg.llm_layers() {
        let id = crate::config::builtin_id(&layer)?;
        has_redactor |= registry.llm_middleware_role(id) == Some(MiddlewareRole::Redactor);
        layers.push(registry.build_llm_middleware(id, &cfg.plugin_settings(&layer, host)?)?);
    }
    Ok((layers, has_redactor))
}

/// The `preapproved` rules of the tools that are too broad to be safe, as `(tool, rule, why)`.
fn risky_rules(plugged: &[tool_calls::Plugged], host: &Host) -> Vec<(String, String, String)> {
    plugged
        .iter()
        .flat_map(|p| {
            p.risky_rules(host)
                .into_iter()
                .map(|(rule, why)| (p.name().to_string(), rule, why))
        })
        .collect()
}
