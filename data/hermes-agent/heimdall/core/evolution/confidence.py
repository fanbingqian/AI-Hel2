"""Confidence evolution — dynamic confidence scores that evolve over time.

Confidence is not static. It changes through:
  - Manual boost/decay (user feedback)
  - Time-based decay: confidence(t) = confidence_0 * e^(-λt)
  - Verification events reset decay

Different entity types decay at different rates (configurable λ values).
"""

from __future__ import annotations

import logging
import math
import time
from dataclasses import dataclass
from typing import Optional

logger = logging.getLogger(__name__)

# Default decay rates (λ) per entity type — higher = faster decay
DEFAULT_DECAY_RATES: dict[str, float] = {
    "concept": 0.002,   # ~90% confidence after 1 year
    "skill": 0.003,     # skills can become outdated
    "tool": 0.02,       # tools change versions quickly
    "event": 0.05,      # events are transient
    "person": 0.005,    # people info changes slowly
    "content": 0.01,    # content references may go stale
    "artifact": 0.015,  # artifacts between tool and content
    "project": 0.02,    # projects evolve
    "organization": 0.005,
    "location": 0.002,
}

# Confidence thresholds
UNVERIFIED_THRESHOLD = 0.3   # < 0.3 → status = "unverified"
ARCHIVED_THRESHOLD = 0.1     # < 0.1 → status = "archived"
VERIFIED_THRESHOLD = 0.8     # >= 0.8 → status = "verified"


@dataclass
class ConfidenceResult:
    entity_id: str
    old_confidence: float
    new_confidence: float
    action: str  # "boosted" | "decayed" | "time_decayed" | "archived"


