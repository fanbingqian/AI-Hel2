use std::sync::Mutex;

use tauri::State;

use crate::services::canvas_service::CanvasService;

pub struct CanvasState {
    pub service: Mutex<CanvasService>,
}

#[tauri::command]
pub async fn canvas_open(
    state: State<'_, CanvasState>,
    path: String,
) -> Result<String, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.open(&path).map(|c| serde_json::to_string(&c).unwrap_or_default())
}

#[tauri::command]
pub async fn canvas_save(
    state: State<'_, CanvasState>,
    path: String,
    content: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let doc: crate::models::canvas::CanvasDocument = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    service.save(&path, &doc).map_err(|e| e.to_string())
}
