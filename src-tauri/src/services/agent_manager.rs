use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const DEFAULT_PORT: u16 = 18642;
const HEALTH_CHECK_TIMEOUT_MS: u64 = 1500;
const HEALTH_POLL_INTERVAL_MS: u64 = 500;
const STARTUP_TIMEOUT_SECS: u64 = 30;
const SHUTDOWN_GRACE_SECS: u64 = 5;
const MAX_AUTO_RESTART_ATTEMPTS: u32 = 5;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub port: u16,
    pub version: Option<String>,
    pub healthy: bool,
    pub error: Option<String>,
}

// Minimal config read for port — avoids pulling in full ConfigService
#[derive(Debug, Deserialize)]
struct GatewayConfig {
    platforms: Option<PlatformsSection>,
}

#[derive(Debug, Deserialize)]
struct PlatformsSection {
    api_server: Option<ApiServerSection>,
}

#[derive(Debug, Deserialize)]
struct ApiServerSection {
    enabled: Option<bool>,
    extra: Option<ApiServerExtra>,
}

#[derive(Debug, Deserialize)]
struct ApiServerExtra {
    port: Option<u16>,
    host: Option<String>,
}

pub struct AgentManager {
    hermes_home: PathBuf,
    child: Mutex<Option<Child>>,
    port: u16,
    api_url: String,
    gateway_start_time: Mutex<Option<Instant>>,
    consecutive_failures: AtomicU32,
}

impl AgentManager {
    pub fn new(hermes_home: &std::path::Path) -> Self {
        let port = read_agent_port(hermes_home);
        let api_url = format!("http://127.0.0.1:{port}");
        Self {
            hermes_home: hermes_home.to_path_buf(),
            child: Mutex::new(None),
            port,
            api_url,
            gateway_start_time: Mutex::new(None),
            consecutive_failures: AtomicU32::new(0),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    /// Resolve the application root directory.
    /// Checks multiple locations: exe-relative (production), cargo target parent (dev),
    /// and a hardcoded fallback.
    fn app_dir() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let exe_dir = exe.parent()?;

        let candidates: Vec<PathBuf> = vec![
            // production: hermes-agent/ next to the exe
            exe_dir.to_path_buf(),
            // cargo run: target/debug/../../ or target/release/../../
            exe_dir.join("..").join(".."),
            exe_dir.join("..").join("..").join(".."),
            // Project root relative to Cargo.toml (src-tauri/)
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."),
        ];

        for dir in &candidates {
            let agent_dir = dir.join("hermes-agent");
            if agent_dir.exists() && agent_dir.join("hermes_cli").join("main.py").exists() {
                let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
                log::info!("Detected app directory: {}", canonical.display());
                return Some(canonical);
            }
        }

        None
    }

    /// Detect the Python executable path.
    /// Priority: bundled hermes-agent/venv in app dir → hermes_home venv → system python
    fn python_path(&self) -> PathBuf {
        let scripts = if cfg!(windows) { "Scripts" } else { "bin" };

        // Priority 1: Bundled hermes-agent in app directory
        if let Some(app_dir) = Self::app_dir() {
            let bundled_venv = app_dir.join("hermes-agent").join("venv");
            if cfg!(windows) {
                let pythonw = bundled_venv.join(scripts).join("pythonw.exe");
                if pythonw.exists() {
                    return pythonw;
                }
            }
            let python = if cfg!(windows) {
                bundled_venv.join(scripts).join("python.exe")
            } else {
                bundled_venv.join(scripts).join("python")
            };
            if python.exists() {
                return python;
            }
        }

        // Priority 2: hermes-agent in hermes_home (user-managed copy)
        let home_venv = self.hermes_home.join("hermes-agent").join("venv");
        if cfg!(windows) {
            let pythonw = home_venv.join(scripts).join("pythonw.exe");
            if pythonw.exists() {
                return pythonw;
            }
        }
        let python = if cfg!(windows) {
            home_venv.join(scripts).join("python.exe")
        } else {
            home_venv.join(scripts).join("python")
        };
        if python.exists() {
            return python;
        }

        // Fallback: system python3 / python
        PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
    }

    /// Find the hermes_cli/main.py script path, checking common locations.
    fn find_agent_script(&self) -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();

        // Priority 1: Bundled hermes-agent in app directory
        if let Some(app_dir) = Self::app_dir() {
            candidates.push(
                app_dir.join("hermes-agent").join("hermes_cli").join("main.py")
            );
        }

        // Priority 2: hermes-agent in hermes_home
        candidates.push(
            self.hermes_home.join("hermes-agent").join("hermes_cli").join("main.py")
        );

        // Priority 3: Bundled hermes-agent in project root (dev convenience)
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("hermes-agent").join("hermes_cli").join("main.py")
        );

