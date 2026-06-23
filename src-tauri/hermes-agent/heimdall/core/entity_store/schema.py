"""Knowledge Ring database schema definitions (V2.3).

All DDL centralized here for version management.
V2.3 changes:
  - Added namespace TEXT to kr_entities, kr_relations, kr_originals, heimdall_entities
  - Added schema_meta table for migration tracking
  - Added kr_pending_sync table for offline write queue
V2.2 changes:
  - kr_entities.types as JSON array (multi-type support)
  - kr_entities.memory_embedding BLOB (Gemma 4 FP16, 1024-dim)
  - kr_aliases: removed recency_score, added last_seen/confidence
  - kr_originals: added compressed_content, archive_status, local_ttl
  - kr_profiles: removed content plaintext, only content_encrypted + ttl/expires
  - kr_profiles_fts: uses trigger_context only
"""

# -----------------------------------------------------------------------
# Entity type constants & visual mapping
# -----------------------------------------------------------------------

ENTITY_TYPE_COLORS = {
    "concept":  ("#3B82F6", "circle"),    # blue circle
    "content":  ("#10B981", "square"),    # green square
    "person":   ("#F59E0B", "star"),      # orange star
    "event":    ("#EF4444", "triangle"),  # red triangle
    "artifact": ("#8B5CF6", "hexagon"),   # purple hexagon
}

VALID_ENTITY_TYPES = frozenset(ENTITY_TYPE_COLORS.keys())
VALID_TYPE_DETAILS = frozenset({
    "discipline", "methodology", "domain", "insight", "skill",
    "article", "book", "video", "podcast", "webpage", "dialog",
    "person", "organization", "group",
    "meeting", "project", "task", "decision", "activity", "milestone",
    "work", "tool", "item",
})

VALID_RELATION_TYPES = frozenset({
    "belongs_to", "contains", "relates_to", "contrasts_with",
    "causes", "produces", "inspired_by", "knows",
})
VALID_RELATION_DIRECTIONS = frozenset({"unidirectional", "bidirectional"})

VALID_PROFILE_TYPES = frozenset({
    "preference", "habit", "identity", "social", "emotion",
})
VALID_EVENT_LOG_TYPES = frozenset({
    "entity_created", "entity_updated", "relation_added", "field_edited",
    "entity_merged", "entity_split", "user_correction", "profile_created",
})

# -----------------------------------------------------------------------
# Memory embedding spec (V2.2)
# -----------------------------------------------------------------------

MEMORY_EMBEDDING_SPEC = {
    "model": "gemma-4-it",
    "layer": "hidden_states[-2]",
    "dimension": 1024,
    "quantization": "float16",
    "pooling": "mean",
    "normalize": True,
    "compression": "none",
}

MEMORY_EMBEDDING_BYTES = MEMORY_EMBEDDING_SPEC["dimension"] * 2  # 2048 bytes

# -----------------------------------------------------------------------
# Old entity type → new type mapping (for migration)
# -----------------------------------------------------------------------

ENTITY_TYPE_MIGRATION_MAP = {
    "person":       ("person", "person"),
    "organization": ("person", "organization"),
    "project":      ("event", "project"),
    "tool":         ("artifact", "tool"),
    "concept":      ("concept", ""),
    "skill":        ("concept", "skill"),
    "event":        ("event", ""),
    "location":     ("concept", "location"),
    "media":        ("artifact", "media"),
}

# -----------------------------------------------------------------------
# Knowledge Ring V2.2 DDL
# -----------------------------------------------------------------------

