use std::sync::Mutex;

use tauri::State;

use crate::services::wiki_service::{WikiService, WikiFileMeta};

pub struct WikiState {
    pub service: Mutex<WikiService>,
}

#[tauri::command]
pub async fn get_wiki_file_tree(
    state: State<'_, WikiState>,
) -> Result<Vec<crate::services::wiki_service::FileNode>, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.get_file_tree(None).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_wiki_file(
    state: State<'_, WikiState>,
    path: String,
) -> Result<String, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.read_file(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_wiki_file(
    state: State<'_, WikiState>,
    path: String,
    content: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.write_file(&path, &content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_wiki_item(
    state: State<'_, WikiState>,
    parent_path: String,
    name: String,
    kind: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let parent = parent_path.trim_end_matches('/');
    let full_path = if parent.is_empty() {
        name.clone()
    } else {
        format!("{}/{}", parent, name)
    };
    match kind.as_str() {
        "folder" | "dir" => service.create_dir(&full_path).map_err(|e| e.to_string()),
        _ => service.create_file(&full_path).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn delete_wiki_item(
    wiki_state: State<'_, WikiState>,
    knowledge_state: State<'_, crate::commands::knowledge::KnowledgeState>,
    path: String,
) -> Result<(), String> {
    {
        // Step 1: Delete file (sync lock — drop before async work)
        let service = wiki_state.service.lock().map_err(|e| e.to_string())?;
        service.delete_file(&path, false).map_err(|e| e.to_string())?;
    }
    // Step 2: Cascade-delete knowledge graph entities (async lock)
    let ks = knowledge_state.service.lock().await;
    let _ = ks.cascade_delete_document(&path);
    Ok(())
}

#[tauri::command]
pub async fn rename_wiki_item(
    state: State<'_, WikiState>,
    path: String,
    new_name: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let parent = std::path::Path::new(&path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("");
    let new_path = if parent.is_empty() {
        new_name.clone()
    } else {
        format!("{}/{}", parent, new_name)
    };
    service.rename_file(&path, &new_path).map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn move_wiki_item(
    state: State<'_, WikiState>,
    from_path: String,
    to_path: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.move_file(&from_path, &to_path).map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_wiki_dirs(
    state: State<'_, WikiState>,
    namespace: Option<String>,
) -> Result<Vec<String>, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.list_dirs(namespace.as_deref())
}

#[tauri::command]
pub async fn list_all_knowledge_files(
    state: State<'_, WikiState>,
    namespace: Option<String>,
) -> Result<Vec<WikiFileMeta>, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.list_all_files(namespace.as_deref())
}

#[tauri::command]
pub async fn show_in_folder(
    state: State<'_, WikiState>,
    path: String,
) -> Result<(), String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let full = service.resolve_path(&path)
        .map_err(|e| e.to_string())?;
    // On Windows/macOS/Linux open the parent folder in the file manager.
    // open::that uses ShellExecuteW on Windows, open -R on macOS,
    // and xdg-open on Linux — all proven to work reliably.
    let parent = full.parent().unwrap_or(&full);
    if !parent.exists() {
        return Err(format!("目录不存在: {}", parent.display()));
    }
    open::that(parent).map_err(|e| format!("无法打开资源管理器: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn upload_wiki_file(
    state: State<'_, WikiState>,
    source_path: String,
    target_dir: Option<String>,
    target_name: Option<String>,
) -> Result<String, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.upload_file(&source_path, target_dir.as_deref(), target_name.as_deref())
}

#[tauri::command]
pub async fn write_wiki_file_base64(
    state: State<'_, WikiState>,
    path: String,
    data: String,
) -> Result<(), String> {
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &data,
    ).map_err(|e| format!("Base64 decode failed: {e}"))?;
    let service = state.service.lock().map_err(|e| e.to_string())?;
    let full = service.resolve_path(&path).map_err(|e| e.to_string())?;
    if let Some(p) = full.parent() { std::fs::create_dir_all(p).ok(); }
    std::fs::write(&full, &bytes).map_err(|e| format!("写入二进制文件失败: {e}"))
}

#[tauri::command]
pub async fn read_wiki_file_base64(
    state: State<'_, WikiState>,
    path: String,
) -> Result<String, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.read_file_base64(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn upload_wiki_files(
    state: State<'_, WikiState>,
    paths: Vec<String>,
    target_dir: Option<String>,
) -> Result<Vec<String>, String> {
    let service = state.service.lock().map_err(|e| e.to_string())?;
    service.upload_files(&paths, target_dir.as_deref())
}
