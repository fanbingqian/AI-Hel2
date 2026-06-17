use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const DEFAULT_PORT: u16 = 18789;
const HEALTH_POLL_INTERVAL_MS: u64 = 500;
const STARTUP_TIMEOUT_SECS: u64 = 30;

/// Manages the OpenClaw gateway process lifecycle.
/// Detects, auto-configures, starts, health-checks, and stops OpenClaw.
pub struct OpenClawLauncher {
    child: Mutex<Option<Child>>,
    port: u16,
    config_path: PathBuf,
    binary_path: Option<PathBuf>,
}

impl OpenClawLauncher {
    pub fn new() -> Self {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:".into());
        let config_path = PathBuf::from(&home).join(".openclaw").join("openclaw.json");
        let port = read_openclaw_port(&config_path).unwrap_or(DEFAULT_PORT);
        let binary_path = find_openclaw_binary();

        log::info!(
            "[OpenClaw] init: port={}, binary={:?}, config={}",
            port,
            binary_path,
            config_path.display()
        );

        Self {
            child: Mutex::new(None),
            port,
            config_path,
            binary_path,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    /// Check if OpenClaw is running and the chat endpoint is available.
    pub fn is_chat_ready(&self) -> bool {
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", self.port);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();

        let body = serde_json::json!({"model": "__probe__", "messages": []});
        match client.post(&url).json(&body).send() {
            Ok(resp) => {
                // 404 = HTTP API disabled, 401 = auth missing — neither is ready
                let ok = resp.status().as_u16() != 404 && resp.status().as_u16() != 401;
                if ok {
                    log::info!("[OpenClaw] chat endpoint ready (HTTP {})", resp.status());
                } else if resp.status().as_u16() == 401 {
                    log::warn!("[OpenClaw] chat endpoint returned 401 — API key missing or invalid");
                }
                ok
            }
            Err(_) => false,
        }
    }

    /// Ensure the HTTP API chat completions endpoint is enabled in openclaw.json.
    /// Returns true if the config was modified (requires restart to take effect).
    pub fn ensure_http_api_enabled(&self) -> Result<bool, String> {
        let raw = fs::read_to_string(&self.config_path)
            .map_err(|e| format!("读取 openclaw.json 失败: {e}"))?;
        let mut v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("解析 openclaw.json 失败: {e}"))?;

        let gateway = &v["gateway"];
        let http = &gateway["http"];
        let chat_enabled = http["endpoints"]["chatCompletions"]["enabled"].as_bool();

        if chat_enabled == Some(true) {
            log::info!("[OpenClaw] HTTP API chat completions already enabled");
            return Ok(false);
        }

        // Add http section
        if v["gateway"]["http"].is_null() {
            v["gateway"]["http"] = serde_json::json!({});
        }
        if v["gateway"]["http"]["endpoints"].is_null() {
            v["gateway"]["http"]["endpoints"] = serde_json::json!({});
        }
        v["gateway"]["http"]["endpoints"]["chatCompletions"] = serde_json::json!({"enabled": true});

        let updated = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
        fs::write(&self.config_path, updated)
            .map_err(|e| format!("写入 openclaw.json 失败: {e}"))?;

        log::info!("[OpenClaw] HTTP API chat completions enabled in config");
        Ok(true)
    }

    /// Start the OpenClaw gateway and wait for it to become ready.
    pub fn start(&self) -> Result<(), String> {
        if self.is_chat_ready() {
            log::info!("[OpenClaw] already running and chat-ready on port {}", self.port);
            return Ok(());
        }

        // Auto-configure HTTP API if needed
        match self.ensure_http_api_enabled() {
            Ok(true) => log::info!("[OpenClaw] config updated — may need restart"),
            Ok(false) => {}
            Err(e) => log::warn!("[OpenClaw] auto-config failed: {e}"),
        }

        // If already running (health passes but chat isn't ready), we need a restart
        if self.is_health_ok() {
            log::info!("[OpenClaw] running but chat endpoint disabled, restarting...");
            self.kill_existing();
            std::thread::sleep(Duration::from_secs(2));
        }

        let binary = self
            .binary_path
            .as_ref()
            .ok_or_else(|| "OpenClaw binary not found. Install with: npm install -g openclaw".to_string())?;

        log::info!("[OpenClaw] spawning: {}", binary.display());
        let child = spawn_openclaw(binary)?;
        log::info!("[OpenClaw] spawned (pid={})", child.id());

        {
            let mut guard = self.child.lock().map_err(|e| e.to_string())?;
            *guard = Some(child);
        }

        self.wait_until_ready(Duration::from_secs(STARTUP_TIMEOUT_SECS))
    }

    fn is_health_ok(&self) -> bool {
        let url = format!("http://127.0.0.1:{}/health", self.port);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        client.get(&url).send().map(|r| r.status().is_success()).unwrap_or(false)
    }

    fn kill_existing(&self) {
        log::info!("[OpenClaw] killing existing process on port {}", self.port);
        // Try graceful shutdown via API if available
        if let Ok(client) = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
        {
            let _ = client
                .post(format!("http://127.0.0.1:{}/shutdown", self.port))
                .send();
        }
        // Fallback: kill by port (Windows)
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/PID"])
                .arg(find_pid_by_port(self.port).unwrap_or_default().to_string())
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
    }

    fn wait_until_ready(&self, timeout: Duration) -> Result<(), String> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.is_chat_ready() {
                log::info!(
                    "[OpenClaw] ready on port {} after {:.1}s",
                    self.port,
                    start.elapsed().as_secs_f32()
                );
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(HEALTH_POLL_INTERVAL_MS));
        }
        Err(format!(
            "OpenClaw did not become ready within {}s on port {}",
            timeout.as_secs(),
            self.port
        ))
    }

    /// Gracefully shut down the managed OpenClaw process.
    pub fn shutdown(&self) {
        let mut guard = match self.child.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(mut child) = guard.take() else {
            return;
        };
        let pid = child.id();
        log::info!("[OpenClaw] shutting down (pid={pid})...");
        let _ = child.kill();
        let _ = child.wait();
    }
}

