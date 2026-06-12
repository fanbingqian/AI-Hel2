-- Migration 007: Merge 'content' entity type into '__file__'
-- Content entities were document anchors with name=filename — same as __file__
UPDATE cache_entities SET entity_type = '__file__' WHERE entity_type = 'content';
