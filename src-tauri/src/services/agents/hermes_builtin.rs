use std::pin::Pin;
use std::time::Duration;

use futures::StreamExt;

use super::agent_interface::{
    AgentCapabilities, AgentInterface, ChatEvent, ChatOptions, ChatEventStream, UsageInfo,
};
use crate::services::connection_service::ConnectionService;

pub struct HermesBuiltinAgent {
    id: String,
    display_name: String,
    models: Vec<String>,
    vision_models: Vec<String>,
    reasoning_models: Vec<String>,
    connection: ConnectionService,
}

impl HermesBuiltinAgent {
    pub fn new(
        id: &str,
        display_name: &str,
        base_url: &str,
        models: Vec<String>,
        vision_models: Vec<String>,
        reasoning_models: Vec<String>,
    ) -> Self {
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            models,
            vision_models,
            reasoning_models,
            connection: ConnectionService::new_with_url(base_url),
        }
    }

    pub fn connection(&self) -> &ConnectionService {
        &self.connection
    }
}

impl AgentInterface for HermesBuiltinAgent {
    fn id(&self) -> &str { &self.id }
    fn display_name(&self) -> &str { &self.display_name }
    fn agent_type(&self) -> &str { "hermes_builtin" }
    fn models(&self) -> &[String] { &self.models }
    fn vision_models(&self) -> &[String] { &self.vision_models }
    fn reasoning_models(&self) -> &[String] { &self.reasoning_models }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            supports_streaming: true,
            supports_tools: true,
            supports_reasoning: true,
            max_context_tokens: Some(200_000),
        }
    }

    fn health_check(&self) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        let base = self.connection.agent_url();
        let stripped = base.strip_suffix("/v1").unwrap_or(base);
        let url = format!("{stripped}/health");
        let client = self.connection.client().clone();
        Box::pin(async move {
            let resp = client.get(&url).send().await.map_err(|e| format!("health check: {e}"))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("health check HTTP {}", resp.status()))
            }
        })
    }

    fn chat_stream(&self, opts: ChatOptions) -> ChatEventStream {
        let url = format!("{}/chat/completions", self.connection.agent_url());
        let client = self.connection.client().clone();
        let messages = opts.messages.clone();

        let stream = async_stream::stream! {
            let request = serde_json::json!({
                "model": opts.model,
                "messages": messages.iter().map(|m| serde_json::json!({
                    "role": m.role,
                    "content": m.content
                })).collect::<Vec<_>>(),
                "stream": true,
                "session_id": opts.session_id,
            });

            let resp = match client
                .post(&url)
                .json(&request)
                .timeout(Duration::from_secs(1800))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield ChatEvent::Error { message: format!("请求 Agent 失败: {e}"), retryable: true };
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                yield ChatEvent::Error { message: format!("Agent 返回错误 {status}: {body}"), retryable: true };
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut pending_event: Option<String> = None;
            let mut accumulated_content = String::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield ChatEvent::Error { message: format!("流读取失败: {e}"), retryable: true };
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(line_end) = buf.find('\n') {
                    let mut line = buf[..line_end].to_string();
                    buf = buf[line_end + 1..].to_string();
                    if line.ends_with('\r') { line.pop(); }

                    let trimmed = line.trim();
                    if trimmed.is_empty() { pending_event = None; continue; }
                    if trimmed.starts_with(": ") { continue; }
                    if let Some(ev) = trimmed.strip_prefix("event: ") {
                        pending_event = Some(ev.to_string());
                        continue;
                    }
                    if let Some(data) = trimmed.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            yield ChatEvent::Done { usage: None, session_id: None };
                            pending_event = None;
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                            if let Some(err) = v.get("error") {
                                let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
                                yield ChatEvent::Error { message: msg.to_string(), retryable: true };
                                pending_event = None;
                                continue;
                            }
                            // Tool progress event
                            if let Some("hermes.tool.progress") = pending_event.as_deref() {
                                yield ChatEvent::ToolProgress {
                                    tool: v.get("tool").and_then(|t| t.as_str()).unwrap_or("").into(),
                                    label: v.get("label").and_then(|l| l.as_str()).unwrap_or("").into(),
                                    emoji: v.get("emoji").and_then(|e| e.as_str()).map(String::from),
                                    tool_call_id: v.get("toolCallId").and_then(|i| i.as_str()).map(String::from),
                                    status: v.get("status").and_then(|s| s.as_str()).map(String::from),
                                };
                                pending_event = None;
                                continue;
                            }
                            // Standard delta — defend against providers that send
                            // cumulative delta.content instead of incremental tokens.
                            if let Some(choices) = v.get("choices") {
                                if let Some(first) = choices.get(0) {
                                    if let Some(delta) = first.get("delta") {
                                        let raw = delta.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                        let reasoning = delta.get("reasoning_content")
                                            .or_else(|| delta.get("thinking"))
                                            .and_then(|c| c.as_str());
                                        if raw == accumulated_content.as_str() {
                                            // Exact duplicate — skip
                                        } else if raw.starts_with(accumulated_content.as_str()) {
                                            let incremental = raw[accumulated_content.len()..].to_string();
                                            if !incremental.is_empty() || reasoning.is_some() {
                                                accumulated_content.push_str(&incremental);
                                                yield ChatEvent::Delta {
                                                    content: incremental,
                                                    reasoning_content: reasoning.map(String::from),
                                                };
                                            }
                                        } else {
                                            if !accumulated_content.is_empty() {
                                                accumulated_content.clear();
                                            }
                                            if !raw.is_empty() || reasoning.is_some() {
                                                accumulated_content.push_str(raw);
                                                yield ChatEvent::Delta {
                                                    content: raw.to_string(),
                                                    reasoning_content: reasoning.map(String::from),
                                                };
                                            }
                                        }
                                    }
                                    if first.get("finish_reason").and_then(|f| f.as_str()).filter(|f| *f != "null").is_some() {
                                        let usage = v.get("usage").map(|u| UsageInfo {
                                            prompt_tokens: u.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                                            completion_tokens: u.get("completion_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                                            total_tokens: u.get("total_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                                        });
                                        yield ChatEvent::Done { usage, session_id: None };
                                    }
                                }
                            }
                        }
                        pending_event = None;
                    }
                }
            }
        };

        Box::pin(stream)
    }
}
