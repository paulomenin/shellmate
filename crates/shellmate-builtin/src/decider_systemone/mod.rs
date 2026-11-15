//! Decider plugin for decision models over the System One wire format: a `POST` with a `state` and typed
//! `questions`, answered with labels and probabilities. The path is `/v1/systemone` below the endpoint unless
//! a preset or the `path` setting says otherwise. Served by Ollama (decision models such as `laya`), by
//! Ollaya, and by hosted APIs that use the same format.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use shellmate_api::{
    parse_model_ids, resolve_api_key, resolve_endpoint, Answer, Decider, DecisionRequest,
    DecisionResponse, Error, Preset, ProviderError, ProviderErrorKind, ProviderInfo, Question,
    Registry, Secret, PRESETS,
};
use std::sync::Arc;
use std::time::Duration;

pub const PLUGIN_ID: &str = "systemone";

/// TypeSafe serves its decision models itself, at the default path. It is a decider preset only: a provider
/// would reach `/v1/chat/completions` there, which it does not serve.
const TYPESAFE: Preset = Preset::new(
    "typesafe",
    "https://api.typesafe.ai",
    Some("TYPESAFE_API_KEY"),
);

const SHARED_LEN: usize = PRESETS.len();

/// The presets of a decider: the ones every LLM server shares (so a name means the same server in a provider
/// and in a decider), and the hosts that only serve decision models.
static DECIDER_PRESETS: [Preset; SHARED_LEN + 1] = {
    let mut all = [TYPESAFE; SHARED_LEN + 1];
    let mut i = 0;
    while i < SHARED_LEN {
        all[i] = PRESETS[i];
        i += 1;
    }
    all
};

/// The presets known to serve the decision format, for `setup` to offer as checkers. The rest of the table
/// are LLM hosts that a provider can use but that do not serve it.
pub fn checker_presets() -> Vec<&'static Preset> {
    DECIDER_PRESETS
        .iter()
        .filter(|p| matches!(p.name, "openrouter" | "ollama" | "typesafe"))
        .collect()
}

/// Where the server lists its models, below the base URL. TypeSafe, OpenRouter and Ollama answer here.
const MODELS_PATH: &str = "/v1/models";

/// Where the format is served below the endpoint, unless a preset or the `path` setting says otherwise.
const DEFAULT_PATH: &str = "/v1/systemone";

/// The path a preset's own endpoint serves the format at, where it is not [`DEFAULT_PATH`]. The path is the
/// plugin's knowledge, not part of the shared preset: a provider does not need it.
fn preset_path(preset: &str) -> Option<&'static str> {
    match preset {
        // OpenRouter serves decision models on its alpha Decisions API
        "openrouter" => Some("/alpha/decisions"),
        _ => None,
    }
}

/// A `path` setting must be a plain path below the endpoint, so it cannot change the host or add a query.
fn check_path(path: &str) -> Result<(), Error> {
    let bad = !path.starts_with('/')
        || path.contains("//")
        || path.contains("..")
        || path.contains("://")
        || path
            .chars()
            .any(|c| matches!(c, '?' | '#' | '\\') || c.is_whitespace() || c.is_control());
    if bad {
        return Err(Error::new(
            "config.invalid",
            format!("decider `path` `{path}` is not a plain path"),
        )
        .with_hint("start it with `/` and leave out `..`, `?`, `#` and spaces, e.g. path = \"/v1/systemone\""));
    }
    Ok(())
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub preset: Option<String>,
    /// Base URL without the path.
    pub endpoint: Option<String>,
    /// Where the decision API lives below the endpoint. Default `/v1/systemone`, or the preset's own.
    pub path: Option<String>,
    pub model: String,
    pub api_key: Option<Secret>,
    pub timeout_secs: Option<u64>,
}

pub fn register(registry: &mut Registry) {
    registry.register_decider(
        PLUGIN_ID,
        Box::new(|settings| Ok(Arc::new(SystemOne::from_settings(settings)?) as Arc<dyn Decider>)),
    );
}

