"""HEIMDALL Social Graph — 3D social relationship model.

Social relationships are modeled as three-dimensional vectors:
  - intensity:  0.0–1.0  (how strong is the relationship)
  - valence:    -1.0–1.0  (positive to negative sentiment)
  - volatility: 0.0–1.0  (how much the valence changes)

The social graph is materialized from memory edges via the entity store.
Health score = f(intensity, valence, volatility).

Knowledge Ring V1.0: Also reads from kr_relations for explicit 'knows' type
relationships, merging with historical memory_edges data.

Reconnect suggestions are derived from:
  - last_seen > 90 days (inactivity threshold)
  - evidence_count ≥ 10 (minimum interaction history)
  - intensity > 0 (had meaningful interaction)
"""

from __future__ import annotations

import logging
import time
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)

# Inactivity threshold for reconnect suggestions (90 days in seconds)
RECONNECT_INACTIVITY_SECONDS = 90 * 24 * 3600
# Minimum interaction count for reconnect consideration
RECONNECT_MIN_INTERACTIONS = 10


class SocialGraph:
    """Manages the 3D social graph derived from memory edges.

    The graph is NOT a separate store — it's a read-side projection
    of the entity store's heimdall_social_graph and heimdall_memory_edges
    tables. All writes go through EntityStore.
    """

    def __init__(self, store: EntityStore):
        self.store = store

    # ------------------------------------------------------------------
    # Edge management
    # ------------------------------------------------------------------

    def record_interaction(
        self,
        source_entity_id: str,
        target_entity_id: str,
        relationship_type: str = "",
        emotion: float = 0.0,
    ) -> None:
        """Record an interaction between two entities.

        Updates intensity, valence, and volatility using online updates.
        """
        self.store.upsert_social_edge(
            source_entity_id=source_entity_id,
            target_entity_id=target_entity_id,
            relationship_type=relationship_type,
            emotion=max(-1.0, min(1.0, emotion)),
        )

    # ------------------------------------------------------------------
    # Query
    # ------------------------------------------------------------------

    def get_connections(self, entity_id: str) -> list[dict]:
        """Get all social connections for an entity, ordered by intensity."""
        return self.store.get_social_edges(entity_id)

    def get_strongest_connections(self, entity_id: str, top_n: int = 5) -> list[dict]:
        """Get the strongest connections for an entity."""
        edges = self.store.get_social_edges(entity_id)
        return sorted(edges, key=lambda e: e.get("intensity", 0), reverse=True)[:top_n]

    def get_health_score(self, entity_id: str) -> float:
        """Aggregate health score across all connections for an entity."""
        edges = self.store.get_social_edges(entity_id)
        if not edges:
            return 0.5
        return sum(e.get("health_score", 0.5) for e in edges) / len(edges)

    # ------------------------------------------------------------------
    # Reconnect suggestions
    # ------------------------------------------------------------------

    def get_reconnect_suggestions(self) -> list[dict]:
        """Get social connections that may need reconnection.

        Criteria: inactive > 90 days, ≥ 10 past interactions.
        Returns list of dicts with target_name, intensity, last_seen, days_inactive.
        """
        edges = self.store.get_reconnect_suggestions(
            inactivity_seconds=RECONNECT_INACTIVITY_SECONDS
        )
        suggestions = []
        for edge in edges:
            days_inactive = (time.time() - edge.get("last_seen", 0)) / 86400
            suggestions.append({
                "entity_id": edge.get("target_entity_id"),
                "name": edge.get("target_name", "未知联系人"),
                "type": edge.get("target_type", "person"),
                "intensity": edge.get("intensity", 0),
                "valence": edge.get("valence", 0),
                "health_score": edge.get("health_score", 0.5),
                "last_seen": edge.get("last_seen", 0),
                "days_inactive": round(days_inactive),
                "interaction_count": edge.get("evidence_count", 0),
            })
        return suggestions

    # ------------------------------------------------------------------
    # Stats
    # ------------------------------------------------------------------

    def get_stats(self) -> dict:
        """Get social graph statistics."""
        if not self.store._conn:
            return {"total_edges": 0, "total_persons": 0, "avg_intensity": 0}

        row = self.store._conn.execute(
            "SELECT COUNT(*) as cnt, AVG(intensity) as avg_intensity FROM heimdall_social_graph"
        ).fetchone()
        persons = self.store._conn.execute(
            "SELECT COUNT(*) as cnt FROM heimdall_entities WHERE entity_type = 'person' AND status = 'active'"
        ).fetchone()

        return {
            "total_edges": row["cnt"] if row else 0,
            "total_persons": persons["cnt"] if persons else 0,
            "avg_intensity": round(row["avg_intensity"] or 0, 3) if row else 0,
        }

    # ------------------------------------------------------------------
    # Knowledge Ring V1.0 — knows relations from kr_relations
    # ------------------------------------------------------------------

    def get_knows_connections(self, entity_id: str = "") -> list[dict]:
        """Get explicit 'knows' relationships from the Knowledge Ring relations table.

        Merges with legacy social graph data for backward compatibility.
        If entity_id is empty, returns all knows relationships.
        """
        if not self.store._conn:
            return []

        if entity_id:
            rows = self.store._conn.execute(
                "SELECT r.*, e1.name as source_name, e1.type as source_type, "
                "e2.name as target_name, e2.type as target_type "
                "FROM kr_relations r "
                "JOIN kr_entities e1 ON r.source_id = e1.entity_id "
                "JOIN kr_entities e2 ON r.target_id = e2.entity_id "
                "WHERE r.type = 'knows' AND (r.source_id = ? OR r.target_id = ?) "
                "ORDER BY r.confidence DESC",
                (entity_id, entity_id),
            ).fetchall()
        else:
            rows = self.store._conn.execute(
                "SELECT r.*, e1.name as source_name, e1.type as source_type, "
                "e2.name as target_name, e2.type as target_type "
                "FROM kr_relations r "
                "JOIN kr_entities e1 ON r.source_id = e1.entity_id "
                "JOIN kr_entities e2 ON r.target_id = e2.entity_id "
                "WHERE r.type = 'knows' "
                "ORDER BY r.confidence DESC"
            ).fetchall()

        return [dict(r) for r in rows]

    def get_social_with_ring_data(self, entity_id: str = "") -> dict:
        """Get combined social data from both legacy social graph and Knowledge Ring.

        Returns:
            {"legacy": [...], "knows": [...], "reconnect": [...]}
        """
        return {
            "legacy": self.get_connections(entity_id) if entity_id else [],
            "knows": self.get_knows_connections(entity_id),
            "reconnect": self.get_reconnect_suggestions(),
        }
