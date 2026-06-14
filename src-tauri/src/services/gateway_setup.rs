use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use tokio::task;

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
    hermes_home: PathBuf,       // AI-Hel2 data dir (for sessions/logs)
    agent_home: PathBuf,        // Agent's ~/.hermes (for config.yaml)
    active_sessions: Mutex<HashMap<String, QrSessionInfo>>,
}

impl GatewaySetupService {
    pub fn new(hermes_home: &std::path::Path) -> Self {
        let script_path = Self::find_helper_script();
        let python_path = Self::find_python();

        // Agent's home: ~/.hermes (where config.yaml lives)
        let agent_home = dirs_home().join(".hermes");

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
            agent_home,
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

        // Collect keys already present (QR platforms + any from config.yaml)
        let mut seen_keys: std::collections::HashSet<String> =
            result.iter().map(|p| p.key.clone()).collect();

        // Add non-QR platforms from config.yaml (already configured, but not in QR list)
        if let Some(cfg) = platforms_obj {
            for (key, _val) in cfg {
                if !seen_keys.contains(key) {
                    let enabled = _val
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let label = Self::platform_label(key);
                    result.push(PlatformConfigStatus {
                        key: key.clone(),
                        label,
                        enabled,
                        configured: true,
                        has_qr: false,
                    });
                    seen_keys.insert(key.clone());
                }
            }
        }

        // Add all known non-QR platforms that aren't already in the list,
        // so users can discover them even before configuring.
        for (key, label) in Self::ALL_NON_QR_PLATFORMS {
            if !seen_keys.contains(*key) {
                result.push(PlatformConfigStatus {
                    key: key.to_string(),
                    label: label.to_string(),
                    enabled: false,
                    configured: false,
                    has_qr: false,
                });
                seen_keys.insert(key.to_string());
            }
        }

        // Sort: QR platforms first, then alphabetically
        result.sort_by(|a, b| {
            b.has_qr.cmp(&a.has_qr)
                .then_with(|| a.label.cmp(&b.label))
        });

        Ok(result)
    }

    /// All non-QR platforms that have adapter code in hermes-agent.
    /// These are shown in the UI even when not yet configured in config.yaml,
    /// so users can discover them.
    const ALL_NON_QR_PLATFORMS: &[(&str, &str)] = &[
        ("telegram", "Telegram"),
        ("discord", "Discord"),
        ("whatsapp", "WhatsApp"),
        ("signal", "Signal"),
        ("slack", "Slack"),
        ("matrix", "Matrix"),
        ("mattermost", "Mattermost"),
        ("bluebubbles", "iMessage (BlueBubbles)"),
        ("email", "Email"),
        ("sms", "SMS"),
        ("homeassistant", "Home Assistant"),
        ("webhook", "Webhook"),
        ("msgraph_webhook", "Microsoft Graph"),
        ("api_server", "API Server"),
        ("yuanbao", "元宝"),
        ("google_chat", "Google Chat"),
        ("irc", "IRC"),
        ("line", "LINE"),
        ("simplex", "SimpleX Chat"),
        ("teams", "Microsoft Teams"),
    ];

    fn agent_config_path(&self) -> PathBuf {
        self.agent_home.join("config.yaml")
    }

    fn agent_cron_dir(&self) -> PathBuf {
        self.agent_home.join("cron")
    }

