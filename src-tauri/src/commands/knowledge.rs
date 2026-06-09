use std::sync::Arc;

use tauri::{Emitter, State};
use tokio::sync::Mutex;

use crate::models::knowledge::*;
use crate::services::file_watcher::FileWatcherService;
use crate::services::knowledge_service::KnowledgeService;

pub struct KnowledgeState {
    pub service: Mutex<Arc<KnowledgeService>>,
    pub file_watcher: Arc<std::sync::Mutex<FileWatcherService>>,
}

#[tauri::command]
pub async fn get_graph_data(
    state: State<'_, KnowledgeState>,
    namespace: Option<String>,
    view_mode: Option<String>,
    focal_node: Option<String>,
    hops: Option<u32>,
) -> Result<GraphData, String> {
    let service = state.service.lock().await;
    service
        .get_graph_data(
            namespace.as_deref(),
            view_mode.as_deref().unwrap_or("entity"),
            focal_node.as_deref(),
            hops,
        )
        .await
}

#[tauri::command]
pub async fn get_entity_detail(
    state: State<'_, KnowledgeState>,
    entity_id: String,
) -> Result<EntityDetail, String> {
    let service = state.service.lock().await;
    service.get_entity_detail(&entity_id).await
}

#[tauri::command]
pub async fn get_lint_warnings(
    state: State<'_, KnowledgeState>,
    namespace: Option<String>,
) -> Result<Vec<LintWarning>, String> {
    let service = state.service.lock().await;
    service.get_lint_warnings(namespace.as_deref())
}

