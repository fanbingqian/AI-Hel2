"""HEIMDALL Importance Scoring Engine (P2.3).

Composite importance score for knowledge entities, combining confidence,
PageRank centrality, occurrence frequency, recency decay, and bridge score.

Formula:
  importance = 0.30×confidence + 0.30×pagerank_norm + 0.20×occurrence_norm
             + 0.15×recency + 0.05×bridge_score

Levels: critical (≥0.80), high (≥0.60), medium (≥0.35), low (<0.35)
"""

from __future__ import annotations

import logging
import math
import time
from typing import Optional

logger = logging.getLogger(__name__)

# Weight coefficients
W_CONFIDENCE = 0.30
W_PAGERANK = 0.30
W_OCCURRENCE = 0.20
W_RECENCY = 0.15
W_BRIDGE = 0.05

# Level thresholds
LEVEL_CRITICAL = 0.80
LEVEL_HIGH = 0.60
LEVEL_MEDIUM = 0.35

# Normalization caps
MAX_OCCURRENCE = 100  # cap for occurrence_count normalization
MAX_PAGERANK = 10.0   # cap for pagerank normalization
RECENCY_HALF_LIFE_DAYS = 30.0  # half-life for exponential recency decay


def importance_level(score: float) -> str:
    """Map importance score to a human-readable level."""
    if score >= LEVEL_CRITICAL:
        return "critical"
    if score >= LEVEL_HIGH:
        return "high"
    if score >= LEVEL_MEDIUM:
        return "medium"
    return "low"


def compute_importance(
    confidence: float = 0.5,
    pagerank: float = 1.0,
    occurrence_count: int = 1,
    last_seen_at: Optional[float] = None,
    bridge_score: float = 0.0,
) -> float:
    """Compute composite importance score for a single entity.

    All inputs are normalized to [0, 1] before weighting.
    """
    # Confidence: already in [0, 1]
    conf_norm = max(0.0, min(confidence, 1.0))

    # PageRank: normalize to [0, 1] with cap
    pr_norm = min(pagerank / MAX_PAGERANK, 1.0)

    # Occurrence: log-scale then cap normalize
    occ_norm = min(math.log1p(occurrence_count) / math.log1p(MAX_OCCURRENCE), 1.0)

    # Recency: exponential decay, 1.0 = just seen, approaches 0 over time
    if last_seen_at is not None:
        days = max(0.0, (time.time() - last_seen_at) / 86400.0)
        decay_rate = math.log(2) / RECENCY_HALF_LIFE_DAYS
        recency = math.exp(-decay_rate * days)
    else:
        recency = 0.5  # neutral default for unknown timestamps

    # Bridge: already in [0, 1] range
    bridge_norm = min(bridge_score, 1.0)

    score = (
        W_CONFIDENCE * conf_norm
        + W_PAGERANK * pr_norm
        + W_OCCURRENCE * occ_norm
        + W_RECENCY * recency
        + W_BRIDGE * bridge_norm
    )

    return round(max(0.0, min(score, 1.0)), 6)


def batch_compute_importance(entities: list[dict]) -> dict[str, tuple[float, str]]:
    """Compute importance for a list of entity dicts.

    Returns {entity_id: (score, level)} for all entities.
    """
    results: dict[str, tuple[float, str]] = {}
    for entity in entities:
        eid = entity.get("entity_id", "")
        if not eid:
            continue
        score = compute_importance(
            confidence=entity.get("confidence", 0.5),
            pagerank=entity.get("pagerank", 1.0),
            occurrence_count=entity.get("occurrence_count", 1),
            last_seen_at=entity.get("last_seen_at"),
            bridge_score=entity.get("bridge_score", 0.0),
        )
        results[eid] = (score, importance_level(score))
    return results


