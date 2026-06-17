use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

use cpal::traits::{DeviceTrait, HostTrait};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use crate::services::whisper_service::{WhisperService, WhisperResult};

pub struct WhisperState {
    pub is_recording: Arc<AtomicBool>,
    pub stop_signal: Arc<AtomicBool>,
    /// Channel receiver for the background recording thread result.
    /// `start_ptt_recording` spawns the thread and stores rx here;
    /// `stop_ptt_recording` takes it and awaits the result.
    pub result_rx: std::sync::Mutex<Option<oneshot::Receiver<Result<WhisperResult, String>>>>,
}

// ════════════════════════════════════════════════════════════════
// Helpers (kept from original)
// ════════════════════════════════════════════════════════════════

/// Detect a usable Python interpreter, trying multiple candidates.
fn python_exe() -> PathBuf {
    let scripts = if cfg!(windows) { "Scripts" } else { "bin" };

    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for depth in &["", "..", ".."] {
        let candidate = project_root.join(depth).join("hermes-agent").join("venv").join(scripts).join("python.exe");
        if candidate.exists() {
            return candidate;
        }
    }

    for name in &["python", "python3"] {
        let path = if cfg!(windows) {
            PathBuf::from(format!("{name}.exe"))
        } else {
            PathBuf::from(*name)
        };
        let mut cmd = Command::new(&path);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        if cmd.arg("--version").output().is_ok() {
            return path;
        }
    }

    if cfg!(windows) { PathBuf::from("py") } else { PathBuf::from("python3") }
}

/// Strip Markdown formatting for clean TTS speech.
fn strip_markdown(text: &str) -> String {
    let mut s = text.to_string();
    s = regex::Regex::new(r"(?s)```[^`]*```").unwrap().replace_all(&s, "").to_string();
    s = regex::Regex::new(r"`([^`]+)`").unwrap().replace_all(&s, "$1").to_string();
    s = regex::Regex::new(r"!\[[^\]]*\]\([^)]+\)").unwrap().replace_all(&s, "").to_string();
    s = regex::Regex::new(r"\[([^\]]*)\]\([^)]+\)").unwrap().replace_all(&s, "$1").to_string();
    s = regex::Regex::new(r"\*\*([^*]+)\*\*").unwrap().replace_all(&s, "$1").to_string();
    s = regex::Regex::new(r"\*([^*]+)\*").unwrap().replace_all(&s, "$1").to_string();
    s = regex::Regex::new(r"(?m)^#{1,6}\s+").unwrap().replace_all(&s, "").to_string();
    s = regex::Regex::new(r"(?m)^[-*_]{3,}\s*$").unwrap().replace_all(&s, "").to_string();
    s = regex::Regex::new(r"(?m)^>\s+").unwrap().replace_all(&s, "").to_string();
    s = regex::Regex::new(r"\n{3,}").unwrap().replace_all(&s, "\n\n").to_string();
    s.trim().to_string()
}

fn tts_model_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..").join("models").join("sherpa-onnx-vits-zh-ll")
}

fn tts_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src").join("services").join("tts_service.py")
}

// ════════════════════════════════════════════════════════════════
// TTS Commands (unchanged from original)
// ════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn tts_speak(text: String, voice: Option<String>) -> Result<String, String> {
    let speaker: u8 = voice.as_deref().and_then(|v| v.parse::<u8>().ok()).unwrap_or(0).min(4);
    let clean_text = strip_markdown(&text);

    log::info!("TTS speak: {} chars", clean_text.len());

    let tmp_dir = std::env::temp_dir();
    let output_path = tmp_dir.join(format!("tts_{}.wav", uuid::Uuid::new_v4()));

    let model_dir = tts_model_dir();
    let python = python_exe();
    let script = tts_script_path();

    let mut cmd = Command::new(&python);
    cmd.env("PYTHONIOENCODING", "utf-8")
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
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)] { cmd.creation_flags(CREATE_NO_WINDOW); }
    let mut child = cmd.spawn()
        .map_err(|e| format!("无法启动TTS进程 ({}): {e}", python.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(clean_text.as_bytes()).ok();
    }

    let output = child.wait_with_output().map_err(|e| format!("TTS进程异常: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&output_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("TTS合成失败: {}", stderr.lines().last().unwrap_or("")));
    }

    let bytes = std::fs::read(&output_path).map_err(|e| format!("无法读取TTS音频: {e}"))?;
    let _ = std::fs::remove_file(&output_path);
    Ok(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes))
}

