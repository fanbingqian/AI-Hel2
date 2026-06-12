-- Migration 006: Add community_id to cache_entities for graph community collapse
ALTER TABLE cache_entities ADD COLUMN community_id INTEGER;