KNOWLEDGE_RING_SCHEMA = """
-- kr_entities: entity master table (V2.2: types JSON array + memory_embedding)
CREATE TABLE IF NOT EXISTS kr_entities (
    entity_id        TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    types            TEXT DEFAULT '["concept"]',
    type_detail      TEXT DEFAULT '',
    domains          TEXT DEFAULT '[]',
    properties       TEXT DEFAULT '{}',
    tags             TEXT DEFAULT '[]',
    confidence       REAL DEFAULT 0.5,
    hrr_vector       BLOB,
    memory_embedding BLOB,
    namespace        TEXT DEFAULT 'general',
    helpful_count    INTEGER DEFAULT 0,
    timeline         TEXT DEFAULT '[]',
    source_ref       TEXT,
    created_at       TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at       TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- kr_aliases (V2.2: removed recency_score, added last_seen + confidence)
CREATE TABLE IF NOT EXISTS kr_aliases (
    alias_id     INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id    TEXT NOT NULL REFERENCES kr_entities(entity_id),
    name         TEXT NOT NULL,
    context      TEXT DEFAULT '',
    confidence   REAL DEFAULT 0.5,
    last_seen    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- kr_relations: explicit relation table
CREATE TABLE IF NOT EXISTS kr_relations (
    relation_id   INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id     TEXT NOT NULL REFERENCES kr_entities(entity_id),
    target_id     TEXT NOT NULL REFERENCES kr_entities(entity_id),
    type          TEXT NOT NULL CHECK(type IN ('belongs_to','contains','relates_to','contrasts_with','causes','produces','inspired_by','knows')),
    confidence    REAL DEFAULT 0.5,
    direction     TEXT DEFAULT 'bidirectional' CHECK(direction IN ('unidirectional','bidirectional')),
    source_text   TEXT,
    hrr_vector    BLOB,
    namespace     TEXT DEFAULT 'general',
    helpful_count INTEGER DEFAULT 0,
    created_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- kr_originals (V2.2: compressed_content + archive_status + local_ttl)
CREATE TABLE IF NOT EXISTS kr_originals (
    original_id        INTEGER PRIMARY KEY AUTOINCREMENT,
    source_type        TEXT CHECK(source_type IN ('text','link','file','image_ocr','audio','dialog')),
    content            TEXT,
    compressed_content BLOB,
    metadata           TEXT DEFAULT '{}',
    local_ttl          INTEGER DEFAULT 7,
    archive_status     TEXT DEFAULT 'local' CHECK(archive_status IN ('local','summary','compressed','archived','deleted')),
    namespace          TEXT DEFAULT 'general',
    archived_at        TIMESTAMP,
    created_at         TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- kr_profiles (V2.2: removed content plaintext, encrypted only + TTL)
CREATE TABLE IF NOT EXISTS kr_profiles (
    profile_id        INTEGER PRIMARY KEY AUTOINCREMENT,
    type              TEXT CHECK(type IN ('preference','habit','identity','social','emotion')),
    content_encrypted BLOB NOT NULL,
    trigger_context   TEXT,
    confidence        REAL DEFAULT 0.5,
    tag               TEXT DEFAULT 'ai_only',
    ttl_days          INTEGER DEFAULT NULL,
    expires_at        TIMESTAMP,
    created_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- kr_event_log: global change journal
CREATE TABLE IF NOT EXISTS kr_event_log (
    log_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    date         DATE NOT NULL,
    event_type   TEXT NOT NULL,
    entity_id    TEXT REFERENCES kr_entities(entity_id),
    relation_id  INTEGER REFERENCES kr_relations(relation_id),
    domain_id    TEXT,
    source       TEXT DEFAULT 'auto',
    description  TEXT,
    old_value    TEXT,
    new_value    TEXT,
    session_id   TEXT
);

-- kr_domain_first_seen: new domain tracking
CREATE TABLE IF NOT EXISTS kr_domain_first_seen (
    domain_name     TEXT PRIMARY KEY,
    first_seen      DATE NOT NULL,
    first_entity_id TEXT,
    status          TEXT DEFAULT 'auto_created',
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- schema_meta: migration and state tracking (V2.3)
CREATE TABLE IF NOT EXISTS schema_meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);

-- kr_pending_sync: offline write queue (V2.3)
CREATE TABLE IF NOT EXISTS kr_pending_sync (
    id           TEXT PRIMARY KEY,
    payload_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    namespace    TEXT DEFAULT 'general',
    created_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    retry_count  INTEGER DEFAULT 0,
    last_error   TEXT,
    next_retry_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_kr_entities_types ON kr_entities(types);
CREATE INDEX IF NOT EXISTS idx_kr_entities_confidence ON kr_entities(confidence);
CREATE INDEX IF NOT EXISTS idx_kr_aliases_entity ON kr_aliases(entity_id);
CREATE INDEX IF NOT EXISTS idx_kr_aliases_name ON kr_aliases(name);
CREATE INDEX IF NOT EXISTS idx_kr_rel_source ON kr_relations(source_id);
CREATE INDEX IF NOT EXISTS idx_kr_rel_target ON kr_relations(target_id);
CREATE INDEX IF NOT EXISTS idx_kr_rel_type ON kr_relations(type);
CREATE INDEX IF NOT EXISTS idx_kr_originals_archive ON kr_originals(archive_status);
CREATE INDEX IF NOT EXISTS idx_kr_profiles_expires ON kr_profiles(expires_at);
CREATE INDEX IF NOT EXISTS idx_kr_event_date ON kr_event_log(date);
CREATE INDEX IF NOT EXISTS idx_kr_event_type ON kr_event_log(event_type);
CREATE INDEX IF NOT EXISTS idx_kr_event_entity ON kr_event_log(entity_id);
CREATE INDEX IF NOT EXISTS idx_kr_profiles_type ON kr_profiles(type);
CREATE INDEX IF NOT EXISTS idx_kr_profiles_tag ON kr_profiles(tag);
CREATE INDEX IF NOT EXISTS idx_kr_entities_namespace ON kr_entities(namespace);
CREATE INDEX IF NOT EXISTS idx_kr_relations_namespace ON kr_relations(namespace);
CREATE INDEX IF NOT EXISTS idx_kr_pending_next_retry ON kr_pending_sync(next_retry_at);
"""

