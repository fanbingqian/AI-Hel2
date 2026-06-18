use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::StreamExt;
use tauri::{AppHandle, Emitter, State};

use crate::services::agents::agent_interface::{AgentInterface, ChatEvent, ChatMessage, ChatOptions};
use crate::services::agents::AgentRegistry;
use crate::services::connection_service::ConnectionService;
use crate::services::hermes_agent::{HermesAgentService, ParsedUsage, SSEChatEvent};

// ── Legacy AgentState for backward compatibility with existing Hermes flow ──

pub struct AgentState {
    pub agent: HermesAgentService,
    pub cancel_flag: Arc<AtomicBool>,
}

impl AgentState {
    pub fn new(connection: ConnectionService) -> Self {
        Self {
            agent: HermesAgentService::new(connection),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Registry state wrapper for multi-agent routing.
pub struct AgentRegistryState {
    pub registry: Arc<tokio::sync::RwLock<AgentRegistry>>,
}

#[derive(Clone, serde::Serialize)]
pub struct StreamDelta {
    pub content: String,
    pub reasoning_content: Option<String>,
}

#[derive(Clone, serde::Serialize)]
pub struct StreamDone {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub session_id: Option<String>,
}

#[derive(Clone, serde::Serialize)]
pub struct StreamError {
    pub message: String,
    pub retryable: bool,
}

// ── Legacy SSE helpers (for Hermes path) ──

fn emit_sse_event(
    app: &AppHandle,
    data: &str,
    event_type: Option<&str>,
    current_session_id: &mut Option<String>,
    accumulated: &mut String,
) {
    if let Some(event) = HermesAgentService::parse_sse_line(data, event_type) {
        match event {
            SSEChatEvent::Delta { content, reasoning_content } => {
                // v0.15 Agent sends standard OpenAI incremental deltas.
                // Just accumulate for the done event and emit directly.
                if !content.is_empty() {
                    accumulated.push_str(&content);
                }
                if !content.is_empty() || reasoning_content.is_some() {
                    let _ = app.emit("chat:delta", StreamDelta { content, reasoning_content });
                }
            }
            SSEChatEvent::Done { usage, session_id } => {
                *current_session_id = session_id.or(current_session_id.clone());
                let usage_info = usage.unwrap_or(ParsedUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                });
                let _ = app.emit(
                    "chat:done",
                    StreamDone {
                        prompt_tokens: usage_info.prompt_tokens,
                        completion_tokens: usage_info.completion_tokens,
                        total_tokens: usage_info.total_tokens,
                        session_id: current_session_id.clone(),
                    },
                );
            }
            SSEChatEvent::ToolProgress { tool, label, emoji, tool_call_id, status } => {
                let _ = app.emit(
                    "chat:tool-progress",
                    serde_json::json!({
                        "tool": tool,
                        "label": label,
                        "emoji": emoji,
                        "toolCallId": tool_call_id,
                        "status": status,
                    }),
                );
            }
            SSEChatEvent::Error { message, retryable } => {
                let _ = app.emit("chat:error", StreamError { message, retryable });
            }
        }
    }
}

fn extract_error_from_response(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(err) = v.get("error") {
            if let Some(msg) = err.get("message").and_then(|m| m.as_str()) {
                return msg.to_string();
            }
        }
        if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
            if choices.is_empty() {
                return "Agent 返回空响应 (无 choices)".to_string();
            }
        }
    }
    if body.len() > 300 {
        format!("Agent 返回: {}...", &body[..300])
    } else if body.is_empty() {
        "Agent 返回空响应".to_string()
    } else {
        format!("Agent 返回: {body}")
    }
}

// ── Multi-agent chat completions ──

fn convert_messages(msgs: Vec<ChatMessage>) -> Vec<crate::services::hermes_agent::ChatMessage> {
    msgs.into_iter()
        .map(|m| crate::services::hermes_agent::ChatMessage {
            role: m.role,
            content: m.content,
        })
        .collect()
}

#[tauri::command]
pub async fn chat_completions(
    app: AppHandle,
    legacy_state: State<'_, AgentState>,
    registry_state: State<'_, AgentRegistryState>,
    messages: Vec<ChatMessage>,
    model: Option<String>,
    session_id: Option<String>,
) -> Result<(), String> {
    // Read default agent config from agents.json (AgentInfo is sanitized, no base_url)
    let (agent_type, default_model, base_url) = resolve_default_agent();
    let model = model.filter(|m| !m.is_empty()).unwrap_or(default_model);
    let hermes_msgs = convert_messages(messages);

    if agent_type == "openai_compatible" && !base_url.is_empty() {
        run_openai_compatible_chat(&app, &model, &base_url, hermes_msgs, session_id).await
    } else {
        run_hermes_chat(&app, &legacy_state, hermes_msgs, &model, session_id).await
    }
}

fn resolve_default_agent() -> (String, String, String) {
    let path = std::path::PathBuf::from(
        std::env::var("AI_HEL2_HOME").unwrap_or_else(|_| {
            std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()) + "/.ai-hel2"
        })
    ).join("agents.json");
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&raw) {
            let default_id = data["default_agent_id"].as_str().unwrap_or("hermes-builtin");
            if let Some(agents) = data["agents"].as_array() {
                for a in agents {
                    if a["id"].as_str() == Some(default_id) && a["enabled"].as_bool().unwrap_or(true) {
                        let at = a["agent_type"].as_str().unwrap_or("hermes_builtin");
                        let bu = a["config"]["base_url"].as_str().unwrap_or("");
                        let ms: Vec<_> = a["config"]["models"].as_array()
                            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                            .unwrap_or_default();
                        let m = ms.first().copied().unwrap_or("deepseek-v4-flash");
                        return (at.to_string(), m.to_string(), bu.to_string());
                    }
                }
            }
        }
    }
    ("hermes_builtin".into(), "deepseek-v4-flash".into(), String::new())
}

