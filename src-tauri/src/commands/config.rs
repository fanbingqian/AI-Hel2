use std::fs;
use std::sync::Mutex;

use tauri::State;

use crate::services::config_service::ConfigService;

pub struct ConfigState {
    pub service: Mutex<ConfigService>,
}

#[derive(Clone, serde::Serialize)]
pub struct CronJobInfo {
    pub id: String,
    pub name: String,
    pub cron: String,
    pub operation: String,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub last_error: Option<String>,
}

#[tauri::command]
pub async fn get_config(
    state: State<'_, ConfigState>,
) -> Result<crate::services::config_service::HermesConfig, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.read_config()
}

#[tauri::command]
pub async fn save_config(
    state: State<'_, ConfigState>,
    updates: serde_json::Value,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.write_config(&updates)
}

#[tauri::command]
pub async fn update_api_key(
    state: State<'_, ConfigState>,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let env_path = service.hermes_home().join(".env");
    let content = fs::read_to_string(&env_path).unwrap_or_default();
    let env_key = format!("{}_API_KEY", provider.to_uppercase());
    let lines: Vec<&str> = content.lines()
        .filter(|l| !l.trim_start().starts_with(&format!("{}=", env_key)))
        .collect();
    let mut new_content = lines.join("\n");
    if !new_content.ends_with('\n') { new_content.push('\n'); }
    new_content.push_str(&format!("{}={}\n", env_key, api_key.trim()));
    let _ = fs::create_dir_all(service.hermes_home());
    fs::write(&env_path, &new_content).map_err(|e| format!("写入 .env 失败: {e}"))?;

    // Also seed the agent's HERMES_HOME/.env so the Python Agent can find the API key
    let agent_home = service.hermes_home().join("hermes");
    let _ = fs::create_dir_all(&agent_home);
    let agent_env = agent_home.join(".env");
    fs::write(&agent_env, &new_content).map_err(|e| format!("写入 Agent .env 失败: {e}"))?;
    log::info!("API key written to {} and {}", env_path.display(), agent_env.display());

    Ok(())
}

#[tauri::command]
pub async fn verify_api_key(
    base_url: String,
    api_key: String,
    model: String,
) -> Result<String, String> {
    // Auto-append /v1 if missing (Hermes format URLs omit it)
    let base = base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else {
        format!("{}/v1/chat/completions", base)
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 5,
    });
    let mut req = client.post(&url)
        .header("Content-Type", "application/json")
        .json(&body);
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    match req.send().await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(format!("连接成功 (HTTP {})", status.as_u16()))
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                Err(format!("API Key 无效 (HTTP {})", status.as_u16()))
            } else {
                let body = resp.text().await.unwrap_or_default();
                let preview: String = body.chars().take(300).collect();
                Err(format!("服务器返回 HTTP {}: {}", status.as_u16(), preview))
            }
        }
        Err(e) => {
            if e.is_timeout() {
                Err("连接超时（10秒）— 请确认服务是否启动".into())
            } else if e.is_connect() {
                Err(format!("无法连接到 {} — 请确认服务已启动且 URL 正确", url))
            } else {
                Err(format!("连接失败: {}", e))
            }
        }
    }
}

