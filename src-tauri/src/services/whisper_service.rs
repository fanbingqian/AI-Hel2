use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Manager;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// The result of a recording + transcription cycle
#[derive(Clone)]
pub struct WhisperResult {
    pub text: String,
    pub duration_secs: f64,
}

pub struct WhisperService {
    pub recording: Mutex<bool>,
}

impl WhisperService {
    pub fn new() -> Self {
        Self {
            recording: Mutex::new(false),
        }
    }

    /// Locate the whisper.cpp CLI binary.
    /// Priority: 1) bundled resource, 2) next to exe, 3) project models dir, 4) PATH
    pub fn find_whisper_exe(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
        // 1. Bundled resource
        if let Ok(resource_dir) = app_handle.path().resource_dir() {
            let bundled = resource_dir.join("whisper").join("whisper-cli.exe");
            if bundled.exists() {
                log::info!("[Whisper] Found bundled: {}", bundled.display());
                return Ok(bundled);
            }
        }

        // 2. Next to the current executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let sidecar = exe_dir.join("whisper-cli.exe");
                if sidecar.exists() {
                    return Ok(sidecar);
                }
            }
        }

        // 3. Project models dir (development)
        let project_models = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("models")
            .join("whisper-cli.exe");
        if project_models.exists() {
            return Ok(project_models);
        }

        // 4. Fallback: PATH
        Ok(PathBuf::from("whisper-cli"))
    }

    /// Find the ggml model file.
    /// Priority: 1) bundled, 2) models dir next to exe, 3) project models, 4) hermes_home/models
    pub fn find_model(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
        // 1. Bundled resource
        if let Ok(resource_dir) = app_handle.path().resource_dir() {
            let bundled = resource_dir.join("whisper").join("ggml-base.bin");
            if bundled.exists() {
                return Ok(bundled);
            }
        }

        // 2. models dir next to executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let side = exe_dir.join("models").join("ggml-base.bin");
                if side.exists() {
                    return Ok(side);
                }
            }
        }

        // 3. Project models dir (development)
        let project_models = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("models")
            .join("ggml-base.bin");
        if project_models.exists() {
            return Ok(project_models);
        }

        // 4. hermes_home/models
        if let Ok(resource_dir) = app_handle.path().resource_dir() {
            let home_models = resource_dir
                .parent()
                .unwrap_or(&resource_dir)
                .join("models")
                .join("ggml-base.bin");
            if home_models.exists() {
                return Ok(home_models);
            }
        }

        Err("whisper model (ggml-base.bin) not found".into())
    }

    /// Find the best input audio device.
    pub fn find_best_input_device(
    ) -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
        let host = cpal::default_host();

        // Try default input device first
        if let Some(device) = host.default_input_device() {
            if let Ok(config) = device.default_input_config() {
                let name = device.name().unwrap_or_default();
                log::info!("[Whisper] Using input device: {name}");
                return Ok((device, config));
            }
        }

        // Fallback: iterate all devices
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                if let Ok(config) = device.default_input_config() {
                    let name = device.name().unwrap_or_default();
                    log::info!("[Whisper] Fallback device: {name}");
                    return Ok((device, config));
                }
            }
        }

        Err("No audio input device found".into())
    }

    /// Record audio from the given device.
    /// Stops when `stop_signal` becomes true or `max_duration_secs` is reached.
    pub fn record_audio(
        &self,
        max_duration_secs: f64,
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<(Vec<f32>, u32), String> {
        use std::sync::mpsc;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;

        let (tx, rx) = mpsc::sync_channel::<f32>(4096);

        *self.recording.lock().unwrap() = true;

        let stop_signal_clone = stop_signal.clone();
        let err_fn = |err| log::error!("[Whisper] Audio stream error: {err}");

        let audio_config = cpal::StreamConfig {
            channels: config.channels(),
            sample_rate: config.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = device
            .build_input_stream(
                &audio_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if stop_signal_clone.load(Ordering::Relaxed) {
                        return;
                    }
                    for chunk in data.chunks(channels) {
                        let sample = if channels == 1 {
                            chunk[0]
                        } else {
                            (chunk[0] + chunk.get(1).copied().unwrap_or(chunk[0])) / 2.0
                        };
                        let _ = tx.try_send(sample);
                    }
                },
                err_fn,
                Some(std::time::Duration::from_secs_f64(max_duration_secs)),
            )
            .map_err(|e| format!("Failed to open audio stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {e}"))?;

        let max_samples = (sample_rate as f64 * max_duration_secs) as usize;
        let mut samples = Vec::with_capacity(max_samples.min(480000));
        let start = std::time::Instant::now();

        // We need to sleep for a minimum amount of time to let the mic actually capture
        // meaningful audio (at least 300ms)
        let min_record_ms = 300;

        loop {
            let timeout = if stop_signal.load(Ordering::Relaxed)
                && start.elapsed().as_millis() >= min_record_ms as u128
            {
                // Already waited minimum time, break immediately
                break;
            } else {
                std::time::Duration::from_millis(50)
            };

            match rx.recv_timeout(timeout) {
                Ok(sample) => {
                    samples.push(sample);
                    if samples.len() >= max_samples {
                        break;
                    }
                    // Check stop signal on every batch
                    if stop_signal.load(Ordering::Relaxed)
                        && start.elapsed().as_millis() >= min_record_ms as u128
                    {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if stop_signal.load(Ordering::Relaxed)
                        && start.elapsed().as_millis() >= min_record_ms as u128
                    {
                        break;
                    }
                    if start.elapsed().as_secs_f64() > max_duration_secs {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        drop(stream);
        *self.recording.lock().unwrap() = false;

        log::info!(
            "[Whisper] Recorded {} samples @ {}Hz ({:.1}s)",
            samples.len(),
            sample_rate,
            samples.len() as f64 / sample_rate as f64
        );

        Ok((samples, sample_rate))
    }

    /// Write raw f32 mono samples to a 16kHz 16-bit WAV file.
    fn write_wav(path: &std::path::Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer =
            hound::WavWriter::create(path, spec).map_err(|e| format!("Failed to create WAV: {e}"))?;

        // Resample to 16kHz if needed (simple linear interpolation)
        let resampled = if sample_rate != 16000 {
            let ratio = 16000.0 / sample_rate as f64;
            let output_len = (samples.len() as f64 * ratio) as usize;
            let mut resampled = Vec::with_capacity(output_len);
            for i in 0..output_len {
                let src_idx = (i as f64 / ratio) as usize;
                let t = (i as f64 / ratio) - src_idx as f64;
                let s0 = samples[src_idx.min(samples.len() - 1)] as f64;
                let s1 = samples[(src_idx + 1).min(samples.len() - 1)] as f64;
                resampled.push(s0 * (1.0 - t) + s1 * t);
            }
            resampled
        } else {
            samples.iter().map(|&s| s as f64).collect()
        };

        for &sample in &resampled {
            let clamped = (sample.max(-1.0).min(1.0) * 32767.0) as i16;
            writer
                .write_sample(clamped)
                .map_err(|e| format!("WAV write error: {e}"))?;
        }

        writer
            .finalize()
            .map_err(|e| format!("WAV finalize error: {e}"))?;
        Ok(())
    }

    /// Run whisper.cpp transcription on a WAV file.
    pub fn transcribe(
        app_handle: &tauri::AppHandle,
        wav_path: &std::path::Path,
    ) -> Result<String, String> {
        let whisper_exe = Self::find_whisper_exe(app_handle)?;
        let model_path = Self::find_model(app_handle)?;

        let output = Command::new(&whisper_exe)
            .arg("-m")
            .arg(&model_path)
            .arg("-f")
            .arg(wav_path)
            .arg("-l")
            .arg("zh")
            .arg("-nt")
            .arg("--no-prints")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("whisper.cpp execution failed: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::error!("[Whisper] stderr: {}", stderr);
            return Err(format!(
                "whisper.cpp failed: {}",
                stderr.lines().last().unwrap_or("unknown error")
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let text = stdout
            .lines()
            .filter(|l| !l.starts_with('[') && !l.starts_with("whisper_"))
            .collect::<Vec<&str>>()
            .join(" ")
            .trim()
            .to_string();

        log::info!("[Whisper] Transcription: '{}'", text);
        Ok(text)
    }

    /// Full pipeline: record audio → write WAV → transcribe → return text
    pub fn record_and_transcribe(
        app_handle: &tauri::AppHandle,
        max_duration_secs: f64,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<WhisperResult, String> {
        let (device, config) = Self::find_best_input_device()?;
        let service = WhisperService::new();

        let start = std::time::Instant::now();
        let (samples, sample_rate) =
            service.record_audio(max_duration_secs, &device, &config, stop_signal)?;
        let duration = start.elapsed().as_secs_f64();

        if samples.is_empty() {
            return Ok(WhisperResult {
                text: String::new(),
                duration_secs: duration,
            });
        }

        // Energy check — skip obviously-silent recordings
        let energy: f32 = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
        let energy_db = 10.0 * (energy.max(1e-10)).log10();
        if energy_db < -50.0 {
            log::info!("[Whisper] Audio too quiet ({} dB), likely silence", energy_db as i32);
            return Ok(WhisperResult {
                text: String::new(),
                duration_secs: duration,
            });
        }

        let wav_path =
            std::env::temp_dir().join(format!("whisper_{}.wav", uuid::Uuid::new_v4()));
        Self::write_wav(&wav_path, &samples, sample_rate)?;

        let text = Self::transcribe(app_handle, &wav_path)?;
        let _ = std::fs::remove_file(&wav_path);

        Ok(WhisperResult {
            text,
            duration_secs: duration,
        })
    }
}
