use std::process::{Command, Child};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use cpal::traits::{DeviceTrait, HostTrait};
use tauri::{AppHandle, Emitter, Manager};
use crate::services::stt_service::SttService;
use crate::commands::chat::AgentState;

pub struct SttState {
    pub service: std::sync::Mutex<SttService>,
    /// Active barge-in monitor child process (if running)
    pub barge_in_child: std::sync::Mutex<Option<Child>>,
}

/// Detect a usable Python interpreter, trying multiple candidates.
fn python_exe() -> PathBuf {
    let scripts = if cfg!(windows) { "Scripts" } else { "bin" };

    // Priority 1: Bundled hermes-agent venv in project
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for depth in &["", "..", ".."] {
        let candidate = project_root.join(depth).join("hermes-agent").join("venv").join(scripts).join("python.exe");
        if candidate.exists() {
            return candidate;
        }
    }

    // Priority 2: System python/python3
    for name in &["python", "python3"] {
        let path = if cfg!(windows) {
            PathBuf::from(format!("{name}.exe"))
        } else {
            PathBuf::from(*name)
        };
        // Quick check if it's on PATH by trying to run --version
        if Command::new(&path).arg("--version").output().is_ok() {
            return path;
        }
    }

    // Fallback: Windows Python launcher
    if cfg!(windows) {
        PathBuf::from("py")
    } else {
        PathBuf::from("python3")
    }
}

/// Strip Markdown formatting for clean TTS speech.
fn strip_markdown(text: &str) -> String {
    let mut s = text.to_string();

    // Remove code blocks (```...```)
    s = regex::Regex::new(r"(?s)```[^`]*```").unwrap().replace_all(&s, "").to_string();
    // Remove inline code (`...`)
    s = regex::Regex::new(r"`([^`]+)`").unwrap().replace_all(&s, "$1").to_string();
    // Remove images (![alt](url))
    s = regex::Regex::new(r"!\[[^\]]*\]\([^)]+\)").unwrap().replace_all(&s, "").to_string();
    // Remove link syntax, keep text: [text](url) → text
    s = regex::Regex::new(r"\[([^\]]*)\]\([^)]+\)").unwrap().replace_all(&s, "$1").to_string();
    // Remove bold/italic markers
    s = regex::Regex::new(r"\*\*([^*]+)\*\*").unwrap().replace_all(&s, "$1").to_string();
    s = regex::Regex::new(r"\*([^*]+)\*").unwrap().replace_all(&s, "$1").to_string();
    // Remove heading markers (# ## etc.)
    s = regex::Regex::new(r"(?m)^#{1,6}\s+").unwrap().replace_all(&s, "").to_string();
    // Remove horizontal rules
    s = regex::Regex::new(r"(?m)^[-*_]{3,}\s*$").unwrap().replace_all(&s, "").to_string();
    // Remove blockquote markers
    s = regex::Regex::new(r"(?m)^>\s+").unwrap().replace_all(&s, "").to_string();
    // Collapse multiple newlines
    s = regex::Regex::new(r"\n{3,}").unwrap().replace_all(&s, "\n\n").to_string();
    // Trim
    s.trim().to_string()
}

fn tts_model_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("models")
        .join("sherpa-onnx-vits-zh-ll")
}

fn tts_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("services")
        .join("tts_service.py")
}

/// Generate speech audio using sherpa-onnx VITS Chinese TTS (Apache 2.0).
/// Returns base64-encoded WAV data.
#[tauri::command]
pub async fn tts_speak(
    text: String,
    voice: Option<String>,
) -> Result<String, String> {
    // voice param now maps to speaker ID (0-4)
    let speaker: u8 = voice
        .as_deref()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(0)
        .min(4);

    let clean_text = strip_markdown(&text);

    log::info!(
        "TTS speak: {} chars, text='{}'",
        clean_text.len(),
        &clean_text[..clean_text.len().min(120)]
    );

    // Sequential segment TTS on the frontend handles long text by splitting at
    // sentence boundaries. Each backend call receives a single short segment.
    let final_text = clean_text;

    let tmp_dir = std::env::temp_dir();
    let filename = format!("tts_{}.wav", uuid::Uuid::new_v4());
    let output_path = tmp_dir.join(&filename);

    let model_dir = tts_model_dir();
    let python = python_exe();
    let script = tts_script_path();

    let mut child = Command::new(&python)
        .env("PYTHONIOENCODING", "utf-8")
        .arg(script.to_str().unwrap())
        .arg("--tokens").arg(model_dir.join("tokens.txt").to_str().unwrap())
        .arg("--model").arg(model_dir.join("model.onnx").to_str().unwrap())
        .arg("--lexicon").arg(model_dir.join("lexicon.txt").to_str().unwrap())
        .arg("--dict-dir").arg(model_dir.join("dict").to_str().unwrap())
        .arg("--speaker").arg(speaker.to_string())
        .arg("--speed").arg("1.1")
        .arg("--output").arg(output_path.to_str().unwrap())
        .arg("--num-threads").arg("4")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动TTS进程 ({}): {e}", python.display()))?;

    // Write text to stdin
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(final_text.as_bytes()).ok();
    }

    let output = child.wait_with_output()
        .map_err(|e| format!("TTS进程异常: {e}"))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&output_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("TTS合成失败: {}", stderr.lines().last().unwrap_or("")));
    }

    let bytes = std::fs::read(&output_path)
        .map_err(|e| format!("无法读取TTS音频: {e}"))?;
    let _ = std::fs::remove_file(&output_path);

    Ok(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes))
}

