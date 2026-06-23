"""HEIMDALL Pivot Moment Detection — life-transition narrative engine.

V3.0 Phase 2: detects bridge nodes whose connected memories have
timestamp gaps ≤7 days, indicating pivotal life transitions.
Generates human-readable narratives for monthly/annual summaries.

Algorithm:
  1. For each bridge entity, collect all connected memory edge timestamps
  2. Sort by time, find adjacent pairs with gap ≤7 days
  3. Score by: cross-community diversity × emotional magnitude × recency
  4. Generate narrative template from entity types + emotion deltas
"""

from __future__ import annotations

import logging
import time
from collections import defaultdict
from datetime import datetime
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)

PIVOT_WINDOW_SECONDS = 7 * 86400  # 7-day window for pivot detection
PIVOT_MIN_EMOTION_DELTA = 0.3     # minimum emotion change for significance
PIVOT_MAX_RESULTS = 10


class PivotDetector:
    """Detects pivotal life transition moments from the entity graph.

    A pivot moment occurs when a bridge entity connects memories from
    different domains within a short time window, indicating a life
    transition or breakthrough.

    Usage:
        detector = PivotDetector(store)
        pivots = detector.detect()
        # [{entity, timestamp, narrative, score, connected_memories}, ...]
    """

    def __init__(self, store: EntityStore):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def detect(self, top_n: int = PIVOT_MAX_RESULTS) -> list[dict]:
        """Detect pivot moments from bridge entities.

        Returns list of pivot moments sorted by significance score.
        """
        bridges = self._get_bridge_entities()
        if not bridges:
            logger.info("Pivot detection: no bridge entities found")
            return []

        pivots = []
        for bridge in bridges:
            eid = bridge["entity_id"]
            mem_edges = self._get_memory_timestamps(eid)
            if len(mem_edges) < 2:
                continue

            # Find timestamp gaps ≤7 days
            gaps = self._find_pivot_gaps(mem_edges)
            if not gaps:
                continue

            for gap in gaps:
                pivot = self._build_pivot(bridge, gap, mem_edges)
                if pivot:
                    pivots.append(pivot)

        # Sort by score descending
        pivots.sort(key=lambda p: p["score"], reverse=True)
        return pivots[:top_n]

    def detect_for_summary(self) -> list[dict]:
        """Detect pivots for monthly summary — capped at top 3."""
        return self.detect(top_n=3)

    # ------------------------------------------------------------------
    # Detection logic
    # ------------------------------------------------------------------

    def _find_pivot_gaps(self, edges: list[dict]) -> list[dict]:
        """Find pairs of memory edges within the 7-day pivot window.

        Each edge must have entity_type diversity (different domains)
        and sufficient emotional signal.
        """
        # Sort by timestamp
        sorted_edges = sorted(edges, key=lambda e: e.get("timestamp", 0))
        gaps = []

        for i in range(len(sorted_edges)):
            for j in range(i + 1, len(sorted_edges)):
                t1 = sorted_edges[i].get("timestamp", 0)
                t2 = sorted_edges[j].get("timestamp", 0)
                if t2 - t1 > PIVOT_WINDOW_SECONDS:
                    break  # too far apart

                e1_type = sorted_edges[i].get("entity_type", "")
                e2_type = sorted_edges[j].get("entity_type", "")
                domain_change = e1_type != e2_type

                e1_emotion = sorted_edges[i].get("emotion") or 0
                e2_emotion = sorted_edges[j].get("emotion") or 0
                emotion_delta = abs(e2_emotion - e1_emotion)

                if domain_change or emotion_delta > PIVOT_MIN_EMOTION_DELTA:
                    gaps.append({
                        "edge1": sorted_edges[i],
                        "edge2": sorted_edges[j],
                        "gap_days": round((t2 - t1) / 86400.0, 1),
                        "domain_change": domain_change,
                        "emotion_delta": round(emotion_delta, 3),
                    })

        return gaps

    def _build_pivot(
        self, bridge: dict, gap: dict, all_edges: list[dict]
    ) -> Optional[dict]:
        """Build a pivot moment narrative from a detected gap."""
        e1 = gap["edge1"]
        e2 = gap["edge2"]

        # Score components
        cross_community = 1.0 if gap["domain_change"] else 0.5
        emotion_magnitude = min(gap["emotion_delta"] / 0.5, 1.0)
        recency = self._recency_score(e2.get("timestamp", 0))
        bridge_score = bridge.get("bridge_score", 0.3)

        score = (
            0.35 * cross_community
            + 0.25 * emotion_magnitude
            + 0.15 * recency
            + 0.25 * min(bridge_score, 1.0)
        )

        # Generate narrative
        narrative = self._generate_narrative(
            bridge.get("display_name", "某件事"),
            bridge.get("entity_type", "concept"),
            gap,
        )

        return {
            "entity_id": bridge["entity_id"],
            "entity_name": bridge.get("display_name", "?"),
            "entity_type": bridge.get("entity_type", "concept"),
            "bridge_score": bridge.get("bridge_score", 0),
            "score": round(score, 4),
            "timestamp": e2.get("timestamp", 0),
            "date": datetime.fromtimestamp(e2.get("timestamp", time.time())).strftime("%Y-%m-%d"),
            "gap_days": gap["gap_days"],
            "emotion_delta": gap["emotion_delta"],
            "domain_change": gap["domain_change"],
            "narrative": narrative,
            "connected": [
                {
                    "timestamp": e1.get("timestamp"),
                    "date": datetime.fromtimestamp(e1.get("timestamp", 0)).strftime("%Y-%m-%d"),
                    "role": e1.get("role", ""),
                    "emotion": e1.get("emotion"),
                },
                {
                    "timestamp": e2.get("timestamp"),
                    "date": datetime.fromtimestamp(e2.get("timestamp", 0)).strftime("%Y-%m-%d"),
                    "role": e2.get("role", ""),
                    "emotion": e2.get("emotion"),
                },
            ],
        }

    def _generate_narrative(
        self, name: str, etype: str, gap: dict
    ) -> str:
        """Generate a human-readable pivot moment narrative.

        Templates follow V3.0 spec: describe what changed, connected domains,
        and emotional shift without making predictions.
        """
        e1_date = datetime.fromtimestamp(
            gap["edge1"].get("timestamp", 0)
        ).strftime("%m月%d日")
        e2_date = datetime.fromtimestamp(
            gap["edge2"].get("timestamp", 0)
        ).strftime("%m月%d日")

        if gap["domain_change"] and gap["emotion_delta"] > 0.3:
            return (
                f"{e1_date}到{e2_date}，'{name}'连接了不同领域，"
                f"情绪发生了显著变化——这可能是一个转折点"
            )
        elif gap["domain_change"]:
            return (
                f"{e1_date}开始接触'{name}'，在{e2_date}前产生了跨领域连接，"
                f"开始连接你的不同生活维度"
            )
        elif gap["emotion_delta"] > 0.3:
            direction = "上升" if gap["emotion_delta"] > 0 else "变化"
            return (
                f"{e1_date}到{e2_date}，围绕'{name}'的情绪发生了{direction}，"
                f"标志着一个重要时刻"
            )
        else:
            return (
                f"{e1_date}至{e2_date}，'{name}'成为连接不同记忆的枢纽"
            )

    def _recency_score(self, timestamp: float) -> float:
        """Score based on how recent the event is (1.0 = today, 0.5 = 1 year)."""
        now = time.time()
        days_ago = (now - timestamp) / 86400.0
        return max(0.1, 1.0 - days_ago / 365.0)

    # ------------------------------------------------------------------
    # Data fetching
    # ------------------------------------------------------------------

    def _get_bridge_entities(self) -> list[dict]:
        if not self._store._conn:
            return []
        rows = self._store._conn.execute(
            "SELECT * FROM heimdall_entities "
            "WHERE is_bridge = 1 AND status = 'active' "
            "ORDER BY bridge_score DESC"
        ).fetchall()
        return [dict(r) for r in rows]

    def _get_memory_timestamps(self, entity_id: str) -> list[dict]:
        """Get memory edges with entity types for a given entity."""
        if not self._store._conn:
            return []

        # Get edges directly connected to this entity
        rows = self._store._conn.execute(
            "SELECT me.timestamp, me.role, me.emotion, "
            "e.entity_type, e.display_name "
            "FROM heimdall_memory_edges me "
            "JOIN heimdall_entities e ON me.entity_id = e.entity_id "
            "WHERE me.entity_id = ? "
            "ORDER BY me.timestamp ASC",
            (entity_id,),
        ).fetchall()

        results = [dict(r) for r in rows]

        # Also get edges from socially connected entities (2-hop context)
        try:
            neighbors = self._store._conn.execute(
                "SELECT source_entity_id, target_entity_id FROM heimdall_social_graph "
                "WHERE source_entity_id = ? OR target_entity_id = ?",
                (entity_id, entity_id),
            ).fetchall()

            neighbor_ids = set()
            for nb in neighbors:
                nid = nb["source_entity_id"] if nb["target_entity_id"] == entity_id else nb["target_entity_id"]
                if nid != entity_id:
                    neighbor_ids.add(nid)

            if neighbor_ids:
                placeholders = ",".join("?" for _ in neighbor_ids)
                n_rows = self._store._conn.execute(
                    f"SELECT me.timestamp, me.role, me.emotion, "
                    f"e.entity_type, e.display_name "
                    f"FROM heimdall_memory_edges me "
                    f"JOIN heimdall_entities e ON me.entity_id = e.entity_id "
                    f"WHERE me.entity_id IN ({placeholders}) "
                    f"ORDER BY me.timestamp ASC",
                    list(neighbor_ids),
                ).fetchall()
                results.extend(dict(r) for r in n_rows)
        except Exception:
            pass

        return results
