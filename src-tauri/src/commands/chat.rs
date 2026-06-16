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
    messages: Vec<ChatMessage>,
    model: Option<String>,
    session_id: Option<String>,
) -> Result<(), String> {
    let model = model.filter(|m| !m.is_empty()).unwrap_or_else(|| "deepseek-v4-flash".into());
    let hermes_msgs = convert_messages(messages);
    run_hermes_chat(&app, &legacy_state, hermes_msgs, &model, session_id).await
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
