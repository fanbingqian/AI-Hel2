-- Migration 005: Add inferred column to cache_relations for transitive reasoning
ALTER TABLE cache_relations ADD COLUMN inferred INTEGER DEFAULT 0;
