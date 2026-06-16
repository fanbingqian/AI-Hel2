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
    resource_dir: Mutex<Option<PathBuf>>,
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
            resource_dir: Mutex::new(None),
        }
    }

    pub fn set_resource_dir(&self, dir: PathBuf) {
        let mut rd = self.resource_dir.lock().unwrap();
        *rd = Some(dir);
    }

    /// Install the aihel plugin to ~/.hermes/plugins/ on first run.
    fn install_aihel_plugin(&self) {
        let agent_home = dirs_home().join(".hermes");
        let target = agent_home.join("plugins").join("aihel");
        if target.exists() { return; }

        // Find plugin source: check extracted hermes-agent, then app_dir
        let source = self.hermes_home.join("hermes-agent").join("plugins").join("aihel");
        let source = if source.exists() { source }
        else if let Some(app_dir) = self.app_dir() {
            app_dir.join("hermes-agent").join("plugins").join("aihel")
        } else { return; };

        if !source.exists() { return; }
        let _ = std::fs::create_dir_all(&target);
        for entry in std::fs::read_dir(&source).into_iter().flatten().flatten() {
            let src = entry.path();
            if src.extension().and_then(|e| e.to_str()) == Some("py") {
                let _ = std::fs::copy(&src, target.join(src.file_name().unwrap()));
            }
        }
        log::info!("AI-Hel plugin installed to {}", target.display());
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
    fn app_dir(&self) -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let exe_dir = exe.parent()?;

        let mut candidates: Vec<PathBuf> = vec![
            // production: hermes-agent/ next to the exe (NSIS copy)
            exe_dir.to_path_buf(),
        ];

        // hermes_home extraction (ZIP extracted here by extract_agent_zip)
        candidates.push(self.hermes_home.clone());

        // Tauri resource directory (bundled resources extracted here)
        if let Some(rd) = self.resource_dir.lock().ok().and_then(|r| r.clone()) {
            candidates.push(rd);
        }

        // cargo run: target/debug/../../ or target/release/../../
        candidates.push(exe_dir.join("..").join(".."));
        candidates.push(exe_dir.join("..").join("..").join(".."));
        // Project root relative to Cargo.toml (src-tauri/)
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));

        for dir in &candidates {
            let agent_dir = dir.join("hermes-agent");
            let main_py = agent_dir.join("hermes_cli").join("main.py");
            log::info!("[AgentManager] Checking: {} → exists={} main_py={}",
                agent_dir.display(), agent_dir.exists(), main_py.exists());
            if agent_dir.exists() && main_py.exists() {
                let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
                log::info!("[AgentManager] FOUND at: {}", canonical.display());
                return Some(canonical);
            }
        }

        // Last resort: try to extract from bundled ZIP
        // Check both resource dir AND exe dir for the ZIP
        let zip_dirs: Vec<PathBuf> = {
            let mut dirs = Vec::new();
            dirs.push(exe_dir.to_path_buf()); // D:\ where exe lives
            if let Some(rd) = self.resource_dir.lock().ok().and_then(|r| r.clone()) {
                dirs.push(rd); // C:\Users\...\AppData\Local\...
            }
            dirs
        };
        for zip_dir in &zip_dirs {
            if let Some(dir) = self.extract_agent_zip(zip_dir) {
                return Some(dir);
            }
        }

        log::error!("[AgentManager] hermes-agent NOT FOUND in any candidate directory");
        None
    }

    /// Extract hermes-agent.zip to hermes_home/hermes-agent on first run or update.
    /// Returns the parent directory containing the extracted hermes-agent.
    fn extract_agent_zip(&self, resource_dir: &std::path::Path) -> Option<PathBuf> {
        let zip_path = resource_dir.join("hermes-agent.zip");
        if !zip_path.exists() {
            return None;
        }
        // Extract to hermes_home so the agent persists across app updates
        // (the resource dir is wiped on each update, but hermes_home survives).
        let target_dir = self.hermes_home.join("hermes-agent");

        // Check if we need to re-extract: only if ZIP is newer or target missing
        let should_extract = if !target_dir.exists() {
            true
        } else {
            // Compare the ZIP modification time with a marker file we write after extraction
            let marker = target_dir.join(".extract_done");
            if !marker.exists() {
                true
            } else {
                let zip_mtime = std::fs::metadata(&zip_path).ok()
                    .and_then(|m| m.modified().ok());
                let marker_mtime = std::fs::metadata(&marker).ok()
                    .and_then(|m| m.modified().ok());
                match (zip_mtime, marker_mtime) {
                    (Some(z), Some(m)) => z > m,
                    _ => true,
                }
            }
        };

        if !should_extract {
            log::info!("[AgentManager] hermes-agent already up to date at {}", target_dir.display());
            return Some(self.hermes_home.clone());
        }

        // Remove old extraction if present
        if target_dir.exists() {
            let _ = fs::remove_dir_all(&target_dir);
        }

        log::info!("[AgentManager] Extracting {} to {} ...", zip_path.display(), target_dir.display());
        let file = match std::fs::File::open(&zip_path) {
            Ok(f) => f,
            Err(e) => { log::error!("[AgentManager] Cannot open ZIP: {}", e); return None; }
        };
        let mut archive = match zip::ZipArchive::new(file) {
            Ok(a) => a,
            Err(e) => { log::error!("[AgentManager] Cannot read ZIP: {}", e); return None; }
        };
        log::info!("[AgentManager] ZIP has {} files", archive.len());
        if let Err(e) = archive.extract(&target_dir) {
            log::error!("[AgentManager] Failed to extract: {}", e);
            return None;
        }

        // Write marker file to track extraction time
        let _ = fs::write(target_dir.join(".extract_done"), "ok");

        log::info!("[AgentManager] Extracted {} files to {}", archive.len(), target_dir.display());
        Some(self.hermes_home.clone())
    }

    /// Detect the Python executable path.
    /// Priority: embedded python → bundled venv → hermes_home venv → system python
    fn python_path(&self) -> PathBuf {
        let scripts = if cfg!(windows) { "Scripts" } else { "bin" };

        // Priority 0: Embedded portable Python (no venv, no paths — truly portable)
        if let Some(app_dir) = self.app_dir() {
            let embedded = app_dir.join("hermes-agent").join("python").join("python.exe");
            if embedded.exists() {
                log::info!("Using embedded Python: {}", embedded.display());
                return embedded;
            }
        }

        // Priority 1: Bundled hermes-agent/venv in app directory
        if let Some(app_dir) = self.app_dir() {
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
        if let Some(app_dir) = self.app_dir() {
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
        // Check dev venv first (dev machine only)
        let venv_python = PathBuf::from(r"D:\hermes-agent-forAI-Hel2\.venv\Scripts\python.exe");
        let python = if venv_python.exists() {
            venv_python
        } else {
            self.python_path()
        };
        log::info!("Spawning Agent with Python: {}", python.display());

        let mut cmd = Command::new(&python);
        cmd.arg("-u"); // unbuffered stdout/stderr — errors visible immediately
        // Use absolute script path to bypass Python module import issues
        if let Some(script) = self.find_agent_script() {
            let script_path = script.to_string_lossy().trim_start_matches("\\\\?\\").to_string();
            cmd.arg(&script_path).arg("gateway").arg("run").arg("--replace");
        } else {
            cmd.args(["-m", "hermes_cli.main", "gateway", "run", "--replace"]);
        }

        // Redirect stdout/stderr to log file for debugging
        let stderr_log = self.hermes_home.join("gateway-stderr.log");
        let log_file = fs::File::create(&stderr_log).ok();
        if let Some(f) = log_file {
            cmd.stdout(std::process::Stdio::from(f.try_clone().unwrap()))
                .stderr(std::process::Stdio::from(f))
                .stdin(std::process::Stdio::null());
        } else {
            cmd.stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null());
        }
        log::info!("Agent stderr log: {}", stderr_log.display());

        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Set PYTHONPATH so Python can find hermes_cli package
        if let Some(app_dir) = self.app_dir() {
            let agent_dir = app_dir.join("hermes-agent");
            if agent_dir.exists() {
                // Strip \\?\ prefix from canonicalized paths - Python doesn't handle it
                let pythonpath = agent_dir.to_string_lossy().trim_start_matches("\\\\?\\").to_string();
                cmd.env("PYTHONPATH", &pythonpath);
                log::info!("PYTHONPATH={}", pythonpath);
            }
        }

        // HERMES_HOME: Agent uses its own directory (~/.hermes)
        let agent_home = dirs_home().join(".hermes");
        let _ = std::fs::create_dir_all(&agent_home);
        let agent_config = agent_home.join("config.yaml");
        if !agent_config.exists() {
            let default_config = "model:\n  default: deepseek-v4-flash\nproviders:\n  deepseek:\n    base_url: \"https://api.deepseek.com\"\n    models:\n      - \"deepseek-v4-flash\"\n      - \"deepseek-v4-pro\"\n";
            let _ = std::fs::write(&agent_config, default_config);
        }
        cmd.env("HERMES_HOME", agent_home.to_str().unwrap_or("."));
        // Allow open access on localhost (no user auth required)
        cmd.env("GATEWAY_ALLOW_ALL_USERS", "true");

        // Enable the API server (OpenAI-compatible /v1/chat/completions endpoint)
        cmd.env("API_SERVER_ENABLED", "true");
        cmd.env("API_SERVER_HOST", "127.0.0.1");
        cmd.env("API_SERVER_PORT", &self.port.to_string());
        // Default provider settings (overridden by config.yaml if present)
        cmd.env("API_SERVER_KEY", "aihel2-local-dev");
        cmd.env("API_SERVER_ALLOW_ALL_USERS", "true");
        cmd.env("HERMES_INFERENCE_PROVIDER", "deepseek");
        cmd.env("HERMES_INFERENCE_MODEL", "deepseek-v4-flash");
        cmd.env("API_SERVER_MODEL_NAME", "deepseek-v4-flash");

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

    /// Quick health check — HTTP GET to /health with Bearer auth (API server requires it).
    pub fn health_check(&self) -> Result<bool, String> {
        let url = format!("{}/health", self.api_url);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;

        match client.get(&url).send() {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                log::warn!("Health check failed: {}", e);
                Ok(false)
            },
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
        let log_path = self.hermes_home.join("aihel2-startup.log");
        let mut log_file = fs::File::create(&log_path).ok();
        let mut log = |msg: &str| {
            if let Some(ref mut f) = log_file {
                let line = format!("{} {}\n", chrono::Local::now().format("%H:%M:%S"), msg);
                let _ = f.write_all(line.as_bytes());
            }
            log::info!("{}", msg);
        };

        log(&format!("=== AI-Hel2 Agent Startup ==="));
        log(&format!("HERMES_HOME: {}", self.hermes_home.display()));
        log(&format!("Port: {}", self.port));
        log(&format!("exe path: {:?}", std::env::current_exe()));

        if self.health_check().unwrap_or(false) {
            log("Agent already running");
            return Ok(());
        }

        // Auto-configure API server
        self.ensure_api_server_config();
        self.ensure_dot_env();
        log(&format!("config.yaml exists: {}", self.hermes_home.join("config.yaml").exists()));

        // Install aihel plugin to ~/.hermes/plugins/ on first run
        self.install_aihel_plugin();

        let python = self.python_path();
        log(&format!("Python path: {}", python.display()));
        log(&format!("Python exists: {}", python.exists()));

        let script = self.find_agent_script();
        log(&format!("Agent script: {:?}", script));
        log(&format!("Script exists: {}", script.as_ref().map(|s| s.exists()).unwrap_or(false)));

        let child = self.spawn_agent()?;
        log(&format!("Agent spawned (pid={})", child.id()));

        {
            let mut guard = self.child.lock().map_err(|e| e.to_string())?;
            *guard = Some(child);
        }

        // Track gateway start time for health check window (upstream hermes.ts:695-698)
        if let Ok(mut t) = self.gateway_start_time.lock() {
            *t = Some(Instant::now());
        }

        // Inline health polling with logged errors
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(STARTUP_TIMEOUT_SECS) {
            let url = format!("{}/health", self.api_url);
            match reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS))
                .build()
            {
                Ok(client) => match client.get(&url).send() {
                    Ok(resp) if resp.status().is_success() => {
                        log(&format!("Agent ready ({:.1}s)", start.elapsed().as_secs_f32()));
                        return Ok(());
                    }
                    Ok(resp) => log(&format!("Health returned {}", resp.status())),
                    Err(e) => log(&format!("Health check error: {}", e)),
                },
                Err(e) => log(&format!("HTTP client error: {}", e)),
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err("Agent did not become ready".to_string())
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
        let content = fs::read_to_string(&config_path).unwrap_or_default();
        let our_port = format!("port: {}", self.port);
        if content.contains("api_server") && content.contains(&our_port) {
            return; // Already configured with correct port
        }

        let api_cfg = format!(
            "platforms:\n  api_server:\n    enabled: true\n    extra:\n      port: {}\n      host: \"127.0.0.1\"\n      key: \"aihel2-local-dev\"\n      model_name: \"deepseek-v4-flash\"\n",
            self.port
        );

        if content.is_empty() {
            let _ = fs::create_dir_all(self.hermes_home.as_path());
            // Include default model + toolsets so the Gateway has full capabilities
            let full_cfg = format!(
                "model:\n  default: deepseek-v4-flash\n  max_tokens: 8192\n\
                 agent:\n  toolsets:\n    - hermes-cli\n    - search\n    - memory\n    - file\n    - nexus\n\
                 platform_toolsets:\n  api_server:\n    - hermes-cli\n    - search\n    - memory\n    - file\n    - nexus\n\n{}",
                api_cfg
            );
            if let Err(e) = fs::write(&config_path, &full_cfg) {
                log::warn!("Failed to create config.yaml: {e}");
            } else {
                log::info!("Created config.yaml with api_server on port {}", self.port);
            }
        } else {
            // Also ensure toolsets are present for existing configs
            let mut append = String::new();
            if !content.contains("platform_toolsets:") {
                append.push_str("\nagent:\n  toolsets:\n    - hermes-cli\n    - search\n    - memory\n    - file\n    - nexus\nplatform_toolsets:\n  api_server:\n    - hermes-cli\n    - search\n    - memory\n    - file\n    - nexus\n");
            }
            append.push_str(&format!("\n# Auto-configured by AI-Hel2\n{}", api_cfg));
            if let Err(e) = fs::OpenOptions::new().append(true).open(&config_path)
                .and_then(|mut f| f.write_all(append.as_bytes()))
            {
                log::warn!("Failed to append api_server config: {e}");
            } else {
                log::info!("Appended api_server config on port {}", self.port);
            }
        }
    }

    /// Ensure .env has required Gateway settings for local access.
    /// Writes to Agent's ~/.hermes/.env so the Agent process can read it.
    fn ensure_dot_env(&self) {
        let env_path = dirs_home().join(".hermes").join(".env");
        let mut content = fs::read_to_string(&env_path).unwrap_or_default();
        let mut changed = false;
        if !content.contains("GATEWAY_ALLOW_ALL_USERS") {
            content.push_str("\nGATEWAY_ALLOW_ALL_USERS=true\n");
            changed = true;
        }
        if !content.contains("API_SERVER_KEY") {
            content.push_str("API_SERVER_KEY=aihel2-local-dev\n");
            changed = true;
        }
        if changed {
            let _ = fs::create_dir_all(dirs_home().join(".hermes"));
            let _ = fs::write(&env_path, &content);
            log::info!("Wrote .env at {}", env_path.display());
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
