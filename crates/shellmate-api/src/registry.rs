use crate::{ChatRequest, ChatStream, Decider, Error, Middleware, Provider, Store, Tool, ToolCtor};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Builds a provider instance from its `settings` table (already converted to JSON; secret
/// references such as `env:NAME` are resolved by the host before this point).
pub type ProviderCtor =
    Box<dyn Fn(&serde_json::Value) -> Result<Arc<dyn Provider>, Error> + Send + Sync>;

/// A provider constructor that can be kept and called later.
pub type ProviderBuilder =
    Arc<dyn Fn(&serde_json::Value) -> Result<Arc<dyn Provider>, Error> + Send + Sync>;

/// Builds a decision-model backend from its settings table.
pub type DeciderCtor =
    Box<dyn Fn(&serde_json::Value) -> Result<Arc<dyn Decider>, Error> + Send + Sync>;

/// Builds an `llm`-stage middleware layer from its settings table.
pub type LlmMiddlewareCtor = Box<
    dyn Fn(&serde_json::Value) -> Result<Arc<dyn Middleware<ChatRequest, ChatStream>>, Error>
        + Send
        + Sync,
>;

/// Builds a conversation store from its settings table.
pub type StoreCtor = Box<dyn Fn(&serde_json::Value) -> Result<Arc<dyn Store>, Error> + Send + Sync>;

/// What a model plugin answers with. The plugin id says which: `openai-compatible` is chat, `systemone` is
/// decision. A config table never repeats it, so the two cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// Chat or completion: a [`Provider`].
    Chat,
    /// A decision model: a [`Decider`].
    Decision,
}

/// What a middleware plugin is for. The core uses roles to enforce invariants: a `Redactor` must be
/// active before anything is sent to a remote provider (INV-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiddlewareRole {
    Redactor,
    Filter,
    Observer,
}

/// A named set of plugin constructors of one kind. The same lookup, error and listing for every kind of
/// plugin; `M` is what the host wants to know about a plugin without building it (a middleware's role).
struct Catalog<T: ?Sized, M = ()> {
    /// What a missing id is called in the error: "no {what} plugin `x`".
    what: &'static str,
    /// The start of the hint that lists the ids: "{hint}: a, b".
    hint: &'static str,
    entries: BTreeMap<String, (M, Ctor<T>)>,
}

type Ctor<T> = Arc<dyn Fn(&serde_json::Value) -> Result<Arc<T>, Error> + Send + Sync>;

impl<T: ?Sized, M: Copy> Catalog<T, M> {
    fn new(what: &'static str, hint: &'static str) -> Self {
        Self {
            what,
            hint,
            entries: BTreeMap::new(),
        }
    }

    /// A second registration under the same id replaces the first.
    fn insert(&mut self, id: &str, meta: M, ctor: Ctor<T>) {
        self.entries.insert(id.to_string(), (meta, ctor));
    }

    /// The constructor itself, for building later without the registry.
    fn ctor(&self, id: &str) -> Result<Ctor<T>, Error> {
        let (_, ctor) = self.entries.get(id).ok_or_else(|| {
            let known: Vec<_> = self.ids().collect();
            Error::new(
                "config.unknown_plugin",
                format!("no {} plugin `{id}`", self.what),
            )
            .with_hint(format!("{}: {}", self.hint, known.join(", ")))
        })?;
        Ok(ctor.clone())
    }

    fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    fn meta(&self, id: &str) -> Option<M> {
        self.entries.get(id).map(|(m, _)| *m)
    }

    fn build(&self, id: &str, settings: &serde_json::Value) -> Result<Arc<T>, Error> {
        self.ctor(id)?(settings)
    }
}

