import { invoke } from "@tauri-apps/api/core";
import type { GraphData, EntityDetail, EntitySummary } from "../types/knowledge";
import type { FileNode } from "../types/wiki";

// ── Chat ──

export function chatSend(messages: Array<{ role: string; content: string }>, model: string, sessionId: string | null) {
  return invoke("chat_completions", { messages, model, sessionId });
}

export function abortChat() {
  return invoke("abort_chat");
}

export function generateTitle(firstUserMsg: string, firstAiMsg: string, model: string): Promise<string> {
  return invoke("generate_title", { firstUserMsg, firstAiMsg, model });
}

// ── Knowledge ──

export function getGraphData(namespace?: string, viewMode?: string): Promise<GraphData> {
  return invoke("get_graph_data", { namespace, viewMode });
}

export function getEntityDetail(entityId: string): Promise<EntityDetail> {
  return invoke("get_entity_detail", { entityId });
}

export function getLintWarnings(namespace?: string | null): Promise<Array<{
  warning_type: string;
  entity_id?: string;
  entity_name: string;
  message: string;
  severity: string;
}>> {
  return invoke("get_lint_warnings", { namespace });
}

export function searchEntities(query: string, entityType?: string, limit?: number): Promise<EntitySummary[]> {
  return invoke("search_entities", { query, entityType, limit });
}

export function referenceEntityToChat(entityId: string): Promise<{ entity_name: string; entity_type: string; summary: string; markdown_ref: string }> {
  return invoke("reference_entity_to_chat", { entityId });
}

export function getSmartDisplay(namespace: string | null, config: {
  mode: string;
  focal_node: string | null;
  hops: number;
  budget: number;
  min_per_type: number;
  tier1_cap: number;
  tier2_cap: number;
  search_query?: string | null;
  type_filter?: string | null;
  namespace_filter?: string | null;
  min_importance?: number | null;
  show_orphans?: boolean | null;
}): Promise<GraphData> {
  return invoke("get_smart_display", { namespace, config });
}

export function getInferences(namespace?: string | null, limit?: number, status?: string): Promise<Array<{
  id: string;
  from_id: string;
  to_id: string;
  relation_type: string;
  confidence: number;
  reason: string;
  status: string;
}>> {
  return invoke("get_inferences", { namespace: namespace ?? null, limit: limit ?? null, status: status ?? null });
}

export function extractEntitiesFromText(text: string, namespace?: string, source?: string) {
  return invoke("extract_entities_from_text", { text, namespace, source });
}

export function saveChatToKnowledge(title: string, content: string, namespace?: string) {
  return invoke("save_chat_to_knowledge", { sessionTitle: title, messagesJson: content, namespace: namespace ?? null });
}

// ── Wiki ──

export function getWikiFileTree(): Promise<FileNode[]> {
  return invoke("get_wiki_file_tree");
}

export function readWikiFile(path: string): Promise<string> {
  return invoke("read_wiki_file", { path });
}

export function readWikiFileBase64(path: string): Promise<string> {
  return invoke("read_wiki_file_base64", { path });
}

export function writeWikiFile(path: string, content: string): Promise<void> {
  return invoke("write_wiki_file", { path, content });
}

export function createWikiItem(parentPath: string, name: string, kind: string): Promise<void> {
  return invoke("create_wiki_item", { parentPath, name, kind });
}

export function deleteWikiItem(path: string): Promise<void> {
  return invoke("delete_wiki_item", { path });
}

export function renameWikiItem(path: string, newName: string): Promise<void> {
  return invoke("rename_wiki_item", { path, newName });
}

export function moveWikiItem(fromPath: string, toPath: string): Promise<void> {
  return invoke("move_wiki_item", { fromPath, toPath });
}

export function showInFolder(path: string): Promise<void> {
  return invoke("show_in_folder", { path });
}

export function uploadWikiFile(sourcePath: string, targetDir?: string, targetName?: string): Promise<string> {
  return invoke("upload_wiki_file", { sourcePath, targetDir, targetName });
}

export function uploadWikiFiles(paths: string[], targetDir?: string): Promise<string[]> {
  return invoke("upload_wiki_files", { paths, targetDir });
}

export function nexusExtractFromFile(filePath: string, sourceType?: string) {
  return invoke("nexus_extract_from_file", { filePath, sourceType });
}

export function getNamespaces(): Promise<Array<{ name: string; entity_count: number; relation_count: number }>> {
  return invoke("get_namespaces");
}

