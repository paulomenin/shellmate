//! The JSON the API sends back, and how HTTP errors map to error kinds.

use serde::Deserialize;
use serde_json::{json, Value};
use shellmate_api::{Message, Role, ToolChoice, ToolRequest, ToolSpec};

#[derive(Deserialize)]
pub(crate) struct WireResponse {
    pub(crate) choices: Vec<WireChoice>,
    pub(crate) usage: Option<WireUsage>,
}

#[derive(Deserialize)]
pub(crate) struct WireChoice {
    pub(crate) message: Option<WireMessage>,
    pub(crate) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct WireMessage {
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<WireToolCall>,
}

/// A call as a server writes it, whole (a reply that is not streamed).
#[derive(Deserialize)]
pub(crate) struct WireToolCall {
    pub(crate) id: Option<String>,
    pub(crate) function: WireFunction,
}

#[derive(Deserialize, Default)]
pub(crate) struct WireFunction {
    pub(crate) name: Option<String>,
    /// A JSON text, as OpenAI sends it. Some local servers send the object itself.
    pub(crate) arguments: Option<Value>,
}

/// The arguments as the JSON text a [`ToolRequest`] holds.
pub(crate) fn arguments_text(v: Option<Value>) -> String {
    match v {
        Some(Value::String(s)) => s,
        Some(Value::Null) | None => "{}".to_string(),
        Some(other) => other.to_string(),
    }
}

impl WireToolCall {
    /// A call without a name is not a call. A call without an id gets one from its position.
    pub(crate) fn into_request(self, position: usize) -> Option<ToolRequest> {
        Some(ToolRequest {
            id: self
                .id
                .filter(|i| !i.is_empty())
                .unwrap_or_else(|| format!("call_{position}")),
            name: self.function.name.filter(|n| !n.is_empty())?,
            arguments: arguments_text(self.function.arguments),
        })
    }
}

/// The messages as the API takes them. A call the assistant made is a `tool_calls` entry and its result a
/// `tool` message under the same id; everything else is a role and a text.
pub(crate) fn wire_messages(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| match m.role {
            Role::Tool => json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id.as_deref().unwrap_or_default(),
                "content": m.content,
            }),
            Role::Assistant if !m.tool_calls.is_empty() => json!({
                "role": "assistant",
                // what the API itself sends for a message that is only calls
                "content": if m.content.is_empty() { Value::Null } else { Value::String(m.content.clone()) },
                "tool_calls": m.tool_calls.iter().map(|c| json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": c.arguments },
                })).collect::<Vec<_>>(),
            }),
            role => json!({
                "role": match role {
                    Role::System => "system",
                    Role::User => "user",
                    _ => "assistant",
                },
                "content": m.content,
            }),
        })
        .collect()
}

/// The `tools` array of a request.
pub(crate) fn wire_tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": { "name": t.name, "description": t.description, "parameters": t.parameters },
            })
        })
        .collect()
}

/// `tool_choice` for a request that lists tools: only "none" has to be said, because "auto" is the default.
pub(crate) fn wire_tool_choice(choice: ToolChoice) -> Option<&'static str> {
    (choice == ToolChoice::None).then_some("none")
}

#[derive(Deserialize)]
pub(crate) struct WireUsage {
    pub(crate) prompt_tokens: Option<u32>,
    pub(crate) completion_tokens: Option<u32>,
}

/// Pull `error.message` out of an error body, bounded in size; never echo the request.
pub(crate) fn error_message(body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned());
    msg.chars().take(300).collect()
}
