mod commands;
mod errors;
mod models;
mod services;


use commands::auth::AuthState;
use commands::canvas::CanvasState;
use commands::chat::{AgentRegistryState, AgentState};
use commands::config::ConfigState;
use commands::knowledge::KnowledgeState;
use commands::wiki::WikiState;
use services::agents::AgentRegistry;
use services::agent_manager::AgentManager;
use services::canvas_service::CanvasService;
use services::config_service::ConfigService;
use services::connection_service::ConnectionService;
use services::file_watcher::FileWatcherService;
use services::gateway_setup::GatewaySetupService;
use services::knowledge_service::KnowledgeService;
use services::session_service::SessionService;
use services::whisper_service::WhisperService;
use services::wiki_service::WikiService;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tokio::sync::mpsc;

pub use errors::AppError;

/// Recursively migrate .md files from old to new wiki directory.
/// Skips noise directories: _auto, _trash, and nested heimdall/wiki/ duplicates.
fn migrate_wiki_files(old: &Path, new: &Path, moved: &mut u32) -> Result<(), String> {
    for entry in std::fs::read_dir(old).map_err(|e| format!("read_dir: {e}"))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let src = entry.path();
        let fname = src.file_name().unwrap().to_string_lossy();
        if src.is_dir() {
            // Skip noise and duplicate directories
            if fname == "_auto" || fname == "_trash" || fname == "heimdall" {
                continue;
            }
            let sub_new = new.join(fname.as_ref());
            std::fs::create_dir_all(&sub_new)
                .map_err(|e| format!("create_dir {:?}: {e}", sub_new))?;
            migrate_wiki_files(&src, &sub_new, moved)?;
        } else if src.extension().and_then(|e| e.to_str()) == Some("md") {
            let dest = new.join(fname.as_ref());
            // Skip if destination already exists (dedup)
            if dest.exists() {
                continue;
            }
            std::fs::rename(&src, &dest)
                .map_err(|e| format!("rename {:?}: {e}", src))?;
            *moved += 1;
        }
    }
    Ok(())
}