/// Preview a TTS speaker voice with a short fixed phrase.
#[tauri::command]
pub async fn tts_preview(speaker: u8) -> Result<String, String> {
    tts_speak("你好，这是我的声音".to_string(), Some(speaker.to_string())).await
}

/// Check whether voice dependencies (sherpa-onnx, sounddevice, numpy) are installed.
/// Returns a list of missing packages, or empty vec if all OK.
#[tauri::command]
pub async fn check_voice_deps() -> Result<Vec<String>, String> {
    let python = python_exe();
    let mut missing: Vec<String> = Vec::new();

    // Check sherpa-onnx (ASR + TTS)
    let asr_check = Command::new(&python)
        .args(["-c", "import sherpa_onnx"])
        .output();
    match asr_check {
        Ok(o) if o.status.success() => {}
        _ => missing.push("sherpa-onnx (pip install sherpa-onnx)".into()),
    }

    // Check sounddevice (mic recording)
    let sd_check = Command::new(&python)
        .args(["-c", "import sounddevice"])
        .output();
    match sd_check {
        Ok(o) if o.status.success() => {}
        _ => missing.push("sounddevice (pip install sounddevice)".into()),
    }

    // Check numpy (audio processing)
    let np_check = Command::new(&python)
        .args(["-c", "import numpy"])
        .output();
    match np_check {
        Ok(o) if o.status.success() => {}
        _ => missing.push("numpy (pip install numpy)".into()),
    }

    // Check scipy (audio resampling)
    let scipy_check = Command::new(&python)
        .args(["-c", "from scipy import signal"])
        .output();
    match scipy_check {
        Ok(o) if o.status.success() => {}
        _ => missing.push("scipy (pip install scipy)".into()),
    }

    Ok(missing)
}

/// Pre-warm the sherpa-onnx ASR daemon (load model once, keep it resident).
/// This replaces the old one-shot model preload — the daemon stays alive
/// so every subsequent voice request skips the 1.7s model-load overhead.
#[tauri::command]
pub async fn prewarm_voice_model(
    state: tauri::State<'_, SttState>,
) -> Result<String, String> {
    let model_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..").join("models")
        .join("sherpa-onnx-streaming-zipformer-ctc-zh");
    let tokens = model_dir.join("tokens.txt");
    let model = model_dir.join("model.onnx");

    if !tokens.exists() || !model.exists() {
        return Err("ASR模型文件未找到，请先下载模型".into());
    }

    let python = python_exe();
    let py_str = python.to_str().unwrap_or("python");

    // This spawns the daemon and waits for READY (model loaded)
    let svc = state.service.lock().map_err(|e| format!("Lock error: {e}"))?;
    svc.ensure_daemon(py_str)?;

    Ok("model loaded (daemon mode)".into())
}

/// Diagnostic: list audio input devices and check Python setup.
#[tauri::command]
pub async fn voice_diagnose() -> Result<String, String> {
    let mut lines: Vec<String> = Vec::new();

    // Python info
    let py = python_exe();
    lines.push(format!("Python: {}", py.display()));
    match Command::new(&py).arg("--version").output() {
        Ok(o) => lines.push(format!("  version: {}", String::from_utf8_lossy(&o.stdout).trim())),
        Err(e) => lines.push(format!("  error: {e}")),
    }

    // Audio devices
    let host = cpal::default_host();
    match host.default_input_device() {
        Some(d) => {
            lines.push(format!("Default input: {}", d.name().unwrap_or_default()));
            match d.default_input_config() {
                Ok(c) => lines.push(format!("  {}ch, {}Hz, {:?}", c.channels(), c.sample_rate().0, c.sample_format())),
                Err(e) => lines.push(format!("  config error: {e}")),
            }
        }
        None => lines.push("Default input: NONE".into()),
    }

    // List all input devices
    match host.input_devices() {
        Ok(devices) => {
            for (i, d) in devices.enumerate() {
                let name = d.name().unwrap_or_default();
                let configs: Vec<String> = match d.supported_input_configs() {
                    Ok(cfgs) => cfgs.map(|c| format!("{}ch/{}Hz", c.channels(), c.max_sample_rate().0)).collect(),
                    Err(_) => vec!["error".into()],
                };
                lines.push(format!("  [{i}] {name}: {configs:?}"));
            }
        }
        Err(e) => lines.push(format!("Input devices error: {e}")),
    }

    Ok(lines.join("\n"))
}

