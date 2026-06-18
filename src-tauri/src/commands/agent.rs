use tauri::{Emitter, State};

use crate::commands::chat::AgentRegistryState;
use crate::services::agent_manager::{AgentManager, AgentStatus};
use crate::services::agents::AgentInfo;

#[tauri::command]
pub fn agent_status(
    state: State<'_, AgentManager>,
) -> AgentStatus {
    state.status()
}

#[tauri::command]
pub fn restart_agent(
    state: State<'_, AgentManager>,
) -> Result<(), String> {
    state.restart()
}

#[tauri::command]
pub fn get_agent_logs(
    state: State<'_, AgentManager>,
    lines: Option<usize>,
) -> Vec<String> {
    state.recent_logs(lines.unwrap_or(50))
}

// ── Multi-agent registry commands ──

#[tauri::command]
pub async fn list_agents(
    state: State<'_, AgentRegistryState>,
) -> Result<Vec<AgentInfo>, String> {
    Ok(state.registry.read().await.list().await)
}

#[tauri::command]
pub async fn add_agent(
    state: State<'_, AgentRegistryState>,
    id: String,
    display_name: String,
    agent_type: String,
    base_url: String,
    api_key: Option<String>,
    models: Vec<String>,
    vision_models: Vec<String>,
    reasoning_models: Vec<String>,
) -> Result<(), String> {
    let config = crate::services::agents::AgentConfig {
        id,
        display_name,
        agent_type,
        enabled: true,
        config: crate::services::agents::AgentConnectionConfig {
            base_url, api_key, models, vision_models, reasoning_models,
            ..Default::default()
        },
        detected: None,
        detected_path: None,
        added_manually: Some(true),
    };
    state.registry.read().await.add_manual(config)
}

#[tauri::command]
pub async fn remove_agent(
    state: State<'_, AgentRegistryState>,
    id: String,
) -> Result<(), String> {
    state.registry.read().await.remove(&id)
}

#[tauri::command]
pub async fn set_agent_enabled(
    state: State<'_, AgentRegistryState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state.registry.read().await.set_enabled(&id, enabled)
}

#[tauri::command]
pub async fn set_default_agent(
    state: State<'_, AgentRegistryState>,
    id: String,
) -> Result<(), String> {
    state.registry.read().await.set_default(&id)
}

#[tauri::command]
pub async fn re_detect_agents(
    app: tauri::AppHandle,
    state: State<'_, AgentRegistryState>,
) -> Result<(), String> {
    let registry = state.registry.read().await;
    let detected = registry.background_scan().await;
    registry.merge_detected(&detected)?;
    let _ = app.emit("agents:updated", ());
    Ok(())
}

#[tauri::command]
pub async fn update_agent_config(
    state: State<'_, AgentRegistryState>,
    id: String,
    base_url: Option<String>,
    api_key: Option<String>,
    models: Option<Vec<String>>,
    vision_models: Option<Vec<String>>,
    reasoning_models: Option<Vec<String>>,
    #[allow(unused)] vision_base_url: Option<String>,
    #[allow(unused)] vision_api_key: Option<String>,
    #[allow(unused)] reasoning_base_url: Option<String>,
    #[allow(unused)] reasoning_api_key: Option<String>,
) -> Result<(), String> {
    state.registry.read().await.update_config(
        &id, base_url, api_key, models, vision_models, reasoning_models,
        vision_base_url, reasoning_base_url, vision_api_key, reasoning_api_key,
    )
}

#[tauri::command]
pub async fn fetch_ollama_models(base_url: String) -> Result<Vec<String>, String> {
    let url = format!(
        "{}/api/tags",
        base_url.trim_end_matches('/').trim_end_matches("/v1")
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let resp = client.get(&url).send().await
        .map_err(|e| format!("无法连接 Ollama ({}): {}", url, e))?;
    let body: serde_json::Value = resp.json().await
        .map_err(|e| format!("解析 Ollama 响应失败: {e}"))?;
    let models: Vec<String> = body["models"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["name"].as_str().map(String::from))
        .collect();
    if models.is_empty() {
        return Err("Ollama 返回了空的模型列表，请确认已拉取模型 (ollama pull <model>)".into());
    }
    Ok(models)
}

// ── OpenClaw lifecycle commands ──

#[tauri::command]
pub fn openclaw_configure() -> Result<String, String> {
    let launcher = crate::services::openclaw_launcher::OpenClawLauncher::new();
    match launcher.ensure_http_api_enabled() {
        Ok(true) => Ok("OpenClaw HTTP API 已启用，请重启 OpenClaw 以生效".into()),
        Ok(false) => Ok("OpenClaw HTTP API 已处于启用状态".into()),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub fn openclaw_start() -> Result<String, String> {
    let launcher = crate::services::openclaw_launcher::OpenClawLauncher::new();
    launcher.start()?;
    Ok(format!("OpenClaw 已启动 (端口 {})", launcher.port()))
}
