-- Migration 001: Initial schema (baseline)
-- This captures the existing schema from init_cache_tables() as of pre-Nexus.

CREATE TABLE IF NOT EXISTS cache_entities (
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_relations_triple
    ON cache_relations(from_id, to_id, relation_type);
