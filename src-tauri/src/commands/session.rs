use std::sync::Mutex;

use tauri::State;

use crate::services::session_service::{SearchResult, Session, SessionDetail, SessionService};

pub struct SessionState {
    pub service: Mutex<SessionService>,
}

#[tauri::command]
pub fn list_sessions(
    state: State<'_, SessionService>,
) -> Result<Vec<Session>, String> {
    state.list_sessions()
}

#[tauri::command]
pub fn get_session(
    state: State<'_, SessionService>,
    session_id: String,
) -> Result<SessionDetail, String> {
    state.get_session(&session_id)
}

#[tauri::command]
pub fn search_sessions(
    state: State<'_, SessionService>,
    query: String,
) -> Result<Vec<SearchResult>, String> {
    state.search_sessions(&query)
}

#[tauri::command]
pub fn rename_session(
    state: State<'_, SessionService>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    state.rename_session(&session_id, &title)
}

#[tauri::command]
pub fn delete_session(
    state: State<'_, SessionService>,
    session_id: String,
) -> Result<(), String> {
    state.delete_session(&session_id)
}

#[tauri::command]
pub fn upsert_session(
    state: State<'_, SessionService>,
    id: String,
    title: String,
    model: String,
    created_at: String,
    updated_at: String,
    agent_id: Option<String>,
) -> Result<(), String> {
    state.upsert_session(&id, &title, &model, agent_id.as_deref(), &created_at, &updated_at)
}

#[tauri::command]
pub fn add_message(
    state: State<'_, SessionService>,
    session_id: String,
    role: String,
    content: String,
) -> Result<String, String> {
    state.add_message(&session_id, &role, &content)
}