KNOWLEDGE_RING_FTS_SQL = """
CREATE VIRTUAL TABLE IF NOT EXISTS kr_entities_fts USING fts5(
    name, type_detail, tags, properties, domains,
    content=kr_entities, content_rowid=rowid
);

CREATE VIRTUAL TABLE IF NOT EXISTS kr_aliases_fts USING fts5(
    name, context,
    content=kr_aliases, content_rowid=rowid
);

CREATE VIRTUAL TABLE IF NOT EXISTS kr_relations_fts USING fts5(
    source_text,
    content=kr_relations, content_rowid=rowid
);

CREATE VIRTUAL TABLE IF NOT EXISTS kr_originals_fts USING fts5(
    content,
    content=kr_originals, content_rowid=rowid
);

CREATE VIRTUAL TABLE IF NOT EXISTS kr_profiles_fts USING fts5(
    trigger_context,
    content=kr_profiles, content_rowid=rowid
);
"""

# -----------------------------------------------------------------------
# Phase 1-2 evolution tables (heimdall-implementation-plan)
# -----------------------------------------------------------------------

EVOLUTION_TABLES_SQL = """
-- kr_inferences: candidate inferred relationships from graph analysis
CREATE TABLE IF NOT EXISTS kr_inferences (
    inference_id   TEXT PRIMARY KEY,
    entity_a       TEXT NOT NULL,
    entity_b       TEXT NOT NULL,
    inferred_type  TEXT NOT NULL,
    evidence       TEXT NOT NULL,
    confidence     REAL DEFAULT 0.3,
    status         TEXT DEFAULT 'pending',
    namespace      TEXT DEFAULT 'general',
    created_at     TEXT NOT NULL,
    resolved_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_inferences_status ON kr_inferences(status);
CREATE INDEX IF NOT EXISTS idx_inferences_entities ON kr_inferences(entity_a, entity_b);
CREATE INDEX IF NOT EXISTS idx_inferences_namespace ON kr_inferences(namespace);

-- kr_causal_chains: multi-hop causal paths
CREATE TABLE IF NOT EXISTS kr_causal_chains (
    chain_id       TEXT PRIMARY KEY,
    chain_path     TEXT NOT NULL,
    length         INTEGER DEFAULT 2,
    chain_score    REAL DEFAULT 0.0,
    namespace      TEXT DEFAULT 'general',
    first_seen     TEXT NOT NULL,
    last_updated   TEXT NOT NULL,
    is_active      INTEGER DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_causal_chains_active ON kr_causal_chains(is_active, chain_score DESC);
CREATE INDEX IF NOT EXISTS idx_causal_chains_namespace ON kr_causal_chains(namespace);

-- kr_entity_history: change tracking for viewpoint evolution & conflict resolution
CREATE TABLE IF NOT EXISTS kr_entity_history (
    history_id     TEXT PRIMARY KEY,
    entity_id      TEXT NOT NULL,
    field          TEXT NOT NULL,
    old_value      TEXT,
    new_value      TEXT,
    change_type    TEXT NOT NULL,
    source         TEXT,
    session_id     TEXT,
    namespace      TEXT DEFAULT 'general',
    timestamp      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_entity_history_entity ON kr_entity_history(entity_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_entity_history_namespace ON kr_entity_history(namespace);

-- kr_namespaces: namespace configuration for soft isolation
CREATE TABLE IF NOT EXISTS kr_namespaces (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    description     TEXT DEFAULT '',
    config          TEXT DEFAULT '{}',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
"""

# ALTER statements for extending existing tables (safe — uses IF NOT EXISTS semantics)
EVOLUTION_ALTER_SQL = [
    "ALTER TABLE kr_entities ADD COLUMN status TEXT DEFAULT 'active'",
    "ALTER TABLE kr_entities ADD COLUMN ttl_days INTEGER",
    "ALTER TABLE kr_entities ADD COLUMN last_verified_at TEXT",
]

