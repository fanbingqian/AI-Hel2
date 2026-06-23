"""Obsolescence marking — detect and mark stale knowledge.

Entities naturally become outdated over time. This module provides:
  - TTL-based expiration checks per entity type
  - Manual mark-as-outdated functionality
  - Batch scan for stale entities
"""

from __future__ import annotations

import logging
import math
import time
from dataclasses import dataclass
from datetime import datetime, timedelta
from typing import Optional

logger = logging.getLogger(__name__)

# Default TTL in days per entity type
DEFAULT_TTL_DAYS: dict[str, int] = {
    "concept": 365,    # concepts are durable (~1 year)
    "skill": 180,      # skills can become outdated (~6 months)
    "tool": 90,        # tools change versions (~3 months)
    "event": 30,       # events are transient (~1 month)
    "person": 365,     # people info changes slowly
    "content": 180,    # content references may go stale
    "artifact": 90,
    "project": 90,
    "organization": 365,
    "location": 365,
}

DEFAULT_TTL = 180  # 6 months


@dataclass
class ObsolescenceResult:
    entity_id: str
    action: str  # "marked_outdated" | "already_archived" | "still_active"
    reason: str
    ttl_days: int
    age_days: int


class ObsolescenceManager:
    """Manage knowledge staleness through TTL and manual triggers."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def mark_outdated(
        self,
        entity_id: str,
        reason: str = "manual",
    ) -> dict:
        """Mark a single entity as outdated."""
        conn = self._store._conn
        now = time.strftime("%Y-%m-%dT%H:%M:%S")
        conn.execute(
            "UPDATE kr_entities SET status = 'outdated', updated_at = ? WHERE entity_id = ?",
            (now, entity_id),
        )
        conn.commit()
        return {"entity_id": entity_id, "status": "outdated", "updated_at": now}

    def scan_stale(
        self,
        namespace: str = "general",
        reference_date: Optional[str] = None,
    ) -> list[ObsolescenceResult]:
        """Scan for entities that have exceeded their TTL.

        Returns list of ObsolescenceResult with actions taken.
        """
        conn = self._store._conn
        rows = conn.execute(
            """SELECT entity_id, name, types, updated_at, created_at, ttl_days, status
               FROM kr_entities
               WHERE namespace = ? AND status NOT IN ('archived', 'outdated')""",
            (namespace,),
        ).fetchall()

        ref_date = (datetime.fromisoformat(reference_date) if reference_date
                    else datetime.now())

        results: list[ObsolescenceResult] = []
        for row in rows:
            import json

            # Determine TTL
            ttl = row["ttl_days"]
            if not ttl:
                types_raw = row["types"]
                try:
                    types_list = json.loads(types_raw) if isinstance(types_raw, str) else (types_raw or ["concept"])
                except Exception:
                    types_list = ["concept"]
                primary_type = types_list[0] if types_list else "concept"
                ttl = DEFAULT_TTL_DAYS.get(primary_type, DEFAULT_TTL)

            # Determine age
            ts_str = row["updated_at"] or row["created_at"]
            if not ts_str:
                continue

            try:
                entity_dt = datetime.fromisoformat(ts_str.replace("Z", "+00:00").split("+")[0].split("Z")[0])
            except (ValueError, TypeError):
                continue

            age_days = (ref_date - entity_dt.replace(tzinfo=None)).days

            if age_days > ttl:
                self.mark_outdated(row["entity_id"], reason=f"TTL expired: {age_days}d > {ttl}d")
                results.append(ObsolescenceResult(
                    entity_id=row["entity_id"],
                    action="marked_outdated",
                    reason=f"Age {age_days}d exceeds TTL {ttl}d",
                    ttl_days=ttl,
                    age_days=age_days,
                ))

        if results:
            logger.info(
                "Staleness scan: %d entities marked outdated (namespace=%s)",
                len(results), namespace,
            )

        return results

    def refresh(
        self,
        entity_id: str,
    ) -> dict:
        """Refresh an entity's timestamp (reset the TTL clock)."""
        conn = self._store._conn
        now = time.strftime("%Y-%m-%dT%H:%M:%S")
        conn.execute(
            "UPDATE kr_entities SET updated_at = ?, last_verified_at = ?, status = CASE WHEN status = 'outdated' THEN 'active' ELSE status END WHERE entity_id = ?",
            (now, now, entity_id),
        )
        conn.commit()
        return {"entity_id": entity_id, "refreshed_at": now}
