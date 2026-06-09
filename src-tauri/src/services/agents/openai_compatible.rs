use std::pin::Pin;
use std::time::Duration;

use futures::StreamExt;
use serde_json::Value;

use super::agent_interface::{
    AgentCapabilities, AgentInterface, ChatEvent, ChatOptions, ChatEventStream, UsageInfo,
};
use super::nexus_tools;

pub struct OpenAICompatibleAgent {
    id: String,
    display_name: String,
    agent_type: String,
    models: Vec<String>,
    vision_models: Vec<String>,
    reasoning_models: Vec<String>,
    base_url: String,
    api_key: Option<String>,
    client: reqwest::Client,
    /// Maximum tool-call round-trips to prevent infinite loops.
    max_tool_rounds: usize,
}

impl OpenAICompatibleAgent {
    pub fn new(
        id: &str,
        display_name: &str,
        agent_type: &str,
        base_url: &str,
        api_key: Option<String>,
        models: Vec<String>,
        vision_models: Vec<String>,
        reasoning_models: Vec<String>,
    ) -> Self {
        let builder = reqwest::Client::builder().timeout(Duration::from_secs(600));
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            agent_type: agent_type.to_string(),
            models,
            vision_models,
            reasoning_models,
            base_url: base_url.to_string(),
            api_key: api_key.clone(),
            client: builder.build().unwrap_or_default(),
            max_tool_rounds: 5,
        }
    }

    fn build_request(&self, opts: &ChatOptions, tools: bool) -> Value {
        let mut req = serde_json::json!({
            "model": opts.model,
            "messages": opts.messages.iter().map(|m| serde_json::json!({
                "role": m.role,
                "content": m.content,
            })).collect::<Vec<_>>(),
            "stream": true,
        });
        if tools {
            req["tools"] = serde_json::json!(nexus_tools::nexus_tool_definitions());
            req["tool_choice"] = serde_json::json!("auto");
        }
        req
    }

    async fn send_streaming(&self, body: &Value) -> Result<ChatEventStream, String> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut req = self.client.post(&url).json(body);
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {body_text}"));
        }

        let byte_stream = resp.bytes_stream();
        let stream = async_stream::stream! {
            let mut buf = String::new();
            let mut tool_calls_buf: Vec<Value> = Vec::new();
            let mut accumulated = String::new();
            tokio::pin!(byte_stream);

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield ChatEvent::Error { message: format!("流错误: {e}"), retryable: true };
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(line_end) = buf.find('\n') {
                    let mut line = buf[..line_end].to_string();
                    buf = buf[line_end + 1..].to_string();
                    if line.ends_with('\r') { line.pop(); }
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    if trimmed.starts_with(": ") { continue; }
                    if let Some(data) = trimmed.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            // Flush accumulated tool calls via a Done event
                            // The caller (chat_stream) handles tool_calls post-processing
                            yield ChatEvent::Done { usage: None, session_id: None };
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<Value>(data) {
                            if let Some(choices) = v.get("choices") {
                                if let Some(first) = choices.get(0) {
                                    if let Some(delta) = first.get("delta") {
                                        // Collect tool calls
                                        if let Some(tc_delta) = delta.get("tool_calls") {
                                            if let Some(arr) = tc_delta.as_array() {
                                                for tc in arr {
                                                    let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                                    while tool_calls_buf.len() <= idx {
                                                        tool_calls_buf.push(serde_json::json!({
                                                            "id": "", "function": {"name": "", "arguments": ""}
                                                        }));
                                                    }
                                                    if let Some(tc_id) = tc.get("id").and_then(|i| i.as_str()) {
                                                        tool_calls_buf[idx]["id"] = Value::String(tc_id.to_string());
                                                    }
                                                    if let Some(func) = tc.get("function") {
                                                        if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                                            if !name.is_empty() {
                                                                tool_calls_buf[idx]["function"]["name"] = Value::String(name.to_string());
                                                            }
                                                        }
                                                        if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                                            let cur = tool_calls_buf[idx]["function"]["arguments"].as_str().unwrap_or("");
                                                            tool_calls_buf[idx]["function"]["arguments"] = Value::String(format!("{cur}{args}"));
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        let raw = delta.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                        let reasoning = delta.get("reasoning_content")
                                            .or_else(|| delta.get("thinking"))
                                            .and_then(|c| c.as_str());
                                        if raw == accumulated.as_str() {
                                            // Exact duplicate — skip
                                        } else if raw.starts_with(accumulated.as_str()) {
                                            let incremental = raw[accumulated.len()..].to_string();
                                            if !incremental.is_empty() || reasoning.is_some() {
                                                accumulated.push_str(&incremental);
                                                yield ChatEvent::Delta {
                                                    content: incremental,
                                                    reasoning_content: reasoning.map(String::from),
                                                };
                                            }
                                        } else {
                                            if !accumulated.is_empty() {
                                                accumulated.clear();
                                            }
                                            if !raw.is_empty() || reasoning.is_some() {
                                                accumulated.push_str(raw);
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
                                        let finish_reason = first.get("finish_reason").and_then(|f| f.as_str()).unwrap_or("");
                                        if finish_reason == "tool_calls" {
                                            yield ChatEvent::Done {
                                                usage,
                                                session_id: None,
                                            };
                                        } else {
                                            yield ChatEvent::Done { usage, session_id: None };
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

impl AgentInterface for OpenAICompatibleAgent {
    fn id(&self) -> &str { &self.id }
    fn display_name(&self) -> &str { &self.display_name }
    fn agent_type(&self) -> &str { &self.agent_type }
    fn models(&self) -> &[String] { &self.models }
    fn vision_models(&self) -> &[String] { &self.vision_models }
    fn reasoning_models(&self) -> &[String] { &self.reasoning_models }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            supports_streaming: true,
            supports_tools: true,
            supports_reasoning: false,
            max_context_tokens: None,
        }
    }

    fn health_check(&self) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        let url = format!("{}/models", self.base_url);
        let mut req = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        Box::pin(async move {
            let resp = req.send().await.map_err(|e| format!("health check: {e}"))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("health check HTTP {}", resp.status()))
            }
        })
    }

    fn chat_stream(&self, opts: ChatOptions) -> ChatEventStream {
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let client = self.client.clone();
        let max_rounds = self.max_tool_rounds;

        let stream = async_stream::stream! {
            let mut messages: Vec<Value> = opts.messages.iter().map(|m| {
                serde_json::json!({"role": m.role, "content": m.content})
            }).collect();
            let mut round = 0;

            loop {
                round += 1;
                let body = serde_json::json!({
                    "model": opts.model,
                    "messages": &messages,
                    "stream": true,
                    "tools": nexus_tools::nexus_tool_definitions(),
                    "tool_choice": "auto",
                });

                let url = format!("{base_url}/chat/completions");
                log::info!("[OpenAICompat] POST {}", url);
                let mut req = client.post(&url).json(&body);
                if let Some(ref key) = api_key {
                    req = req.header("Authorization", format!("Bearer {key}"));
                }

                let resp = match req.send().await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("[OpenAICompat] request failed url={url} error={e}");
                        yield ChatEvent::Error { message: format!("请求失败: {e}"), retryable: true };
                        return;
                    }
                };

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body_text = resp.text().await.unwrap_or_default();
                    yield ChatEvent::Error { message: format!("HTTP {status}: {body_text}"), retryable: true };
                    return;
                }

                let mut byte_stream = resp.bytes_stream();
                let mut buf = String::new();
                let mut finish_reason: Option<String> = None;
                let mut usage_info: Option<UsageInfo> = None;
                let mut tool_calls_buf: Vec<Value> = Vec::new();
                let mut accumulated = String::new();

                while let Some(chunk_result) = byte_stream.next().await {
                    let chunk = match chunk_result {
                        Ok(c) => c,
                        Err(e) => {
                            yield ChatEvent::Error { message: format!("流错误: {e}"), retryable: true };
                            return;
                        }
                    };
                    buf.push_str(&String::from_utf8_lossy(&chunk));

                    while let Some(line_end) = buf.find('\n') {
                        let mut line = buf[..line_end].to_string();
                        buf = buf[line_end + 1..].to_string();
                        if line.ends_with('\r') { line.pop(); }
                        let trimmed = line.trim();
                        if trimmed.is_empty() { continue; }
                        if trimmed.starts_with(": ") { continue; }
                        if let Some(data) = trimmed.strip_prefix("data: ") {
                            if data == "[DONE]" { continue; }
                            if let Ok(v) = serde_json::from_str::<Value>(data) {
                                if let Some(choices) = v.get("choices") {
                                    if let Some(first) = choices.get(0) {
                                        if let Some(delta) = first.get("delta") {
                                            if let Some(tc_delta) = delta.get("tool_calls") {
                                                if let Some(arr) = tc_delta.as_array() {
                                                    for tc in arr {
                                                        let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                                        while tool_calls_buf.len() <= idx {
                                                            tool_calls_buf.push(serde_json::json!({
                                                                "id": "", "type": "function",
                                                                "function": {"name": "", "arguments": ""}
                                                            }));
                                                        }
                                                        if let Some(tc_id) = tc.get("id").and_then(|i| i.as_str()) {
                                                            tool_calls_buf[idx]["id"] = Value::String(tc_id.to_string());
                                                        }
                                                        if let Some(func) = tc.get("function") {
                                                            if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                                                if !name.is_empty() {
                                                                    tool_calls_buf[idx]["function"]["name"] = Value::String(name.to_string());
                                                                }
                                                            }
                                                            if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                                                let cur = tool_calls_buf[idx]["function"]["arguments"].as_str().unwrap_or("");
                                                                tool_calls_buf[idx]["function"]["arguments"] = Value::String(format!("{cur}{args}"));
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                            let raw = delta.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                            let reasoning = delta.get("reasoning_content")
                                                .or_else(|| delta.get("thinking"))
                                                .and_then(|c| c.as_str());
                                            if raw == accumulated.as_str() {
                                                // Exact duplicate — skip
                                            } else if raw.starts_with(accumulated.as_str()) {
                                                let incremental = raw[accumulated.len()..].to_string();
                                                if !incremental.is_empty() || reasoning.is_some() {
                                                    accumulated.push_str(&incremental);
                                                    yield ChatEvent::Delta {
                                                        content: incremental,
                                                        reasoning_content: reasoning.map(String::from),
                                                    };
                                                }
                                            } else {
                                                if !accumulated.is_empty() {
                                                    accumulated.clear();
                                                }
                                                if !raw.is_empty() || reasoning.is_some() {
                                                    accumulated.push_str(raw);
                                                    yield ChatEvent::Delta {
                                                        content: raw.to_string(),
                                                        reasoning_content: reasoning.map(String::from),
                                                    };
                                                }
                                            }
                                        }
                                        if let Some(fr) = first.get("finish_reason").and_then(|f| f.as_str()) {
                                            if fr != "null" {
                                                finish_reason = Some(fr.to_string());
                                                usage_info = v.get("usage").map(|u| UsageInfo {
                                                    prompt_tokens: u.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                                                    completion_tokens: u.get("completion_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                                                    total_tokens: u.get("total_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Handle tool_calls if finish_reason == "tool_calls" and we have tool calls
                if finish_reason.as_deref() == Some("tool_calls") && !tool_calls_buf.is_empty() && round <= max_rounds {
                    // Emit tool progress events
                    for tc in &tool_calls_buf {
                        let name = tc["function"]["name"].as_str().unwrap_or("");
                        yield ChatEvent::ToolProgress {
                            tool: name.to_string(),
                            label: format!("调用 {name}"),
                            emoji: Some("🔧".into()),
                            tool_call_id: tc["id"].as_str().map(String::from),
                            status: Some("running".into()),
                        };
                    }

                    // Add assistant message with tool_calls
                    let assistant_tool_calls: Vec<Value> = tool_calls_buf.iter().map(|tc| {
                        serde_json::json!({
                            "id": tc["id"],
                            "type": "function",
                            "function": {
                                "name": tc["function"]["name"],
                                "arguments": tc["function"]["arguments"]
                            }
                        })
                    }).collect();
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": null,
                        "tool_calls": assistant_tool_calls,
                    }));

                    // Execute each tool call and emit results
                    for tc in &tool_calls_buf {
                        let name = tc["function"]["name"].as_str().unwrap_or("");
                        let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                        let args: Value = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));

                        match nexus_tools::execute_nexus_tool(name, &args).await {
                            Ok(result) => {
                                yield ChatEvent::ToolProgress {
                                    tool: name.to_string(),
                                    label: format!("{name} 完成"),
                                    emoji: Some("✅".into()),
                                    tool_call_id: tc["id"].as_str().map(String::from),
                                    status: Some("completed".into()),
                                };
                                messages.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tc["id"],
                                    "content": result,
                                }));
                            }
                            Err(e) => {
                                yield ChatEvent::ToolProgress {
                                    tool: name.to_string(),
                                    label: format!("{name} 失败"),
                                    emoji: Some("❌".into()),
                                    tool_call_id: tc["id"].as_str().map(String::from),
                                    status: Some("error".into()),
                                };
                                messages.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tc["id"],
                                    "content": format!("错误: {e}"),
                                }));
                            }
                        }
                    }

                    // Continue loop to get next response
                    continue;
                }

                // No tool calls or max rounds reached — done
                yield ChatEvent::Done { usage: usage_info, session_id: None };
                return;
            }
        };

        Box::pin(stream)
    }
}
