//! The static checks of a parsed config (no secrets read, nothing built).

use super::*;

impl Config {
    /// Checks that need no plugin registry.
    pub(super) fn check_static(&self) -> Result<()> {
        if self.version != 1 {
            return Err(Error::new(
                "config.invalid",
                format!("unsupported config version {}", self.version),
            )
            .with_hint("this build understands `version = 1`"));
        }
        if self.runtime.mode != "oneshot" {
            return Err(Error::new(
                "config.unsupported",
                format!(
                    "runtime.mode = \"{}\" is not supported yet",
                    self.runtime.mode
                ),
            )
            .with_hint("use \"oneshot\""));
        }
        if self.context.git_log_lines > 20 {
            return Err(Error::new(
                "config.invalid",
                "context.git_log_lines must be 0..=20",
            ));
        }
        if self.context.default_level > 4 {
            return Err(Error::new(
                "config.invalid",
                "context.default_level must be 0..=4",
            ));
        }
        for (key, action) in &self.input.shortcuts {
            Shortcut::parse(key)
                .map_err(|e| Error::new(e.code, format!("input.shortcuts: {}", e.message)))?;
            if !KNOWN_ACTIONS.contains(&action.as_str()) {
                return Err(Error::new(
                    "config.invalid",
                    format!("input.shortcuts: `{key}` maps to unknown action `{action}`"),
                )
                .with_hint(format!("known actions: {}", KNOWN_ACTIONS.join(", "))));
            }
        }
        if self.conversation.store != "none" {
            builtin_id(&self.conversation.store)
                .map_err(|e| Error::new(e.code, format!("conversation.store: {}", e.message)))?;
        }
        parse_duration(&self.audit.retention)
            .map_err(|e| Error::new(e.code, format!("audit.retention: {}", e.message)))?;
        parse_duration(&self.conversation.idle_timeout)
            .map_err(|e| Error::new(e.code, format!("conversation.idle_timeout: {}", e.message)))?;
        let cmd_timeout = parse_duration(&self.secrets.cmd_timeout)
            .map_err(|e| Error::new(e.code, format!("secrets.cmd_timeout: {}", e.message)))?;
        if cmd_timeout.is_zero() || cmd_timeout > std::time::Duration::from_secs(120) {
            return Err(Error::new(
                "config.invalid",
                "secrets.cmd_timeout must be between 1s and 120s",
            ));
        }
        for name in &self.secrets.pass_env {
            let ok = !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !ok {
                return Err(Error::new(
                    "config.invalid",
                    format!("secrets.pass_env: `{name}` is not an environment variable name"),
                ));
            }
        }
        if self.conversation.history.strategy != "window" {
            return Err(Error::new(
                "config.unsupported",
                format!(
                    "conversation.history.strategy = \"{}\" is not supported yet",
                    self.conversation.history.strategy
                ),
            )
            .with_hint("use \"window\""));
        }
        for (name, a) in &self.actions {
            if !LLM_ACTIONS.contains(&name.as_str()) {
                return Err(Error::new(
                    "config.invalid",
                    format!("actions.{name}: unknown action"),
                )
                .with_hint(format!("actions with settings: {}", LLM_ACTIONS.join(", "))));
            }
            if let Some(t) = &a.idle_timeout {
                parse_duration(t).map_err(|e| {
                    Error::new(
                        e.code,
                        format!("actions.{name}.idle_timeout: {}", e.message),
                    )
                })?;
            }
        }
        parse_duration(&self.validator.timeout)
            .map_err(|e| Error::new(e.code, format!("validator.timeout: {}", e.message)))?;
        if !(0.5..=1.0).contains(&self.validator.min_confidence) {
            return Err(Error::new(
                "config.invalid",
                "validator.min_confidence must be between 0.5 and 1",
            ));
        }
        for (list, names) in [
            ("llm.chain", &self.llm.chain),
            ("validator.chain", &self.validator.chain),
        ] {
            for name in names {
                if !self.models.contains_key(name) {
                    let known: Vec<_> = self.models.keys().map(String::as_str).collect();
                    return Err(Error::new(
                        "config.invalid",
                        format!("{list} refers to unknown model `{name}`"),
                    )
                    .with_hint(if known.is_empty() {
                        "no models are defined (add a [models.<name>] table)".to_string()
                    } else {
                        format!("defined models: {}", known.join(", "))
                    }));
                }
            }
        }
        for (name, m) in &self.models {
            builtin_id(&m.plugin)
                .map_err(|e| Error::new(e.code, format!("models.{name}: {}", e.message)))?;
            let p = self.providers.get(&m.provider).ok_or_else(|| {
                let known: Vec<_> = self.providers.keys().map(String::as_str).collect();
                Error::new(
                    "config.invalid",
                    format!("models.{name} refers to unknown provider `{}`", m.provider),
                )
                .with_hint(if known.is_empty() {
                    "no providers are defined (add a [providers.<name>] table)".to_string()
                } else {
                    format!("defined providers: {}", known.join(", "))
                })
            })?;
            for key in m.settings.keys() {
                if p.settings.contains_key(key) {
                    return Err(Error::new(
                        "config.invalid",
                        format!(
                            "`{key}` is set in models.{name} and in providers.{}",
                            m.provider
                        ),
                    )
                    .with_hint("set it in one place: the provider for the connection, the model for the model"));
                }
            }
        }
        for id in self
            .pipeline
            .llm
            .iter()
            .filter(|i| i.as_str() != "...")
            .chain(self.plugins.keys())
        {
            builtin_id(id)?;
        }
        for (name, p) in &self.providers {
            if p.max_context_level.is_some_and(|l| l > 4) {
                return Err(Error::new(
                    "config.invalid",
                    format!("providers.{name}.max_context_level must be 0..=4"),
                ));
            }
        }
        Ok(())
    }
}
