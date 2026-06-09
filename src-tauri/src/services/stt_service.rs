use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Child, ChildStdin, ChildStdout, Stdio};
use std::sync::Mutex;
use std::fs;
use log;

pub struct SttService {
    daemon: Mutex<Option<DaemonProcess>>,
    active_stop_file: Mutex<Option<String>>,
}

struct DaemonProcess {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
}

impl SttService {
    pub fn new() -> Self {
        Self {
            daemon: Mutex::new(None),
            active_stop_file: Mutex::new(None),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.active_stop_file.lock().unwrap().is_some()
    }

    fn asr_script_path() -> String {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .join("src")
            .join("services")
            .join("asr_service.py")
            .to_str()
            .unwrap()
            .to_string()
    }

    fn model_dir() -> String {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .join("..")
            .join("models")
            .join("sherpa-onnx-streaming-zipformer-ctc-zh")
            .to_str()
            .unwrap()
            .to_string()
    }

    /// Spawn the ASR daemon if not already running. Blocks until READY.
    /// Public so prewarm_voice_model can trigger early daemon startup.
    pub fn ensure_daemon(&self, python: &str) -> Result<(), String> {
        let mut guard = self.daemon.lock().unwrap();

        // Check if existing daemon is still alive
        if let Some(ref mut daemon) = *guard {
            match daemon.child.try_wait() {
                Ok(None) => return Ok(()), // still running
                Ok(Some(status)) => {
                    log::warn!("[ASR] Daemon exited unexpectedly ({}), restarting...", status);
                }
                Err(_) => {
                    log::warn!("[ASR] Cannot check daemon status, restarting...");
                }
            }
            *guard = None;
        }

        let script = Self::asr_script_path();
        let model_dir = Self::model_dir();
        let tokens = format!("{}/tokens.txt", model_dir);
        let model = format!("{}/model.onnx", model_dir);

        let mut child = Command::new(python)
            .env("PYTHONIOENCODING", "utf-8")
            .arg(&script)
            .arg("--tokens").arg(&tokens)
            .arg("--model").arg(&model)
            .arg("--num-threads").arg("4")
            .arg("--daemon")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("无法启动ASR守护进程: {e}"))?;

        let pid = child.id();
        let stdin = child.stdin.take().ok_or("无法获取守护进程stdin")?;
        let stdout = child.stdout.take().ok_or("无法获取守护进程stdout")?;
        let mut stdout_reader = BufReader::new(stdout);

        // Wait for READY signal (model loaded)
        let mut ready_line = String::new();
        stdout_reader.read_line(&mut ready_line)
            .map_err(|e| format!("守护进程启动失败: {e}"))?;

        if !ready_line.trim().eq("READY") {
            let _ = child.kill();
            return Err(format!("守护进程返回异常: {}", ready_line.trim()));
        }

        log::info!("[ASR] Daemon ready (pid={}, model preloaded)", pid);

        *guard = Some(DaemonProcess {
            stdin,
            stdout: stdout_reader,
            child,
        });

        Ok(())
    }

    /// Start recording. In daemon mode this sends a RECORD command via stdin
    /// and returns immediately — the daemon records in the background.
    pub fn start(&self, python: &str, max_duration: f64) -> Result<(), String> {
        if self.is_recording() {
            return Err("已在录音中".into());
        }

        self.ensure_daemon(python)?;

        let tmp_dir = std::env::temp_dir();
        let uuid = uuid::Uuid::new_v4().to_string();
        let stop_file = tmp_dir.join(format!("asr_{}.stop", uuid));
        let stop_file_str = stop_file.to_str().unwrap().to_string();
        let _ = fs::remove_file(&stop_file);

        let mut guard = self.daemon.lock().unwrap();
        let daemon = guard.as_mut().ok_or("守护进程未运行")?;

        let cmd = format!("RECORD {} {}\n", max_duration, stop_file_str);
        daemon.stdin.write_all(cmd.as_bytes())
            .map_err(|e| format!("发送录制指令失败: {e}"))?;
        daemon.stdin.flush()
            .map_err(|e| format!("刷新指令失败: {e}"))?;

        log::info!("[ASR] Recording started (stop-file: {})", stop_file_str);
        *self.active_stop_file.lock().unwrap() = Some(stop_file_str);
        Ok(())
    }

    /// Wait for the daemon to finish recording and return transcribed text.
    /// Blocks until RESULT/ERROR line arrives on daemon stdout.
    pub fn wait_result(&self) -> Result<String, String> {
        let mut guard = self.daemon.lock().unwrap();
        let daemon = guard.as_mut().ok_or("守护进程未运行")?;

        let mut line = String::new();
        daemon.stdout.read_line(&mut line)
            .map_err(|e| format!("读取识别结果失败: {e}"))?;

        // Clean up stop file
        if let Some(sf) = self.active_stop_file.lock().unwrap().take() {
            let _ = fs::remove_file(&sf);
        }

        let trimmed = line.trim();
        if let Some(text) = trimmed.strip_prefix("RESULT ") {
            log::info!("[ASR] Result: '{}'", text);
            Ok(text.to_string())
        } else if let Some(err) = trimmed.strip_prefix("ERROR ") {
            log::error!("[ASR] Error: {}", err);
            Err(err.to_string())
        } else if trimmed.is_empty() {
            Ok(String::new())
        } else {
            log::warn!("[ASR] Unexpected daemon output: {}", trimmed);
            Ok(trimmed.to_string())
        }
    }

    /// Stop recording by creating the stop file, then wait for the result.
    /// The daemon detects the stop file in its recording loop and returns text.
    pub fn stop(&self) -> Result<String, String> {
        // Create stop file to signal the daemon
        let stop_file = self.active_stop_file.lock().unwrap().clone();
        match stop_file {
            Some(ref sf) => {
                fs::write(sf, "stop").ok();
                log::info!("[ASR] Stop signal sent via {}", sf);
            }
            None => return Err("未在录音".into()),
        }

        // Wait for the daemon to process and return result
        self.wait_result()
    }

    /// Kill the daemon process (called on app shutdown). Idempotent.
    pub fn shutdown(&self) {
        let mut daemon_opt = match self.daemon.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return, // mutex poisoned, can't recover
        };
        if let Some(mut daemon) = daemon_opt {
            let _ = daemon.stdin.write_all(b"STOP\n");
            let _ = daemon.stdin.flush();
            let _ = daemon.child.wait();
            log::info!("[ASR] Daemon shut down");
        }
    }
}

impl Drop for SttService {
    fn drop(&mut self) {
        self.shutdown();
    }
}
