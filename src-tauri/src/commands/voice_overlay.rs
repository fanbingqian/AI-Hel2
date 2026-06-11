//! Voice overlay window management.
//! The overlay is a small centered circle showing recording status.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Create the voice overlay window on startup.
/// Hidden by default; shown during PTT recording.
pub fn create_overlay_window(app: &tauri::App) {
    let overlay = WebviewWindowBuilder::new(
        app,
        "voice-overlay",
        WebviewUrl::App("index.html?window=voice-overlay".into()),
    )
    .title("AI-Hel2 Voice")
    .inner_size(220.0, 220.0)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false)
    .build();

    if let Ok(overlay) = overlay {
        // Center on primary monitor
        if let Ok(Some(monitor)) = overlay.primary_monitor() {
            let size = monitor.size();
            let w = 220.0;
            let h = 220.0;
            let x = ((size.width as f64 - w) / 2.0) as i32;
            let y = ((size.height as f64 - h) / 2.0) as i32;
            let _ = overlay.set_position(tauri::Position::Physical(
                tauri::PhysicalPosition { x, y },
            ));
        }
    }
}

/// Show the overlay. Returns whether it was already visible.
/// Does NOT steal focus — overlay is display-only.
pub fn show(app: &AppHandle) -> bool {
    if let Some(overlay) = app.get_webview_window("voice-overlay") {
        let was_visible = overlay.is_visible().unwrap_or(false);
        let _ = overlay.show();
        let _ = overlay.emit("voice-overlay:state", serde_json::json!({
            "recording": true,
            "duration": 0,
        }));
        was_visible
    } else {
        false
    }
}

/// Hide the overlay.
pub fn hide(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window("voice-overlay") {
        let _ = overlay.emit("voice-overlay:state", serde_json::json!({
            "recording": false,
            "duration": 0,
        }));
        let _ = overlay.hide();
    }
}

/// Update overlay state during recording.
pub fn update_state(app: &AppHandle, elapsed_secs: f64) {
    if let Some(overlay) = app.get_webview_window("voice-overlay") {
        let _ = overlay.emit("voice-overlay:state", serde_json::json!({
            "recording": true,
            "duration": elapsed_secs,
        }));
    }
}
