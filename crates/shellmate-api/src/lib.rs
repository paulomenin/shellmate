//! The shellmate plugin API.
//!
//! Plugins depend only on this crate. It is runtime-agnostic: futures appear in
//! signatures (via `async_trait`), but no tokio types do.

mod decider;
mod endpoint;
mod error;
mod middleware;
mod provider;
mod registry;
mod secret;
mod store;
mod stream;
mod tool;
mod types;

pub use decider::{Answer, Decider, DecisionRequest, DecisionResponse, Question};
pub use endpoint::check_endpoint;
pub use error::{Error, Result};
pub use middleware::{Middleware, Next, Terminal};
pub use provider::{Provider, ProviderError, ProviderErrorKind, ProviderInfo};
pub use registry::{
    DeciderCtor, LlmMiddlewareCtor, MiddlewareRole, ProviderBuilder, ProviderCtor, Registry,
    StoreCtor,
};
pub use secret::Secret;
pub use store::{busy, ConversationInfo, ConversationLock, Current, Store};
pub use stream::{
    collect, stream_from, stream_of_response, ChatStream, EventStream, StreamEvent, VecStream,
};
pub use tool::{
    ApprovalKind, ApprovalMode, ApprovalNeed, ApprovalSetting, Assessment, CommandRisk, FileTarget,
    FileText, Granted, Mark, Permission, Prepared, ProcessEnv, ProcessOutput, ProcessSpec, Refusal,
    Risk, ToBuffer, Tone, Tool, ToolAction, ToolCall, ToolCtor, ToolCtx, ToolInfo, ToolNotice,
    ToolOutput, ToolServices, ToolSettings, TOOL_LABEL,
};
pub use types::{ChatRequest, ChatResponse, Message, Role, Usage};
