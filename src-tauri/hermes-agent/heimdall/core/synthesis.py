"""Cross-document synthesis — merge and deduplicate entities across sources.

When the same entity appears in multiple documents or sessions, synthesize
a unified view by merging properties, boosting confidence, and preserving
source provenance.
"""

from __future__ import annotations

import json
import logging
import re
import time
import uuid
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Any, Optional

logger = logging.getLogger(__name__)


@dataclass
class SynthesisResult:
    entity_id: str
    merged_from: list[str]  # source entity_ids that were merged
    final_confidence: float
    property_merges: int
    conflicts_resolved: int


class SynthesisEngine:
    """Synthesize a unified knowledge view across multiple sources."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def find_duplicates(
        self,
        namespace: str = "general",
        similarity_threshold: float = 0.8,
    ) -> list[tuple[str, str, float]]:
        """Find potential duplicate entities by name similarity.

        Returns list of (entity_id_a, entity_id_b, similarity_score).
        """
        conn = self._store._conn
        rows = conn.execute(
            "SELECT entity_id, name FROM kr_entities WHERE namespace = ? AND status != 'archived'",
            (namespace,),
        ).fetchall()

        entities = [(r["entity_id"], r["name"].lower().strip()) for r in rows]
        duplicates: list[tuple[str, str, float]] = []

        for i in range(len(entities)):
            for j in range(i + 1, len(entities)):
                eid_a, name_a = entities[i]
                eid_b, name_b = entities[j]
                sim = _name_similarity(name_a, name_b)
                if sim >= similarity_threshold:
                    duplicates.append((eid_a, eid_b, sim))

        return duplicates

    def merge_entities(
        self,
        primary_id: str,
        secondary_ids: list[str],
    ) -> SynthesisResult:
        """Merge secondary entities into the primary, keeping the primary as canonical.

        Properties from secondary entities are merged into primary.
        Relations pointing to secondary entities are re-pointed to primary.
        Secondary entities are marked as archived.
        """
        conn = self._store._conn
        primary = conn.execute(
            "SELECT * FROM kr_entities WHERE entity_id = ?", (primary_id,)
        ).fetchone()
        if not primary:
            raise ValueError(f"Primary entity {primary_id} not found")

        prop_merges = 0
        conflicts_resolved = 0

        primary_props = {}
        if primary["properties"]:
            try:
                primary_props = json.loads(primary["properties"]) if isinstance(primary["properties"], str) else primary["properties"]
            except Exception:
                pass

        for sid in secondary_ids:
            secondary = conn.execute(
                "SELECT * FROM kr_entities WHERE entity_id = ?", (sid,)
            ).fetchone()
            if not secondary:
                continue

            # Merge properties
            sec_props = {}
            if secondary["properties"]:
                try:
                    sec_props = json.loads(secondary["properties"]) if isinstance(secondary["properties"], str) else secondary["properties"]
                except Exception:
                    pass

            for key, val in sec_props.items():
                if key not in primary_props:
                    primary_props[key] = val
                    prop_merges += 1
                elif primary_props[key] != val:
                    # Record conflict for manual resolution
                    conflicts_resolved += 1

            # Re-point relations
            conn.execute(
                "UPDATE kr_relations SET source_id = ? WHERE source_id = ?",
                (primary_id, sid),
            )
            conn.execute(
                "UPDATE kr_relations SET target_id = ? WHERE target_id = ?",
                (primary_id, sid),
            )

            # Archive secondary
            conn.execute(
                "UPDATE kr_entities SET status = 'archived' WHERE entity_id = ?",
                (sid,),
            )

        # Update primary
        conn.execute(
            "UPDATE kr_entities SET properties = ?, confidence = MIN(1.0, confidence + ?), updated_at = ? WHERE entity_id = ?",
            (json.dumps(primary_props, ensure_ascii=False),
             0.05 * len(secondary_ids),
             time.strftime("%Y-%m-%dT%H:%M:%S"),
             primary_id),
        )
        conn.commit()

        new_conf = (primary["confidence"] or 0.5) + 0.05 * len(secondary_ids)

        logger.info(
            "Merged %d entities into %s: %d props, %d conflicts",
            len(secondary_ids), primary_id, prop_merges, conflicts_resolved,
        )

        return SynthesisResult(
            entity_id=primary_id,
            merged_from=secondary_ids,
            final_confidence=min(1.0, new_conf),
            property_merges=prop_merges,
            conflicts_resolved=conflicts_resolved,
        )

    def synthesize(
        self,
        namespace: str = "general",
        auto_merge_threshold: float = 0.95,
    ) -> list[SynthesisResult]:
        """Find and auto-merge high-confidence duplicates."""
        duplicates = self.find_duplicates(namespace)
        results: list[SynthesisResult] = []

        processed: set[str] = set()
        for eid_a, eid_b, sim in duplicates:
            if eid_a in processed or eid_b in processed:
                continue
            if sim >= auto_merge_threshold:
                result = self.merge_entities(eid_a, [eid_b])
                results.append(result)
                processed.add(eid_a)
                processed.add(eid_b)

        return results


def _name_similarity(a: str, b: str) -> float:
    """Simple trigram-based name similarity."""
    if a == b:
        return 1.0
    if not a or not b:
        return 0.0

    def trigrams(s: str) -> set[str]:
        s = f"  {s} "
        return {s[i:i+3] for i in range(len(s)-2)}

    ta = trigrams(a)
    tb = trigrams(b)
    if not ta or not tb:
        return 0.0
    intersection = ta & tb
    union = ta | tb
    return len(intersection) / len(union) if union else 0.0