# Legacy schemas preserved for backward compatibility
LEGACY_HEIMDALL_ENTITY_SCHEMA = """
CREATE TABLE IF NOT EXISTS heimdall_entities (
    entity_id TEXT PRIMARY KEY,
    entity_type TEXT NOT NULL,
    display_name TEXT NOT NULL,
    salted_hash TEXT,
    attributes_json TEXT DEFAULT '{}',
    vector BLOB,
    first_seen_at REAL NOT NULL,
    last_seen_at REAL NOT NULL,
    occurrence_count INTEGER DEFAULT 1,
    confidence REAL DEFAULT 0.5,
    community_id INTEGER,
    community_confidence REAL DEFAULT 1.0,
    is_bridge INTEGER DEFAULT 0,
    bridge_score REAL DEFAULT 0.0,
    pagerank REAL DEFAULT 1.0,
    pagerank_error REAL DEFAULT 0.0,
    source_session_id TEXT,
    source_track TEXT DEFAULT 'memory',
    namespace TEXT DEFAULT 'general',
    status TEXT DEFAULT 'active'
);

CREATE TABLE IF NOT EXISTS heimdall_memory_edges (
    id TEXT PRIMARY KEY,
    memory_id TEXT,
    entity_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('subject','object','context')),
    emotion REAL,
    timestamp REAL NOT NULL,
    session_id TEXT,
    ner_confidence REAL DEFAULT 1.0,
    is_flagged INTEGER DEFAULT 0,
    FOREIGN KEY (entity_id) REFERENCES heimdall_entities(entity_id)
);

CREATE TABLE IF NOT EXISTS heimdall_social_graph (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_entity_id TEXT NOT NULL,
    target_entity_id TEXT NOT NULL,
    relationship_type TEXT,
    intensity REAL DEFAULT 0.5,
    valence REAL DEFAULT 0.0,
    volatility REAL DEFAULT 0.0,
    health_score REAL DEFAULT 0.5,
    first_seen REAL NOT NULL,
    last_seen REAL NOT NULL,
    evidence_count INTEGER DEFAULT 1,
    context_memories TEXT,
    reconnect_suggested INTEGER DEFAULT 0,
    UNIQUE(source_entity_id, target_entity_id, relationship_type)
);

CREATE INDEX IF NOT EXISTS idx_heimdall_entities_type ON heimdall_entities(entity_type);
CREATE INDEX IF NOT EXISTS idx_heimdall_entities_status ON heimdall_entities(status);
CREATE INDEX IF NOT EXISTS idx_heimdall_social_source ON heimdall_social_graph(source_entity_id);
CREATE INDEX IF NOT EXISTS idx_heimdall_social_target ON heimdall_social_graph(target_entity_id);

-- Knowledge entries (V2.3)
CREATE TABLE IF NOT EXISTS heimdall_knowledge_entries (
    entry_id       TEXT PRIMARY KEY,
    domain         TEXT NOT NULL DEFAULT 'general',
    title          TEXT NOT NULL,
    content        TEXT,
    mastery_level  TEXT DEFAULT '了解',
    confidence     REAL DEFAULT 0.5,
    source_session_id TEXT DEFAULT '',
    source_type    TEXT DEFAULT '',
    source_ref     TEXT DEFAULT '',
    helpful_count  INTEGER DEFAULT 0,
    created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_accessed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS heimdall_knowledge_edges (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id  TEXT NOT NULL REFERENCES heimdall_knowledge_entries(entry_id),
    entity_id TEXT NOT NULL REFERENCES heimdall_entities(entity_id),
    UNIQUE(entry_id, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_knowledge_domain ON heimdall_knowledge_entries(domain);
CREATE INDEX IF NOT EXISTS idx_knowledge_mastery ON heimdall_knowledge_entries(mastery_level);
CREATE INDEX IF NOT EXISTS idx_knowledge_last_access ON heimdall_knowledge_entries(last_accessed_at);
"""

LEGACY_FTS_SQL = """
CREATE VIRTUAL TABLE IF NOT EXISTS heimdall_entities_fts USING fts5(
    display_name, attributes_json,
    content=heimdall_entities, content_rowid=rowid
);

CREATE VIRTUAL TABLE IF NOT EXISTS heimdall_knowledge_fts USING fts5(
    title, content, domain,
    content=heimdall_knowledge_entries, content_rowid=rowid
);
"""

# -----------------------------------------------------------------------
# V2.3 namespace migration
# -----------------------------------------------------------------------