    fn platform_label(key: &str) -> String {
        // Check the known list first
        for (k, label) in Self::ALL_NON_QR_PLATFORMS {
            if *k == key { return label.to_string(); }
        }
        match key {
            "weixin" => "微信".into(),
            "wecom" => "企业微信".into(),
            "feishu" => "飞书".into(),
            "qqbot" => "QQ 机器人".into(),
            "dingtalk" => "钉钉".into(),
            other => {
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

    /// Async: Begin QR registration (runs script on blocking thread to avoid UI freeze).
    pub async fn qr_start_async(&self, platform: &str) -> Result<QrSessionInfo, String> {
        let python = self.python_path.clone();
        let script = self.script_path.clone();
        let platform_owned = platform.to_string();
        let output = task::spawn_blocking(move || {
            let mut cmd = Command::new(&python);
            cmd.arg(&script)
                .arg("qr-start")
                .arg(&platform_owned)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .env("PYTHONIOENCODING", "utf-8")
                .env("PYTHONUTF8", "1");
            #[cfg(windows)]
            { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
            let output = cmd.output().map_err(|e| format!("Failed to run gateway helper: {e}"))?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !output.status.success() {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
                        return Err(err.to_string());
                    }
                }
                return Err(format!("Script exited with {}: {}", output.status, stderr.trim()));
            }
            Ok(stdout.trim().to_string())
        }).await.map_err(|e| format!("Gateway task panicked: {e}"))??;

        let parsed: serde_json::Value = serde_json::from_str(&output).map_err(|e| format!("Parse error: {e}"))?;
        if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = parsed.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            return Err(err.to_string());
        }
        let info = QrSessionInfo {
            session_id: parsed.get("session_id").and_then(|v| v.as_str()).ok_or("Missing session_id")?.to_string(),
            platform: platform.to_string(),
            qr_url: parsed.get("qr_url").and_then(|v| v.as_str()).ok_or("Missing qr_url")?.to_string(),
            timeout_seconds: parsed.get("timeout_seconds").and_then(|v| v.as_u64()).unwrap_or(600) as u32,
        };
        {
            let mut sessions = self.active_sessions.lock().map_err(|e| e.to_string())?;
            sessions.insert(platform.to_string(), info.clone());
        }
        self.save_sessions()?;
        Ok(info)
    }

    /// Async: Poll QR registration status (runs script on blocking thread).
    pub async fn qr_poll_async(&self, platform: &str) -> Result<QrPollResult, String> {
        let session_id = {
            let sessions = self.active_sessions.lock().map_err(|e| e.to_string())?;
            sessions.get(platform).map(|s| s.session_id.clone())
                .ok_or_else(|| format!("No active QR session for {}", platform))?
        };
        let python = self.python_path.clone();
        let script = self.script_path.clone();
        let platform_owned = platform.to_string();
        let sid = session_id.clone();
        let output = task::spawn_blocking(move || {
            let mut cmd = Command::new(&python);
            cmd.arg(&script).arg("qr-poll").arg(&platform_owned).arg(&sid)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .env("PYTHONIOENCODING", "utf-8")
                .env("PYTHONUTF8", "1");
            #[cfg(windows)]
            { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
            let output = cmd.output().map_err(|e| format!("Failed to run gateway helper: {e}"))?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !output.status.success() {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
                        return Err(err.to_string());
                    }
                }
                return Err(format!("Script exited with {}: {}", output.status, stderr.trim()));
            }
            Ok(stdout.trim().to_string())
        }).await.map_err(|e| format!("Gateway task panicked: {e}"))??;

        let parsed: serde_json::Value = serde_json::from_str(&output).map_err(|e| format!("Parse error: {e}"))?;
        if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = parsed.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            return Err(err.to_string());
        }
        let status = parsed.get("status").and_then(|v| v.as_str()).unwrap_or("waiting").to_string();
        Ok(QrPollResult {
            status: status.clone(),
            message: parsed.get("message").and_then(|v| v.as_str()).map(String::from),
            qr_url: parsed.get("qr_url").and_then(|v| v.as_str()).map(String::from),
            credentials: parsed.get("credentials").map(|v| {
                let mut creds = HashMap::new();
                if let Some(obj) = v.as_object() {
                    for (k, val) in obj { creds.insert(k.clone(), val.as_str().unwrap_or("").to_string()); }
                }
                creds
            }),
        })
    }

    /// Async: Cancel QR session (runs script on blocking thread).
    pub async fn qr_cancel_async(&self, platform: &str) -> Result<(), String> {
        let session_id = {
            let mut sessions = self.active_sessions.lock().map_err(|e| e.to_string())?;
            sessions.remove(platform).map(|s| s.session_id)
        };
        if let Some(sid) = session_id {
            self.save_sessions()?;
            let python = self.python_path.clone();
            let script = self.script_path.clone();
            let _ = task::spawn_blocking(move || {
                let mut cmd = Command::new(&python);
                cmd.arg(&script).arg("qr-cancel").arg(&sid)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .env("PYTHONIOENCODING", "utf-8")
                    .env("PYTHONUTF8", "1");
                #[cfg(windows)]
                { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
                cmd.output()
            }).await;
        }
        Ok(())
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
        let config_path = self.agent_config_path();
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
        let config_path = self.agent_config_path();

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
        let config_path = self.agent_config_path();
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

    // ── Cron job management ─────────────────────────────────────────────

    fn cron_dir(&self) -> PathBuf {
        self.agent_cron_dir()
    }

    fn cron_jobs_path(&self) -> PathBuf {
        self.cron_dir().join("jobs.json")
    }

    fn cron_output_dir(&self) -> PathBuf {
        self.cron_dir().join("output")
    }

    /// Read all cron jobs from jobs.json.
    fn read_cron_jobs(&self) -> Result<Vec<serde_json::Value>, String> {
        let path = self.cron_jobs_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Read jobs.json error: {e}"))?;
        let jobs: Vec<serde_json::Value> = serde_json::from_str(&content)
            .map_err(|e| format!("Parse jobs.json error: {e}"))?;
        Ok(jobs)
    }

    /// Write all cron jobs to jobs.json (atomic via temp file).
    fn write_cron_jobs(&self, jobs: &[serde_json::Value]) -> Result<(), String> {
        let path = self.cron_jobs_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        let content = serde_json::to_string_pretty(jobs)
            .map_err(|e| format!("Serialize error: {e}"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &content).map_err(|e| format!("Write tmp error: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("Rename error: {e}"))
    }

    /// List all cron jobs.
    pub fn list_cron_jobs(&self) -> Result<Vec<serde_json::Value>, String> {
        self.read_cron_jobs()
    }

    /// Add a new cron job.
    pub fn add_cron_job(&self, job: serde_json::Value) -> Result<serde_json::Value, String> {
        let mut jobs = self.read_cron_jobs()?;
        let mut new_job = job.clone();
        // Generate ID + timestamps if not provided
        if new_job.get("id").and_then(|v| v.as_str()).map_or(true, |s| s.is_empty()) {
            let id = uuid::Uuid::new_v4().to_string()[..12].to_string();
            new_job["id"] = serde_json::Value::String(id);
        }
        if new_job.get("created_at").is_none() {
            new_job["created_at"] = serde_json::Value::String(
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
            );
        }
        if new_job.get("enabled").is_none() {
            new_job["enabled"] = serde_json::Value::Bool(true);
        }
        if new_job.get("state").is_none() {
            new_job["state"] = serde_json::Value::String("scheduled".into());
        }
        jobs.push(new_job.clone());
        self.write_cron_jobs(&jobs)?;
        Ok(new_job)
    }

    /// Update an existing cron job.
    pub fn update_cron_job(&self, job_id: &str, updates: serde_json::Value) -> Result<(), String> {
        let mut jobs = self.read_cron_jobs()?;
        let job = jobs.iter_mut().find(|j| {
            j.get("id").and_then(|v| v.as_str()) == Some(job_id)
        }).ok_or_else(|| format!("Job {} not found", job_id))?;
        if let Some(obj) = updates.as_object() {
            for (k, v) in obj {
                job[k] = v.clone();
            }
        }
        self.write_cron_jobs(&jobs)
    }

    /// Delete a cron job.
    pub fn delete_cron_job(&self, job_id: &str) -> Result<(), String> {
        let mut jobs = self.read_cron_jobs()?;
        let before = jobs.len();
        jobs.retain(|j| j.get("id").and_then(|v| v.as_str()) != Some(job_id));
        if jobs.len() == before {
            return Err(format!("Job {} not found", job_id));
        }
        self.write_cron_jobs(&jobs)
    }

    /// Toggle cron job enabled state.
    pub fn toggle_cron_job(&self, job_id: &str, enabled: bool) -> Result<(), String> {
        let state = if enabled { "scheduled" } else { "paused" };
        self.update_cron_job(job_id, serde_json::json!({
            "enabled": enabled,
            "state": state,
        }))
    }

    /// Trigger a cron job immediately by setting next_run_at to now.
    pub fn trigger_cron_job(&self, job_id: &str) -> Result<(), String> {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        self.update_cron_job(job_id, serde_json::json!({
            "next_run_at": now,
            "state": "scheduled",
        }))
    }

    /// Get recent output for a cron job.
    pub fn get_cron_output(&self, job_id: &str) -> Result<Vec<String>, String> {
        let output_dir = self.cron_output_dir().join(job_id);
        if !output_dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<String> = Vec::new();
        if let Ok(iter) = std::fs::read_dir(&output_dir) {
            for entry in iter.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".md") || name.ends_with(".txt") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        // Truncate to first 500 chars for preview
                        let preview = if content.len() > 500 {
                            format!("{}...", &content[..500])
                        } else {
                            content
                        };
                        entries.push(format!("{}:\n{}", name, preview));
                    }
                }
            }
        }
        entries.sort_by(|a, b| b.cmp(a)); // newest first
        Ok(entries.into_iter().take(5).collect())
    }
}

fn dirs_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp"))
    }
}