/// Resolve the legacy Hermes Agent home directory (~/.hermes).
fn dirs_hermes_home_legacy() -> PathBuf {
    std::env::var("HERMES_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            #[cfg(target_os = "windows")]
            {
                std::env::var("USERPROFILE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("C:"))
                    .join(".hermes")
            }
            #[cfg(not(target_os = "windows"))]
            {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("/tmp"))
                    .join(".hermes")
            }
        })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config_service = ConfigService::new();
    let hermes_home = config_service.hermes_home().to_path_buf();
    std::fs::create_dir_all(&hermes_home).expect("Failed to create AI-Hel2 data directory");
    let mut connection_service = ConnectionService::new();
    connection_service.config.api_key = Some("aihel2-local-dev".into());

    let agent_state = AgentState::new(connection_service);

    let session_service = SessionService::new(&hermes_home)
        .expect("Failed to initialize session database");

    // Agent registry: load persisted agents, seed if first run
    let agent_registry = Arc::new(tokio::sync::RwLock::new(AgentRegistry::new(&hermes_home)));
    {
        let reg = agent_registry.blocking_read();
        if let Err(e) = reg.load_persisted() {
            log::warn!("Agent registry load failed: {e}");
        } else {
            log::info!("Agent registry loaded from agents.json");
        }
    }
    let agent_registry_state = AgentRegistryState {
        registry: agent_registry.clone(),
    };

    let knowledge_service = Arc::new(
        KnowledgeService::new(&hermes_home).expect("Failed to initialize knowledge cache"),
    );

    // Start Nexus HTTP API server (for Python Agent tool access)
    services::nexus_api::start(knowledge_service.clone(), &hermes_home);

    // Migrate old heimdall/wiki/ .md files to new wiki/ path (one-time)
    // Sources: own hermes_home/heimdall/wiki/ AND legacy ~/.hermes/heimdall/wiki/
    let new_wiki = hermes_home.join("wiki");
    std::fs::create_dir_all(&new_wiki).ok();
    let mut total_moved = 0u32;

    let own_old = hermes_home.join("heimdall").join("wiki");
    for old_wiki in [own_old, dirs_hermes_home_legacy().join("heimdall").join("wiki")].iter() {
        if old_wiki.exists() {
            let mut moved = 0u32;
            match migrate_wiki_files(&old_wiki, &new_wiki, &mut moved) {
                Ok(()) => {
                    total_moved += moved;
                    // Only remove if this is the own directory (not the legacy Hermes one)
                    if old_wiki.starts_with(&hermes_home) {
                        let _ = std::fs::remove_dir_all(&old_wiki);
                    }
                }
                Err(e) => log::warn!("Wiki migration from {:?}: {e}", old_wiki),
            }
        }
    }
    if total_moved > 0 {
        log::info!("Wiki migration: {} .md files imported to {:?}", total_moved, new_wiki);
    }

    // File watcher for wiki directory (Arc-held — accessible for event loop + KnowledgeState)
    let wiki_dir = hermes_home.join("wiki");
    let mut file_watcher = FileWatcherService::new(wiki_dir);
    let (file_tx, mut file_rx) = mpsc::channel::<models::knowledge::FileChangeEvent>(64);
    if let Err(e) = file_watcher.start(file_tx) {
        log::warn!("FileWatcher failed to start: {e}");
    }
    let file_watcher = Arc::new(std::sync::Mutex::new(file_watcher));

    let knowledge_state = KnowledgeState {
        service: tokio::sync::Mutex::new(knowledge_service),
        file_watcher: file_watcher.clone(),
    };

    let wiki_service = WikiService::new(&hermes_home.join("wiki"));
    // Seed wiki with default content on first run
    if let Err(e) = wiki_service.seed_wiki() {
        log::warn!("Wiki seed failed (non-fatal): {e}");
    }
    let wiki_state = WikiState {
        service: std::sync::Mutex::new(wiki_service),
    };

    let canvas_service = CanvasService::new(&hermes_home);
    let canvas_state = CanvasState {
        service: std::sync::Mutex::new(canvas_service),
    };

    let config_state = ConfigState {
        service: std::sync::Mutex::new(config_service),
    };

    let auth_state = AuthState {
        config: std::sync::Mutex::new(ConfigService::new()),
    };

    let agent_manager = AgentManager::new(&hermes_home);

    let gateway_setup_service = GatewaySetupService::new(&hermes_home);

    let whisper_state = commands::voice::WhisperState {
        is_recording: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stop_signal: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        result_rx: std::sync::Mutex::new(None),
    };

    #[tauri::command]
    fn toggle_main_window(app: tauri::AppHandle) {
        if let Some(window) = app.get_webview_window("main") {
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
            } else {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(config_state)
        .manage(auth_state)
        .manage(agent_state)
        .manage(session_service)
        .manage(knowledge_state)
        .manage(wiki_state)
        .manage(canvas_state)
        .manage(agent_manager)
        .manage(agent_registry_state)
        .manage(whisper_state)
        .manage(gateway_setup_service)
        .setup(|app| {
            let handle = app.handle().clone();

            // System tray
            use tauri::tray::TrayIconBuilder;
            use tauri::menu::{MenuBuilder, MenuItemBuilder};

            let toggle_item = MenuItemBuilder::with_id("toggle", "显示/隐藏 AI-Hel2")
                .build(app)
                .unwrap_or_else(|_| panic!("Failed to create tray toggle item"));
            let exit_item = MenuItemBuilder::with_id("exit", "退出")
                .build(app)
                .unwrap_or_else(|_| panic!("Failed to create tray exit item"));
            let tray_menu = MenuBuilder::new(app)
                .item(&toggle_item)
                .separator()
                .item(&exit_item)
                .build()
                .unwrap_or_else(|_| panic!("Failed to create tray menu"));

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("AI-Hel2")
                .menu(&tray_menu)
                .on_menu_event(move |app, event| {
                    match event.id().as_ref() {
                        "toggle" => {
                            if let Some(window) = app.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.unminimize();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                        "exit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app);

            // Pill window (平刘海) — top-center floating bar
            {
                let pill = tauri::WebviewWindowBuilder::new(
                    app,
                    "pill",
                    tauri::WebviewUrl::App("index.html?window=pill".into()),
                )
                .title("AI-Hel2 Pill")
                .inner_size(290.0, 52.0)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .visible(true)
                .build();

                if let Ok(pill) = pill {
                    if let Ok(Some(monitor)) = pill.primary_monitor() {
                        let w = 290.0;
                        let x = ((monitor.size().width as f64 - w) / 2.0) as i32;
                        let _ = pill.set_position(tauri::Position::Physical(
                            tauri::PhysicalPosition { x, y: 38 },
                        ));
                    }
                }

            }

            // Voice overlay window — centered recording indicator (hidden by default)
            commands::voice_overlay::create_overlay_window(app);

            // Initial wiki scan — indexes existing markdown files on startup.
            // The file watcher only catches changes after startup, so this
            // ensures existing files (including subdirectories) are indexed.
            {
                let scan_handle = handle.clone();
                std::thread::spawn(move || {
                    let ks = scan_handle.state::<KnowledgeState>();
                    let svc = ks.service.blocking_lock();
                    match svc.scan_wiki_directory() {
                        Ok(result) => {
                            if result.scanned > 0 {
                                log::info!(
                                    "Wiki initial scan: {} files, {} new entities, {} stale cleaned, {} failed",
                                    result.scanned, result.total_new, result.stale_entities_removed, result.failed
                                );
                            }
                            if !result.errors.is_empty() {
                                for e in &result.errors {
                                    log::warn!("Wiki scan error: {e}");
                                }
                            }
                        }
                        Err(e) => log::warn!("Wiki initial scan failed: {e}"),
                    }
                });
            }

            // Start Hermes Agent in background thread
            let startup_handle = handle.clone();
            // Force-extract bundled resources so AgentManager can find them on disk
            let resource_dir = app.path().resource_dir().ok();
            if let Some(ref rd) = resource_dir {
                let am = startup_handle.state::<AgentManager>();
                am.set_resource_dir(rd.clone());
                // Resolve key files to trigger lazy extraction
                let _ = app.path().resolve("hermes-agent/python/python.exe", tauri::path::BaseDirectory::Resource);
                let _ = app.path().resolve("hermes-agent/hermes_cli/main.py", tauri::path::BaseDirectory::Resource);
            }
            std::thread::spawn(move || {
                let am = startup_handle.state::<AgentManager>();
                if let Err(e) = am.start() {
                    log::error!("Agent startup failed: {e}");
                } else {
                    log::info!("Agent started successfully on port {}", am.port());
                }
            });

            // Background agent detection scan (non-blocking)
            let scan_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let reg_state = scan_handle.state::<AgentRegistryState>();
                let registry = reg_state.registry.read().await;
                let detected = registry.background_scan().await;
                drop(registry);
                if !detected.is_empty() {
                    let registry = reg_state.registry.read().await;
                    if let Err(e) = registry.merge_detected(&detected) {
                        log::warn!("Agent detection merge failed: {e}");
                    } else {
                        log::info!("Agent detection found {} new agents", detected.len());
                    }
                    let _ = scan_handle.emit("agents:updated", ());
                }
            });

            // Auto-configure OpenClaw HTTP API endpoint if config file exists
            // but gateway.http.endpoints.chatCompletions is not enabled.
            {
                let oc = services::openclaw_launcher::OpenClawLauncher::new();
                match oc.ensure_http_api_enabled() {
                    Ok(true) => log::info!("OpenClaw HTTP API auto-configured — restart OpenClaw to apply"),
                    Ok(false) => {}
                    Err(e) => log::warn!("OpenClaw auto-config skipped: {e}"),
                }
            }

            // Background health poller — re-probe agent connectivity every 30s,
            // auto-restart gateway on failure, and emit agents:updated on health change.
            let poll_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;

                    // Gateway auto-restart
                    let am = poll_handle.state::<AgentManager>();
                    am.try_auto_restart();

                    let reg_state = poll_handle.state::<AgentRegistryState>();
                    let registry = reg_state.registry.read().await;
                    if registry.tick_health().await {
                        log::info!("[health_poller] agent health changed, notifying frontend");
                        let _ = poll_handle.emit("agents:updated", ());
                    }
                }
            });

            // File watcher event loop — dispatch by file extension
            let ks_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                use crate::models::knowledge::ChangeType;
                while let Some(event) = file_rx.recv().await {
                    log::info!(
                        "File change detected: {} ({:?})",
                        event.file_path,
                        event.change_type
                    );

                    // Notify frontend to refresh document tree
                    let _ = ks_handle.emit(
                        "wiki:files-changed",
                        &serde_json::json!({
                            "path": event.file_path,
                            "change_type": format!("{:?}", event.change_type),
                        }),
                    );

                    // Handle file removal: clean up stale entities
                    if matches!(event.change_type, ChangeType::Removed) {
                        let ext = std::path::Path::new(&event.file_path)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if ext == "md" {
                            let ks = ks_handle.state::<KnowledgeState>();
                            let svc = ks.service.lock().await;
                            let mut files = std::collections::HashSet::new();
                            svc.collect_md_files(&svc.wiki_dir(), &mut files);
                            match svc.cleanup_stale_wiki_entities(&files) {
                                Ok((entities, relations)) => {
                                    if entities > 0 {
                                        log::info!(
                                            "File removed {}: cleaned {} entities, {} relations",
                                            event.file_path, entities, relations
                                        );
                                        let _ = ks_handle.emit(
                                            "knowledge:graph-updated",
                                            &serde_json::json!({
                                                "stale_entities_removed": entities,
                                                "stale_relations_removed": relations,
                                            }),
                                        );
                                    }
                                }
                                Err(e) => log::warn!("Cleanup after removal failed: {e}"),
                            }
                        }
                        continue;
                    }

                    if !matches!(event.change_type, ChangeType::Created | ChangeType::Modified) {
                        continue;
                    }

                    let path = std::path::Path::new(&event.file_path);
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();

                    if ext == "md" {
                        let ks = ks_handle.state::<KnowledgeState>();
                        let svc = ks.service.lock().await;
                        let ns = path
                            .parent()
                            .and_then(|p| p.file_name())
                            .and_then(|n| n.to_str())
                            .unwrap_or("default");
                        match svc.extract_entities(&event.file_path, ns).await {
                            Ok(result) => {
                                log::info!(
                                    "Wiki extraction from {}: {} new, {} updated",
                                    event.file_path, result.new_count, result.updated_count
                                );
                                let _ = ks_handle.emit("knowledge:extraction-complete", &result);
                            }
                            Err(e) => log::warn!(
                                "Wiki extraction failed for {}: {e}",
                                event.file_path
                            ),
                        }
                    } else if matches!(event.change_type, ChangeType::Created)
                        && ext != "md" && ext != "canvas" && !ext.is_empty()
                    {
                        // Auto-classify newly added non-md files (pdf, docx, images, etc.)
                        let ks = ks_handle.state::<KnowledgeState>();
                        let svc = ks.service.lock().await;
                        let dirs = svc.list_wiki_top_dirs();
                        match svc.classify_single_file(&event.file_path, &dirs) {
                            Ok(result) => {
                                let folder = result.get("folder")
                                    .and_then(|v| v.as_str()).unwrap_or("笔记");
                                let new_path = result.get("file_path")
                                    .and_then(|v| v.as_str()).unwrap_or(&event.file_path);
                                log::info!(
                                    "Auto-classified {} → {}/{}",
                                    event.file_path, folder, new_path
                                );
                                let _ = ks_handle.emit("wiki:files-changed", &serde_json::json!({
                                    "path": new_path,
                                    "change_type": "classified",
                                }));
                            }
                            Err(e) => log::warn!(
                                "Auto-classify failed for {}: {e}",
                                event.file_path
                            ),
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Window / Pill
            toggle_main_window,
            // Chat
            commands::chat::chat_completions,
            commands::chat::abort_chat,
            commands::chat::generate_title,
            // Knowledge
            commands::knowledge::get_graph_data,
            commands::knowledge::get_entity_detail,
            commands::knowledge::get_lint_warnings,
            commands::knowledge::search_entities,
            commands::knowledge::find_entity_paths,
            commands::knowledge::build_knowledge_context,
            commands::knowledge::extract_entities,
            commands::knowledge::extract_entities_from_text,
            commands::knowledge::get_knowledge_stats,
            commands::knowledge::get_namespaces,
            commands::knowledge::get_neighbors,
            commands::knowledge::save_chat_to_knowledge,
            commands::knowledge::rescan_wiki,
            commands::knowledge::get_daily_digest,
            commands::knowledge::reference_entity_to_chat,
            commands::knowledge::get_smart_display,
            commands::knowledge::get_inferences,
            // Nexus Knowledge Engine
            commands::knowledge::nexus_store,
            commands::knowledge::nexus_extract_from_file,
            commands::knowledge::nexus_reindex_all,
            commands::knowledge::nexus_reindex_force,
            commands::knowledge::nexus_run_synthesis,
            commands::knowledge::nexus_analyze_types,
            // Nexus P5: Entity CRUD + feedback + merge
            commands::knowledge::nexus_update_entity,
            commands::knowledge::nexus_delete_entity,
            commands::knowledge::nexus_add_relation,
            commands::knowledge::nexus_update_relation,
            commands::knowledge::nexus_delete_relation,
            commands::knowledge::nexus_submit_feedback,
            commands::knowledge::nexus_get_pending_merges,
            commands::knowledge::nexus_confirm_merge,
            commands::knowledge::nexus_ignore_merge,
            commands::knowledge::nexus_batch_operation,
            commands::knowledge::nexus_get_entity_feedback,
            // Document Summarization & Image Description
            commands::knowledge::nexus_summarize_document,
            commands::knowledge::nexus_describe_images,
            commands::knowledge::nexus_auto_classify,
            // Nexus Maintenance
            commands::knowledge::nexus_maintain_quality,
            commands::knowledge::nexus_maintain_cleanup,
            commands::knowledge::nexus_maintain_dedup,
            commands::knowledge::nexus_maintain_fix_migrated,
            commands::knowledge::nexus_get_maintenance_status,
            commands::knowledge::check_nexus_server_health,
            // Layer 2-6 Extended Maintenance
            commands::knowledge::nexus_maintain_classify,
            commands::knowledge::nexus_run_pagerank,
            commands::knowledge::nexus_run_community,
            commands::knowledge::nexus_discover_causal,
            commands::knowledge::nexus_run_transitive,
            commands::knowledge::nexus_scan_conflicts,
            commands::knowledge::nexus_get_evolution,
            commands::knowledge::nexus_verify_synthesis,
            commands::knowledge::nexus_reset_graph,
            // Wiki
            commands::wiki::get_wiki_file_tree,
            commands::wiki::read_wiki_file,
            commands::wiki::write_wiki_file,
            commands::wiki::create_wiki_item,
            commands::wiki::delete_wiki_item,
            commands::wiki::rename_wiki_item,
            commands::wiki::move_wiki_item,
            commands::wiki::list_wiki_dirs,
            commands::wiki::list_all_knowledge_files,
            commands::wiki::read_wiki_file_base64,
            commands::wiki::show_in_folder,
            commands::wiki::upload_wiki_file,
            commands::wiki::upload_wiki_files,
            // Canvas
            commands::canvas::canvas_open,
            commands::canvas::canvas_save,
            // Config
            commands::config::get_config,
            commands::config::save_config,
            commands::config::update_api_key,
            commands::config::verify_api_key,
            commands::config::copy_agent_config_for_nexus,
            commands::config::save_user_profile,
            commands::config::export_data,
            commands::config::import_data,
            commands::config::list_env_keys,
            // Nexus LLM config
            commands::config::get_nexus_config,
            commands::config::save_nexus_config,
            commands::config::check_nexus_llm_connection,
            // Auth
            commands::auth::register_user,
            commands::auth::login_user,
            commands::auth::get_current_user,
            commands::auth::change_password,
            // Voice
            commands::voice::check_voice_deps,
            commands::voice::voice_diagnose,
            commands::voice::tts_speak,
            commands::voice::tts_preview,
            commands::voice::start_ptt_recording,
            commands::voice::stop_ptt_recording,
            commands::voice::cancel_ptt_recording,
            // Session
            commands::session::list_sessions,
            commands::session::get_session,
            commands::session::search_sessions,
            commands::session::rename_session,
            commands::session::delete_session,
            commands::session::upsert_session,
            commands::session::add_message,
            // Agent management
            commands::agent::agent_status,
            commands::agent::restart_agent,
            commands::agent::get_agent_logs,
            // Multi-agent registry
            commands::agent::list_agents,
            commands::agent::add_agent,
            commands::agent::remove_agent,
            commands::agent::set_agent_enabled,
            commands::agent::set_default_agent,
            commands::agent::re_detect_agents,
            commands::agent::update_agent_config,
            commands::agent::openclaw_configure,
            commands::agent::openclaw_start,
            // Gateway setup
            commands::gateway::list_gateway_platforms,
            commands::gateway::list_platform_status,
            commands::gateway::gateway_qr_start,
            commands::gateway::gateway_qr_poll,
            commands::gateway::gateway_qr_cancel,
            commands::gateway::gateway_get_active_session,
            commands::gateway::gateway_get_config,
            commands::gateway::gateway_save_credentials,
            commands::gateway::gateway_save_platform_config,
            commands::gateway::gateway_remove_platform,
            commands::gateway::list_cron_jobs,
            commands::gateway::add_cron_job,
            commands::gateway::update_cron_job,
            commands::gateway::delete_cron_job,
            commands::gateway::toggle_cron_job,
            commands::gateway::trigger_cron_job,
            commands::gateway::get_cron_output,
            // Files
            commands::files::read_text_file,
            commands::files::read_file_base64,
            commands::files::save_avatar,
            commands::files::get_avatar,
            commands::files::capture_screen,
        ])
        .build(tauri::generate_context!())
        .expect("error while building AI-Hel2")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                let am = app_handle.state::<AgentManager>();
                am.shutdown();
            }
        });
}

