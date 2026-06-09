use serde::{Deserialize, Serialize};
use std::pin::Pin;
use futures::Stream;

/// Streaming chat event emitted by any Agent implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChatEvent {
    #[serde(rename = "delta")]
    Delta {
        content: String,
        reasoning_content: Option<String>,
    },
    #[serde(rename = "tool_progress")]
    ToolProgress {
        tool: String,
        label: String,
        emoji: Option<String>,
        tool_call_id: Option<String>,
        status: Option<String>,
    },
    #[serde(rename = "done")]
    Done {
        usage: Option<UsageInfo>,
        session_id: Option<String>,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        retryable: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub model: String,
    pub session_id: Option<String>,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: serde_json::Value,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: serde_json::Value::String(content.into()) }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: serde_json::Value::String(content.into()) }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentCapabilities {
    pub supports_streaming: bool,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
    pub max_context_tokens: Option<u32>,
}

/// Type-erased stream of ChatEvents — the common return type for all agent chat_stream() calls.
pub type ChatEventStream = Pin<Box<dyn Stream<Item = ChatEvent> + Send>>;

/// Every Agent (Hermes builtin, OpenClaw, DeepSeek, OpenAI-compatible, etc.)
/// implements this trait. The registry holds Box<dyn AgentInterface>.
pub trait AgentInterface: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn agent_type(&self) -> &str;
    fn chat_stream(&self, opts: ChatOptions) -> ChatEventStream;
    fn health_check(&self) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>>;
    fn capabilities(&self) -> AgentCapabilities;
    fn models(&self) -> &[String];
    fn vision_models(&self) -> &[String] { &[] }
    fn reasoning_models(&self) -> &[String] { &[] }
}