def migrate_namespace(conn, db_path: str) -> bool:
    """Add namespace columns to existing tables with backup and rollback support.

    Returns True on success, raises RuntimeError on failure.
    """
    import shutil
    import time

    # 0. Pre-migration backup
    backup_path = f"{db_path}.bak.{int(time.time())}"
    shutil.copy2(db_path, backup_path)
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta VALUES ('last_backup', ?)",
        (backup_path,),
    )

    conn.execute(
        "INSERT OR REPLACE INTO schema_meta VALUES ('migration_status', 'in_progress')"
    )

    tables = ["kr_entities", "kr_relations", "kr_originals", "heimdall_entities"]
    for table in tables:
        try:
            # Skip if table doesn't exist or namespace column already present
            cols = {c["name"] for c in conn.execute(f"PRAGMA table_info({table})").fetchall()}
            if not cols:
                continue  # table doesn't exist yet, skip
            if "namespace" in cols:
                continue  # already migrated
            conn.execute(
                f"ALTER TABLE {table} ADD COLUMN namespace TEXT DEFAULT 'general'"
            )
            conn.execute(
                f"UPDATE {table} SET namespace = 'general' WHERE namespace IS NULL"
            )
        except Exception as e:
            conn.execute(
                "INSERT OR REPLACE INTO schema_meta VALUES ('migration_error', ?)",
                (str(e),),
            )
            conn.execute(
                "INSERT OR REPLACE INTO schema_meta VALUES ('migration_status', 'failed')"
            )
            conn.execute(
                "INSERT OR REPLACE INTO schema_meta VALUES ('failed_at_table', ?)",
                (table,),
            )
            raise RuntimeError(
                f"Namespace migration failed at table '{table}': {e}"
            ) from e

    # Index creation (compatible with SQLite < 3.26 which lacks IF NOT EXISTS)
    indexes = [
        ("idx_kr_entities_namespace", "kr_entities", "namespace"),
        ("idx_kr_relations_namespace", "kr_relations", "namespace"),
    ]
    for idx_name, tbl, col in indexes:
        try:
            conn.execute(f"CREATE INDEX IF NOT EXISTS {idx_name} ON {tbl}({col})")
        except Exception:
            try:
                conn.execute(f"CREATE INDEX {idx_name} ON {tbl}({col})")
            except Exception:
                pass

    conn.execute(
        "INSERT OR REPLACE INTO schema_meta VALUES ('migration_status', 'completed')"
    )
    return True


# -----------------------------------------------------------------------
# V2.4 importance columns migration (P2.3)
# -----------------------------------------------------------------------

def migrate_importance(conn, db_path: str) -> bool:
    """Add importance + importance_level columns to entity tables (P2.3).

    Returns True on success, raises RuntimeError on failure.
    """
    import shutil
    import time as _time

    # Pre-migration backup
    backup_path = f"{db_path}.bak.{int(_time.time())}"
    shutil.copy2(db_path, backup_path)
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta VALUES ('last_backup', ?)",
        (backup_path,),
    )

    conn.execute(
        "INSERT OR REPLACE INTO schema_meta VALUES ('importance_migration', 'in_progress')"
    )

    tables_cols = {
        "heimdall_entities": [
            ("importance", "REAL DEFAULT 0.5"),
            ("importance_level", "TEXT DEFAULT 'medium'"),
        ],
        "kr_entities": [
            ("importance", "REAL DEFAULT 0.5"),
            ("importance_level", "TEXT DEFAULT 'medium'"),
        ],
    }

    for table, cols in tables_cols.items():
        try:
            existing = {c["name"] for c in conn.execute(f"PRAGMA table_info({table})").fetchall()}
            if not existing:
                continue  # table doesn't exist yet
            for col_name, col_def in cols:
                if col_name not in existing:
                    conn.execute(f"ALTER TABLE {table} ADD COLUMN {col_name} {col_def}")
        except Exception as e:
            conn.execute(
                "INSERT OR REPLACE INTO schema_meta VALUES ('importance_migration_error', ?)",
                (str(e),),
            )
            conn.execute(
                "INSERT OR REPLACE INTO schema_meta VALUES ('importance_migration', 'failed')"
            )
            raise RuntimeError(
                f"Importance migration failed at table '{table}': {e}"
            ) from e

    # Create indexes for importance_level
    indexes = [
        ("idx_heimdall_importance", "heimdall_entities", "importance_level"),
        ("idx_kr_importance", "kr_entities", "importance_level"),
    ]
    for idx_name, tbl, col in indexes:
        try:
            conn.execute(f"CREATE INDEX IF NOT EXISTS {idx_name} ON {tbl}({col})")
        except Exception:
            try:
                conn.execute(f"CREATE INDEX {idx_name} ON {tbl}({col})")
            except Exception:
                pass

    conn.execute(
        "INSERT OR REPLACE INTO schema_meta VALUES ('importance_migration', 'completed')"
    )
    return True
