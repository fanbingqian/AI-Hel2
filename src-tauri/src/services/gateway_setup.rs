use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// In-memory cache of active QR sessions (platform → session_id mapping).
/// The actual session state lives in temp files managed by the Python helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrSessionInfo {
    pub session_id: String,
    pub platform: String,
    pub qr_url: String,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrPollResult {
    pub status: String, // "waiting" | "scanned" | "refreshed" | "success" | "failed" | "expired"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub key: String,
    pub label: String,
    pub emoji: Option<String>,
    pub description: String,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfigStatus {
    pub key: String,
    pub label: String,
    pub enabled: bool,
    pub configured: bool,
    pub has_qr: bool,
}

pub struct GatewaySetupService {
    script_path: PathBuf,
    python_path: PathBuf,
    hermes_home: PathBuf,
    active_sessions: Mutex<HashMap<String, QrSessionInfo>>,
}

impl GatewaySetupService {
    pub fn new(hermes_home: &std::path::Path) -> Self {
        let script_path = Self::find_helper_script();
        let python_path = Self::find_python();

        // Load persisted sessions (survive app restarts per-platform)
        let sessions_path = hermes_home.join("gateway_sessions.json");
        let active_sessions: HashMap<String, QrSessionInfo> =
            if let Ok(content) = std::fs::read_to_string(&sessions_path) {
                serde_json::from_str(&content).unwrap_or_default()
            } else {
                HashMap::new()
            };

        Self {
            script_path,
            python_path,
            hermes_home: hermes_home.to_path_buf(),
            active_sessions: Mutex::new(active_sessions),
        }
    }

    fn find_helper_script() -> PathBuf {
        let candidates = vec![
            // Production: scripts/ next to exe
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("scripts").join("gateway_setup_helper.py"))),
            // Dev: relative to Cargo.toml
            Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("scripts")
                    .join("gateway_setup_helper.py"),
            ),
            // Dev: relative to Cargo.toml (alternative nesting)
            Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("scripts")
                    .join("gateway_setup_helper.py"),
            ),
        ];

        for p in candidates.into_iter().flatten() {
            if p.exists() {
                log::info!("Found gateway setup helper: {}", p.display());
                return p;
            }
        }
        // Fallback
        PathBuf::from("scripts").join("gateway_setup_helper.py")
    }

    fn find_python() -> PathBuf {
        // Check for bundled venv
        if let Some(app_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            let scripts = if cfg!(windows) { "Scripts" } else { "bin" };
            let venv_python = app_dir
                .join("..")
                .join("hermes-agent")
                .join("venv")
                .join(scripts)
                .join(if cfg!(windows) { "python.exe" } else { "python" });
            if venv_python.exists() {
                return venv_python;
            }
        }

        // Dev: check hermes-agent/venv relative to project root
        let dev_venv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("hermes-agent")
            .join("venv")
            .join(if cfg!(windows) { "Scripts" } else { "bin" })
            .join(if cfg!(windows) { "python.exe" } else { "python" });
        if dev_venv.exists() {
            return dev_venv;
        }

        PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
    }

    fn run_script(&self, args: &[&str]) -> Result<String, String> {
        let mut cmd = Command::new(&self.python_path);
        cmd.arg(&self.script_path)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1");

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to run gateway helper: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            // Try to parse error JSON from stdout first
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout) {
                if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
                    return Err(err.to_string());
                }
            }
            return Err(format!("Script exited with {}: {}", output.status, stderr.trim()));
        }

        Ok(stdout.trim().to_string())
    }

    // ── Platform listing ────────────────────────────────────────────────

    pub fn list_platforms(&self) -> Result<Vec<PlatformInfo>, String> {
        let output = self.run_script(&["list-platforms"])?;
        let parsed: serde_json::Value =
            serde_json::from_str(&output).map_err(|e| format!("Parse error: {e}"))?;

        let platforms: Vec<PlatformInfo> = parsed
            .get("platforms")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(platforms)
    }

    /// Build platform status list merging QR platforms with config.yaml state
    /// and including any additional platforms found only in config.yaml.
    pub fn list_platform_status(&self) -> Result<Vec<PlatformConfigStatus>, String> {
        let qr_platforms = self.list_platforms().unwrap_or_default();
        let config = self.read_gateway_config()?;

        let platforms_obj = config
            .get("platforms")
            .and_then(|v| v.as_object());

        // Start with QR-capable platforms
        let mut result: Vec<PlatformConfigStatus> = qr_platforms
            .into_iter()
            .map(|p| {
                let platform_cfg = platforms_obj.and_then(|cfg| cfg.get(&p.key));
                let enabled = platform_cfg
                    .and_then(|c: &serde_json::Value| c.get("enabled"))
                    .and_then(|v: &serde_json::Value| v.as_bool())
                    .unwrap_or(false);
                let configured = platform_cfg.is_some();
                PlatformConfigStatus {
                    key: p.key,
                    label: p.label,
                    enabled,
                    configured,
                    has_qr: true,
                }
            })
            .collect();

        // Add non-QR platforms found in config.yaml
        let qr_keys: std::collections::HashSet<String> =
            result.iter().map(|p| p.key.clone()).collect();

        if let Some(cfg) = platforms_obj {
            for (key, _val) in cfg {
                if !qr_keys.contains(key) {
                    let enabled = _val
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    // Map known platform keys to human labels
                    let label = Self::platform_label(key);
                    result.push(PlatformConfigStatus {
                        key: key.clone(),
                        label,
                        enabled,
                        configured: true,
                        has_qr: false,
                    });
                }
            }
        }

        // Sort: QR platforms first, then alphabetically
        result.sort_by(|a, b| {
            b.has_qr.cmp(&a.has_qr)
                .then_with(|| a.label.cmp(&b.label))
        });

        Ok(result)
    }

    fn platform_label(key: &str) -> String {
        match key {
            "telegram" => "Telegram".into(),
            "discord" => "Discord".into(),
            "weixin" => "微信".into(),
            "wecom" => "企业微信".into(),
            "feishu" => "飞书".into(),
            "qqbot" => "QQ 机器人".into(),
            "dingtalk" => "钉钉".into(),
            "slack" => "Slack".into(),
            "whatsapp" => "WhatsApp".into(),
            "signal" => "Signal".into(),
            "matrix" => "Matrix".into(),
            "mattermost" => "Mattermost".into(),
            "email" => "Email".into(),
            "sms" => "SMS".into(),
            "imessage" => "iMessage".into(),
            "webhooks" => "Webhooks".into(),
            "api_server" => "API Server".into(),
            "irc" => "IRC".into(),
            "line" => "LINE".into(),
            "teams" => "Microsoft Teams".into(),
            "zalo" => "Zalo".into(),
            "wechat" => "微信 (旧)".into(),
            "kook" => "KOOK".into(),
            "telegram_business" => "Telegram Business".into(),
            "douyin" => "抖音".into(),
            "yuanbao" => "元宝".into(),
            "qqbot" => "QQ 机器人 (需手动配置)".into(),
            other => {
                // Humanize the key: replace underscores, capitalize
                let mut s = other.replace('_', " ");
                if let Some(r) = s.get_mut(0..1) {
                    r.make_ascii_uppercase();
                }
                s
            }
        }
    }

    // ── QR flow ────────────────────────────────────────────────────────

    /// Begin QR registration for a platform. Returns session info with QR URL.
    pub fn qr_start(&self, platform: &str) -> Result<QrSessionInfo, String> {
        let output = self.run_script(&["qr-start", platform])?;
        let parsed: serde_json::Value =
            serde_json::from_str(&output).map_err(|e| format!("Parse error: {e}"))?;

        if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = parsed
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(err.to_string());
        }

        let session_id = parsed
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing session_id")?
            .to_string();
        let qr_url = parsed
            .get("qr_url")
            .and_then(|v| v.as_str())
            .ok_or("Missing qr_url")?
            .to_string();
        let timeout_seconds = parsed
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(600) as u32;

        let info = QrSessionInfo {
            session_id: session_id.clone(),
            platform: platform.to_string(),
            qr_url: qr_url.clone(),
            timeout_seconds,
        };

        // Persist to in-memory cache
        {
            let mut sessions = self.active_sessions.lock().map_err(|e| e.to_string())?;
            sessions.insert(platform.to_string(), info.clone());
        }
        self.save_sessions()?;

        Ok(info)
    }

    /// Poll QR registration status for a platform.
    pub fn qr_poll(&self, platform: &str) -> Result<QrPollResult, String> {
        let session_id = {
            let sessions = self.active_sessions.lock().map_err(|e| e.to_string())?;
            sessions
                .get(platform)
                .map(|s| s.session_id.clone())
                .ok_or_else(|| format!("No active QR session for {}", platform))?
        };

        let output = self.run_script(&["qr-poll", platform, &session_id])?;
        let parsed: serde_json::Value =
            serde_json::from_str(&output).map_err(|e| format!("Parse error: {e}"))?;

        if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = parsed
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(err.to_string());
        }

        let status = parsed
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("waiting")
            .to_string();

        let mut result = QrPollResult {
            status: status.clone(),
            message: None,
            qr_url: None,
            credentials: None,
        };

        if let Some(msg) = parsed.get("message").and_then(|v| v.as_str()) {
            result.message = Some(msg.to_string());
        }
        if let Some(qr) = parsed.get("qr_url").and_then(|v| v.as_str()) {
            result.qr_url = Some(qr.to_string());
        }
        if let Some(creds) = parsed.get("credentials") {
            if let Ok(map) = serde_json::from_value::<HashMap<String, String>>(creds.clone()) {
                result.credentials = Some(map);
            }
        }

        // On terminal statuses, clean up session
        if matches!(
            status.as_str(),
            "success" | "failed" | "expired"
        ) {
            if let Ok(mut sessions) = self.active_sessions.lock() {
                sessions.remove(platform);
            }
            self.save_sessions()?;
        }

        Ok(result)
    }

    /// Cancel an active QR session for a platform.
    pub fn qr_cancel(&self, platform: &str) -> Result<(), String> {
        let session_id = {
            let mut sessions = self.active_sessions.lock().map_err(|e| e.to_string())?;
            sessions.remove(platform).map(|s| s.session_id)
        };

        if let Some(sid) = session_id {
            self.save_sessions()?;
            // Best-effort cancel on Python side
            let _ = self.run_script(&["qr-cancel", &sid]);
        }

        Ok(())
    }

    /// Get active QR session info for a platform (for restoring UI state).
    pub fn get_active_session(&self, platform: &str) -> Option<QrSessionInfo> {
        self.active_sessions
            .lock()
            .ok()
            .and_then(|s| s.get(platform).cloned())
    }

    fn save_sessions(&self) -> Result<(), String> {
        let sessions = self.active_sessions.lock().map_err(|e| e.to_string())?;
        let path = self.hermes_home.join("gateway_sessions.json");
        let content =
            serde_json::to_string_pretty(&*sessions).map_err(|e| format!("Serialize error: {e}"))?;
        std::fs::write(&path, content).map_err(|e| format!("Write error: {e}"))?;
        Ok(())
    }

    // ── Config management ──────────────────────────────────────────────

    /// Read gateway-related config from config.yaml
    pub fn read_gateway_config(&self) -> Result<serde_json::Value, String> {
        let config_path = self.hermes_home.join("config.yaml");
        if !config_path.exists() {
            return Ok(serde_json::json!({"platforms": {}}));
        }

        let content =
            std::fs::read_to_string(&config_path).map_err(|e| format!("Read error: {e}"))?;
        let content = content.strip_prefix('\u{FEFF}').unwrap_or(&content);

        let value: serde_yaml::Value =
            serde_yaml::from_str(content).map_err(|e| format!("Parse YAML error: {e}"))?;

        let platforms = value
            .get("platforms")
            .map(|p| serde_json::to_value(p).unwrap_or(serde_json::json!({})))
            .unwrap_or(serde_json::json!({}));

        Ok(serde_json::json!({"platforms": platforms}))
    }

    /// Save a platform configuration to config.yaml
    pub fn save_platform_config(
        &self,
        platform_key: &str,
        config: &serde_json::Value,
    ) -> Result<(), String> {
        let config_path = self.hermes_home.join("config.yaml");

        let mut root: serde_yaml::Value = if config_path.exists() {
            let content =
                std::fs::read_to_string(&config_path).map_err(|e| format!("Read error: {e}"))?;
            let content = content.strip_prefix('\u{FEFF}').unwrap_or(&content);
            serde_yaml::from_str(content).unwrap_or(serde_yaml::Value::Mapping(Default::default()))
        } else {
            serde_yaml::Value::Mapping(Default::default())
        };

        // Convert JSON config to YAML value
        let platform_yaml: serde_yaml::Value =
            serde_json::from_value(config.clone()).map_err(|e| format!("Convert error: {e}"))?;

        // Insert or update the platform section
        if let Some(mapping) = root.as_mapping_mut() {
            // Get or create platforms section
            let platforms_key = serde_yaml::Value::String("platforms".into());
            if let Some(platforms_val) = mapping.get_mut(&platforms_key) {
                if let Some(platforms_map) = platforms_val.as_mapping_mut() {
                    platforms_map.insert(
                        serde_yaml::Value::String(platform_key.into()),
                        platform_yaml,
                    );
                }
            } else {
                let mut platforms_map = serde_yaml::Mapping::new();
                platforms_map.insert(
                    serde_yaml::Value::String(platform_key.into()),
                    platform_yaml,
                );
                mapping.insert(platforms_key, serde_yaml::Value::Mapping(platforms_map));
            }
        }

        let yaml =
            serde_yaml::to_string(&root).map_err(|e| format!("Serialize YAML error: {e}"))?;

        // Atomic write
        let tmp_path = config_path.with_extension("tmp");
        std::fs::write(&tmp_path, &yaml).map_err(|e| format!("Write tmp error: {e}"))?;
        std::fs::rename(&tmp_path, &config_path).map_err(|e| format!("Rename error: {e}"))
    }

    /// Remove a platform configuration from config.yaml
    pub fn remove_platform_config(&self, platform_key: &str) -> Result<(), String> {
        let config_path = self.hermes_home.join("config.yaml");
        if !config_path.exists() {
            return Ok(());
        }

        let content =
            std::fs::read_to_string(&config_path).map_err(|e| format!("Read error: {e}"))?;
        let content = content.strip_prefix('\u{FEFF}').unwrap_or(&content);

        let mut root: serde_yaml::Value =
            serde_yaml::from_str(content).map_err(|e| format!("Parse YAML error: {e}"))?;

        if let Some(mapping) = root.as_mapping_mut() {
            let platforms_key = serde_yaml::Value::String("platforms".into());
            if let Some(platforms_val) = mapping.get_mut(&platforms_key) {
                if let Some(platforms_map) = platforms_val.as_mapping_mut() {
                    platforms_map.remove(&serde_yaml::Value::String(platform_key.into()));
                }
            }
        }

        let yaml =
            serde_yaml::to_string(&root).map_err(|e| format!("Serialize YAML error: {e}"))?;

        let tmp_path = config_path.with_extension("tmp");
        std::fs::write(&tmp_path, &yaml).map_err(|e| format!("Write tmp error: {e}"))?;
        std::fs::rename(&tmp_path, &config_path).map_err(|e| format!("Rename error: {e}"))
    }

    /// Save credentials from QR scan to env vars and platform config.
    pub fn save_platform_credentials(
        &self,
        platform_key: &str,
        credentials: &HashMap<String, String>,
    ) -> Result<(), String> {
        // Build platform config based on platform type
        let config = match platform_key {
            "weixin" => {
                let mut extra = serde_json::Map::new();
                if let Some(v) = credentials.get("account_id") {
                    extra.insert("account_id".into(), serde_json::Value::String(v.clone()));
                }
                if let Some(v) = credentials.get("base_url") {
                    extra.insert("base_url".into(), serde_json::Value::String(v.clone()));
                }
                if let Some(v) = credentials.get("user_id") {
                    extra.insert("user_id".into(), serde_json::Value::String(v.clone()));
                }
                serde_json::json!({
                    "enabled": true,
                    "token": credentials.get("token").cloned().unwrap_or_default(),
                    "extra": extra,
                })
            }
            "wecom" => serde_json::json!({
                "enabled": true,
                "extra": {
                    "bot_id": credentials.get("bot_id").cloned().unwrap_or_default(),
                    "secret": credentials.get("secret").cloned().unwrap_or_default(),
                },
            }),
            "feishu" => serde_json::json!({
                "enabled": true,
                "extra": {
                    "app_id": credentials.get("app_id").cloned().unwrap_or_default(),
                    "app_secret": credentials.get("app_secret").cloned().unwrap_or_default(),
                    "domain": credentials.get("domain").cloned().unwrap_or_else(|| "feishu".into()),
                },
            }),
            "qqbot" => serde_json::json!({
                "enabled": true,
                "extra": {
                    "app_id": credentials.get("app_id").cloned().unwrap_or_default(),
                    "client_secret": credentials.get("client_secret").cloned().unwrap_or_default(),
                },
            }),
            "dingtalk" => serde_json::json!({
                "enabled": true,
                "extra": {
                    "client_id": credentials.get("client_id").cloned().unwrap_or_default(),
                    "client_secret": credentials.get("client_secret").cloned().unwrap_or_default(),
                },
            }),
            _ => serde_json::json!({
                "enabled": true,
                "extra": credentials.clone(),
            }),
        };

        self.save_platform_config(platform_key, &config)?;

        Ok(())
    }
}