pub struct SystemOne {
    client: reqwest::Client,
    url: String,
    models_url: String,
    model: String,
    api_key: Option<Secret>,
}

impl SystemOne {
    /// Build from the settings table. A hosted preset's conventional key variable is read from the process
    /// environment.
    pub fn from_settings(value: &Value) -> Result<Self, Error> {
        Self::from_settings_with(value, &|name| std::env::var(name).ok())
    }

    /// Like [`SystemOne::from_settings`], reading environment variables through `lookup`.
    pub fn from_settings_with(
        value: &Value,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, Error> {
        let s: Settings = serde_json::from_value(value.clone())
            .map_err(|e| Error::new("config.invalid", format!("decider settings: {e}")))?;
        let (endpoint, preset) = resolve_endpoint(
            "decider",
            &DECIDER_PRESETS,
            s.endpoint.as_deref(),
            s.preset.as_deref(),
        )?;
        let api_key = resolve_api_key(s.api_key, preset, s.endpoint.is_some(), lookup);
        // an explicit `path` wins; a preset's path applies only while the endpoint is the preset's own
        let path = match (&s.path, preset, &s.endpoint) {
            (Some(p), _, _) => {
                check_path(p)?;
                p.as_str()
            }
            (None, Some(p), None) => preset_path(p.name).unwrap_or(DEFAULT_PATH),
            _ => DEFAULT_PATH,
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(s.timeout_secs.unwrap_or(10)))
            .user_agent(concat!("shellmate/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::new("provider.error", format!("http client: {e}")))?;
        Ok(Self {
            client,
            url: format!("{}{path}", endpoint.trim_end_matches('/')),
            models_url: format!("{}{MODELS_PATH}", endpoint.trim_end_matches('/')),
            model: s.model,
            api_key,
        })
    }
}

/// The wire request body.
fn wire_request(model: &str, req: &DecisionRequest) -> Value {
    let state: Map<String, Value> = req
        .state
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let questions: Map<String, Value> = req
        .questions
        .iter()
        .map(|(id, q)| {
            let body = match q {
                Question::Choice {
                    instructions,
                    criteria,
                } => json!({
                    "type": "choice",
                    "instructions": instructions,
                    "criteria": criteria
                        .iter()
                        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                        .collect::<Map<String, Value>>(),
                }),
                Question::YesNo { instructions } => {
                    json!({"type": "noul", "instructions": instructions})
                }
            };
            (id.clone(), body)
        })
        .collect();
    json!({"model": model, "state": state, "questions": questions})
}

#[derive(Deserialize)]
struct WireResponse {
    answers: Map<String, Value>,
}

/// Read the answers for the questions that were asked. A missing or malformed answer is an error: a
/// decision that cannot be read must not be mistaken for an answer.
fn parse_response(body: &str, asked: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
    let bad = |m: String| ProviderError::new(ProviderErrorKind::Other, m);
    let wire: WireResponse =
        serde_json::from_str(body).map_err(|e| bad(format!("unexpected response format: {e}")))?;
    let mut out = DecisionResponse::default();
    for (id, q) in &asked.questions {
        let a = wire
            .answers
            .get(id)
            .ok_or_else(|| bad(format!("no answer for question `{id}`")))?;
        let num = |v: &Value| v.as_f64().map(|f| f.clamp(0.0, 1.0) as f32);
        let answer = match q {
            Question::YesNo { .. } => Answer::YesNo {
                probability: a
                    .get("noul")
                    .and_then(num)
                    .ok_or_else(|| bad(format!("answer `{id}` has no probability")))?,
            },
            Question::Choice { criteria, .. } => {
                let label = a
                    .get("choice")
                    .and_then(Value::as_str)
                    .filter(|l| criteria.iter().any(|(c, _)| c == l))
                    .ok_or_else(|| bad(format!("answer `{id}` is not one of the labels")))?;
                let probabilities = criteria
                    .iter()
                    .map(|(c, _)| {
                        (
                            c.clone(),
                            a["probabilities"].get(c).and_then(num).unwrap_or(0.0),
                        )
                    })
                    .collect();
                Answer::Choice {
                    label: label.to_string(),
                    confidence: a.get("confidence").and_then(num).unwrap_or(0.0),
                    probabilities,
                }
            }
        };
        out.answers.push((id.clone(), answer));
    }
    Ok(out)
}

fn classify_status(status: u16) -> ProviderErrorKind {
    match status {
        401 | 403 => ProviderErrorKind::Auth,
        408 => ProviderErrorKind::Timeout,
        429 => ProviderErrorKind::RateLimited,
        400 | 404 | 413 | 422 => ProviderErrorKind::BadRequest,
        500..=599 => ProviderErrorKind::Unavailable,
        _ => ProviderErrorKind::Other,
    }
}

#[async_trait]
impl Decider for SystemOne {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            plugin: PLUGIN_ID.into(),
            model: self.model.clone(),
            endpoint: Some(self.url.clone()),
        }
    }

    async fn models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let mut rb = self
            .client
            .get(&self.models_url)
            .timeout(Duration::from_secs(10));
        if let Some(k) = &self.api_key {
            rb = rb.bearer_auth(k.expose());
        }
        let resp = rb.send().await.map_err(|e| {
            let kind = if e.is_timeout() {
                ProviderErrorKind::Timeout
            } else {
                ProviderErrorKind::Unavailable
            };
            ProviderError::new(kind, e.without_url().to_string())
        })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            ProviderError::new(ProviderErrorKind::Unavailable, e.without_url().to_string())
        })?;
        if !status.is_success() {
            let mut msg = format!(
                "HTTP {}: {}",
                status.as_u16(),
                text.trim().chars().take(300).collect::<String>()
            );
            if status.as_u16() == 404 {
                msg.push_str(" (the endpoint should be a base URL without `/v1`)");
            }
            return Err(ProviderError::new(classify_status(status.as_u16()), msg));
        }
        Ok(Some(parse_model_ids(&text)))
    }

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        let mut rb = self
            .client
            .post(&self.url)
            .json(&wire_request(&self.model, request));
        if let Some(k) = &self.api_key {
            rb = rb.bearer_auth(k.expose());
        }
        let resp = rb.send().await.map_err(|e| {
            let kind = if e.is_timeout() {
                ProviderErrorKind::Timeout
            } else {
                ProviderErrorKind::Unavailable
            };
            ProviderError::new(kind, e.without_url().to_string())
        })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            ProviderError::new(ProviderErrorKind::Unavailable, e.without_url().to_string())
        })?;
        if !status.is_success() {
            let msg: String = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| {
                    v["error"]["message"]
                        .as_str()
                        .or_else(|| v["error"].as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| text.trim().to_owned())
                .chars()
                .take(300)
                .collect();
            return Err(ProviderError::new(
                classify_status(status.as_u16()),
                format!("HTTP {}: {msg}", status.as_u16()),
            ));
        }
        parse_response(&text, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellmate_testing::MockHttpServer;

    fn req() -> DecisionRequest {
        DecisionRequest {
            state: vec![("request".into(), "show disk usage".into())],
            questions: vec![
                (
                    "match".into(),
                    Question::YesNo {
                        instructions: "Does it match?".into(),
                    },
                ),
                (
                    "risk".into(),
                    Question::Choice {
                        instructions: "How risky?".into(),
                        criteria: vec![
                            ("safe".into(), "reads only".into()),
                            ("danger".into(), "destroys data".into()),
                        ],
                    },
                ),
            ],
        }
    }

    fn settings(url: &str) -> Value {
        json!({"endpoint": url, "model": "laya"})
    }

    #[test]
    fn the_wire_request_has_the_expected_shape() {
        let w = wire_request("laya", &req());
        assert_eq!(w["model"], "laya");
        assert_eq!(w["state"]["request"], "show disk usage");
        assert_eq!(w["questions"]["match"]["type"], "noul");
        assert_eq!(w["questions"]["risk"]["type"], "choice");
        assert_eq!(
            w["questions"]["risk"]["criteria"]["danger"],
            "destroys data"
        );
    }

    #[tokio::test]
    async fn models_lists_ids_with_the_key_only_and_names_a_wrong_url() {
        let server = MockHttpServer::start(200, r#"{"data":[{"id":"jev-latest"}]}"#).await;
        let d = SystemOne::from_settings(
            &json!({"model": "m", "endpoint": server.url(), "api_key": "sk-test"}),
        )
        .unwrap();
        assert_eq!(
            d.models().await.unwrap(),
            Some(vec!["jev-latest".to_string()])
        );
        let seen = server.last_request().await;
        assert!(seen.starts_with("GET /v1/models"), "{seen}");
        assert!(seen
            .to_lowercase()
            .contains("authorization: bearer sk-test"));
        assert!(
            !seen.contains("questions") && !seen.contains("state"),
            "{seen}"
        );

        let rejected = MockHttpServer::start(401, "no").await;
        let d =
            SystemOne::from_settings(&json!({"model": "m", "endpoint": rejected.url()})).unwrap();
        assert_eq!(d.models().await.unwrap_err().kind, ProviderErrorKind::Auth);

        let missing = MockHttpServer::start(404, "nope").await;
        let d =
            SystemOne::from_settings(&json!({"model": "m", "endpoint": missing.url()})).unwrap();
        assert!(d
            .models()
            .await
            .unwrap_err()
            .message
            .contains("without `/v1`"));
    }

    #[test]
    fn only_hosts_known_to_serve_the_format_are_offered_as_checkers() {
        let names: Vec<_> = checker_presets().iter().map(|p| p.name).collect();
        assert_eq!(names, ["openrouter", "ollama", "typesafe"]);
    }

    #[test]
    fn the_typesafe_preset_is_for_deciders_and_reads_its_own_key_variable() {
        let env = |n: &str| (n == "TYPESAFE_API_KEY").then(|| "ts-env".to_string());
        let d = SystemOne::from_settings_with(
            &json!({"model": "jev-latest", "preset": "typesafe"}),
            &env,
        )
        .unwrap();
        assert_eq!(d.url, "https://api.typesafe.ai/v1/systemone");
        assert_eq!(
            d.api_key.map(|k| k.expose().to_string()).as_deref(),
            Some("ts-env")
        );
        // a provider does not know it: it would not serve chat completions there
        assert!(!PRESETS.iter().any(|p| p.name == "typesafe"));
    }

    #[test]
    fn the_path_comes_from_the_setting_then_the_preset_then_the_default() {
        let url = |v: Value| SystemOne::from_settings(&v).unwrap().url;
        assert_eq!(
            url(json!({"model": "m", "preset": "openrouter"})),
            "https://openrouter.ai/api/alpha/decisions"
        );
        assert_eq!(
            url(json!({"model": "m", "preset": "ollama"})),
            "http://localhost:11434/v1/systemone"
        );
        assert_eq!(
            url(json!({"model": "m", "preset": "openrouter", "path": "/v2/decide"})),
            "https://openrouter.ai/api/v2/decide"
        );
        // a custom endpoint does not inherit the preset's path
        assert_eq!(
            url(json!({"model": "m", "preset": "openrouter", "endpoint": "https://proxy.example"})),
            "https://proxy.example/v1/systemone"
        );
        assert_eq!(
            url(json!({"model": "m", "endpoint": "https://proxy.example/", "path": "/x"})),
            "https://proxy.example/x"
        );
    }

    #[test]
    fn a_path_that_could_change_the_host_or_the_query_is_refused() {
        for bad in [
            "v1/systemone",
            "//evil.example/x",
            "/a/../b",
            "/x?key=1",
            "/x#frag",
            "http://evil.example",
            "/a b",
            "/a\\b",
            "",
        ] {
            let r = SystemOne::from_settings(
                &json!({"model": "m", "endpoint": "https://ok.example", "path": bad}),
            );
            assert!(r.is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_hosted_preset_supplies_its_key_variable_and_a_custom_endpoint_never_gets_it() {
        let env = |n: &str| (n == "OPENAI_API_KEY").then(|| "sk-env".to_string());
        let key = |v: Value| {
            SystemOne::from_settings_with(&v, &env)
                .unwrap()
                .api_key
                .map(|k| k.expose().to_string())
        };
        assert_eq!(
            key(json!({"model": "m", "preset": "openai"})).as_deref(),
            Some("sk-env")
        );
        assert_eq!(
            key(json!({"model": "m", "preset": "openai", "api_key": "own"})).as_deref(),
            Some("own")
        );
        assert_eq!(
            key(json!({"model": "m", "preset": "openai", "endpoint": "https://proxy.example"})),
            None
        );
        assert_eq!(key(json!({"model": "m", "preset": "ollama"})), None);
    }

    #[tokio::test]
    async fn answers_are_parsed_and_the_endpoint_path_is_added() {
        let body = json!({"model":"laya:en","answers":{
            "match":{"type":"noul","noul":0.12},
            "risk":{"type":"choice","choice":"safe","confidence":0.8,"probabilities":{"safe":0.9,"danger":0.1}}},
            "usage":{"input_tokens":5,"output_tokens":0}})
        .to_string();
        let server = MockHttpServer::start(200, &body).await;
        let d = SystemOne::from_settings(&settings(&server.url())).unwrap();
        let r = d.decide(&req()).await.unwrap();
        assert_eq!(r.get("match"), Some(&Answer::YesNo { probability: 0.12 }));
        let Some(Answer::Choice {
            label,
            confidence,
            probabilities,
        }) = r.get("risk")
        else {
            panic!()
        };
        assert_eq!((label.as_str(), *confidence), ("safe", 0.8));
        assert_eq!(probabilities[1], ("danger".to_string(), 0.1));
        assert!(server.last_request().await.contains("/v1/systemone"));
    }

    #[tokio::test]
    async fn unreadable_answers_are_errors_not_decisions() {
        for body in [
            r#"{"answers":{}}"#.to_string(),
            r#"{"answers":{"match":{"noul":0.1},"risk":{"choice":"made-up"}}}"#.to_string(),
            "not json".to_string(),
        ] {
            let server = MockHttpServer::start(200, &body).await;
            let d = SystemOne::from_settings(&settings(&server.url())).unwrap();
            assert!(d.decide(&req()).await.is_err(), "{body}");
        }
        let server = MockHttpServer::start(404, r#"{"error":{"message":"model not found"}}"#).await;
        let d = SystemOne::from_settings(&settings(&server.url())).unwrap();
        let e = d.decide(&req()).await.unwrap_err();
        assert_eq!(e.kind, ProviderErrorKind::BadRequest);
        assert!(e.message.contains("model not found"));
    }

    #[test]
    fn settings_are_validated() {
        assert!(SystemOne::from_settings(&json!({"model": "m"})).is_err());
        assert!(SystemOne::from_settings(&json!({"model": "m", "preset": "nope"})).is_err());
        // the presets are the ones providers use, and the hosts that only serve decision models
        assert!(PRESETS.iter().all(|p| DECIDER_PRESETS.contains(p)));
        for p in &DECIDER_PRESETS {
            let d = SystemOne::from_settings(&json!({"model": "m", "preset": p.name})).unwrap();
            let path = preset_path(p.name).unwrap_or(DEFAULT_PATH);
            assert_eq!(d.url, format!("{}{path}", p.endpoint), "{}", p.name);
        }
        assert!(
            SystemOne::from_settings(&json!({"model": "m", "endpoint": "http://10.1.1.1:1"}))
                .is_err()
        );
        assert!(SystemOne::from_settings(
            &json!({"model": "m", "endpoint": "https://x.example", "typo": 1})
        )
        .is_err());
    }
}