export function listWikiDirs(namespace?: string): Promise<string[]> {
  return invoke("list_wiki_dirs", { namespace: namespace ?? null });
}

export function listAllKnowledgeFiles(namespace?: string): Promise<Array<{ path: string; name: string; size: number; modified: number; file_type: string; title: string; tags: string[] }>> {
  return invoke("list_all_knowledge_files", { namespace: namespace ?? null });
}

// ── Canvas ──

export function canvasOpen(path: string): Promise<string> {
  return invoke("canvas_open", { path });
}

export function canvasSave(path: string, content: string): Promise<void> {
  return invoke("canvas_save", { path, content });
}

// ── Session ──

export function listSessions() {
  return invoke("list_sessions");
}

export function getSession(sessionId: string) {
  return invoke("get_session", { sessionId });
}

export function deleteSession(sessionId: string) {
  return invoke("delete_session", { sessionId });
}

export function renameSession(sessionId: string, title: string) {
  return invoke("rename_session", { sessionId, title });
}

// ── Auth ──

export function registerUser(username: string, email: string, password: string) {
  return invoke("register_user", { username, email, password });
}

export function loginUser(username: string, password: string) {
  return invoke("login_user", { username, password });
}

export function getCurrentUser() {
  return invoke("get_current_user");
}

export function changePassword(username: string, oldPassword: string, newPassword: string) {
  return invoke("change_password", { username, oldPassword, newPassword });
}

// ── Config ──

export function getConfig() {
  return invoke("get_config");
}

export function saveConfig(updates: any) {
  return invoke("save_config", { updates });
}

export function updateApiKey(provider: string, apiKey: string) {
  return invoke("update_api_key", { provider, apiKey });
}

export function saveUserProfile(name: string, email: string) {
  return invoke("save_user_profile", { name, email });
}

export function exportData() {
  return invoke("export_data");
}

export function importData(zipPath: string) {
  return invoke("import_data", { zipPath });
}

export function listEnvKeys() {
  return invoke("list_env_keys");
}

// ── Agent ──

export function getAgentStatus() {
  return invoke("agent_status");
}

export function restartAgent() {
  return invoke("restart_agent");
}

export function getAgentLogs(lines?: number) {
  return invoke("get_agent_logs", { lines: lines ?? 50 });
}

// ── Voice ──

export function checkVoiceDeps(): Promise<string[]> {
  return invoke("check_voice_deps");
}

export function ttsSpeak(text: string, voice?: string): Promise<string> {
  return invoke("tts_speak", { text, voice });
}

export function voiceDiagnose(): Promise<string> {
  return invoke("voice_diagnose");
}

// ── Files ──

export function readTextFile(path: string): Promise<string> {
  return invoke("read_text_file", { path });
}

export function readFileBase64(path: string): Promise<string> {
  return invoke("read_file_base64", { path });
}

export function captureScreen(): Promise<string> {
  return invoke("capture_screen");
}

// ── Avatar ──

export function saveAvatar(base64Data: string): Promise<void> {
  return invoke("save_avatar", { base64Data });
}

export function getAvatar(): Promise<string> {
  return invoke("get_avatar");
}

// ── Nexus P5: Entity Editor ──

export function nexusUpdateEntity(entityId: string, updates: Record<string, unknown>) {
  return invoke("nexus_update_entity", { entityId, updates });
}

export function nexusDeleteEntity(entityId: string) {
  return invoke("nexus_delete_entity", { entityId });
}

export function nexusAddRelation(fromId: string, toId: string, relationType: string, label?: string, confidence?: number, namespace?: string) {
  return invoke("nexus_add_relation", { fromId, toId, relationType, label, confidence, namespace });
}

export function nexusUpdateRelation(relationId: string, relationType?: string, label?: string, confidence?: number) {
  return invoke("nexus_update_relation", { relationId, relationType, label, confidence });
}

export function nexusDeleteRelation(relationId: string) {
  return invoke("nexus_delete_relation", { relationId });
}

export function nexusSubmitFeedback(entityId: string, action: string, reason?: string) {
  return invoke("nexus_submit_feedback", { entityId, action, reason });
}

export function nexusGetPendingMerges() {
  return invoke("nexus_get_pending_merges");
}

export function nexusConfirmMerge(mergeId: string) {
  return invoke("nexus_confirm_merge", { mergeId });
}

