use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::connection_service::ConnectionService;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: serde_json::Value,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: serde_json::Value::String(content.into()),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: serde_json::Value::String(content.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum SSEChatEvent {
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
        #[serde(default)]
        usage: Option<ParsedUsage>,
        #[serde(default)]
        session_id: Option<String>,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(default)]
        retryable: bool,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParsedUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

pub struct HermesAgentService {
    connection: ConnectionService,
}

impl HermesAgentService {
    pub fn new(connection: ConnectionService) -> Self {
        Self { connection }
    }

    pub async fn chat_completions(
        &self,
        messages: Vec<ChatMessage>,
        model: &str,
        session_id: Option<&str>,
    ) -> Result<reqwest::Response, String> {
        let url = format!("{}/v1/chat/completions", self.connection.agent_url());

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages,
            stream: true,
        };

        let mut req_builder = self
            .connection
            .client()
            .post(&url)
            .json(&request)
            .timeout(Duration::from_secs(1800));

        if let Some(ref key) = self.connection.config.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {key}"));
        }
        if let Some(sid) = session_id {
            req_builder = req_builder.header("X-Hermes-Session-Id", sid);
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| format!("请求 Agent 失败: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Agent 返回错误 {status}: {body}"));
        }

        Ok(resp)
    }

    /// Non-streaming chat completion for title generation.
    /// Returns the generated text without any SSE/event overhead.
    pub async fn generate_title(
        &self,
        first_user_msg: &str,
        first_ai_msg: &str,
        model: &str,
    ) -> Result<String, String> {
        let url = format!("{}/v1/chat/completions", self.connection.agent_url());

        let messages = vec![
            ChatMessage::system(
                "你是一个标题生成助手。根据对话内容生成一个简短的会话标题（5-15个字）。\n\
                 要求：\n\
                 - 只返回标题文本，不要加引号、标点或任何解释\n\
                 - 标题应概括对话的核心主题\n\
                 - 使用用户的语言",
            ),
            ChatMessage::user(format!(
                "用户消息：{first_user_msg}\n\nAI 回复：{first_ai_msg}"
            )),
        ];

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages,
            stream: false,
        };

        let resp = self
            .connection
            .client()
            .post(&url)
            .json(&request)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("标题生成请求失败: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("标题生成错误 {status}: {body}"));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("标题响应解析失败: {e}"))?;

        let title = json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("新会话")
            .trim()
            .replace(['"', '\''], "")
            .chars()
            .take(50)
            .collect::<String>();

        Ok(title)
    }

    /// Non-streaming probe to get the real error when streaming returns empty.
    /// Sends the same messages as a non-streaming request and returns the raw
    /// response body so the caller can extract the error message.
    pub async fn probe_real_error(
        &self,
        messages: Vec<ChatMessage>,
        model: &str,
        session_id: Option<&str>,
    ) -> Result<String, String> {
        let url = format!("{}/v1/chat/completions", self.connection.agent_url());

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages,
            stream: false,
        };

        let resp = self
            .connection
            .client()
            .post(&url)
            .json(&request)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("探查请求失败: {e}"))?;

        let body = resp.text().await.unwrap_or_default();
        Ok(body)
    }

    /// Parse a single SSE data line into an event.
    /// `event_type` is the preceding `event:` line value (e.g. "hermes.tool.progress"), if any.
    pub fn parse_sse_line(line: &str, event_type: Option<&str>) -> Option<SSEChatEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                return Some(SSEChatEvent::Done { usage: None, session_id: None });
            }

            let v: serde_json::Value = serde_json::from_str(data).ok()?;

            // Tool progress events arrive with event: hermes.tool.progress
            if let Some("hermes.tool.progress") = event_type {
                return Some(SSEChatEvent::ToolProgress {
                    tool: v.get("tool").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                    label: v.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string(),
                    emoji: v.get("emoji").and_then(|e| e.as_str()).map(String::from),
                    tool_call_id: v.get("toolCallId").and_then(|i| i.as_str()).map(String::from),
                    status: v.get("status").and_then(|s| s.as_str()).map(String::from),
                });
            }

            // Standard chat completion chunk: has choices[0].delta.content
            if let Some(choices) = v.get("choices") {
                if let Some(first) = choices.get(0) {
                    if let Some(delta) = first.get("delta") {
                        let content = delta.get("content").and_then(|c| c.as_str()).map(String::from);
                        let reasoning = delta
                            .get("reasoning_content")
                            .or_else(|| delta.get("thinking"))
                            .and_then(|c| c.as_str())
                            .map(String::from);
                        if content.is_some() || reasoning.is_some() {
                            return Some(SSEChatEvent::Delta {
                                content: content.unwrap_or_default(),
                                reasoning_content: reasoning,
                            });
                        }
                    }
                    // finish_reason present → Done
                    if first.get("finish_reason").and_then(|f| f.as_str()).filter(|f| *f != "null").is_some() {
                        let usage = v.get("usage").map(|u| ParsedUsage {
                            prompt_tokens: u.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                            completion_tokens: u.get("completion_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                            total_tokens: u.get("total_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                        });
                        return Some(SSEChatEvent::Done { usage, session_id: None });
                    }
                }
            }

            // Unknown data — emit as empty delta (no-op)
            Some(SSEChatEvent::Delta { content: String::new(), reasoning_content: None })
        } else {
            None
        }
    }
}
