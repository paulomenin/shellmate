//! Named presets: the endpoint and key variable for well-known services.

/// Presets provide a default endpoint and, for hosted services, the environment variable that conventionally
/// holds the API key. `endpoint` overrides the endpoint.
pub(crate) const PRESETS: &[Preset] = &[
    Preset::new(
        "openai",
        "https://api.openai.com/v1",
        Some("OPENAI_API_KEY"),
    ),
    Preset::new(
        "openrouter",
        "https://openrouter.ai/api/v1",
        Some("OPENROUTER_API_KEY"),
    ),
    Preset::new("ollama", "http://localhost:11434/v1", None),
    Preset::new("lmstudio", "http://localhost:1234/v1", None),
    Preset::new("localai", "http://localhost:8080/v1", None),
];

pub(crate) struct Preset {
    pub(crate) name: &'static str,
    pub(crate) endpoint: &'static str,
    pub(crate) key_env: Option<&'static str>,
}

impl Preset {
    const fn new(
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