async fn run_openai_compatible_chat(
    app: &AppHandle,
    model: &str,
    base_url: &str,
    messages: Vec<crate::services::hermes_agent::ChatMessage>,
    _session_id: Option<String>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "messages": messages.iter().map(|m| serde_json::json!({
            "role": m.role, "content": m.content
        })).collect::<Vec<_>>(),
        "stream": true
    });
    let resp = client.post(&url).json(&body).send().await
        .map_err(|e| format!("连接本地模型失败 ({}): {}", url, e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("本地模型返回错误 HTTP {}: {}", status, text));
    }
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    let mut received = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("流读取失败: {e}"))?;
        buf.extend_from_slice(&chunk);
        while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..nl]).to_string();
            buf.drain(..=nl);
            if line.starts_with("data: ") {
                let data = line[6..].trim().to_string();
                if data == "[DONE]" { break; }
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                    if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                        received = true;
                        let _ = app.emit("chat:delta", StreamDelta {
                            content: content.to_string(),
                            reasoning_content: None,
                        });
                    }
                }
            }
        }
    }
    if received {
        let _ = app.emit("chat:done", StreamDone { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0, session_id: None });
    } else {
        let _ = app.emit("chat:error", StreamError { message: "本地模型未返回内容".into(), retryable: true });
    }
    let _ = app.emit("agent:status", serde_json::json!({ "working": false }));
    Ok(())
}

async fn run_hermes_chat(
    app: &AppHandle,
    state: &State<'_, AgentState>,
    messages: Vec<crate::services::hermes_agent::ChatMessage>,
    model: &str,
    session_id: Option<String>,
) -> Result<(), String> {
    state.cancel_flag.store(false, Ordering::SeqCst);

    // Notify pill: agent started working
    let _ = app.emit("agent:status", serde_json::json!({
        "working": true,
        "message": "正在思考…",
        "elapsed": 0,
        "steps": []
    }));

    let resp = state
        .agent
        .chat_completions(messages.clone(), model, session_id.as_deref())
        .await
        .map_err(|e| {
            let _ = app.emit("agent:status", serde_json::json!({ "working": false }));
            let _ = app.emit("chat:error", StreamError { message: e.clone(), retryable: true });
            e
        })?;

    let header_session_id = resp
        .headers()
        .get("x-hermes-session-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let mut byte_stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut pending_event: Option<String> = None;
    let mut current_session_id: Option<String> = header_session_id;
    let mut received_content = false;
    let mut done_emitted = false;
    let mut accumulated_content = String::new();

    while let Some(chunk_result) = byte_stream.next().await {
        if state.cancel_flag.load(Ordering::Relaxed) {
            let _ = app.emit("chat:error", StreamError { message: "已中止".to_string(), retryable: false });
            return Ok(());
        }

        let chunk = chunk_result.map_err(|e| format!("流读取失败: {e}"))?;
        buf.extend_from_slice(&chunk);

        // Find last complete line (ending with \n) in the byte buffer.
        // Only decode complete lines to avoid splitting multi-byte UTF-8
        // characters across chunks, which causes garbled/duplicated text.
        let last_nl = buf.iter().rposition(|&b| b == b'\n');
        let ready_bytes: Vec<u8> = if let Some(pos) = last_nl {
            buf.drain(..=pos).collect()
        } else {
            continue; // no complete line yet, wait for more bytes
        };

        let ready_text = String::from_utf8_lossy(&ready_bytes);
        for raw_line in ready_text.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() { pending_event = None; continue; }
            if trimmed.starts_with(": ") { continue; }
            if let Some(ev) = trimmed.strip_prefix("event: ") {
                pending_event = Some(ev.to_string());
                continue;
            }
            if let Some(data) = trimmed.strip_prefix("data: ") {
                if data == "[DONE]" {
                    done_emitted = true;
                } else {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(err) = parsed.get("error") {
                            let err_msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误").to_string();
                            let _ = app.emit("chat:error", StreamError { message: err_msg, retryable: true });
                        }
                    }
                }
                if trimmed.contains("\"delta\"") || trimmed.contains("\"content\"") {
                    received_content = true;
                }
                emit_sse_event(app, trimmed, pending_event.as_deref(), &mut current_session_id, &mut accumulated_content);
                pending_event = None;
            }
        }
    }

    if !received_content {
        log::warn!("Streaming returned no content, probing with non-streaming request...");
        match state.agent.probe_real_error(messages, model, session_id.as_deref()).await {
            Ok(body) => {
                let err_msg = extract_error_from_response(&body);
                let _ = app.emit("chat:error", StreamError { message: err_msg, retryable: true });
            }
            Err(probe_err) => {
                let _ = app.emit("chat:error", StreamError {
                    message: format!("Agent 返回空响应: {probe_err}"),
                    retryable: true,
                });
            }
        }
    } else if !done_emitted {
        // Stream ended with content but no [DONE] marker — emit done so the
        // frontend exits loading state.  If [DONE] was already processed we
        // skip this to avoid a duplicate chat:done event.
        let _ = app.emit(
            "chat:done",
            StreamDone {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                session_id: current_session_id.clone(),
            },
        );
    }

    // Notify pill: agent finished
    let _ = app.emit("agent:status", serde_json::json!({ "working": false }));
    Ok(())
}

#[tauri::command]
pub fn abort_chat(state: State<'_, AgentState>) {
    state.cancel_flag.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub async fn generate_title(
    state: State<'_, AgentState>,
    first_user_msg: String,
    first_ai_msg: String,
    model: String,
) -> Result<String, String> {
    state.agent.generate_title(&first_user_msg, &first_ai_msg, &model).await
}
