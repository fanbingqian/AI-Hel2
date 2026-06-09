-- Migration 002: Nexus Knowledge Engine schema
-- Adds source tracking, LLM confidence, content indexing, synthesis, and feedback tables.

-- 8.1 New columns on cache_entities
ALTER TABLE cache_entities ADD COLUMN source_type TEXT DEFAULT 'unknown';
ALTER TABLE cache_entities ADD COLUMN llm_confidence REAL;
ALTER TABLE cache_entities ADD COLUMN source_count INTEGER DEFAULT 1;
ALTER TABLE cache_entities ADD COLUMN namespace TEXT DEFAULT '未分类';
ALTER TABLE cache_entities ADD COLUMN content_hash TEXT;
ALTER TABLE cache_entities ADD COLUMN feedback_score REAL DEFAULT 0.0;

-- 8.4 New tables

CREATE TABLE IF NOT EXISTS cache_synthesis (
    id TEXT PRIMARY KEY,
    entity_a_id TEXT NOT NULL,
    entity_b_id TEXT NOT NULL,
    method TEXT NOT NULL,
    inferred_relation_type TEXT NOT NULL DEFAULT 'related_to',
    confidence REAL NOT NULL DEFAULT 0.25,
    reasoning TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (entity_a_id) REFERENCES cache_entities(id),
    FOREIGN KEY (entity_b_id) REFERENCES cache_entities(id)
);

CREATE TABLE IF NOT EXISTS cache_ontology (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    type_name TEXT NOT NULL,
    usage_count INTEGER DEFAULT 1,
    canonical_suggestion TEXT,
    similar_types TEXT,
    status TEXT DEFAULT 'pending',
    last_analyzed TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cache_content_index (
    source_path TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    extracted_at TEXT,
    entity_count INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS cache_extraction_feedback (
    id TEXT PRIMARY KEY,
    entity_id TEXT,
    entity_name TEXT,
    action TEXT NOT NULL,
    score REAL NOT NULL,
    source_type TEXT,
    entity_type TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_feedback_created ON cache_extraction_feedback(created_at);

CREATE TABLE IF NOT EXISTS cache_pending_merge (
    id TEXT PRIMARY KEY,
    entity_a_id TEXT NOT NULL,
    entity_b_id TEXT NOT NULL,
    entity_a_name TEXT NOT NULL,
    entity_b_name TEXT NOT NULL,
    similarity REAL NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at TEXT NOT NULL
);