/// Start microphone recording with sherpa-onnx streaming ASR.
/// Recording continues until silence or stop file triggers the daemon.
#[tauri::command]
pub async fn voice_start_listening(
    state: tauri::State<'_, SttState>,
) -> Result<(), String> {
    let svc = state.service.lock().map_err(|e| format!("Lock error: {e}"))?;
    let python = python_exe();
    let py_str = python.to_str().unwrap_or("python");
    svc.start(py_str, 30.0)
}

/// Stop recording and return transcribed text from sherpa-onnx ASR.
#[tauri::command]
pub async fn voice_stop_listening(
    state: tauri::State<'_, SttState>,
) -> Result<String, String> {
    let svc = state.service.lock().map_err(|e| format!("Lock error: {e}"))?;
    svc.stop()
}

/// Combined record + transcribe: click once, speak, auto-stop on endpoint/silence.
/// Uses daemon mode: model is preloaded, only recording + recognition overhead.
/// Lock is released between start and wait so voice_stop_listening can interrupt.
#[tauri::command]
pub async fn voice_listen_once(
    state: tauri::State<'_, SttState>,
) -> Result<String, String> {
    let python_str = python_exe().to_str().unwrap_or("python").to_string();

    // Phase 1: Start recording (fast, lock held briefly)
    {
        let svc = state.service.lock().map_err(|e| format!("Lock error: {e}"))?;
        svc.start(&python_str, 30.0)?;
    }

    log::info!("[Voice] ASR listen_once started, waiting for silence...");

    // Phase 2: Wait for daemon result (lock released so voice_stop_listening can interrupt)
    let text = {
        let svc = state.service.lock().map_err(|e| format!("Lock error: {e}"))?;
        svc.wait_result()?
    };

    log::info!("[Voice] ASR listen_once completed, text length: {}", text.len());
    Ok(text)
}

/// Start barge-in monitor during TTS playback.
/// Spawns barge_in.py in background. When speech is detected, triggers
/// the interrupt chain: abort chat → emit event so frontend stops TTS.
#[tauri::command]
pub async fn voice_start_barge_in_monitor(
    app: AppHandle,
    state: tauri::State<'_, SttState>,
) -> Result<(), String> {
    // Kill any existing barge-in monitor
    {
        let mut guard = state.barge_in_child.lock().map_err(|e| format!("Lock error: {e}"))?;
        if let Some(ref mut child) = *guard {
            let _ = child.kill();
        }
        *guard = None;
    }

    let python = python_exe();
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("services")
        .join("barge_in.py");

    let child = Command::new(&python)
        .env("PYTHONIOENCODING", "utf-8")
        .arg(script.to_str().unwrap())
        .arg("--threshold").arg("0.008")
        .arg("--confirm-frames").arg("5")
        .arg("--timeout").arg("10")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("无法启动barge-in监控: {e}"))?;

    log::info!("[BargeIn] Monitor started (pid={})", child.id());

    // Store child handle so we can kill it later
    {
        let mut guard = state.barge_in_child.lock().map_err(|e| format!("Lock error: {e}"))?;
        *guard = Some(child);
    }

    // Spawn background thread to wait for barge-in result
    let app_clone = app.clone();
    std::thread::spawn(move || {
        // We need to take the child out of state to wait on it
        let child_opt = {
            // Access state via app handle — careful with lifetimes
            match app_clone.try_state::<SttState>() {
                Some(stt) => {
                    match stt.barge_in_child.lock() {
                        Ok(mut guard) => guard.take(),
                        Err(_) => None,
                    }
                }
                None => None,
            }
        };

        let Some(child) = child_opt else {
            return;
        };

        match child.wait_with_output() {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                log::info!("[BargeIn] Result: {text}");
                if text.contains("SPEECH") {
                    // Trigger interrupt: set cancel_flag to abort SSE streaming
                    if let Some(agent) = app_clone.try_state::<AgentState>() {
                        agent.cancel_flag.store(true, Ordering::SeqCst);
                    }
                    // Emit event so frontend stops TTS and optionally starts listening
                    let _ = app_clone.emit("voice:interrupted", true);
                    log::info!("[BargeIn] Interrupt chain triggered");
                }
            }
            Err(e) => {
                log::warn!("[BargeIn] Monitor error: {e}");
            }
        }
    });

    Ok(())
}

/// Stop a running barge-in monitor (called when TTS finishes normally).
#[tauri::command]
pub async fn voice_stop_barge_in_monitor(
    state: tauri::State<'_, SttState>,
) -> Result<(), String> {
    let mut guard = state.barge_in_child.lock().map_err(|e| format!("Lock error: {e}"))?;
    if let Some(ref mut child) = *guard {
        let _ = child.kill();
        log::info!("[BargeIn] Monitor stopped");
    }
    *guard = None;
    Ok(())
}