#[tauri::command]
pub async fn search_entities(
    state: State<'_, KnowledgeState>,
    query: String,
    entity_type: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<EntitySummary>, String> {
    let service = state.service.lock().await;
    service.search_entities(&query, entity_type.as_deref(), limit).await
}

#[tauri::command]
pub async fn find_entity_paths(
    state: State<'_, KnowledgeState>,
    from_id: String,
    to_id: String,
    max_hops: Option<u32>,
) -> Result<PathResult, String> {
    let service = state.service.lock().await;
    service.find_entity_paths(&from_id, &to_id, max_hops).await
}

#[tauri::command]
pub async fn build_knowledge_context(
    state: State<'_, KnowledgeState>,
    text: String,
) -> Result<KnowledgeContext, String> {
    let service = state.service.lock().await;
    service.build_knowledge_context(&text).await
}

#[tauri::command]
pub async fn extract_entities(
    state: State<'_, KnowledgeState>,
    file_path: String,
    namespace: String,
) -> Result<ExtractionCompleteEvent, String> {
    let service = state.service.lock().await;
    service.extract_entities(&file_path, &namespace).await
}

/// Pull Model: extract entities directly from chat text after chat:done event.
/// Uses Heimdall /api/extract with local regex fallback.
#[tauri::command]
pub async fn extract_entities_from_text(
    app: tauri::AppHandle,
    state: State<'_, KnowledgeState>,
    text: String,
    namespace: Option<String>,
    source: Option<String>,
) -> Result<ExtractionCompleteEvent, String> {
    let service = state.service.lock().await;
    let result = service
        .extract_entities_from_text(
            &text,
            namespace.as_deref().unwrap_or("chat"),
            source.as_deref(),
        )
        .await?;
    let _ = app.emit("knowledge:extraction-complete", &result);
    Ok(result)
}

#[tauri::command]
pub async fn get_knowledge_stats(
    state: State<'_, KnowledgeState>,
) -> Result<KnowledgeStats, String> {
    let service = state.service.lock().await;
    service.get_stats().await
}

#[tauri::command]
pub async fn get_namespaces(
    state: State<'_, KnowledgeState>,
) -> Result<Vec<NamespaceInfo>, String> {
    let service = state.service.lock().await;
    service.get_namespaces().await
}

#[tauri::command]
pub async fn get_neighbors(
    state: State<'_, KnowledgeState>,
    entity_id: String,
    namespace: Option<String>,
) -> Result<NeighborResult, String> {
    let service = state.service.lock().await;
    service.get_neighbors(&entity_id, namespace.as_deref()).await
}

/// Save chat conversation to knowledge wiki as structured .md.
/// This is a synchronous I/O operation — no .await needed.
#[tauri::command]
pub async fn save_chat_to_knowledge(
    state: State<'_, KnowledgeState>,
    session_title: String,
    messages_json: String,
    namespace: Option<String>,
) -> Result<ChatKnowledgeSaveResult, String> {
    let service = state.service.lock().await;
    service.save_chat_to_knowledge(
        &session_title,
        &messages_json,
        namespace.as_deref().unwrap_or("chat"),
    )
}

#[tauri::command]
pub async fn rescan_wiki(
    state: State<'_, KnowledgeState>,
) -> Result<ScanWikiResult, String> {
    let service = state.service.lock().await;
    service.scan_wiki_directory()
}

#[tauri::command]
pub async fn get_daily_digest(
    state: State<'_, KnowledgeState>,
) -> Result<DailyDigest, String> {
    let service = state.service.lock().await;
    service.get_daily_digest().await
}

#[tauri::command]
pub async fn reference_entity_to_chat(
    state: State<'_, KnowledgeState>,
    entity_id: String,
) -> Result<EntityReference, String> {
    let service = state.service.lock().await;
    service.reference_entity_to_chat(&entity_id).await
}

#[tauri::command]
pub async fn get_smart_display(
    state: State<'_, KnowledgeState>,
    namespace: Option<String>,
    config: SmartDisplayConfig,
) -> Result<SmartDisplayResult, String> {
    let service = state.service.lock().await;
    service.get_smart_display(namespace.as_deref(), &config).await
}

#[tauri::command]
pub async fn get_inferences(
    state: State<'_, KnowledgeState>,
    namespace: Option<String>,
    limit: Option<u32>,
    status: Option<String>,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().await;
    service.get_inferences(namespace.as_deref(), limit, status.as_deref()).await
}

// ── Nexus Knowledge Engine Commands ──

#[tauri::command]
pub async fn nexus_store(
    state: State<'_, KnowledgeState>,
    text: String,
    source_type: String,
    source_path: Option<String>,
    context: Option<String>,
) -> Result<NexusStoreResult, String> {
    let service = state.service.lock().await;
    service.nexus_store(
        &text,
        &source_type,
        source_path.as_deref(),
        context.as_deref(),
    )
}

#[tauri::command]
pub async fn nexus_extract_from_file(
    state: State<'_, KnowledgeState>,
    file_path: String,
    source_type: Option<String>,
) -> Result<NexusStoreResult, String> {
    let service = state.service.lock().await;
    service.nexus_extract_from_file(&file_path, source_type.as_deref())
}

#[tauri::command]
pub async fn nexus_reindex_all(
    state: State<'_, KnowledgeState>,
) -> Result<NexusReindexResult, String> {
    let service = state.service.lock().await;
    service.nexus_reindex_all()
}

#[tauri::command]
pub async fn nexus_run_synthesis(
    state: State<'_, KnowledgeState>,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().await;
    service.nexus_run_synthesis()
}

#[tauri::command]
pub async fn nexus_analyze_types(
    state: State<'_, KnowledgeState>,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().await;
    service.nexus_analyze_types()
}

// ── Nexus P5: Entity/Relation CRUD + Feedback + Merge ──

#[tauri::command]
pub async fn nexus_update_entity(
    state: State<'_, KnowledgeState>,
    entity_id: String,
    updates: serde_json::Value,
) -> Result<(), String> {
    let service = state.service.lock().await;
    service.nexus_update_entity(&entity_id, &updates)
}

#[tauri::command]
pub async fn nexus_delete_entity(
    state: State<'_, KnowledgeState>,
    entity_id: String,
) -> Result<(), String> {
    let service = state.service.lock().await;
    service.nexus_delete_entity(&entity_id)
}

#[tauri::command]
pub async fn nexus_add_relation(
    state: State<'_, KnowledgeState>,
    from_id: String,
    to_id: String,
    relation_type: String,
    label: Option<String>,
    confidence: Option<f64>,
    namespace: Option<String>,
) -> Result<String, String> {
    let service = state.service.lock().await;
    service.nexus_add_relation(&from_id, &to_id, &relation_type, label.as_deref(), confidence, namespace.as_deref())
}

#[tauri::command]
pub async fn nexus_update_relation(
    state: State<'_, KnowledgeState>,
    relation_id: String,
    relation_type: Option<String>,
    label: Option<String>,
    confidence: Option<f64>,
) -> Result<(), String> {
    let service = state.service.lock().await;
    service.nexus_update_relation(&relation_id, relation_type.as_deref(), label.as_deref(), confidence)
}

#[tauri::command]
pub async fn nexus_delete_relation(
    state: State<'_, KnowledgeState>,
    relation_id: String,
) -> Result<(), String> {
    let service = state.service.lock().await;
    service.nexus_delete_relation(&relation_id)
}

#[tauri::command]
pub async fn nexus_submit_feedback(
    state: State<'_, KnowledgeState>,
    entity_id: String,
    action: String,
    reason: Option<String>,
) -> Result<(), String> {
    let service = state.service.lock().await;
    service.nexus_submit_feedback(&entity_id, &action, reason.as_deref())
}

#[tauri::command]
pub async fn nexus_get_pending_merges(
    state: State<'_, KnowledgeState>,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().await;
    service.nexus_get_pending_merges()
}

#[tauri::command]
pub async fn nexus_confirm_merge(
    state: State<'_, KnowledgeState>,
    merge_id: String,
) -> Result<(), String> {
    let service = state.service.lock().await;
    service.nexus_confirm_merge(&merge_id)
}

#[tauri::command]
pub async fn nexus_ignore_merge(
    state: State<'_, KnowledgeState>,
    merge_id: String,
) -> Result<(), String> {
    let service = state.service.lock().await;
    service.nexus_ignore_merge(&merge_id)
}

#[tauri::command]
pub async fn nexus_batch_operation(
    state: State<'_, KnowledgeState>,
    action: String,
    namespace: Option<String>,
    source_type: Option<String>,
    min_confidence: Option<f64>,
    entity_ids: Option<Vec<String>>,
) -> Result<BatchOperationResult, String> {
    let service = state.service.lock().await;
    let affected = service.nexus_batch_operation(
        &action,
        namespace.as_deref(),
        source_type.as_deref(),
        min_confidence,
        entity_ids.as_deref(),
    )?;
    Ok(BatchOperationResult { affected, action })
}

#[tauri::command]
pub async fn nexus_get_entity_feedback(
    state: State<'_, KnowledgeState>,
    entity_id: String,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().await;
    service.nexus_get_entity_feedback(&entity_id)
}

// ── Document Summarization & Image Description ──

/// Summarize a document (pdf/docx/pptx): extract text → LLM summary → save .md.
/// Returns the relative wiki path of the generated .md summary file.
#[tauri::command]
pub async fn nexus_summarize_document(
    state: State<'_, KnowledgeState>,
    file_path: String,
) -> Result<String, String> {
    let service = state.service.lock().await;
    service.nexus_summarize_document(&file_path)
}

/// Describe one or more images via multimodal LLM and save as .md.
/// Single image → `<stem>.md`, multiple images → `<title>.md`.
/// Returns the relative wiki path of the generated .md file.
#[tauri::command]
pub async fn nexus_describe_images(
    state: State<'_, KnowledgeState>,
    file_paths: Vec<String>,
    title: Option<String>,
) -> Result<String, String> {
    let service = state.service.lock().await;
    service.nexus_describe_images(&file_paths, title.as_deref())
}

/// Auto-classify and archive a document: extract text → LLM classify → move to folder → add frontmatter.
/// Returns JSON {folder, title, tags, file_path}.
#[tauri::command]
pub async fn nexus_auto_classify(
    state: State<'_, KnowledgeState>,
    file_path: String,
) -> Result<serde_json::Value, String> {
    let service = state.service.lock().await;
    service.nexus_auto_classify(&file_path)
}

// ── Nexus Maintenance Commands ──

#[tauri::command]
pub async fn nexus_maintain_quality(
    state: State<'_, KnowledgeState>,
) -> Result<MaintenanceReport, String> {
    let service = state.service.lock().await;
    service.nexus_maintain_quality()
}

#[tauri::command]
pub async fn nexus_maintain_cleanup(
    state: State<'_, KnowledgeState>,
) -> Result<MaintenanceReport, String> {
    let service = state.service.lock().await;
    service.nexus_maintain_cleanup()
}

#[tauri::command]
pub async fn nexus_maintain_dedup(
    state: State<'_, KnowledgeState>,
) -> Result<MaintenanceReport, String> {
    let service = state.service.lock().await;
    service.nexus_maintain_dedup()
}

#[tauri::command]
pub async fn nexus_maintain_fix_migrated(
    state: State<'_, KnowledgeState>,
) -> Result<MaintenanceReport, String> {
    let service = state.service.lock().await;
    service.nexus_maintain_fix_migrated()
}

#[tauri::command]
pub async fn nexus_get_maintenance_status(
    state: State<'_, KnowledgeState>,
) -> Result<MaintenanceStatus, String> {
    let service = state.service.lock().await;
    service.nexus_get_maintenance_status()
}

// ── Layer 2-6 Extended Maintenance ──

#[tauri::command]
pub async fn nexus_maintain_classify(
    state: State<'_, KnowledgeState>,
    full_scan: Option<bool>,
) -> Result<MaintenanceReport, String> {
    let service = state.service.lock().await;
    service.nexus_maintain_classify(full_scan.unwrap_or(false))
}

#[tauri::command]
pub async fn nexus_run_pagerank(
    state: State<'_, KnowledgeState>,
) -> Result<PageRankReport, String> {
    let service = state.service.lock().await;
    service.nexus_run_pagerank()
}

#[tauri::command]
pub async fn nexus_run_community(
    state: State<'_, KnowledgeState>,
) -> Result<CommunityReport, String> {
    let service = state.service.lock().await;
    service.nexus_run_community()
}

#[tauri::command]
pub async fn nexus_discover_causal(
    state: State<'_, KnowledgeState>,
    entity_id: String,
) -> Result<CausalChainReport, String> {
    let service = state.service.lock().await;
    service.nexus_discover_causal(&entity_id)
}

#[tauri::command]
pub async fn nexus_run_transitive(
    state: State<'_, KnowledgeState>,
) -> Result<TransitiveReport, String> {
    let service = state.service.lock().await;
    service.nexus_run_transitive()
}

#[tauri::command]
pub async fn nexus_scan_conflicts(
    state: State<'_, KnowledgeState>,
) -> Result<ConflictReport, String> {
    let service = state.service.lock().await;
    service.nexus_scan_conflicts()
}

#[tauri::command]
pub async fn nexus_get_evolution(
    state: State<'_, KnowledgeState>,
    entity_id: String,
) -> Result<EvolutionReport, String> {
    let service = state.service.lock().await;
    service.nexus_get_evolution(&entity_id)
}

#[tauri::command]
pub async fn nexus_verify_synthesis(
    state: State<'_, KnowledgeState>,
) -> Result<VerifyReport, String> {
    let service = state.service.lock().await;
    service.nexus_verify_synthesis()
}
