use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// The result of a tool call the assistant made natively (see [`Message::tool_calls`]).
    Tool,
}

/// One call a model made to a tool, in the provider's own protocol (native tool calling).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRequest {
    /// The provider's id for the call. The result is sent back under the same id.
    pub id: String,
    pub name: String,
    /// The arguments as a JSON text, as the model wrote them. It may not be valid JSON; the core checks.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Calls the assistant made natively in this message. Empty for every other message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolRequest>,
    /// For a [`Role::Tool`] message: the id of the call it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    /// A message that is only text, for any role.
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }
    /// An assistant message that made native tool calls (`content` is what it said before them, maybe nothing).
    pub fn assistant_calling(content: impl Into<String>, calls: Vec<ToolRequest>) -> Self {
        Self {
            tool_calls: calls,
            ..Self::new(Role::Assistant, content)
        }
    }
    /// The result of the call with id `call_id`.
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(call_id.into()),
            ..Self::new(Role::Tool, content)
        }
    }
}

/// A tool as a model is told about it when the provider takes tools natively.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// A JSON Schema object describing the arguments.
    pub parameters: serde_json::Value,
}

/// Whether the model may call tools in this request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoice {
    /// The model decides.
    #[default]
    Auto,
    /// The model must answer in text (the tools stay listed, because some providers need them for a
    /// conversation that already holds tool calls).
    None,
}

/// Provider-neutral chat request. Everything that crosses a plugin boundary is serializable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub request_id: String,
    pub messages: Vec<Message>,
    /// The tools the model may call natively. Empty: tools, if any, are offered in the prompt text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    #[serde(default, skip_serializing_if = "is_auto")]
    pub tool_choice: ToolChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Key a redactor should derive placeholders from, so the same value gets the same placeholder in
    /// every turn of a conversation. `None` means a fresh random key per request. Never serialized.
    #[serde(skip)]
    pub redaction_key: Option<[u8; 16]>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub text: String,
    /// Native tool calls the model made, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolRequest>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

fn is_auto(c: &ToolChoice) -> bool {
    *c == ToolChoice::Auto
}