        for p in &candidates {
            if p.exists() {
                log::info!("Found agent script: {}", p.display());
                return Some(p.clone());
            }
        }
        None
    }

    /// Spawn the agent gateway process.
    /// Uses the dedicated Python venv at D:\\hermes-agent-forAI-Hel2\\.venv
    fn spawn_agent(&self) -> Result<Child, String> {
        let venv_python = PathBuf::from(r"D:\hermes-agent-forAI-Hel2\.venv\Scripts\python.exe");
        let program: PathBuf;
        let args: Vec<&str>;

        if venv_python.exists() {
            program = venv_python;
            args = vec!["-m", "hermes_cli.main", "gateway", "run", "--replace"];
        } else {
            // Fallback: system hermes CLI
            let python = self.python_path();
            log::info!("D:\\hermes-agent-forAI-Hel2 venv not found, falling back to {}", python.display());
            program = python;
            args = vec!["-m", "hermes_cli.main", "gateway", "run", "--replace"];
        }

        log::info!("Spawning Agent: {} {}", program.display(), args.join(" "));

        let mut cmd = Command::new(&program);
        cmd.args(&args);

        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null());

        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Pass HERMES_HOME so the gateway finds config.yaml and .env
        // Prefer D:\ai-hel2-data; fall back to C:\.ai-hel2
        let hermes_home = if std::path::Path::new(r"D:\ai-hel2-data").exists() {
            r"D:\ai-hel2-data"
        } else {
            self.hermes_home.to_str().unwrap_or(r"C:\Users\58451\.ai-hel2")
        };
        cmd.env("HERMES_HOME", hermes_home);
        // Safety net: override config.yaml model settings so even if
        // config is corrupted/reverted, the gateway uses the right provider
        cmd.env("HERMES_INFERENCE_PROVIDER", "deepseek");
        cmd.env("API_SERVER_MODEL_NAME", "deepseek-v4-flash");
        cmd.env("API_SERVER_KEY", "aihel2-local-dev");

        // Force Git Bash over WSL bash on Windows
        #[cfg(windows)]
        {
            let git_bash = std::env::var("ProgramFiles")
                .map(|pf| format!("{pf}\\Git\\bin\\bash.exe"))
                .unwrap_or_else(|_| r"C:\Program Files\Git\bin\bash.exe".to_string());
            if std::path::Path::new(&git_bash).exists() {
                cmd.env("HERMES_GIT_BASH_PATH", &git_bash);
            }
        }

        cmd.spawn().map_err(|e| format!("Failed to spawn Agent: {e}"))
    }

    /// Detect the inference provider from .env.
    /// Checks HERMES_INFERENCE_PROVIDER first, then falls back to *_API_KEY entries.
    fn detect_inference_provider(&self) -> Option<String> {
        let env_path = self.hermes_home.join(".env");
        if !env_path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&env_path).ok()?;
        // Check for explicit HERMES_INFERENCE_PROVIDER first
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(val) = line.strip_prefix("HERMES_INFERENCE_PROVIDER=") {
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
        // Fallback: detect from *_API_KEY entries
        let known_keys: &[(&str, &str)] = &[
            ("DEEPSEEK_API_KEY", "deepseek"),
            ("ANTHROPIC_API_KEY", "anthropic"),
            ("OPENAI_API_KEY", "openai"),
            ("OPENROUTER_API_KEY", "openrouter"),
            ("ZAI_API_KEY", "zai"),
            ("KIMI_API_KEY", "kimi-coding"),
        ];
        for (env_key, provider) in known_keys {
            let prefix = format!("{}=", env_key);
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with(&prefix) {
                    let val = line[prefix.len()..].trim().trim_matches('"').trim_matches('\'');
                    if !val.is_empty() {
                        return Some(provider.to_string());
                    }
                }
            }
        }
        None
    }

    /// Quick health check — non-blocking HTTP GET to /health.
    pub fn health_check(&self) -> Result<bool, String> {
        let url = format!("{}/health", self.api_url);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;

        match client.get(&url).send() {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    /// Check if agent process is alive (PID check as fallback).
    fn is_process_alive(&self) -> bool {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(ref mut child) = *guard {
                match child.try_wait() {
                    Ok(None) => return true,  // still running
                    Ok(Some(_)) => {
                        // exited — collect exit status
                        let _ = guard.take();
                        return false;
                    }
                    Err(_) => return false,
                }
            }
        }
        // No managed child — check PID file
        self.check_pid_file()
    }

    /// Fallback: check gateway.pid file.
    /// Supports both plain integer and JSON `{"pid": 1234}` formats.
    fn check_pid_file(&self) -> bool {
        if let Some(pid) = self.read_pid_from_file() {
            // Signal 0 check — only works on Unix
            #[cfg(unix)]
            unsafe {
                let ret = libc::kill(pid as i32, 0);
                return ret == 0;
            }
            #[cfg(not(unix))]
            {
                // On Windows, PID existence is best-effort
                let _ = pid;
                return true;
            }
        }
        false
    }

    /// Full status with version info.
    pub fn status(&self) -> AgentStatus {
        let process_alive = self.is_process_alive();
        let healthy = process_alive && self.health_check().unwrap_or(false);

        let version = if healthy {
            self.fetch_version()
        } else {
            None
        };

        let pid = self.read_pid();

        AgentStatus {
            running: healthy,
            pid,
            port: self.port,
            version,
            healthy,
            error: if !process_alive {
                Some("Agent process not running".into())
            } else if !healthy {
                Some("Agent process alive but /health endpoint not responding".into())
            } else {
                None
            },
        }
    }

    pub(crate) fn fetch_version(&self) -> Option<String> {
        let url = format!("{}/health", self.api_url);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS))
            .build()
            .ok()?;
        let resp = client.get(&url).send().ok()?;
        let body: serde_json::Value = resp.json().ok()?;
        body.get("version")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    fn read_pid(&self) -> Option<u32> {
        if let Ok(guard) = self.child.lock() {
            if let Some(ref child) = *guard {
                return Some(child.id());
            }
        }
        self.read_pid_from_file()
    }

    /// Read PID from gateway.pid, supporting both plain integer and JSON formats.
    /// Mirrors upstream hermes.ts:849-855.
    fn read_pid_from_file(&self) -> Option<u32> {
        let pid_path = self.hermes_home.join("gateway.pid");
        let content = fs::read_to_string(&pid_path).ok()?;
        let trimmed = content.trim();
        // Try JSON format first: {"pid": 1234, ...}
        if trimmed.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(pid) = v.get("pid").and_then(|p| p.as_u64()) {
                    return Some(pid as u32);
                }
            }
        }
        // Fallback: plain integer
        trimmed.parse::<u32>().ok()
    }

    /// Start the agent: spawn process, then poll /health until ready or timeout.
    pub fn start(&self) -> Result<(), String> {
        if self.health_check().unwrap_or(false) {
            log::info!("Agent already running on port {}", self.port);
            return Ok(());
        }

        // Auto-configure API server in config.yaml if missing (upstream hermes.ts:130-150)
        self.ensure_api_server_config();

        let child = self.spawn_agent()?;
        log::info!("Agent spawned (pid={})", child.id());

        {
            let mut guard = self.child.lock().map_err(|e| e.to_string())?;
            *guard = Some(child);
        }

        // Track gateway start time for health check window (upstream hermes.ts:695-698)
        if let Ok(mut t) = self.gateway_start_time.lock() {
            *t = Some(Instant::now());
        }

        self.wait_until_ready(Duration::from_secs(STARTUP_TIMEOUT_SECS))
    }

    /// Returns true if the gateway was started within the last 8 seconds.
    /// Used to decide whether to wait for API readiness vs failing fast.
    /// Mirrors upstream hermes.ts:695-698.
    pub fn gateway_recently_started(&self) -> bool {
        if let Ok(guard) = self.gateway_start_time.lock() {
            if let Some(t) = *guard {
                return t.elapsed().as_secs() < 8;
            }
        }
        false
    }

    /// Ensure API server is configured in config.yaml.
    /// If api_server section is missing, append it.
    fn ensure_api_server_config(&self) {
        let config_path = self.hermes_home.join("config.yaml");
        if !config_path.exists() {
            return;
        }
        let content = match fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        if content.contains("api_server") {
            return;
        }
        let addition = format!(
            "\n# Desktop app API server (auto-configured by AI-Hel2)\n\
             platforms:\n  api_server:\n    enabled: true\n    extra:\n\
             \x20     port: {}\n      host: \"127.0.0.1\"\n\
             \x20     key: \"aihel2-local-dev\"\n\
             \x20     model_name: \"deepseek-v4-flash\"\n",
            self.port
        );
        if let Err(e) = fs::OpenOptions::new().append(true).open(&config_path)
            .and_then(|mut f| f.write_all(addition.as_bytes()))
        {
            log::warn!("Failed to auto-configure api_server in config.yaml: {e}");
        } else {
            log::info!("Auto-configured api_server in config.yaml (port {})", self.port);
        }
    }

    /// Poll /health endpoint every 500ms until ready or timeout.
    fn wait_until_ready(&self, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if self.health_check().unwrap_or(false) {
                log::info!(
                    "Agent ready on port {} after {:.1}s",
                    self.port,
                    start.elapsed().as_secs_f32()
                );
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(HEALTH_POLL_INTERVAL_MS));
        }

        Err(format!(
            "Agent did not become ready within {}s on port {}",
            timeout.as_secs(),
            self.port
        ))
    }

    /// Gracefully shut down the agent.
    pub fn shutdown(&self) {
        let mut guard = match self.child.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        let Some(mut child) = guard.take() else {
            log::info!("No agent child process to shut down");
            return;
        };

        let pid = child.id();
        log::info!("Shutting down Agent (pid={pid})...");

        // Try graceful kill first
        if let Err(e) = child.kill() {
            log::warn!("Failed to kill agent process: {e}");
        }

        // Wait for exit with grace period
        let start = std::time::Instant::now();
        let grace = Duration::from_secs(SHUTDOWN_GRACE_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    log::info!("Agent exited with status: {status:?}");
                    break;
                }
                Ok(None) if start.elapsed() < grace => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                _ => {
                    log::warn!("Agent did not exit within grace period, force killing...");
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }

        // Clean up PID file
        let pid_path = self.hermes_home.join("gateway.pid");
        if pid_path.exists() {
            let _ = fs::remove_file(&pid_path);
        }

        // Reset gateway start time
        if let Ok(mut t) = self.gateway_start_time.lock() {
            *t = None;
        }
    }

    /// Restart the agent with a brief delay between stop and start.
    pub fn restart(&self) -> Result<(), String> {
        log::info!("Restarting Agent...");
        self.shutdown();
        std::thread::sleep(Duration::from_millis(500));
        self.start()
    }

    /// Periodic health check with automatic restart on failure.
    /// Returns true if the gateway is healthy (or was successfully restarted).
    /// Tracks consecutive failures and stops retrying after MAX_AUTO_RESTART_ATTEMPTS.
    pub fn try_auto_restart(&self) -> bool {
        if self.health_check().unwrap_or(false) {
            // Healthy — reset failure counter
            self.consecutive_failures.store(0, Ordering::SeqCst);
            return true;
        }

        let failures = self.consecutive_failures.load(Ordering::SeqCst) + 1;
        self.consecutive_failures.store(failures, Ordering::SeqCst);

        if failures > MAX_AUTO_RESTART_ATTEMPTS {
            log::error!(
                "[auto_restart] gateway unhealthy for {} consecutive checks (max {}), giving up",
                failures - 1,
                MAX_AUTO_RESTART_ATTEMPTS
            );
            return false;
        }

        log::warn!(
            "[auto_restart] gateway unhealthy (attempt {}/{}), restarting...",
            failures,
            MAX_AUTO_RESTART_ATTEMPTS
        );

        match self.restart() {
            Ok(()) => {
                log::info!("[auto_restart] gateway restarted successfully");
                self.consecutive_failures.store(0, Ordering::SeqCst);
                true
            }
            Err(e) => {
                log::error!("[auto_restart] gateway restart failed: {e}");
                false
            }
        }
    }

    /// Read recent agent stderr log lines.
    pub fn recent_logs(&self, lines: usize) -> Vec<String> {
        let log_path = self.hermes_home.join("gateway-stderr.log");
        if !log_path.exists() {
            return vec!["(no agent log file)".into()];
        }
        let file = match fs::File::open(&log_path) {
            Ok(f) => f,
            Err(e) => return vec![format!("Cannot open log: {e}")],
        };
        let reader = BufReader::new(file);
        let all: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
        let start = if all.len() > lines { all.len() - lines } else { 0 };
        all[start..].to_vec()
    }
}

fn read_agent_port(hermes_home: &std::path::Path) -> u16 {
    let config_path = hermes_home.join("config.yaml");
    if !config_path.exists() {
        return DEFAULT_PORT;
    }
    match fs::read_to_string(&config_path) {
        Ok(content) => match serde_yaml::from_str::<GatewayConfig>(&content) {
            Ok(cfg) => cfg
                .platforms
                .and_then(|p| p.api_server)
                .and_then(|a| {
                    if a.enabled.unwrap_or(true) {
                        a.extra.and_then(|e| e.port)
                    } else {
                        None
                    }
                })
                .unwrap_or(DEFAULT_PORT),
            Err(_) => DEFAULT_PORT,
        },
        Err(_) => DEFAULT_PORT,
    }
}
