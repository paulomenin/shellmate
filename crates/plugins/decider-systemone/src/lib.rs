//! Decider plugin for decision models over the System One wire format: `POST {endpoint}/v1/systemone`
//! with a `state` and typed `questions`, answered with labels and probabilities. Served by Ollama
//! (decision models such as `laya`), by Ollaya, and by hosted APIs that use the same format.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use shellmate_api::{
    Answer, Decider, DecisionRequest, DecisionResponse, Error, ProviderError, ProviderErrorKind,
    ProviderInfo, Question, Registry, Secret,
};
use std::sync::Arc;
use std::time::Duration;

pub const PLUGIN_ID: &str = "systemone";

const PRESETS: &[(&str, &str)] = &[("ollama", "http://localhost:11434")];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub preset: Option<String>,
    /// Base URL without `/v1/systemone`.
    pub endpoint: Option<String>,
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
    model: String,
    api_key: Option<Secret>,
}

impl SystemOne {
    pub fn from_settings(value: &Value) -> Result<Self, Error> {
        let s: Settings = serde_json::from_value(value.clone())
            .map_err(|e| Error::new("config.invalid", format!("decider settings: {e}")))?;
        let endpoint = match (&s.endpoint, &s.preset) {
            (Some(e), _) => e.clone(),
            (None, Some(p)) => PRESETS
                .iter()
                .find(|(n, _)| n == p)
                .map(|(_, e)| e.to_string())
                .ok_or_else(|| {
                    let names: Vec<_> = PRESETS.iter().map(|(n, _)| *n).collect();
                    Error::new("config.invalid", format!("unknown preset `{p}`"))
                        .with_hint(format!("presets: {}", names.join(", ")))
                })?,
            (None, None) => {
                return Err(
                    Error::new("config.invalid", "decider needs `preset` or `endpoint`")
                        .with_hint("e.g. preset = \"ollama\""),
                )
            }
        };
        shellmate_api::check_endpoint(&endpoint)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(s.timeout_secs.unwrap_or(10)))
            .user_agent(concat!("shellmate/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::new("provider.error", format!("http client: {e}")))?;
        Ok(Self {
            client,
            url: format!("{}/v1/systemone", endpoint.trim_end_matches('/')),
            model: s.model,
            api_key: s.api_key,
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
        assert!(SystemOne::from_settings(&json!({"model": "m", "preset": "ollama"})).is_ok());
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
