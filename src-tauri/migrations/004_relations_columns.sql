-- Migration 004: Add columns to cache_relations for Nexus P5 editor support
ALTER TABLE cache_relations ADD COLUMN source_type TEXT DEFAULT 'extracted';
ALTER TABLE cache_relations ADD COLUMN hidden INTEGER DEFAULT 0;
ALTER TABLE cache_relations ADD COLUMN confidence REAL DEFAULT 0.5;
ALTER TABLE cache_relations ADD COLUMN namespace TEXT DEFAULT 'default';
ALTER TABLE cache_relations ADD COLUMN created_at TEXT;
ALTER TABLE cache_relations ADD COLUMN updated_at TEXT;