#[tauri::command]
pub async fn save_user_profile(
    state: State<'_, ConfigState>,
    name: String,
    email: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let user_json = serde_json::json!({
        "user": { "name": &name, "email": &email }
    });
    service.write_config(&user_json)?;

    // Also update the stored user in users.json
    let users_path = service.hermes_home().join("users.json");
    if users_path.exists() {
        let content = fs::read_to_string(&users_path)
            .map_err(|e| format!("读取用户数据失败: {e}"))?;
        let mut users: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("解析用户数据失败: {e}"))?;
        if let Some(obj) = users.as_object_mut() {
            if let Some(user) = obj.get_mut(&name) {
                if let Some(u) = user.as_object_mut() {
                    u.insert("email".into(), serde_json::Value::String(email));
                }
            }
        }
        let updated = serde_json::to_string_pretty(&users)
            .map_err(|e| format!("序列化失败: {e}"))?;
        let tmp = users_path.with_extension("tmp");
        fs::write(&tmp, &updated).map_err(|e| format!("写入失败: {e}"))?;
        fs::rename(&tmp, &users_path).map_err(|e| format!("保存失败: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn export_data(
    state: State<'_, ConfigState>,
) -> Result<String, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let home = service.hermes_home();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
    let zip_name = format!("AI-Hel2-backup-{}.zip", now);
    let zip_path = home.join(&zip_name);

    let files = ["config.yaml", ".env", "state.db", "knowledge_cache.db", "sessions.json", "users.json"];
    let mut existing: Vec<std::path::PathBuf> = files.iter()
        .map(|f| home.join(f))
        .filter(|p| p.exists())
        .collect();

    // Include wiki directory if it exists
    let wiki_dir = home.join("wiki");
    if wiki_dir.exists() {
        existing.push(wiki_dir);
    }

    if existing.is_empty() {
        return Err("无数据可导出".into());
    }

    let file = fs::File::create(&zip_path).map_err(|e| format!("创建ZIP失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    for path in &existing {
        if path.is_dir() {
            let prefix = home.to_string_lossy().replace('\\', "/");
            let walk_dir = path.clone();
            for entry in walkdir::WalkDir::new(&walk_dir) {
                let entry = entry.map_err(|e| format!("遍历目录失败: {e}"))?;
                if entry.file_type().is_file() {
                    let rel = entry.path().to_string_lossy().replace('\\', "/");
                    let rel = rel.strip_prefix(&prefix).unwrap_or(&rel).trim_start_matches('/');
                    zip.start_file(rel, options).map_err(|e| format!("ZIP写入失败: {e}"))?;
                    let data = fs::read(entry.path()).map_err(|e| format!("读取文件失败: {e}"))?;
                    std::io::Write::write_all(&mut zip, &data).map_err(|e| format!("ZIP写入失败: {e}"))?;
                }
            }
        } else {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            zip.start_file(name.as_ref(), options).map_err(|e| format!("ZIP写入失败: {e}"))?;
            let data = fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
            std::io::Write::write_all(&mut zip, &data).map_err(|e| format!("ZIP写入失败: {e}"))?;
        }
    }

    zip.finish().map_err(|e| format!("ZIP完成失败: {e}"))?;
    Ok(zip_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn import_data(
    state: State<'_, ConfigState>,
    zip_path: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let home = service.hermes_home().to_path_buf();

    let file = fs::File::open(&zip_path)
        .map_err(|e| format!("打开ZIP文件失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("解析ZIP失败: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("读取ZIP条目失败: {e}"))?;
        let name = entry.name();
        // Security: block path traversal
        if name.contains("..") || name.starts_with('/') {
            continue;
        }
        let dest = home.join(name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        if entry.is_file() {
            let mut out = fs::File::create(&dest).map_err(|e| format!("创建文件失败: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("解压失败: {e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_env_keys(
    state: State<'_, ConfigState>,
) -> Result<Vec<(String, bool)>, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let env = service.read_env();
    // Return known API key names with their configured status
    let known_keys = [
        "OPENROUTER_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY",
        "GOOGLE_API_KEY", "XAI_API_KEY", "GROQ_API_KEY", "DEEPSEEK_API_KEY",
        "TOGETHER_API_KEY", "FIREWORKS_API_KEY", "CEREBRAS_API_KEY",
        "MISTRAL_API_KEY", "PERPLEXITY_API_KEY", "GLM_API_KEY",
        "KIMI_API_KEY", "MINIMAX_API_KEY", "HF_TOKEN",
    ];
    Ok(known_keys.iter().map(|k| {
        (k.to_string(), env.contains_key(*k) && !env.get(*k).unwrap_or(&String::new()).is_empty())
    }).collect())
}

// ── Nexus LLM Config ──

#[tauri::command]
pub async fn get_nexus_config(
    state: State<'_, ConfigState>,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.read_nexus_config()
}

#[tauri::command]
pub async fn save_nexus_config(
    state: State<'_, ConfigState>,
    config: serde_json::Value,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.write_nexus_config(&config)
}

#[tauri::command]
pub async fn copy_agent_config_for_nexus(
    state: State<'_, ConfigState>,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let hermes_home = service.hermes_home().to_path_buf();
    drop(service);

    // Read the default agent from agents.json (same logic as agent_manager + chat)
    let agents_path = hermes_home.join("agents.json");
    let mut provider = String::new();
    let mut model = String::new();
    let mut api_key = String::new();
    let mut base_url = String::new();

    if let Ok(raw) = std::fs::read_to_string(&agents_path) {
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&raw) {
            let default_id = data["default_agent_id"].as_str().unwrap_or("hermes-builtin");
            if let Some(agents) = data["agents"].as_array() {
                for a in agents {
                    if a["id"].as_str() == Some(default_id) && a["enabled"].as_bool().unwrap_or(true) {
                        let at = a["agent_type"].as_str().unwrap_or("hermes_builtin");
                        model = a["config"]["models"].as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|v| v.as_str())
                            .unwrap_or("deepseek-v4-flash")
                            .to_string();
                        base_url = a["config"]["base_url"].as_str().unwrap_or("").to_string();
                        api_key = a["config"]["api_key"].as_str().unwrap_or("").to_string();
                        // Derive provider name from URL as best-effort label
                        provider = if base_url.contains("deepseek") { "deepseek".to_string() }
                            else if base_url.contains("openai") { "openai".to_string() }
                            else if base_url.contains("anthropic") { "anthropic".to_string() }
                            else if base_url.contains("groq") { "groq".to_string() }
                            else if base_url.contains("openrouter") { "openrouter".to_string() }
                            else if base_url.contains("localhost") || base_url.contains("127.0.0.1") { "local".to_string() }
                            else { at.to_string() };
                        break;
                    }
                }
            }
        }
    }

    // Fallback: read from .env if agents.json didn't provide api_key
    if api_key.is_empty() {
        let svc2 = crate::services::config_service::ConfigService::new();
        let env = svc2.read_env();
        for (key, val) in &env {
            if key.ends_with("_API_KEY") && !val.is_empty() && provider.is_empty() {
                provider = key.trim_end_matches("_API_KEY").to_lowercase();
                api_key = val.clone();
                break;
            }
        }
    }

    let is_local = base_url.contains("localhost") || base_url.contains("127.0.0.1") || provider == "local" || provider == "ollama";
    Ok(serde_json::json!({
        "llm_provider": provider,
        "llm_model": model,
        "llm_api_key": if is_local { "local" } else { &api_key },
        "llm_base_url": if is_local { "" } else { &base_url },
        "llm_mode": if is_local { "follow_agent" } else { "custom" },
    }))
}

#[tauri::command]
pub async fn check_nexus_llm_connection(
    state: State<'_, ConfigState>,
    config: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    // Extract all config data first, drop lock before async HTTP call
    let (api_key, base_url, model) = {
        let service = state.service.lock().map_err(|e| e.to_string())?;

        let nexus_config = if let Some(c) = config {
            c
        } else {
            service.read_nexus_config()?
        };

        let llm_mode = nexus_config.get("llm_mode").and_then(|v| v.as_str()).unwrap_or("follow_agent");

        if llm_mode == "custom" {
            (
                nexus_config.get("llm_api_key").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("").to_string(),
                nexus_config.get("llm_base_url").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("https://api.anthropic.com/v1").to_string(),
                nexus_config.get("llm_model").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("claude-sonnet-4-6").to_string(),
            )
        } else {
            let model_cfg = service.read_config().unwrap_or_default();
            let provider = model_cfg.model.provider;

            // When using Hermes builtin agent, probe through Hermes directly
            if provider == "hermes-builtin" {
                ("".to_string(), "http://127.0.0.1:18642/v1".to_string(), model_cfg.model.name)
            } else {
                let env = service.read_env();
                let key = env.get(&format!("{}_API_KEY", provider.to_uppercase()))
                    .cloned()
                    .or(model_cfg.model.api_key)
                    .unwrap_or_default();
                let url = match provider.as_str() {
                    "openai" => "https://api.openai.com/v1".to_string(),
                    "deepseek" => "https://api.deepseek.com/v1".to_string(),
                    _ => "https://api.anthropic.com/v1".to_string(),
                };
                let model_name = if model_cfg.model.name.is_empty() {
                    match provider.as_str() {
                        "deepseek" => "deepseek-v4-pro".to_string(),
                        "openai" => "gpt-4o".to_string(),
                        _ => "claude-sonnet-4-6".to_string(),
                    }
                } else {
                    model_cfg.model.name
                };
                (key, url, model_name)
            }
            }
    }; // lock dropped here

    if api_key.is_empty() && base_url != "http://127.0.0.1:18642/v1" {
        return Ok(serde_json::json!({
            "ok": false, "model": model, "latency_ms": 0,
            "error": "No API key configured."
        }));
    }

    let start = std::time::Instant::now();
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "Hi"}],
    });

    let url = if base_url.contains("anthropic") {
        format!("{}/messages", base_url.trim_end_matches('/'))
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };

    let mut req_builder = client.post(&url).json(&body).timeout(std::time::Duration::from_secs(15));
    if base_url.contains("anthropic") {
        req_builder = req_builder
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01");
    } else if !api_key.is_empty() {
        req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
    }

    match req_builder.send().await {
        Ok(resp) => {
            let latency = start.elapsed().as_millis() as u64;
            let status = resp.status();
            if status.is_success() {
                Ok(serde_json::json!({"ok": true, "model": model, "latency_ms": latency, "error": null}))
            } else {
                let body_text = resp.text().await.unwrap_or_default();
                Ok(serde_json::json!({
                    "ok": false, "model": model, "latency_ms": latency,
                    "error": format!("HTTP {}: {}", status.as_u16(), &body_text[..body_text.len().min(200)])
                }))
            }
        }
        Err(e) => {
            Ok(serde_json::json!({
                "ok": false, "model": model, "latency_ms": start.elapsed().as_millis() as u64,
                "error": format!("Connection failed: {e}")
            }))
        }
    }
}
