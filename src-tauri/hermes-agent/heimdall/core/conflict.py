"""Conflict detection — identify contradictory information across sources.

When new information arrives for an existing entity, compare properties
to detect three conflict types:
  - NEGATION: A field had a value, now it's empty/removed
  - REPLACEMENT: A field had one value, now it has a different value
  - DEEPENING: A field was empty, now it has a value (not a conflict)
"""

from __future__ import annotations

import json
import logging
import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Optional

logger = logging.getLogger(__name__)


@dataclass
class Conflict:
    """A detected conflict for a specific entity field."""
    conflict_id: str
    entity_id: str
    field: str
    old_value: Optional[str]
    new_value: Optional[str]
    change_type: str  # "negation" | "replacement" | "deepening"
    source: str
    session_id: str
    namespace: str
    timestamp: str


class ConflictDetector:
    """Detect and manage conflicting entity property changes."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def check(
        self,
        entity_id: str,
        new_properties: dict[str, Any],
        source: str = "manual",
        session_id: str = "",
        namespace: str = "general",
    ) -> list[Conflict]:
        """Compare new properties against existing entity and return conflicts.

        Only fields that differ from the current state are reported.
        """
        existing = self._get_entity_properties(entity_id)
        if not existing:
            return []

        conflicts: list[Conflict] = []
        now = time.strftime("%Y-%m-%dT%H:%M:%S")

        for field, new_val in new_properties.items():
            old_val = existing.get(field)
            old_str = self._serialize(old_val)
            new_str = self._serialize(new_val)

            if old_str == new_str:
                continue

            change_type = self._classify(old_val, new_val)

            conflict = Conflict(
                conflict_id=_gen_conflict_id(entity_id, field, now),
                entity_id=entity_id,
                field=field,
                old_value=old_str,
                new_value=new_str,
                change_type=change_type,
                source=source,
                session_id=session_id,
                namespace=namespace,
                timestamp=now,
            )
            conflicts.append(conflict)

        return conflicts

    def scan_namespace(
        self, namespace: str = "general", status: str = "pending"
    ) -> list[dict]:
        """List unresolved conflict records for a namespace."""
        conn = self._store._conn
        rows = conn.execute(
            """SELECT * FROM kr_entity_history
               WHERE namespace = ? AND change_type IN ('negation', 'replacement')
               ORDER BY timestamp DESC""",
            (namespace,),
        ).fetchall()
        return [dict(r) for r in rows]

    def resolve(
        self,
        history_id: str,
        resolution: str,
    ) -> dict:
        """Resolve a conflict record.

        resolution: "accept_new" | "keep_old" | "merge" | "dismiss"
        Returns the updated history record.
        """
        conn = self._store._conn
        now = time.strftime("%Y-%m-%dT%H:%M:%S")
        conn.execute(
            """UPDATE kr_entity_history
               SET change_type = ?, new_value = json_set(
                 COALESCE(new_value, '{}'), '$.resolution', ?, '$.resolved_at', ?
               )
               WHERE history_id = ?""",
            ("confirmation", resolution, now, history_id),
        )
        conn.commit()

        # If accepting new value, update the entity directly
        if resolution in ("accept_new", "merge"):
            row = conn.execute(
                "SELECT entity_id, field, new_value FROM kr_entity_history WHERE history_id = ?",
                (history_id,),
            ).fetchone()
            if row:
                try:
                    new_val = json.loads(row["new_value"]) if row["new_value"] else None
                except (json.JSONDecodeError, TypeError):
                    new_val = row["new_value"]
                self._apply_resolution(row["entity_id"], row["field"], new_val)

        return {"history_id": history_id, "resolution": resolution, "resolved_at": now}

    def record_change(
        self,
        entity_id: str,
        field: str,
        old_value: Optional[str],
        new_value: Optional[str],
        change_type: str = "update",
        source: str = "manual",
        session_id: str = "",
        namespace: str = "general",
    ) -> str:
        """Record any property change to kr_entity_history (not just conflicts)."""
        history_id = _gen_conflict_id(entity_id, field, time.strftime("%Y-%m-%dT%H:%M:%S"))
        conn = self._store._conn
        conn.execute(
            """INSERT INTO kr_entity_history
               (history_id, entity_id, field, old_value, new_value, change_type,
                source, session_id, namespace, timestamp)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (history_id, entity_id, field, old_value, new_value, change_type,
             source, session_id, namespace, time.strftime("%Y-%m-%dT%H:%M:%S")),
        )
        conn.commit()
        return history_id

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _get_entity_properties(self, entity_id: str) -> dict[str, Any]:
        conn = self._store._conn
        row = conn.execute(
            "SELECT properties FROM kr_entities WHERE entity_id = ?", (entity_id,)
        ).fetchone()
        if not row or not row["properties"]:
            return {}
        try:
            return json.loads(row["properties"]) if isinstance(row["properties"], str) else row["properties"]
        except (json.JSONDecodeError, TypeError):
            return {}

    def _serialize(self, val: Any) -> Optional[str]:
        if val is None:
            return None
        if isinstance(val, str):
            return val
        return json.dumps(val, ensure_ascii=False)

    def _classify(self, old_val: Any, new_val: Any) -> str:
        """Classify the type of change."""
        old_empty = old_val is None or old_val == "" or old_val == []
        new_empty = new_val is None or new_val == "" or new_val == []

        if old_empty and not new_empty:
            return "deepening"
        if not old_empty and new_empty:
            return "negation"
        return "replacement"

    def _apply_resolution(self, entity_id: str, field: str, value: Any) -> None:
        """Update an entity's property field with the resolved value."""
        conn = self._store._conn
        row = conn.execute(
            "SELECT properties FROM kr_entities WHERE entity_id = ?", (entity_id,)
        ).fetchone()
        if not row:
            return

        props = {}
        if row["properties"]:
            try:
                props = json.loads(row["properties"]) if isinstance(row["properties"], str) else row["properties"]
            except (json.JSONDecodeError, TypeError):
                pass

        import copy
        props = copy.deepcopy(props) if props else {}
        props[field] = value

        conn.execute(
            "UPDATE kr_entities SET properties = ?, updated_at = ? WHERE entity_id = ?",
            (json.dumps(props, ensure_ascii=False), time.strftime("%Y-%m-%dT%H:%M:%S"), entity_id),
        )
        conn.commit()


def _gen_conflict_id(entity_id: str, field: str, timestamp: str) -> str:
    ns = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
    raw = f"{entity_id}|{field}|{timestamp}"
    return str(uuid.uuid5(ns, raw))
