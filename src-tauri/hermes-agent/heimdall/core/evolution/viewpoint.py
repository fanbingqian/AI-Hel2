"""Viewpoint change tracking — detect how entity properties evolve over time.

Uses kr_entity_history to track what changed, when, and in what direction.
Surfaces entities with significant "drift" (many changes over time).
"""

from __future__ import annotations

import json
import logging
import time
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Any, Optional

logger = logging.getLogger(__name__)


@dataclass
class ViewpointChange:
    """A single property change for an entity."""
    field: str
    old_value: Optional[str]
    new_value: Optional[str]
    change_type: str  # "negation" | "replacement" | "deepening" | "update"
    timestamp: str
    source: str = ""


@dataclass
class ViewpointSummary:
    """Summary of viewpoint evolution for one entity."""
    entity_id: str
    changes: list[ViewpointChange]
    change_count: int
    latest_change_at: Optional[str]


class ViewpointTracker:
    """Track and summarize how entity properties evolve over time."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def get_evolution(self, entity_id: str) -> ViewpointSummary:
        """Get the full change history for an entity."""
        conn = self._store._conn
        rows = conn.execute(
            """SELECT field, old_value, new_value, change_type, source, timestamp
               FROM kr_entity_history
               WHERE entity_id = ?
               ORDER BY timestamp ASC""",
            (entity_id,),
        ).fetchall()

        changes = [
            ViewpointChange(
                field=r["field"],
                old_value=r["old_value"],
                new_value=r["new_value"],
                change_type=r["change_type"],
                timestamp=r["timestamp"],
                source=r["source"] or "",
            )
            for r in rows
        ]

        return ViewpointSummary(
            entity_id=entity_id,
            changes=changes,
            change_count=len(changes),
            latest_change_at=changes[-1].timestamp if changes else None,
        )

    def find_drifted(
        self,
        namespace: str = "general",
        min_changes: int = 2,
    ) -> list[dict]:
        """Find entities with significant drift (>= min_changes property changes)."""
        conn = self._store._conn
        rows = conn.execute(
            """SELECT entity_id, COUNT(*) as change_count,
                      GROUP_CONCAT(field) as fields_changed,
                      MAX(timestamp) as latest_change
               FROM kr_entity_history
               WHERE namespace = ?
               GROUP BY entity_id
               HAVING COUNT(*) >= ?
               ORDER BY change_count DESC""",
            (namespace, min_changes),
        ).fetchall()

        results: list[dict] = []
        for r in rows:
            fields = r["fields_changed"].split(",") if r["fields_changed"] else []
            results.append({
                "entity_id": r["entity_id"],
                "change_count": r["change_count"],
                "fields_changed": list(set(fields)),
                "latest_change": r["latest_change"],
            })

        logger.info(
            "Drift scan: %d entities with >=%d changes (namespace=%s)",
            len(results), min_changes, namespace,
        )
        return results

    def classify_changes(
        self, entity_id: str
    ) -> dict[str, list[dict]]:
        """Classify changes by type: negation, replacement, deepening."""
        summary = self.get_evolution(entity_id)
        classified: dict[str, list[dict]] = defaultdict(list)
        for c in summary.changes:
            classified[c.change_type].append({
                "field": c.field,
                "old_value": c.old_value,
                "new_value": c.new_value,
                "timestamp": c.timestamp,
            })
        return dict(classified)