#[tauri::command]
pub async fn tts_preview(speaker: u8) -> Result<String, String> {
    tts_speak("你好，这是我的声音".to_string(), Some(speaker.to_string())).await
}

// ════════════════════════════════════════════════════════════════
// Dependency & Diagnostic Commands (adapted for whisper)
// ════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn check_voice_deps(app: AppHandle) -> Result<Vec<String>, String> {
    let python = python_exe();
    let mut missing: Vec<String> = Vec::new();

    // sherpa-onnx (TTS)
    let mut cmd = Command::new(&python);
    cmd.args(["-c", "import sherpa_onnx"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    match cmd.output() {
        Ok(o) if o.status.success() => {}
        _ => missing.push("sherpa-onnx (pip install sherpa-onnx)".into()),
    }

    // numpy (TTS)
    let mut cmd2 = Command::new(&python);
    cmd2.args(["-c", "import numpy"]);
    #[cfg(windows)]
    cmd2.creation_flags(CREATE_NO_WINDOW);
    match cmd2.output() {
        Ok(o) if o.status.success() => {}
        _ => missing.push("numpy (pip install numpy)".into()),
    }

    // whisper.cpp binary
    match WhisperService::find_whisper_exe(&app) {
        Ok(_) => {}
        Err(e) => missing.push(format!("whisper.cpp: {e}")),
    }

    // whisper model
    match WhisperService::find_model(&app) {
        Ok(_) => {}
        Err(e) => missing.push(format!("whisper model: {e}")),
    }

    Ok(missing)
}

#[tauri::command]
pub async fn voice_diagnose(app: AppHandle) -> Result<String, String> {
    let mut lines: Vec<String> = Vec::new();

    // Python
    let py = python_exe();
    lines.push(format!("Python: {}", py.display()));
    let mut cmd = Command::new(&py);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    match cmd.arg("--version").output() {
        Ok(o) => lines.push(format!("  version: {}", String::from_utf8_lossy(&o.stdout).trim())),
        Err(e) => lines.push(format!("  error: {e}")),
    }

    // whisper.cpp
    match WhisperService::find_whisper_exe(&app) {
        Ok(p) => lines.push(format!("whisper.cpp: {}", p.display())),
        Err(e) => lines.push(format!("whisper.cpp: NOT FOUND ({e})")),
    }
    match WhisperService::find_model(&app) {
        Ok(p) => lines.push(format!("Model: {}", p.display())),
        Err(e) => lines.push(format!("Model: NOT FOUND ({e})")),
    }

    // Audio devices
    let host = cpal::default_host();
    match host.default_input_device() {
        Some(d) => {
            lines.push(format!("Default input: {}", d.name().unwrap_or_default()));
            match d.default_input_config() {
                Ok(c) => lines.push(format!("  {}ch, {}Hz", c.channels(), c.sample_rate().0)),
                Err(e) => lines.push(format!("  config error: {e}")),
            }
        }
        None => lines.push("Default input: NONE".into()),
    }

    if let Ok(devices) = host.input_devices() {
        for (i, d) in devices.enumerate() {
            lines.push(format!("  [{i}] {}", d.name().unwrap_or_default()));
        }
    }

    Ok(lines.join("\n"))
}

// ════════════════════════════════════════════════════════════════
// PTT (Push-to-Talk) Commands — whisper-based
//
// Architecture:
//   start_ptt_recording  → spawns background recording thread → returns immediately
//   stop_ptt_recording   → signals stop → awaits thread result → returns transcription
//   cancel_ptt_recording → signals stop → discards result
//
// No overlay window is shown; the frontend VoiceInput component
// provides the visual recording state (waveform, timer, etc.).
// ════════════════════════════════════════════════════════════════

/// Start recording in a background thread.
/// Recording continues until `stop_ptt_recording` or `cancel_ptt_recording` is called,
/// or until the max duration (30s) is reached.
#[tauri::command]
pub async fn start_ptt_recording(
    app: AppHandle,
    state: tauri::State<'_, WhisperState>,
) -> Result<(), String> {
    if state.is_recording.load(Ordering::SeqCst) {
        return Err("Already recording".into());
    }

    state.is_recording.store(true, Ordering::SeqCst);
    state.stop_signal.store(false, Ordering::SeqCst);

    let stop_signal = state.stop_signal.clone();
    let app_handle = app.clone();

    // Create a oneshot channel so stop_ptt_recording can await the result
    let (tx, rx) = oneshot::channel();

    // Store the receiver so stop_ptt_recording can take it
    *state.result_rx.lock().unwrap() = Some(rx);

    // Spawn the recording in a blocking thread — audio capture runs synchronously
    // until stop_signal is set or max duration is reached
    tokio::task::spawn_blocking(move || {
        let result = WhisperService::record_and_transcribe(&app_handle, 30.0, stop_signal);
        let _ = tx.send(result);
    });

    let _ = app.emit("voice:recording-started", true);
    log::info!("[PTT] Recording started (background thread)");
    Ok(())
}

/// Signal the recording thread to stop and wait for the transcription result.
#[tauri::command]
pub async fn stop_ptt_recording(
    app: AppHandle,
    state: tauri::State<'_, WhisperState>,
) -> Result<String, String> {
    if !state.is_recording.load(Ordering::SeqCst) {
        return Err("Not recording".into());
    }

    // Signal the recording thread to stop
    state.stop_signal.store(true, Ordering::SeqCst);
    state.is_recording.store(false, Ordering::SeqCst);

    // Take the result receiver (set by start_ptt_recording)
    let rx = state.result_rx.lock().unwrap().take();

    log::info!("[PTT] Stopping, awaiting transcription...");

    let result = match rx {
        Some(rx) => {
            // Await the result from the background recording thread
            rx.await
                .map_err(|_| "Recording thread terminated unexpectedly".to_string())?
        }
        None => {
            // This shouldn't happen in normal flow, but handle gracefully
            return Err("No recording in progress".into());
        }
    };

    match result {
        Ok(r) => {
            if r.text.is_empty() {
                let _ = app.emit("voice:recording-stopped", "");
                Err("未检测到语音".into())
            } else {
                let _ = app.emit("voice:recording-stopped", &r.text);
                log::info!("[PTT] Transcription: '{}' ({}s audio)", r.text, r.duration_secs);
                Ok(r.text.clone())
            }
        }
        Err(e) => {
            log::error!("[PTT] Transcription failed: {e}");
            let _ = app.emit("voice:recording-stopped", "");
            Err(format!("语音识别失败: {e}"))
        }
    }
}

/// Cancel an active recording. Signals the recording thread to stop
/// but does NOT wait for or return the transcription result.
#[tauri::command]
pub async fn cancel_ptt_recording(
    app: AppHandle,
    state: tauri::State<'_, WhisperState>,
) -> Result<(), String> {
    state.stop_signal.store(true, Ordering::SeqCst);
    state.is_recording.store(false, Ordering::SeqCst);

    // Discard the receiver — the background thread will finish and drop its result
    let _ = state.result_rx.lock().unwrap().take();

    let _ = app.emit("voice:recording-cancelled", true);
    log::info!("[PTT] Recording cancelled");
    Ok(())
}
