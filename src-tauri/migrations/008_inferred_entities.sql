-- Migration 008: Add inferred column to cache_entities for transitive reasoning entities
ALTER TABLE cache_entities ADD COLUMN inferred INTEGER DEFAULT 0;
