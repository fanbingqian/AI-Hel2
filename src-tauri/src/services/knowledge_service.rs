use crate::models::knowledge::*;
use rusqlite::Connection as SqliteConnection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// UUID v5 namespace for deterministic Wiki entity IDs.
/// Uses a custom namespace so same-name entities share the same ID across documents.
const WIKI_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1,
    0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

/// Max entities extracted per document to prevent graph noise.
const MAX_ENTITIES_PER_DOC: u32 = 30;

/// Stop words filtered from entity name matches.
const STOP_WORDS: &[&str] = &[
    "的", "了", "在", "是", "我", "有", "和", "就", "不", "人", "都", "一", "一个",
    "这个", "那个", "什么", "怎么", "哪", "吗", "吧", "呢", "啊",
    "the", "a", "an", "is", "are", "was", "were", "this", "that",
    "it", "of", "in", "on", "to", "for", "and", "or", "but",
    "test", "new", "old", "foo", "bar", "baz",
];

/// Passthrough — entity_type is now free-form String, stored as-is in DB.
fn parse_entity_type(raw: &str) -> EntityType {
    raw.trim_matches('"').to_string()
}

/// Passthrough — relation_type is now free-form String, stored as-is in DB.
fn parse_relation_type(raw: &str) -> RelationType {
    raw.trim_matches('"').to_string()
}

pub struct KnowledgeService {
    heimdall_url: String,
    client: reqwest::Client,
    cache_db: Mutex<SqliteConnection>,
    wiki_dir: PathBuf,
    hermes_home: PathBuf,
}

impl KnowledgeService {
    pub fn new(hermes_home: &Path) -> Result<Self, String> {
        let cache_path = hermes_home.join("knowledge_cache.db");
        let db = SqliteConnection::open(&cache_path)
            .map_err(|e| format!("无法打开知识缓存数据库: {e}"))?;

        let wiki_dir = hermes_home.join("wiki");

        let service = Self {
            heimdall_url: "http://127.0.0.1:8765".into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            cache_db: Mutex::new(db),
            wiki_dir,
            hermes_home: hermes_home.to_path_buf(),
        };

        service.init_cache_tables()?;
        service.migrate_from_heimdall_db()?;
        Ok(service)
    }

    /// Return the wiki directory path (for file system operations).
    pub fn wiki_dir(&self) -> PathBuf {
        self.wiki_dir.clone()
    }

    /// Recursively scan the wiki directory: extract entities from all existing .md files,
    /// then clean up entities whose source files no longer exist on disk.
    /// Called on startup and on manual reindex.
    pub fn scan_wiki_directory(&self) -> Result<ScanWikiResult, String> {
        let mut scanned = 0u32;
        let mut failed = 0u32;
        let mut total_new = 0u32;
        let total_updated = 0u32;
        let mut errors: Vec<String> = Vec::new();

        // Phase 1: Collect all existing .md file paths
        let mut existing_files: std::collections::HashSet<String> = std::collections::HashSet::new();
        self.collect_md_files(&self.wiki_dir, &mut existing_files);

        // Phase 2: Extract/re-extract entities from each file via Nexus (LLM extraction)
        for file_path in &existing_files {
            match self.nexus_extract_from_file(file_path, None) {
                Ok(result) => {
                    scanned += 1;
                    total_new += result.entity_count;
                }
                Err(e) => {
                    failed += 1;
                    errors.push(format!("{file_path}: {e}"));
                }
            }
        }

        // Phase 3: Clean up stale entities from deleted files
        let (stale_entities, stale_relations) = self.cleanup_stale_wiki_entities(&existing_files)?;

        if scanned > 0 || failed > 0 || stale_entities > 0 {
            log::info!(
                "Wiki scan: {} files scanned ({} new, {} updated), {} failed, {} stale entities cleaned",
                scanned, total_new, total_updated, failed, stale_entities
            );
        }

        Ok(ScanWikiResult {
            scanned,
            failed,
            total_new,
            total_updated,
            stale_entities_removed: stale_entities,
            stale_relations_removed: stale_relations,
            errors,
        })
    }