/// Where first-party and (later) external plugins register themselves.
pub struct Registry {
    providers: Catalog<dyn Provider>,
    llm_middleware: Catalog<dyn Middleware<ChatRequest, ChatStream>, Option<MiddlewareRole>>,
    stores: Catalog<dyn Store>,
    deciders: Catalog<dyn Decider>,
    tools: Catalog<dyn Tool>,
    /// Tool names in the order they were registered: the order the model is offered them in.
    tool_order: Vec<String>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            providers: Catalog::new("provider", "available providers"),
            llm_middleware: Catalog::new("llm middleware", "available"),
            stores: Catalog::new("store", "available stores"),
            deciders: Catalog::new("decider", "available deciders"),
            tools: Catalog::new("tool", "available tools"),
            tool_order: Vec::new(),
        }
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// `id` is the plugin id without the `builtin:` prefix, e.g. `openai-compatible`.
    pub fn register_provider(&mut self, id: &str, ctor: ProviderCtor) {
        assert!(
            self.deciders.meta(id).is_none(),
            "plugin id `{id}` is already a decision model"
        );
        self.providers.insert(id, (), ctor.into());
    }

    pub fn provider_ids(&self) -> impl Iterator<Item = &str> {
        self.providers.ids()
    }

    /// The constructor of a provider plugin, to build an instance later (secrets that are fetched on first
    /// use) without keeping the registry around.
    pub fn provider_builder(&self, id: &str) -> Result<ProviderBuilder, Error> {
        self.providers.ctor(id)
    }

    pub fn build_provider(
        &self,
        id: &str,
        settings: &serde_json::Value,
    ) -> Result<Arc<dyn Provider>, Error> {
        self.providers.build(id, settings)
    }

    pub fn register_decider(&mut self, id: &str, ctor: DeciderCtor) {
        assert!(
            self.providers.meta(id).is_none(),
            "plugin id `{id}` is already a chat model"
        );
        self.deciders.insert(id, (), ctor.into());
    }

    /// Which kind of model the plugin `id` is, or an error that lists every model plugin there is.
    pub fn model_kind(&self, id: &str) -> Result<ModelKind, Error> {
        if self.providers.meta(id).is_some() {
            return Ok(ModelKind::Chat);
        }
        if self.deciders.meta(id).is_some() {
            return Ok(ModelKind::Decision);
        }
        let known: Vec<_> = self.providers.ids().chain(self.deciders.ids()).collect();
        Err(
            Error::new("config.unknown_plugin", format!("no model plugin `{id}`"))
                .with_hint(format!("available model plugins: {}", known.join(", "))),
        )
    }

    pub fn decider_ids(&self) -> impl Iterator<Item = &str> {
        self.deciders.ids()
    }

    pub fn build_decider(
        &self,
        id: &str,
        settings: &serde_json::Value,
    ) -> Result<Arc<dyn Decider>, Error> {
        self.deciders.build(id, settings)
    }

    /// Register an `llm`-stage middleware. `id` is the plugin id without the `builtin:` prefix.
    pub fn register_llm_middleware(
        &mut self,
        id: &str,
        role: Option<MiddlewareRole>,
        ctor: LlmMiddlewareCtor,
    ) {
        self.llm_middleware.insert(id, role, ctor.into());
    }

    pub fn llm_middleware_role(&self, id: &str) -> Option<MiddlewareRole> {
        self.llm_middleware.meta(id).flatten()
    }

    pub fn build_llm_middleware(
        &self,
        id: &str,
        settings: &serde_json::Value,
    ) -> Result<Arc<dyn Middleware<ChatRequest, ChatStream>>, Error> {
        self.llm_middleware.build(id, settings)
    }

    /// Register a tool. `id` is the tool's name, the one the model writes and `[tools.<name>]` uses.
    pub fn register_tool(&mut self, id: &str, ctor: ToolCtor) {
        if !self.tool_order.iter().any(|t| t == id) {
            self.tool_order.push(id.to_string());
        }
        self.tools.insert(id, (), ctor.into());
    }

    /// The names of the tools in the order they were registered, which is the order they are offered in.
    pub fn tool_ids(&self) -> impl Iterator<Item = &str> {
        self.tool_order.iter().map(String::as_str)
    }

    pub fn build_tool(
        &self,
        id: &str,
        settings: &serde_json::Value,
    ) -> Result<Arc<dyn Tool>, Error> {
        self.tools.build(id, settings)
    }

    pub fn register_store(&mut self, id: &str, ctor: StoreCtor) {
        self.stores.insert(id, (), ctor.into());
    }

    pub fn store_ids(&self) -> impl Iterator<Item = &str> {
        self.stores.ids()
    }

    pub fn build_store(
        &self,
        id: &str,
        settings: &serde_json::Value,
    ) -> Result<Arc<dyn Store>, Error> {
        self.stores.build(id, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatResponse;
    use crate::{ProviderError, ProviderInfo};

    struct Named(&'static str);

    #[async_trait::async_trait]
    impl Provider for Named {
        fn info(&self) -> ProviderInfo {
            ProviderInfo {
                plugin: self.0.into(),
                model: "m".into(),
                endpoint: None,
            }
        }
        async fn chat(&self, _: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse {
                text: String::new(),
                usage: None,
                finish_reason: None,
            })
        }
    }

    fn provider(name: &'static str) -> ProviderCtor {
        Box::new(move |_| Ok(Arc::new(Named(name))))
    }

    #[test]
    fn a_registered_plugin_is_built_with_its_settings() {
        let mut r = Registry::new();
        r.register_provider(
            "echo",
            Box::new(|s| {
                assert_eq!(s["model"], "x");
                Ok(Arc::new(Named("echo")))
            }),
        );
        let p = r
            .build_provider("echo", &serde_json::json!({"model": "x"}))
            .unwrap();
        assert_eq!(p.info().plugin, "echo");
    }

    #[test]
    fn the_plugin_id_says_which_kind_of_model_it_is() {
        struct D;
        #[async_trait::async_trait]
        impl Decider for D {
            fn info(&self) -> crate::ProviderInfo {
                crate::ProviderInfo {
                    plugin: "d".into(),
                    model: "m".into(),
                    endpoint: None,
                }
            }
            async fn decide(
                &self,
                _: &crate::DecisionRequest,
            ) -> Result<crate::DecisionResponse, crate::ProviderError> {
                Ok(Default::default())
            }
        }
        let mut r = Registry::new();
        r.register_provider("chat", provider("chat"));
        r.register_decider("decide", Box::new(|_| Ok(Arc::new(D))));
        assert_eq!(r.model_kind("chat").unwrap(), ModelKind::Chat);
        assert_eq!(r.model_kind("decide").unwrap(), ModelKind::Decision);
        let e = r.model_kind("nope").unwrap_err();
        assert_eq!(e.code, "config.unknown_plugin");
        assert!(e.hint.unwrap().contains("chat, decide"));
    }

    #[test]
    #[should_panic(expected = "already a chat model")]
    fn one_id_cannot_be_both_kinds() {
        let mut r = Registry::new();
        r.register_provider("x", provider("x"));
        r.register_decider("x", Box::new(|_| unreachable!()));
    }

    #[test]
    fn an_unknown_id_names_the_kind_and_lists_what_exists() {
        let mut r = Registry::new();
        r.register_provider("b", provider("b"));
        r.register_provider("a", provider("a"));
        let e = r
            .build_provider("zzz", &serde_json::json!({}))
            .err()
            .unwrap();
        assert_eq!(e.code, "config.unknown_plugin");
        assert_eq!(e.message, "no provider plugin `zzz`");
        assert_eq!(e.hint.as_deref(), Some("available providers: a, b"));
        assert_eq!(r.provider_ids().collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn every_kind_reports_its_own_name_and_an_empty_list() {
        let r = Registry::new();
        let none = serde_json::json!({});
        let msg = |e: Error| (e.message, e.hint.unwrap());
        assert_eq!(
            msg(r.build_decider("x", &none).err().unwrap()),
            (
                "no decider plugin `x`".into(),
                "available deciders: ".into()
            )
        );
        assert_eq!(
            msg(r.build_store("x", &none).err().unwrap()),
            ("no store plugin `x`".into(), "available stores: ".into())
        );
        assert_eq!(
            msg(r.build_llm_middleware("x", &none).err().unwrap()),
            ("no llm middleware plugin `x`".into(), "available: ".into())
        );
    }

    #[test]
    fn a_second_registration_replaces_the_first() {
        let mut r = Registry::new();
        r.register_provider("p", provider("first"));
        r.register_provider("p", provider("second"));
        let p = r.build_provider("p", &serde_json::json!({})).unwrap();
        assert_eq!(p.info().plugin, "second");
        assert_eq!(r.provider_ids().count(), 1);
    }

    #[test]
    fn a_constructor_error_passes_through() {
        let mut r = Registry::new();
        r.register_provider(
            "bad",
            Box::new(|_| Err(Error::new("config.invalid", "nope"))),
        );
        let e = r
            .build_provider("bad", &serde_json::json!({}))
            .err()
            .unwrap();
        assert_eq!(e.code, "config.invalid");
    }

    #[test]
    fn a_middleware_role_is_known_without_building_it() {
        struct Pass;
        #[async_trait::async_trait]
        impl Middleware<ChatRequest, ChatStream> for Pass {
            fn id(&self) -> &str {
                "pass"
            }
            async fn call(
                &self,
                request: ChatRequest,
                next: crate::Next<'_, ChatRequest, ChatStream>,
            ) -> Result<ChatStream, Error> {
                next.run(request).await
            }
        }
        let mut r = Registry::new();
        r.register_llm_middleware(
            "redact",
            Some(MiddlewareRole::Redactor),
            Box::new(|_| Ok(Arc::new(Pass))),
        );
        r.register_llm_middleware("plain", None, Box::new(|_| Ok(Arc::new(Pass))));
        assert_eq!(
            r.llm_middleware_role("redact"),
            Some(MiddlewareRole::Redactor)
        );
        assert_eq!(r.llm_middleware_role("plain"), None);
        assert_eq!(r.llm_middleware_role("missing"), None);
    }

    #[test]
    fn a_tool_is_registered_by_its_name_and_built_with_its_settings() {
        use crate::{
            Assessment, Granted, Prepared, ToolAction, ToolCall, ToolCtx, ToolInfo, ToolOutput,
            ToolSettings,
        };
        struct Echo;
        #[async_trait::async_trait]
        impl Tool for Echo {
            fn info(&self) -> ToolInfo {
                ToolInfo {
                    name: "echo",
                    permissions: vec![],
                    approval: crate::ApprovalMode::Never,
                    untrusted_output: false,
                }
            }
            fn offer(&self, _: ToolAction, _: &ToolSettings) -> Option<String> {
                None
            }
            async fn assess(&self, _: &ToolCall, _: &ToolCtx<'_>) -> Assessment {
                Assessment::Allow(Prepared::new(()))
            }
            async fn run(&self, _: Prepared, _: &Granted, _: &ToolCtx<'_>) -> ToolOutput {
                unreachable!("not run in this test")
            }
        }
        let mut r = Registry::new();
        r.register_tool(
            "echo",
            Box::new(|s| {
                assert_eq!(s["x"], 1);
                Ok(Arc::new(Echo))
            }),
        );
        assert_eq!(r.tool_ids().collect::<Vec<_>>(), ["echo"]);
        assert_eq!(
            r.build_tool("echo", &serde_json::json!({"x": 1}))
                .unwrap()
                .info()
                .name,
            "echo"
        );
        let e = r.build_tool("nope", &serde_json::json!({})).err().unwrap();
        assert_eq!(e.message, "no tool plugin `nope`");
        assert_eq!(e.hint.as_deref(), Some("available tools: echo"));
    }
}
