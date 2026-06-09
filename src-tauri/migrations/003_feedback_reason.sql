-- Migration 003: Add reason column to cache_extraction_feedback
ALTER TABLE cache_extraction_feedback ADD COLUMN reason TEXT DEFAULT '';