export function nexusIgnoreMerge(mergeId: string) {
  return invoke("nexus_ignore_merge", { mergeId });
}

export function nexusBatchOperation(action: string, namespace?: string, sourceType?: string, minConfidence?: number, entityIds?: string[]) {
  return invoke("nexus_batch_operation", { action, namespace, sourceType, minConfidence, entityIds });
}

export function nexusGetEntityFeedback(entityId: string) {
  return invoke("nexus_get_entity_feedback", { entityId });
}

export function nexusRunSynthesis() {
  return invoke("nexus_run_synthesis");
}

export function nexusAnalyzeTypes() {
  return invoke("nexus_analyze_types");
}

export function nexusSummarizeDocument(filePath: string): Promise<string> {
  return invoke("nexus_summarize_document", { filePath });
}

export function nexusDescribeImages(filePaths: string[], title?: string): Promise<string> {
  return invoke("nexus_describe_images", { filePaths, title: title ?? null });
}

export function nexusAutoClassify(filePath: string): Promise<{ folder: string; title: string; tags: string[]; file_path: string; skipped?: boolean; reason?: string }> {
  return invoke("nexus_auto_classify", { filePath });
}

// ── Nexus Maintenance ──

export interface MaintenanceReport {
  task: string;
  status: string;
  started_at: string;
  completed_at: string;
  entities_scanned: number;
  entities_fixed: number;
  llm_calls: number;
  tokens_used: number;
  summary: string;
  details: MaintenanceDetail[];
}

export interface MaintenanceDetail {
  entity_id: string | null;
  entity_name: string;
  action: string;
  reason: string;
}

export interface MaintenanceStatus {
  last_maintenance: string | null;
  total_entities: number;
  low_quality_count: number;
  orphan_count: number;
  stale_count: number;
  duplicate_candidates: number;
  migration_needs_fix: number;
  recent_tasks: MaintenanceTaskSummary[];
}

export interface MaintenanceTaskSummary {
  task: string;
  status: string;
  completed_at: string | null;
  entities_fixed: number;
  summary: string;
}

export function nexusMaintainQuality(): Promise<MaintenanceReport> {
  return invoke("nexus_maintain_quality");
}

export function nexusMaintainCleanup(): Promise<MaintenanceReport> {
  return invoke("nexus_maintain_cleanup");
}

export function nexusMaintainDedup(): Promise<MaintenanceReport> {
  return invoke("nexus_maintain_dedup");
}

export function nexusMaintainFixMigrated(): Promise<MaintenanceReport> {
  return invoke("nexus_maintain_fix_migrated");
}

export function nexusGetMaintenanceStatus(): Promise<MaintenanceStatus> {
  return invoke("nexus_get_maintenance_status");
}

export function nexusReindexForce(): Promise<{
  files_processed: number;
  entities_total: number;
  relations_total: number;
  skipped: number;
  errors: string[];
}> {
  return invoke("nexus_reindex_force");
}

// ── Layer 2-6 Extended Maintenance ──

export interface PageRankReport {
  total_entities: number;
  iterations: number;
  converged: boolean;
  top_entities: PageRankEntry[];
  core_count: number;
}

export interface PageRankEntry {
  entity_id: string;
  name: string;
  score: number;
}

export interface CommunityReport {
  communities: number;
  modularity: number;
  total_entities: number;
  iterations: number;
  assignments: Array<{ entity_id: string; community_id: number }>;
}

export interface CausalChainReport {
  entity_id: string;
  entity_name: string;
  forward_chains: CausalStep[][];
  backward_chains: CausalStep[][];
}

export interface CausalStep {
  entity_id: string;
  entity_name: string;
  relation_type: string;
  depth: number;
}

export interface TransitiveReport {
  scanned: number;
  inferred: number;
  skipped_existing: number;
}

export interface ConflictReport {
  scanned_pairs: number;
  conflicts_found: number;
  conflicts: ConflictEntry[];
}

export interface ConflictEntry {
  entity_a: string;
  entity_b: string;
  relation_a: string;
  relation_b: string;
  target: string;
}

export interface EvolutionReport {
  entity_id: string;
  entity_name: string;
  time_windows: number;
  summary: string;
}

export interface VerifyReport {
  total_edges: number;
  verified: number;
  rejected: number;
  batches: number;
  llm_calls: number;
}

export function nexusMaintainClassify(fullScan?: boolean): Promise<MaintenanceReport> {
  return invoke("nexus_maintain_classify", { fullScan: fullScan ?? false });
}