class ImportanceEngine:
    """Importance scoring engine backed by EntityStore.

    Usage:
        engine = ImportanceEngine(store)
        engine.recalc_all()                    # batch recalculate
        results = engine.get_top_entities(20)  # top-N by importance
    """

    def __init__(self, store):
        self._store = store

    def recalc_all(self, namespace: Optional[str] = None) -> int:
        """Recalculate importance for all active entities.

        Updates importance + importance_level columns in both heimdall_entities
        and kr_entities tables. Returns count of updated entities.
        """
        conn = self._store._conn
        if not conn:
            return 0

        updated = 0

        # -- Legacy heimdall_entities --
        try:
            if namespace:
                rows = conn.execute(
                    "SELECT entity_id, confidence, pagerank, occurrence_count, "
                    "last_seen_at, bridge_score FROM heimdall_entities "
                    "WHERE status = 'active' AND namespace = ?",
                    (namespace,),
                ).fetchall()
            else:
                rows = conn.execute(
                    "SELECT entity_id, confidence, pagerank, occurrence_count, "
                    "last_seen_at, bridge_score FROM heimdall_entities "
                    "WHERE status = 'active'"
                ).fetchall()

            for row in rows:
                score = compute_importance(
                    confidence=row["confidence"] or 0.5,
                    pagerank=row["pagerank"] or 1.0,
                    occurrence_count=row["occurrence_count"] or 1,
                    last_seen_at=row["last_seen_at"],
                    bridge_score=row["bridge_score"] or 0.0,
                )
                level = importance_level(score)
                conn.execute(
                    "UPDATE heimdall_entities SET importance = ?, importance_level = ? "
                    "WHERE entity_id = ?",
                    (score, level, row["entity_id"]),
                )
                updated += 1
        except Exception as e:
            logger.warning("heimdall_entities importance recalc skipped: %s", e)

        # -- Knowledge Ring kr_entities --
        try:
            if namespace:
                kr_rows = conn.execute(
                    "SELECT entity_id, confidence FROM kr_entities WHERE namespace = ?",
                    (namespace,),
                ).fetchall()
            else:
                kr_rows = conn.execute(
                    "SELECT entity_id, confidence FROM kr_entities"
                ).fetchall()

            for row in kr_rows:
                eid = row["entity_id"]
                # kr_entities don't have pagerank/bridge directly; use
                # confidence + occurrence from relation count as proxy
                rel_count_row = conn.execute(
                    "SELECT COUNT(*) as cnt FROM kr_relations "
                    "WHERE source_id = ? OR target_id = ?",
                    (eid, eid),
                ).fetchone()
                rel_count = rel_count_row["cnt"] if rel_count_row else 0

                # Get recency from updated_at
                updated_row = conn.execute(
                    "SELECT updated_at FROM kr_entities WHERE entity_id = ?",
                    (eid,),
                ).fetchone()

                score = compute_importance(
                    confidence=row["confidence"] or 0.5,
                    pagerank=1.0,  # kr_entities don't have pagerank yet
                    occurrence_count=max(rel_count, 1),
                    last_seen_at=None,  # use neutral default
                    bridge_score=0.0,
                )
                level = importance_level(score)
                conn.execute(
                    "UPDATE kr_entities SET importance = ?, importance_level = ? "
                    "WHERE entity_id = ?",
                    (score, level, eid),
                )
                updated += 1
        except Exception as e:
            logger.warning("kr_entities importance recalc skipped: %s", e)

        try:
            conn.commit()
        except Exception:
            pass

        logger.info("Importance recalc: %d entities updated", updated)
        return updated

    def get_top_entities(
        self, limit: int = 20, namespace: Optional[str] = None
    ) -> list[dict]:
        """Return top-N entities by importance score across both tables."""
        conn = self._store._conn
        if not conn:
            return []

        results: list[dict] = []

        try:
            ns_filter = "AND namespace = ?" if namespace else ""
            ns_params = (namespace,) if namespace else ()

            legacy_rows = conn.execute(
                f"SELECT entity_id, display_name AS name, entity_type, confidence, "
                f"pagerank, occurrence_count, bridge_score, importance, importance_level "
                f"FROM heimdall_entities "
                f"WHERE status = 'active' AND importance IS NOT NULL {ns_filter} "
                f"ORDER BY importance DESC LIMIT ?",
                (*ns_params, limit),
            ).fetchall()
            for r in legacy_rows:
                results.append(dict(r))
        except Exception:
            pass

        try:
            ns_filter2 = "AND namespace = ?" if namespace else ""
            ns_params2 = (namespace,) if namespace else ()

            kr_rows = conn.execute(
                f"SELECT entity_id, name, types AS entity_type, confidence, "
                f"importance, importance_level "
                f"FROM kr_entities "
                f"WHERE importance IS NOT NULL {ns_filter2} "
                f"ORDER BY importance DESC LIMIT ?",
                (*ns_params2, limit),
            ).fetchall()
            for r in kr_rows:
                d = dict(r)
                # Parse types JSON to single string
                if d.get("entity_type"):
                    try:
                        import json
                        types = json.loads(d["entity_type"])
                        d["entity_type"] = types[0] if types else "concept"
                    except Exception:
                        d["entity_type"] = "concept"
                results.append(d)
        except Exception:
            pass

        # Re-sort merged results
        results.sort(key=lambda x: x.get("importance", 0), reverse=True)
        return results[:limit]

    def get_level_counts(self, namespace: Optional[str] = None) -> dict[str, int]:
        """Return count of entities per importance level."""
        conn = self._store._conn
        if not conn:
            return {"critical": 0, "high": 0, "medium": 0, "low": 0}

        counts = {"critical": 0, "high": 0, "medium": 0, "low": 0}
        ns_filter = "AND namespace = ?" if namespace else ""
        ns_params = (namespace,) if namespace else ()

        try:
            rows = conn.execute(
                f"SELECT importance_level, COUNT(*) as cnt FROM heimdall_entities "
                f"WHERE status = 'active' AND importance_level IS NOT NULL {ns_filter} "
                f"GROUP BY importance_level",
                ns_params,
            ).fetchall()
            for r in rows:
                if r["importance_level"] in counts:
                    counts[r["importance_level"]] += r["cnt"]
        except Exception:
            pass

        try:
            kr_rows = conn.execute(
                f"SELECT importance_level, COUNT(*) as cnt FROM kr_entities "
                f"WHERE importance_level IS NOT NULL {ns_filter} "
                f"GROUP BY importance_level",
                ns_params,
            ).fetchall()
            for r in kr_rows:
                if r["importance_level"] in counts:
                    counts[r["importance_level"]] += r["cnt"]
        except Exception:
            pass

        return counts
