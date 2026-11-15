use crate::{check_endpoint, Error, Secret};

/// A named base URL for a well-known service. The endpoint has no version or path: the plugin knows where its
/// API lives below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub name: &'static str,
    pub endpoint: &'static str,
    /// The environment variable that conventionally holds the API key of a hosted service.
    pub key_env: Option<&'static str>,
}

impl Preset {
    pub const fn new(
        name: &'static str,
        endpoint: &'static str,
        key_env: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            endpoint,
            key_env,
        }
    }
}

/// The services that plugins reaching an LLM server name by `preset`. Providers and deciders share the list, so
/// a name means the same server in both. A plugin passes it (or its own table) to [`resolve_endpoint`].
pub const PRESETS: &[Preset] = &[
    Preset::new(
        "openrouter",
        "https://openrouter.ai/api",
        Some("OPENROUTER_API_KEY"),
    ),
    Preset::new("openai", "https://api.openai.com", Some("OPENAI_API_KEY")),
    Preset::new("ollama", "http://localhost:11434", None),
    Preset::new("lmstudio", "http://localhost:1234", None),
    Preset::new("localai", "http://localhost:8080", None),
];

/// The API key of a plugin. An explicit `api_key` always wins. Otherwise a hosted preset supplies its
/// conventional variable, read through `lookup`, but only while the endpoint is the preset's own: pass
/// `custom_endpoint` as true when the settings gave an `endpoint`, and a key the user did not hand to that
/// server is never sent to it. An empty variable is no key.
pub fn resolve_api_key(
    explicit: Option<Secret>,
    preset: Option<&Preset>,
    custom_endpoint: bool,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<Secret> {
    if explicit.is_some() {
        return explicit;
    }
    match (preset.and_then(|p| p.key_env), custom_endpoint) {
        (Some(var), false) => lookup(var).filter(|v| !v.is_empty()).map(Secret::new),
        _ => None,
    }
}

/// Pick the endpoint from a settings table: an explicit `endpoint` wins, otherwise the named `preset`.
/// `kind` ("provider", "decider") only words the error. The endpoint is checked with [`check_endpoint`].
///
/// Returns the endpoint and the preset that supplied it or, when `endpoint` is set, the preset that was also
/// named (so a plugin can tell whether the endpoint is the preset's own).
pub fn resolve_endpoint(
    kind: &str,
    presets: &'static [Preset],
    endpoint: Option<&str>,
    preset: Option<&str>,
) -> Result<(String, Option<&'static Preset>), Error> {
    let found = preset.and_then(|n| presets.iter().find(|p| p.name == n));
    let endpoint = match (endpoint, preset) {
        (Some(e), _) => e.to_string(),
        (None, Some(p)) => found.map(|p| p.endpoint.to_string()).ok_or_else(|| {
            let names: Vec<_> = presets.iter().map(|p| p.name).collect();
            Error::new("config.invalid", format!("unknown preset `{p}`"))
                .with_hint(format!("presets: {}", names.join(", ")))
        })?,
        (None, None) => {
            return Err(Error::new(
                "config.invalid",
                format!("{kind} needs `preset` or `endpoint`"),
            )
            .with_hint("e.g. preset = \"openrouter\""))
        }
    };
    check_endpoint(&endpoint)?;
    Ok((endpoint, found))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &[Preset] = &[
        Preset::new("hosted", "https://api.example.com", Some("EXAMPLE_KEY")),
        Preset::new("local", "http://localhost:1", None),
    ];

    #[test]
    fn a_preset_supplies_its_endpoint() {
        let (e, p) = resolve_endpoint("provider", TABLE, None, Some("hosted")).unwrap();
        assert_eq!(e, "https://api.example.com");
        assert_eq!(p.unwrap().key_env, Some("EXAMPLE_KEY"));
    }

    #[test]
    fn an_explicit_endpoint_wins_and_still_reports_the_named_preset() {
        let (e, p) = resolve_endpoint(
            "provider",
            TABLE,
            Some("https://proxy.example"),
            Some("hosted"),
        )
        .unwrap();
        assert_eq!(e, "https://proxy.example");
        assert_eq!(p.unwrap().name, "hosted");
        let (_, p) = resolve_endpoint("provider", TABLE, Some("https://x.example"), None).unwrap();
        assert!(p.is_none());
    }

    #[test]
    fn the_shared_table_is_all_base_urls_that_pass_the_endpoint_check() {
        for p in PRESETS {
            assert!(check_endpoint(p.endpoint).is_ok(), "{}", p.name);
            assert!(!p.endpoint.ends_with('/'), "{}", p.name);
            assert!(!p.endpoint.ends_with("/v1"), "{}", p.name);
        }
    }

    #[test]
    fn a_key_comes_from_the_preset_variable_only_when_nothing_else_gives_one() {
        let hosted = &TABLE[0];
        let env = |n: &str| (n == "EXAMPLE_KEY").then(|| "from-env".to_string());
        let key = |k: Option<&str>, p: Option<&Preset>, custom: bool| {
            resolve_api_key(k.map(Secret::new), p, custom, &env).map(|s| s.expose().to_string())
        };
        assert_eq!(key(None, Some(hosted), false).as_deref(), Some("from-env"));
        assert_eq!(key(Some("x"), Some(hosted), false).as_deref(), Some("x"));
        // a custom endpoint never receives the preset's key
        assert_eq!(key(None, Some(hosted), true), None);
        assert_eq!(key(None, Some(&TABLE[1]), false), None);
        assert_eq!(key(None, None, false), None);
        let empty = |_: &str| Some(String::new());
        assert!(resolve_api_key(None, Some(hosted), false, &empty).is_none());
    }

    #[test]
    fn an_unknown_preset_lists_the_known_ones() {
        let err = resolve_endpoint("provider", TABLE, None, Some("nope")).unwrap_err();
        assert!(err.to_string().contains("unknown preset `nope`"));
        assert!(format!("{err:?}").contains("hosted, local"));
    }

    #[test]
    fn neither_preset_nor_endpoint_names_the_kind() {
        let err = resolve_endpoint("decider", TABLE, None, None).unwrap_err();
        assert!(err.to_string().contains("decider needs"));
    }

    #[test]
    fn the_endpoint_is_checked_even_when_it_comes_from_a_preset() {
        const BAD: &[Preset] = &[Preset::new("bad", "http://10.0.0.5:1", None)];
        assert!(resolve_endpoint("provider", BAD, None, Some("bad")).is_err());
        assert!(resolve_endpoint("provider", TABLE, Some("ftp://x"), None).is_err());
    }
}