export function nexusRunPagerank(): Promise<PageRankReport> {
  return invoke("nexus_run_pagerank");
}

export function nexusRunCommunity(): Promise<CommunityReport> {
  return invoke("nexus_run_community");
}

export function nexusDiscoverCausal(entityId: string): Promise<CausalChainReport> {
  return invoke("nexus_discover_causal", { entityId });
}

export function nexusRunTransitive(): Promise<TransitiveReport> {
  return invoke("nexus_run_transitive");
}

export function nexusScanConflicts(): Promise<ConflictReport> {
  return invoke("nexus_scan_conflicts");
}

export function nexusGetEvolution(entityId: string): Promise<EvolutionReport> {
  return invoke("nexus_get_evolution", { entityId });
}

export function nexusResetGraph(): Promise<{
  ok: boolean;
  deleted_entities: number;
  deleted_relations: number;
  cleared_content_index: number;
  kept_document_entities: number;
  next_step: string;
}> {
  return invoke("nexus_reset_graph");
}

export function nexusVerifySynthesis(): Promise<VerifyReport> {
  return invoke("nexus_verify_synthesis");
}

// ── Gateway ──

export interface GatewayPlatform {
  key: string;
  label: string;
  emoji?: string;
  description: string;
  timeout_seconds: number;
}

export interface PlatformConfigStatus {
  key: string;
  label: string;
  enabled: boolean;
  configured: boolean;
  has_qr: boolean;
}

export interface QrSessionInfo {
  session_id: string;
  platform: string;
  qr_url: string;
  timeout_seconds: number;
}

export interface QrPollResult {
  status: string;
  message?: string;
  qr_url?: string;
  credentials?: Record<string, string>;
}

export function listGatewayPlatforms(): Promise<GatewayPlatform[]> {
  return invoke("list_gateway_platforms");
}

export function listPlatformStatus(): Promise<PlatformConfigStatus[]> {
  return invoke("list_platform_status");
}

export function gatewayQrStart(platform: string): Promise<QrSessionInfo> {
  return invoke("gateway_qr_start", { platform });
}

export function gatewayQrPoll(platform: string): Promise<QrPollResult> {
  return invoke("gateway_qr_poll", { platform });
}

export function gatewayQrCancel(platform: string): Promise<void> {
  return invoke("gateway_qr_cancel", { platform });
}

export function gatewayGetActiveSession(platform: string): Promise<QrSessionInfo | null> {
  return invoke("gateway_get_active_session", { platform });
}

export function gatewayGetConfig(): Promise<unknown> {
  return invoke("gateway_get_config");
}

export function gatewaySaveCredentials(platform: string, credentials: Record<string, string>): Promise<void> {
  return invoke("gateway_save_credentials", { platform, credentials });
}

export function gatewaySavePlatformConfig(platform: string, config: unknown): Promise<void> {
  return invoke("gateway_save_platform_config", { platform, config });
}

export function gatewayRemovePlatform(platform: string): Promise<void> {
  return invoke("gateway_remove_platform", { platform });
}

// ── Cron ──

export interface CronJob {
  id: string;
  name: string;
  schedule: { kind: string; expr?: string; run_at?: string; minutes?: number; display: string };
  schedule_display: string;
  prompt?: string;
  enabled: boolean;
  state: string;
  deliver?: string;
  skills?: string[];
  created_at: string;
  next_run_at: string | null;
  last_run_at: string | null;
  last_status: string | null;
  last_error: string | null;
}

export function listCronJobs(): Promise<CronJob[]> {
  return invoke("list_cron_jobs");
}

export function addCronJob(job: Record<string, unknown>): Promise<CronJob> {
  return invoke("add_cron_job", { job });
}

export function updateCronJob(jobId: string, updates: Record<string, unknown>): Promise<void> {
  return invoke("update_cron_job", { jobId, updates });
}

export function deleteCronJob(jobId: string): Promise<void> {
  return invoke("delete_cron_job", { jobId });
}

export function toggleCronJob(jobId: string, enabled: boolean): Promise<void> {
  return invoke("toggle_cron_job", { jobId, enabled });
}

export function triggerCronJob(jobId: string): Promise<void> {
  return invoke("trigger_cron_job", { jobId });
}

export function getCronOutput(jobId: string): Promise<string[]> {
  return invoke("get_cron_output", { jobId });
}