    /// Collect all .md file paths (absolute) recursively under a directory.
    pub fn collect_md_files(&self, dir: &Path, files: &mut std::collections::HashSet<String>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name == "_trash" {
                    continue;
                }
                self.collect_md_files(&path, files);
            } else if path.extension().map_or(false, |ext| ext == "md") {
                files.insert(path.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    /// Remove entities and relations whose source file no longer exists in the wiki directory.
    /// Returns (entities_removed, relations_removed).
    pub fn cleanup_stale_wiki_entities(
        &self,
        existing_files: &std::collections::HashSet<String>,
    ) -> Result<(u32, u32), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let wiki_prefix = self.wiki_dir.to_string_lossy().replace('\\', "/");

        // Find all wiki-file-sourced entities (source_file starts with wiki dir path)
        let mut stmt = db
            .prepare("SELECT id, source_file FROM cache_entities WHERE source_file LIKE ?1")
            .map_err(|e| e.to_string())?;

        let mut stale_entity_ids: Vec<String> = Vec::new();
        let like_pattern = format!("{wiki_prefix}%");
        let rows = stmt
            .query_map(rusqlite::params![like_pattern], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        for row in rows {
            let (entity_id, source_file) = row.map_err(|e| e.to_string())?;
            if let Some(sf) = source_file {
                // Normalize path separators for comparison
                let sf = sf.replace('\\', "/");
                if !existing_files.contains(&sf)
                    && !existing_files.iter().any(|f| f.ends_with(&sf) || sf.ends_with(f))
                {
                    stale_entity_ids.push(entity_id);
                }
            }
        }

        if stale_entity_ids.is_empty() {
            return Ok((0, 0));
        }

        // Delete stale relations first, then entities
        let mut relations_removed = 0u32;
        for chunk in stale_entity_ids.chunks(50) {
            let placeholders: Vec<String> = chunk.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let placeholders2: Vec<String> = chunk.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let sql_rel = format!(
                "DELETE FROM cache_relations WHERE from_id IN ({}) OR to_id IN ({})",
                placeholders.join(","),
                placeholders2.join(",")
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for id in chunk {
                params.push(Box::new(id.clone()));
            }
            for id in chunk {
                params.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            relations_removed += db
                .execute(&sql_rel, param_refs.as_slice())
                .map_err(|e| e.to_string())? as u32;
        }

        let mut entities_removed = 0u32;
        for chunk in stale_entity_ids.chunks(50) {
            let placeholders: Vec<String> = chunk.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let sql_ent = format!("DELETE FROM cache_entities WHERE id IN ({})", placeholders.join(","));
            let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();
            entities_removed += db
                .execute(&sql_ent, params.as_slice())
                .map_err(|e| e.to_string())? as u32;
        }

        log::info!(
            "Cleaned up {} stale entities and {} relations from deleted wiki files",
            entities_removed,
            relations_removed
        );
        Ok((entities_removed, relations_removed))
    }

    /// Run pending schema migrations, then backfill Nexus columns for old data.
    fn run_migrations(db: &SqliteConnection) -> Result<(), String> {
        // Ensure cache_meta table exists (may not if this is a fresh database)
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS cache_meta (key TEXT PRIMARY KEY, value TEXT);"
        ).map_err(|e| format!("Migration: create cache_meta failed: {e}"))?;

        let current_version: i32 = db
            .query_row(
                "SELECT COALESCE(CAST(value AS INTEGER), 0) FROM cache_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Embedded migrations — keep in sync with src-tauri/migrations/*.sql
        let migrations: &[(&str, &str)] = &[
            ("001_init", include_str!("../../migrations/001_init.sql")),
            ("002_nexus_schema", include_str!("../../migrations/002_nexus_schema.sql")),
            ("003_feedback_reason", include_str!("../../migrations/003_feedback_reason.sql")),
            ("004_relations_columns", include_str!("../../migrations/004_relations_columns.sql")),
            ("005_inferred_edges", include_str!("../../migrations/005_inferred_edges.sql")),
        ];

        for (i, (_name, sql)) in migrations.iter().enumerate() {
            let version = (i + 1) as i32;
            if version > current_version {
                log::info!("[Migration] Running migration {version}: {_name}");
                db.execute_batch(sql)
                    .map_err(|e| format!("Migration {_name} failed: {e}"))?;
                db.execute(
                    "INSERT OR REPLACE INTO cache_meta (key, value) VALUES ('schema_version', ?1)",
                    rusqlite::params![version.to_string()],
                )
                .map_err(|e| format!("Migration: update schema_version failed: {e}"))?;
                log::info!("[Migration] {_name} complete, schema_version={version}");
            }
        }

        // Backfill Nexus columns for existing data (idempotent)
        if current_version < 2 {
            log::info!("[Migration] Running Nexus backfill...");
            // source_type: infer from source_file patterns
            db.execute_batch(
                "UPDATE cache_entities SET source_type = CASE
                    WHEN source_file IS NULL OR source_file = '' THEN 'unknown'
                    WHEN name LIKE '%[[%]]%' THEN 'wikilink'
                    WHEN source_file LIKE 'source:%' THEN 'chat'
                    WHEN source_file LIKE 'chat%' THEN 'chat'
                    WHEN source_file LIKE '%.md' THEN 'wiki'
                    WHEN source_file LIKE 'extract_%' THEN 'chat'
                    ELSE 'wiki'
                END
                WHERE source_type = 'unknown' OR source_type IS NULL;"
            ).map_err(|e| format!("Backfill source_type failed: {e}"))?;

            // namespace: extract from source_file path segments
            db.execute_batch(
                "UPDATE cache_entities SET namespace = 'chat'
                 WHERE (source_file LIKE 'chat%' OR source_file LIKE 'source:%' OR source_file LIKE 'extract_%')
                 AND (namespace = '未分类' OR namespace IS NULL);"
            ).map_err(|e| format!("Backfill namespace failed: {e}"))?;

            // llm_confidence: copy existing confidence value
            db.execute_batch(
                "UPDATE cache_entities SET llm_confidence = confidence
                 WHERE llm_confidence IS NULL;"
            ).map_err(|e| format!("Backfill llm_confidence failed: {e}"))?;

            log::info!("[Migration] Nexus backfill complete");
        }

        Ok(())
    }

    fn init_cache_tables(&self) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("启用 WAL 模式失败: {e}"))?;

        Self::run_migrations(&db)?;

        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS cache_entities (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                description TEXT DEFAULT '',
                aliases TEXT DEFAULT '[]',
                properties TEXT DEFAULT '{}',
                confidence REAL DEFAULT 0.5,
                source_file TEXT,
                created_at TEXT DEFAULT '',
                updated_at TEXT DEFAULT '',
                color TEXT,
                hidden INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS cache_relations (
                id TEXT PRIMARY KEY,
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                relation_type TEXT NOT NULL,
                label TEXT,
                weight REAL DEFAULT 0.5,
                bidirectional INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS cache_meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE IF NOT EXISTS cache_operations_log (
                id TEXT PRIMARY KEY,
                operation TEXT NOT NULL,
                entity_id TEXT,
                entity_name TEXT,
                details TEXT,
                timestamp TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS cache_pending_sync (
                id TEXT PRIMARY KEY,
                sync_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                retries INTEGER DEFAULT 0,
                last_error TEXT
            );
            CREATE TABLE IF NOT EXISTS cache_entity_scores (
                entity_id TEXT PRIMARY KEY,
                manual_boost REAL DEFAULT 0,
                view_count INTEGER DEFAULT 0,
                last_viewed TEXT,
                reference_count INTEGER DEFAULT 0,
                last_referenced TEXT,
                focus_count INTEGER DEFAULT 0,
                last_focused TEXT,
                updated_at TEXT NOT NULL DEFAULT ''
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS cache_entities_fts USING fts5(
                name, description, aliases, content=''
            );
            CREATE TRIGGER IF NOT EXISTS cache_entities_fts_insert AFTER INSERT ON cache_entities BEGIN
                INSERT INTO cache_entities_fts(rowid, name, description, aliases)
                VALUES (new.rowid, new.name, new.description, new.aliases);
            END;
            CREATE TRIGGER IF NOT EXISTS cache_entities_fts_delete AFTER DELETE ON cache_entities BEGIN
                INSERT INTO cache_entities_fts(cache_entities_fts, rowid, name, description, aliases)
                VALUES ('delete', old.rowid, old.name, old.description, old.aliases);
            END;
            CREATE TRIGGER IF NOT EXISTS cache_entities_fts_update AFTER UPDATE ON cache_entities BEGIN
                INSERT INTO cache_entities_fts(cache_entities_fts, rowid, name, description, aliases)
                VALUES ('delete', old.rowid, old.name, old.description, old.aliases);
                INSERT INTO cache_entities_fts(rowid, name, description, aliases)
                VALUES (new.rowid, new.name, new.description, new.aliases);
            END;
",
        )
        .map_err(|e| format!("初始化缓存表失败: {e}"))?;

        // ── Nexus maintenance log & entity snapshots ──
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS cache_maintenance_log (
                id TEXT PRIMARY KEY,
                task TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                entities_scanned INTEGER DEFAULT 0,
                entities_fixed INTEGER DEFAULT 0,
                llm_calls INTEGER DEFAULT 0,
                tokens_used INTEGER DEFAULT 0,
                status TEXT DEFAULT 'running',
                summary TEXT DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS cache_entity_snapshots (
                id TEXT PRIMARY KEY,
                entity_id TEXT NOT NULL,
                desc TEXT NOT NULL,
                captured_at TEXT NOT NULL,
                source_path TEXT,
                FOREIGN KEY (entity_id) REFERENCES cache_entities(id)
            );
            CREATE TABLE IF NOT EXISTS cache_synthesis_log (
                id TEXT PRIMARY KEY,
                task TEXT NOT NULL,
                rule TEXT,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                edges_created INTEGER DEFAULT 0,
                edges_verified INTEGER DEFAULT 0,
                entities_scanned INTEGER DEFAULT 0,
                llm_calls INTEGER DEFAULT 0,
                tokens_used INTEGER DEFAULT 0,
                status TEXT DEFAULT 'running'
            );",
        )
        .map_err(|e| format!("初始化维护表失败: {e}"))?;

        // Migrate legacy entity types to hermes-desktop 8-type system
        let _ = db.execute_batch(
            "UPDATE cache_entities SET entity_type = CASE
                WHEN entity_type = '\"content\"'   THEN '\"document\"'
                WHEN entity_type = '\"event\"'     THEN '\"project\"'
                WHEN entity_type = '\"artifact\"'  THEN '\"tool\"'
                WHEN entity_type IN ('\"function\"','\"class\"','\"method\"','\"interface\"','\"module\"','\"variable\"')
                                                   THEN '\"concept\"'
                ELSE entity_type
            END;
            UPDATE cache_relations SET relation_type = CASE
                WHEN relation_type = '\"references\"'  THEN '\"related_to\"'
                WHEN relation_type = '\"preceded_by\"' THEN '\"related_to\"'
                WHEN relation_type = '\"implements\"'  THEN '\"related_to\"'
                WHEN relation_type = '\"contradicts\"' THEN '\"opposes\"'
                WHEN relation_type = '\"calls\"'       THEN '\"uses\"'
                WHEN relation_type = '\"imports\"'     THEN '\"uses\"'
                WHEN relation_type = '\"defines\"'     THEN '\"contains\"'
                WHEN relation_type = '\"extends\"'     THEN '\"related_to\"'
                WHEN relation_type = '\"parameter_of\"'THEN '\"part_of\"'
                WHEN relation_type = '\"co_occurs\"'   THEN '\"related_to\"'
                ELSE relation_type
            END;
            DROP TABLE IF EXISTS code_projects;",
        );

        // Migrate legacy co_occurs edges → related_to, then purge co_occurs
        let _ = db.execute_batch(
            "INSERT OR IGNORE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
             SELECT hex(randomblob(16)), from_id, to_id, 'related_to', label, weight, bidirectional
             FROM cache_relations WHERE relation_type = 'related_to';
             DELETE FROM cache_relations WHERE relation_type = 'related_to';",
        );

        // Deduplicate before creating UNIQUE index
        let _ = db.execute_batch(
            "DELETE FROM cache_relations WHERE rowid NOT IN (
                SELECT MIN(rowid) FROM cache_relations GROUP BY from_id, to_id, relation_type
            );",
        );

        // Merge case-insensitive duplicate entities (name differs only by case)
        let _ = db.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _dedup_map AS
             SELECT e1.id AS old_id,
                    (SELECT e2.id FROM cache_entities e2 WHERE LOWER(e2.name) = LOWER(e1.name) ORDER BY e2.rowid LIMIT 1) AS new_id
             FROM cache_entities e1
             WHERE e1.id != (SELECT e2.id FROM cache_entities e2 WHERE LOWER(e2.name) = LOWER(e1.name) ORDER BY e2.rowid LIMIT 1);
             UPDATE cache_relations SET from_id = (SELECT new_id FROM _dedup_map WHERE old_id = cache_relations.from_id)
             WHERE from_id IN (SELECT old_id FROM _dedup_map);
             UPDATE cache_relations SET to_id = (SELECT new_id FROM _dedup_map WHERE old_id = cache_relations.to_id)
             WHERE to_id IN (SELECT old_id FROM _dedup_map);
             DELETE FROM cache_entities WHERE id IN (SELECT old_id FROM _dedup_map);
             DROP TABLE IF EXISTS _dedup_map;",
        );

        // UNIQUE index to prevent future duplicates (idempotent via IF NOT EXISTS)
        let _ = db.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_relations_unique
            ON cache_relations(from_id, to_id, relation_type);",
        );

        Ok(())
    }

    /// One-time migration: heimdall.db → knowledge_cache.db on first startup.
    /// Runs only when cache_entities is empty and heimdall.db exists.
    /// After migration, heimdall.db is renamed to heimdall.db.migrated.{date} as backup.
    fn migrate_from_heimdall_db(&self) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Skip if cache_entities already has data
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM cache_entities", [], |r| r.get(0))
            .unwrap_or(0);
        if count > 0 {
            return Ok(());
        }

        // Collect all heimdall.db sources to merge (own + legacy ~/.hermes)
        let own_path = self.hermes_home.join("heimdall").join("heimdall.db");
        let legacy_path = {
            let home = std::env::var("HERMES_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    let base = std::env::var("USERPROFILE")
                        .unwrap_or_else(|_| "C:".into());
                    std::path::PathBuf::from(base).join(".hermes")
                });
            home.join("heimdall").join("heimdall.db")
        };

        let mut paths_to_try: Vec<std::path::PathBuf> = Vec::new();
        if own_path.exists() {
            paths_to_try.push(own_path);
        }
        if legacy_path.exists() && !paths_to_try.contains(&legacy_path) {
            paths_to_try.push(legacy_path);
        }
        if paths_to_try.is_empty() {
            return Ok(());
        }

        log::info!(
            "[Nexus Migration] Found {} heimdall.db source(s) to merge",
            paths_to_try.len()
        );
        let start = std::time::Instant::now();

        let mut total_entities = 0u32;
        let mut total_relations = 0u32;

        for heimdall_db_path in &paths_to_try {
        log::info!(
            "[Nexus Migration] Processing {:?}...",
            heimdall_db_path
        );

        let hdb = rusqlite::Connection::open(&heimdall_db_path)
            .map_err(|e| format!("Failed to open heimdall.db: {e}"))?;

        // --- Phase 1: heimdall_entities → cache_entities ---
        let mut entity_count = 0u32;
        {
            let mut stmt = hdb
                .prepare(
                    "SELECT entity_id, entity_type, display_name, attributes_json, \
                     confidence, first_seen_at, last_seen_at, namespace \
                     FROM heimdall_entities WHERE status IS NULL OR status != 'deleted'",
                )
                .map_err(|e| format!("Failed to query heimdall_entities: {e}"))?;

            let rows: Vec<(
                String,
                String,
                String,
                String,
                f64,
                f64,
                f64,
                String,
            )> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        row.get::<_, Option<f64>>(4)?.unwrap_or(0.5),
                        row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                        row.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                        row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "general".into()),
                    ))
                })
                .map_err(|e| format!("Failed to read heimdall_entities: {e}"))?
                .filter_map(|r| r.ok())
                .collect();

            for (
                entity_id,
                entity_type,
                display_name,
                attributes_json,
                confidence,
                first_seen,
                last_seen,
                namespace,
            ) in &rows
            {
                let description = serde_json::from_str::<serde_json::Value>(attributes_json)
                    .ok()
                    .and_then(|v| {
                        v.get("description")
                            .and_then(|d| d.as_str().map(|s| s.to_string()))
                            .or_else(|| {
                                v.get("summary")
                                    .and_then(|d| d.as_str().map(|s| s.to_string()))
                            })
                    })
                    .unwrap_or_default();

                let mapped_type = match entity_type.as_str() {
                    "content" => "document",
                    "event" => "project",
                    "artifact" => "tool",
                    _ => entity_type.as_str(),
                };

                let created_at = if *first_seen > 0.0 {
                    chrono::DateTime::from_timestamp(*first_seen as i64, 0)
                        .map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let updated_at = if *last_seen > 0.0 {
                    chrono::DateTime::from_timestamp(*last_seen as i64, 0)
                        .map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                db.execute(
                    "INSERT OR IGNORE INTO cache_entities \
                     (id, name, entity_type, description, aliases, properties, \
                      confidence, source_file, created_at, updated_at, namespace, source_type) \
                     VALUES (?1, ?2, ?3, ?4, '[]', '{}', ?5, NULL, ?6, ?7, ?8, 'heimdall_migrated')",
                    rusqlite::params![
                        entity_id,
                        display_name,
                        mapped_type,
                        description,
                        confidence,
                        created_at,
                        updated_at,
                        namespace,
                    ],
                )
                .ok();
                entity_count += 1;
            }
        }

        // --- Phase 2: heimdall_social_graph → cache_relations ---
        let mut relation_count = 0u32;
        {
            let has_social = hdb
                .query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name='heimdall_social_graph'", [], |_| Ok(()))
                .is_ok();

            if has_social {
                let mut stmt = hdb
                    .prepare(
                        "SELECT source_entity_id, target_entity_id, relationship_type, intensity \
                         FROM heimdall_social_graph",
                    )
                    .map_err(|e| format!("Failed to query heimdall_social_graph: {e}"))?;

                let rows: Vec<(String, String, String, f64)> = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?.unwrap_or_else(|| "relates_to".into()),
                            row.get::<_, Option<f64>>(3)?.unwrap_or(0.5),
                        ))
                    })
                    .map_err(|e| format!("Failed to read heimdall_social_graph: {e}"))?
                    .filter_map(|r| r.ok())
                    .collect();

                for (source_id, target_id, rel_type, intensity) in &rows {
                    let relation_type = match rel_type.as_str() {
                        "belongs_to" | "contains" | "causes" | "produces" | "inspired_by" | "knows" => {
                            "related_to"
                        }
                        "contrasts_with" => "opposes",
                        _ => "related_to",
                    };

                    let id = uuid::Uuid::new_v4().to_string();
                    db.execute(
                        "INSERT OR IGNORE INTO cache_relations \
                         (id, from_id, to_id, relation_type, label, weight, namespace, source_type) \
                         VALUES (?1, ?2, ?3, ?4, NULL, ?5, 'general', 'heimdall_migrated')",
                        rusqlite::params![id, source_id, target_id, relation_type, intensity],
                    )
                    .ok();
                    relation_count += 1;
                }
            }
        }

        // --- Phase 3: heimdall_memory_edges → co-occurrence relations ---
        {
            let has_edges = hdb
                .query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name='heimdall_memory_edges'", [], |_| Ok(()))
                .is_ok();

            if has_edges {
                let mut stmt = hdb
                    .prepare(
                        "SELECT memory_id, entity_id FROM heimdall_memory_edges \
                         WHERE memory_id IS NOT NULL ORDER BY memory_id",
                    )
                    .map_err(|e| format!("Failed to query heimdall_memory_edges: {e}"))?;

                let rows: Vec<(String, String)> = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();

                let mut groups: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for (mem_id, entity_id) in &rows {
                    groups.entry(mem_id.clone()).or_default().push(entity_id.clone());
                }

                let mut seen_pairs: std::collections::HashSet<(String, String)> =
                    std::collections::HashSet::new();
                for entities in groups.values() {
                    if entities.len() < 2 {
                        continue;
                    }
                    for i in 0..entities.len() {
                        for j in (i + 1)..entities.len() {
                            let pair = if entities[i] < entities[j] {
                                (entities[i].clone(), entities[j].clone())
                            } else {
                                (entities[j].clone(), entities[i].clone())
                            };
                            if seen_pairs.contains(&pair) {
                                continue;
                            }
                            seen_pairs.insert(pair.clone());

                            let id = uuid::Uuid::new_v4().to_string();
                            db.execute(
                                "INSERT OR IGNORE INTO cache_relations \
                                 (id, from_id, to_id, relation_type, label, weight, namespace, source_type) \
                                 VALUES (?1, ?2, ?3, 'related_to', 'co-occurrence', 0.3, 'general', 'heimdall_migrated')",
                                rusqlite::params![id, pair.0, pair.1],
                            )
                            .ok();
                            relation_count += 1;
                        }
                    }
                }
            }
        }

        // --- Phase 4: cache_entity_scores ---
        {
            let mut stmt = db
                .prepare("SELECT id FROM cache_entities")
                .map_err(|e| e.to_string())?;
            let entity_ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();

            for entity_id in &entity_ids {
                let ref_count: i64 = db
                    .query_row(
                        "SELECT COUNT(*) FROM cache_relations WHERE from_id = ?1 OR to_id = ?1",
                        rusqlite::params![entity_id],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);

                db.execute(
                    "INSERT OR IGNORE INTO cache_entity_scores (entity_id, view_count, reference_count, updated_at) \
                     VALUES (?1, 0, ?2, '')",
                    rusqlite::params![entity_id, ref_count],
                )
                .ok();
            }
        }

        // Close heimdall connection before rename
        drop(hdb);
        total_entities += entity_count;
        total_relations += relation_count;

        // Rename this source DB as migrated backup
        let date_str = chrono::Local::now().format("%Y%m%d");
        let backup_path = heimdall_db_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(format!("heimdall.db.migrated.{date_str}"));
        if let Err(e) = std::fs::rename(&heimdall_db_path, &backup_path) {
            log::warn!("[Nexus Migration] Failed to rename {:?}: {e}", heimdall_db_path);
        } else {
            log::info!(
                "[Nexus Migration] {:?} renamed to {}",
                heimdall_db_path,
                backup_path.display()
            );
        }
        } // end for each heimdall_db_path

        let elapsed = start.elapsed();

        // Write migration log
        let log_dir = self.hermes_home.join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        let log_path = log_dir.join("nexus_migration.log");
        let log_entry = format!(
            "[{}] Migrated {} entities, {} relations from {} source(s) → knowledge_cache.db in {:.2}s\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            total_entities,
            total_relations,
            paths_to_try.len(),
            elapsed.as_secs_f64(),
        );
        let _ = std::fs::write(&log_path, &log_entry);

        log::info!(
            "[Nexus Migration] Complete: {} entities, {} relations from {} source(s) in {:.2}s",
            total_entities,
            total_relations,
            paths_to_try.len(),
            elapsed.as_secs_f64()
        );

        Ok(())
    }

    #[allow(dead_code)]
    pub fn heimdall_url(&self) -> &str {
        &self.heimdall_url
    }

    pub async fn get_graph_data(
        &self,
        namespace: Option<&str>,
        view_mode: &str,
        focal_node: Option<&str>,
        hops: Option<u32>,
    ) -> Result<GraphData, String> {
        // Local-first: query local SQLite cache, fallback to Heimdall HTTP
        match self.query_cache_graph_data(namespace) {
            Ok(mut data) if !data.entities.is_empty() => {
                // Merge file nodes and wikilink edges
                if let Ok((file_entities, file_relations)) = self.build_file_nodes_and_edges() {
                    let total = (data.entities.len() + file_entities.len()) as u32;
                    data.entities.extend(file_entities);
                    data.relations.extend(file_relations);
                    data.total_entity_count = total;
                }
                Ok(data)
            }
            local_result => {
                // Local empty or error — try Heimdall fallback
                let mut url = format!("{}/api/knowledge/graph-data?mode={}", self.heimdall_url, view_mode);
                if let Some(ns) = namespace {
                    url.push_str(&format!("&ns={ns}"));
                }
                if let Some(f) = focal_node {
                    url.push_str(&format!("&focal={f}"));
                }
                if let Some(h) = hops {
                    url.push_str(&format!("&hops={h}"));
                }

                match self.client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<GraphData>().await {
                            Ok(mut data) => {
                                data.offline = false;
                                let _ = self.upsert_entity_cache(&data.entities);
                                let _ = self.upsert_relation_cache(&data.relations);
                                Ok(data)
                            }
                            Err(e) => {
                                log::warn!("Heimdall graph parse failed: {e}");
                                local_result
                            }
                        }
                    }
                    Ok(resp) => {
                        log::warn!("Heimdall graph HTTP {}", resp.status());
                        local_result
                    }
                    Err(e) => {
                        log::warn!("Heimdall graph unreachable: {e}");
                        local_result
                    }
                }
            }
        }
    }

    fn query_cache_graph_data(&self, _namespace: Option<&str>) -> Result<GraphData, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let mut stmt = db
            .prepare("SELECT id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at, color, hidden FROM cache_entities WHERE hidden = 0 ORDER BY confidence DESC LIMIT 500")
            .map_err(|e| e.to_string())?;

        let entities: Vec<Entity> = stmt
            .query_map([], |row: &rusqlite::Row<'_>| {
                let aliases_str: String = row.get(4).unwrap_or_default();
                let aliases: Vec<String> = serde_json::from_str(&aliases_str).unwrap_or_default();
                let props_str: String = row.get(5).unwrap_or_default();
                let properties: serde_json::Value = serde_json::from_str(&props_str).unwrap_or_default();
                let raw_type: String = row.get(2)?;
                Ok(Entity {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    entity_type: parse_entity_type(&raw_type),
                    description: row.get(3)?,
                    aliases,
                    properties,
                    confidence: row.get(6)?,
                    source_file: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    color: row.get(10)?,
                    hidden: row.get::<_, i32>(11)? != 0,
                    ..Default::default()
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let mut rel_stmt = db
            .prepare("SELECT id, from_id, to_id, relation_type, label, weight, bidirectional FROM cache_relations")
            .map_err(|e| e.to_string())?;

        let relations: Vec<Relation> = rel_stmt
            .query_map([], |row| {
                let raw_rel_type: String = row.get(3)?;
                Ok(Relation {
                    id: row.get(0)?,
                    from_id: row.get(1)?,
                    to_id: row.get(2)?,
                    relation_type: parse_relation_type(&raw_rel_type),
                    label: row.get(4)?,
                    weight: row.get(5)?,
                    bidirectional: row.get::<_, i32>(6)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let entity_ids: Vec<String> = entities.iter().map(|e| e.id.clone()).collect();
        let relations: Vec<Relation> = relations
            .into_iter()
            .filter(|r| entity_ids.contains(&r.from_id) || entity_ids.contains(&r.to_id))
            .collect();

        Ok(GraphData {
            entities,
            relations,
            namespace: None,
            total_entity_count: 0,
            offline: true,
        })
    }

    pub fn get_entity_detail_local(&self, entity_id: &str) -> Result<EntityDetail, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let entity = db
            .query_row(
                "SELECT id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at, color, hidden FROM cache_entities WHERE id = ?1",
                rusqlite::params![entity_id],
                |row| {
                    let aliases_str: String = row.get(4).unwrap_or_default();
                    let aliases: Vec<String> = serde_json::from_str(&aliases_str).unwrap_or_default();
                    let props_str: String = row.get(5).unwrap_or_default();
                    let properties: serde_json::Value = serde_json::from_str(&props_str).unwrap_or_default();
                    Ok(Entity {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        entity_type: parse_entity_type(&row.get::<_, String>(2)?),
                        description: row.get(3)?,
                        aliases,
                        properties,
                        confidence: row.get(6)?,
                        source_file: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        color: row.get(10)?,
                        hidden: row.get::<_, i32>(11)? != 0,
                        ..Default::default()
                    })
                },
            )
            .map_err(|e| format!("实体未找到: {e}"))?;

        let mut rel_stmt = db
            .prepare("SELECT id, from_id, to_id, relation_type, label, weight, bidirectional FROM cache_relations WHERE from_id = ?1 OR to_id = ?1")
            .map_err(|e| e.to_string())?;

        let all_relations: Vec<Relation> = rel_stmt
            .query_map(rusqlite::params![entity_id], |row| {
                Ok(Relation {
                    id: row.get(0)?,
                    from_id: row.get(1)?,
                    to_id: row.get(2)?,
                    relation_type: parse_relation_type(&row.get::<_, String>(3)?),
                    label: row.get(4)?,
                    weight: row.get(5)?,
                    bidirectional: row.get::<_, i32>(6)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let inbound: Vec<Relation> = all_relations.iter()
            .filter(|r| r.to_id == entity_id)
            .cloned()
            .collect();
        let outbound: Vec<Relation> = all_relations.iter()
            .filter(|r| r.from_id == entity_id)
            .cloned()
            .collect();

        Ok(EntityDetail {
            entity,
            inbound_relations: inbound,
            outbound_relations: outbound,
            lint_warnings: Vec::new(),
        })
    }

    pub async fn get_entity_detail(&self, entity_id: &str) -> Result<EntityDetail, String> {
        // Local-first: query local SQLite cache, fallback to Heimdall HTTP
        match self.get_entity_detail_local(entity_id) {
            Ok(detail) if detail.entity.id == entity_id => Ok(detail),
            local_result => {
                let url = format!("{}/api/entities/{entity_id}", self.heimdall_url);
                match self.client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<EntityDetail>().await {
                            Ok(detail) => Ok(detail),
                            Err(e) => {
                                log::warn!("Heimdall entity detail parse failed: {e}");
                                local_result
                            }
                        }
                    }
                    Ok(resp) => {
                        log::warn!("Heimdall entity detail HTTP {}", resp.status());
                        local_result
                    }
                    Err(e) => {
                        log::warn!("Heimdall entity detail unreachable: {e}");
                        local_result
                    }
                }
            }
        }
    }

    /// Scan all entities and relations for data quality issues.
    pub fn get_lint_warnings(&self, namespace: Option<&str>) -> Result<Vec<LintWarning>, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let mut warnings: Vec<LintWarning> = Vec::new();

        // ── orphan: entities with degree 0 ──
        let orphan_sql = if namespace.is_some() {
            "SELECT e.id, e.name FROM cache_entities e
             WHERE e.hidden = 0 AND e.namespace = ?1
             AND e.id NOT IN (SELECT DISTINCT from_id FROM cache_relations)
             AND e.id NOT IN (SELECT DISTINCT to_id FROM cache_relations)"
        } else {
            "SELECT e.id, e.name FROM cache_entities e
             WHERE e.hidden = 0
             AND e.id NOT IN (SELECT DISTINCT from_id FROM cache_relations)
             AND e.id NOT IN (SELECT DISTINCT to_id FROM cache_relations)"
        };
        if let Ok(mut stmt) = db.prepare(orphan_sql) {
            let rows: Vec<_> = if let Some(ns) = namespace {
                stmt.query_map(rusqlite::params![ns], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            } else {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            };
            for (id, name) in rows {
                warnings.push(LintWarning {
                    warning_type: "orphan".into(),
                    entity_id: Some(id),
                    entity_name: name,
                    message: "无任何关联关系".into(),
                    severity: "medium".into(),
                });
            }
        }

        // ── dead_link: relations pointing to missing entities ──
        let dead_sql = if namespace.is_some() {
            "SELECT r.id, r.from_id, r.to_id FROM cache_relations r
             WHERE r.namespace = ?1
             AND (r.from_id NOT IN (SELECT id FROM cache_entities WHERE hidden = 0)
                  OR r.to_id NOT IN (SELECT id FROM cache_entities WHERE hidden = 0))"
        } else {
            "SELECT r.id, r.from_id, r.to_id FROM cache_relations r
             WHERE r.from_id NOT IN (SELECT id FROM cache_entities WHERE hidden = 0)
                OR r.to_id NOT IN (SELECT id FROM cache_entities WHERE hidden = 0)"
        };
        if let Ok(mut stmt) = db.prepare(dead_sql) {
            let rows: Vec<_> = if let Some(ns) = namespace {
                stmt.query_map(rusqlite::params![ns], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            } else {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            };
            for (_id, from_id, to_id) in rows {
                warnings.push(LintWarning {
                    warning_type: "dead_link".into(),
                    entity_id: None,
                    entity_name: format!("{from_id} → {to_id}"),
                    message: "关系指向不存在的实体".into(),
                    severity: "high".into(),
                });
            }
        }

        // ── low_confidence: confidence < 0.3 ──
        let low_sql = if namespace.is_some() {
            "SELECT id, name, confidence FROM cache_entities WHERE hidden = 0 AND namespace = ?1 AND confidence < 0.3"
        } else {
            "SELECT id, name, confidence FROM cache_entities WHERE hidden = 0 AND confidence < 0.3"
        };
        if let Ok(mut stmt) = db.prepare(low_sql) {
            let rows: Vec<_> = if let Some(ns) = namespace {
                stmt.query_map(rusqlite::params![ns], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f32>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            } else {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f32>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            };
            for (id, name, conf) in rows {
                warnings.push(LintWarning {
                    warning_type: "low_confidence".into(),
                    entity_id: Some(id),
                    entity_name: name,
                    message: format!("置信度仅 {:.2}", conf),
                    severity: "low".into(),
                });
            }
        }

        // ── stale: not updated in 30 days ──
        let stale_sql = if namespace.is_some() {
            "SELECT id, name, updated_at FROM cache_entities
             WHERE hidden = 0 AND namespace = ?1 AND updated_at != ''
             AND updated_at < date('now', '-30 days')"
        } else {
            "SELECT id, name, updated_at FROM cache_entities
             WHERE hidden = 0 AND updated_at != ''
             AND updated_at < date('now', '-30 days')"
        };
        if let Ok(mut stmt) = db.prepare(stale_sql) {
            let rows: Vec<_> = if let Some(ns) = namespace {
                stmt.query_map(rusqlite::params![ns], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            } else {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            };
            for (id, name, _updated) in rows {
                warnings.push(LintWarning {
                    warning_type: "stale".into(),
                    entity_id: Some(id),
                    entity_name: name,
                    message: "超过 30 天未更新".into(),
                    severity: "low".into(),
                });
            }
        }

        // ── duplicate: same name (case-insensitive), different id ──
        let dup_sql = if namespace.is_some() {
            "SELECT LOWER(name) AS ln, COUNT(*) AS cnt, GROUP_CONCAT(id) AS ids FROM cache_entities
             WHERE hidden = 0 AND namespace = ?1
             GROUP BY LOWER(name) HAVING cnt > 1"
        } else {
            "SELECT LOWER(name) AS ln, COUNT(*) AS cnt, GROUP_CONCAT(id) AS ids FROM cache_entities
             WHERE hidden = 0
             GROUP BY LOWER(name) HAVING cnt > 1"
        };
        if let Ok(mut stmt) = db.prepare(dup_sql) {
            let rows: Vec<_> = if let Some(ns) = namespace {
                stmt.query_map(rusqlite::params![ns], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?, row.get::<_, String>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            } else {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?, row.get::<_, String>(2)?))
                }).ok().into_iter().flat_map(|r| r.flatten()).collect()
            };
            for (name, cnt, ids) in rows {
                warnings.push(LintWarning {
                    warning_type: "duplicate".into(),
                    entity_id: None,
                    entity_name: name,
                    message: format!("{cnt} 个实体名称相同 (IDs: {ids})"),
                    severity: "medium".into(),
                });
            }
        }

        Ok(warnings)
    }

    pub fn search_entities_local(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<EntitySummary>, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let limit = limit.unwrap_or(20).min(100) as i32;
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }

        // Try FTS5 first with prefix matching
        let fts_query = q.split_whitespace()
            .map(|w| format!("\"{}\"*", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" ");

        let mut results: Vec<EntitySummary> = Vec::new();
        let fts_sql = format!(
            "SELECT e.id, e.name, e.entity_type, e.description, e.confidence
             FROM cache_entities_fts f
             JOIN cache_entities e ON e.rowid = f.rowid
             WHERE cache_entities_fts MATCH ?1 AND e.hidden = 0
             ORDER BY rank
             LIMIT ?2"
        );

        let fts_ok = db.prepare(&fts_sql)
            .and_then(|mut stmt| {
                let rows = stmt.query_map(rusqlite::params![fts_query, limit], |row| {
                    Ok(EntitySummary {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        entity_type: parse_entity_type(&row.get::<_, String>(2)?),
                        description: row.get(3)?,
                        match_score: row.get::<_, f32>(4).unwrap_or(0.5),
                    })
                });
                match rows {
                    Ok(mapped) => {
                        for r in mapped.flatten() {
                            results.push(r);
                        }
                        Ok::<_, rusqlite::Error>(())
                    }
                    Err(e) => Err(e),
                }
            });

        // If FTS returned results, filter by type and return
        if fts_ok.is_ok() && !results.is_empty() {
            if let Some(et) = entity_type {
                let et_str = format!("\"{}\"", et);
                results.retain(|r| {
                    serde_json::to_string(&r.entity_type)
                        .map(|s| s == et_str)
                        .unwrap_or(false)
                });
            }
            results.truncate(limit as usize);
            return Ok(results);
        }

        // Fallback to LIKE search
        let like_pattern = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        let mut sql = String::from(
            "SELECT id, name, entity_type, description, confidence
             FROM cache_entities
             WHERE hidden = 0 AND (name LIKE ?1 OR description LIKE ?1 OR aliases LIKE ?1)"
        );
        if entity_type.is_some() {
            sql.push_str(" AND entity_type = ?3");
        }
        sql.push_str(" ORDER BY confidence DESC LIMIT ?2");

        let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(like_pattern),
            Box::new(limit),
        ];
        if let Some(et) = entity_type {
            params.push(Box::new(et.to_string()));
        }

        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |row| {
                Ok(EntitySummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    entity_type: parse_entity_type(&row.get::<_, String>(2)?),
                    description: row.get(3)?,
                    match_score: row.get::<_, f32>(4).unwrap_or(0.5),
                })
            },
        ).map_err(|e| e.to_string())?;

        for r in rows.flatten() {
            results.push(r);
        }
        results.truncate(limit as usize);
        Ok(results)
    }

    pub async fn search_entities(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<EntitySummary>, String> {
        // Local-first: query local SQLite FTS5, fallback to Heimdall HTTP
        match self.search_entities_local(query, entity_type, limit) {
            Ok(results) if !results.is_empty() => Ok(results),
            local_result => {
                let mut url = format!("{}/api/entities/search?q={query}", self.heimdall_url);
                if let Some(t) = entity_type {
                    url.push_str(&format!("&type={t}"));
                }
                if let Some(l) = limit {
                    url.push_str(&format!("&limit={l}"));
                }

                match self.client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<Vec<EntitySummary>>().await {
                            Ok(results) if !results.is_empty() => Ok(results),
                            Ok(_) => local_result,
                            Err(e) => {
                                log::warn!("Heimdall search parse failed: {e}");
                                local_result
                            }
                        }
                    }
                    Ok(resp) => {
                        log::warn!("Heimdall search HTTP {}", resp.status());
                        local_result
                    }
                    Err(e) => {
                        log::warn!("Heimdall search unreachable: {e}");
                        local_result
                    }
                }
            }
        }
    }

    pub fn find_paths_local(
        &self,
        from_id: &str,
        to_id: &str,
        max_hops: u32,
    ) -> Result<PathResult, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Load all cached entities and relations
        let mut stmt = db
            .prepare("SELECT id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at, color, hidden FROM cache_entities WHERE hidden = 0")
            .map_err(|e| e.to_string())?;
        let entities: Vec<Entity> = stmt
            .query_map([], |row| {
                let aliases_str: String = row.get(4).unwrap_or_default();
                let aliases: Vec<String> = serde_json::from_str(&aliases_str).unwrap_or_default();
                let props_str: String = row.get(5).unwrap_or_default();
                let properties: serde_json::Value = serde_json::from_str(&props_str).unwrap_or_default();
                Ok(Entity {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    entity_type: parse_entity_type(&row.get::<_, String>(2)?),
                    description: row.get(3)?,
                    aliases,
                    properties,
                    confidence: row.get(6)?,
                    source_file: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    color: row.get(10)?,
                    hidden: false,
                    ..Default::default()
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let entity_map: std::collections::HashMap<String, Entity> = entities
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect();

        let mut rel_stmt = db
            .prepare("SELECT id, from_id, to_id, relation_type, label, weight, bidirectional FROM cache_relations")
            .map_err(|e| e.to_string())?;
        let relations: Vec<Relation> = rel_stmt
            .query_map([], |row| {
                Ok(Relation {
                    id: row.get(0)?,
                    from_id: row.get(1)?,
                    to_id: row.get(2)?,
                    relation_type: parse_relation_type(&row.get::<_, String>(3)?),
                    label: row.get(4)?,
                    weight: row.get(5)?,
                    bidirectional: row.get::<_, i32>(6)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Build adjacency list
        let mut adj: std::collections::HashMap<String, Vec<(String, Relation)>> = std::collections::HashMap::new();
        for r in &relations {
            adj.entry(r.from_id.clone()).or_default().push((r.to_id.clone(), r.clone()));
            if r.bidirectional {
                adj.entry(r.to_id.clone()).or_default().push((r.from_id.clone(), r.clone()));
            }
        }

        // BFS to find paths
        let mut paths: Vec<EntityPath> = Vec::new();
        let mut queue: std::collections::VecDeque<(String, Vec<String>, Vec<Relation>, f32)> =
            std::collections::VecDeque::new();
        queue.push_back((from_id.to_string(), vec![from_id.to_string()], vec![], 0.0));

        while let Some((current, visited_ids, rel_path, total_weight)) = queue.pop_front() {
            if visited_ids.len() > max_hops as usize + 1 {
                continue;
            }
            if current == to_id && !rel_path.is_empty() {
                let path_entities: Vec<Entity> = visited_ids
                    .iter()
                    .filter_map(|id| entity_map.get(id).cloned())
                    .collect();
                paths.push(EntityPath {
                    entities: path_entities,
                    relations: rel_path,
                    distance: visited_ids.len() as u32 - 1,
                    total_weight,
                });
                if paths.len() >= 10 {
                    break;
                }
                continue;
            }
            if let Some(neighbors) = adj.get(&current) {
                for (next_id, rel) in neighbors {
                    if !visited_ids.contains(next_id) {
                        let mut new_visited = visited_ids.clone();
                        new_visited.push(next_id.clone());
                        let mut new_rels = rel_path.clone();
                        new_rels.push(rel.clone());
                        queue.push_back((next_id.clone(), new_visited, new_rels, total_weight + rel.weight));
                    }
                }
            }
        }

        let from_entity = entity_map.get(from_id).cloned()
            .unwrap_or_else(|| Entity {
                id: from_id.to_string(),
                name: from_id.to_string(),
                entity_type: "concept".to_string(),
                description: String::new(),
                aliases: vec![],
                properties: serde_json::Value::Null,
                confidence: 0.0,
                source_file: None,
                created_at: String::new(),
                updated_at: String::new(),
                color: None,
                hidden: false,
                ..Default::default()
            });
        let to_entity = entity_map.get(to_id).cloned()
            .unwrap_or_else(|| Entity {
                id: to_id.to_string(),
                name: to_id.to_string(),
                entity_type: "concept".to_string(),
                description: String::new(),
                aliases: vec![],
                properties: serde_json::Value::Null,
                confidence: 0.0,
                source_file: None,
                created_at: String::new(),
                updated_at: String::new(),
                color: None,
                hidden: false,
                ..Default::default()
            });

        Ok(PathResult {
            paths,
            from_entity,
            to_entity,
        })
    }

    pub async fn find_entity_paths(
        &self,
        from_id: &str,
        to_id: &str,
        max_hops: Option<u32>,
    ) -> Result<PathResult, String> {
        let hops = max_hops.unwrap_or(4);

        // Local-first: BFS on local cache, fallback to Heimdall HTTP
        match self.find_paths_local(from_id, to_id, hops) {
            Ok(result) if !result.paths.is_empty() => Ok(result),
            local_result => {
                let url = format!(
                    "{}/api/knowledge/paths?from={from_id}&to={to_id}&max_hops={hops}",
                    self.heimdall_url
                );

                match self.client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<PathResult>().await {
                            Ok(result) if !result.paths.is_empty() => Ok(result),
                            Ok(_) => local_result,
                            Err(e) => {
                                log::warn!("Heimdall paths parse failed: {e}");
                                local_result
                            }
                        }
                    }
                    Ok(resp) => {
                        log::warn!("Heimdall paths HTTP {}", resp.status());
                        local_result
                    }
                    Err(e) => {
                        log::warn!("Heimdall paths unreachable: {e}");
                        local_result
                    }
                }
            }
        }
    }

    pub async fn build_knowledge_context(
        &self,
        text: &str,
    ) -> Result<KnowledgeContext, String> {
        // Scan text for entity mentions and build context
        let entities = self.search_entities(text, None, Some(5)).await?;

        let entity_refs: Vec<String> = entities
            .iter()
            .map(|e| format!("- {} ({})", e.name, e.description))
            .collect();

        let entity_references = if entity_refs.is_empty() {
            String::new()
        } else {
            entity_refs.join("\n")
        };

        // Try to get knowledge snapshot from Heimdall
        let snapshot_url = format!("{}/api/knowledge/overview", self.heimdall_url);
        let knowledge_snapshot = match self.client.get(&snapshot_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                resp.text().await.unwrap_or_default()
            }
            _ => String::new(),
        };

        Ok(KnowledgeContext {
            knowledge_snapshot,
            entity_references,
        })
    }

    /// Build a compact knowledge context snapshot for pre-message injection (P1.3).
    ///
    /// This is the "Push" model: knowledge is injected into the Agent's context
    /// BEFORE the conversation starts, so the Agent doesn't need to call
    /// heimdall_knowledge tool.
    ///
    /// Queries (all from local SQLite cache for robustness):
    ///   1. Recent entities (last `days` days)
    ///   2. FTS5 search by user message
    ///   3. Knowledge snapshot from Heimdall (graceful fallback)
    ///
    /// Budget: ~800 chars to keep overhead minimal.
    /// Build the knowledge map for nexus_map tool.
    /// Returns domain distribution, key entities per domain, subdomains, and cross-domain bridges.
    pub fn build_knowledge_map(&self) -> Result<serde_json::Value, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let total_entities: i64 = db
            .query_row("SELECT COUNT(*) FROM cache_entities WHERE hidden = 0", [], |r| r.get(0))
            .unwrap_or(0);
        let total_relations: i64 = db
            .query_row("SELECT COUNT(*) FROM cache_relations", [], |r| r.get(0))
            .unwrap_or(0);

        // Query 1: Domain stats + subdomains (entity_type distribution per namespace)
        let mut domain_stmt = db
            .prepare(
                "SELECT namespace, COUNT(*) as cnt,
                        GROUP_CONCAT(DISTINCT entity_type) as subdomains
                 FROM cache_entities WHERE hidden = 0
                 GROUP BY namespace ORDER BY cnt DESC",
            )
            .map_err(|e| e.to_string())?;

        let domain_rows: Vec<(String, i64, String)> = domain_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Query 2: Top key entities per domain (by view_count + reference_count, max 5)
        let mut key_stmt = db
            .prepare(
                "SELECT ce.namespace, ce.name
                 FROM cache_entities ce
                 LEFT JOIN cache_entity_scores ces ON ce.id = ces.entity_id
                 WHERE ce.hidden = 0
                 ORDER BY (COALESCE(ces.view_count, 0) + COALESCE(ces.reference_count, 0)) DESC",
            )
            .map_err(|e| e.to_string())?;

        let key_rows: Vec<(String, String)> = key_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Group key entities by namespace (max 5 each)
        let mut key_by_ns: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (ns, name) in &key_rows {
            let entry = key_by_ns.entry(ns.clone()).or_default();
            if entry.len() < 5 {
                entry.push(name.clone());
            }
        }

        // Query 3: Cross-domain bridges
        let mut bridge_stmt = db
            .prepare(
                "SELECT ce1.namespace as domain_a, ce2.namespace as domain_b,
                        COUNT(*) as relation_count,
                        (SELECT ce1_inner.name || ' → ' || ce2_inner.name
                         FROM cache_relations cr_inner
                         JOIN cache_entities ce1_inner ON cr_inner.from_id = ce1_inner.id
                         JOIN cache_entities ce2_inner ON cr_inner.to_id = ce2_inner.id
                         WHERE ce1_inner.namespace = ce1.namespace
                           AND ce2_inner.namespace = ce2.namespace
                           AND ce1_inner.hidden = 0 AND ce2_inner.hidden = 0
                         LIMIT 1) as example_pair
                 FROM cache_relations cr
                 JOIN cache_entities ce1 ON cr.from_id = ce1.id
                 JOIN cache_entities ce2 ON cr.to_id = ce2.id
                 WHERE ce1.namespace != ce2.namespace
                   AND ce1.hidden = 0 AND ce2.hidden = 0
                 GROUP BY domain_a, domain_b
                 ORDER BY relation_count DESC",
            )
            .map_err(|e| e.to_string())?;

        let bridge_rows: Vec<(String, String, i64, Option<String>)> = bridge_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Build domains JSON
        let mut domain_set: std::collections::HashSet<String> = std::collections::HashSet::new();
        let domains: Vec<serde_json::Value> = domain_rows
            .iter()
            .map(|(ns, cnt, subdomains)| {
                domain_set.insert(ns.clone());
                let sub_list: Vec<&str> = subdomains
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                let key_entities = key_by_ns.get(ns).cloned().unwrap_or_default();
                let empty_vec: Vec<String> = Vec::new();
                serde_json::json!({
                    "name": ns,
                    "entity_count": cnt,
                    "key_entities": key_entities,
                    "subdomains": sub_list,
                    "connected_to": empty_vec,
                })
            })
            .collect();

        // Fill connected_to per domain
        let mut domains: Vec<serde_json::Value> = domains;
        for d in domains.iter_mut() {
            let ns = d["name"].as_str().unwrap_or("");
            let mut connected: Vec<String> = Vec::new();
            for (a, b, _, _) in &bridge_rows {
                if a == ns && !connected.contains(b) {
                    connected.push(b.clone());
                }
                if b == ns && !connected.contains(a) {
                    connected.push(a.clone());
                }
            }
            d["connected_to"] = serde_json::json!(connected);
        }

        // Build bridges JSON
        let bridges: Vec<serde_json::Value> = bridge_rows
            .iter()
            .map(|(a, b, cnt, example)| {
                let strength = if *cnt >= 20 {
                    "强"
                } else if *cnt >= 5 {
                    "中"
                } else {
                    "弱"
                };
                serde_json::json!({
                    "domain_a": a,
                    "domain_b": b,
                    "strength": strength,
                    "relation_count": cnt,
                    "example": example.clone().unwrap_or_default(),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "knowledge_map": {
                "total_entities": total_entities,
                "total_relations": total_relations,
                "domains": domains,
                "bridges": bridges,
            }
        }))
    }

    pub async fn reference_entity_to_chat(
        &self,
        entity_id: &str,
    ) -> Result<EntityReference, String> {
        let detail = self.get_entity_detail(entity_id).await?;
        let name = detail.entity.name.clone();
        Ok(EntityReference {
            entity_name: name.clone(),
            entity_type: detail.entity.entity_type,
            summary: detail.entity.description,
            markdown_ref: format!("@[{name}]"),
        })
    }

    /// Extract entities directly from text (no file required).
    /// Nexus extract: calls nexus_store for LLM-based extraction.
    /// Falls back to local regex extraction if Nexus LLM is unavailable.
    pub async fn extract_entities_from_text(
        &self,
        text: &str,
        namespace: &str,
        source: Option<&str>,
    ) -> Result<ExtractionCompleteEvent, String> {
        // Redirect to Nexus LLM extraction
        match self.nexus_store(text, source.unwrap_or("chat"), None, None) {
            Ok(result) => {
                log::info!(
                    "Nexus text extraction complete: {} entities, {} relations (source: {:?})",
                    result.entity_count, result.relation_count, source
                );
                Ok(ExtractionCompleteEvent {
                    new_count: result.entity_count,
                    updated_count: 0,
                    source_file: source.unwrap_or("chat").to_string(),
                    snapshot_updated: false,
                })
            }
            Err(e) => {
                log::warn!("Nexus text extract failed: {e}, falling back to local regex");
                self.extract_entities_from_text_local(text, namespace, source)
            }
        }
    }

    /// Local fallback: regex-based entity extraction from text.
    /// Operates on text strings instead of file paths.
    /// Reuses the same regex patterns as extract_entities_local.
    fn extract_entities_from_text_local(
        &self,
        text: &str,
        _namespace: &str,
        source: Option<&str>,
    ) -> Result<ExtractionCompleteEvent, String> {
        if text.trim().is_empty() {
            return Ok(ExtractionCompleteEvent {
                new_count: 0,
                updated_count: 0,
                source_file: source.unwrap_or("chat").to_string(),
                snapshot_updated: false,
            });
        }

        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let source_label = source.unwrap_or("chat");
        let mut new_count: u32 = 0;
        let mut total_entities: u32 = 0;
        let mut extracted_entity_ids: Vec<String> = Vec::new();

        // Create a virtual source document entity for relation anchoring
        let doc_id = format!("source:{}", source_label);
        {
            let _ = db.execute(
                "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                 VALUES (?1, ?2, 'content', '', '[]', '{}', 0.5, ?3, ?4, ?4)",
                rusqlite::params![doc_id, source_label, source_label, now],
            );
            extracted_entity_ids.push(doc_id.clone());
        }

        // Pattern 1: [[wikilinks]]
        let wikilink_re = regex::Regex::new(r"\[\[([^\]]+)\]\]").ok();
        if let Some(ref re) = wikilink_re {
            for cap in re.captures_iter(text) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 2 || name.len() > 80 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, 'concept', '', '[]', '{}', 0.55, ?3, ?4, ?4)",
                    rusqlite::params![id, name, source_label, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
                let rel_id = uuid::Uuid::new_v4().to_string();
                let _ = db.execute(
                    "INSERT OR REPLACE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                     VALUES (?1, ?2, ?3, 'uses', 'mentioned', 0.4, 0)",
                    rusqlite::params![rel_id, doc_id, id],
                );
            }
        }

        // Pattern 2: 《书名》
        let book_re = regex::Regex::new(r"《([^》]{1,40})》").ok();
        if let Some(ref re) = book_re {
            for cap in re.captures_iter(text) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 1 || name.len() > 40 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, 'content', ?3, '[]', '{}', 0.5, ?4, ?5, ?5)",
                    rusqlite::params![id, name, format!("从对话提取: 《{name}》"), source_label, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    extracted_entity_ids.push(id.clone());
                }
            }
        }

        // Pattern 3: person — Chinese role titles
        // Matches role suffixes and extracts the preceding 2-4 Chinese characters as person name.
        let person_re = regex::Regex::new(
            r"(\p{Han}{2,4})(先生|女士|老师|教授|医生|律师|创始人|经理|主任|导演|作者|开发者|维护者|架构师|产品经理|工程师|设计师|负责人)"
        ).ok();
        if let Some(ref re) = person_re {
            for cap in re.captures_iter(text) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 2 || name.len() > 4 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, 'person', '', '[]', '{}', 0.45, ?3, ?4, ?4)",
                    rusqlite::params![id, name, source_label, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
            }
        }
        // English name prefix pattern: Dr./Mr./Mrs./Ms./Prof. Name
        let en_person_re = regex::Regex::new(r"(?i)(Dr\.|Mr\.|Mrs\.|Ms\.|Prof\.)\s*([A-Z][a-z]+)").ok();
        if let Some(ref re) = en_person_re {
            for cap in re.captures_iter(text) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 2 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, 'person', '', '[]', '{}', 0.5, ?3, ?4, ?4)",
                    rusqlite::params![id, name, source_label, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
            }
        }

        // Date-event patterns removed in Nexus P1 — unreliable prefix-based extraction
        // Doc-anchor relations removed — noise in graph view

        log::info!(
            "Local text extraction from {:?}: {} new entities",
            source, new_count
        );

        Ok(ExtractionCompleteEvent {
            new_count,
            updated_count: 0,
            source_file: source_label.to_string(),
            snapshot_updated: new_count > 0,
        })
    }

    fn extract_entities_local(
        &self,
        file_path: &str,
        _namespace: &str,
    ) -> Result<ExtractionCompleteEvent, String> {
        // Read wiki file content
        let target = self.wiki_dir.join(file_path);
        let content = if target.exists() {
            std::fs::read_to_string(&target).unwrap_or_default()
        } else {
            return Ok(ExtractionCompleteEvent {
                new_count: 0,
                updated_count: 0,
                source_file: file_path.to_string(),
                snapshot_updated: false,
            });
        };

        if content.trim().is_empty() {
            return Ok(ExtractionCompleteEvent {
                new_count: 0,
                updated_count: 0,
                source_file: file_path.to_string(),
                snapshot_updated: false,
            });
        }

        let mut new_count: u32 = 0;
        let updated_count: u32 = 0;
        let mut total_entities: u32 = 0;
        let mut extracted_entity_ids: Vec<String> = Vec::new();

        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let doc_name = std::path::Path::new(file_path)
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or(file_path);

        // Ensure document entity exists (shared anchor for all patterns)
        let doc_id = format!("doc:{}", file_path);
        {
            let now = chrono::Utc::now().to_rfc3339();
            let _ = db.execute(
                "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                 VALUES (?1, ?2, ?3, '', '[]', '{}', 0.5, ?4, ?5, ?5)",
                rusqlite::params![doc_id, doc_name, "content", file_path, now],
            );
            extracted_entity_ids.push(doc_id.clone());
        }

        // Pattern 1: [[wikilinks]] — strongest signal
        let wikilink_re = regex::Regex::new(r"\[\[([^\]]+)\]\]").ok();
        if let Some(ref re) = wikilink_re {
            for cap in re.captures_iter(&content) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 2 || name.len() > 80 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                let now = chrono::Utc::now().to_rfc3339();
                let entity_type = "concept".to_string();
                if db.execute(
                    "INSERT INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, ?3, '', '[]', '{}', 0.7, ?4, ?5, ?5)
                     ON CONFLICT(id) DO NOTHING",
                    rusqlite::params![id, name, entity_type, file_path, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
                // Create relation to document entity
                let rel_id = uuid::Uuid::new_v4().to_string();
                let _ = db.execute(
                    "INSERT OR REPLACE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                     VALUES (?1, ?2, ?3, 'uses', 'related_to', 0.6, 0)",
                    rusqlite::params![rel_id, doc_id, id],
                );
            }
        }

        // Pattern 2: 《书名》 — Chinese book titles
        let book_re = regex::Regex::new(r"《([^》]{1,40})》").ok();
        if let Some(ref re) = book_re {
            for cap in re.captures_iter(&content) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 1 || name.len() > 40 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                let now = chrono::Utc::now().to_rfc3339();
                let entity_type = "document".to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '[]', '{}', 0.6, ?5, ?6, ?6)",
                    rusqlite::params![id, name, entity_type, format!("从文档提取: 《{name}》"), file_path, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
            }
        }

        // Pattern 3: 【术语】— Chinese brackets for terminology
        let term_re = regex::Regex::new(r"【([^】]{1,40})】").ok();
        if let Some(ref re) = term_re {
            for cap in re.captures_iter(&content) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 1 || name.len() > 40 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                let now = chrono::Utc::now().to_rfc3339();
                let entity_type = "concept".to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '[]', '{}', 0.65, ?5, ?6, ?6)",
                    rusqlite::params![id, name, entity_type, format!("从文档提取术语: 【{name}】"), file_path, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
            }
        }

        // Pattern 5: person — Chinese role titles
        let person_re = regex::Regex::new(
            r"(\p{Han}{2,4})(先生|女士|老师|教授|医生|律师|创始人|经理|主任|导演|作者|开发者|维护者|架构师|产品经理|工程师|设计师|负责人)"
        ).ok();
        if let Some(ref re) = person_re {
            for cap in re.captures_iter(&content) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 2 || name.len() > 4 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                let now = chrono::Utc::now().to_rfc3339();
                let entity_type = "person".to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, ?3, '', '[]', '{}', 0.45, ?4, ?5, ?5)",
                    rusqlite::params![id, name, entity_type, file_path, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
            }
        }
        let en_person_re = regex::Regex::new(r"(?i)(Dr\.|Mr\.|Mrs\.|Ms\.|Prof\.)\s*([A-Z][a-z]+)").ok();
        if let Some(ref re) = en_person_re {
            for cap in re.captures_iter(&content) {
                if total_entities >= MAX_ENTITIES_PER_DOC { break; }
                let name = cap.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                if name.len() < 2 { continue; }
                let name_lower = name.to_lowercase();
                if STOP_WORDS.iter().any(|w| *w == name || w.to_lowercase() == name_lower) { continue; }
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.trim().to_lowercase().as_bytes()).to_string();
                let now = chrono::Utc::now().to_rfc3339();
                let entity_type = "person".to_string();
                if db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, ?3, '', '[]', '{}', 0.5, ?4, ?5, ?5)",
                    rusqlite::params![id, name, entity_type, file_path, now],
                ).unwrap_or(0) > 0 {
                    new_count += 1;
                    total_entities += 1;
                    extracted_entity_ids.push(id.clone());
                }
            }
        }

        // Date-event patterns removed in Nexus P1 — unreliable prefix-based extraction
        // Doc-anchor relations removed — noise in graph view

        // Update last extraction timestamp
        let now = chrono::Utc::now().to_rfc3339();
        let _ = db.execute(
            "INSERT OR REPLACE INTO cache_meta (key, value) VALUES ('last_extraction', ?1)",
            rusqlite::params![now],
        );

        log::info!(
            "Local entity extraction from {file_path}: {new_count} new, {updated_count} updated"
        );

        Ok(ExtractionCompleteEvent {
            new_count,
            updated_count,
            source_file: file_path.to_string(),
            snapshot_updated: new_count > 0 || updated_count > 0,
        })
    }

    /// Unified save-to-knowledge for right-click menu actions.
    ///
    /// Save chat conversation to a .md file in wiki/chat/.
    /// Pure file generation — no entity extraction (FileWatcher handles that).
    /// No heimdall_id or auto-generated markers (Nexus pipeline).
    pub async fn save_chat_to_knowledge(
        &self,
        text: &str,
        source_label: &str,
        _action_type: &str,
    ) -> Result<ChatKnowledgeSaveResult, String> {
        let now = chrono::Utc::now();
        let date_str = now.format("%Y%m%d").to_string();
        let now_str = now.to_rfc3339();

        // File name: {title}_{date}.md — sanitize for filesystem
        let safe_title: String = source_label
            .chars()
            .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
            .take(60)
            .collect();
        let file_name = if safe_title.is_empty() {
            format!("chat_{date_str}.md")
        } else {
            format!("{safe_title}_{date_str}.md")
        };

        // Store in wiki/chat/ subdirectory
        let chat_dir = self.wiki_dir.join("chat");
        std::fs::create_dir_all(&chat_dir)
            .map_err(|e| format!("创建 chat 目录失败: {e}"))?;

        let markdown = format!(
            "---\ntitle: {source_label}\nsource: chat\ncreated: {now_str}\ntype: chat-knowledge\n---\n\n# {source_label}\n\n> **来源**: 对话记录\n> **时间**: {now_str}\n\n{text}\n"
        );

        let target = chat_dir.join(&file_name);
        std::fs::write(&target, &markdown)
            .map_err(|e| format!("写入文件失败: {e}"))?;

        log::info!(
            "save_chat_to_knowledge: {} (chat sync to wiki/chat/)",
            file_name
        );

        // Synchronously extract entities after saving
        let target_str = target.to_string_lossy().to_string();
        match self.extract_entities(&target_str, "chat").await {
            Ok(result) => {
                log::info!(
                    "save_chat_to_knowledge extraction: {} new entities",
                    result.new_count
                );
                Ok(ChatKnowledgeSaveResult {
                    file_name: file_name.clone(),
                    new_count: result.new_count,
                    entities: Vec::new(),
                })
            }
            Err(e) => {
                log::warn!("save_chat_to_knowledge extraction failed (non-fatal): {e}");
                Ok(ChatKnowledgeSaveResult {
                    file_name,
                    new_count: 0,
                    entities: Vec::new(),
                })
            }
        }
    }

    // ── Nexus Knowledge Engine ──

    /// Detect a usable Python interpreter.
    fn find_python() -> PathBuf {
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
            if std::process::Command::new(&path).arg("--version").output().is_ok() {
                return path;
            }
        }
        if cfg!(windows) { PathBuf::from("py") } else { PathBuf::from("python3") }
    }

    /// Compute SHA256 hash of content for dedup.
    fn content_hash(content: &str) -> String {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Determine source_type from a file path.
    fn infer_source_type(path: &str) -> &str {
        if path.contains("canvas") || path.ends_with(".canvas") {
            "canvas"
        } else if path.ends_with(".md") {
            "wiki"
        } else if path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg") {
            "upload_image"
        } else if path.ends_with(".pdf") || path.ends_with(".docx") || path.ends_with(".txt") {
            "upload_doc"
        } else {
            "wiki"
        }
    }

    /// Path to extract_service.py (created in P3).
    fn extract_service_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("services")
            .join("extract_service.py")
    }

    fn has_api_key(vars: &[(String, String)]) -> bool {
        vars.iter().any(|(k, v)| k == "NEXUS_LLM_API_KEY" && !v.is_empty())
    }

    /// Spawn extract_service.py subprocess and return parsed JSON result.
    /// Build Nexus LLM env vars from config.yaml (mirrors ConfigService::nexus_env_vars).
    fn nexus_env_vars(&self) -> Vec<(String, String)> {
        let config_path = self.hermes_home.join("config.yaml");
        let nexus = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
            .and_then(|v| {
                v.get("nexus")
                    .map(|n| serde_json::to_value(n).unwrap_or_default())
            })
            .unwrap_or_else(|| serde_json::json!({"llm_mode": "custom"}));

        let llm_mode = nexus.get("llm_mode").and_then(|v| v.as_str()).unwrap_or("follow_agent");

        if llm_mode == "custom" {
            let provider = nexus.get("llm_provider").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("anthropic");
            let model = nexus.get("llm_model").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("deepseek-v4-flash");
            let api_key = nexus.get("llm_api_key").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("");
            let base_url = nexus.get("llm_base_url").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("");
            vec![
                ("NEXUS_LLM_MODE".into(), "custom".into()),
                ("NEXUS_LLM_PROVIDER".into(), provider.into()),
                ("NEXUS_LLM_MODEL".into(), model.into()),
                ("NEXUS_LLM_API_KEY".into(), api_key.into()),
                ("NEXUS_LLM_BASE_URL".into(), base_url.into()),
            ]
        } else {
            // follow_agent: read agent model config from same config.yaml
            let config: serde_json::Value = std::fs::read_to_string(&config_path)
                .ok()
                .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
                .and_then(|v| serde_json::to_value(v).ok())
                .unwrap_or_default();

            let model_name = config
                .get("model").and_then(|m| m.get("default").or_else(|| m.get("name"))).and_then(|n| n.as_str())
                .unwrap_or("deepseek-v4-flash");
            let provider = config
                .get("model").and_then(|m| m.get("provider")).and_then(|p| p.as_str())
                .unwrap_or("deepseek");

            // When using Hermes builtin agent, route Nexus LLM calls through Hermes
            if provider == "hermes-builtin" {
                return vec![
                    ("NEXUS_LLM_MODE".into(), "follow_agent".into()),
                    ("NEXUS_LLM_PROVIDER".into(), "hermes_builtin".into()),
                    ("NEXUS_LLM_MODEL".into(), model_name.to_string()),
                    ("NEXUS_LLM_BASE_URL".into(), "http://127.0.0.1:18642/v1".into()),
                ];
            }

            // Collect API keys from model config
            let mut vars = vec![
                ("NEXUS_LLM_MODE".into(), "follow_agent".into()),
                ("NEXUS_LLM_PROVIDER".into(), provider.to_string()),
                ("NEXUS_LLM_MODEL".into(), model_name.to_string()),
            ];

            if let Some(api_key) = config
                .get("model").and_then(|m| m.get("api_key")).and_then(|k| k.as_str())
            {
                vars.push(("NEXUS_LLM_API_KEY".into(), api_key.to_string()));
            }

            // Read base_url from model config
            if let Some(base_url) = config
                .get("model").and_then(|m| m.get("base_url")).and_then(|u| u.as_str())
            {
                vars.push(("NEXUS_LLM_BASE_URL".into(), base_url.to_string()));
            }

            // Also check env_vars in model config
            if let Some(env_vars) = config
                .get("model").and_then(|m| m.get("env_vars")).and_then(|e| e.as_object())
            {
                for (k, v) in env_vars {
                    if let Some(val) = v.as_str() {
                        match k.as_str() {
                            "ANTHROPIC_API_KEY" | "DEEPSEEK_API_KEY" | "OPENAI_API_KEY" => {
                                vars.push(("NEXUS_LLM_API_KEY".into(), val.to_string()));
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Fallback: check legacy ~/.hermes/config.yaml for provider API key
            if !Self::has_api_key(&vars) {
                let legacy_home = std::env::var("HERMES_HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| {
                        #[cfg(target_os = "windows")]
                        {
                            std::env::var("USERPROFILE")
                                .map(std::path::PathBuf::from)
                                .unwrap_or_else(|_| std::path::PathBuf::from("C:"))
                                .join(".hermes")
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            std::env::var("HOME")
                                .map(std::path::PathBuf::from)
                                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
                                .join(".hermes")
                        }
                    });
                let legacy_config_path = legacy_home.join("config.yaml");
                if let Ok(legacy_raw) = std::fs::read_to_string(&legacy_config_path) {
                    if let Ok(legacy_config) = serde_yaml::from_str::<serde_yaml::Value>(&legacy_raw) {
                        let key_paths: &[&[&str]] = match provider {
                            "deepseek" => &[&["providers", "deepseek", "api_key"]],
                            "anthropic" => &[&["providers", "anthropic", "api_key"]],
                            "openai" => &[&["providers", "openai", "api_key"]],
                            _ => &[&["providers", provider, "api_key"]],
                        };
                        for path_segments in key_paths {
                            let mut val: Option<&serde_yaml::Value> = Some(&legacy_config);
                            for seg in *path_segments {
                                val = val.and_then(|v| v.get(*seg));
                            }
                            if let Some(key) = val.and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                vars.push(("NEXUS_LLM_API_KEY".into(), key.to_string()));
                                break;
                            }
                        }
                    }
                }
            }

            vars
        }
    }

    fn run_extract_service(
        &self,
        python: &str,
        mode: &str,
        text: &str,
        context: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let script = Self::extract_service_script();
        if !script.exists() {
            return Err("extract_service.py not found — will be available in P3".into());
        }

        let mut cmd = std::process::Command::new(python);
        cmd.env("PYTHONIOENCODING", "utf-8");
        // Pass Nexus LLM env vars from config.yaml to the subprocess
        for (k, v) in self.nexus_env_vars() {
            cmd.env(&k, &v);
        }
        cmd.arg(script.to_str().unwrap())
            .arg("--mode").arg(mode)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(ctx) = context {
            cmd.arg("--context").arg(ctx);
        }

        #[cfg(windows)] { cmd.creation_flags(CREATE_NO_WINDOW); }
        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to start extract_service.py: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(text.as_bytes()).ok();
        }

        let output = child.wait_with_output()
            .map_err(|e| format!("extract_service.py error: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("extract_service.py failed: {}", stderr.lines().last().unwrap_or("")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse extract_service.py JSON: {e} — raw: {}", &stdout[..stdout.len().min(200)]))
    }

    /// Store raw content for Nexus extraction.
    /// Writes to cache_content_index and spawns extract_service.py for LLM extraction.
    /// source_type="canvas" maps directly to entities/relations without LLM.
    pub fn nexus_store(
        &self,
        text: &str,
        source_type: &str,
        source_path: Option<&str>,
        context: Option<&str>,
    ) -> Result<NexusStoreResult, String> {
        let hash = Self::content_hash(text);
        let path = source_path.unwrap_or(source_type);

        {
            let db = self.cache_db.lock().map_err(|e| e.to_string())?;

            // Check if already indexed with same hash
            let existing_hash: Option<String> = db
                .query_row(
                    "SELECT content_hash FROM cache_content_index WHERE source_path = ?1",
                    rusqlite::params![path],
                    |row| row.get(0),
                )
                .ok();

            if existing_hash.as_deref() == Some(&hash) {
                return Ok(NexusStoreResult {
                    entity_count: 0,
                    relation_count: 0,
                    skipped: true,
                });
            }

            // Insert/update content index
            db.execute(
                "INSERT OR REPLACE INTO cache_content_index (source_path, source_type, content_hash, extracted_at, entity_count)
                 VALUES (?1, ?2, ?3, NULL, 0)",
                rusqlite::params![path, source_type, hash],
            ).map_err(|e| format!("nexus_store: insert content_index failed: {e}"))?;
        } // release db lock before potentially long-running extraction

        // Canvas special path: direct node/edge → entity/relation mapping, no LLM
        if source_type == "canvas" {
            return self.nexus_store_canvas(text, path);
        }

        // Spawn extract_service.py for LLM extraction
        let python = Self::find_python();
        let mode = match source_type {
            "upload_image" => "image",
            "upload_doc" => "document",
            _ => "text",
        };

        let result = match self.run_extract_service(
            python.to_str().unwrap_or("python"),
            mode,
            text,
            context,
        ) {
            Ok(json) => {
                let (ec, rc) = self.write_extraction_result(&json, path, source_type)?;
                NexusStoreResult { entity_count: ec, relation_count: rc, skipped: false }
            }
            Err(e) => {
                log::warn!("[Nexus] extract_service.py failed for {path}: {e}");
                NexusStoreResult { entity_count: 0, relation_count: 0, skipped: false }
            }
        };

        // Update content_index with extraction timestamp
        if let Ok(db) = self.cache_db.lock() {
            let now = chrono::Utc::now().to_rfc3339();
            db.execute(
                "UPDATE cache_content_index SET extracted_at = ?1, entity_count = ?2 WHERE source_path = ?3",
                rusqlite::params![now, result.entity_count, path],
            ).ok();
        }

        log::info!("[Nexus] Stored: path={path}, type={source_type}, {ec} entities, {rc} relations",
            ec = result.entity_count, rc = result.relation_count);

        Ok(result)
    }

    /// Canvas direct mapping: parse JSON nodes/edges → entities/relations.
    fn nexus_store_canvas(&self, json_text: &str, path: &str) -> Result<NexusStoreResult, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();

        let data: serde_json::Value = serde_json::from_str(json_text)
            .map_err(|e| format!("Canvas JSON parse error: {e}"))?;

        let mut entity_count = 0u32;
        let mut relation_count = 0u32;

        // Map nodes → entities
        if let Some(nodes) = data.get("nodes").and_then(|n| n.as_array()) {
            for node in nodes {
                let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = node.get("text").or(node.get("label")).and_then(|v| v.as_str()).unwrap_or(id);
                let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("concept");

                if id.is_empty() || name.is_empty() { continue; }

                let entity_id = format!("canvas:{}", id);
                db.execute(
                    "INSERT OR REPLACE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, llm_confidence, source_type, source_file, namespace, created_at, updated_at)
                     VALUES (?1, ?2, ?3, '', '[]', ?4, 0.7, 0.7, 'canvas', ?5, '画板', ?6, ?6)",
                    rusqlite::params![entity_id, name, node_type, json_text, path, now],
                ).unwrap_or(0);
                entity_count += 1;
            }
        }

        // Map edges → relations
        if let Some(edges) = data.get("edges").and_then(|e| e.as_array()) {
            for edge in edges {
                let from = edge.get("from").or(edge.get("fromNode")).and_then(|v| v.as_str()).unwrap_or("");
                let to = edge.get("to").or(edge.get("toNode")).and_then(|v| v.as_str()).unwrap_or("");
                let label = edge.get("label").and_then(|v| v.as_str()).unwrap_or("related_to");

                if from.is_empty() || to.is_empty() { continue; }

                let from_id = format!("canvas:{}", from);
                let to_id = format!("canvas:{}", to);
                let rel_id = uuid::Uuid::new_v4().to_string();

                db.execute(
                    "INSERT OR REPLACE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                     VALUES (?1, ?2, ?3, 'related_to', ?4, 0.5, 0)",
                    rusqlite::params![rel_id, from_id, to_id, label],
                ).unwrap_or(0);
                relation_count += 1;
            }
        }

        // Update content_index
        db.execute(
            "UPDATE cache_content_index SET extracted_at = ?1, entity_count = ?2 WHERE source_path = ?3",
            rusqlite::params![now, entity_count, path],
        ).ok();

        log::info!("[Nexus] Canvas direct mapping: {entity_count} entities, {relation_count} relations");

        Ok(NexusStoreResult {
            entity_count,
            relation_count,
            skipped: false,
        })
    }

    /// Extract knowledge from a file: read content, spawn extract_service.py, write to KG.
    pub fn nexus_extract_from_file(
        &self,
        file_path: &str,
        source_type: Option<&str>,
    ) -> Result<NexusStoreResult, String> {
        let full_path = self.wiki_dir.join(file_path);
        if !full_path.exists() {
            return Err(format!("File not found: {}", full_path.display()));
        }

        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| format!("Read file failed: {e}"))?;

        if content.trim().is_empty() {
            return Ok(NexusStoreResult { entity_count: 0, relation_count: 0, skipped: true });
        }

        let hash = Self::content_hash(&content);
        let st = source_type.unwrap_or_else(|| Self::infer_source_type(file_path));
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Dedup by hash
        let existing_hash: Option<String> = db
            .query_row(
                "SELECT content_hash FROM cache_content_index WHERE source_path = ?1",
                rusqlite::params![file_path],
                |row| row.get(0),
            )
            .ok();

        if existing_hash.as_deref() == Some(&hash) {
            return Ok(NexusStoreResult { entity_count: 0, relation_count: 0, skipped: true });
        }

        // Register in content_index
        db.execute(
            "INSERT OR REPLACE INTO cache_content_index (source_path, source_type, content_hash, extracted_at, entity_count)
             VALUES (?1, ?2, ?3, NULL, 0)",
            rusqlite::params![file_path, st, hash],
        ).map_err(|e| format!("Insert content_index failed: {e}"))?;
        drop(db);

        // Spawn extract_service.py
        let python = Self::find_python();
        let mode = match st {
            "upload_image" => "image",
            "upload_doc" => "document",
            _ => "text",
        };

        let result = match self.run_extract_service(
            python.to_str().unwrap_or("python"),
            mode,
            &content,
            None,
        ) {
            Ok(json) => {
                let (ec, rc) = self.write_extraction_result(&json, file_path, st)?;
                NexusStoreResult { entity_count: ec, relation_count: rc, skipped: false }
            }
            Err(e) => {
                log::warn!("[Nexus] extract_service.py failed: {e}, falling back to local regex");
                let extract_result = self.extract_entities_local(file_path, "default")?;
                NexusStoreResult {
                    entity_count: extract_result.new_count,
                    relation_count: 0,
                    skipped: false,
                }
            }
        };

        // Update content_index
        let db2 = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now2 = chrono::Utc::now().to_rfc3339();
        db2.execute(
            "UPDATE cache_content_index SET extracted_at = ?1, entity_count = ?2 WHERE source_path = ?3",
            rusqlite::params![now2, result.entity_count, file_path],
        ).ok();

        Ok(result)
    }

    /// Write extraction JSON (from extract_service.py) into cache_entities + cache_relations.
    fn write_extraction_result(
        &self,
        json: &serde_json::Value,
        source_path: &str,
        source_type: &str,
    ) -> Result<(u32, u32), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut entity_count = 0u32;
        let mut relation_count = 0u32;

        // Name → ID map for relation resolution
        let mut name_to_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        if let Some(entities) = json.get("entities").and_then(|e| e.as_array()) {
            for ent in entities {
                let name = ent.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if name.is_empty() { continue; }

                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, name.to_lowercase().as_bytes()).to_string();
                let entity_type = ent.get("type").and_then(|v| v.as_str()).unwrap_or("concept");
                let description = ent.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let namespace = ent.get("namespace").and_then(|v| v.as_str()).unwrap_or("未分类");
                let llm_confidence = ent.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);
                let properties = ent.get("properties").map(|p| serde_json::to_string(p).unwrap_or_default()).unwrap_or_else(|| "{}".to_string());

                db.execute(
                    "INSERT INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, llm_confidence, source_count, source_type, source_file, namespace, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6, ?6, 1, ?7, ?8, ?9, ?10, ?10)
                     ON CONFLICT(id) DO UPDATE SET
                        llm_confidence = MAX(llm_confidence, excluded.llm_confidence),
                        source_count = source_count + 1,
                        updated_at = excluded.updated_at,
                        description = CASE WHEN excluded.description != '' THEN excluded.description ELSE description END,
                        properties = CASE WHEN excluded.properties != '{}' THEN excluded.properties ELSE properties END",
                    rusqlite::params![id, name, entity_type, description, properties, llm_confidence, source_type, source_path, namespace, now],
                ).ok();
                name_to_id.insert(name.to_lowercase(), id);
                entity_count += 1;
            }
        }

        if let Some(relations) = json.get("relations").and_then(|r| r.as_array()) {
            for rel in relations {
                let from_name = rel.get("from").and_then(|v| v.as_str()).unwrap_or("");
                let to_name = rel.get("to").and_then(|v| v.as_str()).unwrap_or("");
                let rel_type = rel.get("type").and_then(|v| v.as_str()).unwrap_or("related_to");
                let confidence = rel.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);

                let from_id = name_to_id.get(&from_name.to_lowercase()).cloned()
                    .unwrap_or_else(|| uuid::Uuid::new_v5(&WIKI_NAMESPACE, from_name.to_lowercase().as_bytes()).to_string());
                let to_id = name_to_id.get(&to_name.to_lowercase()).cloned()
                    .unwrap_or_else(|| uuid::Uuid::new_v5(&WIKI_NAMESPACE, to_name.to_lowercase().as_bytes()).to_string());

                let rel_id = uuid::Uuid::new_v4().to_string();
                db.execute(
                    "INSERT OR REPLACE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                    rusqlite::params![rel_id, from_id, to_id, rel_type, rel_type, confidence],
                ).ok();
                relation_count += 1;
            }
        }

        Ok((entity_count, relation_count))
    }

    /// Full reindex: iterate all wiki files and session extracts, re-extract each.
    pub fn nexus_reindex_all(&self) -> Result<NexusReindexResult, String> {
        let mut files_processed = 0u32;
        let mut entities_total = 0u32;
        let mut relations_total = 0u32;
        let mut skipped = 0u32;
        let mut errors: Vec<String> = Vec::new();

        // Collect wiki files
        let mut wiki_files: std::collections::HashSet<String> = std::collections::HashSet::new();
        self.collect_md_files(&self.wiki_dir, &mut wiki_files);

        for file_path in &wiki_files {
            match self.nexus_extract_from_file(file_path, None) {
                Ok(r) => {
                    files_processed += 1;
                    entities_total += r.entity_count;
                    relations_total += r.relation_count;
                    if r.skipped { skipped += 1; }
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", file_path));
                }
            }
        }

        // Clear pending content_index entries
        {
            let db = self.cache_db.lock().map_err(|e| e.to_string())?;
            let now = chrono::Utc::now().to_rfc3339();
            db.execute(
                "UPDATE cache_content_index SET extracted_at = ?1 WHERE extracted_at IS NULL",
                rusqlite::params![now],
            ).ok();
        }

        log::info!(
            "[Nexus] Reindex complete: {files_processed} files, {entities_total} entities, {relations_total} relations, {skipped} skipped, {} errors",
            errors.len()
        );

        Ok(NexusReindexResult {
            files_processed,
            entities_total,
            relations_total,
            skipped,
            errors,
        })
    }

    // ── Document Summarization & Image Description (§5) ──

    /// Path to file_tools.py (document text extraction + image base64 encoding).
    fn file_tools_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("services")
            .join("file_tools.py")
    }

    /// Call file_tools.py and return parsed JSON.
    fn run_file_tools(
        &self,
        python: &str,
        action: &str,
        path: &Path,
    ) -> Result<serde_json::Value, String> {
        let script = Self::file_tools_script();
        if !script.exists() {
            return Err("file_tools.py not found".into());
        }

        let mut cmd = std::process::Command::new(python);
        cmd.env("PYTHONIOENCODING", "utf-8");
        for (k, v) in self.nexus_env_vars() {
            cmd.env(&k, &v);
        }
        cmd.arg(script.to_str().unwrap())
            .arg(action)
            .arg(path.to_str().unwrap())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = cmd.output()
            .map_err(|e| format!("Failed to start file_tools.py: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("file_tools.py failed: {}", stderr.lines().last().unwrap_or("")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse file_tools.py JSON: {e} — raw: {}", &stdout[..stdout.len().min(200)]))
    }

    /// Call extract_service.py --mode summarize, returns raw markdown text.
    fn run_summarize_service(
        &self,
        python: &str,
        text: &str,
        file_type: &str,
        file_name: &str,
    ) -> Result<String, String> {
        let script = Self::extract_service_script();
        if !script.exists() {
            return Err("extract_service.py not found".into());
        }

        let mut cmd = std::process::Command::new(python);
        cmd.env("PYTHONIOENCODING", "utf-8");
        for (k, v) in self.nexus_env_vars() {
            cmd.env(&k, &v);
        }
        cmd.arg(script.to_str().unwrap())
            .arg("--mode").arg("summarize")
            .arg("--file-type").arg(file_type)
            .arg("--file-name").arg(file_name)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to start extract_service.py summarize: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(text.as_bytes()).ok();
        }

        let output = child.wait_with_output()
            .map_err(|e| format!("extract_service.py summarize error: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Summarize failed: {}", stderr.lines().last().unwrap_or("")));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Call extract_service.py --mode describe_images, returns raw markdown text.
    fn run_describe_images_service(
        &self,
        python: &str,
        images_json: &str,
        image_count: u32,
    ) -> Result<String, String> {
        let script = Self::extract_service_script();
        if !script.exists() {
            return Err("extract_service.py not found".into());
        }

        let mut cmd = std::process::Command::new(python);
        cmd.env("PYTHONIOENCODING", "utf-8");
        for (k, v) in self.nexus_env_vars() {
            cmd.env(&k, &v);
        }
        cmd.arg(script.to_str().unwrap())
            .arg("--mode").arg("describe_images")
            .arg("--image-count").arg(image_count.to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to start extract_service.py describe_images: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(images_json.as_bytes()).ok();
        }

        let output = child.wait_with_output()
            .map_err(|e| format!("extract_service.py describe_images error: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Image description failed: {}", stderr.lines().last().unwrap_or("")));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Summarize a document (pdf/docx/pptx): extract text → LLM summary → save .md.
    /// Returns the relative wiki path of the generated .md file.
    pub fn nexus_summarize_document(&self, file_path: &str) -> Result<String, String> {
        let full_path = self.wiki_dir.join(file_path);
        if !full_path.exists() {
            return Err(format!("File not found: {}", full_path.display()));
        }

        let ext = full_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let file_name = full_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document");

        // Generate .md path alongside original file
        let stem = full_path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("summary");
        let parent = full_path.parent().unwrap_or(&self.wiki_dir);
        let md_name = format!("{}.md", stem);
        let md_full_path = parent.join(&md_name);

        // Determine wiki-relative path for the md file
        let md_relative = md_full_path
            .strip_prefix(&self.wiki_dir)
            .unwrap_or(std::path::Path::new(&md_name))
            .to_str()
            .unwrap_or(&md_name)
            .to_string();

        // Check if summary already exists
        if md_full_path.exists() {
            log::info!("[Nexus] Summary already exists: {}", md_relative);
            return Ok(md_relative);
        }

        let python = Self::find_python();
        let python_str = python.to_str().unwrap_or("python");

        // Step 1: Extract text via file_tools.py
        let ft_json = self.run_file_tools(python_str, "extract", &full_path)?;
        let text = ft_json.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let ft_error = ft_json.get("error").and_then(|v| v.as_str());

        if text.is_empty() {
            if let Some(e) = ft_error {
                if !e.is_empty() {
                    return Err(format!("Text extraction failed: {e}"));
                }
            }
            return Ok(md_relative); // Empty but valid — skip silently
        }

        log::info!(
            "[Nexus] Extracted {} chars from {} (type={})",
            text.len(), file_path, ext
        );

        // Step 2: Summarize via LLM
        let summary = self.run_summarize_service(python_str, text, &ext, file_name)?;

        if summary.trim().is_empty() {
            return Err("LLM returned empty summary".into());
        }

        log::info!(
            "[Nexus] Summary generated: {} chars → {}",
            summary.len(), md_relative
        );

        // Step 3: Save .md file with source_file frontmatter linking back to original
        let source_rel = file_path.replace('\\', "/");
        let today = chrono::Utc::now().format("%Y-%m-%d");
        let content_with_fm = format!(
            "---\nsource_file: \"{}\"\ncreated: \"{}\"\n---\n\n{}",
            source_rel, today, summary,
        );
        std::fs::write(&md_full_path, &content_with_fm)
            .map_err(|e| format!("Failed to write summary file: {e}"))?;

        Ok(md_relative)
    }

    /// Describe one or more images via multimodal LLM and save as .md.
    /// Single image → `<stem>.md`, multiple images → `<title>.md`.
    /// Returns the relative wiki path of the generated .md file.
    pub fn nexus_describe_images(
        &self,
        file_paths: &[String],
        title: Option<&str>,
    ) -> Result<String, String> {
        if file_paths.is_empty() {
            return Err("No images provided".into());
        }

        let python = Self::find_python();
        let python_str = python.to_str().unwrap_or("python");

        // Step 1: Get base64 for each image
        let mut base64_list: Vec<String> = Vec::new();
        let mut first_parent: Option<PathBuf> = None;
        let mut first_stem: Option<String> = None;

        for rel_path in file_paths {
            let full_path = self.wiki_dir.join(rel_path);
            if !full_path.exists() {
                log::warn!("[Nexus] Image not found, skipping: {}", rel_path);
                continue;
            }

            let b64_json = self.run_file_tools(python_str, "image_b64", &full_path)?;
            let b64 = b64_json.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let b64_error = b64_json.get("error").and_then(|v| v.as_str());

            if b64.is_empty() {
                if let Some(e) = b64_error {
                    log::warn!("[Nexus] Image base64 failed for {}: {}", rel_path, e);
                }
                continue;
            }

            base64_list.push(b64.to_string());

            if first_parent.is_none() {
                first_parent = Some(full_path.parent().unwrap_or(&self.wiki_dir).to_path_buf());
                first_stem = Some(
                    full_path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("image")
                        .to_string()
                );
            }
        }

        if base64_list.is_empty() {
            return Err("No valid images to process".into());
        }

        let parent = first_parent.unwrap_or_else(|| self.wiki_dir.clone());
        let image_count = base64_list.len() as u32;

        // Step 2: Determine output .md name
        let md_name = if image_count == 1 {
            format!("{}.md", first_stem.unwrap_or_else(|| "image".into()))
        } else {
            let t = title.unwrap_or("图集");
            // Sanitize title for filename
            let safe: String = t.chars()
                .map(|c| if c == '/' || c == '\\' || c == ':' || c == '<' || c == '>' || c == '|' || c == '?' || c == '*' || c == '"' { '_' } else { c })
                .collect();
            format!("{}.md", safe.trim())
        };

        let md_full_path = parent.join(&md_name);
        let md_relative = md_full_path
            .strip_prefix(&self.wiki_dir)
            .unwrap_or(std::path::Path::new(&md_name))
            .to_str()
            .unwrap_or(&md_name)
            .to_string();

        // Check if description already exists
        if md_full_path.exists() {
            log::info!("[Nexus] Image description already exists: {}", md_relative);
            return Ok(md_relative);
        }

        // Step 3: Build JSON input and call describe_images
        let input = serde_json::json!({
            "images": base64_list,
            "title": title.unwrap_or(""),
        });
        let input_str = serde_json::to_string(&input)
            .map_err(|e| format!("JSON serialization failed: {e}"))?;

        let description = self.run_describe_images_service(python_str, &input_str, image_count)?;

        if description.trim().is_empty() {
            return Err("LLM returned empty image description".into());
        }

        log::info!(
            "[Nexus] Image description generated: {} chars for {} images → {}",
            description.len(), image_count, md_relative
        );

        // Step 4: Save .md file with source_files frontmatter linking back to originals
        let source_list: Vec<String> = file_paths.iter()
            .map(|p| format!("\"{}\"", p.replace('\\', "/")))
            .collect();
        let source_list_str = source_list.join(", ");
        let today = chrono::Utc::now().format("%Y-%m-%d");
        let content_with_fm = format!(
            "---\nsource_files: [{}]\ncreated: \"{}\"\n---\n\n{}",
            source_list_str, today, description,
        );
        std::fs::write(&md_full_path, &content_with_fm)
            .map_err(|e| format!("Failed to write image description file: {e}"))?;

        Ok(md_relative)
    }

    /// Call extract_service.py --mode classify, returns JSON {folder, title, tags}.
    fn run_classify_service(
        &self,
        python: &str,
        text: &str,
        file_type: &str,
        file_name: &str,
        existing_dirs: &[String],
    ) -> Result<serde_json::Value, String> {
        let script = Self::extract_service_script();
        if !script.exists() {
            return Err("extract_service.py not found".into());
        }

        let dirs_json = serde_json::to_string(existing_dirs)
            .unwrap_or_else(|_| "[]".to_string());

        let mut cmd = std::process::Command::new(python);
        cmd.env("PYTHONIOENCODING", "utf-8");
        for (k, v) in self.nexus_env_vars() {
            cmd.env(&k, &v);
        }
        cmd.arg(script.to_str().unwrap())
            .arg("--mode").arg("classify")
            .arg("--file-type").arg(file_type)
            .arg("--file-name").arg(file_name)
            .arg("--existing-dirs").arg(&dirs_json)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to start extract_service.py classify: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(text.as_bytes()).ok();
        }

        let output = child.wait_with_output()
            .map_err(|e| format!("extract_service.py classify error: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Classify failed: {}", stderr.lines().last().unwrap_or("")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse classify JSON: {e} — raw: {}", &stdout[..stdout.len().min(300)]))
    }

    /// Collect top-level directory names in the wiki folder.
    pub fn list_wiki_top_dirs(&self) -> Vec<String> {
        let mut dirs: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.wiki_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('_') && !name.starts_with('.') {
                            dirs.push(name.to_string());
                        }
                    }
                }
            }
        }
        dirs.sort();
        dirs
    }

    /// Auto-classify and archive a document: extract text → LLM classify → move to folder → add frontmatter.
    /// Returns JSON {folder, title, tags, file_path: new_relative_path}.
    pub fn nexus_auto_classify(&self, file_path: &str) -> Result<serde_json::Value, String> {
        let full_path = self.wiki_dir.join(file_path);
        if !full_path.exists() {
            return Err(format!("File not found: {}", full_path.display()));
        }

        let ext = full_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let file_name = full_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document");

        // Skip markdown files — they already have frontmatter and location
        if ext == "md" || ext == "canvas" {
            return Ok(serde_json::json!({"skipped": true, "reason": "md/canvas不需要分类"}));
        }

        let python = Self::find_python();
        let python_str = python.to_str().unwrap_or("python");

        // Step 1: Extract text via file_tools.py
        let ft_json = self.run_file_tools(python_str, "extract", &full_path)?;
        let text = ft_json.get("text").and_then(|v| v.as_str()).unwrap_or("");

        if text.is_empty() {
            return Ok(serde_json::json!({"skipped": true, "reason": "无法提取文本"}));
        }

        // Step 2: Classify via LLM
        let existing_dirs = self.list_wiki_top_dirs();
        let classify_result = self.run_classify_service(
            python_str,
            text,
            &ext,
            file_name,
            &existing_dirs,
        )?;

        let folder = classify_result.get("folder")
            .and_then(|v| v.as_str())
            .unwrap_or("笔记");
        let title = classify_result.get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(file_name);
        let tags: Vec<String> = classify_result.get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        log::info!(
            "[Nexus] Classify: {} → folder='{folder}', title='{title}', tags={tags:?}",
            file_path
        );

        // Step 3: Ensure target folder exists
        let target_dir = self.wiki_dir.join(folder);
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("创建目录失败: {e}"))?;

        // Step 4: Move file to target folder
        let new_name = full_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled");
        let new_full_path = target_dir.join(new_name);

        // If destination exists with same name, avoid overwrite
        let final_path = if new_full_path.exists() && new_full_path != full_path {
            let stem = full_path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
            let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
            let new_unique = format!("{}_{}.{}", stem, ts, ext);
            target_dir.join(&new_unique)
        } else {
            new_full_path
        };

        if final_path != full_path {
            std::fs::rename(&full_path, &final_path)
                .map_err(|e| format!("移动文件失败: {e}"))?;
            // Move companion .md (or source binary) alongside
            self.move_companions(&full_path, &final_path, &ext);
        }

        let new_relative = final_path
            .strip_prefix(&self.wiki_dir)
            .unwrap_or(&final_path)
            .to_str()
            .unwrap_or(file_path)
            .replace('\\', "/");

        // Step 5: If md file, add/update frontmatter
        if ext == "md" {
            if let Ok(content) = std::fs::read_to_string(&final_path) {
                let updated = self.update_or_add_frontmatter(&content, title, &tags);
                std::fs::write(&final_path, &updated).ok();
            }
        }

        Ok(serde_json::json!({
            "folder": folder,
            "title": title,
            "tags": tags,
            "file_path": new_relative,
        }))
    }

    /// Add or update YAML frontmatter with title and tags.
    fn update_or_add_frontmatter(&self, content: &str, title: &str, tags: &[String]) -> String {
        let tags_yaml = tags.iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(", ");

        if content.starts_with("---") {
            // Has existing frontmatter — update title and tags
            if let Some(end) = content[3..].find("---") {
                let fm_end = end + 6;
                let fm = &content[..fm_end];
                let body = &content[fm_end..];

                let mut new_fm = String::new();
                let mut has_title = false;
                let mut has_tags = false;
                let mut has_created = false;

                for line in fm.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("title:") {
                        new_fm.push_str(&format!("title: \"{}\"\n", title.replace('"', "\\\"")));
                        has_title = true;
                    } else if trimmed.starts_with("tags:") {
                        new_fm.push_str(&format!("tags: [{}]\n", tags_yaml));
                        has_tags = true;
                    } else if trimmed.starts_with("created:") {
                        has_created = true;
                        new_fm.push_str(line);
                        new_fm.push('\n');
                    } else {
                        new_fm.push_str(line);
                        new_fm.push('\n');
                    }
                }

                if !has_title {
                    new_fm.push_str(&format!("title: \"{}\"\n", title.replace('"', "\\\"")));
                }
                if !has_tags {
                    new_fm.push_str(&format!("tags: [{}]\n", tags_yaml));
                }
                if !has_created {
                    let today = chrono::Utc::now().format("%Y-%m-%d");
                    new_fm.push_str(&format!("created: \"{}\"\n", today));
                }

                format!("{}{}", new_fm.trim_end(), body)
            } else {
                content.to_string()
            }
        } else {
            // No frontmatter — add one
            let today = chrono::Utc::now().format("%Y-%m-%d");
            format!(
                "---\ntitle: \"{}\"\ntags: [{}]\ncreated: \"{}\"\n---\n\n{}",
                title.replace('"', "\\\""),
                tags_yaml,
                today,
                content,
            )
        }
    }

    // ── Synthesis Engine (§4) ──

    /// Run all synthesis rules: shared_neighbor, co_occurrence, type_pattern.
    /// Returns counts of inferred relations and ontology patterns found.
    pub fn nexus_run_synthesis(&self) -> Result<serde_json::Value, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut total_inferred = 0u32;

        // Rule 1: Shared neighbor — A→X←B, X degree≥3, A-B no direct edge
        let shared = self.synthesis_shared_neighbor(&db, &now)?;
        total_inferred += shared;

        // Rule 2: Cross-document co-occurrence — A,B in ≥3 source_paths
        let cooc = self.synthesis_co_occurrence(&db, &now)?;
        total_inferred += cooc;

        // Rule 3: Type pattern discovery
        let patterns = self.synthesis_type_patterns(&db, &now)?;

        log::info!(
            "[Nexus] Synthesis complete: {} inferred edges ({} shared-neighbor, {} co-occurrence), {} type patterns",
            total_inferred, shared, cooc, patterns
        );

        Ok(serde_json::json!({
            "inferred_edges": total_inferred,
            "shared_neighbor": shared,
            "co_occurrence": cooc,
            "type_patterns": patterns,
        }))
    }

    /// Rule 1: Shared neighbor — if A→X←B and X has degree≥3 (hub node),
    /// and A-B have no direct edge, infer A—B related_to (confidence 0.25).
    fn synthesis_shared_neighbor(
        &self,
        db: &SqliteConnection,
        now: &str,
    ) -> Result<u32, String> {
        let mut count = 0u32;

        // Find hub nodes: entities with degree ≥ 3
        let mut stmt = db.prepare(
            "SELECT to_id AS hub, COUNT(DISTINCT from_id) AS degree
             FROM cache_relations
             GROUP BY to_id
             HAVING degree >= 3"
        ).map_err(|e| e.to_string())?;

        let hubs: Vec<(String, u32)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        for (hub_id, _degree) in &hubs {
            // Get all entities connected to this hub
            let mut stmt2 = db.prepare(
                "SELECT DISTINCT from_id FROM cache_relations WHERE to_id = ?1"
            ).map_err(|e| e.to_string())?;

            let neighbors: Vec<String> = stmt2.query_map(
                rusqlite::params![hub_id],
                |row| row.get::<_, String>(0),
            ).map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

            // For each pair of neighbors, check if A-B already has a direct edge
            for i in 0..neighbors.len() {
                for j in (i + 1)..neighbors.len() {
                    let a = &neighbors[i];
                    let b = &neighbors[j];
                    if a == b { continue; }

                    // Check existing edge (either direction, any type)
                    let exists: bool = db.query_row(
                        "SELECT COUNT(*) > 0 FROM cache_relations
                         WHERE (from_id = ?1 AND to_id = ?2) OR (from_id = ?2 AND to_id = ?1)",
                        rusqlite::params![a, b],
                        |row| row.get(0),
                    ).unwrap_or(false);

                    if exists { continue; }

                    // Check existing synthesis
                    let synth_exists: bool = db.query_row(
                        "SELECT COUNT(*) > 0 FROM cache_synthesis
                         WHERE (entity_a_id = ?1 AND entity_b_id = ?2) OR (entity_a_id = ?2 AND entity_b_id = ?1)",
                        rusqlite::params![a, b],
                        |row| row.get(0),
                    ).unwrap_or(false);

                    if synth_exists { continue; }

                    // Insert inferred edge
                    let synth_id = uuid::Uuid::new_v4().to_string();
                    db.execute(
                        "INSERT INTO cache_synthesis (id, entity_a_id, entity_b_id, method, inferred_relation_type, confidence, reasoning, created_at)
                         VALUES (?1, ?2, ?3, 'shared_neighbor', 'related_to', 0.25, ?4, ?5)",
                        rusqlite::params![
                            synth_id, a, b,
                            format!("共享枢纽节点 {hub_id}(度={_degree})"),
                            now,
                        ],
                    ).ok();

                    // Also insert as cache_relation (inferred, low weight)
                    let rel_id = uuid::Uuid::new_v4().to_string();
                    db.execute(
                        "INSERT OR REPLACE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                         VALUES (?1, ?2, ?3, 'related_to', 'inferred', 0.25, 1)",
                        rusqlite::params![rel_id, a, b],
                    ).ok();

                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Rule 2: Cross-document co-occurrence.
    /// If A and B appear together in ≥3 different source_paths, infer co_occurs.
    fn synthesis_co_occurrence(
        &self,
        db: &SqliteConnection,
        now: &str,
    ) -> Result<u32, String> {
        let mut count = 0u32;

        // Find entity pairs that co-occur in multiple source_paths via source_file
        // Use cache_entities with same source_file to find co-occurring entities
        let mut stmt = db.prepare(
            "SELECT e1.id, e2.id, COUNT(DISTINCT e1.source_file) AS shared_docs
             FROM cache_entities e1
             JOIN cache_entities e2 ON e1.source_file = e2.source_file AND e1.id < e2.id
             WHERE e1.source_file IS NOT NULL AND e1.source_file != ''
             GROUP BY e1.id, e2.id
             HAVING shared_docs >= 3
             LIMIT 200"
        ).map_err(|e| e.to_string())?;

        let pairs: Vec<(String, String, u32)> = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        for (a_id, b_id, doc_count) in &pairs {
            // Check existing edge
            let exists: bool = db.query_row(
                "SELECT COUNT(*) > 0 FROM cache_relations
                 WHERE (from_id = ?1 AND to_id = ?2) OR (from_id = ?2 AND to_id = ?1)",
                rusqlite::params![a_id, b_id],
                |row| row.get(0),
            ).unwrap_or(false);

            if exists { continue; }

            // Check existing synthesis
            let synth_exists: bool = db.query_row(
                "SELECT COUNT(*) > 0 FROM cache_synthesis
                 WHERE (entity_a_id = ?1 AND entity_b_id = ?2) OR (entity_a_id = ?2 AND entity_b_id = ?1)",
                rusqlite::params![a_id, b_id],
                |row| row.get(0),
            ).unwrap_or(false);

            if synth_exists { continue; }

            let confidence = (0.2_f64 + (*doc_count as f64) * 0.05).min(0.5);

            let synth_id = uuid::Uuid::new_v4().to_string();
            db.execute(
                "INSERT INTO cache_synthesis (id, entity_a_id, entity_b_id, method, inferred_relation_type, confidence, reasoning, created_at)
                 VALUES (?1, ?2, ?3, 'co_occurrence', 'co_occurs', ?4, ?5, ?6)",
                rusqlite::params![
                    synth_id, a_id, b_id, confidence,
                    format!("在 {doc_count} 个文档中共现"),
                    now,
                ],
            ).ok();

            let rel_id = uuid::Uuid::new_v4().to_string();
            db.execute(
                "INSERT OR REPLACE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                 VALUES (?1, ?2, ?3, 'related_to', 'co_occurs', ?4, 1)",
                rusqlite::params![rel_id, a_id, b_id, confidence],
            ).ok();

            count += 1;
        }

        Ok(count)
    }

    /// Rule 3: Type pattern discovery.
    /// If ≥80% of entities of a given entity_type share the same relation_type,
    /// record the pattern in cache_ontology.
    fn synthesis_type_patterns(
        &self,
        db: &SqliteConnection,
        now: &str,
    ) -> Result<u32, String> {
        let mut patterns = 0u32;

        // For each entity_type, find dominant relation_type patterns
        let mut stmt = db.prepare(
            "SELECT e.entity_type, r.relation_type, COUNT(*) AS cnt
             FROM cache_entities e
             JOIN cache_relations r ON (e.id = r.from_id OR e.id = r.to_id)
             WHERE e.entity_type NOT IN ('concept', 'content')  -- skip generic types
             GROUP BY e.entity_type, r.relation_type
             ORDER BY e.entity_type, cnt DESC"
        ).map_err(|e| e.to_string())?;

        let rows: Vec<(String, String, u32)> = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        // Group by entity_type, check if top relation_type exceeds 80%
        let mut type_totals: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut type_top: std::collections::HashMap<String, (String, u32)> = std::collections::HashMap::new();

        for (etype, rel_type, cnt) in &rows {
            *type_totals.entry(etype.clone()).or_insert(0) += cnt;
            let entry = type_top.entry(etype.clone()).or_insert((rel_type.clone(), 0));
            if cnt > &entry.1 {
                *entry = (rel_type.clone(), *cnt);
            }
        }

        for (etype, (rel_type, top_cnt)) in &type_top {
            let total = type_totals.get(etype).copied().unwrap_or(1);
            let ratio = *top_cnt as f64 / total as f64;
            if ratio >= 0.8 && total >= 5 {
                let id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, format!("pattern:{etype}:{rel_type}").as_bytes()).to_string();
                db.execute(
                    "INSERT OR REPLACE INTO cache_ontology (id, category, type_name, usage_count, canonical_suggestion, similar_types, status, last_analyzed)
                     VALUES (?1, 'relation_type', ?2, ?3, ?4, '[]', 'pending', ?5)",
                    rusqlite::params![id, rel_type, total, format!("{:.0}% 的 {etype} 实体通过 {rel_type} 连接", ratio * 100.0), now],
                ).ok();
                patterns += 1;
            }
        }

        Ok(patterns)
    }

    /// Type convergence analysis (§6.2).
    /// Count entity_type frequencies, cluster low-frequency types, generate merge suggestions.
    pub fn nexus_analyze_types(&self) -> Result<serde_json::Value, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();

        // Check last analysis time
        let _last_analysis: Option<String> = db.query_row(
            "SELECT value FROM cache_meta WHERE key = 'last_type_analysis'",
            [],
            |row| row.get(0),
        ).ok();

        // Collect all entity_type frequencies
        let mut stmt = db.prepare(
            "SELECT entity_type, COUNT(*) as cnt FROM cache_entities GROUP BY entity_type ORDER BY cnt DESC"
        ).map_err(|e| e.to_string())?;

        let types: Vec<(String, u32)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        let total_types = types.len();
        let low_freq: Vec<&(String, u32)> = types.iter().filter(|(_, cnt)| *cnt < 3).collect();
        let mut suggestions = 0u32;

        // Cluster low-frequency types by Levenshtein similarity
        for i in 0..low_freq.len() {
            for j in (i + 1)..low_freq.len() {
                let (name_a, _) = &low_freq[i];
                let (name_b, _) = &low_freq[j];

                let sim = strsim::normalized_levenshtein(name_a, name_b);
                if sim > 0.6 {
                    // Check if suggestion already exists
                    let exists: bool = db.query_row(
                        "SELECT COUNT(*) > 0 FROM cache_ontology
                         WHERE category = 'entity_type' AND type_name = ?1 AND canonical_suggestion = ?2",
                        rusqlite::params![name_a, name_b],
                        |row| row.get(0),
                    ).unwrap_or(false);

                    if !exists {
                        let id = uuid::Uuid::new_v4().to_string();
                        let similar = serde_json::json!([name_a, name_b]).to_string();
                        db.execute(
                            "INSERT INTO cache_ontology (id, category, type_name, usage_count, canonical_suggestion, similar_types, status, last_analyzed)
                             VALUES (?1, 'entity_type', ?2, ?3, ?4, ?5, 'pending', ?6)",
                            rusqlite::params![
                                id,
                                name_a,
                                types.iter().find(|(n, _)| n == name_a).map(|(_, c)| *c).unwrap_or(0),
                                name_b,
                                similar,
                                now,
                            ],
                        ).ok();
                        suggestions += 1;
                    }
                }
            }
        }

        // Update last analysis timestamp
        db.execute(
            "INSERT OR REPLACE INTO cache_meta (key, value) VALUES ('last_type_analysis', ?1)",
            rusqlite::params![now],
        ).ok();

        log::info!(
            "[Nexus] Type analysis: {total_types} types, {} low-frequency, {suggestions} merge suggestions",
            low_freq.len()
        );

        Ok(serde_json::json!({
            "total_types": total_types,
            "low_frequency_types": low_freq.len(),
            "merge_suggestions": suggestions,
            "types": types.iter().map(|(name, cnt)| serde_json::json!({"name": name, "count": cnt})).collect::<Vec<_>>(),
        }))
    }

    pub async fn extract_entities(
        &self,
        file_path: &str,
        _namespace: &str,
    ) -> Result<ExtractionCompleteEvent, String> {
        // Redirect to Nexus LLM extraction
        match self.nexus_extract_from_file(file_path, None) {
            Ok(result) => Ok(ExtractionCompleteEvent {
                new_count: result.entity_count,
                updated_count: 0,
                source_file: file_path.to_string(),
                snapshot_updated: false,
            }),
            Err(e) => {
                log::warn!("Nexus extract failed: {e}, falling back to local regex");
                self.extract_entities_local(file_path, _namespace)
            }
        }
    }

    pub async fn run_lint_checks(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<LintWarning>, String> {
        let graph = self
            .get_graph_data(namespace, "entity", None, None)
            .await?;
        let entities = &graph.entities;
        let relations = &graph.relations;
        let mut warnings: Vec<LintWarning> = Vec::new();

        // Build id sets
        let entity_ids: std::collections::HashSet<&str> =
            entities.iter().map(|e| e.id.as_str()).collect();
        let _entity_names: std::collections::HashMap<&str, &Entity> =
            entities.iter().map(|e| (e.name.as_str(), e)).collect();

        // 1. Orphan nodes: entities with no relations
        let mut connected: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for r in relations {
            connected.insert(r.from_id.as_str());
            connected.insert(r.to_id.as_str());
        }
        for e in entities {
            if !connected.contains(e.id.as_str()) && !e.hidden {
                warnings.push(LintWarning {
                    warning_type: "orphan_node".into(),
                    entity_id: Some(e.id.clone()),
                    entity_name: e.name.clone(),
                    message: format!("孤立节点: \"{}\" 没有任何关系连接", e.name),
                    severity: "warn".into(),
                });
            }
        }

        // 2. Dead links: relations pointing to non-existent entities
        for r in relations {
            if !entity_ids.contains(r.from_id.as_str()) {
                warnings.push(LintWarning {
                    warning_type: "dead_link".into(),
                    entity_id: Some(r.id.clone()),
                    entity_name: r.from_id.clone(),
                    message: format!("死链接: 关系 {} 的源实体不存在", r.id),
                    severity: "error".into(),
                });
            }
            if !entity_ids.contains(r.to_id.as_str()) {
                warnings.push(LintWarning {
                    warning_type: "dead_link".into(),
                    entity_id: Some(r.id.clone()),
                    entity_name: r.to_id.clone(),
                    message: format!("死链接: 关系 {} 的目标实体不存在", r.id),
                    severity: "error".into(),
                });
            }
        }

        // 3. Low confidence entities
        for e in entities {
            if e.confidence < 0.5 && !e.hidden {
                warnings.push(LintWarning {
                    warning_type: "low_confidence".into(),
                    entity_id: Some(e.id.clone()),
                    entity_name: e.name.clone(),
                    message: format!(
                        "低置信度: \"{}\" 置信度仅 {:.0}%",
                        e.name,
                        e.confidence * 100.0
                    ),
                    severity: "info".into(),
                });
            }
        }

        // 4. Stale entities: not updated in 30+ days
        let thirty_days = chrono::TimeDelta::days(30);
        let now = chrono::Utc::now();
        for e in entities {
            if let Ok(updated) = chrono::DateTime::parse_from_rfc3339(&e.updated_at) {
                let updated_utc = updated.with_timezone(&chrono::Utc);
                if now - updated_utc > thirty_days && !e.hidden {
                    warnings.push(LintWarning {
                        warning_type: "stale_entity".into(),
                        entity_id: Some(e.id.clone()),
                        entity_name: e.name.clone(),
                        message: format!("过期实体: \"{}\" 已超过30天未更新", e.name),
                        severity: "info".into(),
                    });
                }
            }
        }

        // 5. Duplicate entities: similar names
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                let dist = strsim::levenshtein(
                    &names[i].to_lowercase(),
                    &names[j].to_lowercase(),
                );
                let max_len = names[i].len().max(names[j].len()) as f64;
                let similarity = 1.0 - (dist as f64 / max_len.max(1.0));
                if similarity > 0.85 && entities[i].id != entities[j].id {
                    warnings.push(LintWarning {
                        warning_type: "duplicate_entity".into(),
                        entity_id: Some(entities[i].id.clone()),
                        entity_name: entities[i].name.clone(),
                        message: format!(
                            "疑似重复: \"{}\" 与 \"{}\" 名称相似度 {:.0}%",
                            entities[i].name,
                            entities[j].name,
                            similarity * 100.0
                        ),
                        severity: "warn".into(),
                    });
                }
            }
        }

        Ok(warnings)
    }

    // ── Smart Display ──

    /// Entity type is now free-form String — just return as-is.
    fn entity_type_str(t: &EntityType) -> &str {
        t.as_str()
    }

    /// weighted degree = Σ w(e) for all edges incident to the entity.
    /// Uses the degree_map (count) + average weight from relations to approximate.
    fn compute_weighted_degree(
        entity_id: &str,
        degree: u32,
        relations: &[Relation],
    ) -> f32 {
        if degree == 0 {
            return 0.0;
        }
        let total_weight: f32 = relations
            .iter()
            .filter(|r| r.from_id == entity_id || r.to_id == entity_id)
            .map(|r| r.weight)
            .sum();
        total_weight
    }

    /// structural_score = normalize(Σw) × 0.6 + confidence × 0.4
    fn compute_structural_score(
        entity: &Entity,
        weighted_degree: f32,
        w95: f32,
    ) -> f32 {
        let normalized_wd = (weighted_degree / w95.max(0.001)).min(1.0);
        normalized_wd * 0.6 + entity.confidence * 0.4
    }

    /// Compute user_score from cache_entity_scores row + decay
    fn compute_user_score(
        scores: &std::collections::HashMap<String, (f32, f32)>, // entity_id -> (manual_boost, interaction_decay)
        entity_id: &str,
    ) -> f32 {
        if let Some((manual, interaction)) = scores.get(entity_id) {
            (*manual + *interaction).min(1.0)
        } else {
            0.0
        }
    }

    /// display_score = structural × 0.7 + user × 0.3
    fn compute_display_score(
        structural_score: f32,
        user_score: f32,
    ) -> f32 {
        structural_score * 0.7 + user_score * 0.3
    }

    /// Load entity interaction scores from cache_entity_scores with decay applied.
    fn load_entity_scores(&self) -> Result<std::collections::HashMap<String, (f32, f32)>, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db
            .prepare("SELECT entity_id, manual_boost, view_count, last_viewed, reference_count, last_referenced, focus_count, last_focused FROM cache_entity_scores")
            .map_err(|e| e.to_string())?;
        let now = chrono::Utc::now();
        let mut scores = std::collections::HashMap::new();
        let rows: Vec<(String, f32, i32, Option<String>, i32, Option<String>, i32, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                    row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        for (eid, manual, vc, lv, rc, lr, fc, lf) in rows {
            let interaction = Self::decay_interaction(
                &now, vc, lv.as_deref(), 7.0, 0.05,
            ) + Self::decay_interaction(
                &now, rc, lr.as_deref(), 30.0, 0.10,
            ) + Self::decay_interaction(
                &now, fc, lf.as_deref(), 3.0, 0.03,
            );
            scores.insert(eid, (manual, interaction.min(1.0)));
        }
        Ok(scores)
    }

    fn decay_interaction(
        now: &chrono::DateTime<chrono::Utc>,
        count: i32,
        last_time: Option<&str>,
        half_life_days: f32,
        weight_per: f32,
    ) -> f32 {
        if count == 0 {
            return 0.0;
        }
        let days_since = last_time
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| {
                (now.signed_duration_since(t.with_timezone(&chrono::Utc)))
                    .num_days()
                    .max(0) as f32
            })
            .unwrap_or(half_life_days * 2.0); // no timestamp → assume fully decayed
        let decay = 0.5_f32.powf(days_since / half_life_days);
        count as f32 * decay * weight_per
    }

    pub fn set_entity_importance(
        &self,
        entity_id: &str,
        manual_boost: f32,
    ) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO cache_entity_scores (entity_id, manual_boost, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(entity_id) DO UPDATE SET manual_boost = excluded.manual_boost, updated_at = excluded.updated_at",
            rusqlite::params![entity_id, manual_boost, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn record_entity_view(&self, entity_id: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO cache_entity_scores (entity_id, view_count, last_viewed, updated_at)
             VALUES (?1, 1, ?2, ?2)
             ON CONFLICT(entity_id) DO UPDATE SET
                view_count = view_count + 1,
                last_viewed = excluded.last_viewed,
                updated_at = excluded.updated_at",
            rusqlite::params![entity_id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_entity_score(
        &self,
        entity_id: &str,
    ) -> Result<serde_json::Value, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db
            .prepare("SELECT manual_boost, view_count, last_viewed, reference_count, last_referenced, focus_count, last_focused FROM cache_entity_scores WHERE entity_id = ?1")
            .map_err(|e| e.to_string())?;
        let row = stmt.query_row(rusqlite::params![entity_id], |row| {
            Ok(serde_json::json!({
                "entity_id": entity_id,
                "manual_boost": row.get::<_, f32>(0)?,
                "view_count": row.get::<_, i32>(1)?,
                "last_viewed": row.get::<_, Option<String>>(2)?,
                "reference_count": row.get::<_, i32>(3)?,
                "last_referenced": row.get::<_, Option<String>>(4)?,
                "focus_count": row.get::<_, i32>(5)?,
                "last_focused": row.get::<_, Option<String>>(6)?,
            }))
        });
        match row {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(serde_json::json!({
                "entity_id": entity_id,
                "manual_boost": 0.0,
                "view_count": 0,
                "reference_count": 0,
                "focus_count": 0,
            })),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn get_smart_display(
        &self,
        namespace: Option<&str>,
        config: &crate::models::knowledge::SmartDisplayConfig,
    ) -> Result<crate::models::knowledge::SmartDisplayResult, String> {
        let graph = self
            .get_graph_data(namespace, "entity", config.focal_node.as_deref(), Some(config.hops))
            .await?;

        let entities = graph.entities;
        let relations = graph.relations;

        // Compute degree + weighted degree for each entity
        let mut degree_map: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
        let mut wd_map: std::collections::HashMap<&str, f32> = std::collections::HashMap::new();
        for r in &relations {
            *degree_map.entry(r.from_id.as_str()).or_insert(0) += 1;
            *degree_map.entry(r.to_id.as_str()).or_insert(0) += 1;
            *wd_map.entry(r.from_id.as_str()).or_insert(0.0) += r.weight;
            *wd_map.entry(r.to_id.as_str()).or_insert(0.0) += r.weight;
        }
        // 95th percentile of weighted degrees for normalization
        let mut all_wd: Vec<f32> = wd_map.values().copied().collect();
        all_wd.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let w95 = if all_wd.is_empty() { 1.0 } else {
            let idx = ((all_wd.len() - 1) as f32 * 0.95) as usize;
            all_wd[idx].max(0.001)
        };

        // Load persisted interaction scores (manual_boost + decayed interactions)
        let user_scores = self.load_entity_scores().unwrap_or_default();

        let min_per_type = config.min_per_type.max(1) as usize;
        let tier1_cap = config.tier1_cap.max(50).min(300) as usize;
        let tier2_cap = config.tier2_cap.max(100).min(600) as usize;
        let total_cap: usize = 800;

        let _entity_map: std::collections::HashMap<&str, &Entity> =
            entities.iter().map(|e| (e.id.as_str(), e)).collect();

        // B4: build degree map for orphan detection
        let deg_for_orphan: std::collections::HashMap<&str, u32> = degree_map.clone();

        // Score each visible entity with new dual-axis formula
        let search_lower = config.search_query.as_ref().map(|q| q.to_lowercase());
        let type_filter = config.type_filter.as_ref();
        let min_imp = config.min_importance.unwrap_or(0.0);
        let show_orphans = config.show_orphans.unwrap_or(true);
        let mut scored: Vec<(f32, &Entity)> = entities
            .iter()
            .filter(|e| {
                if e.hidden { return false; }
                // Search query filter
                if let Some(ref q) = search_lower {
                    if !q.is_empty() && !e.name.to_lowercase().contains(q) && !e.entity_type.to_lowercase().contains(q) {
                        return false;
                    }
                }
                // Type filter
                if let Some(tf) = type_filter {
                    if !tf.is_empty() && Self::entity_type_str(&e.entity_type) != tf.as_str() {
                        return false;
                    }
                }
                // Namespace filter (client-side only — Entity model has no namespace field)
                // Min importance filter
                if e.importance.unwrap_or(0.0) < min_imp {
                    return false;
                }
                // Orphan filter
                if !show_orphans {
                    let deg = deg_for_orphan.get(e.id.as_str()).copied().unwrap_or(0);
                    if deg == 0 { return false; }
                }
                true
            })
            .map(|e| {
                let wd = wd_map.get(e.id.as_str()).copied().unwrap_or(0.0);
                let structural = Self::compute_structural_score(e, wd, w95);
                let user = Self::compute_user_score(&user_scores, &e.id);
                let display = Self::compute_display_score(structural, user);
                (display, e)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // ── Tier assignment ──
        let mut tier: std::collections::HashMap<String, u8> = std::collections::HashMap::new();
        let mut tier1_type_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let type_names: [&str; 11] = [
            "concept", "content", "person", "event", "artifact",
            "function", "class", "method", "interface", "module", "variable",
        ];

        // Pass 1: min_per_type for T1
        for t in &type_names {
            for (_, e) in &scored {
                if Self::entity_type_str(&e.entity_type) == *t {
                    let cnt = tier1_type_counts.entry(t).or_insert(0);
                    if *cnt < min_per_type {
                        tier.insert(e.id.clone(), 1);
                        *cnt += 1;
                    } else {
                        break;
                    }
                }
            }
        }

        // Pass 2: fill T1 by display_score
        for (_, e) in &scored {
            if tier.contains_key(e.id.as_str()) { continue; }
            if tier.len() >= total_cap { break; }
            if tier.values().filter(|&&v| v == 1).count() >= tier1_cap {
                break;
            }
            tier.insert(e.id.clone(), 1);
        }

        // Pass 3: fill T2 by display_score
        for (_, e) in &scored {
            if tier.contains_key(e.id.as_str()) { continue; }
            if tier.len() >= total_cap { break; }
            let _t1_count = tier.values().filter(|&&v| v == 1).count();
            let t2_count = tier.values().filter(|&&v| v == 2).count();
            if t2_count >= tier2_cap { break; }
            tier.insert(e.id.clone(), 2);
        }

        // Pass 4: remaining → T3
        for (_, e) in &scored {
            if tier.contains_key(e.id.as_str()) { continue; }
            if tier.len() >= total_cap { break; }
            tier.insert(e.id.clone(), 3);
        }

        // ── Build result: all entities with tier, all relations (except T3↔T3) ──
        let mut result_entities: Vec<Entity> = Vec::with_capacity(entities.len());
        for e in &entities {
            let mut e = (*e).clone();
            e.display_tier = tier.get(e.id.as_str()).copied();
            result_entities.push(e);
        }

        let tier_for: std::collections::HashMap<&str, u8> = tier.iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();

        // Keep relations unless both ends are T3 (satellite ↔ satellite = noise)
        let filtered_relations: Vec<Relation> = relations
            .into_iter()
            .filter(|r| {
                let ta = tier_for.get(r.from_id.as_str()).copied().unwrap_or(0);
                let tb = tier_for.get(r.to_id.as_str()).copied().unwrap_or(0);
                !(ta == 3 && tb == 3)
            })
            .collect();

        let total = entities.iter().filter(|e| !e.hidden).count() as u32;
        let t1 = tier.values().filter(|&&v| v == 1).count() as u32;
        let t2 = tier.values().filter(|&&v| v == 2).count() as u32;
        let t3 = tier.values().filter(|&&v| v == 3).count() as u32;
        let hidden = total.saturating_sub(t1 + t2 + t3);

        let mut type_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for e in entities.iter().filter(|e| !e.hidden) {
            *type_map
                .entry(Self::entity_type_str(&e.entity_type).to_string())
                .or_insert(0) += 1;
        }
        let entity_types: Vec<crate::models::knowledge::TypeCount> = type_map
            .into_iter()
            .map(|(entity_type, count)| crate::models::knowledge::TypeCount { entity_type, count })
            .collect();

        Ok(crate::models::knowledge::SmartDisplayResult {
            entities: result_entities,
            relations: filtered_relations,
            stats: crate::models::knowledge::SmartDisplayStats {
                total_entities: total,
                displayed_entities: t1 + t2 + t3,
                hidden_entities: hidden,
                tier1_count: t1,
                tier2_count: t2,
                tier3_count: t3,
                entity_types,
            },
        })
    }

    // ── Importance Recalculation ──

    /// Local cache-level importance recalculation (simplified: degree + recency).
    pub fn recalculate_importance(&self) -> Result<u32, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let mut degree_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut rel_stmt = db
            .prepare("SELECT from_id, to_id FROM cache_relations")
            .map_err(|e| e.to_string())?;
        let _ = rel_stmt.query_map([], |row| {
            let from_id: String = row.get(0)?;
            let to_id: String = row.get(1)?;
            *degree_map.entry(from_id).or_insert(0) += 1;
            *degree_map.entry(to_id).or_insert(0) += 1;
            Ok(())
        });

        let max_degree = degree_map.values().max().copied().unwrap_or(1) as f32;
        let thirty_days_ago = (chrono::Utc::now() - chrono::TimeDelta::days(30)).to_rfc3339();

        let mut stmt = db
            .prepare(
                "SELECT id, confidence, updated_at FROM cache_entities WHERE hidden = 0"
            )
            .map_err(|e| e.to_string())?;
        let entities: Vec<(String, f32, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let mut update_stmt = db
            .prepare("UPDATE cache_entities SET confidence = ?1 WHERE id = ?2")
            .map_err(|e| e.to_string())?;

        let mut updated: u32 = 0;
        for (id, confidence, updated_at) in &entities {
            let degree = *degree_map.get(id).unwrap_or(&0) as f32;
            let degree_boost = (degree / max_degree.max(0.001) * 0.2).min(0.2);
            let stale_penalty = if updated_at.as_str() < thirty_days_ago.as_str() {
                -0.1f32
            } else {
                0.0
            };
            let new_importance = (confidence + degree_boost + stale_penalty).clamp(0.1, 1.0);
            if (new_importance - confidence).abs() > 0.005 {
                update_stmt
                    .execute(rusqlite::params![new_importance, id])
                    .map_err(|e| e.to_string())?;
                updated += 1;
            }
        }

        log::info!("Recalculated importance for {updated} entities");
        Ok(updated)
    }

    /// P2.3: Call Heimdall /api/importance/recalc for full composite importance scoring.
    pub async fn recalc_heimdall_importance(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let mut url = format!("{}/api/importance/recalc", self.heimdall_url);
        if let Some(ns) = namespace {
            url.push_str(&format!("?namespace={ns}"));
        }
        let resp = self.client.post(&url).send().await.map_err(|e| e.to_string())?;
        let body = resp.text().await.map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;
        Ok(v)
    }

    /// P2.3: Call Heimdall /api/importance/top for high-importance entities.
    pub async fn get_top_importance_entities(
        &self,
        limit: u32,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let mut url = format!("{}/api/importance/top?limit={limit}", self.heimdall_url);
        if let Some(ns) = namespace {
            url.push_str(&format!("&namespace={ns}"));
        }
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let body = resp.text().await.map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;
        Ok(v)
    }

    /// P2.3: Call Heimdall /api/importance/levels for level distribution.
    pub async fn get_importance_levels(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let mut url = format!("{}/api/importance/levels", self.heimdall_url);
        if let Some(ns) = namespace {
            url.push_str(&format!("?namespace={ns}"));
        }
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let body = resp.text().await.map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;
        Ok(v)
    }

    // ── Neighbors ──

    pub async fn get_neighbors(
        &self,
        entity_id: &str,
        _namespace: Option<&str>,
    ) -> Result<crate::models::knowledge::NeighborResult, String> {
        let detail = self.get_entity_detail(entity_id).await?;
        let entity = detail.entity;

        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Get all neighbor IDs from relations
        let mut neighbor_ids: Vec<String> = Vec::new();
        for r in &detail.inbound_relations {
            if !neighbor_ids.contains(&r.from_id) {
                neighbor_ids.push(r.from_id.clone());
            }
        }
        for r in &detail.outbound_relations {
            if !neighbor_ids.contains(&r.to_id) {
                neighbor_ids.push(r.to_id.clone());
            }
        }

        let mut neighbors = Vec::new();
        let mut all_relations = detail.inbound_relations.clone();
        all_relations.extend(detail.outbound_relations);

        if !neighbor_ids.is_empty() {
            let placeholders = neighbor_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at, color, hidden FROM cache_entities WHERE id IN ({})",
                placeholders
            );

            let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
            let ids: Vec<&dyn rusqlite::types::ToSql> = neighbor_ids
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();

            neighbors = stmt
                .query_map(ids.as_slice(), |row| {
                    let aliases_str: String = row.get(4).unwrap_or_default();
                    let aliases: Vec<String> = serde_json::from_str(&aliases_str).unwrap_or_default();
                    let props_str: String = row.get(5).unwrap_or_default();
                    let properties: serde_json::Value = serde_json::from_str(&props_str).unwrap_or_default();
                    Ok(Entity {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        entity_type: parse_entity_type(&row.get::<_, String>(2)?),
                        description: row.get(3)?,
                        aliases,
                        properties,
                        confidence: row.get(6)?,
                        source_file: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        color: row.get(10)?,
                        hidden: row.get::<_, i32>(11)? != 0,
                        ..Default::default()
                    })
                })
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
        }

        Ok(crate::models::knowledge::NeighborResult {
            entity,
            neighbors,
            relations: all_relations,
        })
    }

    // ── Namespaces ──

    pub async fn get_namespaces(&self) -> Result<Vec<crate::models::knowledge::NamespaceInfo>, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Extract namespace from source_file patterns (e.g., "namespace/file.md")
        let mut stmt = db
            .prepare("SELECT source_file FROM cache_entities WHERE source_file IS NOT NULL AND source_file != ''")
            .map_err(|e| e.to_string())?;

        let mut ns_map: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();

        let files: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        for file in &files {
            if let Some((ns, _)) = file.split_once('/') {
                let entry = ns_map.entry(ns.to_string()).or_insert((0, 0));
                entry.0 += 1;
            } else {
                let entry = ns_map.entry("default".to_string()).or_insert((0, 0));
                entry.0 += 1;
            }
        }

        // Count relations per namespace (approximate via entity membership)
        let namespaces: Vec<crate::models::knowledge::NamespaceInfo> = ns_map
            .into_iter()
            .map(|(name, (entity_count, relation_count))| {
                crate::models::knowledge::NamespaceInfo {
                    name,
                    entity_count,
                    relation_count,
                }
            })
            .collect();

        Ok(namespaces)
    }

    // ── Knowledge Stats ──

    pub async fn get_stats(&self) -> Result<crate::models::knowledge::KnowledgeStats, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let entity_count: u32 = db
            .query_row("SELECT COUNT(*) FROM cache_entities WHERE hidden = 0", [], |row| row.get(0))
            .unwrap_or(0);

        let relation_count: u32 = db
            .query_row("SELECT COUNT(*) FROM cache_relations", [], |row| row.get(0))
            .unwrap_or(0);

        let avg_confidence: f32 = db
            .query_row("SELECT AVG(confidence) FROM cache_entities WHERE hidden = 0", [], |row| row.get(0))
            .unwrap_or(0.0);

        // Entity type breakdown
        let mut stmt = db
            .prepare("SELECT entity_type, COUNT(*) FROM cache_entities WHERE hidden = 0 GROUP BY entity_type")
            .map_err(|e| e.to_string())?;
        let entity_type_breakdown: Vec<crate::models::knowledge::TypeCount> = stmt
            .query_map([], |row| {
                let t: String = row.get(0)?;
                let c: u32 = row.get(1)?;
                Ok(crate::models::knowledge::TypeCount {
                    entity_type: t,
                    count: c,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Relation type breakdown
        let mut rel_stmt = db
            .prepare("SELECT relation_type, COUNT(*) FROM cache_relations GROUP BY relation_type")
            .map_err(|e| e.to_string())?;
        let relation_type_breakdown: Vec<crate::models::knowledge::TypeCount> = rel_stmt
            .query_map([], |row| {
                let t: String = row.get(0)?;
                let c: u32 = row.get(1)?;
                Ok(crate::models::knowledge::TypeCount {
                    entity_type: t,
                    count: c,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Orphan count
        let orphan_count: u32 = entity_count.saturating_sub(
            db.query_row(
                "SELECT COUNT(DISTINCT e.id) FROM cache_entities e
                 INNER JOIN cache_relations r ON e.id = r.from_id OR e.id = r.to_id
                 WHERE e.hidden = 0",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0),
        );

        // Last extraction
        let last_extraction: Option<String> = db
            .query_row(
                "SELECT value FROM cache_meta WHERE key = 'last_extraction'",
                [],
                |row| row.get(0),
            )
            .ok();

        // Namespace count
        let namespace_count: u32 = db
            .query_row(
                "SELECT COUNT(DISTINCT substr(source_file, 1, instr(source_file || '/', '/') - 1)) FROM cache_entities WHERE source_file IS NOT NULL AND source_file != ''",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        Ok(crate::models::knowledge::KnowledgeStats {
            total_entities: entity_count,
            total_relations: relation_count,
            entity_type_breakdown,
            relation_type_breakdown,
            avg_confidence,
            orphan_count,
            namespace_count,
            last_extraction,
            db_size_bytes: 0,
        })
    }

    // ── Cross-Document Relation Discovery ──

    /// Discover relations between documents in the same folder or with the same tags
    pub fn discover_cross_document_relations(&self) -> Result<u32, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Get all document entities (those with source_file set)
        let mut stmt = db
            .prepare("SELECT id, name, source_file FROM cache_entities WHERE source_file IS NOT NULL AND source_file != '' AND hidden = 0")
            .map_err(|e| e.to_string())?;
        let docs: Vec<(String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get::<_, String>(2)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let mut created: u32 = 0;

        // 1. Same folder → related_to relations
        let mut folder_groups: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for (id, _, source_file) in &docs {
            if let Some((folder, _)) = source_file.rsplit_once('/') {
                folder_groups.entry(folder.to_string()).or_default().push(id.clone());
            }
        }
        for (_folder, doc_ids) in &folder_groups {
            if doc_ids.len() < 2 { continue; }
            for i in 0..doc_ids.len() {
                for j in (i + 1)..doc_ids.len() {
                    let rel_id = uuid::Uuid::new_v4().to_string();
                    let result = db.execute(
                        "INSERT OR IGNORE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                         VALUES (?1, ?2, ?3, 'related_to', '同目录', 0.4, 1)",
                        rusqlite::params![rel_id, doc_ids[i], doc_ids[j]],
                    );
                    if result.is_ok() && db.changes() > 0 { created += 1; }
                }
            }
        }

        // 2. Same tag → related_to relations (use frontmatter tags from wiki files)
        // Read wiki files to get tag associations with entity IDs
        let mut tag_entity_map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for (id, name, source_file) in &docs {
            let target = self.wiki_dir.join(source_file);
            if let Ok(content) = std::fs::read_to_string(&target) {
                if let Ok(tags) = Self::extract_tags_from_frontmatter(&content) {
                    for tag in tags {
                        tag_entity_map.entry(tag).or_default().push(id.clone());
                    }
                    // Also relate the document entity to concept entities with matching names
                    let _name = name.clone();
                }
            }
        }
        for (_tag, doc_ids) in &tag_entity_map {
            if doc_ids.len() < 2 { continue; }
            for i in 0..doc_ids.len() {
                for j in (i + 1)..doc_ids.len() {
                    let rel_id = uuid::Uuid::new_v4().to_string();
                    let result = db.execute(
                        "INSERT OR IGNORE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                         VALUES (?1, ?2, ?3, 'related_to', '同标签', 0.3, 1)",
                        rusqlite::params![rel_id, doc_ids[i], doc_ids[j]],
                    );
                    if result.is_ok() && db.changes() > 0 { created += 1; }
                }
            }
        }

        log::info!("Discovered {created} cross-document relations");
        Ok(created)
    }

    fn extract_tags_from_frontmatter(content: &str) -> Result<Vec<String>, String> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") { return Ok(Vec::new()); }
        let after_first = &trimmed[3..];
        if let Some(end_idx) = after_first.find("\n---") {
            let fm_text = &after_first[..end_idx];
            for line in fm_text.lines() {
                let line = line.trim();
                if line.starts_with("tags:") || line.starts_with("标签:") {
                    let val = line.splitn(2, ':').nth(1).unwrap_or("").trim();
                    let mut tags: Vec<String> = val
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .split(',')
                        .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                    tags.sort();
                    return Ok(tags);
                }
            }
        }
        Ok(Vec::new())
    }

    // ── Canvas Sync ──

    /// Sync a Canvas JSON file's nodes and edges into the knowledge graph.
    /// Returns (new_entities, new_relations).
    pub fn sync_canvas_to_graph(&self, canvas_path: &str) -> Result<(u32, u32), String> {
        let canvas_dir = std::path::Path::new(".")
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("canvases");
        let file_path = canvas_dir.join(canvas_path);

        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("读取 Canvas 文件失败: {e}"))?;
        let doc: crate::models::canvas::CanvasDocument =
            serde_json::from_str(&content)
                .map_err(|e| format!("解析 Canvas JSON 失败: {e}"))?;

        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut new_entities: u32 = 0;
        let mut new_relations: u32 = 0;

        // Map node id → entity id for relation creation
        let mut node_entity_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        // Create entities from nodes (skip group nodes)
        for node in &doc.nodes {
            if node.node_type == "group" {
                continue;
            }
            let text = node.content.trim().to_string();
            if text.is_empty() {
                continue;
            }

            // Use existing entity_ref or create new entity
            let entity_id = if let Some(ref entity_ref) = node.entity_ref {
                entity_ref.clone()
            } else {
                let eid = format!(
                    "canvas:{}:{}",
                    canvas_path.trim_end_matches(".canvas.json"),
                    node.id
                );
                let name = text.lines().next().unwrap_or(&text).trim();
                let name = if name.len() > 80 { &name[..80] } else { name };

                let entity_type = "concept".to_string();
                db.execute(
                    "INSERT OR IGNORE INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '[]', '{}', 0.5, ?5, ?6, ?6)",
                    rusqlite::params![eid, name, entity_type, text, canvas_path, now],
                )
                .map_err(|e| format!("创建 Canvas 实体失败: {e}"))?;
                if db.changes() > 0 {
                    new_entities += 1;
                }
                eid
            };

            node_entity_map.insert(node.id.clone(), entity_id.clone());
        }

        // Create relations from connections
        let cn_label_map: std::collections::HashMap<&str, &str> = [
            ("使用", "uses"),
            ("依赖", "depends_on"),
            ("包含", "contains"),
            ("创建", "created_by"),
            ("前置", "preceded_by"),
            ("实现", "implements"),
            ("矛盾", "contradicts"),
            ("引用", "references"),
            ("关联", "related_to"),
        ]
        .iter()
        .cloned()
        .collect();

        for conn in &doc.connections {
            let from_eid = match node_entity_map.get(&conn.from_id) {
                Some(id) => id.clone(),
                None => continue,
            };
            let to_eid = match node_entity_map.get(&conn.to_id) {
                Some(id) => id.clone(),
                None => continue,
            };

            let rel_type = cn_label_map
                .get(conn.line_type.as_str())
                .copied()
                .unwrap_or("related_to");
            let rel_id = format!(
                "canvas_rel:{}:{}:{}",
                canvas_path, conn.from_id, conn.to_id
            );

            db.execute(
                "INSERT OR IGNORE INTO cache_relations (id, from_id, to_id, relation_type, label, weight, bidirectional)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                rusqlite::params![rel_id, from_eid, to_eid, rel_type, conn.line_type, 0.5],
            )
            .map_err(|e| format!("创建 Canvas 关系失败: {e}"))?;
            if db.changes() > 0 {
                new_relations += 1;
            }
        }

        log::info!(
            "Canvas sync complete for {canvas_path}: {new_entities} entities, {new_relations} relations"
        );
        Ok((new_entities, new_relations))
    }

    // ── Operation Log ──

    fn log_operation(
        &self,
        operation: &str,
        entity_id: Option<&str>,
        entity_name: Option<&str>,
        details: Option<&str>,
    ) {
        if let Ok(db) = self.cache_db.lock() {
            let id = format!("op_{}", chrono::Utc::now().timestamp_millis());
            let timestamp = chrono::Utc::now().to_rfc3339();
            let _ = db.execute(
                "INSERT INTO cache_operations_log (id, operation, entity_id, entity_name, details, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![id, operation, entity_id, entity_name, details, timestamp],
            );
            // Cleanup old entries (>30 days)
            let cutoff = (chrono::Utc::now() - chrono::TimeDelta::days(30)).to_rfc3339();
            let _ = db.execute(
                "DELETE FROM cache_operations_log WHERE timestamp < ?1",
                rusqlite::params![cutoff],
            );
        }
    }

    pub async fn get_operation_log(
        &self,
        limit: Option<u32>,
    ) -> Result<Vec<crate::models::knowledge::OperationLogEntry>, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let limit = limit.unwrap_or(50).min(200);

        let mut stmt = db
            .prepare("SELECT id, operation, entity_id, entity_name, details, timestamp FROM cache_operations_log ORDER BY timestamp DESC LIMIT ?1")
            .map_err(|e| e.to_string())?;

        let entries: Vec<crate::models::knowledge::OperationLogEntry> = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(crate::models::knowledge::OperationLogEntry {
                    id: row.get(0)?,
                    operation: row.get(1)?,
                    entity_id: row.get(2)?,
                    entity_name: row.get(3)?,
                    details: row.get(4)?,
                    timestamp: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    // ── Daily Digest ──

    pub async fn get_daily_digest(&self) -> Result<crate::models::knowledge::DailyDigest, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let new_entities: u32 = db
            .query_row(
                "SELECT COUNT(*) FROM cache_entities WHERE date(created_at) = ?1",
                rusqlite::params![today],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let updated_entities: u32 = db
            .query_row(
                "SELECT COUNT(*) FROM cache_entities WHERE date(updated_at) = ?1 AND date(created_at) != ?1",
                rusqlite::params![today],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let deleted_entities: u32 = db
            .query_row(
                "SELECT COUNT(*) FROM cache_operations_log WHERE operation = 'delete_entity' AND date(timestamp) = ?1",
                rusqlite::params![today],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let new_relations: u32 = db
            .query_row(
                "SELECT COUNT(*) FROM cache_operations_log WHERE operation = 'add_relation' AND date(timestamp) = ?1",
                rusqlite::params![today],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Top operations today
        let top_operations: Vec<String> = {
            let mut stmt = db
                .prepare("SELECT operation, COUNT(*) as cnt FROM cache_operations_log WHERE date(timestamp) = ?1 GROUP BY operation ORDER BY cnt DESC LIMIT 5")
                .map_err(|e| e.to_string())?;
            let results: Vec<String> = stmt.query_map(rusqlite::params![today], |row| {
                let op: String = row.get(0)?;
                let cnt: u32 = row.get(1)?;
                Ok(format!("{op} ({cnt})"))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
            results
        };

        // Active namespaces
        let active_namespaces: Vec<String> = {
            let mut stmt = db
                .prepare("SELECT DISTINCT substr(source_file, 1, instr(source_file || '/', '/') - 1) as ns FROM cache_entities WHERE date(updated_at) = ?1 AND source_file IS NOT NULL AND source_file != '' LIMIT 10")
                .map_err(|e| e.to_string())?;
            let results: Vec<String> = stmt.query_map(rusqlite::params![today], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            results
        };

        Ok(crate::models::knowledge::DailyDigest {
            date: today,
            new_entities,
            updated_entities,
            deleted_entities,
            new_relations,
            top_operations,
            active_namespaces,
        })
    }

    // ── Backup ──

    pub fn backup_knowledge(&self, output_path: &str) -> Result<String, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let backup_path = if output_path.is_empty() {
            let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            format!("knowledge_backup_{ts}.db")
        } else {
            output_path.to_string()
        };

        // Use SQLite backup API
        let mut target = rusqlite::Connection::open(&backup_path)
            .map_err(|e| format!("无法创建备份数据库: {e}"))?;

        let backup = rusqlite::backup::Backup::new(&*db, &mut target)
            .map_err(|e| format!("备份初始化失败: {e}"))?;
        backup
            .run_to_completion(100, std::time::Duration::from_millis(0), None)
            .map_err(|e| format!("备份执行失败: {e}"))?;

        log::info!("Knowledge cache backed up to: {backup_path}");
        Ok(backup_path)
    }

    // ── Offline Sync Queue ──

    pub fn enqueue_pending_sync(&self, sync_type: &str, payload: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let id = format!("sync_{}", chrono::Utc::now().timestamp_millis());
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO cache_pending_sync (id, sync_type, payload, created_at, retries) VALUES (?1, ?2, ?3, ?4, 0)",
            rusqlite::params![id, sync_type, payload, now],
        )
        .map_err(|e| format!("入队待同步数据失败: {e}"))?;
        Ok(())
    }

    pub fn get_pending_syncs(
        &self,
        limit: Option<u32>,
    ) -> Result<Vec<crate::models::knowledge::PendingSyncItem>, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let limit = limit.unwrap_or(10);
        let mut stmt = db
            .prepare("SELECT id, sync_type, payload, retries FROM cache_pending_sync WHERE retries < 5 ORDER BY created_at ASC LIMIT ?1")
            .map_err(|e| e.to_string())?;
        let items: Vec<crate::models::knowledge::PendingSyncItem> = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(crate::models::knowledge::PendingSyncItem {
                    id: row.get(0)?,
                    sync_type: row.get(1)?,
                    payload: row.get(2)?,
                    retries: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(items)
    }

    pub fn mark_sync_complete(&self, sync_id: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "DELETE FROM cache_pending_sync WHERE id = ?1",
            rusqlite::params![sync_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn increment_sync_retry(&self, sync_id: &str, error: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "UPDATE cache_pending_sync SET retries = retries + 1, last_error = ?1 WHERE id = ?2",
            rusqlite::params![error, sync_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── Document graph (wiki doc ↔ entity connections) ──

    pub async fn get_document_graph(
        &self,
        file_path: &str,
    ) -> Result<GraphData, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let mut stmt = db
            .prepare("SELECT id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at, color, hidden FROM cache_entities WHERE source_file = ?1 AND hidden = 0")
            .map_err(|e| e.to_string())?;

        let entities: Vec<Entity> = stmt
            .query_map(rusqlite::params![file_path], |row| {
                let aliases_str: String = row.get(4).unwrap_or_default();
                let aliases: Vec<String> = serde_json::from_str(&aliases_str).unwrap_or_default();
                let props_str: String = row.get(5).unwrap_or_default();
                let properties: serde_json::Value = serde_json::from_str(&props_str).unwrap_or_default();
                Ok(Entity {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    entity_type: parse_entity_type(&row.get::<_, String>(2)?),
                    description: row.get(3)?,
                    aliases,
                    properties,
                    confidence: row.get(6)?,
                    source_file: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    color: row.get(10)?,
                    hidden: row.get::<_, i32>(11)? != 0,
                    ..Default::default()
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let entity_ids: std::collections::HashSet<String> =
            entities.iter().map(|e| e.id.clone()).collect();

        let mut relations = Vec::new();
        if !entity_ids.is_empty() {
            let placeholders = entity_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT id, from_id, to_id, relation_type, label, weight, bidirectional FROM cache_relations WHERE from_id IN ({0}) OR to_id IN ({0})",
                placeholders
            );

            let ids: Vec<&dyn rusqlite::types::ToSql> = entity_ids
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();

            let mut rel_stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
            relations = rel_stmt
                .query_map(ids.as_slice(), |row| {
                    Ok(Relation {
                        id: row.get(0)?,
                        from_id: row.get(1)?,
                        to_id: row.get(2)?,
                        relation_type: parse_relation_type(&row.get::<_, String>(3)?),
                        label: row.get(4)?,
                        weight: row.get(5)?,
                        bidirectional: row.get::<_, i32>(6)? != 0,
                    })
                })
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
        }

        Ok(GraphData {
            entities,
            relations,
            namespace: None,
            total_entity_count: 0,
            offline: false,
        })
    }

    /// Build file nodes from wiki .md files with wikilink edges and file→entity contains edges.
    pub fn build_file_nodes_and_edges(&self) -> Result<(Vec<Entity>, Vec<Relation>), String> {
        let mut md_files = std::collections::HashSet::new();
        self.collect_md_files(&self.wiki_dir, &mut md_files);

        let wiki_prefix = self.wiki_dir.to_string_lossy().replace('\\', "/");
        let mut file_entities = Vec::new();
        let mut file_relations = Vec::new();
        // Map filename (without .md) → file entity id
        let mut fname_to_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        for full_path in &md_files {
            let rel_path = full_path.strip_prefix(&wiki_prefix)
                .unwrap_or(full_path)
                .trim_start_matches('/');
            let file_name = std::path::Path::new(rel_path)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let file_id = uuid::Uuid::new_v5(&WIKI_NAMESPACE, format!("__file__{}", full_path).as_bytes()).to_string();
            let file_entity = Entity {
                id: file_id.clone(),
                name: file_name.clone(),
                entity_type: "__file__".to_string(),
                description: rel_path.to_string(),
                aliases: vec![],
                properties: serde_json::json!({"file_path": rel_path}),
                confidence: 1.0,
                source_file: Some(full_path.clone()),
                created_at: String::new(),
                updated_at: String::new(),
                color: Some("#9CA3AF".to_string()),
                hidden: false,
                ..Default::default()
            };
            fname_to_id.insert(file_name.to_lowercase(), file_id.clone());
            file_entities.push(file_entity);

            // Parse wikilinks from file content
            if let Ok(content) = std::fs::read_to_string(full_path) {
                let re = regex::Regex::new(r"\[\[([^\]|#]+)(?:[|#][^\]]+)?\]\]").unwrap();
                for cap in re.captures_iter(&content) {
                    let target_name = cap.get(1).unwrap().as_str().to_string();
                    let target_id = uuid::Uuid::new_v5(
                        &WIKI_NAMESPACE,
                        format!("__file__{}/{}", wiki_prefix, target_name).as_bytes(),
                    ).to_string();
                    let rel_id = uuid::Uuid::new_v4().to_string();
                    file_relations.push(Relation {
                        id: rel_id,
                        from_id: file_id.clone(),
                        to_id: target_id,
                        relation_type: "wikilink".to_string(),
                        label: Some("wikilink".to_string()),
                        weight: 1.0,
                        bidirectional: false,
                    });
                }
            }
        }

        // Create file→entity "contains" edges from source_file
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db
            .prepare("SELECT id, source_file FROM cache_entities WHERE source_file IS NOT NULL AND source_file != '' AND hidden = 0")
            .map_err(|e| e.to_string())?;
        let entity_file_pairs: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        for (entity_id, source_file) in &entity_file_pairs {
            let file_name = std::path::Path::new(source_file)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Some(file_id) = fname_to_id.get(&file_name.to_lowercase()) {
                let rel_id = uuid::Uuid::new_v4().to_string();
                file_relations.push(Relation {
                    id: rel_id,
                    from_id: file_id.clone(),
                    to_id: entity_id.clone(),
                    relation_type: "contains".to_string(),
                    label: Some("contains".to_string()),
                    weight: 1.0,
                    bidirectional: false,
                });
            }
        }

        Ok((file_entities, file_relations))
    }

    fn upsert_entity_cache(&self, entities: &[Entity]) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        for e in entities {
            let aliases = serde_json::to_string(&e.aliases).unwrap_or_default();
            let properties = serde_json::to_string(&e.properties).unwrap_or_default();
            let entity_type =
                serde_json::to_string(&e.entity_type).unwrap_or_else(|_| "\"concept\"".into());

            db.execute(
                "INSERT OR REPLACE INTO cache_entities
                 (id, name, entity_type, description, aliases, properties, confidence, source_file, created_at, updated_at, color, hidden)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    e.id,
                    e.name,
                    entity_type,
                    e.description,
                    aliases,
                    properties,
                    e.confidence,
                    e.source_file,
                    e.created_at,
                    e.updated_at,
                    e.color,
                    e.hidden as i32,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn upsert_relation_cache(&self, relations: &[Relation]) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        for r in relations {
            let rel_type =
                serde_json::to_string(&r.relation_type).unwrap_or_else(|_| "\"related_to\"".into());

            db.execute(
                "INSERT OR REPLACE INTO cache_relations
                 (id, from_id, to_id, relation_type, label, weight, bidirectional)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    r.id,
                    r.from_id,
                    r.to_id,
                    rel_type,
                    r.label,
                    r.weight,
                    r.bidirectional as i32,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }


    // ── Obsidian Export (P2.2) ──

    /// Export all knowledge entities and relations to Obsidian markdown files.
    ///
    /// Each entity gets its own `.md` file with YAML frontmatter containing
    /// `heimdall_id` and `auto-generated: true` tags for FileWatcher exclusion.
    /// Relations are rendered as `[[wikilinks]]` with `—relates_to→` annotations.
    pub fn export_to_obsidian(
        &self,
        vault_path: &str,
        namespace: Option<&str>,
    ) -> Result<u32, String> {
        let vault = std::path::Path::new(vault_path);
        if !vault.exists() {
            std::fs::create_dir_all(vault).map_err(|e| format!("Cannot create vault dir: {e}"))?;
        }

        let heimdall_dir = vault.join("heimdall");
        std::fs::create_dir_all(&heimdall_dir).map_err(|e| format!("Cannot create heimdall dir: {e}"))?;

        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Fetch entities
        let ns_filter = if namespace.is_some() { "AND namespace = ?1" } else { "" };
        let ns_param = namespace.unwrap_or("");

        let entity_sql = if namespace.is_some() {
            format!("SELECT * FROM cache_entities WHERE hidden = 0 {ns_filter}")
        } else {
            "SELECT * FROM cache_entities WHERE hidden = 0".to_string()
        };

        let mut stmt = db.prepare(&entity_sql).map_err(|e| e.to_string())?;
        let entities: Vec<crate::models::knowledge::Entity> = if namespace.is_some() {
            stmt.query_map(rusqlite::params![ns_param], |row| {
                Ok(crate::models::knowledge::Entity {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    entity_type: parse_entity_type(&row.get::<_, String>("entity_type")?),
                    description: row.get("description")?,
                    aliases: serde_json::from_str(&row.get::<_, String>("aliases")?).unwrap_or_default(),
                    properties: serde_json::from_str(&row.get::<_, String>("properties")?).unwrap_or_default(),
                    confidence: row.get("confidence")?,
                    source_file: row.get("source_file")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                    color: row.get("color")?,
                    hidden: row.get::<_, i32>("hidden")? != 0,
                    importance: row.get("importance").ok(),
                    importance_level: row.get("importance_level").ok(),
                    display_tier: None,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            stmt.query_map([], |row| {
                Ok(crate::models::knowledge::Entity {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    entity_type: parse_entity_type(&row.get::<_, String>("entity_type")?),
                    description: row.get("description")?,
                    aliases: serde_json::from_str(&row.get::<_, String>("aliases")?).unwrap_or_default(),
                    properties: serde_json::from_str(&row.get::<_, String>("properties")?).unwrap_or_default(),
                    confidence: row.get("confidence")?,
                    source_file: row.get("source_file")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                    color: row.get("color")?,
                    hidden: row.get::<_, i32>("hidden")? != 0,
                    importance: row.get("importance").ok(),
                    importance_level: row.get("importance_level").ok(),
                    display_tier: None,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect()
        };

        // Build relation lookup: entity_id → [(target_name, rel_type)]
        let mut outgoing: std::collections::HashMap<String, Vec<(String, String)>> = std::collections::HashMap::new();
        {
            let mut rel_stmt = db
                .prepare("SELECT from_id, to_id, relation_type FROM cache_relations")
                .map_err(|e| e.to_string())?;
            let _ = rel_stmt.query_map([], |row| {
                let from_id: String = row.get(0)?;
                let to_id: String = row.get(1)?;
                let rel_type: String = row.get(2)?;
                // We'll resolve names in a second pass
                outgoing.entry(from_id).or_default().push((to_id.clone(), rel_type.clone()));
                Ok(())
            });
        }

        // Build id → name map for wikilink resolution
        let id_to_name: std::collections::HashMap<String, String> = entities
            .iter()
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect();

        // Sanitize filename helper
        let safe_name = |name: &str| -> String {
            name.chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
                .collect::<String>()
                .trim()
                .replace("  ", " ")
                .trim()
                .to_string()
        };

        let mut exported: u32 = 0;
        let mut entity_index: Vec<String> = Vec::new();

        let timestamp = chrono::Utc::now().to_rfc3339();

        for entity in &entities {
            if entity.name.is_empty() {
                continue;
            }

            let filename = safe_name(&entity.name);
            if filename.is_empty() {
                continue;
            }

            // Build markdown content
            let mut md = String::new();

            // YAML frontmatter with dual-protection markers
            md.push_str("---\n");
            md.push_str(&format!("heimdall_id: \"{}\"\n", entity.id));
            md.push_str(&format!("entity_type: {:?}\n", entity.entity_type));
            md.push_str(&format!("confidence: {:.2}\n", entity.confidence));
            if let Some(ref imp) = entity.importance_level {
                md.push_str(&format!("importance: {}\n", imp));
            }
            md.push_str("tags:\n");
            md.push_str("  - auto-generated\n");
            md.push_str("  - knowledge-graph\n");
            md.push_str(&format!("created: {}\n", entity.created_at));
            md.push_str(&format!("updated: {}\n", entity.updated_at));
            md.push_str(&format!("exported_at: {}\n", timestamp));
            md.push_str("---\n\n");

            // Title
            md.push_str(&format!("# {}\n\n", entity.name));

            // Aliases
            if !entity.aliases.is_empty() {
                md.push_str("**别名**: ");
                let aliases: Vec<&str> = entity.aliases.iter().map(|a| a.as_str()).collect();
                md.push_str(&aliases.join("、"));
                md.push_str("\n\n");
            }

            // Description / properties
            if !entity.description.is_empty() {
                md.push_str(&format!("{}\n\n", entity.description));
            }
            if entity.properties.as_object().map_or(false, |o| !o.is_empty()) {
                if let Ok(props_str) = serde_json::to_string_pretty(&entity.properties) {
                    if props_str != "{}" {
                        md.push_str("## Properties\n\n");
                        md.push_str(&format!("```json\n{}\n```\n\n", props_str));
                    }
                }
            }

            // Outgoing relations as wikilinks
            if let Some(rels) = outgoing.get(&entity.id) {
                if !rels.is_empty() {
                    md.push_str("## Relations\n\n");
                    for (target_id, rel_type) in rels {
                        if let Some(target_name) = id_to_name.get(target_id) {
                            let arrow = match rel_type.as_str() {
                                "belongs_to" => "→ belongs to",
                                "contains" => "→ contains",
                                "relates_to" => "— relates to",
                                "contrasts_with" => "⇄ contrasts with",
                                "causes" => "→ causes",
                                "produces" => "→ produces",
                                "inspired_by" => "← inspired by",
                                "knows" => "— knows",
                                _ => "— relates to",
                            };
                            md.push_str(&format!("- [[{}]] {} [[{}]]\n", entity.name, arrow, target_name));
                        }
                    }
                    md.push('\n');
                }
            }

            // Footer with source info
            if let Some(ref src) = entity.source_file {
                if !src.is_empty() {
                    md.push_str(&format!("---\n*源文件: `{}`*\n", src));
                }
            }

            let file_path = heimdall_dir.join(format!("{}.md", filename));
            std::fs::write(&file_path, &md).map_err(|e| format!("Write error for {}: {e}", file_path.display()))?;

            entity_index.push(format!("- [[heimdall/{}|{}]] ({:?})", filename, entity.name, entity.entity_type));
            exported += 1;
        }

        // Write index file
        if !entity_index.is_empty() {
            let mut index_md = String::from("---\nheimdall_id: \"index\"\ntags:\n  - auto-generated\n  - knowledge-graph\nexported_at: ");
            index_md.push_str(&timestamp);
            index_md.push_str(&format!("\nentity_count: {}\n", exported));
            index_md.push_str("---\n\n# Knowledge Graph Index\n\n");
            index_md.push_str(&format!("*{} entities exported at {}*\n\n", exported, timestamp));
            for line in &entity_index {
                index_md.push_str(line);
                index_md.push('\n');
            }
            std::fs::write(heimdall_dir.join("_index.md"), &index_md)
                .map_err(|e| format!("Write index error: {e}"))?;
        }

        log::info!("Exported {exported} entities to Obsidian vault at {vault_path}");
        Ok(exported)
    }
}

// ==================================================================
// Phase 1-5: Heimdall Evolution API Methods
// ==================================================================

impl KnowledgeService {
    pub async fn run_inference(
        &self,
        namespace: Option<&str>,
        min_shared_neighbors: Option<u32>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({
            "namespace": namespace.unwrap_or("general"),
            "min_shared_neighbors": min_shared_neighbors.unwrap_or(2),
        });
        let url = format!("{}/api/inference/run", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_inferences(
        &self,
        namespace: Option<&str>,
        limit: Option<u32>,
        status: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/inference/candidates?namespace={}&limit={}&status={}",
            self.heimdall_url,
            namespace.unwrap_or("general"),
            limit.unwrap_or(10),
            status.unwrap_or("pending"),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let mut json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(json["inferences"].take())
    }

    pub async fn confirm_inference(&self, inference_id: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/inference/{}/confirm", self.heimdall_url, inference_id);
        let resp = self.client.post(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn reject_inference(&self, inference_id: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/inference/{}/reject", self.heimdall_url, inference_id);
        let resp = self.client.post(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn rebuild_causal(&self, namespace: Option<&str>) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({"namespace": namespace.unwrap_or("general")});
        let url = format!("{}/api/evolution/causal/rebuild", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_causal_chains(
        &self,
        namespace: Option<&str>,
        min_score: Option<f64>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/evolution/causal/chains?namespace={}&min_score={}",
            self.heimdall_url,
            namespace.unwrap_or("general"),
            min_score.unwrap_or(0.3),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let mut json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(json["chains"].take())
    }

    // ==================================================================
    // Phase 2: Conflict + Confidence
    // ==================================================================

    pub async fn check_conflict(
        &self,
        entity_id: &str,
        properties: &serde_json::Value,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({
            "entity_id": entity_id,
            "properties": properties,
            "namespace": namespace.unwrap_or("general"),
        });
        let url = format!("{}/api/evolution/conflicts/check", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_conflicts(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/evolution/conflicts?namespace={}",
            self.heimdall_url,
            namespace.unwrap_or("general"),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let mut json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(json["conflicts"].take())
    }

    pub async fn resolve_conflict(
        &self,
        history_id: &str,
        resolution: &str,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({"resolution": resolution});
        let url = format!("{}/api/evolution/conflicts/{}/resolve", self.heimdall_url, history_id);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn boost_confidence(
        &self,
        entity_id: &str,
        amount: Option<f64>,
        reason: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({
            "entity_id": entity_id,
            "amount": amount.unwrap_or(0.1),
            "reason": reason.unwrap_or("manual_confirm"),
        });
        let url = format!("{}/api/evolution/confidence/boost", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn decay_confidence(
        &self,
        entity_id: &str,
        amount: Option<f64>,
        reason: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({
            "entity_id": entity_id,
            "amount": amount.unwrap_or(0.05),
            "reason": reason.unwrap_or("marked_outdated"),
        });
        let url = format!("{}/api/evolution/confidence/decay", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn apply_time_decay(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({"namespace": namespace.unwrap_or("general")});
        let url = format!("{}/api/evolution/confidence/apply-time-decay", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    // ==================================================================
    // Phase 3: Hypothesis + Viewpoint
    // ==================================================================

    pub async fn get_hypothesis_gaps(
        &self,
        namespace: Option<&str>,
        entity_type: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let mut url = format!(
            "{}/api/hypothesis/gaps?namespace={}",
            self.heimdall_url,
            namespace.unwrap_or("general"),
        );
        if let Some(et) = entity_type {
            url.push_str(&format!("&entity_type={et}"));
        }
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let mut json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(json["gaps"].take())
    }

    pub async fn dismiss_hypothesis(&self, hypothesis_id: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/hypothesis/{}/dismiss", self.heimdall_url, hypothesis_id);
        let resp = self.client.post(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_viewpoint_evolution(
        &self,
        entity_id: &str,
    ) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/evolution/viewpoint/{entity_id}", self.heimdall_url);
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(match json.get("changes").and_then(|v| v.as_array()) {
            Some(arr) => serde_json::Value::Array(arr.clone()),
            None => serde_json::Value::Array(vec![]),
        })
    }

    pub async fn get_drifted_entities(
        &self,
        namespace: Option<&str>,
        min_changes: Option<u32>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/evolution/viewpoint/drifted?namespace={}&min_changes={}",
            self.heimdall_url,
            namespace.unwrap_or("general"),
            min_changes.unwrap_or(2),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let mut json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(json["entities"].take())
    }

    // ==================================================================
    // Phase 4: Synthesis + Obsolescence
    // ==================================================================

    pub async fn find_duplicates(
        &self,
        namespace: Option<&str>,
        threshold: Option<f64>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/synthesis/duplicates?namespace={}&threshold={}",
            self.heimdall_url,
            namespace.unwrap_or("general"),
            threshold.unwrap_or(0.8),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn merge_entities(
        &self,
        primary_id: &str,
        secondary_ids: &[String],
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({"primary_id": primary_id, "secondary_ids": secondary_ids});
        let url = format!("{}/api/synthesis/merge", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn mark_entity_outdated(
        &self,
        entity_id: &str,
        reason: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({"entity_id": entity_id, "reason": reason.unwrap_or("manual")});
        let url = format!("{}/api/evolution/obsolescence/mark-outdated", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn scan_stale(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({"namespace": namespace.unwrap_or("general")});
        let url = format!("{}/api/evolution/obsolescence/scan", self.heimdall_url);
        let resp = self.client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    // ==================================================================
    // Phase 5: Knowledge Tree + Namespace Config
    // ==================================================================

    pub async fn get_tree(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/tree?namespace={}",
            self.heimdall_url,
            namespace.unwrap_or("general"),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_subtree(
        &self,
        entity_id: &str,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/tree/{}/subtree?namespace={}",
            self.heimdall_url,
            entity_id,
            namespace.unwrap_or("general"),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_breadcrumb(
        &self,
        entity_id: &str,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/tree/{}/breadcrumb?namespace={}",
            self.heimdall_url,
            entity_id,
            namespace.unwrap_or("general"),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_leaves(
        &self,
        entity_id: &str,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/tree/{}/leaves?namespace={}",
            self.heimdall_url,
            entity_id,
            namespace.unwrap_or("general"),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn search_tree(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: Option<u32>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/api/tree/search?q={}&namespace={}&limit={}",
            self.heimdall_url,
            query,
            namespace.unwrap_or("general"),
            limit.unwrap_or(20),
        );
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_namespace_config(&self) -> Result<serde_json::Value, String> {
        let url = format!("{}/api/namespaces/config", self.heimdall_url);
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn update_namespace_config(
        &self,
        namespace_name: &str,
        description: Option<&str>,
        retriever_weights: Option<&serde_json::Value>,
        entity_type_weights: Option<&serde_json::Value>,
        context_injection: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let mut body = serde_json::json!({});
        if let Some(d) = description { body["description"] = serde_json::json!(d); }
        if let Some(r) = retriever_weights { body["retriever_weights"] = r.clone(); }
        if let Some(e) = entity_type_weights { body["entity_type_weights"] = e.clone(); }
        if let Some(c) = context_injection { body["context_injection"] = c.clone(); }
        let url = format!("{}/api/namespaces/config/{namespace_name}", self.heimdall_url);
        let resp = self.client.put(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn create_namespace_config(
        &self,
        namespace_name: &str,
        description: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({"description": description.unwrap_or("")});
        let url = format!("{}/api/namespaces/config/{namespace_name}", self.heimdall_url);
        let resp = self.client.put(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    // ── Nexus P5: Entity/Relation CRUD + Feedback + Merge ──

    /// Create a new entity in the knowledge base.
    pub fn nexus_create_entity(
        &self,
        id: &str,
        name: &str,
        entity_type: &str,
        description: &str,
        aliases: &str,
        properties: &str,
        now: &str,
    ) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO cache_entities (id, name, entity_type, description, aliases, properties, confidence, llm_confidence, source_count, source_type, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0.5, 0.5, 1, 'agent', ?7, ?7)",
            rusqlite::params![id, name, entity_type, description, aliases, properties, now],
        )
        .map_err(|e| format!("创建实体失败: {e}"))?;
        Ok(())
    }

    /// Update entity fields. Writes feedback record for audit trail.
    pub fn nexus_update_entity(
        &self,
        entity_id: &str,
        updates: &serde_json::Value,
    ) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        if let Some(name) = updates.get("name").and_then(|v| v.as_str()) {
            db.execute(
                "UPDATE cache_entities SET name = ?1 WHERE id = ?2",
                rusqlite::params![name, entity_id],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(tp) = updates.get("entity_type").and_then(|v| v.as_str()) {
            db.execute(
                "UPDATE cache_entities SET entity_type = ?1 WHERE id = ?2",
                rusqlite::params![tp, entity_id],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(ns) = updates.get("namespace").and_then(|v| v.as_str()) {
            db.execute(
                "UPDATE cache_entities SET namespace = ?1 WHERE id = ?2",
                rusqlite::params![ns, entity_id],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(desc) = updates.get("description").and_then(|v| v.as_str()) {
            db.execute(
                "UPDATE cache_entities SET description = ?1 WHERE id = ?2",
                rusqlite::params![desc, entity_id],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(conf) = updates.get("confidence").and_then(|v| v.as_f64()) {
            db.execute(
                "UPDATE cache_entities SET confidence = ?1, llm_confidence = MAX(llm_confidence, ?1) WHERE id = ?2",
                rusqlite::params![conf, entity_id],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(hidden) = updates.get("hidden").and_then(|v| v.as_bool()) {
            db.execute(
                "UPDATE cache_entities SET hidden = ?1 WHERE id = ?2",
                rusqlite::params![hidden as i32, entity_id],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Soft-delete entity (set hidden=true). Record feedback.
    pub fn nexus_delete_entity(&self, entity_id: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "UPDATE cache_entities SET hidden = 1 WHERE id = ?1",
            rusqlite::params![entity_id],
        ).map_err(|e| e.to_string())?;
        // Also hide all relations involving this entity
        db.execute(
            "UPDATE cache_relations SET hidden = 1 WHERE from_id = ?1 OR to_id = ?1",
            rusqlite::params![entity_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Add a new relation between two entities.
    pub fn nexus_add_relation(
        &self,
        from_id: &str,
        to_id: &str,
        relation_type: &str,
        label: Option<&str>,
        confidence: Option<f64>,
        namespace: Option<&str>,
    ) -> Result<String, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        let ns = namespace.unwrap_or("default");
        let conf = confidence.unwrap_or(1.0);
        // source_type='manual' marks user-created relations
        db.execute(
            "INSERT INTO cache_relations (id, from_id, to_id, relation_type, label, confidence, namespace, source_type, hidden, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'manual', 0, ?8, ?8)",
            rusqlite::params![id, from_id, to_id, relation_type, label.unwrap_or(""), conf, ns, chrono::Utc::now().to_rfc3339()],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Update a relation's type and label.
    pub fn nexus_update_relation(
        &self,
        relation_id: &str,
        relation_type: Option<&str>,
        label: Option<&str>,
        confidence: Option<f64>,
    ) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        if let Some(rt) = relation_type {
            db.execute(
                "UPDATE cache_relations SET relation_type = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![rt, chrono::Utc::now().to_rfc3339(), relation_id],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(lbl) = label {
            db.execute(
                "UPDATE cache_relations SET label = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![lbl, chrono::Utc::now().to_rfc3339(), relation_id],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(c) = confidence {
            db.execute(
                "UPDATE cache_relations SET confidence = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![c, chrono::Utc::now().to_rfc3339(), relation_id],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Soft-delete a relation.
    pub fn nexus_delete_relation(&self, relation_id: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "UPDATE cache_relations SET hidden = 1 WHERE id = ?1",
            rusqlite::params![relation_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Submit feedback for an entity extraction. Records to cache_extraction_feedback.
    pub fn nexus_submit_feedback(
        &self,
        entity_id: &str,
        action: &str, // hide, show, boost, delete, unlink
        reason: Option<&str>,
    ) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // Apply the action
        match action {
            "hide" => {
                db.execute("UPDATE cache_entities SET hidden = 1 WHERE id = ?1", rusqlite::params![entity_id]).ok();
            }
            "show" => {
                db.execute("UPDATE cache_entities SET hidden = 0 WHERE id = ?1", rusqlite::params![entity_id]).ok();
            }
            "boost" => {
                db.execute(
                    "UPDATE cache_entities SET confidence = MIN(1.0, confidence + 0.2), llm_confidence = MIN(1.0, llm_confidence + 0.2) WHERE id = ?1",
                    rusqlite::params![entity_id],
                ).ok();
            }
            "delete" => {
                db.execute("DELETE FROM cache_relations WHERE from_id = ?1 OR to_id = ?1", rusqlite::params![entity_id]).ok();
                db.execute("DELETE FROM cache_entities WHERE id = ?1", rusqlite::params![entity_id]).ok();
            }
            _ => {}
        }

        // Record feedback
        let entity_info: (String, String, String) = db.query_row(
            "SELECT name, COALESCE(source_type,'unknown'), COALESCE(entity_type,'concept') FROM cache_entities WHERE id = ?1",
            rusqlite::params![entity_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap_or_else(|_| (String::new(), "unknown".into(), "concept".into()));

        let score = match action {
            "boost" => 1.0,
            "delete" => -1.0,
            "show" => 0.5,
            _ => 0.0,
        };

        db.execute(
            "INSERT INTO cache_extraction_feedback (id, entity_id, entity_name, source_type, entity_type, action, score, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![id, entity_id, entity_info.0, entity_info.1, entity_info.2, action, score, reason.unwrap_or(""), now],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Get pending merge suggestions from cache_ontology.
    pub fn nexus_get_pending_merges(&self) -> Result<serde_json::Value, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db.prepare(
            "SELECT id, category, type_name, usage_count, canonical_suggestion, similar_types, status, last_analyzed
             FROM cache_ontology WHERE status = 'pending' ORDER BY last_analyzed DESC"
        ).map_err(|e| e.to_string())?;

        let rows: Vec<serde_json::Value> = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "category": row.get::<_, String>(1)?,
                "type_name": row.get::<_, String>(2)?,
                "usage_count": row.get::<_, i32>(3)?,
                "canonical_suggestion": row.get::<_, String>(4)?,
                "similar_types": row.get::<_, String>(5)?,
                "status": row.get::<_, String>(6)?,
                "last_analyzed": row.get::<_, Option<String>>(7)?,
            }))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        Ok(serde_json::json!(rows))
    }

    /// Confirm a merge suggestion: merge entity types or relation types.
    pub fn nexus_confirm_merge(&self, merge_id: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Load the suggestion
        let category: String = db.query_row(
            "SELECT category FROM cache_ontology WHERE id = ?1", rusqlite::params![merge_id], |row| row.get(0),
        ).map_err(|e| format!("merge not found: {e}"))?;

        let canonical: String = db.query_row(
            "SELECT canonical_suggestion FROM cache_ontology WHERE id = ?1", rusqlite::params![merge_id], |row| row.get(0),
        ).map_err(|e| format!("merge not found: {e}"))?;

        let similar_str: String = db.query_row(
            "SELECT similar_types FROM cache_ontology WHERE id = ?1", rusqlite::params![merge_id], |row| row.get(0),
        ).map_err(|e| format!("merge not found: {e}"))?;

        if category == "entity_type" {
            // Parse similar_types JSON array and merge each
            if let Ok(types) = serde_json::from_str::<Vec<String>>(&similar_str) {
                for t in &types {
                    if t != &canonical {
                        db.execute(
                            "UPDATE cache_entities SET entity_type = ?1 WHERE entity_type = ?2",
                            rusqlite::params![canonical, t],
                        ).ok();
                    }
                }
            }
        } else if category == "relation_type" {
            // Rename relation_type
            if let Ok(types) = serde_json::from_str::<Vec<String>>(&similar_str) {
                for t in &types {
                    if t != &canonical {
                        db.execute(
                            "UPDATE cache_relations SET relation_type = ?1 WHERE relation_type = ?2",
                            rusqlite::params![canonical, t],
                        ).ok();
                    }
                }
            }
        }

        // Mark as confirmed
        db.execute(
            "UPDATE cache_ontology SET status = 'confirmed' WHERE id = ?1",
            rusqlite::params![merge_id],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Dismiss a merge suggestion.
    pub fn nexus_ignore_merge(&self, merge_id: &str) -> Result<(), String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "UPDATE cache_ontology SET status = 'ignored' WHERE id = ?1",
            rusqlite::params![merge_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Batch operation on entities matching filter criteria.
    pub fn nexus_batch_operation(
        &self,
        action: &str,
        namespace: Option<&str>,
        source_type: Option<&str>,
        min_confidence: Option<f64>,
        entity_ids: Option<&[String]>,
    ) -> Result<u32, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let mut affected = 0u32;

        // Build the set of target entity IDs
        let targets: Vec<String> = if let Some(ids) = entity_ids {
            ids.to_vec()
        } else {
            let mut sql = "SELECT id FROM cache_entities WHERE 1=1".to_string();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ns) = namespace {
                sql.push_str(" AND namespace = ?");
                params.push(Box::new(ns.to_string()));
            }
            if let Some(st) = source_type {
                sql.push_str(" AND source_type = ?");
                params.push(Box::new(st.to_string()));
            }
            if let Some(mc) = min_confidence {
                sql.push_str(" AND confidence >= ?");
                params.push(Box::new(mc));
            }

            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
            let rows: Vec<String> = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        match action {
            "hide" => {
                for id in &targets {
                    db.execute("UPDATE cache_entities SET hidden = 1 WHERE id = ?1", rusqlite::params![id]).ok();
                    affected += 1;
                }
            }
            "show" => {
                for id in &targets {
                    db.execute("UPDATE cache_entities SET hidden = 0 WHERE id = ?1", rusqlite::params![id]).ok();
                    affected += 1;
                }
            }
            "delete" => {
                for id in &targets {
                    db.execute("DELETE FROM cache_relations WHERE from_id = ?1 OR to_id = ?1", rusqlite::params![id]).ok();
                    db.execute("DELETE FROM cache_entities WHERE id = ?1", rusqlite::params![id]).ok();
                    affected += 1;
                }
            }
            "change_namespace" => {
                if let Some(new_ns) = namespace {
                    for id in &targets {
                        db.execute(
                            "UPDATE cache_entities SET namespace = ?1 WHERE id = ?2",
                            rusqlite::params![new_ns, id],
                        ).ok();
                        affected += 1;
                    }
                }
            }
            _ => return Err(format!("Unknown batch action: {action}")),
        }

        Ok(affected)
    }

    /// Get feedback history for an entity.
    pub fn nexus_get_entity_feedback(&self, entity_id: &str) -> Result<serde_json::Value, String> {
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db.prepare(
            "SELECT id, entity_id, entity_name, source_type, action, reason, created_at
             FROM cache_extraction_feedback WHERE entity_id = ?1 ORDER BY created_at DESC"
        ).map_err(|e| e.to_string())?;

        let rows: Vec<serde_json::Value> = stmt.query_map(rusqlite::params![entity_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "entity_id": row.get::<_, String>(1)?,
                "entity_name": row.get::<_, String>(2)?,
                "source_type": row.get::<_, String>(3)?,
                "action": row.get::<_, String>(4)?,
                "reason": row.get::<_, String>(5)?,
                "created_at": row.get::<_, String>(6)?,
            }))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        Ok(serde_json::json!(rows))
    }
}

// ==================================================================
// Nexus Maintenance Methods
// ==================================================================

impl KnowledgeService {
    /// Run quality scoring on all entities (rule-based, no LLM).
    /// Grades: A (high), B (medium), C (low), D (very low).
    pub fn nexus_maintain_quality(&self) -> Result<crate::models::knowledge::MaintenanceReport, String> {
        use crate::models::knowledge::MaintenanceReport;
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let task_id = uuid::Uuid::new_v4().to_string();

        // Count quality tiers
        let total: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0", [], |r| r.get(0)
        ).unwrap_or(0);

        let high: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0 AND description != '' AND id IN (SELECT DISTINCT from_id FROM cache_relations UNION SELECT DISTINCT to_id FROM cache_relations)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        let medium: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0 AND description != '' AND id NOT IN (SELECT DISTINCT from_id FROM cache_relations UNION SELECT DISTINCT to_id FROM cache_relations)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        let low: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0 AND (description = '' OR description IS NULL) AND id IN (SELECT DISTINCT from_id FROM cache_relations UNION SELECT DISTINCT to_id FROM cache_relations)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        let very_low: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0 AND (description = '' OR description IS NULL) AND id NOT IN (SELECT DISTINCT from_id FROM cache_relations UNION SELECT DISTINCT to_id FROM cache_relations)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        // Hide D-quality pipeline entities (no desc, no relations, low confidence)
        let hidden = db.execute(
            "UPDATE cache_entities SET hidden = 1 WHERE hidden = 0 AND (description = '' OR description IS NULL) AND id NOT IN (SELECT DISTINCT from_id FROM cache_relations UNION SELECT DISTINCT to_id FROM cache_relations) AND confidence < 0.4",
            [],
        ).unwrap_or(0) as u32;

        let summary = format!(
            "质量评分完成: 总计{}, A级{}, B级{}, C级{}, D级{}, 已隐藏{}个低质量实体",
            total, high, medium, low, very_low, hidden
        );

        let _ = db.execute(
            "INSERT INTO cache_maintenance_log (id, task, started_at, completed_at, entities_scanned, entities_fixed, status, summary) VALUES (?1, 'quality', ?2, ?2, ?3, ?4, 'completed', ?5)",
            rusqlite::params![task_id, now, total, hidden, summary],
        );

        Ok(MaintenanceReport {
            task: "quality".into(),
            status: "completed".into(),
            started_at: now.clone(),
            completed_at: now,
            entities_scanned: total,
            entities_fixed: hidden,
            llm_calls: 0,
            tokens_used: 0,
            summary,
            details: vec![],
        })
    }

    /// Run orphan detection + stale detection (rule-based, no LLM).
    /// Orphans: 0 inbound + 0 outbound edges → archive (hidden=1).
    /// Stale: source file deleted or last_accessed > 90 days → mark stale.
    pub fn nexus_maintain_cleanup(&self) -> Result<crate::models::knowledge::MaintenanceReport, String> {
        use crate::models::knowledge::{MaintenanceReport, MaintenanceDetail};
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let task_id = uuid::Uuid::new_v4().to_string();

        // Find absolute orphans: 0 inbound + 0 outbound edges
        let mut stmt = db.prepare(
            "SELECT e.id, e.name FROM cache_entities e
             WHERE e.hidden = 0
               AND e.id NOT IN (SELECT DISTINCT from_id FROM cache_relations)
               AND e.id NOT IN (SELECT DISTINCT to_id FROM cache_relations)"
        ).map_err(|e| e.to_string())?;

        let orphans: Vec<(String, String)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        let mut orphan_count = 0u32;
        let mut details = Vec::new();
        for (eid, name) in &orphans {
            db.execute("UPDATE cache_entities SET hidden = 1 WHERE id = ?1", rusqlite::params![eid]).ok();
            details.push(MaintenanceDetail {
                entity_id: Some(eid.clone()),
                entity_name: name.clone(),
                action: "archive".into(),
                reason: "孤岛实体: 无入边无出边".into(),
            });
            orphan_count += 1;
        }

        // Stale detection: entities whose source_file doesn't exist on disk
        let mut stale_stmt = db.prepare(
            "SELECT id, name, source_file FROM cache_entities WHERE hidden = 0 AND source_file IS NOT NULL AND source_file != ''"
        ).map_err(|e| e.to_string())?;

        let mut stale_count = 0u32;
        let rows: Vec<(String, String, String)> = stale_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        for (eid, name, source_file) in &rows {
            let abs_path = self.wiki_dir.join(source_file);
            if !abs_path.exists() {
                db.execute(
                    "UPDATE cache_entities SET hidden = 1 WHERE id = ?1",
                    rusqlite::params![eid],
                ).ok();
                details.push(MaintenanceDetail {
                    entity_id: Some(eid.clone()),
                    entity_name: name.clone(),
                    action: "mark_stale".into(),
                    reason: format!("源文件已删除: {source_file}"),
                });
                stale_count += 1;
            }
        }

        let summary = format!(
            "清理完成: 归档{}个孤岛实体, 标记{}个过期实体",
            orphan_count, stale_count
        );

        let _ = db.execute(
            "INSERT INTO cache_maintenance_log (id, task, started_at, completed_at, entities_scanned, entities_fixed, status, summary) VALUES (?1, 'cleanup', ?2, ?2, ?3, ?4, 'completed', ?5)",
            rusqlite::params![task_id, now, orphans.len() as u32 + rows.len() as u32, orphan_count + stale_count, summary],
        );

        Ok(MaintenanceReport {
            task: "cleanup".into(),
            status: "completed".into(),
            started_at: now.clone(),
            completed_at: now,
            entities_scanned: orphans.len() as u32 + rows.len() as u32,
            entities_fixed: orphan_count + stale_count,
            llm_calls: 0,
            tokens_used: 0,
            summary,
            details,
        })
    }

    /// Fix heimdall_migrated entities with confidence=0.
    /// If they have a source_path pointing to an existing wiki file, mark for re-extraction.
    pub fn nexus_maintain_fix_migrated(&self) -> Result<crate::models::knowledge::MaintenanceReport, String> {
        use crate::models::knowledge::{MaintenanceReport, MaintenanceDetail};
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let task_id = uuid::Uuid::new_v4().to_string();

        let mut stmt = db.prepare(
            "SELECT id, name, source_file FROM cache_entities
             WHERE hidden = 0 AND confidence = 0 AND entity_type LIKE '%heimdall_migrated%'"
        ).map_err(|e| e.to_string())?;

        let candidates: Vec<(String, String, Option<String>)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        let total = candidates.len() as u32;
        let mut fixed = 0u32;
        let mut details = Vec::new();

        for (eid, name, source_file) in &candidates {
            if let Some(ref sf) = source_file {
                let abs_path = self.wiki_dir.join(sf);
                if abs_path.exists() {
                    // Trigger re-extraction
                    match self.nexus_extract_from_file(sf, None) {
                        Ok(_) => {
                            fixed += 1;
                            details.push(MaintenanceDetail {
                                entity_id: Some(eid.clone()),
                                entity_name: name.clone(),
                                action: "re_extract".into(),
                                reason: format!("从 {sf} 重新提取"),
                            });
                        }
                        Err(e) => {
                            details.push(MaintenanceDetail {
                                entity_id: Some(eid.clone()),
                                entity_name: name.clone(),
                                action: "re_extract_failed".into(),
                                reason: e,
                            });
                        }
                    }
                }
            }
        }

        let summary = format!("迁移修复: 扫描{}个, 重提取{}个", total, fixed);
        let _ = db.execute(
            "INSERT INTO cache_maintenance_log (id, task, started_at, completed_at, entities_scanned, entities_fixed, status, summary) VALUES (?1, 'fix_migrated', ?2, ?2, ?3, ?4, 'completed', ?5)",
            rusqlite::params![task_id, now, total, fixed, summary],
        );

        Ok(MaintenanceReport {
            task: "fix_migrated".into(),
            status: "completed".into(),
            started_at: now.clone(),
            completed_at: now,
            entities_scanned: total,
            entities_fixed: fixed,
            llm_calls: 0,
            tokens_used: 0,
            summary,
            details,
        })
    }

    /// Find duplicate candidate pairs using the funnel approach (SQL-based),
    /// then call maintain_dedup.py for LLM judgment.
    pub fn nexus_maintain_dedup(&self) -> Result<crate::models::knowledge::MaintenanceReport, String> {
        use crate::models::knowledge::{MaintenanceReport, MaintenanceDetail};
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let task_id = uuid::Uuid::new_v4().to_string();

        // Funnel filter 1: same or similar name (Levenshtein-like via LOWER equality or prefix)
        let mut stmt = db.prepare(
            "SELECT e1.id, e1.name, e1.entity_type, e1.description,
                    e2.id, e2.name, e2.entity_type, e2.description
             FROM cache_entities e1
             JOIN cache_entities e2 ON e1.rowid < e2.rowid
             WHERE e1.hidden = 0 AND e2.hidden = 0
               AND LOWER(e1.name) = LOWER(e2.name)
               AND e1.entity_type = e2.entity_type
             LIMIT 50"
        ).map_err(|e| e.to_string())?;

        let pairs: Vec<serde_json::Value> = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "entity_a": {
                    "id": row.get::<_, String>(0)?,
                    "name": row.get::<_, String>(1)?,
                    "entity_type": row.get::<_, String>(2)?,
                    "desc": row.get::<_, String>(3)?,
                },
                "entity_b": {
                    "id": row.get::<_, String>(4)?,
                    "name": row.get::<_, String>(5)?,
                    "entity_type": row.get::<_, String>(6)?,
                    "desc": row.get::<_, String>(7)?,
                },
            }))
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        let total_pairs = pairs.len() as u32;
        if total_pairs == 0 {
            return Ok(MaintenanceReport {
                task: "dedup".into(),
                status: "completed".into(),
                started_at: now.clone(),
                completed_at: now,
                entities_scanned: 0,
                entities_fixed: 0,
                llm_calls: 0,
                tokens_used: 0,
                summary: "未发现疑似重复实体".into(),
                details: vec![],
            });
        }

        // Spawn maintain_dedup.py for LLM judgment
        let json_input = serde_json::to_string(&pairs).unwrap_or_default();
        let env_vars = self.nexus_env_vars();
        let py_path = Self::python_script_path("maintain_dedup.py");

        let mut cmd = std::process::Command::new("python");
        cmd.arg(&py_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (k, v) in &env_vars {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| format!("启动 maintain_dedup.py 失败: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(json_input.as_bytes());
        }

        let output = child.wait_with_output().map_err(|e| format!("等待 dedup 脚本失败: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("去重脚本异常: {stderr}"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let judgments: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap_or_default();

        let mut merged = 0u32;
        let mut details = Vec::new();
        for j in &judgments {
            let idx = j["index"].as_u64().unwrap_or(0) as usize;
            if j["same"].as_bool().unwrap_or(false) && idx < pairs.len() {
                let a_id = pairs[idx]["entity_a"]["id"].as_str().unwrap_or("");
                let b_id = pairs[idx]["entity_b"]["id"].as_str().unwrap_or("");
                let a_name = pairs[idx]["entity_a"]["name"].as_str().unwrap_or("");
                let b_name = pairs[idx]["entity_b"]["name"].as_str().unwrap_or("");

                // Merge B into A: move B's relations to A, then hide B
                let _ = db.execute(
                    "UPDATE cache_relations SET from_id = ?1 WHERE from_id = ?2",
                    rusqlite::params![a_id, b_id],
                );
                let _ = db.execute(
                    "UPDATE cache_relations SET to_id = ?1 WHERE to_id = ?2",
                    rusqlite::params![a_id, b_id],
                );
                let _ = db.execute(
                    "UPDATE cache_entities SET hidden = 1 WHERE id = ?1",
                    rusqlite::params![b_id],
                );

                details.push(MaintenanceDetail {
                    entity_id: Some(b_id.to_string()),
                    entity_name: format!("{a_name} ← {b_name}"),
                    action: "merge".into(),
                    reason: "LLM判断为同一实体".into(),
                });
                merged += 1;
            }
        }

        let summary = format!("去重完成: {total_pairs}对候选, LLM确认{merged}对重复, 已合并");
        let _ = db.execute(
            "INSERT INTO cache_maintenance_log (id, task, started_at, completed_at, entities_scanned, entities_fixed, llm_calls, status, summary) VALUES (?1, 'dedup', ?2, ?2, ?3, ?4, ?5, 'completed', ?6)",
            rusqlite::params![task_id, now, total_pairs, merged, (total_pairs + 9) / 10, summary],
        );

        Ok(MaintenanceReport {
            task: "dedup".into(),
            status: "completed".into(),
            started_at: now.clone(),
            completed_at: now,
            entities_scanned: total_pairs * 2,
            entities_fixed: merged,
            llm_calls: (total_pairs + 9) / 10,
            tokens_used: 0,
            summary,
            details,
        })
    }

    /// Get current maintenance status summary.
    pub fn nexus_get_maintenance_status(&self) -> Result<crate::models::knowledge::MaintenanceStatus, String> {
        use crate::models::knowledge::{MaintenanceStatus, MaintenanceTaskSummary};
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let total: u32 = db.query_row("SELECT COUNT(*) FROM cache_entities WHERE hidden = 0", [], |r| r.get(0)).unwrap_or(0);

        let low_quality: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0 AND (description = '' OR description IS NULL) AND id NOT IN (SELECT DISTINCT from_id FROM cache_relations UNION SELECT DISTINCT to_id FROM cache_relations)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        let orphan: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities e WHERE e.hidden = 0 AND e.id NOT IN (SELECT DISTINCT from_id FROM cache_relations) AND e.id NOT IN (SELECT DISTINCT to_id FROM cache_relations)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        let stale: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0 AND source_file IS NOT NULL AND source_file != ''",
            [], |r| r.get::<_, u32>(0)
        ).unwrap_or(0);

        let dup_candidates: u32 = db.query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM cache_entities e1 JOIN cache_entities e2 ON e1.rowid < e2.rowid WHERE e1.hidden = 0 AND e2.hidden = 0 AND LOWER(e1.name) = LOWER(e2.name) AND e1.entity_type = e2.entity_type LIMIT 100)",
            [], |r| r.get(0)
        ).unwrap_or(0);

        let migration_fix: u32 = db.query_row(
            "SELECT COUNT(*) FROM cache_entities WHERE hidden = 0 AND confidence = 0 AND entity_type LIKE '%heimdall_migrated%'",
            [], |r| r.get(0)
        ).unwrap_or(0);

        // Recent tasks
        let mut stmt = db.prepare(
            "SELECT task, status, completed_at, entities_fixed, summary FROM cache_maintenance_log ORDER BY started_at DESC LIMIT 5"
        ).map_err(|e| e.to_string())?;

        let recent: Vec<MaintenanceTaskSummary> = stmt.query_map([], |row| {
            Ok(MaintenanceTaskSummary {
                task: row.get::<_, String>(0)?,
                status: row.get::<_, String>(1)?,
                completed_at: row.get::<_, Option<String>>(2)?,
                entities_fixed: row.get::<_, u32>(3)?,
                summary: row.get::<_, String>(4)?,
            })
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        let last = recent.first().and_then(|r| r.completed_at.clone());

        Ok(MaintenanceStatus {
            last_maintenance: last,
            total_entities: total,
            low_quality_count: low_quality,
            orphan_count: orphan,
            stale_count: stale,
            duplicate_candidates: dup_candidates,
            migration_needs_fix: migration_fix,
            recent_tasks: recent,
        })
    }

    /// Get the path to a Python script in the services directory.
    fn python_script_path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("services")
            .join(name)
    }

    // ── Layer 2: Classification Fix & Document Organizing ──

    /// Classify maintenance: (1) fix entity_type quoting, (2) batch-classify
    /// unorganized wiki documents via LLM and move them into typed folders.
    /// - `full_scan=true`: rescan all non-md files in the entire wiki tree.
    /// - `full_scan=false` (default): only scan root-level unclassified files.
    pub fn nexus_maintain_classify(&self, full_scan: bool) -> Result<crate::models::knowledge::MaintenanceReport, String> {
        use crate::models::knowledge::{MaintenanceReport, MaintenanceDetail};
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let task_id = uuid::Uuid::new_v4().to_string();

        let mut fixed = 0u32;
        let mut details = Vec::new();
        let mut llm_calls = 0u32;

        // Fix 1: Remove surrounding quotes from entity_type (rule, no LLM)
        let mut stmt = db.prepare(
            "SELECT id, name, entity_type FROM cache_entities WHERE entity_type LIKE '\"%' OR entity_type LIKE '%\"'"
        ).map_err(|e| e.to_string())?;

        let quoted: Vec<(String, String, String)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        }).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

        for (eid, name, etype) in &quoted {
            let cleaned = etype.trim_matches('"').to_string();
            if cleaned != *etype {
                db.execute("UPDATE cache_entities SET entity_type = ?1 WHERE id = ?2",
                    rusqlite::params![cleaned, eid]).ok();
                fixed += 1;
                details.push(MaintenanceDetail {
                    entity_id: Some(eid.clone()), entity_name: name.clone(),
                    action: "unquote_type".into(),
                    reason: format!("{etype} → {cleaned}"),
                });
            }
        }
        let quoted_count = quoted.len() as u32;
        // Explicitly drop statement and DB lock before LLM calls
        drop(stmt);
        drop(quoted);
        drop(db);
        let candidates = self.collect_unclassified_files(full_scan);
        let existing_dirs = self.list_wiki_top_dirs();

        let max_files = 20usize;
        let mut classified = 0u32;
        let mut failed = 0u32;

        for file_path in candidates.iter().take(max_files) {
            match self.classify_single_file(file_path, &existing_dirs) {
                Ok(result) => {
                    let folder = result.get("folder").and_then(|v| v.as_str()).unwrap_or("笔记");
                    let title = result.get("title").and_then(|v| v.as_str()).unwrap_or(file_path);
                    let tags: Vec<String> = result.get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                        .unwrap_or_default();
                    let new_path = result.get("file_path").and_then(|v| v.as_str()).unwrap_or(file_path);

                    classified += 1;
                    llm_calls += 1;
                    details.push(MaintenanceDetail {
                        entity_id: None,
                        entity_name: file_path.clone(),
                        action: "classify_doc".into(),
                        reason: format!("→ {folder}/{title} tags:{tags:?}"),
                    });
                    log::info!("[classify] {file_path} → {new_path} folder={folder}");
                }
                Err(e) => {
                    failed += 1;
                    details.push(MaintenanceDetail {
                        entity_id: None,
                        entity_name: file_path.clone(),
                        action: "classify_error".into(),
                        reason: e,
                    });
                }
            }
        }

        let total_scanned = quoted_count + candidates.len().min(max_files) as u32;
        let summary = format!(
            "分类维护: 修复{}个引号类型, 成功归类{}个文档, 失败{}个",
            fixed, classified, failed
        );

        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let _ = db.execute(
            "INSERT INTO cache_maintenance_log (id, task, started_at, completed_at, entities_scanned, entities_fixed, status, summary, llm_calls) VALUES (?1, 'classify', ?2, ?2, ?3, ?4, 'completed', ?5, ?6)",
            rusqlite::params![task_id, now, total_scanned, fixed + classified, summary, llm_calls],
        );

        Ok(MaintenanceReport {
            task: "classify".into(), status: "completed".into(),
            started_at: now.clone(), completed_at: now,
            entities_scanned: total_scanned,
            entities_fixed: fixed + classified,
            llm_calls, tokens_used: 0,
            summary, details,
        })
    }

    /// Collect non-md files that need classification.
    /// - `full_scan=true`: traverse entire wiki tree (depth 5).
    /// - `full_scan=false`: only root-level files (unclassified, no named folder).
    fn collect_unclassified_files(&self, full_scan: bool) -> Vec<String> {
        let mut files: Vec<String> = Vec::new();
        let skip_dirs: std::collections::HashSet<&str> =
            ["_auto", "_trash", ".git", "node_modules"].iter().cloned().collect();
        let max_depth = if full_scan { 5u32 } else { 1u32 };

        fn walk(
            base: &std::path::Path,
            current: &std::path::Path,
            files: &mut Vec<String>,
            skip_dirs: &std::collections::HashSet<&str>,
            depth: u32,
            max_depth: u32,
            full_scan: bool,
        ) {
            if depth > max_depth { return; }
            let entries = match std::fs::read_dir(current) {
                Ok(e) => e,
                Err(_) => return,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let fname = path.file_name().unwrap_or_default().to_string_lossy();
                if fname.starts_with('.') { continue; }

                if path.is_dir() {
                    if skip_dirs.contains(fname.as_ref()) { continue; }
                    walk(base, &path, files, skip_dirs, depth + 1, max_depth, full_scan);
                } else {
                    let ext = path.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if ext == "md" || ext == "canvas" { continue; }
                    let rel = path.strip_prefix(base)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    if full_scan {
                        // Full scan: include all non-md files in entire tree
                        files.push(rel);
                    } else {
                        // Incremental: only root-level unclassified files
                        let parent = path.parent()
                            .map(|p| p == base)
                            .unwrap_or(true);
                        if parent {
                            files.push(rel);
                        }
                    }
                }
            }
        }

        walk(&self.wiki_dir, &self.wiki_dir, &mut files, &skip_dirs, 0, max_depth, full_scan);
        files.sort();
        files
    }

    /// Move the companion .md file when a binary source file is relocated.
    /// For PDF/DOCX/PPTX/images: moves `{stem}.md` from old dir to new dir.
    /// For .md files with `source_file` frontmatter: moves the referenced source.
    fn move_companions(&self, old_abs: &std::path::Path, new_abs: &std::path::Path, ext: &str) {
        let binary_exts: std::collections::HashSet<&str> = [
            "pdf", "docx", "pptx", "xlsx",
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tiff",
            "txt", "csv", "json", "xml",
        ].iter().cloned().collect();

        if binary_exts.contains(ext) {
            // Moving a binary file → check for same-stem .md companion in source dir
            if let (Some(stem), Some(parent), Some(new_parent)) = (
                old_abs.file_stem().and_then(|s| s.to_str()),
                old_abs.parent(),
                new_abs.parent(),
            ) {
                let companion = parent.join(format!("{}.md", stem));
                if companion.exists() {
                    let new_stem = new_abs.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(stem);
                    let new_companion = new_parent.join(format!("{}.md", new_stem));
                    if new_companion != companion {
                        let _ = std::fs::create_dir_all(new_parent);
                        if let Err(e) = std::fs::rename(&companion, &new_companion) {
                            log::warn!(
                                "[classify] Failed to move companion MD {} → {}: {e}",
                                companion.display(), new_companion.display()
                            );
                        } else {
                            log::info!(
                                "[classify] Moved companion MD: {} → {}",
                                companion.display(), new_companion.display()
                            );
                        }
                    }
                }
            }
        }
    }

    /// Run a single file through the classify pipeline and move it.
    /// Public so the file-watcher can auto-classify newly added files.
    pub fn classify_single_file(
        &self,
        file_path: &str,
        existing_dirs: &[String],
    ) -> Result<serde_json::Value, String> {
        let full_path = self.wiki_dir.join(file_path);
        if !full_path.exists() {
            return Err(format!("File not found: {}", full_path.display()));
        }

        let ext = full_path.extension()
            .and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let file_name = full_path.file_name()
            .and_then(|n| n.to_str()).unwrap_or("document");

        let python = Self::find_python();
        let python_str = python.to_str().unwrap_or("python");

        // Step 1: Extract text
        let ft_json = self.run_file_tools(python_str, "extract", &full_path)?;
        let text = ft_json.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.is_empty() {
            return Err("无法提取文本".into());
        }

        // Step 2: Classify via LLM
        let classify_result = self.run_classify_service(
            python_str, text, &ext, file_name, existing_dirs,
        )?;

        let folder = classify_result.get("folder")
            .and_then(|v| v.as_str()).unwrap_or("笔记");
        let title = classify_result.get("title")
            .and_then(|v| v.as_str()).unwrap_or(file_name);
        let tags: Vec<String> = classify_result.get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        // Step 3: Ensure target folder exists (creates new types as needed)
        let target_dir = self.wiki_dir.join(folder);
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("创建目录失败: {e}"))?;

        // Step 4: Move file
        let new_name = full_path.file_name()
            .and_then(|n| n.to_str()).unwrap_or("untitled");
        let new_full_path = target_dir.join(new_name);
        let final_path = if new_full_path.exists() && new_full_path != full_path {
            let stem = full_path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
            let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
            target_dir.join(format!("{}_{}.{}", stem, ts, ext))
        } else {
            new_full_path
        };

        if final_path != full_path {
            std::fs::rename(&full_path, &final_path)
                .map_err(|e| format!("移动文件失败: {e}"))?;
            // Move companion .md (or source binary) alongside
            self.move_companions(&full_path, &final_path, &ext);
        }

        let new_relative = final_path.strip_prefix(&self.wiki_dir)
            .unwrap_or(&final_path)
            .to_str().unwrap_or(file_path)
            .replace('\\', "/");

        // Step 5: If md, add frontmatter
        if ext == "md" {
            if let Ok(content) = std::fs::read_to_string(&final_path) {
                let updated = self.update_or_add_frontmatter(&content, title, &tags);
                std::fs::write(&final_path, &updated).ok();
            }
        }

        Ok(serde_json::json!({
            "folder": folder,
            "title": title,
            "tags": tags,
            "file_path": new_relative,
        }))
    }

    // ── Layer 4: PageRank ──

    /// Compute PageRank scores for all entities and write to cache_entity_scores.
    pub fn nexus_run_pagerank(&self) -> Result<crate::models::knowledge::PageRankReport, String> {
        use crate::models::knowledge::{PageRankReport, PageRankEntry};
        use std::collections::HashMap;
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        // Build adjacency: for each edge from→to, from contributes to to
        let mut stmt = db.prepare("SELECT from_id, to_id FROM cache_relations").map_err(|e| e.to_string())?;
        let edges: Vec<(String, String)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

        // Get all entity IDs
        let mut estmt = db.prepare("SELECT id, name FROM cache_entities WHERE hidden = 0").map_err(|e| e.to_string())?;
        let entities: Vec<(String, String)> = estmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

        if entities.is_empty() {
            return Ok(PageRankReport { total_entities: 0, iterations: 0, converged: false, top_entities: vec![], core_count: 0 });
        }

        let n = entities.len() as f64;
        let d = 0.85; // damping factor
        let max_iter = 100;
        let tol = 1e-6;

        // Build out-degree map
        let mut out_deg: HashMap<String, u32> = HashMap::new();
        for (from, _) in &edges {
            *out_deg.entry(from.clone()).or_insert(0) += 1;
        }

        // Initialize PageRank
        let mut pr: HashMap<String, f64> = entities.iter().map(|(id, _)| (id.clone(), 1.0 / n)).collect();
        let mut new_pr: HashMap<String, f64> = HashMap::new();

        let mut iterations = 0u32;
        let mut converged = false;

        for iter in 0..max_iter {
            iterations = iter + 1;
            // Compute sum of PR/out_deg for dangling nodes
            let dangling_sum: f64 = entities.iter()
                .filter(|(id, _)| out_deg.get(id).unwrap_or(&0) == &0)
                .map(|(id, _)| pr.get(id).copied().unwrap_or(0.0))
                .sum();
            let base = (1.0 - d) / n + d * dangling_sum / n;

            for (eid, _) in &entities {
                new_pr.insert(eid.clone(), base);
            }

            // Add contributions from edges
            for (from, to) in &edges {
                let deg = out_deg.get(from).copied().unwrap_or(1) as f64;
                let contrib = d * pr.get(from).copied().unwrap_or(0.0) / deg;
                if let Some(v) = new_pr.get_mut(to) {
                    *v += contrib;
                }
            }

            // Check convergence
            let mut delta = 0.0f64;
            for (eid, _) in &entities {
                let old = pr.get(eid).copied().unwrap_or(0.0);
                let new = new_pr.get(eid).copied().unwrap_or(0.0);
                delta += (new - old).abs();
            }
            std::mem::swap(&mut pr, &mut new_pr);

            if delta < tol {
                converged = true;
                break;
            }
        }

        // Save scores
        let mean = pr.values().sum::<f64>() / pr.len().max(1) as f64;
        let mut top: Vec<PageRankEntry> = entities.iter()
            .filter_map(|(id, name)| {
                let score = pr.get(id).copied().unwrap_or(0.0);
                Some(PageRankEntry { entity_id: id.clone(), name: name.clone(), score })
            })
            .collect();
        top.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let core_count = top.iter().filter(|e| e.score > mean * 2.0).count() as u32;

        // Write scores to DB
        let now = chrono::Utc::now().to_rfc3339();
        for entry in &top {
            let _ = db.execute(
                "INSERT OR REPLACE INTO cache_entity_scores (entity_id, importance_score, updated_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![entry.entity_id, entry.score, now],
            );
        }

        let top10: Vec<PageRankEntry> = top.into_iter().take(10).collect();

        Ok(PageRankReport {
            total_entities: entities.len() as u32,
            iterations,
            converged,
            top_entities: top10,
            core_count,
        })
    }

    // ── Layer 4: Louvain Community Detection ──

    pub fn nexus_run_community(&self) -> Result<crate::models::knowledge::CommunityReport, String> {
        use crate::models::knowledge::CommunityReport;
        use std::collections::HashMap;
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let mut estmt = db.prepare("SELECT id FROM cache_entities WHERE hidden = 0").map_err(|e| e.to_string())?;
        let entity_ids: Vec<String> = estmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

        if entity_ids.is_empty() {
            return Ok(CommunityReport { communities: 0, modularity: 0.0, total_entities: 0, iterations: 0 });
        }

        // Build adjacency
        let mut stmt = db.prepare("SELECT from_id, to_id FROM cache_relations").map_err(|e| e.to_string())?;
        let edges: Vec<(String, String)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

        let idx: HashMap<String, usize> = entity_ids.iter().enumerate().map(|(i, id)| (id.clone(), i)).collect();
        let n = entity_ids.len();
        let m = edges.len() as f64;

        // Build adjacency list
        let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
        let mut weights: HashMap<(usize, usize), f64> = HashMap::new();
        for (from, to) in &edges {
            if let (Some(&fi), Some(&ti)) = (idx.get(from), idx.get(to)) {
                adj[fi].push(ti);
                adj[ti].push(fi);
                *weights.entry((fi.min(ti), fi.max(ti))).or_insert(0.0) += 1.0;
            }
        }

        // Node degrees (weighted)
        let mut degree: Vec<f64> = vec![0.0; n];
        for (&(u, v), &w) in &weights {
            degree[u] += w;
            degree[v] += w;
        }

        // Initialize each node in its own community
        let mut community: Vec<usize> = (0..n).collect();
        let mut comm_size: Vec<usize> = vec![1; n];
        let mut comm_degree: Vec<f64> = degree.clone();

        let max_iter = 20;
        let mut iterations = 0u32;

        for _ in 0..max_iter {
            iterations += 1;
            let mut moved = false;

            for u in 0..n {
                let old_comm = community[u];
                let mut best_comm = old_comm;
                let mut best_gain = 0.0f64;

                // Build neighbor community weights
                let mut neigh_comm: HashMap<usize, f64> = HashMap::new();
                for &v in &adj[u] {
                    let w = weights.get(&(u.min(v), u.max(v))).copied().unwrap_or(0.0);
                    *neigh_comm.entry(community[v]).or_insert(0.0) += w;
                }

                let deg_u = degree[u];
                let total_m = 2.0 * m;

                for (&c, &w_uc) in &neigh_comm {
                    if c == old_comm { continue; }
                    let gain = (w_uc - comm_degree[c] * deg_u / total_m) / m;
                    if gain > best_gain {
                        best_gain = gain;
                        best_comm = c;
                    }
                }

                if best_comm != old_comm {
                    community[u] = best_comm;
                    comm_size[old_comm] -= 1;
                    comm_size[best_comm] += 1;
                    comm_degree[old_comm] -= deg_u;
                    comm_degree[best_comm] += deg_u;
                    moved = true;
                }
            }

            if !moved { break; }
        }

        // Count communities
        let unique_comms: std::collections::HashSet<usize> = community.iter().cloned().collect();

        // Compute modularity
        let total_m = 2.0 * m;
        let mut q = 0.0f64;
        for u in 0..n {
            for &v in &adj[u] {
                if u < v && community[u] == community[v] {
                    let w = weights.get(&(u, v)).copied().unwrap_or(0.0);
                    q += w - degree[u] * degree[v] / total_m;
                }
            }
        }
        q /= m.max(1.0);

        // Write community IDs to entity scores
        let now = chrono::Utc::now().to_rfc3339();
        for (i, eid) in entity_ids.iter().enumerate() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO cache_entity_scores (entity_id, community_id, updated_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![eid, community[i] as u32, now],
            );
        }

        Ok(CommunityReport {
            communities: unique_comms.len() as u32,
            modularity: q,
            total_entities: n as u32,
            iterations,
        })
    }

    // ── Layer 4: Causal Chain Discovery ──

    pub fn nexus_discover_causal(&self, entity_id: &str) -> Result<crate::models::knowledge::CausalChainReport, String> {
        use crate::models::knowledge::{CausalChainReport, CausalStep};
        use std::collections::{HashSet, VecDeque};
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let entity_name: String = db.query_row(
            "SELECT name FROM cache_entities WHERE id = ?1",
            rusqlite::params![entity_id],
            |r| r.get(0),
        ).unwrap_or_else(|_| entity_id.to_string());

        fn bfs_causal(
            db: &rusqlite::Connection, start: &str, forward: bool, max_depth: u32,
        ) -> Vec<Vec<CausalStep>> {
            let causal_types: std::collections::HashSet<&str> = ["cause", "leads_to", "triggers", "results_in", "prevents", "enables", "causes"].iter().cloned().collect();
            let mut chains: Vec<Vec<CausalStep>> = Vec::new();
            let mut visited: HashSet<String> = HashSet::new();
            let mut queue: VecDeque<(String, u32, Vec<CausalStep>)> = VecDeque::new();
            visited.insert(start.to_string());
            queue.push_back((start.to_string(), 0, vec![]));

            while let Some((current, depth, path)) = queue.pop_front() {
                if depth >= max_depth { continue; }
                let query = if forward {
                    "SELECT r.to_id, e.name, r.relation_type FROM cache_relations r
                     JOIN cache_entities e ON e.id = r.to_id
                     WHERE r.from_id = ?1"
                } else {
                    "SELECT r.from_id, e.name, r.relation_type FROM cache_relations r
                     JOIN cache_entities e ON e.id = r.from_id
                     WHERE r.to_id = ?1"
                };
                if let Ok(mut stmt) = db.prepare(query) {
                    if let Ok(rows) = stmt.query_map(rusqlite::params![current], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                    }) {
                        for row in rows.flatten() {
                            let (next_id, next_name, rel_type) = row;
                            if causal_types.contains(rel_type.as_str()) && visited.insert(next_id.clone()) {
                                let mut new_path = path.clone();
                                new_path.push(CausalStep {
                                    entity_id: next_id.clone(), entity_name: next_name.clone(),
                                    relation_type: rel_type, depth: depth + 1,
                                });
                                chains.push(new_path.clone());
                                queue.push_back((next_id, depth + 1, new_path));
                            }
                        }
                    }
                }
            }
            chains
        }

        let forward = bfs_causal(&db, entity_id, true, 5);
        let backward = bfs_causal(&db, entity_id, false, 5);

        Ok(CausalChainReport {
            entity_id: entity_id.to_string(),
            entity_name,
            forward_chains: forward,
            backward_chains: backward,
        })
    }

    // ── Layer 5: Transitive Reasoning ──

    pub fn nexus_run_transitive(&self) -> Result<crate::models::knowledge::TransitiveReport, String> {
        use crate::models::knowledge::TransitiveReport;
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();

        // ── Discover all transitive relation types in the graph ──
        // Instead of hardcoding 4 types, scan what's actually used.
        let mut type_stmt = db.prepare(
            "SELECT DISTINCT relation_type FROM cache_relations"
        ).map_err(|e| e.to_string())?;
        let all_types: Vec<String> = type_stmt.query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Common transitive relation patterns — expand the hardcoded list
        // with these prefixes.  e.g. is_a, subclass_of, part_of, located_in,
        // belongs_to, contains, derived_from, depends_on, etc.
        let transitive_prefixes = [
            "is_a", "subclass", "part_of", "located", "belongs",
            "contains", "derived", "depends", "requires", "implies",
        ];
        let transitive_types: Vec<String> = all_types.into_iter().filter(|t| {
            transitive_prefixes.iter().any(|p| t.starts_with(p))
        }).collect();
        // Fallback to known types if the graph is empty or has no matches
        let transitive_types = if transitive_types.is_empty() {
            vec!["is_a".into(), "part_of".into(), "located_in".into(), "belongs_to".into()]
        } else {
            transitive_types
        };

        let mut total_scanned = 0u32;
        let mut total_inferred = 0u32;
        let mut skipped = 0u32;
        let mut iterations = 0u32;
        const MAX_ITERATIONS: u32 = 10;

        // ── Transaction wrapper ──
        db.execute("BEGIN", []).map_err(|e| e.to_string())?;

        // ── Iterative transitive closure until convergence ──
        loop {
            iterations += 1;
            let mut round_inferred = 0u32;

            for ttype in &transitive_types {
                // Find 2-hop paths: A → B → C where both edges share the same transitive type.
                // Include newly-inferred edges from previous iterations so multi-hop chains
                // (A → B → C → D) eventually close to A → D.
                let mut stmt = db.prepare(
                    "SELECT r1.from_id, r2.to_id, r1.relation_type, MIN(r1.weight, r2.weight)
                     FROM cache_relations r1
                     JOIN cache_relations r2 ON r1.to_id = r2.from_id AND r1.relation_type = r2.relation_type
                     WHERE r1.relation_type = ?1
                       AND r1.from_id != r2.to_id"
                ).map_err(|e| e.to_string())?;

                let candidates: Vec<(String, String, String, f64)> = stmt.query_map(
                    rusqlite::params![ttype],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, f64>(3)?))
                ).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

                total_scanned += candidates.len() as u32;

                for (from_id, to_id, rel_type, weight) in &candidates {
                    // Skip if any edge already exists (original or previously inferred)
                    let exists: bool = db.query_row(
                        "SELECT COUNT(*) > 0 FROM cache_relations WHERE from_id = ?1 AND to_id = ?2 AND relation_type = ?3",
                        rusqlite::params![from_id, to_id, rel_type],
                        |r| r.get(0),
                    ).unwrap_or(false);

                    if exists {
                        skipped += 1;
                        continue;
                    }

                    // Inferred edges get lower confidence — capped at 0.5
                    let confidence = (weight * 0.9).min(0.5);
                    let new_id = uuid::Uuid::new_v4().to_string();
                    let _ = db.execute(
                        "INSERT INTO cache_relations (id, from_id, to_id, relation_type, weight, bidirectional, inferred, source_type) VALUES (?1, ?2, ?3, ?4, ?5, 0, 1, 'inferred')",
                        rusqlite::params![new_id, from_id, to_id, rel_type, confidence],
                    );
                    total_inferred += 1;
                    round_inferred += 1;
                }
            }

            if round_inferred == 0 || iterations >= MAX_ITERATIONS {
                break;
            }
        }

        // ── Commit ──
        db.execute("COMMIT", []).map_err(|e| e.to_string())?;

        // Log to synthesis_log
        let task_id = uuid::Uuid::new_v4().to_string();
        let _ = db.execute(
            "INSERT INTO cache_synthesis_log (id, task, rule, started_at, completed_at, edges_created, entities_scanned, status) VALUES (?1, 'transitive', 'transitive_closure', ?2, ?2, ?3, ?4, 'completed')",
            rusqlite::params![task_id, now, total_inferred, total_scanned],
        );

        Ok(TransitiveReport { scanned: total_scanned, inferred: total_inferred, skipped_existing: skipped })
    }

    // ── Layer 5: Conflict Detection ──

    pub fn nexus_scan_conflicts(&self) -> Result<crate::models::knowledge::ConflictReport, String> {
        use crate::models::knowledge::{ConflictReport, ConflictEntry};
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let conflict_pairs = [("supports", "opposes"), ("agrees", "disagrees"), ("proves", "disproves")];
        let mut conflicts: Vec<ConflictEntry> = Vec::new();
        let mut total_pairs_scanned = 0u32;

        for (t1, t2) in &conflict_pairs {
            // Count potential conflict pairs for this relation pair
            let pair_count: u32 = db.query_row(
                "SELECT COUNT(*) FROM cache_relations r1
                 JOIN cache_relations r2 ON r1.from_id = r2.from_id AND r1.to_id = r2.to_id
                 WHERE r1.relation_type = ?1 AND r2.relation_type = ?2 AND r1.id < r2.id",
                rusqlite::params![t1, t2],
                |r| r.get(0),
            ).unwrap_or(0);
            total_pairs_scanned += pair_count;

            // Fetch up to 100 conflicts per pair for display
            let mut stmt = db.prepare(
                "SELECT e1.name, r1.relation_type, r2.relation_type, r1.to_id, e3.name
                 FROM cache_relations r1
                 JOIN cache_relations r2 ON r1.from_id = r2.from_id AND r1.to_id = r2.to_id
                 JOIN cache_entities e1 ON e1.id = r1.from_id
                 JOIN cache_entities e3 ON e3.id = r1.to_id
                 WHERE r1.relation_type = ?1 AND r2.relation_type = ?2
                   AND r1.id < r2.id
                 ORDER BY r1.from_id
                 LIMIT 100"
            ).map_err(|e| e.to_string())?;

            let rows: Vec<(String, String, String, String, String)> = stmt.query_map(
                rusqlite::params![t1, t2],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            ).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

            for (entity_name, ra, rb, target_id, target_name) in &rows {
                conflicts.push(ConflictEntry {
                    entity_a: entity_name.clone(),
                    entity_b: String::new(), // same entity holds both conflicting relations
                    relation_a: ra.clone(),
                    relation_b: rb.clone(),
                    target: format!("{target_name} ({target_id})"),
                });
            }
        }

        Ok(ConflictReport {
            scanned_pairs: total_pairs_scanned,
            conflicts_found: conflicts.len() as u32,
            conflicts,
        })
    }

    // ── Layer 5: Evolution Timeline ──

    pub fn nexus_get_evolution(&self, entity_id: &str) -> Result<crate::models::knowledge::EvolutionReport, String> {
        use crate::models::knowledge::EvolutionReport;
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;

        let entity_name: String = db.query_row(
            "SELECT name FROM cache_entities WHERE id = ?1", rusqlite::params![entity_id], |r| r.get(0)
        ).unwrap_or_default();

        // Get snapshots ordered by time
        let mut stmt = db.prepare(
            "SELECT desc, captured_at FROM cache_entity_snapshots WHERE entity_id = ?1 ORDER BY captured_at ASC"
        ).map_err(|e| e.to_string())?;

        let snapshots: Vec<(String, String)> = stmt.query_map(rusqlite::params![entity_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

        // If no snapshots, create one from current description
        let descs: Vec<String> = if snapshots.is_empty() {
            let desc: String = db.query_row(
                "SELECT description FROM cache_entities WHERE id = ?1", rusqlite::params![entity_id], |r| r.get(0)
            ).unwrap_or_default();
            if desc.is_empty() { vec![] } else { vec![desc] }
        } else {
            snapshots.into_iter().map(|(d, _)| d).collect()
        };

        // Build simple timeline summary
        let windows = descs.len() as u32;
        let summary = if windows == 0 {
            "暂无演化数据".to_string()
        } else if windows == 1 {
            format!("{}: 单一版本，未检测到变化", entity_name)
        } else {
            format!("{}: 共 {} 个时间窗口的描述变化", entity_name, windows)
        };

        Ok(EvolutionReport {
            entity_id: entity_id.to_string(),
            entity_name,
            time_windows: windows,
            summary,
        })
    }

    // ── Layer 6: Synthesis Edge Verification ──

    /// Verify low-confidence synthesis edges using LLM batch judgment.
    /// Spawns verify_edges.py for the LLM call.
    pub fn nexus_verify_synthesis(&self) -> Result<crate::models::knowledge::VerifyReport, String> {
        use crate::models::knowledge::VerifyReport;
        let db = self.cache_db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();

        let query = "SELECT r.id, r.from_id, e1.name, r.to_id, e2.name, r.relation_type, r.weight
             FROM cache_relations r
             JOIN cache_entities e1 ON e1.id = r.from_id
             JOIN cache_entities e2 ON e2.id = r.to_id
             WHERE r.weight < 0.4
             LIMIT 50";

        let mut stmt = db.prepare(query).map_err(|e| e.to_string())?;
        let candidates: Vec<(String, String, String, String, String, String, f64)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
        }).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();

        let total = candidates.len() as u32;
        if total == 0 {
            return Ok(VerifyReport { total_edges: 0, verified: 0, rejected: 0, batches: 0, llm_calls: 0 });
        }

        // Build JSON array of edge objects for verification
        let edges_json: Vec<serde_json::Value> = candidates.iter().map(|(_id, _fid, fname, _tid, tname, rtype, weight)| {
            serde_json::json!({
                "from": fname,
                "to": tname,
                "relation": rtype,
                "confidence": weight,
            })
        }).collect();

        let env_vars = self.nexus_env_vars();

        if !Self::has_api_key(&env_vars) {
            return Err("未配置 LLM API Key，无法验证合成边".into());
        }

        let py_path = Self::python_script_path("verify_edges.py");
        let json_input = serde_json::to_string(&edges_json).unwrap_or_default();

        let mut cmd = std::process::Command::new("python");
        cmd.arg(&py_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (k, v) in &env_vars {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| format!("启动验证脚本失败: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(json_input.as_bytes());
        }
        let output = child.wait_with_output().map_err(|e| format!("等待验证脚本失败: {e}"))?;

        let mut verified = 0u32;
        let mut rejected = 0u32;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(judgments) = serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                for j in &judgments {
                    if let Some(idx) = j["index"].as_u64() {
                        if idx < candidates.len() as u64 {
                            let (edge_id, _, _, _, _, _, weight) = &candidates[idx as usize];
                            if j["valid"].as_bool().unwrap_or(false) {
                                let new_w = (weight * 2.0).min(0.7);
                                let _ = db.execute("UPDATE cache_relations SET weight = ?1 WHERE id = ?2",
                                    rusqlite::params![new_w, edge_id]);
                                verified += 1;
                            } else {
                                let _ = db.execute("DELETE FROM cache_relations WHERE id = ?1",
                                    rusqlite::params![edge_id]);
                                rejected += 1;
                            }
                        }
                    }
                }
            }
        }

        let batches = (total + 9) / 10;
        let task_id = uuid::Uuid::new_v4().to_string();
        let _ = db.execute(
            "INSERT INTO cache_synthesis_log (id, task, rule, started_at, completed_at, edges_created, edges_verified, entities_scanned, llm_calls, status) VALUES (?1, 'verify', 'llm_verify', ?2, ?2, 0, ?3, ?4, ?5, 'completed')",
            rusqlite::params![task_id, now, verified, total, batches],
        );

        Ok(VerifyReport { total_edges: total, verified, rejected, batches, llm_calls: batches })
    }
}