class ConfidenceManager:
    """Manage entity confidence scores and time-based decay."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def boost(
        self,
        entity_id: str,
        amount: float = 0.1,
        reason: str = "manual_confirm",
    ) -> ConfidenceResult:
        """Increase confidence for an entity (capped at 1.0)."""
        old = self._get_confidence(entity_id)
        new = min(1.0, old + amount)
        self._set_confidence(entity_id, new)
        self._record_change(entity_id, old, new, reason)

        if new >= VERIFIED_THRESHOLD and old < VERIFIED_THRESHOLD:
            self._update_status(entity_id, "verified")

        return ConfidenceResult(
            entity_id=entity_id,
            old_confidence=old,
            new_confidence=new,
            action="boosted",
        )

    def decay(
        self,
        entity_id: str,
        amount: float = 0.05,
        reason: str = "marked_outdated",
    ) -> ConfidenceResult:
        """Manually decrease confidence (floor at 0.0)."""
        old = self._get_confidence(entity_id)
        new = max(0.0, old - amount)
        self._set_confidence(entity_id, new)
        self._record_change(entity_id, old, new, reason)

        action = "decayed"
        if new < ARCHIVED_THRESHOLD:
            self._update_status(entity_id, "archived")
            action = "archived"
        elif new < UNVERIFIED_THRESHOLD:
            self._update_status(entity_id, "unverified")

        return ConfidenceResult(
            entity_id=entity_id,
            old_confidence=old,
            new_confidence=new,
            action=action,
        )

    def apply_time_decay(
        self,
        namespace: str = "general",
        reference_time: Optional[float] = None,
    ) -> list[ConfidenceResult]:
        """Apply exponential time decay to all entities in a namespace.

        confidence(t) = confidence_0 * exp(-λ * weeks_since_update)

        Only entities with `last_verified_at` set are decayed from that
        timestamp; others are decayed from `updated_at`.
        """
        ref = reference_time or time.time()
        conn = self._store._conn
        rows = conn.execute(
            """SELECT entity_id, confidence, types, updated_at, last_verified_at
               FROM kr_entities
               WHERE namespace = ? AND status NOT IN ('archived')""",
            (namespace,),
        ).fetchall()

        results: list[ConfidenceResult] = []
        for row in rows:
            entity_id = row["entity_id"]
            old_conf = row["confidence"] or 0.5

            # Determine the reference timestamp for decay
            verified_at = row["last_verified_at"]
            updated_at = row["updated_at"]
            ts_str = verified_at or updated_at
            if not ts_str:
                continue

            try:
                entity_ts = _parse_timestamp(ts_str)
            except (ValueError, TypeError):
                continue

            weeks_elapsed = max(0, (ref - entity_ts) / (7 * 24 * 3600))

            # Get decay rate for this entity's type
            types_raw = row["types"]
            try:
                import json
                types_list = json.loads(types_raw) if isinstance(types_raw, str) else (types_raw or ["concept"])
            except Exception:
                types_list = ["concept"]
            primary_type = types_list[0] if types_list else "concept"
            lam = DEFAULT_DECAY_RATES.get(primary_type, 0.01)

            new_conf = old_conf * math.exp(-lam * weeks_elapsed)
            new_conf = round(max(0.0, new_conf), 4)

            if abs(new_conf - old_conf) < 0.001:
                continue

            self._set_confidence(entity_id, new_conf)

            action = "time_decayed"
            if new_conf < ARCHIVED_THRESHOLD:
                self._update_status(entity_id, "archived")
                action = "archived"
            elif new_conf < UNVERIFIED_THRESHOLD:
                self._update_status(entity_id, "unverified")

            results.append(ConfidenceResult(
                entity_id=entity_id,
                old_confidence=old_conf,
                new_confidence=new_conf,
                action=action,
            ))

        if results:
            logger.info(
                "Time decay applied: %d entities updated (namespace=%s)",
                len(results), namespace,
            )

        return results

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _get_confidence(self, entity_id: str) -> float:
        conn = self._store._conn
        row = conn.execute(
            "SELECT confidence FROM kr_entities WHERE entity_id = ?", (entity_id,)
        ).fetchone()
        return row["confidence"] if row and row["confidence"] is not None else 0.5

    def _set_confidence(self, entity_id: str, value: float) -> None:
        conn = self._store._conn
        conn.execute(
            "UPDATE kr_entities SET confidence = ?, updated_at = ? WHERE entity_id = ?",
            (value, time.strftime("%Y-%m-%dT%H:%M:%S"), entity_id),
        )
        conn.commit()

    def _update_status(self, entity_id: str, status: str) -> None:
        conn = self._store._conn
        try:
            conn.execute(
                "UPDATE kr_entities SET status = ? WHERE entity_id = ?",
                (status, entity_id),
            )
            conn.commit()
        except Exception:
            pass  # status column might not exist yet (ALTER TABLE may have failed)

    def _record_change(
        self, entity_id: str, old: float, new: float, reason: str
    ) -> None:
        conn = self._store._conn
        import uuid as _uuid
        ns = _uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
        history_id = str(_uuid.uuid5(ns, f"{entity_id}|confidence|{time.time()}"))
        try:
            conn.execute(
                """INSERT INTO kr_entity_history
                   (history_id, entity_id, field, old_value, new_value, change_type, source, timestamp)
                   VALUES (?, ?, 'confidence', ?, ?, ?, ?, ?)""",
                (history_id, entity_id, str(old), str(new), "decay", reason,
                 time.strftime("%Y-%m-%dT%H:%M:%S")),
            )
            conn.commit()
        except Exception:
            pass


def _parse_timestamp(ts: str) -> float:
    """Parse various timestamp formats to Unix epoch."""
    import datetime
    formats = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S.%f",
        "%Y-%m-%d",
    ]
    for fmt in formats:
        try:
            dt = datetime.datetime.strptime(ts, fmt)
            return dt.timestamp()
        except ValueError:
            continue
    # ISO format with timezone
    try:
        dt = datetime.datetime.fromisoformat(ts)
        return dt.timestamp()
    except (ValueError, TypeError):
        raise ValueError(f"Cannot parse timestamp: {ts}")
