use std::collections::HashMap;
use tauri::State;

use crate::services::gateway_setup::{
    GatewaySetupService, PlatformConfigStatus, PlatformInfo, QrPollResult, QrSessionInfo,
};

/// List all gateway platforms with their QR support status.
#[tauri::command]
pub fn list_gateway_platforms(
    service: State<'_, GatewaySetupService>,
) -> Result<Vec<PlatformInfo>, String> {
    service.list_platforms()
}

/// List platforms with their current config status from config.yaml.
#[tauri::command]
pub fn list_platform_status(
    service: State<'_, GatewaySetupService>,
) -> Result<Vec<PlatformConfigStatus>, String> {
    service.list_platform_status()
}

/// Begin QR registration for a platform. Returns session info with QR URL.
#[tauri::command]
pub async fn gateway_qr_start(
    service: State<'_, GatewaySetupService>,
    platform: String,
) -> Result<QrSessionInfo, String> {
    service.qr_start_async(&platform).await
}

/// Poll QR registration status for a platform.
#[tauri::command]
pub async fn gateway_qr_poll(
    service: State<'_, GatewaySetupService>,
    platform: String,
) -> Result<QrPollResult, String> {
    service.qr_poll_async(&platform).await
}

/// Cancel an active QR registration session for a platform.
#[tauri::command]
pub async fn gateway_qr_cancel(
    service: State<'_, GatewaySetupService>,
    platform: String,
) -> Result<(), String> {
    service.qr_cancel_async(&platform).await
}

/// Get active QR session info for a platform (for restoring UI state after reload).
#[tauri::command]
pub fn gateway_get_active_session(
    service: State<'_, GatewaySetupService>,
    platform: String,
) -> Result<Option<QrSessionInfo>, String> {
    Ok(service.get_active_session(&platform))
}

/// Read the full gateway config from config.yaml.
#[tauri::command]
pub fn gateway_get_config(
    service: State<'_, GatewaySetupService>,
) -> Result<serde_json::Value, String> {
    service.read_gateway_config()
}

/// Save credentials from QR scan and write to config.yaml.
#[tauri::command]
pub fn gateway_save_credentials(
    service: State<'_, GatewaySetupService>,
    platform: String,
    credentials: HashMap<String, String>,
) -> Result<(), String> {
    service.save_platform_credentials(&platform, &credentials)
}

/// Save platform config directly (for manual config mode).
#[tauri::command]
pub fn gateway_save_platform_config(
    service: State<'_, GatewaySetupService>,
    platform: String,
    config: serde_json::Value,
) -> Result<(), String> {
    service.save_platform_config(&platform, &config)
}

/// Remove a platform from config.yaml (disable/unregister).
#[tauri::command]
pub fn gateway_remove_platform(
    service: State<'_, GatewaySetupService>,
    platform: String,
) -> Result<(), String> {
    service.remove_platform_config(&platform)
}

// ── Cron job commands ──

#[tauri::command]
pub fn list_cron_jobs(
    service: State<'_, GatewaySetupService>,
) -> Result<Vec<serde_json::Value>, String> {
    service.list_cron_jobs()
}

#[tauri::command]
pub fn add_cron_job(
    service: State<'_, GatewaySetupService>,
    job: serde_json::Value,
) -> Result<serde_json::Value, String> {
    service.add_cron_job(job)
}

#[tauri::command]
pub fn update_cron_job(
    service: State<'_, GatewaySetupService>,
    jobId: String,
    updates: serde_json::Value,
) -> Result<(), String> {
    service.update_cron_job(&jobId, updates)
}

#[tauri::command]
pub fn delete_cron_job(
    service: State<'_, GatewaySetupService>,
    jobId: String,
) -> Result<(), String> {
    service.delete_cron_job(&jobId)
}

#[tauri::command]
pub fn toggle_cron_job(
    service: State<'_, GatewaySetupService>,
    jobId: String,
    enabled: bool,
) -> Result<(), String> {
    service.toggle_cron_job(&jobId, enabled)
}

#[tauri::command]
pub fn trigger_cron_job(
    service: State<'_, GatewaySetupService>,
    jobId: String,
) -> Result<(), String> {
    service.trigger_cron_job(&jobId)
}

#[tauri::command]
pub fn get_cron_output(
    service: State<'_, GatewaySetupService>,
    jobId: String,
) -> Result<Vec<String>, String> {
    service.get_cron_output(&jobId)
}