// ── Helpers ──

fn find_openclaw_binary() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:".into());

    // Priority 1: npm global install (Windows)
    #[cfg(target_os = "windows")]
    {
        let npm_path = PathBuf::from(&home)
            .join("AppData")
            .join("Roaming")
            .join("npm")
            .join("openclaw.cmd");
        if npm_path.exists() {
            return Some(npm_path);
        }
    }

    // Priority 2: npm global (Unix)
    #[cfg(not(target_os = "windows"))]
    {
        for p in &[
            PathBuf::from(&home).join(".local").join("bin").join("openclaw"),
            PathBuf::from("/usr/local/bin/openclaw"),
        ] {
            if p.exists() {
                return Some(p);
            }
        }
    }

    // Priority 3: PATH lookup
    #[cfg(windows)]
    let which = "where";
    #[cfg(not(windows))]
    let which = "which";

    let mut which_cmd = Command::new(which);
    which_cmd.arg("openclaw");
    #[cfg(windows)]
    which_cmd.creation_flags(0x08000000);
    if let Ok(output) = which_cmd.output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    None
}

fn read_openclaw_port(config_path: &PathBuf) -> Option<u16> {
    let raw = fs::read_to_string(config_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v["gateway"]["port"].as_u64().map(|p| p as u16)
}

fn spawn_openclaw(binary: &PathBuf) -> Result<Child, String> {
    let mut cmd = Command::new(binary);
    cmd.arg("gateway")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null());

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to spawn OpenClaw: {e}"))
}

#[cfg(windows)]
fn find_pid_by_port(port: u16) -> Option<u32> {
    let output = Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.contains(&format!("127.0.0.1:{port}")) || line.contains(&format!("0.0.0.0:{port}")) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(pid_str) = parts.last() {
                return pid_str.parse().ok();
            }
        }
    }
    None
}
